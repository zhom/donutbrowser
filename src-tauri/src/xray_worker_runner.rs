use crate::proxy_runner::find_sidecar_executable;
#[cfg(unix)]
use crate::proxy_storage::is_process_running;
use crate::proxy_storage::{process_identity_matches, resolve_process_start_time};
use crate::xray::{build_client_config_json, parse_share_link, XrayClientRuntime};
use crate::xray_worker_storage::{
  create_xray_worker_log, delete_xray_worker_config, generate_xray_worker_id,
  get_xray_worker_config, get_xray_worker_config_from_path, list_xray_worker_configs,
  save_xray_worker_config, save_xray_worker_config_to_path, unstarted_worker_is_stale,
  write_xray_runtime_config, xray_worker_config_path, xray_worker_log_path, XrayWorkerConfig,
};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const READY_CHECK_TIMEOUT: Duration = Duration::from_millis(750);
static XRAY_BINARY_VERIFIED: AtomicBool = AtomicBool::new(false);
static XRAY_START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn structured_error(code: &str) -> Box<dyn std::error::Error> {
  serde_json::json!({ "code": code }).to_string().into()
}

fn structured_error_with_detail(
  code: &str,
  detail: impl std::fmt::Display,
) -> Box<dyn std::error::Error> {
  serde_json::json!({ "code": code, "params": { "detail": detail.to_string() } })
    .to_string()
    .into()
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_major_version(version: &str) -> Option<u64> {
  version.trim().split('.').next()?.parse().ok()
}

#[cfg(target_os = "macos")]
fn ensure_supported_macos_version() -> Result<(), Box<dyn std::error::Error>> {
  let output = Command::new("/usr/bin/sw_vers")
    .arg("-productVersion")
    .output()
    .map_err(|_| structured_error("XRAY_UNAVAILABLE"))?;
  let version = String::from_utf8_lossy(&output.stdout);
  if !output.status.success() {
    return Err(structured_error("XRAY_UNAVAILABLE"));
  }
  if parse_macos_major_version(&version).is_some_and(|major| major < 12) {
    return Err(structured_error("XRAY_UNSUPPORTED_OS"));
  }
  Ok(())
}

fn ensure_xray_binary() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
  #[cfg(target_os = "macos")]
  ensure_supported_macos_version()?;

  let executable =
    find_sidecar_executable("xray").map_err(|_| structured_error("XRAY_UNAVAILABLE"))?;
  if XRAY_BINARY_VERIFIED.load(Ordering::Acquire) {
    return Ok(executable);
  }

  let mut command = Command::new(&executable);
  command.arg("version");
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
  }

  let output = command
    .output()
    .map_err(|_| structured_error("XRAY_UNAVAILABLE"))?;
  let version_output = String::from_utf8_lossy(&output.stdout);
  if !output.status.success() || !version_output.trim_start().starts_with("Xray ") {
    return Err(structured_error("XRAY_UNAVAILABLE"));
  }

  XRAY_BINARY_VERIFIED.store(true, Ordering::Release);
  Ok(executable)
}

async fn authenticated_socks_ready(config: &XrayWorkerConfig) -> bool {
  if !config
    .xray_pid
    .is_some_and(|pid| process_identity_matches(pid, config.xray_pid_start_time))
  {
    return false;
  }

  matches!(
    tokio::time::timeout(READY_CHECK_TIMEOUT, async {
      let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", config.local_port)).await?;
      stream.write_all(&[5, 1, 2]).await?;
      let mut method = [0_u8; 2];
      stream.read_exact(&mut method).await?;
      if method != [5, 2] {
        return Err(std::io::Error::other(
          "Xray SOCKS endpoint did not request password authentication",
        ));
      }

      let username = config.username.as_bytes();
      let password = config.password.as_bytes();
      let mut auth = Vec::with_capacity(username.len() + password.len() + 3);
      auth.extend_from_slice(&[1, username.len() as u8]);
      auth.extend_from_slice(username);
      auth.push(password.len() as u8);
      auth.extend_from_slice(password);
      stream.write_all(&auth).await?;
      let mut response = [0_u8; 2];
      stream.read_exact(&mut response).await?;
      if response != [1, 0] {
        return Err(std::io::Error::other(
          "Xray SOCKS endpoint rejected local authentication",
        ));
      }
      Ok::<(), std::io::Error>(())
    })
    .await,
    Ok(Ok(()))
  )
}

async fn wait_until_ready(
  id: &str,
  supervisor_pid: u32,
  supervisor_start_time: u64,
) -> Result<XrayWorkerConfig, Box<dyn std::error::Error>> {
  let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
  loop {
    if let Some(config) = get_xray_worker_config(id) {
      if !process_identity_matches(supervisor_pid, Some(supervisor_start_time)) {
        let log = std::fs::read_to_string(xray_worker_log_path(id)).unwrap_or_default();
        let tail: String = {
          let chars: Vec<char> = log.trim().chars().collect();
          chars[chars.len().saturating_sub(1500)..].iter().collect()
        };
        log::warn!(
          "Xray worker exited during startup id={id} pid={supervisor_pid} log=\"{}\"",
          crate::log_redaction::Plain(&crate::log_redaction::text(&tail))
        );
        return Err(structured_error("XRAY_START_FAILED"));
      }
      if config.pid == Some(supervisor_pid)
        && config.pid_start_time == Some(supervisor_start_time)
        && config.ready
        && authenticated_socks_ready(&config).await
      {
        return Ok(config);
      }
    }

    if tokio::time::Instant::now() >= deadline {
      return Err(structured_error("XRAY_START_FAILED"));
    }
    tokio::time::sleep(POLL_INTERVAL).await;
  }
}

pub async fn start_xray_worker(
  profile_id: Option<&str>,
  vless_uri: &str,
) -> Result<XrayWorkerConfig, Box<dyn std::error::Error>> {
  start_xray_worker_for_owner(profile_id, vless_uri, std::process::id()).await
}

/// Starts a worker that lives until `owner_pid` exits, or until it is stopped
/// or re-bound with `set_browser_pid`. The `donut-proxy xray start` command
/// passes the pid of the process that called it, because its own pid is gone
/// the moment it prints the result.
pub async fn start_xray_worker_for_owner(
  profile_id: Option<&str>,
  vless_uri: &str,
  owner_pid: u32,
) -> Result<XrayWorkerConfig, Box<dyn std::error::Error>> {
  let _start_guard = XRAY_START_LOCK.lock().await;
  parse_share_link(vless_uri)
    .map_err(|error| -> Box<dyn std::error::Error> { crate::vless_config_error(&error).into() })?;
  crate::proxy_runner::ensure_sidecar_version().await?;
  ensure_xray_binary()?;
  let owner_start_time =
    resolve_process_start_time(owner_pid).ok_or_else(|| structured_error("XRAY_START_FAILED"))?;

  for config in list_xray_worker_configs() {
    let dead = config
      .pid
      .is_some_and(|pid| !process_identity_matches(pid, config.pid_start_time));
    if dead || unstarted_worker_is_stale(&config) {
      let _ = stop_xray_worker(&config.id).await;
    }
  }

  if let Some(profile_id) = profile_id {
    let mut workers = list_xray_worker_configs()
      .into_iter()
      .filter(|config| config.profile_id.as_deref() == Some(profile_id))
      .collect::<Vec<_>>();
    workers.sort_unstable_by_key(|config| std::cmp::Reverse(config.created_at));
    let mut reusable = None;
    for mut existing in workers {
      let candidate = reusable.is_none()
        && existing.vless_uri == vless_uri
        && existing
          .pid
          .is_some_and(|pid| process_identity_matches(pid, existing.pid_start_time))
        && existing.ready
        && worker_is_leased_to(&existing, owner_pid, owner_start_time);
      if candidate
        && persist_browser_identity(&mut existing, owner_pid, owner_start_time)
        && authenticated_socks_ready(&existing).await
      {
        reusable = Some(existing);
      } else {
        let _ = stop_xray_worker(&existing.id).await;
      }
    }
    if let Some(existing) = reusable {
      return Ok(existing);
    }
  }

  let mut last_error = None;
  for attempt in 1..=3 {
    match spawn_xray_worker(profile_id, vless_uri, owner_pid, owner_start_time).await {
      Ok(worker) => return Ok(worker),
      Err(error) => {
        log::debug!("Xray worker start attempt failed attempt={attempt}/3 err=\"{error}\"");
        last_error = Some(error.to_string());
      }
    }
  }
  Err(
    last_error
      .map(Into::into)
      .unwrap_or_else(|| structured_error("XRAY_START_FAILED")),
  )
}

async fn spawn_xray_worker(
  profile_id: Option<&str>,
  vless_uri: &str,
  owner_pid: u32,
  owner_start_time: u64,
) -> Result<XrayWorkerConfig, Box<dyn std::error::Error>> {
  let started = std::time::Instant::now();
  let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?;
  let local_port = listener
    .local_addr()
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?
    .port();
  drop(listener);

  let id = generate_xray_worker_id();
  let mut pending = PendingSupervisor {
    id: id.clone(),
    pid: None,
    pid_start_time: None,
    armed: true,
  };
  let username = format!("donut_{}", uuid::Uuid::new_v4().simple());
  let password = uuid::Uuid::new_v4().simple().to_string();
  let mut config = XrayWorkerConfig::new(
    id.clone(),
    profile_id.map(str::to_string),
    vless_uri.to_string(),
    local_port,
    username,
    password,
  );
  config.browser_pid = Some(owner_pid);
  config.browser_pid_start_time = Some(owner_start_time);
  save_xray_worker_config(&config)
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?;

  let supervisor = find_sidecar_executable("donut-proxy")
    .map_err(|_| structured_error("PROXY_SIDECAR_VERSION_MISMATCH"))?;
  let config_path = xray_worker_config_path(&id);
  let log_file = create_xray_worker_log(&id)
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?;
  let mut command = Command::new(supervisor);
  command
    .arg("xray-worker")
    .arg("start")
    .arg("--config-path")
    .arg(&config_path)
    .stdin(Stdio::null())
    .stdout(Stdio::from(log_file.try_clone().map_err(|error| {
      structured_error_with_detail("XRAY_START_FAILED", error)
    })?))
    .stderr(Stdio::from(log_file));

  #[cfg(unix)]
  {
    use std::os::unix::process::CommandExt;
    unsafe {
      command.pre_exec(|| {
        if libc::setsid() == -1 {
          return Err(std::io::Error::last_os_error());
        }
        Ok(())
      });
    }
  }

  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
  }

  let mut child = command
    .spawn()
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?;
  let supervisor_pid = child.id();
  let Some(supervisor_start_time) = resolve_process_start_time(supervisor_pid) else {
    terminate_supervisor_unchecked(supervisor_pid);
    let _ = child.wait();
    return Err(structured_error("XRAY_START_FAILED"));
  };
  pending.pid = Some(supervisor_pid);
  pending.pid_start_time = Some(supervisor_start_time);
  drop(spawn_supervisor_reaper(child));

  let ready = wait_until_ready(&id, supervisor_pid, supervisor_start_time).await?;
  pending.armed = false;
  log::info!(
    "Xray worker started id={id} pid={supervisor_pid} xray_pid={} port={local_port} profile={} elapsed_ms={}",
    ready
      .xray_pid
      .map(|pid| pid.to_string())
      .unwrap_or_else(|| "none".to_string()),
    profile_id.unwrap_or("none"),
    started.elapsed().as_millis()
  );
  Ok(ready)
}

pub fn set_browser_pid(worker_id: &str, browser_pid: u32) -> bool {
  if browser_pid == 0 {
    return false;
  }
  let Some(browser_pid_start_time) = resolve_process_start_time(browser_pid) else {
    log::warn!(
      "Browser start time unknown, Xray worker owner not recorded xray_worker={worker_id} browser_pid={browser_pid}"
    );
    return false;
  };
  let Some(mut config) = get_xray_worker_config(worker_id) else {
    return false;
  };
  persist_browser_identity(&mut config, browser_pid, browser_pid_start_time)
}

fn persist_browser_identity(
  config: &mut XrayWorkerConfig,
  browser_pid: u32,
  browser_pid_start_time: u64,
) -> bool {
  config.browser_pid = Some(browser_pid);
  config.browser_pid_start_time = Some(browser_pid_start_time);
  if !crate::xray_worker_storage::update_xray_worker_config(config) {
    log::warn!(
      "Xray worker owner write failed xray_worker={} browser_pid={browser_pid}",
      config.id
    );
    return false;
  }
  true
}

fn worker_is_leased_to(config: &XrayWorkerConfig, pid: u32, start_time: u64) -> bool {
  config.browser_pid == Some(pid) && config.browser_pid_start_time == Some(start_time)
}

struct PendingSupervisor {
  id: String,
  pid: Option<u32>,
  pid_start_time: Option<u64>,
  armed: bool,
}

fn spawn_supervisor_reaper(mut child: Child) -> std::thread::JoinHandle<()> {
  std::thread::spawn(move || {
    let _ = child.wait();
  })
}

impl Drop for PendingSupervisor {
  fn drop(&mut self) {
    if self.armed {
      let xray_process = get_xray_worker_config(&self.id)
        .and_then(|config| config.xray_pid.zip(config.xray_pid_start_time));
      delete_xray_worker_config(&self.id);
      if let Some(pid) = self.pid {
        if self.pid_start_time.is_none() || process_identity_matches(pid, self.pid_start_time) {
          terminate_supervisor_unchecked(pid);
          log::info!(
            "Xray worker stopped id={} pid={pid} reason=start_failed",
            self.id
          );
        }
      }
      if let Some((pid, start_time)) = xray_process {
        if process_identity_matches(pid, Some(start_time)) {
          terminate_process_unchecked(pid);
        }
      }
      delete_xray_worker_config(&self.id);
    }
  }
}

#[cfg(unix)]
fn terminate_supervisor_unchecked(pid: u32) {
  let group = -(pid as i32);
  unsafe {
    libc::kill(group, libc::SIGTERM);
  }
  std::thread::sleep(Duration::from_millis(250));
  if is_process_running(pid) {
    unsafe {
      libc::kill(group, libc::SIGKILL);
    }
  }
}

#[cfg(windows)]
fn terminate_supervisor_unchecked(pid: u32) {
  use std::os::windows::process::CommandExt;
  const CREATE_NO_WINDOW: u32 = 0x08000000;
  let _ = Command::new("taskkill")
    .args(["/T", "/F", "/PID", &pid.to_string()])
    .creation_flags(CREATE_NO_WINDOW)
    .output();
}

#[cfg(unix)]
fn terminate_process_unchecked(pid: u32) {
  unsafe {
    libc::kill(pid as i32, libc::SIGTERM);
  }
  std::thread::sleep(Duration::from_millis(250));
  if is_process_running(pid) {
    unsafe {
      libc::kill(pid as i32, libc::SIGKILL);
    }
  }
}

#[cfg(windows)]
fn terminate_process_unchecked(pid: u32) {
  use std::os::windows::process::CommandExt;
  const CREATE_NO_WINDOW: u32 = 0x08000000;
  let _ = Command::new("taskkill")
    .args(["/F", "/PID", &pid.to_string()])
    .creation_flags(CREATE_NO_WINDOW)
    .output();
}

pub fn stop_xray_worker_now(id: &str) -> Result<bool, Box<dyn std::error::Error>> {
  let Some(config) = get_xray_worker_config(id) else {
    return Ok(false);
  };

  delete_xray_worker_config(id);
  let supervisor = config
    .pid
    .filter(|pid| process_identity_matches(*pid, config.pid_start_time));
  if let Some(pid) = supervisor {
    terminate_supervisor_unchecked(pid);
  }
  let xray = config
    .xray_pid
    .filter(|pid| process_identity_matches(*pid, config.xray_pid_start_time));
  if let Some(pid) = xray {
    terminate_process_unchecked(pid);
  }
  delete_xray_worker_config(id);
  let pid_text = |pid: Option<u32>| {
    pid
      .map(|pid| pid.to_string())
      .unwrap_or_else(|| "none".to_string())
  };
  log::info!(
    "Xray worker stopped id={id} pid={} xray_pid={} profile={} how={}",
    pid_text(config.pid),
    pid_text(config.xray_pid),
    config.profile_id.as_deref().unwrap_or("none"),
    if supervisor.is_some() || xray.is_some() {
      "terminated"
    } else {
      "already_gone"
    }
  );
  Ok(true)
}

pub async fn stop_xray_worker(id: &str) -> Result<bool, Box<dyn std::error::Error>> {
  stop_xray_worker_now(id)
}

pub async fn stop_xray_worker_by_profile_id(
  profile_id: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
  let workers = list_xray_worker_configs()
    .into_iter()
    .filter(|config| config.profile_id.as_deref() == Some(profile_id))
    .collect::<Vec<_>>();
  let mut stopped = false;
  for worker in workers {
    stopped |= stop_xray_worker(&worker.id).await?;
  }
  Ok(stopped)
}

struct ManagedChild {
  child: Child,
}

impl ManagedChild {
  fn new(child: Child) -> Self {
    Self { child }
  }

  fn id(&self) -> u32 {
    self.child.id()
  }

  fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
    self.child.try_wait()
  }
}

impl Drop for ManagedChild {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

pub async fn run_xray_worker(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
  let mut config = get_xray_worker_config_from_path(config_path)
    .ok_or_else(|| structured_error("XRAY_START_FAILED"))?;
  let supervisor_pid = std::process::id();
  config.ready = false;
  config.xray_pid = None;
  config.xray_pid_start_time = None;
  config.pid = Some(supervisor_pid);
  config.pid_start_time = Some(
    resolve_process_start_time(supervisor_pid)
      .ok_or_else(|| structured_error("XRAY_START_FAILED"))?,
  );
  save_xray_worker_config_to_path(&config, config_path)
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?;
  let parsed = parse_share_link(&config.vless_uri)
    .map_err(|error| -> Box<dyn std::error::Error> { crate::vless_config_error(&error).into() })?;
  let runtime = XrayClientRuntime {
    listen_port: config.local_port,
    username: config.username.clone(),
    password: config.password.clone(),
  };
  let runtime_json = build_client_config_json(&parsed.config, &runtime)
    .map_err(|error| -> Box<dyn std::error::Error> { crate::vless_config_error(&error).into() })?;
  write_xray_runtime_config(&config.id, runtime_json.as_bytes())
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?;
  let runtime_path = crate::xray_worker_storage::xray_runtime_config_path(&config.id);

  let executable = ensure_xray_binary()?;
  let mut command = Command::new(executable);
  command
    .arg("run")
    .arg("-c")
    .arg(&runtime_path)
    .stdin(Stdio::null())
    .stdout(Stdio::inherit())
    .stderr(Stdio::inherit());

  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
  }

  let child = command
    .spawn()
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?;
  let mut child = ManagedChild::new(child);
  let xray_pid = child.id();
  config.xray_pid = Some(xray_pid);
  config.xray_pid_start_time = Some(
    resolve_process_start_time(xray_pid).ok_or_else(|| structured_error("XRAY_START_FAILED"))?,
  );
  save_xray_worker_config_to_path(&config, config_path)
    .map_err(|error| structured_error_with_detail("XRAY_START_FAILED", error))?;

  let readiness_result: Result<(), Box<dyn std::error::Error>> = async {
    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    loop {
      match child.try_wait() {
        Ok(Some(_)) => return Err(structured_error("XRAY_START_FAILED")),
        Ok(None) => {}
        Err(error) => {
          return Err(structured_error_with_detail("XRAY_START_FAILED", error));
        }
      }

      if authenticated_socks_ready(&config).await {
        tokio::time::sleep(POLL_INTERVAL).await;
        match child.try_wait() {
          Ok(Some(_)) => return Err(structured_error("XRAY_START_FAILED")),
          Ok(None) => {}
          Err(error) => {
            return Err(structured_error_with_detail("XRAY_START_FAILED", error));
          }
        }
        if authenticated_socks_ready(&config).await {
          return Ok(());
        }
      }

      if tokio::time::Instant::now() >= deadline {
        return Err(structured_error("XRAY_START_FAILED"));
      }
      tokio::time::sleep(POLL_INTERVAL).await;
    }
  }
  .await;
  if let Err(error) = readiness_result {
    drop(child);
    delete_xray_worker_config(&config.id);
    return Err(error);
  }

  config.ready = true;
  if let Err(error) = save_xray_worker_config_to_path(&config, config_path) {
    drop(child);
    delete_xray_worker_config(&config.id);
    return Err(structured_error_with_detail("XRAY_START_FAILED", error));
  }

  let mut state_misses = 0_u8;
  let mut browser_misses = 0_u8;
  let result = loop {
    match child.try_wait() {
      Ok(Some(_)) => break Err(structured_error("XRAY_START_FAILED")),
      Ok(None) => {}
      Err(error) => break Err(structured_error_with_detail("XRAY_START_FAILED", error)),
    }
    match get_xray_worker_config_from_path(config_path) {
      Some(latest) => {
        state_misses = 0;
        if let Some(browser_pid) = latest.browser_pid {
          if process_identity_matches(browser_pid, latest.browser_pid_start_time) {
            browser_misses = 0;
          } else {
            browser_misses += 1;
            if browser_misses >= 2 {
              break Ok(());
            }
          }
        } else {
          browser_misses = 0;
        }
      }
      None => {
        state_misses += 1;
        if state_misses >= 2 {
          break Ok(());
        }
      }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
  };

  drop(child);
  delete_xray_worker_config(&config.id);
  result
}

#[cfg(test)]
#[path = "xray_worker_runner_tests.rs"]
mod tests;

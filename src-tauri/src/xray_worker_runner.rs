use crate::proxy_runner::find_sidecar_executable;
#[cfg(unix)]
use crate::proxy_storage::is_process_running;
use crate::proxy_storage::{process_identity_matches, process_start_time};
use crate::xray::{build_client_config_json, parse_vless_uri, XrayClientRuntime};
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

fn resolve_process_start_time(pid: u32) -> Option<u64> {
  for _ in 0..25 {
    if let Some(start_time) = process_start_time(pid) {
      return Some(start_time);
    }
    std::thread::sleep(Duration::from_millis(10));
  }
  None
}

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
        if !log.is_empty() {
          log::error!("Xray worker {id} exited during startup: {log}");
        }
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
  let _start_guard = XRAY_START_LOCK.lock().await;
  parse_vless_uri(vless_uri)
    .map_err(|error| structured_error_with_detail("VLESS_CONFIG_INVALID", error))?;
  crate::proxy_runner::ensure_sidecar_version().await?;
  ensure_xray_binary()?;
  let owner_pid = std::process::id();
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
    match spawn_xray_worker(profile_id, vless_uri).await {
      Ok(worker) => return Ok(worker),
      Err(error) => {
        if attempt < 3 {
          log::warn!(
            "Xray worker startup attempt {attempt} failed; retrying with a new local port"
          );
        }
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
) -> Result<XrayWorkerConfig, Box<dyn std::error::Error>> {
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
  let owner_pid = std::process::id();
  config.browser_pid = Some(owner_pid);
  config.browser_pid_start_time = Some(
    resolve_process_start_time(owner_pid).ok_or_else(|| structured_error("XRAY_START_FAILED"))?,
  );
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
  Ok(ready)
}

pub fn set_browser_pid(worker_id: &str, browser_pid: u32) -> bool {
  if browser_pid == 0 {
    return false;
  }
  let Some(browser_pid_start_time) = resolve_process_start_time(browser_pid) else {
    log::warn!("Failed to resolve browser PID {browser_pid} identity for Xray worker {worker_id}");
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
      "Failed to persist browser PID {browser_pid} on Xray worker {}",
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
  if let Some(pid) = config
    .pid
    .filter(|pid| process_identity_matches(*pid, config.pid_start_time))
  {
    terminate_supervisor_unchecked(pid);
  }
  if let Some(pid) = config
    .xray_pid
    .filter(|pid| process_identity_matches(*pid, config.xray_pid_start_time))
  {
    terminate_process_unchecked(pid);
  }
  delete_xray_worker_config(id);
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
  let parsed = parse_vless_uri(&config.vless_uri)
    .map_err(|error| structured_error_with_detail("VLESS_CONFIG_INVALID", error))?;
  let runtime = XrayClientRuntime {
    listen_port: config.local_port,
    username: config.username.clone(),
    password: config.password.clone(),
  };
  let runtime_json = build_client_config_json(&parsed.config, &runtime)
    .map_err(|error| structured_error_with_detail("VLESS_CONFIG_INVALID", error))?;
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
mod tests {
  use super::*;

  #[cfg(unix)]
  fn short_lived_child() -> Child {
    Command::new("sh")
      .args(["-c", "sleep 0.2"])
      .spawn()
      .unwrap()
  }

  #[cfg(windows)]
  fn short_lived_child() -> Child {
    Command::new("cmd")
      .args(["/C", "ping -n 2 127.0.0.1 >NUL"])
      .spawn()
      .unwrap()
  }

  #[test]
  fn supervisor_child_is_reaped_after_exit() {
    let child = short_lived_child();
    let pid = child.id();
    let start_time = resolve_process_start_time(pid).unwrap();
    spawn_supervisor_reaper(child).join().unwrap();
    assert!(!process_identity_matches(pid, Some(start_time)));
  }

  fn readiness_config(port: u16) -> XrayWorkerConfig {
    let mut config = XrayWorkerConfig::new(
      "readiness".to_string(),
      None,
      "vless://unused".to_string(),
      port,
      "local-user".to_string(),
      "local-password".to_string(),
    );
    let pid = std::process::id();
    config.xray_pid = Some(pid);
    config.xray_pid_start_time = process_start_time(pid);
    config
  }

  #[test]
  fn parses_macos_major_versions_for_sidecar_compatibility() {
    assert_eq!(parse_macos_major_version("11.7.10\n"), Some(11));
    assert_eq!(parse_macos_major_version("12.0"), Some(12));
    assert_eq!(parse_macos_major_version("15.5.1"), Some(15));
    assert_eq!(parse_macos_major_version("unknown"), None);
  }

  #[tokio::test]
  async fn readiness_requires_the_expected_authenticated_socks_endpoint() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
      .await
      .unwrap();
    let config = readiness_config(listener.local_addr().unwrap().port());
    let expected_username = config.username.clone();
    let expected_password = config.password.clone();
    let server = tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let mut greeting = [0_u8; 3];
      stream.read_exact(&mut greeting).await.unwrap();
      assert_eq!(greeting, [5, 1, 2]);
      stream.write_all(&[5, 2]).await.unwrap();

      let mut header = [0_u8; 2];
      stream.read_exact(&mut header).await.unwrap();
      assert_eq!(header[0], 1);
      let mut username = vec![0_u8; header[1] as usize];
      stream.read_exact(&mut username).await.unwrap();
      let password_len = stream.read_u8().await.unwrap();
      let mut password = vec![0_u8; password_len as usize];
      stream.read_exact(&mut password).await.unwrap();
      assert_eq!(username, expected_username.as_bytes());
      assert_eq!(password, expected_password.as_bytes());
      stream.write_all(&[1, 0]).await.unwrap();
    });

    assert!(authenticated_socks_ready(&config).await);
    server.await.unwrap();
  }

  #[tokio::test]
  async fn readiness_rejects_an_unrelated_listener_on_the_reserved_port() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
      .await
      .unwrap();
    let config = readiness_config(listener.local_addr().unwrap().port());
    let server = tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let mut greeting = [0_u8; 3];
      stream.read_exact(&mut greeting).await.unwrap();
      stream.write_all(&[5, 0]).await.unwrap();
    });

    assert!(!authenticated_socks_ready(&config).await);
    server.await.unwrap();
  }

  #[test]
  fn browser_identity_is_persisted_on_the_exact_worker() {
    let temp = tempfile::tempdir().unwrap();
    let _cache_guard = crate::app_dirs::set_test_cache_dir(temp.path().to_path_buf());
    let id = format!("xray-browser-owner-{}", uuid::Uuid::new_v4());
    let config = XrayWorkerConfig::new(
      id.clone(),
      Some("profile".to_string()),
      "vless://unused".to_string(),
      1080,
      "local-user".to_string(),
      "local-password".to_string(),
    );
    save_xray_worker_config(&config).unwrap();

    let browser_pid = std::process::id();
    assert!(set_browser_pid(&id, browser_pid));
    let saved = get_xray_worker_config(&id).unwrap();
    assert_eq!(saved.browser_pid, Some(browser_pid));
    assert_eq!(
      saved.browser_pid_start_time,
      process_start_time(browser_pid)
    );

    assert!(delete_xray_worker_config(&id));
  }

  #[test]
  fn reusable_worker_requires_and_persists_the_exact_live_owner() {
    let temp = tempfile::tempdir().unwrap();
    let _cache_guard = crate::app_dirs::set_test_cache_dir(temp.path().to_path_buf());
    let id = format!("xray-worker-lease-{}", uuid::Uuid::new_v4());
    let mut config = XrayWorkerConfig::new(
      id.clone(),
      Some("profile".to_string()),
      "vless://unused".to_string(),
      1080,
      "local-user".to_string(),
      "local-password".to_string(),
    );
    config.browser_pid = Some(u32::MAX);
    config.browser_pid_start_time = Some(1);
    save_xray_worker_config(&config).unwrap();

    let owner_pid = std::process::id();
    let owner_start_time = process_start_time(owner_pid).unwrap();
    assert!(!worker_is_leased_to(&config, owner_pid, owner_start_time));
    assert!(persist_browser_identity(
      &mut config,
      owner_pid,
      owner_start_time
    ));
    assert!(worker_is_leased_to(&config, owner_pid, owner_start_time));
    let saved = get_xray_worker_config(&id).unwrap();
    assert_eq!(saved.browser_pid, Some(owner_pid));
    assert_eq!(saved.browser_pid_start_time, Some(owner_start_time));

    assert!(delete_xray_worker_config(&id));
  }
}

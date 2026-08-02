use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
  pub id: String,
  pub upstream_url: String, // Can be "DIRECT" for direct proxy
  pub local_port: Option<u16>,
  pub ignore_proxy_certificate: Option<bool>,
  pub local_url: Option<String>,
  pub pid: Option<u32>,
  #[serde(default)]
  pub profile_id: Option<String>,
  #[serde(default)]
  pub bypass_rules: Vec<String>,
  #[serde(default)]
  pub blocklist_file: Option<String>,
  /// When true, `blocklist_file` is treated as an ALLOW list: the browser may
  /// only reach domains in the file; everything else is blocked.
  #[serde(default)]
  pub dns_allowlist_mode: bool,
  /// Protocol the local worker serves to the browser: "socks5" (Wayfern/Chromium so QUIC and
  /// WebRTC UDP can be proxied without leaking the real IP). Independent of
  /// `upstream_url`, which is the real upstream proxy/VPN this worker dials.
  #[serde(default)]
  pub local_protocol: Option<String>,
  /// PID of the browser process this worker serves, recorded by the GUI after
  /// launch. The detached worker watches this and self-terminates when the
  /// browser dies, so it dies with its browser even if the GUI has exited or
  /// restarted. `None` until launch completes (the worker keeps running while
  /// it is `None`, up to `UNCLAIMED_WORKER_GRACE_SECS`).
  #[serde(default)]
  pub browser_pid: Option<u32>,
  /// Start time of `browser_pid`, pinning it to one exact process. Without it a
  /// recycled PID reads as "my browser is still alive" and the worker outlives
  /// its browser forever — the orphan users report. `None` on configs written
  /// before this field existed; those fall back to a bare existence check so an
  /// upgrade never reaps a worker whose browser is still running.
  #[serde(default)]
  pub browser_pid_start_time: Option<u64>,
}

impl ProxyConfig {
  pub fn new(id: String, upstream_url: String, local_port: Option<u16>) -> Self {
    Self {
      id,
      upstream_url,
      local_port,
      ignore_proxy_certificate: None,
      local_url: None,
      pid: None,
      profile_id: None,
      bypass_rules: Vec::new(),
      blocklist_file: None,
      dns_allowlist_mode: false,
      local_protocol: None,
      browser_pid: None,
      browser_pid_start_time: None,
    }
  }

  pub fn with_profile_id(mut self, profile_id: Option<String>) -> Self {
    self.profile_id = profile_id;
    self
  }

  pub fn with_bypass_rules(mut self, bypass_rules: Vec<String>) -> Self {
    self.bypass_rules = bypass_rules;
    self
  }

  pub fn with_blocklist_file(mut self, blocklist_file: Option<String>) -> Self {
    self.blocklist_file = blocklist_file;
    self
  }

  pub fn with_dns_allowlist_mode(mut self, allowlist_mode: bool) -> Self {
    self.dns_allowlist_mode = allowlist_mode;
    self
  }

  pub fn with_local_protocol(mut self, local_protocol: Option<String>) -> Self {
    self.local_protocol = local_protocol;
    self
  }

  /// "socks5" or "http" (default). Lowercased for case-insensitive matching.
  pub fn local_protocol_or_default(&self) -> String {
    self
      .local_protocol
      .as_deref()
      .unwrap_or("http")
      .to_lowercase()
  }
}

pub fn build_proxy_url(
  proxy_type: &str,
  host: &str,
  port: u16,
  username: Option<&str>,
  password: Option<&str>,
) -> String {
  let mut url = format!("{}://", proxy_type.to_lowercase());
  if let (Some(user), Some(pass)) = (username, password) {
    url.push_str(&format!(
      "{}:{}@",
      urlencoding::encode(user),
      urlencoding::encode(pass)
    ));
  } else if let Some(user) = username {
    url.push_str(&format!("{}@", urlencoding::encode(user)));
  }
  url.push_str(host);
  url.push(':');
  url.push_str(&port.to_string());
  url
}

pub fn get_storage_dir() -> PathBuf {
  crate::app_dirs::proxy_workers_dir()
}

pub fn save_proxy_config(config: &ProxyConfig) -> Result<(), Box<dyn std::error::Error>> {
  let storage_dir = get_storage_dir();
  fs::create_dir_all(&storage_dir)?;

  let file_path = storage_dir.join(format!("{}.json", config.id));
  let content = serde_json::to_string_pretty(config)?;
  crate::app_dirs::write_owner_only(&file_path, content.as_bytes())?;

  Ok(())
}

pub fn get_proxy_config(id: &str) -> Option<ProxyConfig> {
  let storage_dir = get_storage_dir();
  let file_path = storage_dir.join(format!("{}.json", id));

  if !file_path.exists() {
    return None;
  }

  match fs::read_to_string(&file_path) {
    Ok(content) => serde_json::from_str(&content).ok(),
    Err(_) => None,
  }
}

pub fn delete_proxy_config(id: &str) -> bool {
  let storage_dir = get_storage_dir();
  let file_path = storage_dir.join(format!("{}.json", id));

  if !file_path.exists() {
    return false;
  }

  fs::remove_file(&file_path).is_ok()
}

pub fn list_proxy_configs() -> Vec<ProxyConfig> {
  let storage_dir = get_storage_dir();

  if !storage_dir.exists() {
    return Vec::new();
  }

  let mut configs = Vec::new();
  if let Ok(entries) = fs::read_dir(&storage_dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().is_some_and(|ext| ext == "json") {
        if let Ok(content) = fs::read_to_string(&path) {
          if let Ok(config) = serde_json::from_str::<ProxyConfig>(&content) {
            configs.push(config);
          }
        }
      }
    }
  }

  configs
}

pub fn update_proxy_config(config: &ProxyConfig) -> bool {
  let storage_dir = get_storage_dir();
  let file_path = storage_dir.join(format!("{}.json", config.id));

  if !file_path.exists() {
    return false;
  }

  let Ok(content) = serde_json::to_string_pretty(config) else {
    return false;
  };
  if crate::app_dirs::write_owner_only(&file_path, content.as_bytes()).is_err() {
    return false;
  }
  true
}

pub fn generate_proxy_id() -> String {
  format!(
    "proxy_{}_{}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs(),
    rand::random::<u32>()
  )
}

/// Has this process exited even though the process table still lists it?
///
/// Both platforms have a way for a dead process to keep its PID and its
/// process-table entry, which makes a bare existence check report it as alive.
///
/// Unix: a zombie, exited but not reaped by its parent. It persists for the
/// whole life of the parent, and since the workers and browsers we spawn are
/// detached and never waited on, that is forever. Treating one as running is
/// what would keep a dead browser's worker alive.
///
/// Windows: no zombie status, but the same hazard. The process object outlives
/// the last thread that exited, and its PID stays reserved for as long as any
/// handle to it is open, so a snapshot taken right after a kill can still list
/// a process that is already gone. `WaitForSingleObject` with a zero timeout
/// answers authoritatively: a signalled process object has exited, whatever the
/// snapshot says.
fn process_has_exited(process: &sysinfo::Process) -> bool {
  if process.status() == sysinfo::ProcessStatus::Zombie {
    return true;
  }

  #[cfg(target_os = "windows")]
  {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
      OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    unsafe {
      // Opening fails for processes we are not allowed to touch (another user,
      // elevated, protected). Those we cannot query, so keep the snapshot's
      // verdict rather than declaring a live process dead.
      let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, process.pid().as_u32()) else {
        return false;
      };
      let signalled = WaitForSingleObject(handle, 0) == WAIT_OBJECT_0;
      let _ = CloseHandle(handle);
      signalled
    }
  }

  #[cfg(not(target_os = "windows"))]
  {
    false
  }
}

pub fn process_start_time(pid: u32) -> Option<u64> {
  use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
  let pid = sysinfo::Pid::from_u32(pid);
  let mut system = System::new();
  system.refresh_processes_specifics(
    ProcessesToUpdate::Some(&[pid]),
    true,
    ProcessRefreshKind::nothing(),
  );
  system
    .process(pid)
    .filter(|process| !process_has_exited(process))
    .map(sysinfo::Process::start_time)
}

/// Whether a process from an existing `System` snapshot is genuinely alive.
/// Callers that already hold a full scan use this instead of re-querying, but
/// must still exclude the dead-yet-listed the same way `process_start_time`
/// does.
pub fn snapshot_process_is_alive(process: &sysinfo::Process) -> bool {
  !process_has_exited(process)
}

pub fn is_process_running(pid: u32) -> bool {
  process_start_time(pid).is_some()
}

pub fn process_identity_matches(pid: u32, expected_start_time: Option<u64>) -> bool {
  expected_start_time.is_some_and(|expected| process_start_time(pid) == Some(expected))
}

/// Read a just-spawned process's start time, retrying briefly. A process is not
/// always visible in the process table the instant `spawn` returns, and giving
/// up would leave the caller unable to pin the PID to an identity.
pub fn resolve_process_start_time(pid: u32) -> Option<u64> {
  for _ in 0..25 {
    if let Some(start_time) = process_start_time(pid) {
      return Some(start_time);
    }
    std::thread::sleep(std::time::Duration::from_millis(10));
  }
  None
}

/// Seconds since a worker config's id was minted. Ids are
/// `proxy_{unix_secs}_{rand}` (see `generate_proxy_id`); anything else, or a
/// timestamp in the future, reads as age 0 so callers stay conservative.
pub fn proxy_config_age_secs(id: &str) -> u64 {
  let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
    return 0;
  };
  id.strip_prefix("proxy_")
    .and_then(|rest| rest.split('_').next())
    .and_then(|secs| secs.parse::<u64>().ok())
    .map(|created_at| now.as_secs().saturating_sub(created_at))
    .unwrap_or(0)
}

/// How long a worker may run without the GUI claiming it by recording a browser
/// PID. Covers the launch window (worker starts before the browser); past it the
/// GUI that spawned this worker is gone and nothing will ever claim it.
pub const UNCLAIMED_WORKER_GRACE_SECS: u64 = 300;

/// Is the browser this worker serves still the same live process?
///
/// Identity-checked whenever a start time was recorded. Configs written before
/// `browser_pid_start_time` existed fall back to a bare existence check, so
/// upgrading never reaps a worker whose browser is still running.
pub fn browser_owner_is_alive(config: &ProxyConfig) -> bool {
  let Some(pid) = config.browser_pid.filter(|pid| *pid != 0) else {
    return false;
  };
  match config.browser_pid_start_time {
    Some(start_time) => process_identity_matches(pid, Some(start_time)),
    None => is_process_running(pid),
  }
}

/// What the detached worker's self-reaping supervisor should do this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorVerdict {
  /// Keep serving.
  Keep,
  /// Our config is gone — the GUI stopped us, or the worker was superseded.
  ExitConfigRemoved,
  /// The browser we were started for is gone (debounced by the caller).
  ExitOwnerGone,
  /// No browser was ever recorded and the launch window has long passed.
  ExitNeverClaimed,
}

/// Pure decision table for the worker supervisor. `owner_alive` is injected so
/// every branch is testable without spawning browsers; production passes
/// `browser_owner_is_alive`.
pub fn supervisor_verdict(
  config: Option<&ProxyConfig>,
  age_secs: u64,
  owner_alive: impl Fn(&ProxyConfig) -> bool,
) -> SupervisorVerdict {
  let Some(config) = config else {
    return SupervisorVerdict::ExitConfigRemoved;
  };

  match config.browser_pid {
    Some(pid) if pid != 0 => {
      if owner_alive(config) {
        SupervisorVerdict::Keep
      } else {
        SupervisorVerdict::ExitOwnerGone
      }
    }
    // Never claimed. Keep serving through the launch window — the browser may
    // still be starting — then give up rather than idling forever.
    _ => {
      if age_secs >= UNCLAIMED_WORKER_GRACE_SECS {
        SupervisorVerdict::ExitNeverClaimed
      } else {
        SupervisorVerdict::Keep
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn proxy_url_encodes_credentials() {
    let url = build_proxy_url(
      "HTTP",
      "test.example.com",
      8080,
      Some("user@domain.com"),
      Some("pass word!"),
    );
    assert_eq!(
      url,
      "http://user%40domain.com:pass%20word%21@test.example.com:8080"
    );
  }

  #[test]
  fn process_identity_requires_the_observed_start_time() {
    let pid = std::process::id();
    let start_time = process_start_time(pid).expect("current process should be visible");
    assert!(process_identity_matches(pid, Some(start_time)));
    assert!(!process_identity_matches(
      pid,
      Some(start_time.saturating_add(1))
    ));
    assert!(!process_identity_matches(pid, None));
  }

  #[test]
  fn test_is_process_running_detects_current_process() {
    let pid = std::process::id();
    assert!(
      is_process_running(pid),
      "is_process_running must detect the current process (PID {pid})"
    );
  }

  #[test]
  fn test_is_process_running_returns_false_for_dead_pid() {
    // Spawn a short-lived child and wait for it to exit
    let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
      .args(if cfg!(windows) {
        vec!["/C", "exit"]
      } else {
        vec![]
      })
      .spawn()
      .expect("failed to spawn child");
    let pid = child.id();
    child.wait().expect("child failed");

    // `child` is deliberately still in scope. On Windows its open handle keeps
    // the exited process a live kernel object that the snapshot still reports,
    // which is exactly the case `process_has_exited` has to see through.
    assert!(
      !is_process_running(pid),
      "is_process_running must return false for a dead process (PID {pid})"
    );
  }

  fn owned_config(browser_pid: Option<u32>, browser_pid_start_time: Option<u64>) -> ProxyConfig {
    let mut config = ProxyConfig::new("proxy_1_2".to_string(), "DIRECT".to_string(), Some(1080));
    config.browser_pid = browser_pid;
    config.browser_pid_start_time = browser_pid_start_time;
    config
  }

  #[test]
  fn owner_liveness_is_pinned_to_the_exact_process_that_was_recorded() {
    let pid = std::process::id();
    let start_time = process_start_time(pid).expect("current process should be visible");

    assert!(browser_owner_is_alive(&owned_config(
      Some(pid),
      Some(start_time)
    )));
    // Same PID, different process: what a recycled PID looks like. Treating
    // this as alive is what stranded workers forever.
    assert!(!browser_owner_is_alive(&owned_config(
      Some(pid),
      Some(start_time.saturating_add(1))
    )));
    // Written before start times were recorded: existence is all we have, and
    // an upgrade must not reap a worker whose browser is still running.
    assert!(browser_owner_is_alive(&owned_config(Some(pid), None)));
    assert!(!browser_owner_is_alive(&owned_config(
      Some(u32::MAX),
      Some(start_time)
    )));
    assert!(!browser_owner_is_alive(&owned_config(None, None)));
    assert!(!browser_owner_is_alive(&owned_config(Some(0), None)));
  }

  #[test]
  fn supervisor_keeps_serving_a_live_owner_and_exits_a_dead_one() {
    let alive = |_: &ProxyConfig| true;
    let dead = |_: &ProxyConfig| false;
    let claimed = owned_config(Some(4321), Some(99));

    assert_eq!(
      supervisor_verdict(Some(&claimed), 0, alive),
      SupervisorVerdict::Keep
    );
    assert_eq!(
      supervisor_verdict(Some(&claimed), 0, dead),
      SupervisorVerdict::ExitOwnerGone
    );
    // A live owner outranks age: a long-running browser is not an orphan.
    assert_eq!(
      supervisor_verdict(Some(&claimed), UNCLAIMED_WORKER_GRACE_SECS * 10, alive),
      SupervisorVerdict::Keep
    );
  }

  #[test]
  fn supervisor_exits_when_its_config_is_gone() {
    assert_eq!(
      supervisor_verdict(None, 0, |_| true),
      SupervisorVerdict::ExitConfigRemoved
    );
  }

  #[test]
  fn supervisor_gives_up_on_a_worker_no_browser_ever_claimed() {
    // The GUI records the owner right after launch. Inside the window the
    // browser may still be starting, so keep serving...
    for unclaimed in [owned_config(None, None), owned_config(Some(0), None)] {
      assert_eq!(
        supervisor_verdict(Some(&unclaimed), 0, |_| false),
        SupervisorVerdict::Keep
      );
      assert_eq!(
        supervisor_verdict(
          Some(&unclaimed),
          UNCLAIMED_WORKER_GRACE_SECS.saturating_sub(1),
          |_| false
        ),
        SupervisorVerdict::Keep
      );
      // ...past it the GUI that spawned this worker is gone and nothing will
      // ever claim it, so it must not idle forever.
      assert_eq!(
        supervisor_verdict(Some(&unclaimed), UNCLAIMED_WORKER_GRACE_SECS, |_| false),
        SupervisorVerdict::ExitNeverClaimed
      );
    }
  }

  #[test]
  fn config_age_is_read_from_the_generated_id() {
    let fresh = generate_proxy_id();
    assert!(
      proxy_config_age_secs(&fresh) <= 1,
      "a just-generated id should read as brand new, got {}",
      proxy_config_age_secs(&fresh)
    );

    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs();
    assert_eq!(
      proxy_config_age_secs(&format!("proxy_{}_123", now.saturating_sub(600))),
      600
    );
    // Unparsable and future-dated ids read as brand new so callers stay
    // conservative rather than reaping something they can't date.
    assert_eq!(proxy_config_age_secs("not-a-proxy-id"), 0);
    assert_eq!(proxy_config_age_secs("proxy_abc_123"), 0);
    assert_eq!(proxy_config_age_secs(&format!("proxy_{}_1", now + 600)), 0);
  }

  /// An exited child that nobody has waited on stays in the process table as a
  /// zombie for as long as its parent lives — and everything we spawn is
  /// detached and never waited on. Reporting one as running would keep a dead
  /// browser's worker alive for the whole life of the app.
  #[cfg(unix)]
  #[test]
  fn an_exited_but_unreaped_child_is_not_running() {
    let child = std::process::Command::new("true")
      .stdin(std::process::Stdio::null())
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .spawn()
      .expect("failed to spawn child");
    let pid = child.id();
    let start_time = process_start_time(pid);

    // Deliberately leak the handle: waiting would reap the zombie and destroy
    // the very condition under test.
    std::mem::forget(child);

    let mut observed_dead = false;
    for _ in 0..100 {
      if !is_process_running(pid) {
        observed_dead = true;
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
      observed_dead,
      "an exited child that was never waited on must not read as running (PID {pid})"
    );
    // The same must hold for the identity check the workers actually use.
    assert!(!process_identity_matches(pid, start_time));

    // Reap it now so the test process doesn't leave one behind.
    unsafe {
      libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), 0);
    }
  }

  #[test]
  fn configs_written_before_owner_start_times_still_load() {
    let legacy = serde_json::json!({
      "id": "proxy_1_2",
      "upstream_url": "DIRECT",
      "local_port": 1080,
      "ignore_proxy_certificate": null,
      "local_url": "http://127.0.0.1:1080",
      "pid": 42,
      "browser_pid": 4242
    });
    let config: ProxyConfig = serde_json::from_value(legacy).unwrap();
    assert_eq!(config.browser_pid, Some(4242));
    assert_eq!(config.browser_pid_start_time, None);
  }

  #[test]
  fn test_is_process_running_returns_false_for_nonexistent_pid() {
    // PID 0 is the "System Idle Process" on Windows and sysinfo reports it as running,
    // so only assert on non-Windows platforms where PID 0 is not a real user process.
    #[cfg(not(windows))]
    assert!(
      !is_process_running(0),
      "is_process_running must return false for PID 0"
    );
    // Very high PID unlikely to exist
    assert!(
      !is_process_running(u32::MAX),
      "is_process_running must return false for PID u32::MAX"
    );
  }
}

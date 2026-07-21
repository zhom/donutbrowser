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
  /// it is `None`).
  #[serde(default)]
  pub browser_pid: Option<u32>,
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

pub fn is_process_running(pid: u32) -> bool {
  use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
  let pid = sysinfo::Pid::from_u32(pid);
  // Refresh only the queried PID with the minimal refresh kind: this is a
  // pure existence check, and callers (worker supervisors every 15s, GUI
  // cleanup loops) must not pay for a full system process-table scan.
  let mut system = System::new();
  system.refresh_processes_specifics(
    ProcessesToUpdate::Some(&[pid]),
    true,
    ProcessRefreshKind::nothing(),
  );
  system.process(pid).is_some()
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
    // On Windows a terminated process remains a live kernel object (and sysinfo
    // keeps reporting it) until the LAST handle to it is closed. std::Child
    // holds that handle until dropped, so the check below would otherwise see
    // the just-exited process as still running. Drop the handle, then allow a
    // brief moment for the OS to reclaim the process before asserting. (In
    // production these PIDs belong to detached browsers/workers that no handle
    // outlives, so is_process_running already observes their exit promptly.)
    drop(child);

    let mut became_dead = false;
    for _ in 0..50 {
      if !is_process_running(pid) {
        became_dead = true;
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
      became_dead,
      "is_process_running must return false for a dead process (PID {pid})"
    );
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

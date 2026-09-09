use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const UNSTARTED_WORKER_GRACE_SECS: u64 = 60;
const TRANSIENT_IO_RETRY_ATTEMPTS: u32 = 25;
const TRANSIENT_IO_RETRY_DELAY: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrayWorkerConfig {
  pub id: String,
  pub profile_id: Option<String>,
  pub vless_uri: String,
  pub local_port: u16,
  pub username: String,
  pub password: String,
  #[serde(default)]
  pub created_at: u64,
  pub pid: Option<u32>,
  #[serde(default)]
  pub pid_start_time: Option<u64>,
  pub xray_pid: Option<u32>,
  #[serde(default)]
  pub xray_pid_start_time: Option<u64>,
  #[serde(default)]
  pub ready: bool,
  #[serde(default)]
  pub browser_pid: Option<u32>,
  #[serde(default)]
  pub browser_pid_start_time: Option<u64>,
}

impl XrayWorkerConfig {
  pub fn new(
    id: String,
    profile_id: Option<String>,
    vless_uri: String,
    local_port: u16,
    username: String,
    password: String,
  ) -> Self {
    Self {
      id,
      profile_id,
      vless_uri,
      local_port,
      username,
      password,
      created_at: now_secs(),
      pid: None,
      pid_start_time: None,
      xray_pid: None,
      xray_pid_start_time: None,
      ready: false,
      browser_pid: None,
      browser_pid_start_time: None,
    }
  }

  pub fn local_proxy_settings(&self) -> crate::browser::ProxySettings {
    crate::browser::ProxySettings {
      proxy_type: "socks5".to_string(),
      host: "127.0.0.1".to_string(),
      port: self.local_port,
      username: Some(self.username.clone()),
      password: Some(self.password.clone()),
      vless_uri: None,
    }
  }
}

fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

pub fn unstarted_worker_is_stale(config: &XrayWorkerConfig) -> bool {
  config.pid.is_none()
    && (config.created_at == 0
      || now_secs().saturating_sub(config.created_at) > UNSTARTED_WORKER_GRACE_SECS)
}

fn ensure_private_storage_dir() -> std::io::Result<PathBuf> {
  let directory = crate::proxy_storage::get_storage_dir();
  fs::create_dir_all(&directory)?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
  }
  Ok(directory)
}

fn atomic_write_owner_only(path: &Path, content: &[u8]) -> std::io::Result<()> {
  let parent = path
    .parent()
    .ok_or_else(|| std::io::Error::other("worker path has no parent"))?;
  fs::create_dir_all(parent)?;
  let mut temporary = tempfile::Builder::new()
    .prefix(".xray-state-")
    .tempfile_in(parent)?;
  crate::app_dirs::restrict_to_owner(temporary.path());
  temporary.write_all(content)?;
  temporary.flush()?;
  temporary.as_file().sync_all()?;

  let mut attempt = 0;
  loop {
    match temporary.persist(path) {
      Ok(_) => break,
      Err(error)
        if attempt < TRANSIENT_IO_RETRY_ATTEMPTS && io_error_is_transient(&error.error) =>
      {
        temporary = error.file;
        attempt += 1;
        std::thread::sleep(TRANSIENT_IO_RETRY_DELAY);
      }
      Err(error) => return Err(error.error),
    }
  }

  crate::app_dirs::restrict_to_owner(path);
  Ok(())
}

/// Windows refuses to replace or open a file another handle holds without
/// `FILE_SHARE_DELETE`, and virus scanners open files exactly that way. The
/// supervisor rewrites worker state while the GUI polls it, so both the rename
/// and the read fail spuriously under that race. Those are worth retrying;
/// every other error is real and must surface.
#[cfg(windows)]
fn io_error_is_transient(error: &std::io::Error) -> bool {
  const ERROR_ACCESS_DENIED: i32 = 5;
  const ERROR_SHARING_VIOLATION: i32 = 32;
  const ERROR_LOCK_VIOLATION: i32 = 33;
  matches!(
    error.raw_os_error(),
    Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
  )
}

#[cfg(not(windows))]
fn io_error_is_transient(_error: &std::io::Error) -> bool {
  false
}

fn read_worker_state(path: &Path) -> Option<Vec<u8>> {
  let mut attempt = 0;
  loop {
    match fs::read(path) {
      Ok(content) => return Some(content),
      Err(error) if attempt < TRANSIENT_IO_RETRY_ATTEMPTS && io_error_is_transient(&error) => {
        attempt += 1;
        std::thread::sleep(TRANSIENT_IO_RETRY_DELAY);
      }
      Err(_) => return None,
    }
  }
}

pub fn xray_worker_config_path(id: &str) -> PathBuf {
  crate::proxy_storage::get_storage_dir().join(format!("xray_worker_{id}.json"))
}

pub fn xray_runtime_config_path(id: &str) -> PathBuf {
  crate::proxy_storage::get_storage_dir().join(format!("xray_runtime_{id}.json"))
}

pub fn xray_worker_log_path(id: &str) -> PathBuf {
  crate::proxy_storage::get_storage_dir().join(format!("xray_worker_{id}.log"))
}

fn xray_worker_tombstone_path(id: &str) -> PathBuf {
  crate::proxy_storage::get_storage_dir().join(format!("xray_worker_{id}.stopped"))
}

fn worker_is_tombstoned(id: &str) -> bool {
  xray_worker_tombstone_path(id).exists()
}

/// How long a tombstone has to outlive its worker.
///
/// It only has to survive long enough to beat a write already in flight from
/// the process that owned that id. A day is many orders of magnitude more than
/// that, and bounds a directory that otherwise gains a file per worker forever.
const TOMBSTONE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Drop tombstones old enough that nothing could still be racing them.
fn prune_stale_tombstones() {
  let Ok(entries) = fs::read_dir(crate::proxy_storage::get_storage_dir()) else {
    return;
  };
  for entry in entries.flatten() {
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("stopped") {
      continue;
    }
    let aged_out = path
      .metadata()
      .and_then(|meta| meta.modified())
      .map(|modified| {
        modified
          .elapsed()
          .map(|age| age > TOMBSTONE_TTL)
          .unwrap_or(false)
      })
      .unwrap_or(false);
    if aged_out {
      let _ = fs::remove_file(&path);
    }
  }
}

pub fn create_xray_worker_log(id: &str) -> std::io::Result<std::fs::File> {
  ensure_private_storage_dir()?;
  if worker_is_tombstoned(id) {
    return Err(std::io::Error::new(
      std::io::ErrorKind::NotFound,
      "Xray worker has stopped",
    ));
  }
  let path = xray_worker_log_path(id);
  let file = crate::app_dirs::create_owner_only(&path)?;
  if worker_is_tombstoned(id) {
    drop(file);
    let _ = fs::remove_file(path);
    return Err(std::io::Error::new(
      std::io::ErrorKind::NotFound,
      "Xray worker has stopped",
    ));
  }
  Ok(file)
}

pub fn write_xray_runtime_config(id: &str, content: &[u8]) -> std::io::Result<()> {
  ensure_private_storage_dir()?;
  if worker_is_tombstoned(id) {
    return Err(std::io::Error::new(
      std::io::ErrorKind::NotFound,
      "Xray worker has stopped",
    ));
  }
  let path = xray_runtime_config_path(id);
  atomic_write_owner_only(&path, content)?;
  if worker_is_tombstoned(id) {
    let _ = fs::remove_file(path);
    return Err(std::io::Error::new(
      std::io::ErrorKind::NotFound,
      "Xray worker has stopped",
    ));
  }
  Ok(())
}

pub fn save_xray_worker_config(
  config: &XrayWorkerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
  save_xray_worker_config_to_path(config, &xray_worker_config_path(&config.id))
}

pub fn save_xray_worker_config_to_path(
  config: &XrayWorkerConfig,
  path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
  ensure_private_storage_dir()?;
  if worker_is_tombstoned(&config.id) {
    return Err(
      std::io::Error::new(std::io::ErrorKind::NotFound, "Xray worker has stopped").into(),
    );
  }
  let content = serde_json::to_vec_pretty(config)?;
  atomic_write_owner_only(path, &content)?;
  if worker_is_tombstoned(&config.id) {
    let _ = fs::remove_file(path);
    return Err(
      std::io::Error::new(std::io::ErrorKind::NotFound, "Xray worker has stopped").into(),
    );
  }
  Ok(())
}

pub fn get_xray_worker_config(id: &str) -> Option<XrayWorkerConfig> {
  get_xray_worker_config_from_path(&xray_worker_config_path(id))
}

pub fn get_xray_worker_config_from_path(path: &Path) -> Option<XrayWorkerConfig> {
  serde_json::from_slice(&read_worker_state(path)?).ok()
}

pub fn update_xray_worker_config(config: &XrayWorkerConfig) -> bool {
  let path = xray_worker_config_path(&config.id);
  path.exists() && save_xray_worker_config_to_path(config, &path).is_ok()
}

pub fn delete_xray_worker_config(id: &str) -> bool {
  if ensure_private_storage_dir().is_ok() {
    let _ = atomic_write_owner_only(&xray_worker_tombstone_path(id), b"");
  }
  let path = xray_worker_config_path(id);
  let deleted = !path.exists() || fs::remove_file(path).is_ok();
  let _ = fs::remove_file(xray_runtime_config_path(id));
  let _ = fs::remove_file(xray_worker_log_path(id));
  deleted
}

pub fn list_xray_worker_configs() -> Vec<XrayWorkerConfig> {
  // Cheap, and this is the one call every sweep already makes.
  prune_stale_tombstones();
  let storage_dir = crate::proxy_storage::get_storage_dir();
  let Ok(entries) = fs::read_dir(storage_dir) else {
    return Vec::new();
  };

  entries
    .flatten()
    .filter_map(|entry| {
      let path = entry.path();
      let name = path.file_name()?.to_str()?;
      if !name.starts_with("xray_worker_") || !name.ends_with(".json") {
        return None;
      }
      get_xray_worker_config_from_path(&path)
    })
    .collect()
}

pub fn find_xray_worker_by_profile_id(profile_id: &str) -> Option<XrayWorkerConfig> {
  list_xray_worker_configs()
    .into_iter()
    .filter(|config| config.profile_id.as_deref() == Some(profile_id))
    .max_by_key(|config| config.created_at)
}

pub fn generate_xray_worker_id() -> String {
  format!(
    "xrayw_{}_{}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs(),
    rand::random::<u32>()
  )
}

#[cfg(test)]
#[path = "xray_worker_storage_tests.rs"]
mod tests;

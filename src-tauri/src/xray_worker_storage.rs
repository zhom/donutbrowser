use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const UNSTARTED_WORKER_GRACE_SECS: u64 = 60;

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
  temporary.persist(path).map_err(|error| error.error)?;
  crate::app_dirs::restrict_to_owner(path);
  Ok(())
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
  let content = fs::read(path).ok()?;
  serde_json::from_slice(&content).ok()
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
mod tests {
  use super::*;

  fn test_config(id: &str) -> XrayWorkerConfig {
    XrayWorkerConfig::new(
      id.to_string(),
      Some("profile".to_string()),
      "vless://example".to_string(),
      1080,
      "local-user".to_string(),
      "local-password".to_string(),
    )
  }

  #[test]
  fn local_proxy_settings_use_authenticated_loopback_socks() {
    let config = test_config("id");

    let proxy = config.local_proxy_settings();
    assert_eq!(proxy.proxy_type, "socks5");
    assert_eq!(proxy.host, "127.0.0.1");
    assert_eq!(proxy.port, 1080);
    assert_eq!(proxy.username.as_deref(), Some("local-user"));
    assert_eq!(proxy.password.as_deref(), Some("local-password"));
    assert!(proxy.vless_uri.is_none());
  }

  #[test]
  fn worker_storage_round_trips_updates_lists_and_securely_cleans_runtime_files() {
    let temp = tempfile::tempdir().unwrap();
    let _cache_guard = crate::app_dirs::set_test_cache_dir(temp.path().to_path_buf());
    let id = format!("xray-storage-test-{}", uuid::Uuid::new_v4());
    let mut config = test_config(&id);

    save_xray_worker_config(&config).unwrap();
    assert_eq!(get_xray_worker_config(&id).unwrap().username, "local-user");
    assert_eq!(
      find_xray_worker_by_profile_id("profile").unwrap().id,
      config.id
    );
    assert!(list_xray_worker_configs()
      .iter()
      .any(|candidate| candidate.id == id));

    config.pid = Some(41);
    config.xray_pid = Some(42);
    config.browser_pid = Some(43);
    assert!(update_xray_worker_config(&config));
    let updated = get_xray_worker_config(&id).unwrap();
    assert_eq!(updated.pid, Some(41));
    assert_eq!(updated.xray_pid, Some(42));
    assert_eq!(updated.browser_pid, Some(43));

    let runtime_path = xray_runtime_config_path(&id);
    write_xray_runtime_config(&id, b"{\"runtime\":true}").unwrap();
    let log_path = xray_worker_log_path(&id);
    drop(create_xray_worker_log(&id).unwrap());

    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(
        std::fs::metadata(crate::proxy_storage::get_storage_dir())
          .unwrap()
          .permissions()
          .mode()
          & 0o777,
        0o700
      );
      assert_eq!(
        std::fs::metadata(xray_worker_config_path(&id))
          .unwrap()
          .permissions()
          .mode()
          & 0o777,
        0o600
      );
      assert_eq!(
        std::fs::metadata(&runtime_path)
          .unwrap()
          .permissions()
          .mode()
          & 0o777,
        0o600
      );
      assert_eq!(
        std::fs::metadata(&log_path).unwrap().permissions().mode() & 0o777,
        0o600
      );
    }

    assert!(delete_xray_worker_config(&id));
    assert!(get_xray_worker_config(&id).is_none());
    assert!(!runtime_path.exists());
    assert!(!log_path.exists());
    assert!(!update_xray_worker_config(&config));
    assert!(write_xray_runtime_config(&id, b"{}").is_err());
    assert!(create_xray_worker_log(&id).is_err());
  }

  #[test]
  fn fresh_unstarted_workers_have_a_grace_period_but_legacy_entries_are_stale() {
    let fresh = test_config("fresh");
    assert!(!unstarted_worker_is_stale(&fresh));

    let mut legacy = test_config("legacy");
    legacy.created_at = 0;
    assert!(unstarted_worker_is_stale(&legacy));

    legacy.pid = Some(1);
    assert!(!unstarted_worker_is_stale(&legacy));
  }

  #[test]
  fn atomic_state_updates_never_expose_partial_json() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.json");
    atomic_write_owner_only(&path, br#"{"value":0}"#).unwrap();
    let complete = Arc::new(AtomicBool::new(false));
    let writer_complete = complete.clone();
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
      for value in 1..=500 {
        let content = serde_json::to_vec(&serde_json::json!({ "value": value })).unwrap();
        atomic_write_owner_only(&writer_path, &content).unwrap();
      }
      writer_complete.store(true, Ordering::Release);
    });

    while !complete.load(Ordering::Acquire) {
      let content = std::fs::read(&path).unwrap();
      let value: serde_json::Value = serde_json::from_slice(&content).unwrap();
      assert!(value["value"].is_number());
    }
    writer.join().unwrap();
  }

  #[cfg(unix)]
  #[test]
  fn atomic_state_write_replaces_a_symlink_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let victim = temp.path().join("victim");
    let state = temp.path().join("state.json");
    std::fs::write(&victim, "untouched").unwrap();
    symlink(&victim, &state).unwrap();

    atomic_write_owner_only(&state, br#"{"safe":true}"#).unwrap();

    assert_eq!(std::fs::read_to_string(victim).unwrap(), "untouched");
    assert_eq!(
      serde_json::from_slice::<serde_json::Value>(&std::fs::read(state).unwrap()).unwrap()["safe"],
      true
    );
  }

  #[test]
  fn legacy_worker_config_defaults_missing_browser_pid() {
    let value = serde_json::json!({
      "id": "legacy",
      "profile_id": "profile",
      "vless_uri": "vless://example",
      "local_port": 1080,
      "username": "user",
      "password": "password",
      "pid": 1,
      "xray_pid": 2
    });
    let config: XrayWorkerConfig = serde_json::from_value(value).unwrap();
    assert_eq!(config.created_at, 0);
    assert_eq!(config.pid_start_time, None);
    assert_eq!(config.xray_pid_start_time, None);
    assert!(!config.ready);
    assert_eq!(config.browser_pid, None);
    assert_eq!(config.browser_pid_start_time, None);
  }
}

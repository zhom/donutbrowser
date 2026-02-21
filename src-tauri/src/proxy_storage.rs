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
    }
  }

  pub fn with_profile_id(mut self, profile_id: Option<String>) -> Self {
    self.profile_id = profile_id;
    self
  }
}

pub fn get_storage_dir() -> PathBuf {
  crate::app_dirs::proxies_dir()
}

pub fn save_proxy_config(config: &ProxyConfig) -> Result<(), Box<dyn std::error::Error>> {
  let storage_dir = get_storage_dir();
  fs::create_dir_all(&storage_dir)?;

  let file_path = storage_dir.join(format!("{}.json", config.id));
  let content = serde_json::to_string_pretty(config)?;
  fs::write(&file_path, content)?;

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

  match serde_json::to_string_pretty(config) {
    Ok(content) => fs::write(&file_path, content).is_ok(),
    Err(_) => false,
  }
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
  use sysinfo::{Pid, System};
  let system = System::new();
  system.process(Pid::from(pid as usize)).is_some()
}

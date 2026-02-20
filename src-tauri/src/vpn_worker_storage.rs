use crate::proxy_storage::get_storage_dir;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnWorkerConfig {
  pub id: String,
  pub vpn_id: String,
  pub vpn_type: String,
  pub config_file_path: String,
  pub local_port: Option<u16>,
  pub local_url: Option<String>,
  pub pid: Option<u32>,
}

impl VpnWorkerConfig {
  pub fn new(id: String, vpn_id: String, vpn_type: String, config_file_path: String) -> Self {
    Self {
      id,
      vpn_id,
      vpn_type,
      config_file_path,
      local_port: None,
      local_url: None,
      pid: None,
    }
  }
}

pub fn save_vpn_worker_config(config: &VpnWorkerConfig) -> Result<(), Box<dyn std::error::Error>> {
  let storage_dir = get_storage_dir();
  fs::create_dir_all(&storage_dir)?;

  let file_path = storage_dir.join(format!("vpn_worker_{}.json", config.id));
  let content = serde_json::to_string_pretty(config)?;
  fs::write(&file_path, content)?;

  Ok(())
}

pub fn get_vpn_worker_config(id: &str) -> Option<VpnWorkerConfig> {
  let storage_dir = get_storage_dir();
  let file_path = storage_dir.join(format!("vpn_worker_{}.json", id));

  if !file_path.exists() {
    return None;
  }

  match fs::read_to_string(&file_path) {
    Ok(content) => serde_json::from_str(&content).ok(),
    Err(_) => None,
  }
}

pub fn delete_vpn_worker_config(id: &str) -> bool {
  let storage_dir = get_storage_dir();
  let file_path = storage_dir.join(format!("vpn_worker_{}.json", id));

  if !file_path.exists() {
    return false;
  }

  fs::remove_file(&file_path).is_ok()
}

pub fn list_vpn_worker_configs() -> Vec<VpnWorkerConfig> {
  let storage_dir = get_storage_dir();

  if !storage_dir.exists() {
    return Vec::new();
  }

  let mut configs = Vec::new();
  if let Ok(entries) = fs::read_dir(&storage_dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with("vpn_worker_") && name.ends_with(".json") {
          if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<VpnWorkerConfig>(&content) {
              configs.push(config);
            }
          }
        }
      }
    }
  }

  configs
}

pub fn find_vpn_worker_by_vpn_id(vpn_id: &str) -> Option<VpnWorkerConfig> {
  list_vpn_worker_configs()
    .into_iter()
    .find(|c| c.vpn_id == vpn_id)
}

pub fn generate_vpn_worker_id() -> String {
  format!(
    "vpnw_{}_{}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs(),
    rand::random::<u32>()
  )
}

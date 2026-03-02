use crate::camoufox_manager::CamoufoxConfig;
use crate::wayfern_manager::WayfernConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum SyncStatus {
  #[default]
  Disabled,
  Syncing,
  Synced,
  Error,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncMode {
  #[default]
  Disabled,
  Regular,
  Encrypted,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrowserProfile {
  pub id: uuid::Uuid,
  pub name: String,
  pub browser: String,
  pub version: String,
  #[serde(default)]
  pub proxy_id: Option<String>, // Reference to stored proxy
  #[serde(default)]
  pub vpn_id: Option<String>, // Reference to stored VPN config
  #[serde(default)]
  pub process_id: Option<u32>,
  #[serde(default)]
  pub last_launch: Option<u64>,
  #[serde(default = "default_release_type")]
  pub release_type: String, // "stable" or "nightly"
  #[serde(default)]
  pub camoufox_config: Option<CamoufoxConfig>, // Camoufox configuration
  #[serde(default)]
  pub wayfern_config: Option<WayfernConfig>, // Wayfern configuration
  #[serde(default)]
  pub group_id: Option<String>, // Reference to profile group
  #[serde(default)]
  pub tags: Vec<String>, // Free-form tags
  #[serde(default)]
  pub note: Option<String>, // User note
  #[serde(default)]
  pub sync_mode: SyncMode,
  #[serde(default)]
  pub encryption_salt: Option<String>,
  #[serde(default)]
  pub last_sync: Option<u64>, // Timestamp of last successful sync (epoch seconds)
  #[serde(default)]
  pub host_os: Option<String>, // OS where profile was created ("macos", "windows", "linux")
  #[serde(default)]
  pub ephemeral: bool,
  #[serde(default)]
  pub extension_group_id: Option<String>,
  #[serde(default)]
  pub proxy_bypass_rules: Vec<String>,
}

pub fn default_release_type() -> String {
  "stable".to_string()
}

pub fn get_host_os() -> String {
  if cfg!(target_os = "macos") {
    "macos".to_string()
  } else if cfg!(target_os = "windows") {
    "windows".to_string()
  } else {
    "linux".to_string()
  }
}

impl BrowserProfile {
  /// Get the path to the profile data directory (profiles/{uuid}/profile)
  pub fn get_profile_data_path(&self, profiles_dir: &Path) -> PathBuf {
    profiles_dir.join(self.id.to_string()).join("profile")
  }

  /// Returns true when the profile was created on a different OS than the current host.
  /// Profiles without an `os` field (backward compat) are treated as native.
  pub fn is_cross_os(&self) -> bool {
    match &self.host_os {
      Some(host_os) => host_os != &get_host_os(),
      None => false,
    }
  }

  /// Returns true if sync is enabled (either Regular or Encrypted mode).
  pub fn is_sync_enabled(&self) -> bool {
    self.sync_mode != SyncMode::Disabled
  }

  /// Returns true if sync uses E2E encryption.
  pub fn is_encrypted_sync(&self) -> bool {
    self.sync_mode == SyncMode::Encrypted
  }
}

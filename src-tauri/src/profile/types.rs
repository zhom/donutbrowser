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

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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
  pub launch_hook: Option<String>,
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
  #[serde(default)]
  pub created_by_id: Option<String>,
  #[serde(default)]
  pub created_by_email: Option<String>,
  #[serde(default)]
  pub dns_blocklist: Option<String>,
  /// True when the on-disk profile dir is encrypted with a per-profile password.
  /// Decryption goes to a RAM-backed ephemeral dir, never to disk.
  #[serde(default)]
  pub password_protected: bool,
  /// Profile creation timestamp (epoch seconds, UTC). `None` for legacy
  /// profiles that pre-date this field — those are treated as ancient by
  /// any staleness check.
  #[serde(default)]
  pub created_at: Option<u64>,
  /// Unix seconds of the last meaningful metadata edit (name, tags, note,
  /// proxy/vpn/group/extension assignment, launch hook, bypass rules, dns).
  /// Source of truth for metadata sync conflict resolution (last-write-wins);
  /// NOT bumped by browser-file changes, which sync via the file manifest.
  #[serde(default)]
  pub updated_at: Option<u64>,
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

  /// Resolve the OS this profile was created on. Checks `host_os` first,
  /// then falls back to the fingerprint config's `os` field (for profiles
  /// created before `host_os` was introduced or synced without it).
  pub fn resolved_os(&self) -> Option<&str> {
    self
      .host_os
      .as_deref()
      .or_else(|| self.camoufox_config.as_ref().and_then(|c| c.os.as_deref()))
      .or_else(|| self.wayfern_config.as_ref().and_then(|c| c.os.as_deref()))
  }

  /// Returns true when the profile was created on a different OS than the current host.
  /// Checks `host_os` first, then falls back to the browser config's `os` field.
  pub fn is_cross_os(&self) -> bool {
    match self.resolved_os() {
      Some(os) => os != get_host_os(),
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

//! Profile templates: saved settings that new profiles are made from, one at a
//! time or many at once. A template never holds a fingerprint: every profile
//! made from it gets its own device.

use crate::sync::config_entities::{ConfigEntity, ConfigKind, ConfigStore};
use crate::wayfern_manager::WayfernConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TemplateSettings {
  /// What the profiles made from the template are named after.
  #[serde(default)]
  pub name_prefix: Option<String>,
  #[serde(default)]
  pub proxy_id: Option<String>,
  #[serde(default)]
  pub vpn_id: Option<String>,
  /// Proxies to spread new profiles across, one each in turn. Wins over
  /// `proxy_id` when not empty.
  #[serde(default)]
  pub proxy_ids: Vec<String>,
  #[serde(default)]
  pub group_id: Option<String>,
  #[serde(default)]
  pub tags: Vec<String>,
  #[serde(default)]
  pub status_id: Option<String>,
  #[serde(default)]
  pub note: Option<String>,
  #[serde(default)]
  pub extension_group_id: Option<String>,
  #[serde(default)]
  pub dns_blocklist: Option<String>,
  #[serde(default)]
  pub dns_server: Option<String>,
  #[serde(default)]
  pub proxy_bypass_rules: Vec<String>,
  #[serde(default)]
  pub start_urls: Vec<String>,
  #[serde(default)]
  pub launch_hook: Option<String>,
  #[serde(default)]
  pub ephemeral: bool,
  #[serde(default)]
  pub clear_on_close: bool,
  /// Fingerprint settings (OS, screen bounds, WebRTC, images, WebGL,
  /// session restore, launch arguments). The device fields are always empty.
  #[serde(default)]
  pub wayfern_config: Option<WayfernConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileTemplate {
  pub id: String,
  pub name: String,
  #[serde(default)]
  pub settings: TemplateSettings,
  #[serde(default)]
  pub position: u64,
  #[serde(default)]
  pub updated_at: u64,
}

static STORE: ConfigStore<ProfileTemplate> = ConfigStore::new("profile_templates.json");

impl ConfigEntity for ProfileTemplate {
  const KIND: ConfigKind = ConfigKind::Template;
  fn id(&self) -> &str {
    &self.id
  }
  fn updated_at(&self) -> u64 {
    self.updated_at
  }
  fn store() -> &'static ConfigStore<Self> {
    &STORE
  }
}

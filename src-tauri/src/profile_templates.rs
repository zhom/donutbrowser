//! Profile templates: saved settings that new profiles are made from, one at a
//! time or many at once. A template never holds a device: every profile made
//! from it gets its own.

use crate::profile::patch::ProfilePatch;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::sync::config_entities::{self, ConfigEntity, ConfigKind, ConfigStore};
use crate::wayfern_manager::WayfernConfig;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_TEMPLATES: usize = 100;
const MAX_NAME_CHARS: usize = 60;
/// The most profiles one bulk creation makes.
pub const MAX_BULK_CREATE: u32 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, Default, utoipa::ToSchema)]
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
  #[schema(value_type = Option<Object>)]
  pub wayfern_config: Option<WayfernConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
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

fn code(code: &str) -> String {
  crate::backend_error(code)
}

fn trimmed(value: Option<String>) -> Option<String> {
  value
    .map(|v| v.trim().to_string())
    .filter(|v| !v.is_empty())
}

fn checked_name(name: &str) -> Result<String, String> {
  let name = name.trim();
  if name.is_empty() {
    return Err(code("NAME_CANNOT_BE_EMPTY"));
  }
  if name.chars().count() > MAX_NAME_CHARS {
    return Err(code("TEMPLATE_NAME_TOO_LONG"));
  }
  Ok(name.to_string())
}

/// What describes a kind of device, never one device: the fields that
/// identify a device, and a camera file that only exists on this computer,
/// stay out of a template.
fn portable(mut config: WayfernConfig) -> WayfernConfig {
  config.fingerprint = None;
  config.identity_id = None;
  config.identity_baseline = None;
  config.identity_overrides = None;
  config.persona = None;
  config.location = None;
  config.geo_proxy_signature = None;
  config.proxy = None;
  config.camera_file = None;
  config.camera_crop = None;
  config
}

impl TemplateSettings {
  /// Every value checked and in its stored form.
  pub async fn checked(self) -> Result<Self, String> {
    let name_prefix = trimmed(self.name_prefix);
    if name_prefix
      .as_ref()
      .is_some_and(|prefix| prefix.chars().count() > MAX_NAME_CHARS)
    {
      return Err(code("TEMPLATE_NAME_TOO_LONG"));
    }
    let proxy_id = trimmed(self.proxy_id);
    let vpn_id = trimmed(self.vpn_id);
    let mut proxy_ids: Vec<String> = Vec::new();
    for id in self.proxy_ids {
      let id = id.trim().to_string();
      if !id.is_empty() && !proxy_ids.contains(&id) {
        proxy_ids.push(id);
      }
    }
    if vpn_id.is_some() && (proxy_id.is_some() || !proxy_ids.is_empty()) {
      return Err(code("PROXY_AND_VPN_MUTUALLY_EXCLUSIVE"));
    }

    let checked = ProfilePatch {
      status_id: Some(self.status_id),
      tags_add: self.tags,
      note: Some(self.note),
      group_id: Some(self.group_id),
      extension_group_id: Some(self.extension_group_id),
      dns_blocklist: Some(self.dns_blocklist),
      dns_server: Some(self.dns_server),
      proxy_bypass_rules: Some(self.proxy_bypass_rules),
      start_urls: Some(self.start_urls),
      launch_hook: Some(self.launch_hook),
      ..Default::default()
    }
    .validated()?;

    let mut wayfern_config = self.wayfern_config.map(portable);
    if let Some(config) = wayfern_config.as_mut() {
      config.launch_args = config.checked_launch_args().await?;
    }

    Ok(Self {
      name_prefix,
      proxy_id,
      vpn_id,
      proxy_ids,
      group_id: checked.group_id.flatten(),
      tags: checked.tags_add,
      status_id: checked.status_id.flatten(),
      note: trimmed(checked.note.flatten()),
      extension_group_id: checked.extension_group_id.flatten(),
      dns_blocklist: checked.dns_blocklist.flatten(),
      dns_server: checked.dns_server.flatten(),
      proxy_bypass_rules: checked.proxy_bypass_rules.unwrap_or_default(),
      start_urls: checked.start_urls.unwrap_or_default(),
      launch_hook: checked.launch_hook.flatten(),
      ephemeral: self.ephemeral,
      // An ephemeral profile keeps nothing, so there is nothing to clear.
      clear_on_close: self.clear_on_close && !self.ephemeral,
      wayfern_config,
    })
  }

  /// The settings profile creation takes no value for, set right after it.
  fn after_creation(&self) -> ProfilePatch {
    ProfilePatch {
      status_id: self.status_id.clone().map(Some),
      tags_add: self.tags.clone(),
      note: self.note.clone().map(Some),
      extension_group_id: self.extension_group_id.clone().map(Some),
      proxy_bypass_rules: (!self.proxy_bypass_rules.is_empty())
        .then(|| self.proxy_bypass_rules.clone()),
      start_urls: (!self.start_urls.is_empty()).then(|| self.start_urls.clone()),
      clear_on_close: self.clear_on_close.then_some(true),
      ..Default::default()
    }
  }

  /// A profile's settings, without its name, its note or its device: those
  /// belong to that one profile.
  fn from_profile(profile: &BrowserProfile) -> Self {
    Self {
      name_prefix: None,
      proxy_id: profile.proxy_id.clone(),
      vpn_id: profile.vpn_id.clone(),
      proxy_ids: Vec::new(),
      group_id: profile.group_id.clone(),
      tags: profile.tags.clone(),
      status_id: profile.status_id.clone(),
      note: None,
      extension_group_id: profile.extension_group_id.clone(),
      dns_blocklist: profile.dns_blocklist.clone(),
      dns_server: profile.dns_server.clone(),
      proxy_bypass_rules: profile.proxy_bypass_rules.clone(),
      start_urls: profile.start_urls.clone(),
      launch_hook: profile.launch_hook.clone(),
      ephemeral: profile.ephemeral,
      clear_on_close: profile.clear_on_close,
      wayfern_config: profile.wayfern_config.clone().map(portable),
    }
  }
}

pub fn list() -> Vec<ProfileTemplate> {
  let mut templates = STORE.list();
  templates.sort_by(|a, b| {
    a.position
      .cmp(&b.position)
      .then_with(|| a.name.cmp(&b.name))
  });
  templates
}

pub fn get(template_id: &str) -> Result<ProfileTemplate, String> {
  STORE
    .get(template_id)
    .ok_or_else(|| code("TEMPLATE_NOT_FOUND"))
}

pub async fn create(name: &str, settings: TemplateSettings) -> Result<ProfileTemplate, String> {
  let name = checked_name(name)?;
  let settings = settings.checked().await?;
  let template = STORE.mutate(|templates| {
    if templates.len() >= MAX_TEMPLATES {
      return Err(code("TEMPLATE_LIMIT_REACHED"));
    }
    if templates.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) {
      return Err(code("TEMPLATE_EXISTS"));
    }
    let template = ProfileTemplate {
      id: uuid::Uuid::new_v4().to_string(),
      name,
      settings,
      position: templates.iter().map(|t| t.position + 1).max().unwrap_or(0),
      updated_at: crate::proxy_manager::now_secs(),
    };
    templates.push(template.clone());
    Ok(template)
  })?;
  config_entities::changed_locally::<ProfileTemplate>(&template.id);
  crate::team_activity::record("template.create", &template.id, Some(&template.name), None);
  log::info!("Template created template={}", template.id);
  Ok(template)
}

pub async fn update(
  template_id: &str,
  name: &str,
  settings: TemplateSettings,
) -> Result<ProfileTemplate, String> {
  let name = checked_name(name)?;
  let settings = settings.checked().await?;
  let template = STORE.mutate(|templates| {
    if templates
      .iter()
      .any(|t| t.id != template_id && t.name.eq_ignore_ascii_case(&name))
    {
      return Err(code("TEMPLATE_EXISTS"));
    }
    let template = templates
      .iter_mut()
      .find(|t| t.id == template_id)
      .ok_or_else(|| code("TEMPLATE_NOT_FOUND"))?;
    template.name = name;
    template.settings = settings;
    template.updated_at = crate::proxy_manager::now_secs();
    Ok(template.clone())
  })?;
  config_entities::changed_locally::<ProfileTemplate>(&template.id);
  crate::team_activity::record("template.update", &template.id, Some(&template.name), None);
  Ok(template)
}

pub fn delete(template_id: &str) -> Result<(), String> {
  let removed = STORE.mutate(|templates| {
    let before = templates.len();
    templates.retain(|t| t.id != template_id);
    Ok(templates.len() != before)
  })?;
  if !removed {
    return Err(code("TEMPLATE_NOT_FOUND"));
  }
  config_entities::deleted_locally::<ProfileTemplate>(template_id);
  crate::team_activity::record("template.delete", template_id, None, None);
  log::info!("Template deleted template={template_id}");
  Ok(())
}

pub async fn save_profile_as_template(
  profile_id: &str,
  name: &str,
) -> Result<ProfileTemplate, String> {
  let profile = ProfileManager::instance()
    .list_profiles()
    .map_err(|e| crate::wrap_backend_error(e, "Failed to list profiles"))?
    .into_iter()
    .find(|profile| profile.id.to_string() == profile_id)
    .ok_or_else(|| code("PROFILE_NOT_FOUND"))?;
  create(name, TemplateSettings::from_profile(&profile)).await
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct BulkCreateResult {
  /// The profiles made, in order.
  #[schema(value_type = Vec<Object>)]
  pub created: Vec<BrowserProfile>,
  /// Why creation stopped before the count was reached: an error code JSON,
  /// as the single-profile errors. `None` when it was not stopped by an error.
  pub error: Option<String>,
  /// The person stopped it before the count was reached.
  pub cancelled: bool,
}

#[derive(Clone, Serialize)]
struct BulkCreateProgress {
  done: u32,
  total: u32,
}

static BULK_RUNNING: AtomicBool = AtomicBool::new(false);
static BULK_CANCEL: AtomicBool = AtomicBool::new(false);

struct BulkRun;

impl Drop for BulkRun {
  fn drop(&mut self) {
    BULK_RUNNING.store(false, Ordering::SeqCst);
  }
}

/// Names for `count` new profiles: the name as it is for one, else the name
/// with numbers that continue after the highest one already in use, so a
/// second batch never collides with the first.
fn names_for(base: &str, count: u32, existing: &[String]) -> Vec<String> {
  if count == 1 {
    return vec![base.to_string()];
  }
  let prefix = format!("{} ", base.to_lowercase());
  let highest = existing
    .iter()
    .filter_map(|name| {
      let lower = name.to_lowercase();
      lower.strip_prefix(prefix.as_str())?.parse::<u64>().ok()
    })
    .max()
    .unwrap_or(0);
  (1..=u64::from(count))
    .map(|n| format!("{base} {}", highest + n))
    .collect()
}

fn latest_wayfern_version() -> Option<String> {
  let mut versions = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance()
    .get_downloaded_versions("wayfern");
  versions.sort_by(|a, b| crate::api_client::compare_versions(b, a));
  versions.into_iter().next()
}

/// Make `count` profiles with these settings. Everything is checked first,
/// each proxy once; then the profiles are made one by one until the count is
/// reached, an error stops the run, or the person cancels it. What was made
/// before a stop stays.
pub async fn create_profiles(
  app_handle: &tauri::AppHandle,
  settings: TemplateSettings,
  name: &str,
  count: u32,
) -> Result<BulkCreateResult, String> {
  if !(1..=MAX_BULK_CREATE).contains(&count) {
    return Err(
      serde_json::json!({
        "code": "BULK_CREATE_COUNT_INVALID",
        "params": { "max": MAX_BULK_CREATE.to_string() }
      })
      .to_string(),
    );
  }
  let base = name.trim();
  if base.is_empty() {
    return Err(code("NAME_CANNOT_BE_EMPTY"));
  }
  if BULK_RUNNING.swap(true, Ordering::SeqCst) {
    return Err(code("BULK_CREATE_BUSY"));
  }
  let _run = BulkRun;
  BULK_CANCEL.store(false, Ordering::SeqCst);

  let settings = settings.checked().await?;
  let after = settings.after_creation().validated()?;
  let version = latest_wayfern_version().ok_or_else(|| {
    serde_json::json!({ "code": "BROWSER_NOT_DOWNLOADED", "params": { "browser": "Wayfern" } })
      .to_string()
  })?;
  let os = settings
    .wayfern_config
    .as_ref()
    .and_then(|c| c.os.as_deref());
  if !crate::cloud_auth::CLOUD_AUTH
    .is_fingerprint_os_allowed(os)
    .await
  {
    return Err(code("FINGERPRINT_REQUIRES_PRO"));
  }
  let routes: Vec<Option<String>> = if settings.proxy_ids.is_empty() {
    vec![settings.proxy_id.clone()]
  } else {
    settings.proxy_ids.iter().cloned().map(Some).collect()
  };
  for proxy_id in &routes {
    crate::validate_profile_network(proxy_id.as_deref(), settings.vpn_id.as_deref()).await?;
  }

  let manager = ProfileManager::instance();
  let existing: Vec<String> = manager
    .list_profiles()
    .map_err(|e| crate::wrap_backend_error(e, "Failed to list profiles"))?
    .into_iter()
    .map(|profile| profile.name)
    .collect();
  let names = names_for(base, count, &existing);

  let started = std::time::Instant::now();
  let mut created = Vec::with_capacity(names.len());
  let mut error = None;
  let mut cancelled = false;
  for (index, name) in names.iter().enumerate() {
    if BULK_CANCEL.load(Ordering::SeqCst) {
      cancelled = true;
      break;
    }
    // A batch stops at the hourly allowance on every plan. One profile past
    // a free allowance is a warning; a batch past it gets generation
    // blocked upstream.
    let allowance = crate::profile_generation_limiter::get_profile_creation_allowance().await;
    if let Some(retry_after) = allowance.retry_after_secs {
      error = Some(
        serde_json::json!({
          "code": "PROFILE_GENERATION_LIMIT_REACHED",
          "params": {
            "limit": allowance.per_hour.to_string(),
            "retryAfterSeconds": retry_after.to_string(),
          }
        })
        .to_string(),
      );
      break;
    }
    let made = manager
      .create_profile_with_group(
        app_handle,
        name,
        "wayfern",
        &version,
        "stable",
        routes[index % routes.len()].clone(),
        settings.vpn_id.clone(),
        Some(settings.wayfern_config.clone().unwrap_or_default()),
        settings.group_id.clone(),
        settings.ephemeral,
        settings.dns_blocklist.clone(),
        settings.dns_server.clone(),
        settings.launch_hook.clone(),
      )
      .await
      .and_then(|profile| manager.finish_new_profile(profile, &after));
    match made {
      Ok(profile) => created.push(profile),
      Err(e) => {
        error = Some(crate::wrap_backend_error(e, "Failed to create profile"));
        break;
      }
    }
    let progress = BulkCreateProgress {
      done: index as u32 + 1,
      total: count,
    };
    if let Err(e) = crate::events::emit("profiles-bulk-create-progress", progress) {
      log::debug!("Event emit failed event=profiles-bulk-create-progress err=\"{e}\"");
    }
  }

  let elapsed_ms = started.elapsed().as_millis();
  match &error {
    Some(err) => log::warn!(
      "Profiles created in bulk requested={count} created={} elapsed_ms={elapsed_ms} err=\"{err}\"",
      created.len()
    ),
    None => log::info!(
      "Profiles created in bulk requested={count} created={} cancelled={cancelled} elapsed_ms={elapsed_ms}",
      created.len()
    ),
  }
  if created.is_empty() {
    if let Some(error) = error {
      return Err(error);
    }
  }
  Ok(BulkCreateResult {
    created,
    error,
    cancelled,
  })
}

/// Make profiles from a saved template. The name defaults to the template's
/// name prefix, else its name.
pub async fn create_from_template(
  app_handle: &tauri::AppHandle,
  template_id: &str,
  name: Option<&str>,
  count: u32,
) -> Result<BulkCreateResult, String> {
  let template = get(template_id)?;
  let name = name
    .map(str::trim)
    .filter(|name| !name.is_empty())
    .map(str::to_string)
    .or_else(|| template.settings.name_prefix.clone())
    .unwrap_or_else(|| template.name.clone());
  create_profiles(app_handle, template.settings, &name, count).await
}

#[tauri::command]
pub fn list_profile_templates() -> Vec<ProfileTemplate> {
  list()
}

#[tauri::command]
pub async fn create_profile_template(
  name: String,
  settings: TemplateSettings,
) -> Result<ProfileTemplate, String> {
  create(&name, settings).await
}

#[tauri::command]
pub async fn update_profile_template(
  template_id: String,
  name: String,
  settings: TemplateSettings,
) -> Result<ProfileTemplate, String> {
  update(&template_id, &name, settings).await
}

#[tauri::command]
pub fn delete_profile_template(template_id: String) -> Result<(), String> {
  delete(&template_id)
}

#[tauri::command]
pub async fn save_profile_template_from_profile(
  profile_id: String,
  name: String,
) -> Result<ProfileTemplate, String> {
  save_profile_as_template(&profile_id, &name).await
}

#[tauri::command]
pub async fn bulk_create_profiles(
  app_handle: tauri::AppHandle,
  settings: TemplateSettings,
  name: String,
  count: u32,
) -> Result<BulkCreateResult, String> {
  create_profiles(&app_handle, settings, &name, count).await
}

/// Stop a running bulk creation after the profile being made now.
#[tauri::command]
pub fn cancel_bulk_create_profiles() {
  BULK_CANCEL.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_continue_after_the_highest_number_in_use() {
    let existing = vec![
      "Shop 1".to_string(),
      "shop 7".to_string(),
      "Shop 3b".to_string(),
      "Shopper 9".to_string(),
    ];
    assert_eq!(
      names_for("Shop", 3, &existing),
      ["Shop 8", "Shop 9", "Shop 10"]
    );
    assert_eq!(names_for("Mail", 2, &existing), ["Mail 1", "Mail 2"]);
    assert_eq!(
      names_for("Shop", 1, &existing),
      ["Shop"],
      "one profile keeps the name as typed"
    );
  }

  #[test]
  fn a_template_never_carries_a_device() {
    let config = WayfernConfig {
      fingerprint: Some("{\"a\":1}".to_string()),
      identity_id: Some("id".to_string()),
      identity_overrides: Some("{}".to_string()),
      location: Some("{}".to_string()),
      camera_file: Some("/Users/me/cam.png".to_string()),
      os: Some("windows".to_string()),
      block_images: Some(true),
      ..Default::default()
    };
    let kept = portable(config);
    assert!(kept.fingerprint.is_none() && kept.identity_id.is_none());
    assert!(kept.identity_overrides.is_none() && kept.location.is_none());
    assert!(kept.camera_file.is_none());
    assert_eq!(kept.os.as_deref(), Some("windows"));
    assert_eq!(kept.block_images, Some(true));

    let profile = BrowserProfile {
      name: "Shop".to_string(),
      note: Some("login for shop".to_string()),
      tags: vec!["shop".to_string()],
      wayfern_config: Some(WayfernConfig {
        fingerprint: Some("{}".to_string()),
        ..Default::default()
      }),
      ..Default::default()
    };
    let settings = TemplateSettings::from_profile(&profile);
    assert_eq!(settings.tags, ["shop"]);
    assert_eq!(settings.note, None, "a note belongs to its one profile");
    assert!(settings.wayfern_config.unwrap().fingerprint.is_none());
  }

  #[tokio::test]
  async fn settings_are_checked_and_tidied() {
    let settings = TemplateSettings {
      name_prefix: Some("  Shop ".to_string()),
      proxy_ids: vec![" a ".to_string(), "a".to_string(), "".to_string()],
      tags: vec![" x ".to_string(), "x".to_string()],
      ephemeral: true,
      clear_on_close: true,
      start_urls: vec!["shop.example".to_string()],
      ..Default::default()
    }
    .checked()
    .await
    .unwrap();
    assert_eq!(settings.name_prefix.as_deref(), Some("Shop"));
    assert_eq!(settings.proxy_ids, ["a"]);
    assert_eq!(settings.tags, ["x"]);
    assert!(!settings.clear_on_close);
    assert_eq!(settings.start_urls, ["https://shop.example/"]);

    let both = TemplateSettings {
      proxy_id: Some("p".to_string()),
      vpn_id: Some("v".to_string()),
      ..Default::default()
    };
    assert!(both
      .checked()
      .await
      .unwrap_err()
      .contains("PROXY_AND_VPN_MUTUALLY_EXCLUSIVE"));
  }
}

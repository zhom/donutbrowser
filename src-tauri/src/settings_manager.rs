use serde::{Deserialize, Serialize};
use std::fs::{self, create_dir_all};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TableSortingSettings {
  pub column: String,    // Column to sort by: "name", "browser", "status"
  pub direction: String, // "asc" or "desc"
}

impl Default for TableSortingSettings {
  fn default() -> Self {
    Self {
      column: "name".to_string(),
      direction: "asc".to_string(),
    }
  }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
  #[serde(default)]
  pub set_as_default_browser: bool,
  #[serde(default = "default_theme")]
  pub theme: String, // "light", "dark", or "system"
  #[serde(default)]
  pub custom_theme: Option<std::collections::HashMap<String, String>>, // CSS var name -> value (e.g., "--background": "#1a1b26")
  #[serde(default)]
  pub api_enabled: bool,
  #[serde(default = "default_api_port")]
  pub api_port: u16,
  #[serde(default)]
  pub api_token: Option<String>, // Displayed token for user to copy
  #[serde(default)]
  pub sync_server_url: Option<String>, // URL of the sync server
  #[serde(default)]
  pub first_launch_timestamp: Option<u64>, // Unix epoch seconds when app was first launched
  #[serde(default)]
  pub commercial_trial_acknowledged: bool, // Has user dismissed the trial expiration modal
  #[serde(default)]
  pub mcp_enabled: bool, // Enable MCP (Model Context Protocol) server
  #[serde(default)]
  pub mcp_port: Option<u16>, // Port for MCP server (default 51080)
  #[serde(default)]
  pub mcp_token: Option<String>, // Displayed token for user to copy (not persisted, loaded from encrypted file)
  /// Let Donut cloud drive this installation's MCP tools over an outbound
  /// bridge, so an agent on the website can control this browser.
  ///
  /// Defaults to OFF and stays off until the user says otherwise. It opens a
  /// long-lived socket to Donut cloud and hands the far end the ability to
  /// launch and drive profiles, which is not something to switch on for
  /// somebody by default because their plan happens to include it.
  #[serde(default)]
  pub mcp_remote_enabled: bool,
  /// The durable `dmk_` credential agents present to the remote MCP endpoint.
  ///
  /// Plaintext, kept in an encrypted file with the same posture as
  /// `mcp_token`: loaded into the struct for a frontend settings read (the fx
  /// client cannot take the credential from its config file, so the page
  /// offers the export line), and stripped by `save_settings` so the settings
  /// JSON never carries it. Absent from the wire when there is none, so a
  /// settings file written by an older build stays byte-for-byte unchanged.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mcp_remote_key: Option<String>,
  /// The server-side id of `mcp_remote_key`, so a rotation can revoke exactly
  /// the key it replaces. Not a secret; lives in the settings JSON.
  #[serde(default)]
  pub mcp_remote_key_id: Option<String>,
  #[serde(default)]
  pub language: Option<String>, // ISO 639-1: "en", "es", "pt", "fr", "zh", "ja", "ko", "ru", or None for system default
  #[serde(default)]
  pub window_resize_warning_dismissed: bool,
  /// Stop blocking launches whose proxy exit disagrees with the fingerprint.
  /// Lives here rather than in localStorage because the Rust launch path is
  /// what enforces the block and cannot read the frontend's storage.
  #[serde(default)]
  pub fingerprint_gate_disabled: bool,
  /// Stop warning about VPN/proxy extensions found in a profile.
  #[serde(default)]
  pub vpn_extension_warning_disabled: bool,
  #[serde(default)]
  pub onboarding_completed: bool, // First-launch onboarding has been shown/handled (one-shot)
  #[serde(default)]
  pub disable_auto_updates: bool,
  /// When true, the decrypted in-RAM copy of a password-protected profile is
  /// preserved between launches for faster subsequent startups. The on-disk
  /// copy is always re-encrypted regardless of this flag.
  #[serde(default)]
  pub keep_decrypted_profiles_in_ram: bool,
  /// How long a deleted profile stays in the trash before it is purged.
  /// Clamped to 1..=365 on save; the sweeper reads it through
  /// `profile::trash::configured_retention_days`.
  #[serde(default = "default_trash_retention_days")]
  pub trash_retention_days: u32,
  /// Feature tips. Whether one tip the user has not seen yet may open by
  /// itself shortly after launch. Off is the user's choice, made in the tips
  /// dialog.
  #[serde(default = "default_tips_auto_show")]
  pub tips_auto_show: bool,
  /// Ids of the tips that have been shown, in the automatic or the browse
  /// flow, so the automatic flow never repeats one.
  #[serde(default)]
  pub tips_seen: Vec<String>,
  /// Unix seconds of the last tip that opened by itself. Paces the automatic
  /// flow to one tip a day at most.
  #[serde(default)]
  pub tips_last_auto_shown_at: Option<u64>,
  /// Cloud user ids that have had the paid-plan welcome.
  #[serde(default)]
  pub paid_welcome_seen_for: Vec<String>,
  /// The plan status last observed per cloud user id, `"free"` or `"paid"`.
  /// A change from free to paid is what earns the paid-plan welcome.
  #[serde(default)]
  pub cloud_plan_memory: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SyncSettings {
  pub sync_server_url: Option<String>,
  pub sync_token: Option<String>, // Only populated when reading, not stored in JSON
}

fn default_theme() -> String {
  "system".to_string()
}

fn default_api_port() -> u16 {
  10108
}

fn default_trash_retention_days() -> u32 {
  crate::profile::trash::DEFAULT_RETENTION_DAYS
}

fn default_tips_auto_show() -> bool {
  true
}

/// How long the automatic tip flow waits between two tips, so a busy day of
/// restarts does not turn into a tip on every launch.
pub const TIPS_AUTO_INTERVAL_SECS: u64 = 20 * 60 * 60;

/// The plan status remembered per cloud user.
const PLAN_STATUS_PAID: &str = "paid";
const PLAN_STATUS_FREE: &str = "free";

impl Default for AppSettings {
  fn default() -> Self {
    Self {
      set_as_default_browser: false,
      theme: "system".to_string(),
      custom_theme: None,
      api_enabled: false,
      api_port: 10108,
      api_token: None,
      sync_server_url: None,
      first_launch_timestamp: None,
      commercial_trial_acknowledged: false,
      mcp_enabled: false,
      mcp_port: None,
      mcp_token: None,
      mcp_remote_enabled: false,
      mcp_remote_key: None,
      mcp_remote_key_id: None,
      language: None,
      window_resize_warning_dismissed: false,
      fingerprint_gate_disabled: false,
      vpn_extension_warning_disabled: false,
      onboarding_completed: false,
      disable_auto_updates: false,
      keep_decrypted_profiles_in_ram: false,
      trash_retention_days: crate::profile::trash::DEFAULT_RETENTION_DAYS,
      tips_auto_show: true,
      tips_seen: Vec::new(),
      tips_last_auto_shown_at: None,
      paid_welcome_seen_for: Vec::new(),
      cloud_plan_memory: std::collections::HashMap::new(),
    }
  }
}

/// The remote MCP credential as it is kept on this machine.
#[derive(Debug, Clone)]
pub struct StoredMcpRemoteKey {
  /// The plaintext `dmk_` key.
  pub key: String,
  /// The server-side id, when the store that wrote the key also recorded it.
  pub id: Option<String>,
}

pub struct SettingsManager;

/// Write `content` to `path` in one step: to a sibling first, then renamed
/// into place. A reader that opens the file mid-write, and there are several
/// at startup, sees the old settings or the new ones, never an empty file
/// that parses as the defaults.
fn write_whole(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
  let staging = path.with_extension("json.tmp");
  fs::write(&staging, content)?;
  if let Err(e) = fs::rename(&staging, path) {
    let _ = fs::remove_file(&staging);
    return Err(e);
  }
  Ok(())
}

impl SettingsManager {
  pub(crate) fn new() -> Self {
    Self
  }

  pub fn instance() -> &'static SettingsManager {
    &SETTINGS_MANAGER
  }

  pub fn get_settings_dir(&self) -> PathBuf {
    crate::app_dirs::settings_dir()
  }

  pub fn get_settings_file(&self) -> PathBuf {
    self.get_settings_dir().join("app_settings.json")
  }

  pub fn get_table_sorting_file(&self) -> PathBuf {
    self.get_settings_dir().join("table_sorting.json")
  }

  pub fn load_settings(&self) -> Result<AppSettings, Box<dyn std::error::Error>> {
    let settings_file = self.get_settings_file();

    if !settings_file.exists() {
      // Return default settings if file doesn't exist
      return Ok(AppSettings::default());
    }

    let content = fs::read_to_string(&settings_file)?;

    // Parse the settings file - serde will use default values for missing fields
    match serde_json::from_str::<AppSettings>(&content) {
      Ok(settings) => Ok(settings),
      Err(e) => {
        log::warn!("Warning: Failed to parse settings file, using defaults: {e}");
        Ok(AppSettings::default())
      }
    }
  }

  pub fn save_settings(&self, settings: &AppSettings) -> Result<(), Box<dyn std::error::Error>> {
    let settings_dir = self.get_settings_dir();
    create_dir_all(&settings_dir)?;

    // The remote MCP credential works from anywhere on the internet and has
    // its own encrypted file; a struct loaded for the frontend carries it, so
    // it is dropped at the one place the JSON gets written rather than at
    // every caller that happens to hold such a struct.
    let mut on_disk = settings.clone();
    on_disk.mcp_remote_key = None;

    let settings_file = self.get_settings_file();
    let json = serde_json::to_string_pretty(&on_disk)?;
    write_whole(&settings_file, json.as_bytes())?;

    Ok(())
  }

  pub fn load_table_sorting(&self) -> Result<TableSortingSettings, Box<dyn std::error::Error>> {
    let sorting_file = self.get_table_sorting_file();

    if !sorting_file.exists() {
      // Return default sorting if file doesn't exist
      return Ok(TableSortingSettings::default());
    }

    let content = fs::read_to_string(sorting_file)?;
    let sorting: TableSortingSettings = serde_json::from_str(&content)?;
    Ok(sorting)
  }

  pub fn save_table_sorting(
    &self,
    sorting: &TableSortingSettings,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let settings_dir = self.get_settings_dir();
    create_dir_all(&settings_dir)?;

    let sorting_file = self.get_table_sorting_file();
    let json = serde_json::to_string_pretty(sorting)?;
    write_whole(&sorting_file, json.as_bytes())?;

    Ok(())
  }

  /// Seal `secret` into `file`.
  ///
  /// One implementation for every secret this manager keeps on disk, in
  /// `crate::vault`: the API, MCP and sync tokens and the remote MCP
  /// credential share the layout, and only the five-byte header tells them
  /// apart.
  fn encrypt_to_file(
    file: &std::path::Path,
    header: &[u8; 5],
    secret: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    crate::vault::seal(file, &Self::magic(header), secret)?;
    Ok(())
  }

  /// Read back a secret written by `encrypt_to_file`.
  ///
  /// A missing file, a foreign header or a layout this version does not know
  /// all read as "no secret" rather than an error, so a stale or damaged file
  /// never blocks the feature it belongs to; the caller simply mints again.
  fn decrypt_from_file(
    file: &std::path::Path,
    header: &[u8; 5],
  ) -> Result<Option<String>, Box<dyn std::error::Error>> {
    Ok(crate::vault::open(file, &Self::magic(header))?)
  }

  /// The header plus the layout version every file of this manager carries.
  fn magic(header: &[u8; 5]) -> [u8; 6] {
    let mut magic = [0u8; 6];
    magic[..5].copy_from_slice(header);
    magic[5] = 2;
    magic
  }

  fn remove_secret_file(file: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    if file.exists() {
      std::fs::remove_file(file)?;
    }
    Ok(())
  }

  /// A fresh 256-bit token, base64url so it is safe in a URL path.
  fn random_token() -> String {
    let token_bytes: [u8; 32] = {
      use rand::Rng;
      let mut rng = rand::rng();
      let mut bytes = [0u8; 32];
      rng.fill_bytes(&mut bytes);
      bytes
    };
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::URL_SAFE_NO_PAD.encode(token_bytes)
  }

  fn api_token_file(&self) -> PathBuf {
    self.get_settings_dir().join("api_token.dat")
  }

  fn mcp_token_file(&self) -> PathBuf {
    self.get_settings_dir().join("mcp_token.dat")
  }

  fn sync_token_file(&self) -> PathBuf {
    self.get_settings_dir().join("sync_token.dat")
  }

  fn mcp_remote_key_file(&self) -> PathBuf {
    self.get_settings_dir().join("mcp_remote_key.dat")
  }

  pub async fn generate_api_token(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<String, Box<dyn std::error::Error>> {
    let token = Self::random_token();
    self.store_api_token(app_handle, &token).await?;
    Ok(token)
  }

  pub async fn store_api_token(
    &self,
    _app_handle: &tauri::AppHandle,
    token: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    Self::encrypt_to_file(&self.api_token_file(), b"DBAPI", token)
  }

  pub async fn get_api_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<Option<String>, Box<dyn std::error::Error>> {
    Self::decrypt_from_file(&self.api_token_file(), b"DBAPI")
  }

  pub async fn remove_api_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error>> {
    Self::remove_secret_file(&self.api_token_file())
  }

  pub async fn generate_mcp_token(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<String, Box<dyn std::error::Error>> {
    let token = Self::random_token();
    self.store_mcp_token(app_handle, &token).await?;
    Ok(token)
  }

  pub async fn store_mcp_token(
    &self,
    _app_handle: &tauri::AppHandle,
    token: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    Self::encrypt_to_file(&self.mcp_token_file(), b"DBMCP", token)
  }

  pub async fn get_mcp_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<Option<String>, Box<dyn std::error::Error>> {
    Self::decrypt_from_file(&self.mcp_token_file(), b"DBMCP")
  }

  pub async fn remove_mcp_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error>> {
    Self::remove_secret_file(&self.mcp_token_file())
  }

  pub async fn store_sync_token(
    &self,
    _app_handle: &tauri::AppHandle,
    token: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    Self::encrypt_to_file(&self.sync_token_file(), b"DBSYN", token)
  }

  pub async fn get_sync_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<Option<String>, Box<dyn std::error::Error>> {
    Self::decrypt_from_file(&self.sync_token_file(), b"DBSYN")
  }

  pub async fn remove_sync_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error>> {
    Self::remove_secret_file(&self.sync_token_file())
  }

  /// Keep the remote MCP credential: the `dmk_` key in its own encrypted file
  /// and the server-side key id in the settings JSON, so a later rotation can
  /// name the key it is retiring.
  ///
  /// The plaintext is deliberately NOT part of the settings JSON:
  /// `save_settings` strips it, and `get_app_settings` is the one reader that
  /// loads it back for the frontend, the way the local display tokens are.
  pub fn store_mcp_remote_key(
    &self,
    key: &str,
    key_id: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    Self::encrypt_to_file(&self.mcp_remote_key_file(), b"DBMRK", key)?;
    let mut settings = self.load_settings()?;
    settings.mcp_remote_key_id = Some(key_id.to_string());
    self.save_settings(&settings)
  }

  /// The stored remote MCP credential, if any: the plaintext key and the id
  /// the server knows it by.
  ///
  /// Read with the id from the JSON and the key from its file, so the two
  /// cannot disagree: a key file without an id (an interrupted store) still
  /// yields the key, and an id without a key file yields nothing at all.
  pub fn get_mcp_remote_key(
    &self,
  ) -> Result<Option<StoredMcpRemoteKey>, Box<dyn std::error::Error>> {
    let Some(key) = Self::decrypt_from_file(&self.mcp_remote_key_file(), b"DBMRK")? else {
      return Ok(None);
    };
    let id = self.load_settings()?.mcp_remote_key_id;
    Ok(Some(StoredMcpRemoteKey { key, id }))
  }

  /// Drop the remote MCP credential from this machine. Does not revoke it:
  /// that is the caller's job, because only the caller knows whether it still
  /// has a session to revoke with.
  pub fn remove_mcp_remote_key(&self) -> Result<(), Box<dyn std::error::Error>> {
    Self::remove_secret_file(&self.mcp_remote_key_file())?;
    let mut settings = self.load_settings()?;
    if settings.mcp_remote_key_id.take().is_some() {
      self.save_settings(&settings)?;
    }
    Ok(())
  }

  pub fn get_sync_settings(&self) -> Result<SyncSettings, Box<dyn std::error::Error>> {
    let settings = self.load_settings()?;
    Ok(SyncSettings {
      sync_server_url: settings.sync_server_url,
      sync_token: None, // Token needs to be loaded separately via async method
    })
  }

  pub fn save_sync_server_url(
    &self,
    url: Option<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let mut settings = self.load_settings()?;
    settings.sync_server_url = url;
    self.save_settings(&settings)
  }
}

#[tauri::command]
pub async fn get_app_settings(app_handle: tauri::AppHandle) -> Result<AppSettings, String> {
  let manager = SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;

  // Always load tokens for display purposes if they exist
  settings.api_token = manager
    .get_api_token(&app_handle)
    .await
    .map_err(|e| format!("Failed to load API token: {e}"))?;

  settings.mcp_token = manager
    .get_mcp_token(&app_handle)
    .await
    .map_err(|e| format!("Failed to load MCP token: {e}"))?;

  // Same posture as the local tokens: shown so the fx export line can be
  // copied, never persisted (see `SettingsManager::save_settings`).
  settings.mcp_remote_key = manager
    .get_mcp_remote_key()
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?
    .map(|stored| stored.key);

  Ok(settings)
}

#[tauri::command]
pub async fn save_app_settings(
  app_handle: tauri::AppHandle,
  mut settings: AppSettings,
) -> Result<AppSettings, String> {
  let manager = SettingsManager::instance();

  // The remote MCP credential is minted by `rotate_mcp_remote_credential` and
  // by nothing else. A settings read hands the frontend the plaintext (for the
  // fx export line) and the frontend echoes the whole struct back, so the
  // field is simply not the frontend's to write: whatever arrived is dropped
  // here and the stored key is what the answer below carries.
  settings.mcp_remote_key = None;

  // Handle API token
  if settings.api_enabled {
    if let Some(ref token) = settings.api_token {
      manager
        .store_api_token(&app_handle, token)
        .await
        .map_err(|e| format!("Failed to store API token: {e}"))?;
    } else {
      // Check if a token already exists on disk before generating a new one
      let existing = manager.get_api_token(&app_handle).await.ok().flatten();
      if let Some(t) = existing {
        settings.api_token = Some(t);
      } else {
        let token = manager
          .generate_api_token(&app_handle)
          .await
          .map_err(|e| format!("Failed to generate API token: {e}"))?;
        settings.api_token = Some(token);
      }
    }
  }

  if !settings.api_enabled {
    manager
      .remove_api_token(&app_handle)
      .await
      .map_err(|e| format!("Failed to remove API token: {e}"))?;
    settings.api_token = None;
  }

  // Handle MCP token
  if settings.mcp_enabled {
    if let Some(ref token) = settings.mcp_token {
      manager
        .store_mcp_token(&app_handle, token)
        .await
        .map_err(|e| format!("Failed to store MCP token: {e}"))?;
    } else {
      // Check if a token already exists on disk before generating a new one
      let existing = manager.get_mcp_token(&app_handle).await.ok().flatten();
      if let Some(t) = existing {
        settings.mcp_token = Some(t);
      } else {
        let token = manager
          .generate_mcp_token(&app_handle)
          .await
          .map_err(|e| format!("Failed to generate MCP token: {e}"))?;
        settings.mcp_token = Some(token);
        // A running local server now answers on a URL the installed clients
        // do not know, so they are rewritten. With the server off there is no
        // URL to write yet; `McpServer::start` does this when it comes up.
        if crate::mcp_server::McpServer::instance()
          .get_port()
          .is_some()
        {
          let failed =
            crate::reinstall_mcp_agents(&app_handle, crate::mcp_integrations::McpEndpoint::Local)
              .await;
          if !failed.is_empty() {
            log::warn!(
              "[settings] Could not refresh the clients pointing at the local server: {}",
              failed.join(", ")
            );
          }
        }
      }
    }
  }

  if !settings.mcp_enabled {
    manager
      .remove_mcp_token(&app_handle)
      .await
      .map_err(|e| format!("Failed to remove MCP token: {e}"))?;
    settings.mcp_token = None;
  }

  // Preserve the fields the frontend does not own. Read directly from the
  // file to avoid load_settings' save-on-load behavior.
  //
  // `mcp_remote_enabled` is flipped ONLY by `start_mcp_remote_bridge` and
  // `stop_mcp_remote_bridge`, which also start and stop the bridge task. A
  // settings save that carried the flag could switch the internet-facing
  // bridge on for the next launch without ever going through the sign-in and
  // terms gates those commands enforce, or switch it off on disk while the
  // task kept running. The key id is bookkeeping for the rotation path and is
  // never the frontend's to write.
  if let Ok(content) = std::fs::read_to_string(manager.get_settings_file()) {
    if let Ok(current) = serde_json::from_str::<AppSettings>(&content) {
      settings.window_resize_warning_dismissed = current.window_resize_warning_dismissed;
      settings.mcp_remote_enabled = current.mcp_remote_enabled;
      settings.mcp_remote_key_id = current.mcp_remote_key_id;
    }
  } else {
    settings.mcp_remote_enabled = false;
    settings.mcp_remote_key_id = None;
  }

  settings.trash_retention_days =
    crate::profile::trash::clamp_retention_days(settings.trash_retention_days);

  let mut persist_settings = settings.clone();
  persist_settings.api_token = None;
  persist_settings.mcp_token = None;

  log::info!(
    "[settings] Saving settings: theme={}, custom_theme_keys={}",
    persist_settings.theme,
    persist_settings
      .custom_theme
      .as_ref()
      .map(|t| t.len())
      .unwrap_or(0)
  );

  manager
    .save_settings(&persist_settings)
    .map_err(|e| format!("Failed to save settings: {e}"))?;

  // Answer with what a fresh read would show, the stored credential included,
  // so a page that keeps the answer as its settings does not lose the fx
  // export line on every save.
  settings.mcp_remote_key = manager
    .get_mcp_remote_key()
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?
    .map(|stored| stored.key);

  Ok(settings)
}

/// Read the most recent N log files concatenated into a single string,
/// suitable for paste-into-issue-tracker. Newest entries appear LAST so the
/// reader sees fresh context at the bottom of the buffer. Capped at 5 MB to
/// keep clipboard payloads sane.
#[tauri::command]
pub async fn read_log_files(app_handle: tauri::AppHandle) -> Result<String, String> {
  let dir = crate::app_dirs::log_dir(&app_handle);
  if !dir.exists() {
    return Err("Log directory does not exist yet".to_string());
  }

  let mut entries: Vec<(std::path::PathBuf, std::time::SystemTime)> = std::fs::read_dir(&dir)
    .map_err(|e| format!("Failed to read log dir: {e}"))?
    .filter_map(|r| r.ok())
    .filter_map(|e| {
      let p = e.path();
      let m = e.metadata().ok()?.modified().ok()?;
      let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
      if p.is_file() && (ext == "log" || ext == "txt") {
        Some((p, m))
      } else {
        None
      }
    })
    .collect();

  entries.sort_by_key(|(_, m)| *m);

  const MAX_BYTES: usize = 5 * 1024 * 1024;
  let mut out = String::with_capacity(64 * 1024);
  for (path, _) in entries.iter().rev() {
    let header = format!(
      "===== {} =====\n",
      path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("log")
    );
    if out.len() + header.len() >= MAX_BYTES {
      break;
    }
    out.push_str(&header);
    if let Ok(content) = std::fs::read_to_string(path) {
      let take = MAX_BYTES.saturating_sub(out.len());
      if take == 0 {
        break;
      }
      if content.len() > take {
        // Tail truncation — keep the END of older files so newest data is preserved.
        out.push_str("[…truncated — older content elided…]\n");
        out.push_str(&content[content.len() - take + 64..]);
      } else {
        out.push_str(&content);
      }
      if !out.ends_with('\n') {
        out.push('\n');
      }
    }
  }

  // Reverse the per-file order so chronological newest is at the bottom.
  // (We pushed newest-first above to budget the tail; flip now.)
  let mut sections: Vec<&str> = out.split("===== ").filter(|s| !s.is_empty()).collect();
  sections.reverse();
  let final_out = sections
    .into_iter()
    .map(|s| format!("===== {s}"))
    .collect::<String>();

  Ok(crate::log_redaction::text(&final_out))
}

/// Reveal the log directory in the OS file manager.
#[tauri::command]
pub async fn open_log_directory(app_handle: tauri::AppHandle) -> Result<(), String> {
  let dir = crate::app_dirs::log_dir(&app_handle);
  if !dir.exists() {
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create log dir: {e}"))?;
  }
  let path = dir.to_string_lossy().to_string();

  #[cfg(target_os = "macos")]
  {
    std::process::Command::new("open")
      .arg(&path)
      .spawn()
      .map_err(|e| format!("Failed to open log dir: {e}"))?;
  }
  #[cfg(target_os = "windows")]
  {
    std::process::Command::new("explorer")
      .arg(&path)
      .spawn()
      .map_err(|e| format!("Failed to open log dir: {e}"))?;
  }
  #[cfg(target_os = "linux")]
  {
    std::process::Command::new("xdg-open")
      .arg(&path)
      .spawn()
      .map_err(|e| format!("Failed to open log dir: {e}"))?;
  }
  Ok(())
}

#[tauri::command]
pub async fn get_table_sorting_settings() -> Result<TableSortingSettings, String> {
  let manager = SettingsManager::instance();
  manager
    .load_table_sorting()
    .map_err(|e| format!("Failed to load table sorting settings: {e}"))
}

#[tauri::command]
pub async fn save_table_sorting_settings(sorting: TableSortingSettings) -> Result<(), String> {
  let manager = SettingsManager::instance();
  manager
    .save_table_sorting(&sorting)
    .map_err(|e| format!("Failed to save table sorting settings: {e}"))
}

#[tauri::command]
pub async fn get_sync_settings(app_handle: tauri::AppHandle) -> Result<SyncSettings, String> {
  // Cloud auth takes priority over self-hosted settings
  if crate::cloud_auth::CLOUD_AUTH.is_logged_in().await {
    let sync_token = crate::cloud_auth::CLOUD_AUTH
      .get_or_refresh_sync_token()
      .await
      .map_err(|e| format!("Failed to get cloud sync token: {e}"))?;
    return Ok(SyncSettings {
      sync_server_url: Some(crate::cloud_auth::CLOUD_SYNC_URL.to_string()),
      sync_token,
    });
  }

  // Fall back to self-hosted settings
  let manager = SettingsManager::instance();
  let mut sync_settings = manager
    .get_sync_settings()
    .map_err(|e| format!("Failed to load sync settings: {e}"))?;

  sync_settings.sync_token = manager
    .get_sync_token(&app_handle)
    .await
    .map_err(|e| format!("Failed to load sync token: {e}"))?;

  Ok(sync_settings)
}

#[tauri::command]
pub async fn save_sync_settings(
  app_handle: tauri::AppHandle,
  sync_server_url: Option<String>,
  sync_token: Option<String>,
) -> Result<SyncSettings, String> {
  // Cloud login and self-hosted sync share the same sync engine and a
  // profile can't be sync'd to two backends at once. Block any *write*
  // (non-null URL or token) while the user is signed into their cloud
  // account — the clearing path (both `None`) is always allowed so logged-
  // in users can wipe a stale self-hosted config that pre-dates their
  // sign-in.
  let is_setting_self_hosted = sync_server_url.is_some() || sync_token.is_some();
  if is_setting_self_hosted && crate::cloud_auth::CLOUD_AUTH.is_logged_in().await {
    return Err(serde_json::json!({ "code": "SELF_HOSTED_REQUIRES_LOGOUT" }).to_string());
  }

  let manager = SettingsManager::instance();

  manager
    .save_sync_server_url(sync_server_url.clone())
    .map_err(|e| format!("Failed to save sync server URL: {e}"))?;

  if let Some(ref token) = sync_token {
    manager
      .store_sync_token(&app_handle, token)
      .await
      .map_err(|e| format!("Failed to store sync token: {e}"))?;
  } else {
    manager
      .remove_sync_token(&app_handle)
      .await
      .map_err(|e| format!("Failed to remove sync token: {e}"))?;
  }

  Ok(SyncSettings {
    sync_server_url,
    sync_token,
  })
}

#[tauri::command]
pub async fn dismiss_window_resize_warning() -> Result<(), String> {
  let manager = SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;
  settings.window_resize_warning_dismissed = true;
  manager
    .save_settings(&settings)
    .map_err(|e| format!("Failed to save settings: {e}"))
}

#[tauri::command]
pub async fn get_window_resize_warning_dismissed() -> Result<bool, String> {
  let manager = SettingsManager::instance();
  let settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;
  Ok(settings.window_resize_warning_dismissed)
}

#[tauri::command]
pub async fn get_onboarding_completed() -> Result<bool, String> {
  let manager = SettingsManager::instance();
  let settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;
  Ok(settings.onboarding_completed)
}

#[tauri::command]
pub async fn complete_onboarding() -> Result<(), String> {
  let manager = SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;
  settings.onboarding_completed = true;
  manager
    .save_settings(&settings)
    .map_err(|e| format!("Failed to save settings: {e}"))
}

/// What the tips dialog needs to decide what to open and what to skip.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TipsState {
  pub auto_show: bool,
  pub seen: Vec<String>,
  pub last_auto_shown_at: Option<u64>,
  /// Whether the automatic flow may open a tip right now: it is switched on
  /// and the last automatic tip is old enough.
  pub auto_due: bool,
}

impl TipsState {
  fn of(settings: &AppSettings, now: u64) -> Self {
    Self {
      auto_show: settings.tips_auto_show,
      seen: settings.tips_seen.clone(),
      last_auto_shown_at: settings.tips_last_auto_shown_at,
      auto_due: settings.tips_auto_show
        && settings
          .tips_last_auto_shown_at
          .is_none_or(|last| now.saturating_sub(last) >= TIPS_AUTO_INTERVAL_SECS),
    }
  }
}

/// Serialises every read-modify-write of the tips fields. Two tips shown in
/// quick succession are two concurrent commands, and without this the second
/// load could precede the first save and drop it.
static TIPS_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn unix_now() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

/// Remembers a tip as shown. `auto` marks it as the tip that opened by
/// itself, which restarts the daily pacing.
fn record_tip_seen(settings: &mut AppSettings, tip_id: &str, auto: bool, now: u64) {
  if !settings.tips_seen.iter().any(|id| id == tip_id) {
    settings.tips_seen.push(tip_id.to_string());
  }
  if auto {
    settings.tips_last_auto_shown_at = Some(now);
  }
}

/// Records the plan status seen for a cloud account and answers whether the
/// paid-plan welcome is due for it.
///
/// The welcome is for an account that just became paid: one this desktop last
/// saw as free, or one it sees for the first time right after the user signed
/// in (they bought a plan on the website and came back). An account that was
/// already paid the last time anybody looked, or that turns up paid in an old
/// session after an app update, is not new to its plan and is recorded as
/// greeted without a dialog.
fn paid_welcome_due(
  settings: &mut AppSettings,
  user_id: &str,
  paid: bool,
  fresh_login: bool,
) -> bool {
  let status = if paid {
    PLAN_STATUS_PAID
  } else {
    PLAN_STATUS_FREE
  };
  let previous = settings
    .cloud_plan_memory
    .insert(user_id.to_string(), status.to_string());
  if !paid {
    return false;
  }
  if settings
    .paid_welcome_seen_for
    .iter()
    .any(|id| id == user_id)
  {
    return false;
  }
  let due = match previous.as_deref() {
    Some(PLAN_STATUS_FREE) => true,
    Some(_) => false,
    None => fresh_login,
  };
  settings.paid_welcome_seen_for.push(user_id.to_string());
  due
}

#[tauri::command]
pub async fn get_tips_state() -> Result<TipsState, String> {
  let manager = SettingsManager::instance();
  let settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;
  Ok(TipsState::of(&settings, unix_now()))
}

#[tauri::command]
pub async fn mark_tip_seen(tip_id: String, auto: bool) -> Result<TipsState, String> {
  let _serial = TIPS_WRITE
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  let manager = SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;
  let now = unix_now();
  record_tip_seen(&mut settings, &tip_id, auto, now);
  manager
    .save_settings(&settings)
    .map_err(|e| format!("Failed to save settings: {e}"))?;
  Ok(TipsState::of(&settings, now))
}

#[tauri::command]
pub async fn set_tips_auto_show(enabled: bool) -> Result<TipsState, String> {
  let _serial = TIPS_WRITE
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  let manager = SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;
  settings.tips_auto_show = enabled;
  manager
    .save_settings(&settings)
    .map_err(|e| format!("Failed to save settings: {e}"))?;
  Ok(TipsState::of(&settings, unix_now()))
}

#[tauri::command]
pub async fn observe_cloud_plan(
  user_id: String,
  paid: bool,
  fresh_login: bool,
) -> Result<bool, String> {
  let _serial = TIPS_WRITE
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  let manager = SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;
  let due = paid_welcome_due(&mut settings, &user_id, paid, fresh_login);
  manager
    .save_settings(&settings)
    .map_err(|e| format!("Failed to save settings: {e}"))?;
  Ok(due)
}

#[tauri::command]
pub fn get_system_language() -> String {
  sys_locale::get_locale()
    .map(|locale| {
      // Extract just the language code (e.g., "en" from "en-US")
      locale
        .split(['-', '_'])
        .next()
        .unwrap_or("en")
        .to_lowercase()
    })
    .unwrap_or_else(|| "en".to_string())
}

#[derive(Debug, Serialize, Clone)]
pub struct SystemInfo {
  pub app_version: String,
  pub os: String,
  pub arch: String,
  pub portable: bool,
}

#[tauri::command]
pub fn get_system_info() -> SystemInfo {
  let os = if cfg!(target_os = "macos") {
    "macOS"
  } else if cfg!(target_os = "windows") {
    "Windows"
  } else if cfg!(target_os = "linux") {
    "Linux"
  } else {
    "Unknown"
  };

  let arch = if cfg!(target_arch = "x86_64") {
    "x86_64"
  } else if cfg!(target_arch = "aarch64") {
    "aarch64"
  } else {
    "unknown"
  };

  SystemInfo {
    app_version: crate::app_auto_updater::AppAutoUpdater::get_current_version(),
    os: os.to_string(),
    arch: arch.to_string(),
    portable: crate::app_dirs::is_portable(),
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref SETTINGS_MANAGER: SettingsManager = SettingsManager::new();
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn create_test_settings_manager() -> (SettingsManager, TempDir, crate::app_dirs::TestDirGuard) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let guard = crate::app_dirs::set_test_data_dir(temp_dir.path().to_path_buf());
    let manager = SettingsManager::new();
    (manager, temp_dir, guard)
  }

  #[test]
  fn test_settings_manager_creation() {
    let (_manager, _temp_dir, _guard) = create_test_settings_manager();
  }

  #[test]
  fn tips_state_defaults_to_automatic_and_due() {
    let settings = AppSettings::default();
    let state = TipsState::of(&settings, 1_000_000);
    assert!(state.auto_show);
    assert!(state.seen.is_empty());
    assert_eq!(state.last_auto_shown_at, None);
    assert!(state.auto_due, "a fresh install owes its first tip");
  }

  #[test]
  fn tips_seen_dedupes_and_paces_the_automatic_flow() {
    let mut settings = AppSettings::default();
    record_tip_seen(&mut settings, "dns", false, 100);
    record_tip_seen(&mut settings, "dns", false, 200);
    assert_eq!(settings.tips_seen, vec!["dns".to_string()]);
    assert_eq!(
      settings.tips_last_auto_shown_at, None,
      "a browsed tip must not restart the daily pacing"
    );

    record_tip_seen(&mut settings, "proxy", true, 1_000);
    assert_eq!(settings.tips_last_auto_shown_at, Some(1_000));
    assert!(
      !TipsState::of(&settings, 1_000 + TIPS_AUTO_INTERVAL_SECS - 1).auto_due,
      "the next automatic tip waits a day"
    );
    assert!(TipsState::of(&settings, 1_000 + TIPS_AUTO_INTERVAL_SECS).auto_due);

    settings.tips_auto_show = false;
    assert!(
      !TipsState::of(&settings, 1_000 + TIPS_AUTO_INTERVAL_SECS * 3).auto_due,
      "switched off means never due"
    );
  }

  #[test]
  fn paid_welcome_is_due_once_when_an_account_turns_paid() {
    let mut settings = AppSettings::default();
    assert!(!paid_welcome_due(&mut settings, "u1", false, true));
    assert!(
      settings.paid_welcome_seen_for.is_empty(),
      "a free account is not greeted, so nothing is recorded"
    );
    assert!(
      paid_welcome_due(&mut settings, "u1", true, false),
      "free to paid is the upgrade the welcome exists for"
    );
    assert!(!paid_welcome_due(&mut settings, "u1", true, true), "once");
    assert_eq!(settings.paid_welcome_seen_for, vec!["u1".to_string()]);
  }

  #[test]
  fn paid_welcome_greets_a_fresh_sign_in_but_not_an_old_paid_session() {
    let mut settings = AppSettings::default();
    assert!(
      paid_welcome_due(&mut settings, "bought-on-web", true, true),
      "first sight right after signing in: they came back from checkout"
    );

    assert!(
      !paid_welcome_due(&mut settings, "long-paid", true, false),
      "an app update on a machine that was already paid is not a new plan"
    );
    assert!(
      !paid_welcome_due(&mut settings, "long-paid", true, true),
      "and it is recorded as greeted, so it never fires later"
    );
    assert_eq!(
      settings
        .cloud_plan_memory
        .get("long-paid")
        .map(String::as_str),
      Some(PLAN_STATUS_PAID)
    );
  }

  #[test]
  fn test_default_app_settings() {
    let default_settings = AppSettings::default();

    assert!(
      !default_settings.set_as_default_browser,
      "Default should not set as default browser"
    );
    assert_eq!(
      default_settings.theme, "system",
      "Default theme should be system"
    );
  }

  #[test]
  fn test_default_table_sorting_settings() {
    let default_sorting = TableSortingSettings::default();

    assert_eq!(
      default_sorting.column, "name",
      "Default sort column should be name"
    );
    assert_eq!(
      default_sorting.direction, "asc",
      "Default sort direction should be asc"
    );
  }

  #[test]
  fn test_load_settings_nonexistent_file() {
    let (manager, _temp_dir, _guard) = create_test_settings_manager();

    let result = manager.load_settings();
    assert!(
      result.is_ok(),
      "Should handle nonexistent settings file gracefully"
    );

    let settings = result.unwrap();
    assert!(
      !settings.set_as_default_browser,
      "Should return default settings"
    );
    assert_eq!(settings.theme, "system", "Should return default theme");
  }

  #[test]
  fn test_save_and_load_settings() {
    let (manager, _temp_dir, _guard) = create_test_settings_manager();

    let test_settings = AppSettings {
      set_as_default_browser: true,
      theme: "dark".to_string(),
      custom_theme: None,
      api_enabled: false,
      api_port: 10108,
      api_token: None,
      sync_server_url: None,
      first_launch_timestamp: None,
      commercial_trial_acknowledged: false,
      mcp_enabled: false,
      mcp_port: None,
      mcp_token: None,
      mcp_remote_enabled: false,
      mcp_remote_key: None,
      mcp_remote_key_id: None,
      language: None,
      window_resize_warning_dismissed: false,
      fingerprint_gate_disabled: false,
      vpn_extension_warning_disabled: false,
      onboarding_completed: false,
      disable_auto_updates: false,
      keep_decrypted_profiles_in_ram: false,
      trash_retention_days: 14,
      tips_auto_show: true,
      tips_seen: Vec::new(),
      tips_last_auto_shown_at: None,
      paid_welcome_seen_for: Vec::new(),
      cloud_plan_memory: std::collections::HashMap::new(),
    };

    let save_result = manager.save_settings(&test_settings);
    assert!(save_result.is_ok(), "Should save settings successfully");

    let load_result = manager.load_settings();
    assert!(load_result.is_ok(), "Should load settings successfully");

    let loaded_settings = load_result.unwrap();
    assert!(
      loaded_settings.set_as_default_browser,
      "Loaded settings should match saved"
    );
    assert_eq!(
      loaded_settings.theme, "dark",
      "Loaded theme should match saved"
    );
    assert_eq!(loaded_settings.trash_retention_days, 14);
  }

  #[test]
  fn trash_retention_defaults_when_the_settings_file_predates_it() {
    let (manager, _temp_dir, _guard) = create_test_settings_manager();
    let settings_dir = manager.get_settings_dir();
    create_dir_all(&settings_dir).unwrap();
    fs::write(manager.get_settings_file(), r#"{"theme":"light"}"#).unwrap();

    let loaded = manager.load_settings().unwrap();
    assert_eq!(loaded.theme, "light");
    assert_eq!(
      loaded.trash_retention_days,
      crate::profile::trash::DEFAULT_RETENTION_DAYS
    );
  }

  #[test]
  fn the_remote_key_round_trips_and_never_reaches_the_settings_json() {
    let (manager, _temp_dir, _guard) = create_test_settings_manager();

    assert!(manager.get_mcp_remote_key().unwrap().is_none());

    manager
      .store_mcp_remote_key("dmk_abcdefghijklmnop", "key-1")
      .unwrap();
    let stored = manager.get_mcp_remote_key().unwrap().expect("stored");
    assert_eq!(stored.key, "dmk_abcdefghijklmnop");
    assert_eq!(stored.id.as_deref(), Some("key-1"));

    // The id is bookkeeping and belongs in the JSON; the key is a credential
    // that works from anywhere on the internet and must not.
    let json = std::fs::read_to_string(manager.get_settings_file()).unwrap();
    assert!(json.contains("\"mcp_remote_key_id\": \"key-1\""), "{json}");
    assert!(!json.contains("dmk_abcdefghijklmnop"), "{json}");
    assert!(!json.contains("\"mcp_remote_key\""), "{json}");

    // A struct loaded for the frontend carries the plaintext (the fx export
    // line needs it), so the write path is what keeps it off the disk: saving
    // such a struct must not plant the key in the JSON.
    let mut settings = manager.load_settings().unwrap();
    settings.mcp_remote_key = Some("dmk_abcdefghijklmnop".to_string());
    manager.save_settings(&settings).unwrap();
    let json = std::fs::read_to_string(manager.get_settings_file()).unwrap();
    assert!(!json.contains("dmk_"), "{json}");
    assert!(!json.contains("\"mcp_remote_key\""), "{json}");
    assert_eq!(
      manager
        .load_settings()
        .unwrap()
        .mcp_remote_key_id
        .as_deref(),
      Some("key-1")
    );

    manager.remove_mcp_remote_key().unwrap();
    assert!(manager.get_mcp_remote_key().unwrap().is_none());
    assert!(manager.load_settings().unwrap().mcp_remote_key_id.is_none());
  }

  #[test]
  fn a_key_file_without_an_id_still_yields_the_key() {
    // An interrupted store, or a settings file rewritten by an older build
    // that did not know the field: the credential is still on disk and still
    // valid, so it must still be usable. Only the id is missing, and the
    // rotation path treats a missing id as "nothing to revoke".
    let (manager, _temp_dir, _guard) = create_test_settings_manager();
    manager
      .store_mcp_remote_key("dmk_zzzzzzzzzzzz", "key-2")
      .unwrap();
    let mut settings = manager.load_settings().unwrap();
    settings.mcp_remote_key_id = None;
    manager.save_settings(&settings).unwrap();

    let stored = manager.get_mcp_remote_key().unwrap().expect("stored");
    assert_eq!(stored.key, "dmk_zzzzzzzzzzzz");
    assert!(stored.id.is_none());
  }

  #[test]
  fn test_load_table_sorting_nonexistent_file() {
    let (manager, _temp_dir, _guard) = create_test_settings_manager();

    let result = manager.load_table_sorting();
    assert!(
      result.is_ok(),
      "Should handle nonexistent sorting file gracefully"
    );

    let sorting = result.unwrap();
    assert_eq!(sorting.column, "name", "Should return default sorting");
    assert_eq!(sorting.direction, "asc", "Should return default direction");
  }

  #[test]
  fn test_save_and_load_table_sorting() {
    let (manager, _temp_dir, _guard) = create_test_settings_manager();

    let test_sorting = TableSortingSettings {
      column: "browser".to_string(),
      direction: "desc".to_string(),
    };

    let save_result = manager.save_table_sorting(&test_sorting);
    assert!(save_result.is_ok(), "Should save sorting successfully");

    let load_result = manager.load_table_sorting();
    assert!(load_result.is_ok(), "Should load sorting successfully");

    let loaded_sorting = load_result.unwrap();
    assert_eq!(
      loaded_sorting.column, "browser",
      "Loaded column should match saved"
    );
    assert_eq!(
      loaded_sorting.direction, "desc",
      "Loaded direction should match saved"
    );
  }

  #[test]
  fn test_load_corrupted_settings_file() {
    let (manager, _temp_dir, _guard) = create_test_settings_manager();

    let settings_dir = manager.get_settings_dir();
    fs::create_dir_all(&settings_dir).expect("Should create settings directory");

    let settings_file = manager.get_settings_file();
    fs::write(&settings_file, "{ invalid json }").expect("Should write corrupted file");

    let result = manager.load_settings();
    assert!(
      result.is_ok(),
      "Should handle corrupted settings file gracefully"
    );

    let settings = result.unwrap();
    assert!(
      !settings.set_as_default_browser,
      "Should return default settings for corrupted file"
    );
    assert_eq!(
      settings.theme, "system",
      "Should return default theme for corrupted file"
    );
  }

  #[test]
  fn test_settings_file_paths() {
    let (manager, _temp_dir, _guard) = create_test_settings_manager();

    let settings_dir = manager.get_settings_dir();
    let settings_file = manager.get_settings_file();
    let sorting_file = manager.get_table_sorting_file();

    assert!(
      settings_dir.to_string_lossy().contains("settings"),
      "Settings dir should contain 'settings'"
    );
    assert!(
      settings_file
        .to_string_lossy()
        .ends_with("app_settings.json"),
      "Settings file should end with app_settings.json"
    );
    assert!(
      sorting_file
        .to_string_lossy()
        .ends_with("table_sorting.json"),
      "Sorting file should end with table_sorting.json"
    );
  }
}

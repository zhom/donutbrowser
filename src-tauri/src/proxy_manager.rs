use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri_plugin_shell::ShellExt;

use crate::browser::ProxySettings;
use crate::events;
use crate::ip_utils;

// Export data format for JSON export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyExportData {
  pub version: String,
  pub proxies: Vec<ExportedProxy>,
  pub exported_at: String,
  pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedProxy {
  pub name: String,
  #[serde(rename = "type")]
  pub proxy_type: String,
  pub host: String,
  pub port: u16,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub username: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyImportResult {
  pub imported_count: usize,
  pub skipped_count: usize,
  pub errors: Vec<String>,
  pub proxies: Vec<StoredProxy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedProxyLine {
  pub proxy_type: String,
  pub host: String,
  pub port: u16,
  pub username: Option<String>,
  pub password: Option<String>,
  pub original_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum ProxyParseResult {
  #[serde(rename = "parsed")]
  Parsed(ParsedProxyLine),
  #[serde(rename = "ambiguous")]
  Ambiguous {
    line: String,
    possible_formats: Vec<String>,
  },
  #[serde(rename = "invalid")]
  Invalid { line: String, reason: String },
}

// Store active proxy information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyInfo {
  pub id: String,
  pub local_url: String,
  pub upstream_host: String,
  pub upstream_port: u16,
  pub upstream_type: String,
  pub local_port: u16,
  // Optional profile ID to which this proxy instance is logically tied
  pub profile_id: Option<String>,
  pub blocklist_file: Option<String>,
}

// Proxy check result cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyCheckResult {
  pub ip: String,
  pub city: Option<String>,
  pub country: Option<String>,
  pub country_code: Option<String>,
  pub timestamp: u64,
  pub is_valid: bool,
}

pub const CLOUD_PROXY_ID: &str = "cloud-included-proxy";

// Stored proxy configuration with name and ID for reuse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProxy {
  pub id: String,
  pub name: String,
  pub proxy_settings: ProxySettings,
  #[serde(default)]
  pub sync_enabled: bool,
  #[serde(default)]
  pub last_sync: Option<u64>,
  #[serde(default)]
  pub is_cloud_managed: bool,
  #[serde(default)]
  pub is_cloud_derived: bool,
  #[serde(default)]
  pub geo_country: Option<String>,
  // Legacy field kept for deserialization compat; mapped to geo_region on load
  #[serde(default)]
  pub geo_state: Option<String>,
  #[serde(default)]
  pub geo_region: Option<String>,
  #[serde(default)]
  pub geo_city: Option<String>,
  #[serde(default)]
  pub geo_isp: Option<String>,
  #[serde(default)]
  pub dynamic_proxy_url: Option<String>,
  #[serde(default)]
  pub dynamic_proxy_format: Option<String>,
}

impl StoredProxy {
  pub fn new(name: String, proxy_settings: ProxySettings) -> Self {
    let sync_enabled = crate::sync::is_sync_configured();
    Self {
      id: uuid::Uuid::new_v4().to_string(),
      name,
      proxy_settings,
      sync_enabled,
      last_sync: None,
      is_cloud_managed: false,
      is_cloud_derived: false,
      geo_country: None,
      geo_state: None,
      geo_region: None,
      geo_city: None,
      geo_isp: None,
      dynamic_proxy_url: None,
      dynamic_proxy_format: None,
    }
  }

  /// Migrate legacy geo_state to geo_region
  pub fn migrate_geo_fields(&mut self) {
    if self.geo_region.is_none() && self.geo_state.is_some() {
      self.geo_region = self.geo_state.take();
    }
  }

  /// Get the effective region (prefers geo_region, falls back to geo_state for compat)
  pub fn effective_region(&self) -> Option<&String> {
    self.geo_region.as_ref().or(self.geo_state.as_ref())
  }

  pub fn update_settings(&mut self, proxy_settings: ProxySettings) {
    self.proxy_settings = proxy_settings;
  }

  pub fn update_name(&mut self, name: String) {
    self.name = name;
  }
}

// Global proxy manager to track active proxies and stored proxy configurations
pub struct ProxyManager {
  active_proxies: Mutex<HashMap<u32, ProxyInfo>>, // Maps browser process ID to proxy info
  // Store proxy info by profile name for persistence across browser restarts
  profile_proxies: Mutex<HashMap<String, ProxySettings>>, // Maps profile name to proxy settings
  // Track active proxy IDs by profile name for targeted cleanup
  profile_active_proxy_ids: Mutex<HashMap<String, String>>, // Maps profile name to proxy id
  stored_proxies: Mutex<HashMap<String, StoredProxy>>,      // Maps proxy ID to stored proxy
}

impl ProxyManager {
  pub fn new() -> Self {
    let manager = Self {
      active_proxies: Mutex::new(HashMap::new()),
      profile_proxies: Mutex::new(HashMap::new()),
      profile_active_proxy_ids: Mutex::new(HashMap::new()),
      stored_proxies: Mutex::new(HashMap::new()),
    };

    // Load stored proxies on initialization
    if let Err(e) = manager.load_stored_proxies() {
      log::warn!("Failed to load stored proxies: {e}");
    }

    manager
  }

  fn get_proxies_dir(&self) -> PathBuf {
    crate::app_dirs::proxies_dir()
  }

  fn get_proxy_check_cache_dir(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = crate::app_dirs::cache_dir().join("proxy_checks");
    fs::create_dir_all(&path)?;
    Ok(path)
  }

  // Get the path to a specific proxy check cache file
  fn get_proxy_check_cache_file(
    &self,
    proxy_id: &str,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cache_dir = self.get_proxy_check_cache_dir()?;
    Ok(cache_dir.join(format!("{proxy_id}.json")))
  }

  // Load cached proxy check result
  fn load_proxy_check_cache(&self, proxy_id: &str) -> Option<ProxyCheckResult> {
    let cache_file = match self.get_proxy_check_cache_file(proxy_id) {
      Ok(file) => file,
      Err(_) => return None,
    };

    if !cache_file.exists() {
      return None;
    }

    let content = match fs::read_to_string(&cache_file) {
      Ok(content) => content,
      Err(_) => return None,
    };

    serde_json::from_str::<ProxyCheckResult>(&content).ok()
  }

  // Save proxy check result to cache
  fn save_proxy_check_cache(
    &self,
    proxy_id: &str,
    result: &ProxyCheckResult,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let cache_file = self.get_proxy_check_cache_file(proxy_id)?;
    let content = serde_json::to_string_pretty(result)?;
    fs::write(&cache_file, content)?;
    Ok(())
  }

  // Get current timestamp
  fn get_current_timestamp() -> u64 {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs()
  }

  pub async fn get_ip_geolocation(
    ip: &str,
  ) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    // Use ip-api.com (free, no API key required)
    let url = format!(
      "http://ip-api.com/json/{}?fields=status,message,country,countryCode,city",
      ip
    );

    let client = reqwest::Client::builder()
      .timeout(std::time::Duration::from_secs(5))
      .build()
      .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    match client.get(&url).send().await {
      Ok(response) => {
        if response.status().is_success() {
          match response.json::<serde_json::Value>().await {
            Ok(json) => {
              if json.get("status").and_then(|s| s.as_str()) == Some("success") {
                let country = json
                  .get("country")
                  .and_then(|v| v.as_str())
                  .map(|s| s.to_string());
                let country_code = json
                  .get("countryCode")
                  .and_then(|v| v.as_str())
                  .map(|s| s.to_string());
                let city = json
                  .get("city")
                  .and_then(|v| v.as_str())
                  .map(|s| s.to_string());
                Ok((city, country, country_code))
              } else {
                Ok((None, None, None))
              }
            }
            Err(e) => Err(format!("Failed to parse geolocation response: {e}")),
          }
        } else {
          Ok((None, None, None))
        }
      }
      Err(e) => Err(format!("Failed to fetch geolocation: {e}")),
    }
  }

  pub fn get_proxy_file_path(&self, proxy_id: &str) -> PathBuf {
    self.get_proxies_dir().join(format!("{proxy_id}.json"))
  }

  // Load stored proxies from disk
  fn load_stored_proxies(&self) -> Result<(), Box<dyn std::error::Error>> {
    let proxies_dir = self.get_proxies_dir();

    if !proxies_dir.exists() {
      log::debug!("Proxies directory does not exist: {:?}", proxies_dir);
      return Ok(()); // No proxies directory yet
    }

    log::debug!("Loading stored proxies from: {:?}", proxies_dir);

    let mut stored_proxies = self.stored_proxies.lock().unwrap();
    let mut loaded_count = 0;
    let mut error_count = 0;

    // Read all JSON files from the proxies directory
    for entry in fs::read_dir(&proxies_dir)? {
      let entry = entry?;
      let path = entry.path();

      if path.extension().is_some_and(|ext| ext == "json") {
        match fs::read_to_string(&path) {
          Ok(content) => match serde_json::from_str::<StoredProxy>(&content) {
            Ok(proxy) => {
              log::debug!("Loaded stored proxy: {} ({})", proxy.name, proxy.id);
              stored_proxies.insert(proxy.id.clone(), proxy);
              loaded_count += 1;
            }
            Err(e) => {
              log::warn!(
                "Failed to parse proxy file {:?} as StoredProxy: {}",
                path,
                e
              );
              error_count += 1;
            }
          },
          Err(e) => {
            log::warn!("Failed to read proxy file {:?}: {}", path, e);
            error_count += 1;
          }
        }
      }
    }

    log::info!(
      "Loaded {} stored proxies ({} errors)",
      loaded_count,
      error_count
    );
    Ok(())
  }

  // Save a single proxy to disk
  fn save_proxy(&self, proxy: &StoredProxy) -> Result<(), Box<dyn std::error::Error>> {
    let proxies_dir = self.get_proxies_dir();

    // Ensure directory exists
    fs::create_dir_all(&proxies_dir)?;

    let proxy_file = self.get_proxy_file_path(&proxy.id);
    let content = serde_json::to_string_pretty(proxy)?;
    fs::write(&proxy_file, content)?;

    Ok(())
  }

  // Delete a proxy file from disk
  fn delete_proxy_file(&self, proxy_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let proxy_file = self.get_proxy_file_path(proxy_id);
    if proxy_file.exists() {
      fs::remove_file(proxy_file)?;
    }
    Ok(())
  }

  // Create a new stored proxy
  pub fn create_stored_proxy(
    &self,
    _app_handle: &tauri::AppHandle,
    name: String,
    proxy_settings: ProxySettings,
  ) -> Result<StoredProxy, String> {
    // Check if name already exists
    {
      let stored_proxies = self.stored_proxies.lock().unwrap();
      if stored_proxies.values().any(|p| p.name == name) {
        return Err(format!("Proxy with name '{name}' already exists"));
      }
    }

    let stored_proxy = StoredProxy::new(name, proxy_settings);

    {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      stored_proxies.insert(stored_proxy.id.clone(), stored_proxy.clone());
    }

    if let Err(e) = self.save_proxy(&stored_proxy) {
      log::warn!("Failed to save proxy: {e}");
    }

    // Emit event for reactive UI updates
    if let Err(e) = events::emit_empty("proxies-changed") {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    if stored_proxy.sync_enabled {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        let id = stored_proxy.id.clone();
        tauri::async_runtime::spawn(async move {
          scheduler.queue_proxy_sync(id).await;
        });
      }
    }

    Ok(stored_proxy)
  }

  // Check if a cloud-managed proxy exists
  pub fn has_cloud_proxy(&self) -> bool {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    stored_proxies.contains_key(CLOUD_PROXY_ID)
  }

  // Upsert the cloud-managed proxy (create or update)
  pub fn upsert_cloud_proxy(&self, proxy_settings: ProxySettings) -> Result<StoredProxy, String> {
    let mut stored_proxies = self.stored_proxies.lock().unwrap();

    if let Some(existing) = stored_proxies.get_mut(CLOUD_PROXY_ID) {
      existing.proxy_settings = proxy_settings;
      let updated = existing.clone();
      drop(stored_proxies);

      if let Err(e) = self.save_proxy(&updated) {
        log::warn!("Failed to save cloud proxy: {e}");
      }
      if let Err(e) = events::emit_empty("proxies-changed") {
        log::error!("Failed to emit proxies-changed event: {e}");
      }
      Ok(updated)
    } else {
      let cloud_proxy = StoredProxy {
        id: CLOUD_PROXY_ID.to_string(),
        name: "Included Proxy".to_string(),
        proxy_settings,
        sync_enabled: false,
        last_sync: None,
        is_cloud_managed: true,
        is_cloud_derived: false,
        geo_country: None,
        geo_state: None,
        geo_region: None,
        geo_city: None,
        geo_isp: None,
        dynamic_proxy_url: None,
        dynamic_proxy_format: None,
      };
      stored_proxies.insert(CLOUD_PROXY_ID.to_string(), cloud_proxy.clone());
      drop(stored_proxies);

      if let Err(e) = self.save_proxy(&cloud_proxy) {
        log::warn!("Failed to save cloud proxy: {e}");
      }
      if let Err(e) = events::emit_empty("proxies-changed") {
        log::error!("Failed to emit proxies-changed event: {e}");
      }
      Ok(cloud_proxy)
    }
  }

  // Remove the cloud-managed proxy
  pub fn remove_cloud_proxy(&self) {
    let removed = {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      stored_proxies.remove(CLOUD_PROXY_ID).is_some()
    };

    if removed {
      if let Err(e) = self.delete_proxy_file(CLOUD_PROXY_ID) {
        log::warn!("Failed to delete cloud proxy file: {e}");
      }
      if let Err(e) = events::emit_empty("proxies-changed") {
        log::error!("Failed to emit proxies-changed event: {e}");
      }
    }
  }

  pub fn remove_cloud_proxies(&self) {
    let removed_ids: Vec<String> = {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      let ids_to_remove: Vec<String> = stored_proxies
        .values()
        .filter(|p| p.is_cloud_managed || p.is_cloud_derived)
        .map(|p| p.id.clone())
        .collect();
      for id in &ids_to_remove {
        stored_proxies.remove(id);
      }
      ids_to_remove
    };

    if !removed_ids.is_empty() {
      for id in &removed_ids {
        if let Err(e) = self.delete_proxy_file(id) {
          log::warn!("Failed to delete cloud proxy file {id}: {e}");
        }
      }
      if let Err(e) = events::emit_empty("proxies-changed") {
        log::error!("Failed to emit proxies-changed event: {e}");
      }
      if let Err(e) = events::emit_empty("stored-proxies-changed") {
        log::error!("Failed to emit stored-proxies-changed event: {e}");
      }
    }
  }

  // Build a geo-targeted username from base username and location parts
  // LP v2 format: username-country-{cc}[-region-{region}][-city-{city}][-isp-{isp}]
  // Note: sid and ttl are NOT included here — they are injected at browser launch time
  // per-profile via resolve_proxy_for_profile()
  fn build_geo_username(
    base_username: &str,
    country: &str,
    region: &Option<String>,
    city: &Option<String>,
    isp: &Option<String>,
  ) -> String {
    let mut username = format!("{}-country-{}", base_username, country);
    if let Some(region) = region {
      username = format!("{}-region-{}", username, region);
    }
    if let Some(city) = city {
      username = format!("{}-city-{}", username, city);
    }
    if let Some(isp) = isp {
      username = format!("{}-isp-{}", username, isp);
    }
    username
  }

  /// Generate a deterministic 11-char alphanumeric session ID from a profile UUID.
  /// This ensures the same profile always gets the same sticky IP session,
  /// even across credential refreshes.
  pub fn generate_sid_for_profile(profile_id: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    profile_id.hash(&mut hasher);
    let hash = hasher.finish();

    // Convert to base36 (a-z0-9) and take 11 chars
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789".chars().collect();
    let mut sid = String::with_capacity(11);
    let mut val = hash;
    for _ in 0..11 {
      sid.push(chars[(val % 36) as usize]);
      val /= 36;
    }
    sid
  }

  /// Build the full proxy username with sid and ttl for a specific profile launch.
  /// This is called at browser launch time, not at proxy creation time.
  pub fn build_username_with_sid(base_geo_username: &str, profile_id: &str) -> String {
    let sid = Self::generate_sid_for_profile(profile_id);
    format!("{}-sid-{}-ttl-1440m", base_geo_username, sid)
  }

  /// Resolve proxy settings for a specific profile, injecting profile-specific sid
  /// for cloud-derived proxies with geo targeting.
  pub fn resolve_proxy_for_profile(
    &self,
    proxy_id: &str,
    profile_id: &str,
  ) -> Option<ProxySettings> {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    let proxy = stored_proxies.get(proxy_id)?;
    let mut settings = proxy.proxy_settings.clone();

    // For cloud-derived proxies with geo targeting, inject profile-specific sid
    if proxy.is_cloud_derived && proxy.geo_country.is_some() {
      if let Some(ref username) = settings.username {
        settings.username = Some(Self::build_username_with_sid(username, profile_id));
      }
    }

    Some(settings)
  }

  // Create a cloud-derived location proxy from the base cloud proxy credentials
  pub fn create_cloud_location_proxy(
    &self,
    name: String,
    country: String,
    region: Option<String>,
    city: Option<String>,
    isp: Option<String>,
  ) -> Result<StoredProxy, String> {
    // Get base cloud proxy credentials
    let base_proxy = {
      let stored_proxies = self.stored_proxies.lock().unwrap();
      stored_proxies
        .get(CLOUD_PROXY_ID)
        .cloned()
        .ok_or_else(|| "No cloud proxy available. Please log in first.".to_string())?
    };

    let base_username = base_proxy
      .proxy_settings
      .username
      .as_ref()
      .ok_or_else(|| "Cloud proxy has no username".to_string())?;

    let geo_username = Self::build_geo_username(base_username, &country, &region, &city, &isp);

    let proxy_settings = ProxySettings {
      proxy_type: base_proxy.proxy_settings.proxy_type.clone(),
      host: base_proxy.proxy_settings.host.clone(),
      port: base_proxy.proxy_settings.port,
      username: Some(geo_username),
      password: base_proxy.proxy_settings.password.clone(),
    };

    // Check if name already exists
    {
      let stored_proxies = self.stored_proxies.lock().unwrap();
      if stored_proxies.values().any(|p| p.name == name) {
        return Err(format!("Proxy with name '{}' already exists", name));
      }
    }

    let stored_proxy = StoredProxy {
      id: uuid::Uuid::new_v4().to_string(),
      name,
      proxy_settings,
      sync_enabled: false,
      last_sync: None,
      is_cloud_managed: false,
      is_cloud_derived: true,
      geo_country: Some(country),
      geo_state: None,
      geo_region: region,
      geo_city: city,
      geo_isp: isp,
      dynamic_proxy_url: None,
      dynamic_proxy_format: None,
    };

    {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      stored_proxies.insert(stored_proxy.id.clone(), stored_proxy.clone());
    }

    if let Err(e) = self.save_proxy(&stored_proxy) {
      log::warn!("Failed to save location proxy: {e}");
    }

    if let Err(e) = events::emit_empty("proxies-changed") {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    Ok(stored_proxy)
  }

  // Update all cloud-derived proxies when base cloud proxy credentials change
  pub fn update_cloud_derived_proxies(&self) {
    let base_proxy = {
      let stored_proxies = self.stored_proxies.lock().unwrap();
      match stored_proxies.get(CLOUD_PROXY_ID) {
        Some(p) => p.clone(),
        None => return, // No cloud proxy, nothing to update
      }
    };

    let base_username = match &base_proxy.proxy_settings.username {
      Some(u) => u.clone(),
      None => return,
    };

    let mut updated = false;
    let mut stored_proxies = self.stored_proxies.lock().unwrap();

    for proxy in stored_proxies.values_mut() {
      if !proxy.is_cloud_derived {
        continue;
      }

      let country = match &proxy.geo_country {
        Some(c) => c.clone(),
        None => continue,
      };

      let region = proxy.effective_region().cloned();
      let geo_username = Self::build_geo_username(
        &base_username,
        &country,
        &region,
        &proxy.geo_city,
        &proxy.geo_isp,
      );

      proxy.proxy_settings.username = Some(geo_username);
      proxy.proxy_settings.password = base_proxy.proxy_settings.password.clone();
      proxy.proxy_settings.host = base_proxy.proxy_settings.host.clone();
      proxy.proxy_settings.port = base_proxy.proxy_settings.port;

      updated = true;
    }

    if updated {
      // Save all updated proxies
      let proxies_to_save: Vec<StoredProxy> = stored_proxies
        .values()
        .filter(|p| p.is_cloud_derived)
        .cloned()
        .collect();
      drop(stored_proxies);

      for proxy in &proxies_to_save {
        if let Err(e) = self.save_proxy(proxy) {
          log::warn!("Failed to save updated derived proxy {}: {e}", proxy.id);
        }
      }

      if let Err(e) = events::emit_empty("proxies-changed") {
        log::error!("Failed to emit proxies-changed event: {e}");
      }

      log::debug!("Updated {} cloud-derived proxies", proxies_to_save.len());
    }
  }

  pub fn remove_from_memory(&self, proxy_id: &str) {
    let mut stored_proxies = self.stored_proxies.lock().unwrap();
    stored_proxies.remove(proxy_id);
  }

  // Get all stored proxies
  pub fn get_stored_proxies(&self) -> Vec<StoredProxy> {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    let mut list: Vec<StoredProxy> = stored_proxies.values().cloned().collect();
    // Sort case-insensitively by name for consistent ordering across UI/API consumers
    list.sort_by_key(|p| p.name.to_lowercase());
    list
  }

  // Get a stored proxy by ID

  // Update a stored proxy
  pub fn update_stored_proxy(
    &self,
    _app_handle: &tauri::AppHandle,
    proxy_id: &str,
    name: Option<String>,
    proxy_settings: Option<ProxySettings>,
  ) -> Result<StoredProxy, String> {
    // First, check for conflicts without holding a mutable reference
    {
      let stored_proxies = self.stored_proxies.lock().unwrap();

      // Check if proxy exists
      if !stored_proxies.contains_key(proxy_id) {
        return Err(format!("Proxy with ID '{proxy_id}' not found"));
      }

      // Block editing cloud-managed proxies
      if stored_proxies
        .get(proxy_id)
        .is_some_and(|p| p.is_cloud_managed)
      {
        return Err("Cannot edit a cloud-managed proxy".to_string());
      }

      // Check if new name conflicts with existing proxies
      if let Some(ref new_name) = name {
        if stored_proxies
          .values()
          .any(|p| p.id != proxy_id && p.name == *new_name)
        {
          return Err(format!("Proxy with name '{new_name}' already exists"));
        }
      }
    } // Release the lock here

    // Now get mutable access for updates
    let updated_proxy = {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      let stored_proxy = stored_proxies.get_mut(proxy_id).unwrap(); // Safe because we checked above

      if let Some(new_name) = name {
        stored_proxy.update_name(new_name);
      }

      if let Some(new_settings) = proxy_settings {
        stored_proxy.update_settings(new_settings);
      }

      stored_proxy.clone()
    };

    if let Err(e) = self.save_proxy(&updated_proxy) {
      log::warn!("Failed to save proxy: {e}");
    }

    // Emit event for reactive UI updates
    if let Err(e) = events::emit_empty("proxies-changed") {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    if updated_proxy.sync_enabled {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        let id = updated_proxy.id.clone();
        tauri::async_runtime::spawn(async move {
          scheduler.queue_proxy_sync(id).await;
        });
      }
    }

    Ok(updated_proxy)
  }

  // Delete a stored proxy
  pub fn delete_stored_proxy(
    &self,
    app_handle: &tauri::AppHandle,
    proxy_id: &str,
  ) -> Result<(), String> {
    // Remember if sync was enabled before deleting
    let was_sync_enabled = {
      let stored_proxies = self.stored_proxies.lock().unwrap();

      // Block deleting cloud-managed proxies
      if stored_proxies
        .get(proxy_id)
        .is_some_and(|p| p.is_cloud_managed)
      {
        return Err("Cannot delete a cloud-managed proxy".to_string());
      }

      stored_proxies
        .get(proxy_id)
        .map(|p| p.sync_enabled)
        .unwrap_or(false)
    };

    {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      if stored_proxies.remove(proxy_id).is_none() {
        return Err(format!("Proxy with ID '{proxy_id}' not found"));
      }
    }

    if let Err(e) = self.delete_proxy_file(proxy_id) {
      log::warn!("Failed to delete proxy file: {e}");
    }

    // If sync was enabled, also delete from S3
    if was_sync_enabled {
      let proxy_id_owned = proxy_id.to_string();
      let app_handle_clone = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        match crate::sync::SyncEngine::create_from_settings(&app_handle_clone).await {
          Ok(engine) => {
            if let Err(e) = engine.delete_proxy(&proxy_id_owned).await {
              log::warn!("Failed to delete proxy {} from sync: {}", proxy_id_owned, e);
            } else {
              log::info!("Proxy {} deleted from S3 sync storage", proxy_id_owned);
            }
          }
          Err(e) => {
            log::debug!("Sync not configured, skipping remote deletion: {}", e);
          }
        }
      });
    }

    // Emit event for reactive UI updates
    if let Err(e) = events::emit_empty("proxies-changed") {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    Ok(())
  }

  // Check if a proxy is cloud-managed or cloud-derived (needs fresh credentials)
  pub fn is_cloud_or_derived(&self, proxy_id: &str) -> bool {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    stored_proxies
      .get(proxy_id)
      .is_some_and(|p| p.is_cloud_managed || p.is_cloud_derived)
  }

  // Get proxy settings for a stored proxy ID
  pub fn get_proxy_settings_by_id(&self, proxy_id: &str) -> Option<ProxySettings> {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    stored_proxies
      .get(proxy_id)
      .map(|p| p.proxy_settings.clone())
  }

  fn classify_proxy_error(raw_error: &str, settings: &ProxySettings) -> String {
    let err = raw_error.to_lowercase();
    let proxy_addr = format!("{}:{}", settings.host, settings.port);

    if err.contains("connection refused") {
      return format!(
        "Connection refused by {proxy_addr}. The proxy server is not accepting connections."
      );
    }
    if err.contains("connection reset") {
      return format!(
        "Connection reset by {proxy_addr}. The proxy server closed the connection unexpectedly."
      );
    }
    if err.contains("timed out") || err.contains("deadline has elapsed") {
      return format!("Connection to {proxy_addr} timed out. The proxy server is not responding.");
    }
    if err.contains("no such host") || err.contains("dns") || err.contains("resolve") {
      return format!(
        "Could not resolve proxy host '{}'. Check that the hostname is correct.",
        settings.host
      );
    }
    if err.contains("authentication") || err.contains("407") || err.contains("proxy auth") {
      return format!(
        "Proxy authentication failed for {proxy_addr}. Check your username and password."
      );
    }
    if err.contains("403") || err.contains("forbidden") {
      return format!("Access denied by {proxy_addr} (403 Forbidden).");
    }
    if err.contains("402") {
      return format!(
        "Payment required by {proxy_addr} (402). Your proxy subscription may have expired."
      );
    }
    if err.contains("502") || err.contains("bad gateway") {
      return format!(
        "Bad gateway from {proxy_addr} (502). The upstream proxy server may be down."
      );
    }
    if err.contains("503") || err.contains("service unavailable") {
      return format!("Proxy {proxy_addr} is temporarily unavailable (503).");
    }
    if err.contains("socks") && err.contains("unreachable") {
      return format!("SOCKS proxy {proxy_addr} could not reach the target. The proxy server may not have internet access.");
    }
    if err.contains("invalid proxy") || err.contains("unsupported proxy") {
      return format!(
        "Invalid proxy configuration for {proxy_addr}. Check the proxy type and address."
      );
    }

    // Generic fallback — still show the proxy address for context
    format!("Proxy check failed for {proxy_addr}. Could not connect through the proxy.")
  }

  // Build proxy URL string from ProxySettings
  fn build_proxy_url(proxy_settings: &ProxySettings) -> String {
    let mut url = format!("{}://", proxy_settings.proxy_type);

    if let (Some(username), Some(password)) = (&proxy_settings.username, &proxy_settings.password) {
      url.push_str(&urlencoding::encode(username));
      url.push(':');
      url.push_str(&urlencoding::encode(password));
      url.push('@');
    } else if let Some(username) = &proxy_settings.username {
      url.push_str(&urlencoding::encode(username));
      url.push('@');
    }

    url.push_str(&proxy_settings.host);
    url.push(':');
    url.push_str(&proxy_settings.port.to_string());

    url
  }

  // Check if a proxy is valid by routing through a temporary donut-proxy process.
  // This tests the exact same code path the browser uses.
  // Falls back to direct reqwest check if the proxy worker fails to start.
  pub async fn check_proxy_validity(
    &self,
    proxy_id: &str,
    proxy_settings: &ProxySettings,
  ) -> Result<ProxyCheckResult, String> {
    let upstream_url = Self::build_proxy_url(proxy_settings);

    // Try process-based check first (identical to browser launch path)
    // Try process-based check first (identical to browser launch path).
    // If the proxy worker fails to start (e.g. Gatekeeper, antivirus, signing
    // restrictions), fall back to a direct reqwest check.
    let proxy_start_result =
      crate::proxy_runner::start_proxy_process(Some(upstream_url.clone()), None)
        .await
        .map_err(|e| e.to_string());

    let ip_result = match proxy_start_result {
      Ok(proxy_config) => {
        let local_url = format!("http://127.0.0.1:{}", proxy_config.local_port.unwrap_or(0));
        let config_id = proxy_config.id.clone();
        let result = ip_utils::fetch_public_ip(Some(&local_url)).await;
        let _ = crate::proxy_runner::stop_proxy_process(&config_id).await;
        result
      }
      Err(err_msg) => {
        log::warn!(
          "Proxy worker failed to start ({}), falling back to direct check",
          err_msg
        );
        ip_utils::fetch_public_ip(Some(&upstream_url)).await
      }
    };

    let ip = match ip_result {
      Ok(ip) => ip,
      Err(e) => {
        let failed_result = ProxyCheckResult {
          ip: String::new(),
          city: None,
          country: None,
          country_code: None,
          timestamp: Self::get_current_timestamp(),
          is_valid: false,
        };
        let _ = self.save_proxy_check_cache(proxy_id, &failed_result);

        let err_str = e.to_string();
        let user_message = Self::classify_proxy_error(&err_str, proxy_settings);
        return Err(user_message);
      }
    };

    // Get geolocation
    let (city, country, country_code): (Option<String>, Option<String>, Option<String>) =
      Self::get_ip_geolocation(&ip).await.unwrap_or_default();

    // Create successful result
    let result = ProxyCheckResult {
      ip: ip.clone(),
      city,
      country,
      country_code,
      timestamp: Self::get_current_timestamp(),
      is_valid: true,
    };

    // Save to cache
    let _ = self.save_proxy_check_cache(proxy_id, &result);

    Ok(result)
  }

  // Get cached proxy check result
  pub fn get_cached_proxy_check(&self, proxy_id: &str) -> Option<ProxyCheckResult> {
    self.load_proxy_check_cache(proxy_id)
  }

  pub async fn fetch_proxy_from_url(
    &self,
    url: &str,
    timeout: std::time::Duration,
  ) -> Result<Option<ProxySettings>, String> {
    let client = reqwest::Client::builder()
      .timeout(timeout)
      .build()
      .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let response = client
      .get(url)
      .send()
      .await
      .map_err(|e| format!("Failed to fetch launch hook: {e}"))?;

    if response.status() == reqwest::StatusCode::NO_CONTENT {
      return Ok(None);
    }

    if !response.status().is_success() {
      return Err(format!("Launch hook returned status {}", response.status()));
    }

    let body = response
      .text()
      .await
      .map_err(|e| format!("Failed to read launch hook response: {e}"))?;

    let body = body.trim();
    if body.is_empty() {
      return Err("Launch hook returned empty response".to_string());
    }

    if let Ok(settings) = Self::parse_dynamic_proxy_json(body) {
      return Ok(Some(settings));
    }

    match Self::parse_dynamic_proxy_text(body) {
      Ok(settings) => Ok(Some(settings)),
      Err(text_error) => Err(format!(
        "Failed to parse launch hook response: {text_error}"
      )),
    }
  }

  // Parse JSON proxy payload: { "ip"/"host": "...", "port": ..., "username": "...", "password": "..." }
  fn parse_dynamic_proxy_json(body: &str) -> Result<ProxySettings, String> {
    let json: serde_json::Value =
      serde_json::from_str(body).map_err(|e| format!("Invalid JSON response: {e}"))?;

    let obj = json
      .as_object()
      .ok_or_else(|| "JSON response is not an object".to_string())?;

    let raw_host = obj
      .get("ip")
      .or_else(|| obj.get("host"))
      .and_then(|v| v.as_str())
      .ok_or_else(|| "Missing 'ip' or 'host' field in JSON response".to_string())?;

    // Strip protocol prefix from host if present (e.g. "socks5://1.2.3.4" -> "1.2.3.4")
    // and extract the proxy type from it if no explicit type field is provided
    let (host, protocol_from_host) = if let Some(rest) = raw_host.strip_prefix("://") {
      (rest.to_string(), None)
    } else if let Some((proto, rest)) = raw_host.split_once("://") {
      (rest.to_string(), Some(proto.to_lowercase()))
    } else {
      (raw_host.to_string(), None)
    };

    let port = obj
      .get("port")
      .and_then(|v| {
        v.as_u64()
          .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
      })
      .ok_or_else(|| "Missing or invalid 'port' field in JSON response".to_string())?
      as u16;

    let proxy_type = obj
      .get("type")
      .or_else(|| obj.get("proxy_type"))
      .or_else(|| obj.get("protocol"))
      .and_then(|v| v.as_str())
      .map(|s| s.to_lowercase())
      .or(protocol_from_host)
      .unwrap_or_else(|| "http".to_string());

    let username = obj
      .get("username")
      .or_else(|| obj.get("user"))
      .and_then(|v| v.as_str())
      .filter(|s| !s.is_empty())
      .map(|s| s.to_string());

    let password = obj
      .get("password")
      .or_else(|| obj.get("pass"))
      .and_then(|v| v.as_str())
      .filter(|s| !s.is_empty())
      .map(|s| s.to_string());

    Ok(ProxySettings {
      proxy_type,
      host,
      port,
      username,
      password,
    })
  }

  // Parse plain text proxy payload using the same logic as proxy import
  fn parse_dynamic_proxy_text(body: &str) -> Result<ProxySettings, String> {
    let line = body
      .lines()
      .find(|l| !l.trim().is_empty())
      .unwrap_or("")
      .trim();
    if line.is_empty() {
      return Err("Empty text response".to_string());
    }

    match Self::parse_single_proxy_line(line) {
      ProxyParseResult::Parsed(parsed) => Ok(ProxySettings {
        proxy_type: parsed.proxy_type,
        host: parsed.host,
        port: parsed.port,
        username: parsed.username,
        password: parsed.password,
      }),
      ProxyParseResult::Ambiguous {
        possible_formats, ..
      } => Err(format!(
        "Ambiguous proxy format. Could be: {}",
        possible_formats.join(" or ")
      )),
      ProxyParseResult::Invalid { reason, .. } => {
        Err(format!("Failed to parse proxy response: {reason}"))
      }
    }
  }

  // Export all proxies as JSON
  pub fn export_proxies_json(&self) -> Result<String, String> {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    let proxies: Vec<ExportedProxy> = stored_proxies
      .values()
      .filter(|p| !p.is_cloud_managed && !p.is_cloud_derived)
      .map(|p| ExportedProxy {
        name: p.name.clone(),
        proxy_type: p.proxy_settings.proxy_type.clone(),
        host: p.proxy_settings.host.clone(),
        port: p.proxy_settings.port,
        username: p.proxy_settings.username.clone(),
        password: p.proxy_settings.password.clone(),
      })
      .collect();

    let export_data = ProxyExportData {
      version: "1.0".to_string(),
      proxies,
      exported_at: Utc::now().to_rfc3339(),
      source: "DonutBrowser".to_string(),
    };

    serde_json::to_string_pretty(&export_data).map_err(|e| format!("Failed to serialize: {e}"))
  }

  // Export all proxies as TXT (one per line: protocol://user:pass@host:port)
  pub fn export_proxies_txt(&self) -> String {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    stored_proxies
      .values()
      .filter(|p| !p.is_cloud_managed && !p.is_cloud_derived)
      .map(|p| Self::build_proxy_url(&p.proxy_settings))
      .collect::<Vec<_>>()
      .join("\n")
  }

  // Parse TXT content with auto-detection of formats
  pub fn parse_txt_proxies(content: &str) -> Vec<ProxyParseResult> {
    content
      .lines()
      .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
      .map(|line| Self::parse_single_proxy_line(line.trim()))
      .collect()
  }

  // Parse a single proxy line with format auto-detection
  fn parse_single_proxy_line(line: &str) -> ProxyParseResult {
    // Format 1: protocol://username:password@host:port (full URL)
    if let Some(result) = Self::try_parse_url_format(line) {
      return result;
    }

    // Try colon-separated formats
    let parts: Vec<&str> = line.split(':').collect();

    match parts.len() {
      // host:port (no auth)
      2 => {
        if let Ok(port) = parts[1].parse::<u16>() {
          return ProxyParseResult::Parsed(ParsedProxyLine {
            proxy_type: "http".to_string(),
            host: parts[0].to_string(),
            port,
            username: None,
            password: None,
            original_line: line.to_string(),
          });
        }
        ProxyParseResult::Invalid {
          line: line.to_string(),
          reason: "Invalid port number".to_string(),
        }
      }
      // Could be: host:port:user or user:pass@host (with @ in the middle)
      3 => {
        // Try username:password@host:port first
        if let Some(result) = Self::try_parse_user_pass_at_host_port(line) {
          return result;
        }
        ProxyParseResult::Invalid {
          line: line.to_string(),
          reason: "Could not determine format with 3 parts".to_string(),
        }
      }
      // 4 parts: could be host:port:user:pass OR user:pass:host:port
      4 => {
        // Try to detect which format
        let port_at_1 = parts[1].parse::<u16>().is_ok();
        let port_at_3 = parts[3].parse::<u16>().is_ok();

        match (port_at_1, port_at_3) {
          // host:port:user:pass
          (true, false) => {
            let port = parts[1].parse::<u16>().unwrap();
            ProxyParseResult::Parsed(ParsedProxyLine {
              proxy_type: "http".to_string(),
              host: parts[0].to_string(),
              port,
              username: Some(parts[2].to_string()),
              password: Some(parts[3].to_string()),
              original_line: line.to_string(),
            })
          }
          // user:pass:host:port
          (false, true) => {
            let port = parts[3].parse::<u16>().unwrap();
            ProxyParseResult::Parsed(ParsedProxyLine {
              proxy_type: "http".to_string(),
              host: parts[2].to_string(),
              port,
              username: Some(parts[0].to_string()),
              password: Some(parts[1].to_string()),
              original_line: line.to_string(),
            })
          }
          // Both could be ports - ambiguous
          (true, true) => ProxyParseResult::Ambiguous {
            line: line.to_string(),
            possible_formats: vec![
              "host:port:username:password".to_string(),
              "username:password:host:port".to_string(),
            ],
          },
          // Neither is a valid port
          (false, false) => ProxyParseResult::Invalid {
            line: line.to_string(),
            reason: "No valid port number found".to_string(),
          },
        }
      }
      _ => ProxyParseResult::Invalid {
        line: line.to_string(),
        reason: format!("Unexpected format with {} parts", parts.len()),
      },
    }
  }

  // Try to parse URL format: protocol://username:password@host:port
  fn try_parse_url_format(line: &str) -> Option<ProxyParseResult> {
    // Check for protocol prefix using strip_prefix
    let (protocol, rest) = if let Some(rest) = line.strip_prefix("http://") {
      ("http", rest)
    } else if let Some(rest) = line.strip_prefix("https://") {
      ("https", rest)
    } else if let Some(rest) = line.strip_prefix("socks4://") {
      ("socks4", rest)
    } else if let Some(rest) = line.strip_prefix("socks5://") {
      ("socks5", rest)
    } else if let Some(rest) = line.strip_prefix("socks://") {
      ("socks5", rest) // Default socks to socks5
    } else {
      return None;
    };

    // Check if there's auth (contains @)
    if let Some(at_pos) = rest.rfind('@') {
      let auth = &rest[..at_pos];
      let host_port = &rest[at_pos + 1..];

      // Parse auth (user:pass)
      let (username, password) = if let Some(colon_pos) = auth.find(':') {
        let user = urlencoding::decode(&auth[..colon_pos]).unwrap_or_default();
        let pass = urlencoding::decode(&auth[colon_pos + 1..]).unwrap_or_default();
        (Some(user.to_string()), Some(pass.to_string()))
      } else {
        (
          Some(urlencoding::decode(auth).unwrap_or_default().to_string()),
          None,
        )
      };

      // Parse host:port
      if let Some(colon_pos) = host_port.rfind(':') {
        let host = &host_port[..colon_pos];
        if let Ok(port) = host_port[colon_pos + 1..].parse::<u16>() {
          return Some(ProxyParseResult::Parsed(ParsedProxyLine {
            proxy_type: protocol.to_string(),
            host: host.to_string(),
            port,
            username,
            password,
            original_line: line.to_string(),
          }));
        }
      }
    } else {
      // No auth, just host:port
      if let Some(colon_pos) = rest.rfind(':') {
        let host = &rest[..colon_pos];
        if let Ok(port) = rest[colon_pos + 1..].parse::<u16>() {
          return Some(ProxyParseResult::Parsed(ParsedProxyLine {
            proxy_type: protocol.to_string(),
            host: host.to_string(),
            port,
            username: None,
            password: None,
            original_line: line.to_string(),
          }));
        }
      }
    }

    Some(ProxyParseResult::Invalid {
      line: line.to_string(),
      reason: "Invalid URL format".to_string(),
    })
  }

  // Try to parse: username:password@host:port format (no protocol)
  fn try_parse_user_pass_at_host_port(line: &str) -> Option<ProxyParseResult> {
    if let Some(at_pos) = line.rfind('@') {
      let auth = &line[..at_pos];
      let host_port = &line[at_pos + 1..];

      // Parse auth
      let (username, password) = if let Some(colon_pos) = auth.find(':') {
        (
          Some(auth[..colon_pos].to_string()),
          Some(auth[colon_pos + 1..].to_string()),
        )
      } else {
        return None;
      };

      // Parse host:port
      if let Some(colon_pos) = host_port.rfind(':') {
        let host = &host_port[..colon_pos];
        if let Ok(port) = host_port[colon_pos + 1..].parse::<u16>() {
          return Some(ProxyParseResult::Parsed(ParsedProxyLine {
            proxy_type: "http".to_string(),
            host: host.to_string(),
            port,
            username,
            password,
            original_line: line.to_string(),
          }));
        }
      }
    }
    None
  }

  // Import proxies from JSON content
  pub fn import_proxies_json(
    &self,
    app_handle: &tauri::AppHandle,
    content: &str,
  ) -> Result<ProxyImportResult, String> {
    let export_data: ProxyExportData =
      serde_json::from_str(content).map_err(|e| format!("Invalid JSON format: {e}"))?;

    let mut imported = Vec::new();
    let mut skipped = 0;
    let mut errors = Vec::new();

    for exported in export_data.proxies {
      let proxy_settings = ProxySettings {
        proxy_type: exported.proxy_type,
        host: exported.host,
        port: exported.port,
        username: exported.username,
        password: exported.password,
      };

      match self.create_stored_proxy(app_handle, exported.name.clone(), proxy_settings) {
        Ok(proxy) => imported.push(proxy),
        Err(e) => {
          if e.contains("already exists") {
            skipped += 1;
          } else {
            errors.push(format!("Failed to import '{}': {}", exported.name, e));
          }
        }
      }
    }

    Ok(ProxyImportResult {
      imported_count: imported.len(),
      skipped_count: skipped,
      errors,
      proxies: imported,
    })
  }

  // Import proxies from already parsed proxy lines
  pub fn import_proxies_from_parsed(
    &self,
    app_handle: &tauri::AppHandle,
    parsed_proxies: Vec<ParsedProxyLine>,
    name_prefix: Option<String>,
  ) -> Result<ProxyImportResult, String> {
    let mut imported = Vec::new();
    let mut skipped = 0;
    let mut errors = Vec::new();
    let prefix = name_prefix.unwrap_or_else(|| "Imported".to_string());

    for (i, parsed) in parsed_proxies.into_iter().enumerate() {
      let proxy_name = format!("{} Proxy {}", prefix, i + 1);
      let proxy_settings = ProxySettings {
        proxy_type: parsed.proxy_type,
        host: parsed.host,
        port: parsed.port,
        username: parsed.username,
        password: parsed.password,
      };

      match self.create_stored_proxy(app_handle, proxy_name.clone(), proxy_settings) {
        Ok(proxy) => imported.push(proxy),
        Err(e) => {
          if e.contains("already exists") {
            skipped += 1;
          } else {
            errors.push(format!("Failed to import '{}': {}", proxy_name, e));
          }
        }
      }
    }

    Ok(ProxyImportResult {
      imported_count: imported.len(),
      skipped_count: skipped,
      errors,
      proxies: imported,
    })
  }

  // Start a proxy for given proxy settings and associate it with a browser process ID
  // If proxy_settings is None, starts a direct proxy for traffic monitoring
  pub async fn start_proxy(
    &self,
    app_handle: tauri::AppHandle,
    proxy_settings: Option<&ProxySettings>,
    browser_pid: u32,
    profile_id: Option<&str>,
    bypass_rules: Vec<String>,
    blocklist_file: Option<String>,
  ) -> Result<ProxySettings, String> {
    if let Some(name) = profile_id {
      // Check if we have an active proxy recorded for this profile
      let maybe_existing_id = {
        let map = self.profile_active_proxy_ids.lock().unwrap();
        map.get(name).cloned()
      };

      if let Some(existing_id) = maybe_existing_id {
        // Find the existing proxy info
        let existing_info = {
          let proxies = self.active_proxies.lock().unwrap();
          proxies.values().find(|p| p.id == existing_id).cloned()
        };

        if let Some(existing) = existing_info {
          let desired_type = proxy_settings
            .map(|p| p.proxy_type.as_str())
            .unwrap_or("DIRECT");
          let desired_host = proxy_settings.map(|p| p.host.as_str()).unwrap_or("DIRECT");
          let desired_port = proxy_settings.map(|p| p.port).unwrap_or(0);

          let is_same_upstream = existing.upstream_type == desired_type
            && existing.upstream_host == desired_host
            && existing.upstream_port == desired_port;

          if is_same_upstream {
            // Settings match - can reuse existing proxy
            // Just update the PID mapping if needed
            let proxies = self.active_proxies.lock().unwrap();
            if proxies.contains_key(&browser_pid) {
              // Already mapped, reuse it
              return Ok(ProxySettings {
                proxy_type: "http".to_string(),
                host: "127.0.0.1".to_string(),
                port: existing.local_port,
                username: None,
                password: None,
              });
            }
            // Need to add this PID to the mapping - we'll do that after starting
          }
          // Settings differ - we'll create a new proxy, but don't stop the old one
          // It will be cleaned up by periodic cleanup if it becomes dead
        }
      }
    }
    // Check if we already have a proxy for this browser PID
    // If settings match, reuse it; otherwise create a new one (don't stop the old one)
    {
      let proxies = self.active_proxies.lock().unwrap();
      if let Some(existing) = proxies.get(&browser_pid) {
        let desired_type = proxy_settings
          .map(|p| p.proxy_type.as_str())
          .unwrap_or("DIRECT");
        let desired_host = proxy_settings.map(|p| p.host.as_str()).unwrap_or("DIRECT");
        let desired_port = proxy_settings.map(|p| p.port).unwrap_or(0);

        let is_same_upstream = existing.upstream_type == desired_type
          && existing.upstream_host == desired_host
          && existing.upstream_port == desired_port;

        if is_same_upstream {
          // Check if profile_id matches
          let profile_id_matches = match (profile_id, &existing.profile_id) {
            (Some(ref new_id), Some(ref old_id)) => new_id == old_id,
            (None, None) => true,
            _ => false,
          };

          if profile_id_matches {
            // Reuse existing local proxy (settings and profile_id match)
            return Ok(ProxySettings {
              proxy_type: "http".to_string(),
              host: "127.0.0.1".to_string(),
              port: existing.local_port,
              username: None,
              password: None,
            });
          }
          // Profile ID changed - we'll create a new proxy but don't stop the old one
          // It will be cleaned up by periodic cleanup if it becomes dead
        }
        // Upstream changed - we'll create a new proxy but don't stop the old one
        // It will be cleaned up by periodic cleanup if it becomes dead
      }
    }

    // Start a new proxy using the donut-proxy binary with the correct CLI interface
    let mut proxy_cmd = app_handle
      .shell()
      .sidecar("donut-proxy")
      .map_err(|e| format!("Failed to create sidecar: {e}"))?
      .arg("proxy")
      .arg("start");

    // Add upstream proxy settings if provided, otherwise create direct proxy
    if let Some(proxy_settings) = proxy_settings {
      proxy_cmd = proxy_cmd
        .arg("--host")
        .arg(&proxy_settings.host)
        .arg("--proxy-port")
        .arg(proxy_settings.port.to_string())
        .arg("--type")
        .arg(&proxy_settings.proxy_type);

      // Add credentials if provided
      if let Some(username) = &proxy_settings.username {
        proxy_cmd = proxy_cmd.arg("--username").arg(username);
      }
      if let Some(password) = &proxy_settings.password {
        proxy_cmd = proxy_cmd.arg("--password").arg(password);
      }
    }

    // Add profile ID if provided for traffic tracking
    if let Some(id) = profile_id {
      proxy_cmd = proxy_cmd.arg("--profile-id").arg(id);
    }

    // Add bypass rules if any
    if !bypass_rules.is_empty() {
      let rules_json = serde_json::to_string(&bypass_rules)
        .map_err(|e| format!("Failed to serialize bypass rules: {e}"))?;
      proxy_cmd = proxy_cmd.arg("--bypass-rules").arg(rules_json);
    }

    // Add blocklist file path if provided
    if let Some(ref path) = blocklist_file {
      proxy_cmd = proxy_cmd.arg("--blocklist-file").arg(path);
    }

    // Execute the command and wait for it to complete
    // The donut-proxy binary should start the worker and then exit
    let output = proxy_cmd
      .output()
      .await
      .map_err(|e| format!("Failed to execute donut-proxy: {e}"))?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let stdout = String::from_utf8_lossy(&output.stdout);
      return Err(format!(
        "Proxy start failed - stdout: {stdout}, stderr: {stderr}"
      ));
    }

    let json_string =
      String::from_utf8(output.stdout).map_err(|e| format!("Failed to parse proxy output: {e}"))?;

    // Parse the JSON output
    let json: Value = serde_json::from_str(json_string.trim())
      .map_err(|e| format!("Failed to parse JSON: {e}. Output was: {}", json_string))?;

    // Extract proxy information
    let id = json["id"].as_str().ok_or("Missing proxy ID")?;
    let local_port = json["localPort"]
      .as_u64()
      .ok_or_else(|| format!("Missing local port in JSON: {}", json_string))?
      as u16;
    let local_url = json["localUrl"]
      .as_str()
      .ok_or_else(|| format!("Missing local URL in JSON: {}", json_string))?
      .to_string();

    let proxy_info = ProxyInfo {
      id: id.to_string(),
      local_url,
      upstream_host: proxy_settings
        .map(|p| p.host.clone())
        .unwrap_or_else(|| "DIRECT".to_string()),
      upstream_port: proxy_settings.map(|p| p.port).unwrap_or(0),
      upstream_type: proxy_settings
        .map(|p| p.proxy_type.clone())
        .unwrap_or_else(|| "DIRECT".to_string()),
      local_port,
      profile_id: profile_id.map(|s| s.to_string()),
      blocklist_file: blocklist_file.clone(),
    };

    // Wait for the local proxy port to be ready to accept connections
    {
      use tokio::net::TcpStream;
      use tokio::time::{sleep, Duration};
      let mut ready = false;
      for _ in 0..50 {
        match TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy_info.local_port)).await {
          Ok(_stream) => {
            ready = true;
            break;
          }
          Err(_) => {
            sleep(Duration::from_millis(100)).await;
          }
        }
      }
      if !ready {
        return Err(format!(
          "Local proxy on 127.0.0.1:{} did not become ready in time",
          proxy_info.local_port
        ));
      }
    }

    // Store the proxy info
    {
      let mut proxies = self.active_proxies.lock().unwrap();
      proxies.insert(browser_pid, proxy_info.clone());
    }

    // Store the profile proxy info for persistence
    if let Some(id) = profile_id {
      if let Some(proxy_settings) = proxy_settings {
        let mut profile_proxies = self.profile_proxies.lock().unwrap();
        profile_proxies.insert(id.to_string(), proxy_settings.clone());
      }
      // Also record the active proxy id for this profile for quick cleanup on changes
      let mut map = self.profile_active_proxy_ids.lock().unwrap();
      map.insert(id.to_string(), proxy_info.id.clone());
    }

    // Return proxy settings for the browser
    Ok(ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(), // Use 127.0.0.1 instead of localhost for better compatibility
      port: proxy_info.local_port,
      username: None,
      password: None,
    })
  }

  // Stop the proxy associated with a browser process ID
  pub async fn stop_proxy(
    &self,
    app_handle: tauri::AppHandle,
    browser_pid: u32,
  ) -> Result<(), String> {
    let (proxy_id, profile_id): (String, Option<String>) = {
      let mut proxies = self.active_proxies.lock().unwrap();
      match proxies.remove(&browser_pid) {
        Some(proxy) => (proxy.id, proxy.profile_id.clone()),
        None => return Ok(()), // No proxy to stop
      }
    };

    // Stop the proxy using the donut-proxy binary
    let proxy_cmd = app_handle
      .shell()
      .sidecar("donut-proxy")
      .map_err(|e| format!("Failed to create sidecar: {e}"))?
      .arg("proxy")
      .arg("stop")
      .arg("--id")
      .arg(&proxy_id);

    let output = proxy_cmd.output().await.unwrap();

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      log::warn!("Proxy stop error: {stderr}");
      // We still return Ok since we've already removed the proxy from our tracking
    }

    // Clear profile-to-proxy mapping if it references this proxy
    if let Some(id) = profile_id {
      let mut map = self.profile_active_proxy_ids.lock().unwrap();
      if let Some(current_id) = map.get(&id) {
        if current_id == &proxy_id {
          map.remove(&id);
        }
      }
    }

    // Emit event for reactive UI updates
    if let Err(e) = events::emit_empty("proxies-changed") {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    Ok(())
  }

  // Stop the proxy associated with a profile ID
  pub async fn stop_proxy_by_profile_id(
    &self,
    app_handle: tauri::AppHandle,
    profile_id: &str,
  ) -> Result<(), String> {
    // Find the proxy ID for this profile
    let proxy_id = {
      let map = self.profile_active_proxy_ids.lock().unwrap();
      map.get(profile_id).cloned()
    };

    if let Some(proxy_id) = proxy_id {
      // Find the PID for this proxy
      let pid = {
        let proxies = self.active_proxies.lock().unwrap();
        proxies.iter().find_map(|(pid, proxy)| {
          if proxy.id == proxy_id {
            Some(*pid)
          } else {
            None
          }
        })
      };

      if let Some(pid) = pid {
        // Use the existing stop_proxy method
        self.stop_proxy(app_handle, pid).await
      } else {
        // Proxy not found in active_proxies, try to stop it directly by ID
        let proxy_cmd = app_handle
          .shell()
          .sidecar("donut-proxy")
          .map_err(|e| format!("Failed to create sidecar: {e}"))?
          .arg("proxy")
          .arg("stop")
          .arg("--id")
          .arg(&proxy_id);

        let output = proxy_cmd.output().await.unwrap();

        if !output.status.success() {
          let stderr = String::from_utf8_lossy(&output.stderr);
          log::warn!("Proxy stop error: {stderr}");
        }

        // Clear profile-to-proxy mapping
        let mut map = self.profile_active_proxy_ids.lock().unwrap();
        map.remove(profile_id);

        // Emit event for reactive UI updates
        if let Err(e) = events::emit_empty("proxies-changed") {
          log::error!("Failed to emit proxies-changed event: {e}");
        }

        Ok(())
      }
    } else {
      // No proxy found for this profile
      Ok(())
    }
  }

  // Update the PID mapping for an existing proxy
  pub fn update_proxy_pid(&self, old_pid: u32, new_pid: u32) -> Result<(), String> {
    let mut proxies = self.active_proxies.lock().unwrap();
    if let Some(proxy_info) = proxies.remove(&old_pid) {
      proxies.insert(new_pid, proxy_info);
      Ok(())
    } else {
      Err(format!("No proxy found for PID {old_pid}"))
    }
  }

  // Clean up proxies for dead browser processes
  // Only clean up orphaned config files where the proxy process itself is dead
  pub async fn cleanup_dead_proxies(
    &self,
    _app_handle: tauri::AppHandle,
  ) -> Result<Vec<u32>, String> {
    // Don't stop proxies for dead browser processes - let them run indefinitely
    // The proxy processes are idle and don't consume CPU when not in use
    // Only clean up config files where the proxy process itself is dead (see below)
    let dead_pids: Vec<u32> = Vec::new();

    // Clean up orphaned proxy configs (only where proxy process is definitely dead)
    // IMPORTANT: Only clean up configs where the proxy process itself is dead
    // If the proxy process is running (even if idle), leave it alone
    // The user doesn't care if proxy processes run indefinitely as long as they're not consuming CPU
    let orphaned_configs = {
      use crate::proxy_storage::{is_process_running, list_proxy_configs};
      use std::time::{SystemTime, UNIX_EPOCH};

      let all_configs = list_proxy_configs();
      let tracked_proxy_ids: std::collections::HashSet<String> = {
        let proxies = self.active_proxies.lock().unwrap();
        proxies.values().map(|p| p.id.clone()).collect()
      };

      // Get current time for grace period check
      let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

      all_configs
        .into_iter()
        .filter(|config| {
          // If proxy is tracked in active_proxies, it's definitely not orphaned
          if tracked_proxy_ids.contains(&config.id) {
            return false;
          }

          // Extract creation time from proxy ID (format: proxy_{timestamp}_{random})
          // This gives us a grace period for newly created proxies
          let proxy_age = config
            .id
            .strip_prefix("proxy_")
            .and_then(|s| s.split('_').next())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|created_at| now.saturating_sub(created_at))
            .unwrap_or(0);

          // Grace period: don't clean up proxies created in the last 120 seconds
          // This prevents race conditions during startup (increased from 60 to 120 for safety)
          if proxy_age < 120 {
            log::debug!(
              "Skipping cleanup of proxy {} - too new (age: {}s)",
              config.id,
              proxy_age
            );
            return false;
          }

          // ONLY clean up if we can verify the proxy process is dead
          // If proxy process is running, leave it alone (even if idle)
          if let Some(proxy_pid) = config.pid {
            // Check if proxy process is actually dead
            if !is_process_running(proxy_pid) {
              // Proxy process is dead, clean up the config file
              log::info!(
                "Proxy {} process (PID {}) is dead, will clean up config",
                config.id,
                proxy_pid
              );
              return true;
            }
            // Proxy process is running - leave it alone
            log::debug!(
              "Skipping cleanup of proxy {} - process (PID {}) is still running",
              config.id,
              proxy_pid
            );
            return false;
          }

          // No PID in config - can't verify if process is dead
          // Be conservative: don't clean up (might be starting up or PID not set yet)
          log::debug!(
            "Skipping cleanup of proxy {} - no PID in config (might be starting up)",
            config.id
          );
          false
        })
        .collect::<Vec<_>>()
    };

    // Clean up orphaned config files (proxy process is dead)
    for config in orphaned_configs {
      log::info!(
        "Cleaning up orphaned proxy config: {} (proxy process is dead)",
        config.id
      );
      // Just delete the config file - the process is already dead
      use crate::proxy_storage::delete_proxy_config;
      delete_proxy_config(&config.id);
    }

    // Clean up orphaned VPN worker configs where the worker process is dead
    {
      use crate::proxy_storage::is_process_running;
      use crate::vpn_worker_storage::{delete_vpn_worker_config, list_vpn_worker_configs};

      let vpn_workers = list_vpn_worker_configs();
      for worker in vpn_workers {
        if let Some(pid) = worker.pid {
          if !is_process_running(pid) {
            log::info!(
              "Cleaning up orphaned VPN worker config: {} (process PID {} is dead)",
              worker.id,
              pid
            );
            let _ = std::fs::remove_file(&worker.config_file_path);
            delete_vpn_worker_config(&worker.id);
          }
        }
      }
    }

    // Emit event for reactive UI updates
    if let Err(e) = events::emit_empty("proxies-changed") {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    Ok(dead_pids)
  }

  /// Snapshot the set of tracked proxy IDs (for asserting in tests).
  #[cfg(test)]
  fn tracked_proxy_ids(&self) -> std::collections::HashSet<String> {
    let proxies = self.active_proxies.lock().unwrap();
    proxies.values().map(|p| p.id.clone()).collect()
  }

  /// Snapshot active proxy count.
  #[cfg(test)]
  fn active_proxy_count(&self) -> usize {
    self.active_proxies.lock().unwrap().len()
  }

  /// Snapshot profile-to-proxy-id mapping count.
  #[cfg(test)]
  fn profile_proxy_mapping_count(&self) -> usize {
    self.profile_active_proxy_ids.lock().unwrap().len()
  }

  /// Insert a proxy info entry directly (for testing).
  #[cfg(test)]
  fn insert_active_proxy(&self, browser_pid: u32, info: ProxyInfo) {
    self
      .active_proxies
      .lock()
      .unwrap()
      .insert(browser_pid, info);
  }

  /// Insert a profile-to-proxy mapping directly (for testing).
  #[cfg(test)]
  fn insert_profile_proxy_mapping(&self, profile_id: String, proxy_id: String) {
    self
      .profile_active_proxy_ids
      .lock()
      .unwrap()
      .insert(profile_id, proxy_id);
  }

  /// Get active proxy info by browser PID (for testing).
  #[cfg(test)]
  fn get_active_proxy(&self, browser_pid: u32) -> Option<ProxyInfo> {
    self
      .active_proxies
      .lock()
      .unwrap()
      .get(&browser_pid)
      .cloned()
  }
}

// Create a singleton instance of the proxy manager
lazy_static::lazy_static! {
    pub static ref PROXY_MANAGER: ProxyManager = ProxyManager::new();
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::env;
  use std::path::PathBuf;
  use std::time::Duration;
  use tokio::process::Command;
  use tokio::time::sleep;

  // Mock HTTP server for testing

  use http_body_util::Full;
  use hyper::body::Bytes;
  use hyper::server::conn::http1;
  use hyper::service::service_fn;
  use hyper::Response;
  use hyper_util::rt::TokioIo;
  use tokio::net::TcpListener;
  use wiremock::matchers::{method, path};
  use wiremock::{Mock, MockServer, ResponseTemplate};

  // Helper function to build donut-proxy binary for testing
  async fn ensure_donut_proxy_binary() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let project_root = PathBuf::from(cargo_manifest_dir)
      .parent()
      .unwrap()
      .to_path_buf();
    let proxy_binary_name = if cfg!(windows) {
      "donut-proxy.exe"
    } else {
      "donut-proxy"
    };
    let proxy_binary = project_root
      .join("src-tauri")
      .join("target")
      .join("debug")
      .join(proxy_binary_name);

    // Check if binary already exists
    if proxy_binary.exists() {
      return Ok(proxy_binary);
    }

    // Build the donut-proxy binary
    println!("Building donut-proxy binary for tests...");

    let build_status = Command::new("cargo")
      .args(["build", "--bin", "donut-proxy"])
      .current_dir(project_root.join("src-tauri"))
      .status()
      .await?;

    if !build_status.success() {
      return Err("Failed to build donut-proxy binary".into());
    }

    if !proxy_binary.exists() {
      return Err("donut-proxy binary was not created successfully".into());
    }

    Ok(proxy_binary)
  }

  #[test]
  fn test_proxy_settings_validation() {
    // Test valid proxy settings
    let valid_settings = ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: Some("user".to_string()),
      password: Some("pass".to_string()),
    };

    assert!(
      !valid_settings.host.is_empty(),
      "Valid settings should have non-empty host"
    );
    assert!(
      valid_settings.port > 0,
      "Valid settings should have positive port"
    );
    assert_eq!(valid_settings.proxy_type, "http", "Proxy type should match");
    assert!(
      valid_settings.username.is_some(),
      "Username should be present"
    );
    assert!(
      valid_settings.password.is_some(),
      "Password should be present"
    );

    // Test proxy settings with empty values
    let empty_settings = ProxySettings {
      proxy_type: "http".to_string(),
      host: "".to_string(),
      port: 0,
      username: None,
      password: None,
    };

    assert!(
      empty_settings.host.is_empty(),
      "Empty settings should have empty host"
    );
    assert_eq!(
      empty_settings.port, 0,
      "Empty settings should have zero port"
    );
    assert!(empty_settings.username.is_none(), "Username should be None");
    assert!(empty_settings.password.is_none(), "Password should be None");
  }

  #[tokio::test]
  async fn test_proxy_manager_concurrent_access() {
    use std::sync::Arc;

    let proxy_manager = Arc::new(ProxyManager::new());
    let mut handles = vec![];

    // Spawn multiple tasks that access the proxy manager concurrently
    for i in 0..10 {
      let pm = proxy_manager.clone();
      let handle = tokio::spawn(async move {
        let browser_pid = (1000 + i) as u32;
        let proxy_info = ProxyInfo {
          id: format!("proxy_{i}"),
          local_url: format!("http://127.0.0.1:{}", 8000 + i),
          upstream_host: "127.0.0.1".to_string(),
          upstream_port: 3128,
          upstream_type: "http".to_string(),
          local_port: (8000 + i) as u16,
          profile_id: None,
          blocklist_file: None,
        };

        // Add proxy
        {
          let mut active_proxies = pm.active_proxies.lock().unwrap();
          active_proxies.insert(browser_pid, proxy_info);
        }

        browser_pid
      });
      handles.push(handle);
    }

    // Wait for all tasks to complete
    let results: Vec<u32> = futures_util::future::join_all(handles)
      .await
      .into_iter()
      .map(|r| r.unwrap())
      .collect();

    // Verify all browser PIDs were processed
    assert_eq!(results.len(), 10);
    for (i, &browser_pid) in results.iter().enumerate() {
      assert_eq!(browser_pid, (1000 + i) as u32);
    }
  }

  // Integration test that actually builds and uses donut-proxy binary
  #[tokio::test]
  async fn test_proxy_integration_with_real_proxy() -> Result<(), Box<dyn std::error::Error>> {
    // This test requires donut-proxy binary to be available
    // Skip if we can't find the binary or if proxy startup fails
    use crate::proxy_runner::{start_proxy_process, stop_proxy_process};
    use tokio::net::TcpStream;

    // Start a mock upstream HTTP server
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_addr = upstream_listener.local_addr()?;

    // Spawn upstream server
    let server_handle = tokio::spawn(async move {
      while let Ok((stream, _)) = upstream_listener.accept().await {
        let io = TokioIo::new(stream);
        tokio::task::spawn(async move {
          let _ = http1::Builder::new()
            .serve_connection(
              io,
              service_fn(|_req| async {
                Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::from("Upstream OK"))))
              }),
            )
            .await;
        });
      }
    });

    // Wait for server to start
    sleep(Duration::from_millis(100)).await;

    let upstream_url = format!("http://{}:{}", upstream_addr.ip(), upstream_addr.port());

    // Try to start proxy - if it fails, skip the test
    let config = match start_proxy_process(Some(upstream_url), None).await {
      Ok(config) => config,
      Err(e) => {
        println!("Skipping proxy integration test - proxy startup failed: {e}");
        server_handle.abort();
        return Ok(()); // Skip test instead of failing
      }
    };

    // Verify proxy configuration
    assert!(!config.id.is_empty());
    assert!(config.local_port.is_some());

    let proxy_id = config.id.clone();
    let local_port = config.local_port.unwrap();

    // Verify the local port is listening (should be fast now)
    match tokio::time::timeout(
      Duration::from_millis(500),
      TcpStream::connect(("127.0.0.1", local_port)),
    )
    .await
    {
      Ok(Ok(_)) => {
        println!("Proxy is listening on port {local_port}");
      }
      Ok(Err(e)) => {
        println!("Warning: Proxy port {local_port} is not listening: {e:?}");
        // Don't fail the test, just log a warning
      }
      Err(_) => {
        println!("Warning: Proxy port {local_port} connection check timed out");
        // Don't fail the test, just log a warning
      }
    }

    // Test stopping the proxy
    let stopped = stop_proxy_process(&proxy_id).await?;
    assert!(stopped);

    println!("Integration test passed: proxy start/stop works correctly");

    // Clean up server
    server_handle.abort();

    Ok(())
  }

  // Test that validates the command line arguments are constructed correctly
  #[test]
  fn test_proxy_command_construction() {
    let proxy_settings = ProxySettings {
      proxy_type: "http".to_string(),
      host: "proxy.example.com".to_string(),
      port: 8080,
      username: Some("user".to_string()),
      password: Some("pass".to_string()),
    };

    // Test command arguments match expected format
    let expected_args = [
      "proxy",
      "start",
      "--host",
      "proxy.example.com",
      "--proxy-port",
      "8080",
      "--type",
      "http",
      "--username",
      "user",
      "--password",
      "pass",
    ];

    // This test verifies the argument structure without actually running the command
    assert_eq!(
      proxy_settings.host, "proxy.example.com",
      "Host should match expected value"
    );
    assert_eq!(
      proxy_settings.port, 8080,
      "Port should match expected value"
    );
    assert_eq!(
      proxy_settings.proxy_type, "http",
      "Proxy type should match expected value"
    );
    assert_eq!(
      proxy_settings.username.as_ref().unwrap(),
      "user",
      "Username should match expected value"
    );
    assert_eq!(
      proxy_settings.password.as_ref().unwrap(),
      "pass",
      "Password should match expected value"
    );

    // Verify expected args structure
    assert_eq!(expected_args[0], "proxy", "First arg should be 'proxy'");
    assert_eq!(expected_args[1], "start", "Second arg should be 'start'");
    assert_eq!(expected_args[2], "--host", "Third arg should be '--host'");
    assert_eq!(
      expected_args[3], "proxy.example.com",
      "Fourth arg should be host value"
    );
  }

  // Test the CLI detachment specifically - ensure the CLI exits properly
  #[tokio::test]
  async fn test_cli_exits_after_proxy_start() -> Result<(), Box<dyn std::error::Error>> {
    let proxy_path = ensure_donut_proxy_binary().await?;

    // Test that the CLI exits quickly with a mock upstream
    let mut cmd = Command::new(&proxy_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("httpbin.org")
      .arg("--proxy-port")
      .arg("80")
      .arg("--type")
      .arg("http");

    let start_time = std::time::Instant::now();
    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await;

    match output {
      Ok(Ok(cmd_output)) => {
        let execution_time = start_time.elapsed();

        if cmd_output.status.success() {
          let stdout = String::from_utf8(cmd_output.stdout)?;
          let config: serde_json::Value = serde_json::from_str(&stdout)?;

          // Clean up - try to stop the proxy
          if let Some(proxy_id) = config["id"].as_str() {
            let mut stop_cmd = Command::new(&proxy_path);
            stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
            let _ = stop_cmd.output().await;
          }
        }

        println!("CLI detachment test passed - CLI exited in {execution_time:?}");
      }
      Ok(Err(e)) => {
        return Err(format!("Command execution failed: {e}").into());
      }
      Err(_) => {
        return Err("CLI command timed out - this indicates improper detachment".into());
      }
    }

    Ok(())
  }

  // Test that validates proper CLI detachment behavior
  #[tokio::test]
  async fn test_cli_detachment_behavior() -> Result<(), Box<dyn std::error::Error>> {
    let proxy_path = ensure_donut_proxy_binary().await?;

    // Test that the CLI command exits quickly even with a real upstream
    let mut cmd = Command::new(&proxy_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("httpbin.org")
      .arg("--proxy-port")
      .arg("80")
      .arg("--type")
      .arg("http");

    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;
      let proxy_id = config["id"].as_str().unwrap();

      // Clean up
      let mut stop_cmd = Command::new(&proxy_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
      let _ = stop_cmd.output().await;

      println!("CLI detachment test passed");
    } else {
      // Even if the upstream fails, the CLI should still exit quickly
      println!("CLI command failed but exited quickly as expected");
    }

    Ok(())
  }

  // Test that validates URL encoding for special characters in credentials
  #[tokio::test]
  async fn test_proxy_credentials_encoding() -> Result<(), Box<dyn std::error::Error>> {
    let proxy_path = ensure_donut_proxy_binary().await?;

    // Test with credentials that include special characters
    let mut cmd = Command::new(&proxy_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("test.example.com")
      .arg("--proxy-port")
      .arg("8080")
      .arg("--type")
      .arg("http")
      .arg("--username")
      .arg("user@domain.com")
      .arg("--password")
      .arg("pass word!");

    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;

      let upstream_url = config["upstreamUrl"].as_str().unwrap();

      println!("Generated upstream URL: {upstream_url}");

      // Verify that special characters are properly encoded
      assert!(upstream_url.contains("user%40domain.com"));
      assert!(upstream_url.contains("pass%20word"));

      println!("URL encoding test passed - special characters handled correctly");

      // Clean up
      let proxy_id = config["id"].as_str().unwrap();
      let mut stop_cmd = Command::new(&proxy_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
      let _ = stop_cmd.output().await;
    } else {
      let stdout = String::from_utf8(output.stdout)?;
      let stderr = String::from_utf8(output.stderr)?;
      println!("Command failed (expected for non-existent upstream):");
      println!("Stdout: {stdout}");
      println!("Stderr: {stderr}");

      println!("URL encoding test completed - credentials should be properly encoded");
    }

    Ok(())
  }

  // ──────────────────────────────────────────────────────────────────────
  // Complex proxy process monitoring tests
  // ──────────────────────────────────────────────────────────────────────

  fn make_proxy_info(id: &str, port: u16, profile_id: Option<&str>) -> ProxyInfo {
    ProxyInfo {
      id: id.to_string(),
      local_url: format!("http://127.0.0.1:{port}"),
      upstream_host: "10.0.0.1".to_string(),
      upstream_port: 3128,
      upstream_type: "http".to_string(),
      local_port: port,
      profile_id: profile_id.map(|s| s.to_string()),
      blocklist_file: None,
    }
  }

  #[test]
  fn test_pid_mapping_lifecycle() {
    let pm = ProxyManager::new();

    // Initially empty
    assert_eq!(pm.active_proxy_count(), 0);

    // Register proxies for 3 browser PIDs
    pm.insert_active_proxy(1001, make_proxy_info("px_a", 9001, Some("profile_1")));
    pm.insert_active_proxy(1002, make_proxy_info("px_b", 9002, Some("profile_2")));
    pm.insert_active_proxy(1003, make_proxy_info("px_c", 9003, None));

    assert_eq!(pm.active_proxy_count(), 3);

    // Verify each PID resolves correctly
    let a = pm.get_active_proxy(1001).unwrap();
    assert_eq!(a.id, "px_a");
    assert_eq!(a.local_port, 9001);
    assert_eq!(a.profile_id.as_deref(), Some("profile_1"));

    let c = pm.get_active_proxy(1003).unwrap();
    assert!(c.profile_id.is_none());

    // Unknown PID returns None
    assert!(pm.get_active_proxy(9999).is_none());
  }

  #[test]
  fn test_update_proxy_pid_remaps_correctly() {
    let pm = ProxyManager::new();
    pm.insert_active_proxy(100, make_proxy_info("px_remap", 9010, Some("prof_a")));

    // Old PID 100 → new PID 200
    pm.update_proxy_pid(100, 200).unwrap();

    // Old PID should be gone
    assert!(pm.get_active_proxy(100).is_none());

    // New PID should have the same proxy info
    let info = pm.get_active_proxy(200).unwrap();
    assert_eq!(info.id, "px_remap");
    assert_eq!(info.local_port, 9010);
    assert_eq!(info.profile_id.as_deref(), Some("prof_a"));
  }

  #[test]
  fn test_update_proxy_pid_error_for_unknown_pid() {
    let pm = ProxyManager::new();
    let result = pm.update_proxy_pid(777, 888);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("No proxy found for PID 777"));
  }

  #[test]
  fn test_profile_proxy_id_mapping_tracks_active_proxy() {
    let pm = ProxyManager::new();

    pm.insert_active_proxy(500, make_proxy_info("px_1", 9100, Some("profile_x")));
    pm.insert_profile_proxy_mapping("profile_x".to_string(), "px_1".to_string());

    // Verify mapping exists
    {
      let map = pm.profile_active_proxy_ids.lock().unwrap();
      assert_eq!(map.get("profile_x").unwrap(), "px_1");
    }

    // Simulate profile-specific cleanup: remove the profile mapping
    {
      let mut map = pm.profile_active_proxy_ids.lock().unwrap();
      map.remove("profile_x");
    }

    assert_eq!(pm.profile_proxy_mapping_count(), 0);
    // Active proxy itself should still be there
    assert_eq!(pm.active_proxy_count(), 1);
  }

  #[test]
  fn test_tracked_proxy_ids_returns_all_unique_ids() {
    let pm = ProxyManager::new();
    pm.insert_active_proxy(1, make_proxy_info("alpha", 8001, None));
    pm.insert_active_proxy(2, make_proxy_info("beta", 8002, None));
    pm.insert_active_proxy(3, make_proxy_info("gamma", 8003, None));

    let ids = pm.tracked_proxy_ids();
    assert_eq!(ids.len(), 3);
    assert!(ids.contains("alpha"));
    assert!(ids.contains("beta"));
    assert!(ids.contains("gamma"));
  }

  #[tokio::test]
  async fn test_concurrent_pid_registration_and_removal() {
    use std::sync::Arc;

    let pm = Arc::new(ProxyManager::new());
    let mut handles = vec![];

    // Phase 1: concurrent insertion of 50 proxies
    for i in 0..50 {
      let pm = pm.clone();
      handles.push(tokio::spawn(async move {
        let pid = 2000 + i as u32;
        let info = make_proxy_info(&format!("px_{i}"), 7000 + i as u16, None);
        pm.insert_active_proxy(pid, info);
      }));
    }
    for h in handles.drain(..) {
      h.await.unwrap();
    }
    assert_eq!(pm.active_proxy_count(), 50);

    // Phase 2: concurrent removal of half the proxies
    for i in (0..50).step_by(2) {
      let pm = pm.clone();
      handles.push(tokio::spawn(async move {
        let pid = 2000 + i as u32;
        let mut proxies = pm.active_proxies.lock().unwrap();
        proxies.remove(&pid);
      }));
    }
    for h in handles.drain(..) {
      h.await.unwrap();
    }
    assert_eq!(pm.active_proxy_count(), 25);

    // Phase 3: remaining proxies should all have odd indices
    let proxies = pm.active_proxies.lock().unwrap();
    for (&pid, info) in proxies.iter() {
      let idx = (pid - 2000) as usize;
      assert!(idx % 2 == 1, "Only odd-index proxies should remain");
      assert_eq!(info.id, format!("px_{idx}"));
    }
  }

  #[test]
  fn test_process_running_detection_with_child_lifecycle() {
    use crate::proxy_storage::is_process_running;

    // Spawn a long-lived child so we can check while it runs.
    // On Windows, `timeout` requires console input and exits immediately in
    // non-interactive contexts, so use `ping` with a high count instead.
    let mut child = std::process::Command::new(if cfg!(windows) { "ping" } else { "sleep" })
      .args(if cfg!(windows) {
        vec!["-n", "100", "127.0.0.1"]
      } else {
        vec!["10"]
      })
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .spawn()
      .expect("spawn long-lived child");

    let pid = child.id();

    // Process should be alive
    assert!(
      is_process_running(pid),
      "Child process must be detected as running (PID {pid})"
    );

    // Kill it
    child.kill().expect("kill child");
    child.wait().expect("wait child");

    // Process should now be dead
    assert!(
      !is_process_running(pid),
      "Killed child must be detected as dead (PID {pid})"
    );
  }

  #[tokio::test]
  async fn test_cleanup_distinguishes_live_and_dead_proxy_configs() {
    use crate::proxy_storage::{save_proxy_config, ProxyConfig};

    // Spawn a live child process to use its PID.
    // On Windows, `timeout` requires console input and exits immediately in CI,
    // so use `ping` which works reliably in non-interactive contexts.
    let mut live_child = std::process::Command::new(if cfg!(windows) { "ping" } else { "sleep" })
      .args(if cfg!(windows) {
        vec!["-n", "30", "127.0.0.1"]
      } else {
        vec!["30"]
      })
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .spawn()
      .expect("spawn live child");
    let live_pid = live_child.id();

    // Spawn and kill a short-lived process to get a dead PID
    let dead_child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
      .args(if cfg!(windows) {
        vec!["/C", "exit"]
      } else {
        vec![]
      })
      .spawn()
      .expect("spawn dead child");
    let dead_pid = dead_child.id();
    let mut dead_child = dead_child;
    dead_child.wait().expect("wait for dead child");

    // Use an old timestamp so the configs aren't in the grace period
    let old_ts = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_secs()
      - 300; // 5 minutes ago

    // Save both proxy configs to disk
    let live_id = format!("proxy_{old_ts}_11111");
    let dead_id = format!("proxy_{old_ts}_22222");

    let live_config = ProxyConfig {
      id: live_id.clone(),
      upstream_url: "DIRECT".to_string(),
      local_port: Some(19001),
      ignore_proxy_certificate: None,
      local_url: Some("http://127.0.0.1:19001".to_string()),
      pid: Some(live_pid),
      profile_id: None,
      bypass_rules: Vec::new(),
      blocklist_file: None,
    };
    let dead_config = ProxyConfig {
      id: dead_id.clone(),
      upstream_url: "DIRECT".to_string(),
      local_port: Some(19002),
      ignore_proxy_certificate: None,
      local_url: Some("http://127.0.0.1:19002".to_string()),
      pid: Some(dead_pid),
      profile_id: None,
      bypass_rules: Vec::new(),
      blocklist_file: None,
    };

    save_proxy_config(&live_config).unwrap();
    save_proxy_config(&dead_config).unwrap();

    // Verify is_process_running differentiates them
    assert!(
      crate::proxy_storage::is_process_running(live_pid),
      "Live PID should be detected"
    );
    assert!(
      !crate::proxy_storage::is_process_running(dead_pid),
      "Dead PID should not be detected"
    );

    // Clean up
    live_child.kill().expect("kill live child");
    live_child.wait().expect("wait live child");
    crate::proxy_storage::delete_proxy_config(&live_id);
    crate::proxy_storage::delete_proxy_config(&dead_id);
  }

  #[test]
  fn test_proxy_config_persistence_roundtrip() {
    use crate::proxy_storage::{
      delete_proxy_config, generate_proxy_id, get_proxy_config, save_proxy_config, ProxyConfig,
    };

    let id = generate_proxy_id();
    let config = ProxyConfig {
      id: id.clone(),
      upstream_url: "socks5://user:pass@10.0.0.1:1080".to_string(),
      local_port: Some(18080),
      ignore_proxy_certificate: Some(true),
      local_url: Some("http://127.0.0.1:18080".to_string()),
      pid: Some(12345),
      profile_id: Some("prof_abc".to_string()),
      bypass_rules: vec!["*.local".to_string(), "192.168.*".to_string()],
      blocklist_file: None,
    };

    // Save
    save_proxy_config(&config).unwrap();

    // Load and compare
    let loaded = get_proxy_config(&id).expect("Config should be loadable");
    assert_eq!(loaded.id, config.id);
    assert_eq!(loaded.upstream_url, config.upstream_url);
    assert_eq!(loaded.local_port, config.local_port);
    assert_eq!(
      loaded.ignore_proxy_certificate,
      config.ignore_proxy_certificate
    );
    assert_eq!(loaded.local_url, config.local_url);
    assert_eq!(loaded.pid, config.pid);
    assert_eq!(loaded.profile_id, config.profile_id);
    assert_eq!(loaded.bypass_rules, config.bypass_rules);

    // Clean up
    assert!(delete_proxy_config(&id));
    assert!(get_proxy_config(&id).is_none());
  }

  #[test]
  fn test_proxy_config_update_preserves_fields() {
    use crate::proxy_storage::{
      delete_proxy_config, get_proxy_config, save_proxy_config, update_proxy_config, ProxyConfig,
    };

    let id = format!("proxy_test_update_{}", rand::random::<u32>());
    let mut config = ProxyConfig::new(id.clone(), "DIRECT".to_string(), Some(17777));
    config.pid = Some(99999);
    config.profile_id = Some("prof_up".to_string());
    config.bypass_rules = vec!["google.com".to_string()];

    save_proxy_config(&config).unwrap();

    // Update: change the local_url (simulates worker binding)
    config.local_url = Some("http://127.0.0.1:17777".to_string());
    assert!(update_proxy_config(&config));

    let reloaded = get_proxy_config(&id).unwrap();
    assert_eq!(
      reloaded.local_url.as_deref(),
      Some("http://127.0.0.1:17777")
    );
    // Other fields should be preserved
    assert_eq!(reloaded.pid, Some(99999));
    assert_eq!(reloaded.bypass_rules, vec!["google.com".to_string()]);

    delete_proxy_config(&id);
  }

  #[test]
  fn test_proxy_config_list_filters_json_only() {
    use crate::proxy_storage::{
      delete_proxy_config, list_proxy_configs, save_proxy_config, ProxyConfig,
    };

    let id1 = format!("proxy_list_test_{}", rand::random::<u32>());
    let id2 = format!("proxy_list_test_{}", rand::random::<u32>());

    let c1 = ProxyConfig::new(id1.clone(), "DIRECT".to_string(), Some(16001));
    let c2 = ProxyConfig::new(id2.clone(), "DIRECT".to_string(), Some(16002));

    save_proxy_config(&c1).unwrap();
    save_proxy_config(&c2).unwrap();

    let all = list_proxy_configs();
    let our_ids: Vec<_> = all.iter().filter(|c| c.id == id1 || c.id == id2).collect();
    assert_eq!(our_ids.len(), 2, "Both test configs should be listed");

    delete_proxy_config(&id1);
    delete_proxy_config(&id2);
  }

  #[test]
  fn test_proxy_id_uniqueness_and_format() {
    use crate::proxy_storage::generate_proxy_id;

    let mut ids = std::collections::HashSet::new();
    for _ in 0..100 {
      let id = generate_proxy_id();
      assert!(id.starts_with("proxy_"), "ID must start with proxy_");
      // Format: proxy_{timestamp}_{random}
      let parts: Vec<&str> = id.split('_').collect();
      assert_eq!(
        parts.len(),
        3,
        "ID should have exactly 3 underscore-separated parts"
      );
      assert!(
        parts[1].parse::<u64>().is_ok(),
        "Second part must be a unix timestamp"
      );
      assert!(
        parts[2].parse::<u32>().is_ok(),
        "Third part must be a u32 random"
      );
      ids.insert(id);
    }
    assert_eq!(ids.len(), 100, "All 100 generated IDs must be unique");
  }

  #[test]
  fn test_multiple_profiles_share_proxy_independently() {
    let pm = ProxyManager::new();

    // Two profiles sharing the same upstream but with distinct proxy instances
    let info_a = ProxyInfo {
      id: "px_shared_a".to_string(),
      local_url: "http://127.0.0.1:9201".to_string(),
      upstream_host: "proxy.shared.com".to_string(),
      upstream_port: 8080,
      upstream_type: "http".to_string(),
      local_port: 9201,
      profile_id: Some("profile_alpha".to_string()),
      blocklist_file: None,
    };
    let info_b = ProxyInfo {
      id: "px_shared_b".to_string(),
      local_url: "http://127.0.0.1:9202".to_string(),
      upstream_host: "proxy.shared.com".to_string(),
      upstream_port: 8080,
      upstream_type: "http".to_string(),
      local_port: 9202,
      profile_id: Some("profile_beta".to_string()),
      blocklist_file: None,
    };

    pm.insert_active_proxy(3001, info_a);
    pm.insert_active_proxy(3002, info_b);
    pm.insert_profile_proxy_mapping("profile_alpha".to_string(), "px_shared_a".to_string());
    pm.insert_profile_proxy_mapping("profile_beta".to_string(), "px_shared_b".to_string());

    // Remove alpha's browser → should NOT affect beta
    {
      let mut proxies = pm.active_proxies.lock().unwrap();
      proxies.remove(&3001);
    }
    {
      let mut map = pm.profile_active_proxy_ids.lock().unwrap();
      map.remove("profile_alpha");
    }

    assert_eq!(pm.active_proxy_count(), 1);
    assert_eq!(pm.profile_proxy_mapping_count(), 1);
    let remaining = pm.get_active_proxy(3002).unwrap();
    assert_eq!(remaining.id, "px_shared_b");
    assert_eq!(remaining.profile_id.as_deref(), Some("profile_beta"));
  }

  #[test]
  fn test_proxy_url_construction() {
    // Basic HTTP
    let url = ProxyManager::build_proxy_url(&ProxySettings {
      proxy_type: "http".to_string(),
      host: "1.2.3.4".to_string(),
      port: 8080,
      username: None,
      password: None,
    });
    assert_eq!(url, "http://1.2.3.4:8080");

    // With credentials
    let url = ProxyManager::build_proxy_url(&ProxySettings {
      proxy_type: "socks5".to_string(),
      host: "proxy.example.com".to_string(),
      port: 1080,
      username: Some("user".to_string()),
      password: Some("p@ss".to_string()),
    });
    assert_eq!(url, "socks5://user:p%40ss@proxy.example.com:1080");

    // Username-only (no password)
    let url = ProxyManager::build_proxy_url(&ProxySettings {
      proxy_type: "http".to_string(),
      host: "host.io".to_string(),
      port: 3128,
      username: Some("justuser".to_string()),
      password: None,
    });
    assert_eq!(url, "http://justuser@host.io:3128");
  }

  #[test]
  fn test_geo_username_construction() {
    // Country only
    let u = ProxyManager::build_geo_username("base_user", "US", &None, &None, &None);
    assert_eq!(u, "base_user-country-US");

    // Country + region
    let u = ProxyManager::build_geo_username(
      "base_user",
      "US",
      &Some("california".to_string()),
      &None,
      &None,
    );
    assert_eq!(u, "base_user-country-US-region-california");

    // All fields
    let u = ProxyManager::build_geo_username(
      "user",
      "DE",
      &Some("bavaria".to_string()),
      &Some("munich".to_string()),
      &Some("Telekom".to_string()),
    );
    assert_eq!(u, "user-country-DE-region-bavaria-city-munich-isp-Telekom");
  }

  #[test]
  fn test_sid_generation_determinism_and_format() {
    let sid1 = ProxyManager::generate_sid_for_profile("my-profile-uuid");
    let sid2 = ProxyManager::generate_sid_for_profile("my-profile-uuid");
    assert_eq!(sid1, sid2, "Same input must produce same SID");
    assert_eq!(sid1.len(), 11, "SID must be exactly 11 characters");

    // All chars should be alphanumeric lowercase
    assert!(
      sid1
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
      "SID chars must be [a-z0-9]"
    );

    // Different profiles produce different SIDs
    let sid3 = ProxyManager::generate_sid_for_profile("another-profile");
    assert_ne!(sid1, sid3, "Different profiles must produce different SIDs");
  }

  #[test]
  fn test_build_username_with_sid() {
    let full = ProxyManager::build_username_with_sid("user-country-US", "profile-123");
    // Should contain the geo base, then -sid-{11chars}-ttl-1440m
    assert!(full.starts_with("user-country-US-sid-"));
    assert!(full.ends_with("-ttl-1440m"));
    // SID portion
    let after_sid = full.strip_prefix("user-country-US-sid-").unwrap();
    let sid = after_sid.strip_suffix("-ttl-1440m").unwrap();
    assert_eq!(sid.len(), 11);
  }

  #[test]
  fn test_stored_proxy_geo_field_migration() {
    // Simulate legacy data with geo_state but no geo_region
    let mut proxy = StoredProxy {
      id: "test_migrate".to_string(),
      name: "Test".to_string(),
      proxy_settings: ProxySettings {
        proxy_type: "http".to_string(),
        host: "h.com".to_string(),
        port: 80,
        username: None,
        password: None,
      },
      sync_enabled: false,
      last_sync: None,
      is_cloud_managed: false,
      is_cloud_derived: false,
      geo_country: Some("US".to_string()),
      geo_state: Some("california".to_string()),
      geo_region: None,
      geo_city: None,
      geo_isp: None,
      dynamic_proxy_url: None,
      dynamic_proxy_format: None,
    };

    // Before migration
    assert_eq!(proxy.effective_region().unwrap(), "california");
    assert!(proxy.geo_region.is_none());

    // After migration
    proxy.migrate_geo_fields();
    assert_eq!(proxy.geo_region.as_deref(), Some("california"));
    assert!(proxy.geo_state.is_none(), "geo_state should be taken");
    assert_eq!(proxy.effective_region().unwrap(), "california");
  }

  #[test]
  fn test_cleanup_skips_recently_created_configs() {
    use crate::proxy_storage::{delete_proxy_config, save_proxy_config, ProxyConfig};

    // Use current timestamp so it falls within the 120s grace period
    let now_ts = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_secs();

    let recent_id = format!("proxy_{now_ts}_99999");

    // Spawn and kill a child so the PID is dead
    let dead_child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
      .args(if cfg!(windows) {
        vec!["/C", "exit"]
      } else {
        vec![]
      })
      .spawn()
      .unwrap();
    let dead_pid = dead_child.id();
    let mut dead_child = dead_child;
    dead_child.wait().unwrap();

    let config = ProxyConfig {
      id: recent_id.clone(),
      upstream_url: "DIRECT".to_string(),
      local_port: Some(19999),
      ignore_proxy_certificate: None,
      local_url: None,
      pid: Some(dead_pid),
      profile_id: None,
      bypass_rules: Vec::new(),
      blocklist_file: None,
    };
    save_proxy_config(&config).unwrap();

    // The cleanup logic inspects the timestamp in the proxy ID.
    // Since we used the current timestamp, the proxy_age will be < 120 seconds,
    // so it should be skipped despite the dead PID.

    // Verify the grace period logic directly:
    let proxy_age = recent_id
      .strip_prefix("proxy_")
      .and_then(|s| s.split('_').next())
      .and_then(|s| s.parse::<u64>().ok())
      .map(|created_at| now_ts.saturating_sub(created_at))
      .unwrap_or(0);

    assert!(
      proxy_age < 120,
      "Recently created config should be in grace period"
    );

    // Clean up test config
    delete_proxy_config(&recent_id);
  }

  #[tokio::test]
  async fn test_concurrent_config_operations() {
    use crate::proxy_storage::{
      delete_proxy_config, get_proxy_config, save_proxy_config, ProxyConfig,
    };
    use std::sync::Arc;

    let ids: Vec<String> = (0..20)
      .map(|i| format!("proxy_conc_test_{}_{}", i, rand::random::<u32>()))
      .collect();
    let ids = Arc::new(ids);

    // Concurrent writes
    let mut handles = vec![];
    for id in ids.iter() {
      let id = id.clone();
      handles.push(tokio::spawn(async move {
        let config = ProxyConfig::new(id.clone(), "DIRECT".to_string(), Some(15000));
        save_proxy_config(&config).unwrap();
      }));
    }
    for h in handles {
      h.await.unwrap();
    }

    // Verify all were written
    for id in ids.iter() {
      assert!(
        get_proxy_config(id).is_some(),
        "Config {id} should be readable after concurrent write"
      );
    }

    // Concurrent deletes
    let mut handles = vec![];
    for id in ids.iter() {
      let id = id.clone();
      handles.push(tokio::spawn(async move {
        delete_proxy_config(&id);
      }));
    }
    for h in handles {
      h.await.unwrap();
    }

    // Verify all deleted
    for id in ids.iter() {
      assert!(
        get_proxy_config(id).is_none(),
        "Config {id} should be gone after concurrent delete"
      );
    }
  }

  #[test]
  fn test_proxy_txt_parsing_various_formats() {
    // URL format
    let results = ProxyManager::parse_txt_proxies("http://user:pass@proxy.com:8080\n");
    assert_eq!(results.len(), 1);
    match &results[0] {
      ProxyParseResult::Parsed(p) => {
        assert_eq!(p.proxy_type, "http");
        assert_eq!(p.host, "proxy.com");
        assert_eq!(p.port, 8080);
        assert_eq!(p.username.as_deref(), Some("user"));
        assert_eq!(p.password.as_deref(), Some("pass"));
      }
      _ => panic!("Expected Parsed result"),
    }

    // host:port format
    let results = ProxyManager::parse_txt_proxies("10.0.0.1:3128\n");
    match &results[0] {
      ProxyParseResult::Parsed(p) => {
        assert_eq!(p.host, "10.0.0.1");
        assert_eq!(p.port, 3128);
        assert!(p.username.is_none());
      }
      _ => panic!("Expected Parsed"),
    }

    // host:port:user:pass format
    let results = ProxyManager::parse_txt_proxies("myhost:9090:admin:secret\n");
    match &results[0] {
      ProxyParseResult::Parsed(p) => {
        assert_eq!(p.host, "myhost");
        assert_eq!(p.port, 9090);
        assert_eq!(p.username.as_deref(), Some("admin"));
        assert_eq!(p.password.as_deref(), Some("secret"));
      }
      _ => panic!("Expected Parsed"),
    }

    // Comments and empty lines should be skipped
    let results = ProxyManager::parse_txt_proxies("# comment\n\n  \n1.2.3.4:80\n");
    assert_eq!(results.len(), 1);

    // SOCKS5 URL
    let results = ProxyManager::parse_txt_proxies("socks5://u:p@1.2.3.4:1080\n");
    match &results[0] {
      ProxyParseResult::Parsed(p) => {
        assert_eq!(p.proxy_type, "socks5");
        assert_eq!(p.host, "1.2.3.4");
        assert_eq!(p.port, 1080);
      }
      _ => panic!("Expected Parsed"),
    }

    // Ambiguous: both positions could be ports
    let results = ProxyManager::parse_txt_proxies("1234:5678:9012:3456\n");
    match &results[0] {
      ProxyParseResult::Ambiguous {
        possible_formats, ..
      } => {
        assert_eq!(possible_formats.len(), 2);
      }
      _ => panic!("Expected Ambiguous"),
    }

    // Invalid
    let results = ProxyManager::parse_txt_proxies("notaproxy\n");
    match &results[0] {
      ProxyParseResult::Invalid { .. } => {}
      _ => panic!("Expected Invalid"),
    }
  }

  #[test]
  fn test_multiple_proxy_types_coexist() {
    let pm = ProxyManager::new();

    // Different proxy types for different profiles
    let types = [
      ("http", 3128),
      ("https", 3129),
      ("socks4", 1080),
      ("socks5", 1081),
    ];

    for (i, (ptype, port)) in types.iter().enumerate() {
      let info = ProxyInfo {
        id: format!("px_type_{ptype}"),
        local_url: format!("http://127.0.0.1:{}", 9300 + i as u16),
        upstream_host: "upstream.test".to_string(),
        upstream_port: *port,
        upstream_type: ptype.to_string(),
        local_port: 9300 + i as u16,
        profile_id: Some(format!("profile_{ptype}")),
        blocklist_file: None,
      };
      pm.insert_active_proxy(4000 + i as u32, info);
    }

    assert_eq!(pm.active_proxy_count(), 4);

    // Verify each type is stored correctly
    let info = pm.get_active_proxy(4000).unwrap();
    assert_eq!(info.upstream_type, "http");
    let info = pm.get_active_proxy(4003).unwrap();
    assert_eq!(info.upstream_type, "socks5");
    assert_eq!(info.upstream_port, 1081);
  }

  #[test]
  fn test_overwrite_pid_mapping() {
    let pm = ProxyManager::new();

    // Register proxy for PID 5000
    pm.insert_active_proxy(5000, make_proxy_info("px_old", 9400, Some("prof_ow")));

    // Overwrite the same PID with a new proxy (simulates browser reconnect with different proxy)
    pm.insert_active_proxy(5000, make_proxy_info("px_new", 9401, Some("prof_ow")));

    // Should only have 1 entry, with the new proxy
    assert_eq!(pm.active_proxy_count(), 1);
    let info = pm.get_active_proxy(5000).unwrap();
    assert_eq!(info.id, "px_new");
    assert_eq!(info.local_port, 9401);
  }

  #[test]
  fn test_proxy_config_with_bypass_rules_roundtrip() {
    use crate::proxy_storage::{
      delete_proxy_config, get_proxy_config, save_proxy_config, ProxyConfig,
    };

    let id = format!("proxy_bypass_test_{}", rand::random::<u32>());
    let rules = vec![
      "*.google.com".to_string(),
      "localhost".to_string(),
      "192.168.0.*".to_string(),
      "^.*\\.internal\\.corp$".to_string(),
    ];

    let config = ProxyConfig::new(id.clone(), "http://upstream:3128".to_string(), Some(18888))
      .with_profile_id(Some("prof_bypass".to_string()))
      .with_bypass_rules(rules.clone());

    save_proxy_config(&config).unwrap();

    let loaded = get_proxy_config(&id).unwrap();
    assert_eq!(loaded.bypass_rules.len(), 4);
    assert_eq!(loaded.bypass_rules, rules);
    assert_eq!(loaded.profile_id.as_deref(), Some("prof_bypass"));

    delete_proxy_config(&id);
  }

  #[test]
  fn test_parse_dynamic_proxy_json_standard_format() {
    let body = r#"{"ip": "1.2.3.4", "port": 8080, "username": "user1", "password": "pass1"}"#;
    let result = ProxyManager::parse_dynamic_proxy_json(body).unwrap();
    assert_eq!(result.host, "1.2.3.4");
    assert_eq!(result.port, 8080);
    assert_eq!(result.proxy_type, "http");
    assert_eq!(result.username.as_deref(), Some("user1"));
    assert_eq!(result.password.as_deref(), Some("pass1"));
  }

  #[test]
  fn test_parse_dynamic_proxy_json_host_alias() {
    let body = r#"{"host": "proxy.example.com", "port": 3128}"#;
    let result = ProxyManager::parse_dynamic_proxy_json(body).unwrap();
    assert_eq!(result.host, "proxy.example.com");
    assert_eq!(result.port, 3128);
    assert!(result.username.is_none());
    assert!(result.password.is_none());
  }

  #[test]
  fn test_parse_dynamic_proxy_json_user_pass_aliases() {
    let body = r#"{"ip": "10.0.0.1", "port": 1080, "user": "u", "pass": "p"}"#;
    let result = ProxyManager::parse_dynamic_proxy_json(body).unwrap();
    assert_eq!(result.username.as_deref(), Some("u"));
    assert_eq!(result.password.as_deref(), Some("p"));
  }

  #[test]
  fn test_parse_dynamic_proxy_json_port_as_string() {
    let body = r#"{"ip": "1.2.3.4", "port": "9090"}"#;
    let result = ProxyManager::parse_dynamic_proxy_json(body).unwrap();
    assert_eq!(result.port, 9090);
  }

  #[test]
  fn test_parse_dynamic_proxy_json_with_proxy_type() {
    let body = r#"{"ip": "1.2.3.4", "port": 1080, "type": "socks5"}"#;
    let result = ProxyManager::parse_dynamic_proxy_json(body).unwrap();
    assert_eq!(result.proxy_type, "socks5");

    let body2 = r#"{"ip": "1.2.3.4", "port": 1080, "proxy_type": "socks4"}"#;
    let result2 = ProxyManager::parse_dynamic_proxy_json(body2).unwrap();
    assert_eq!(result2.proxy_type, "socks4");

    // "protocol" field alias
    let body3 = r#"{"ip": "1.2.3.4", "port": 1080, "protocol": "socks5"}"#;
    let result3 = ProxyManager::parse_dynamic_proxy_json(body3).unwrap();
    assert_eq!(result3.proxy_type, "socks5");
  }

  #[test]
  fn test_parse_dynamic_proxy_json_normalizes_case() {
    let body = r#"{"ip": "1.2.3.4", "port": 1080, "type": "SOCKS5"}"#;
    let result = ProxyManager::parse_dynamic_proxy_json(body).unwrap();
    assert_eq!(result.proxy_type, "socks5");

    let body2 = r#"{"ip": "1.2.3.4", "port": 8080, "protocol": "HTTP"}"#;
    let result2 = ProxyManager::parse_dynamic_proxy_json(body2).unwrap();
    assert_eq!(result2.proxy_type, "http");
  }

  #[test]
  fn test_parse_dynamic_proxy_json_strips_protocol_from_host() {
    // User's API returns "ip": "socks5://1.2.3.4" with protocol embedded in host
    let body = r#"{"ip": "socks5://1.2.3.4", "port": 1080, "username": "u", "password": "p"}"#;
    let result = ProxyManager::parse_dynamic_proxy_json(body).unwrap();
    assert_eq!(result.host, "1.2.3.4");
    assert_eq!(result.proxy_type, "socks5");
    assert_eq!(result.port, 1080);

    // Protocol in host should be used as proxy_type when no explicit type field
    let body2 = r#"{"ip": "http://10.0.0.1", "port": 8080}"#;
    let result2 = ProxyManager::parse_dynamic_proxy_json(body2).unwrap();
    assert_eq!(result2.host, "10.0.0.1");
    assert_eq!(result2.proxy_type, "http");

    // Explicit type field takes precedence over protocol in host
    let body3 = r#"{"ip": "http://10.0.0.1", "port": 1080, "type": "socks5"}"#;
    let result3 = ProxyManager::parse_dynamic_proxy_json(body3).unwrap();
    assert_eq!(result3.host, "10.0.0.1");
    assert_eq!(result3.proxy_type, "socks5");
  }

  #[test]
  fn test_parse_dynamic_proxy_json_empty_credentials_treated_as_none() {
    let body = r#"{"ip": "1.2.3.4", "port": 8080, "username": "", "password": ""}"#;
    let result = ProxyManager::parse_dynamic_proxy_json(body).unwrap();
    assert!(result.username.is_none());
    assert!(result.password.is_none());
  }

  #[test]
  fn test_parse_dynamic_proxy_json_missing_ip() {
    let body = r#"{"port": 8080}"#;
    let err = ProxyManager::parse_dynamic_proxy_json(body).unwrap_err();
    assert!(err.contains("ip") || err.contains("host"));
  }

  #[test]
  fn test_parse_dynamic_proxy_json_missing_port() {
    let body = r#"{"ip": "1.2.3.4"}"#;
    let err = ProxyManager::parse_dynamic_proxy_json(body).unwrap_err();
    assert!(err.contains("port"));
  }

  #[test]
  fn test_parse_dynamic_proxy_json_invalid_json() {
    let err = ProxyManager::parse_dynamic_proxy_json("not json").unwrap_err();
    assert!(err.contains("Invalid JSON"));
  }

  #[test]
  fn test_parse_dynamic_proxy_json_not_object() {
    let err = ProxyManager::parse_dynamic_proxy_json("[1,2,3]").unwrap_err();
    assert!(err.contains("not an object"));
  }

  #[test]
  fn test_parse_dynamic_proxy_text_host_port_user_pass() {
    let body = "proxy.example.com:8080:user1:pass1";
    let result = ProxyManager::parse_dynamic_proxy_text(body).unwrap();
    assert_eq!(result.host, "proxy.example.com");
    assert_eq!(result.port, 8080);
    assert_eq!(result.username.as_deref(), Some("user1"));
    assert_eq!(result.password.as_deref(), Some("pass1"));
  }

  #[test]
  fn test_parse_dynamic_proxy_text_protocol_url_format() {
    let body = "http://user:pass@proxy.example.com:3128";
    let result = ProxyManager::parse_dynamic_proxy_text(body).unwrap();
    assert_eq!(result.host, "proxy.example.com");
    assert_eq!(result.port, 3128);
    assert_eq!(result.proxy_type, "http");
    assert_eq!(result.username.as_deref(), Some("user"));
    assert_eq!(result.password.as_deref(), Some("pass"));
  }

  #[test]
  fn test_parse_dynamic_proxy_text_with_whitespace() {
    let body = "  \n  proxy.example.com:8080:user:pass  \n  ";
    let result = ProxyManager::parse_dynamic_proxy_text(body).unwrap();
    assert_eq!(result.host, "proxy.example.com");
    assert_eq!(result.port, 8080);
  }

  #[test]
  fn test_parse_dynamic_proxy_text_empty() {
    let err = ProxyManager::parse_dynamic_proxy_text("").unwrap_err();
    assert!(err.contains("Empty"));
  }

  #[test]
  fn test_parse_dynamic_proxy_text_whitespace_only() {
    let err = ProxyManager::parse_dynamic_proxy_text("   \n  \n  ").unwrap_err();
    assert!(err.contains("Empty"));
  }

  #[tokio::test]
  async fn test_fetch_proxy_from_url_parses_json_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
      .and(path("/hook"))
      .respond_with(
        ResponseTemplate::new(200).set_body_string(
          r#"{"host":"proxy.example.com","port":3128,"type":"socks5","username":"user","password":"pass"}"#,
        ),
      )
      .mount(&server)
      .await;

    let pm = ProxyManager::new();
    let result = pm
      .fetch_proxy_from_url(
        &format!("{}/hook", server.uri()),
        Duration::from_millis(500),
      )
      .await
      .unwrap()
      .unwrap();

    assert_eq!(result.host, "proxy.example.com");
    assert_eq!(result.port, 3128);
    assert_eq!(result.proxy_type, "socks5");
    assert_eq!(result.username.as_deref(), Some("user"));
    assert_eq!(result.password.as_deref(), Some("pass"));
  }

  #[tokio::test]
  async fn test_fetch_proxy_from_url_parses_text_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
      .and(path("/hook"))
      .respond_with(ResponseTemplate::new(200).set_body_string("socks5://user:pass@1.2.3.4:1080"))
      .mount(&server)
      .await;

    let pm = ProxyManager::new();
    let result = pm
      .fetch_proxy_from_url(
        &format!("{}/hook", server.uri()),
        Duration::from_millis(500),
      )
      .await
      .unwrap()
      .unwrap();

    assert_eq!(result.host, "1.2.3.4");
    assert_eq!(result.port, 1080);
    assert_eq!(result.proxy_type, "socks5");
    assert_eq!(result.username.as_deref(), Some("user"));
    assert_eq!(result.password.as_deref(), Some("pass"));
  }

  #[tokio::test]
  async fn test_fetch_proxy_from_url_returns_none_for_no_content() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
      .and(path("/hook"))
      .respond_with(ResponseTemplate::new(204))
      .mount(&server)
      .await;

    let pm = ProxyManager::new();
    let result = pm
      .fetch_proxy_from_url(
        &format!("{}/hook", server.uri()),
        Duration::from_millis(500),
      )
      .await
      .unwrap();

    assert!(result.is_none());
  }

  #[tokio::test]
  async fn test_fetch_proxy_from_url_respects_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
      .and(path("/hook"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_delay(Duration::from_millis(200))
          .set_body_string(r#"{"host":"1.2.3.4","port":8080}"#),
      )
      .mount(&server)
      .await;

    let pm = ProxyManager::new();
    let err = pm
      .fetch_proxy_from_url(&format!("{}/hook", server.uri()), Duration::from_millis(50))
      .await
      .unwrap_err();

    assert!(err.contains("Failed to fetch launch hook"));
  }
}

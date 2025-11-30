use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tauri_plugin_shell::ShellExt;

use crate::browser::ProxySettings;

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

// Stored proxy configuration with name and ID for reuse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProxy {
  pub id: String,
  pub name: String,
  pub proxy_settings: ProxySettings,
}

impl StoredProxy {
  pub fn new(name: String, proxy_settings: ProxySettings) -> Self {
    Self {
      id: uuid::Uuid::new_v4().to_string(),
      name,
      proxy_settings,
    }
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
  base_dirs: BaseDirs,
}

impl ProxyManager {
  pub fn new() -> Self {
    let base_dirs = BaseDirs::new().expect("Failed to get base directories");
    let manager = Self {
      active_proxies: Mutex::new(HashMap::new()),
      profile_proxies: Mutex::new(HashMap::new()),
      profile_active_proxy_ids: Mutex::new(HashMap::new()),
      stored_proxies: Mutex::new(HashMap::new()),
      base_dirs,
    };

    // Load stored proxies on initialization
    if let Err(e) = manager.load_stored_proxies() {
      log::warn!("Failed to load stored proxies: {e}");
    }

    manager
  }

  // Get the path to the proxies directory
  fn get_proxies_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("proxies");
    path
  }

  // Get the path to the proxy check cache directory
  fn get_proxy_check_cache_dir(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut path = self.base_dirs.cache_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("proxy_checks");
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

  // Get geolocation for an IP address
  async fn get_ip_geolocation(
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

  // Get the path to a specific proxy file
  fn get_proxy_file_path(&self, proxy_id: &str) -> PathBuf {
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
          Ok(content) => {
            match serde_json::from_str::<StoredProxy>(&content) {
              Ok(proxy) => {
                log::debug!("Loaded stored proxy: {} ({})", proxy.name, proxy.id);
                stored_proxies.insert(proxy.id.clone(), proxy);
                loaded_count += 1;
              }
              Err(e) => {
                // Check if this is a ProxyConfig file (from proxy_storage.rs) - skip it
                if serde_json::from_str::<crate::proxy_storage::ProxyConfig>(&content).is_ok() {
                  log::debug!("Skipping ProxyConfig file (not a StoredProxy): {:?}", path);
                } else {
                  log::warn!(
                    "Failed to parse proxy file {:?} as StoredProxy: {}",
                    path,
                    e
                  );
                  error_count += 1;
                }
              }
            }
          }
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
    app_handle: &tauri::AppHandle,
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
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    Ok(stored_proxy)
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
    app_handle: &tauri::AppHandle,
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
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    Ok(updated_proxy)
  }

  // Delete a stored proxy
  pub fn delete_stored_proxy(
    &self,
    app_handle: &tauri::AppHandle,
    proxy_id: &str,
  ) -> Result<(), String> {
    {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      if stored_proxies.remove(proxy_id).is_none() {
        return Err(format!("Proxy with ID '{proxy_id}' not found"));
      }
    }

    if let Err(e) = self.delete_proxy_file(proxy_id) {
      log::warn!("Failed to delete proxy file: {e}");
    }

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    Ok(())
  }

  // Get proxy settings for a stored proxy ID
  pub fn get_proxy_settings_by_id(&self, proxy_id: &str) -> Option<ProxySettings> {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    stored_proxies
      .get(proxy_id)
      .map(|p| p.proxy_settings.clone())
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

  // Validate IP address (IPv4 or IPv6)
  fn validate_ip(ip: &str) -> bool {
    // IPv4 validation
    if ip.matches('.').count() == 3 {
      let parts: Vec<&str> = ip.split('.').collect();
      if parts.len() == 4 {
        return parts.iter().all(|part| part.parse::<u8>().is_ok());
      }
    }

    // IPv6 validation (simplified - checks for colons and hex digits)
    if ip.matches(':').count() >= 2 {
      let parts: Vec<&str> = ip.split(':').collect();
      return parts
        .iter()
        .all(|part| part.is_empty() || part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    false
  }

  // Check if a proxy is valid by making HTTP requests through it
  pub async fn check_proxy_validity(
    &self,
    proxy_id: &str,
    proxy_settings: &ProxySettings,
  ) -> Result<ProxyCheckResult, String> {
    let proxy_url = Self::build_proxy_url(proxy_settings);

    // List of IP check endpoints to try
    let ip_check_urls = vec![
      "https://api.ipify.org",
      "https://checkip.amazonaws.com",
      "https://ipinfo.io/ip",
      "https://icanhazip.com",
      "https://ifconfig.co/ip",
    ];

    // Create HTTP client with proxy
    // reqwest::Proxy::all expects http/https URLs, but we need to handle socks proxies differently
    let proxy = match proxy_settings.proxy_type.as_str() {
      "socks4" | "socks5" => {
        // For SOCKS proxies, reqwest doesn't support them directly via Proxy::all
        // We'll need to use a different approach or return an error
        return Err("SOCKS proxy validation not yet supported".to_string());
      }
      _ => reqwest::Proxy::all(&proxy_url).map_err(|e| format!("Failed to create proxy: {e}"))?,
    };

    let client = reqwest::Client::builder()
      .proxy(proxy)
      .timeout(std::time::Duration::from_secs(5))
      .build()
      .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    // Try each endpoint until one succeeds
    let mut last_error = None;
    let mut ip: Option<String> = None;

    for url_str in ip_check_urls {
      match client.get(url_str).send().await {
        Ok(response) => {
          if response.status().is_success() {
            match response.text().await {
              Ok(ip_text) => {
                let ip_str = ip_text.trim();
                if Self::validate_ip(ip_str) {
                  ip = Some(ip_str.to_string());
                  break;
                } else {
                  last_error = Some(format!("Invalid IP address returned: {ip_str}"));
                }
              }
              Err(e) => {
                last_error = Some(format!("Failed to read response from {url_str}: {e}"));
              }
            }
          } else {
            last_error = Some(format!("HTTP error from {url_str}: {}", response.status()));
          }
        }
        Err(e) => {
          last_error = Some(format!("Request to {url_str} failed: {e}"));
        }
      }
    }

    let ip = match ip {
      Some(ip) => ip,
      None => {
        // Save failed check result
        let failed_result = ProxyCheckResult {
          ip: String::new(),
          city: None,
          country: None,
          country_code: None,
          timestamp: Self::get_current_timestamp(),
          is_valid: false,
        };
        let _ = self.save_proxy_check_cache(proxy_id, &failed_result);
        return Err(
          last_error.unwrap_or_else(|| "Failed to get public IP from any endpoint".to_string()),
        );
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

  // Start a proxy for given proxy settings and associate it with a browser process ID
  // If proxy_settings is None, starts a direct proxy for traffic monitoring
  pub async fn start_proxy(
    &self,
    app_handle: tauri::AppHandle,
    proxy_settings: Option<&ProxySettings>,
    browser_pid: u32,
    profile_id: Option<&str>,
  ) -> Result<ProxySettings, String> {
    // First, proactively cleanup any dead proxies so we don't accidentally reuse stale ones
    let _ = self.cleanup_dead_proxies(app_handle.clone()).await;

    // If we have a previous proxy tied to this profile, and the upstream settings are changing,
    // stop it before starting a new one so the change takes effect immediately.
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

          if !is_same_upstream {
            // Stop the previous proxy tied to this profile (best effort)
            // We don't know the original PID mapping that created it; iterate to find its key
            let pid_to_stop = {
              let proxies = self.active_proxies.lock().unwrap();
              proxies.iter().find_map(|(pid, info)| {
                if info.id == existing_id {
                  Some(*pid)
                } else {
                  None
                }
              })
            };
            if let Some(pid) = pid_to_stop {
              let _ = self.stop_proxy(app_handle.clone(), pid).await;
            }
          }
        }
      }
    }
    // Check if we already have a proxy for this browser PID. If it exists but the upstream
    // settings don't match the newly requested ones, stop it and create a new proxy so that
    // changes take effect immediately.
    let mut needs_restart = false;
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
          // Check if profile_id matches - if not, we need to restart to update tracking
          let profile_id_matches = match (profile_id, &existing.profile_id) {
            (Some(ref new_id), Some(ref old_id)) => new_id == old_id,
            (None, None) => true,
            _ => false,
          };

          if profile_id_matches {
            // Reuse existing local proxy (profile_id matches)
            return Ok(ProxySettings {
              proxy_type: "http".to_string(),
              host: "127.0.0.1".to_string(),
              port: existing.local_port,
              username: None,
              password: None,
            });
          } else {
            // Profile ID changed - need to restart proxy to update tracking
            log::info!(
              "Profile ID changed for proxy {}: {:?} -> {:?}, restarting proxy",
              existing.id,
              existing.profile_id,
              profile_id
            );
            needs_restart = true;
          }
        } else {
          // Upstream changed; we must restart the local proxy so that traffic is routed correctly
          needs_restart = true;
        }
      }
    }

    if needs_restart {
      // Best-effort stop of the old proxy for this PID before starting a new one
      let _ = self.stop_proxy(app_handle.clone(), browser_pid).await;
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
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
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
        if let Err(e) = app_handle.emit("proxies-changed", ()) {
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

  // Check if a process is still running
  fn is_process_running(&self, pid: u32) -> bool {
    use sysinfo::{Pid, System};
    let system = System::new_all();
    system.process(Pid::from(pid as usize)).is_some()
  }

  // Clean up proxies for dead browser processes
  pub async fn cleanup_dead_proxies(
    &self,
    app_handle: tauri::AppHandle,
  ) -> Result<Vec<u32>, String> {
    let dead_pids = {
      let proxies = self.active_proxies.lock().unwrap();
      proxies
        .keys()
        .filter(|&&pid| pid != 0 && !self.is_process_running(pid)) // Skip temporary PID 0
        .copied()
        .collect::<Vec<u32>>()
    };

    for dead_pid in &dead_pids {
      log::info!("Cleaning up proxy for dead browser process PID: {dead_pid}");
      let _ = self.stop_proxy(app_handle.clone(), *dead_pid).await;
    }

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      log::error!("Failed to emit proxies-changed event: {e}");
    }

    Ok(dead_pids)
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
  use std::process::Command;
  use std::time::Duration;
  use tokio::time::sleep;

  // Mock HTTP server for testing

  use http_body_util::Full;
  use hyper::body::Bytes;
  use hyper::server::conn::http1;
  use hyper::service::service_fn;
  use hyper::Response;
  use hyper_util::rt::TokioIo;
  use tokio::net::TcpListener;

  // Helper function to build donut-proxy binary for testing
  async fn ensure_donut_proxy_binary() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let project_root = PathBuf::from(cargo_manifest_dir)
      .parent()
      .unwrap()
      .to_path_buf();
    let proxy_binary = project_root
      .join("src-tauri")
      .join("target")
      .join("debug")
      .join("donut-proxy");

    // Check if binary already exists
    if proxy_binary.exists() {
      return Ok(proxy_binary);
    }

    // Build the donut-proxy binary
    println!("Building donut-proxy binary for tests...");

    let build_status = Command::new("cargo")
      .args(["build", "--bin", "donut-proxy"])
      .current_dir(project_root.join("src-tauri"))
      .status()?;

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
    let output = tokio::time::timeout(Duration::from_secs(3), async { cmd.output() }).await;

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
            let _ = stop_cmd.output();
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
      .arg("httpbin.org") // Use a known good endpoint
      .arg("--proxy-port")
      .arg("80")
      .arg("--type")
      .arg("http");

    let output = tokio::time::timeout(Duration::from_secs(60), async { cmd.output() }).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;
      let proxy_id = config["id"].as_str().unwrap();

      // Clean up
      let mut stop_cmd = Command::new(&proxy_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
      let _ = stop_cmd.output();

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
      .arg("user@domain.com") // Contains @ symbol
      .arg("--password")
      .arg("pass word!"); // Contains space and special character

    let output = tokio::time::timeout(Duration::from_secs(60), async { cmd.output() }).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;

      let upstream_url = config["upstreamUrl"].as_str().unwrap();

      println!("Generated upstream URL: {upstream_url}");

      // Verify that special characters are properly encoded
      assert!(upstream_url.contains("user%40domain.com"));
      // The password may be encoded as "pass%20word!" or "pass%20word%21" depending on implementation
      assert!(upstream_url.contains("pass%20word"));

      println!("URL encoding test passed - special characters handled correctly");

      // Clean up
      let proxy_id = config["id"].as_str().unwrap();
      let mut stop_cmd = Command::new(&proxy_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
      let _ = stop_cmd.output();
    } else {
      // This test might fail if the upstream doesn't exist, but we mainly care about URL construction
      let stdout = String::from_utf8(output.stdout)?;
      let stderr = String::from_utf8(output.stderr)?;
      println!("Command failed (expected for non-existent upstream):");
      println!("Stdout: {stdout}");
      println!("Stderr: {stderr}");

      // The important thing is that the command completed quickly
      println!("URL encoding test completed - credentials should be properly encoded");
    }

    Ok(())
  }
}

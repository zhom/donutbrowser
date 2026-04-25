use aes_gcm::{
  aead::{Aead, AeadCore, KeyInit, OsRng},
  Aes256Gcm, Key, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use chrono::Utc;
use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::browser::ProxySettings;
use crate::proxy_manager::PROXY_MANAGER;
use crate::settings_manager::SettingsManager;
use crate::sync;

pub const CLOUD_API_URL: &str = "https://api.donutbrowser.com";
pub const CLOUD_SYNC_URL: &str = "https://sync.donutbrowser.com";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudUser {
  pub id: String,
  pub email: String,
  pub plan: String,
  #[serde(rename = "planPeriod")]
  pub plan_period: Option<String>,
  #[serde(rename = "subscriptionStatus")]
  pub subscription_status: String,
  #[serde(rename = "profileLimit")]
  pub profile_limit: i64,
  #[serde(rename = "cloudProfilesUsed")]
  pub cloud_profiles_used: i64,
  #[serde(rename = "proxyBandwidthLimitMb")]
  pub proxy_bandwidth_limit_mb: i64,
  #[serde(rename = "proxyBandwidthUsedMb")]
  pub proxy_bandwidth_used_mb: i64,
  #[serde(rename = "proxyBandwidthExtraMb", default)]
  pub proxy_bandwidth_extra_mb: i64,
  #[serde(rename = "teamId", default)]
  pub team_id: Option<String>,
  #[serde(rename = "teamName", default)]
  pub team_name: Option<String>,
  #[serde(rename = "teamRole", default)]
  pub team_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudAuthState {
  pub user: CloudUser,
  pub logged_in_at: String,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeChallengeResponse {
  #[serde(rename = "challengeId")]
  challenge_id: String,
  prefix: String,
  difficulty: u32,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeExchangeResponse {
  #[serde(rename = "accessToken")]
  access_token: String,
  #[serde(rename = "refreshToken")]
  refresh_token: String,
  user: CloudUser,
}

#[derive(Debug, Deserialize)]
struct RefreshTokenResponse {
  #[serde(rename = "accessToken")]
  access_token: String,
  #[serde(rename = "refreshToken")]
  refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct SyncTokenResponse {
  #[serde(rename = "syncToken")]
  sync_token: String,
}

#[derive(Debug, Deserialize)]
struct WayfernTokenResponse {
  token: String,
  #[serde(rename = "expiresIn")]
  #[allow(dead_code)]
  expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationItem {
  pub code: String,
  pub name: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CloudProxyConfigResponse {
  host: String,
  port: u16,
  username: Option<String>,
  password: Option<String>,
  protocol: String,
  #[serde(rename = "bandwidthLimitMb")]
  bandwidth_limit_mb: i64,
  #[serde(rename = "bandwidthUsedMb")]
  bandwidth_used_mb: i64,
}

pub struct CloudAuthManager {
  client: Client,
  state: Mutex<Option<CloudAuthState>>,
  refresh_lock: tokio::sync::Mutex<()>,
  wayfern_token: Mutex<Option<String>>,
}

lazy_static! {
  pub static ref CLOUD_AUTH: CloudAuthManager = CloudAuthManager::new();
}

impl CloudAuthManager {
  fn new() -> Self {
    let state = Self::load_auth_state_from_disk();
    Self {
      client: Client::new(),
      state: Mutex::new(state),
      refresh_lock: tokio::sync::Mutex::new(()),
      wayfern_token: Mutex::new(None),
    }
  }

  // --- Settings directory (reuse SettingsManager path) ---

  fn get_settings_dir() -> PathBuf {
    SettingsManager::instance().get_settings_dir()
  }

  fn get_vault_password() -> String {
    env!("DONUT_BROWSER_VAULT_PASSWORD").to_string()
  }

  // --- Encrypted file storage (same pattern as settings_manager.rs) ---

  fn encrypt_and_store(file_path: &PathBuf, header: &[u8; 5], data: &str) -> Result<(), String> {
    if let Some(parent) = file_path.parent() {
      fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    let vault_password = Self::get_vault_password();
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
      .hash_password(vault_password.as_bytes(), &salt)
      .map_err(|e| format!("Argon2 key derivation failed: {e}"))?;
    let hash_value = password_hash.hash.unwrap();
    let hash_bytes = hash_value.as_bytes();
    let key_bytes: [u8; 32] = hash_bytes[..32]
      .try_into()
      .map_err(|_| "Invalid key length".to_string())?;
    let key = Key::<Aes256Gcm>::from(key_bytes);
    let cipher = Aes256Gcm::new(&key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
      .encrypt(&nonce, data.as_bytes())
      .map_err(|e| format!("Encryption failed: {e}"))?;

    let mut file_data = Vec::new();
    file_data.extend_from_slice(header);
    file_data.push(2u8);
    let salt_str = salt.as_str();
    file_data.push(salt_str.len() as u8);
    file_data.extend_from_slice(salt_str.as_bytes());
    file_data.extend_from_slice(&nonce);
    file_data.extend_from_slice(&(ciphertext.len() as u32).to_le_bytes());
    file_data.extend_from_slice(&ciphertext);

    fs::write(file_path, file_data).map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(())
  }

  fn decrypt_from_file(file_path: &PathBuf, header: &[u8; 5]) -> Result<Option<String>, String> {
    if !file_path.exists() {
      return Ok(None);
    }

    let file_data = fs::read(file_path).map_err(|e| format!("Failed to read file: {e}"))?;

    if file_data.len() < 6 || &file_data[0..5] != header {
      return Ok(None);
    }

    let version = file_data[5];
    if version != 2 {
      return Ok(None);
    }

    let mut offset = 6;
    if offset >= file_data.len() {
      return Ok(None);
    }
    let salt_len = file_data[offset] as usize;
    offset += 1;

    if offset + salt_len > file_data.len() {
      return Ok(None);
    }
    let salt_bytes = &file_data[offset..offset + salt_len];
    let salt_str = std::str::from_utf8(salt_bytes).map_err(|_| "Invalid salt encoding")?;
    let salt = SaltString::from_b64(salt_str).map_err(|_| "Invalid salt format")?;
    offset += salt_len;

    if offset + 12 > file_data.len() {
      return Ok(None);
    }
    let nonce_bytes: [u8; 12] = file_data[offset..offset + 12]
      .try_into()
      .map_err(|_| "Invalid nonce length".to_string())?;
    let nonce = Nonce::from(nonce_bytes);
    offset += 12;

    if offset + 4 > file_data.len() {
      return Ok(None);
    }
    let ciphertext_len = u32::from_le_bytes([
      file_data[offset],
      file_data[offset + 1],
      file_data[offset + 2],
      file_data[offset + 3],
    ]) as usize;
    offset += 4;

    if offset + ciphertext_len > file_data.len() {
      return Ok(None);
    }
    let ciphertext = &file_data[offset..offset + ciphertext_len];

    let vault_password = Self::get_vault_password();
    let argon2 = Argon2::default();
    let password_hash = argon2
      .hash_password(vault_password.as_bytes(), &salt)
      .map_err(|e| format!("Argon2 key derivation failed: {e}"))?;
    let hash_value = password_hash.hash.unwrap();
    let hash_bytes = hash_value.as_bytes();
    let key_bytes: [u8; 32] = hash_bytes[..32]
      .try_into()
      .map_err(|_| "Invalid key length".to_string())?;
    let key = Key::<Aes256Gcm>::from(key_bytes);
    let cipher = Aes256Gcm::new(&key);
    let plaintext = cipher
      .decrypt(&nonce, ciphertext)
      .map_err(|_| "Decryption failed".to_string())?;

    match String::from_utf8(plaintext) {
      Ok(token) => Ok(Some(token)),
      Err(_) => Ok(None),
    }
  }

  // --- Token storage methods ---

  fn store_access_token(token: &str) -> Result<(), String> {
    let path = Self::get_settings_dir().join("cloud_access_token.dat");
    Self::encrypt_and_store(&path, b"DBCAT", token)
  }

  pub(crate) fn load_access_token() -> Result<Option<String>, String> {
    let path = Self::get_settings_dir().join("cloud_access_token.dat");
    Self::decrypt_from_file(&path, b"DBCAT")
  }

  fn store_refresh_token(token: &str) -> Result<(), String> {
    let path = Self::get_settings_dir().join("cloud_refresh_token.dat");
    Self::encrypt_and_store(&path, b"DBCRT", token)
  }

  fn load_refresh_token() -> Result<Option<String>, String> {
    let path = Self::get_settings_dir().join("cloud_refresh_token.dat");
    Self::decrypt_from_file(&path, b"DBCRT")
  }

  fn store_cloud_sync_token(token: &str) -> Result<(), String> {
    let path = Self::get_settings_dir().join("cloud_sync_token.dat");
    Self::encrypt_and_store(&path, b"DBCST", token)
  }

  fn load_cloud_sync_token() -> Result<Option<String>, String> {
    let path = Self::get_settings_dir().join("cloud_sync_token.dat");
    Self::decrypt_from_file(&path, b"DBCST")
  }

  fn store_auth_state(state: &CloudAuthState) -> Result<(), String> {
    let path = Self::get_settings_dir().join("cloud_auth_state.json");
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    }
    let json =
      serde_json::to_string_pretty(state).map_err(|e| format!("Failed to serialize: {e}"))?;
    fs::write(path, json).map_err(|e| format!("Failed to write auth state: {e}"))?;
    Ok(())
  }

  fn load_auth_state_from_disk() -> Option<CloudAuthState> {
    let path = Self::get_settings_dir().join("cloud_auth_state.json");
    if !path.exists() {
      return None;
    }
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
  }

  fn delete_all_cloud_files() {
    let dir = Self::get_settings_dir();
    let files = [
      "cloud_access_token.dat",
      "cloud_refresh_token.dat",
      "cloud_sync_token.dat",
      "cloud_auth_state.json",
    ];
    for f in &files {
      let path = dir.join(f);
      if path.exists() {
        let _ = fs::remove_file(path);
      }
    }
  }

  // --- JWT expiry check ---

  fn is_jwt_expiring_soon(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
      return true;
    }

    use base64::{engine::general_purpose, Engine as _};
    let payload = match general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
      Ok(bytes) => bytes,
      Err(_) => {
        // Try standard base64 with padding
        match general_purpose::STANDARD.decode(parts[1]) {
          Ok(bytes) => bytes,
          Err(_) => return true,
        }
      }
    };

    let json: serde_json::Value = match serde_json::from_slice(&payload) {
      Ok(v) => v,
      Err(_) => return true,
    };

    let exp = match json.get("exp").and_then(|v| v.as_i64()) {
      Some(exp) => exp,
      None => return true,
    };

    let now = Utc::now().timestamp();
    exp - now < 120
  }

  // --- API methods ---

  pub async fn exchange_device_code(&self, code: &str) -> Result<CloudAuthState, String> {
    let challenge_url = format!("{CLOUD_API_URL}/api/auth/device-code/challenge");
    let challenge_response = self
      .client
      .post(&challenge_url)
      .send()
      .await
      .map_err(|e| format!("Failed to fetch challenge: {e}"))?;

    if !challenge_response.status().is_success() {
      let status = challenge_response.status();
      let body = challenge_response.text().await.unwrap_or_default();
      return Err(format!("Challenge request failed ({status}): {body}"));
    }

    let challenge: DeviceCodeChallengeResponse = challenge_response
      .json()
      .await
      .map_err(|e| format!("Failed to parse challenge: {e}"))?;

    let nonce = solve_pow(&challenge.prefix, challenge.difficulty)
      .ok_or_else(|| "Failed to solve proof-of-work".to_string())?;

    let exchange_url = format!("{CLOUD_API_URL}/api/auth/device-code/exchange");
    let response = self
      .client
      .post(&exchange_url)
      .json(&serde_json::json!({
        "code": code,
        "challengeId": challenge.challenge_id,
        "nonce": nonce,
      }))
      .send()
      .await
      .map_err(|e| format!("Failed to verify code: {e}"))?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(format!("Login failed ({status}): {body}"));
    }

    let result: DeviceCodeExchangeResponse = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse response: {e}"))?;

    // Store tokens
    log::info!(
      "Storing access token (len={}) and refresh token (len={})",
      result.access_token.len(),
      result.refresh_token.len()
    );
    Self::store_access_token(&result.access_token)?;
    Self::store_refresh_token(&result.refresh_token)?;

    // Verify tokens survived the encrypt/decrypt round-trip
    match Self::load_access_token() {
      Ok(Some(loaded)) if loaded == result.access_token => {
        log::info!(
          "Access token verified after store/load (len={})",
          loaded.len()
        );
      }
      Ok(Some(loaded)) => {
        log::error!(
          "Access token CORRUPTED during store/load: original_len={}, loaded_len={}",
          result.access_token.len(),
          loaded.len()
        );
      }
      Ok(None) => {
        log::error!("Access token missing immediately after store");
      }
      Err(e) => {
        log::error!("Failed to load access token for verification: {e}");
      }
    }

    // Build and persist auth state
    let auth_state = CloudAuthState {
      user: result.user,
      logged_in_at: Utc::now().to_rfc3339(),
    };
    Self::store_auth_state(&auth_state)?;

    log::info!(
      "Login successful: plan={}, subscription_status={}, proxy_bandwidth_limit={}MB",
      auth_state.user.plan,
      auth_state.user.subscription_status,
      auth_state.user.proxy_bandwidth_limit_mb
    );

    // Update in-memory state
    let mut state = self.state.lock().await;
    *state = Some(auth_state.clone());

    Ok(auth_state)
  }

  pub async fn refresh_access_token(&self) -> Result<(), String> {
    let _guard = self.refresh_lock.lock().await;
    log::info!("Refreshing access token (holding lock)...");

    let refresh_token =
      Self::load_refresh_token()?.ok_or_else(|| "No refresh token stored".to_string())?;

    let url = format!("{CLOUD_API_URL}/api/auth/token/refresh");
    let response = self
      .client
      .post(&url)
      .json(&serde_json::json!({ "refreshToken": refresh_token }))
      .send()
      .await
      .map_err(|e| format!("Failed to refresh token: {e}"))?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      log::warn!("Token refresh failed ({status}): {body}");
      return Err(format!("Token refresh failed ({status}): {body}"));
    }

    let result: RefreshTokenResponse = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse response: {e}"))?;

    Self::store_access_token(&result.access_token)?;
    Self::store_refresh_token(&result.refresh_token)?;

    log::info!("Access token refreshed successfully");
    Ok(())
  }

  /// Invalidate the session: clear all auth state and notify the frontend.
  /// Only call this when the session is definitively dead (explicit logout
  /// or repeated background refresh failures).
  pub async fn invalidate_session(&self) {
    log::warn!("Invalidating session — clearing all auth state");
    PROXY_MANAGER.remove_cloud_proxy();
    self.clear_auth().await;
    let _ = crate::events::emit_empty("cloud-auth-expired");
  }

  pub async fn fetch_profile(&self) -> Result<CloudUser, String> {
    let user = self
      .api_call_with_retry(|access_token| {
        let url = format!("{CLOUD_API_URL}/api/auth/me");
        let client = self.client.clone();
        async move {
          let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch profile: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Profile fetch failed ({status}): {body}"));
          }

          response
            .json::<CloudUser>()
            .await
            .map_err(|e| format!("Failed to parse profile: {e}"))
        }
      })
      .await?;

    // Update cached state
    let mut state = self.state.lock().await;
    if let Some(auth_state) = state.as_mut() {
      auth_state.user = user.clone();
      let _ = Self::store_auth_state(auth_state);
    }

    Ok(user)
  }

  pub async fn get_or_refresh_sync_token(&self) -> Result<Option<String>, String> {
    if !self.is_logged_in().await {
      return Ok(None);
    }

    // Check cached sync token
    if let Ok(Some(token)) = Self::load_cloud_sync_token() {
      if !Self::is_jwt_expiring_soon(&token) {
        return Ok(Some(token));
      }
    }

    // Fetch new sync token
    let sync_token = self
      .api_call_with_retry(|access_token| {
        let url = format!("{CLOUD_API_URL}/api/auth/sync-token");
        let client = self.client.clone();
        async move {
          let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to get sync token: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Sync token request failed ({status}): {body}"));
          }

          let result: SyncTokenResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse sync token response: {e}"))?;

          Ok(result.sync_token)
        }
      })
      .await?;

    Self::store_cloud_sync_token(&sync_token)?;
    Ok(Some(sync_token))
  }

  pub async fn logout(&self) -> Result<(), String> {
    // Clear wayfern token
    self.clear_wayfern_token().await;

    // Disconnect profile lock manager
    crate::team_lock::PROFILE_LOCK.disconnect().await;

    // Try to call the logout API (best-effort)
    if let Ok(Some(access_token)) = Self::load_access_token() {
      let refresh_token = Self::load_refresh_token().ok().flatten();
      let url = format!("{CLOUD_API_URL}/api/auth/logout");
      let mut body = serde_json::json!({});
      if let Some(rt) = &refresh_token {
        body = serde_json::json!({ "refreshToken": rt });
      }
      let _ = self
        .client
        .post(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .json(&body)
        .send()
        .await;
    }

    // Remove cloud proxy on logout
    PROXY_MANAGER.remove_cloud_proxy();

    self.clear_auth().await;
    Ok(())
  }

  pub async fn is_logged_in(&self) -> bool {
    let state = self.state.lock().await;
    state.is_some()
  }

  pub async fn has_active_paid_subscription(&self) -> bool {
    let state = self.state.lock().await;
    match &*state {
      Some(auth) => {
        auth.user.plan != "free"
          && (auth.user.subscription_status == "active"
            || auth.user.plan_period.as_deref() == Some("lifetime"))
      }
      None => false,
    }
  }

  /// Non-async version that uses try_lock, defaults to false if lock can't be acquired.
  pub fn has_active_paid_subscription_sync(&self) -> bool {
    match self.state.try_lock() {
      Ok(state) => match &*state {
        Some(auth) => {
          auth.user.plan != "free"
            && (auth.user.subscription_status == "active"
              || auth.user.plan_period.as_deref() == Some("lifetime"))
        }
        None => false,
      },
      Err(_) => false,
    }
  }

  pub async fn is_fingerprint_os_allowed(&self, fingerprint_os: Option<&str>) -> bool {
    let host_os = crate::profile::types::get_host_os();
    match fingerprint_os {
      None => true,
      Some(os) if os == host_os => true,
      Some(_) => self.has_active_paid_subscription().await,
    }
  }

  pub async fn is_on_team_plan(&self) -> bool {
    if let Some(state) = self.get_user().await {
      return state.user.team_id.is_some();
    }
    false
  }

  pub async fn get_user(&self) -> Option<CloudAuthState> {
    let state = self.state.lock().await;
    state.clone()
  }

  async fn clear_auth(&self) {
    let mut state = self.state.lock().await;
    *state = None;
    Self::delete_all_cloud_files();
  }

  /// API call with 401 retry: if first attempt gets 401, refresh access token and retry once.
  /// Uses refresh_lock to prevent concurrent token rotations from racing.
  pub async fn api_call_with_retry<F, Fut, T>(&self, make_request: F) -> Result<T, String>
  where
    F: Fn(String) -> Fut + Send,
    Fut: std::future::Future<Output = Result<T, String>> + Send,
  {
    let access_token = Self::load_access_token()?.ok_or_else(|| "Not logged in".to_string())?;

    match make_request(access_token.clone()).await {
      Ok(result) => Ok(result),
      Err(e) if e.contains("(401") || e.contains("Unauthorized") => {
        log::info!("Got 401/Unauthorized response, attempting token refresh...");

        // Check if another caller already refreshed while we waited
        let current_token = Self::load_access_token()?.unwrap_or_default();
        if current_token != access_token && !current_token.is_empty() {
          log::info!("Token was already refreshed by another caller, retrying...");
          return make_request(current_token).await;
        }

        self.refresh_access_token().await?;
        let new_token =
          Self::load_access_token()?.ok_or_else(|| "Not logged in after refresh".to_string())?;
        log::info!("Token refreshed, retrying request...");
        make_request(new_token).await
      }
      Err(e) => Err(e),
    }
  }

  /// Fetch proxy configuration from the cloud backend
  async fn fetch_proxy_config(&self) -> Result<Option<CloudProxyConfigResponse>, String> {
    // Check cached user state for proxy bandwidth (subscription or extra)
    {
      let state = self.state.lock().await;
      match &*state {
        Some(auth)
          if auth.user.proxy_bandwidth_limit_mb > 0 || auth.user.proxy_bandwidth_extra_mb > 0 => {}
        _ => return Ok(None),
      }
    }

    match self
      .api_call_with_retry(|access_token| {
        let url = format!("{CLOUD_API_URL}/api/proxy/config");
        let client = self.client.clone();
        async move {
          let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch proxy config: {e}"))?;

          let status = response.status();
          if status == reqwest::StatusCode::FORBIDDEN {
            let body = response.text().await.unwrap_or_default();
            log::warn!("Proxy config returned 403: {body}");
            return Err("__403__".to_string());
          }

          if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Proxy config fetch failed ({status}): {body}"));
          }

          response
            .json::<CloudProxyConfigResponse>()
            .await
            .map_err(|e| format!("Failed to parse proxy config: {e}"))
        }
      })
      .await
    {
      Ok(config) => Ok(Some(config)),
      Err(e) if e.contains("__403__") => Ok(None),
      Err(e) => {
        log::warn!("Failed to fetch cloud proxy config: {e}");
        Ok(None)
      }
    }
  }

  /// Sync the cloud-managed proxy: fetch config and upsert or remove
  pub async fn sync_cloud_proxy(&self) {
    log::info!("Syncing cloud proxy configuration...");
    match self.fetch_proxy_config().await {
      Ok(Some(config)) => {
        log::info!(
          "Cloud proxy config received: host={}, port={}, protocol={}",
          config.host,
          config.port,
          config.protocol
        );
        let settings = ProxySettings {
          proxy_type: config.protocol,
          host: config.host,
          port: config.port,
          username: config.username,
          password: config.password,
        };
        match PROXY_MANAGER.upsert_cloud_proxy(settings) {
          Ok(_) => {
            log::info!("Cloud proxy synced successfully");
            // Propagate credential changes to derived location proxies
            PROXY_MANAGER.update_cloud_derived_proxies();
          }
          Err(e) => log::warn!("Failed to upsert cloud proxy: {e}"),
        }
      }
      Ok(None) => {
        log::info!("No cloud proxy config available (user may not have proxy bandwidth)");
        PROXY_MANAGER.remove_cloud_proxy();
      }
      Err(e) => {
        log::error!("Failed to sync cloud proxy: {e}");
      }
    }
  }

  /// Report the number of sync-enabled profiles to the cloud backend
  pub async fn report_sync_profile_count(&self, count: i64) -> Result<(), String> {
    self
      .api_call_with_retry(|access_token| {
        let url = format!("{CLOUD_API_URL}/api/auth/sync-profile-usage");
        let client = reqwest::Client::new();
        async move {
          let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&serde_json::json!({ "count": count }))
            .send()
            .await
            .map_err(|e| format!("Failed to report profile usage: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Profile usage report failed ({status}): {body}"));
          }

          Ok(())
        }
      })
      .await
  }

  /// Fetch country list from the cloud backend
  pub async fn fetch_countries(&self) -> Result<Vec<LocationItem>, String> {
    self
      .api_call_with_retry(|access_token| {
        let url = format!("{CLOUD_API_URL}/api/proxy/locations/countries");
        let client = self.client.clone();
        async move {
          let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch countries: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Countries fetch failed ({status}): {body}"));
          }

          response
            .json::<Vec<LocationItem>>()
            .await
            .map_err(|e| format!("Failed to parse countries: {e}"))
        }
      })
      .await
  }

  /// Fetch region list for a country from the cloud backend
  pub async fn fetch_regions(&self, country: &str) -> Result<Vec<LocationItem>, String> {
    let country = country.to_string();
    self
      .api_call_with_retry(move |access_token| {
        let url = format!(
          "{CLOUD_API_URL}/api/proxy/locations/regions?country={}",
          country
        );
        let client = reqwest::Client::new();
        async move {
          let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch regions: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Regions fetch failed ({status}): {body}"));
          }

          response
            .json::<Vec<LocationItem>>()
            .await
            .map_err(|e| format!("Failed to parse regions: {e}"))
        }
      })
      .await
  }

  /// Fetch city list for a country, optionally filtered by region
  pub async fn fetch_cities(
    &self,
    country: &str,
    region: Option<&str>,
  ) -> Result<Vec<LocationItem>, String> {
    let country = country.to_string();
    let region = region.map(|s| s.to_string());
    self
      .api_call_with_retry(move |access_token| {
        let mut url = format!(
          "{CLOUD_API_URL}/api/proxy/locations/cities?country={}",
          country
        );
        if let Some(ref r) = region {
          url.push_str(&format!("&region={}", r));
        }
        let client = reqwest::Client::new();
        async move {
          let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch cities: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Cities fetch failed ({status}): {body}"));
          }

          response
            .json::<Vec<LocationItem>>()
            .await
            .map_err(|e| format!("Failed to parse cities: {e}"))
        }
      })
      .await
  }

  /// Fetch ISP list for a country, optionally filtered by region and city
  pub async fn fetch_isps(
    &self,
    country: &str,
    region: Option<&str>,
    city: Option<&str>,
  ) -> Result<Vec<LocationItem>, String> {
    let country = country.to_string();
    let region = region.map(|s| s.to_string());
    let city = city.map(|s| s.to_string());
    self
      .api_call_with_retry(move |access_token| {
        let mut url = format!(
          "{CLOUD_API_URL}/api/proxy/locations/isps?country={}",
          country
        );
        if let Some(ref r) = region {
          url.push_str(&format!("&region={}", r));
        }
        if let Some(ref c) = city {
          url.push_str(&format!("&city={}", c));
        }
        let client = reqwest::Client::new();
        async move {
          let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch ISPs: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("ISPs fetch failed ({status}): {body}"));
          }

          response
            .json::<Vec<LocationItem>>()
            .await
            .map_err(|e| format!("Failed to parse ISPs: {e}"))
        }
      })
      .await
  }

  /// Request a wayfern token from the cloud API. Only succeeds for paid users.
  pub async fn request_wayfern_token(&self) -> Result<(), String> {
    if !self.has_active_paid_subscription().await {
      self.clear_wayfern_token().await;
      return Ok(());
    }

    let token = self
      .api_call_with_retry(|access_token| {
        let url = format!("{CLOUD_API_URL}/api/auth/wayfern-start");
        let client = reqwest::Client::new();
        async move {
          let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to request wayfern token: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Wayfern token request failed ({status}): {body}"));
          }

          let result: WayfernTokenResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse wayfern token response: {e}"))?;

          Ok(result.token)
        }
      })
      .await?;

    let mut wt = self.wayfern_token.lock().await;
    *wt = Some(token);
    log::info!("Wayfern token acquired");
    Ok(())
  }

  /// Get the current wayfern token, if any.
  pub async fn get_wayfern_token(&self) -> Option<String> {
    let wt = self.wayfern_token.lock().await;
    wt.clone()
  }

  /// Clear the cached wayfern token.
  pub async fn clear_wayfern_token(&self) {
    let mut wt = self.wayfern_token.lock().await;
    *wt = None;
  }

  /// Background loop that refreshes the sync token periodically
  pub async fn start_sync_token_refresh_loop(app_handle: tauri::AppHandle) {
    let mut wayfern_refresh_counter: u32 = 0;
    loop {
      tokio::time::sleep(std::time::Duration::from_secs(600)).await; // 10 minutes

      if !CLOUD_AUTH.is_logged_in().await {
        continue;
      }

      wayfern_refresh_counter += 1;

      // Proactively refresh the access token if it's expired or expiring soon.
      // This runs first so subsequent API calls use a fresh token.
      if let Ok(Some(token)) = Self::load_access_token() {
        if Self::is_jwt_expiring_soon(&token) {
          if let Err(e) = CLOUD_AUTH.refresh_access_token().await {
            log::warn!("Failed to refresh cloud access token: {e}");
            // If the refresh token itself was rejected, session is irrecoverable
            if e.contains("(401") || e.contains("Unauthorized") {
              log::warn!("Refresh token rejected — invalidating session");
              CLOUD_AUTH.invalidate_session().await;
              continue;
            }
          }
        }
      }

      match CLOUD_AUTH.get_or_refresh_sync_token().await {
        Ok(Some(_)) => {
          log::debug!("Cloud sync token refreshed successfully");
        }
        Ok(None) => {}
        Err(e) => {
          log::warn!("Failed to refresh cloud sync token: {e}");
        }
      }

      // Refresh profile data periodically
      if let Err(e) = CLOUD_AUTH.fetch_profile().await {
        log::debug!("Failed to refresh cloud profile: {e}");
      }

      // Reconnect profile lock manager if needed
      if let Some(auth_state) = CLOUD_AUTH.get_user().await {
        if auth_state.user.plan != "free" && !crate::team_lock::PROFILE_LOCK.is_connected().await {
          crate::team_lock::PROFILE_LOCK.connect().await;
        }
      }

      // Sync cloud proxy credentials
      CLOUD_AUTH.sync_cloud_proxy().await;

      // Refresh wayfern token every 10 hours (60 iterations of 10-minute loop)
      if wayfern_refresh_counter >= 60 {
        wayfern_refresh_counter = 0;
        if CLOUD_AUTH.has_active_paid_subscription().await {
          if let Err(e) = CLOUD_AUTH.request_wayfern_token().await {
            log::warn!("Failed to refresh wayfern token: {e}");
          }
        } else {
          CLOUD_AUTH.clear_wayfern_token().await;
        }
      }

      let _ = &app_handle; // keep app_handle alive
    }
  }
}

fn solve_pow(prefix: &str, difficulty: u32) -> Option<String> {
  if difficulty == 0 || difficulty > 32 {
    return None;
  }
  let prefix_bytes = prefix.as_bytes();
  let mut buf = Vec::with_capacity(prefix_bytes.len() + 24);
  for nonce in 0u64..u64::MAX {
    buf.clear();
    buf.extend_from_slice(prefix_bytes);
    let nonce_str = nonce.to_string();
    buf.extend_from_slice(nonce_str.as_bytes());
    let digest = Sha256::digest(&buf);
    if has_leading_zero_bits(&digest, difficulty) {
      return Some(nonce_str);
    }
  }
  None
}

fn has_leading_zero_bits(digest: &[u8], bits: u32) -> bool {
  let full_bytes = (bits / 8) as usize;
  if digest.len() < full_bytes + 1 {
    return false;
  }
  for &b in &digest[..full_bytes] {
    if b != 0 {
      return false;
    }
  }
  let remainder = bits % 8;
  if remainder == 0 {
    return true;
  }
  let mask = 0xffu8 << (8 - remainder);
  (digest[full_bytes] & mask) == 0
}

// --- Tauri commands ---

#[tauri::command]
pub async fn cloud_exchange_device_code(
  app_handle: tauri::AppHandle,
  code: String,
) -> Result<CloudAuthState, String> {
  let state = CLOUD_AUTH.exchange_device_code(&code).await?;

  let has_subscription = CLOUD_AUTH.has_active_paid_subscription().await;
  log::info!(
    "Post-login: plan={}, has_active_subscription={}",
    state.user.plan,
    has_subscription
  );

  // Pre-fetch sync token so sync can start immediately
  if has_subscription {
    log::info!("Pre-fetching sync token...");
    match CLOUD_AUTH.get_or_refresh_sync_token().await {
      Ok(Some(_)) => log::info!("Sync token pre-fetched successfully"),
      Ok(None) => log::warn!("Sync token not available despite active subscription"),
      Err(e) => log::error!("Failed to pre-fetch sync token after login: {e}"),
    }

    // Request wayfern token for paid users
    if let Err(e) = CLOUD_AUTH.request_wayfern_token().await {
      log::warn!("Failed to request wayfern token after login: {e}");
    }
  }

  // Sync cloud proxy after login
  CLOUD_AUTH.sync_cloud_proxy().await;

  // Connect profile lock manager for paid users
  if state.user.plan != "free" {
    crate::team_lock::PROFILE_LOCK.connect().await;
  }

  let _ = crate::events::emit_empty("cloud-auth-changed");

  let _ = &app_handle;
  Ok(state)
}

#[tauri::command]
pub async fn cloud_get_user() -> Result<Option<CloudAuthState>, String> {
  Ok(CLOUD_AUTH.get_user().await)
}

#[tauri::command]
pub async fn cloud_refresh_profile() -> Result<CloudUser, String> {
  CLOUD_AUTH.fetch_profile().await
}

#[tauri::command]
pub async fn cloud_logout(app_handle: tauri::AppHandle) -> Result<(), String> {
  CLOUD_AUTH.logout().await?;

  // Clear sync settings if they point to the cloud URL (prevent leak into Self-Hosted tab)
  let manager = crate::settings_manager::SettingsManager::instance();
  if let Ok(sync_settings) = manager.get_sync_settings() {
    if sync_settings.sync_server_url.as_deref() == Some(CLOUD_SYNC_URL) {
      let _ = manager.save_sync_server_url(None);
    }
  }
  let _ = manager.remove_sync_token(&app_handle).await;

  // Remove cloud-managed and cloud-derived proxies
  crate::proxy_manager::PROXY_MANAGER.remove_cloud_proxies();

  let _ = crate::events::emit_empty("cloud-auth-changed");
  Ok(())
}

#[tauri::command]
pub async fn cloud_has_active_subscription() -> Result<bool, String> {
  Ok(CLOUD_AUTH.has_active_paid_subscription().await)
}

#[tauri::command]
pub async fn cloud_get_wayfern_token() -> Result<Option<String>, String> {
  Ok(CLOUD_AUTH.get_wayfern_token().await)
}

#[tauri::command]
pub async fn cloud_refresh_wayfern_token() -> Result<Option<String>, String> {
  CLOUD_AUTH.request_wayfern_token().await?;
  Ok(CLOUD_AUTH.get_wayfern_token().await)
}

#[tauri::command]
pub async fn cloud_get_countries() -> Result<Vec<LocationItem>, String> {
  CLOUD_AUTH.fetch_countries().await
}

#[tauri::command]
pub async fn cloud_get_regions(country: String) -> Result<Vec<LocationItem>, String> {
  CLOUD_AUTH.fetch_regions(&country).await
}

#[tauri::command]
pub async fn cloud_get_cities(
  country: String,
  region: Option<String>,
) -> Result<Vec<LocationItem>, String> {
  CLOUD_AUTH.fetch_cities(&country, region.as_deref()).await
}

#[tauri::command]
pub async fn cloud_get_isps(
  country: String,
  region: Option<String>,
  city: Option<String>,
) -> Result<Vec<LocationItem>, String> {
  CLOUD_AUTH
    .fetch_isps(&country, region.as_deref(), city.as_deref())
    .await
}

#[tauri::command]
pub async fn create_cloud_location_proxy(
  name: String,
  country: String,
  region: Option<String>,
  city: Option<String>,
  isp: Option<String>,
) -> Result<crate::proxy_manager::StoredProxy, String> {
  // If no cloud proxy exists yet, attempt to sync it first
  if !PROXY_MANAGER.has_cloud_proxy() {
    CLOUD_AUTH.sync_cloud_proxy().await;
  }
  PROXY_MANAGER.create_cloud_location_proxy(name, country, region, city, isp)
}

#[derive(Debug, Serialize)]
pub struct CloudProxyUsage {
  pub used_mb: i64,
  pub limit_mb: i64,
  pub remaining_mb: i64,
  pub recurring_limit_mb: i64,
  pub extra_limit_mb: i64,
}

#[derive(Debug, Deserialize)]
struct ProxyUsageResponse {
  #[serde(rename = "usedMb")]
  used_mb: i64,
  #[serde(rename = "limitMb")]
  limit_mb: i64,
  #[serde(rename = "remainingMb")]
  remaining_mb: i64,
  #[serde(rename = "recurringLimitMb", default)]
  recurring_limit_mb: i64,
  #[serde(rename = "extraLimitMb", default)]
  extra_limit_mb: i64,
}

#[tauri::command]
pub async fn cloud_get_proxy_usage() -> Result<Option<CloudProxyUsage>, String> {
  let (has_proxy, cached_recurring, cached_extra) = {
    let state = CLOUD_AUTH.state.lock().await;
    match &*state {
      Some(auth)
        if auth.user.proxy_bandwidth_limit_mb > 0 || auth.user.proxy_bandwidth_extra_mb > 0 =>
      {
        (
          true,
          auth.user.proxy_bandwidth_limit_mb,
          auth.user.proxy_bandwidth_extra_mb,
        )
      }
      _ => return Ok(None),
    }
  };

  if !has_proxy {
    return Ok(None);
  }

  // Fetch live usage from the API
  match CLOUD_AUTH
    .api_call_with_retry(|access_token| {
      let url = format!("{CLOUD_API_URL}/api/proxy/usage");
      let client = reqwest::Client::new();
      async move {
        let response = client
          .get(&url)
          .header("Authorization", format!("Bearer {access_token}"))
          .send()
          .await
          .map_err(|e| format!("Failed to fetch proxy usage: {e}"))?;

        if !response.status().is_success() {
          return Err(format!(
            "Proxy usage API returned status {}",
            response.status()
          ));
        }

        response
          .json::<ProxyUsageResponse>()
          .await
          .map_err(|e| format!("Failed to parse proxy usage: {e}"))
      }
    })
    .await
  {
    Ok(usage) => Ok(Some(CloudProxyUsage {
      used_mb: usage.used_mb,
      limit_mb: usage.limit_mb,
      remaining_mb: usage.remaining_mb,
      recurring_limit_mb: if usage.recurring_limit_mb > 0 {
        usage.recurring_limit_mb
      } else {
        cached_recurring
      },
      extra_limit_mb: if usage.recurring_limit_mb > 0 {
        usage.extra_limit_mb
      } else {
        cached_extra
      },
    })),
    Err(e) => {
      log::warn!("Failed to fetch live proxy usage, falling back to cached: {e}");
      // Fallback to cached values
      let state = CLOUD_AUTH.state.lock().await;
      match &*state {
        Some(auth) => {
          let used = auth.user.proxy_bandwidth_used_mb;
          let total = cached_recurring + cached_extra;
          Ok(Some(CloudProxyUsage {
            used_mb: used,
            limit_mb: total,
            remaining_mb: (total - used).max(0),
            recurring_limit_mb: cached_recurring,
            extra_limit_mb: cached_extra,
          }))
        }
        _ => Ok(None),
      }
    }
  }
}

#[tauri::command]
pub async fn restart_sync_service(app_handle: tauri::AppHandle) -> Result<(), String> {
  // Stop existing scheduler
  if let Some(scheduler) = sync::get_global_scheduler() {
    scheduler.stop();
  }

  // Restart sync pipeline
  let app_handle_sync = app_handle.clone();
  tauri::async_runtime::spawn(async move {
    let mut subscription_manager = sync::SubscriptionManager::new();
    let work_rx = subscription_manager.take_work_receiver();

    if let Err(e) = subscription_manager.start(app_handle_sync.clone()).await {
      log::warn!("Failed to start sync subscription: {e}");
      return;
    }

    if let Some(work_rx) = work_rx {
      let scheduler = Arc::new(sync::SyncScheduler::new());
      sync::set_global_scheduler(scheduler.clone());

      scheduler.sync_all_enabled_profiles(&app_handle_sync).await;

      match sync::SyncEngine::create_from_settings(&app_handle_sync).await {
        Ok(engine) => {
          if let Err(e) = engine
            .check_for_missing_synced_profiles(&app_handle_sync)
            .await
          {
            log::warn!("Failed to check for missing profiles: {}", e);
          }
          if let Err(e) = engine
            .check_for_missing_synced_entities(&app_handle_sync)
            .await
          {
            log::warn!("Failed to check for missing entities: {}", e);
          }
        }
        Err(e) => {
          log::warn!("Sync not configured, skipping missing profile check: {}", e);
        }
      }

      scheduler
        .clone()
        .start(app_handle_sync.clone(), work_rx)
        .await;
      log::info!("Sync scheduler restarted");
    }
  });

  Ok(())
}

use aes_gcm::{
  aead::{Aead, AeadCore, KeyInit, OsRng},
  Aes256Gcm, Key, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use chrono::Utc;
use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

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
  pub plan_period: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudAuthState {
  pub user: CloudUser,
  pub logged_in_at: String,
}

#[derive(Debug, Deserialize)]
struct OtpRequestResponse {
  message: String,
}

#[derive(Debug, Deserialize)]
struct OtpVerifyResponse {
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

pub struct CloudAuthManager {
  client: Client,
  state: Mutex<Option<CloudAuthState>>,
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

  fn load_access_token() -> Result<Option<String>, String> {
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

  pub async fn request_otp(&self, email: &str) -> Result<String, String> {
    let url = format!("{CLOUD_API_URL}/api/auth/otp/request");
    let response = self
      .client
      .post(&url)
      .json(&serde_json::json!({ "email": email }))
      .send()
      .await
      .map_err(|e| format!("Failed to request OTP: {e}"))?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(format!("OTP request failed ({status}): {body}"));
    }

    let result: OtpRequestResponse = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse response: {e}"))?;

    Ok(result.message)
  }

  pub async fn verify_otp(&self, email: &str, code: &str) -> Result<CloudAuthState, String> {
    let url = format!("{CLOUD_API_URL}/api/auth/otp/verify");
    let response = self
      .client
      .post(&url)
      .json(&serde_json::json!({ "email": email, "code": code }))
      .send()
      .await
      .map_err(|e| format!("Failed to verify OTP: {e}"))?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(format!("OTP verification failed ({status}): {body}"));
    }

    let result: OtpVerifyResponse = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse response: {e}"))?;

    // Store tokens
    Self::store_access_token(&result.access_token)?;
    Self::store_refresh_token(&result.refresh_token)?;

    // Build and persist auth state
    let auth_state = CloudAuthState {
      user: result.user,
      logged_in_at: Utc::now().to_rfc3339(),
    };
    Self::store_auth_state(&auth_state)?;

    // Update in-memory state
    let mut state = self.state.lock().await;
    *state = Some(auth_state.clone());

    Ok(auth_state)
  }

  pub async fn refresh_access_token(&self) -> Result<(), String> {
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
      if status == reqwest::StatusCode::UNAUTHORIZED {
        // Refresh token expired — clear everything
        self.clear_auth().await;
        let _ = crate::events::emit_empty("cloud-auth-expired");
        return Err("Session expired. Please log in again.".to_string());
      }
      let body = response.text().await.unwrap_or_default();
      return Err(format!("Token refresh failed ({status}): {body}"));
    }

    let result: RefreshTokenResponse = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse response: {e}"))?;

    Self::store_access_token(&result.access_token)?;
    Self::store_refresh_token(&result.refresh_token)?;

    Ok(())
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
      Some(auth) => auth.user.plan != "free" && auth.user.subscription_status == "active",
      None => false,
    }
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
  async fn api_call_with_retry<F, Fut, T>(&self, make_request: F) -> Result<T, String>
  where
    F: Fn(String) -> Fut + Send,
    Fut: std::future::Future<Output = Result<T, String>> + Send,
  {
    let access_token = Self::load_access_token()?.ok_or_else(|| "Not logged in".to_string())?;

    match make_request(access_token).await {
      Ok(result) => Ok(result),
      Err(e) if e.contains("(401)") => {
        // Try refreshing the access token
        self.refresh_access_token().await?;
        let new_token =
          Self::load_access_token()?.ok_or_else(|| "Not logged in after refresh".to_string())?;
        make_request(new_token).await
      }
      Err(e) => Err(e),
    }
  }

  /// Background loop that refreshes the sync token periodically
  pub async fn start_sync_token_refresh_loop(app_handle: tauri::AppHandle) {
    loop {
      tokio::time::sleep(std::time::Duration::from_secs(600)).await; // 10 minutes

      if !CLOUD_AUTH.is_logged_in().await {
        continue;
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

      // Also refresh the access token if needed
      if let Ok(Some(token)) = Self::load_access_token() {
        if Self::is_jwt_expiring_soon(&token) {
          if let Err(e) = CLOUD_AUTH.refresh_access_token().await {
            log::warn!("Failed to refresh cloud access token: {e}");
          }
        }
      }

      // Refresh profile data periodically
      if let Err(e) = CLOUD_AUTH.fetch_profile().await {
        log::debug!("Failed to refresh cloud profile: {e}");
      }

      let _ = &app_handle; // keep app_handle alive
    }
  }
}

// --- Tauri commands ---

#[tauri::command]
pub async fn cloud_request_otp(email: String) -> Result<String, String> {
  CLOUD_AUTH.request_otp(&email).await
}

#[tauri::command]
pub async fn cloud_verify_otp(
  app_handle: tauri::AppHandle,
  email: String,
  code: String,
) -> Result<CloudAuthState, String> {
  let state = CLOUD_AUTH.verify_otp(&email, &code).await?;

  // Pre-fetch sync token so sync can start immediately
  if CLOUD_AUTH.has_active_paid_subscription().await {
    if let Err(e) = CLOUD_AUTH.get_or_refresh_sync_token().await {
      log::warn!("Failed to pre-fetch sync token after login: {e}");
    }
  }

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
  let _ = &app_handle;
  Ok(())
}

#[tauri::command]
pub async fn cloud_has_active_subscription() -> Result<bool, String> {
  Ok(CLOUD_AUTH.has_active_paid_subscription().await)
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
        }
        Err(e) => {
          log::debug!("Sync not configured, skipping missing profile check: {}", e);
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

use aes_gcm::{
  aead::{Aead, KeyInit},
  Aes256Gcm, Key, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use chrono::Utc;
use lazy_static::lazy_static;
use rand::RngExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use tokio::sync::Mutex;

use crate::browser::ProxySettings;
use crate::proxy_manager::PROXY_MANAGER;
use crate::settings_manager::SettingsManager;
use crate::sync;

pub const CLOUD_API_URL: &str = "https://api.donutbrowser.com";
pub const CLOUD_SYNC_URL: &str = "https://sync.donutbrowser.com";

/// Default per-hour cap on local automation API / MCP requests. Mirrors the
/// backend's DEFAULT_REQUESTS_PER_HOUR.
const DEFAULT_REQUESTS_PER_HOUR: i64 = 100;

/// Capability + limit set the account is entitled to, derived from its plan.
/// Mirrors `apps/backend/src/plans/entitlements.ts`. Features are gated on these
/// flags instead of a single "is paid?" boolean, so a plan like "solo" (cloud
/// backup + nightly cookie bot, no automation, no fingerprint editing, no
/// hands-on remote session) is just data here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entitlements {
  #[serde(default)]
  pub active: bool,
  #[serde(rename = "browserAutomation", default)]
  pub browser_automation: bool,
  #[serde(rename = "crossOsFingerprints", default)]
  pub cross_os_fingerprints: bool,
  #[serde(rename = "cloudBackup", default)]
  pub cloud_backup: bool,
  #[serde(rename = "teamCollaboration", default)]
  pub team_collaboration: bool,
  /// Overnight profile warming on a leased remote host. Present on the wire
  /// since the cookie-bot release; a field missing here is silently dropped on
  /// the way to the UI, which is why every mirror of this struct has to move
  /// together.
  #[serde(rename = "cookieBot", default)]
  pub cookie_bot: bool,
  /// Whether the plan may open a HANDS-ON remote session. Distinct from
  /// `cookie_bot`: solo funds a nightly bot out of its remote hours but may not
  /// drive a remote browser itself, so anything that offers interactive remote
  /// control must read THIS rather than `remote_browser_hours > 0`.
  #[serde(rename = "remoteInteractive", default)]
  pub remote_interactive: bool,
  #[serde(rename = "profileLimit", default)]
  pub profile_limit: i64,
  #[serde(rename = "requestsPerHour", default)]
  pub requests_per_hour: i64,
  /// Per-seat monthly remote-session allowance. Reporting only — a team pools
  /// it across seats, so the spendable figure comes from the quota route.
  #[serde(rename = "remoteBrowserHours", default)]
  pub remote_browser_hours: i64,
}

/// Local fallback mirror of the backend plan -> capability matrix, used only when
/// the server hasn't sent an entitlements object (older cached state / backend).
fn derive_entitlements(
  plan: &str,
  plan_period: Option<&str>,
  subscription_status: &str,
  profile_limit: i64,
) -> Entitlements {
  let active =
    plan != "free" && (subscription_status == "active" || plan_period == Some("lifetime"));
  if !active {
    return Entitlements {
      active: false,
      browser_automation: false,
      cross_os_fingerprints: false,
      cloud_backup: false,
      team_collaboration: false,
      cookie_bot: false,
      remote_interactive: false,
      profile_limit: 0,
      requests_per_hour: 0,
      remote_browser_hours: 0,
    };
  }
  // Tuple order: (browser_automation, cross_os_fingerprints, cloud_backup,
  // team_collaboration, cookie_bot, remote_interactive).
  //
  // pro and any unrecognized paid plan -> pro-level (never team). Solo is the
  // one row where cookie_bot and browser_automation disagree, which is why
  // cookie_bot can no longer be derived from browser_automation below.
  let (
    browser_automation,
    cross_os_fingerprints,
    cloud_backup,
    team_collaboration,
    cookie_bot,
    remote_interactive,
  ) = match plan {
    "solo" => (false, false, true, false, true, false),
    "team" | "enterprise" => (true, true, true, true, true, true),
    _ => (true, true, true, false, true, true),
  };
  Entitlements {
    active,
    browser_automation,
    cross_os_fingerprints,
    cloud_backup,
    team_collaboration,
    cookie_bot,
    remote_interactive,
    profile_limit,
    requests_per_hour: if browser_automation {
      DEFAULT_REQUESTS_PER_HOUR
    } else {
      0
    },
    // Deliberately 0 in the fallback: the allowance is the server's to state and
    // guessing it here would show a customer hours they may not have.
    remote_browser_hours: 0,
  }
}

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
  // This desktop session's position among the user's active devices, oldest
  // first. Ordinal 1 is the primary device — the only one that can run browser
  // automation. `default` keeps older login/state payloads (which lack these
  // fields) deserializing cleanly.
  #[serde(rename = "deviceOrdinal", default)]
  pub device_ordinal: Option<i64>,
  #[serde(rename = "deviceCount", default)]
  pub device_count: Option<i64>,
  #[serde(rename = "isPrimaryDevice", default)]
  pub is_primary_device: Option<bool>,
  /// Capability/limit set derived from the plan by the backend. `default` (None)
  /// keeps older login/state payloads deserializing; resolve via `entitlements()`.
  #[serde(default)]
  pub entitlements: Option<Entitlements>,
}

impl CloudUser {
  /// Authoritative entitlements: the server-sent set when present, else derived
  /// locally from the plan fields (keeps older cached state / backends working).
  pub fn entitlements(&self) -> Entitlements {
    if let Some(e) = &self.entitlements {
      // Returned verbatim, INCLUDING the `#[serde(default)]` false that a
      // backend older than this release leaves on `cookie_bot` /
      // `remote_interactive`. Repairing it here is impossible anyway — serde's
      // default erases the difference between "sent false" and "not sent" — and
      // it is not this layer's job: nothing in Rust gates on either flag, and
      // `getEntitlements()` in `src/lib/entitlements.ts` fills both gaps at the
      // single point every UI consumer already goes through.
      return e.clone();
    }
    derive_entitlements(
      &self.plan,
      self.plan_period.as_deref(),
      &self.subscription_status,
      self.profile_limit,
    )
  }
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
    // Bound every cloud API call so no single slow / hung request can stall
    // the startup chain (sync-token → proxy-config → wayfern-token), which
    // otherwise gates Wayfern launch behind whichever endpoint is slowest.
    let client = Client::builder()
      .timeout(std::time::Duration::from_secs(15))
      .connect_timeout(std::time::Duration::from_secs(5))
      .build()
      .unwrap_or_else(|_| Client::new());
    Self {
      client,
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
    let salt_bytes: [u8; 16] = rand::rng().random();
    let salt =
      SaltString::encode_b64(&salt_bytes).map_err(|e| format!("Failed to encode salt: {e}"))?;
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
    let nonce_bytes: [u8; 12] = rand::rng().random();
    let nonce = Nonce::from(nonce_bytes);
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
    crate::app_dirs::restrict_to_owner(file_path);
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
    fs::write(&path, json).map_err(|e| format!("Failed to write auth state: {e}"))?;
    crate::app_dirs::restrict_to_owner(&path);
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
      // The backend returns { message, code, … } for 4xx (e.g. the 3-device
      // limit or a temporary security block). Surface the human-readable
      // message rather than the raw JSON so the sign-in screen is clear.
      let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
          v.get("message")
            .and_then(|m| m.as_str())
            .map(std::string::ToString::to_string)
        })
        .unwrap_or_else(|| format!("Login failed ({status})"));
      return Err(message);
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
      log::warn!("Token refresh failed ({status})");
      return Err(format!("Token refresh failed ({status})"));
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

  /// Resolve this session's entitlements (server-sent or locally derived).
  pub async fn entitlements(&self) -> Option<Entitlements> {
    let state = self.state.lock().await;
    state.as_ref().map(|auth| auth.user.entitlements())
  }

  /// Account is in a paid/active state. Used for the "any active plan" gates
  /// (sync token); per-feature access uses the capability helpers.
  pub async fn has_active_paid_subscription(&self) -> bool {
    #[cfg(feature = "e2e")]
    if crate::e2e_automation_enabled()
      && std::env::var_os("WAYFERN_TEST_TOKEN").is_some_and(|token| !token.is_empty())
    {
      return true;
    }

    self.entitlements().await.map(|e| e.active).unwrap_or(false)
  }

  /// Whether this session's plan entitles it to a Wayfern automation token.
  ///
  /// The token IS the automation entitlement, so this is `browser_automation`
  /// and NOT `has_active_paid_subscription`. Gating the mint on "any active
  /// plan" meant a Solo account — active, paying, and deliberately sold without
  /// automation or fingerprint editing — asked for a token on every startup,
  /// every login and every 10-hour refresh, collected a 403 each time, and got
  /// the "account temporarily restricted" toast that belongs to the
  /// multiple-device rule. Nothing was restricted; the plan simply does not
  /// include the feature.
  ///
  /// Reads the entitlement directly rather than going through
  /// `can_use_browser_automation`, whose e2e override would send the browser
  /// suite off to the live API for a token it already has as a test value.
  pub async fn is_entitled_to_wayfern_token(&self) -> bool {
    self
      .entitlements()
      .await
      .is_some_and(|e| e.active && e.browser_automation)
  }

  /// Non-async version that uses try_lock, defaults to false if lock can't be acquired.
  pub fn has_active_paid_subscription_sync(&self) -> bool {
    match self.state.try_lock() {
      Ok(state) => state
        .as_ref()
        .map(|auth| auth.user.entitlements().active)
        .unwrap_or(false),
      Err(_) => false,
    }
  }

  /// Launch/drive profiles programmatically (local API + MCP automation).
  /// Whether this account may run the nightly Cookie Bot.
  ///
  /// NOT `can_use_browser_automation`. Solo is exactly the plan where the two
  /// disagree — it pays for a nightly bot and has `browser_automation: false` —
  /// so gating the bot on automation refused a Solo customer the one feature
  /// their plan is sold on, and answered 402 while their scheduled runs kept
  /// working server-side.
  pub async fn can_use_cookie_bot(&self) -> bool {
    self
      .entitlements()
      .await
      .map(|e| e.cookie_bot)
      .unwrap_or(false)
  }

  pub async fn can_use_browser_automation(&self) -> bool {
    #[cfg(feature = "e2e")]
    if crate::e2e_automation_enabled()
      && std::env::var_os("WAYFERN_TEST_TOKEN").is_some_and(|token| !token.is_empty())
    {
      return true;
    }

    self
      .entitlements()
      .await
      .map(|e| e.browser_automation)
      .unwrap_or(false)
  }

  /// Edit fingerprints / use a non-native OS fingerprint.
  pub async fn can_use_cross_os_fingerprints(&self) -> bool {
    #[cfg(feature = "e2e")]
    if crate::e2e_automation_enabled()
      && std::env::var_os("WAYFERN_TEST_TOKEN").is_some_and(|token| !token.is_empty())
    {
      return true;
    }

    self
      .entitlements()
      .await
      .map(|e| e.cross_os_fingerprints)
      .unwrap_or(false)
  }

  /// Cloud profile sync / backup (async).
  pub async fn can_use_cloud_backup(&self) -> bool {
    self
      .entitlements()
      .await
      .map(|e| e.cloud_backup)
      .unwrap_or(false)
  }

  /// Cloud profile sync / backup (non-async, try_lock; false if unavailable).
  pub fn can_use_cloud_backup_sync(&self) -> bool {
    match self.state.try_lock() {
      Ok(state) => state
        .as_ref()
        .map(|auth| auth.user.entitlements().cloud_backup)
        .unwrap_or(false),
      Err(_) => false,
    }
  }

  /// Identity and positive per-hour cap for the shared REST/MCP automation
  /// limiter. No active automation entitlement means no limiter entry; the
  /// capability gates still reject paid operations independently.
  pub async fn automation_rate_limit(&self) -> Option<(String, u64)> {
    #[cfg(feature = "e2e")]
    if crate::e2e_automation_enabled() {
      if let Ok(limit) = std::env::var("DONUT_E2E_REQUESTS_PER_HOUR") {
        if let Ok(limit) = limit.parse::<u64>() {
          if limit > 0 {
            return Some(("e2e-automation".to_string(), limit));
          }
        }
      }
    }

    let state = self.get_user().await?;
    let limit = state.user.entitlements().requests_per_hour;
    (limit > 0).then_some((state.user.id, limit as u64))
  }

  pub async fn is_fingerprint_os_allowed(&self, fingerprint_os: Option<&str>) -> bool {
    let host_os = crate::profile::types::get_host_os();
    match fingerprint_os {
      None => true,
      Some(os) if os == host_os => true,
      Some(_) => self.can_use_cross_os_fingerprints().await,
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
            log::warn!("Proxy config returned 403");
            return Err("__403__".to_string());
          }

          if !response.status().is_success() {
            return Err(format!("Proxy config fetch failed ({status})"));
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
          vless_uri: None,
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

  /// Request a wayfern token from the cloud API. Only succeeds for plans that
  /// include browser automation.
  ///
  /// Self-gating on purpose: every caller used to repeat the check, and the one
  /// they repeated was the wrong one. A plan without automation is not an error
  /// state here — it clears any stale token and reports success, because there
  /// is nothing to fetch and nothing wrong.
  pub async fn request_wayfern_token(&self) -> Result<(), String> {
    if !self.is_entitled_to_wayfern_token().await {
      // Ok(()) here means callers log nothing, so a session that declined to
      // mint left no trace at all and looked identical to one that succeeded.
      log::info!(
        "Skipping wayfern token request: the cached plan does not include browser automation"
      );
      self.clear_wayfern_token().await;
      return Ok(());
    }

    let result = self
      .api_call_with_retry(|access_token| {
        let url = format!("{CLOUD_API_URL}/api/auth/wayfern-start");
        // Bound the request: without a timeout, an unreachable
        // api.donutbrowser.com hangs the background fetch indefinitely,
        // which in turn forces wayfern_manager's launch-time wait to
        // exhaust its full polling budget every time.
        let client = reqwest::Client::builder()
          .timeout(std::time::Duration::from_secs(8))
          .connect_timeout(std::time::Duration::from_secs(4))
          .build()
          .unwrap_or_else(|_| reqwest::Client::new());
        async move {
          let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to request wayfern token: {e}"))?;

          if !response.status().is_success() {
            let status = response.status();
            // The body carries WHICH rule refused: a device-family conflict or
            // a plan that lacks automation. They need different handling, so
            // keep the text instead of collapsing every failure to a status.
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
      .await;

    let token = match result {
      Ok(token) => token,
      Err(e) => {
        // A 403 rejects the entitlement without invalidating the login session.
        // Clear the browser token and refresh account state before notifying UI.
        if e.contains("(403") || e.contains("Forbidden") {
          log::warn!("Wayfern token blocked by backend (403): {e}");
          self.clear_wayfern_token().await;
          if let Err(fetch_err) = self.fetch_profile().await {
            log::warn!("Profile re-fetch after wayfern block failed: {fetch_err}");
          }
          // Only the device rules produce a restriction the user can lift, and
          // the toast tells them to sign other devices out — so only those may
          // raise it. A plan-level refusal that slipped past the gate above
          // (cached entitlements the re-fetch just corrected) must stay silent:
          // telling a Solo customer they are "temporarily restricted" describes
          // a lockout that does not exist and hides the real answer, which is
          // that their plan does not include browser automation.
          if is_device_restriction(&e) {
            let _ = crate::events::emit_empty("wayfern-paid-blocked");
          }
        }
        return Err(e);
      }
    };

    let mut wt = self.wayfern_token.lock().await;
    *wt = Some(token);
    log::info!("Wayfern token acquired");
    Ok(())
  }

  /// Get the current wayfern token, if any.
  pub async fn get_wayfern_token(&self) -> Option<String> {
    #[cfg(feature = "e2e")]
    if crate::e2e_automation_enabled() {
      if let Some(token) = std::env::var_os("WAYFERN_TEST_TOKEN")
        .filter(|token| !token.is_empty())
        .and_then(|token| token.into_string().ok())
      {
        return Some(token);
      }
    }

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

      // Refresh profile data periodically. A failure here leaves the cached
      // plan stale, which silently gates paid features, so it belongs at warn
      // rather than debug where the shipped log level hides it.
      if let Err(e) = CLOUD_AUTH.fetch_profile().await {
        log::warn!("Failed to refresh cloud profile: {e}");
      }

      // Reconnect profile lock manager if needed
      if let Some(auth_state) = CLOUD_AUTH.get_user().await {
        if auth_state.user.plan != "free" && !crate::team_lock::PROFILE_LOCK.is_connected().await {
          crate::team_lock::PROFILE_LOCK.connect().await;
        }
      }

      // Sync cloud proxy credentials
      CLOUD_AUTH.sync_cloud_proxy().await;

      // Refresh wayfern token every 10 hours (60 iterations of 10-minute loop).
      // request_wayfern_token owns the entitlement check and clears the cached
      // token when the plan doesn't include automation.
      //
      // Also mint one as soon as the plan starts granting it. `fetch_profile`
      // above picks up an upgrade within ten minutes, but nothing watched that
      // transition, so a session that signed in before upgrading stayed
      // tokenless for up to ten hours while reporting the feature as unlocked.
      let missing_entitled_token = CLOUD_AUTH.is_entitled_to_wayfern_token().await
        && CLOUD_AUTH.get_wayfern_token().await.is_none();
      if wayfern_refresh_counter >= 60 || missing_entitled_token {
        wayfern_refresh_counter = 0;
        if let Err(e) = CLOUD_AUTH.request_wayfern_token().await {
          log::warn!("Failed to refresh wayfern token: {e}");
        }
      }

      let _ = &app_handle; // keep app_handle alive
    }
  }
}

/// Whether a rejected wayfern-token request was refused by one of the
/// device-family rules (automation is pinned to the primary desktop session)
/// rather than by the plan's capabilities.
///
/// Matches on the backend's message because that is the only thing that
/// distinguishes them: both arrive as a bare 403. Only these two are a state
/// the user can clear themselves, which is what the toast asks them to do.
fn is_device_restriction(error: &str) -> bool {
  error.contains("primary device") || error.contains("requires the desktop app")
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
  let mut state = CLOUD_AUTH.exchange_device_code(&code).await?;

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
  state.user.entitlements = Some(state.user.entitlements());
  Ok(state)
}

#[tauri::command]
pub async fn cloud_get_user() -> Result<Option<CloudAuthState>, String> {
  Ok(CLOUD_AUTH.get_user().await.map(|mut state| {
    // Always hand the frontend a resolved entitlements object so it never has to
    // derive capabilities itself (covers older cached state with no entitlements).
    state.user.entitlements = Some(state.user.entitlements());
    state
  }))
}

#[tauri::command]
pub async fn cloud_refresh_profile() -> Result<CloudUser, String> {
  let mut user = CLOUD_AUTH.fetch_profile().await?;
  user.entitlements = Some(user.entitlements());

  // Minting the token is what actually unlocks cross-OS fingerprints, and it
  // only happened at login, at startup and once every 10 hours. An account
  // that upgraded after its last sign-in therefore refreshed into the correct
  // entitlements while still holding no token, and "Refresh" did not fix it.
  // Only mint when one is genuinely missing, so this stays a no-op afterwards.
  if CLOUD_AUTH.is_entitled_to_wayfern_token().await
    && CLOUD_AUTH.get_wayfern_token().await.is_none()
  {
    if let Err(e) = CLOUD_AUTH.request_wayfern_token().await {
      log::warn!("Refresh could not obtain a wayfern token: {e}");
    }
  }

  Ok(user)
}

#[tauri::command]
pub async fn cloud_logout(app_handle: tauri::AppHandle) -> Result<(), String> {
  CLOUD_AUTH.logout().await?;

  // Always clear the stored sync URL and token on cloud logout. While the
  // user was signed in, the cloud auth flow populated these with the hosted
  // sync server's URL + a server-issued token — leaving them in place would
  // pre-fill the Self-Hosted tab with our production URL and a token the
  // user never typed. The cloud-URL-only check we used to do here missed
  // trailing-slash / scheme variants and any future cloud endpoint moves.
  let manager = crate::settings_manager::SettingsManager::instance();
  let _ = manager.save_sync_server_url(None);
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
  // Rebuilding the pipeline reaches the network, so do it off the command and
  // let the caller's dialog close. `start_pipeline` retires the previous
  // scheduler and the previous subscription itself.
  tauri::async_runtime::spawn(async move {
    sync::start_pipeline(app_handle).await;
  });

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn active_solo() -> Entitlements {
    derive_entitlements("solo", Some("monthly"), "active", 20)
  }

  #[test]
  fn solo_is_active_without_browser_automation() {
    let solo = active_solo();
    assert!(solo.active, "solo is a paid, active plan");
    assert!(solo.cloud_backup, "solo buys cloud profile backups");
    assert!(solo.cookie_bot, "solo buys the nightly cookie bot");
    assert!(
      !solo.browser_automation,
      "solo is sold without browser automation"
    );
    assert!(
      !solo.cross_os_fingerprints,
      "solo is sold without fingerprint editing"
    );
  }

  #[test]
  fn wayfern_token_is_gated_on_automation_not_on_being_paid() {
    // The regression this guards: gating the mint on `active` asked for a token
    // on behalf of a Solo account, which the backend answers with a 403.
    let solo = active_solo();
    assert!(!(solo.active && solo.browser_automation));

    let pro = derive_entitlements("pro", Some("monthly"), "active", 50);
    assert!(pro.active && pro.browser_automation);
  }

  #[test]
  fn only_the_device_rules_read_as_a_restriction() {
    assert!(is_device_restriction(
      "Wayfern token request failed (403 Forbidden): {\"message\":\"Browser automation is restricted to your primary device. Log out other devices to use it here.\",\"statusCode\":403}"
    ));
    assert!(is_device_restriction(
      "Wayfern token request failed (403 Forbidden): {\"message\":\"Browser automation requires the desktop app. Open Donut Browser and try again.\",\"statusCode\":403}"
    ));
    // A plan-level refusal is not a restriction, and must not raise the toast
    // that tells the user to sign other devices out.
    assert!(!is_device_restriction(
      "Wayfern token request failed (403 Forbidden): {\"message\":\"Browser automation subscription required\",\"statusCode\":403}"
    ));
    assert!(!is_device_restriction(
      "Wayfern token request failed (500 Internal Server Error): "
    ));
  }
}

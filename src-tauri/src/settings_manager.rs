use serde::{Deserialize, Serialize};
use std::fs::{self, create_dir_all};
use std::path::PathBuf;

use aes_gcm::{
  aead::{Aead, KeyInit},
  Aes256Gcm, Key, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use rand::RngExt;

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
  #[serde(default)]
  pub language: Option<String>, // ISO 639-1: "en", "es", "pt", "fr", "zh", "ja", "ko", "ru", or None for system default
  #[serde(default)]
  pub window_resize_warning_dismissed: bool,
  #[serde(default)]
  pub onboarding_completed: bool, // First-launch onboarding has been shown/handled (one-shot)
  #[serde(default)]
  pub disable_auto_updates: bool,
  /// When true, the decrypted in-RAM copy of a password-protected profile is
  /// preserved between launches for faster subsequent startups. The on-disk
  /// copy is always re-encrypted regardless of this flag.
  #[serde(default)]
  pub keep_decrypted_profiles_in_ram: bool,
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
      language: None,
      window_resize_warning_dismissed: false,
      onboarding_completed: false,
      disable_auto_updates: false,
      keep_decrypted_profiles_in_ram: false,
    }
  }
}

pub struct SettingsManager;

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

    let settings_file = self.get_settings_file();
    let json = serde_json::to_string_pretty(settings)?;
    fs::write(settings_file, json)?;

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
    fs::write(sorting_file, json)?;

    Ok(())
  }

  fn get_vault_password() -> String {
    env!("DONUT_BROWSER_VAULT_PASSWORD").to_string()
  }

  pub async fn generate_api_token(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<String, Box<dyn std::error::Error>> {
    // Generate a secure random token (base64 encoded for URL safety)
    let token_bytes: [u8; 32] = {
      use rand::Rng;
      let mut rng = rand::rng();
      let mut bytes = [0u8; 32];
      rng.fill_bytes(&mut bytes);
      bytes
    };
    use base64::{engine::general_purpose, Engine as _};
    let token = general_purpose::URL_SAFE_NO_PAD.encode(token_bytes);

    // Store token securely
    self.store_api_token(app_handle, &token).await?;

    Ok(token)
  }

  pub async fn store_api_token(
    &self,
    _app_handle: &tauri::AppHandle,
    token: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    // Store token in an encrypted file using Argon2 + AES-GCM
    let token_file = self.get_settings_dir().join("api_token.dat");

    // Create directory if it doesn't exist
    if let Some(parent) = token_file.parent() {
      std::fs::create_dir_all(parent)?;
    }

    let vault_password = Self::get_vault_password();

    // Generate a random salt for Argon2
    let salt_bytes: [u8; 16] = rand::rng().random();
    let salt =
      SaltString::encode_b64(&salt_bytes).map_err(|e| format!("Failed to encode salt: {e}"))?;

    // Use Argon2 to derive a 32-byte key from the vault password
    let argon2 = Argon2::default();
    let password_hash = argon2
      .hash_password(vault_password.as_bytes(), &salt)
      .map_err(|e| format!("Argon2 key derivation failed: {e}"))?;
    let hash_value = password_hash.hash.unwrap();
    let hash_bytes = hash_value.as_bytes();

    // Take first 32 bytes for AES-256 key
    let key_bytes: [u8; 32] = hash_bytes[..32]
      .try_into()
      .map_err(|_| "Invalid key length")?;
    let key = Key::<Aes256Gcm>::from(key_bytes);
    let cipher = Aes256Gcm::new(&key);

    // Generate a random nonce
    let nonce_bytes: [u8; 12] = rand::rng().random();
    let nonce = Nonce::from(nonce_bytes);

    // Encrypt the token
    let ciphertext = cipher
      .encrypt(&nonce, token.as_bytes())
      .map_err(|e| format!("Encryption failed: {e}"))?;

    // Create file data with header, salt, nonce, and encrypted data
    let mut file_data = Vec::new();
    file_data.extend_from_slice(b"DBAPI"); // 5-byte header
    file_data.push(2u8); // Version 2 (Argon2 + AES-GCM)

    // Store salt length and salt
    let salt_str = salt.as_str();
    file_data.push(salt_str.len() as u8);
    file_data.extend_from_slice(salt_str.as_bytes());

    // Store nonce (12 bytes for AES-GCM)
    file_data.extend_from_slice(&nonce);

    // Store ciphertext length and ciphertext
    file_data.extend_from_slice(&(ciphertext.len() as u32).to_le_bytes());
    file_data.extend_from_slice(&ciphertext);

    std::fs::write(&token_file, file_data)?;
    crate::app_dirs::restrict_to_owner(std::path::Path::new(&token_file));
    Ok(())
  }

  pub async fn get_api_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let token_file = self.get_settings_dir().join("api_token.dat");

    if !token_file.exists() {
      return Ok(None);
    }

    let file_data = std::fs::read(token_file)?;

    // Validate header
    if file_data.len() < 6 || &file_data[0..5] != b"DBAPI" {
      return Ok(None);
    }

    let version = file_data[5];

    // Only support Argon2 + AES-GCM (version 2)
    if version != 2 {
      return Ok(None);
    }

    // Argon2 + AES-GCM decryption
    let mut offset = 6;

    // Read salt
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

    // Read nonce (12 bytes)
    if offset + 12 > file_data.len() {
      return Ok(None);
    }
    let nonce_bytes: [u8; 12] = file_data[offset..offset + 12]
      .try_into()
      .map_err(|_| "Invalid nonce length")?;
    let nonce = Nonce::from(nonce_bytes);
    offset += 12;

    // Read ciphertext
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

    // Derive key using Argon2
    let vault_password = Self::get_vault_password();
    let argon2 = Argon2::default();
    let password_hash = argon2
      .hash_password(vault_password.as_bytes(), &salt)
      .map_err(|e| format!("Argon2 key derivation failed: {e}"))?;
    let hash_value = password_hash.hash.unwrap();
    let hash_bytes = hash_value.as_bytes();

    let key_bytes: [u8; 32] = hash_bytes[..32]
      .try_into()
      .map_err(|_| "Invalid key length")?;
    let key = Key::<Aes256Gcm>::from(key_bytes);
    let cipher = Aes256Gcm::new(&key);

    // Decrypt the token
    let plaintext = cipher
      .decrypt(&nonce, ciphertext)
      .map_err(|_| "Decryption failed")?;

    match String::from_utf8(plaintext) {
      Ok(token) => Ok(Some(token)),
      Err(_) => Ok(None),
    }
  }

  pub async fn remove_api_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let token_file = self.get_settings_dir().join("api_token.dat");

    if token_file.exists() {
      std::fs::remove_file(token_file)?;
    }

    Ok(())
  }

  pub async fn generate_mcp_token(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<String, Box<dyn std::error::Error>> {
    let token_bytes: [u8; 32] = {
      use rand::Rng;
      let mut rng = rand::rng();
      let mut bytes = [0u8; 32];
      rng.fill_bytes(&mut bytes);
      bytes
    };
    use base64::{engine::general_purpose, Engine as _};
    let token = general_purpose::URL_SAFE_NO_PAD.encode(token_bytes);
    self.store_mcp_token(app_handle, &token).await?;
    Ok(token)
  }

  pub async fn store_mcp_token(
    &self,
    _app_handle: &tauri::AppHandle,
    token: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let token_file = self.get_settings_dir().join("mcp_token.dat");

    if let Some(parent) = token_file.parent() {
      std::fs::create_dir_all(parent)?;
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
      .map_err(|_| "Invalid key length")?;
    let key = Key::<Aes256Gcm>::from(key_bytes);
    let cipher = Aes256Gcm::new(&key);
    let nonce_bytes: [u8; 12] = rand::rng().random();
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
      .encrypt(&nonce, token.as_bytes())
      .map_err(|e| format!("Encryption failed: {e}"))?;

    let mut file_data = Vec::new();
    file_data.extend_from_slice(b"DBMCP"); // 5-byte header for MCP token
    file_data.push(2u8); // Version 2 (Argon2 + AES-GCM)
    let salt_str = salt.as_str();
    file_data.push(salt_str.len() as u8);
    file_data.extend_from_slice(salt_str.as_bytes());
    file_data.extend_from_slice(&nonce);
    file_data.extend_from_slice(&(ciphertext.len() as u32).to_le_bytes());
    file_data.extend_from_slice(&ciphertext);

    std::fs::write(&token_file, file_data)?;
    crate::app_dirs::restrict_to_owner(std::path::Path::new(&token_file));
    Ok(())
  }

  pub async fn get_mcp_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let token_file = self.get_settings_dir().join("mcp_token.dat");

    if !token_file.exists() {
      return Ok(None);
    }

    let file_data = std::fs::read(token_file)?;

    if file_data.len() < 6 || &file_data[0..5] != b"DBMCP" {
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
      .map_err(|_| "Invalid nonce length")?;
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
      .map_err(|_| "Invalid key length")?;
    let key = Key::<Aes256Gcm>::from(key_bytes);
    let cipher = Aes256Gcm::new(&key);
    let plaintext = cipher
      .decrypt(&nonce, ciphertext)
      .map_err(|_| "Decryption failed")?;

    match String::from_utf8(plaintext) {
      Ok(token) => Ok(Some(token)),
      Err(_) => Ok(None),
    }
  }

  pub async fn remove_mcp_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let token_file = self.get_settings_dir().join("mcp_token.dat");

    if token_file.exists() {
      std::fs::remove_file(token_file)?;
    }

    Ok(())
  }

  pub async fn store_sync_token(
    &self,
    _app_handle: &tauri::AppHandle,
    token: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let token_file = self.get_settings_dir().join("sync_token.dat");

    if let Some(parent) = token_file.parent() {
      std::fs::create_dir_all(parent)?;
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
      .map_err(|_| "Invalid key length")?;
    let key = Key::<Aes256Gcm>::from(key_bytes);
    let cipher = Aes256Gcm::new(&key);
    let nonce_bytes: [u8; 12] = rand::rng().random();
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
      .encrypt(&nonce, token.as_bytes())
      .map_err(|e| format!("Encryption failed: {e}"))?;

    let mut file_data = Vec::new();
    file_data.extend_from_slice(b"DBSYN"); // 5-byte header for sync
    file_data.push(2u8); // Version 2 (Argon2 + AES-GCM)
    let salt_str = salt.as_str();
    file_data.push(salt_str.len() as u8);
    file_data.extend_from_slice(salt_str.as_bytes());
    file_data.extend_from_slice(&nonce);
    file_data.extend_from_slice(&(ciphertext.len() as u32).to_le_bytes());
    file_data.extend_from_slice(&ciphertext);

    std::fs::write(&token_file, file_data)?;
    crate::app_dirs::restrict_to_owner(std::path::Path::new(&token_file));
    Ok(())
  }

  pub async fn get_sync_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let token_file = self.get_settings_dir().join("sync_token.dat");

    if !token_file.exists() {
      return Ok(None);
    }

    let file_data = std::fs::read(token_file)?;

    if file_data.len() < 6 || &file_data[0..5] != b"DBSYN" {
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
      .map_err(|_| "Invalid nonce length")?;
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
      .map_err(|_| "Invalid key length")?;
    let key = Key::<Aes256Gcm>::from(key_bytes);
    let cipher = Aes256Gcm::new(&key);
    let plaintext = cipher
      .decrypt(&nonce, ciphertext)
      .map_err(|_| "Decryption failed")?;

    match String::from_utf8(plaintext) {
      Ok(token) => Ok(Some(token)),
      Err(_) => Ok(None),
    }
  }

  pub async fn remove_sync_token(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let token_file = self.get_settings_dir().join("sync_token.dat");

    if token_file.exists() {
      std::fs::remove_file(token_file)?;
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

  Ok(settings)
}

#[tauri::command]
pub async fn save_app_settings(
  app_handle: tauri::AppHandle,
  mut settings: AppSettings,
) -> Result<AppSettings, String> {
  let manager = SettingsManager::instance();

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

  // Preserve server-managed flags that the frontend may not have up-to-date.
  // Read directly from file to avoid load_settings' save-on-load behavior.
  if let Ok(content) = std::fs::read_to_string(manager.get_settings_file()) {
    if let Ok(current) = serde_json::from_str::<AppSettings>(&content) {
      settings.window_resize_warning_dismissed = current.window_resize_warning_dismissed;
    }
  }

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
      language: None,
      window_resize_warning_dismissed: false,
      onboarding_completed: false,
      disable_auto_updates: false,
      keep_decrypted_profiles_in_ram: false,
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

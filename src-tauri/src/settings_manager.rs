use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::fs::{self, create_dir_all};
use std::path::PathBuf;

use crate::api_client::ApiClient;
use crate::version_updater;

use aes_gcm::{
  aead::{Aead, AeadCore, KeyInit, OsRng},
  Aes256Gcm, Key, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};

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
    }
  }
}

pub struct SettingsManager {
  base_dirs: BaseDirs,
}

impl SettingsManager {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
    }
  }

  pub fn instance() -> &'static SettingsManager {
    &SETTINGS_MANAGER
  }

  pub fn get_settings_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("settings");
    path
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
      Ok(settings) => {
        // Save the settings back to ensure any missing fields are written with defaults
        if let Err(e) = self.save_settings(&settings) {
          eprintln!("Warning: Failed to update settings file with defaults: {e}");
        }
        Ok(settings)
      }
      Err(e) => {
        eprintln!("Warning: Failed to parse settings file, using defaults: {e}");
        let default_settings = AppSettings::default();

        // Try to save default settings to fix the corrupted file
        if let Err(save_error) = self.save_settings(&default_settings) {
          eprintln!("Warning: Failed to save default settings: {save_error}");
        }

        Ok(default_settings)
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

  pub fn should_show_settings_on_startup(&self) -> Result<bool, Box<dyn std::error::Error>> {
    // Always return false - we don't show settings on startup anymore
    Ok(false)
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
      use rand::RngCore;
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
    let salt = SaltString::generate(&mut OsRng);

    // Use Argon2 to derive a 32-byte key from the vault password
    let argon2 = Argon2::default();
    let password_hash = argon2
      .hash_password(vault_password.as_bytes(), &salt)
      .map_err(|e| format!("Argon2 key derivation failed: {e}"))?;
    let hash_value = password_hash.hash.unwrap();
    let hash_bytes = hash_value.as_bytes();

    // Take first 32 bytes for AES-256 key
    let key = Key::<Aes256Gcm>::from_slice(&hash_bytes[..32]);
    let cipher = Aes256Gcm::new(key);

    // Generate a random nonce
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

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

    std::fs::write(token_file, file_data)?;
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
    let nonce_bytes = &file_data[offset..offset + 12];
    let nonce = Nonce::from_slice(nonce_bytes);
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

    let key = Key::<Aes256Gcm>::from_slice(&hash_bytes[..32]);
    let cipher = Aes256Gcm::new(key);

    // Decrypt the token
    let plaintext = cipher
      .decrypt(nonce, ciphertext)
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
}

#[tauri::command]
pub async fn get_app_settings(app_handle: tauri::AppHandle) -> Result<AppSettings, String> {
  let manager = SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))?;

  // Always load token for display purposes if it exists
  settings.api_token = manager
    .get_api_token(&app_handle)
    .await
    .map_err(|e| format!("Failed to load API token: {e}"))?;

  Ok(settings)
}

#[tauri::command]
pub async fn save_app_settings(
  app_handle: tauri::AppHandle,
  mut settings: AppSettings,
) -> Result<AppSettings, String> {
  let manager = SettingsManager::instance();

  if settings.api_enabled {
    if let Some(ref token) = settings.api_token {
      manager
        .store_api_token(&app_handle, token)
        .await
        .map_err(|e| format!("Failed to store API token: {e}"))?;
    } else {
      let token = manager
        .generate_api_token(&app_handle)
        .await
        .map_err(|e| format!("Failed to generate API token: {e}"))?;
      settings.api_token = Some(token);
    }
  }

  // If API is being disabled, remove the token
  if !settings.api_enabled {
    manager
      .remove_api_token(&app_handle)
      .await
      .map_err(|e| format!("Failed to remove API token: {e}"))?;
    settings.api_token = None;
  }

  let mut persist_settings = settings.clone();
  persist_settings.api_token = None;
  manager
    .save_settings(&persist_settings)
    .map_err(|e| format!("Failed to save settings: {e}"))?;

  Ok(settings)
}

#[tauri::command]
pub async fn should_show_settings_on_startup() -> Result<bool, String> {
  let manager = SettingsManager::instance();
  manager
    .should_show_settings_on_startup()
    .map_err(|e| format!("Failed to check prompt setting: {e}"))
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
pub async fn clear_all_version_cache_and_refetch(
  app_handle: tauri::AppHandle,
) -> Result<(), String> {
  let api_client = ApiClient::instance();

  // Clear all cache first
  api_client
    .clear_all_cache()
    .map_err(|e| format!("Failed to clear version cache: {e}"))?;

  // Disable all browsers during the update process
  let auto_updater = crate::auto_updater::AutoUpdater::instance();
  let supported_browsers =
    crate::browser_version_manager::BrowserVersionManager::instance().get_supported_browsers();

  // Load current state and disable all browsers
  let mut state = auto_updater
    .load_auto_update_state()
    .map_err(|e| format!("Failed to load auto update state: {e}"))?;
  for browser in &supported_browsers {
    state.disabled_browsers.insert(browser.clone());
  }
  auto_updater
    .save_auto_update_state(&state)
    .map_err(|e| format!("Failed to save auto update state: {e}"))?;

  let updater = version_updater::get_version_updater();
  let updater_guard = updater.lock().await;

  let result = updater_guard
    .trigger_manual_update(&app_handle)
    .await
    .map_err(|e| format!("Failed to trigger version update: {e}"));

  // Re-enable all browsers after the update completes (regardless of success/failure)
  let mut final_state = auto_updater.load_auto_update_state().unwrap_or_default();
  for browser in &supported_browsers {
    final_state.disabled_browsers.remove(browser);
  }
  if let Err(e) = auto_updater.save_auto_update_state(&final_state) {
    eprintln!("Warning: Failed to re-enable browsers after cache clear: {e}");
  }

  result?;
  Ok(())
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref SETTINGS_MANAGER: SettingsManager = SettingsManager::new();
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::env;
  use tempfile::TempDir;

  fn create_test_settings_manager() -> (SettingsManager, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Set up a temporary home directory for testing
    env::set_var("HOME", temp_dir.path());

    let manager = SettingsManager::new();
    (manager, temp_dir)
  }

  #[test]
  fn test_settings_manager_creation() {
    let (_manager, _temp_dir) = create_test_settings_manager();
    // Test passes if no panic occurs
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
    let (manager, _temp_dir) = create_test_settings_manager();

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
    let (manager, _temp_dir) = create_test_settings_manager();

    let test_settings = AppSettings {
      set_as_default_browser: true,
      theme: "dark".to_string(),
      custom_theme: None,
      api_enabled: false,
      api_port: 10108,
      api_token: None,
    };

    // Save settings
    let save_result = manager.save_settings(&test_settings);
    assert!(save_result.is_ok(), "Should save settings successfully");

    // Load settings back
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
    let (manager, _temp_dir) = create_test_settings_manager();

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
    let (manager, _temp_dir) = create_test_settings_manager();

    let test_sorting = TableSortingSettings {
      column: "browser".to_string(),
      direction: "desc".to_string(),
    };

    // Save sorting
    let save_result = manager.save_table_sorting(&test_sorting);
    assert!(save_result.is_ok(), "Should save sorting successfully");

    // Load sorting back
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
  fn test_should_show_settings_on_startup() {
    let (manager, _temp_dir) = create_test_settings_manager();

    let result = manager.should_show_settings_on_startup();
    assert!(result.is_ok(), "Should not fail");

    let should_show = result.unwrap();
    assert!(
      !should_show,
      "Should always return false as per implementation"
    );
  }

  #[test]
  fn test_load_corrupted_settings_file() {
    let (manager, _temp_dir) = create_test_settings_manager();

    // Create settings directory
    let settings_dir = manager.get_settings_dir();
    fs::create_dir_all(&settings_dir).expect("Should create settings directory");

    // Write corrupted JSON
    let settings_file = manager.get_settings_file();
    fs::write(&settings_file, "{ invalid json }").expect("Should write corrupted file");

    // Should handle corrupted file gracefully
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
    let (manager, _temp_dir) = create_test_settings_manager();

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

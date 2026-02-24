//! Encrypted storage for VPN configurations.

use super::config::{VpnConfig, VpnError, VpnType};
use aes_gcm::{
  aead::{Aead, KeyInit},
  Aes256Gcm, Nonce,
};
use chrono::Utc;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Storage format version for migration support
const STORAGE_VERSION: u32 = 1;

/// Stored VPN configs container
#[derive(Debug, Serialize, Deserialize)]
struct VpnStorageData {
  version: u32,
  configs: Vec<StoredVpnConfig>,
}

/// Encrypted VPN config as stored on disk
#[derive(Debug, Serialize, Deserialize)]
struct StoredVpnConfig {
  id: String,
  name: String,
  vpn_type: VpnType,
  encrypted_data: String, // Base64 encoded encrypted config
  nonce: String,          // Base64 encoded nonce
  created_at: i64,
  last_used: Option<i64>,
  #[serde(default)]
  sync_enabled: bool,
  #[serde(default)]
  last_sync: Option<u64>,
}

/// VPN storage manager with encryption
pub struct VpnStorage {
  storage_path: PathBuf,
  encryption_key: [u8; 32],
}

impl Default for VpnStorage {
  fn default() -> Self {
    Self::new()
  }
}

impl VpnStorage {
  /// Create a new VPN storage manager
  pub fn new() -> Self {
    let storage_path = Self::get_storage_path();
    let encryption_key = Self::get_or_create_key();

    Self {
      storage_path,
      encryption_key,
    }
  }

  /// Create a VPN storage manager with a custom storage directory
  pub fn with_dir(dir: &std::path::Path) -> Self {
    let storage_path = dir.join("vpn_configs.json");
    let key_path = dir.join(".vpn_key");

    let encryption_key = if key_path.exists() {
      if let Ok(key_data) = fs::read(&key_path) {
        if key_data.len() == 32 {
          let mut key = [0u8; 32];
          key.copy_from_slice(&key_data);
          key
        } else {
          let key: [u8; 32] = rand::rng().random();
          let _ = fs::write(&key_path, key);
          key
        }
      } else {
        let key: [u8; 32] = rand::rng().random();
        let _ = fs::write(&key_path, key);
        key
      }
    } else {
      let key: [u8; 32] = rand::rng().random();
      let _ = fs::write(&key_path, key);
      key
    };

    Self {
      storage_path,
      encryption_key,
    }
  }

  /// Get the storage file path
  fn get_storage_path() -> PathBuf {
    let vpn_dir = crate::app_dirs::vpn_dir();
    if !vpn_dir.exists() {
      let _ = fs::create_dir_all(&vpn_dir);
    }
    Self::migrate_from_old_location(&vpn_dir);
    vpn_dir.join("vpn_configs.json")
  }

  /// Get or create the encryption key
  fn get_or_create_key() -> [u8; 32] {
    let key_path = crate::app_dirs::vpn_dir().join(".vpn_key");

    if key_path.exists() {
      if let Ok(key_data) = fs::read(&key_path) {
        if key_data.len() == 32 {
          let mut key = [0u8; 32];
          key.copy_from_slice(&key_data);
          return key;
        }
      }
    }

    // Generate a new key
    let key: [u8; 32] = rand::rng().random();
    let _ = fs::write(&key_path, key);

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
    }

    key
  }

  /// Migrate VPN configs from the old ProjectDirs location to the new app_dirs location.
  fn migrate_from_old_location(new_dir: &std::path::Path) {
    let old_dir = match directories::ProjectDirs::from("com", "donut", "donutbrowser") {
      Some(dirs) => dirs.data_local_dir().to_path_buf(),
      None => return,
    };

    for filename in &["vpn_configs.json", ".vpn_key"] {
      let old_path = old_dir.join(filename);
      let new_path = new_dir.join(filename);
      if old_path.exists() && !new_path.exists() {
        let _ = fs::copy(&old_path, &new_path);
      }
    }
  }

  /// Load storage data from disk
  fn load_storage(&self) -> Result<VpnStorageData, VpnError> {
    if !self.storage_path.exists() {
      return Ok(VpnStorageData {
        version: STORAGE_VERSION,
        configs: Vec::new(),
      });
    }

    let content = fs::read_to_string(&self.storage_path)
      .map_err(|e| VpnError::Storage(format!("Failed to read storage file: {e}")))?;

    serde_json::from_str(&content)
      .map_err(|e| VpnError::Storage(format!("Failed to parse storage file: {e}")))
  }

  /// Save storage data to disk
  fn save_storage(&self, data: &VpnStorageData) -> Result<(), VpnError> {
    let content = serde_json::to_string_pretty(data)
      .map_err(|e| VpnError::Storage(format!("Failed to serialize storage: {e}")))?;

    fs::write(&self.storage_path, content)
      .map_err(|e| VpnError::Storage(format!("Failed to write storage file: {e}")))?;

    Ok(())
  }

  /// Encrypt config data
  fn encrypt(&self, data: &str) -> Result<(String, String), VpnError> {
    let cipher = Aes256Gcm::new_from_slice(&self.encryption_key)
      .map_err(|e| VpnError::Encryption(format!("Failed to create cipher: {e}")))?;

    let nonce_bytes: [u8; 12] = rand::rng().random();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
      .encrypt(nonce, data.as_bytes())
      .map_err(|e| VpnError::Encryption(format!("Encryption failed: {e}")))?;

    Ok((
      base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &ciphertext),
      base64::Engine::encode(&base64::engine::general_purpose::STANDARD, nonce_bytes),
    ))
  }

  /// Decrypt config data
  fn decrypt(&self, encrypted_data: &str, nonce_str: &str) -> Result<String, VpnError> {
    let cipher = Aes256Gcm::new_from_slice(&self.encryption_key)
      .map_err(|e| VpnError::Encryption(format!("Failed to create cipher: {e}")))?;

    let ciphertext =
      base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encrypted_data)
        .map_err(|e| VpnError::Encryption(format!("Failed to decode ciphertext: {e}")))?;

    let nonce_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, nonce_str)
      .map_err(|e| VpnError::Encryption(format!("Failed to decode nonce: {e}")))?;

    if nonce_bytes.len() != 12 {
      return Err(VpnError::Encryption("Invalid nonce length".to_string()));
    }

    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = cipher
      .decrypt(nonce, ciphertext.as_ref())
      .map_err(|e| VpnError::Encryption(format!("Decryption failed: {e}")))?;

    String::from_utf8(plaintext)
      .map_err(|e| VpnError::Encryption(format!("Failed to decode plaintext: {e}")))
  }

  /// Save a VPN configuration
  pub fn save_config(&self, config: &VpnConfig) -> Result<(), VpnError> {
    let mut storage = self.load_storage()?;

    // Encrypt the config data
    let (encrypted_data, nonce) = self.encrypt(&config.config_data)?;

    let stored = StoredVpnConfig {
      id: config.id.clone(),
      name: config.name.clone(),
      vpn_type: config.vpn_type,
      encrypted_data,
      nonce,
      created_at: config.created_at,
      last_used: config.last_used,
      sync_enabled: config.sync_enabled,
      last_sync: config.last_sync,
    };

    // Update existing or add new
    if let Some(pos) = storage.configs.iter().position(|c| c.id == config.id) {
      storage.configs[pos] = stored;
    } else {
      storage.configs.push(stored);
    }

    self.save_storage(&storage)
  }

  /// Load a VPN configuration by ID
  pub fn load_config(&self, id: &str) -> Result<VpnConfig, VpnError> {
    let storage = self.load_storage()?;

    let stored = storage
      .configs
      .iter()
      .find(|c| c.id == id)
      .ok_or_else(|| VpnError::NotFound(id.to_string()))?;

    let config_data = self.decrypt(&stored.encrypted_data, &stored.nonce)?;

    Ok(VpnConfig {
      id: stored.id.clone(),
      name: stored.name.clone(),
      vpn_type: stored.vpn_type,
      config_data,
      created_at: stored.created_at,
      last_used: stored.last_used,
      sync_enabled: stored.sync_enabled,
      last_sync: stored.last_sync,
    })
  }

  /// List all VPN configurations (without decrypted config data)
  pub fn list_configs(&self) -> Result<Vec<VpnConfig>, VpnError> {
    let storage = self.load_storage()?;

    Ok(
      storage
        .configs
        .iter()
        .map(|stored| VpnConfig {
          id: stored.id.clone(),
          name: stored.name.clone(),
          vpn_type: stored.vpn_type,
          config_data: String::new(), // Don't include config data in list
          created_at: stored.created_at,
          last_used: stored.last_used,
          sync_enabled: stored.sync_enabled,
          last_sync: stored.last_sync,
        })
        .collect(),
    )
  }

  /// Delete a VPN configuration
  pub fn delete_config(&self, id: &str) -> Result<(), VpnError> {
    let mut storage = self.load_storage()?;

    let initial_len = storage.configs.len();
    storage.configs.retain(|c| c.id != id);

    if storage.configs.len() == initial_len {
      return Err(VpnError::NotFound(id.to_string()));
    }

    self.save_storage(&storage)
  }

  /// Update last_used timestamp
  pub fn update_last_used(&self, id: &str) -> Result<(), VpnError> {
    let mut storage = self.load_storage()?;

    if let Some(config) = storage.configs.iter_mut().find(|c| c.id == id) {
      config.last_used = Some(Utc::now().timestamp());
      self.save_storage(&storage)
    } else {
      Err(VpnError::NotFound(id.to_string()))
    }
  }

  /// Create a VPN config manually from validated data
  pub fn create_config_manual(
    &self,
    name: &str,
    vpn_type: VpnType,
    config_data: &str,
  ) -> Result<VpnConfig, VpnError> {
    // Validate the config by parsing it
    match vpn_type {
      VpnType::WireGuard => {
        super::parse_wireguard_config(config_data)?;
      }
      VpnType::OpenVPN => {
        super::parse_openvpn_config(config_data)?;
      }
    }

    let id = Uuid::new_v4().to_string();
    let sync_enabled = crate::sync::is_sync_configured();

    let config = VpnConfig {
      id,
      name: name.to_string(),
      vpn_type,
      config_data: config_data.to_string(),
      created_at: Utc::now().timestamp(),
      last_used: None,
      sync_enabled,
      last_sync: None,
    };

    self.save_config(&config)?;

    Ok(config)
  }

  /// Update the name of an existing VPN config
  pub fn update_config_name(&self, id: &str, new_name: &str) -> Result<VpnConfig, VpnError> {
    let mut config = self.load_config(id)?;
    config.name = new_name.to_string();
    self.save_config(&config)?;
    Ok(config)
  }

  /// Update sync fields on a VPN config
  pub fn update_sync_fields(
    &self,
    id: &str,
    sync_enabled: bool,
    last_sync: Option<u64>,
  ) -> Result<(), VpnError> {
    let mut storage = self.load_storage()?;

    if let Some(config) = storage.configs.iter_mut().find(|c| c.id == id) {
      config.sync_enabled = sync_enabled;
      config.last_sync = last_sync;
      self.save_storage(&storage)
    } else {
      Err(VpnError::NotFound(id.to_string()))
    }
  }

  /// Import a VPN config from raw content
  pub fn import_config(
    &self,
    content: &str,
    filename: &str,
    name: Option<String>,
  ) -> Result<VpnConfig, VpnError> {
    let vpn_type = super::detect_vpn_type(content, filename)?;

    // Validate the config by parsing it
    match vpn_type {
      VpnType::WireGuard => {
        super::parse_wireguard_config(content)?;
      }
      VpnType::OpenVPN => {
        super::parse_openvpn_config(content)?;
      }
    }

    let id = Uuid::new_v4().to_string();
    let display_name = name.unwrap_or_else(|| {
      // Generate name from filename
      let base = filename.trim_end_matches(".conf").trim_end_matches(".ovpn");
      format!("{} ({})", base, vpn_type)
    });
    let sync_enabled = crate::sync::is_sync_configured();

    let config = VpnConfig {
      id,
      name: display_name,
      vpn_type,
      config_data: content.to_string(),
      created_at: Utc::now().timestamp(),
      last_used: None,
      sync_enabled,
      last_sync: None,
    };

    self.save_config(&config)?;

    Ok(config)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn create_test_storage() -> (VpnStorage, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = VpnStorage::with_dir(temp_dir.path());
    (storage, temp_dir)
  }

  #[test]
  fn test_encrypt_decrypt_roundtrip() {
    let (storage, _temp) = create_test_storage();
    let original = "This is a secret VPN configuration";

    let (encrypted, nonce) = storage.encrypt(original).unwrap();
    let decrypted = storage.decrypt(&encrypted, &nonce).unwrap();

    assert_eq!(original, decrypted);
  }

  #[test]
  fn test_save_and_load_config() {
    let (storage, _temp) = create_test_storage();

    let config = VpnConfig {
      id: "test-id-123".to_string(),
      name: "Test VPN".to_string(),
      vpn_type: VpnType::WireGuard,
      config_data: "[Interface]\nPrivateKey = test\n[Peer]\nPublicKey = peer".to_string(),
      created_at: 1234567890,
      last_used: None,
      sync_enabled: false,
      last_sync: None,
    };

    storage.save_config(&config).unwrap();
    let loaded = storage.load_config("test-id-123").unwrap();

    assert_eq!(loaded.id, config.id);
    assert_eq!(loaded.name, config.name);
    assert_eq!(loaded.vpn_type, config.vpn_type);
    assert_eq!(loaded.config_data, config.config_data);
  }

  #[test]
  fn test_list_configs() {
    let (storage, _temp) = create_test_storage();

    let config1 = VpnConfig {
      id: "id-1".to_string(),
      name: "VPN 1".to_string(),
      vpn_type: VpnType::WireGuard,
      config_data: "secret1".to_string(),
      created_at: 1000,
      last_used: None,
      sync_enabled: false,
      last_sync: None,
    };

    let config2 = VpnConfig {
      id: "id-2".to_string(),
      name: "VPN 2".to_string(),
      vpn_type: VpnType::OpenVPN,
      config_data: "secret2".to_string(),
      created_at: 2000,
      last_used: Some(3000),
      sync_enabled: false,
      last_sync: None,
    };

    storage.save_config(&config1).unwrap();
    storage.save_config(&config2).unwrap();

    let configs = storage.list_configs().unwrap();
    assert_eq!(configs.len(), 2);

    // Config data should be empty in listing
    assert!(configs[0].config_data.is_empty());
    assert!(configs[1].config_data.is_empty());
  }

  #[test]
  fn test_delete_config() {
    let (storage, _temp) = create_test_storage();

    let config = VpnConfig {
      id: "delete-me".to_string(),
      name: "To Delete".to_string(),
      vpn_type: VpnType::WireGuard,
      config_data: "data".to_string(),
      created_at: 1000,
      last_used: None,
      sync_enabled: false,
      last_sync: None,
    };

    storage.save_config(&config).unwrap();
    assert!(storage.load_config("delete-me").is_ok());

    storage.delete_config("delete-me").unwrap();
    assert!(storage.load_config("delete-me").is_err());
  }

  #[test]
  fn test_load_nonexistent_config() {
    let (storage, _temp) = create_test_storage();
    let result = storage.load_config("nonexistent");
    assert!(result.is_err());
  }
}

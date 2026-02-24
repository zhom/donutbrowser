use aes_gcm::{
  aead::{Aead, AeadCore, KeyInit, OsRng},
  Aes256Gcm, Key,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

const E2E_FILE_HEADER: &[u8] = b"DBE2E";
const E2E_FILE_VERSION: u8 = 1;

fn get_e2e_password_path() -> std::path::PathBuf {
  crate::app_dirs::settings_dir().join("e2e_password.dat")
}

fn get_vault_password() -> String {
  env!("DONUT_BROWSER_VAULT_PASSWORD").to_string()
}

pub fn store_e2e_password(password: &str) -> Result<(), String> {
  let file_path = get_e2e_password_path();

  if let Some(parent) = file_path.parent() {
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
  }

  let vault_password = get_vault_password();
  let salt = SaltString::generate(&mut OsRng);
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
  let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

  let ciphertext = cipher
    .encrypt(&nonce, password.as_bytes())
    .map_err(|e| format!("Encryption failed: {e}"))?;

  let mut file_data = Vec::new();
  file_data.extend_from_slice(E2E_FILE_HEADER);
  file_data.push(E2E_FILE_VERSION);

  let salt_str = salt.as_str();
  file_data.push(salt_str.len() as u8);
  file_data.extend_from_slice(salt_str.as_bytes());
  file_data.extend_from_slice(&nonce);
  file_data.extend_from_slice(&(ciphertext.len() as u32).to_le_bytes());
  file_data.extend_from_slice(&ciphertext);

  std::fs::write(&file_path, file_data)
    .map_err(|e| format!("Failed to write e2e password file: {e}"))?;

  Ok(())
}

pub fn load_e2e_password() -> Result<Option<String>, String> {
  let file_path = get_e2e_password_path();
  if !file_path.exists() {
    return Ok(None);
  }

  let file_data =
    std::fs::read(&file_path).map_err(|e| format!("Failed to read e2e password file: {e}"))?;

  if file_data.len() < E2E_FILE_HEADER.len() + 1 {
    return Ok(None);
  }

  if &file_data[..E2E_FILE_HEADER.len()] != E2E_FILE_HEADER {
    return Ok(None);
  }

  let version = file_data[E2E_FILE_HEADER.len()];
  if version != E2E_FILE_VERSION {
    return Ok(None);
  }

  let mut offset = E2E_FILE_HEADER.len() + 1;

  if offset >= file_data.len() {
    return Ok(None);
  }
  let salt_len = file_data[offset] as usize;
  offset += 1;

  if offset + salt_len > file_data.len() {
    return Ok(None);
  }
  let salt_str = std::str::from_utf8(&file_data[offset..offset + salt_len])
    .map_err(|_| "Invalid salt encoding")?;
  offset += salt_len;

  let salt = SaltString::from_b64(salt_str).map_err(|e| format!("Invalid salt: {e}"))?;

  if offset + 12 > file_data.len() {
    return Ok(None);
  }
  let nonce_bytes: [u8; 12] = file_data[offset..offset + 12]
    .try_into()
    .map_err(|_| "Invalid nonce")?;
  let nonce = aes_gcm::Nonce::from(nonce_bytes);
  offset += 12;

  if offset + 4 > file_data.len() {
    return Ok(None);
  }
  let ciphertext_len =
    u32::from_le_bytes(file_data[offset..offset + 4].try_into().unwrap()) as usize;
  offset += 4;

  if offset + ciphertext_len > file_data.len() {
    return Ok(None);
  }
  let ciphertext = &file_data[offset..offset + ciphertext_len];

  let vault_password = get_vault_password();
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
    .map_err(|e| format!("Decryption failed: {e}"))?;

  let password =
    String::from_utf8(plaintext).map_err(|e| format!("Invalid UTF-8 in password: {e}"))?;

  Ok(Some(password))
}

pub fn has_e2e_password() -> bool {
  get_e2e_password_path().exists()
}

pub fn remove_e2e_password() -> Result<(), String> {
  let file_path = get_e2e_password_path();
  if file_path.exists() {
    std::fs::remove_file(&file_path)
      .map_err(|e| format!("Failed to remove e2e password file: {e}"))?;
  }
  Ok(())
}

/// Derive a per-profile encryption key using Argon2id
pub fn derive_profile_key(user_password: &str, profile_salt: &str) -> Result<[u8; 32], String> {
  let salt_bytes = BASE64
    .decode(profile_salt)
    .map_err(|e| format!("Invalid salt encoding: {e}"))?;

  let salt = SaltString::encode_b64(&salt_bytes)
    .map_err(|e| format!("Failed to create salt string: {e}"))?;

  let argon2 = Argon2::default();
  let password_hash = argon2
    .hash_password(user_password.as_bytes(), &salt)
    .map_err(|e| format!("Key derivation failed: {e}"))?;
  let hash_value = password_hash.hash.unwrap();
  let hash_bytes = hash_value.as_bytes();

  let mut key = [0u8; 32];
  key.copy_from_slice(&hash_bytes[..32]);
  Ok(key)
}

/// Generate a random 16-byte salt, base64-encoded
pub fn generate_salt() -> String {
  let mut salt = [0u8; 16];
  use aes_gcm::aead::rand_core::RngCore;
  OsRng.fill_bytes(&mut salt);
  BASE64.encode(salt)
}

/// Encrypt bytes with AES-256-GCM. Output format: [nonce 12B][ciphertext]
pub fn encrypt_bytes(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
  let aes_key = Key::<Aes256Gcm>::from(*key);
  let cipher = Aes256Gcm::new(&aes_key);
  let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

  let ciphertext = cipher
    .encrypt(&nonce, plaintext)
    .map_err(|e| format!("Encryption failed: {e}"))?;

  let mut output = Vec::with_capacity(12 + ciphertext.len());
  output.extend_from_slice(&nonce);
  output.extend_from_slice(&ciphertext);
  Ok(output)
}

/// Decrypt bytes encrypted with encrypt_bytes. Input format: [nonce 12B][ciphertext]
pub fn decrypt_bytes(key: &[u8; 32], encrypted: &[u8]) -> Result<Vec<u8>, String> {
  if encrypted.len() < 12 {
    return Err("Encrypted data too short".to_string());
  }

  let nonce_bytes: [u8; 12] = encrypted[..12].try_into().map_err(|_| "Invalid nonce")?;
  let nonce = aes_gcm::Nonce::from(nonce_bytes);
  let ciphertext = &encrypted[12..];

  let aes_key = Key::<Aes256Gcm>::from(*key);
  let cipher = Aes256Gcm::new(&aes_key);

  cipher
    .decrypt(&nonce, ciphertext)
    .map_err(|e| format!("Decryption failed: {e}"))
}

// Tauri commands

#[tauri::command]
pub fn set_e2e_password(password: String) -> Result<(), String> {
  if password.len() < 8 {
    return Err("Password must be at least 8 characters".to_string());
  }
  store_e2e_password(&password)
}

#[tauri::command]
pub fn check_has_e2e_password() -> bool {
  has_e2e_password()
}

#[tauri::command]
pub fn delete_e2e_password() -> Result<(), String> {
  remove_e2e_password()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_encrypt_decrypt_roundtrip() {
    let key = [42u8; 32];
    let plaintext = b"Hello, World!";
    let encrypted = encrypt_bytes(&key, plaintext).unwrap();
    let decrypted = decrypt_bytes(&key, &encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
  }

  #[test]
  fn test_encrypt_decrypt_empty_data() {
    let key = [1u8; 32];
    let plaintext = b"";
    let encrypted = encrypt_bytes(&key, plaintext).unwrap();
    let decrypted = decrypt_bytes(&key, &encrypted).unwrap();
    assert_eq!(decrypted, plaintext.to_vec());
  }

  #[test]
  fn test_encrypt_decrypt_large_data() {
    let key = [7u8; 32];
    let plaintext = vec![0xABu8; 1_048_576]; // 1MB
    let encrypted = encrypt_bytes(&key, &plaintext).unwrap();
    let decrypted = decrypt_bytes(&key, &encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
  }

  #[test]
  fn test_different_keys_different_ciphertext() {
    let key1 = [1u8; 32];
    let key2 = [2u8; 32];
    let plaintext = b"same data";
    let encrypted1 = encrypt_bytes(&key1, plaintext).unwrap();
    let encrypted2 = encrypt_bytes(&key2, plaintext).unwrap();
    // Nonces are random so ciphertexts will differ regardless,
    // but decrypting with wrong key should fail
    assert!(decrypt_bytes(&key2, &encrypted1).is_err());
    assert!(decrypt_bytes(&key1, &encrypted2).is_err());
  }

  #[test]
  fn test_nonce_uniqueness() {
    let key = [5u8; 32];
    let plaintext = b"same data encrypted twice";
    let encrypted1 = encrypt_bytes(&key, plaintext).unwrap();
    let encrypted2 = encrypt_bytes(&key, plaintext).unwrap();
    // Different nonces should produce different ciphertext
    assert_ne!(encrypted1, encrypted2);
    // But both should decrypt to the same plaintext
    assert_eq!(
      decrypt_bytes(&key, &encrypted1).unwrap(),
      decrypt_bytes(&key, &encrypted2).unwrap()
    );
  }

  #[test]
  fn test_wrong_key_fails() {
    let key = [10u8; 32];
    let wrong_key = [20u8; 32];
    let plaintext = b"secret data";
    let encrypted = encrypt_bytes(&key, plaintext).unwrap();
    assert!(decrypt_bytes(&wrong_key, &encrypted).is_err());
  }

  #[test]
  fn test_key_derivation_deterministic() {
    let salt = generate_salt();
    let key1 = derive_profile_key("my_password", &salt).unwrap();
    let key2 = derive_profile_key("my_password", &salt).unwrap();
    assert_eq!(key1, key2);
  }

  #[test]
  fn test_key_derivation_different_salts() {
    let salt1 = generate_salt();
    let salt2 = generate_salt();
    let key1 = derive_profile_key("my_password", &salt1).unwrap();
    let key2 = derive_profile_key("my_password", &salt2).unwrap();
    assert_ne!(key1, key2);
  }

  #[test]
  fn test_salt_generation_unique() {
    let salt1 = generate_salt();
    let salt2 = generate_salt();
    assert_ne!(salt1, salt2);
  }

  #[test]
  fn test_password_storage_roundtrip() {
    let password = "test_password_12345";
    store_e2e_password(password).unwrap();
    assert!(has_e2e_password());
    let loaded = load_e2e_password().unwrap();
    assert_eq!(loaded, Some(password.to_string()));
    remove_e2e_password().unwrap();
    assert!(!has_e2e_password());
  }

  #[test]
  fn test_decrypt_too_short_data() {
    let key = [1u8; 32];
    assert!(decrypt_bytes(&key, &[0u8; 5]).is_err());
  }
}

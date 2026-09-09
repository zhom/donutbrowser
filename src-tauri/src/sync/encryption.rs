use aes_gcm::{
  aead::{Aead, KeyInit},
  Aes256Gcm, Key,
};
use argon2::Argon2;
use base64::{
  engine::general_purpose::{STANDARD as BASE64, STANDARD_NO_PAD as SALT_B64},
  Engine,
};

/// Derive a 32-byte AES key from a password and a raw salt with Argon2id at
/// the crate's default parameters (m=19456 KiB, t=2, p=1, 32-byte output).
///
/// ONE function for every vault in the app, so the parameters can never drift
/// between the sync, settings and cloud-auth stores. Byte-compatible with the
/// PHC-string path used before argon2 0.6: that path hashed the DECODED salt
/// with the same defaults and the key was its 32-byte output, which is exactly
/// what `hash_password_into` produces here. A different parameter set would
/// silently lock every user out of their encrypted data, so the defaults are
/// pinned by the test below rather than trusted.
pub fn derive_vault_key(password: &[u8], salt: &[u8]) -> Result<[u8; 32], String> {
  // Filled by the KDF. It starts as noise rather than zeros so that no
  // failure path can ever hand back an all-zero key.
  let mut key: [u8; 32] = rand::rng().random();
  Argon2::default()
    .hash_password_into(password, salt, &mut key)
    .map_err(|e| format!("Argon2 key derivation failed: {e}"))?;
  Ok(key)
}

/// The on-disk salt encoding: PHC "B64", the standard alphabet with no
/// padding, exactly what the retired `SaltString` wrote, so files written by
/// earlier builds decode unchanged.
pub fn encode_salt(salt: &[u8]) -> String {
  SALT_B64.encode(salt)
}

pub fn decode_salt(salt: &str) -> Result<Vec<u8>, String> {
  SALT_B64
    .decode(salt)
    .map_err(|e| format!("Invalid salt: {e}"))
}
use rand::RngExt;
use std::collections::HashMap;
use std::sync::Mutex;

/// Argon2id is intentionally expensive (~80–150 ms per call). During an
/// encryption rollover, every synced entity (proxy, group, vpn, extension,
/// extension group, profile metadata) goes through `derive_profile_key`,
/// which without caching means hundreds of sequential 100 ms derivations.
///
/// Cache the derived key keyed on (sha256(password), salt). Entries are
/// evicted on `set_e2e_password` / `delete_e2e_password` so a password
/// change cannot use stale keys.
type DerivedKeyCache = HashMap<([u8; 32], String), [u8; 32]>;
static KEY_CACHE: std::sync::LazyLock<Mutex<DerivedKeyCache>> =
  std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn password_fingerprint(pwd: &str) -> [u8; 32] {
  use sha2::{Digest, Sha256};
  let mut hasher = Sha256::new();
  hasher.update(pwd.as_bytes());
  hasher.finalize().into()
}

fn invalidate_key_cache() {
  if let Ok(mut cache) = KEY_CACHE.lock() {
    cache.clear();
  }
}

fn get_e2e_password_path() -> std::path::PathBuf {
  crate::app_dirs::settings_dir().join("e2e_password.dat")
}

/// Header plus layout version of the sync password file.
const E2E_MAGIC: [u8; 6] = *b"DBE2E\x01";

pub fn store_e2e_password(password: &str) -> Result<(), String> {
  invalidate_key_cache();
  crate::vault::seal(&get_e2e_password_path(), &E2E_MAGIC, password)
}

pub fn load_e2e_password() -> Result<Option<String>, String> {
  crate::vault::open(&get_e2e_password_path(), &E2E_MAGIC)
}

pub fn has_e2e_password() -> bool {
  get_e2e_password_path().exists()
}

pub fn remove_e2e_password() -> Result<(), String> {
  invalidate_key_cache();
  let file_path = get_e2e_password_path();
  if file_path.exists() {
    std::fs::remove_file(&file_path)
      .map_err(|e| format!("Failed to remove e2e password file: {e}"))?;
  }
  Ok(())
}

/// Derive a per-profile encryption key using Argon2id, with an in-process
/// cache keyed on `(sha256(password), salt)`. Repeated calls with the same
/// password+salt are O(1); a password change calls `invalidate_key_cache`
/// to drop stale entries.
pub fn derive_profile_key(user_password: &str, profile_salt: &str) -> Result<[u8; 32], String> {
  let pwd_fp = password_fingerprint(user_password);
  let cache_key = (pwd_fp, profile_salt.to_string());

  if let Ok(cache) = KEY_CACHE.lock() {
    if let Some(cached) = cache.get(&cache_key) {
      return Ok(*cached);
    }
  }

  let salt_bytes = BASE64
    .decode(profile_salt)
    .map_err(|e| format!("Invalid salt encoding: {e}"))?;

  let key = derive_vault_key(user_password.as_bytes(), &salt_bytes)?;

  if let Ok(mut cache) = KEY_CACHE.lock() {
    cache.insert(cache_key, key);
  }

  Ok(key)
}

/// Generate a random 16-byte salt, base64-encoded
pub fn generate_salt() -> String {
  let salt: [u8; 16] = rand::rng().random();
  BASE64.encode(salt)
}

/// Encrypt bytes with AES-256-GCM. Output format: [nonce 12B][ciphertext]
pub fn encrypt_bytes(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
  let aes_key = Key::<Aes256Gcm>::from(*key);
  let cipher = Aes256Gcm::new(&aes_key);
  let nonce_bytes: [u8; 12] = rand::rng().random();
  let nonce = aes_gcm::Nonce::from(nonce_bytes);

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

/// Versioned encryption envelope used for non-profile entities (proxies,
/// VPNs, groups, extensions, extension groups). Each upload has its own
/// random per-entity salt so the bucket can't be rainbow-table-attacked
/// even with a shared password across many entities.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct EncryptedEnvelope {
  /// Format version. Increment when changing how `ct` is structured.
  pub v: u32,
  /// Base64 of the per-entity salt. Plaintext on the wire — salts are public.
  pub salt: String,
  /// Base64 of `nonce(12B) || AES-256-GCM ciphertext` (output of `encrypt_bytes`).
  pub ct: String,
}

/// Wrap a plaintext JSON byte slice into an encrypted envelope if the user
/// has E2E enabled. Returns `(payload_bytes, content_type)` ready to upload.
/// On no-password, returns the original JSON unchanged.
pub fn maybe_seal_for_upload(json: &[u8]) -> Result<(Vec<u8>, &'static str), String> {
  let pwd = match load_e2e_password()? {
    Some(p) => p,
    None => return Ok((json.to_vec(), "application/json")),
  };
  let salt = generate_salt();
  let key = derive_profile_key(&pwd, &salt)?;
  let ct = encrypt_bytes(&key, json)?;
  let envelope = EncryptedEnvelope {
    v: 1,
    salt,
    ct: BASE64.encode(&ct),
  };
  let payload =
    serde_json::to_vec(&envelope).map_err(|e| format!("Failed to serialize envelope: {e}"))?;
  Ok((payload, "application/json"))
}

/// Reverse of `maybe_seal_for_upload`. Returns the inner plaintext JSON
/// bytes regardless of whether `raw` was an envelope or legacy plaintext.
///
/// Distinguishes three cases:
/// - `raw` is plaintext JSON, no password set → returns `raw` unchanged.
/// - `raw` is an envelope, password set → decrypts and returns plaintext.
/// - `raw` is an envelope, no password set → returns `Err(EncryptedEnvelope)`
///   so callers (subscription / startup probe) can show "enter password to
///   continue syncing" UI.
pub fn maybe_unseal_after_download(raw: &[u8]) -> Result<Vec<u8>, String> {
  // Try parsing as envelope first; envelopes are JSON objects with a "v" field.
  if let Ok(env) = serde_json::from_slice::<EncryptedEnvelope>(raw) {
    if env.v != 1 {
      return Err(format!("Unsupported envelope version: {}", env.v));
    }
    let pwd = load_e2e_password()?.ok_or_else(|| "ENCRYPTION_PASSWORD_REQUIRED".to_string())?;
    let key = derive_profile_key(&pwd, &env.salt)?;
    let ct = BASE64
      .decode(&env.ct)
      .map_err(|e| format!("Invalid envelope ciphertext: {e}"))?;
    return decrypt_bytes(&key, &ct);
  }
  // Not an envelope — legacy plaintext. Caller will JSON-parse it directly.
  Ok(raw.to_vec())
}

// Tauri commands

#[tauri::command]
pub async fn set_e2e_password(password: String) -> Result<(), String> {
  if password.len() < 8 {
    return Err("Password must be at least 8 characters".to_string());
  }
  enforce_team_owner_for_encryption_change().await?;
  store_e2e_password(&password)
}

#[tauri::command]
pub fn check_has_e2e_password() -> bool {
  has_e2e_password()
}

#[tauri::command]
pub fn verify_e2e_password(password: String) -> Result<bool, String> {
  match load_e2e_password()? {
    Some(stored) => Ok(stored == password),
    None => Err(serde_json::json!({ "code": "NO_E2E_PASSWORD_SET" }).to_string()),
  }
}

#[tauri::command]
pub async fn delete_e2e_password() -> Result<(), String> {
  enforce_team_owner_for_encryption_change().await?;
  remove_e2e_password()
}

/// Only the team owner may flip the E2E password state — otherwise members
/// could lock each other out by changing the key.
async fn enforce_team_owner_for_encryption_change() -> Result<(), String> {
  use crate::cloud_auth::CLOUD_AUTH;
  if let Some(state) = CLOUD_AUTH.get_user().await {
    if state.user.effective_plan() == "team" && state.user.team_role.as_deref() != Some("owner") {
      return Err("TEAM_OWNER_ONLY".to_string());
    }
  }
  Ok(())
}

#[cfg(test)]
#[path = "encryption_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "encryption_vault_key_tests.rs"]
mod vault_key_tests;

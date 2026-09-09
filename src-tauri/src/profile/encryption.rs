//! Per-file encryption for password-protected profiles.
//!
//! Each on-disk file in `profiles/{uuid}/profile/` has:
//! - **Filename**: `urlsafe_no_pad(HMAC-SHA256(profile_key, plaintext_relpath))[..32]`.
//!   Deterministic so cross-machine sync sees stable filenames; same plaintext
//!   path with same key always produces the same on-disk name.
//! - **Content**: `nonce(12B) || AES-256-GCM(profile_key, path_len(2B-LE) || plaintext_path || file_bytes)`.
//!   The plaintext relpath is encoded inside the ciphertext so a launch can
//!   reconstruct the directory tree without a separate manifest.
//!
//! Wrong password fails the AES-GCM auth tag on the first decrypt, which
//! doubles as password verification.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ring::hmac;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::sync::encryption::{decrypt_bytes, derive_profile_key, encrypt_bytes, generate_salt};

/// Length of the on-disk HMAC filename in chars.
const HMAC_FILENAME_LEN: usize = 32;

/// Marker file written into encrypted profile dirs so launch code can verify
/// the password before attempting to decrypt actual user data files.
const VERIFY_FILE_NAME: &str = ".donut-pw-verify";
const VERIFY_FILE_PATH: &str = "__donut_pw_verify__";

lazy_static::lazy_static! {
  /// In-memory cache of derived per-profile encryption keys, keyed by profile UUID.
  /// Only populated while a profile is unlocked / running. Never persisted.
  static ref KEY_CACHE: Mutex<HashMap<uuid::Uuid, [u8; 32]>> = Mutex::new(HashMap::new());
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
  #[error("io error: {0}")]
  Io(String),
  #[error("encryption error: {0}")]
  Encryption(String),
  #[error("invalid password")]
  WrongPassword,
  #[error("invalid file format")]
  InvalidFormat,
}

pub type PasswordResult<T> = Result<T, PasswordError>;

impl From<std::io::Error> for PasswordError {
  fn from(e: std::io::Error) -> Self {
    PasswordError::Io(e.to_string())
  }
}

/// Compute the HMAC-SHA256 derived on-disk filename for a plaintext relative path.
pub fn hmac_filename(key: &[u8; 32], plaintext_relpath: &str) -> String {
  let signing_key = hmac::Key::new(hmac::HMAC_SHA256, key);
  let tag = hmac::sign(&signing_key, plaintext_relpath.as_bytes());
  let encoded = URL_SAFE_NO_PAD.encode(tag.as_ref());
  encoded.chars().take(HMAC_FILENAME_LEN).collect()
}

/// Encrypt a single file's contents with its plaintext relative path embedded.
pub fn encrypt_profile_file(
  key: &[u8; 32],
  plaintext_relpath: &str,
  file_bytes: &[u8],
) -> PasswordResult<Vec<u8>> {
  let path_bytes = plaintext_relpath.as_bytes();
  if path_bytes.len() > u16::MAX as usize {
    return Err(PasswordError::Encryption("relpath too long".into()));
  }
  let mut plaintext = Vec::with_capacity(2 + path_bytes.len() + file_bytes.len());
  plaintext.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
  plaintext.extend_from_slice(path_bytes);
  plaintext.extend_from_slice(file_bytes);
  encrypt_bytes(key, &plaintext).map_err(PasswordError::Encryption)
}

/// Decrypt one file's bytes back into `(plaintext_relpath, file_bytes)`.
pub fn decrypt_profile_file(
  key: &[u8; 32],
  encrypted_bytes: &[u8],
) -> PasswordResult<(String, Vec<u8>)> {
  let plaintext = decrypt_bytes(key, encrypted_bytes).map_err(|_| PasswordError::WrongPassword)?;
  if plaintext.len() < 2 {
    return Err(PasswordError::InvalidFormat);
  }
  let path_len = u16::from_le_bytes([plaintext[0], plaintext[1]]) as usize;
  if plaintext.len() < 2 + path_len {
    return Err(PasswordError::InvalidFormat);
  }
  let path = std::str::from_utf8(&plaintext[2..2 + path_len])
    .map_err(|_| PasswordError::InvalidFormat)?
    .to_string();
  let content = plaintext[2 + path_len..].to_vec();
  Ok((path, content))
}

fn build_excludes(patterns: &[&str]) -> GlobSet {
  let mut builder = GlobSetBuilder::new();
  for p in patterns {
    if let Ok(g) = Glob::new(p) {
      builder.add(g);
    }
  }
  builder.build().unwrap_or_else(|_| GlobSet::empty())
}

fn walk_files(
  base: &Path,
  current: &Path,
  excludes: &GlobSet,
  out: &mut Vec<(String, PathBuf)>,
) -> std::io::Result<()> {
  for entry in std::fs::read_dir(current)? {
    let entry = entry?;
    let path = entry.path();
    let relative = path
      .strip_prefix(base)
      .map(|p| p.to_string_lossy().replace('\\', "/"))
      .unwrap_or_default();

    if excludes.is_match(&relative) {
      continue;
    }

    let metadata = match entry.metadata() {
      Ok(m) => m,
      Err(_) => continue,
    };

    if metadata.is_dir() {
      walk_files(base, &path, excludes, out)?;
    } else if metadata.is_file() {
      out.push((relative, path));
    }
  }
  Ok(())
}

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  let tmp = path.with_extension("donut-tmp");
  std::fs::write(&tmp, data)?;
  std::fs::rename(&tmp, path)
}

fn write_verifier(key: &[u8; 32], encrypted_dir: &Path) -> PasswordResult<()> {
  let encrypted = encrypt_profile_file(key, VERIFY_FILE_PATH, b"donut-verify")?;
  let path = encrypted_dir.join(VERIFY_FILE_NAME);
  atomic_write(&path, &encrypted)?;
  Ok(())
}

/// Verify a derived key against an encrypted profile dir. Returns Ok(()) on
/// success, `Err(WrongPassword)` if the password is wrong, or another error
/// for I/O / format problems.
pub fn verify_key_against_dir(key: &[u8; 32], encrypted_dir: &Path) -> PasswordResult<()> {
  let path = encrypted_dir.join(VERIFY_FILE_NAME);
  if !path.exists() {
    return Err(PasswordError::InvalidFormat);
  }
  let bytes = std::fs::read(&path)?;
  let (relpath, content) = decrypt_profile_file(key, &bytes)?;
  if relpath != VERIFY_FILE_PATH || content != b"donut-verify" {
    return Err(PasswordError::InvalidFormat);
  }
  Ok(())
}

/// Encrypt every file under `plaintext_dir` into `encrypted_dir`, replacing
/// it. Files matching `exclude_patterns` are dropped.
pub fn encrypt_profile_dir(
  key: &[u8; 32],
  plaintext_dir: &Path,
  encrypted_dir: &Path,
  exclude_patterns: &[&str],
) -> PasswordResult<()> {
  if encrypted_dir.exists() {
    std::fs::remove_dir_all(encrypted_dir)?;
  }
  std::fs::create_dir_all(encrypted_dir)?;

  let excludes = build_excludes(exclude_patterns);
  let mut files = Vec::new();
  if plaintext_dir.exists() {
    walk_files(plaintext_dir, plaintext_dir, &excludes, &mut files)?;
  }

  for (relpath, abs) in files {
    let bytes = std::fs::read(&abs)?;
    let encrypted = encrypt_profile_file(key, &relpath, &bytes)?;
    let on_disk = encrypted_dir.join(hmac_filename(key, &relpath));
    atomic_write(&on_disk, &encrypted)?;
  }

  write_verifier(key, encrypted_dir)?;
  Ok(())
}

/// Decrypt every file in `encrypted_dir` back into `plaintext_dir` (which is
/// created if missing). Returns the per-file mtimes captured after writing,
/// keyed by plaintext relpath. Caller can use them as the "before-launch"
/// snapshot to skip unchanged files on re-encrypt.
pub fn decrypt_profile_dir(
  key: &[u8; 32],
  encrypted_dir: &Path,
  plaintext_dir: &Path,
) -> PasswordResult<HashMap<String, SystemTime>> {
  std::fs::create_dir_all(plaintext_dir)?;
  let mut mtimes = HashMap::new();

  let entries: Vec<_> = std::fs::read_dir(encrypted_dir)?
    .filter_map(|r| r.ok())
    .collect();

  for entry in entries {
    let path = entry.path();
    if !path.is_file() {
      continue;
    }
    let name = match path.file_name().and_then(|n| n.to_str()) {
      Some(n) => n,
      None => continue,
    };
    if name == VERIFY_FILE_NAME {
      continue;
    }
    let bytes = std::fs::read(&path)?;
    let (relpath, content) = decrypt_profile_file(key, &bytes)?;
    let dest = plaintext_dir.join(&relpath);
    if let Some(parent) = dest.parent() {
      std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&dest, &content)?;
    if let Ok(m) = dest.metadata().and_then(|m| m.modified()) {
      mtimes.insert(relpath, m);
    }
  }

  Ok(mtimes)
}

/// Re-encrypt the contents of `plaintext_dir` back into `encrypted_dir`,
/// preserving on-disk filenames for files whose plaintext content didn't
/// change. Returns the number of files re-encrypted.
///
/// `before_launch_mtimes` is the snapshot captured by `decrypt_profile_dir`.
/// Files whose mtime hasn't moved are left untouched on disk.
pub fn reencrypt_changed_files(
  key: &[u8; 32],
  plaintext_dir: &Path,
  encrypted_dir: &Path,
  exclude_patterns: &[&str],
  before_launch_mtimes: &HashMap<String, SystemTime>,
) -> PasswordResult<usize> {
  std::fs::create_dir_all(encrypted_dir)?;
  let excludes = build_excludes(exclude_patterns);

  let mut current_files = Vec::new();
  if plaintext_dir.exists() {
    walk_files(plaintext_dir, plaintext_dir, &excludes, &mut current_files)?;
  }

  let mut current_paths: HashSet<String> = HashSet::new();
  let mut rewrote = 0usize;
  for (relpath, abs) in current_files {
    current_paths.insert(relpath.clone());

    let cur_mtime = abs.metadata().and_then(|m| m.modified()).ok();
    let unchanged = match (cur_mtime, before_launch_mtimes.get(&relpath)) {
      (Some(now), Some(before)) => now == *before,
      _ => false,
    };
    if unchanged {
      continue;
    }

    let bytes = std::fs::read(&abs)?;
    let encrypted = encrypt_profile_file(key, &relpath, &bytes)?;
    let on_disk = encrypted_dir.join(hmac_filename(key, &relpath));
    atomic_write(&on_disk, &encrypted)?;
    rewrote += 1;
  }

  // Delete on-disk files for plaintext paths that no longer exist
  let valid_names: HashSet<String> = current_paths
    .iter()
    .map(|p| hmac_filename(key, p))
    .collect();

  for entry in std::fs::read_dir(encrypted_dir)?.flatten() {
    let path = entry.path();
    if !path.is_file() {
      continue;
    }
    let name = match path.file_name().and_then(|n| n.to_str()) {
      Some(n) => n.to_string(),
      None => continue,
    };
    if name == VERIFY_FILE_NAME {
      continue;
    }
    if !valid_names.contains(&name) {
      let _ = std::fs::remove_file(&path);
    }
  }

  write_verifier(key, encrypted_dir)?;
  Ok(rewrote)
}

/// Re-encrypt every file under `encrypted_dir` from `old_key` to `new_key` in
/// place. Used when changing a profile password without launching it.
pub fn rekey_profile_dir(
  old_key: &[u8; 32],
  new_key: &[u8; 32],
  encrypted_dir: &Path,
) -> PasswordResult<()> {
  let entries: Vec<_> = std::fs::read_dir(encrypted_dir)?
    .filter_map(|r| r.ok())
    .collect();

  let mut decrypted: Vec<(String, Vec<u8>)> = Vec::new();
  for entry in &entries {
    let path = entry.path();
    if !path.is_file() {
      continue;
    }
    let name = match path.file_name().and_then(|n| n.to_str()) {
      Some(n) => n,
      None => continue,
    };
    if name == VERIFY_FILE_NAME {
      continue;
    }
    let bytes = std::fs::read(&path)?;
    let (relpath, content) = decrypt_profile_file(old_key, &bytes)?;
    decrypted.push((relpath, content));
  }

  // Decryption succeeded for every file; safe to rewrite the directory.
  for entry in entries {
    let path = entry.path();
    if path.is_file() {
      let _ = std::fs::remove_file(&path);
    }
  }

  for (relpath, content) in decrypted {
    let encrypted = encrypt_profile_file(new_key, &relpath, &content)?;
    let on_disk = encrypted_dir.join(hmac_filename(new_key, &relpath));
    atomic_write(&on_disk, &encrypted)?;
  }

  write_verifier(new_key, encrypted_dir)?;
  Ok(())
}

// ---------- key cache ----------

pub fn cache_key(profile_id: uuid::Uuid, key: [u8; 32]) {
  if let Ok(mut guard) = KEY_CACHE.lock() {
    guard.insert(profile_id, key);
  }
}

pub fn get_cached_key(profile_id: &uuid::Uuid) -> Option<[u8; 32]> {
  KEY_CACHE.lock().ok()?.get(profile_id).copied()
}

pub fn drop_cached_key(profile_id: &uuid::Uuid) {
  if let Ok(mut guard) = KEY_CACHE.lock() {
    guard.remove(profile_id);
  }
}

pub fn has_cached_key(profile_id: &uuid::Uuid) -> bool {
  KEY_CACHE
    .lock()
    .map(|g| g.contains_key(profile_id))
    .unwrap_or(false)
}

/// Convenience: derive + verify against the encrypted dir + cache the key on success.
pub fn unlock(
  profile_id: uuid::Uuid,
  password: &str,
  salt: &str,
  encrypted_dir: &Path,
) -> PasswordResult<()> {
  let key = derive_profile_key(password, salt).map_err(PasswordError::Encryption)?;
  verify_key_against_dir(&key, encrypted_dir)?;
  cache_key(profile_id, key);
  Ok(())
}

pub fn fresh_salt() -> String {
  generate_salt()
}

#[cfg(test)]
#[path = "encryption_tests.rs"]
mod tests;

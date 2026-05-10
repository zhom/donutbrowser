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
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn make_key() -> [u8; 32] {
    derive_profile_key("hunter2", &generate_salt()).unwrap()
  }

  #[test]
  fn test_hmac_filename_deterministic() {
    let key = [7u8; 32];
    let a = hmac_filename(&key, "Default/Cookies");
    let b = hmac_filename(&key, "Default/Cookies");
    assert_eq!(a, b);
    assert_eq!(a.len(), HMAC_FILENAME_LEN);
  }

  #[test]
  fn test_hmac_filename_different_keys() {
    let a = hmac_filename(&[1u8; 32], "Default/Cookies");
    let b = hmac_filename(&[2u8; 32], "Default/Cookies");
    assert_ne!(a, b);
  }

  #[test]
  fn test_hmac_filename_different_paths() {
    let key = [1u8; 32];
    let a = hmac_filename(&key, "Default/Cookies");
    let b = hmac_filename(&key, "Default/Login Data");
    assert_ne!(a, b);
  }

  #[test]
  fn test_file_roundtrip() {
    let key = make_key();
    let original = b"hello world".to_vec();
    let encrypted = encrypt_profile_file(&key, "Default/Cookies", &original).unwrap();
    let (path, content) = decrypt_profile_file(&key, &encrypted).unwrap();
    assert_eq!(path, "Default/Cookies");
    assert_eq!(content, original);
  }

  #[test]
  fn test_file_wrong_key_fails() {
    let key1 = make_key();
    let key2 = make_key();
    let encrypted = encrypt_profile_file(&key1, "Cookies", b"data").unwrap();
    assert!(matches!(
      decrypt_profile_file(&key2, &encrypted),
      Err(PasswordError::WrongPassword)
    ));
  }

  #[test]
  fn test_file_truncated_ciphertext() {
    let key = make_key();
    let encrypted = encrypt_profile_file(&key, "x", b"y").unwrap();
    // Drop the auth tag
    let truncated = &encrypted[..encrypted.len() - 1];
    assert!(decrypt_profile_file(&key, truncated).is_err());
  }

  #[test]
  fn test_dir_roundtrip() {
    let key = make_key();
    let work = TempDir::new().unwrap();
    let plain = work.path().join("plain");
    let enc = work.path().join("enc");
    std::fs::create_dir_all(plain.join("Default")).unwrap();
    std::fs::write(plain.join("Default/Cookies"), b"sqlite-data").unwrap();
    std::fs::write(plain.join("Default/Bookmarks"), b"{\"x\":1}").unwrap();
    std::fs::write(plain.join("Local State"), b"state").unwrap();

    encrypt_profile_dir(&key, &plain, &enc, &[]).unwrap();

    // No plaintext filenames on disk
    let names: Vec<String> = std::fs::read_dir(&enc)
      .unwrap()
      .filter_map(|e| e.ok())
      .map(|e| e.file_name().to_string_lossy().into_owned())
      .collect();
    for n in &names {
      assert!(!n.contains("Cookies"), "plaintext leaked: {n}");
      assert!(!n.contains("Bookmarks"));
      assert!(!n.contains("Local State"));
    }

    // Verify file present
    assert!(enc.join(VERIFY_FILE_NAME).exists());

    let restored = work.path().join("restored");
    let mtimes = decrypt_profile_dir(&key, &enc, &restored).unwrap();
    assert_eq!(mtimes.len(), 3);

    assert_eq!(
      std::fs::read(restored.join("Default/Cookies")).unwrap(),
      b"sqlite-data"
    );
    assert_eq!(
      std::fs::read(restored.join("Default/Bookmarks")).unwrap(),
      b"{\"x\":1}"
    );
    assert_eq!(
      std::fs::read(restored.join("Local State")).unwrap(),
      b"state"
    );
  }

  #[test]
  fn test_dir_excludes() {
    let key = make_key();
    let work = TempDir::new().unwrap();
    let plain = work.path().join("plain");
    let enc = work.path().join("enc");
    std::fs::create_dir_all(plain.join("Default/Cache")).unwrap();
    std::fs::write(plain.join("Default/Cookies"), b"keep").unwrap();
    std::fs::write(plain.join("Default/Cache/data"), b"drop").unwrap();

    encrypt_profile_dir(&key, &plain, &enc, &["**/Cache/**"]).unwrap();

    let restored = work.path().join("restored");
    let mtimes = decrypt_profile_dir(&key, &enc, &restored).unwrap();

    // Only Cookies (1 file) should be present, not Cache contents
    assert_eq!(mtimes.len(), 1);
    assert!(mtimes.contains_key("Default/Cookies"));
    assert!(restored.join("Default/Cookies").exists());
    assert!(!restored.join("Default/Cache/data").exists());
  }

  #[test]
  fn test_verify_against_wrong_key() {
    let key1 = make_key();
    let key2 = make_key();
    let work = TempDir::new().unwrap();
    let plain = work.path().join("plain");
    let enc = work.path().join("enc");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("file"), b"data").unwrap();
    encrypt_profile_dir(&key1, &plain, &enc, &[]).unwrap();
    assert!(verify_key_against_dir(&key1, &enc).is_ok());
    assert!(matches!(
      verify_key_against_dir(&key2, &enc),
      Err(PasswordError::WrongPassword)
    ));
  }

  #[test]
  fn test_reencrypt_skips_unchanged() {
    let key = make_key();
    let work = TempDir::new().unwrap();
    let plain = work.path().join("plain");
    let enc = work.path().join("enc");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("a"), b"AAA").unwrap();
    std::fs::write(plain.join("b"), b"BBB").unwrap();
    encrypt_profile_dir(&key, &plain, &enc, &[]).unwrap();

    let restored = work.path().join("restored");
    let snapshot = decrypt_profile_dir(&key, &enc, &restored).unwrap();

    // Capture pre-rewrite ciphertext bytes
    let name_a = hmac_filename(&key, "a");
    let name_b = hmac_filename(&key, "b");
    let cipher_a_before = std::fs::read(enc.join(&name_a)).unwrap();
    let cipher_b_before = std::fs::read(enc.join(&name_b)).unwrap();

    // Modify only "a" in the restored tree
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(restored.join("a"), b"AAA-CHANGED").unwrap();

    let rewrote = reencrypt_changed_files(&key, &restored, &enc, &[], &snapshot).unwrap();
    assert_eq!(rewrote, 1);

    let cipher_a_after = std::fs::read(enc.join(&name_a)).unwrap();
    let cipher_b_after = std::fs::read(enc.join(&name_b)).unwrap();
    assert_ne!(
      cipher_a_before, cipher_a_after,
      "changed file should have new ciphertext"
    );
    assert_eq!(
      cipher_b_before, cipher_b_after,
      "unchanged file should have stable ciphertext"
    );
  }

  #[test]
  fn test_reencrypt_handles_added_and_removed() {
    let key = make_key();
    let work = TempDir::new().unwrap();
    let plain = work.path().join("plain");
    let enc = work.path().join("enc");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("keep"), b"k").unwrap();
    std::fs::write(plain.join("delete"), b"d").unwrap();
    encrypt_profile_dir(&key, &plain, &enc, &[]).unwrap();

    let restored = work.path().join("restored");
    let snapshot = decrypt_profile_dir(&key, &enc, &restored).unwrap();

    std::fs::remove_file(restored.join("delete")).unwrap();
    std::fs::write(restored.join("new"), b"n").unwrap();

    reencrypt_changed_files(&key, &restored, &enc, &[], &snapshot).unwrap();

    let names: HashSet<String> = std::fs::read_dir(&enc)
      .unwrap()
      .filter_map(|e| e.ok())
      .map(|e| e.file_name().to_string_lossy().into_owned())
      .collect();

    assert!(names.contains(&hmac_filename(&key, "keep")));
    assert!(names.contains(&hmac_filename(&key, "new")));
    assert!(!names.contains(&hmac_filename(&key, "delete")));
    assert!(names.contains(VERIFY_FILE_NAME));
  }

  #[test]
  fn test_rekey_changes_filenames_and_content() {
    let old = make_key();
    let new = make_key();
    let work = TempDir::new().unwrap();
    let plain = work.path().join("plain");
    let enc = work.path().join("enc");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("x"), b"data").unwrap();
    encrypt_profile_dir(&old, &plain, &enc, &[]).unwrap();

    let old_name = hmac_filename(&old, "x");
    let new_name = hmac_filename(&new, "x");
    assert_ne!(old_name, new_name);

    rekey_profile_dir(&old, &new, &enc).unwrap();

    assert!(!enc.join(&old_name).exists());
    assert!(enc.join(&new_name).exists());
    verify_key_against_dir(&new, &enc).unwrap();
    assert!(matches!(
      verify_key_against_dir(&old, &enc),
      Err(PasswordError::WrongPassword)
    ));

    let restored = work.path().join("restored");
    decrypt_profile_dir(&new, &enc, &restored).unwrap();
    assert_eq!(std::fs::read(restored.join("x")).unwrap(), b"data");
  }

  #[test]
  fn test_atomic_write_leaves_original_intact_if_tmp_lingers() {
    let work = TempDir::new().unwrap();
    let target = work.path().join("file");
    std::fs::write(&target, b"original").unwrap();

    // Simulate a stale tmp from a crashed write
    std::fs::write(target.with_extension("donut-tmp"), b"partial").unwrap();

    // A successful write should overwrite the original even when stale tmp exists
    atomic_write(&target, b"new").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
  }

  #[test]
  fn test_key_cache_lifecycle() {
    let id = uuid::Uuid::new_v4();
    assert!(!has_cached_key(&id));
    cache_key(id, [9u8; 32]);
    assert!(has_cached_key(&id));
    assert_eq!(get_cached_key(&id), Some([9u8; 32]));
    drop_cached_key(&id);
    assert!(!has_cached_key(&id));
  }

  #[test]
  fn test_unlock_helper() {
    let work = TempDir::new().unwrap();
    let plain = work.path().join("plain");
    let enc = work.path().join("enc");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("x"), b"data").unwrap();

    let salt = generate_salt();
    let key = derive_profile_key("correct horse", &salt).unwrap();
    encrypt_profile_dir(&key, &plain, &enc, &[]).unwrap();

    let id = uuid::Uuid::new_v4();
    drop_cached_key(&id);
    assert!(unlock(id, "wrong", &salt, &enc).is_err());
    assert!(!has_cached_key(&id));
    assert!(unlock(id, "correct horse", &salt, &enc).is_ok());
    assert!(has_cached_key(&id));
    drop_cached_key(&id);
  }
}

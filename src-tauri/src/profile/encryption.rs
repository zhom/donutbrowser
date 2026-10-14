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
//! Wrong password fails the AES-GCM auth tag on the verifier file, which
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

/// Extension of a file `atomic_write` has not renamed into place yet. One left
/// by a crash is never a sealed file.
const TMP_EXTENSION: &str = "donut-tmp";

/// What a sealed copy leaves out: caches the browser rebuilds, lock and temp
/// files, crash and metrics dumps, and the files Donut writes again before
/// every launch. Everything else is kept, the SQLite journals and write-ahead
/// logs included: without them a database the browser did not close cleanly
/// opens torn instead of rolled back.
pub const SEALED_EXCLUDE_PATTERNS: &[&str] = &[
  "**/Cache/**",
  "**/Code Cache/**",
  "**/GPUCache/**",
  "**/GrShaderCache/**",
  "**/ShaderCache/**",
  "**/DawnCache/**",
  "**/DawnGraphiteCache/**",
  "**/DawnWebGPUCache/**",
  "**/GraphiteDawnCache/**",
  "**/Service Worker/CacheStorage/**",
  "**/Service Worker/ScriptCache/**",
  "**/blob_storage/**",
  "**/Crashpad/**",
  "**/Crash Reports/**",
  "**/BrowserMetrics*",
  "**/BrowserMetrics/**",
  "**/optimization_guide_model_store/**",
  "**/Safe Browsing/**",
  "**/component_crx_cache/**",
  "**/cache2/**",
  "**/startupCache/**",
  "**/safebrowsing/**",
  "**/storage/temporary/**",
  "**/storage/default/*/cache/**",
  "**/datareporting/**",
  "**/saved-telemetry-pings/**",
  "**/crashes/**",
  "**/minidumps/**",
  "**/*-shm",
  "**/LOCK",
  "**/LOG",
  "**/LOG.old",
  "**/SingletonLock",
  "**/SingletonSocket",
  "**/SingletonCookie",
  "**/.DS_Store",
  "*.tmp",
  "**/*.donut-tmp",
  "**/wayfern-identity.json",
  "**/wayfern-persona.json",
  BASELINE_FILE_NAME,
];

/// The file in a decrypted copy that records what the sealed copy holds. It is
/// written only once the copy is complete and removed before the copy is
/// erased, so a directory without it is never mistaken for a profile.
pub const BASELINE_FILE_NAME: &str = ".donut-baseline.json";

/// A file's size and modification time, enough to tell that it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileStamp {
  secs: u64,
  nanos: u32,
  len: u64,
}

impl FileStamp {
  fn of(metadata: &std::fs::Metadata) -> Option<Self> {
    let modified = metadata
      .modified()
      .ok()?
      .duration_since(SystemTime::UNIX_EPOCH)
      .ok()?;
    Some(Self {
      secs: modified.as_secs(),
      nanos: modified.subsec_nanos(),
      len: metadata.len(),
    })
  }
}

/// The stamp of every file of a decrypted copy, by plaintext relpath, as of the
/// moment the sealed copy last held the same bytes.
pub type Baseline = HashMap<String, FileStamp>;

/// The result of decrypting a sealed copy.
pub struct Decrypted {
  pub baseline: Baseline,
  /// Sealed files that did not open with the right key: damaged, and skipped.
  pub unreadable: usize,
}

/// The stamp of the sealed copy as a whole. Every seal and every password
/// change rewrites the verifier, and a sync download replaces it, so a new
/// stamp means the sealed copy changed.
pub fn sealed_stamp(encrypted_dir: &Path) -> Option<FileStamp> {
  FileStamp::of(&std::fs::metadata(encrypted_dir.join(VERIFY_FILE_NAME)).ok()?)
}

/// Whether `plaintext_dir` differs from `baseline`: a file added, changed or
/// removed.
pub fn has_changes(
  plaintext_dir: &Path,
  exclude_patterns: &[&str],
  baseline: &Baseline,
) -> PasswordResult<bool> {
  let excludes = build_excludes(exclude_patterns);
  let mut files = Vec::new();
  walk_files(plaintext_dir, plaintext_dir, &excludes, &mut files)?;
  if files.len() != baseline.len() {
    return Ok(true);
  }
  for (relpath, abs) in files {
    if FileStamp::of(&std::fs::metadata(&abs)?).as_ref() != baseline.get(&relpath) {
      return Ok(true);
    }
  }
  Ok(false)
}

fn is_tmp_name(name: &str) -> bool {
  Path::new(name)
    .extension()
    .is_some_and(|ext| ext == TMP_EXTENSION)
}

/// In-memory cache of derived per-profile encryption keys, keyed by profile UUID.
/// Only populated while a profile is unlocked / running. Never persisted.
static KEY_CACHE: std::sync::LazyLock<Mutex<HashMap<uuid::Uuid, [u8; 32]>>> =
  std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

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

    // An error, never a skip: a file left out here is a file the sealed copy
    // deletes.
    let metadata = entry.metadata()?;

    if metadata.is_dir() {
      walk_files(base, &path, excludes, out)?;
    } else if metadata.is_file() {
      out.push((relative, path));
    }
  }
  Ok(())
}

pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  let tmp = path.with_extension(TMP_EXTENSION);
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
/// created if missing), and return the baseline of what it wrote.
///
/// The key is checked against the verifier first, so a file that does not
/// open after that is damaged, not a wrong password: it is skipped and counted
/// rather than failing the whole profile.
pub fn decrypt_profile_dir(
  key: &[u8; 32],
  encrypted_dir: &Path,
  plaintext_dir: &Path,
) -> PasswordResult<Decrypted> {
  verify_key_against_dir(key, encrypted_dir)?;
  std::fs::create_dir_all(plaintext_dir)?;
  let mut decrypted = Decrypted {
    baseline: Baseline::new(),
    unreadable: 0,
  };

  for entry in std::fs::read_dir(encrypted_dir)? {
    let path = entry?.path();
    if !path.is_file() {
      continue;
    }
    let name = match path.file_name().and_then(|n| n.to_str()) {
      Some(n) => n,
      None => continue,
    };
    if name == VERIFY_FILE_NAME || is_tmp_name(name) {
      continue;
    }
    let bytes = std::fs::read(&path)?;
    let Ok((relpath, content)) = decrypt_profile_file(key, &bytes) else {
      decrypted.unreadable += 1;
      continue;
    };
    let dest = plaintext_dir.join(&relpath);
    if let Some(parent) = dest.parent() {
      std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&dest, &content)?;
    if let Some(stamp) = FileStamp::of(&std::fs::metadata(&dest)?) {
      decrypted.baseline.insert(relpath, stamp);
    }
  }

  Ok(decrypted)
}

/// Re-encrypt the contents of `plaintext_dir` back into `encrypted_dir`,
/// preserving on-disk filenames for files whose plaintext content didn't
/// change. Returns the number of files re-encrypted and the new baseline.
///
/// A file whose stamp matches `baseline` is already sealed and is left alone.
/// A sealed file whose plaintext is gone is deleted.
pub fn reencrypt_changed_files(
  key: &[u8; 32],
  plaintext_dir: &Path,
  encrypted_dir: &Path,
  exclude_patterns: &[&str],
  baseline: &Baseline,
) -> PasswordResult<(usize, Baseline)> {
  std::fs::create_dir_all(encrypted_dir)?;
  let excludes = build_excludes(exclude_patterns);

  let mut current_files = Vec::new();
  if plaintext_dir.exists() {
    walk_files(plaintext_dir, plaintext_dir, &excludes, &mut current_files)?;
  }

  let mut current_paths: HashSet<String> = HashSet::new();
  let mut sealed = Baseline::new();
  let mut rewrote = 0usize;
  for (relpath, abs) in current_files {
    current_paths.insert(relpath.clone());

    // Taken before the read, so a write that lands during it shows up as a
    // change next time rather than being missed. A file with no usable time
    // gets no entry and is sealed again every time.
    let stamp = FileStamp::of(&std::fs::metadata(&abs)?);
    if let Some(stamp) = stamp {
      sealed.insert(relpath.clone(), stamp);
      if baseline.get(&relpath) == Some(&stamp) {
        continue;
      }
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
  Ok((rewrote, sealed))
}

/// Re-encrypt every file under `encrypted_dir` from `old_key` to `new_key` in
/// place. Used when changing a profile password without launching it.
/// Returns how many damaged files were dropped: they open with neither key.
pub fn rekey_profile_dir(
  old_key: &[u8; 32],
  new_key: &[u8; 32],
  encrypted_dir: &Path,
) -> PasswordResult<usize> {
  verify_key_against_dir(old_key, encrypted_dir)?;
  let entries: Vec<_> = std::fs::read_dir(encrypted_dir)?
    .filter_map(|r| r.ok())
    .collect();

  let mut decrypted: Vec<(String, Vec<u8>)> = Vec::new();
  let mut unreadable = 0usize;
  for entry in &entries {
    let path = entry.path();
    if !path.is_file() {
      continue;
    }
    let name = match path.file_name().and_then(|n| n.to_str()) {
      Some(n) => n,
      None => continue,
    };
    if name == VERIFY_FILE_NAME || is_tmp_name(name) {
      continue;
    }
    let bytes = std::fs::read(&path)?;
    match decrypt_profile_file(old_key, &bytes) {
      Ok(file) => decrypted.push(file),
      Err(_) => unreadable += 1,
    }
  }

  // Every readable file is in memory; safe to rewrite the directory.
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
  Ok(unreadable)
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

//! Tauri commands for profile password lifecycle: set, change, remove,
//! unlock, lock, status.
//!
//! All error responses returned to the frontend are JSON-encoded
//! `{ "code": "<ERROR_CODE>", "params"?: { ... } }` so the UI can render a
//! localized message. Helpers `err_code` / `err_with` build them. The set of
//! codes is documented at `BackendErrorCode` in TypeScript; keep them in sync.

use crate::events;
use crate::profile::encryption::{
  cache_key, decrypt_profile_dir, drop_cached_key, encrypt_profile_dir, fresh_salt, get_cached_key,
  has_cached_key, rekey_profile_dir, unlock as unlock_dir, verify_key_against_dir,
};
use crate::profile::ProfileManager;
use crate::sync::encryption::derive_profile_key;
use crate::sync::manifest::DEFAULT_EXCLUDE_PATTERNS;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Build a JSON error payload with just a code.
fn err_code(code: &'static str) -> String {
  json!({ "code": code }).to_string()
}

/// Build a JSON error payload with a code and params.
fn err_with(code: &'static str, params: &[(&str, String)]) -> String {
  let mut map = serde_json::Map::new();
  for (k, v) in params {
    map.insert((*k).to_string(), serde_json::Value::String(v.clone()));
  }
  json!({ "code": code, "params": serde_json::Value::Object(map) }).to_string()
}

/// Internal-error wrapper used for unexpected failures; the detail string is
/// raw English (developer-facing) but the surrounding template translates.
fn err_internal(detail: impl std::fmt::Display) -> String {
  err_with("INTERNAL_ERROR", &[("detail", detail.to_string())])
}

lazy_static::lazy_static! {
  /// Per-profile snapshot of plaintext file mtimes captured at launch time.
  /// Used by `complete_after_quit` to skip re-encrypting unchanged files.
  static ref LAUNCH_SNAPSHOTS: Mutex<HashMap<uuid::Uuid, HashMap<String, SystemTime>>> =
    Mutex::new(HashMap::new());

  /// Profile IDs whose ephemeral dir is currently populated and matches the
  /// on-disk encrypted state, so we can skip re-decrypting on the next launch
  /// when `keep_decrypted_profiles_in_ram` is enabled.
  static ref POPULATED_EPHEMERAL: Mutex<HashSet<uuid::Uuid>> = Mutex::new(HashSet::new());

  /// Per-profile failed unlock attempt tracking for rate-limiting.
  static ref FAILED_ATTEMPTS: Mutex<HashMap<uuid::Uuid, FailureRecord>> = Mutex::new(HashMap::new());

  /// Per-profile lock serializing the whole check-lockout -> verify -> record
  /// window. `check_lockout` and `record_failed_attempt` each take and release
  /// `FAILED_ATTEMPTS` independently, with an Argon2 verification between them,
  /// so without this a burst of concurrent attempts all read the same stale
  /// count before any of them increments it and one lockout window admits as
  /// many guesses as there are worker threads.
  static ref ATTEMPT_LOCKS: Mutex<HashMap<uuid::Uuid, Arc<tokio::sync::Mutex<()>>>> =
    Mutex::new(HashMap::new());
}

/// The attempt lock for one profile. The std map lock is released before the
/// caller awaits the returned lock, so it is never held across an await.
///
/// A poisoned map degrades to serialized rather than silently unserialized.
fn attempt_lock(profile_id: &uuid::Uuid) -> Arc<tokio::sync::Mutex<()>> {
  let mut guard = ATTEMPT_LOCKS
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  // An entry only the map itself still references has no attempt in progress,
  // so dropping it here keeps a long-lived process from accumulating one lock
  // per profile ever touched. A live holder always keeps the count above 1.
  guard.retain(|_, lock| Arc::strong_count(lock) > 1);
  guard
    .entry(*profile_id)
    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
    .clone()
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct FailureRecord {
  count: u32,
  /// Stored as epoch seconds for portable on-disk persistence.
  last_failed_at_secs: u64,
}

impl FailureRecord {
  fn last_failed_at(&self) -> SystemTime {
    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(self.last_failed_at_secs)
  }
}

fn now_epoch_secs() -> u64 {
  SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

fn lockout_sidecar_path(profile_id: &uuid::Uuid) -> PathBuf {
  ProfileManager::instance()
    .get_profiles_dir()
    .join(profile_id.to_string())
    .join(".unlock-attempts.json")
}

fn load_persisted_record(profile_id: &uuid::Uuid) -> Option<FailureRecord> {
  let path = lockout_sidecar_path(profile_id);
  let content = std::fs::read_to_string(&path).ok()?;
  serde_json::from_str(&content).ok()
}

fn persist_record(profile_id: &uuid::Uuid, record: &FailureRecord) {
  let path = lockout_sidecar_path(profile_id);
  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  if let Ok(json) = serde_json::to_string(record) {
    let _ = std::fs::write(&path, json);
  }
}

fn clear_persisted_record(profile_id: &uuid::Uuid) {
  let path = lockout_sidecar_path(profile_id);
  let _ = std::fs::remove_file(&path);
}

/// Read the current FailureRecord, falling back to disk if the in-memory
/// cache doesn't have one (e.g. fresh app launch).
fn current_record(profile_id: &uuid::Uuid) -> Option<FailureRecord> {
  if let Ok(guard) = FAILED_ATTEMPTS.lock() {
    if let Some(rec) = guard.get(profile_id) {
      return Some(*rec);
    }
  }
  let from_disk = load_persisted_record(profile_id)?;
  if let Ok(mut guard) = FAILED_ATTEMPTS.lock() {
    guard.insert(*profile_id, from_disk);
  }
  Some(from_disk)
}

/// Lockout schedule. Index is the failure count (1-based); returns the
/// duration the user must wait before the next attempt is allowed.
/// Attempts 1-4 have no lockout; attempt 5 onward triggers progressive
/// back-off, capped at 24 hours.
fn lockout_for_count(count: u32) -> Option<std::time::Duration> {
  use std::time::Duration;
  let secs: u64 = match count {
    0..=4 => return None,
    5 => 60,
    6 => 5 * 60,
    7 => 15 * 60,
    8 => 60 * 60,
    9 => 2 * 60 * 60,
    10 => 4 * 60 * 60,
    11 => 8 * 60 * 60,
    _ => 24 * 60 * 60,
  };
  Some(Duration::from_secs(secs))
}

/// Returns Ok(()) if no lockout is active, or Err with remaining seconds.
fn check_lockout(profile_id: &uuid::Uuid) -> Result<(), u64> {
  let Some(record) = current_record(profile_id) else {
    return Ok(());
  };
  let Some(lockout) = lockout_for_count(record.count) else {
    return Ok(());
  };
  let elapsed = SystemTime::now()
    .duration_since(record.last_failed_at())
    .unwrap_or_default();
  if elapsed >= lockout {
    Ok(())
  } else {
    Err((lockout - elapsed).as_secs().max(1))
  }
}

fn record_failed_attempt(profile_id: uuid::Uuid) {
  let updated = if let Ok(mut guard) = FAILED_ATTEMPTS.lock() {
    let entry = guard.entry(profile_id).or_insert(FailureRecord {
      count: 0,
      last_failed_at_secs: now_epoch_secs(),
    });
    entry.count = entry.count.saturating_add(1);
    entry.last_failed_at_secs = now_epoch_secs();
    Some(*entry)
  } else {
    None
  };
  if let Some(record) = updated {
    persist_record(&profile_id, &record);
  }
}

fn clear_failed_attempts(profile_id: &uuid::Uuid) {
  if let Ok(mut guard) = FAILED_ATTEMPTS.lock() {
    guard.remove(profile_id);
  }
  clear_persisted_record(profile_id);
}

const MIN_PASSWORD_LEN: usize = 8;

fn validate_password(password: &str) -> Result<(), String> {
  if password.len() < MIN_PASSWORD_LEN {
    return Err(err_with(
      "PASSWORD_TOO_SHORT",
      &[("min", MIN_PASSWORD_LEN.to_string())],
    ));
  }
  Ok(())
}

fn parse_uuid(profile_id: &str) -> Result<uuid::Uuid, String> {
  uuid::Uuid::parse_str(profile_id).map_err(|_| err_code("INVALID_PROFILE_ID"))
}

fn load_profile(profile_id: &uuid::Uuid) -> Result<crate::profile::BrowserProfile, String> {
  let manager = ProfileManager::instance();
  let profiles = manager.list_profiles().map_err(err_internal)?;
  profiles
    .into_iter()
    .find(|p| p.id == *profile_id)
    .ok_or_else(|| err_code("PROFILE_NOT_FOUND"))
}

fn profile_data_dir(profile: &crate::profile::BrowserProfile) -> PathBuf {
  profile.get_profile_data_path(&ProfileManager::instance().get_profiles_dir())
}

fn emit_profiles_changed() {
  let _ = events::emit_empty("profiles-changed");
}

#[tauri::command]
pub async fn is_profile_locked(profile_id: String) -> Result<bool, String> {
  let id = parse_uuid(&profile_id)?;
  let profile = load_profile(&id)?;
  if !profile.password_protected {
    return Ok(false);
  }
  Ok(!has_cached_key(&id))
}

#[tauri::command]
pub async fn set_profile_password(profile_id: String, password: String) -> Result<(), String> {
  validate_password(&password)?;
  let id = parse_uuid(&profile_id)?;
  let mut profile = load_profile(&id)?;

  if profile.password_protected {
    return Err(err_code("PROFILE_ALREADY_PROTECTED"));
  }

  // Ephemeral profiles live in RAM-backed dirs that get wiped on quit, so
  // there's no on-disk data to encrypt. The two features are mutually
  // exclusive by design — fail loudly rather than silently producing a
  // half-broken state where `password_protected` is true but the encrypted
  // dir vanishes between launches.
  if profile.ephemeral {
    return Err(err_code("PROFILE_EPHEMERAL"));
  }

  if profile
    .process_id
    .is_some_and(crate::proxy_storage::is_process_running)
  {
    return Err(err_code("PROFILE_RUNNING"));
  }

  let plaintext_dir = profile_data_dir(&profile);
  // An empty/missing profile dir is fine — we just produce an encrypted dir
  // that contains only the verifier file. This lets callers attach a password
  // immediately on creation, before the browser has run.
  if !plaintext_dir.exists() {
    std::fs::create_dir_all(&plaintext_dir).map_err(err_internal)?;
  }

  let salt = fresh_salt();
  let key = derive_profile_key(&password, &salt).map_err(err_internal)?;

  // Encrypt into a sibling staging dir, then atomically swap.
  let staging = plaintext_dir.with_extension("encrypting");
  if staging.exists() {
    let _ = std::fs::remove_dir_all(&staging);
  }
  encrypt_profile_dir(&key, &plaintext_dir, &staging, DEFAULT_EXCLUDE_PATTERNS)
    .map_err(err_internal)?;

  // Move plaintext aside, swap in encrypted, then delete plaintext.
  let backup = plaintext_dir.with_extension("plaintext-backup");
  if backup.exists() {
    let _ = std::fs::remove_dir_all(&backup);
  }
  std::fs::rename(&plaintext_dir, &backup).map_err(err_internal)?;
  if let Err(e) = std::fs::rename(&staging, &plaintext_dir) {
    let _ = std::fs::rename(&backup, &plaintext_dir);
    return Err(err_internal(e));
  }
  if let Err(e) = std::fs::remove_dir_all(&backup) {
    log::warn!(
      "Failed to remove plaintext backup at {}: {e}",
      backup.display()
    );
  }

  profile.password_protected = true;
  profile.encryption_salt = Some(salt);
  ProfileManager::instance()
    .save_profile(&profile)
    .map_err(err_internal)?;

  cache_key(id, key);
  crate::sync::queue_profile_sync_if_eligible(&profile);
  emit_profiles_changed();
  Ok(())
}

/// Verify a profile password without unlocking. Used by the Settings UI's
/// "Validate" button so users can confirm they remember the password without
/// performing a destructive change. Honors the same lockout schedule as
/// `unlock_profile` so a brute-force attacker can't bypass rate-limiting by
/// hammering this command.
#[tauri::command]
pub async fn verify_profile_password(profile_id: String, password: String) -> Result<(), String> {
  let id = parse_uuid(&profile_id)?;
  let profile = load_profile(&id)?;
  if !profile.password_protected {
    return Err(err_code("PROFILE_NOT_PROTECTED"));
  }
  // Bound, never dropped early: it must cover check_lockout through the
  // record/clear branches below. See `attempt_lock`.
  let attempt = attempt_lock(&id);
  let _attempt_guard = attempt.lock().await;
  if let Err(secs) = check_lockout(&id) {
    return Err(err_with("LOCKED_OUT", &[("seconds", secs.to_string())]));
  }
  let salt = profile
    .encryption_salt
    .as_deref()
    .ok_or_else(|| err_code("PROFILE_MISSING_SALT"))?;
  let key = derive_profile_key(&password, salt).map_err(err_internal)?;
  let dir = profile_data_dir(&profile);
  match verify_key_against_dir(&key, &dir) {
    Ok(()) => {
      clear_failed_attempts(&id);
      Ok(())
    }
    Err(crate::profile::encryption::PasswordError::WrongPassword) => {
      record_failed_attempt(id);
      Err(err_code("INCORRECT_PASSWORD"))
    }
    Err(other) => Err(err_internal(other)),
  }
}

#[tauri::command]
pub async fn unlock_profile(profile_id: String, password: String) -> Result<(), String> {
  let id = parse_uuid(&profile_id)?;
  let profile = load_profile(&id)?;
  if !profile.password_protected {
    return Err(err_code("PROFILE_NOT_PROTECTED"));
  }
  // Bound, never dropped early: it must cover check_lockout through the
  // record/clear branches below. See `attempt_lock`.
  let attempt = attempt_lock(&id);
  let _attempt_guard = attempt.lock().await;
  if let Err(secs) = check_lockout(&id) {
    return Err(err_with("LOCKED_OUT", &[("seconds", secs.to_string())]));
  }
  let salt = profile
    .encryption_salt
    .as_deref()
    .ok_or_else(|| err_code("PROFILE_MISSING_SALT"))?;

  match unlock_dir(id, &password, salt, &profile_data_dir(&profile)) {
    Ok(()) => {
      clear_failed_attempts(&id);
      Ok(())
    }
    Err(crate::profile::encryption::PasswordError::WrongPassword) => {
      record_failed_attempt(id);
      Err(err_code("INCORRECT_PASSWORD"))
    }
    Err(other) => Err(err_internal(other)),
  }
}

#[tauri::command]
pub async fn lock_profile(profile_id: String) -> Result<(), String> {
  let id = parse_uuid(&profile_id)?;
  let profile = load_profile(&id)?;
  if !profile.password_protected {
    return Ok(());
  }
  if profile
    .process_id
    .is_some_and(crate::proxy_storage::is_process_running)
  {
    return Err(err_code("PROFILE_RUNNING"));
  }
  drop_cached_key(&id);
  // Purge any leftover ephemeral dir in case keep_decrypted_profiles_in_ram was on.
  crate::ephemeral_dirs::remove_ephemeral_dir(&id.to_string());
  emit_profiles_changed();
  Ok(())
}

#[tauri::command]
pub async fn change_profile_password(
  profile_id: String,
  old_password: String,
  new_password: String,
) -> Result<(), String> {
  validate_password(&new_password)?;
  let id = parse_uuid(&profile_id)?;
  let mut profile = load_profile(&id)?;

  if !profile.password_protected {
    return Err(err_code("PROFILE_NOT_PROTECTED"));
  }
  if profile
    .process_id
    .is_some_and(crate::proxy_storage::is_process_running)
  {
    return Err(err_code("PROFILE_RUNNING"));
  }

  // Bound, never dropped early: it must cover check_lockout through the
  // record/clear branches below. See `attempt_lock`.
  let attempt = attempt_lock(&id);
  let _attempt_guard = attempt.lock().await;
  if let Err(secs) = check_lockout(&id) {
    return Err(err_with("LOCKED_OUT", &[("seconds", secs.to_string())]));
  }

  let old_salt = profile
    .encryption_salt
    .as_deref()
    .ok_or_else(|| err_code("PROFILE_MISSING_SALT"))?;
  let old_key = derive_profile_key(&old_password, old_salt).map_err(err_internal)?;
  let dir = profile_data_dir(&profile);
  if let Err(e) = verify_key_against_dir(&old_key, &dir) {
    return match e {
      crate::profile::encryption::PasswordError::WrongPassword => {
        record_failed_attempt(id);
        Err(err_code("INCORRECT_PASSWORD"))
      }
      other => Err(err_internal(other)),
    };
  }
  clear_failed_attempts(&id);

  let new_salt = fresh_salt();
  let new_key = derive_profile_key(&new_password, &new_salt).map_err(err_internal)?;
  rekey_profile_dir(&old_key, &new_key, &dir).map_err(err_internal)?;

  profile.encryption_salt = Some(new_salt);
  ProfileManager::instance()
    .save_profile(&profile)
    .map_err(err_internal)?;

  drop_cached_key(&id);
  cache_key(id, new_key);
  crate::sync::queue_profile_sync_if_eligible(&profile);
  emit_profiles_changed();
  Ok(())
}

#[tauri::command]
pub async fn remove_profile_password(profile_id: String, password: String) -> Result<(), String> {
  let id = parse_uuid(&profile_id)?;
  let mut profile = load_profile(&id)?;
  if !profile.password_protected {
    return Err(err_code("PROFILE_NOT_PROTECTED"));
  }
  if profile
    .process_id
    .is_some_and(crate::proxy_storage::is_process_running)
  {
    return Err(err_code("PROFILE_RUNNING"));
  }

  // Bound, never dropped early: it must cover check_lockout through the
  // record/clear branches below. See `attempt_lock`.
  let attempt = attempt_lock(&id);
  let _attempt_guard = attempt.lock().await;
  if let Err(secs) = check_lockout(&id) {
    return Err(err_with("LOCKED_OUT", &[("seconds", secs.to_string())]));
  }

  let salt = profile
    .encryption_salt
    .as_deref()
    .ok_or_else(|| err_code("PROFILE_MISSING_SALT"))?;
  let key = derive_profile_key(&password, salt).map_err(err_internal)?;
  let encrypted_dir = profile_data_dir(&profile);
  if let Err(e) = verify_key_against_dir(&key, &encrypted_dir) {
    return match e {
      crate::profile::encryption::PasswordError::WrongPassword => {
        record_failed_attempt(id);
        Err(err_code("INCORRECT_PASSWORD"))
      }
      other => Err(err_internal(other)),
    };
  }
  clear_failed_attempts(&id);

  let staging = encrypted_dir.with_extension("decrypting");
  if staging.exists() {
    let _ = std::fs::remove_dir_all(&staging);
  }
  decrypt_profile_dir(&key, &encrypted_dir, &staging).map_err(err_internal)?;

  let backup = encrypted_dir.with_extension("encrypted-backup");
  if backup.exists() {
    let _ = std::fs::remove_dir_all(&backup);
  }
  std::fs::rename(&encrypted_dir, &backup).map_err(err_internal)?;
  if let Err(e) = std::fs::rename(&staging, &encrypted_dir) {
    let _ = std::fs::rename(&backup, &encrypted_dir);
    return Err(err_internal(e));
  }
  if let Err(e) = std::fs::remove_dir_all(&backup) {
    log::warn!(
      "Failed to remove encrypted backup at {}: {e}",
      backup.display()
    );
  }

  profile.password_protected = false;
  profile.encryption_salt = None;
  ProfileManager::instance()
    .save_profile(&profile)
    .map_err(err_internal)?;

  drop_cached_key(&id);
  crate::sync::queue_profile_sync_if_eligible(&profile);
  emit_profiles_changed();
  Ok(())
}

// ---------- helpers used by browser_runner ----------

/// Capture a per-file mtime snapshot of the given decrypted dir.
fn snapshot_mtimes(plaintext_dir: &Path) -> HashMap<String, SystemTime> {
  let mut out: HashMap<String, SystemTime> = HashMap::new();
  fn walk(
    base: &Path,
    current: &Path,
    out: &mut HashMap<String, SystemTime>,
  ) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
      let entry = entry?;
      let path = entry.path();
      let meta = entry.metadata()?;
      if meta.is_dir() {
        walk(base, &path, out)?;
      } else if meta.is_file() {
        let rel = path
          .strip_prefix(base)
          .map(|p| p.to_string_lossy().replace('\\', "/"))
          .unwrap_or_default();
        if let Ok(m) = meta.modified() {
          out.insert(rel, m);
        }
      }
    }
    Ok(())
  }
  let _ = walk(plaintext_dir, plaintext_dir, &mut out);
  out
}

/// Decrypt a password-protected profile's encrypted dir into an ephemeral
/// dir, take a mtime snapshot for diff-on-quit, and return the ephemeral
/// path the browser should launch from.
///
/// Returns an error if the profile isn't unlocked yet — the frontend should
/// prompt for the password and call `unlock_profile` first.
pub fn prepare_for_launch(profile: &crate::profile::BrowserProfile) -> Result<PathBuf, String> {
  let id = profile.id;
  let key = get_cached_key(&id).ok_or_else(|| err_code("PROFILE_LOCKED"))?;

  let id_str = id.to_string();
  let ephemeral = match crate::ephemeral_dirs::get_ephemeral_dir(&id_str) {
    Some(p) => p,
    None => crate::ephemeral_dirs::create_ephemeral_dir(&id_str).map_err(err_internal)?,
  };

  let already_populated = POPULATED_EPHEMERAL
    .lock()
    .map(|g| g.contains(&id))
    .unwrap_or(false);

  let encrypted_dir = profile_data_dir(profile);

  let snapshot = if already_populated && ephemeral_has_files(&ephemeral) {
    // Reusing a kept-in-RAM copy from the previous session; just snapshot.
    snapshot_mtimes(&ephemeral)
  } else {
    // Wipe any stale contents and re-decrypt.
    if let Err(e) = clear_dir_contents(&ephemeral) {
      log::warn!("Failed to clear stale ephemeral contents: {e}");
    }
    decrypt_profile_dir(&key, &encrypted_dir, &ephemeral).map_err(|e| match e {
      crate::profile::encryption::PasswordError::WrongPassword => err_code("INCORRECT_PASSWORD"),
      other => err_internal(other),
    })?
  };

  if let Ok(mut guard) = LAUNCH_SNAPSHOTS.lock() {
    guard.insert(id, snapshot);
  }
  if let Ok(mut guard) = POPULATED_EPHEMERAL.lock() {
    guard.insert(id);
  }

  Ok(ephemeral)
}

fn ephemeral_has_files(dir: &Path) -> bool {
  std::fs::read_dir(dir)
    .map(|mut iter| iter.next().is_some())
    .unwrap_or(false)
}

fn clear_dir_contents(dir: &Path) -> std::io::Result<()> {
  if !dir.exists() {
    return Ok(());
  }
  for entry in std::fs::read_dir(dir)? {
    let entry = entry?;
    let path = entry.path();
    if path.is_dir() {
      std::fs::remove_dir_all(&path)?;
    } else {
      std::fs::remove_file(&path)?;
    }
  }
  Ok(())
}

fn read_keep_decrypted_setting() -> bool {
  crate::settings_manager::SettingsManager::instance()
    .load_settings()
    .map(|s| s.keep_decrypted_profiles_in_ram)
    .unwrap_or(false)
}

/// Synchronous core of `complete_after_quit`: re-encrypts ephemeral → disk
/// and (unless `keep_decrypted` is true) drops cached key + purges ephemeral.
/// Returns the number of files re-encrypted, or `None` if there was nothing
/// to do. Public for testability.
pub fn complete_after_quit_blocking(
  profile: &crate::profile::BrowserProfile,
  keep_decrypted: bool,
) -> Option<usize> {
  use crate::profile::encryption::reencrypt_changed_files;

  let id = profile.id;
  if !profile.password_protected {
    return None;
  }

  // Snapshot is an optimization (skip re-encrypting unchanged files). When
  // it's missing — e.g. natural-exit detection firing twice, or status
  // checker firing for a profile whose snapshot was already consumed — we
  // fall back to treating every ephemeral file as new. Empty `before`
  // forces all files through encrypt, which is slower but correct.
  let snapshot = LAUNCH_SNAPSHOTS
    .lock()
    .ok()
    .and_then(|mut g| g.remove(&id))
    .unwrap_or_default();

  let id_str = id.to_string();
  let ephemeral = crate::ephemeral_dirs::get_ephemeral_dir(&id_str)?;
  let encrypted = profile_data_dir(profile);
  let key = get_cached_key(&id)?;

  let result = match reencrypt_changed_files(
    &key,
    &ephemeral,
    &encrypted,
    DEFAULT_EXCLUDE_PATTERNS,
    &snapshot,
  ) {
    Ok(n) => {
      log::info!("Re-encrypted {n} changed file(s) for profile {id}");
      Some(n)
    }
    Err(e) => {
      log::error!("Re-encryption failed for profile {id}: {e}");
      None
    }
  };

  if keep_decrypted {
    log::info!("Keeping decrypted copy of profile {id} in RAM (per settings)");
  } else {
    drop_cached_key(&id);
    if let Ok(mut guard) = POPULATED_EPHEMERAL.lock() {
      guard.remove(&id);
    }
    crate::ephemeral_dirs::remove_ephemeral_dir(&id_str);
  }

  result
}

/// Re-encrypt a password-protected profile's ephemeral dir back to the
/// on-disk encrypted dir after the browser process exits. Optionally purges
/// the ephemeral dir + cached key based on the global setting. Returns the
/// number of files re-encrypted (`None` when nothing to do or the profile
/// isn't protected).
///
/// Callers that release a queued sync run after a browser quit MUST await
/// this future — releasing sync while re-encryption is still in-flight
/// uploads the stale on-disk snapshot and leaves the fresh ciphertext
/// orphaned until the next scheduler tick.
pub async fn complete_after_quit_and_wait(
  profile: &crate::profile::BrowserProfile,
) -> Option<usize> {
  if !profile.password_protected {
    return None;
  }
  let keep_decrypted = read_keep_decrypted_setting();
  let profile = profile.clone();

  tokio::task::spawn_blocking(move || complete_after_quit_blocking(&profile, keep_decrypted))
    .await
    .unwrap_or_else(|e| {
      log::error!("complete_after_quit_and_wait join error: {e}");
      None
    })
}

#[cfg(test)]
#[path = "password_tests.rs"]
mod tests;

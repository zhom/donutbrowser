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
  has_cached_key, has_changes, reencrypt_changed_files, rekey_profile_dir, sealed_stamp,
  unlock as unlock_dir, verify_key_against_dir, Baseline, FileStamp, PasswordError,
  BASELINE_FILE_NAME, SEALED_EXCLUDE_PATTERNS,
};
use crate::profile::ProfileManager;
use crate::sync::encryption::derive_profile_key;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

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

/// The error a password command returns when the decrypted copy it must seal
/// first cannot be sealed. The key is the one the command just verified, so
/// `NoKey` cannot happen here.
fn seal_error(error: SealError) -> String {
  match error {
    SealError::InUse(_) => err_code("PROFILE_RUNNING"),
    SealError::NoKey => err_code("PROFILE_LOCKED"),
    SealError::Failed(e) => err_internal(e),
  }
}

/// Per-profile lock around everything that touches a profile's decrypted copy
/// or rewrites its sealed one. A quit is sealed by both the Stop button and the
/// exit watcher; unserialized, one of them read files the other was already
/// zeroing, and the sealed copy kept the zeros.
static COPY_LOCKS: std::sync::LazyLock<Mutex<HashMap<uuid::Uuid, Arc<Mutex<()>>>>> =
  std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
/// Per-profile failed unlock attempt tracking for rate-limiting.
static FAILED_ATTEMPTS: std::sync::LazyLock<Mutex<HashMap<uuid::Uuid, FailureRecord>>> =
  std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
/// Per-profile lock serializing the whole check-lockout -> verify -> record
/// window. `check_lockout` and `record_failed_attempt` each take and release
/// `FAILED_ATTEMPTS` independently, with an Argon2 verification between them,
/// so without this a burst of concurrent attempts all read the same stale
/// count before any of them increments it and one lockout window admits as
/// many guesses as there are worker threads.
static ATTEMPT_LOCKS: std::sync::LazyLock<Mutex<HashMap<uuid::Uuid, Arc<tokio::sync::Mutex<()>>>>> =
  std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

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

/// The copy lock for one profile. Held only on blocking threads: preparing a
/// launch, sealing after a quit, and the password commands.
fn copy_lock(profile_id: &uuid::Uuid) -> Arc<Mutex<()>> {
  let mut guard = COPY_LOCKS
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  guard.retain(|_, lock| Arc::strong_count(lock) > 1);
  guard.entry(*profile_id).or_default().clone()
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
  encrypt_profile_dir(&key, &plaintext_dir, &staging, SEALED_EXCLUDE_PATTERNS)
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
      "Plaintext backup removal failed path=\"{}\" err=\"{e}\"",
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
  {
    let lock = copy_lock(&id);
    let _copy = lock
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner);
    let key = get_cached_key(&id);
    match seal_copy(&id, &profile_data_dir(&profile), key.as_ref(), false) {
      // Sealed and erased, or changes that wait for the next unlock.
      Ok(Some(_)) | Err(SealError::NoKey) => {}
      // Debris or nothing: either way there is nothing to keep.
      Ok(None) => {
        crate::ephemeral_dirs::remove_ephemeral_dir(&id.to_string());
      }
      Err(SealError::InUse(_)) => return Err(err_code("PROFILE_RUNNING")),
      Err(SealError::Failed(e)) => return Err(err_internal(e)),
    }
  }
  drop_cached_key(&id);
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
  {
    let lock = copy_lock(&id);
    let _copy = lock
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner);
    // A copy with changes the old key has not sealed yet would otherwise keep
    // them out of the sealed profile for good.
    seal_copy(&id, &dir, Some(&old_key), true).map_err(seal_error)?;
    let damaged = rekey_profile_dir(&old_key, &new_key, &dir).map_err(err_internal)?;
    if damaged > 0 {
      log::error!("Damaged sealed files dropped on password change profile={id} count={damaged}");
    }
  }

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

  let lock = copy_lock(&id);
  let _copy = lock
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  seal_copy(&id, &encrypted_dir, Some(&key), false).map_err(seal_error)?;
  // Debris an earlier erase left; the profile is about to be plaintext where
  // it lives, and no copy of it may linger elsewhere.
  crate::ephemeral_dirs::remove_ephemeral_dir(&id.to_string());

  let staging = encrypted_dir.with_extension("decrypting");
  if staging.exists() {
    let _ = std::fs::remove_dir_all(&staging);
  }
  let decrypted = decrypt_profile_dir(&key, &encrypted_dir, &staging).map_err(err_internal)?;
  if decrypted.unreadable > 0 {
    log::error!(
      "Damaged sealed files dropped on password removal profile={id} count={}",
      decrypted.unreadable
    );
  }

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
      "Encrypted backup removal failed path=\"{}\" err=\"{e}\"",
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

// ---------- the decrypted copy a launch runs on ----------
//
// A launch decrypts the sealed profile into a copy the browser runs on, and the
// quit seals the copy's changes back. The copy holds `BASELINE_FILE_NAME`, the
// stamps of its files as of the last time the sealed copy matched it, from the
// moment it is complete until just before it is erased. So a copy with a
// baseline is a whole profile whose newer files still need sealing, and a copy
// without one is debris from an interrupted decrypt or erase.

/// What `BASELINE_FILE_NAME` holds.
#[derive(serde::Serialize, serde::Deserialize)]
struct CopyRecord {
  files: Baseline,
  /// The sealed copy's stamp when it last matched this copy.
  sealed: Option<FileStamp>,
}

/// How long a seal waits for the browser's processes to let go of the copy.
const RELEASE_WAIT: Duration = Duration::from_secs(10);
/// How many times a failed seal is retried, a little later each time.
const SEAL_ATTEMPTS: u32 = 3;

fn read_record(dir: &Path) -> Option<CopyRecord> {
  let bytes = std::fs::read(dir.join(BASELINE_FILE_NAME)).ok()?;
  serde_json::from_slice(&bytes).ok()
}

/// Record that the copy in `dir` matches the sealed copy in `store` as of now.
fn write_record(dir: &Path, files: Baseline, store: &Path) -> std::io::Result<()> {
  let record = CopyRecord {
    files,
    sealed: sealed_stamp(store),
  };
  let json = serde_json::to_vec(&record).map_err(std::io::Error::other)?;
  crate::profile::encryption::atomic_write(&dir.join(BASELINE_FILE_NAME), &json)
}

/// Processes started on `dir` as their user data dir, child processes
/// included: on Windows the network service still holds the cookie database
/// for a moment after the browser process has exited.
fn processes_using(dir: &Path) -> Vec<u32> {
  use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};

  let Ok(dir) = dir.canonicalize() else {
    return Vec::new();
  };
  let system = System::new_with_specifics(
    RefreshKind::nothing()
      .with_processes(ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always)),
  );
  system
    .processes()
    .iter()
    .filter(|(_, process)| {
      process.cmd().iter().any(|arg| {
        arg
          .to_str()
          .and_then(|arg| arg.strip_prefix("--user-data-dir="))
          .and_then(|value| Path::new(value.trim_matches('"')).canonicalize().ok())
          .is_some_and(|value| value == dir)
      })
    })
    .map(|(pid, _)| pid.as_u32())
    .collect()
}

/// Wait until no process runs on `dir`. Returns the ones still there at the
/// limit.
fn wait_until_released(dir: &Path) -> Result<(), Vec<u32>> {
  let started = Instant::now();
  loop {
    let pids = processes_using(dir);
    if pids.is_empty() {
      return Ok(());
    }
    if started.elapsed() >= RELEASE_WAIT {
      return Err(pids);
    }
    std::thread::sleep(Duration::from_millis(250));
  }
}

/// Seal the copy's changes into `store` and record the new baseline.
fn seal(key: &[u8; 32], dir: &Path, store: &Path, baseline: &Baseline) -> Result<usize, String> {
  let mut attempt = 1;
  loop {
    let sealed = reencrypt_changed_files(key, dir, store, SEALED_EXCLUDE_PATTERNS, baseline)
      .map_err(|e| e.to_string())
      .and_then(|(rewrote, sealed)| {
        write_record(dir, sealed, store)
          .map(|()| rewrote)
          .map_err(|e| e.to_string())
      });
    match sealed {
      Err(_) if attempt < SEAL_ATTEMPTS => {
        std::thread::sleep(Duration::from_millis(500) * attempt);
        attempt += 1;
      }
      result => return result,
    }
  }
}

/// Mark the copy as no longer a profile. Done before every erase, so an erase
/// that stops halfway leaves debris rather than something taken for one.
fn forget_record(dir: &Path) -> std::io::Result<()> {
  match std::fs::remove_file(dir.join(BASELINE_FILE_NAME)) {
    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
    _ => Ok(()),
  }
}

/// Erase the copy and forget the key.
fn erase_copy(profile_id: &uuid::Uuid, dir: &Path) {
  if let Err(e) = forget_record(dir) {
    log::error!(
      "Decrypted copy kept, its baseline could not be removed profile={profile_id} err=\"{e}\""
    );
    return;
  }
  drop_cached_key(profile_id);
  crate::ephemeral_dirs::remove_ephemeral_dir(&profile_id.to_string());
}

/// Erase a profile's decrypted copy, sealed or not, and forget its key: the
/// profile is being deleted.
pub fn discard_copy(profile_id: &uuid::Uuid) {
  let lock = copy_lock(profile_id);
  let _copy = lock
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  let id = profile_id.to_string();
  if let Some(dir) = crate::ephemeral_dirs::get_ephemeral_dir(&id) {
    if let Err(e) = forget_record(&dir) {
      log::error!(
        "Decrypted copy baseline not removed before erase profile={profile_id} err=\"{e}\""
      );
    }
  }
  drop_cached_key(profile_id);
  crate::ephemeral_dirs::remove_ephemeral_dir(&id);
}

/// Why a copy was not sealed.
enum SealError {
  /// The profile is locked; the copy waits for the next unlock.
  NoKey,
  /// A browser still runs on the copy.
  InUse(Vec<u32>),
  Failed(String),
}

/// Seal the profile's decrypted copy, if it has one with changes to seal, then
/// erase it unless `keep`. `Ok(None)` means there was no copy to seal: none was
/// made, or another caller already sealed and erased it. Hold the copy lock.
fn seal_copy(
  profile_id: &uuid::Uuid,
  store: &Path,
  key: Option<&[u8; 32]>,
  keep: bool,
) -> Result<Option<usize>, SealError> {
  let Some(dir) = crate::ephemeral_dirs::get_ephemeral_dir(&profile_id.to_string()) else {
    return Ok(None);
  };
  let Some(record) = read_record(&dir) else {
    return Ok(None);
  };
  let key = key.ok_or(SealError::NoKey)?;
  wait_until_released(&dir).map_err(SealError::InUse)?;
  let started = Instant::now();
  let rewrote = seal(key, &dir, store, &record.files).map_err(SealError::Failed)?;
  log::info!(
    "Profile re-encrypted profile={profile_id} changed_files={rewrote} keep_decrypted={keep} elapsed_ms={}",
    started.elapsed().as_millis()
  );
  if !keep {
    erase_copy(profile_id, &dir);
  }
  Ok(Some(rewrote))
}

/// Make the decrypted copy a launch runs on, and return its path.
///
/// A copy that is still there with a baseline is the newest state of the
/// profile, kept in RAM by the setting or left by a quit that could not seal
/// it: it is sealed and used as it is. Anything else is decrypted afresh.
///
/// Returns an error if the profile isn't unlocked yet — the frontend should
/// prompt for the password and call `unlock_profile` first.
pub fn prepare_for_launch(profile: &crate::profile::BrowserProfile) -> Result<PathBuf, String> {
  let id = profile.id;
  let key = get_cached_key(&id).ok_or_else(|| err_code("PROFILE_LOCKED"))?;
  let store = profile_data_dir(profile);
  let lock = copy_lock(&id);
  let _copy = lock
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);

  let id_str = id.to_string();
  let mut dir = match crate::ephemeral_dirs::get_ephemeral_dir(&id_str) {
    Some(dir) => dir,
    None => crate::ephemeral_dirs::create_ephemeral_dir(&id_str).map_err(err_internal)?,
  };
  if let Err(pids) = wait_until_released(&dir) {
    log::warn!(
      "Launch refused, a browser still runs on the decrypted copy profile={id} pids={pids:?}"
    );
    return Err(err_code("PROFILE_RUNNING"));
  }

  if let Some(record) = read_record(&dir) {
    let replaced = record.sealed != sealed_stamp(&store);
    if replaced && !has_changes(&dir, SEALED_EXCLUDE_PATTERNS, &record.files).unwrap_or(true) {
      // Sync brought a newer sealed profile while this copy waited, unused.
      log::info!("Kept decrypted copy replaced by a newer sealed profile profile={id}");
    } else {
      if replaced {
        log::warn!(
          "Sealed profile changed while its decrypted copy had changes, the copy's files win profile={id}"
        );
      }
      // The copy is the newest state either way; a failed seal only means
      // the quit seals more.
      match seal(&key, &dir, &store, &record.files) {
        Ok(0) => {}
        Ok(rewrote) => {
          log::info!("Unsealed profile changes re-encrypted profile={id} changed_files={rewrote}")
        }
        Err(e) => {
          log::error!("Profile re-encryption before launch failed profile={id} err=\"{e}\"")
        }
      }
      return Ok(dir);
    }
  }

  if std::fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_some()) {
    if forget_record(&dir).is_err() || !crate::ephemeral_dirs::remove_ephemeral_dir(&id_str) {
      return Err(err_internal(
        "the previous decrypted copy could not be erased",
      ));
    }
    dir = crate::ephemeral_dirs::create_ephemeral_dir(&id_str).map_err(err_internal)?;
  }

  let decrypted = decrypt_profile_dir(&key, &store, &dir)
    .map_err(|e| match e {
      PasswordError::WrongPassword => err_code("INCORRECT_PASSWORD"),
      other => err_internal(other),
    })
    .and_then(|decrypted| {
      write_record(&dir, decrypted.baseline, &store)
        .map(|()| decrypted.unreadable)
        .map_err(err_internal)
    });
  let unreadable = match decrypted {
    Ok(unreadable) => unreadable,
    Err(e) => {
      crate::ephemeral_dirs::remove_ephemeral_dir(&id_str);
      return Err(e);
    }
  };
  if unreadable > 0 {
    log::error!("Damaged sealed files skipped profile={id} count={unreadable}");
  }
  Ok(dir)
}

fn read_keep_decrypted_setting() -> bool {
  crate::settings_manager::SettingsManager::instance()
    .load_settings()
    .map(|s| s.keep_decrypted_profiles_in_ram)
    .unwrap_or(false)
}

/// Seal the copy the browser just quit, then erase it unless `keep_decrypted`.
/// Returns the number of files re-encrypted, or `None` when nothing was sealed:
/// no copy, already sealed by the other caller of the same quit, or a failure,
/// in which case the copy and the key stay for the next launch to seal.
fn seal_after_quit(profile_id: uuid::Uuid, store: &Path, keep_decrypted: bool) -> Option<usize> {
  let lock = copy_lock(&profile_id);
  let _copy = lock
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  let key = get_cached_key(&profile_id);
  match seal_copy(&profile_id, store, key.as_ref(), keep_decrypted) {
    Ok(rewrote) => rewrote,
    Err(SealError::NoKey) => {
      log::warn!("Profile changes kept until the profile is unlocked profile={profile_id}");
      None
    }
    Err(SealError::InUse(pids)) => {
      log::warn!(
        "Profile not re-encrypted, a browser still runs on it profile={profile_id} pids={pids:?}"
      );
      None
    }
    Err(SealError::Failed(e)) => {
      log::error!(
        "Profile re-encryption failed, decrypted copy kept profile={profile_id} err=\"{e}\""
      );
      None
    }
  }
}

/// `complete_after_quit_and_wait` on the calling thread, for tests.
#[cfg(test)]
pub fn complete_after_quit_blocking(
  profile: &crate::profile::BrowserProfile,
  keep_decrypted: bool,
) -> Option<usize> {
  if !profile.password_protected {
    return None;
  }
  seal_after_quit(profile.id, &profile_data_dir(profile), keep_decrypted)
}

/// Re-encrypt a password-protected profile's decrypted copy back to the
/// on-disk encrypted dir after the browser process exits. Purges the copy and
/// the cached key unless the global setting keeps them. Returns the number of
/// files re-encrypted (`None` when nothing was sealed or the profile isn't
/// protected).
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
  let store = profile_data_dir(profile);
  let profile_id = profile.id;

  tokio::task::spawn_blocking(move || seal_after_quit(profile_id, &store, keep_decrypted))
    .await
    .unwrap_or_else(|e| {
      log::error!("Profile re-encryption task panicked err=\"{e}\"");
      None
    })
}

#[cfg(test)]
#[path = "password_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "password_browser_tests.rs"]
mod browser_tests;

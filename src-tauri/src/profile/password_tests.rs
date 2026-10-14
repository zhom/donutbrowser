use super::*;
use crate::profile::BrowserProfile;
use tempfile::TempDir;

fn make_profile(name: &str) -> BrowserProfile {
  BrowserProfile {
    id: uuid::Uuid::new_v4(),
    name: name.to_string(),
    browser: "wayfern".to_string(),
    version: "1.0".to_string(),
    release_type: "stable".to_string(),
    ..Default::default()
  }
}

fn populate_plaintext_dir(dir: &Path) {
  std::fs::create_dir_all(dir.join("Default")).unwrap();
  std::fs::write(dir.join("Default/Cookies"), b"sqlite-data").unwrap();
  std::fs::write(dir.join("Default/Bookmarks"), b"{\"x\":1}").unwrap();
  std::fs::write(dir.join("Local State"), b"local-state").unwrap();
  // Cache files should be excluded:
  std::fs::create_dir_all(dir.join("Default/Cache")).unwrap();
  std::fs::write(dir.join("Default/Cache/data_0"), b"cache-blob").unwrap();
}

fn parse_err_code(err: &str) -> Option<&'static str> {
  let v: serde_json::Value = serde_json::from_str(err).ok()?;
  let code = v.get("code")?.as_str()?;
  Some(match code {
    "INCORRECT_PASSWORD" => "INCORRECT_PASSWORD",
    "LOCKED_OUT" => "LOCKED_OUT",
    "PROFILE_NOT_FOUND" => "PROFILE_NOT_FOUND",
    "PROFILE_NOT_PROTECTED" => "PROFILE_NOT_PROTECTED",
    "PROFILE_ALREADY_PROTECTED" => "PROFILE_ALREADY_PROTECTED",
    "PROFILE_RUNNING" => "PROFILE_RUNNING",
    "PROFILE_MISSING_SALT" => "PROFILE_MISSING_SALT",
    "PROFILE_LOCKED" => "PROFILE_LOCKED",
    "INVALID_PROFILE_ID" => "INVALID_PROFILE_ID",
    "PASSWORD_TOO_SHORT" => "PASSWORD_TOO_SHORT",
    "INTERNAL_ERROR" => "INTERNAL_ERROR",
    _ => return None,
  })
}

fn parse_err_param(err: &str, key: &str) -> Option<String> {
  let v: serde_json::Value = serde_json::from_str(err).ok()?;
  Some(v.get("params")?.get(key)?.as_str()?.to_string())
}

fn fresh_test_state(id: &uuid::Uuid) {
  drop_cached_key(id);
  crate::ephemeral_dirs::remove_ephemeral_dir(&id.to_string());
}

fn profile_full_path(profile: &BrowserProfile, profiles_dir: &Path) -> PathBuf {
  profiles_dir.join(profile.id.to_string()).join("profile")
}

/// Scratch data and decrypted-copy directories, seen by this thread and by
/// the threads a seal runs on. The thread-local override alone would send a
/// seal on another thread to the real data directory.
struct Scratch {
  _data: TempDir,
  _ephemeral: TempDir,
  _guard: crate::app_dirs::TestDirGuard,
}

impl Scratch {
  fn new() -> Self {
    let data = TempDir::new().unwrap();
    let ephemeral = TempDir::new().unwrap();
    std::env::set_var("DONUTBROWSER_DATA_DIR", data.path());
    std::env::set_var("DONUTBROWSER_EPHEMERAL_ROOT", ephemeral.path());
    let guard = crate::app_dirs::set_test_data_dir(data.path().to_path_buf());
    Self {
      _data: data,
      _ephemeral: ephemeral,
      _guard: guard,
    }
  }
}

impl Drop for Scratch {
  fn drop(&mut self) {
    std::env::remove_var("DONUTBROWSER_EPHEMERAL_ROOT");
    std::env::remove_var("DONUTBROWSER_DATA_DIR");
  }
}

const PASSWORD: &str = "hunter2!";

/// A protected profile with `populate_plaintext_dir` content, unlocked.
fn protected_profile(name: &str) -> (BrowserProfile, PathBuf) {
  let profile = make_profile(name);
  let store = profile_full_path(&profile, &ProfileManager::instance().get_profiles_dir());
  populate_plaintext_dir(&store);
  ProfileManager::instance().save_profile(&profile).unwrap();
  fresh_test_state(&profile.id);
  tokio::runtime::Runtime::new()
    .unwrap()
    .block_on(set_profile_password(
      profile.id.to_string(),
      PASSWORD.into(),
    ))
    .unwrap();
  (load_profile(&profile.id).unwrap(), store)
}

/// Write as a browser does, which is never in the same file-time tick as the
/// decrypt before it (on NTFS a tick is about 15 ms).
fn browser_writes(path: &Path, contents: impl AsRef<[u8]>) {
  std::thread::sleep(Duration::from_millis(50));
  std::fs::write(path, contents).unwrap();
}

/// What the sealed copy decrypts to, by relpath.
fn sealed_files(profile: &BrowserProfile, store: &Path) -> HashMap<String, Vec<u8>> {
  let key = derive_profile_key(PASSWORD, profile.encryption_salt.as_deref().unwrap()).unwrap();
  let out = TempDir::new().unwrap();
  let decrypted = crate::profile::encryption::decrypt_profile_dir(&key, store, out.path()).unwrap();
  assert_eq!(decrypted.unreadable, 0, "the sealed copy has damaged files");
  decrypted
    .baseline
    .keys()
    .map(|relpath| {
      (
        relpath.clone(),
        std::fs::read(out.path().join(relpath)).unwrap(),
      )
    })
    .collect()
}

#[test]
#[serial_test::serial]
fn integration_set_password_encrypts_dir() {
  let temp = TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(temp.path().to_path_buf());

  let mut profile = make_profile("test-set");
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let plain_dir = profile_full_path(&profile, &profiles_dir);
  populate_plaintext_dir(&plain_dir);
  ProfileManager::instance().save_profile(&profile).unwrap();

  fresh_test_state(&profile.id);

  let rt = tokio::runtime::Runtime::new().unwrap();
  rt.block_on(set_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
  ))
  .unwrap();

  profile = ProfileManager::instance()
    .list_profiles()
    .unwrap()
    .into_iter()
    .find(|p| p.id == profile.id)
    .unwrap();
  assert!(profile.password_protected);
  assert!(profile.encryption_salt.is_some());

  // No plaintext filenames should remain on disk
  let names: Vec<String> = std::fs::read_dir(&plain_dir)
    .unwrap()
    .filter_map(|e| e.ok())
    .map(|e| e.file_name().to_string_lossy().into_owned())
    .collect();
  for n in &names {
    assert!(!n.contains("Cookies"), "plaintext name leaked: {n}");
    assert!(!n.contains("Bookmarks"));
    assert!(!n.contains("Local State"));
  }

  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn integration_full_lifecycle_persists_data() {
  let temp = TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(temp.path().to_path_buf());

  let profile = make_profile("test-lifecycle");
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let plain_dir = profile_full_path(&profile, &profiles_dir);
  populate_plaintext_dir(&plain_dir);
  ProfileManager::instance().save_profile(&profile).unwrap();

  fresh_test_state(&profile.id);

  let rt = tokio::runtime::Runtime::new().unwrap();
  rt.block_on(set_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
  ))
  .unwrap();

  let mut profile = ProfileManager::instance()
    .list_profiles()
    .unwrap()
    .into_iter()
    .find(|p| p.id == profile.id)
    .unwrap();

  // Simulate launch: prepare_for_launch decrypts to ephemeral
  let ephemeral = prepare_for_launch(&profile).unwrap();
  assert_eq!(
    std::fs::read(ephemeral.join("Default/Cookies")).unwrap(),
    b"sqlite-data"
  );

  // Simulate user activity: modify Cookies, leave Bookmarks alone
  std::thread::sleep(std::time::Duration::from_millis(1100));
  std::fs::write(ephemeral.join("Default/Cookies"), b"sqlite-modified").unwrap();

  // Capture pre-quit ciphertext for the unchanged Bookmarks file
  let key = get_cached_key(&profile.id).unwrap();
  let bookmarks_name = crate::profile::encryption::hmac_filename(&key, "Default/Bookmarks");
  let bookmarks_cipher_before = std::fs::read(plain_dir.join(&bookmarks_name)).unwrap();

  // Simulate quit (purge=true): re-encrypts and clears cached key + ephemeral
  let n = complete_after_quit_blocking(&profile, false);
  assert!(n.is_some(), "should have re-encrypted at least one file");
  assert!(
    get_cached_key(&profile.id).is_none(),
    "key should be dropped"
  );
  assert!(
    crate::ephemeral_dirs::get_ephemeral_dir(&profile.id.to_string()).is_none(),
    "ephemeral should be purged"
  );

  // Unchanged file's ciphertext should be byte-identical
  let bookmarks_cipher_after = std::fs::read(plain_dir.join(&bookmarks_name)).unwrap();
  assert_eq!(
    bookmarks_cipher_before, bookmarks_cipher_after,
    "unchanged file's ciphertext should be stable across quit"
  );

  // Wrong password rejected
  let r = rt.block_on(unlock_profile(profile.id.to_string(), "wrong".into()));
  assert!(r.is_err());

  // Correct password unlocks
  rt.block_on(unlock_profile(profile.id.to_string(), "hunter2!".into()))
    .unwrap();

  // Re-launch and verify the modification persisted
  profile = ProfileManager::instance()
    .list_profiles()
    .unwrap()
    .into_iter()
    .find(|p| p.id == profile.id)
    .unwrap();
  let ephemeral2 = prepare_for_launch(&profile).unwrap();
  assert_eq!(
    std::fs::read(ephemeral2.join("Default/Cookies")).unwrap(),
    b"sqlite-modified",
    "modification should persist across the encrypt/decrypt cycle"
  );
  assert_eq!(
    std::fs::read(ephemeral2.join("Default/Bookmarks")).unwrap(),
    b"{\"x\":1}",
    "unchanged file should still be present"
  );

  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn integration_keep_decrypted_keeps_ephemeral_but_still_re_encrypts() {
  let temp = TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(temp.path().to_path_buf());

  let profile = make_profile("test-keep");
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let plain_dir = profile_full_path(&profile, &profiles_dir);
  populate_plaintext_dir(&plain_dir);
  ProfileManager::instance().save_profile(&profile).unwrap();

  fresh_test_state(&profile.id);

  let rt = tokio::runtime::Runtime::new().unwrap();
  rt.block_on(set_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
  ))
  .unwrap();

  let profile = ProfileManager::instance()
    .list_profiles()
    .unwrap()
    .into_iter()
    .find(|p| p.id == profile.id)
    .unwrap();
  let ephemeral = prepare_for_launch(&profile).unwrap();
  std::thread::sleep(std::time::Duration::from_millis(1100));
  std::fs::write(ephemeral.join("Default/Cookies"), b"new-bytes").unwrap();

  // keep_decrypted=true: ephemeral stays, key stays cached
  let n = complete_after_quit_blocking(&profile, true);
  assert!(n.is_some());
  assert!(
    get_cached_key(&profile.id).is_some(),
    "key should still be cached"
  );
  assert!(
    crate::ephemeral_dirs::get_ephemeral_dir(&profile.id.to_string()).is_some(),
    "ephemeral should be preserved"
  );

  // The on-disk encrypted dir was still updated
  let key = get_cached_key(&profile.id).unwrap();
  let cookies_name = crate::profile::encryption::hmac_filename(&key, "Default/Cookies");
  let cipher = std::fs::read(plain_dir.join(&cookies_name)).unwrap();
  let (path, content) = crate::profile::encryption::decrypt_profile_file(&key, &cipher).unwrap();
  assert_eq!(path, "Default/Cookies");
  assert_eq!(content, b"new-bytes");

  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn integration_change_and_remove_password() {
  let temp = TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(temp.path().to_path_buf());

  let profile = make_profile("test-change");
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let plain_dir = profile_full_path(&profile, &profiles_dir);
  populate_plaintext_dir(&plain_dir);
  ProfileManager::instance().save_profile(&profile).unwrap();

  fresh_test_state(&profile.id);
  let rt = tokio::runtime::Runtime::new().unwrap();

  rt.block_on(set_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
  ))
  .unwrap();
  let salt_v1 = ProfileManager::instance()
    .list_profiles()
    .unwrap()
    .into_iter()
    .find(|p| p.id == profile.id)
    .unwrap()
    .encryption_salt
    .clone()
    .unwrap();

  // Wrong old password should fail
  let r = rt.block_on(change_profile_password(
    profile.id.to_string(),
    "wrong".into(),
    "newpassword!".into(),
  ));
  assert!(r.is_err());

  // Correct old password works, salt should change
  rt.block_on(change_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
    "newpassword!".into(),
  ))
  .unwrap();
  let salt_v2 = ProfileManager::instance()
    .list_profiles()
    .unwrap()
    .into_iter()
    .find(|p| p.id == profile.id)
    .unwrap()
    .encryption_salt
    .clone()
    .unwrap();
  assert_ne!(salt_v1, salt_v2, "salt should rotate on password change");

  // Old password rejected, new accepted
  assert!(rt
    .block_on(unlock_profile(profile.id.to_string(), "hunter2!".into()))
    .is_err());
  rt.block_on(unlock_profile(
    profile.id.to_string(),
    "newpassword!".into(),
  ))
  .unwrap();

  // Remove password: data should be plaintext again
  rt.block_on(remove_profile_password(
    profile.id.to_string(),
    "newpassword!".into(),
  ))
  .unwrap();

  let final_profile = ProfileManager::instance()
    .list_profiles()
    .unwrap()
    .into_iter()
    .find(|p| p.id == profile.id)
    .unwrap();
  assert!(!final_profile.password_protected);
  assert!(final_profile.encryption_salt.is_none());
  assert_eq!(
    std::fs::read(plain_dir.join("Default/Cookies")).unwrap(),
    b"sqlite-data"
  );

  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn integration_empty_profile_session_survives_restart() {
  let temp = TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(temp.path().to_path_buf());

  // Mimic a freshly created profile with no browser data yet
  let profile = make_profile("test-empty");
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let plain_dir = profile_full_path(&profile, &profiles_dir);
  std::fs::create_dir_all(&plain_dir).unwrap();
  ProfileManager::instance().save_profile(&profile).unwrap();
  fresh_test_state(&profile.id);

  let rt = tokio::runtime::Runtime::new().unwrap();
  rt.block_on(set_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
  ))
  .unwrap();

  // After encrypting an empty profile, only the verifier file lives on disk
  let on_disk_count = std::fs::read_dir(&plain_dir).unwrap().count();
  assert_eq!(
    on_disk_count, 1,
    "fresh encrypted profile should have only the verifier file"
  );

  let profile = ProfileManager::instance()
    .list_profiles()
    .unwrap()
    .into_iter()
    .find(|p| p.id == profile.id)
    .unwrap();

  // Launch — ephemeral starts empty (only the verifier in encrypted, which is skipped)
  let ephemeral = prepare_for_launch(&profile).unwrap();
  let names: Vec<_> = std::fs::read_dir(&ephemeral)
    .unwrap()
    .map(|entry| entry.unwrap().file_name())
    .collect();
  assert_eq!(
    names,
    [crate::profile::encryption::BASELINE_FILE_NAME],
    "ephemeral should hold only its baseline for a fresh encrypted profile"
  );

  // Simulate the browser writing a session
  std::fs::create_dir_all(ephemeral.join("Default")).unwrap();
  std::fs::write(ephemeral.join("Default/Cookies"), b"session-cookies").unwrap();
  std::fs::write(ephemeral.join("Default/places.sqlite"), b"places-data").unwrap();
  std::fs::write(ephemeral.join("prefs.js"), b"user_pref(\"x\", 1);").unwrap();

  // Browser exits — re-encrypt back to disk
  let n = complete_after_quit_blocking(&profile, false);
  assert!(
    matches!(n, Some(rewrote) if rewrote >= 3),
    "expected at least 3 files re-encrypted, got {n:?}"
  );

  // Encrypted dir should now have verifier + 3 user files
  let on_disk_count = std::fs::read_dir(&plain_dir).unwrap().count();
  assert!(
    on_disk_count >= 4,
    "encrypted dir should contain session data + verifier, got {on_disk_count} files"
  );

  // Simulate full app restart: drop key, drop ephemeral tracking, remove ephemeral
  fresh_test_state(&profile.id);

  // Unlock with same password
  rt.block_on(unlock_profile(profile.id.to_string(), "hunter2!".into()))
    .unwrap();

  // Re-launch — session must come back
  let ephemeral2 = prepare_for_launch(&profile).unwrap();
  assert_eq!(
    std::fs::read(ephemeral2.join("Default/Cookies")).unwrap(),
    b"session-cookies",
    "Cookies should survive across encrypt/quit/restart/unlock cycle"
  );
  assert_eq!(
    std::fs::read(ephemeral2.join("Default/places.sqlite")).unwrap(),
    b"places-data"
  );
  assert_eq!(
    std::fs::read(ephemeral2.join("prefs.js")).unwrap(),
    b"user_pref(\"x\", 1);"
  );

  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn integration_progressive_backoff_on_wrong_password() {
  let temp = TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(temp.path().to_path_buf());

  let profile = make_profile("test-backoff");
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let plain_dir = profile_full_path(&profile, &profiles_dir);
  populate_plaintext_dir(&plain_dir);
  ProfileManager::instance().save_profile(&profile).unwrap();
  fresh_test_state(&profile.id);
  clear_failed_attempts(&profile.id);

  let rt = tokio::runtime::Runtime::new().unwrap();
  rt.block_on(set_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
  ))
  .unwrap();
  drop_cached_key(&profile.id);

  // First 4 wrong attempts produce the INCORRECT_PASSWORD code
  for _ in 0..4 {
    let err = rt
      .block_on(unlock_profile(profile.id.to_string(), "wrong".into()))
      .unwrap_err();
    assert_eq!(parse_err_code(&err), Some("INCORRECT_PASSWORD"));
  }

  // 5th wrong attempt also returns the code, but the next one will be locked out
  let err = rt
    .block_on(unlock_profile(profile.id.to_string(), "wrong".into()))
    .unwrap_err();
  assert_eq!(parse_err_code(&err), Some("INCORRECT_PASSWORD"));

  // 6th attempt is rate-limited regardless of password correctness
  let err = rt
    .block_on(unlock_profile(profile.id.to_string(), "hunter2!".into()))
    .unwrap_err();
  assert_eq!(parse_err_code(&err), Some("LOCKED_OUT"));
  let secs = parse_err_param(&err, "seconds")
    .and_then(|v| v.parse::<u64>().ok())
    .unwrap();
  assert!(secs > 0 && secs <= 60, "expected 1m countdown, got {secs}s");

  // Bypass the timer by manually expiring last_failed_at past the lockout
  if let Ok(mut guard) = FAILED_ATTEMPTS.lock() {
    if let Some(record) = guard.get_mut(&profile.id) {
      record.last_failed_at_secs = now_epoch_secs().saturating_sub(120);
    }
  }
  if let Some(record) = FAILED_ATTEMPTS
    .lock()
    .ok()
    .and_then(|g| g.get(&profile.id).copied())
  {
    persist_record(&profile.id, &record);
  }

  // Correct password now succeeds, clearing the failure history
  rt.block_on(unlock_profile(profile.id.to_string(), "hunter2!".into()))
    .unwrap();
  let post = FAILED_ATTEMPTS
    .lock()
    .map(|g| g.contains_key(&profile.id))
    .unwrap_or(true);
  assert!(!post, "successful unlock should clear failure record");

  fresh_test_state(&profile.id);
  clear_failed_attempts(&profile.id);
}

#[test]
#[serial_test::serial]
fn integration_lockout_survives_restart() {
  let temp = TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(temp.path().to_path_buf());

  let profile = make_profile("test-restart");
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let plain_dir = profile_full_path(&profile, &profiles_dir);
  populate_plaintext_dir(&plain_dir);
  ProfileManager::instance().save_profile(&profile).unwrap();
  fresh_test_state(&profile.id);
  clear_failed_attempts(&profile.id);

  let rt = tokio::runtime::Runtime::new().unwrap();
  rt.block_on(set_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
  ))
  .unwrap();
  drop_cached_key(&profile.id);

  // 5 wrong attempts to trigger lockout
  for _ in 0..5 {
    let _ = rt.block_on(unlock_profile(profile.id.to_string(), "wrong".into()));
  }

  // Sidecar file should now exist
  let sidecar = lockout_sidecar_path(&profile.id);
  assert!(sidecar.exists(), "sidecar should be persisted to disk");

  // Simulate app restart by clearing the in-memory cache (but NOT the sidecar)
  if let Ok(mut g) = FAILED_ATTEMPTS.lock() {
    g.clear();
  }

  // Lockout should still apply because state was loaded from disk
  let err = rt
    .block_on(unlock_profile(profile.id.to_string(), "hunter2!".into()))
    .unwrap_err();
  assert_eq!(
    parse_err_code(&err),
    Some("LOCKED_OUT"),
    "expected lockout to persist across restart, got: {err}"
  );

  fresh_test_state(&profile.id);
  clear_failed_attempts(&profile.id);
}

#[tokio::test]
async fn attempt_lock_serializes_one_profile_without_blocking_others() {
  let a = uuid::Uuid::new_v4();
  let b = uuid::Uuid::new_v4();

  // One lock per profile is what turns check-lockout -> verify -> record
  // into a critical section instead of a check-then-act race.
  assert!(Arc::ptr_eq(&attempt_lock(&a), &attempt_lock(&a)));
  assert!(!Arc::ptr_eq(&attempt_lock(&a), &attempt_lock(&b)));

  let held = attempt_lock(&a);
  let guard = held.lock().await;
  assert!(
    attempt_lock(&a).try_lock().is_err(),
    "a concurrent attempt on the same profile must wait for the window"
  );
  assert!(
    attempt_lock(&b).try_lock().is_ok(),
    "a different profile must not be serialized behind it"
  );
  drop(guard);
  assert!(attempt_lock(&a).try_lock().is_ok());
}

#[test]
fn lockout_schedule_progression() {
  use std::time::Duration;
  assert_eq!(lockout_for_count(0), None);
  assert_eq!(lockout_for_count(4), None);
  assert_eq!(lockout_for_count(5), Some(Duration::from_secs(60)));
  assert_eq!(lockout_for_count(6), Some(Duration::from_secs(5 * 60)));
  assert_eq!(lockout_for_count(7), Some(Duration::from_secs(15 * 60)));
  assert_eq!(lockout_for_count(8), Some(Duration::from_secs(60 * 60)));
  assert_eq!(lockout_for_count(9), Some(Duration::from_secs(2 * 3600)));
  assert_eq!(lockout_for_count(10), Some(Duration::from_secs(4 * 3600)));
  assert_eq!(lockout_for_count(11), Some(Duration::from_secs(8 * 3600)));
  assert_eq!(lockout_for_count(12), Some(Duration::from_secs(24 * 3600)));
  assert_eq!(lockout_for_count(50), Some(Duration::from_secs(24 * 3600)));
}

#[test]
#[serial_test::serial]
fn integration_lock_drops_key() {
  let temp = TempDir::new().unwrap();
  let _guard = crate::app_dirs::set_test_data_dir(temp.path().to_path_buf());

  let profile = make_profile("test-lock");
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let plain_dir = profile_full_path(&profile, &profiles_dir);
  populate_plaintext_dir(&plain_dir);
  ProfileManager::instance().save_profile(&profile).unwrap();
  fresh_test_state(&profile.id);

  let rt = tokio::runtime::Runtime::new().unwrap();
  rt.block_on(set_profile_password(
    profile.id.to_string(),
    "hunter2!".into(),
  ))
  .unwrap();
  assert!(get_cached_key(&profile.id).is_some());
  assert!(!rt
    .block_on(is_profile_locked(profile.id.to_string()))
    .unwrap());

  rt.block_on(lock_profile(profile.id.to_string())).unwrap();
  assert!(get_cached_key(&profile.id).is_none());
  assert!(rt
    .block_on(is_profile_locked(profile.id.to_string()))
    .unwrap());

  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn a_quit_both_callers_seal_is_sealed_once_and_intact() {
  let _scratch = Scratch::new();
  let (profile, store) = protected_profile("test-race");

  for round in 0..5u8 {
    if !has_cached_key(&profile.id) {
      tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(unlock_profile(profile.id.to_string(), PASSWORD.into()))
        .unwrap();
    }
    let copy = prepare_for_launch(&profile).unwrap();
    // Enough files that one seal is still running when the other starts.
    std::fs::create_dir_all(copy.join("Default/IndexedDB")).unwrap();
    browser_writes(&copy.join("Default/Cookies"), [round + 1; 64]);
    for i in 0..300 {
      let file = copy.join("Default/IndexedDB").join(format!("{i:04}.ldb"));
      std::fs::write(&file, vec![round + 1; 16 * 1024]).unwrap();
    }

    let callers: Vec<_> = (0..2)
      .map(|_| {
        let store = store.clone();
        std::thread::spawn(move || seal_after_quit(profile.id, &store, false))
      })
      .collect();
    let results: Vec<Option<usize>> = callers.into_iter().map(|c| c.join().unwrap()).collect();

    assert_eq!(
      results.iter().filter(|r| r.is_some()).count(),
      1,
      "exactly one caller seals, got {results:?}"
    );
    assert!(crate::ephemeral_dirs::get_ephemeral_dir(&profile.id.to_string()).is_none());
    assert!(!has_cached_key(&profile.id));
    let sealed = sealed_files(&profile, &store);
    assert_eq!(sealed["Default/Cookies"], vec![round + 1; 64]);
    for i in 0..300 {
      assert_eq!(
        sealed[&format!("Default/IndexedDB/{i:04}.ldb")],
        vec![round + 1; 16 * 1024],
        "round {round} file {i}"
      );
    }
  }
  fresh_test_state(&profile.id);
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn a_seal_that_fails_keeps_the_copy_for_the_next_launch_to_seal() {
  use std::os::unix::fs::PermissionsExt;
  let _scratch = Scratch::new();
  let (profile, store) = protected_profile("test-failed-seal");

  let copy = prepare_for_launch(&profile).unwrap();
  browser_writes(&copy.join("Default/Cookies"), b"logged-in");

  std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o555)).unwrap();
  let sealed = complete_after_quit_blocking(&profile, false);
  std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o755)).unwrap();

  assert_eq!(sealed, None);
  assert!(has_cached_key(&profile.id), "the key stays to seal later");
  assert_eq!(
    std::fs::read(copy.join("Default/Cookies")).unwrap(),
    b"logged-in",
    "the only copy of the session must survive a failed seal"
  );
  assert_eq!(
    sealed_files(&profile, &store)["Default/Cookies"],
    b"sqlite-data"
  );

  let relaunched = prepare_for_launch(&profile).unwrap();
  assert_eq!(relaunched, copy);
  assert_eq!(
    std::fs::read(relaunched.join("Default/Cookies")).unwrap(),
    b"logged-in"
  );
  assert_eq!(
    sealed_files(&profile, &store)["Default/Cookies"],
    b"logged-in"
  );
  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn changes_from_a_quit_without_the_key_are_sealed_after_unlock() {
  let _scratch = Scratch::new();
  let (profile, store) = protected_profile("test-no-key");

  let copy = prepare_for_launch(&profile).unwrap();
  browser_writes(&copy.join("Default/Cookies"), b"logged-in");
  // The app restarted while the browser ran: the copy is there, the key is not.
  drop_cached_key(&profile.id);

  assert_eq!(complete_after_quit_blocking(&profile, false), None);
  assert_eq!(
    std::fs::read(copy.join("Default/Cookies")).unwrap(),
    b"logged-in"
  );

  tokio::runtime::Runtime::new()
    .unwrap()
    .block_on(unlock_profile(profile.id.to_string(), PASSWORD.into()))
    .unwrap();
  let relaunched = prepare_for_launch(&profile).unwrap();
  assert_eq!(
    std::fs::read(relaunched.join("Default/Cookies")).unwrap(),
    b"logged-in"
  );
  assert_eq!(
    sealed_files(&profile, &store)["Default/Cookies"],
    b"logged-in"
  );
  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn debris_of_an_interrupted_erase_is_not_launched_on() {
  let _scratch = Scratch::new();
  let (profile, _store) = protected_profile("test-debris");

  let copy = crate::ephemeral_dirs::create_ephemeral_dir(&profile.id.to_string()).unwrap();
  std::fs::create_dir_all(copy.join("Default")).unwrap();
  // A hot journal next to a freshly decrypted database would roll it back.
  std::fs::write(copy.join("Default/Cookies-journal"), [0u8; 512]).unwrap();
  std::fs::write(copy.join("Default/Cookies"), [0u8; 512]).unwrap();

  let launched = prepare_for_launch(&profile).unwrap();
  assert!(!launched.join("Default/Cookies-journal").exists());
  assert_eq!(
    std::fs::read(launched.join("Default/Cookies")).unwrap(),
    b"sqlite-data"
  );
  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn a_launch_stopped_before_its_quit_was_sealed_keeps_its_changes() {
  let _scratch = Scratch::new();
  let (profile, store) = protected_profile("test-relaunch");

  // A launch whose browser wrote and was then killed, with no quit sealed.
  let copy = prepare_for_launch(&profile).unwrap();
  browser_writes(&copy.join("os_crypt_key"), [7u8; 32]);
  browser_writes(&copy.join("Default/Cookies"), b"under-key-7");

  // The retry reuses the copy and must not take those files as sealed.
  let retry = prepare_for_launch(&profile).unwrap();
  assert_eq!(retry, copy);
  assert_eq!(complete_after_quit_blocking(&profile, false), Some(0));

  let sealed = sealed_files(&profile, &store);
  assert_eq!(sealed["os_crypt_key"], [7u8; 32]);
  assert_eq!(sealed["Default/Cookies"], b"under-key-7");
  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn lock_and_password_removal_seal_a_pending_copy_first() {
  let _scratch = Scratch::new();
  let rt = tokio::runtime::Runtime::new().unwrap();
  let (profile, store) = protected_profile("test-pending");

  let copy = prepare_for_launch(&profile).unwrap();
  browser_writes(&copy.join("Default/Cookies"), b"before-lock");
  rt.block_on(lock_profile(profile.id.to_string())).unwrap();
  assert!(!has_cached_key(&profile.id));
  assert!(crate::ephemeral_dirs::get_ephemeral_dir(&profile.id.to_string()).is_none());
  assert_eq!(
    sealed_files(&profile, &store)["Default/Cookies"],
    b"before-lock"
  );

  rt.block_on(unlock_profile(profile.id.to_string(), PASSWORD.into()))
    .unwrap();
  let copy = prepare_for_launch(&profile).unwrap();
  browser_writes(&copy.join("Default/Cookies"), b"before-removal");
  rt.block_on(remove_profile_password(
    profile.id.to_string(),
    PASSWORD.into(),
  ))
  .unwrap();
  assert!(crate::ephemeral_dirs::get_ephemeral_dir(&profile.id.to_string()).is_none());
  assert!(!copy.exists(), "no plaintext copy may outlive the password");
  assert_eq!(
    std::fs::read(store.join("Default/Cookies")).unwrap(),
    b"before-removal"
  );
  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn a_password_change_seals_a_pending_copy_with_the_old_key() {
  let _scratch = Scratch::new();
  let rt = tokio::runtime::Runtime::new().unwrap();
  let (profile, store) = protected_profile("test-change-pending");

  let copy = prepare_for_launch(&profile).unwrap();
  browser_writes(&copy.join("Default/Cookies"), b"pending");
  rt.block_on(change_profile_password(
    profile.id.to_string(),
    PASSWORD.into(),
    "another password".into(),
  ))
  .unwrap();

  let profile = load_profile(&profile.id).unwrap();
  let key = derive_profile_key(
    "another password",
    profile.encryption_salt.as_deref().unwrap(),
  )
  .unwrap();
  let out = TempDir::new().unwrap();
  crate::profile::encryption::decrypt_profile_dir(&key, &store, out.path()).unwrap();
  assert_eq!(
    std::fs::read(out.path().join("Default/Cookies")).unwrap(),
    b"pending"
  );
  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn the_sealed_copy_keeps_database_journals_and_drops_caches() {
  let _scratch = Scratch::new();
  let (profile, store) = protected_profile("test-journals");

  let copy = prepare_for_launch(&profile).unwrap();
  browser_writes(&copy.join("Default/Cookies-journal"), b"hot-journal");
  browser_writes(&copy.join("Default/History-wal"), b"wal-frames");
  browser_writes(&copy.join("Default/Secure Preferences"), b"{}");
  std::fs::create_dir_all(copy.join("Default/Code Cache/js")).unwrap();
  browser_writes(&copy.join("Default/Code Cache/js/index"), b"cache");
  complete_after_quit_blocking(&profile, false).unwrap();

  let sealed = sealed_files(&profile, &store);
  assert_eq!(sealed["Default/Cookies-journal"], b"hot-journal");
  assert_eq!(sealed["Default/History-wal"], b"wal-frames");
  assert_eq!(sealed["Default/Secure Preferences"], b"{}");
  assert!(!sealed.contains_key("Default/Code Cache/js/index"));
  assert!(!sealed.contains_key(crate::profile::encryption::BASELINE_FILE_NAME));
  fresh_test_state(&profile.id);
}

#[test]
#[serial_test::serial]
fn a_kept_copy_gives_way_to_a_newer_sealed_profile_from_sync() {
  let _scratch = Scratch::new();
  let (profile, store) = protected_profile("test-kept-sync");

  let copy = prepare_for_launch(&profile).unwrap();
  complete_after_quit_blocking(&profile, true).unwrap();
  assert!(copy.join("Default/Cookies").exists(), "the copy is kept");

  // Another device's session arrives through sync.
  let key = get_cached_key(&profile.id).unwrap();
  let elsewhere = TempDir::new().unwrap();
  crate::profile::encryption::decrypt_profile_dir(&key, &store, elsewhere.path()).unwrap();
  browser_writes(
    &elsewhere.path().join("Default/Cookies"),
    b"from-the-other-device",
  );
  crate::profile::encryption::encrypt_profile_dir(
    &key,
    elsewhere.path(),
    &store,
    crate::profile::encryption::SEALED_EXCLUDE_PATTERNS,
  )
  .unwrap();

  let relaunched = prepare_for_launch(&profile).unwrap();
  assert_eq!(
    std::fs::read(relaunched.join("Default/Cookies")).unwrap(),
    b"from-the-other-device"
  );
  complete_after_quit_blocking(&profile, false).unwrap();
  assert_eq!(
    sealed_files(&profile, &store)["Default/Cookies"],
    b"from-the-other-device"
  );
  fresh_test_state(&profile.id);
}

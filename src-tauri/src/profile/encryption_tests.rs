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

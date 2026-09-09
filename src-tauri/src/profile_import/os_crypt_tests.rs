use super::*;
use tempfile::TempDir;

#[test]
fn empty_password_key_matches_chromium_constant() {
  // Locks the constant against the value Chromium hardcodes in encryptor.cc.
  assert_eq!(derive_key(b"", POSIX_ITERATIONS), EMPTY_PASSWORD_KEY);
}

#[test]
fn peanuts_key_matches_known_vector() {
  // PBKDF2-HMAC-SHA1("peanuts", "saltysalt", 1, 16). Any drift here silently
  // breaks every Linux `--password-store=basic` import.
  assert_eq!(
    derive_key(POSIX_FALLBACK_PASSWORD, POSIX_ITERATIONS),
    [
      0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27,
      0x78
    ]
  );
}

#[test]
fn cbc_round_trip() {
  let key = CryptoKey::Aes128Cbc(derive_key(b"hunter2", MAC_ITERATIONS));
  let sealed = key.encrypt(b"session-token").expect("encrypt");
  assert_eq!(key.decrypt(&sealed).expect("decrypt"), b"session-token");
}

#[test]
fn cbc_round_trip_empty_plaintext() {
  let key = CryptoKey::Aes128Cbc(derive_key(b"hunter2", MAC_ITERATIONS));
  let sealed = key.encrypt(b"").expect("encrypt");
  // PKCS7 always emits a full padding block, so this must not be empty.
  assert_eq!(sealed.len(), 16);
  assert!(key.decrypt(&sealed).expect("decrypt").is_empty());
}

#[test]
fn gcm_round_trip_with_fresh_nonce_each_time() {
  let key = CryptoKey::Aes256Gcm([7u8; 32]);
  let a = key.encrypt(b"session-token").expect("encrypt");
  let b = key.encrypt(b"session-token").expect("encrypt");
  assert_ne!(a, b, "nonce must be random per call");
  assert_eq!(key.decrypt(&a).expect("decrypt"), b"session-token");
  assert_eq!(key.decrypt(&b).expect("decrypt"), b"session-token");
}

#[test]
fn gcm_rejects_tampered_ciphertext() {
  let key = CryptoKey::Aes256Gcm([7u8; 32]);
  let mut sealed = key.encrypt(b"session-token").expect("encrypt");
  let last = sealed.len() - 1;
  sealed[last] ^= 0xff;
  assert!(key.decrypt(&sealed).is_none());
}

#[test]
fn target_key_is_stable_across_calls() {
  let dir = TempDir::new().unwrap();
  let first = TargetKey::ensure(dir.path()).expect("mint");
  let sealed = first.encrypt(b"value").expect("encrypt");

  let second = TargetKey::ensure(dir.path()).expect("reuse");
  // Re-running import over the same directory must not orphan what the
  // previous run wrote.
  let key_file = std::fs::read(dir.path().join(KEY_FILE_NAME)).unwrap();
  let reloaded = TargetKey::from_file_contents(&key_file).expect("reload");
  assert_eq!(
    reloaded.encrypt(b"probe").map(|v| v[..3].to_vec()),
    second.encrypt(b"probe").map(|v| v[..3].to_vec())
  );

  let mut keyring = SourceKeyring::default();
  let contents = std::fs::read(dir.path().join(KEY_FILE_NAME)).unwrap();
  install_host_key(&mut keyring, &contents);
  match keyring.decrypt(&sealed) {
    Decrypted::Value(v) => assert_eq!(v, b"value"),
    _ => panic!("target key must round-trip through the source keyring"),
  }
}

#[test]
fn minted_key_matches_wayfern_file_format() {
  let dir = TempDir::new().unwrap();
  TargetKey::ensure(dir.path()).expect("mint");
  let contents = std::fs::read(dir.path().join(KEY_FILE_NAME)).unwrap();

  #[cfg(target_os = "windows")]
  assert_eq!(
    contents.len(),
    32,
    "DPAPIKeyProvider only adopts a 32-byte portable key"
  );

  #[cfg(not(target_os = "windows"))]
  {
    // The non-Windows key file is base64(16 random bytes) = 24 ASCII chars.
    assert_eq!(contents.len(), 24);
    let text = String::from_utf8(contents).expect("ascii");
    assert!(
      base64::engine::general_purpose::STANDARD
        .decode(&text)
        .map(|b| b.len())
        == Ok(16),
      "expected base64 of 16 bytes, got {text}"
    );
  }

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(dir.path().join(KEY_FILE_NAME))
      .unwrap()
      .permissions()
      .mode();
    assert_eq!(mode & 0o777, 0o600);
  }
}

#[test]
fn unknown_tag_is_treated_as_plaintext_not_as_loss() {
  let keyring = SourceKeyring::default();
  assert!(matches!(
    keyring.decrypt(b"plain cookie value"),
    Decrypted::NotEncrypted
  ));
}

#[test]
fn app_bound_records_are_flagged_unrecoverable() {
  let keyring = SourceKeyring::default();
  let mut sealed = b"v20".to_vec();
  sealed.extend_from_slice(&[0u8; 40]);
  assert!(matches!(keyring.decrypt(&sealed), Decrypted::Unrecoverable));
  assert!(
    keyring.saw_app_bound.get(),
    "v20 must be reported to the user, not silently dropped"
  );
}

#[test]
fn missing_key_for_known_tag_is_unrecoverable() {
  let keyring = SourceKeyring::default();
  let mut sealed = b"v10".to_vec();
  sealed.extend_from_slice(&[0u8; 32]);
  assert!(matches!(keyring.decrypt(&sealed), Decrypted::Unrecoverable));
}

#[test]
fn empty_password_fallback_recovers_the_record() {
  // A record sealed with the empty-password key must still open when the
  // keyring holds a different primary key, mirroring Chromium.
  let sealed_body = CryptoKey::Aes128Cbc(EMPTY_PASSWORD_KEY)
    .encrypt(b"legacy")
    .unwrap();
  let mut stored = b"v10".to_vec();
  stored.extend_from_slice(&sealed_body);

  let keyring = SourceKeyring {
    v10: Some(CryptoKey::Aes128Cbc(derive_key(b"a different key", 1003))),
    ..Default::default()
  };
  match keyring.decrypt(&stored) {
    Decrypted::Value(v) => assert_eq!(v, b"legacy"),
    _ => panic!("empty-password fallback must be attempted"),
  }
}

/// Load the host-format key into a keyring under the host tag, for tests
/// that need to verify what we wrote is what Wayfern will read.
fn install_host_key(keyring: &mut SourceKeyring, contents: &[u8]) {
  #[cfg(target_os = "windows")]
  {
    let bytes: [u8; 32] = contents.try_into().unwrap();
    keyring.v10 = Some(CryptoKey::Aes256Gcm(bytes));
  }
  #[cfg(target_os = "macos")]
  {
    keyring.v10 = Some(CryptoKey::Aes128Cbc(derive_key(contents, MAC_ITERATIONS)));
  }
  #[cfg(target_os = "linux")]
  {
    keyring.v11 = Some(CryptoKey::Aes128Cbc(derive_key(contents, POSIX_ITERATIONS)));
  }
}

use super::*;
use crate::profile_import::os_crypt::{derive_key, CryptoKey};
use tempfile::TempDir;

fn source_keyring_with(password: &[u8]) -> SourceKeyring {
  // Match the host's CBC iteration count so tests exercise the real path.
  #[cfg(target_os = "linux")]
  let key = CryptoKey::Aes128Cbc(derive_key(
    password,
    super::super::os_crypt::POSIX_ITERATIONS,
  ));
  #[cfg(not(target_os = "linux"))]
  let key = CryptoKey::Aes128Cbc(derive_key(password, super::super::os_crypt::MAC_ITERATIONS));

  #[cfg(target_os = "linux")]
  return SourceKeyring {
    v11: Some(key),
    ..Default::default()
  };
  #[cfg(not(target_os = "linux"))]
  SourceKeyring {
    v10: Some(key),
    ..Default::default()
  }
}

fn seal_as_source(keyring: &SourceKeyring, plaintext: &[u8]) -> Vec<u8> {
  let (tag, key) = if let Some(k) = keyring.v10.as_ref() {
    (b"v10", k)
  } else {
    (b"v11", keyring.v11.as_ref().unwrap())
  };
  let mut out = tag.to_vec();
  out.extend_from_slice(&key.encrypt(plaintext).unwrap());
  out
}

fn make_cookie_db(path: &Path, version: i64) -> Connection {
  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  let conn = Connection::open(path).unwrap();
  conn
    .execute_batch(
      "CREATE TABLE cookies(
         creation_utc INTEGER NOT NULL,
         host_key TEXT NOT NULL,
         top_frame_site_key TEXT NOT NULL DEFAULT '',
         name TEXT NOT NULL,
         value TEXT NOT NULL DEFAULT '',
         encrypted_value BLOB NOT NULL DEFAULT '',
         path TEXT NOT NULL DEFAULT '/'
       );
       CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);",
    )
    .unwrap();
  conn
    .execute(
      "INSERT INTO meta VALUES('version', ?1)",
      [version.to_string()],
    )
    .unwrap();
  conn
    .execute(
      "INSERT INTO meta VALUES('last_compatible_version', ?1)",
      [version.to_string()],
    )
    .unwrap();
  conn
}

#[test]
fn v24_cookie_is_reframed_for_the_target_key() {
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  let cookie_path = layout::host_cookie_path(&default_dir);

  let source = source_keyring_with(b"source-password");
  let mut framed = Sha256::digest(b"example.com").to_vec();
  framed.extend_from_slice(b"tasty");
  let sealed = seal_as_source(&source, &framed);

  let conn = make_cookie_db(&cookie_path, 24);
  conn
    .execute(
      "INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path)
       VALUES(0, 'example.com', '', 'sid', '', ?1, '/')",
      rusqlite::params![sealed],
    )
    .unwrap();
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let mut report = ProfileImportReport::default();
  reencrypt_cookies(&default_dir, &source, &target, &mut report);

  assert_eq!(report.cookies_migrated, 1);
  assert_eq!(report.cookies_unrecoverable, 0);

  // Read it back exactly the way Wayfern will.
  let conn = Connection::open(&cookie_path).unwrap();
  let (value, encrypted): (String, Vec<u8>) = conn
    .query_row("SELECT value, encrypted_value FROM cookies", [], |r| {
      Ok((r.get(0)?, r.get(1)?))
    })
    .unwrap();
  assert!(
    value.is_empty(),
    "a row with both value and encrypted_value set is dropped at load"
  );

  let target_keyring = target_as_keyring(dir.path());
  let Decrypted::Value(plain) = target_keyring.decrypt(&encrypted) else {
    panic!("target must be able to open what it sealed");
  };
  assert_eq!(&plain[..32], &Sha256::digest(b"example.com")[..]);
  assert_eq!(&plain[32..], b"tasty");
}

#[test]
fn cookie_sealed_as_sqlite_text_is_still_recovered() {
  // Chromium's own v23->v24 migration binds `encrypted_value` with
  // BindString, so an established profile's cookies carry storage class TEXT
  // in a column declared BLOB. Reading them as a strict blob returns empty,
  // which used to blank every cookie and report it as migrated.
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  let cookie_path = layout::host_cookie_path(&default_dir);

  let source = source_keyring_with(b"source-password");
  let mut framed = Sha256::digest(b"example.com").to_vec();
  framed.extend_from_slice(b"tasty");
  let sealed = seal_as_source(&source, &framed);

  let conn = make_cookie_db(&cookie_path, 24);
  conn
    .execute(
      "INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path)
       VALUES(0, 'example.com', '', 'sid', '', CAST(?1 AS TEXT), '/')",
      rusqlite::params![sealed],
    )
    .unwrap();
  let stored_type: String = conn
    .query_row("SELECT typeof(encrypted_value) FROM cookies", [], |r| {
      r.get(0)
    })
    .unwrap();
  assert_eq!(
    stored_type, "text",
    "fixture must reproduce Chromium's binding"
  );
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let mut report = ProfileImportReport::default();
  reencrypt_cookies(&default_dir, &source, &target, &mut report);

  assert_eq!(report.cookies_migrated, 1);
  let conn = Connection::open(&cookie_path).unwrap();
  let encrypted: Vec<u8> = conn
    .query_row("SELECT encrypted_value FROM cookies", [], |r| r.get(0))
    .unwrap();
  let Decrypted::Value(plain) = target_as_keyring(dir.path()).decrypt(&encrypted) else {
    panic!("expected a readable cookie");
  };
  assert_eq!(&plain[32..], b"tasty", "the cookie value must survive");
}

#[test]
fn password_note_sealed_as_sqlite_text_is_still_recovered() {
  // `password_notes.value` is written with BindString on every platform, so
  // this is not an edge case — it is how the column always looks.
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  std::fs::create_dir_all(&default_dir).unwrap();

  let source = source_keyring_with(b"source-password");
  let sealed = seal_as_source(&source, b"a private note");

  let conn = Connection::open(default_dir.join("Login Data")).unwrap();
  conn
    .execute_batch(
      "CREATE TABLE logins(password_value BLOB);
       CREATE TABLE password_notes(id INTEGER PRIMARY KEY, value BLOB);",
    )
    .unwrap();
  conn
    .execute(
      "INSERT INTO password_notes(value) VALUES(CAST(?1 AS TEXT))",
      rusqlite::params![sealed],
    )
    .unwrap();
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let mut report = ProfileImportReport::default();
  reencrypt_profile(&default_dir, &source, &target, &mut report);

  assert_eq!(report.logins_migrated, 1);
  let conn = Connection::open(default_dir.join("Login Data")).unwrap();
  let stored: Vec<u8> = conn
    .query_row("SELECT value FROM password_notes", [], |r| r.get(0))
    .unwrap();
  let Decrypted::Value(plain) = target_as_keyring(dir.path()).decrypt(&stored) else {
    panic!("note must be readable with the target key");
  };
  assert_eq!(plain, b"a private note");
}

#[test]
fn windows_extension_paths_are_recognised_as_absolute_on_every_host() {
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("Secure Preferences");
  std::fs::write(
    &path,
    serde_json::json!({
      "extensions": { "settings": {
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa": { "path": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/1.0_0" },
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb": { "path": "C:\\Program Files\\Google\\Chrome\\Application\\151.0.0\\resources\\pdf" },
        "cccccccccccccccccccccccccccccccc": { "path": "//host/share/ext" }
      }}
    })
    .to_string(),
  )
  .unwrap();

  let mut report = ProfileImportReport::default();
  sanitize_secure_preferences(&path, &mut report);

  let value: serde_json::Value =
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  let settings = value["extensions"]["settings"].as_object().unwrap();
  assert_eq!(
    settings.len(),
    1,
    "a Windows-syntax path is still absolute when imported onto macOS"
  );
  assert!(settings.contains_key("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
  assert_eq!(report.extensions_migrated, 1);
}

#[test]
fn plaintext_cookie_is_sealed_and_value_cleared() {
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  let cookie_path = layout::host_cookie_path(&default_dir);

  let conn = make_cookie_db(&cookie_path, 24);
  conn
    .execute(
      "INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path)
       VALUES(0, 'example.com', '', 'sid', 'plain', X'', '/')",
      [],
    )
    .unwrap();
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let source = source_keyring_with(b"unused");
  let mut report = ProfileImportReport::default();
  reencrypt_cookies(&default_dir, &source, &target, &mut report);

  assert_eq!(report.cookies_migrated, 1);
  let conn = Connection::open(&cookie_path).unwrap();
  let (value, encrypted): (String, Vec<u8>) = conn
    .query_row("SELECT value, encrypted_value FROM cookies", [], |r| {
      Ok((r.get(0)?, r.get(1)?))
    })
    .unwrap();
  assert!(value.is_empty());
  let Decrypted::Value(plain) = target_as_keyring(dir.path()).decrypt(&encrypted) else {
    panic!("expected a readable cookie");
  };
  assert_eq!(&plain[32..], b"plain");
}

#[test]
fn v23_cookie_has_no_prefix_to_strip_and_is_upgraded_to_v24() {
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  let cookie_path = layout::host_cookie_path(&default_dir);

  let source = source_keyring_with(b"source-password");
  // v23 stores the bare value, with no SHA256(host) prefix.
  let sealed = seal_as_source(&source, b"tasty");

  let conn = make_cookie_db(&cookie_path, 23);
  conn
    .execute(
      "INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path)
       VALUES(0, 'example.com', '', 'sid', '', ?1, '/')",
      rusqlite::params![sealed],
    )
    .unwrap();
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let mut report = ProfileImportReport::default();
  reencrypt_cookies(&default_dir, &source, &target, &mut report);

  assert_eq!(report.cookies_migrated, 1);
  let conn = Connection::open(&cookie_path).unwrap();
  let version: String = conn
    .query_row("SELECT value FROM meta WHERE key='version'", [], |r| {
      r.get(0)
    })
    .unwrap();
  assert_eq!(
    version, "24",
    "we wrote v24 framing, so the store must declare v24 or Chromium re-prefixes it"
  );

  let encrypted: Vec<u8> = conn
    .query_row("SELECT encrypted_value FROM cookies", [], |r| r.get(0))
    .unwrap();
  let Decrypted::Value(plain) = target_as_keyring(dir.path()).decrypt(&encrypted) else {
    panic!("expected a readable cookie");
  };
  assert_eq!(&plain[32..], b"tasty");
}

#[test]
fn unrecoverable_cookie_row_is_deleted_and_counted() {
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  let cookie_path = layout::host_cookie_path(&default_dir);

  let conn = make_cookie_db(&cookie_path, 24);
  let mut app_bound = b"v20".to_vec();
  app_bound.extend_from_slice(&[0u8; 48]);
  conn
    .execute(
      "INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path)
       VALUES(0, 'example.com', '', 'sid', '', ?1, '/')",
      rusqlite::params![app_bound],
    )
    .unwrap();
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let source = source_keyring_with(b"source-password");
  let mut report = ProfileImportReport::default();
  reencrypt_cookies(&default_dir, &source, &target, &mut report);

  assert_eq!(report.cookies_unrecoverable, 1);
  assert_eq!(report.cookies_migrated, 0);
  let conn = Connection::open(&cookie_path).unwrap();
  let remaining: i64 = conn
    .query_row("SELECT count(*) FROM cookies", [], |r| r.get(0))
    .unwrap();
  assert_eq!(remaining, 0, "a row no key can open is dead weight");
}

#[test]
fn cookie_store_older_than_chromium_migrates_is_removed_with_a_warning() {
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  let cookie_path = layout::host_cookie_path(&default_dir);
  make_cookie_db(&cookie_path, 22);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let source = source_keyring_with(b"x");
  let mut report = ProfileImportReport::default();
  reencrypt_cookies(&default_dir, &source, &target, &mut report);

  assert!(report
    .warnings
    .contains(&warning::STORE_TOO_OLD.to_string()));
  assert!(!cookie_path.exists());
}

#[test]
fn passwords_are_reencrypted() {
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  std::fs::create_dir_all(&default_dir).unwrap();

  let source = source_keyring_with(b"source-password");
  let sealed = seal_as_source(&source, b"hunter2");

  let conn = Connection::open(default_dir.join("Login Data")).unwrap();
  conn
    .execute_batch("CREATE TABLE logins(origin_url VARCHAR, password_value BLOB);")
    .unwrap();
  conn
    .execute(
      "INSERT INTO logins VALUES('https://example.com', ?1)",
      rusqlite::params![sealed],
    )
    .unwrap();
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let mut report = ProfileImportReport::default();
  reencrypt_profile(&default_dir, &source, &target, &mut report);

  assert_eq!(report.logins_migrated, 1);
  let conn = Connection::open(default_dir.join("Login Data")).unwrap();
  let stored: Vec<u8> = conn
    .query_row("SELECT password_value FROM logins", [], |r| r.get(0))
    .unwrap();
  let Decrypted::Value(plain) = target_as_keyring(dir.path()).decrypt(&stored) else {
    panic!("password must be readable with the target key");
  };
  assert_eq!(plain, b"hunter2");
}

#[test]
fn missing_optional_tables_are_not_an_error() {
  // `password_notes` and most payment tables only exist on some schemas.
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  std::fs::create_dir_all(&default_dir).unwrap();
  let conn = Connection::open(default_dir.join("Login Data")).unwrap();
  conn
    .execute_batch("CREATE TABLE logins(password_value BLOB);")
    .unwrap();
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let source = source_keyring_with(b"x");
  let mut report = ProfileImportReport::default();
  reencrypt_profile(&default_dir, &source, &target, &mut report);
  assert_eq!(report.logins_migrated, 0);
}

#[test]
fn secure_preferences_keeps_extensions_and_drops_protection() {
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("Secure Preferences");
  std::fs::write(
    &path,
    serde_json::json!({
      "protection": { "macs": { "extensions": { "settings": "deadbeef" } }, "super_mac": "x" },
      "extensions": { "settings": {
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa": { "path": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/1.0_0" },
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb": { "path": "/Applications/Chromium.app/Contents/Resources/x" }
      }}
    })
    .to_string(),
  )
  .unwrap();

  let mut report = ProfileImportReport::default();
  sanitize_secure_preferences(&path, &mut report);

  let value: serde_json::Value =
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  assert!(value.get("protection").is_none());
  let settings = value["extensions"]["settings"].as_object().unwrap();
  assert!(
    settings.contains_key("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    "a relative path is the user's real extension and must survive"
  );
  assert!(
    !settings.contains_key("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    "an absolute path points into the source browser's bundle"
  );
  assert_eq!(report.extensions_migrated, 1);
  assert!(report
    .warnings
    .contains(&warning::SECURE_PREFERENCES_RESET.to_string()));
}

#[test]
fn preferences_lose_machine_paths_and_crash_state() {
  let dir = TempDir::new().unwrap();
  let path = dir.path().join("Preferences");
  std::fs::write(
    &path,
    serde_json::json!({
      "download": { "default_directory": "/Users/someone-else/Downloads" },
      "profile": { "exit_type": "Crashed", "exited_cleanly": false, "name": "Person 1" },
      "intl": { "accept_languages": "de,de-DE" }
    })
    .to_string(),
  )
  .unwrap();

  let mut report = ProfileImportReport::default();
  sanitize_preferences(&path, &mut report);

  let value: serde_json::Value =
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  assert!(value["download"].get("default_directory").is_none());
  assert_eq!(value["profile"]["exit_type"], "Normal");
  assert_eq!(value["profile"]["exited_cleanly"], true);
  assert!(value["intl"].get("accept_languages").is_none());
  assert_eq!(
    value["profile"]["name"], "Person 1",
    "unrelated preferences must be preserved"
  );
}

#[test]
fn plaintext_cookies_still_migrate_when_no_source_key_is_available() {
  // A declined Keychain prompt loses the encrypted rows, but a profile whose
  // cookies were stored in plaintext has nothing to lose. Reporting zero for
  // it would be the same silent-empty-import failure this work exists to fix.
  let dir = TempDir::new().unwrap();
  let default_dir = dir.path().join("Default");
  let cookie_path = layout::host_cookie_path(&default_dir);

  let conn = make_cookie_db(&cookie_path, 24);
  conn
    .execute(
      "INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path)
       VALUES(0, 'example.com', '', 'sid', 'plain', X'', '/')",
      [],
    )
    .unwrap();
  let mut sealed_elsewhere = b"v10".to_vec();
  sealed_elsewhere.extend_from_slice(&[9u8; 32]);
  conn
    .execute(
      "INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path)
       VALUES(1, 'other.example', '', 'sid', '', ?1, '/')",
      rusqlite::params![sealed_elsewhere],
    )
    .unwrap();
  drop(conn);

  let target = TargetKey::ensure(dir.path()).unwrap();
  let empty = SourceKeyring::default();
  let mut report = ProfileImportReport::default();
  finalize_profile(&default_dir, &empty, &target, &mut report);

  assert_eq!(
    report.cookies_migrated, 1,
    "the plaintext row is recoverable"
  );
  assert_eq!(report.cookies_unrecoverable, 1, "the sealed row is not");
  assert!(report
    .warnings
    .contains(&warning::SECRETS_NOT_MIGRATED.to_string()));
}

#[test]
fn bookmarks_are_counted_recursively() {
  let roots = serde_json::json!({
    "bookmark_bar": { "type": "folder", "children": [
      { "type": "url", "url": "https://a.example" },
      { "type": "folder", "children": [{ "type": "url", "url": "https://b.example" }] }
    ]},
    "other": { "type": "folder", "children": [] }
  });
  assert_eq!(count_bookmarks(Some(&roots)), 2);
}

/// Load the freshly minted `os_crypt_key` back as a keyring, so tests assert
/// against what Wayfern will actually do rather than against our own writer.
fn target_as_keyring(user_data_dir: &Path) -> SourceKeyring {
  let contents =
    std::fs::read(user_data_dir.join(crate::profile_import::os_crypt::KEY_FILE_NAME)).unwrap();
  #[cfg(target_os = "windows")]
  {
    let bytes: [u8; 32] = contents.as_slice().try_into().unwrap();
    SourceKeyring {
      v10: Some(CryptoKey::Aes256Gcm(bytes)),
      ..Default::default()
    }
  }
  #[cfg(target_os = "macos")]
  {
    SourceKeyring {
      v10: Some(CryptoKey::Aes128Cbc(derive_key(
        &contents,
        super::super::os_crypt::MAC_ITERATIONS,
      ))),
      ..Default::default()
    }
  }
  #[cfg(target_os = "linux")]
  {
    SourceKeyring {
      v11: Some(CryptoKey::Aes128Cbc(derive_key(
        &contents,
        super::super::os_crypt::POSIX_ITERATIONS,
      ))),
      ..Default::default()
    }
  }
}

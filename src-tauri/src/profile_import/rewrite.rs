//! Re-sealing the copied profile with Wayfern's key, and stripping the state
//! that is bound to the machine it came from.
//!
//! Every encrypted store in a Chromium profile goes through one
//! `os_crypt_async::Encryptor`, so "migrate the secrets" is really one loop
//! over a handful of `(table, column)` pairs plus the cookie store, which is
//! the only one with extra framing.

use super::copy;
use super::layout;
use super::os_crypt::{Decrypted, SourceKeyring, TargetKey};
use super::report::{warning, ProfileImportReport};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Cookie DB schema versions this code understands.
///
/// 24 is current (`kCurrentVersionNumber`); it frames the encrypted plaintext
/// as `SHA256(host_key) || value`. 23 is the last version Chromium will still
/// migrate forward, and it has no prefix. Anything older is deleted by
/// Chromium on open, so carrying it over would be a silent loss.
const COOKIE_VERSION_CURRENT: i64 = 24;
const COOKIE_VERSION_MIN: i64 = 23;

/// `(table, column)` pairs holding a bare os_crypt value — no extra framing.
/// Sourced from the Chromium 151 tree rather than from memory:
/// `login_database.cc`, `password_notes_table.cc`, `token_service_table.cc`,
/// `payments_autofill_table.cc`.
const LOGIN_COLUMNS: &[(&str, &str)] = &[("logins", "password_value"), ("password_notes", "value")];

const WEB_DATA_COLUMNS: &[(&str, &str)] = &[
  ("credit_cards", "card_number_encrypted"),
  ("local_ibans", "value_encrypted"),
  ("local_stored_cvc", "value_encrypted"),
  ("server_stored_cvc", "value_encrypted"),
  ("generic_payment_instruments", "serialized_value_encrypted"),
  ("token_service", "encrypted_token"),
];

#[derive(Default)]
struct Counts {
  migrated: usize,
  unrecoverable: usize,
}

fn table_exists(conn: &Connection, table: &str) -> bool {
  conn
    .query_row(
      "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
      [table],
      |_| Ok(()),
    )
    .is_ok()
}

/// Read a stored ciphertext, accepting either SQLite storage class.
///
/// The columns are declared BLOB, but Chromium does not always bind them as
/// one: the cookie v23->v24 migration writes `encrypted_value` with
/// `sqlite3_bind_text` (`sqlite_persistent_cookie_store.cc` `BindString`), and
/// `password_notes.value` is written with `BindString` on every platform. BLOB
/// columns have no affinity, so those values keep storage class TEXT forever.
/// `row.get::<_, Vec<u8>>` demands a Blob and errors on Text — which would read
/// back as empty and silently blank the secret. Chromium itself reads these
/// with `ColumnString`/`ColumnBlobAsString`, which accept both; so do we.
fn column_bytes(row: &rusqlite::Row<'_>, index: usize) -> Vec<u8> {
  row
    .get_ref(index)
    .ok()
    .and_then(|value| value.as_bytes().ok())
    .map(<[u8]>::to_vec)
    .unwrap_or_default()
}

fn open_rw(path: &Path) -> Option<Connection> {
  if !path.is_file() {
    return None;
  }
  match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE) {
    Ok(conn) => Some(conn),
    Err(e) => {
      log::warn!("Could not open {} for re-encryption: {e}", path.display());
      None
    }
  }
}

/// Re-seal one plain `(table, column)` pair.
fn reencrypt_column(
  conn: &Connection,
  table: &str,
  column: &str,
  source: &SourceKeyring,
  target: &TargetKey,
) -> Counts {
  let mut counts = Counts::default();
  if !table_exists(conn, table) {
    return counts;
  }

  // Table and column names are compile-time constants from the lists above,
  // never user input, so interpolating them is safe.
  let rows: Vec<(i64, Vec<u8>)> = {
    let Ok(mut stmt) = conn.prepare(&format!(
      "SELECT rowid, {column} FROM {table} WHERE {column} IS NOT NULL"
    )) else {
      return counts;
    };
    let Ok(mapped) = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, column_bytes(row, 1))))
    else {
      return counts;
    };
    mapped.flatten().collect()
  };

  for (rowid, stored) in rows {
    if stored.is_empty() {
      continue;
    }
    let plaintext = match source.decrypt(&stored) {
      Decrypted::Value(v) => v,
      // Already plaintext: seal it so the store is uniform.
      Decrypted::NotEncrypted => stored.clone(),
      Decrypted::Unrecoverable => {
        counts.unrecoverable += 1;
        // Blank rather than leave a blob no key can open. Chromium logs a
        // decrypt failure for every such row on every load, and the user gets
        // a password entry that can never be revealed.
        let _ = conn.execute(
          &format!("UPDATE {table} SET {column} = X'' WHERE rowid = ?1"),
          [rowid],
        );
        continue;
      }
    };

    let Some(sealed) = target.encrypt(&plaintext) else {
      counts.unrecoverable += 1;
      continue;
    };
    if conn
      .execute(
        &format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2"),
        rusqlite::params![sealed, rowid],
      )
      .is_ok()
    {
      counts.migrated += 1;
    } else {
      counts.unrecoverable += 1;
    }
  }

  counts
}

/// Re-seal the cookie store.
///
/// Cookies are the one store with extra framing: since schema v24 the
/// encrypted plaintext is `SHA256(host_key) || value`, and Chromium drops any
/// row whose prefix does not match (`kHashFailed`) as well as any row where
/// both `value` and `encrypted_value` are non-empty.
fn reencrypt_cookies(
  default_dir: &Path,
  source: &SourceKeyring,
  target: &TargetKey,
  report: &mut ProfileImportReport,
) {
  let path = layout::host_cookie_path(default_dir);
  let Some(conn) = open_rw(&path) else {
    return;
  };
  if !table_exists(&conn, "cookies") {
    return;
  }

  let version: i64 = conn
    .query_row("SELECT value FROM meta WHERE key='version'", [], |r| {
      r.get::<_, String>(0)
    })
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(COOKIE_VERSION_CURRENT);

  if version < COOKIE_VERSION_MIN {
    // Chromium deletes and recreates a store this old on first launch, so
    // copying it would look like a successful import of nothing.
    drop(conn);
    let _ = std::fs::remove_file(&path);
    report.warn(warning::STORE_TOO_OLD);
    return;
  }
  if version > COOKIE_VERSION_CURRENT {
    drop(conn);
    let _ = std::fs::remove_file(&path);
    report.warn(warning::STORE_TOO_NEW);
    return;
  }

  // Only a v24 store carries the hash prefix; a v23 one does not.
  let source_has_prefix = version >= COOKIE_VERSION_CURRENT;

  let rows: Vec<(i64, String, String, Vec<u8>)> = {
    let Ok(mut stmt) = conn.prepare("SELECT rowid, host_key, value, encrypted_value FROM cookies")
    else {
      return;
    };
    let Ok(mapped) = stmt.query_map([], |row| {
      Ok((
        row.get::<_, i64>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2).unwrap_or_default(),
        column_bytes(row, 3),
      ))
    }) else {
      return;
    };
    mapped.flatten().collect()
  };

  let mut doomed: Vec<i64> = Vec::new();

  for (rowid, host_key, plain_value, stored) in rows {
    let value = if stored.is_empty() {
      // Written plaintext, either by an old Chromium or by our own cookie
      // import. Seal it so the store ends up uniform.
      plain_value.into_bytes()
    } else {
      match source.decrypt(&stored) {
        Decrypted::Value(mut decrypted) => {
          if source_has_prefix {
            let expected: [u8; 32] = Sha256::digest(host_key.as_bytes()).into();
            if decrypted.len() >= 32 && decrypted[..32] == expected {
              decrypted.drain(..32);
            } else if decrypted.len() >= 32 {
              // The prefix is mandatory at v24 and does not match. The row is
              // corrupt or belongs to another host; Chromium would drop it.
              doomed.push(rowid);
              report.cookies_unrecoverable += 1;
              continue;
            }
          }
          decrypted
        }
        Decrypted::NotEncrypted => stored.clone(),
        Decrypted::Unrecoverable => {
          doomed.push(rowid);
          report.cookies_unrecoverable += 1;
          continue;
        }
      }
    };

    // v24 framing, unconditionally: we normalise the store to the current
    // version below, so every row must carry the prefix.
    let mut framed = Sha256::digest(host_key.as_bytes()).to_vec();
    framed.extend_from_slice(&value);

    let Some(sealed) = target.encrypt(&framed) else {
      doomed.push(rowid);
      report.cookies_unrecoverable += 1;
      continue;
    };

    // `value` must be cleared: a row with both set is dropped at load.
    if conn
      .execute(
        "UPDATE cookies SET encrypted_value = ?1, value = '' WHERE rowid = ?2",
        rusqlite::params![sealed, rowid],
      )
      .is_ok()
    {
      report.cookies_migrated += 1;
    } else {
      doomed.push(rowid);
      report.cookies_unrecoverable += 1;
    }
  }

  for rowid in doomed {
    let _ = conn.execute("DELETE FROM cookies WHERE rowid = ?1", [rowid]);
  }

  // Every row now uses v24 framing, so declare the store current and spare
  // Chromium a migration that would double-prefix what we just wrote.
  let _ = conn.execute(
    "UPDATE meta SET value = ?1 WHERE key = 'version'",
    [COOKIE_VERSION_CURRENT.to_string()],
  );
  let _ = conn.execute(
    "UPDATE meta SET value = ?1 WHERE key = 'last_compatible_version'",
    [COOKIE_VERSION_CURRENT.to_string()],
  );
}

/// Re-seal every store, and count what came across.
pub fn reencrypt_profile(
  default_dir: &Path,
  source: &SourceKeyring,
  target: &TargetKey,
  report: &mut ProfileImportReport,
) {
  reencrypt_cookies(default_dir, source, target, report);

  if let Some(conn) = open_rw(&default_dir.join("Login Data")) {
    for (table, column) in LOGIN_COLUMNS {
      let counts = reencrypt_column(&conn, table, column, source, target);
      report.passwords_migrated += counts.migrated;
      report.passwords_unrecoverable += counts.unrecoverable;
    }
  }

  if let Some(conn) = open_rw(&default_dir.join("Web Data")) {
    for (table, column) in WEB_DATA_COLUMNS {
      let counts = reencrypt_column(&conn, table, column, source, target);
      report.payment_methods_migrated += counts.migrated;
      report.payment_methods_unrecoverable += counts.unrecoverable;
    }
  }

  if source.saw_app_bound.get() {
    report.warn(warning::APP_BOUND_ENCRYPTED);
  }
}

/// Strip the `protection` block from `Secure Preferences`.
///
/// The MACs in it are keyed by a seed that only Google-branded builds compile
/// in, plus a machine id, so they can never validate under Wayfern and every
/// `ENFORCE_ON_LOAD` pref resets on first launch regardless. Deleting the
/// whole file would be worse: `extensions.settings` lives here and is
/// registered at `NO_ENFORCEMENT`, so it survives an invalid MAC — that is the
/// only reason imported extensions appear at all.
fn sanitize_secure_preferences(path: &Path, report: &mut ProfileImportReport) {
  let Ok(raw) = std::fs::read_to_string(path) else {
    return;
  };
  let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
    return;
  };
  let Some(object) = value.as_object_mut() else {
    return;
  };

  if object.remove("protection").is_some() {
    report.warn(warning::SECURE_PREFERENCES_RESET);
  }
  strip_absolute_extension_paths(object, report);

  if let Ok(serialized) = serde_json::to_string(&value) {
    let _ = std::fs::write(path, serialized);
  }
}

/// Absolute in the *source's* path syntax, not merely the host's.
///
/// `Path::is_absolute` answers for the platform it is compiled on, so a Windows
/// path in a profile imported onto macOS reads as relative and the dead entry
/// survives. Profiles move between platforms routinely (that is what the ZIP
/// import is for), so mirror `base::IsPathAbsolute` instead: a POSIX leading
/// slash, a UNC double separator, or a drive letter.
fn is_absolute_in_any_syntax(path: &str) -> bool {
  let bytes = path.as_bytes();
  match bytes {
    [b'/', ..] => true,
    [a, b, ..] if matches!(a, b'\\' | b'/') && matches!(b, b'\\' | b'/') => true,
    [drive, b':', sep, ..] if drive.is_ascii_alphabetic() && matches!(sep, b'\\' | b'/') => true,
    _ => false,
  }
}

/// Drop extension entries whose `path` is absolute.
///
/// A relative path (`<id>/<version>_0`) is a real user extension living inside
/// the profile, and it came across with the copy. An absolute one points into
/// the source browser's app bundle at a pinned build — a component extension
/// that Wayfern registers for itself, and a dead path if left behind.
fn strip_absolute_extension_paths(
  root: &mut serde_json::Map<String, serde_json::Value>,
  report: &mut ProfileImportReport,
) {
  let Some(settings) = root
    .get_mut("extensions")
    .and_then(|e| e.get_mut("settings"))
    .and_then(|s| s.as_object_mut())
  else {
    return;
  };

  let doomed: Vec<String> = settings
    .iter()
    .filter(|(_, entry)| {
      entry
        .get("path")
        .and_then(|p| p.as_str())
        .is_some_and(is_absolute_in_any_syntax)
    })
    .map(|(id, _)| id.clone())
    .collect();

  if !doomed.is_empty() {
    report.warn(warning::EXTENSIONS_PARTIAL);
  }
  for id in doomed {
    settings.remove(&id);
  }
  report.extensions_migrated = settings.len();
}

/// Remove per-machine state from `Preferences`.
fn sanitize_preferences(path: &Path, report: &mut ProfileImportReport) {
  let Ok(raw) = std::fs::read_to_string(path) else {
    return;
  };
  let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
    return;
  };
  let Some(object) = value.as_object_mut() else {
    return;
  };

  // Download paths point at directories on the source machine.
  for (section, key) in [
    ("download", "default_directory"),
    ("savefile", "default_directory"),
    ("download", "last_directory"),
    ("selectfile", "last_directory"),
  ] {
    if let Some(map) = object.get_mut(section).and_then(|s| s.as_object_mut()) {
      map.remove(key);
    }
  }

  // Tell Chromium the previous session ended cleanly, or the imported profile
  // opens with a "restore pages?" bubble for a crash that never happened.
  if let Some(profile) = object.get_mut("profile").and_then(|p| p.as_object_mut()) {
    profile.insert(
      "exit_type".to_string(),
      serde_json::Value::String("Normal".to_string()),
    );
    profile.insert("exited_cleanly".to_string(), serde_json::Value::Bool(true));
  }

  // Languages are part of the fingerprint Wayfern applies at launch. Carrying
  // the source machine's list would contradict it, which is exactly the kind
  // of inconsistency an anti-detect profile exists to avoid.
  if let Some(intl) = object.get_mut("intl").and_then(|i| i.as_object_mut()) {
    intl.remove("accept_languages");
    intl.remove("selected_languages");
  }

  strip_absolute_extension_paths(object, report);

  if let Ok(serialized) = serde_json::to_string(&value) {
    let _ = std::fs::write(path, serialized);
  }
}

/// Count what survived, for the report.
fn tally(default_dir: &Path, report: &mut ProfileImportReport) {
  if let Some(conn) = open_rw(&default_dir.join("History")) {
    if let Ok(count) = conn.query_row("SELECT count(*) FROM urls", [], |r| r.get::<_, i64>(0)) {
      report.history_entries = count.max(0) as usize;
    }
  }

  if let Ok(raw) = std::fs::read_to_string(default_dir.join("Bookmarks")) {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
      report.bookmarks = count_bookmarks(value.get("roots"));
    }
  }

  report.local_storage_origins =
    copy::count_leveldb_origins(&default_dir.join("Local Storage").join("leveldb"));
}

fn count_bookmarks(node: Option<&serde_json::Value>) -> usize {
  let Some(node) = node else { return 0 };
  match node {
    serde_json::Value::Object(map) => {
      if map.get("type").and_then(|t| t.as_str()) == Some("url") {
        return 1;
      }
      map.values().map(|v| count_bookmarks(Some(v))).sum()
    }
    serde_json::Value::Array(items) => items.iter().map(|v| count_bookmarks(Some(v))).sum(),
    _ => 0,
  }
}

/// Everything that has to happen to a freshly copied `Default/` before the
/// browser sees it.
pub fn finalize_profile(
  default_dir: &Path,
  source: &SourceKeyring,
  target: &TargetKey,
  report: &mut ProfileImportReport,
) {
  sanitize_preferences(&default_dir.join("Preferences"), report);
  sanitize_secure_preferences(&default_dir.join("Secure Preferences"), report);

  if source.is_empty() {
    // No source key. Say so — but still run the pass. Rows that were stored in
    // plaintext (an old profile, a browser that could not reach its keyring,
    // or our own cookie importer) are perfectly recoverable and get sealed
    // with the target key; only the genuinely encrypted ones are lost, and
    // they were lost the moment the key was unavailable. Skipping the pass
    // here would report zero cookies carried for a profile that has plenty.
    report.warn(warning::SECRETS_NOT_MIGRATED);
  }
  reencrypt_profile(default_dir, source, target, report);

  tally(default_dir, report);
}

#[cfg(test)]
mod tests {
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

    assert_eq!(report.passwords_migrated, 1);
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

    assert_eq!(report.passwords_migrated, 1);
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
    assert_eq!(report.passwords_migrated, 0);
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
}

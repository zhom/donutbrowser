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
      report.logins_migrated += counts.migrated;
      report.logins_unrecoverable += counts.unrecoverable;
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
#[path = "rewrite_tests.rs"]
mod tests;

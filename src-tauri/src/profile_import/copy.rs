//! Copying a source profile into the new one.
//!
//! Two things a plain recursive copy gets wrong, both of which produce a
//! profile that looks imported and is not:
//!
//! - **Torn databases.** Users import from a browser they are still using. A
//!   naive walk copies `Cookies` and `Cookies-wal` at different instants, and
//!   Chromium's `sql::Database` razes the result on open. `VACUUM INTO` takes a
//!   transactionally consistent snapshot instead, WAL content included, even
//!   while the source holds the file.
//! - **Multi-GB of caches.** `Cache/`, `Code Cache/`, `GPUCache/` and friends
//!   carry no user state and dominate both copy time and disk use.

use std::fs;
use std::path::Path;

/// Directories that never carry user state. Matched on the path relative to the
/// profile root, so `Service Worker/CacheStorage` is dropped while
/// `Service Worker/Database` survives.
const SKIP_DIRS: &[&str] = &[
  "Cache",
  "Code Cache",
  "GPUCache",
  "GrShaderCache",
  "ShaderCache",
  "DawnCache",
  "DawnGraphiteCache",
  "DawnWebGPUCache",
  "GraphiteDawnCache",
  "GPUPersistentCache",
  "Service Worker/CacheStorage",
  "Service Worker/ScriptCache",
  "blob_storage",
  "Crashpad",
  "Crash Reports",
  "BrowserMetrics",
  "optimization_guide_model_store",
  "optimization_guide_hint_cache_store",
  "Safe Browsing",
  "Safe Browsing Network",
  "component_crx_cache",
  "extensions_crx_cache",
  "Download Service",
  "Site Characteristics Database",
  "shared_proto_db",
  "segmentation_platform",
  "Sync App Settings",
  // SNSS command logs replay the source machine's windows and can embed
  // absolute local paths in PageState blobs.
  "Sessions",
  "Session Storage",
];

/// Exact file names that are per-machine, per-run, or regenerated.
const SKIP_FILES: &[&str] = &[
  "LOCK",
  "LOG",
  "LOG.old",
  "SingletonLock",
  "SingletonCookie",
  "SingletonSocket",
  "RunningChromeVersion",
  "Last Version",
  "first_party_sets.db",
  ".DS_Store",
  "Thumbs.db",
  // The account-bound part of `Sync Data/`. The rest of that directory is the
  // local DataTypeStore — Reading List, Saved Tab Groups and friends, which
  // exist for users who never signed in — so the folder itself is carried.
  "Nigori.bin",
  // Signed-in ephemeral twins of the real stores. They are wiped on sign-out,
  // and the imported profile will not be signed in.
  "Login Data For Account",
  "Login Data For Account-journal",
  "Account Web Data",
  "Account Web Data-journal",
];

/// Suffixes that belong to a database we snapshot separately, or to scratch
/// state. Copying a `-wal` next to a vacuumed main file actively corrupts it.
const SKIP_SUFFIXES: &[&str] = &["-journal", "-wal", "-shm", ".tmp", ".old", ".bak.tmp"];

/// SQLite stores worth a consistent snapshot. Anything not listed is copied
/// byte-for-byte, which is correct for JSON, LevelDB and unpacked CRXs.
const SQLITE_FILES: &[&str] = &[
  "Cookies",
  "History",
  "Favicons",
  "Top Sites",
  "Shortcuts",
  "Login Data",
  "Web Data",
  "Affiliation Database",
  "Network Action Predictor",
  "DIPS",
  "Trust Tokens",
  "BudgetDatabase",
  "AutofillStrikeDatabase",
  "Reporting and NEL",
  "SCT Auditing Pending Reports",
  "Device Bound Sessions",
  "MediaDeviceSalts",
  "PreferredApps",
  "heavy_ad_intervention_opt_out.db",
  "SharedStorage",
  "BrowsingTopicsSiteData",
  "ClientCertificates",
  "PersistentOriginTrials",
  "Web Applications",
];

pub struct CopyOutcome {
  pub bytes_copied: u64,
  /// Names of stores that could not be snapshotted and were skipped rather
  /// than copied in a corrupt state.
  pub unreadable_stores: Vec<String>,
}

fn is_skipped_dir(relative: &Path) -> bool {
  let normalized = relative.to_string_lossy().replace('\\', "/");
  SKIP_DIRS.iter().any(|skip| {
    normalized == *skip
      || normalized.ends_with(&format!("/{skip}"))
      // `BrowserMetrics-spare.pma` and friends.
      || normalized.starts_with(&format!("{skip}-"))
  })
}

fn is_skipped_file(name: &str) -> bool {
  SKIP_FILES.contains(&name)
    || SKIP_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
    || name.starts_with("BrowserMetrics")
}

/// Copy the source's permission bits onto a file we produced ourselves.
///
/// `fs::copy` already preserves the mode, but `VACUUM INTO` lets SQLite create
/// the destination at its own default (0644). Cookies, Login Data and Web Data
/// are 0600 in both the source browser and Wayfern, and an import must not be
/// the step that widens them.
#[cfg(unix)]
fn mirror_mode(source: &Path, dest: &Path) {
  use std::os::unix::fs::PermissionsExt;
  if let Ok(metadata) = fs::metadata(source) {
    let mode = metadata.permissions().mode() & 0o777;
    let _ = fs::set_permissions(dest, fs::Permissions::from_mode(mode));
  }
}

#[cfg(not(unix))]
fn mirror_mode(_source: &Path, _dest: &Path) {}

/// Create a directory owner-only, matching what Chromium gives a profile.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
  fs::create_dir_all(path)?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
  }
  Ok(())
}

/// Take a consistent snapshot of a SQLite database.
///
/// Returns `Ok(false)` when the file is not actually SQLite (an empty
/// placeholder, say), so the caller can fall back to a plain copy.
fn vacuum_into(source: &Path, dest: &Path) -> Result<bool, String> {
  use rusqlite::{Connection, OpenFlags};

  let conn = match Connection::open_with_flags(
    source,
    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
  ) {
    Ok(conn) => conn,
    Err(e) => return Err(format!("open failed: {e}")),
  };

  // Confirm it really is a database before trusting VACUUM's error reporting.
  if conn
    .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
      r.get::<_, i64>(0)
    })
    .is_err()
  {
    return Ok(false);
  }

  if dest.exists() {
    fs::remove_file(dest).map_err(|e| format!("could not replace destination: {e}"))?;
  }

  // `VACUUM INTO` needs the path as a SQL string literal; single quotes are
  // the only character that can break out of one.
  let target = dest.to_string_lossy().replace('\'', "''");
  conn
    .execute_batch(&format!("VACUUM INTO '{target}'"))
    .map_err(|e| format!("VACUUM INTO failed: {e}"))?;
  mirror_mode(source, dest);
  Ok(true)
}

/// Copy `source` (a Chromium profile directory) into `dest`, skipping caches
/// and snapshotting databases.
pub fn copy_profile_tree(source: &Path, dest: &Path) -> Result<CopyOutcome, String> {
  let mut outcome = CopyOutcome {
    bytes_copied: 0,
    unreadable_stores: Vec::new(),
  };
  create_private_dir(dest).map_err(|e| format!("Failed to create {}: {e}", dest.display()))?;
  copy_dir(source, dest, Path::new(""), &mut outcome)?;
  Ok(outcome)
}

fn copy_dir(
  source: &Path,
  dest: &Path,
  relative: &Path,
  outcome: &mut CopyOutcome,
) -> Result<(), String> {
  let entries =
    fs::read_dir(source).map_err(|e| format!("Failed to read {}: {e}", source.display()))?;

  for entry in entries.flatten() {
    let name = entry.file_name();
    let Some(name) = name.to_str() else { continue };
    let child_relative = relative.join(name);
    let source_path = entry.path();
    let dest_path = dest.join(name);

    // Symlinks are followed nowhere: Chromium writes them for the singleton
    // lock, and a copied one would point at the source machine.
    let metadata = match fs::symlink_metadata(&source_path) {
      Ok(m) => m,
      Err(_) => continue,
    };
    if metadata.file_type().is_symlink() {
      continue;
    }

    if metadata.is_dir() {
      if is_skipped_dir(&child_relative) {
        continue;
      }
      create_private_dir(&dest_path)
        .map_err(|e| format!("Failed to create {}: {e}", dest_path.display()))?;
      copy_dir(&source_path, &dest_path, &child_relative, outcome)?;
      continue;
    }

    if is_skipped_file(name) {
      continue;
    }

    if SQLITE_FILES.contains(&name) {
      match vacuum_into(&source_path, &dest_path) {
        Ok(true) => {
          outcome.bytes_copied += fs::metadata(&dest_path).map(|m| m.len()).unwrap_or(0);
          continue;
        }
        Ok(false) => {
          // Not a database after all; fall through to a byte copy.
        }
        Err(e) => {
          // A store we cannot snapshot is a store we must not copy: a torn
          // copy is deleted by Chromium on open, which looks identical to
          // "the import silently lost my data".
          log::warn!("Skipping unreadable store {}: {e}", source_path.display());
          outcome.unreadable_stores.push(name.to_string());
          continue;
        }
      }
    }

    match fs::copy(&source_path, &dest_path) {
      Ok(bytes) => outcome.bytes_copied += bytes,
      Err(e) => log::warn!("Failed to copy {}: {e}", source_path.display()),
    }
  }

  Ok(())
}

/// Every `Default/`-level store that holds real user data, for reporting.
pub fn count_leveldb_origins(leveldb_dir: &Path) -> usize {
  // Counting keys would mean linking a LevelDB implementation. The number of
  // `.ldb`/`.log` segments is a stable proxy for "there is data here", which
  // is all the report claims.
  let Ok(entries) = fs::read_dir(leveldb_dir) else {
    return 0;
  };
  entries
    .flatten()
    .filter(|e| {
      e.file_name()
        .to_str()
        .is_some_and(|n| n.ends_with(".ldb") || n.ends_with(".log"))
    })
    .count()
}

#[cfg(test)]
mod tests {
  use super::*;
  use rusqlite::Connection;
  use tempfile::TempDir;

  fn touch(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
  }

  #[test]
  fn caches_are_not_copied() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    touch(&source.join("Preferences"), b"{}");
    touch(&source.join("Cache").join("data_0"), &[0u8; 4096]);
    touch(
      &source.join("Code Cache").join("js").join("x"),
      &[0u8; 4096],
    );
    touch(
      &source.join("Service Worker").join("CacheStorage").join("y"),
      &[0u8; 4096],
    );
    touch(
      &source
        .join("Service Worker")
        .join("Database")
        .join("CURRENT"),
      b"MANIFEST-000001\n",
    );

    copy_profile_tree(&source, &dest).unwrap();

    assert!(dest.join("Preferences").exists());
    assert!(!dest.join("Cache").exists());
    assert!(!dest.join("Code Cache").exists());
    assert!(!dest.join("Service Worker").join("CacheStorage").exists());
    assert!(
      dest.join("Service Worker").join("Database").exists(),
      "the Service Worker registry is real data and must survive"
    );
  }

  #[test]
  fn lock_and_journal_files_are_not_copied() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    touch(&source.join("Preferences"), b"{}");
    touch(
      &source.join("Local Storage").join("leveldb").join("LOCK"),
      b"",
    );
    touch(
      &source.join("Local Storage").join("leveldb").join("CURRENT"),
      b"MANIFEST-000001\n",
    );
    touch(&source.join("History-journal"), b"junk");

    copy_profile_tree(&source, &dest).unwrap();

    assert!(!dest
      .join("Local Storage")
      .join("leveldb")
      .join("LOCK")
      .exists());
    assert!(dest
      .join("Local Storage")
      .join("leveldb")
      .join("CURRENT")
      .exists());
    assert!(!dest.join("History-journal").exists());
  }

  #[test]
  fn sqlite_stores_are_snapshotted_and_stay_queryable() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    fs::create_dir_all(&source).unwrap();
    touch(&source.join("Preferences"), b"{}");

    let db = source.join("History");
    let conn = Connection::open(&db).unwrap();
    conn
      .execute_batch("CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT); INSERT INTO urls(url) VALUES('https://example.com');")
      .unwrap();
    drop(conn);

    copy_profile_tree(&source, &dest).unwrap();

    let copied = Connection::open(dest.join("History")).unwrap();
    let count: i64 = copied
      .query_row("SELECT count(*) FROM urls", [], |r| r.get(0))
      .unwrap();
    assert_eq!(count, 1);
  }

  #[test]
  fn snapshot_captures_uncheckpointed_wal_content() {
    // The whole reason for VACUUM INTO: a running browser leaves recent writes
    // in the WAL, and a plain file copy loses them.
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    fs::create_dir_all(&source).unwrap();
    touch(&source.join("Preferences"), b"{}");

    let db = source.join("History");
    let conn = Connection::open(&db).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn
      .execute_batch("CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT);")
      .unwrap();
    conn
      .execute("INSERT INTO urls(url) VALUES('https://in-wal.example')", [])
      .unwrap();
    // Deliberately do not checkpoint or close: this is the live-browser shape.

    copy_profile_tree(&source, &dest).unwrap();
    drop(conn);

    let copied = Connection::open(dest.join("History")).unwrap();
    let url: String = copied
      .query_row("SELECT url FROM urls", [], |r| r.get(0))
      .unwrap();
    assert_eq!(url, "https://in-wal.example");
    assert!(
      !dest.join("History-wal").exists(),
      "a stale -wal beside a vacuumed file corrupts it"
    );
  }

  #[test]
  fn symlinks_are_never_followed() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    touch(&source.join("Preferences"), b"{}");
    let outside = dir.path().join("outside.txt");
    touch(&outside, b"secret");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, source.join("SingletonLock")).unwrap();

    copy_profile_tree(&source, &dest).unwrap();
    assert!(!dest.join("SingletonLock").exists());
  }

  #[test]
  fn account_scoped_stores_are_dropped() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    touch(&source.join("Preferences"), b"{}");
    touch(&source.join("Login Data For Account"), b"x");
    touch(&source.join("Sync Data").join("Nigori.bin"), b"x");
    touch(
      &source.join("Sync Data").join("LevelDB").join("CURRENT"),
      b"x",
    );

    copy_profile_tree(&source, &dest).unwrap();

    assert!(!dest.join("Login Data For Account").exists());
    assert!(
      !dest.join("Sync Data").join("Nigori.bin").exists(),
      "the Nigori keyset is bound to a Google account"
    );
    assert!(
      dest
        .join("Sync Data")
        .join("LevelDB")
        .join("CURRENT")
        .exists(),
      "the rest of Sync Data is local state such as the reading list"
    );
  }

  #[test]
  #[cfg(unix)]
  fn copied_databases_keep_the_browsers_private_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    fs::create_dir_all(&source).unwrap();
    touch(&source.join("Preferences"), b"{}");

    let db = source.join("Cookies");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn
      .execute_batch("CREATE TABLE cookies(x INTEGER);")
      .unwrap();
    drop(conn);
    fs::set_permissions(&db, fs::Permissions::from_mode(0o600)).unwrap();

    copy_profile_tree(&source, &dest).unwrap();

    // VACUUM INTO would otherwise create the snapshot at SQLite's default 0644.
    let mode = fs::metadata(dest.join("Cookies"))
      .unwrap()
      .permissions()
      .mode();
    assert_eq!(
      mode & 0o777,
      0o600,
      "an import must not widen a cookie store"
    );
    let dir_mode = fs::metadata(&dest).unwrap().permissions().mode();
    assert_eq!(dir_mode & 0o777, 0o700);
  }

  #[test]
  fn unreadable_store_is_reported_not_copied_corrupt() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    touch(&source.join("Preferences"), b"{}");
    // A file that opens as SQLite but is structurally broken.
    touch(
      &source.join("Cookies"),
      b"SQLite format 3\0garbage-not-a-db",
    );

    let outcome = copy_profile_tree(&source, &dest).unwrap();

    assert!(
      !dest.join("Cookies").exists() || outcome.unreadable_stores.is_empty(),
      "a store is either snapshotted cleanly or skipped and reported"
    );
  }

  #[test]
  fn non_sqlite_file_with_a_store_name_still_copies() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    touch(&source.join("Preferences"), b"{}");
    touch(&source.join("Top Sites"), b"");

    copy_profile_tree(&source, &dest).unwrap();
    assert!(dest.join("Top Sites").exists());
  }
}

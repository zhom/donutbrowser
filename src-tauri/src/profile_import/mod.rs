//! Turning someone else's browser profile into one Wayfern will actually load.
//!
//! The old importer copied a source profile directory verbatim onto the new
//! profile's `--user-data-dir`. Chromium reads `<user-data-dir>/Default/`, so
//! every imported file sat one level above where the browser looked and the
//! profile came up empty — and even in the right place the secrets would not
//! have opened, because they are sealed with a key held in the source
//! machine's Keychain / DPAPI / secret service that Wayfern never consults.
//!
//! This module does the whole job: classify the source, recover its key, copy
//! with consistent database snapshots, put the files where Chromium reads them,
//! re-seal every secret with Wayfern's portable key, and report exactly what
//! came across.

pub mod copy;
pub mod keyring;
pub mod layout;
pub mod os_crypt;
pub mod report;
pub mod rewrite;

use layout::RejectReason;
use report::{warning, ProfileImportReport};
use std::path::Path;

/// The profile subdirectory Chromium reads when no `--profile-directory` is
/// passed (`chrome_constants.cc` `kInitialProfile`). Donut never passes one.
pub const INITIAL_PROFILE_DIR: &str = "Default";

/// Import `source` into `dest_user_data_dir`, which becomes the new profile's
/// `--user-data-dir`.
///
/// Never fails because part of the data could not be carried: partial results
/// plus an honest report beat an all-or-nothing import that leaves the user
/// with nothing and no explanation. It fails only when the source is not
/// importable at all, or when the target key cannot be established — without
/// that key, anything written would be unreadable forever.
pub fn import_into(
  source: &Path,
  dest_user_data_dir: &Path,
  source_family: &str,
  allow_running: bool,
) -> Result<ProfileImportReport, String> {
  let shape = layout::classify(source).map_err(|reason| match reason {
    RejectReason::Firefox => serde_json::json!({
      "code": "IMPORT_SOURCE_NOT_CHROMIUM",
      "params": { "family": "Firefox" }
    })
    .to_string(),
    RejectReason::NotChromium => serde_json::json!({
      "code": "IMPORT_SOURCE_NOT_CHROMIUM",
      "params": { "family": "" }
    })
    .to_string(),
  })?;

  let mut report = ProfileImportReport::default();

  if let Some(running) = running_source_browser(&shape) {
    if !allow_running {
      return Err(
        serde_json::json!({
          "code": "IMPORT_SOURCE_BROWSER_RUNNING",
          "params": { "browser": running }
        })
        .to_string(),
      );
    }
    // Databases are snapshotted transactionally, but LevelDB site data is
    // copied as files and can be mid-write.
    report.warn(warning::SOURCE_BROWSER_RUNNING);
  }

  // Mint the target key first. Everything after this point is written to be
  // readable with it, and a profile whose key could not be persisted would
  // lose every secret the first time the browser exits.
  let target = os_crypt::TargetKey::ensure(dest_user_data_dir)?;

  // Recover the source key before the copy: on macOS this may prompt, and
  // asking before a multi-GB copy respects the user's time.
  let source_keys = keyring::recover_source_keys(
    source_family,
    &shape.profile_dir,
    shape.user_data_dir.as_deref(),
    &mut report,
  );

  let default_dir = dest_user_data_dir.join(INITIAL_PROFILE_DIR);
  let outcome = copy::copy_profile_tree(&shape.profile_dir, &default_dir)?;
  report.bytes_copied = outcome.bytes_copied;
  if !outcome.unreadable_stores.is_empty() {
    report.warn(warning::STORE_UNREADABLE);
  }

  layout::normalize_network_dir(&default_dir)
    .map_err(|e| format!("Failed to place network data: {e}"))?;

  rewrite::finalize_profile(&default_dir, &source_keys, &target, &mut report);

  Ok(report)
}

/// Is the browser that owns this profile currently running?
///
/// Matched on the profile path in the process command line rather than on the
/// executable name: the user may well have Chrome open on a *different*
/// profile, which is no reason to block the import.
fn running_source_browser(shape: &layout::SourceShape) -> Option<String> {
  use sysinfo::{ProcessRefreshKind, RefreshKind, System};

  let system = System::new_with_specifics(
    RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
  );

  let needle = shape
    .user_data_dir
    .as_deref()
    .unwrap_or(&shape.profile_dir)
    .to_string_lossy()
    .to_string();
  if needle.is_empty() {
    return None;
  }

  for process in system.processes().values() {
    let name = process.name().to_string_lossy().to_lowercase();
    let looks_like_a_browser = name.contains("chrome")
      || name.contains("chromium")
      || name.contains("brave")
      || name.contains("edge")
      || name.contains("vivaldi")
      || name.contains("opera")
      || name.contains("arc")
      || name.contains("yandex");
    if !looks_like_a_browser {
      continue;
    }
    // Donut's own browser is Wayfern; never report it as the source.
    if name.contains("wayfern") {
      continue;
    }
    if process
      .cmd()
      .iter()
      .any(|arg| arg.to_string_lossy().contains(&needle))
    {
      return Some(process.name().to_string_lossy().to_string());
    }
  }

  None
}

/// Move a profile that an earlier build imported into the broken root layout
/// down into `Default/`, where the browser reads it.
///
/// Without this, everything those users imported stays stranded: their real
/// data sits at `profile/Cookies` while Wayfern reads and writes
/// `profile/Default/Cookies`. Their secrets remain unreadable — the source key
/// was never captured and cannot be recovered after the fact — but history,
/// bookmarks, extensions and site data become visible again.
///
/// Returns `Ok(true)` when a repair was performed.
pub fn repair_legacy_layout(user_data_dir: &Path) -> Result<bool, String> {
  let default_dir = user_data_dir.join(INITIAL_PROFILE_DIR);

  // The broken shape is exactly: profile markers at the root, and no `Default/`
  // for the browser to have used instead.
  let has_root_profile = user_data_dir.join("Preferences").exists()
    || user_data_dir.join("History").exists()
    || user_data_dir.join("Cookies").exists();
  if !has_root_profile || default_dir.exists() {
    return Ok(false);
  }

  // Root-level files that belong to the user-data dir, not to the profile.
  const ROOT_LEVEL: &[&str] = &[
    "Local State",
    "os_crypt_key",
    "First Run",
    "Last Version",
    "Variations",
    "ChromeFeatureState",
    "RunningChromeVersion",
    "SingletonLock",
    "SingletonCookie",
    "SingletonSocket",
    "user.js",
    "metadata.json",
    ".donut-sync",
  ];

  let staging = user_data_dir.join(".donut-import-repair");
  if staging.exists() {
    std::fs::remove_dir_all(&staging).map_err(|e| format!("Failed to clear staging: {e}"))?;
  }
  std::fs::create_dir_all(&staging).map_err(|e| format!("Failed to create staging: {e}"))?;

  let entries =
    std::fs::read_dir(user_data_dir).map_err(|e| format!("Failed to read profile: {e}"))?;
  for entry in entries.flatten() {
    let name = entry.file_name();
    let Some(name_str) = name.to_str() else {
      continue;
    };
    if ROOT_LEVEL.contains(&name_str) || name_str == ".donut-import-repair" {
      continue;
    }
    std::fs::rename(entry.path(), staging.join(name_str))
      .map_err(|e| format!("Failed to relocate {name_str}: {e}"))?;
  }

  std::fs::rename(&staging, &default_dir)
    .map_err(|e| format!("Failed to install {INITIAL_PROFILE_DIR}: {e}"))?;

  // Now that the files are in the right place, put the network data where this
  // platform reads it too.
  let _ = layout::normalize_network_dir(&default_dir);
  // And make sure the profile has a key, so the browser does not mint one
  // mid-session and lose whatever it writes.
  let _ = os_crypt::TargetKey::ensure(user_data_dir);

  log::info!(
    "Repaired legacy import layout at {} (moved profile content into {INITIAL_PROFILE_DIR}/)",
    user_data_dir.display()
  );
  Ok(true)
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn touch(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
  }

  #[test]
  fn import_places_everything_under_default() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("Chrome").join("Default");
    let dest = dir.path().join("profile");
    touch(&source.join("Preferences"), b"{}");
    touch(&source.join("Bookmarks"), b"{\"roots\":{}}");

    let report = import_into(&source, &dest, "chromium", true).expect("import");

    assert!(
      dest.join("Default").join("Preferences").exists(),
      "Chromium reads Default/, not the user-data-dir root"
    );
    assert!(
      !dest.join("Preferences").exists(),
      "nothing profile-scoped belongs at the root"
    );
    assert!(dest.join(os_crypt::KEY_FILE_NAME).exists());
    assert!(report.bytes_copied > 0);
  }

  #[test]
  fn import_rejects_a_firefox_profile_by_name() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("xyz.default-release");
    let dest = dir.path().join("profile");
    touch(&source.join("prefs.js"), b"");
    touch(&source.join("places.sqlite"), b"");

    let err = import_into(&source, &dest, "firefox", true).expect_err("must reject");
    assert!(err.contains("IMPORT_SOURCE_NOT_CHROMIUM"));
    assert!(
      err.contains("Firefox"),
      "the user needs to be told why, not just that it failed"
    );
  }

  #[test]
  fn import_rejects_an_arbitrary_folder() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("holiday-photos");
    let dest = dir.path().join("profile");
    touch(&source.join("IMG_0001.jpg"), b"not a profile");

    let err = import_into(&source, &dest, "chromium", true).expect_err("must reject");
    assert!(err.contains("IMPORT_SOURCE_NOT_CHROMIUM"));
  }

  #[test]
  fn import_is_rerunnable_over_the_same_destination() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("Default");
    let dest = dir.path().join("profile");
    touch(&source.join("Preferences"), b"{}");

    import_into(&source, &dest, "chromium", true).expect("first");
    let key = std::fs::read(dest.join(os_crypt::KEY_FILE_NAME)).unwrap();
    import_into(&source, &dest, "chromium", true).expect("second");
    assert_eq!(
      std::fs::read(dest.join(os_crypt::KEY_FILE_NAME)).unwrap(),
      key,
      "re-running must not orphan what the first run encrypted"
    );
  }

  #[test]
  fn legacy_layout_is_repaired_into_default() {
    let dir = TempDir::new().unwrap();
    let profile = dir.path().join("profile");
    // Exactly what the old importer produced.
    touch(&profile.join("Preferences"), b"{}");
    touch(&profile.join("History"), b"");
    touch(
      &profile
        .join("Local Storage")
        .join("leveldb")
        .join("CURRENT"),
      b"",
    );
    touch(&profile.join("Local State"), b"{}");

    assert!(repair_legacy_layout(&profile).unwrap());

    assert!(profile.join("Default").join("Preferences").exists());
    assert!(profile.join("Default").join("History").exists());
    assert!(profile
      .join("Default")
      .join("Local Storage")
      .join("leveldb")
      .join("CURRENT")
      .exists());
    assert!(
      profile.join("Local State").exists(),
      "Local State belongs to the user-data dir, not the profile"
    );
    assert!(profile.join(os_crypt::KEY_FILE_NAME).exists());
    assert!(!profile.join(".donut-import-repair").exists());
  }

  #[test]
  fn repair_leaves_a_healthy_profile_alone() {
    let dir = TempDir::new().unwrap();
    let profile = dir.path().join("profile");
    touch(&profile.join("Default").join("Preferences"), b"{}");
    touch(&profile.join("Local State"), b"{}");

    assert!(!repair_legacy_layout(&profile).unwrap());
    assert!(profile.join("Default").join("Preferences").exists());
    assert!(!profile.join("Default").join("Default").exists());
  }

  #[test]
  fn repair_is_a_no_op_on_an_empty_profile() {
    let dir = TempDir::new().unwrap();
    let profile = dir.path().join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    assert!(!repair_legacy_layout(&profile).unwrap());
  }

  #[test]
  fn repair_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let profile = dir.path().join("profile");
    touch(&profile.join("Preferences"), b"{}");

    assert!(repair_legacy_layout(&profile).unwrap());
    assert!(!repair_legacy_layout(&profile).unwrap());
    assert!(profile.join("Default").join("Preferences").exists());
  }
}

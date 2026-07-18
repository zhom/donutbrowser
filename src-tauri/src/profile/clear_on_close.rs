//! Clear-on-close: wipe a profile's browsing data when the browser exits,
//! keeping extensions and bookmarks (and the settings files Chromium needs to
//! keep those working). The middle ground between a fully persistent profile
//! and a RAM-backed ephemeral one.

use std::fs;
use std::path::Path;

use crate::profile::types::BrowserProfile;

/// Entries kept inside a Chromium profile directory (one with `Preferences`).
/// Everything else — cookies, History, caches, storage, sessions, autofill,
/// saved logins — is browsing data and gets removed.
const PROFILE_KEEP: &[&str] = &[
  "Bookmarks",
  "Bookmarks.bak",
  "Extensions",
  "Extension State",
  "Extension Rules",
  "Extension Scripts",
  "Extension Cookies",
  "Local Extension Settings",
  "Managed Extension Settings",
  // Preferences hold the extension registry + user settings; deleting them
  // disables every installed extension, so they stay.
  "Preferences",
  "Secure Preferences",
];

/// Entries kept at the user-data-dir top level (outside profile subdirs).
const TOP_KEEP: &[&str] = &["Local State", "First Run"];

fn is_kept(name: &str) -> bool {
  PROFILE_KEEP.contains(&name) || TOP_KEEP.contains(&name)
}

/// Whether a top-level entry name is one of Chromium's profile directories.
///
/// Identity must not rest on `Preferences` existing. Chromium writes it lazily,
/// so a crash — or a user deleting a corrupt copy, a standard troubleshooting
/// step since it regenerates — leaves a populated `Default/` without it. Such a
/// directory would then be taken for junk and removed wholesale, destroying the
/// Extensions and Bookmarks this feature exists to preserve.
fn is_profile_dir_name(name: &str) -> bool {
  matches!(name, "Default" | "Guest Profile" | "System Profile")
    || name
      .strip_prefix("Profile ")
      .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn remove_entry(path: &Path) {
  let result = if path.is_dir() {
    fs::remove_dir_all(path)
  } else {
    fs::remove_file(path)
  };
  if let Err(e) = result {
    log::warn!("clear-on-close: failed to remove {}: {e}", path.display());
  }
}

/// Wipe browsing data inside a Chromium profile dir, keeping `PROFILE_KEEP`
/// (and `TOP_KEEP`, harmless at this level).
fn clear_chromium_profile_dir(profile_dir: &Path) -> usize {
  let mut cleared = 0usize;
  let Ok(entries) = fs::read_dir(profile_dir) else {
    return 0;
  };
  for entry in entries.flatten() {
    let name = entry.file_name();
    let Some(name) = name.to_str() else { continue };
    if is_kept(name) {
      continue;
    }
    remove_entry(&entry.path());
    cleared += 1;
  }
  cleared
}

/// Clear browsing data in a Wayfern user-data directory. Handles both
/// layouts: profile content at the root (imported profiles copy a Chromium
/// profile dir directly) and the standard `Default` / `Profile N` subdirs
/// Chromium creates itself. Top-level cache dirs (ShaderCache, GrShaderCache,
/// component_crx_cache, …) are removed; `Local State` stays because it holds
/// the os_crypt key extensions may rely on.
pub fn clear_user_data_dir(user_data_dir: &Path) -> usize {
  if !user_data_dir.exists() {
    return 0;
  }

  // Root itself is a profile dir (legacy/imported layout).
  if user_data_dir.join("Preferences").exists() {
    return clear_chromium_profile_dir(user_data_dir);
  }

  let mut cleared = 0usize;
  let Ok(entries) = fs::read_dir(user_data_dir) else {
    return 0;
  };
  for entry in entries.flatten() {
    let name = entry.file_name();
    let Some(name) = name.to_str() else { continue };
    if is_kept(name) {
      continue;
    }
    let path = entry.path();
    if path.is_dir() && (path.join("Preferences").exists() || is_profile_dir_name(name)) {
      // A profile subdir (Default / Profile N) — clear inside, keep the dir.
      cleared += clear_chromium_profile_dir(&path);
    } else {
      remove_entry(&path);
      cleared += 1;
    }
  }
  cleared
}

/// Clear a profile's browsing data if its `clear_on_close` flag is set.
/// No-ops for ephemeral (already wiped) and password-protected (dir is
/// re-encrypted; plaintext never persists) profiles. Runs the filesystem work
/// on a blocking thread.
pub async fn clear_profile_browsing_data(profile: &BrowserProfile) {
  if !profile.clear_on_close || profile.ephemeral || profile.password_protected {
    return;
  }

  let profiles_dir = crate::profile::ProfileManager::instance().get_profiles_dir();
  let user_data_dir = crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);
  let name = profile.name.clone();

  let cleared = tokio::task::spawn_blocking(move || clear_user_data_dir(&user_data_dir))
    .await
    .unwrap_or(0);

  log::info!("clear-on-close: cleared {cleared} browsing-data entries for profile '{name}'");
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn touch(dir: &Path, name: &str) {
    fs::write(dir.join(name), "x").unwrap();
  }

  fn mkdir(dir: &Path, name: &str) {
    fs::create_dir_all(dir.join(name)).unwrap();
  }

  #[test]
  fn clears_root_profile_layout_keeping_extensions_and_bookmarks() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    touch(dir, "Preferences");
    touch(dir, "Secure Preferences");
    touch(dir, "Bookmarks");
    touch(dir, "History");
    touch(dir, "Web Data");
    touch(dir, "Login Data");
    mkdir(dir, "Extensions");
    mkdir(dir, "Local Extension Settings");
    mkdir(dir, "Cache");
    mkdir(dir, "Network");
    touch(&dir.join("Network"), "Cookies");
    mkdir(dir, "Local Storage");

    let cleared = clear_user_data_dir(dir);
    assert!(
      cleared >= 5,
      "should clear History/WebData/Login/Cache/Network/LocalStorage, got {cleared}"
    );

    assert!(dir.join("Preferences").exists());
    assert!(dir.join("Bookmarks").exists());
    assert!(dir.join("Extensions").exists());
    assert!(dir.join("Local Extension Settings").exists());
    assert!(!dir.join("History").exists());
    assert!(!dir.join("Web Data").exists());
    assert!(!dir.join("Login Data").exists());
    assert!(!dir.join("Cache").exists());
    assert!(!dir.join("Network").exists(), "Network/Cookies must go");
    assert!(!dir.join("Local Storage").exists());
  }

  #[test]
  fn clears_standard_user_data_layout() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    touch(dir, "Local State");
    mkdir(dir, "ShaderCache");
    mkdir(dir, "Default");
    let default = dir.join("Default");
    touch(&default, "Preferences");
    touch(&default, "Bookmarks");
    touch(&default, "History");
    mkdir(&default, "Extensions");
    mkdir(&default, "IndexedDB");

    clear_user_data_dir(dir);

    assert!(dir.join("Local State").exists());
    assert!(!dir.join("ShaderCache").exists());
    assert!(default.join("Preferences").exists());
    assert!(default.join("Bookmarks").exists());
    assert!(default.join("Extensions").exists());
    assert!(!default.join("History").exists());
    assert!(!default.join("IndexedDB").exists());
  }

  #[test]
  fn profile_subdir_without_preferences_keeps_extensions_and_bookmarks() {
    // Chromium writes Preferences lazily, so a crash (or a user removing a
    // corrupt copy) can leave a populated Default/ without it. Identifying
    // profile dirs by Preferences alone would delete the whole directory.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    touch(dir, "Local State");
    mkdir(dir, "Default");
    let default = dir.join("Default");
    touch(&default, "Bookmarks");
    touch(&default, "History");
    mkdir(&default, "Extensions");
    mkdir(&default, "Local Extension Settings");
    mkdir(&default, "IndexedDB");

    clear_user_data_dir(dir);

    assert!(default.exists(), "the profile dir must survive");
    assert!(default.join("Bookmarks").exists());
    assert!(default.join("Extensions").exists());
    assert!(default.join("Local Extension Settings").exists());
    // Browsing data inside it is still cleared.
    assert!(!default.join("History").exists());
    assert!(!default.join("IndexedDB").exists());
  }

  #[test]
  fn numbered_profile_dirs_are_recognised() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    mkdir(dir, "Profile 2");
    let p2 = dir.join("Profile 2");
    touch(&p2, "Bookmarks");
    touch(&p2, "History");

    clear_user_data_dir(dir);

    assert!(p2.join("Bookmarks").exists());
    assert!(!p2.join("History").exists());
  }

  #[test]
  fn non_profile_dirs_are_still_removed_wholesale() {
    // The permissive branch must not turn into "never delete a directory":
    // top-level caches are browsing data and have to go.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    mkdir(dir, "ShaderCache");
    mkdir(dir, "component_crx_cache");
    mkdir(dir, "Profileless");

    clear_user_data_dir(dir);

    assert!(!dir.join("ShaderCache").exists());
    assert!(!dir.join("component_crx_cache").exists());
    assert!(
      !dir.join("Profileless").exists(),
      "a name that merely starts with 'Profile' is not a profile dir"
    );
  }

  #[test]
  fn profile_dir_name_matching() {
    assert!(is_profile_dir_name("Default"));
    assert!(is_profile_dir_name("Profile 1"));
    assert!(is_profile_dir_name("Profile 42"));
    assert!(is_profile_dir_name("Guest Profile"));
    assert!(is_profile_dir_name("System Profile"));
    assert!(!is_profile_dir_name("Profile"));
    assert!(!is_profile_dir_name("Profile "));
    assert!(!is_profile_dir_name("Profile abc"));
    assert!(!is_profile_dir_name("ShaderCache"));
  }

  #[test]
  fn missing_dir_is_a_noop() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(clear_user_data_dir(&tmp.path().join("nope")), 0);
  }
}

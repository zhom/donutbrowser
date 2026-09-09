//! Recoverable delete for profiles.
//!
//! A deleted profile is moved to `<data root>/trash/<profile_id>/` instead of
//! being destroyed, so an accidental delete of a profile that carries logins
//! can be undone. Each entry holds:
//!
//! - `profile.json`: the full `BrowserProfile` at the moment of deletion.
//! - `manifest.json`: when it was trashed, when it expires, how big it is.
//! - `profile/`: the profile's own data directory, moved as is. Chromium's
//!   cache-only directories are pruned first; the browser rebuilds them on
//!   the next launch, so keeping them would only make the trash heavy.
//!
//! A password-protected profile is moved in its encrypted at-rest form and
//! stays protected while it sits here. Ephemeral profiles never land here;
//! their data lives in RAM and is gone the moment the browser exits.
//!
//! From the cloud's point of view a trashed profile is deleted: the sync
//! tombstone is written by the same path a permanent delete uses. Restoring
//! re-registers the profile under its original id and routes it through the
//! normal sync-enable path so it wins the stale tombstone.

use crate::profile::types::BrowserProfile;
use crate::profile::ProfileManager;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub const DEFAULT_RETENTION_DAYS: u32 = 30;
pub const MIN_RETENTION_DAYS: u32 = 1;
pub const MAX_RETENTION_DAYS: u32 = 365;
/// How often expired entries are swept while the app runs.
pub const PURGE_INTERVAL_SECS: u64 = 6 * 60 * 60;

const SECS_PER_DAY: u64 = 24 * 60 * 60;
const PROFILE_FILE: &str = "profile.json";
const MANIFEST_FILE: &str = "manifest.json";
const DATA_DIR: &str = "profile";
const METADATA_FILE: &str = "metadata.json";
const RESTORED_SUFFIX: &str = "(restored)";

/// Chromium directories that only ever hold caches, relative to the profile
/// data directory. Every one of them is recreated by the browser on demand.
const CACHE_DIRS: [&str; 8] = [
  "Cache",
  "Code Cache",
  "GPUCache",
  "GrShaderCache",
  "ShaderCache",
  "DawnCache",
  "Service Worker/CacheStorage",
  "Service Worker/ScriptCache",
];

/// Serialises every trash mutation so a restore cannot interleave with a
/// purge of the same entry.
static TRASH_MUTATION: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashManifest {
  pub deleted_at: u64,
  pub expires_at: u64,
  pub size_bytes: u64,
  pub original_name: String,
}

/// What the Trash page shows for one entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashedProfileSummary {
  pub id: String,
  pub name: String,
  pub browser: String,
  pub version: String,
  pub deleted_at: u64,
  pub expires_at: u64,
  pub size_bytes: u64,
  #[serde(default)]
  pub group_id: Option<String>,
  pub password_protected: bool,
}

pub fn trash_dir() -> PathBuf {
  crate::app_dirs::data_dir().join("trash")
}

pub fn mutation_lock() -> MutexGuard<'static, ()> {
  TRASH_MUTATION
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn clamp_retention_days(days: u32) -> u32 {
  days.clamp(MIN_RETENTION_DAYS, MAX_RETENTION_DAYS)
}

/// The retention the user configured, already clamped to the allowed range.
pub fn configured_retention_days() -> u32 {
  crate::settings_manager::SettingsManager::instance()
    .load_settings()
    .map(|settings| clamp_retention_days(settings.trash_retention_days))
    .unwrap_or(DEFAULT_RETENTION_DAYS)
}

fn err_internal(e: impl std::fmt::Display) -> String {
  crate::backend_error_with_detail("INTERNAL_ERROR", e)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
  let json = serde_json::to_string_pretty(value).map_err(err_internal)?;
  let tmp = path.with_extension("json.tmp");
  fs::write(&tmp, json).map_err(err_internal)?;
  fs::rename(&tmp, path).map_err(err_internal)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
  let content = fs::read_to_string(path).map_err(err_internal)?;
  serde_json::from_str(&content).map_err(err_internal)
}

/// Remove the cache-only directories from a profile data directory. Returns
/// the directories that were actually removed.
pub fn prune_cache_dirs(data_dir: &Path) -> Vec<PathBuf> {
  let mut removed = Vec::new();
  for relative in CACHE_DIRS {
    let dir = data_dir.join(relative);
    if !dir.is_dir() {
      continue;
    }
    match fs::remove_dir_all(&dir) {
      Ok(()) => removed.push(dir),
      Err(e) => log::warn!("Could not prune cache dir {}: {e}", dir.display()),
    }
  }
  removed
}

/// Total size of every regular file under `path`. Symlinks are not followed.
pub fn dir_size(path: &Path) -> u64 {
  let Ok(entries) = fs::read_dir(path) else {
    return 0;
  };
  entries
    .flatten()
    .map(|entry| {
      let path = entry.path();
      match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => dir_size(&path),
        Ok(meta) if meta.is_file() => meta.len(),
        _ => 0,
      }
    })
    .sum()
}

fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
  fs::create_dir_all(to)?;
  for entry in fs::read_dir(from)? {
    let entry = entry?;
    let source = entry.path();
    let target = to.join(entry.file_name());
    let meta = fs::symlink_metadata(&source)?;
    if meta.is_dir() {
      copy_dir_recursive(&source, &target)?;
    } else if meta.is_file() {
      fs::copy(&source, &target)?;
    }
  }
  Ok(())
}

/// Move a directory: a rename when both sides share a volume, otherwise a
/// copy followed by removal of the source. A failed copy leaves the source
/// untouched and no half-written target behind.
pub fn move_dir(from: &Path, to: &Path) -> std::io::Result<()> {
  if let Some(parent) = to.parent() {
    fs::create_dir_all(parent)?;
  }
  match fs::rename(from, to) {
    Ok(()) => Ok(()),
    Err(rename_error) => {
      log::info!(
        "Rename of {} failed ({rename_error}); copying instead",
        from.display()
      );
      if let Err(copy_error) = copy_dir_recursive(from, to) {
        let _ = fs::remove_dir_all(to);
        return Err(copy_error);
      }
      fs::remove_dir_all(from)
    }
  }
}

/// Move a profile's directory into the trash and record when it expires.
///
/// `profiles_dir/<id>/` becomes `trash_root/<id>/`; `metadata.json` is
/// replaced by `profile.json` (the struct handed in, with any process id
/// cleared) and `manifest.json` is added. An older trash entry under the same
/// id is dropped: ids survive a restore, so the profile being trashed now is
/// the newer copy.
pub fn trash_profile(
  profiles_dir: &Path,
  trash_root: &Path,
  profile: &BrowserProfile,
  retention_days: u32,
  now: u64,
) -> Result<TrashManifest, String> {
  let id = profile.id.to_string();
  let source_dir = profiles_dir.join(&id);
  let target_dir = trash_root.join(&id);

  fs::create_dir_all(trash_root).map_err(err_internal)?;
  if target_dir.exists() {
    fs::remove_dir_all(&target_dir).map_err(err_internal)?;
  }

  if !profile.password_protected {
    let removed = prune_cache_dirs(&source_dir.join(DATA_DIR));
    if !removed.is_empty() {
      log::info!(
        "Pruned {} cache director{} from profile {id} before trashing",
        removed.len(),
        if removed.len() == 1 { "y" } else { "ies" }
      );
    }
  }

  if source_dir.exists() {
    move_dir(&source_dir, &target_dir).map_err(err_internal)?;
  } else {
    fs::create_dir_all(&target_dir).map_err(err_internal)?;
  }
  let _ = fs::remove_file(target_dir.join(METADATA_FILE));

  let mut stored = profile.clone();
  stored.process_id = None;
  write_json(&target_dir.join(PROFILE_FILE), &stored)?;

  let manifest = TrashManifest {
    deleted_at: now,
    expires_at: now.saturating_add(u64::from(clamp_retention_days(retention_days)) * SECS_PER_DAY),
    size_bytes: dir_size(&target_dir.join(DATA_DIR)),
    original_name: profile.name.clone(),
  };
  write_json(&target_dir.join(MANIFEST_FILE), &manifest)?;
  Ok(manifest)
}

/// Read one entry. `TRASH_ENTRY_NOT_FOUND` when there is no such entry.
pub fn read_entry(
  trash_root: &Path,
  profile_id: &str,
) -> Result<(BrowserProfile, TrashManifest), String> {
  let entry_dir = trash_root.join(profile_id);
  let profile_file = entry_dir.join(PROFILE_FILE);
  let manifest_file = entry_dir.join(MANIFEST_FILE);
  if !profile_file.is_file() || !manifest_file.is_file() {
    return Err(crate::backend_error("TRASH_ENTRY_NOT_FOUND"));
  }
  Ok((read_json(&profile_file)?, read_json(&manifest_file)?))
}

/// Every readable entry, newest deletion first. Unreadable entries are
/// skipped with a warning rather than hiding the whole trash.
pub fn list_entries(trash_root: &Path) -> Vec<(BrowserProfile, TrashManifest)> {
  let Ok(entries) = fs::read_dir(trash_root) else {
    return Vec::new();
  };
  let mut listed: Vec<(BrowserProfile, TrashManifest)> = entries
    .flatten()
    .filter(|entry| entry.path().is_dir())
    .filter_map(|entry| {
      let name = entry.file_name();
      let id = name.to_string_lossy();
      match read_entry(trash_root, &id) {
        Ok(found) => Some(found),
        Err(e) => {
          log::warn!("Skipping unreadable trash entry {id}: {e}");
          None
        }
      }
    })
    .collect();
  listed.sort_by_key(|(_, manifest)| std::cmp::Reverse(manifest.deleted_at));
  listed
}

pub fn summaries(trash_root: &Path) -> Vec<TrashedProfileSummary> {
  list_entries(trash_root)
    .into_iter()
    .map(|(profile, manifest)| TrashedProfileSummary {
      id: profile.id.to_string(),
      name: profile.name,
      browser: profile.browser,
      version: profile.version,
      deleted_at: manifest.deleted_at,
      expires_at: manifest.expires_at,
      size_bytes: manifest.size_bytes,
      group_id: profile.group_id,
      password_protected: profile.password_protected,
    })
    .collect()
}

fn normalized_name(name: &str) -> String {
  name.trim().to_lowercase()
}

/// Pick a name that no live profile carries: the original when it is free,
/// otherwise `name (restored)`, `name (restored 2)`, and so on.
pub fn unique_restored_name(name: &str, taken: &HashSet<String>) -> String {
  if !taken.contains(&normalized_name(name)) {
    return name.to_string();
  }
  let mut attempt = 1u32;
  loop {
    let candidate = if attempt == 1 {
      format!("{name} {RESTORED_SUFFIX}")
    } else {
      format!(
        "{name} {} {attempt})",
        RESTORED_SUFFIX.trim_end_matches(')')
      )
    };
    if !taken.contains(&normalized_name(&candidate)) {
      return candidate;
    }
    attempt += 1;
  }
}

/// Move a trashed profile back under `profiles_dir` and return the profile as
/// it must be saved: same id, identity, proxy and tags; the group only when it
/// still exists; a fresh `updated_at` so it wins any stale sync tombstone.
///
/// `TRASH_RESTORE_CONFLICT` when a live profile already carries the id.
pub fn restore_profile(
  profiles_dir: &Path,
  trash_root: &Path,
  profile_id: &str,
  live_profiles: &[BrowserProfile],
  group_exists: &dyn Fn(&str) -> bool,
  now: u64,
) -> Result<BrowserProfile, String> {
  let (mut profile, _manifest) = read_entry(trash_root, profile_id)?;

  if live_profiles.iter().any(|live| live.id == profile.id) {
    return Err(crate::backend_error("TRASH_RESTORE_CONFLICT"));
  }

  let target_dir = profiles_dir.join(profile_id);
  if target_dir.exists() {
    // Nothing registered lives here (a registered profile has metadata.json
    // and would have been caught above), so this is leftover garbage.
    log::warn!(
      "Removing stale directory {} before restoring profile {profile_id}",
      target_dir.display()
    );
    fs::remove_dir_all(&target_dir).map_err(err_internal)?;
  }

  let taken: HashSet<String> = live_profiles
    .iter()
    .map(|live| normalized_name(&live.name))
    .collect();
  profile.name = unique_restored_name(&profile.name, &taken);
  if let Some(group_id) = profile.group_id.clone() {
    if !group_exists(&group_id) {
      profile.group_id = None;
    }
  }
  profile.process_id = None;
  profile.updated_at = Some(now);

  let entry_dir = trash_root.join(profile_id);
  move_dir(&entry_dir, &target_dir).map_err(err_internal)?;
  let _ = fs::remove_file(target_dir.join(PROFILE_FILE));
  let _ = fs::remove_file(target_dir.join(MANIFEST_FILE));
  write_json(&target_dir.join(METADATA_FILE), &profile)?;
  Ok(profile)
}

/// Destroy one entry for good. `TRASH_ENTRY_NOT_FOUND` when absent.
pub fn purge_entry(trash_root: &Path, profile_id: &str) -> Result<(), String> {
  let entry_dir = trash_root.join(profile_id);
  if !entry_dir.is_dir() {
    return Err(crate::backend_error("TRASH_ENTRY_NOT_FOUND"));
  }
  fs::remove_dir_all(&entry_dir).map_err(err_internal)
}

/// Destroy every entry. Returns the ids that were removed.
pub fn purge_all(trash_root: &Path) -> Result<Vec<String>, String> {
  let ids: Vec<String> = list_entries(trash_root)
    .into_iter()
    .map(|(profile, _)| profile.id.to_string())
    .collect();
  for id in &ids {
    purge_entry(trash_root, id)?;
  }
  Ok(ids)
}

/// Destroy every entry whose expiry has passed. Returns the ids removed.
pub fn purge_expired(trash_root: &Path, now: u64) -> Vec<String> {
  list_entries(trash_root)
    .into_iter()
    .filter(|(_, manifest)| manifest.expires_at <= now)
    .filter_map(|(profile, _)| {
      let id = profile.id.to_string();
      match purge_entry(trash_root, &id) {
        Ok(()) => Some(id),
        Err(e) => {
          log::warn!("Could not purge expired trash entry {id}: {e}");
          None
        }
      }
    })
    .collect()
}

/// A profile whose browser process is alive on this machine cannot be
/// trashed: its data directory is in use. A stale process id (the browser
/// crashed) does not count, and a cross-OS profile can never be running here.
pub fn is_running_locally(profile: &BrowserProfile) -> bool {
  profile
    .process_id
    .is_some_and(crate::proxy_storage::is_process_running)
    && !profile.is_cross_os()
}

fn command_error(e: Box<dyn std::error::Error>, context: &str) -> String {
  let msg = e.to_string();
  if msg.starts_with('{') {
    msg
  } else {
    format!("{context}: {msg}")
  }
}

/// Sweep expired entries now and again every `PURGE_INTERVAL_SECS`.
pub fn start_expiry_sweeper() {
  tauri::async_runtime::spawn(async move {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(PURGE_INTERVAL_SECS));
    loop {
      interval.tick().await;
      let purged = ProfileManager::instance().purge_expired_trash();
      if purged > 0 {
        log::info!(
          "Purged {purged} expired trash entr{}",
          if purged == 1 { "y" } else { "ies" }
        );
      }
    }
  });
}

#[tauri::command]
pub fn list_trashed_profiles() -> Result<Vec<TrashedProfileSummary>, String> {
  Ok(summaries(&trash_dir()))
}

#[tauri::command]
pub async fn restore_trashed_profile(
  app_handle: tauri::AppHandle,
  profile_id: String,
) -> Result<BrowserProfile, String> {
  let manager = ProfileManager::instance();
  let mut profile = manager
    .restore_trashed_profile(&profile_id)
    .map_err(|e| command_error(e, "Failed to restore profile"))?;

  if profile.is_sync_enabled() {
    // The cloud saw a delete (a tombstone was written when the profile was
    // trashed). Re-enabling through the normal path clears that tombstone
    // and queues the re-upload. When that path refuses (sync no longer
    // configured, a cross-OS copy), sync is switched off on the restored
    // profile so the next reconcile keeps the local copy instead of
    // honouring the tombstone.
    let mode = if profile.is_encrypted_sync() {
      "Encrypted"
    } else {
      "Regular"
    };
    if let Err(e) =
      crate::sync::set_profile_sync_mode(app_handle.clone(), profile_id.clone(), mode.to_string())
        .await
    {
      log::warn!("Restored profile {profile_id} could not re-enable sync ({e}); leaving sync off");
      profile.sync_mode = crate::profile::types::SyncMode::Disabled;
      manager
        .save_profile(&profile)
        .map_err(|e| command_error(e, "Failed to save restored profile"))?;
      let _ = crate::events::emit_empty("profiles-changed");
    }
  }

  Ok(profile)
}

#[tauri::command]
pub fn purge_trashed_profile(profile_id: String) -> Result<(), String> {
  ProfileManager::instance()
    .purge_trashed_profile(&profile_id)
    .map_err(|e| command_error(e, "Failed to delete trashed profile"))
}

#[tauri::command]
pub fn empty_trash() -> Result<usize, String> {
  ProfileManager::instance()
    .empty_trash()
    .map_err(|e| command_error(e, "Failed to empty trash"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::wayfern_manager::WayfernConfig;
  use tempfile::TempDir;

  const NOW: u64 = 1_700_000_000;

  fn sample_profile(name: &str) -> BrowserProfile {
    BrowserProfile {
      id: uuid::Uuid::new_v4(),
      name: name.to_string(),
      browser: "wayfern".to_string(),
      version: "150.0.7871.100".to_string(),
      proxy_id: Some("proxy-1".to_string()),
      group_id: Some("group-1".to_string()),
      tags: vec!["shop".to_string(), "eu".to_string()],
      release_type: "stable".to_string(),
      wayfern_config: Some(WayfernConfig {
        identity_id: Some("identity-42".to_string()),
        identity_overrides: Some(r#"{"userAgent":"custom"}"#.to_string()),
        location: Some(r#"{"timezone":"Europe/Berlin"}"#.to_string()),
        ..WayfernConfig::default()
      }),
      updated_at: Some(NOW - 1000),
      ..BrowserProfile::default()
    }
  }

  /// Lay out `profiles/<id>/{metadata.json, profile/...}` the way the app does.
  fn seed_profile(root: &Path, profile: &BrowserProfile, with_caches: bool) -> PathBuf {
    let profiles_dir = root.join("profiles");
    let uuid_dir = profiles_dir.join(profile.id.to_string());
    let data_dir = uuid_dir.join("profile");
    fs::create_dir_all(data_dir.join("Default")).unwrap();
    fs::write(data_dir.join("Default").join("Cookies"), b"cookie-db").unwrap();
    fs::write(data_dir.join("Local State"), b"{}").unwrap();
    if with_caches {
      for relative in CACHE_DIRS {
        let dir = data_dir.join(relative);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("blob"), vec![0u8; 512]).unwrap();
      }
    }
    fs::write(
      uuid_dir.join("metadata.json"),
      serde_json::to_string_pretty(profile).unwrap(),
    )
    .unwrap();
    profiles_dir
  }

  fn group_exists(_: &str) -> bool {
    true
  }

  #[test]
  fn trash_and_restore_round_trip_keeps_identity_and_data() {
    let root = TempDir::new().unwrap();
    let profile = sample_profile("Shop Account");
    let profiles_dir = seed_profile(root.path(), &profile, true);
    let trash_root = root.path().join("trash");

    let manifest = trash_profile(&profiles_dir, &trash_root, &profile, 30, NOW).unwrap();
    assert_eq!(manifest.deleted_at, NOW);
    assert_eq!(manifest.expires_at, NOW + 30 * SECS_PER_DAY);
    assert_eq!(manifest.original_name, "Shop Account");
    assert!(manifest.size_bytes > 0);

    let uuid_dir = profiles_dir.join(profile.id.to_string());
    assert!(!uuid_dir.exists(), "the live directory must be gone");
    let entry_dir = trash_root.join(profile.id.to_string());
    assert!(entry_dir.join("profile.json").is_file());
    assert!(entry_dir.join("manifest.json").is_file());
    assert!(!entry_dir.join("metadata.json").exists());
    assert_eq!(
      fs::read(entry_dir.join("profile").join("Default").join("Cookies")).unwrap(),
      b"cookie-db"
    );
    for relative in CACHE_DIRS {
      assert!(
        !entry_dir.join("profile").join(relative).exists(),
        "{relative} must be pruned before the move"
      );
    }

    let listed = summaries(&trash_root);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, profile.id.to_string());
    assert_eq!(listed[0].name, "Shop Account");
    assert_eq!(listed[0].group_id.as_deref(), Some("group-1"));
    assert!(!listed[0].password_protected);

    let restored = restore_profile(
      &profiles_dir,
      &trash_root,
      &profile.id.to_string(),
      &[],
      &group_exists,
      NOW + 60,
    )
    .unwrap();
    assert_eq!(restored.id, profile.id);
    assert_eq!(restored.name, "Shop Account");
    assert_eq!(restored.proxy_id.as_deref(), Some("proxy-1"));
    assert_eq!(restored.group_id.as_deref(), Some("group-1"));
    assert_eq!(restored.tags, vec!["shop", "eu"]);
    assert_eq!(restored.updated_at, Some(NOW + 60));
    let config = restored.wayfern_config.as_ref().unwrap();
    assert_eq!(config.identity_id.as_deref(), Some("identity-42"));
    assert_eq!(
      config.identity_overrides.as_deref(),
      Some(r#"{"userAgent":"custom"}"#)
    );
    assert_eq!(
      config.location.as_deref(),
      Some(r#"{"timezone":"Europe/Berlin"}"#)
    );

    assert!(!entry_dir.exists(), "the trash entry must be gone");
    assert!(uuid_dir.join("metadata.json").is_file());
    assert!(!uuid_dir.join("profile.json").exists());
    assert!(!uuid_dir.join("manifest.json").exists());
    assert_eq!(
      fs::read(uuid_dir.join("profile").join("Default").join("Cookies")).unwrap(),
      b"cookie-db"
    );
    let on_disk: BrowserProfile =
      serde_json::from_str(&fs::read_to_string(uuid_dir.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(on_disk.id, profile.id);
    assert_eq!(on_disk.updated_at, Some(NOW + 60));
    assert!(summaries(&trash_root).is_empty());
  }

  #[test]
  fn restore_appends_suffix_when_a_live_profile_has_the_name() {
    let root = TempDir::new().unwrap();
    let profile = sample_profile("Shop Account");
    let profiles_dir = seed_profile(root.path(), &profile, false);
    let trash_root = root.path().join("trash");
    trash_profile(&profiles_dir, &trash_root, &profile, 7, NOW).unwrap();

    let mut twin = sample_profile("shop account");
    twin.id = uuid::Uuid::new_v4();
    let mut second_twin = sample_profile("Shop Account (restored)");
    second_twin.id = uuid::Uuid::new_v4();

    let restored = restore_profile(
      &profiles_dir,
      &trash_root,
      &profile.id.to_string(),
      &[twin, second_twin],
      &group_exists,
      NOW,
    )
    .unwrap();
    assert_eq!(restored.name, "Shop Account (restored 2)");
    assert_eq!(restored.id, profile.id);
  }

  #[test]
  fn unique_restored_name_prefers_the_original() {
    let taken: HashSet<String> = ["other".to_string()].into_iter().collect();
    assert_eq!(unique_restored_name("Mine", &taken), "Mine");
    let taken: HashSet<String> = ["mine".to_string()].into_iter().collect();
    assert_eq!(unique_restored_name("Mine", &taken), "Mine (restored)");
  }

  #[test]
  fn restore_refuses_when_a_live_profile_has_the_same_id() {
    let root = TempDir::new().unwrap();
    let profile = sample_profile("Shop Account");
    let profiles_dir = seed_profile(root.path(), &profile, false);
    let trash_root = root.path().join("trash");
    trash_profile(&profiles_dir, &trash_root, &profile, 7, NOW).unwrap();

    let err = restore_profile(
      &profiles_dir,
      &trash_root,
      &profile.id.to_string(),
      std::slice::from_ref(&profile),
      &group_exists,
      NOW,
    )
    .unwrap_err();
    assert!(err.contains("TRASH_RESTORE_CONFLICT"), "{err}");
    assert_eq!(summaries(&trash_root).len(), 1, "the entry must survive");
  }

  #[test]
  fn restore_and_purge_of_a_missing_entry_report_not_found() {
    let root = TempDir::new().unwrap();
    let trash_root = root.path().join("trash");
    let err = restore_profile(
      &root.path().join("profiles"),
      &trash_root,
      "does-not-exist",
      &[],
      &group_exists,
      NOW,
    )
    .unwrap_err();
    assert!(err.contains("TRASH_ENTRY_NOT_FOUND"), "{err}");
    let err = purge_entry(&trash_root, "does-not-exist").unwrap_err();
    assert!(err.contains("TRASH_ENTRY_NOT_FOUND"), "{err}");
  }

  #[test]
  fn restore_clears_the_group_when_it_no_longer_exists() {
    let root = TempDir::new().unwrap();
    let profile = sample_profile("Grouped");
    let profiles_dir = seed_profile(root.path(), &profile, false);
    let trash_root = root.path().join("trash");
    trash_profile(&profiles_dir, &trash_root, &profile, 7, NOW).unwrap();

    let restored = restore_profile(
      &profiles_dir,
      &trash_root,
      &profile.id.to_string(),
      &[],
      &|_| false,
      NOW,
    )
    .unwrap();
    assert_eq!(restored.group_id, None);
    assert_eq!(restored.proxy_id.as_deref(), Some("proxy-1"));
  }

  #[test]
  fn password_protected_entry_is_moved_as_is() {
    let root = TempDir::new().unwrap();
    let mut profile = sample_profile("Vault");
    profile.password_protected = true;
    profile.encryption_salt = Some("salt".to_string());
    let profiles_dir = seed_profile(root.path(), &profile, true);
    let trash_root = root.path().join("trash");
    trash_profile(&profiles_dir, &trash_root, &profile, 7, NOW).unwrap();

    let entry_dir = trash_root.join(profile.id.to_string());
    for relative in CACHE_DIRS {
      assert!(
        entry_dir.join("profile").join(relative).exists(),
        "an encrypted tree is never pruned ({relative})"
      );
    }
    assert!(summaries(&trash_root)[0].password_protected);

    let restored = restore_profile(
      &profiles_dir,
      &trash_root,
      &profile.id.to_string(),
      &[],
      &group_exists,
      NOW,
    )
    .unwrap();
    assert!(restored.password_protected);
    assert_eq!(restored.encryption_salt.as_deref(), Some("salt"));
  }

  #[test]
  fn expiry_purge_removes_only_expired_entries() {
    let root = TempDir::new().unwrap();
    let old = sample_profile("Old");
    let fresh = sample_profile("Fresh");
    let profiles_dir = seed_profile(root.path(), &old, false);
    seed_profile(root.path(), &fresh, false);
    let trash_root = root.path().join("trash");
    trash_profile(&profiles_dir, &trash_root, &old, 1, NOW).unwrap();
    trash_profile(&profiles_dir, &trash_root, &fresh, 30, NOW).unwrap();
    assert_eq!(summaries(&trash_root).len(), 2);

    assert!(purge_expired(&trash_root, NOW + SECS_PER_DAY - 1).is_empty());
    let purged = purge_expired(&trash_root, NOW + SECS_PER_DAY);
    assert_eq!(purged, vec![old.id.to_string()]);
    let remaining = summaries(&trash_root);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, fresh.id.to_string());
    assert!(!trash_root.join(old.id.to_string()).exists());
  }

  #[test]
  fn retention_is_clamped_to_the_allowed_range() {
    assert_eq!(clamp_retention_days(0), MIN_RETENTION_DAYS);
    assert_eq!(clamp_retention_days(30), 30);
    assert_eq!(clamp_retention_days(10_000), MAX_RETENTION_DAYS);
    let root = TempDir::new().unwrap();
    let profile = sample_profile("Clamped");
    let profiles_dir = seed_profile(root.path(), &profile, false);
    let manifest =
      trash_profile(&profiles_dir, &root.path().join("trash"), &profile, 0, NOW).unwrap();
    assert_eq!(manifest.expires_at, NOW + SECS_PER_DAY);
  }

  #[test]
  fn empty_trash_removes_every_entry() {
    let root = TempDir::new().unwrap();
    let first = sample_profile("First");
    let second = sample_profile("Second");
    let profiles_dir = seed_profile(root.path(), &first, false);
    seed_profile(root.path(), &second, false);
    let trash_root = root.path().join("trash");
    trash_profile(&profiles_dir, &trash_root, &first, 7, NOW).unwrap();
    trash_profile(&profiles_dir, &trash_root, &second, 7, NOW + 1).unwrap();

    let listed = summaries(&trash_root);
    assert_eq!(listed[0].name, "Second", "newest deletion is listed first");
    let mut purged = purge_all(&trash_root).unwrap();
    purged.sort();
    let mut expected = vec![first.id.to_string(), second.id.to_string()];
    expected.sort();
    assert_eq!(purged, expected);
    assert!(summaries(&trash_root).is_empty());
  }

  #[test]
  fn trashing_a_profile_again_replaces_the_older_entry() {
    let root = TempDir::new().unwrap();
    let profile = sample_profile("Twice");
    let profiles_dir = seed_profile(root.path(), &profile, false);
    let trash_root = root.path().join("trash");
    trash_profile(&profiles_dir, &trash_root, &profile, 7, NOW).unwrap();
    restore_profile(
      &profiles_dir,
      &trash_root,
      &profile.id.to_string(),
      &[],
      &group_exists,
      NOW,
    )
    .unwrap();
    let uuid_dir = profiles_dir.join(profile.id.to_string());
    fs::write(uuid_dir.join("profile").join("Local State"), b"newer").unwrap();
    // Simulate a leftover entry that a crash left behind under the same id.
    fs::create_dir_all(trash_root.join(profile.id.to_string())).unwrap();
    fs::write(trash_root.join(profile.id.to_string()).join("stale"), b"x").unwrap();

    trash_profile(&profiles_dir, &trash_root, &profile, 7, NOW + 5).unwrap();
    let entry_dir = trash_root.join(profile.id.to_string());
    assert!(!entry_dir.join("stale").exists());
    assert_eq!(
      fs::read(entry_dir.join("profile").join("Local State")).unwrap(),
      b"newer"
    );
    assert_eq!(summaries(&trash_root).len(), 1);
  }

  #[test]
  fn unreadable_entries_are_skipped_not_fatal() {
    let root = TempDir::new().unwrap();
    let profile = sample_profile("Good");
    let profiles_dir = seed_profile(root.path(), &profile, false);
    let trash_root = root.path().join("trash");
    trash_profile(&profiles_dir, &trash_root, &profile, 7, NOW).unwrap();
    let broken = trash_root.join("broken-entry");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("profile.json"), b"not json").unwrap();
    fs::write(broken.join("manifest.json"), b"{}").unwrap();

    let listed = summaries(&trash_root);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Good");
  }

  #[test]
  fn move_dir_copies_when_a_rename_is_impossible() {
    let root = TempDir::new().unwrap();
    let from = root.path().join("from");
    fs::create_dir_all(from.join("nested")).unwrap();
    fs::write(from.join("nested").join("file"), b"payload").unwrap();
    let to = root.path().join("to");
    copy_dir_recursive(&from, &to).unwrap();
    assert_eq!(
      fs::read(to.join("nested").join("file")).unwrap(),
      b"payload"
    );
    assert_eq!(dir_size(&to), 7);
    move_dir(&from, &root.path().join("moved")).unwrap();
    assert!(!from.exists());
    assert_eq!(
      fs::read(root.path().join("moved").join("nested").join("file")).unwrap(),
      b"payload"
    );
  }

  #[test]
  fn running_check_uses_a_live_process() {
    let mut profile = sample_profile("Running");
    profile.process_id = Some(std::process::id());
    assert!(is_running_locally(&profile));
    // A cross-OS profile can never be running on this machine.
    profile.host_os = Some(if cfg!(target_os = "macos") {
      "linux".to_string()
    } else {
      "macos".to_string()
    });
    assert!(!is_running_locally(&profile));
    let mut idle = sample_profile("Idle");
    idle.process_id = None;
    assert!(!is_running_locally(&idle));
  }
}

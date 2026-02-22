use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::profile::BrowserProfile;

lazy_static::lazy_static! {
  static ref EPHEMERAL_DIRS: Mutex<HashMap<String, PathBuf>> = Mutex::new(HashMap::new());
}

pub fn create_ephemeral_dir(profile_id: &str) -> Result<PathBuf, String> {
  let dir_name = format!("donut-ephemeral-{profile_id}");
  let dir_path = std::env::temp_dir().join(dir_name);

  std::fs::create_dir_all(&dir_path).map_err(|e| format!("Failed to create ephemeral dir: {e}"))?;

  EPHEMERAL_DIRS
    .lock()
    .map_err(|e| format!("Failed to lock ephemeral dirs: {e}"))?
    .insert(profile_id.to_string(), dir_path.clone());

  log::info!(
    "Created ephemeral dir for profile {}: {}",
    profile_id,
    dir_path.display()
  );

  Ok(dir_path)
}

pub fn get_ephemeral_dir(profile_id: &str) -> Option<PathBuf> {
  EPHEMERAL_DIRS.lock().ok()?.get(profile_id).cloned()
}

pub fn remove_ephemeral_dir(profile_id: &str) {
  let dir = EPHEMERAL_DIRS
    .lock()
    .ok()
    .and_then(|mut map| map.remove(profile_id));

  if let Some(dir_path) = dir {
    if dir_path.exists() {
      if let Err(e) = std::fs::remove_dir_all(&dir_path) {
        log::warn!("Failed to remove ephemeral dir {}: {e}", dir_path.display());
      } else {
        log::info!(
          "Removed ephemeral dir for profile {}: {}",
          profile_id,
          dir_path.display()
        );
      }
    }
  }
}

pub fn cleanup_stale_dirs() {
  let temp_dir = std::env::temp_dir();
  let entries = match std::fs::read_dir(&temp_dir) {
    Ok(entries) => entries,
    Err(e) => {
      log::warn!("Failed to read temp dir for ephemeral cleanup: {e}");
      return;
    }
  };

  for entry in entries.flatten() {
    if let Some(name) = entry.file_name().to_str() {
      if name.starts_with("donut-ephemeral-") && entry.path().is_dir() {
        if let Err(e) = std::fs::remove_dir_all(entry.path()) {
          log::warn!(
            "Failed to clean up stale ephemeral dir {}: {e}",
            entry.path().display()
          );
        } else {
          log::info!("Cleaned up stale ephemeral dir: {}", entry.path().display());
        }
      }
    }
  }
}

pub fn get_effective_profile_path(profile: &BrowserProfile, profiles_dir: &Path) -> PathBuf {
  if profile.ephemeral {
    if let Some(dir) = get_ephemeral_dir(&profile.id.to_string()) {
      return dir;
    }
  }
  profile.get_profile_data_path(profiles_dir)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn make_test_profile(id: uuid::Uuid, ephemeral: bool) -> BrowserProfile {
    BrowserProfile {
      id,
      name: "test".to_string(),
      browser: "camoufox".to_string(),
      version: "1.0".to_string(),
      proxy_id: None,
      vpn_id: None,
      process_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      camoufox_config: None,
      wayfern_config: None,
      group_id: None,
      tags: Vec::new(),
      note: None,
      sync_enabled: false,
      last_sync: None,
      host_os: None,
      ephemeral,
    }
  }

  #[test]
  fn test_ephemeral_dir_lifecycle() {
    // Test create, get, effective path, remove, and cleanup all in sequence
    // to avoid race conditions between parallel tests.

    // 1. Create and get
    let profile_id = uuid::Uuid::new_v4();
    let id_str = profile_id.to_string();
    let dir = create_ephemeral_dir(&id_str).unwrap();
    assert!(dir.is_dir());
    assert_eq!(get_ephemeral_dir(&id_str), Some(dir.clone()));

    // 2. Effective path for ephemeral profile returns ephemeral dir
    let ephemeral_profile = make_test_profile(profile_id, true);
    let profiles_dir = std::env::temp_dir().join("test_profiles_ephemeral");
    assert_eq!(
      get_effective_profile_path(&ephemeral_profile, &profiles_dir),
      dir
    );

    // 3. Remove cleans up dir and map entry
    remove_ephemeral_dir(&id_str);
    assert!(!dir.exists());
    assert!(get_ephemeral_dir(&id_str).is_none());

    // 4. Effective path for persistent profile returns normal path
    let persistent_profile = make_test_profile(uuid::Uuid::new_v4(), false);
    let expected = persistent_profile.get_profile_data_path(&profiles_dir);
    assert_eq!(
      get_effective_profile_path(&persistent_profile, &profiles_dir),
      expected
    );

    // 5. Cleanup stale dirs
    let stale_id = uuid::Uuid::new_v4().to_string();
    let stale_dir = std::env::temp_dir().join(format!("donut-ephemeral-{stale_id}"));
    std::fs::create_dir_all(&stale_dir).unwrap();
    assert!(stale_dir.exists());
    cleanup_stale_dirs();
    assert!(!stale_dir.exists());
  }
}

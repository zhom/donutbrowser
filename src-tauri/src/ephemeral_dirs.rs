use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::profile::BrowserProfile;

lazy_static::lazy_static! {
  static ref EPHEMERAL_DIRS: Mutex<HashMap<String, PathBuf>> = Mutex::new(HashMap::new());
}

/// Get or create the RAM-backed base directory for ephemeral profiles.
/// Linux: /dev/shm (always tmpfs). macOS: RAM disk via hdiutil. Windows: imdisk RAM disk.
fn get_ephemeral_base_dir() -> Result<PathBuf, String> {
  #[cfg(target_os = "linux")]
  {
    let base = PathBuf::from("/dev/shm/donut-ephemeral");
    std::fs::create_dir_all(&base)
      .map_err(|e| format!("Failed to create ephemeral base in /dev/shm: {e}"))?;
    return Ok(base);
  }

  #[cfg(not(target_os = "linux"))]
  {
    #[cfg(target_os = "macos")]
    {
      if let Ok(mount) = get_or_create_macos_ramdisk() {
        return Ok(mount);
      }
      log::warn!("Failed to create macOS RAM disk, ephemeral profiles may use disk");
    }

    #[cfg(target_os = "windows")]
    {
      if let Ok(mount) = get_or_create_windows_ramdisk() {
        return Ok(mount);
      }
      log::warn!("Failed to create Windows RAM disk, ephemeral profiles may use disk");
    }

    // Fallback
    let base = std::env::temp_dir().join("donut-ephemeral");
    std::fs::create_dir_all(&base)
      .map_err(|e| format!("Failed to create ephemeral base dir: {e}"))?;
    Ok(base)
  }
}

#[cfg(target_os = "macos")]
fn get_or_create_macos_ramdisk() -> Result<PathBuf, String> {
  let mount_point = PathBuf::from("/Volumes/DonutEphemeral");

  // Reuse existing RAM disk from a previous session
  if mount_point.exists() && mount_point.is_dir() {
    return Ok(mount_point);
  }

  // 256 MB in 512-byte sectors
  let sectors = 256 * 2048;
  let output = std::process::Command::new("hdiutil")
    .args(["attach", "-nomount", &format!("ram://{sectors}")])
    .output()
    .map_err(|e| format!("hdiutil attach failed: {e}"))?;

  if !output.status.success() {
    return Err(format!(
      "hdiutil attach failed: {}",
      String::from_utf8_lossy(&output.stderr)
    ));
  }

  let dev = String::from_utf8_lossy(&output.stdout).trim().to_string();

  let fmt = std::process::Command::new("diskutil")
    .args(["erasevolume", "HFS+", "DonutEphemeral", &dev])
    .output()
    .map_err(|e| format!("diskutil erasevolume failed: {e}"))?;

  if !fmt.status.success() {
    let _ = std::process::Command::new("hdiutil")
      .args(["detach", &dev])
      .output();
    return Err(format!(
      "diskutil erasevolume failed: {}",
      String::from_utf8_lossy(&fmt.stderr)
    ));
  }

  log::info!("Created macOS RAM disk at {}", mount_point.display());
  Ok(mount_point)
}

#[cfg(target_os = "windows")]
fn get_or_create_windows_ramdisk() -> Result<PathBuf, String> {
  // Check if a previous RAM disk with our directory already exists
  for letter in ['R', 'Q', 'P', 'O'] {
    let base = PathBuf::from(format!("{}:\\DonutEphemeral", letter));
    if base.exists() && base.is_dir() {
      return Ok(base);
    }
  }

  // Try to create a RAM disk using imdisk (open-source RAM disk driver)
  for letter in ['R', 'Q', 'P', 'O'] {
    let drive = format!("{}:", letter);
    if PathBuf::from(format!("{}\\", drive)).exists() {
      continue;
    }

    let output = std::process::Command::new("imdisk")
      .args(["-a", "-s", "256M", "-m", &drive, "-p", "/fs:ntfs /q /y"])
      .output();

    match output {
      Ok(out) if out.status.success() => {
        let base = PathBuf::from(format!("{}\\DonutEphemeral", drive));
        std::fs::create_dir_all(&base)
          .map_err(|e| format!("Failed to create dir on RAM disk: {e}"))?;
        log::info!("Created Windows RAM disk at {}", base.display());
        return Ok(base);
      }
      Ok(out) => {
        log::debug!(
          "imdisk failed for drive {}: {}",
          drive,
          String::from_utf8_lossy(&out.stderr)
        );
      }
      Err(e) => {
        return Err(format!("imdisk not available: {e}"));
      }
    }
  }

  Err("Could not create Windows RAM disk".to_string())
}

pub fn create_ephemeral_dir(profile_id: &str) -> Result<PathBuf, String> {
  let base = get_ephemeral_base_dir()?;
  let dir_path = base.join(profile_id);

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

/// Recover ephemeral dir mappings on startup by scanning the RAM-backed base dir.
/// Dir names are profile UUIDs, so we re-populate the in-memory HashMap.
/// Also cleans up old disk-based dirs from previous versions.
pub fn recover_ephemeral_dirs() {
  cleanup_legacy_dirs();

  let base = match get_ephemeral_base_dir() {
    Ok(base) => base,
    Err(e) => {
      log::warn!("Cannot recover ephemeral dirs: {e}");
      return;
    }
  };

  let entries = match std::fs::read_dir(&base) {
    Ok(entries) => entries,
    Err(_) => return,
  };

  let mut dirs = match EPHEMERAL_DIRS.lock() {
    Ok(dirs) => dirs,
    Err(_) => return,
  };

  for entry in entries.flatten() {
    if entry.path().is_dir() {
      if let Some(name) = entry.file_name().to_str() {
        if uuid::Uuid::parse_str(name).is_ok() {
          dirs.insert(name.to_string(), entry.path());
          log::info!("Recovered ephemeral dir for profile {}", name);
        }
      }
    }
  }
}

/// Remove old-format ephemeral dirs from /tmp (pre-tmpfs migration).
fn cleanup_legacy_dirs() {
  let temp_dir = std::env::temp_dir();
  let entries = match std::fs::read_dir(&temp_dir) {
    Ok(entries) => entries,
    Err(_) => return,
  };

  for entry in entries.flatten() {
    if let Some(name) = entry.file_name().to_str() {
      if name.starts_with("donut-ephemeral-") && entry.path().is_dir() {
        if let Err(e) = std::fs::remove_dir_all(entry.path()) {
          log::warn!("Failed to clean up legacy ephemeral dir: {e}");
        } else {
          log::info!(
            "Cleaned up legacy ephemeral dir: {}",
            entry.path().display()
          );
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
      sync_mode: crate::profile::types::SyncMode::Disabled,
      encryption_salt: None,
      last_sync: None,
      host_os: None,
      ephemeral,
    }
  }

  #[test]
  fn test_ephemeral_dir_lifecycle() {
    let profile_id = uuid::Uuid::new_v4();
    let id_str = profile_id.to_string();

    let dir = create_ephemeral_dir(&id_str).unwrap();
    assert!(dir.is_dir());
    assert_eq!(get_ephemeral_dir(&id_str), Some(dir.clone()));

    let ephemeral_profile = make_test_profile(profile_id, true);
    let profiles_dir = std::env::temp_dir().join("test_profiles_ephemeral");
    assert_eq!(
      get_effective_profile_path(&ephemeral_profile, &profiles_dir),
      dir
    );

    remove_ephemeral_dir(&id_str);
    assert!(!dir.exists());
    assert!(get_ephemeral_dir(&id_str).is_none());

    let persistent_profile = make_test_profile(uuid::Uuid::new_v4(), false);
    let expected = persistent_profile.get_profile_data_path(&profiles_dir);
    assert_eq!(
      get_effective_profile_path(&persistent_profile, &profiles_dir),
      expected
    );
  }

  #[test]
  fn test_recover_ephemeral_dirs() {
    let base = get_ephemeral_base_dir().unwrap();
    let test_id = uuid::Uuid::new_v4().to_string();
    let test_dir = base.join(&test_id);
    std::fs::create_dir_all(&test_dir).unwrap();

    // Clear the HashMap so recovery has something to find
    EPHEMERAL_DIRS.lock().unwrap().remove(&test_id);
    assert!(get_ephemeral_dir(&test_id).is_none());

    recover_ephemeral_dirs();
    assert_eq!(get_ephemeral_dir(&test_id), Some(test_dir.clone()));

    // Clean up
    remove_ephemeral_dir(&test_id);
  }
}

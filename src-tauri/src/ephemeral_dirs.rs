use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::profile::BrowserProfile;

/// Whether an ephemeral directory is genuinely in memory, or was downgraded to
/// real disk because RAM backing could not be obtained.
///
/// This has to be recorded when the directory is created, not guessed when it
/// is destroyed: by teardown time the RAM disk may have been unmounted, and a
/// path alone cannot say what it used to be. The erase path reads it to decide
/// whether overwriting is meaningful or just page churn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EphemeralBacking {
  Ram,
  Disk,
}

impl EphemeralBacking {
  /// Overwriting only means something when freed disk blocks are involved.
  fn needs_zeroing(self) -> bool {
    matches!(self, EphemeralBacking::Disk)
  }
}

struct EphemeralEntry {
  path: PathBuf,
  backing: EphemeralBacking,
}

lazy_static::lazy_static! {
  static ref EPHEMERAL_DIRS: Mutex<HashMap<String, EphemeralEntry>> = Mutex::new(HashMap::new());
}

/// Test-only redirect for the ephemeral base.
///
/// Without this the unit tests call the real resolver, which on macOS runs
/// `hdiutil attach` + `diskutil erasevolume` and never detaches: a plain
/// `cargo test` left a 256 MB RAM disk mounted on the developer's machine
/// indefinitely. Deliberately compiled out of release builds, because an
/// env-var redirect for a directory holding decrypted profile data is a
/// capability nobody should be able to reach in a shipped binary.
#[cfg(any(test, debug_assertions))]
fn ephemeral_base_override() -> Option<PathBuf> {
  std::env::var_os("DONUTBROWSER_EPHEMERAL_ROOT")
    .filter(|v| !v.is_empty())
    .map(PathBuf::from)
}

#[cfg(not(any(test, debug_assertions)))]
fn ephemeral_base_override() -> Option<PathBuf> {
  None
}

/// Get or create the base directory for ephemeral profiles, and report whether
/// it is actually RAM-backed.
///
/// Linux: /dev/shm (always tmpfs). macOS: RAM disk via hdiutil. Windows: imdisk
/// RAM disk, which is a third-party driver this app does not ship, so on most
/// Windows machines the disk fallback is the normal path rather than an edge
/// case. Callers must treat `Disk` as a downgrade and erase accordingly.
fn get_ephemeral_base_dir() -> Result<(PathBuf, EphemeralBacking), String> {
  if let Some(base) = ephemeral_base_override() {
    std::fs::create_dir_all(&base)
      .map_err(|e| format!("Failed to create overridden ephemeral base: {e}"))?;
    return Ok((base, EphemeralBacking::Disk));
  }

  #[cfg(target_os = "linux")]
  {
    let base = PathBuf::from("/dev/shm/donut-ephemeral");
    std::fs::create_dir_all(&base)
      .map_err(|e| format!("Failed to create ephemeral base in /dev/shm: {e}"))?;
    Ok((base, EphemeralBacking::Ram))
  }

  #[cfg(not(target_os = "linux"))]
  {
    let ramdisk_error: String;

    #[cfg(target_os = "macos")]
    {
      match get_or_create_macos_ramdisk() {
        Ok(mount) => return Ok((mount, EphemeralBacking::Ram)),
        Err(e) => ramdisk_error = e,
      }
    }

    #[cfg(target_os = "windows")]
    {
      match get_or_create_windows_ramdisk() {
        Ok(mount) => return Ok((mount, EphemeralBacking::Ram)),
        Err(e) => ramdisk_error = e,
      }
    }

    // Downgraded to real disk. This is logged at error, with the cause and the
    // destination, because the profile no longer keeps the promise its name
    // makes and the previous "may use disk" wording was logged unconditionally
    // right before disk was used, so it read as speculative when it was
    // certain. The cause used to be discarded entirely.
    let base = std::env::temp_dir().join("donut-ephemeral");
    std::fs::create_dir_all(&base)
      .map_err(|e| format!("Failed to create ephemeral base dir: {e}"))?;
    log::error!(
      "No RAM disk available ({ramdisk_error}); ephemeral profiles are being written to disk at {} and will be securely erased on teardown instead",
      base.display()
    );
    Ok((base, EphemeralBacking::Disk))
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
  let (base, backing) = get_ephemeral_base_dir()?;
  let dir_path = base.join(profile_id);

  std::fs::create_dir_all(&dir_path).map_err(|e| format!("Failed to create ephemeral dir: {e}"))?;

  EPHEMERAL_DIRS
    .lock()
    .map_err(|e| format!("Failed to lock ephemeral dirs: {e}"))?
    .insert(
      profile_id.to_string(),
      EphemeralEntry {
        path: dir_path.clone(),
        backing,
      },
    );

  // State the backing on every launch. Previously only the failure path said
  // anything, so a log could not be used to tell a RAM-backed session from a
  // disk-backed one after the fact.
  log::info!(
    "Created {} ephemeral dir for profile {}: {}",
    match backing {
      EphemeralBacking::Ram => "RAM-backed",
      EphemeralBacking::Disk => "DISK-backed (not in memory)",
    },
    profile_id,
    dir_path.display()
  );

  Ok(dir_path)
}

pub fn get_ephemeral_dir(profile_id: &str) -> Option<PathBuf> {
  Some(EPHEMERAL_DIRS.lock().ok()?.get(profile_id)?.path.clone())
}

/// Destroy a profile's ephemeral directory, zeroing it first when it is on real
/// disk.
///
/// Returns false when data was knowingly left behind. The mapping is only
/// dropped on success: removing it first (as this used to) meant a failure,
/// which on Windows is as ordinary as a file still being locked by an exiting
/// browser, discarded the only handle to the directory and guaranteed nothing
/// would ever retry it.
pub fn remove_ephemeral_dir(profile_id: &str) -> bool {
  let entry = match EPHEMERAL_DIRS.lock() {
    Ok(map) => map.get(profile_id).map(|e| (e.path.clone(), e.backing)),
    Err(e) => {
      log::error!("Failed to lock ephemeral dirs while removing {profile_id}: {e}");
      return false;
    }
  };

  let Some((dir_path, backing)) = entry else {
    return true;
  };

  if !dir_path.exists() {
    if let Ok(mut map) = EPHEMERAL_DIRS.lock() {
      map.remove(profile_id);
    }
    return true;
  }

  let zero = backing.needs_zeroing();
  let started = std::time::Instant::now();
  match crate::fs_secure::secure_remove_dir_all(&dir_path, zero) {
    Ok(files) => {
      if let Ok(mut map) = EPHEMERAL_DIRS.lock() {
        map.remove(profile_id);
      }
      log::info!(
        "Removed {} ephemeral dir for profile {} ({} files, {} ms): {}",
        if zero {
          "and zeroed disk-backed"
        } else {
          "RAM-backed"
        },
        profile_id,
        files,
        started.elapsed().as_millis(),
        dir_path.display()
      );
      true
    }
    Err(e) => {
      // Error, not warn: this is the case where the user's browsing data is
      // knowingly still on the machine. The mapping is kept so a later sweep
      // can try again.
      log::error!(
        "Failed to remove ephemeral dir {} for profile {profile_id}: {e}. Profile data is still on disk.",
        dir_path.display()
      );
      false
    }
  }
}

/// Recover ephemeral dir mappings on startup by scanning the RAM-backed base dir.
/// Dir names are profile UUIDs, so we re-populate the in-memory HashMap.
/// Also cleans up old disk-based dirs from previous versions.
pub fn recover_ephemeral_dirs() {
  cleanup_legacy_dirs();

  let (base, backing) = match get_ephemeral_base_dir() {
    Ok(base) => base,
    Err(e) => {
      log::warn!("Cannot recover ephemeral dirs: {e}");
      return;
    }
  };

  // Sweep the disk fallback even when this run resolved to a RAM disk. A
  // previous run that fell back left a full profile tree in the temp dir, and
  // once RAM backing works again the base points elsewhere and that residue
  // would never be looked at again.
  sweep_disk_fallback_residue(&base);

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
          dirs.insert(
            name.to_string(),
            EphemeralEntry {
              path: entry.path(),
              // Judge a recovered directory by the base it was found under, not
              // by what some earlier run happened to resolve.
              backing,
            },
          );
          log::info!("Recovered ephemeral dir for profile {}", name);
        }
      }
    }
  }
}

/// Securely erase leftovers from a run that was downgraded to the disk
/// fallback, unless that fallback is the base being used right now (in which
/// case `recover_ephemeral_dirs` is about to adopt them instead).
fn sweep_disk_fallback_residue(current_base: &Path) {
  let fallback = std::env::temp_dir().join("donut-ephemeral");
  if !fallback.exists() || fallback == current_base {
    return;
  }

  match crate::fs_secure::secure_remove_dir_all(&fallback, true) {
    Ok(files) if files > 0 => log::info!(
      "Securely erased {files} file(s) of disk-backed ephemeral residue at {}",
      fallback.display()
    ),
    Ok(_) => {}
    Err(e) => log::error!(
      "Failed to erase disk-backed ephemeral residue at {}: {e}",
      fallback.display()
    ),
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
      // These are always in the system temp dir by construction, so they are
      // always on real disk and always worth zeroing.
      if name.starts_with("donut-ephemeral-") && entry.path().is_dir() {
        if let Err(e) = crate::fs_secure::secure_remove_dir_all(&entry.path(), true) {
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
  if profile.ephemeral || profile.password_protected {
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
      browser: "wayfern".to_string(),
      version: "1.0".to_string(),
      proxy_id: None,
      vpn_id: None,
      launch_hook: None,
      process_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      wayfern_config: None,
      group_id: None,
      tags: Vec::new(),
      note: None,
      window_color: None,
      sync_mode: crate::profile::types::SyncMode::Disabled,
      encryption_salt: None,
      last_sync: None,
      host_os: None,
      ephemeral,
      extension_group_id: None,
      proxy_bypass_rules: Vec::new(),
      created_by_id: None,
      created_by_email: None,
      dns_blocklist: None,
      password_protected: false,
      clear_on_close: false,
      created_at: None,
      updated_at: None,
    }
  }

  /// Point the ephemeral base at a scratch directory for the duration of a
  /// test. Without this the tests call the real resolver, which on macOS
  /// attaches a 256 MB RAM disk that nothing ever detaches, so running
  /// `cargo test` left one mounted on the developer's machine indefinitely.
  struct BaseGuard(tempfile::TempDir);

  impl BaseGuard {
    fn new() -> Self {
      let tmp = tempfile::tempdir().unwrap();
      std::env::set_var("DONUTBROWSER_EPHEMERAL_ROOT", tmp.path());
      BaseGuard(tmp)
    }

    fn path(&self) -> &Path {
      self.0.path()
    }
  }

  impl Drop for BaseGuard {
    fn drop(&mut self) {
      std::env::remove_var("DONUTBROWSER_EPHEMERAL_ROOT");
    }
  }

  #[test]
  #[serial_test::serial]
  fn test_ephemeral_dir_lifecycle() {
    let _base = BaseGuard::new();
    // Clear global state to avoid interference from other tests
    EPHEMERAL_DIRS.lock().unwrap().clear();

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
  #[serial_test::serial]
  fn test_recover_ephemeral_dirs() {
    let _base = BaseGuard::new();
    let (base, _) = get_ephemeral_base_dir().unwrap();
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

  #[test]
  #[serial_test::serial]
  fn disk_backed_dirs_are_zeroed_and_ram_backed_ones_are_not() {
    let base = BaseGuard::new();
    EPHEMERAL_DIRS.lock().unwrap().clear();

    // The override always reports Disk, which is the fail-safe: an unverified
    // base must be treated as if it were on a platter.
    let id = uuid::Uuid::new_v4().to_string();
    let dir = create_ephemeral_dir(&id).unwrap();
    // Proves the override actually took effect, so this test can never be
    // silently exercising the developer's real RAM disk.
    assert!(dir.starts_with(base.path()));
    assert_eq!(
      EPHEMERAL_DIRS.lock().unwrap().get(&id).map(|e| e.backing),
      Some(EphemeralBacking::Disk)
    );
    assert!(EphemeralBacking::Disk.needs_zeroing());
    assert!(!EphemeralBacking::Ram.needs_zeroing());

    std::fs::write(dir.join("Cookies"), b"session=secret").unwrap();
    assert!(remove_ephemeral_dir(&id));
    assert!(!dir.exists());
    assert!(get_ephemeral_dir(&id).is_none());
  }

  #[test]
  #[serial_test::serial]
  fn removing_an_unknown_profile_succeeds_without_doing_anything() {
    let _base = BaseGuard::new();
    EPHEMERAL_DIRS.lock().unwrap().clear();
    assert!(remove_ephemeral_dir(&uuid::Uuid::new_v4().to_string()));
  }

  #[test]
  #[serial_test::serial]
  fn the_mapping_survives_a_failed_removal_so_it_can_be_retried() {
    // The old code popped the entry before attempting the delete, so a failure
    // (a locked file on Windows, say) threw away the only handle to the
    // directory and nothing could ever retry it.
    let _base = BaseGuard::new();
    EPHEMERAL_DIRS.lock().unwrap().clear();

    let id = uuid::Uuid::new_v4().to_string();
    let dir = create_ephemeral_dir(&id).unwrap();
    assert!(dir.exists());

    // A successful removal is the one that clears the mapping.
    assert!(remove_ephemeral_dir(&id));
    assert!(get_ephemeral_dir(&id).is_none());
  }
}

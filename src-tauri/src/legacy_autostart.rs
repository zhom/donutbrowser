//! The autostart entry that 0.14 to 0.17 installed for the old tray daemon.
//!
//! 0.18 to 0.24 removed it at launch. 0.25 deleted the daemon and that cleanup
//! with it, so a machine that went from 0.17 or earlier straight to 0.25 or
//! later still carries an entry that points at a binary no longer shipped. On
//! macOS that entry is a LaunchAgent, and Login Items lists Donut as running in
//! the background long after the app has quit.

use std::io;
use std::path::Path;

/// Remove the old daemon's autostart entry, if this machine still has one.
pub fn remove() {
  #[cfg(any(target_os = "macos", target_os = "linux"))]
  {
    #[cfg(target_os = "macos")]
    let entry = dirs::home_dir().map(|home| {
      home
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist"))
    });
    #[cfg(target_os = "linux")]
    let entry = dirs::config_dir().map(|config| config.join("autostart/donut-daemon.desktop"));

    if let Some(path) = entry {
      match remove_entry(&path, unload) {
        Ok(true) => log::info!("Legacy daemon autostart removed path={}", path.display()),
        Ok(false) => {}
        Err(e) => log::warn!(
          "Legacy daemon autostart removal failed path={} err=\"{e}\"",
          path.display()
        ),
      }
    }
  }

  #[cfg(target_os = "windows")]
  {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
    use winreg::RegKey;

    let Ok(run) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
      r"Software\Microsoft\Windows\CurrentVersion\Run",
      KEY_QUERY_VALUE | KEY_SET_VALUE,
    ) else {
      return;
    };
    match run.delete_value("DonutBrowserDaemon") {
      Ok(()) => log::info!("Legacy daemon autostart removed key=HKCU\\Run\\DonutBrowserDaemon"),
      Err(e) if e.kind() == io::ErrorKind::NotFound => {}
      Err(e) => log::warn!(
        "Legacy daemon autostart removal failed key=HKCU\\Run\\DonutBrowserDaemon err=\"{e}\""
      ),
    }
  }
}

#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "com.donutbrowser.daemon";

/// Deleting the plist alone leaves the job loaded until the next login.
#[cfg(target_os = "macos")]
fn unload() {
  let target = format!("gui/{}/{LAUNCH_AGENT_LABEL}", unsafe { libc::getuid() });
  match std::process::Command::new("launchctl")
    .args(["bootout", &target])
    .output()
  {
    Ok(output) if output.status.success() => {
      log::info!("Legacy daemon LaunchAgent unloaded target={target}");
    }
    Ok(_) => {}
    Err(e) => log::warn!("launchctl bootout failed target={target} err=\"{e}\""),
  }
}

#[cfg(target_os = "linux")]
fn unload() {}

/// Delete the entry at `path`, calling `unload` first. `Ok(false)` when there
/// is no entry, in which case `unload` is not called.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn remove_entry(path: &Path, unload: impl FnOnce()) -> io::Result<bool> {
  if path.symlink_metadata().is_err() {
    return Ok(false);
  }
  unload();
  std::fs::remove_file(path)?;
  Ok(true)
}

#[cfg(test)]
mod tests {
  use super::remove_entry;
  use std::cell::Cell;

  #[test]
  fn an_entry_is_unloaded_then_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("com.donutbrowser.daemon.plist");
    std::fs::write(&path, "<plist/>").unwrap();
    let unloaded = Cell::new(false);

    let removed = remove_entry(&path, || {
      assert!(path.exists(), "unload runs while the entry still exists");
      unloaded.set(true);
    })
    .unwrap();

    assert!(removed);
    assert!(unloaded.get());
    assert!(!path.exists());
  }

  #[test]
  fn a_missing_entry_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let unloaded = Cell::new(false);

    let removed = remove_entry(&dir.path().join("donut-daemon.desktop"), || {
      unloaded.set(true)
    })
    .unwrap();

    assert!(!removed);
    assert!(
      !unloaded.get(),
      "nothing is unloaded when there is no entry"
    );
  }
}

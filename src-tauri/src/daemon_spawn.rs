// Daemon Spawn - Start the daemon from the GUI

use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::daemon::autostart;

#[derive(Debug, Deserialize, Default)]
struct DaemonState {
  daemon_pid: Option<u32>,
}

fn get_state_path() -> PathBuf {
  autostart::get_data_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join("daemon-state.json")
}

fn read_state() -> DaemonState {
  let path = get_state_path();
  if path.exists() {
    if let Ok(content) = fs::read_to_string(&path) {
      if let Ok(state) = serde_json::from_str(&content) {
        return state;
      }
    }
  }
  DaemonState::default()
}

pub fn is_daemon_running() -> bool {
  let state = read_state();

  if let Some(pid) = state.daemon_pid {
    #[cfg(unix)]
    {
      unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[cfg(windows)]
    {
      let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .output();
      output
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
      false
    }
  } else {
    false
  }
}

#[cfg(target_os = "macos")]
fn is_dev_mode() -> bool {
  if let Ok(current_exe) = std::env::current_exe() {
    let path_str = current_exe.to_string_lossy();
    path_str.contains("target/debug") || path_str.contains("target/release")
  } else {
    false
  }
}

#[cfg(target_os = "macos")]
fn get_daemon_path() -> Option<PathBuf> {
  // First try to find the daemon binary next to the current executable
  if let Ok(current_exe) = std::env::current_exe() {
    if let Some(exe_dir) = current_exe.parent() {
      let daemon_path = exe_dir.join("donut-daemon");
      if daemon_path.exists() {
        return Some(daemon_path);
      }
    }
  }

  // Try common installation paths
  let paths = [
    PathBuf::from("/Applications/Donut Browser.app/Contents/MacOS/donut-daemon"),
    dirs::home_dir()
      .map(|h| h.join("Applications/Donut Browser.app/Contents/MacOS/donut-daemon"))
      .unwrap_or_default(),
  ];
  paths.into_iter().find(|path| path.exists())
}

#[cfg(any(target_os = "linux", windows))]
fn get_daemon_path() -> Option<PathBuf> {
  // First, try to find it next to the current executable
  if let Ok(current_exe) = std::env::current_exe() {
    let exe_dir = current_exe.parent()?;

    // Check for daemon binary in same directory
    #[cfg(target_os = "windows")]
    let daemon_name = "donut-daemon.exe";
    #[cfg(target_os = "linux")]
    let daemon_name = "donut-daemon";

    let daemon_path = exe_dir.join(daemon_name);
    if daemon_path.exists() {
      return Some(daemon_path);
    }
  }

  // Try to find it in PATH
  #[cfg(target_os = "windows")]
  {
    if let Ok(output) = Command::new("where").arg("donut-daemon").output() {
      if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout);
        let path = path.lines().next()?.trim();
        return Some(PathBuf::from(path));
      }
    }
  }

  #[cfg(target_os = "linux")]
  {
    if let Ok(output) = Command::new("which").arg("donut-daemon").output() {
      if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout);
        let path = path.trim();
        if !path.is_empty() {
          return Some(PathBuf::from(path));
        }
      }
    }
  }

  None
}

pub fn spawn_daemon() -> Result<(), String> {
  // Log the daemon state for debugging
  let state = read_state();
  log::info!("Daemon state before spawn: pid={:?}", state.daemon_pid);

  // Check if already running
  if is_daemon_running() {
    log::info!("Daemon is already running (verified by PID check)");
    return Ok(());
  }

  log::info!("Daemon is not running, attempting to start...");

  // Log current exe location for debugging
  let current_exe = std::env::current_exe().ok();
  log::info!("Current exe: {:?}", current_exe);

  // On macOS, use launchctl to start the daemon via launchd
  // This ensures the daemon runs in the user's Aqua session with WindowServer access
  // and survives app termination since it's managed by launchd, not as a child process
  #[cfg(target_os = "macos")]
  {
    spawn_daemon_macos()?;
  }

  // On Linux, use direct spawn
  #[cfg(target_os = "linux")]
  {
    spawn_daemon_unix()?;
  }

  #[cfg(windows)]
  {
    spawn_daemon_windows()?;
  }

  // Wait for daemon to start (max 3 seconds)
  for i in 0..30 {
    thread::sleep(Duration::from_millis(100));
    if is_daemon_running() {
      log::info!("Daemon started successfully after {}ms", (i + 1) * 100);
      return Ok(());
    }
  }

  // Check if we got a state file at least
  let state = read_state();
  if let Some(pid) = state.daemon_pid {
    log::info!("Daemon appears to have started (PID {} in state file)", pid);
    return Ok(());
  }

  Err("Daemon did not start within timeout".to_string())
}

#[cfg(target_os = "macos")]
fn spawn_daemon_macos() -> Result<(), String> {
  use std::os::unix::process::CommandExt;

  // In dev mode, use direct spawn instead of launchctl
  // This avoids issues with plist paths pointing to wrong binaries
  if is_dev_mode() {
    log::info!("Dev mode detected, using direct spawn instead of launchctl");

    let daemon_path = get_daemon_path().ok_or_else(|| {
      format!(
        "Could not find daemon binary. Current exe: {:?}",
        std::env::current_exe().ok()
      )
    })?;

    log::info!("Spawning daemon from: {:?}", daemon_path);

    // Create a new process group so daemon survives parent exit
    let mut cmd = Command::new(&daemon_path);
    cmd
      .arg("run")
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .process_group(0);

    cmd
      .spawn()
      .map_err(|e| format!("Failed to spawn daemon: {}", e))?;

    return Ok(());
  }

  // Production mode: use launchctl for proper daemon management
  // First, ensure the LaunchAgent plist is installed
  let autostart_enabled = autostart::is_autostart_enabled();
  log::info!("LaunchAgent plist exists: {}", autostart_enabled);

  if !autostart_enabled {
    log::info!("Installing LaunchAgent plist for daemon management");
    autostart::enable_autostart().map_err(|e| format!("Failed to install LaunchAgent: {}", e))?;
    log::info!("LaunchAgent plist installed successfully");
  }

  // Load the launch agent via launchctl
  log::info!("Loading daemon via launchctl...");
  autostart::load_launch_agent().map_err(|e| format!("Failed to load LaunchAgent: {}", e))?;
  log::info!("launchctl load completed");

  // Also explicitly start the agent in case it was already loaded but stopped
  if let Err(e) = autostart::start_launch_agent() {
    log::debug!("launchctl start note (non-fatal): {}", e);
  }

  Ok(())
}

#[cfg(target_os = "linux")]
fn spawn_daemon_unix() -> Result<(), String> {
  use std::os::unix::process::CommandExt;

  let daemon_path = get_daemon_path().ok_or_else(|| {
    format!(
      "Could not find daemon binary. Current exe: {:?}",
      std::env::current_exe().ok()
    )
  })?;

  log::info!("Spawning daemon from: {:?}", daemon_path);

  // Create a new process group so daemon survives parent exit
  let mut cmd = Command::new(&daemon_path);
  cmd
    .arg("run")
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .process_group(0);

  cmd
    .spawn()
    .map_err(|e| format!("Failed to spawn daemon: {}", e))?;

  Ok(())
}

#[cfg(windows)]
fn spawn_daemon_windows() -> Result<(), String> {
  use std::os::windows::process::CommandExt;
  const DETACHED_PROCESS: u32 = 0x00000008;
  const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

  let daemon_path = get_daemon_path().ok_or_else(|| {
    format!(
      "Could not find daemon binary. Current exe: {:?}",
      std::env::current_exe().ok()
    )
  })?;

  log::info!("Spawning daemon from: {:?}", daemon_path);

  Command::new(&daemon_path)
    .arg("run")
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
    .spawn()
    .map_err(|e| format!("Failed to spawn daemon: {}", e))?;

  Ok(())
}

pub fn ensure_daemon_running() -> Result<(), String> {
  if !is_daemon_running() {
    spawn_daemon()?;
  }
  Ok(())
}

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

fn is_daemon_running() -> bool {
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

fn get_daemon_path() -> Option<PathBuf> {
  // First, try to find it next to the current executable
  if let Ok(current_exe) = std::env::current_exe() {
    let exe_dir = current_exe.parent()?;

    // Check for daemon binary in same directory
    #[cfg(target_os = "windows")]
    let daemon_name = "donut-daemon.exe";
    #[cfg(not(target_os = "windows"))]
    let daemon_name = "donut-daemon";

    let daemon_path = exe_dir.join(daemon_name);
    if daemon_path.exists() {
      return Some(daemon_path);
    }

    // On macOS, check inside the app bundle
    #[cfg(target_os = "macos")]
    {
      // If we're in Contents/MacOS, daemon should be there too
      if exe_dir.ends_with("Contents/MacOS") {
        let daemon_path = exe_dir.join(daemon_name);
        if daemon_path.exists() {
          return Some(daemon_path);
        }
      }
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

  #[cfg(unix)]
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
  // Check if already running
  if is_daemon_running() {
    log::info!("Daemon is already running");
    return Ok(());
  }

  // Log current exe location for debugging
  let current_exe = std::env::current_exe().ok();
  log::info!("Current exe: {:?}", current_exe);

  let daemon_path = get_daemon_path().ok_or_else(|| {
    format!(
      "Could not find daemon binary. Current exe: {:?}",
      current_exe
    )
  })?;

  log::info!("Spawning daemon from: {:?}", daemon_path);

  // Use "run" instead of "start" - we handle detachment here
  #[cfg(unix)]
  {
    use std::os::unix::process::CommandExt;

    // Create a new process group so daemon survives parent exit
    // Note: We don't call setsid() because on macOS that disconnects from the WindowServer
    // which prevents the tray icon from appearing. Instead, we just set a new process group.
    let mut cmd = Command::new(&daemon_path);
    cmd
      .arg("run")
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .process_group(0); // Create new process group without new session

    cmd
      .spawn()
      .map_err(|e| format!("Failed to spawn daemon: {}", e))?;
  }

  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

    Command::new(&daemon_path)
      .arg("run")
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
      .spawn()
      .map_err(|e| format!("Failed to spawn daemon: {}", e))?;
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
  if state.daemon_pid.is_some() {
    log::info!(
      "Daemon appears to have started (PID {} in state file)",
      state.daemon_pid.unwrap()
    );
    return Ok(());
  }

  Err("Daemon did not start within timeout".to_string())
}

pub fn ensure_daemon_running() -> Result<(), String> {
  if !is_daemon_running() {
    spawn_daemon()?;
  }
  Ok(())
}

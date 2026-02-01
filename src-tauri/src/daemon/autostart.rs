use directories::ProjectDirs;
use std::fs;
use std::io;
use std::path::PathBuf;

fn get_daemon_path() -> Option<PathBuf> {
  // First try to find the daemon binary in the same directory as the current executable
  if let Ok(current_exe) = std::env::current_exe() {
    let daemon_path = current_exe.parent()?.join(daemon_binary_name());
    if daemon_path.exists() {
      return Some(daemon_path);
    }
  }

  // Try common installation paths
  #[cfg(target_os = "macos")]
  {
    let paths = [
      PathBuf::from("/Applications/Donut Browser.app/Contents/MacOS/donut-daemon"),
      dirs::home_dir()?.join("Applications/Donut Browser.app/Contents/MacOS/donut-daemon"),
    ];
    for path in paths {
      if path.exists() {
        return Some(path);
      }
    }
  }

  #[cfg(target_os = "windows")]
  {
    let paths = [
      dirs::data_local_dir()?.join("Donut Browser/donut-daemon.exe"),
      PathBuf::from("C:\\Program Files\\Donut Browser\\donut-daemon.exe"),
    ];
    for path in paths {
      if path.exists() {
        return Some(path);
      }
    }
  }

  #[cfg(target_os = "linux")]
  {
    let paths = [
      PathBuf::from("/usr/bin/donut-daemon"),
      PathBuf::from("/usr/local/bin/donut-daemon"),
      dirs::home_dir()?.join(".local/bin/donut-daemon"),
    ];
    for path in paths {
      if path.exists() {
        return Some(path);
      }
    }
  }

  None
}

fn daemon_binary_name() -> &'static str {
  #[cfg(windows)]
  {
    "donut-daemon.exe"
  }
  #[cfg(not(windows))]
  {
    "donut-daemon"
  }
}

#[cfg(target_os = "macos")]
pub fn enable_autostart() -> io::Result<()> {
  let daemon_path = get_daemon_path()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Daemon binary not found"))?;

  let plist_dir = dirs::home_dir()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Home directory not found"))?
    .join("Library/LaunchAgents");

  fs::create_dir_all(&plist_dir)?;

  let plist_path = plist_dir.join("com.donutbrowser.daemon.plist");

  // Get log directory (use data directory instead of /tmp)
  let log_dir = get_data_dir()
    .unwrap_or_else(|| PathBuf::from("/tmp"))
    .join("logs");
  fs::create_dir_all(&log_dir)?;

  let plist_content = format!(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.donutbrowser.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{daemon_path}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>StandardOutPath</key>
    <string>{log_dir}/daemon.out.log</string>
    <key>StandardErrorPath</key>
    <string>{log_dir}/daemon.err.log</string>
</dict>
</plist>
"#,
    daemon_path = daemon_path.display(),
    log_dir = log_dir.display()
  );

  fs::write(&plist_path, plist_content)?;

  log::info!("Created launch agent at {:?}", plist_path);
  Ok(())
}

#[cfg(target_os = "macos")]
pub fn get_plist_path() -> Option<PathBuf> {
  dirs::home_dir().map(|h| h.join("Library/LaunchAgents/com.donutbrowser.daemon.plist"))
}

#[cfg(target_os = "macos")]
pub fn disable_autostart() -> io::Result<()> {
  let plist_path = get_plist_path()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Home directory not found"))?;

  if plist_path.exists() {
    // First unload the launch agent if it's loaded
    let _ = unload_launch_agent();
    fs::remove_file(&plist_path)?;
    log::info!("Removed launch agent at {:?}", plist_path);
  }

  Ok(())
}

#[cfg(target_os = "macos")]
pub fn is_autostart_enabled() -> bool {
  get_plist_path().is_some_and(|p| p.exists())
}

#[cfg(target_os = "macos")]
pub fn load_launch_agent() -> io::Result<()> {
  use std::process::Command;

  let plist_path = get_plist_path()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Could not determine plist path"))?;

  if !plist_path.exists() {
    return Err(io::Error::new(
      io::ErrorKind::NotFound,
      "Launch agent plist does not exist",
    ));
  }

  // Use launchctl load to start the daemon via launchd
  // The -w flag writes the "disabled" key to the override plist
  let output = Command::new("launchctl")
    .args(["load", "-w"])
    .arg(&plist_path)
    .output()?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    // "already loaded" is not an error condition for us
    if !stderr.contains("already loaded") {
      return Err(io::Error::other(format!(
        "launchctl load failed: {}",
        stderr
      )));
    }
  }

  log::info!("Loaded launch agent via launchctl");
  Ok(())
}

#[cfg(target_os = "macos")]
pub fn unload_launch_agent() -> io::Result<()> {
  use std::process::Command;

  let plist_path = get_plist_path()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Could not determine plist path"))?;

  if !plist_path.exists() {
    return Ok(());
  }

  let output = Command::new("launchctl")
    .args(["unload"])
    .arg(&plist_path)
    .output()?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Not being loaded is not an error
    if !stderr.contains("Could not find specified service") {
      log::warn!("launchctl unload warning: {}", stderr);
    }
  }

  log::info!("Unloaded launch agent via launchctl");
  Ok(())
}

#[cfg(target_os = "linux")]
pub fn enable_autostart() -> io::Result<()> {
  let daemon_path = get_daemon_path()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Daemon binary not found"))?;

  let autostart_dir = dirs::config_dir()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Config directory not found"))?
    .join("autostart");

  fs::create_dir_all(&autostart_dir)?;

  let desktop_path = autostart_dir.join("donut-daemon.desktop");

  let desktop_content = format!(
    r#"[Desktop Entry]
Type=Application
Name=Donut Browser Daemon
Exec={} start
Hidden=false
NoDisplay=true
X-GNOME-Autostart-enabled=true
"#,
    daemon_path.display()
  );

  fs::write(&desktop_path, desktop_content)?;

  log::info!("Created autostart entry at {:?}", desktop_path);
  Ok(())
}

#[cfg(target_os = "linux")]
pub fn disable_autostart() -> io::Result<()> {
  let desktop_path = dirs::config_dir()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Config directory not found"))?
    .join("autostart/donut-daemon.desktop");

  if desktop_path.exists() {
    fs::remove_file(&desktop_path)?;
    log::info!("Removed autostart entry at {:?}", desktop_path);
  }

  Ok(())
}

#[cfg(target_os = "linux")]
pub fn is_autostart_enabled() -> bool {
  dirs::config_dir()
    .map(|c| c.join("autostart/donut-daemon.desktop").exists())
    .unwrap_or(false)
}

#[cfg(target_os = "windows")]
pub fn enable_autostart() -> io::Result<()> {
  use winreg::enums::HKEY_CURRENT_USER;
  use winreg::RegKey;

  let daemon_path = get_daemon_path()
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Daemon binary not found"))?;

  let hkcu = RegKey::predef(HKEY_CURRENT_USER);
  let (key, _) = hkcu.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;

  key.set_value(
    "DonutBrowserDaemon",
    &format!("\"{}\" start", daemon_path.display()),
  )?;

  log::info!("Added registry autostart entry");
  Ok(())
}

#[cfg(target_os = "windows")]
pub fn disable_autostart() -> io::Result<()> {
  use winreg::enums::HKEY_CURRENT_USER;
  use winreg::RegKey;

  let hkcu = RegKey::predef(HKEY_CURRENT_USER);
  if let Ok(key) = hkcu.open_subkey_with_flags(
    "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
    winreg::enums::KEY_WRITE,
  ) {
    let _ = key.delete_value("DonutBrowserDaemon");
    log::info!("Removed registry autostart entry");
  }

  Ok(())
}

#[cfg(target_os = "windows")]
pub fn is_autostart_enabled() -> bool {
  use winreg::enums::HKEY_CURRENT_USER;
  use winreg::RegKey;

  let hkcu = RegKey::predef(HKEY_CURRENT_USER);
  if let Ok(key) = hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run") {
    key.get_value::<String, _>("DonutBrowserDaemon").is_ok()
  } else {
    false
  }
}

pub fn get_data_dir() -> Option<PathBuf> {
  if let Some(proj_dirs) = ProjectDirs::from("com", "donutbrowser", "Donut Browser") {
    Some(proj_dirs.data_dir().to_path_buf())
  } else {
    dirs::home_dir().map(|h| h.join(".donutbrowser"))
  }
}

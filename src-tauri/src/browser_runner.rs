use crate::proxy_manager::PROXY_MANAGER;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{Pid, System};
use tauri::Emitter;

use crate::browser::{create_browser, BrowserType, ProxySettings};
use crate::browser_version_service::{
  BrowserVersionInfo, BrowserVersionService, BrowserVersionsResult,
};
use crate::camoufox_direct::CamoufoxConfig;
use crate::download::{DownloadProgress, Downloader};
use crate::downloaded_browsers::DownloadedBrowsersRegistry;
use crate::extraction::Extractor;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrowserProfile {
  pub id: uuid::Uuid,
  pub name: String,
  pub browser: String,
  pub version: String,
  #[serde(default)]
  pub proxy_id: Option<String>, // Reference to stored proxy
  #[serde(default)]
  pub process_id: Option<u32>,
  #[serde(default)]
  pub last_launch: Option<u64>,
  #[serde(default = "default_release_type")]
  pub release_type: String, // "stable" or "nightly"
  #[serde(default)]
  pub camoufox_config: Option<CamoufoxConfig>, // Camoufox configuration
  #[serde(default)]
  pub group_id: Option<String>, // Reference to profile group
}

fn default_release_type() -> String {
  "stable".to_string()
}

// Global state to track currently downloading browser-version pairs
lazy_static::lazy_static! {
  static ref DOWNLOADING_BROWSERS: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
}

// Platform-specific modules
#[cfg(target_os = "macos")]
mod macos {
  use super::*;
  use std::ffi::OsString;
  use std::process::Command;

  pub fn is_tor_or_mullvad_browser(exe_name: &str, cmd: &[OsString], browser_type: &str) -> bool {
    match browser_type {
      "mullvad-browser" => {
        let has_mullvad_in_exe = exe_name.contains("mullvad");
        let has_firefox_exe = exe_name == "firefox" || exe_name.contains("firefox-bin");
        let has_mullvad_in_cmd = cmd.iter().any(|arg| {
          let arg_str = arg.to_str().unwrap_or("");
          arg_str.contains("Mullvad Browser.app")
            || arg_str.contains("mullvad")
            || arg_str.contains("Mullvad")
            || arg_str.contains("/Applications/Mullvad Browser.app/")
            || arg_str.contains("MullvadBrowser")
        });

        has_mullvad_in_exe || (has_firefox_exe && has_mullvad_in_cmd)
      }
      "tor-browser" => {
        let has_tor_in_exe = exe_name.contains("tor");
        let has_firefox_exe = exe_name == "firefox" || exe_name.contains("firefox-bin");
        let has_tor_in_cmd = cmd.iter().any(|arg| {
          let arg_str = arg.to_str().unwrap_or("");
          arg_str.contains("Tor Browser.app")
            || arg_str.contains("tor-browser")
            || arg_str.contains("TorBrowser")
            || arg_str.contains("/Applications/Tor Browser.app/")
            || arg_str.contains("TorBrowser-Data")
        });

        has_tor_in_exe || (has_firefox_exe && has_tor_in_cmd)
      }
      _ => false,
    }
  }

  pub async fn launch_browser_process(
    executable_path: &std::path::Path,
    args: &[String],
  ) -> Result<std::process::Child, Box<dyn std::error::Error + Send + Sync>> {
    println!("Launching browser on macOS: {executable_path:?} with args: {args:?}");
    Ok(Command::new(executable_path).args(args).spawn()?)
  }

  pub async fn open_url_in_existing_browser_firefox_like(
    profile: &BrowserProfile,
    url: &str,
    browser_type: BrowserType,
    browser_dir: &Path,
    profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pid = profile.process_id.unwrap();
    let profile_data_path = profile.get_profile_data_path(profiles_dir);

    // First try: Use Firefox remote command
    println!("Trying Firefox remote command for PID: {pid}");
    let browser = create_browser(browser_type);
    if let Ok(executable_path) = browser.get_executable_path(browser_dir) {
      let remote_args = vec![
        "-profile".to_string(),
        profile_data_path.to_string_lossy().to_string(),
        "-new-tab".to_string(),
        url.to_string(),
      ];

      let remote_output = Command::new(executable_path).args(&remote_args).output();

      match remote_output {
        Ok(output) if output.status.success() => {
          println!("Firefox remote command succeeded");
          return Ok(());
        }
        Ok(output) => {
          let stderr = String::from_utf8_lossy(&output.stderr);
          println!(
            "Firefox remote command failed with stderr: {stderr}, trying AppleScript fallback"
          );
        }
        Err(e) => {
          println!("Firefox remote command error: {e}, trying AppleScript fallback");
        }
      }
    }

    // Fallback: Use AppleScript
    let escaped_url = url
      .replace("\"", "\\\"")
      .replace("\\", "\\\\")
      .replace("'", "\\'");

    let script = format!(
      r#"
try
  tell application "System Events"
    -- Find the exact process by PID
    set targetProcess to (first application process whose unix id is {pid})
    
    -- Verify the process exists
    if not (exists targetProcess) then
      error "No process found with PID {pid}"
    end if
    
    -- Get the process name for verification
    set processName to name of targetProcess
    
    -- Bring the process to the front first
    set frontmost of targetProcess to true
    delay 1.0
    
    -- Check if the process has any visible windows
    set windowList to windows of targetProcess
    set hasVisibleWindow to false
    repeat with w in windowList
      if visible of w is true then
        set hasVisibleWindow to true
        exit repeat
      end if
    end repeat
    
    if not hasVisibleWindow then
      -- No visible windows, create a new one
      tell targetProcess
        keystroke "n" using command down
        delay 2.0
      end tell
    end if
    
    -- Ensure the process is frontmost again
    set frontmost of targetProcess to true
    delay 0.5
    
    -- Focus on the address bar and open URL
    tell targetProcess
      -- Open a new tab
      keystroke "t" using command down
      delay 1.5
      
      -- Focus address bar (Cmd+L)
      keystroke "l" using command down
      delay 0.5
      
      -- Type the URL
      keystroke "{escaped_url}"
      delay 0.5
      
      -- Press Enter to navigate
      keystroke return
    end tell
    
    return "Successfully opened URL in " & processName & " (PID: {pid})"
  end tell
on error errMsg number errNum
  return "AppleScript failed: " & errMsg & " (Error " & errNum & ")"
end try
      "#
    );

    println!("Executing AppleScript fallback for Firefox-based browser (PID: {pid})...");
    let output = Command::new("osascript").args(["-e", &script]).output()?;

    if !output.status.success() {
      let error_msg = String::from_utf8_lossy(&output.stderr);
      println!("AppleScript failed: {error_msg}");
      return Err(
        format!(
          "Both Firefox remote command and AppleScript failed. AppleScript error: {error_msg}"
        )
        .into(),
      );
    } else {
      println!("AppleScript succeeded");
    }

    Ok(())
  }

  pub async fn open_url_in_existing_browser_tor_mullvad(
    profile: &BrowserProfile,
    url: &str,
    browser_type: BrowserType,
    browser_dir: &Path,
    _profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pid = profile.process_id.unwrap();

    println!("Opening URL in TOR/Mullvad browser using file-based approach (PID: {pid})");

    // Method 1: Try using a temporary HTML file approach
    println!("Attempting file-based URL opening for TOR/Mullvad browser");

    let temp_dir = std::env::temp_dir();
    let temp_file_name = format!("donut_browser_url_{}.html", std::process::id());
    let temp_file_path = temp_dir.join(&temp_file_name);

    let html_content = format!(
      r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta http-equiv="refresh" content="0; url={url}">
    <title>Redirecting...</title>
    <script>
        window.location.href = "{url}";
    </script>
</head>
<body>
    <p>Redirecting to <a href="{url}">{url}</a>...</p>
</body>
</html>"#
    );

    match std::fs::write(&temp_file_path, html_content) {
      Ok(()) => {
        println!("Created temporary HTML file: {temp_file_path:?}");

        let browser = create_browser(browser_type.clone());
        if let Ok(executable_path) = browser.get_executable_path(browser_dir) {
          let open_result = Command::new("open")
            .args([
              "-a",
              executable_path.to_str().unwrap(),
              temp_file_path.to_str().unwrap(),
            ])
            .output();

          // Clean up the temporary file after a short delay
          let temp_file_path_clone = temp_file_path.clone();
          tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            let _ = std::fs::remove_file(temp_file_path_clone);
          });

          match open_result {
            Ok(output) if output.status.success() => {
              println!("Successfully opened URL using file-based approach");
              return Ok(());
            }
            Ok(output) => {
              let stderr = String::from_utf8_lossy(&output.stderr);
              println!("File-based approach failed: {stderr}");
            }
            Err(e) => {
              println!("File-based approach error: {e}");
            }
          }
        }

        let _ = std::fs::remove_file(&temp_file_path);
      }
      Err(e) => {
        println!("Failed to create temporary HTML file: {e}");
      }
    }

    // Method 2: Try using the 'open' command directly with the URL
    println!("Attempting direct URL opening with 'open' command");

    let browser = create_browser(browser_type.clone());
    if let Ok(executable_path) = browser.get_executable_path(browser_dir) {
      let direct_open_result = Command::new("open")
        .args(["-a", executable_path.to_str().unwrap(), url])
        .output();

      match direct_open_result {
        Ok(output) if output.status.success() => {
          println!("Successfully opened URL using direct 'open' command");
          return Ok(());
        }
        Ok(output) => {
          let stderr = String::from_utf8_lossy(&output.stderr);
          println!("Direct 'open' command failed: {stderr}");
        }
        Err(e) => {
          println!("Direct 'open' command error: {e}");
        }
      }
    }

    // If all methods fail, return a helpful error message
    Err(
      format!(
        "Failed to open URL in existing TOR/Mullvad browser (PID: {pid}). All methods failed:\n\
      1. File-based approach failed\n\
      2. Direct 'open' command failed\n\
      \n\
      This may be due to browser security restrictions or the browser process may have changed.\n\
      Try closing and reopening the browser, or manually paste the URL: {url}"
      )
      .into(),
    )
  }

  pub async fn open_url_in_existing_browser_chromium(
    profile: &BrowserProfile,
    url: &str,
    browser_type: BrowserType,
    browser_dir: &Path,
    _profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pid = profile.process_id.unwrap();

    // First, try using the browser's built-in URL opening capability
    println!("Trying Chromium URL opening for PID: {pid}");

    let browser = create_browser(browser_type);
    if let Ok(executable_path) = browser.get_executable_path(browser_dir) {
      let profile_data_path = profile.get_profile_data_path(_profiles_dir);
      let remote_output = Command::new(executable_path)
        .args([
          &format!("--user-data-dir={}", profile_data_path.to_string_lossy()),
          url,
        ])
        .output();

      match remote_output {
        Ok(output) if output.status.success() => {
          println!("Chromium URL opening succeeded");
          return Ok(());
        }
        Ok(output) => {
          let stderr = String::from_utf8_lossy(&output.stderr);
          println!("Chromium URL opening failed: {stderr}, trying AppleScript");
        }
        Err(e) => {
          println!("Chromium URL opening error: {e}, trying AppleScript");
        }
      }
    }

    // Fallback to AppleScript
    let escaped_url = url
      .replace("\"", "\\\"")
      .replace("\\", "\\\\")
      .replace("'", "\\'");

    let script = format!(
      r#"
try
  tell application "System Events"
    -- Find the exact process by PID
    set targetProcess to (first application process whose unix id is {pid})
    
    -- Verify the process exists
    if not (exists targetProcess) then
      error "No process found with PID {pid}"
    end if
    
    -- Get the process name for verification
    set processName to name of targetProcess
    
    -- Bring the process to the front first
    set frontmost of targetProcess to true
    delay 1.0
    
    -- Check if the process has any visible windows
    set windowList to windows of targetProcess
    set hasVisibleWindow to false
    repeat with w in windowList
      if visible of w is true then
        set hasVisibleWindow to true
        exit repeat
      end if
    end repeat
    
    if not hasVisibleWindow then
      -- No visible windows, create a new one
      tell targetProcess
        keystroke "n" using command down
        delay 2.0
      end tell
    end if
    
    -- Ensure the process is frontmost again
    set frontmost of targetProcess to true
    delay 0.5
    
    -- Focus on the address bar and open URL
    tell targetProcess
      -- Open a new tab
      keystroke "t" using command down
      delay 1.5
      
      -- Focus address bar (Cmd+L)
      keystroke "l" using command down
      delay 0.5
      
      -- Type the URL
      keystroke "{escaped_url}"
      delay 0.5
      
      -- Press Enter to navigate
      keystroke return
    end tell
    
    return "Successfully opened URL in " & processName & " (PID: {pid})"
  end tell
on error errMsg number errNum
  return "AppleScript failed: " & errMsg & " (Error " & errNum & ")"
end try
      "#
    );

    println!("Executing AppleScript for Chromium-based browser (PID: {pid})...");
    let output = Command::new("osascript").args(["-e", &script]).output()?;

    if !output.status.success() {
      let error_msg = String::from_utf8_lossy(&output.stderr);
      println!("AppleScript failed: {error_msg}");
      return Err(
        format!("Failed to open URL in existing Chromium-based browser: {error_msg}").into(),
      );
    } else {
      println!("AppleScript succeeded");
    }

    Ok(())
  }

  pub async fn kill_browser_process_impl(
    pid: u32,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Attempting to kill browser process with PID: {pid}");

    // First try SIGTERM (graceful shutdown)
    let output = Command::new("kill")
      .args(["-TERM", &pid.to_string()])
      .output()
      .map_err(|e| format!("Failed to execute kill command: {e}"))?;

    if !output.status.success() {
      // If SIGTERM fails, try SIGKILL (force kill)
      let output = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .output()?;

      if !output.status.success() {
        return Err(
          format!(
            "Failed to kill process {}: {}",
            pid,
            String::from_utf8_lossy(&output.stderr)
          )
          .into(),
        );
      }
    }

    println!("Successfully killed browser process with PID: {pid}");
    Ok(())
  }
}

#[cfg(target_os = "windows")]
mod windows {
  use super::*;
  use std::ffi::OsString;
  use std::process::Command;

  pub fn is_tor_or_mullvad_browser(exe_name: &str, cmd: &[OsString], browser_type: &str) -> bool {
    let exe_lower = exe_name.to_lowercase();

    // Check for Firefox-based browsers first by executable name
    let is_firefox_family = exe_lower.contains("firefox") || exe_lower.contains(".exe");

    if !is_firefox_family {
      return false;
    }

    // Check command arguments for profile paths and browser-specific indicators
    let cmd_line = cmd
      .iter()
      .map(|s| s.to_string_lossy().to_lowercase())
      .collect::<Vec<_>>()
      .join(" ");

    match browser_type {
      "tor-browser" => {
        // Check for TOR browser specific paths and arguments
        cmd_line.contains("tor")
          || cmd_line.contains("browser\\torbrowser")
          || cmd_line.contains("tor-browser")
          || cmd_line.contains("profile") && (cmd_line.contains("tor") || cmd_line.contains("tbb"))
      }
      "mullvad-browser" => {
        // Check for Mullvad browser specific paths and arguments
        cmd_line.contains("mullvad")
          || cmd_line.contains("browser\\mullvadbrowser")
          || cmd_line.contains("mullvad-browser")
          || cmd_line.contains("profile") && cmd_line.contains("mullvad")
      }
      _ => false,
    }
  }

  pub async fn launch_browser_process(
    executable_path: &std::path::Path,
    args: &[String],
  ) -> Result<std::process::Child, Box<dyn std::error::Error + Send + Sync>> {
    println!(
      "Launching browser on Windows: {:?} with args: {:?}",
      executable_path, args
    );

    // Check if the executable exists
    if !executable_path.exists() {
      return Err(format!("Browser executable not found: {:?}", executable_path).into());
    }

    // On Windows, set up the command with proper working directory
    let mut cmd = Command::new(executable_path);
    cmd.args(args);

    // Set working directory to the executable's directory for better compatibility
    if let Some(parent_dir) = executable_path.parent() {
      cmd.current_dir(parent_dir);
    }

    // For Windows 7 compatibility, set some environment variables
    cmd.env(
      "PROCESSOR_ARCHITECTURE",
      std::env::var("PROCESSOR_ARCHITECTURE").unwrap_or_else(|_| "x86".to_string()),
    );

    // Ensure proper PATH for DLL loading
    if let Some(exe_dir) = executable_path.parent() {
      let mut path_var = std::env::var("PATH").unwrap_or_default();
      if !path_var.is_empty() {
        path_var = format!("{};{}", exe_dir.display(), path_var);
      } else {
        path_var = exe_dir.display().to_string();
      }
      cmd.env("PATH", path_var);
    }

    // Launch the process
    let child = cmd
      .spawn()
      .map_err(|e| format!("Failed to launch browser process: {}", e))?;

    println!(
      "Successfully launched browser process with PID: {}",
      child.id()
    );
    Ok(child)
  }

  pub async fn open_url_in_existing_browser_firefox_like(
    profile: &BrowserProfile,
    url: &str,
    browser_type: BrowserType,
    browser_dir: &Path,
    profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let browser = create_browser(browser_type);
    let executable_path = browser
      .get_executable_path(browser_dir)
      .map_err(|e| format!("Failed to get executable path: {}", e))?;

    let profile_data_path = profile.get_profile_data_path(profiles_dir);

    // For Windows, try using the -requestPending approach for Firefox
    let mut cmd = Command::new(executable_path);
    cmd.args([
      "-profile",
      &profile_data_path.to_string_lossy(),
      "-requestPending",
      "-new-tab",
      url,
    ]);

    // Set working directory
    if let Some(parent_dir) = browser_dir
      .parent()
      .or_else(|| browser_dir.ancestors().nth(1))
    {
      cmd.current_dir(parent_dir);
    }

    let output = cmd.output()?;

    if !output.status.success() {
      // Fallback: try without -requestPending
      let executable_path = browser
        .get_executable_path(browser_dir)
        .map_err(|e| format!("Failed to get executable path: {}", e))?;
      let mut fallback_cmd = Command::new(executable_path);
      let profile_data_path = profile.get_profile_data_path(profiles_dir);
      fallback_cmd.args([
        "-profile",
        &profile_data_path.to_string_lossy(),
        "-new-tab",
        url,
      ]);

      if let Some(parent_dir) = browser_dir
        .parent()
        .or_else(|| browser_dir.ancestors().nth(1))
      {
        fallback_cmd.current_dir(parent_dir);
      }

      let fallback_output = fallback_cmd.output()?;

      if !fallback_output.status.success() {
        return Err(
          format!(
            "Failed to open URL in existing browser: {}",
            String::from_utf8_lossy(&fallback_output.stderr)
          )
          .into(),
        );
      }
    }

    Ok(())
  }

  pub async fn open_url_in_existing_browser_tor_mullvad(
    profile: &BrowserProfile,
    url: &str,
    browser_type: BrowserType,
    browser_dir: &Path,
    profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // On Windows, TOR and Mullvad browsers can sometimes accept URLs via command line
    // even with -no-remote, by launching a new instance that hands off to existing one
    let browser = create_browser(browser_type.clone());
    let executable_path = browser
      .get_executable_path(browser_dir)
      .map_err(|e| format!("Failed to get executable path: {}", e))?;

    let mut cmd = Command::new(&executable_path);
    let profile_data_path = profile.get_profile_data_path(profiles_dir);
    cmd.args(["-profile", &profile_data_path.to_string_lossy(), url]);

    // Set working directory
    if let Some(parent_dir) = browser_dir
      .parent()
      .or_else(|| browser_dir.ancestors().nth(1))
    {
      cmd.current_dir(parent_dir);
    }

    let output = cmd.output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to open URL in existing {}: {}. Note: TOR and Mullvad browsers may require manual URL opening for security reasons.",
          browser_type.as_str(),
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    Ok(())
  }

  pub async fn open_url_in_existing_browser_chromium(
    profile: &BrowserProfile,
    url: &str,
    browser_type: BrowserType,
    browser_dir: &Path,
    profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let browser = create_browser(browser_type.clone());
    let executable_path = browser
      .get_executable_path(browser_dir)
      .map_err(|e| format!("Failed to get executable path: {}", e))?;

    let mut cmd = Command::new(&executable_path);
    cmd.args([
      &format!(
        "--user-data-dir={}",
        profile
          .get_profile_data_path(profiles_dir)
          .to_string_lossy()
      ),
      "--new-window",
      url,
    ]);

    // Set working directory
    if let Some(parent_dir) = browser_dir
      .parent()
      .or_else(|| browser_dir.ancestors().nth(1))
    {
      cmd.current_dir(parent_dir);
    }

    let output = cmd.output()?;

    if !output.status.success() {
      // Try fallback without --new-window
      let mut fallback_cmd = Command::new(&executable_path);
      fallback_cmd.args([
        &format!(
          "--user-data-dir={}",
          profile
            .get_profile_data_path(profiles_dir)
            .to_string_lossy()
        ),
        url,
      ]);

      if let Some(parent_dir) = browser_dir
        .parent()
        .or_else(|| browser_dir.ancestors().nth(1))
      {
        fallback_cmd.current_dir(parent_dir);
      }

      let fallback_output = fallback_cmd.output()?;

      if !fallback_output.status.success() {
        return Err(
          format!(
            "Failed to open URL in existing Chromium-based browser: {}",
            String::from_utf8_lossy(&fallback_output.stderr)
          )
          .into(),
        );
      }
    }

    Ok(())
  }

  pub async fn kill_browser_process_impl(
    pid: u32,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // First try using sysinfo (cross-platform approach)
    let system = System::new_all();
    if let Some(process) = system.process(Pid::from(pid as usize)) {
      if process.kill() {
        println!("Successfully killed browser process with PID: {pid}");
        return Ok(());
      }
    }

    // Fallback to Windows-specific process termination
    use std::process::Command;

    // Try taskkill command as fallback
    let output = Command::new("taskkill")
      .args(["/F", "/PID", &pid.to_string()])
      .output();

    match output {
      Ok(result) => {
        if result.status.success() {
          println!("Successfully killed browser process with PID: {pid} using taskkill");
          Ok(())
        } else {
          Err(
            format!(
              "Failed to kill process {} with taskkill: {}",
              pid,
              String::from_utf8_lossy(&result.stderr)
            )
            .into(),
          )
        }
      }
      Err(e) => Err(format!("Failed to execute taskkill for process {}: {}", pid, e).into()),
    }
  }
}

#[cfg(target_os = "linux")]
mod linux {
  use super::*;
  use std::ffi::OsString;
  use std::process::Command;

  pub fn is_tor_or_mullvad_browser(
    _exe_name: &str,
    _cmd: &[OsString],
    _browser_type: &str,
  ) -> bool {
    // Linux implementation would go here
    false
  }

  pub async fn launch_browser_process(
    executable_path: &std::path::Path,
    args: &[String],
  ) -> Result<std::process::Child, Box<dyn std::error::Error + Send + Sync>> {
    println!(
      "Launching browser on Linux: {:?} with args: {:?}",
      executable_path, args
    );

    // Check if the executable exists and is executable
    if !executable_path.exists() {
      return Err(format!("Browser executable not found: {:?}", executable_path).into());
    }

    // Check if we can read the executable to detect architecture issues early
    if let Err(e) = std::fs::File::open(executable_path) {
      return Err(format!("Cannot access browser executable: {}", e).into());
    }

    // Ensure the executable has proper permissions
    if let Err(e) = std::fs::metadata(executable_path) {
      return Err(format!("Cannot get executable metadata: {}", e).into());
    }

    // On Linux, we might need to set LD_LIBRARY_PATH for some browsers
    let mut cmd = Command::new(executable_path);
    cmd.args(args);

    // For Firefox-based browsers, ensure library path includes the installation directory
    if let Some(install_dir) = executable_path.parent() {
      let mut ld_library_path = Vec::new();

      // Add multiple potential library directories
      let lib_dirs = [
        install_dir.join("lib"),
        install_dir.join("../lib"),    // Parent directory lib
        install_dir.join("../../lib"), // Grandparent directory lib
        install_dir.to_path_buf(),     // Installation directory itself
      ];

      for lib_dir in &lib_dirs {
        if lib_dir.exists() {
          ld_library_path.push(lib_dir.to_string_lossy().to_string());
        }
      }

      // For Firefox specifically, add common system library paths that might be needed
      let firefox_lib_paths = [
        "/usr/lib/firefox",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/lib/x86_64-linux-gnu",
        "/lib/aarch64-linux-gnu",
      ];

      for lib_path in &firefox_lib_paths {
        let path = std::path::Path::new(lib_path);
        if path.exists() {
          ld_library_path.push(lib_path.to_string());
        }
      }

      // Preserve existing LD_LIBRARY_PATH
      if let Ok(existing_path) = std::env::var("LD_LIBRARY_PATH") {
        ld_library_path.push(existing_path);
      }

      // Set the combined LD_LIBRARY_PATH
      if !ld_library_path.is_empty() {
        cmd.env("LD_LIBRARY_PATH", ld_library_path.join(":"));
        println!("Set LD_LIBRARY_PATH to: {}", ld_library_path.join(":"));
      }
    }

    // Additional Linux-specific environment variables for better compatibility
    cmd.env(
      "DISPLAY",
      std::env::var("DISPLAY").unwrap_or(":0".to_string()),
    );

    // Set MOZ_ENABLE_WAYLAND for better Wayland support
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
      cmd.env("MOZ_ENABLE_WAYLAND", "1");
    }

    // Disable GPU acceleration if running in headless environments
    if std::env::var("DISPLAY").is_err() || std::env::var("WAYLAND_DISPLAY").is_err() {
      println!("No display detected, browser may fail to start");
    }

    // Attempt to spawn with better error handling for architecture issues
    match cmd.spawn() {
      Ok(child) => Ok(child),
      Err(e) => {
        // Detect architecture mismatch errors
        if e.kind() == std::io::ErrorKind::Other {
          let error_msg = e.to_string();
          if error_msg.contains("Exec format error") {
            return Err(format!(
              "Architecture mismatch: The browser executable is not compatible with your system architecture ({}). \
              This typically happens when trying to run x86_64 binaries on ARM64 systems. \
              Please use a browser that supports your architecture, such as Zen Browser or Brave. \
              Executable: {:?}",
              std::env::consts::ARCH,
              executable_path
            ).into());
          } else if error_msg.contains("No such file or directory") {
            return Err(format!(
              "Executable or required library not found. This might be due to missing dependencies or incorrect executable path. \
              Try installing missing libraries or verify the browser installation. \
              Executable: {:?}, Error: {}",
              executable_path, error_msg
            ).into());
          }
        }
        Err(format!("Failed to launch browser: {}", e).into())
      }
    }
  }

  pub async fn open_url_in_existing_browser_firefox_like(
    profile: &BrowserProfile,
    url: &str,
    browser_type: BrowserType,
    browser_dir: &Path,
    profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let browser = create_browser(browser_type);
    let executable_path = browser
      .get_executable_path(browser_dir)
      .map_err(|e| format!("Failed to get executable path: {}", e))?;

    let profile_data_path = profile.get_profile_data_path(profiles_dir);
    let output = Command::new(executable_path)
      .args([
        "-profile",
        &profile_data_path.to_string_lossy(),
        "-new-tab",
        url,
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to open URL in existing browser: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    Ok(())
  }

  pub async fn open_url_in_existing_browser_tor_mullvad(
    _profile: &BrowserProfile,
    _url: &str,
    _browser_type: BrowserType,
    _browser_dir: &Path,
    _profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Err("Opening URLs in existing Firefox-based browsers is not supported on Linux when using -no-remote".into())
  }

  pub async fn open_url_in_existing_browser_chromium(
    profile: &BrowserProfile,
    url: &str,
    browser_type: BrowserType,
    browser_dir: &Path,
    profiles_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let browser = create_browser(browser_type);
    let executable_path = browser
      .get_executable_path(browser_dir)
      .map_err(|e| format!("Failed to get executable path: {}", e))?;

    let profile_data_path = profile.get_profile_data_path(profiles_dir);
    let output = Command::new(executable_path)
      .args([
        &format!("--user-data-dir={}", profile_data_path.to_string_lossy()),
        url,
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to open URL in existing Chromium-based browser: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    Ok(())
  }

  pub async fn kill_browser_process_impl(
    pid: u32,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let system = System::new_all();
    if let Some(process) = system.process(Pid::from(pid as usize)) {
      if !process.kill() {
        return Err(format!("Failed to kill process {}", pid).into());
      }
    } else {
      return Err(format!("Process {} not found", pid).into());
    }

    println!("Successfully killed browser process with PID: {pid}");
    Ok(())
  }
}

pub struct BrowserRunner {
  base_dirs: BaseDirs,
}

impl BrowserRunner {
  pub fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
    }
  }

  /// Helper function to kill a browser process by PID only (used during migration)
  async fn kill_browser_process_by_pid(
    pid: u32,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Attempting to kill browser process with PID: {pid} during migration");

    // Kill the process using platform-specific implementation
    #[cfg(target_os = "macos")]
    macos::kill_browser_process_impl(pid).await?;

    #[cfg(target_os = "windows")]
    windows::kill_browser_process_impl(pid).await?;

    #[cfg(target_os = "linux")]
    linux::kill_browser_process_impl(pid).await?;

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("Unsupported platform".into());

    println!("Successfully killed browser process with PID: {pid} during migration");
    Ok(())
  }

  /// Migrate old profile structure to new UUID-based structure
  pub async fn migrate_profiles_to_uuid(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let profiles_dir = self.get_profiles_dir();
    if !profiles_dir.exists() {
      return Ok(vec![]);
    }

    let mut migrated_profiles = Vec::new();

    // Scan for old-format profile files (*.json files directly in profiles directory)
    for entry in fs::read_dir(&profiles_dir)? {
      let entry = entry?;
      let path = entry.path();

      // Look for .json files that are NOT in UUID directories
      if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
        let content = fs::read_to_string(&path)?;

        // Try to parse as old profile format (without UUID)
        let mut old_profile: serde_json::Value = serde_json::from_str(&content)?;

        // Skip if it already has an id field (already migrated)
        if old_profile.get("id").is_some() {
          continue;
        }

        // Generate new UUID for this profile
        let profile_id = uuid::Uuid::new_v4();

        // Extract profile name before mutating
        let profile_name = old_profile["name"]
          .as_str()
          .unwrap_or("unknown")
          .to_string();

        // Check if there's a running browser process for this profile and kill it
        if let Some(process_id_value) = old_profile.get("process_id") {
          if let Some(pid) = process_id_value.as_u64() {
            let pid = pid as u32;
            println!("Found running browser process (PID: {pid}) for profile '{profile_name}' during migration");

            // Kill the process before migration
            if let Err(e) = Self::kill_browser_process_by_pid(pid).await {
              println!(
                "Warning: Failed to kill browser process (PID: {pid}) during migration: {e}"
              );
              // Continue with migration even if kill fails - the process might already be dead
            } else {
              println!(
                "Successfully killed browser process (PID: {pid}) for profile '{profile_name}'"
              );
            }

            // Clear the process_id since we killed it
            old_profile["process_id"] = serde_json::Value::Null;
          }
        }

        let snake_case_name = profile_name.to_lowercase().replace(" ", "_");
        let old_profile_dir = profiles_dir.join(&snake_case_name);

        // Create new UUID directory and profile subdirectory
        let new_profile_dir = profiles_dir.join(profile_id.to_string());
        let new_profile_data_dir = new_profile_dir.join("profile");
        create_dir_all(&new_profile_dir)?;
        create_dir_all(&new_profile_data_dir)?;

        // Now update the profile with UUID (no need to store profile_path anymore)
        old_profile["id"] = serde_json::Value::String(profile_id.to_string());

        // Handle proxy migration - extract proxy to separate storage if it exists
        if let Some(proxy_value) = old_profile.get("proxy").cloned() {
          if !proxy_value.is_null() {
            // Try to deserialize the proxy settings
            if let Ok(proxy_settings) = serde_json::from_value::<ProxySettings>(proxy_value) {
              // Create a stored proxy with the profile name (all proxies are now enabled by default)
              let proxy_name = format!("{profile_name} Proxy");
              match PROXY_MANAGER.create_stored_proxy(proxy_name.clone(), proxy_settings.clone()) {
                Ok(stored_proxy) => {
                  // Update profile to reference the stored proxy
                  old_profile["proxy_id"] = serde_json::Value::String(stored_proxy.id);
                  println!(
                    "Migrated proxy for profile '{}' to stored proxy '{}'",
                    profile_name, stored_proxy.name
                  );
                }
                Err(e) => {
                  println!("Warning: Failed to migrate proxy for profile '{profile_name}': {e}");
                  // If creation fails (e.g., name collision), try to find existing proxy with same settings
                  let existing_proxies = PROXY_MANAGER.get_stored_proxies();
                  if let Some(existing_proxy) = existing_proxies.iter().find(|p| {
                    p.proxy_settings.proxy_type == proxy_settings.proxy_type
                      && p.proxy_settings.host == proxy_settings.host
                      && p.proxy_settings.port == proxy_settings.port
                      && p.proxy_settings.username == proxy_settings.username
                      && p.proxy_settings.password == proxy_settings.password
                  }) {
                    old_profile["proxy_id"] = serde_json::Value::String(existing_proxy.id.clone());
                    println!(
                      "Reused existing proxy '{}' for profile '{}'",
                      existing_proxy.name, profile_name
                    );
                  } else {
                    // Try with a different name if the original failed due to name collision
                    let alt_proxy_name = format!(
                      "{profile_name} Proxy {}",
                      &uuid::Uuid::new_v4().to_string()[..8]
                    );
                    match PROXY_MANAGER
                      .create_stored_proxy(alt_proxy_name.clone(), proxy_settings.clone())
                    {
                      Ok(stored_proxy) => {
                        old_profile["proxy_id"] = serde_json::Value::String(stored_proxy.id);
                        println!(
                          "Migrated proxy for profile '{}' to stored proxy '{}' with fallback name",
                          profile_name, stored_proxy.name
                        );
                      }
                      Err(e2) => {
                        println!("Error: Could not migrate proxy for profile '{profile_name}' even with fallback name: {e2}");
                      }
                    }
                  }
                }
              }
            } else {
              println!(
                "Warning: Could not deserialize proxy settings for profile '{profile_name}'"
              );
            }
          }
        }

        // Always remove the old proxy field after migration attempt, whether successful or not
        if old_profile
          .as_object_mut()
          .unwrap()
          .remove("proxy")
          .is_some()
        {
          println!("Removed legacy proxy field from profile '{profile_name}'");
        }

        // Move old profile directory contents to new UUID/profile directory if it exists
        if old_profile_dir.exists() && old_profile_dir.is_dir() {
          // Copy all contents from old directory to new profile subdirectory
          for entry in fs::read_dir(&old_profile_dir)? {
            let entry = entry?;
            let source_path = entry.path();
            let dest_path = new_profile_data_dir.join(entry.file_name());

            if source_path.is_dir() {
              Self::copy_directory_recursive(&source_path, &dest_path)?;
            } else {
              fs::copy(&source_path, &dest_path)?;
            }
          }

          // Remove old profile directory
          fs::remove_dir_all(&old_profile_dir)?;
          println!(
            "Migrated profile directory: {} -> {}",
            old_profile_dir.display(),
            new_profile_data_dir.display()
          );
        }

        // Save migrated profile as metadata.json in UUID directory
        let metadata_file = new_profile_dir.join("metadata.json");
        let json = serde_json::to_string_pretty(&old_profile)?;
        fs::write(&metadata_file, json)?;

        // Remove old profile JSON file
        fs::remove_file(&path)?;

        migrated_profiles.push(profile_name.clone());
        println!("Migrated profile '{profile_name}' to UUID: {profile_id}");
      }
    }

    if !migrated_profiles.is_empty() {
      println!(
        "Successfully migrated {} profiles to UUID format",
        migrated_profiles.len()
      );
    }

    Ok(migrated_profiles)
  }

  /// Recursively copy directory contents
  fn copy_directory_recursive(
    source: &Path,
    destination: &Path,
  ) -> Result<(), Box<dyn std::error::Error>> {
    if !destination.exists() {
      create_dir_all(destination)?;
    }

    for entry in fs::read_dir(source)? {
      let entry = entry?;
      let source_path = entry.path();
      let dest_path = destination.join(entry.file_name());

      if source_path.is_dir() {
        Self::copy_directory_recursive(&source_path, &dest_path)?;
      } else {
        fs::copy(&source_path, &dest_path)?;
      }
    }

    Ok(())
  }

  // Helper function to check if a process matches TOR/Mullvad browser
  fn is_tor_or_mullvad_browser(
    &self,
    exe_name: &str,
    cmd: &[std::ffi::OsString],
    browser_type: &str,
  ) -> bool {
    #[cfg(target_os = "macos")]
    return macos::is_tor_or_mullvad_browser(exe_name, cmd, browser_type);

    #[cfg(target_os = "windows")]
    return windows::is_tor_or_mullvad_browser(exe_name, cmd, browser_type);

    #[cfg(target_os = "linux")]
    return linux::is_tor_or_mullvad_browser(exe_name, cmd, browser_type);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
      let _ = (exe_name, cmd, browser_type);
      false
    }
  }

  pub fn get_binaries_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("binaries");
    path
  }

  pub fn get_profiles_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("profiles");
    path
  }

  pub fn create_profile(
    &self,
    name: &str,
    browser: &str,
    version: &str,
    release_type: &str,
    proxy_id: Option<String>,
    camoufox_config: Option<CamoufoxConfig>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    self.create_profile_with_group(
      name,
      browser,
      version,
      release_type,
      proxy_id,
      camoufox_config,
      None,
    )
  }

  #[allow(clippy::too_many_arguments)]
  pub fn create_profile_with_group(
    &self,
    name: &str,
    browser: &str,
    version: &str,
    release_type: &str,
    proxy_id: Option<String>,
    camoufox_config: Option<CamoufoxConfig>,
    group_id: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    println!("Attempting to create profile: {name}");

    // Check if a profile with this name already exists (case insensitive)
    let existing_profiles = self.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == name.to_lowercase())
    {
      return Err(format!("Profile with name '{name}' already exists").into());
    }

    // Generate a new UUID for this profile
    let profile_id = uuid::Uuid::new_v4();
    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile_id.to_string());
    let profile_data_dir = profile_uuid_dir.join("profile");
    let profile_file = profile_uuid_dir.join("metadata.json");

    // Create profile directory with UUID and profile subdirectory
    create_dir_all(&profile_uuid_dir)?;
    create_dir_all(&profile_data_dir)?;

    let profile = BrowserProfile {
      id: profile_id,
      name: name.to_string(),
      browser: browser.to_string(),
      version: version.to_string(),
      proxy_id: proxy_id.clone(),
      process_id: None,
      last_launch: None,
      release_type: release_type.to_string(),
      camoufox_config: camoufox_config.clone(),
      group_id: group_id.clone(),
    };

    // Save profile info
    self.save_profile(&profile)?;

    // Verify the profile was saved correctly
    if !profile_file.exists() {
      return Err(format!("Failed to create profile file for '{name}'").into());
    }

    println!("Profile '{name}' created successfully with ID: {profile_id}");

    // Create user.js with common Firefox preferences and apply proxy settings if provided
    if let Some(proxy_id_ref) = &proxy_id {
      if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_ref) {
        self.apply_proxy_settings_to_profile(&profile_data_dir, &proxy_settings, None)?;
      } else {
        // Proxy ID provided but not found, disable proxy
        self.disable_proxy_settings_in_profile(&profile_data_dir)?;
      }
    } else {
      // Create user.js with common Firefox preferences but no proxy
      self.disable_proxy_settings_in_profile(&profile_data_dir)?;
    }

    // Perform cleanup after profile creation to remove any unused binaries
    if let Err(e) = self.cleanup_unused_binaries_internal() {
      println!("Warning: Failed to cleanup unused binaries after profile creation: {e}");
    }

    Ok(profile)
  }

  pub async fn update_profile_proxy(
    &self,
    app_handle: tauri::AppHandle,
    profile_name: &str,
    proxy_id: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Find the profile by name
    let profiles =
      self
        .list_profiles()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to list profiles: {e}").into()
        })?;

    let mut profile = profiles
      .into_iter()
      .find(|p| p.name == profile_name)
      .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Profile {profile_name} not found").into()
      })?;

    // Check if browser is running to manage proxy accordingly
    let browser_is_running = profile.process_id.is_some()
      && self
        .check_browser_status(app_handle.clone(), &profile)
        .await?;

    // If browser is running, stop existing proxy
    if browser_is_running && profile.proxy_id.is_some() {
      if let Some(pid) = profile.process_id {
        let _ = PROXY_MANAGER.stop_proxy(app_handle.clone(), pid).await;
      }
    }

    // Update proxy settings
    profile.proxy_id = proxy_id.clone();

    // Save the updated profile
    self
      .save_profile(&profile)
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Failed to save profile: {e}").into()
      })?;

    // Handle proxy startup/configuration
    if let Some(proxy_id_ref) = &proxy_id {
      if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_ref) {
        if browser_is_running {
          // Browser is running and proxy is enabled, start new proxy
          if let Some(pid) = profile.process_id {
            match PROXY_MANAGER
              .start_proxy(app_handle.clone(), &proxy_settings, pid, Some(profile_name))
              .await
            {
              Ok(internal_proxy_settings) => {
                let browser_runner = BrowserRunner::new();
                let profiles_dir = browser_runner.get_profiles_dir();
                let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");

                // Apply the proxy settings with the internal proxy to the profile directory
                browser_runner
                  .apply_proxy_settings_to_profile(
                    &profile_path,
                    &proxy_settings,
                    Some(&internal_proxy_settings),
                  )
                  .map_err(|e| format!("Failed to update profile proxy: {e}"))?;

                println!("Successfully started proxy for profile: {}", profile.name);

                // Give the proxy a moment to fully start up
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
              }
              Err(e) => {
                eprintln!("Failed to start proxy: {e}");
                // Apply proxy settings without internal proxy
                let profiles_dir = self.get_profiles_dir();
                let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
                self
                  .apply_proxy_settings_to_profile(&profile_path, &proxy_settings, None)
                  .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("Failed to apply proxy settings: {e}").into()
                  })?;
              }
            }
          } else {
            // No PID available, apply proxy settings without internal proxy
            let profiles_dir = self.get_profiles_dir();
            let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
            self
              .apply_proxy_settings_to_profile(&profile_path, &proxy_settings, None)
              .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("Failed to apply proxy settings: {e}").into()
              })?;
          }
        } else {
          // Proxy disabled or browser not running, just apply settings
          let profiles_dir = self.get_profiles_dir();
          let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
          self
            .apply_proxy_settings_to_profile(&profile_path, &proxy_settings, None)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
              format!("Failed to apply proxy settings: {e}").into()
            })?;
        }
      } else {
        // Proxy ID provided but proxy not found, disable proxy
        let profiles_dir = self.get_profiles_dir();
        let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
        self
          .disable_proxy_settings_in_profile(&profile_path)
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to disable proxy settings: {e}").into()
          })?;
      }
    } else {
      // No proxy ID provided, disable proxy
      let profiles_dir = self.get_profiles_dir();
      let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
      self
        .disable_proxy_settings_in_profile(&profile_path)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to disable proxy settings: {e}").into()
        })?;
    }

    Ok(profile)
  }

  pub fn update_profile_version(
    &self,
    profile_name: &str,
    version: &str,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Find the profile by name
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.name == profile_name)
      .ok_or_else(|| format!("Profile {profile_name} not found"))?;

    // Check if the browser is currently running
    if profile.process_id.is_some() {
      return Err(
        "Cannot update version while browser is running. Please stop the browser first.".into(),
      );
    }

    // Verify the new version is downloaded
    let browser_type = BrowserType::from_str(&profile.browser)
      .map_err(|_| format!("Invalid browser type: {}", profile.browser))?;
    let browser = create_browser(browser_type.clone());
    let binaries_dir = self.get_binaries_dir();

    if !browser.is_version_downloaded(version, &binaries_dir) {
      return Err(format!("Browser version {version} is not downloaded").into());
    }

    // Update version
    profile.version = version.to_string();

    // Update the release_type based on the version and browser
    profile.release_type =
      if crate::api_client::is_browser_version_nightly(&profile.browser, version, None) {
        "nightly".to_string()
      } else {
        "stable".to_string()
      };

    // Save the updated profile
    self.save_profile(&profile)?;

    // Always perform cleanup after profile version update to remove unused binaries
    let _ = self.cleanup_unused_binaries_internal();

    Ok(profile)
  }

  /// Internal method to cleanup unused binaries (used by auto-cleanup)
  pub fn cleanup_unused_binaries_internal(
    &self,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // Load current profiles
    let profiles = self
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    // Load registry
    let mut registry = crate::downloaded_browsers::DownloadedBrowsersRegistry::load()?;

    // Get active browser versions (all profiles)
    let active_versions = registry.get_active_browser_versions(&profiles);

    // Get running browser versions (only running profiles)
    let running_versions = registry.get_running_browser_versions(&profiles);

    // Cleanup unused binaries (but keep running ones)
    let cleaned_up = registry.cleanup_unused_binaries(&active_versions, &running_versions)?;

    // Save updated registry
    registry.save()?;

    Ok(cleaned_up)
  }

  pub fn assign_profiles_to_group(
    &self,
    profile_names: Vec<String>,
    group_id: Option<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;

    for profile_name in profile_names {
      let mut profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .ok_or_else(|| format!("Profile '{profile_name}' not found"))?
        .clone();

      // Check if browser is running
      if profile.process_id.is_some() {
        return Err(format!(
          "Cannot modify group for profile '{profile_name}' while browser is running. Please stop the browser first."
        ).into());
      }

      profile.group_id = group_id.clone();
      self.save_profile(&profile)?;
    }

    Ok(())
  }

  pub fn delete_multiple_profiles(
    &self,
    profile_names: Vec<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;

    for profile_name in profile_names {
      let profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .ok_or_else(|| format!("Profile '{profile_name}' not found"))?;

      // Check if browser is running
      if profile.process_id.is_some() {
        return Err(
          format!(
            "Cannot delete profile '{profile_name}' while browser is running. Please stop the browser first."
          )
          .into(),
        );
      }

      // Delete the profile
      let profiles_dir = self.get_profiles_dir();
      let profile_uuid_dir = profiles_dir.join(profile.id.to_string());

      if profile_uuid_dir.exists() {
        std::fs::remove_dir_all(&profile_uuid_dir)?;
      }
    }

    // Always perform cleanup after profile deletion to remove unused binaries
    if let Err(e) = self.cleanup_unused_binaries_internal() {
      println!("Warning: Failed to cleanup unused binaries: {e}");
    }

    Ok(())
  }

  fn get_common_firefox_preferences(&self) -> Vec<String> {
    vec![
      // Disable default browser check
      "user_pref(\"browser.shell.checkDefaultBrowser\", false);".to_string(),
      "user_pref(\"app.update.enabled\", false);".to_string(),
      "user_pref(\"app.update.auto\", false);".to_string(),
      "user_pref(\"app.update.mode\", 2);".to_string(),
      "user_pref(\"app.update.promptWaitTime\", 0);".to_string(),
      "user_pref(\"app.update.service.enabled\", false);".to_string(),
      "user_pref(\"app.update.silent\", true);".to_string(),
      "user_pref(\"app.update.checkInstallTime\", false);".to_string(),
      "user_pref(\"app.update.url\", \"\");".to_string(),
      "user_pref(\"app.update.url.manual\", \"\");".to_string(),
      "user_pref(\"app.update.url.details\", \"\");".to_string(),
      "user_pref(\"app.update.url.override\", \"\");".to_string(),
      "user_pref(\"app.update.interval\", 9999999999);".to_string(),
      "user_pref(\"app.update.background.interval\", 9999999999);".to_string(),
      "user_pref(\"app.update.download.attemptOnce\", false);".to_string(),
      "user_pref(\"app.update.idletime\", -1);".to_string(),
    ]
  }

  fn apply_proxy_settings_to_profile(
    &self,
    profile_data_path: &Path,
    proxy: &ProxySettings,
    internal_proxy: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let user_js_path = profile_data_path.join("user.js");
    let mut preferences = Vec::new();

    // Get the UUID directory (parent of profile data directory)
    let uuid_dir = profile_data_path
      .parent()
      .ok_or("Invalid profile path - cannot find UUID directory")?;

    // Add common Firefox preferences (like disabling default browser check)
    preferences.extend(self.get_common_firefox_preferences());

    // Use embedded PAC template instead of reading from file
    const PAC_TEMPLATE: &str = r#"function FindProxyForURL(url, host) {
  return "{{proxy_url}}";
}"#;

    // Format proxy URL based on type and whether we have an internal proxy
    let proxy_url = if let Some(internal) = internal_proxy {
      // Use internal proxy as the primary proxy
      format!("HTTP {}:{}", internal.host, internal.port)
    } else {
      // Use user-configured proxy directly
      match proxy.proxy_type.as_str() {
        "http" => format!("HTTP {}:{}", proxy.host, proxy.port),
        "https" => format!("HTTPS {}:{}", proxy.host, proxy.port),
        "socks4" => format!("SOCKS4 {}:{}", proxy.host, proxy.port),
        "socks5" => format!("SOCKS5 {}:{}", proxy.host, proxy.port),
        _ => return Err(format!("Unsupported proxy type: {}", proxy.proxy_type).into()),
      }
    };

    // Replace placeholders in PAC file
    let pac_content = PAC_TEMPLATE
      .replace("{{proxy_url}}", &proxy_url)
      .replace("{{proxy_credentials}}", ""); // Credentials are now handled by the PAC file

    // Save PAC file in UUID directory
    let pac_path = uuid_dir.join("proxy.pac");
    fs::write(&pac_path, pac_content)?;

    // Configure Firefox to use the PAC file
    preferences.extend([
      "user_pref(\"network.proxy.type\", 2);".to_string(),
      format!(
        "user_pref(\"network.proxy.autoconfig_url\", \"file://{}\");",
        pac_path.to_string_lossy()
      ),
      "user_pref(\"network.proxy.failover_direct\", false);".to_string(),
      "user_pref(\"network.proxy.socks_remote_dns\", true);".to_string(),
      "user_pref(\"network.proxy.no_proxies_on\", \"\");".to_string(),
      "user_pref(\"signon.autologin.proxy\", true);".to_string(),
      "user_pref(\"network.proxy.share_proxy_settings\", false);".to_string(),
      "user_pref(\"network.automatic-ntlm-auth.allow-proxies\", false);".to_string(),
      "user_pref(\"network.auth-use-sspi\", false);".to_string(),
    ]);

    // Write settings to user.js file
    fs::write(user_js_path, preferences.join("\n"))?;

    Ok(())
  }

  pub fn disable_proxy_settings_in_profile(
    &self,
    profile_data_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let user_js_path = profile_data_path.join("user.js");
    let mut preferences = Vec::new();

    // Get the UUID directory (parent of profile data directory)
    let uuid_dir = profile_data_path
      .parent()
      .ok_or("Invalid profile path - cannot find UUID directory")?;

    // Add common Firefox preferences (like disabling default browser check)
    preferences.extend(self.get_common_firefox_preferences());

    preferences.push("user_pref(\"network.proxy.type\", 0);".to_string());
    preferences.push("user_pref(\"network.proxy.failover_direct\", true);".to_string());

    // Create a direct proxy PAC file in UUID directory
    let pac_content = "function FindProxyForURL(url, host) { return 'DIRECT'; }";
    let pac_path = uuid_dir.join("proxy.pac");
    fs::write(&pac_path, pac_content)?;
    preferences.push(format!(
      "user_pref(\"network.proxy.autoconfig_url\", \"file://{}\");",
      pac_path.to_string_lossy()
    ));

    fs::write(user_js_path, preferences.join("\n"))?;

    Ok(())
  }

  pub fn save_profile(&self, profile: &BrowserProfile) -> Result<(), Box<dyn std::error::Error>> {
    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile.id.to_string());
    let profile_file = profile_uuid_dir.join("metadata.json");

    // Ensure the UUID directory exists
    create_dir_all(&profile_uuid_dir)?;

    let json = serde_json::to_string_pretty(profile)?;
    fs::write(profile_file, json)?;

    Ok(())
  }

  pub fn list_profiles(&self) -> Result<Vec<BrowserProfile>, Box<dyn std::error::Error>> {
    let profiles_dir = self.get_profiles_dir();
    if !profiles_dir.exists() {
      return Ok(vec![]);
    }

    let mut profiles = Vec::new();
    for entry in fs::read_dir(profiles_dir)? {
      let entry = entry?;
      let path = entry.path();

      // Look for UUID directories containing metadata.json
      if path.is_dir() {
        let metadata_file = path.join("metadata.json");
        if metadata_file.exists() {
          let content = fs::read_to_string(metadata_file)?;
          let profile: BrowserProfile = serde_json::from_str(&content)?;
          profiles.push(profile);
        }
      }
    }

    Ok(profiles)
  }

  pub async fn launch_browser(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    local_proxy_settings: Option<&ProxySettings>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Handle camoufox profiles specially using only the direct launcher
    if profile.browser == "camoufox" {
      if let Some(mut camoufox_config) = profile.camoufox_config.clone() {
        // Handle proxy settings for camoufox
        if let Some(proxy_id) = &profile.proxy_id {
          if let Some(stored_proxy) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id) {
            println!("Starting proxy for Camoufox profile: {}", profile.name);

            // Start the proxy and get local proxy settings
            let local_proxy = PROXY_MANAGER
              .start_proxy(
                app_handle.clone(),
                &stored_proxy,
                0, // Use 0 as temporary PID, will be updated later
                Some(&profile.name),
              )
              .await
              .map_err(|e| format!("Failed to start proxy for Camoufox: {e}"))?;

            // Format proxy URL for camoufox
            let proxy_url = format!(
              "{}://{}:{}",
              if stored_proxy.proxy_type == "socks5" || stored_proxy.proxy_type == "socks4" {
                &stored_proxy.proxy_type
              } else {
                "http"
              },
              local_proxy.host,
              local_proxy.port
            );

            // Add username and password if available
            let proxy_url = if let (Some(username), Some(password)) =
              (&stored_proxy.username, &stored_proxy.password)
            {
              format!(
                "{}://{}:{}@{}:{}",
                if stored_proxy.proxy_type == "socks5" || stored_proxy.proxy_type == "socks4" {
                  &stored_proxy.proxy_type
                } else {
                  "http"
                },
                username,
                password,
                local_proxy.host,
                local_proxy.port
              )
            } else {
              proxy_url
            };

            // Set proxy in camoufox config
            camoufox_config.proxy = Some(proxy_url);

            println!("Configured proxy for Camoufox: {:?}", camoufox_config.proxy);
          }
        }

        // Use the existing config or create a test config if none exists
        let final_config = if camoufox_config.timezone.is_some()
          || camoufox_config.screen_min_width.is_some()
          || camoufox_config.window_width.is_some()
        {
          camoufox_config.clone()
        } else {
          // No meaningful config provided, use test config to ensure anti-fingerprinting works
          println!("No Camoufox configuration provided, using test configuration");
          let mut test_config =
            crate::camoufox_direct::CamoufoxDirectLauncher::create_test_config();
          // Preserve any proxy settings from the original config
          test_config.proxy = camoufox_config.proxy.clone();
          test_config.headless = camoufox_config.headless;
          test_config.debug = Some(true); // Enable debug for troubleshooting
          test_config
        };

        // Use the direct camoufox launcher
        let camoufox_result = crate::camoufox_direct::launch_camoufox_profile_direct(
          app_handle.clone(),
          profile.clone(),
          final_config,
          url,
        )
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to launch camoufox: {e}").into()
        })?;

        // Update proxy with actual PID if proxy was started
        if let Some(pid) = camoufox_result.pid {
          if profile.proxy_id.is_some() {
            if let Err(e) = PROXY_MANAGER.update_proxy_pid(0, pid) {
              println!("Warning: Failed to update proxy PID: {e}");
            }
          }
        }

        // Update profile with the process info from camoufox result
        let mut updated_profile = profile.clone();
        updated_profile.process_id = camoufox_result.pid;
        updated_profile.last_launch = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());

        // Save the updated profile
        self.save_process_info(&updated_profile)?;

        // Emit profile update event to frontend
        if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
          println!("Warning: Failed to emit profile update event: {e}");
        }

        return Ok(updated_profile);
      } else {
        return Err("Camoufox profile missing configuration".into());
      }
    }

    // Create browser instance
    let browser_type = BrowserType::from_str(&profile.browser)
      .map_err(|_| format!("Invalid browser type: {}", profile.browser))?;
    let browser = create_browser(browser_type.clone());

    // Get executable path - path structure: binaries/<browser>/<version>/
    let mut browser_dir = self.get_binaries_dir();
    browser_dir.push(&profile.browser);
    browser_dir.push(&profile.version);

    println!("Browser directory: {browser_dir:?}");
    let executable_path = browser
      .get_executable_path(&browser_dir)
      .expect("Failed to get executable path");

    // Prepare the executable (set permissions, etc.)
    if let Err(e) = browser.prepare_executable(&executable_path) {
      println!("Warning: Failed to prepare executable: {e}");
      // Continue anyway, the error might not be critical
    }

    // For Chromium browsers, use local proxy settings if available
    // For Firefox browsers, proxy settings are handled via PAC files
    let stored_proxy_settings = profile
      .proxy_id
      .as_ref()
      .and_then(|id| PROXY_MANAGER.get_proxy_settings_by_id(id));
    let proxy_for_launch_args = match browser_type {
      BrowserType::Chromium | BrowserType::Brave => {
        local_proxy_settings.or(stored_proxy_settings.as_ref())
      }
      _ => None, // Firefox browsers use PAC files, not launch args
    };

    // Get profile data path and launch arguments
    let profiles_dir = self.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    let browser_args = browser
      .create_launch_args(
        &profile_data_path.to_string_lossy(),
        proxy_for_launch_args,
        url,
      )
      .expect("Failed to create launch arguments");

    // Launch browser using platform-specific method
    let child = {
      #[cfg(target_os = "macos")]
      {
        macos::launch_browser_process(&executable_path, &browser_args).await?
      }

      #[cfg(target_os = "windows")]
      {
        windows::launch_browser_process(&executable_path, &browser_args).await?
      }

      #[cfg(target_os = "linux")]
      {
        linux::launch_browser_process(&executable_path, &browser_args).await?
      }

      #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
      {
        return Err("Unsupported platform for browser launching".into());
      }
    };

    let launcher_pid = child.id();

    println!(
      "Launched browser with launcher PID: {} for profile: {}",
      launcher_pid, profile.name
    );

    // For TOR and Mullvad browsers, we need to find the actual browser process
    // because they use launcher scripts that spawn the real browser process
    let mut actual_pid = launcher_pid;

    if matches!(
      browser_type,
      BrowserType::TorBrowser | BrowserType::MullvadBrowser
    ) {
      // Wait a moment for the actual browser process to start
      tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;

      // Find the actual browser process
      let system = System::new_all();
      for (pid, process) in system.processes() {
        let process_name = process.name().to_str().unwrap_or("");
        let process_cmd = process.cmd();
        let pid_u32 = pid.as_u32();

        // Skip if this is the launcher process itself
        if pid_u32 == launcher_pid {
          continue;
        }

        if self.is_tor_or_mullvad_browser(process_name, process_cmd, &profile.browser) {
          println!(
            "Found actual {} browser process: PID {} ({})",
            profile.browser, pid_u32, process_name
          );
          actual_pid = pid_u32;
          break;
        }
      }
    }

    // Update profile with process info and save
    let mut updated_profile = profile.clone();
    updated_profile.process_id = Some(actual_pid);
    updated_profile.last_launch = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());

    self.save_process_info(&updated_profile)?;

    // Apply proxy settings if needed (for Firefox-based browsers)
    if profile.proxy_id.is_some()
      && matches!(
        browser_type,
        BrowserType::Firefox
          | BrowserType::FirefoxDeveloper
          | BrowserType::Zen
          | BrowserType::TorBrowser
          | BrowserType::MullvadBrowser
      )
    {
      // Proxy settings for Firefox-based browsers are applied via user.js file
      // which is already handled in the profile creation process
    }

    // Start proxy if configured and needed (for Chromium-based browsers)
    if let Some(proxy_id) = &profile.proxy_id {
      if let Some(stored_proxy) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id) {
        println!("Starting proxy for profile: {}", profile.name);

        match PROXY_MANAGER
          .start_proxy(
            app_handle.clone(),
            &stored_proxy,
            actual_pid,
            Some(&profile.name),
          )
          .await
        {
          Ok(_) => println!("Proxy started successfully for profile: {}", profile.name),
          Err(e) => println!("Warning: Failed to start proxy: {e}"),
        }
      }
    }

    // Emit profile update event to frontend
    if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
      println!("Warning: Failed to emit profile update event: {e}");
    }

    Ok(updated_profile)
  }

  pub async fn open_url_in_existing_browser(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: &str,
    _internal_proxy_settings: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle camoufox profiles specially using only the direct launcher
    if profile.browser == "camoufox" {
      let camoufox_launcher =
        crate::camoufox_direct::CamoufoxDirectLauncher::new(app_handle.clone());

      // Get the profile path based on the UUID
      let profiles_dir = self.get_profiles_dir();
      let profile_data_path = profile.get_profile_data_path(&profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      // Check if the process is running
      match camoufox_launcher
        .find_camoufox_by_profile(&profile_path_str)
        .await
      {
        Ok(Some(_camoufox_process)) => {
          println!(
            "Opening URL in existing Camoufox process for profile: {}",
            profile.name
          );

          // For Camoufox, we need to launch a new instance with the URL since it doesn't support remote commands
          // This is a limitation of Camoufox's architecture
          return Err("Camoufox doesn't support opening URLs in existing instances. Please close the browser and launch again with the URL.".into());
        }
        Ok(None) => {
          return Err("Camoufox browser is not running".into());
        }
        Err(e) => {
          return Err(format!("Error checking Camoufox process: {e}").into());
        }
      }
    }

    // Use the comprehensive browser status check for non-camoufox browsers
    let is_running = self
      .check_browser_status(app_handle.clone(), profile)
      .await?;

    if !is_running {
      return Err("Browser is not running".into());
    }

    // Get the updated profile with current PID
    let profiles = self.list_profiles().expect("Failed to list profiles");
    let updated_profile = profiles
      .into_iter()
      .find(|p| p.name == profile.name)
      .unwrap_or_else(|| profile.clone());

    // Ensure we have a valid process ID
    if updated_profile.process_id.is_none() {
      return Err("No valid process ID found for the browser".into());
    }

    let browser_type = BrowserType::from_str(&updated_profile.browser)
      .map_err(|_| format!("Invalid browser type: {}", updated_profile.browser))?;

    // Get browser directory for all platforms - path structure: binaries/<browser>/<version>/
    let mut browser_dir = self.get_binaries_dir();
    browser_dir.push(&updated_profile.browser);
    browser_dir.push(&updated_profile.version);

    match browser_type {
      BrowserType::Firefox | BrowserType::FirefoxDeveloper | BrowserType::Zen => {
        #[cfg(target_os = "macos")]
        {
          let profiles_dir = self.get_profiles_dir();
          return macos::open_url_in_existing_browser_firefox_like(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "windows")]
        {
          let profiles_dir = self.get_profiles_dir();
          return windows::open_url_in_existing_browser_firefox_like(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "linux")]
        {
          let profiles_dir = self.get_profiles_dir();
          return linux::open_url_in_existing_browser_firefox_like(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        return Err("Unsupported platform".into());
      }
      BrowserType::MullvadBrowser | BrowserType::TorBrowser => {
        #[cfg(target_os = "macos")]
        {
          let profiles_dir = self.get_profiles_dir();
          return macos::open_url_in_existing_browser_tor_mullvad(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "windows")]
        {
          let profiles_dir = self.get_profiles_dir();
          return windows::open_url_in_existing_browser_tor_mullvad(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "linux")]
        {
          let profiles_dir = self.get_profiles_dir();
          return linux::open_url_in_existing_browser_tor_mullvad(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        return Err("Unsupported platform".into());
      }
      BrowserType::Chromium | BrowserType::Brave => {
        #[cfg(target_os = "macos")]
        {
          let profiles_dir = self.get_profiles_dir();
          return macos::open_url_in_existing_browser_chromium(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "windows")]
        {
          let profiles_dir = self.get_profiles_dir();
          return windows::open_url_in_existing_browser_chromium(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "linux")]
        {
          let profiles_dir = self.get_profiles_dir();
          return linux::open_url_in_existing_browser_chromium(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        return Err("Unsupported platform".into());
      }
      BrowserType::Camoufox => {
        // This should never be reached due to the early return above, but handle it just in case
        Err("Camoufox URL opening should be handled in the early return above".into())
      }
    }
  }

  pub async fn launch_or_open_url(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    internal_proxy_settings: Option<&ProxySettings>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Get the most up-to-date profile data
    let profiles = self.list_profiles().expect("Failed to list profiles");
    let updated_profile = profiles
      .into_iter()
      .find(|p| p.name == profile.name)
      .unwrap_or_else(|| profile.clone());

    // Check if browser is already running
    let is_running = self
      .check_browser_status(app_handle.clone(), &updated_profile)
      .await?;

    // Get the updated profile again after status check (PID might have been updated)
    let profiles = self.list_profiles().expect("Failed to list profiles");
    let final_profile = profiles
      .into_iter()
      .find(|p| p.name == profile.name)
      .unwrap_or_else(|| updated_profile.clone());

    println!(
      "Browser status check - Profile: {}, Running: {}, URL: {:?}, PID: {:?}",
      final_profile.name, is_running, url, final_profile.process_id
    );

    if is_running && url.is_some() {
      // Browser is running and we have a URL to open
      if let Some(url_ref) = url.as_ref() {
        println!("Opening URL in existing browser: {url_ref}");

        // For TOR/Mullvad browsers, add extra verification
        if matches!(
          final_profile.browser.as_str(),
          "tor-browser" | "mullvad-browser"
        ) {
          println!("TOR/Mullvad browser detected - ensuring we have correct PID");
          if final_profile.process_id.is_none() {
            println!(
              "ERROR: No PID found for running TOR/Mullvad browser - this should not happen"
            );
            return Err("No PID found for running browser".into());
          }
        }
        match self
          .open_url_in_existing_browser(
            app_handle.clone(),
            &final_profile,
            url_ref,
            internal_proxy_settings,
          )
          .await
        {
          Ok(()) => {
            println!("Successfully opened URL in existing browser");
            Ok(final_profile)
          }
          Err(e) => {
            println!("Failed to open URL in existing browser: {e}");

            // For Mullvad and Tor browsers, don't fall back to new instance since they use -no-remote
            // and can't have multiple instances with the same profile
            match final_profile.browser.as_str() {
              "mullvad-browser" | "tor-browser" => {
                Err(format!(
                  "Failed to open URL in existing {} browser. Cannot launch new instance due to profile conflict: {}",
                  final_profile.browser, e
                ).into())
              }
              _ => {
                println!(
                  "Falling back to new instance for browser: {}",
                  final_profile.browser
                );
                // Fallback to launching a new instance for other browsers
                self.launch_browser(app_handle.clone(), &final_profile, url, internal_proxy_settings).await
              }
            }
          }
        }
      } else {
        // This case shouldn't happen since we checked is_some() above, but handle it gracefully
        println!("URL was unexpectedly None, launching new browser instance");
        self
          .launch_browser(
            app_handle.clone(),
            &final_profile,
            url,
            internal_proxy_settings,
          )
          .await
      }
    } else {
      // Browser is not running or no URL provided, launch new instance
      if !is_running {
        println!("Launching new browser instance - browser not running");
      } else {
        println!("Launching new browser instance - no URL provided");
      }
      self
        .launch_browser(
          app_handle.clone(),
          &final_profile,
          url,
          internal_proxy_settings,
        )
        .await
    }
  }

  pub fn rename_profile(
    &self,
    old_name: &str,
    new_name: &str,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Check if new name already exists (case insensitive)
    let existing_profiles = self.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == new_name.to_lowercase())
    {
      return Err(format!("Profile with name '{new_name}' already exists").into());
    }

    // Find the profile by old name
    let mut profile = existing_profiles
      .into_iter()
      .find(|p| p.name == old_name)
      .ok_or_else(|| format!("Profile '{old_name}' not found"))?;

    // Update profile name (no need to move directories since we use UUID)
    profile.name = new_name.to_string();

    // Save profile with new name
    self.save_profile(&profile)?;

    Ok(profile)
  }

  fn save_process_info(
    &self,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use the regular save_profile method which handles the UUID structure
    self.save_profile(profile).map_err(|e| {
      let error_string = e.to_string();
      Box::new(std::io::Error::other(error_string)) as Box<dyn std::error::Error + Send + Sync>
    })
  }

  pub fn delete_profile(&self, profile_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Attempting to delete profile: {profile_name}");

    // Find the profile by name
    let profiles = self.list_profiles()?;
    let profile = profiles
      .into_iter()
      .find(|p| p.name == profile_name)
      .ok_or_else(|| format!("Profile '{profile_name}' not found"))?;

    // Check if browser is running
    if profile.process_id.is_some() {
      return Err(
        "Cannot delete profile while browser is running. Please stop the browser first.".into(),
      );
    }

    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile.id.to_string());

    // Delete the entire UUID directory (contains both metadata.json and profile data)
    if profile_uuid_dir.exists() {
      println!("Deleting profile directory: {}", profile_uuid_dir.display());
      fs::remove_dir_all(&profile_uuid_dir)?;
      println!("Profile directory deleted successfully");
    }

    // Verify deletion was successful
    if profile_uuid_dir.exists() {
      return Err(format!("Failed to completely delete profile '{profile_name}'").into());
    }

    println!("Profile '{profile_name}' deleted successfully");

    // Always perform cleanup after profile deletion to remove unused binaries
    if let Err(e) = self.cleanup_unused_binaries_internal() {
      println!("Warning: Failed to cleanup unused binaries: {e}");
    }

    Ok(())
  }

  pub async fn check_browser_status(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Handle camoufox profiles using the same fast approach as other browsers
    // No special handling needed - camoufox uses the same process checking logic

    // For non-camoufox browsers, use the existing logic
    let mut inner_profile = profile.clone();
    let system = System::new_all();
    let mut is_running = false;
    let mut found_pid: Option<u32> = None;

    // First check if the stored PID is still valid
    if let Some(pid) = profile.process_id {
      if let Some(process) = system.process(Pid::from(pid as usize)) {
        let cmd = process.cmd();
        // Verify this process is actually our browser with the correct profile
        let profiles_dir = self.get_profiles_dir();
        let profile_data_path = profile.get_profile_data_path(&profiles_dir);
        let profile_data_path_str = profile_data_path.to_string_lossy();
        let profile_path_match = cmd.iter().any(|s| {
          let arg = s.to_str().unwrap_or("");
          // For Firefox-based browsers (including camoufox), check for exact profile path match
          if profile.browser == "tor-browser"
            || profile.browser == "firefox"
            || profile.browser == "firefox-developer"
            || profile.browser == "mullvad-browser"
            || profile.browser == "zen"
            || profile.browser == "camoufox"
          {
            arg == profile_data_path_str
              || arg == format!("-profile={profile_data_path_str}")
              || (arg == "-profile"
                && cmd
                  .iter()
                  .any(|s2| s2.to_str().unwrap_or("") == profile_data_path_str))
          } else {
            // For Chromium-based browsers, check for user-data-dir
            arg.contains(&format!("--user-data-dir={profile_data_path_str}"))
              || arg == profile_data_path_str
          }
        });

        if profile_path_match {
          is_running = true;
          found_pid = Some(pid);
          println!(
            "Found existing browser process with PID: {} for profile: {}",
            pid, profile.name
          );
        } else {
          println!("PID {pid} exists but doesn't match our profile path exactly, searching for correct process...");
        }
      } else {
        println!("Stored PID {pid} no longer exists, searching for browser process...");
      }
    }

    // If we didn't find the browser with the stored PID, search all processes
    if !is_running {
      for (pid, process) in system.processes() {
        let cmd = process.cmd();
        if cmd.len() >= 2 {
          // Check if this is the right browser executable first
          let exe_name = process.name().to_string_lossy().to_lowercase();
          let is_correct_browser = match profile.browser.as_str() {
            "firefox" => {
              exe_name.contains("firefox")
                && !exe_name.contains("developer")
                && !exe_name.contains("tor")
                && !exe_name.contains("mullvad")
                && !exe_name.contains("camoufox")
            }
            "firefox-developer" => exe_name.contains("firefox") && exe_name.contains("developer"),
            "mullvad-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "mullvad-browser"),
            "tor-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "tor-browser"),
            "zen" => exe_name.contains("zen"),
            "chromium" => exe_name.contains("chromium"),
            "brave" => exe_name.contains("brave"),
            "camoufox" => {
              exe_name.contains("camoufox")
                || (exe_name.contains("firefox")
                  && cmd
                    .iter()
                    .any(|arg| arg.to_str().unwrap_or("").contains("camoufox")))
            }
            _ => false,
          };

          if !is_correct_browser {
            continue;
          }

          // Check for profile path match
          let profiles_dir = self.get_profiles_dir();
          let profile_data_path = profile.get_profile_data_path(&profiles_dir);
          let profile_data_path_str = profile_data_path.to_string_lossy();
          let profile_path_match = cmd.iter().any(|s| {
            let arg = s.to_str().unwrap_or("");
            // For Firefox-based browsers, check for exact profile path match
            if profile.browser == "tor-browser"
              || profile.browser == "firefox"
              || profile.browser == "firefox-developer"
              || profile.browser == "mullvad-browser"
              || profile.browser == "zen"
            {
              arg == profile_data_path_str
                || arg == format!("-profile={profile_data_path_str}")
                || (arg == "-profile"
                  && cmd
                    .iter()
                    .any(|s2| s2.to_str().unwrap_or("") == profile_data_path_str))
            } else {
              // For Chromium-based browsers, check for user-data-dir
              arg.contains(&format!("--user-data-dir={profile_data_path_str}"))
                || arg == profile_data_path_str
            }
          });

          if profile_path_match {
            // Found a matching process
            found_pid = Some(pid.as_u32());
            is_running = true;
            println!(
              "Found browser process with PID: {} for profile: {}",
              pid.as_u32(),
              profile.name
            );
            break;
          }
        }
      }
    }

    // Update the process ID if we found a different one
    if let Some(pid) = found_pid {
      if inner_profile.process_id != Some(pid) {
        inner_profile.process_id = Some(pid);
        if let Err(e) = self.save_profile(&inner_profile) {
          println!("Warning: Failed to update profile with new PID: {e}");
        }
      }
    } else if inner_profile.process_id.is_some() {
      // Clear the PID if no process found
      inner_profile.process_id = None;
      if let Err(e) = self.save_profile(&inner_profile) {
        println!("Warning: Failed to clear profile PID: {e}");
      }
    }

    // Emit profile update event to frontend
    if let Err(e) = app_handle.emit("profile-updated", &inner_profile) {
      println!("Warning: Failed to emit profile update event: {e}");
    }

    Ok(is_running)
  }

  pub fn update_camoufox_config(
    &self,
    profile_name: &str,
    config: crate::camoufox_direct::CamoufoxConfig,
  ) -> Result<(), Box<dyn std::error::Error>> {
    // Find the profile by name
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.name == profile_name)
      .ok_or_else(|| format!("Profile {profile_name} not found"))?;

    // Check if the browser is currently running
    if profile.process_id.is_some() {
      return Err(
        "Cannot update Camoufox configuration while browser is running. Please stop the browser first.".into(),
      );
    }

    // Update the Camoufox configuration
    profile.camoufox_config = Some(config);

    // Save the updated profile
    self.save_profile(&profile)?;

    println!("Camoufox configuration updated for profile '{profile_name}'.");

    Ok(())
  }

  pub async fn kill_browser_process(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle camoufox profiles specially using only the direct launcher
    if profile.browser == "camoufox" {
      let camoufox_launcher =
        crate::camoufox_direct::CamoufoxDirectLauncher::new(app_handle.clone());

      // Try to stop by PID first (faster)
      if let Some(stored_pid) = profile.process_id {
        match camoufox_launcher
          .stop_camoufox(&stored_pid.to_string())
          .await
        {
          Ok(stopped) => {
            if stopped {
              println!("Successfully stopped Camoufox process by PID: {stored_pid}");
            } else {
              println!("Failed to stop Camoufox process by PID: {stored_pid}");
            }
          }
          Err(e) => {
            println!("Error stopping Camoufox process by PID: {e}");
          }
        }
      } else {
        // Fallback: search by profile path
        let profiles_dir = self.get_profiles_dir();
        let profile_data_path = profile.get_profile_data_path(&profiles_dir);
        let profile_path_str = profile_data_path.to_string_lossy();

        match camoufox_launcher
          .find_camoufox_by_profile(&profile_path_str)
          .await
        {
          Ok(Some(camoufox_process)) => {
            match camoufox_launcher.stop_camoufox(&camoufox_process.id).await {
              Ok(stopped) => {
                if stopped {
                  println!(
                    "Successfully stopped Camoufox process: {}",
                    camoufox_process.id
                  );
                } else {
                  println!("Failed to stop Camoufox process: {}", camoufox_process.id);
                }
              }
              Err(e) => {
                println!("Error stopping Camoufox process: {e}");
              }
            }
          }
          Ok(None) => {
            println!(
              "No running Camoufox process found for profile: {}",
              profile.name
            );
          }
          Err(e) => {
            println!("Error finding Camoufox process: {e}");
          }
        }
      }

      // Stop proxy if one was running for this profile
      if let Some(pid) = profile.process_id {
        if let Err(e) = PROXY_MANAGER.stop_proxy(app_handle.clone(), pid).await {
          println!("Warning: Failed to stop proxy for Camoufox profile: {e}");
        }
      }

      // Clear the process ID from the profile
      let mut updated_profile = profile.clone();
      updated_profile.process_id = None;
      self
        .save_process_info(&updated_profile)
        .map_err(|e| format!("Failed to update profile: {e}"))?;

      // Emit profile update event to frontend
      if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
        println!("Warning: Failed to emit profile update event: {e}");
      }

      return Ok(());
    }

    // For non-camoufox browsers, use the existing logic
    let pid = if let Some(pid) = profile.process_id {
      pid
    } else {
      // Try to find the process by searching all processes
      let system = System::new_all();
      let mut found_pid: Option<u32> = None;

      for (pid, process) in system.processes() {
        let cmd = process.cmd();
        if cmd.len() >= 2 {
          // Check if this is the right browser executable first
          let exe_name = process.name().to_string_lossy().to_lowercase();
          let is_correct_browser = match profile.browser.as_str() {
            "firefox" => {
              exe_name.contains("firefox")
                && !exe_name.contains("developer")
                && !exe_name.contains("tor")
                && !exe_name.contains("mullvad")
                && !exe_name.contains("camoufox")
            }
            "firefox-developer" => exe_name.contains("firefox") && exe_name.contains("developer"),
            "mullvad-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "mullvad-browser"),
            "tor-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "tor-browser"),
            "zen" => exe_name.contains("zen"),
            "chromium" => exe_name.contains("chromium"),
            "brave" => exe_name.contains("brave"),
            "camoufox" => {
              exe_name.contains("camoufox")
                || (exe_name.contains("firefox")
                  && cmd
                    .iter()
                    .any(|arg| arg.to_str().unwrap_or("").contains("camoufox")))
            }
            _ => false,
          };

          if !is_correct_browser {
            continue;
          }

          // Check for profile path match
          let profiles_dir = self.get_profiles_dir();
          let profile_data_path = profile.get_profile_data_path(&profiles_dir);
          let profile_data_path_str = profile_data_path.to_string_lossy();
          let profile_path_match = cmd.iter().any(|s| {
            let arg = s.to_str().unwrap_or("");
            // For Firefox-based browsers, check for exact profile path match
            if profile.browser == "tor-browser"
              || profile.browser == "firefox"
              || profile.browser == "firefox-developer"
              || profile.browser == "mullvad-browser"
              || profile.browser == "zen"
            {
              arg == profile_data_path_str || arg == format!("-profile={profile_data_path_str}")
            } else {
              // For Chromium-based browsers, check for user-data-dir
              arg.contains(&format!("--user-data-dir={profile_data_path_str}"))
                || arg == profile_data_path_str
            }
          });

          if profile_path_match {
            found_pid = Some(pid.as_u32());
            break;
          }
        }
      }

      found_pid.ok_or("Browser process not found")?
    };

    println!("Attempting to kill browser process with PID: {pid}");

    // Stop any associated proxy first
    if let Err(e) = PROXY_MANAGER.stop_proxy(app_handle.clone(), pid).await {
      println!("Warning: Failed to stop proxy for PID {pid}: {e}");
    }

    // Kill the process using platform-specific implementation
    #[cfg(target_os = "macos")]
    macos::kill_browser_process_impl(pid).await?;

    #[cfg(target_os = "windows")]
    windows::kill_browser_process_impl(pid).await?;

    #[cfg(target_os = "linux")]
    linux::kill_browser_process_impl(pid).await?;

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("Unsupported platform".into());

    // Clear the process ID from the profile
    let mut updated_profile = profile.clone();
    updated_profile.process_id = None;
    self
      .save_process_info(&updated_profile)
      .map_err(|e| format!("Failed to update profile: {e}"))?;

    Ok(())
  }

  /// Check if browser binaries exist for all profiles and return missing binaries
  pub async fn check_missing_binaries(
    &self,
  ) -> Result<Vec<(String, String, String)>, Box<dyn std::error::Error + Send + Sync>> {
    // Get all profiles
    let profiles = self
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    let mut missing_binaries = Vec::new();

    for profile in profiles {
      let browser_type = match BrowserType::from_str(&profile.browser) {
        Ok(bt) => bt,
        Err(_) => {
          println!(
            "Warning: Invalid browser type '{}' for profile '{}'",
            profile.browser, profile.name
          );
          continue;
        }
      };

      let browser = create_browser(browser_type.clone());
      let binaries_dir = self.get_binaries_dir();
      println!(
        "binaries_dir: {binaries_dir:?} for profile: {}",
        profile.name
      );

      // Check if the version is downloaded
      if !browser.is_version_downloaded(&profile.version, &binaries_dir) {
        missing_binaries.push((profile.name, profile.browser, profile.version));
      }
    }

    Ok(missing_binaries)
  }

  /// Automatically download missing binaries for all profiles
  pub async fn ensure_all_binaries_exist(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // First, clean up any stale registry entries
    if let Ok(mut registry) = DownloadedBrowsersRegistry::load() {
      if let Ok(cleaned_up) = registry.verify_and_cleanup_stale_entries(self) {
        if !cleaned_up.is_empty() {
          println!(
            "Cleaned up {} stale registry entries: {}",
            cleaned_up.len(),
            cleaned_up.join(", ")
          );
        }
      }
    }

    let missing_binaries = self.check_missing_binaries().await?;
    let mut downloaded = Vec::new();

    for (profile_name, browser, version) in missing_binaries {
      println!("Downloading missing binary for profile '{profile_name}': {browser} {version}");

      match self
        .download_browser_impl(app_handle.clone(), browser.clone(), version.clone())
        .await
      {
        Ok(_) => {
          downloaded.push(format!(
            "{browser} {version} (for profile '{profile_name}')"
          ));
        }
        Err(e) => {
          eprintln!("Failed to download {browser} {version} for profile '{profile_name}': {e}");
        }
      }
    }

    Ok(downloaded)
  }

  pub async fn download_browser_impl(
    &self,
    app_handle: tauri::AppHandle,
    browser_str: String,
    version: String,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Check if this browser-version pair is already being downloaded
    let download_key = format!("{browser_str}-{version}");
    {
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      if downloading.contains(&download_key) {
        return Err(format!(
          "Browser '{browser_str}' version '{version}' is already being downloaded. Please wait for the current download to complete."
        ).into());
      }
      // Mark this browser-version pair as being downloaded
      downloading.insert(download_key.clone());
    }

    let browser_type =
      BrowserType::from_str(&browser_str).map_err(|e| format!("Invalid browser type: {e}"))?;
    let browser = create_browser(browser_type.clone());

    // Load registry and check if already downloaded
    let mut registry = DownloadedBrowsersRegistry::load()
      .map_err(|e| format!("Failed to load browser registry: {e}"))?;

    // Check if registry thinks it's downloaded, but also verify files actually exist
    if registry.is_browser_downloaded(&browser_str, &version) {
      let binaries_dir = self.get_binaries_dir();
      let actually_exists = browser.is_version_downloaded(&version, &binaries_dir);

      if actually_exists {
        return Ok(version);
      } else {
        // Registry says it's downloaded but files don't exist - clean up registry
        println!("Registry indicates {browser_str} {version} is downloaded, but files are missing. Cleaning up registry entry.");
        registry.remove_browser(&browser_str, &version);
        registry
          .save()
          .map_err(|e| format!("Failed to save cleaned registry: {e}"))?;
      }
    }

    // Check if browser is supported on current platform before attempting download
    let version_service = BrowserVersionService::new();

    if !version_service
      .is_browser_supported(&browser_str)
      .unwrap_or(false)
    {
      return Err(
        format!(
          "Browser '{}' is not supported on your platform ({} {}). Supported browsers: {}",
          browser_str,
          std::env::consts::OS,
          std::env::consts::ARCH,
          version_service.get_supported_browsers().join(", ")
        )
        .into(),
      );
    }

    let download_info = version_service
      .get_download_info(&browser_str, &version)
      .map_err(|e| format!("Failed to get download info: {e}"))?;

    // Create browser directory
    let mut browser_dir = self.get_binaries_dir();
    browser_dir.push(browser_type.as_str());
    browser_dir.push(&version);

    // Clean up any failed previous download
    if let Err(e) = registry.cleanup_failed_download(&browser_str, &version) {
      println!("Warning: Failed to cleanup previous download: {e}");
    }

    create_dir_all(&browser_dir).map_err(|e| format!("Failed to create browser directory: {e}"))?;

    // Mark download as started in registry
    registry.mark_download_started(&browser_str, &version, browser_dir.clone());
    registry
      .save()
      .map_err(|e| format!("Failed to save registry: {e}"))?;

    // Use the new download module
    let downloader = Downloader::new();
    let download_path = match downloader
      .download_browser(
        &app_handle,
        browser_type.clone(),
        &version,
        &download_info,
        &browser_dir,
      )
      .await
    {
      Ok(path) => path,
      Err(e) => {
        // Clean up failed download
        let _ = registry.cleanup_failed_download(&browser_str, &version);
        let _ = registry.save();
        // Remove browser-version pair from downloading set on error
        {
          let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
          downloading.remove(&download_key);
        }
        return Err(format!("Failed to download browser: {e}").into());
      }
    };

    // Use the new extraction module
    if download_info.is_archive {
      let extractor = Extractor::new();
      match extractor
        .extract_browser(
          &app_handle,
          browser_type.clone(),
          &version,
          &download_path,
          &browser_dir,
        )
        .await
      {
        Ok(_) => {
          // Clean up the downloaded archive
          if let Err(e) = std::fs::remove_file(&download_path) {
            println!("Warning: Could not delete archive file: {e}");
          }
        }
        Err(e) => {
          // Clean up failed download
          let _ = registry.cleanup_failed_download(&browser_str, &version);
          let _ = registry.save();
          // Remove browser-version pair from downloading set on error
          {
            let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
            downloading.remove(&download_key);
          }
          return Err(format!("Failed to extract browser: {e}").into());
        }
      }

      // Give filesystem a moment to settle after extraction
      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Emit verification progress
    let progress = DownloadProgress {
      browser: browser_str.clone(),
      version: version.clone(),
      downloaded_bytes: 0,
      total_bytes: None,
      percentage: 100.0,
      speed_bytes_per_sec: 0.0,
      eta_seconds: None,
      stage: "verifying".to_string(),
    };
    let _ = app_handle.emit("download-progress", &progress);

    // Verify the browser was downloaded correctly
    println!("Verifying download for browser: {browser_str}, version: {version}");

    // Use the browser's own verification method
    let binaries_dir = self.get_binaries_dir();
    if !browser.is_version_downloaded(&version, &binaries_dir) {
      let _ = registry.cleanup_failed_download(&browser_str, &version);
      let _ = registry.save();
      // Remove browser-version pair from downloading set on verification failure
      {
        let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
        downloading.remove(&download_key);
      }
      return Err("Browser download completed but verification failed".into());
    }

    // Mark download as completed in registry
    let _actual_version = if browser_str == "chromium" {
      Some(version.clone())
    } else {
      None
    };

    registry
      .mark_download_completed(&browser_str, &version)
      .map_err(|e| format!("Failed to mark download as completed: {e}"))?;
    registry
      .save()
      .map_err(|e| format!("Failed to save registry: {e}"))?;

    // If this is Camoufox, automatically download GeoIP database
    if browser_str == "camoufox" {
      use crate::geoip_downloader::GeoIPDownloader;

      // Check if GeoIP database is already available
      if !GeoIPDownloader::is_geoip_database_available() {
        println!("Downloading GeoIP database for Camoufox...");

        let geoip_downloader = GeoIPDownloader::new();
        if let Err(e) = geoip_downloader.download_geoip_database(&app_handle).await {
          eprintln!("Warning: Failed to download GeoIP database: {e}");
          // Don't fail the browser download if GeoIP download fails
        } else {
          println!("GeoIP database downloaded successfully");
        }
      } else {
        println!("GeoIP database already available");
      }
    }

    // Emit completion
    let progress = DownloadProgress {
      browser: browser_str.clone(),
      version: version.clone(),
      downloaded_bytes: 0,
      total_bytes: None,
      percentage: 100.0,
      speed_bytes_per_sec: 0.0,
      eta_seconds: Some(0.0),
      stage: "completed".to_string(),
    };
    let _ = app_handle.emit("download-progress", &progress);

    // Remove browser-version pair from downloading set
    {
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      downloading.remove(&download_key);
    }

    Ok(version)
  }

  /// Check if a browser version is downloaded
  pub fn is_browser_downloaded(&self, browser_str: &str, version: &str) -> bool {
    // Always check if files actually exist on disk
    let browser_type = match BrowserType::from_str(browser_str) {
      Ok(bt) => bt,
      Err(_) => {
        println!("Invalid browser type: {browser_str}");
        return false;
      }
    };
    let browser = create_browser(browser_type.clone());
    let binaries_dir = self.get_binaries_dir();
    let files_exist = browser.is_version_downloaded(version, &binaries_dir);

    // If files don't exist but registry thinks they do, clean up the registry
    if !files_exist {
      if let Ok(mut registry) = DownloadedBrowsersRegistry::load() {
        if registry.is_browser_downloaded(browser_str, version) {
          println!("Cleaning up stale registry entry for {browser_str} {version}");
          registry.remove_browser(browser_str, version);
          let _ = registry.save(); // Don't fail if save fails, just log
        }
      }
    }

    files_exist
  }
}

impl BrowserProfile {
  /// Get the path to the profile data directory (profiles/{uuid}/profile)
  pub fn get_profile_data_path(&self, profiles_dir: &Path) -> PathBuf {
    profiles_dir.join(self.id.to_string()).join("profile")
  }
}

#[tauri::command]
pub fn create_browser_profile(
  name: String,
  browser: String,
  version: String,
  release_type: String,
  proxy_id: Option<String>,
  camoufox_config: Option<CamoufoxConfig>,
) -> Result<BrowserProfile, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .create_profile(
      &name,
      &browser,
      &version,
      &release_type,
      proxy_id,
      camoufox_config,
    )
    .map_err(|e| format!("Failed to create profile: {e}"))
}

#[tauri::command]
pub fn list_browser_profiles() -> Result<Vec<BrowserProfile>, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))
}

#[tauri::command]
pub async fn launch_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
) -> Result<BrowserProfile, String> {
  let browser_runner = BrowserRunner::new();

  // Store the internal proxy settings for passing to launch_browser
  let mut internal_proxy_settings: Option<ProxySettings> = None;

  // If the profile has proxy settings, we need to start the proxy first
  // and update the profile with proxy settings before launching
  let profile_for_launch = profile.clone();
  if let Some(proxy_id) = &profile.proxy_id {
    if let Some(proxy) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id) {
      // Use a temporary PID (1) to start the proxy, we'll update it after browser launch
      let temp_pid = 1u32;

      // Start the proxy first
      match PROXY_MANAGER
        .start_proxy(app_handle.clone(), &proxy, temp_pid, Some(&profile.name))
        .await
      {
        Ok(internal_proxy) => {
          let browser_runner = BrowserRunner::new();
          let profiles_dir = browser_runner.get_profiles_dir();
          let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");

          // Store the internal proxy settings for later use
          internal_proxy_settings = Some(internal_proxy.clone());

          // Apply the proxy settings with the internal proxy to the profile directory
          browser_runner
            .apply_proxy_settings_to_profile(&profile_path, &proxy, Some(&internal_proxy))
            .map_err(|e| format!("Failed to update profile proxy: {e}"))?;

          println!("Successfully started proxy for profile: {}", profile.name);

          // Give the proxy a moment to fully start up
          tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
        Err(e) => {
          eprintln!("Failed to start proxy: {e}");
          // Still continue with browser launch, but without proxy
          let browser_runner = BrowserRunner::new();
          let profiles_dir = browser_runner.get_profiles_dir();
          let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");

          // Apply proxy settings without internal proxy
          browser_runner
            .apply_proxy_settings_to_profile(&profile_path, &proxy, None)
            .map_err(|e| format!("Failed to update profile proxy: {e}"))?;
        }
      }
    }
  }

  // Launch browser or open URL in existing instance
  let updated_profile = browser_runner
    .launch_or_open_url(app_handle.clone(), &profile_for_launch, url, internal_proxy_settings.as_ref())
    .await
    .map_err(|e| {
      // Check if this is an architecture compatibility issue
      if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
        if io_error.kind() == std::io::ErrorKind::Other
           && io_error.to_string().contains("Exec format error") {
          return format!("Failed to launch browser: Executable format error. This browser version is not compatible with your system architecture ({}). Please try a different browser or version that supports your platform.", std::env::consts::ARCH);
        }
      }
      format!("Failed to launch browser or open URL: {e}")
    })?;

  // Now update the proxy with the correct PID if we have one
  if let Some(proxy_id) = &profile.proxy_id {
    if PROXY_MANAGER.get_proxy_settings_by_id(proxy_id).is_some() {
      if let Some(actual_pid) = updated_profile.process_id {
        // Update the proxy manager with the correct PID
        match PROXY_MANAGER.update_proxy_pid(1u32, actual_pid) {
          Ok(()) => {
            println!("Updated proxy PID mapping from temp (1) to actual PID: {actual_pid}");
          }
          Err(e) => {
            eprintln!("Failed to update proxy PID mapping: {e}");
          }
        }
      }
    }
  }

  Ok(updated_profile)
}

#[tauri::command]
pub async fn update_profile_proxy(
  app_handle: tauri::AppHandle,
  profile_name: String,
  proxy_id: Option<String>,
) -> Result<BrowserProfile, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .update_profile_proxy(app_handle, &profile_name, proxy_id)
    .await
    .map_err(|e| format!("Failed to update profile: {e}"))
}

#[tauri::command]
pub fn update_profile_version(
  profile_name: String,
  version: String,
) -> Result<BrowserProfile, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .update_profile_version(&profile_name, &version)
    .map_err(|e| format!("Failed to update profile version: {e}"))
}

#[tauri::command]
pub async fn check_browser_status(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
) -> Result<bool, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .check_browser_status(app_handle, &profile)
    .await
    .map_err(|e| format!("Failed to check browser status: {e}"))
}

#[tauri::command]
pub fn rename_profile(
  _app_handle: tauri::AppHandle,
  old_name: &str,
  new_name: &str,
) -> Result<BrowserProfile, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .rename_profile(old_name, new_name)
    .map_err(|e| format!("Failed to delete profile: {e}"))
}

#[tauri::command]
pub fn delete_profile(_app_handle: tauri::AppHandle, profile_name: String) -> Result<(), String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .delete_profile(profile_name.as_str())
    .map_err(|e| format!("Failed to delete profile: {e}"))
}

#[tauri::command]
pub fn get_supported_browsers() -> Result<Vec<String>, String> {
  let service = BrowserVersionService::new();
  Ok(service.get_supported_browsers())
}

#[tauri::command]
pub fn is_browser_supported_on_platform(browser_str: String) -> Result<bool, String> {
  let service = BrowserVersionService::new();
  service
    .is_browser_supported(&browser_str)
    .map_err(|e| format!("Failed to check browser support: {e}"))
}

#[tauri::command]
pub async fn fetch_browser_versions_cached_first(
  browser_str: String,
) -> Result<Vec<BrowserVersionInfo>, String> {
  let service = BrowserVersionService::new();

  // Get cached versions immediately if available
  if let Some(cached_versions) = service.get_cached_browser_versions_detailed(&browser_str) {
    // Check if we should update cache in background
    if service.should_update_cache(&browser_str) {
      // Start background update but return cached data immediately
      let service_clone = BrowserVersionService::new();
      let browser_str_clone = browser_str.clone();
      tokio::spawn(async move {
        if let Err(e) = service_clone
          .fetch_browser_versions_detailed(&browser_str_clone, false)
          .await
        {
          eprintln!("Background version update failed for {browser_str_clone}: {e}");
        }
      });
    }
    Ok(cached_versions)
  } else {
    // No cache available, fetch fresh
    service
      .fetch_browser_versions_detailed(&browser_str, false)
      .await
      .map_err(|e| format!("Failed to fetch detailed browser versions: {e}"))
  }
}

#[tauri::command]
pub async fn fetch_browser_versions_with_count_cached_first(
  browser_str: String,
) -> Result<BrowserVersionsResult, String> {
  let service = BrowserVersionService::new();

  // Get cached versions immediately if available
  if let Some(cached_versions) = service.get_cached_browser_versions(&browser_str) {
    // Check if we should update cache in background
    if service.should_update_cache(&browser_str) {
      // Start background update but return cached data immediately
      let service_clone = BrowserVersionService::new();
      let browser_str_clone = browser_str.clone();
      tokio::spawn(async move {
        if let Err(e) = service_clone
          .fetch_browser_versions_with_count(&browser_str_clone, false)
          .await
        {
          eprintln!("Background version update failed for {browser_str_clone}: {e}");
        }
      });
    }

    // Return cached data in the expected format
    Ok(BrowserVersionsResult {
      versions: cached_versions.clone(),
      new_versions_count: None, // No new versions when returning cached data
      total_versions_count: cached_versions.len(),
    })
  } else {
    // No cache available, fetch fresh
    service
      .fetch_browser_versions_with_count(&browser_str, false)
      .await
      .map_err(|e| format!("Failed to fetch browser versions: {e}"))
  }
}

#[tauri::command]
pub async fn download_browser(
  app_handle: tauri::AppHandle,
  browser_str: String,
  version: String,
) -> Result<String, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .download_browser_impl(app_handle, browser_str, version)
    .await
    .map_err(|e| format!("Failed to download browser: {e}"))
}

#[tauri::command]
pub fn is_browser_downloaded(browser_str: String, version: String) -> bool {
  let browser_runner = BrowserRunner::new();
  browser_runner.is_browser_downloaded(&browser_str, &version)
}

#[tauri::command]
pub fn check_browser_exists(browser_str: String, version: String) -> bool {
  // This is an alias for is_browser_downloaded to provide clearer semantics for auto-updates
  is_browser_downloaded(browser_str, version)
}

#[tauri::command]
pub async fn kill_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
) -> Result<(), String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .kill_browser_process(app_handle, &profile)
    .await
    .map_err(|e| format!("Failed to kill browser: {e}"))
}

#[tauri::command]
pub fn create_browser_profile_new(
  name: String,
  browser_str: String,
  version: String,
  release_type: String,
  proxy_id: Option<String>,
  camoufox_config: Option<CamoufoxConfig>,
) -> Result<BrowserProfile, String> {
  let browser_type =
    BrowserType::from_str(&browser_str).map_err(|e| format!("Invalid browser type: {e}"))?;
  create_browser_profile(
    name,
    browser_type.as_str().to_string(),
    version,
    release_type,
    proxy_id,
    camoufox_config,
  )
}

#[tauri::command]
pub async fn fetch_browser_versions_with_count(
  browser_str: String,
) -> Result<BrowserVersionsResult, String> {
  let service = BrowserVersionService::new();
  service
    .fetch_browser_versions_with_count(&browser_str, false)
    .await
    .map_err(|e| format!("Failed to fetch browser versions: {e}"))
}

#[tauri::command]
pub fn get_downloaded_browser_versions(browser_str: String) -> Result<Vec<String>, String> {
  let registry = DownloadedBrowsersRegistry::load()
    .map_err(|e| format!("Failed to load browser registry: {e}"))?;
  Ok(registry.get_downloaded_versions(&browser_str))
}

#[tauri::command]
pub async fn get_browser_release_types(
  browser_str: String,
) -> Result<crate::browser_version_service::BrowserReleaseTypes, String> {
  let service = BrowserVersionService::new();
  service
    .get_browser_release_types(&browser_str)
    .await
    .map_err(|e| format!("Failed to get release types: {e}"))
}

#[tauri::command]
pub async fn check_missing_binaries() -> Result<Vec<(String, String, String)>, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .check_missing_binaries()
    .await
    .map_err(|e| format!("Failed to check missing binaries: {e}"))
}

#[tauri::command]
pub async fn ensure_all_binaries_exist(
  app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .ensure_all_binaries_exist(&app_handle)
    .await
    .map_err(|e| format!("Failed to ensure all binaries exist: {e}"))
}

#[tauri::command]
pub async fn cleanup_unused_binaries() -> Result<Vec<String>, String> {
  let browser_runner = BrowserRunner::new();
  browser_runner
    .cleanup_unused_binaries_internal()
    .map_err(|e| format!("Failed to cleanup unused binaries: {e}"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::browser::ProxySettings;
  use tempfile::TempDir;

  fn create_test_browser_runner() -> (BrowserRunner, TempDir) {
    let temp_dir = TempDir::new().unwrap();

    // Mock the base directories by setting environment variables
    std::env::set_var("HOME", temp_dir.path());

    let browser_runner = BrowserRunner::new();
    (browser_runner, temp_dir)
  }

  #[test]
  fn test_browser_runner_creation() {
    let (_runner, _temp_dir) = create_test_browser_runner();
    // If we get here without panicking, the test passes
  }

  #[test]
  fn test_get_binaries_dir() {
    let (runner, _temp_dir) = create_test_browser_runner();
    let binaries_dir = runner.get_binaries_dir();

    assert!(binaries_dir.to_string_lossy().contains("DonutBrowser"));
    assert!(binaries_dir.to_string_lossy().contains("binaries"));
  }

  #[test]
  fn test_get_profiles_dir() {
    let (runner, _temp_dir) = create_test_browser_runner();
    let profiles_dir = runner.get_profiles_dir();

    assert!(profiles_dir.to_string_lossy().contains("DonutBrowser"));
    assert!(profiles_dir.to_string_lossy().contains("profiles"));
  }

  #[test]
  fn test_create_profile() {
    let (runner, _temp_dir) = create_test_browser_runner();

    let profile = runner
      .create_profile("Test Profile", "firefox", "139.0", "stable", None, None)
      .unwrap();

    assert_eq!(profile.name, "Test Profile");
    assert_eq!(profile.browser, "firefox");
    assert_eq!(profile.version, "139.0");
    assert!(profile.proxy_id.is_none());
    assert!(profile.process_id.is_none());
  }

  #[test]
  fn test_create_profile_with_proxy() {
    let (runner, _temp_dir) = create_test_browser_runner();

    let _proxy = ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: None,
      password: None,
    };

    let profile = runner
      .create_profile(
        "Test Profile with Proxy",
        "firefox",
        "139.0",
        "stable",
        None, // Tests now use separate proxy storage system
        None, // No camoufox config for this test
      )
      .unwrap();

    assert_eq!(profile.name, "Test Profile with Proxy");
    // Note: Proxy settings are now stored separately in the proxy storage system
    assert!(profile.proxy_id.is_none()); // No proxy assigned in this test
  }

  #[test]
  fn test_save_and_load_profile() {
    let (runner, _temp_dir) = create_test_browser_runner();

    let profile = runner
      .create_profile("Test Save Load", "firefox", "139.0", "stable", None, None)
      .unwrap();

    // Save the profile
    runner.save_profile(&profile).unwrap();

    // Load profiles and verify
    let profiles = runner.list_profiles().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "Test Save Load");
    assert_eq!(profiles[0].browser, "firefox");
    assert_eq!(profiles[0].version, "139.0");
  }

  #[test]
  fn test_rename_profile() {
    let (runner, _temp_dir) = create_test_browser_runner();

    // Create profile
    let _ = runner
      .create_profile("Original Name", "firefox", "139.0", "stable", None, None)
      .unwrap();

    // Rename profile
    let renamed_profile = runner.rename_profile("Original Name", "New Name").unwrap();

    assert_eq!(renamed_profile.name, "New Name");

    // Verify old profile is gone and new one exists
    let profiles = runner.list_profiles().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "New Name");
  }

  #[test]
  fn test_delete_profile() {
    let (runner, _temp_dir) = create_test_browser_runner();

    // Create profile
    let _ = runner
      .create_profile("To Delete", "firefox", "139.0", "stable", None, None)
      .unwrap();

    // Verify profile exists
    let profiles = runner.list_profiles().unwrap();
    assert_eq!(profiles.len(), 1);

    // Delete profile
    runner.delete_profile("To Delete").unwrap();

    // Verify profile is gone
    let profiles = runner.list_profiles().unwrap();
    assert_eq!(profiles.len(), 0);
  }

  #[test]
  fn test_profile_name_sanitization() {
    let (runner, _temp_dir) = create_test_browser_runner();

    // Create profile with spaces and special characters
    let profile = runner
      .create_profile(
        "Test Profile With Spaces",
        "firefox",
        "139.0",
        "stable",
        None,
        None,
      )
      .unwrap();

    // Profile path should contain UUID and end with /profile
    let profiles_dir = runner.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    assert!(profile_data_path
      .to_string_lossy()
      .contains(&profile.id.to_string()));
    assert!(profile_data_path.to_string_lossy().ends_with("/profile"));
    // Profile name should remain unchanged
    assert_eq!(profile.name, "Test Profile With Spaces");
    // Profile should have a valid UUID
    assert!(uuid::Uuid::parse_str(&profile.id.to_string()).is_ok());
  }

  #[test]
  fn test_multiple_profiles() {
    let (runner, _temp_dir) = create_test_browser_runner();

    // Create multiple profiles
    let _ = runner
      .create_profile("Profile 1", "firefox", "139.0", "stable", None, None)
      .unwrap();
    let _ = runner
      .create_profile("Profile 2", "chromium", "1465660", "stable", None, None)
      .unwrap();
    let _ = runner
      .create_profile("Profile 3", "brave", "v1.81.9", "stable", None, None)
      .unwrap();

    // List profiles
    let profiles = runner.list_profiles().unwrap();
    assert_eq!(profiles.len(), 3);

    let profile_names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
    assert!(profile_names.contains(&"Profile 1"));
    assert!(profile_names.contains(&"Profile 2"));
    assert!(profile_names.contains(&"Profile 3"));
  }

  #[test]
  fn test_profile_validation() {
    let (runner, _temp_dir) = create_test_browser_runner();

    // Test that we can't rename to an existing profile name
    let _ = runner
      .create_profile("Profile 1", "firefox", "139.0", "stable", None, None)
      .unwrap();
    let _ = runner
      .create_profile("Profile 2", "firefox", "139.0", "stable", None, None)
      .unwrap();

    // Try to rename profile2 to profile1's name (should fail)
    let result = runner.rename_profile("Profile 2", "Profile 1");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));
  }

  #[test]
  fn test_firefox_default_browser_preferences() {
    let (runner, _temp_dir) = create_test_browser_runner();

    // Create profile without proxy
    let profile = runner
      .create_profile(
        "Test Firefox Preferences",
        "firefox",
        "139.0",
        "stable",
        None,
        None,
      )
      .unwrap();

    // Check that user.js file was created with default browser preference
    let profiles_dir = runner.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    let user_js_path = profile_data_path.join("user.js");
    assert!(user_js_path.exists());

    let user_js_content = std::fs::read_to_string(user_js_path).unwrap();
    assert!(user_js_content.contains("browser.shell.checkDefaultBrowser"));
    assert!(user_js_content.contains("false"));

    // Verify automatic update disabling preferences are present
    assert!(user_js_content.contains("app.update.enabled"));
    assert!(user_js_content.contains("app.update.auto"));

    // Create profile with proxy (proxy object unused in new architecture)
    let _proxy = ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: None,
      password: None,
    };

    let profile_with_proxy = runner
      .create_profile(
        "Test Firefox Preferences Proxy",
        "firefox",
        "139.0",
        "stable",
        None, // Tests now use separate proxy storage system
        None, // No camoufox config for this test
      )
      .unwrap();

    // Check that user.js file contains both proxy settings and default browser preference
    let profile_with_proxy_data_path = profile_with_proxy.get_profile_data_path(&profiles_dir);
    let user_js_path_proxy = profile_with_proxy_data_path.join("user.js");
    assert!(user_js_path_proxy.exists());

    let user_js_content_proxy = std::fs::read_to_string(user_js_path_proxy).unwrap();
    assert!(user_js_content_proxy.contains("browser.shell.checkDefaultBrowser"));
    assert!(user_js_content_proxy.contains("network.proxy.type"));

    // Verify automatic update disabling preferences are present even with proxy
    assert!(user_js_content_proxy.contains("app.update.enabled"));
    assert!(user_js_content_proxy.contains("app.update.auto"));
  }
}

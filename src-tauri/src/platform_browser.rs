use crate::browser::{create_browser, BrowserType};
use crate::profile::BrowserProfile;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

// Platform-specific modules
#[cfg(target_os = "macos")]
pub mod macos {
  use super::*;

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
    // If the executable is inside an app bundle, launch via Launch Services so
    // macOS recognizes the real application for privacy permissions (e.g. Screen Recording).
    // This ensures TCC prompts are attributed to the browser app, not our launcher.
    let mut current = Some(executable_path);
    let mut app_bundle: Option<std::path::PathBuf> = None;
    while let Some(path) = current {
      if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
        if file_name.ends_with(".app") {
          app_bundle = Some(path.to_path_buf());
          break;
        }
      }
      current = path.parent();
    }

    if let Some(app_path) = app_bundle {
      // Use `open -n -a <App>.app --args ...` to launch the app bundle.
      // Note: The returned child PID will belong to `open`, not the browser.
      // The caller should resolve the actual browser PID after launch.
      let mut cmd = Command::new("open");
      cmd.arg("-n");
      cmd.arg("-a");
      cmd.arg(app_path);
      cmd.arg("--args");
      for a in args {
        cmd.arg(a);
      }
      Ok(cmd.spawn()?)
    } else {
      // Fallback: direct spawn if this is not an app bundle
      Ok(Command::new(executable_path).args(args).spawn()?)
    }
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
pub mod windows {
  use super::*;

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
    use sysinfo::{Pid, System};
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
pub mod linux {
  use super::*;

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
    use sysinfo::{Pid, System};
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

use tauri::command;

#[cfg(target_os = "macos")]
mod macos {
  use core_foundation::base::OSStatus;
  use core_foundation::string::CFStringRef;
  use core_foundation::{base::TCFType, string::CFString};

  #[link(name = "CoreServices", kind = "framework")]
  extern "C" {
    fn LSSetDefaultHandlerForURLScheme(scheme: CFStringRef, bundle_id: CFStringRef) -> OSStatus;
    fn LSCopyDefaultHandlerForURLScheme(scheme: CFStringRef) -> CFStringRef;
  }

  pub fn is_default_browser() -> Result<bool, String> {
    let schemes = ["http", "https"];
    let bundle_id = "com.donutbrowser";

    for scheme in schemes {
      let scheme_str = CFString::new(scheme);
      unsafe {
        let current_handler = LSCopyDefaultHandlerForURLScheme(scheme_str.as_concrete_TypeRef());
        if current_handler.is_null() {
          return Ok(false);
        }

        let current_handler_cf = CFString::wrap_under_create_rule(current_handler);
        let current_handler_str = current_handler_cf.to_string();

        if current_handler_str != bundle_id {
          return Ok(false);
        }
      }
    }
    Ok(true)
  }

  pub fn set_as_default_browser() -> Result<(), String> {
    let bundle_id = CFString::new("com.donutbrowser");
    let schemes = ["http", "https"];

    for scheme in schemes {
      let scheme_str = CFString::new(scheme);
      unsafe {
        let status = LSSetDefaultHandlerForURLScheme(
          scheme_str.as_concrete_TypeRef(),
          bundle_id.as_concrete_TypeRef(),
        );
        if status != 0 {
          let error_msg = match status {
            -54 => format!(
              "Failed to set as default browser for scheme '{scheme}'. The app is not properly registered as a browser. Please:\n1. Build and install the app properly\n2. Manually set Donut Browser as default in System Settings > General > Default web browser\n3. Make sure the app is in your Applications folder"
            ),
            _ => format!(
              "Failed to set as default browser for scheme '{scheme}'. Status code: {status}. Please manually set Donut Browser as default in System Settings > General > Default web browser."
            )
          };
          return Err(error_msg);
        }
      }
    }
    Ok(())
  }
}

#[cfg(target_os = "windows")]
mod windows {
  pub fn is_default_browser() -> Result<bool, String> {
    // Windows implementation would go here
    Err("Windows support not implemented yet".to_string())
  }

  pub fn set_as_default_browser() -> Result<(), String> {
    Err("Windows support not implemented yet".to_string())
  }
}

#[cfg(target_os = "linux")]
mod linux {
  use std::process::Command;

  const APP_DESKTOP_NAME: &str = "donutbrowser.desktop";

  pub fn is_default_browser() -> Result<bool, String> {
    // Check if xdg-mime is available
    if !is_xdg_mime_available() {
      return Err("xdg-mime utility not found. Please install xdg-utils package.".to_string());
    }

    let schemes = ["http", "https"];

    for scheme in schemes {
      let mime_type = format!("x-scheme-handler/{}", scheme);

      // Query the current default handler for this scheme
      let output = Command::new("xdg-mime")
        .args(["query", "default", &mime_type])
        .output()
        .map_err(|e| format!("Failed to query default handler for {}: {}", scheme, e))?;

      if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("xdg-mime query failed for {}: {}", scheme, stderr));
      }

      let current_handler = String::from_utf8_lossy(&output.stdout).trim().to_string();

      // Check if our app is the default handler
      if current_handler != APP_DESKTOP_NAME {
        return Ok(false);
      }
    }

    Ok(true)
  }

  pub fn set_as_default_browser() -> Result<(), String> {
    // Check if xdg-mime is available
    if !is_xdg_mime_available() {
      return Err("xdg-mime utility not found. Please install xdg-utils package.".to_string());
    }

    // Check if the desktop file exists in common locations
    if !check_desktop_file_exists() {
      return Err(format!(
        "Desktop file '{}' not found in standard locations. Please ensure the application is properly installed. You can manually set Donut Browser as the default browser in your system settings.",
        APP_DESKTOP_NAME
      ));
    }

    let schemes = ["http", "https"];
    let mut all_succeeded = true;
    let mut error_messages = Vec::new();

    for scheme in schemes {
      let mime_type = format!("x-scheme-handler/{}", scheme);

      // Set our app as the default handler for this scheme
      let output = Command::new("xdg-mime")
        .args(["default", APP_DESKTOP_NAME, &mime_type])
        .output()
        .map_err(|e| format!("Failed to set default handler for {}: {}", scheme, e))?;

      if !output.status.success() {
        all_succeeded = false;
        let stderr = String::from_utf8_lossy(&output.stderr);
        error_messages.push(format!("Failed to set default for {}: {}", scheme, stderr));
      }
    }

    if !all_succeeded {
      return Err(format!(
        "Some xdg-mime commands failed:\n{}\n\nYou may need to:\n1. Run with appropriate permissions\n2. Manually set the default browser in your desktop environment settings\n3. Restart your desktop session",
        error_messages.join("\n")
      ));
    }

    // Give the system a moment to process the changes
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify the changes took effect
    match is_default_browser() {
      Ok(true) => Ok(()),
      Ok(false) => {
        // This is the common case where commands succeed but verification fails
        Err(format!(
          "The xdg-mime commands completed successfully, but Donut Browser is not yet set as the default. This is common on some Linux distributions. Please try one of these options:\n\n1. Restart your desktop session and try again\n2. Log out and log back in\n3. Manually set Donut Browser as the default in your system settings:\n   - GNOME: Settings > Default Applications > Web\n   - KDE: System Settings > Applications > Default Applications > Web Browser\n   - XFCE: Settings > Preferred Applications > Web Browser\n   - Or run: xdg-settings set default-web-browser {}\n\nThe changes may take effect automatically after a desktop restart.",
          APP_DESKTOP_NAME
        ))
      }
      Err(e) => Err(format!(
        "Set as default completed, but verification failed: {}. The changes may still be in effect after restarting your desktop session.",
        e
      ))
    }
  }

  fn is_xdg_mime_available() -> bool {
    Command::new("which")
      .arg("xdg-mime")
      .output()
      .map(|output| output.status.success())
      .unwrap_or(false)
  }

  fn check_desktop_file_exists() -> bool {
    let desktop_locations = [
      "~/.local/share/applications/",
      "/usr/share/applications/",
      "/usr/local/share/applications/",
      "/var/lib/flatpak/exports/share/applications/",
      "~/.local/share/flatpak/exports/share/applications/",
    ];

    for location in &desktop_locations {
      let path = if location.starts_with('~') {
        if let Ok(home) = std::env::var("HOME") {
          location.replace('~', &home)
        } else {
          continue;
        }
      } else {
        location.to_string()
      };

      let full_path = format!("{}{}", path, APP_DESKTOP_NAME);
      if std::path::Path::new(&full_path).exists() {
        return true;
      }
    }

    false
  }
}

#[command]
pub async fn is_default_browser() -> Result<bool, String> {
  #[cfg(target_os = "macos")]
  return macos::is_default_browser();

  #[cfg(target_os = "windows")]
  return windows::is_default_browser();

  #[cfg(target_os = "linux")]
  return linux::is_default_browser();

  #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
  Err("Unsupported platform".to_string())
}

#[command]
pub async fn set_as_default_browser() -> Result<(), String> {
  #[cfg(target_os = "macos")]
  return macos::set_as_default_browser();

  #[cfg(target_os = "windows")]
  return windows::set_as_default_browser();

  #[cfg(target_os = "linux")]
  return linux::set_as_default_browser();

  #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
  Err("Unsupported platform".to_string())
}

#[tauri::command]
pub async fn open_url_with_profile(
  app_handle: tauri::AppHandle,
  profile_name: String,
  url: String,
) -> Result<(), String> {
  use crate::browser_runner::BrowserRunner;

  let runner = BrowserRunner::new();

  // Get the profile by name
  let profiles = runner
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?;
  let profile = profiles
    .into_iter()
    .find(|p| p.name == profile_name)
    .ok_or_else(|| format!("Profile '{profile_name}' not found"))?;

  println!("Opening URL '{url}' with profile '{profile_name}'");

  // Use launch_or_open_url which handles both launching new instances and opening in existing ones
  runner
    .launch_or_open_url(app_handle, &profile, Some(url.clone()))
    .await
    .map_err(|e| {
      println!("Failed to open URL with profile '{profile_name}': {e}");
      format!("Failed to open URL with profile: {e}")
    })?;

  println!("Successfully opened URL '{url}' with profile '{profile_name}'");
  Ok(())
}

#[tauri::command]
pub async fn smart_open_url(
  app_handle: tauri::AppHandle,
  url: String,
  _is_startup: Option<bool>,
) -> Result<String, String> {
  use crate::browser_runner::BrowserRunner;

  let runner = BrowserRunner::new();

  // Get all profiles
  let profiles = runner
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?;

  if profiles.is_empty() {
    return Err("no_profiles".to_string());
  }

  println!(
    "URL opening - Total profiles: {}, checking for running profiles",
    profiles.len()
  );

  // Check for running profiles and find the first one that can handle URLs
  for profile in &profiles {
    // Check if this profile is running
    let is_running = runner
      .check_browser_status(app_handle.clone(), profile)
      .await
      .unwrap_or(false);

    if is_running {
      println!(
        "Found running profile '{}', attempting to open URL",
        profile.name
      );

      // For TOR browser: Check if any other TOR browser is running
      if profile.browser == "tor-browser" {
        let mut other_tor_running = false;
        for p in &profiles {
          if p.browser == "tor-browser"
            && p.name != profile.name
            && runner
              .check_browser_status(app_handle.clone(), p)
              .await
              .unwrap_or(false)
          {
            other_tor_running = true;
            break;
          }
        }

        if other_tor_running {
          continue; // Skip this one, can't have multiple TOR instances
        }
      }

      // For Mullvad browser: skip if running (can't open URLs in running Mullvad)
      if profile.browser == "mullvad-browser" {
        continue;
      }

      // Try to open the URL with this running profile
      match runner
        .launch_or_open_url(app_handle.clone(), profile, Some(url.clone()))
        .await
      {
        Ok(_) => {
          println!(
            "Successfully opened URL '{}' with running profile '{}'",
            url, profile.name
          );
          return Ok(format!("opened_with_profile:{}", profile.name));
        }
        Err(e) => {
          println!(
            "Failed to open URL with running profile '{}': {}",
            profile.name, e
          );
          // Continue to try other profiles or show selector
        }
      }
    }
  }

  println!("No suitable running profiles found, showing profile selector");

  // No suitable running profile found, show the profile selector
  Err("show_selector".to_string())
}

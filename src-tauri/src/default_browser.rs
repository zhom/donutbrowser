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
              "Failed to set as default browser for scheme '{}'. The app is not properly registered as a browser. Please:\n1. Build and install the app properly\n2. Manually set Donut Browser as default in System Settings > General > Default web browser\n3. Make sure the app is in your Applications folder",
              scheme
            ),
            _ => format!(
              "Failed to set as default browser for scheme '{}'. Status code: {}. Please manually set Donut Browser as default in System Settings > General > Default web browser.",
              scheme, status
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
  pub fn is_default_browser() -> Result<bool, String> {
    // Linux implementation would go here
    Err("Linux support not implemented yet".to_string())
  }

  pub fn set_as_default_browser() -> Result<(), String> {
    Err("Linux support not implemented yet".to_string())
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
    .map_err(|e| format!("Failed to list profiles: {}", e))?;
  let profile = profiles
    .into_iter()
    .find(|p| p.name == profile_name)
    .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;

  println!("Opening URL '{}' with profile '{}'", url, profile_name);

  // Use launch_or_open_url which handles both launching new instances and opening in existing ones
  runner
    .launch_or_open_url(app_handle, &profile, Some(url.clone()))
    .await
    .map_err(|e| {
      println!("Failed to open URL with profile '{}': {}", profile_name, e);
      format!("Failed to open URL with profile: {}", e)
    })?;

  println!(
    "Successfully opened URL '{}' with profile '{}'",
    url, profile_name
  );
  Ok(())
}

#[tauri::command]
pub async fn smart_open_url(
  _app_handle: tauri::AppHandle,
  _url: String,
  _is_startup: Option<bool>,
) -> Result<String, String> {
  use crate::browser_runner::BrowserRunner;

  let runner = BrowserRunner::new();

  // Get all profiles
  let profiles = runner
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {}", e))?;

  if profiles.is_empty() {
    return Err("no_profiles".to_string());
  }

  println!(
    "URL opening - Total profiles: {}, showing profile selector",
    profiles.len()
  );

  // Always show the profile selector so the user can choose
  Err("show_selector".to_string())
}

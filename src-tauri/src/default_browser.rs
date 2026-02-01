use tauri::command;

pub struct DefaultBrowser {}

impl DefaultBrowser {
  fn new() -> Self {
    Self {}
  }

  pub fn instance() -> &'static DefaultBrowser {
    &DEFAULT_BROWSER
  }

  pub async fn is_default_browser(&self) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    return macos::is_default_browser();

    #[cfg(target_os = "windows")]
    return windows::is_default_browser();

    #[cfg(target_os = "linux")]
    return linux::is_default_browser();

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err("Unsupported platform".to_string())
  }

  pub async fn set_as_default_browser(&self) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::set_as_default_browser();

    #[cfg(target_os = "windows")]
    return windows::set_as_default_browser();

    #[cfg(target_os = "linux")]
    return linux::set_as_default_browser();

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err("Unsupported platform".to_string())
  }
}

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
  use std::path::Path;
  use winreg::enums::*;
  use winreg::RegKey;

  const APP_NAME: &str = "DonutBrowser";
  const PROG_ID: &str = "DonutBrowser.HTML";

  pub fn is_default_browser() -> Result<bool, String> {
    let schemes = ["http", "https"];

    for scheme in schemes {
      // Check if our browser is set as the default handler for this scheme
      if !is_default_for_scheme(scheme)? {
        return Ok(false);
      }
    }

    Ok(true)
  }

  pub fn set_as_default_browser() -> Result<(), String> {
    // Get the current executable path
    let exe_path = std::env::current_exe()
      .map_err(|e| format!("Failed to get current executable path: {}", e))?;

    let exe_path_str = exe_path
      .to_str()
      .ok_or("Failed to convert executable path to string")?;

    // Verify the executable exists
    if !Path::new(exe_path_str).exists() {
      return Err(format!("Executable not found at: {}", exe_path_str));
    }

    // Register the application
    register_application(exe_path_str)?;

    // Set as default for HTTP and HTTPS
    set_default_for_scheme("http")?;
    set_default_for_scheme("https")?;

    // Register file associations for HTML files
    register_html_file_association(exe_path_str)?;

    // Notify the system of changes
    notify_system_of_changes();

    Ok(())
  }

  fn is_default_for_scheme(scheme: &str) -> Result<bool, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Check Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice
    let path = format!(
      "Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\{}\\UserChoice",
      scheme
    );

    match hkcu.open_subkey(&path) {
      Ok(key) => match key.get_value::<String, _>("ProgId") {
        Ok(prog_id) => Ok(prog_id == PROG_ID),
        Err(_) => Ok(false),
      },
      Err(_) => Ok(false),
    }
  }

  fn register_application(exe_path: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Register in Software\RegisteredApplications
    let (registered_apps, _) = hkcu
      .create_subkey("Software\\RegisteredApplications")
      .map_err(|e| format!("Failed to create RegisteredApplications key: {}", e))?;

    registered_apps
      .set_value(APP_NAME, &format!("Software\\{}", APP_NAME))
      .map_err(|e| format!("Failed to set registered application: {}", e))?;

    // Create application key
    let (app_key, _) = hkcu
      .create_subkey(&format!("Software\\{}", APP_NAME))
      .map_err(|e| format!("Failed to create application key: {}", e))?;

    // Set application properties
    app_key
      .set_value("ApplicationName", &APP_NAME)
      .map_err(|e| format!("Failed to set ApplicationName: {}", e))?;

    app_key
      .set_value(
        "ApplicationDescription",
        &"Donut Browser - Simple Yet Powerful Anti-Detect Browser",
      )
      .map_err(|e| format!("Failed to set ApplicationDescription: {}", e))?;

    app_key
      .set_value("ApplicationIcon", &format!("{},0", exe_path))
      .map_err(|e| format!("Failed to set ApplicationIcon: {}", e))?;

    // Create Capabilities key
    let (capabilities, _) = app_key
      .create_subkey("Capabilities")
      .map_err(|e| format!("Failed to create Capabilities key: {}", e))?;

    capabilities
      .set_value(
        "ApplicationDescription",
        &"Donut Browser - Simple Yet Powerful Anti-Detect Browser",
      )
      .map_err(|e| format!("Failed to set Capabilities description: {}", e))?;

    // Set URL associations
    let (url_assoc, _) = capabilities
      .create_subkey("URLAssociations")
      .map_err(|e| format!("Failed to create URLAssociations key: {}", e))?;

    url_assoc
      .set_value("http", PROG_ID)
      .map_err(|e| format!("Failed to set http association: {}", e))?;

    url_assoc
      .set_value("https", PROG_ID)
      .map_err(|e| format!("Failed to set https association: {}", e))?;

    // Set file associations
    let (file_assoc, _) = capabilities
      .create_subkey("FileAssociations")
      .map_err(|e| format!("Failed to create FileAssociations key: {}", e))?;

    file_assoc
      .set_value(".html", PROG_ID)
      .map_err(|e| format!("Failed to set .html association: {}", e))?;

    file_assoc
      .set_value(".htm", PROG_ID)
      .map_err(|e| format!("Failed to set .htm association: {}", e))?;

    // Register the ProgID
    register_prog_id(exe_path)?;

    Ok(())
  }

  fn register_prog_id(exe_path: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Create ProgID key
    let (prog_id_key, _) = hkcu
      .create_subkey(&format!("Software\\Classes\\{}", PROG_ID))
      .map_err(|e| format!("Failed to create ProgID key: {}", e))?;

    prog_id_key
      .set_value("", &"Donut Browser Document")
      .map_err(|e| format!("Failed to set ProgID default value: {}", e))?;

    prog_id_key
      .set_value("FriendlyTypeName", &"Donut Browser Document")
      .map_err(|e| format!("Failed to set FriendlyTypeName: {}", e))?;

    // Create DefaultIcon key
    let (icon_key, _) = prog_id_key
      .create_subkey("DefaultIcon")
      .map_err(|e| format!("Failed to create DefaultIcon key: {}", e))?;

    icon_key
      .set_value("", &format!("{},0", exe_path))
      .map_err(|e| format!("Failed to set default icon: {}", e))?;

    // Create shell\open\command key
    let (command_key, _) = prog_id_key
      .create_subkey("shell\\open\\command")
      .map_err(|e| format!("Failed to create command key: {}", e))?;

    command_key
      .set_value("", &format!("\"{}\" \"%1\"", exe_path))
      .map_err(|e| format!("Failed to set command: {}", e))?;

    Ok(())
  }

  fn set_default_for_scheme(scheme: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Set in Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.html\UserChoice
    // Note: On Windows 10+, this might require elevated permissions or user interaction
    // through the Settings app due to security restrictions

    // Try to set the association in the user's choice
    let user_choice_path = format!(
      "Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\{}\\UserChoice",
      scheme
    );

    // Note: Setting UserChoice directly may not work on Windows 10+ due to hash verification
    // The user may need to manually set the default browser through Windows Settings
    match hkcu.create_subkey(&user_choice_path) {
      Ok((user_choice, _)) => {
        // Attempt to set the ProgId
        if user_choice.set_value("ProgId", PROG_ID).is_err() {
          // If we can't set UserChoice, that's expected on newer Windows versions
          // The registration is still valuable for the "Open with" menu
        }
      }
      Err(_) => {
        // Expected on newer Windows versions - user must set manually
      }
    }

    Ok(())
  }

  fn register_html_file_association(_exe_path: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Register .html and .htm file associations
    for ext in &[".html", ".htm"] {
      let ext_path = format!("Software\\Classes\\{}", ext);

      match hkcu.create_subkey(&ext_path) {
        Ok((ext_key, _)) => {
          // Set the default value to our ProgID
          let _ = ext_key.set_value("", PROG_ID);
        }
        Err(_) => {
          // Continue if we can't set the file association
        }
      }
    }

    Ok(())
  }

  fn notify_system_of_changes() {
    // Use Windows API to notify the system of association changes
    // This helps refresh the system's understanding of the changes
    unsafe {
      use std::ffi::c_void;

      const HWND_BROADCAST: *mut c_void = 0xffff as *mut c_void;
      const WM_SETTINGCHANGE: u32 = 0x001A;
      const SMTO_ABORTIFHUNG: u32 = 0x0002;

      extern "system" {
        fn SendMessageTimeoutA(
          hWnd: *mut c_void,
          Msg: u32,
          wParam: usize,
          lParam: isize,
          fuFlags: u32,
          uTimeout: u32,
          lpdwResult: *mut u32,
        ) -> isize;
      }

      let mut result: u32 = 0;

      SendMessageTimeoutA(
        HWND_BROADCAST,
        WM_SETTINGCHANGE,
        0,
        c"Software\\Classes".as_ptr() as isize,
        SMTO_ABORTIFHUNG,
        1000,
        &mut result,
      );
    }
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

// Global singleton instance
lazy_static::lazy_static! {
  static ref DEFAULT_BROWSER: DefaultBrowser = DefaultBrowser::new();
}

#[command]
pub async fn is_default_browser() -> Result<bool, String> {
  let default_browser = DefaultBrowser::instance();
  default_browser.is_default_browser().await
}

#[command]
pub async fn set_as_default_browser() -> Result<(), String> {
  let default_browser = DefaultBrowser::instance();
  default_browser.set_as_default_browser().await
}

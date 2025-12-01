use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProxySettings {
  pub proxy_type: String, // "http", "https", "socks4", or "socks5"
  pub host: String,
  pub port: u16,
  pub username: Option<String>,
  pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BrowserType {
  Chromium,
  Firefox,
  FirefoxDeveloper,
  Brave,
  Zen,
  Camoufox,
}

impl BrowserType {
  pub fn as_str(&self) -> &'static str {
    match self {
      BrowserType::Chromium => "chromium",
      BrowserType::Firefox => "firefox",
      BrowserType::FirefoxDeveloper => "firefox-developer",
      BrowserType::Brave => "brave",
      BrowserType::Zen => "zen",
      BrowserType::Camoufox => "camoufox",
    }
  }

  pub fn from_str(s: &str) -> Result<Self, String> {
    match s {
      "chromium" => Ok(BrowserType::Chromium),
      "firefox" => Ok(BrowserType::Firefox),
      "firefox-developer" => Ok(BrowserType::FirefoxDeveloper),
      "brave" => Ok(BrowserType::Brave),
      "zen" => Ok(BrowserType::Zen),
      "camoufox" => Ok(BrowserType::Camoufox),
      _ => Err(format!("Unknown browser type: {s}")),
    }
  }
}

pub trait Browser: Send + Sync {
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>>;
  fn create_launch_args(
    &self,
    profile_path: &str,
    proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>>;
  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool;
  fn prepare_executable(&self, executable_path: &Path) -> Result<(), Box<dyn std::error::Error>>;
}

// Platform-specific modules
#[cfg(target_os = "macos")]
mod macos {
  use super::*;

  pub fn get_firefox_executable_path(
    install_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Find the .app directory
    let app_path = std::fs::read_dir(install_dir)?
      .filter_map(Result::ok)
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .ok_or("Browser app not found")?;

    // Construct the browser executable path
    let mut executable_dir = app_path.path();
    executable_dir.push("Contents");
    executable_dir.push("MacOS");

    // Find the first executable in the MacOS directory
    let executable_path = std::fs::read_dir(&executable_dir)?
      .filter_map(Result::ok)
      .find(|entry| {
        let binding = entry.file_name();
        let name = binding.to_string_lossy();
        name.starts_with("firefox")
          || name.starts_with("zen")
          || name.starts_with("camoufox")
          || name.contains("Browser")
      })
      .map(|entry| entry.path())
      .ok_or("No executable found in MacOS directory")?;

    Ok(executable_path)
  }

  pub fn get_chromium_executable_path(
    install_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Find the .app directory
    let app_path = std::fs::read_dir(install_dir)?
      .filter_map(Result::ok)
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .ok_or("Browser app not found")?;

    // Construct the browser executable path
    let mut executable_dir = app_path.path();
    executable_dir.push("Contents");
    executable_dir.push("MacOS");

    // Find the first executable in the MacOS directory
    let executable_path = std::fs::read_dir(&executable_dir)?
      .filter_map(Result::ok)
      .find(|entry| {
        let binding = entry.file_name();
        let name = binding.to_string_lossy();
        name.contains("Chromium") || name.contains("Brave") || name.contains("Google Chrome")
      })
      .map(|entry| entry.path())
      .ok_or("No executable found in MacOS directory")?;

    Ok(executable_path)
  }

  pub fn is_firefox_version_downloaded(install_dir: &Path) -> bool {
    // On macOS, check for .app files
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "app") {
          return true;
        }
      }
    }
    false
  }

  pub fn is_chromium_version_downloaded(install_dir: &Path) -> bool {
    // On macOS, check for .app files
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "app") {
          return true;
        }
      }
    }
    false
  }

  pub fn prepare_executable(_executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On macOS, no special preparation needed
    Ok(())
  }
}

#[cfg(target_os = "linux")]
mod linux {
  use super::*;
  use std::os::unix::fs::PermissionsExt;

  pub fn get_firefox_executable_path(
    install_dir: &Path,
    browser_type: &BrowserType,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Expected structure examples:
    // - Firefox/Firefox Developer on Linux often extract to: install_dir/firefox/firefox
    // - Some archives may extract directly under: install_dir/firefox or install_dir/firefox-bin
    // - For some flavors we may have: install_dir/<browser_type>/<binary>
    let browser_subdir = install_dir.join(browser_type.as_str());

    // Try common firefox executable locations (nested and flat)
    let possible_executables = match browser_type {
      BrowserType::Firefox | BrowserType::FirefoxDeveloper => vec![
        // Nested "firefox/firefox" or "firefox/firefox-bin"
        install_dir.join("firefox").join("firefox"),
        install_dir.join("firefox").join("firefox-bin"),
        // Flat under version directory
        install_dir.join("firefox"),
        install_dir.join("firefox-bin"),
        // Under a subdirectory matching the browser type
        browser_subdir.join("firefox"),
        browser_subdir.join("firefox-bin"),
      ],
      BrowserType::Zen => {
        vec![browser_subdir.join("zen"), browser_subdir.join("zen-bin")]
      }
      BrowserType::Camoufox => {
        vec![
          install_dir.join("camoufox-bin"),
          install_dir.join("camoufox"),
        ]
      }
      _ => vec![],
    };

    for executable_path in &possible_executables {
      if executable_path.exists() && executable_path.is_file() {
        return Ok(executable_path.clone());
      }
    }

    Err(
      format!(
        "Executable not found for {} in {}",
        browser_type.as_str(),
        install_dir.display(),
      )
      .into(),
    )
  }

  pub fn get_chromium_executable_path(
    install_dir: &Path,
    browser_type: &BrowserType,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let possible_executables = match browser_type {
      BrowserType::Chromium => vec![
        // Direct paths (for manual installations)
        install_dir.join("chromium"),
        install_dir.join("chrome"),
        install_dir.join("chromium-browser"),
        // Subdirectory paths (for downloaded archives)
        install_dir.join("chrome-linux").join("chrome"),
        install_dir.join("chrome-linux").join("chromium"),
        install_dir.join("chromium").join("chromium"),
        install_dir.join("chromium").join("chrome"),
        // Binary subdirectory
        install_dir.join("bin").join("chromium"),
        install_dir.join("bin").join("chrome"),
      ],
      BrowserType::Brave => vec![
        install_dir.join("brave"),
        install_dir.join("brave-browser"),
        install_dir.join("brave-browser-nightly"),
        install_dir.join("brave-browser-beta"),
        // Subdirectory paths
        install_dir.join("brave").join("brave"),
        install_dir.join("brave-browser").join("brave"),
        install_dir.join("bin").join("brave"),
      ],
      _ => vec![],
    };

    for executable_path in &possible_executables {
      if executable_path.exists() && executable_path.is_file() {
        return Ok(executable_path.clone());
      }
    }

    Err(
      format!(
        "Chromium executable not found in {}/{}",
        install_dir.display(),
        browser_type.as_str()
      )
      .into(),
    )
  }

  pub fn is_firefox_version_downloaded(install_dir: &Path, browser_type: &BrowserType) -> bool {
    // Expected structure (most common):
    //   install_dir/<browser>/<binary>
    // However, Firefox Developer tarballs often extract to a "firefox" subfolder
    // rather than "firefox-developer". Support both layouts.
    let browser_subdir = install_dir.join(browser_type.as_str());

    let possible_executables = match browser_type {
      BrowserType::Firefox | BrowserType::FirefoxDeveloper => {
        vec![
          // Preferred: executable inside a subdirectory named after the browser type
          browser_subdir.join("firefox-bin"),
          browser_subdir.join("firefox"),
          // Fallback: executable inside a generic "firefox" subdirectory
          install_dir.join("firefox").join("firefox-bin"),
          install_dir.join("firefox").join("firefox"),
        ]
      }
      BrowserType::Zen => {
        vec![browser_subdir.join("zen"), browser_subdir.join("zen-bin")]
      }
      BrowserType::Camoufox => {
        vec![
          install_dir.join("camoufox-bin"),
          install_dir.join("camoufox"),
        ]
      }
      _ => vec![],
    };

    for exe_path in &possible_executables {
      if exe_path.exists() && exe_path.is_file() {
        return true;
      }
    }

    false
  }

  pub fn is_chromium_version_downloaded(install_dir: &Path, browser_type: &BrowserType) -> bool {
    let possible_executables = match browser_type {
      BrowserType::Chromium => vec![
        // Direct paths (for manual installations)
        install_dir.join("chromium"),
        install_dir.join("chrome"),
        install_dir.join("chromium-browser"),
        // Subdirectory paths (for downloaded archives)
        install_dir.join("chrome-linux").join("chrome"),
        install_dir.join("chrome-linux").join("chromium"),
        install_dir.join("chromium").join("chromium"),
        install_dir.join("chromium").join("chrome"),
        // Binary subdirectory
        install_dir.join("bin").join("chromium"),
        install_dir.join("bin").join("chrome"),
      ],
      BrowserType::Brave => vec![
        install_dir.join("brave"),
        install_dir.join("brave-browser"),
        install_dir.join("brave-browser-nightly"),
        install_dir.join("brave-browser-beta"),
        // Subdirectory paths
        install_dir.join("brave").join("brave"),
        install_dir.join("brave-browser").join("brave"),
        install_dir.join("bin").join("brave"),
      ],
      _ => vec![],
    };

    for exe_path in &possible_executables {
      if exe_path.exists() && exe_path.is_file() {
        return true;
      }
    }

    false
  }

  pub fn prepare_executable(executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On Linux, ensure the executable has proper permissions
    log::info!("Setting execute permissions for: {:?}", executable_path);

    let metadata = std::fs::metadata(executable_path)?;
    let mut permissions = metadata.permissions();

    // Add execute permissions for owner, group, and others
    let mode = permissions.mode();
    permissions.set_mode(mode | 0o755);

    std::fs::set_permissions(executable_path, permissions)?;

    log::info!(
      "Execute permissions set successfully for: {:?}",
      executable_path
    );
    Ok(())
  }
}

#[cfg(target_os = "windows")]
mod windows {
  use super::*;

  pub fn get_firefox_executable_path(
    install_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // On Windows, look for firefox.exe
    let possible_paths = [
      install_dir.join("firefox.exe"),
      install_dir.join("firefox").join("firefox.exe"),
      install_dir.join("bin").join("firefox.exe"),
    ];

    for path in &possible_paths {
      if path.exists() && path.is_file() {
        return Ok(path.clone());
      }
    }

    // Look for any .exe file that might be the browser
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "exe") {
          let name = path.file_stem().unwrap_or_default().to_string_lossy();
          if name.starts_with("firefox")
            || name.starts_with("zen")
            || name.starts_with("camoufox")
            || name.contains("browser")
          {
            return Ok(path);
          }
        }
      }
    }

    Err("Firefox executable not found in Windows installation directory".into())
  }

  pub fn get_chromium_executable_path(
    install_dir: &Path,
    browser_type: &BrowserType,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // On Windows, look for .exe files
    let possible_paths = match browser_type {
      BrowserType::Chromium => vec![
        install_dir.join("chromium.exe"),
        install_dir.join("chrome.exe"),
        install_dir.join("chromium-browser.exe"),
        install_dir.join("bin").join("chromium.exe"),
        // Common archive extraction patterns
        install_dir.join("chrome-win").join("chrome.exe"),
        install_dir.join("chromium").join("chromium.exe"),
        install_dir.join("chromium").join("chrome.exe"),
      ],
      BrowserType::Brave => vec![
        install_dir.join("brave.exe"),
        install_dir.join("brave-browser.exe"),
        install_dir.join("bin").join("brave.exe"),
        // Subdirectory patterns
        install_dir.join("brave").join("brave.exe"),
        install_dir.join("brave-browser").join("brave.exe"),
      ],
      _ => vec![],
    };

    for path in &possible_paths {
      if path.exists() && path.is_file() {
        return Ok(path.clone());
      }
    }

    // Look for any .exe file that might be the browser
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "exe") {
          let name = path.file_stem().unwrap_or_default().to_string_lossy();
          if name.contains("chromium") || name.contains("brave") || name.contains("chrome") {
            return Ok(path);
          }
        }
      }
    }

    Err("Chromium/Brave executable not found in Windows installation directory".into())
  }

  pub fn is_firefox_version_downloaded(install_dir: &Path) -> bool {
    // On Windows, check for .exe files
    let possible_executables = [
      install_dir.join("firefox.exe"),
      install_dir.join("firefox").join("firefox.exe"),
      install_dir.join("bin").join("firefox.exe"),
    ];

    for exe_path in &possible_executables {
      if exe_path.exists() && exe_path.is_file() {
        return true;
      }
    }

    // Check for any .exe file that looks like a browser
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "exe") {
          let name = path.file_stem().unwrap_or_default().to_string_lossy();
          if name.starts_with("firefox")
            || name.starts_with("zen")
            || name.starts_with("camoufox")
            || name.contains("browser")
          {
            return true;
          }
        }
      }
    }

    false
  }

  pub fn is_chromium_version_downloaded(install_dir: &Path, browser_type: &BrowserType) -> bool {
    // On Windows, check for .exe files
    let possible_executables = match browser_type {
      BrowserType::Chromium => vec![
        install_dir.join("chromium.exe"),
        install_dir.join("chrome.exe"),
        install_dir.join("chromium-browser.exe"),
        install_dir.join("bin").join("chromium.exe"),
        // Common archive extraction patterns
        install_dir.join("chrome-win").join("chrome.exe"),
        install_dir.join("chromium").join("chromium.exe"),
        install_dir.join("chromium").join("chrome.exe"),
      ],
      BrowserType::Brave => vec![
        install_dir.join("brave.exe"),
        install_dir.join("brave-browser.exe"),
        install_dir.join("bin").join("brave.exe"),
        // Subdirectory patterns
        install_dir.join("brave").join("brave.exe"),
        install_dir.join("brave-browser").join("brave.exe"),
      ],
      _ => vec![],
    };

    for exe_path in &possible_executables {
      if exe_path.exists() && exe_path.is_file() {
        return true;
      }
    }

    // Check for any .exe file that looks like the browser
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "exe") {
          let name = path.file_stem().unwrap_or_default().to_string_lossy();
          if name.contains("chromium") || name.contains("brave") || name.contains("chrome") {
            return true;
          }
        }
      }
    }

    false
  }

  pub fn prepare_executable(_executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On Windows, no special preparation needed
    Ok(())
  }
}

pub struct FirefoxBrowser {
  browser_type: BrowserType,
}

impl FirefoxBrowser {
  pub fn new(browser_type: BrowserType) -> Self {
    Self { browser_type }
  }
}

impl Browser for FirefoxBrowser {
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::get_firefox_executable_path(install_dir);

    #[cfg(target_os = "linux")]
    return linux::get_firefox_executable_path(install_dir, &self.browser_type);

    #[cfg(target_os = "windows")]
    return windows::get_firefox_executable_path(install_dir);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("Unsupported platform".into())
  }

  fn create_launch_args(
    &self,
    profile_path: &str,
    _proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut args = vec!["-profile".to_string(), profile_path.to_string()];

    // Add remote debugging if requested
    if let Some(port) = remote_debugging_port {
      args.push("--start-debugger-server".to_string());
      args.push(port.to_string());
    }

    // Add headless mode if requested
    if headless {
      args.push("--headless".to_string());
    }

    // Use -no-remote when remote debugging to avoid conflicts with existing instances
    if remote_debugging_port.is_some() {
      args.push("-no-remote".to_string());
    }

    // Firefox-based browsers use profile directory and user.js for proxy configuration
    if let Some(url) = url {
      args.push(url);
    }

    Ok(args)
  }

  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool {
    // Expected structure: binaries/<browser>/<version>
    let browser_dir = binaries_dir.join(self.browser_type.as_str()).join(version);

    log::info!("Firefox browser checking version {version} in directory: {browser_dir:?}");

    if !browser_dir.exists() {
      log::info!("Directory does not exist: {browser_dir:?}");
      return false;
    }

    log::info!("Directory exists, checking for browser files...");

    #[cfg(target_os = "macos")]
    return macos::is_firefox_version_downloaded(&browser_dir);

    #[cfg(target_os = "linux")]
    return linux::is_firefox_version_downloaded(&browser_dir, &self.browser_type);

    #[cfg(target_os = "windows")]
    return windows::is_firefox_version_downloaded(&browser_dir);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
      log::info!("Unsupported platform for browser verification");
      false
    }
  }

  fn prepare_executable(&self, executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::prepare_executable(executable_path);

    #[cfg(target_os = "linux")]
    return linux::prepare_executable(executable_path);

    #[cfg(target_os = "windows")]
    return windows::prepare_executable(executable_path);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("Unsupported platform".into())
  }
}

// Chromium-based browsers (Chromium, Brave)
pub struct ChromiumBrowser {
  #[allow(dead_code)]
  browser_type: BrowserType,
}

impl ChromiumBrowser {
  pub fn new(browser_type: BrowserType) -> Self {
    Self { browser_type }
  }
}

impl Browser for ChromiumBrowser {
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::get_chromium_executable_path(install_dir);

    #[cfg(target_os = "linux")]
    return linux::get_chromium_executable_path(install_dir, &self.browser_type);

    #[cfg(target_os = "windows")]
    return windows::get_chromium_executable_path(install_dir, &self.browser_type);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("Unsupported platform".into())
  }

  fn create_launch_args(
    &self,
    profile_path: &str,
    proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut args = vec![
      format!("--user-data-dir={}", profile_path),
      "--no-default-browser-check".to_string(),
      "--disable-background-mode".to_string(),
      "--disable-component-update".to_string(),
      "--disable-background-timer-throttling".to_string(),
      "--crash-server-url=".to_string(),
      "--disable-updater".to_string(),
      // Disable quit confirmation and session restore prompts
      "--disable-session-crashed-bubble".to_string(),
      "--hide-crash-restore-bubble".to_string(),
      "--disable-infobars".to_string(),
      // Disable QUIC/HTTP3 to ensure traffic goes through HTTP proxy
      "--disable-quic".to_string(),
    ];

    // Add remote debugging if requested
    if let Some(port) = remote_debugging_port {
      args.push("--remote-debugging-address=0.0.0.0".to_string());
      args.push(format!("--remote-debugging-port={port}"));
    }

    // Add headless mode if requested
    if headless {
      args.push("--headless".to_string());
    }

    // Add proxy configuration if provided
    if let Some(proxy) = proxy_settings {
      args.push(format!(
        "--proxy-server=http://{}:{}",
        proxy.host, proxy.port
      ));
    }

    if let Some(url) = url {
      args.push(url);
    }

    Ok(args)
  }

  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool {
    // Expected structure: binaries/<browser>/<version>
    let browser_dir = binaries_dir.join(self.browser_type.as_str()).join(version);

    log::info!("Chromium browser checking version {version} in directory: {browser_dir:?}");

    if !browser_dir.exists() {
      log::info!("Directory does not exist: {browser_dir:?}");
      return false;
    }

    log::info!("Directory exists, checking for browser files...");

    #[cfg(target_os = "macos")]
    return macos::is_chromium_version_downloaded(&browser_dir);

    #[cfg(target_os = "linux")]
    return linux::is_chromium_version_downloaded(&browser_dir, &self.browser_type);

    #[cfg(target_os = "windows")]
    return windows::is_chromium_version_downloaded(&browser_dir, &self.browser_type);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
      log::info!("Unsupported platform for browser verification");
      false
    }
  }

  fn prepare_executable(&self, executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::prepare_executable(executable_path);

    #[cfg(target_os = "linux")]
    return linux::prepare_executable(executable_path);

    #[cfg(target_os = "windows")]
    return windows::prepare_executable(executable_path);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("Unsupported platform".into())
  }
}

pub struct CamoufoxBrowser;

impl CamoufoxBrowser {
  pub fn new() -> Self {
    Self
  }
}

impl Browser for CamoufoxBrowser {
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::get_firefox_executable_path(install_dir);

    #[cfg(target_os = "linux")]
    return linux::get_firefox_executable_path(install_dir, &BrowserType::Camoufox);

    #[cfg(target_os = "windows")]
    return windows::get_firefox_executable_path(install_dir);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("Unsupported platform".into())
  }

  fn create_launch_args(
    &self,
    profile_path: &str,
    _proxy_settings: Option<&ProxySettings>,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // For Camoufox, we handle launching through the camoufox launcher
    // This method won't be used directly, but we provide basic Firefox args as fallback
    let mut args = vec![
      "-profile".to_string(),
      profile_path.to_string(),
      "-no-remote".to_string(),
    ];

    // Add remote debugging if requested
    if let Some(port) = remote_debugging_port {
      args.push("--start-debugger-server".to_string());
      args.push(port.to_string());
    }

    // Add headless mode if requested
    if headless {
      args.push("--headless".to_string());
    }

    if let Some(url) = url {
      args.push(url);
    }

    Ok(args)
  }

  fn is_version_downloaded(&self, version: &str, binaries_dir: &Path) -> bool {
    let install_dir = binaries_dir.join("camoufox").join(version);

    #[cfg(target_os = "macos")]
    return macos::is_firefox_version_downloaded(&install_dir);

    #[cfg(target_os = "linux")]
    return linux::is_firefox_version_downloaded(&install_dir, &BrowserType::Camoufox);

    #[cfg(target_os = "windows")]
    return windows::is_firefox_version_downloaded(&install_dir);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    false
  }

  fn prepare_executable(&self, executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::prepare_executable(executable_path);

    #[cfg(target_os = "linux")]
    return linux::prepare_executable(executable_path);

    #[cfg(target_os = "windows")]
    return windows::prepare_executable(executable_path);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    Err("Unsupported platform".into())
  }
}

pub struct BrowserFactory;

impl BrowserFactory {
  fn new() -> Self {
    Self
  }

  pub fn instance() -> &'static BrowserFactory {
    &BROWSER_FACTORY
  }

  pub fn create_browser(&self, browser_type: BrowserType) -> Box<dyn Browser> {
    match browser_type {
      BrowserType::Firefox | BrowserType::FirefoxDeveloper | BrowserType::Zen => {
        Box::new(FirefoxBrowser::new(browser_type))
      }
      BrowserType::Chromium | BrowserType::Brave => Box::new(ChromiumBrowser::new(browser_type)),
      BrowserType::Camoufox => Box::new(CamoufoxBrowser::new()),
    }
  }
}

// Factory function to create browser instances (kept for backward compatibility)
pub fn create_browser(browser_type: BrowserType) -> Box<dyn Browser> {
  BrowserFactory::instance().create_browser(browser_type)
}

// Add GithubRelease and GithubAsset structs to browser.rs if they don't already exist
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GithubRelease {
  pub tag_name: String,
  #[serde(default)]
  pub name: String,
  pub assets: Vec<GithubAsset>,
  #[serde(default)]
  pub published_at: String,
  #[serde(default)]
  pub is_nightly: bool,
  #[serde(default)]
  pub prerelease: bool,
  #[serde(default)]
  pub draft: bool,
  #[serde(default)]
  pub body: Option<String>,
  #[serde(default)]
  pub html_url: Option<String>,
  #[serde(default)]
  pub id: Option<u64>,
  #[serde(default)]
  pub node_id: Option<String>,
  #[serde(default)]
  pub target_commitish: Option<String>,
  #[serde(default)]
  pub created_at: Option<String>,
  #[serde(default)]
  pub tarball_url: Option<String>,
  #[serde(default)]
  pub zipball_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GithubAsset {
  pub name: String,
  pub browser_download_url: String,
  #[serde(default)]
  pub size: u64,
  #[serde(default)]
  pub download_count: Option<u64>,
  #[serde(default)]
  pub id: Option<u64>,
  #[serde(default)]
  pub node_id: Option<String>,
  #[serde(default)]
  pub label: Option<String>,
  #[serde(default)]
  pub content_type: Option<String>,
  #[serde(default)]
  pub state: Option<String>,
  #[serde(default)]
  pub created_at: Option<String>,
  #[serde(default)]
  pub updated_at: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  #[test]
  fn test_browser_type_conversions() {
    // Test as_str
    assert_eq!(BrowserType::Firefox.as_str(), "firefox");
    assert_eq!(BrowserType::FirefoxDeveloper.as_str(), "firefox-developer");
    assert_eq!(BrowserType::Chromium.as_str(), "chromium");
    assert_eq!(BrowserType::Brave.as_str(), "brave");
    assert_eq!(BrowserType::Zen.as_str(), "zen");
    assert_eq!(BrowserType::Camoufox.as_str(), "camoufox");

    // Test from_str - use expect with descriptive messages instead of unwrap
    assert_eq!(
      BrowserType::from_str("firefox").expect("firefox should be valid"),
      BrowserType::Firefox
    );
    assert_eq!(
      BrowserType::from_str("firefox-developer").expect("firefox-developer should be valid"),
      BrowserType::FirefoxDeveloper
    );
    assert_eq!(
      BrowserType::from_str("chromium").expect("chromium should be valid"),
      BrowserType::Chromium
    );
    assert_eq!(
      BrowserType::from_str("brave").expect("brave should be valid"),
      BrowserType::Brave
    );
    assert_eq!(
      BrowserType::from_str("zen").expect("zen should be valid"),
      BrowserType::Zen
    );
    assert_eq!(
      BrowserType::from_str("camoufox").expect("camoufox should be valid"),
      BrowserType::Camoufox
    );

    // Test invalid browser type - these should properly fail
    let invalid_result = BrowserType::from_str("invalid");
    assert!(
      invalid_result.is_err(),
      "Invalid browser type should return error"
    );

    let empty_result = BrowserType::from_str("");
    assert!(empty_result.is_err(), "Empty string should return error");

    let case_sensitive_result = BrowserType::from_str("Firefox");
    assert!(
      case_sensitive_result.is_err(),
      "Case sensitive check should fail"
    );
  }

  #[test]
  fn test_firefox_launch_args() {
    // Test regular Firefox (should not use -no-remote for normal launch)
    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    let args = browser
      .create_launch_args("/path/to/profile", None, None, None, false)
      .expect("Failed to create launch args for Firefox");
    assert_eq!(args, vec!["-profile", "/path/to/profile"]);
    assert!(
      !args.contains(&"-no-remote".to_string()),
      "Firefox should not use -no-remote for normal launch"
    );

    let args = browser
      .create_launch_args(
        "/path/to/profile",
        None,
        Some("https://example.com".to_string()),
        None,
        false,
      )
      .expect("Failed to create launch args for Firefox with URL");
    assert_eq!(
      args,
      vec!["-profile", "/path/to/profile", "https://example.com"]
    );

    // Test Firefox with remote debugging (should use -no-remote)
    let args = browser
      .create_launch_args("/path/to/profile", None, None, Some(9222), false)
      .expect("Failed to create launch args for Firefox with remote debugging");
    assert!(
      args.contains(&"-no-remote".to_string()),
      "Firefox should use -no-remote for remote debugging"
    );
    assert!(
      args.contains(&"--start-debugger-server".to_string()),
      "Firefox should include debugger server arg"
    );
    assert!(
      args.contains(&"9222".to_string()),
      "Firefox should include debugging port"
    );

    // Test Zen Browser (no special flags without remote debugging)
    let browser = FirefoxBrowser::new(BrowserType::Zen);
    let args = browser
      .create_launch_args("/path/to/profile", None, None, None, false)
      .expect("Failed to create launch args for Zen Browser");
    assert_eq!(args, vec!["-profile", "/path/to/profile"]);

    // Test headless mode
    let args = browser
      .create_launch_args("/path/to/profile", None, None, None, true)
      .expect("Failed to create launch args for Zen Browser headless");
    assert!(
      args.contains(&"--headless".to_string()),
      "Browser should include headless flag when requested"
    );
  }

  #[test]
  fn test_chromium_launch_args() {
    let browser = ChromiumBrowser::new(BrowserType::Chromium);
    let args = browser
      .create_launch_args("/path/to/profile", None, None, None, false)
      .expect("Failed to create launch args for Chromium");

    // Test that basic required arguments are present
    assert!(
      args.contains(&"--user-data-dir=/path/to/profile".to_string()),
      "Chromium args should contain user-data-dir"
    );
    assert!(
      args.contains(&"--no-default-browser-check".to_string()),
      "Chromium args should contain no-default-browser-check"
    );

    // Test that automatic update disabling arguments are present
    assert!(
      args.contains(&"--disable-background-mode".to_string()),
      "Chromium args should contain disable-background-mode"
    );
    assert!(
      args.contains(&"--disable-component-update".to_string()),
      "Chromium args should contain disable-component-update"
    );

    let args_with_url = browser
      .create_launch_args(
        "/path/to/profile",
        None,
        Some("https://example.com".to_string()),
        None,
        false,
      )
      .expect("Failed to create launch args for Chromium with URL");
    assert!(
      args_with_url.contains(&"https://example.com".to_string()),
      "Chromium args should contain the URL"
    );

    // Verify URL is at the end
    assert_eq!(
      args_with_url.last().expect("Args should not be empty"),
      "https://example.com"
    );

    // Test remote debugging
    let args_with_debug = browser
      .create_launch_args("/path/to/profile", None, None, Some(9222), false)
      .expect("Failed to create launch args for Chromium with remote debugging");
    assert!(
      args_with_debug.contains(&"--remote-debugging-port=9222".to_string()),
      "Chromium args should contain remote debugging port"
    );
    assert!(
      args_with_debug.contains(&"--remote-debugging-address=0.0.0.0".to_string()),
      "Chromium args should contain remote debugging address"
    );

    // Test headless mode
    let args_headless = browser
      .create_launch_args("/path/to/profile", None, None, None, true)
      .expect("Failed to create launch args for Chromium headless");
    assert!(
      args_headless.contains(&"--headless".to_string()),
      "Chromium args should contain headless flag when requested"
    );
  }

  #[test]
  fn test_proxy_settings_creation() {
    let proxy = ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: None,
      password: None,
    };

    assert_eq!(proxy.proxy_type, "http");
    assert_eq!(proxy.host, "127.0.0.1");
    assert_eq!(proxy.port, 8080);

    // Test different proxy types
    let socks_proxy = ProxySettings {
      proxy_type: "socks5".to_string(),
      host: "proxy.example.com".to_string(),
      port: 1080,
      username: None,
      password: None,
    };

    assert_eq!(socks_proxy.proxy_type, "socks5");
    assert_eq!(socks_proxy.host, "proxy.example.com");
    assert_eq!(socks_proxy.port, 1080);
  }

  #[test]
  fn test_version_downloaded_check() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let binaries_dir = temp_dir.path();

    // Create a mock Firefox browser installation with new path structure: binaries/<browser>/<version>/
    let browser_dir = binaries_dir.join("firefox").join("139.0");
    fs::create_dir_all(&browser_dir).expect("Failed to create browser directory");

    #[cfg(target_os = "macos")]
    {
      // Create a mock .app directory for macOS
      let app_dir = browser_dir.join("Firefox.app");
      fs::create_dir_all(&app_dir).expect("Failed to create Firefox.app directory");
    }

    #[cfg(target_os = "linux")]
    {
      // Create a mock firefox subdirectory and executable for Linux
      let firefox_subdir = browser_dir.join("firefox");
      fs::create_dir_all(&firefox_subdir).expect("Failed to create firefox subdirectory");
      let executable_path = firefox_subdir.join("firefox");
      fs::write(&executable_path, "mock executable").expect("Failed to write mock executable");

      // Set executable permissions on Linux
      use std::os::unix::fs::PermissionsExt;
      let mut permissions = executable_path
        .metadata()
        .expect("Failed to get file metadata")
        .permissions();
      permissions.set_mode(0o755);
      fs::set_permissions(&executable_path, permissions)
        .expect("Failed to set executable permissions");
    }

    #[cfg(target_os = "windows")]
    {
      // Create a mock firefox.exe for Windows
      let executable_path = browser_dir.join("firefox.exe");
      fs::write(&executable_path, "mock executable").expect("Failed to write mock executable");
    }

    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    assert!(browser.is_version_downloaded("139.0", binaries_dir));
    assert!(!browser.is_version_downloaded("140.0", binaries_dir));

    // Test with Chromium browser with new path structure
    let chromium_dir = binaries_dir.join("chromium").join("1465660");
    fs::create_dir_all(&chromium_dir).expect("Failed to create chromium directory");

    #[cfg(target_os = "macos")]
    {
      let chromium_app_dir = chromium_dir.join("Chromium.app");
      fs::create_dir_all(chromium_app_dir.join("Contents").join("MacOS"))
        .expect("Failed to create Chromium.app structure");

      // Create a mock executable
      let executable_path = chromium_app_dir
        .join("Contents")
        .join("MacOS")
        .join("Chromium");
      fs::write(&executable_path, "mock executable")
        .expect("Failed to write mock Chromium executable");
    }

    #[cfg(target_os = "linux")]
    {
      // Create a mock chromium executable for Linux
      let executable_path = chromium_dir.join("chromium");
      fs::write(&executable_path, "mock executable")
        .expect("Failed to write mock chromium executable");

      // Set executable permissions on Linux
      use std::os::unix::fs::PermissionsExt;
      let mut permissions = executable_path
        .metadata()
        .expect("Failed to get chromium metadata")
        .permissions();
      permissions.set_mode(0o755);
      fs::set_permissions(&executable_path, permissions)
        .expect("Failed to set chromium permissions");
    }

    #[cfg(target_os = "windows")]
    {
      // Create a mock chromium.exe for Windows
      let executable_path = chromium_dir.join("chromium.exe");
      fs::write(&executable_path, "mock executable").expect("Failed to write mock chromium.exe");
    }

    let chromium_browser = ChromiumBrowser::new(BrowserType::Chromium);
    assert!(
      chromium_browser.is_version_downloaded("1465660", binaries_dir),
      "Chromium version should be detected as downloaded"
    );
    assert!(
      !chromium_browser.is_version_downloaded("1465661", binaries_dir),
      "Non-existent Chromium version should not be detected as downloaded"
    );
  }

  #[test]
  fn test_version_downloaded_no_app_directory() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let binaries_dir = temp_dir.path();

    // Create browser directory but no proper executable structure
    let browser_dir = binaries_dir.join("firefox").join("139.0");
    fs::create_dir_all(&browser_dir).expect("Failed to create browser directory");

    // Create some other files but no proper executable structure
    fs::write(browser_dir.join("readme.txt"), "Some content").expect("Failed to write readme file");

    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    assert!(
      !browser.is_version_downloaded("139.0", binaries_dir),
      "Firefox version should not be detected without proper executable structure"
    );
  }

  #[test]
  fn test_browser_type_clone_and_debug() {
    let browser_type = BrowserType::Firefox;
    let cloned = browser_type.clone();
    assert_eq!(browser_type, cloned);

    // Test Debug trait
    let debug_str = format!("{browser_type:?}");
    assert!(debug_str.contains("Firefox"));
  }

  #[test]
  fn test_proxy_settings_serialization() {
    let proxy = ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: None,
      password: None,
    };

    // Test that it can be serialized (implements Serialize)
    let json = serde_json::to_string(&proxy).expect("Failed to serialize proxy settings");
    assert!(json.contains("127.0.0.1"), "JSON should contain host IP");
    assert!(json.contains("8080"), "JSON should contain port number");
    assert!(json.contains("http"), "JSON should contain proxy type");

    // Test that it can be deserialized (implements Deserialize)
    let deserialized: ProxySettings =
      serde_json::from_str(&json).expect("Failed to deserialize proxy settings");
    assert_eq!(
      deserialized.proxy_type, proxy.proxy_type,
      "Proxy type should match"
    );
    assert_eq!(deserialized.host, proxy.host, "Host should match");
    assert_eq!(deserialized.port, proxy.port, "Port should match");
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_FACTORY: BrowserFactory = BrowserFactory::new();
}

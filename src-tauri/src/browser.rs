use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySettings {
  pub proxy_type: String, // "http", "https", "socks4", or "socks5"
  pub host: String,
  pub port: u16,
  pub username: Option<String>,
  pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BrowserType {
  MullvadBrowser,
  Chromium,
  Firefox,
  FirefoxDeveloper,
  Brave,
  Zen,
  TorBrowser,
  Camoufox,
}

impl BrowserType {
  pub fn as_str(&self) -> &'static str {
    match self {
      BrowserType::MullvadBrowser => "mullvad-browser",
      BrowserType::Chromium => "chromium",
      BrowserType::Firefox => "firefox",
      BrowserType::FirefoxDeveloper => "firefox-developer",
      BrowserType::Brave => "brave",
      BrowserType::Zen => "zen",
      BrowserType::TorBrowser => "tor-browser",
      BrowserType::Camoufox => "camoufox",
    }
  }

  pub fn from_str(s: &str) -> Result<Self, String> {
    match s {
      "mullvad-browser" => Ok(BrowserType::MullvadBrowser),
      "chromium" => Ok(BrowserType::Chromium),
      "firefox" => Ok(BrowserType::Firefox),
      "firefox-developer" => Ok(BrowserType::FirefoxDeveloper),
      "brave" => Ok(BrowserType::Brave),
      "zen" => Ok(BrowserType::Zen),
      "tor-browser" => Ok(BrowserType::TorBrowser),
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
          || name.starts_with("mullvad")
          || name.starts_with("zen")
          || name.starts_with("tor")
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
    // Expected structure: install_dir/<browser>/<binary>
    let browser_subdir = install_dir.join(browser_type.as_str());

    // Try firefox first (preferred), then firefox-bin
    let possible_executables = match browser_type {
      BrowserType::Firefox | BrowserType::FirefoxDeveloper => {
        vec![
          browser_subdir.join("firefox"),
          browser_subdir.join("firefox-bin"),
        ]
      }
      BrowserType::MullvadBrowser => {
        vec![
          browser_subdir.join("firefox"),
          browser_subdir.join("mullvad-browser"),
          browser_subdir.join("firefox-bin"),
        ]
      }
      BrowserType::Zen => {
        vec![browser_subdir.join("zen"), browser_subdir.join("zen-bin")]
      }
      BrowserType::TorBrowser => {
        vec![
          browser_subdir.join("firefox"),
          browser_subdir.join("tor-browser"),
          browser_subdir.join("firefox-bin"),
        ]
      }
      BrowserType::Camoufox => {
        vec![
          browser_subdir.join("camoufox-bin"),
          browser_subdir.join("camoufox"),
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
        "Firefox executable not found in {}/{}",
        install_dir.display(),
        browser_type.as_str()
      )
      .into(),
    )
  }

  pub fn get_chromium_executable_path(
    install_dir: &Path,
    browser_type: &BrowserType,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let possible_executables = match browser_type {
      BrowserType::Chromium => vec![install_dir.join("chromium"), install_dir.join("chrome")],
      BrowserType::Brave => vec![
        install_dir.join("brave"),
        install_dir.join("brave-browser"),
        install_dir.join("brave-browser-nightly"),
        install_dir.join("brave-browser-beta"),
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
    // Expected structure: install_dir/<browser>/<binary>
    let browser_subdir = install_dir.join(browser_type.as_str());

    if !browser_subdir.exists() || !browser_subdir.is_dir() {
      return false;
    }

    let possible_executables = match browser_type {
      BrowserType::Firefox | BrowserType::FirefoxDeveloper => {
        vec![
          browser_subdir.join("firefox-bin"),
          browser_subdir.join("firefox"),
        ]
      }
      BrowserType::MullvadBrowser => {
        vec![
          browser_subdir.join("mullvad-browser"),
          browser_subdir.join("firefox-bin"),
          browser_subdir.join("firefox"),
        ]
      }
      BrowserType::Zen => {
        vec![browser_subdir.join("zen"), browser_subdir.join("zen-bin")]
      }
      BrowserType::TorBrowser => {
        vec![
          browser_subdir.join("tor-browser"),
          browser_subdir.join("firefox-bin"),
          browser_subdir.join("firefox"),
        ]
      }
      BrowserType::Camoufox => {
        vec![
          browser_subdir.join("camoufox-bin"),
          browser_subdir.join("camoufox"),
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
      BrowserType::Chromium => vec![install_dir.join("chromium"), install_dir.join("chrome")],
      BrowserType::Brave => vec![
        install_dir.join("brave"),
        install_dir.join("brave-browser"),
        install_dir.join("brave-browser-nightly"),
        install_dir.join("brave-browser-beta"),
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
    println!("Setting execute permissions for: {:?}", executable_path);

    let metadata = std::fs::metadata(executable_path)?;
    let mut permissions = metadata.permissions();

    // Add execute permissions for owner, group, and others
    let mode = permissions.mode();
    permissions.set_mode(mode | 0o755);

    std::fs::set_permissions(executable_path, permissions)?;

    println!(
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
            || name.starts_with("mullvad")
            || name.starts_with("zen")
            || name.starts_with("tor")
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
      ],
      BrowserType::Brave => vec![
        install_dir.join("brave.exe"),
        install_dir.join("brave-browser.exe"),
        install_dir.join("bin").join("brave.exe"),
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
            || name.starts_with("mullvad")
            || name.starts_with("zen")
            || name.starts_with("tor")
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
      ],
      BrowserType::Brave => vec![
        install_dir.join("brave.exe"),
        install_dir.join("brave-browser.exe"),
        install_dir.join("bin").join("brave.exe"),
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
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut args = vec!["-profile".to_string(), profile_path.to_string()];

    // Only use -no-remote for browsers that require it for security (Mullvad, Tor)
    // Regular Firefox browsers can use remote commands for better URL handling
    match self.browser_type {
      BrowserType::MullvadBrowser | BrowserType::TorBrowser => {
        args.push("-no-remote".to_string());
      }
      BrowserType::Firefox
      | BrowserType::FirefoxDeveloper
      | BrowserType::Zen
      | BrowserType::Camoufox => {
        // Don't use -no-remote so we can communicate with existing instances
      }
      _ => {}
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

    println!("Firefox browser checking version {version} in directory: {browser_dir:?}");

    if !browser_dir.exists() {
      println!("Directory does not exist: {browser_dir:?}");
      return false;
    }

    println!("Directory exists, checking for browser files...");

    #[cfg(target_os = "macos")]
    return macos::is_firefox_version_downloaded(&browser_dir);

    #[cfg(target_os = "linux")]
    return linux::is_firefox_version_downloaded(&browser_dir, &self.browser_type);

    #[cfg(target_os = "windows")]
    return windows::is_firefox_version_downloaded(&browser_dir);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
      println!("Unsupported platform for browser verification");
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
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut args = vec![
      format!("--user-data-dir={}", profile_path),
      "--no-default-browser-check".to_string(),
      "--disable-background-mode".to_string(),
      "--disable-component-update".to_string(),
      "--disable-background-timer-throttling".to_string(),
      "--crash-server-url=".to_string(),
      "--disable-updater".to_string(),
    ];

    // Add proxy configuration if provided
    if let Some(proxy) = proxy_settings {
      // Apply proxy settings
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

    println!("Chromium browser checking version {version} in directory: {browser_dir:?}");

    if !browser_dir.exists() {
      println!("Directory does not exist: {browser_dir:?}");
      return false;
    }

    println!("Directory exists, checking for browser files...");

    #[cfg(target_os = "macos")]
    return macos::is_chromium_version_downloaded(&browser_dir);

    #[cfg(target_os = "linux")]
    return linux::is_chromium_version_downloaded(&browser_dir, &self.browser_type);

    #[cfg(target_os = "windows")]
    return windows::is_chromium_version_downloaded(&browser_dir, &self.browser_type);

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
      println!("Unsupported platform for browser verification");
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
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // For Camoufox, we handle launching through the camoufox launcher
    // This method won't be used directly, but we provide basic Firefox args as fallback
    let mut args = vec![
      "-profile".to_string(),
      profile_path.to_string(),
      "-no-remote".to_string(),
    ];

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

// Factory function to create browser instances
pub fn create_browser(browser_type: BrowserType) -> Box<dyn Browser> {
  match browser_type {
    BrowserType::MullvadBrowser
    | BrowserType::Firefox
    | BrowserType::FirefoxDeveloper
    | BrowserType::Zen
    | BrowserType::TorBrowser => Box::new(FirefoxBrowser::new(browser_type)),
    BrowserType::Chromium | BrowserType::Brave => Box::new(ChromiumBrowser::new(browser_type)),
    BrowserType::Camoufox => Box::new(CamoufoxBrowser::new()),
  }
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
    assert_eq!(BrowserType::MullvadBrowser.as_str(), "mullvad-browser");
    assert_eq!(BrowserType::Firefox.as_str(), "firefox");
    assert_eq!(BrowserType::FirefoxDeveloper.as_str(), "firefox-developer");
    assert_eq!(BrowserType::Chromium.as_str(), "chromium");
    assert_eq!(BrowserType::Brave.as_str(), "brave");
    assert_eq!(BrowserType::Zen.as_str(), "zen");
    assert_eq!(BrowserType::TorBrowser.as_str(), "tor-browser");
    assert_eq!(BrowserType::Camoufox.as_str(), "camoufox");

    // Test from_str
    assert_eq!(
      BrowserType::from_str("mullvad-browser").unwrap(),
      BrowserType::MullvadBrowser
    );
    assert_eq!(
      BrowserType::from_str("firefox").unwrap(),
      BrowserType::Firefox
    );
    assert_eq!(
      BrowserType::from_str("firefox-developer").unwrap(),
      BrowserType::FirefoxDeveloper
    );
    assert_eq!(
      BrowserType::from_str("chromium").unwrap(),
      BrowserType::Chromium
    );
    assert_eq!(BrowserType::from_str("brave").unwrap(), BrowserType::Brave);
    assert_eq!(BrowserType::from_str("zen").unwrap(), BrowserType::Zen);
    assert_eq!(
      BrowserType::from_str("tor-browser").unwrap(),
      BrowserType::TorBrowser
    );
    assert_eq!(
      BrowserType::from_str("camoufox").unwrap(),
      BrowserType::Camoufox
    );

    // Test invalid browser type
    assert!(BrowserType::from_str("invalid").is_err());
    assert!(BrowserType::from_str("").is_err());
    assert!(BrowserType::from_str("Firefox").is_err()); // Case sensitive
  }

  #[test]
  fn test_firefox_launch_args() {
    // Test regular Firefox (should not use -no-remote)
    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();
    assert_eq!(args, vec!["-profile", "/path/to/profile"]);
    assert!(!args.contains(&"-no-remote".to_string()));

    let args = browser
      .create_launch_args(
        "/path/to/profile",
        None,
        Some("https://example.com".to_string()),
      )
      .unwrap();
    assert_eq!(
      args,
      vec!["-profile", "/path/to/profile", "https://example.com"]
    );

    // Test Mullvad Browser (should use -no-remote)
    let browser = FirefoxBrowser::new(BrowserType::MullvadBrowser);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();
    assert_eq!(args, vec!["-profile", "/path/to/profile", "-no-remote"]);

    // Test Tor Browser (should use -no-remote)
    let browser = FirefoxBrowser::new(BrowserType::TorBrowser);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();
    assert_eq!(args, vec!["-profile", "/path/to/profile", "-no-remote"]);

    // Test Zen Browser (should not use -no-remote)
    let browser = FirefoxBrowser::new(BrowserType::Zen);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();
    assert_eq!(args, vec!["-profile", "/path/to/profile"]);
    assert!(!args.contains(&"-no-remote".to_string()));
  }

  #[test]
  fn test_chromium_launch_args() {
    let browser = ChromiumBrowser::new(BrowserType::Chromium);
    let args = browser
      .create_launch_args("/path/to/profile", None, None)
      .unwrap();

    // Test that basic required arguments are present
    assert!(args.contains(&"--user-data-dir=/path/to/profile".to_string()));
    assert!(args.contains(&"--no-default-browser-check".to_string()));

    // Test that automatic update disabling arguments are present
    assert!(args.contains(&"--disable-background-mode".to_string()));
    assert!(args.contains(&"--disable-component-update".to_string()));

    let args_with_url = browser
      .create_launch_args(
        "/path/to/profile",
        None,
        Some("https://example.com".to_string()),
      )
      .unwrap();
    assert!(args_with_url.contains(&"https://example.com".to_string()));

    // Verify URL is at the end
    assert_eq!(args_with_url.last().unwrap(), "https://example.com");
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
    let temp_dir = TempDir::new().unwrap();
    let binaries_dir = temp_dir.path();

    // Create a mock Firefox browser installation with new path structure: binaries/<browser>/<version>/
    let browser_dir = binaries_dir.join("firefox").join("139.0");
    fs::create_dir_all(&browser_dir).unwrap();

    // Create a mock .app directory
    let app_dir = browser_dir.join("Firefox.app");
    fs::create_dir_all(&app_dir).unwrap();

    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    assert!(browser.is_version_downloaded("139.0", binaries_dir));
    assert!(!browser.is_version_downloaded("140.0", binaries_dir));

    // Test with Chromium browser with new path structure
    let chromium_dir = binaries_dir.join("chromium").join("1465660");
    fs::create_dir_all(&chromium_dir).unwrap();
    let chromium_app_dir = chromium_dir.join("Chromium.app");
    fs::create_dir_all(chromium_app_dir.join("Contents").join("MacOS")).unwrap();

    // Create a mock executable
    let executable_path = chromium_app_dir
      .join("Contents")
      .join("MacOS")
      .join("Chromium");
    fs::write(&executable_path, "mock executable").unwrap();

    let chromium_browser = ChromiumBrowser::new(BrowserType::Chromium);
    assert!(chromium_browser.is_version_downloaded("1465660", binaries_dir));
    assert!(!chromium_browser.is_version_downloaded("1465661", binaries_dir));
  }

  #[test]
  fn test_version_downloaded_no_app_directory() {
    let temp_dir = TempDir::new().unwrap();
    let binaries_dir = temp_dir.path();

    // Create browser directory but no .app directory with new path structure
    let browser_dir = binaries_dir.join("firefox").join("139.0");
    fs::create_dir_all(&browser_dir).unwrap();

    // Create some other files but no .app
    fs::write(browser_dir.join("readme.txt"), "Some content").unwrap();

    let browser = FirefoxBrowser::new(BrowserType::Firefox);
    assert!(!browser.is_version_downloaded("139.0", binaries_dir));
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
    let json = serde_json::to_string(&proxy).unwrap();
    assert!(json.contains("127.0.0.1"));
    assert!(json.contains("8080"));
    assert!(json.contains("http"));

    // Test that it can be deserialized (implements Deserialize)
    let deserialized: ProxySettings = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.proxy_type, proxy.proxy_type);
    assert_eq!(deserialized.host, proxy.host);
    assert_eq!(deserialized.port, proxy.port);
  }
}

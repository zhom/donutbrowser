use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProxySettings {
  pub proxy_type: String, // "http", "https", "socks4", "socks5", or "ss" (Shadowsocks)
  pub host: String,
  pub port: u16,
  pub username: Option<String>,
  pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BrowserType {
  Wayfern,
}

impl BrowserType {
  pub fn as_str(&self) -> &'static str {
    match self {
      BrowserType::Wayfern => "wayfern",
    }
  }

  pub fn from_str(s: &str) -> Result<Self, String> {
    match s {
      "wayfern" => Ok(BrowserType::Wayfern),
      _ => Err(format!("Unknown browser type: {s}")),
    }
  }
}

#[allow(dead_code)]
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

  pub fn get_wayfern_executable_path(
    install_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Wayfern is Chromium-based, look for Chromium.app
    // Find the .app directory
    let app_path = std::fs::read_dir(install_dir)?
      .filter_map(Result::ok)
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .ok_or("Wayfern app not found")?;

    // Construct the browser executable path
    let mut executable_dir = app_path.path();
    executable_dir.push("Contents");
    executable_dir.push("MacOS");

    // Find the Chromium executable
    let executable_path = std::fs::read_dir(&executable_dir)?
      .filter_map(Result::ok)
      .find(|entry| {
        let binding = entry.file_name();
        let name = binding.to_string_lossy();
        name.contains("Chromium") || name == "Wayfern"
      })
      .map(|entry| entry.path())
      .ok_or("No Wayfern executable found in MacOS directory")?;

    Ok(executable_path)
  }

  pub fn is_wayfern_version_downloaded(install_dir: &Path) -> bool {
    // On macOS, check for .app files (Chromium.app)
    if let Ok(entries) = std::fs::read_dir(install_dir) {
      for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "app") {
          return true;
        }
      }
    }
    false
  }

  #[allow(dead_code)]
  pub fn prepare_executable(_executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On macOS, no special preparation needed
    Ok(())
  }
}

#[cfg(target_os = "linux")]
mod linux {
  use super::*;
  use std::os::unix::fs::PermissionsExt;

  pub fn get_chromium_executable_path(
    install_dir: &Path,
    browser_type: &BrowserType,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let possible_executables = match browser_type {
      BrowserType::Wayfern => vec![
        install_dir.join("chromium"),
        install_dir.join("chrome"),
        install_dir.join("wayfern"),
        install_dir.join("wayfern").join("chromium"),
        install_dir.join("wayfern").join("chrome"),
        install_dir.join("chrome-linux").join("chrome"),
      ],
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

  pub fn is_chromium_version_downloaded(install_dir: &Path, browser_type: &BrowserType) -> bool {
    let possible_executables = match browser_type {
      BrowserType::Wayfern => vec![
        install_dir.join("chromium"),
        install_dir.join("chrome"),
        install_dir.join("wayfern"),
        install_dir.join("wayfern").join("chromium"),
        install_dir.join("wayfern").join("chrome"),
        install_dir.join("chrome-linux").join("chrome"),
      ],
    };

    for exe_path in &possible_executables {
      if exe_path.exists() && exe_path.is_file() {
        return true;
      }
    }

    false
  }

  #[allow(dead_code)]
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

  pub fn get_chromium_executable_path(
    install_dir: &Path,
    browser_type: &BrowserType,
  ) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // On Windows, look for .exe files
    let possible_paths = match browser_type {
      BrowserType::Wayfern => vec![
        install_dir.join("chromium.exe"),
        install_dir.join("chrome.exe"),
        install_dir.join("wayfern.exe"),
        install_dir.join("bin").join("chromium.exe"),
        install_dir.join("wayfern").join("chromium.exe"),
        install_dir.join("wayfern").join("chrome.exe"),
        install_dir.join("chrome-win").join("chrome.exe"),
      ],
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
        if path.extension().is_some_and(|ext| ext == "exe") && is_pe_executable(&path) {
          let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
          if name.contains("chromium") || name.contains("chrome") || name.contains("wayfern") {
            return Ok(path);
          }
        }
      }
    }

    Err("Chromium/Wayfern executable not found in Windows installation directory".into())
  }

  pub fn is_chromium_version_downloaded(install_dir: &Path, browser_type: &BrowserType) -> bool {
    // On Windows, check for .exe files
    let possible_executables = match browser_type {
      BrowserType::Wayfern => vec![
        install_dir.join("chromium.exe"),
        install_dir.join("chrome.exe"),
        install_dir.join("wayfern.exe"),
        install_dir.join("bin").join("chromium.exe"),
        install_dir.join("wayfern").join("chromium.exe"),
        install_dir.join("wayfern").join("chrome.exe"),
        install_dir.join("chrome-win").join("chrome.exe"),
      ],
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

        if path.extension().is_some_and(|ext| ext == "exe") && is_pe_executable(&path) {
          let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
          if name.contains("chromium") || name.contains("chrome") || name.contains("wayfern") {
            return true;
          }
        }
      }
    }

    false
  }

  #[allow(dead_code)]
  pub fn prepare_executable(_executable_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // On Windows, no special preparation needed
    Ok(())
  }
}

/// Wayfern is a Chromium-based anti-detect browser with CDP-based fingerprint injection
pub struct WayfernBrowser;

impl WayfernBrowser {
  pub fn new() -> Self {
    Self
  }
}

impl Browser for WayfernBrowser {
  fn get_executable_path(&self, install_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    return macos::get_wayfern_executable_path(install_dir);

    #[cfg(target_os = "linux")]
    return linux::get_chromium_executable_path(install_dir, &BrowserType::Wayfern);

    #[cfg(target_os = "windows")]
    return windows::get_chromium_executable_path(install_dir, &BrowserType::Wayfern);

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
    // Wayfern uses Chromium-style arguments
    let mut args = vec![
      format!("--user-data-dir={}", profile_path),
      "--no-default-browser-check".to_string(),
      "--disable-background-mode".to_string(),
      "--disable-component-update".to_string(),
      "--disable-background-timer-throttling".to_string(),
      "--crash-server-url=".to_string(),
      "--disable-updater".to_string(),
      "--disable-session-crashed-bubble".to_string(),
      "--hide-crash-restore-bubble".to_string(),
      "--disable-infobars".to_string(),
      // Wayfern-specific args for automation
      "--disable-features=DialMediaRouteProvider".to_string(),
      "--use-mock-keychain".to_string(),
      "--password-store=basic".to_string(),
    ];

    // Add remote debugging port (required for CDP fingerprint injection)
    if let Some(port) = remote_debugging_port {
      args.push("--remote-debugging-address=127.0.0.1".to_string());
      args.push(format!("--remote-debugging-port={port}"));
    }

    // Add headless mode if requested
    if headless {
      args.push("--headless=new".to_string());
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
    let install_dir = binaries_dir.join("wayfern").join(version);

    #[cfg(target_os = "macos")]
    return macos::is_wayfern_version_downloaded(&install_dir);

    #[cfg(target_os = "linux")]
    return linux::is_chromium_version_downloaded(&install_dir, &BrowserType::Wayfern);

    #[cfg(target_os = "windows")]
    return windows::is_chromium_version_downloaded(&install_dir, &BrowserType::Wayfern);

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
      BrowserType::Wayfern => Box::new(WayfernBrowser::new()),
    }
  }
}

/// Check if a file is a valid PE executable by reading its magic bytes (MZ).
/// Returns false for archive files (.zip starts with PK, etc.) that were
/// incorrectly named with a .exe extension.
#[cfg(target_os = "windows")]
fn is_pe_executable(path: &Path) -> bool {
  use std::io::Read;
  let Ok(mut file) = std::fs::File::open(path) else {
    return false;
  };
  let mut magic = [0u8; 2];
  if file.read_exact(&mut magic).is_err() {
    return false;
  }
  magic == [0x4D, 0x5A] // MZ
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

  #[test]
  fn test_wayfern_config_has_no_executable_path() {
    // Verify WayfernConfig does not store executable_path
    let config = crate::wayfern_manager::WayfernConfig::default();
    let json = serde_json::to_value(&config).unwrap();
    assert!(
      json.get("executable_path").is_none(),
      "WayfernConfig should not have executable_path field"
    );
  }

  #[test]
  fn test_profile_data_path_is_dynamic() {
    use crate::profile::BrowserProfile;
    let profiles_dir = std::path::PathBuf::from("/fake/profiles");
    let profile = BrowserProfile {
      id: uuid::Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap(),
      name: "test".to_string(),
      browser: "wayfern".to_string(),
      version: "1.0.0".to_string(),
      proxy_id: None,
      vpn_id: None,
      launch_hook: None,
      process_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      wayfern_config: None,
      group_id: None,
      tags: Vec::new(),
      note: None,
      window_color: None,
      sync_mode: crate::profile::types::SyncMode::Disabled,
      encryption_salt: None,
      last_sync: None,
      host_os: None,
      ephemeral: false,
      extension_group_id: None,
      proxy_bypass_rules: Vec::new(),
      created_by_id: None,
      created_by_email: None,
      dns_blocklist: None,
      password_protected: false,
      created_at: None,
      updated_at: None,
    };

    let path = profile.get_profile_data_path(&profiles_dir);
    assert_eq!(
      path,
      profiles_dir
        .join("12345678-1234-1234-1234-123456789abc")
        .join("profile")
    );
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_FACTORY: BrowserFactory = BrowserFactory::new();
}

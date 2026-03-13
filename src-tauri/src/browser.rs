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
  Camoufox,
  Wayfern,
}

impl BrowserType {
  pub fn as_str(&self) -> &'static str {
    match self {
      BrowserType::Camoufox => "camoufox",
      BrowserType::Wayfern => "wayfern",
    }
  }

  pub fn from_str(s: &str) -> Result<Self, String> {
    match s {
      "camoufox" => Ok(BrowserType::Camoufox),
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

    // Find executables matching the browser name pattern
    let candidates: Vec<_> = std::fs::read_dir(&executable_dir)?
      .filter_map(Result::ok)
      .filter(|entry| {
        let binding = entry.file_name();
        let name = binding.to_string_lossy();
        name.starts_with("firefox") || name.starts_with("camoufox") || name.contains("Browser")
      })
      .map(|entry| entry.path())
      .collect();

    if candidates.is_empty() {
      return Err("No executable found in MacOS directory".into());
    }

    // For Camoufox, validate architecture compatibility
    let executable_path = if candidates.iter().any(|p| {
      p.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("camoufox"))
        .unwrap_or(false)
    }) {
      // Find the executable that matches the current architecture
      let current_arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
      } else if cfg!(target_arch = "aarch64") {
        "arm64"
      } else {
        return Err("Unsupported architecture".into());
      };

      // Try to find an executable that matches the current architecture
      // Use file command to check architecture
      let mut found_executable = None;
      let mut file_command_available = true;

      for candidate in &candidates {
        match std::process::Command::new("file").arg(candidate).output() {
          Ok(output) => {
            if output.status.success() {
              if let Ok(output_str) = String::from_utf8(output.stdout) {
                let is_compatible = if current_arch == "x86_64" {
                  output_str.contains("x86_64") || output_str.contains("i386")
                } else {
                  output_str.contains("arm64") || output_str.contains("aarch64")
                };

                if is_compatible {
                  found_executable = Some(candidate.clone());
                  log::info!(
                    "Found compatible Camoufox executable for {}: {}",
                    current_arch,
                    candidate.display()
                  );
                  break;
                } else {
                  log::warn!(
                    "Skipping incompatible Camoufox executable: {} (architecture: {})",
                    candidate.display(),
                    output_str.trim()
                  );
                }
              }
            } else {
              log::warn!(
                "Failed to check architecture for {}: file command returned non-zero exit code",
                candidate.display()
              );
            }
          }
          Err(e) => {
            log::warn!(
              "Failed to check architecture for {} using file command: {}",
              candidate.display(),
              e
            );
            file_command_available = false;
            // Continue checking other candidates
          }
        }
      }

      // If no compatible executable found but we have candidates, use the first one
      // (fallback for cases where file command isn't available or failed)
      if found_executable.is_none() && !candidates.is_empty() {
        if !file_command_available {
          log::warn!(
            "file command not available, using first candidate: {}",
            candidates[0].display()
          );
        } else {
          log::warn!(
            "No compatible executable found for architecture {}, using first candidate: {}",
            current_arch,
            candidates[0].display()
          );
        }
        found_executable = Some(candidates[0].clone());
      }

      found_executable.ok_or_else(|| {
        format!(
          "No compatible Camoufox executable found for architecture {}. Available executables: {:?}",
          current_arch,
          candidates
        )
      })?
    } else {
      // For other browsers, use the first matching executable
      candidates[0].clone()
    };

    Ok(executable_path)
  }

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
      BrowserType::Wayfern => vec![
        install_dir.join("chromium"),
        install_dir.join("chrome"),
        install_dir.join("wayfern"),
        install_dir.join("wayfern").join("chromium"),
        install_dir.join("wayfern").join("chrome"),
        install_dir.join("chrome-linux").join("chrome"),
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
      BrowserType::Wayfern => vec![
        install_dir.join("chromium"),
        install_dir.join("chrome"),
        install_dir.join("wayfern"),
        install_dir.join("wayfern").join("chromium"),
        install_dir.join("wayfern").join("chrome"),
        install_dir.join("chrome-linux").join("chrome"),
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
          let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
          if name.starts_with("firefox") || name.starts_with("camoufox") || name.contains("browser")
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
      BrowserType::Wayfern => vec![
        install_dir.join("chromium.exe"),
        install_dir.join("chrome.exe"),
        install_dir.join("wayfern.exe"),
        install_dir.join("bin").join("chromium.exe"),
        install_dir.join("wayfern").join("chromium.exe"),
        install_dir.join("wayfern").join("chrome.exe"),
        install_dir.join("chrome-win").join("chrome.exe"),
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
          let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
          if name.starts_with("firefox") || name.starts_with("camoufox") || name.contains("browser")
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
      BrowserType::Wayfern => vec![
        install_dir.join("chromium.exe"),
        install_dir.join("chrome.exe"),
        install_dir.join("wayfern.exe"),
        install_dir.join("bin").join("chromium.exe"),
        install_dir.join("wayfern").join("chromium.exe"),
        install_dir.join("wayfern").join("chrome.exe"),
        install_dir.join("chrome-win").join("chrome.exe"),
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
      "--disable-quic".to_string(),
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
      BrowserType::Camoufox => Box::new(CamoufoxBrowser::new()),
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
  use std::fs;
  use tempfile::TempDir;

  #[test]
  fn test_browser_type_conversions() {
    // Test as_str
    assert_eq!(BrowserType::Camoufox.as_str(), "camoufox");
    assert_eq!(BrowserType::Wayfern.as_str(), "wayfern");

    // Test from_str
    assert_eq!(
      BrowserType::from_str("camoufox").expect("camoufox should be valid"),
      BrowserType::Camoufox
    );
    assert_eq!(
      BrowserType::from_str("wayfern").expect("wayfern should be valid"),
      BrowserType::Wayfern
    );

    // Test invalid browser type - these should properly fail
    let invalid_result = BrowserType::from_str("invalid");
    assert!(
      invalid_result.is_err(),
      "Invalid browser type should return error"
    );

    let empty_result = BrowserType::from_str("");
    assert!(empty_result.is_err(), "Empty string should return error");

    assert!(
      BrowserType::from_str("firefox").is_err(),
      "Removed browser types should return error"
    );
    assert!(
      BrowserType::from_str("chromium").is_err(),
      "Removed browser types should return error"
    );
  }

  #[test]
  fn test_camoufox_launch_args() {
    let browser = CamoufoxBrowser::new();
    let args = browser
      .create_launch_args("/path/to/profile", None, None, None, false)
      .expect("Failed to create launch args for Camoufox");
    assert!(args.contains(&"-profile".to_string()));
    assert!(args.contains(&"/path/to/profile".to_string()));
    assert!(args.contains(&"-no-remote".to_string()));

    let args = browser
      .create_launch_args(
        "/path/to/profile",
        None,
        Some("https://example.com".to_string()),
        None,
        false,
      )
      .expect("Failed to create launch args for Camoufox with URL");
    assert!(args.contains(&"https://example.com".to_string()));

    // Test with remote debugging
    let args = browser
      .create_launch_args("/path/to/profile", None, None, Some(9222), false)
      .expect("Failed to create launch args for Camoufox with remote debugging");
    assert!(args.contains(&"--start-debugger-server".to_string()));
    assert!(args.contains(&"9222".to_string()));

    // Test headless mode
    let args = browser
      .create_launch_args("/path/to/profile", None, None, None, true)
      .expect("Failed to create launch args for Camoufox headless");
    assert!(
      args.contains(&"--headless".to_string()),
      "Browser should include headless flag when requested"
    );
  }

  #[test]
  fn test_wayfern_launch_args() {
    let browser = WayfernBrowser::new();
    let args = browser
      .create_launch_args("/path/to/profile", None, None, None, false)
      .expect("Failed to create launch args for Wayfern");

    assert!(
      args.contains(&"--user-data-dir=/path/to/profile".to_string()),
      "Wayfern args should contain user-data-dir"
    );
    assert!(
      args.contains(&"--no-default-browser-check".to_string()),
      "Wayfern args should contain no-default-browser-check"
    );
    assert!(
      args.contains(&"--disable-background-mode".to_string()),
      "Wayfern args should contain disable-background-mode"
    );
    assert!(
      args.contains(&"--disable-component-update".to_string()),
      "Wayfern args should contain disable-component-update"
    );

    let args_with_url = browser
      .create_launch_args(
        "/path/to/profile",
        None,
        Some("https://example.com".to_string()),
        None,
        false,
      )
      .expect("Failed to create launch args for Wayfern with URL");
    assert!(
      args_with_url.contains(&"https://example.com".to_string()),
      "Wayfern args should contain the URL"
    );
    assert_eq!(
      args_with_url.last().expect("Args should not be empty"),
      "https://example.com"
    );

    // Test remote debugging
    let args_with_debug = browser
      .create_launch_args("/path/to/profile", None, None, Some(9222), false)
      .expect("Failed to create launch args for Wayfern with remote debugging");
    assert!(
      args_with_debug.contains(&"--remote-debugging-port=9222".to_string()),
      "Wayfern args should contain remote debugging port"
    );

    // Test headless mode
    let args_headless = browser
      .create_launch_args("/path/to/profile", None, None, None, true)
      .expect("Failed to create launch args for Wayfern headless");
    assert!(
      args_headless.contains(&"--headless=new".to_string()),
      "Wayfern args should contain headless flag when requested"
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

    // Create a mock Camoufox browser installation
    let browser_dir = binaries_dir.join("camoufox").join("135.0.1");
    fs::create_dir_all(&browser_dir).expect("Failed to create browser directory");

    #[cfg(target_os = "macos")]
    {
      let app_dir = browser_dir.join("Camoufox.app");
      fs::create_dir_all(&app_dir).expect("Failed to create Camoufox.app directory");
    }

    #[cfg(target_os = "linux")]
    {
      let executable_path = browser_dir.join("camoufox");
      fs::write(&executable_path, "mock executable").expect("Failed to write mock executable");

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
      let executable_path = browser_dir.join("firefox.exe");
      fs::write(&executable_path, "mock executable").expect("Failed to write mock executable");
    }

    let browser = CamoufoxBrowser::new();
    assert!(browser.is_version_downloaded("135.0.1", binaries_dir));
    assert!(!browser.is_version_downloaded("999.0", binaries_dir));

    // Test with Wayfern browser
    let wayfern_dir = binaries_dir.join("wayfern").join("1.0.0");
    fs::create_dir_all(&wayfern_dir).expect("Failed to create wayfern directory");

    #[cfg(target_os = "macos")]
    {
      let wayfern_app_dir = wayfern_dir.join("Chromium.app");
      fs::create_dir_all(wayfern_app_dir.join("Contents").join("MacOS"))
        .expect("Failed to create Chromium.app structure");

      let executable_path = wayfern_app_dir
        .join("Contents")
        .join("MacOS")
        .join("Chromium");
      fs::write(&executable_path, "mock executable")
        .expect("Failed to write mock Wayfern executable");
    }

    #[cfg(target_os = "linux")]
    {
      let executable_path = wayfern_dir.join("chromium");
      fs::write(&executable_path, "mock executable")
        .expect("Failed to write mock wayfern executable");

      use std::os::unix::fs::PermissionsExt;
      let mut permissions = executable_path
        .metadata()
        .expect("Failed to get wayfern metadata")
        .permissions();
      permissions.set_mode(0o755);
      fs::set_permissions(&executable_path, permissions)
        .expect("Failed to set wayfern permissions");
    }

    #[cfg(target_os = "windows")]
    {
      let executable_path = wayfern_dir.join("chromium.exe");
      fs::write(&executable_path, "mock executable").expect("Failed to write mock chromium.exe");
    }

    let wayfern_browser = WayfernBrowser::new();
    assert!(
      wayfern_browser.is_version_downloaded("1.0.0", binaries_dir),
      "Wayfern version should be detected as downloaded"
    );
    assert!(
      !wayfern_browser.is_version_downloaded("9.9.9", binaries_dir),
      "Non-existent Wayfern version should not be detected as downloaded"
    );
  }

  #[test]
  fn test_version_downloaded_no_app_directory() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let binaries_dir = temp_dir.path();

    // Create browser directory but no proper executable structure
    let browser_dir = binaries_dir.join("camoufox").join("135.0.1");
    fs::create_dir_all(&browser_dir).expect("Failed to create browser directory");

    // Create some other files but no proper executable structure
    fs::write(browser_dir.join("readme.txt"), "Some content").expect("Failed to write readme file");

    let browser = CamoufoxBrowser::new();
    assert!(
      !browser.is_version_downloaded("135.0.1", binaries_dir),
      "Camoufox version should not be detected without proper executable structure"
    );
  }

  #[test]
  fn test_browser_type_clone_and_debug() {
    let browser_type = BrowserType::Camoufox;
    let cloned = browser_type.clone();
    assert_eq!(browser_type, cloned);

    // Test Debug trait
    let debug_str = format!("{browser_type:?}");
    assert!(debug_str.contains("Camoufox"));
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

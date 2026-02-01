use directories::BaseDirs;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command as TokioCommand;

use crate::browser::{create_browser, BrowserType};
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::profile::ProfileManager;

const ACCEPT_TERMS_FLAG: &str = "--accept-terms-and-conditions";
const MIN_VALID_TIMESTAMP: i64 = 1577836800; // 2020-01-01 00:00:00 UTC

pub struct WayfernTermsManager {
  base_dirs: BaseDirs,
}

impl WayfernTermsManager {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
    }
  }

  pub fn instance() -> &'static WayfernTermsManager {
    &WAYFERN_TERMS_MANAGER
  }

  fn get_license_file_path(&self) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
      // Windows: %APPDATA%\Wayfern\license-accepted
      if let Some(app_data) = std::env::var_os("APPDATA") {
        return PathBuf::from(app_data)
          .join("Wayfern")
          .join("license-accepted");
      }
      // Fallback to home directory
      self
        .base_dirs
        .home_dir()
        .join("AppData")
        .join("Roaming")
        .join("Wayfern")
        .join("license-accepted")
    }

    #[cfg(target_os = "macos")]
    {
      // macOS: ~/Library/Application Support/Wayfern/license-accepted
      self
        .base_dirs
        .home_dir()
        .join("Library")
        .join("Application Support")
        .join("Wayfern")
        .join("license-accepted")
    }

    #[cfg(target_os = "linux")]
    {
      // Linux: ~/.config/Wayfern/license-accepted or $XDG_CONFIG_HOME/Wayfern/license-accepted
      if let Some(xdg_config) = std::env::var_os("XDG_CONFIG_HOME") {
        let xdg_path = PathBuf::from(xdg_config);
        if !xdg_path.as_os_str().is_empty() {
          return xdg_path.join("Wayfern").join("license-accepted");
        }
      }
      self
        .base_dirs
        .home_dir()
        .join(".config")
        .join("Wayfern")
        .join("license-accepted")
    }
  }

  pub fn is_terms_accepted(&self) -> bool {
    let license_file = self.get_license_file_path();

    if !license_file.exists() {
      return false;
    }

    // Read the timestamp from the file
    let contents = match std::fs::read_to_string(&license_file) {
      Ok(c) => c,
      Err(_) => return false,
    };

    // Parse timestamp (Wayfern stores Unix timestamp as text)
    let timestamp: i64 = match contents.trim().parse() {
      Ok(t) => t,
      Err(_) => return false,
    };

    // Check that timestamp is positive and after 2020-01-01
    timestamp >= MIN_VALID_TIMESTAMP
  }

  pub fn is_wayfern_downloaded(&self) -> bool {
    let registry = DownloadedBrowsersRegistry::instance();
    let versions = registry.get_downloaded_versions("wayfern");
    !versions.is_empty()
  }

  fn get_any_wayfern_executable(&self) -> Option<PathBuf> {
    // First try to get executable from any downloaded Wayfern version
    let registry = DownloadedBrowsersRegistry::instance();
    let versions = registry.get_downloaded_versions("wayfern");

    if versions.is_empty() {
      return None;
    }

    // Get first available version
    let version = versions.first()?;

    // Get binaries directory
    let binaries_dir = ProfileManager::instance().get_binaries_dir();
    let mut browser_dir = binaries_dir;
    browser_dir.push("wayfern");
    browser_dir.push(version);

    let browser = create_browser(BrowserType::Wayfern);
    browser.get_executable_path(&browser_dir).ok()
  }

  pub async fn accept_terms(&self) -> Result<(), String> {
    let executable_path = self.get_any_wayfern_executable().ok_or_else(|| {
      "No Wayfern browser downloaded. Please download a Wayfern browser version first.".to_string()
    })?;

    log::info!(
      "Running Wayfern with {} flag: {:?}",
      ACCEPT_TERMS_FLAG,
      executable_path
    );

    #[cfg(target_os = "macos")]
    {
      // On macOS, if it's an app bundle, we need to find the actual executable
      let executable_str = executable_path.to_string_lossy();
      if executable_str.ends_with(".app") {
        // Navigate to Contents/MacOS and find the executable
        let macos_dir = executable_path.join("Contents").join("MacOS");
        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
          for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
              return self.run_accept_command(&path).await;
            }
          }
        }
        return Err("Could not find executable in Wayfern app bundle".to_string());
      }
    }

    self.run_accept_command(&executable_path).await
  }

  async fn run_accept_command(&self, executable_path: &PathBuf) -> Result<(), String> {
    let output = TokioCommand::new(executable_path)
      .arg(ACCEPT_TERMS_FLAG)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .output()
      .await
      .map_err(|e| format!("Failed to run Wayfern: {e}"))?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      log::error!("Wayfern terms acceptance failed: {stderr}");
      return Err(format!(
        "Wayfern terms acceptance failed with exit code: {:?}",
        output.status.code()
      ));
    }

    // Verify the license file was created
    if !self.is_terms_accepted() {
      return Err(
        "Terms acceptance command succeeded but license file was not created".to_string(),
      );
    }

    log::info!("Wayfern terms and conditions accepted successfully");
    Ok(())
  }
}

lazy_static::lazy_static! {
  static ref WAYFERN_TERMS_MANAGER: WayfernTermsManager = WayfernTermsManager::new();
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_license_file_path() {
    let manager = WayfernTermsManager::new();
    let path = manager.get_license_file_path();
    let path_str = path.to_string_lossy();

    assert!(
      path_str.contains("Wayfern"),
      "License file path should contain Wayfern"
    );
    assert!(
      path_str.ends_with("license-accepted"),
      "License file should be named license-accepted"
    );

    #[cfg(target_os = "macos")]
    assert!(
      path_str.contains("Application Support"),
      "macOS path should contain Application Support"
    );

    #[cfg(target_os = "linux")]
    assert!(
      path_str.contains(".config") || std::env::var_os("XDG_CONFIG_HOME").is_some(),
      "Linux path should be in .config or XDG_CONFIG_HOME"
    );
  }

  #[test]
  fn test_is_terms_accepted_no_file() {
    let manager = WayfernTermsManager::new();
    // This test will pass if no license file exists (which is typically the case in test env)
    // The actual behavior depends on whether the file exists
    let _ = manager.is_terms_accepted();
  }
}

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadedBrowserInfo {
  pub browser: String,
  pub version: String,
  pub download_date: u64,
  pub file_path: PathBuf,
  pub verified: bool,
  pub actual_version: Option<String>, // For browsers like Chromium where we track the actual version
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DownloadedBrowsersRegistry {
  pub browsers: HashMap<String, HashMap<String, DownloadedBrowserInfo>>, // browser -> version -> info
}

impl DownloadedBrowsersRegistry {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
    let registry_path = Self::get_registry_path()?;

    if !registry_path.exists() {
      return Ok(Self::new());
    }

    let content = fs::read_to_string(&registry_path)?;
    let registry: DownloadedBrowsersRegistry = serde_json::from_str(&content)?;
    Ok(registry)
  }

  pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
    let registry_path = Self::get_registry_path()?;

    // Ensure parent directory exists
    if let Some(parent) = registry_path.parent() {
      fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string_pretty(self)?;
    fs::write(&registry_path, content)?;
    Ok(())
  }

  fn get_registry_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let base_dirs = BaseDirs::new().ok_or("Failed to get base directories")?;
    let mut path = base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("data");
    path.push("downloaded_browsers.json");
    Ok(path)
  }

  pub fn add_browser(&mut self, info: DownloadedBrowserInfo) {
    self
      .browsers
      .entry(info.browser.clone())
      .or_default()
      .insert(info.version.clone(), info);
  }

  pub fn remove_browser(&mut self, browser: &str, version: &str) -> Option<DownloadedBrowserInfo> {
    self.browsers.get_mut(browser)?.remove(version)
  }

  pub fn is_browser_downloaded(&self, browser: &str, version: &str) -> bool {
    self
      .browsers
      .get(browser)
      .and_then(|versions| versions.get(version))
      .map(|info| info.verified)
      .unwrap_or(false)
  }

  pub fn get_downloaded_versions(&self, browser: &str) -> Vec<String> {
    self
      .browsers
      .get(browser)
      .map(|versions| {
        versions
          .iter()
          .filter(|(_, info)| info.verified)
          .map(|(version, _)| version.clone())
          .collect()
      })
      .unwrap_or_default()
  }

  pub fn mark_download_started(&mut self, browser: &str, version: &str, file_path: PathBuf) {
    let info = DownloadedBrowserInfo {
      browser: browser.to_string(),
      version: version.to_string(),
      download_date: std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs(),
      file_path,
      verified: false,
      actual_version: None,
    };
    self.add_browser(info);
  }

  pub fn mark_download_completed_with_actual_version(
    &mut self,
    browser: &str,
    version: &str,
    actual_version: Option<String>,
  ) -> Result<(), String> {
    if let Some(info) = self
      .browsers
      .get_mut(browser)
      .and_then(|versions| versions.get_mut(version))
    {
      info.verified = true;
      info.actual_version = actual_version;
      Ok(())
    } else {
      Err(format!(
        "Browser {}:{} not found in registry",
        browser, version
      ))
    }
  }

  pub fn cleanup_failed_download(
    &mut self,
    browser: &str,
    version: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(info) = self.remove_browser(browser, version) {
      // Clean up any files that might have been left behind
      if info.file_path.exists() {
        if info.file_path.is_dir() {
          fs::remove_dir_all(&info.file_path)?;
        } else {
          fs::remove_file(&info.file_path)?;
        }
      }

      // Also clean up the browser directory if it exists
      let base_dirs = BaseDirs::new().ok_or("Failed to get base directories")?;
      let mut browser_dir = base_dirs.data_local_dir().to_path_buf();
      browser_dir.push(if cfg!(debug_assertions) {
        "DonutBrowserDev"
      } else {
        "DonutBrowser"
      });
      browser_dir.push("binaries");
      browser_dir.push(browser);
      browser_dir.push(version);

      if browser_dir.exists() {
        fs::remove_dir_all(&browser_dir)?;
      }
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_registry_creation() {
    let registry = DownloadedBrowsersRegistry::new();
    assert!(registry.browsers.is_empty());
  }

  #[test]
  fn test_add_and_get_browser() {
    let mut registry = DownloadedBrowsersRegistry::new();
    let info = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "139.0".to_string(),
      download_date: 1234567890,
      file_path: PathBuf::from("/test/path"),
      verified: true,
      actual_version: None,
    };

    registry.add_browser(info.clone());

    assert!(registry.is_browser_downloaded("firefox", "139.0"));
    assert!(!registry.is_browser_downloaded("firefox", "140.0"));
    assert!(!registry.is_browser_downloaded("chrome", "139.0"));
  }

  #[test]
  fn test_get_downloaded_versions() {
    let mut registry = DownloadedBrowsersRegistry::new();

    let info1 = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "139.0".to_string(),
      download_date: 1234567890,
      file_path: PathBuf::from("/test/path1"),
      verified: true,
      actual_version: None,
    };

    let info2 = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "140.0".to_string(),
      download_date: 1234567891,
      file_path: PathBuf::from("/test/path2"),
      verified: false, // Not verified, should not be included
      actual_version: None,
    };

    let info3 = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "141.0".to_string(),
      download_date: 1234567892,
      file_path: PathBuf::from("/test/path3"),
      verified: true,
      actual_version: None,
    };

    registry.add_browser(info1);
    registry.add_browser(info2);
    registry.add_browser(info3);

    let versions = registry.get_downloaded_versions("firefox");
    assert_eq!(versions.len(), 2);
    assert!(versions.contains(&"139.0".to_string()));
    assert!(versions.contains(&"141.0".to_string()));
    assert!(!versions.contains(&"140.0".to_string()));
  }

  #[test]
  fn test_mark_download_lifecycle() {
    let mut registry = DownloadedBrowsersRegistry::new();

    // Mark download started
    registry.mark_download_started("firefox", "139.0", PathBuf::from("/test/path"));

    // Should not be considered downloaded yet
    assert!(!registry.is_browser_downloaded("firefox", "139.0"));

    // Mark as completed
    registry
      .mark_download_completed_with_actual_version("firefox", "139.0", Some("139.0".to_string()))
      .unwrap();

    // Now should be considered downloaded
    assert!(registry.is_browser_downloaded("firefox", "139.0"));
  }

  #[test]
  fn test_remove_browser() {
    let mut registry = DownloadedBrowsersRegistry::new();
    let info = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "139.0".to_string(),
      download_date: 1234567890,
      file_path: PathBuf::from("/test/path"),
      verified: true,
      actual_version: None,
    };

    registry.add_browser(info);
    assert!(registry.is_browser_downloaded("firefox", "139.0"));

    let removed = registry.remove_browser("firefox", "139.0");
    assert!(removed.is_some());
    assert!(!registry.is_browser_downloaded("firefox", "139.0"));
  }
}

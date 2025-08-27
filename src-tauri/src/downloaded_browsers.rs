use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadedBrowserInfo {
  pub browser: String,
  pub version: String,
  pub file_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct RegistryData {
  pub browsers: HashMap<String, HashMap<String, DownloadedBrowserInfo>>, // browser -> version -> info
}

pub struct DownloadedBrowsersRegistry {
  data: Mutex<RegistryData>,
}

impl DownloadedBrowsersRegistry {
  fn new() -> Self {
    Self {
      data: Mutex::new(RegistryData::default()),
    }
  }

  pub fn instance() -> &'static DownloadedBrowsersRegistry {
    &DOWNLOADED_BROWSERS_REGISTRY
  }

  pub fn load(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let registry_path = Self::get_registry_path()?;

    if !registry_path.exists() {
      return Ok(());
    }

    let content = fs::read_to_string(&registry_path)?;
    let registry_data: RegistryData = serde_json::from_str(&content)?;

    let mut data = self.data.lock().unwrap();
    *data = registry_data;
    Ok(())
  }

  pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let registry_path = Self::get_registry_path()?;

    // Ensure parent directory exists
    if let Some(parent) = registry_path.parent() {
      fs::create_dir_all(parent)?;
    }

    let data = self.data.lock().unwrap();
    let content = serde_json::to_string_pretty(&*data)?;
    fs::write(&registry_path, content)?;
    Ok(())
  }

  fn get_registry_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
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

  pub fn add_browser(&self, info: DownloadedBrowserInfo) {
    let mut data = self.data.lock().unwrap();
    data
      .browsers
      .entry(info.browser.clone())
      .or_default()
      .insert(info.version.clone(), info);
  }

  pub fn remove_browser(&self, browser: &str, version: &str) -> Option<DownloadedBrowserInfo> {
    let mut data = self.data.lock().unwrap();
    data.browsers.get_mut(browser)?.remove(version)
  }

  pub fn is_browser_downloaded(&self, browser: &str, version: &str) -> bool {
    let data = self.data.lock().unwrap();
    data
      .browsers
      .get(browser)
      .and_then(|versions| versions.get(version))
      .is_some()
  }

  pub fn get_downloaded_versions(&self, browser: &str) -> Vec<String> {
    let data = self.data.lock().unwrap();
    data
      .browsers
      .get(browser)
      .map(|versions| versions.keys().cloned().collect())
      .unwrap_or_default()
  }

  pub fn mark_download_started(&self, browser: &str, version: &str, file_path: PathBuf) {
    let info = DownloadedBrowserInfo {
      browser: browser.to_string(),
      version: version.to_string(),
      file_path,
    };
    self.add_browser(info);
  }

  pub fn mark_download_completed(&self, browser: &str, version: &str) -> Result<(), String> {
    let data = self.data.lock().unwrap();
    if data
      .browsers
      .get(browser)
      .and_then(|versions| versions.get(version))
      .is_some()
    {
      Ok(())
    } else {
      Err(format!("Browser {browser}:{version} not found in registry"))
    }
  }

  pub fn cleanup_failed_download(
    &self,
    browser: &str,
    version: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(info) = self.remove_browser(browser, version) {
      // Clean up extracted binaries but preserve downloaded archives
      if info.file_path.exists() {
        if info.file_path.is_dir() {
          // Allowed archive extensions to preserve
          let archive_exts = [
            "zip", "dmg", "tar.xz", "tar.gz", "tar.bz2", "AppImage", "exe", "pkg", "msi",
          ];

          for entry in fs::read_dir(&info.file_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
              fs::remove_dir_all(&path)?;
              continue;
            }

            // For files, preserve if they look like downloaded archives/installers
            let keep = path
              .file_name()
              .and_then(|n| n.to_str())
              .map(|name| {
                // Match suffixes (handles multi-part extensions like .tar.xz)
                archive_exts
                  .iter()
                  .any(|ext| name.to_lowercase().ends_with(&ext.to_lowercase()))
              })
              .unwrap_or(false);

            if !keep {
              fs::remove_file(&path)?;
            }
          }
        } else {
          // It's a file. If it's not an archive, remove it; otherwise preserve it.
          let file_name = info
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
          let archive_exts = [
            "zip", "dmg", "tar.xz", "tar.gz", "tar.bz2", "AppImage", "exe", "pkg", "msi",
          ];
          let is_archive = archive_exts
            .iter()
            .any(|ext| file_name.to_lowercase().ends_with(&ext.to_lowercase()));
          if !is_archive {
            fs::remove_file(&info.file_path)?;
          }
        }
      }
    }
    Ok(())
  }

  /// Find and remove unused browser binaries that are not referenced by any active profiles
  pub fn cleanup_unused_binaries(
    &self,
    active_profiles: &[(String, String)], // (browser, version) pairs
    running_profiles: &[(String, String)], // (browser, version) pairs for running profiles
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let active_set: std::collections::HashSet<(String, String)> =
      active_profiles.iter().cloned().collect();
    let running_set: std::collections::HashSet<(String, String)> =
      running_profiles.iter().cloned().collect();
    let mut cleaned_up = Vec::new();

    // Get pending update versions from auto updater
    let pending_updates =
      match crate::auto_updater::AutoUpdater::instance().get_pending_update_versions() {
        Ok(updates) => updates,
        Err(e) => {
          eprintln!("Warning: Failed to get pending updates for cleanup: {e}");
          std::collections::HashSet::new()
        }
      };

    // Collect all downloaded browsers that are not in active profiles
    let mut to_remove = Vec::new();
    {
      let data = self.data.lock().unwrap();
      for (browser, versions) in &data.browsers {
        for version in versions.keys() {
          let browser_version = (browser.clone(), version.clone());

          // Don't remove if it's used by any active profile
          if active_set.contains(&browser_version) {
            println!("Keeping: {browser} {version} (in use by profile)");
            continue;
          }

          // Don't remove if it's currently running (even if not in active profiles)
          if running_set.contains(&browser_version) {
            println!("Keeping: {browser} {version} (currently running)");
            continue;
          }

          // Don't remove if this version has a pending update for a running profile
          // This handles the case where a running profile has an update downloaded but not yet applied
          if pending_updates.contains(&browser_version) {
            // Check if there are any running profiles for this browser that could be updated
            let has_running_profile_for_browser =
              running_profiles.iter().any(|(b, _)| b == browser);
            if has_running_profile_for_browser {
              println!("Keeping: {browser} {version} (pending update for running profile)");
              continue;
            }
          }

          // Mark for removal
          to_remove.push(browser_version);
          println!("Marking for removal: {browser} {version} (not used by any profile)");
        }
      }
    }

    // Remove unused binaries and their version folders
    for (browser, version) in to_remove {
      if let Err(e) = self.cleanup_failed_download(&browser, &version) {
        eprintln!("Failed to cleanup unused binary {browser}:{version}: {e}");
      } else {
        // After removing the binary, also remove the empty version folder
        if let Err(e) = self.remove_empty_version_folder(&browser, &version) {
          eprintln!("Failed to remove empty version folder for {browser}:{version}: {e}");
        }
        cleaned_up.push(format!("{browser} {version}"));
        println!("Successfully removed unused binary: {browser} {version}");
      }
    }

    if cleaned_up.is_empty() {
      println!("No unused binaries found to clean up");
    } else {
      println!("Cleaned up {} unused binaries", cleaned_up.len());
    }

    Ok(cleaned_up)
  }

  /// Get all browsers and versions referenced by active profiles
  pub fn get_active_browser_versions(
    &self,
    profiles: &[crate::profile::BrowserProfile],
  ) -> Vec<(String, String)> {
    profiles
      .iter()
      .map(|profile| (profile.browser.clone(), profile.version.clone()))
      .collect()
  }

  /// Verify that all registered browsers actually exist on disk and clean up stale entries
  pub fn verify_and_cleanup_stale_entries(
    &self,
    browser_runner: &crate::browser_runner::BrowserRunner,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    use crate::browser::{create_browser, BrowserType};
    let mut cleaned_up = Vec::new();
    let binaries_dir = browser_runner.get_binaries_dir();

    let browsers_to_check: Vec<(String, String)> = {
      let data = self.data.lock().unwrap();
      data
        .browsers
        .iter()
        .flat_map(|(browser, versions)| {
          versions
            .keys()
            .map(|version| (browser.clone(), version.clone()))
        })
        .collect()
    };

    for (browser_str, version) in browsers_to_check {
      if let Ok(browser_type) = BrowserType::from_str(&browser_str) {
        let browser = create_browser(browser_type);
        if !browser.is_version_downloaded(&version, &binaries_dir) {
          // Files don't exist, remove from registry
          if let Some(_removed) = self.remove_browser(&browser_str, &version) {
            cleaned_up.push(format!("{browser_str} {version}"));
            println!("Removed stale registry entry for {browser_str} {version}");
          }
        }
      }
    }

    if !cleaned_up.is_empty() {
      self.save()?;
    }

    Ok(cleaned_up)
  }

  /// Get all browsers and versions that are currently running
  pub fn get_running_browser_versions(
    &self,
    profiles: &[crate::profile::BrowserProfile],
  ) -> Vec<(String, String)> {
    profiles
      .iter()
      .filter(|profile| profile.process_id.is_some())
      .map(|profile| (profile.browser.clone(), profile.version.clone()))
      .collect()
  }

  /// Scan the binaries directory and sync with registry
  /// This ensures the registry reflects what's actually on disk
  pub fn sync_with_binaries_directory(
    &self,
    binaries_dir: &std::path::Path,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut changes = Vec::new();

    if !binaries_dir.exists() {
      return Ok(changes);
    }

    // Scan for actual browser directories
    for browser_entry in fs::read_dir(binaries_dir)? {
      let browser_entry = browser_entry?;
      let browser_path = browser_entry.path();

      if !browser_path.is_dir() {
        continue;
      }

      let browser_name = browser_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

      if browser_name.is_empty() || browser_name.starts_with('.') {
        continue;
      }

      // Scan for version directories within this browser
      for version_entry in fs::read_dir(&browser_path)? {
        let version_entry = version_entry?;
        let version_path = version_entry.path();

        if !version_path.is_dir() {
          continue;
        }

        let version_name = version_path
          .file_name()
          .and_then(|n| n.to_str())
          .unwrap_or("");

        if version_name.is_empty() || version_name.starts_with('.') {
          continue;
        }

        // Only add to registry if this looks like a valid installed browser, not just an archive
        if !self.is_browser_downloaded(browser_name, version_name) {
          if let Ok(browser_type) = crate::browser::BrowserType::from_str(browser_name) {
            let browser = crate::browser::create_browser(browser_type);
            if browser.is_version_downloaded(version_name, binaries_dir) {
              let info = DownloadedBrowserInfo {
                browser: browser_name.to_string(),
                version: version_name.to_string(),
                file_path: version_path.clone(),
              };
              self.add_browser(info);
              changes.push(format!("Added {browser_name} {version_name} to registry"));
            }
          }
        }
      }
    }

    if !changes.is_empty() {
      self.save()?;
    }

    Ok(changes)
  }

  /// Comprehensive cleanup that removes unused binaries and syncs registry
  pub fn comprehensive_cleanup(
    &self,
    binaries_dir: &std::path::Path,
    active_profiles: &[(String, String)],
    running_profiles: &[(String, String)],
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut cleanup_results = Vec::new();

    // First, sync registry with actual binaries on disk
    let sync_results = self.sync_with_binaries_directory(binaries_dir)?;
    cleanup_results.extend(sync_results);

    // Then perform the regular cleanup
    let regular_cleanup = self.cleanup_unused_binaries(active_profiles, running_profiles)?;
    cleanup_results.extend(regular_cleanup);

    // Verify and cleanup stale entries
    let stale_cleanup = self.verify_and_cleanup_stale_entries_simple(binaries_dir)?;
    cleanup_results.extend(stale_cleanup);

    // Clean up any remaining empty folders
    let empty_folder_cleanup = self.cleanup_empty_folders(binaries_dir)?;
    cleanup_results.extend(empty_folder_cleanup);

    if !cleanup_results.is_empty() {
      self.save()?;
    }

    Ok(cleanup_results)
  }

  /// Remove empty version folder after cleanup
  fn remove_empty_version_folder(
    &self,
    browser: &str,
    version: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get binaries directory path
    let base_dirs = directories::BaseDirs::new().ok_or("Failed to get base directories")?;
    let mut binaries_dir = base_dirs.data_local_dir().to_path_buf();
    binaries_dir.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    binaries_dir.push("binaries");

    let version_dir = binaries_dir.join(browser).join(version);

    // Only remove if the directory exists and is empty
    if version_dir.exists() && version_dir.is_dir() {
      if let Ok(mut entries) = fs::read_dir(&version_dir) {
        if entries.next().is_none() {
          // Directory is empty, remove it
          fs::remove_dir(&version_dir)?;
          println!("Removed empty version folder: {}", version_dir.display());

          // Also check if the browser folder is now empty and remove it too
          let browser_dir = binaries_dir.join(browser);
          if browser_dir.exists() && browser_dir.is_dir() {
            if let Ok(mut browser_entries) = fs::read_dir(&browser_dir) {
              if browser_entries.next().is_none() {
                fs::remove_dir(&browser_dir)?;
                println!("Removed empty browser folder: {}", browser_dir.display());
              }
            }
          }
        }
      }
    }

    Ok(())
  }

  /// Clean up existing empty version and browser folders
  pub fn cleanup_empty_folders(
    &self,
    binaries_dir: &std::path::Path,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut cleaned_up = Vec::new();

    if !binaries_dir.exists() {
      return Ok(cleaned_up);
    }

    // Scan for browser directories
    for browser_entry in fs::read_dir(binaries_dir)? {
      let browser_entry = browser_entry?;
      let browser_path = browser_entry.path();

      if !browser_path.is_dir() {
        continue;
      }

      let browser_name = browser_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

      if browser_name.is_empty() || browser_name.starts_with('.') {
        continue;
      }

      let mut empty_version_dirs = Vec::new();
      let mut has_non_empty_versions = false;

      // Scan for version directories within this browser
      for version_entry in fs::read_dir(&browser_path)? {
        let version_entry = version_entry?;
        let version_path = version_entry.path();

        if !version_path.is_dir() {
          has_non_empty_versions = true; // Non-directory files count as non-empty
          continue;
        }

        let version_name = version_path
          .file_name()
          .and_then(|n| n.to_str())
          .unwrap_or("");

        if version_name.is_empty() || version_name.starts_with('.') {
          continue;
        }

        // Check if version directory is empty
        match fs::read_dir(&version_path) {
          Ok(mut entries) => {
            if entries.next().is_none() {
              // Directory is empty
              empty_version_dirs.push((version_path.clone(), version_name.to_string()));
            } else {
              has_non_empty_versions = true;
            }
          }
          Err(_) => {
            has_non_empty_versions = true; // Assume non-empty if we can't read
          }
        }
      }

      // Remove empty version directories
      for (version_path, version_name) in empty_version_dirs {
        if let Err(e) = fs::remove_dir(&version_path) {
          eprintln!(
            "Failed to remove empty version folder {}: {e}",
            version_path.display()
          );
        } else {
          cleaned_up.push(format!(
            "Removed empty version folder: {browser_name}/{version_name}"
          ));
          println!("Removed empty version folder: {}", version_path.display());
        }
      }

      // If browser directory is now empty, remove it too
      if !has_non_empty_versions {
        if let Ok(mut entries) = fs::read_dir(&browser_path) {
          if entries.next().is_none() {
            if let Err(e) = fs::remove_dir(&browser_path) {
              eprintln!(
                "Failed to remove empty browser folder {}: {e}",
                browser_path.display()
              );
            } else {
              cleaned_up.push(format!("Removed empty browser folder: {browser_name}"));
              println!("Removed empty browser folder: {}", browser_path.display());
            }
          }
        }
      }
    }

    Ok(cleaned_up)
  }

  /// Simplified version of verify_and_cleanup_stale_entries that doesn't need BrowserRunner
  pub fn verify_and_cleanup_stale_entries_simple(
    &self,
    binaries_dir: &std::path::Path,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut cleaned_up = Vec::new();
    let mut browsers_to_remove = Vec::new();

    {
      let data = self.data.lock().unwrap();
      for (browser_str, versions) in &data.browsers {
        for version in versions.keys() {
          // Check if the browser directory actually exists
          let browser_dir = binaries_dir.join(browser_str).join(version);
          if !browser_dir.exists() {
            browsers_to_remove.push((browser_str.clone(), version.clone()));
          }
        }
      }
    }

    // Remove stale entries
    for (browser_str, version) in browsers_to_remove {
      if let Some(_removed) = self.remove_browser(&browser_str, &version) {
        cleaned_up.push(format!(
          "Removed stale registry entry for {browser_str} {version}"
        ));
      }
    }

    Ok(cleaned_up)
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref DOWNLOADED_BROWSERS_REGISTRY: DownloadedBrowsersRegistry = {
    let registry = DownloadedBrowsersRegistry::new();
    if let Err(e) = registry.load() {
      eprintln!("Warning: Failed to load downloaded browsers registry: {e}");
    }
    registry
  };
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_registry_creation() {
    let registry = DownloadedBrowsersRegistry::new();
    let data = registry.data.lock().unwrap();
    assert!(data.browsers.is_empty());
  }

  #[test]
  fn test_add_and_get_browser() {
    let registry = DownloadedBrowsersRegistry::new();
    let info = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "139.0".to_string(),
      file_path: PathBuf::from("/test/path"),
    };

    registry.add_browser(info.clone());

    assert!(registry.is_browser_downloaded("firefox", "139.0"));
    assert!(!registry.is_browser_downloaded("firefox", "140.0"));
    assert!(!registry.is_browser_downloaded("chrome", "139.0"));
  }

  #[test]
  fn test_get_downloaded_versions() {
    let registry = DownloadedBrowsersRegistry::new();

    let info1 = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "139.0".to_string(),
      file_path: PathBuf::from("/test/path1"),
    };

    let info2 = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "140.0".to_string(),
      file_path: PathBuf::from("/test/path2"),
    };

    let info3 = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "141.0".to_string(),
      file_path: PathBuf::from("/test/path3"),
    };

    registry.add_browser(info1);
    registry.add_browser(info2);
    registry.add_browser(info3);

    let versions = registry.get_downloaded_versions("firefox");
    assert_eq!(versions.len(), 3);
    assert!(versions.contains(&"139.0".to_string()));
    assert!(versions.contains(&"140.0".to_string()));
    assert!(versions.contains(&"141.0".to_string()));
  }

  #[test]
  fn test_mark_download_lifecycle() {
    let registry = DownloadedBrowsersRegistry::new();

    // Mark download started
    registry.mark_download_started("firefox", "139.0", PathBuf::from("/test/path"));

    // Should be considered downloaded immediately
    assert!(
      registry.is_browser_downloaded("firefox", "139.0"),
      "Browser should be considered downloaded after marking as started"
    );

    // Mark as completed
    registry
      .mark_download_completed("firefox", "139.0")
      .expect("Failed to mark download as completed");

    // Should still be considered downloaded
    assert!(
      registry.is_browser_downloaded("firefox", "139.0"),
      "Browser should still be considered downloaded after completion"
    );
  }

  #[test]
  fn test_remove_browser() {
    let registry = DownloadedBrowsersRegistry::new();
    let info = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "139.0".to_string(),
      file_path: PathBuf::from("/test/path"),
    };

    registry.add_browser(info);
    assert!(
      registry.is_browser_downloaded("firefox", "139.0"),
      "Browser should be downloaded after adding"
    );

    let removed = registry.remove_browser("firefox", "139.0");
    assert!(
      removed.is_some(),
      "Remove operation should return the removed browser info"
    );
    assert!(
      !registry.is_browser_downloaded("firefox", "139.0"),
      "Browser should not be downloaded after removal"
    );
  }

  #[test]
  fn test_twilight_download() {
    let registry = DownloadedBrowsersRegistry::new();

    // Mark twilight download started
    registry.mark_download_started("zen", "twilight", PathBuf::from("/test/zen-twilight"));

    // Check that it's registered
    assert!(
      registry.is_browser_downloaded("zen", "twilight"),
      "Zen twilight version should be registered as downloaded"
    );
  }
}

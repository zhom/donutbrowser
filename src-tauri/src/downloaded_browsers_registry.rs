use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::geoip_downloader::GeoIPDownloader;
use crate::profile::{BrowserProfile, ProfileManager};

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
  profile_manager: &'static ProfileManager,
  auto_updater: &'static crate::auto_updater::AutoUpdater,
  geoip_downloader: &'static GeoIPDownloader,
}

impl DownloadedBrowsersRegistry {
  fn new() -> Self {
    Self {
      data: Mutex::new(RegistryData::default()),
      profile_manager: ProfileManager::instance(),
      auto_updater: crate::auto_updater::AutoUpdater::instance(),
      geoip_downloader: GeoIPDownloader::instance(),
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

  /// Check if browser is registered in the registry (without disk validation)
  /// This method only checks the in-memory registry and does not validate file existence
  pub fn is_browser_registered(&self, browser: &str, version: &str) -> bool {
    let data = self.data.lock().unwrap();
    data
      .browsers
      .get(browser)
      .and_then(|versions| versions.get(version))
      .is_some()
  }

  /// Check if browser is downloaded and files exist on disk
  /// This method validates both registry entry and actual file existence
  pub fn is_browser_downloaded(&self, browser: &str, version: &str) -> bool {
    use crate::browser::{create_browser, BrowserType};

    // First check if browser is registered
    if !self.is_browser_registered(browser, version) {
      return false;
    }

    // Always check if files actually exist on disk
    let browser_type = match BrowserType::from_str(browser) {
      Ok(bt) => bt,
      Err(_) => {
        log::info!("Invalid browser type: {browser}");
        return false;
      }
    };
    let browser_instance = create_browser(browser_type.clone());

    // Get binaries directory
    let binaries_dir = if let Some(base_dirs) = directories::BaseDirs::new() {
      let mut path = base_dirs.data_local_dir().to_path_buf();
      path.push(if cfg!(debug_assertions) {
        "DonutBrowserDev"
      } else {
        "DonutBrowser"
      });
      path.push("binaries");
      path
    } else {
      return false;
    };

    let files_exist = browser_instance.is_version_downloaded(version, &binaries_dir);

    // If files don't exist but registry thinks they do, clean up the registry
    if !files_exist {
      log::info!("Cleaning up stale registry entry for {browser} {version}");
      self.remove_browser(browser, version);
      let _ = self.save(); // Don't fail if save fails, just log
    }

    files_exist
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
    // Only mark download started, don't add to registry yet
    // The browser will be added to registry only after verification succeeds
    log::info!(
      "Marking download started for {}:{} at {}",
      browser,
      version,
      file_path.display()
    );
  }

  pub fn mark_download_completed(
    &self,
    browser: &str,
    version: &str,
    file_path: PathBuf,
  ) -> Result<(), String> {
    // Only mark as completed after verification succeeds
    let info = DownloadedBrowserInfo {
      browser: browser.to_string(),
      version: version.to_string(),
      file_path,
    };
    self.add_browser(info);
    log::info!("Browser {browser}:{version} successfully added to registry after verification");
    Ok(())
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
  fn cleanup_unused_binaries_internal(
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
    let pending_updates = match self.auto_updater.get_pending_update_versions() {
      Ok(updates) => updates,
      Err(e) => {
        log::warn!("Warning: Failed to get pending updates for cleanup: {e}");
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
            log::info!("Keeping: {browser} {version} (in use by profile)");
            continue;
          }

          // Don't remove if it's currently running (even if not in active profiles)
          if running_set.contains(&browser_version) {
            log::info!("Keeping: {browser} {version} (currently running)");
            continue;
          }

          // Don't remove if this version has a pending update for a running profile
          // This handles the case where a running profile has an update downloaded but not yet applied
          if pending_updates.contains(&browser_version) {
            // Check if there are any running profiles for this browser that could be updated
            let has_running_profile_for_browser =
              running_profiles.iter().any(|(b, _)| b == browser);
            if has_running_profile_for_browser {
              log::info!("Keeping: {browser} {version} (pending update for running profile)");
              continue;
            }
          }

          // Mark for removal
          to_remove.push(browser_version);
          log::info!("Marking for removal: {browser} {version} (not used by any profile)");
        }
      }
    }

    // Remove unused binaries and their version folders
    for (browser, version) in to_remove {
      if let Err(e) = self.cleanup_failed_download(&browser, &version) {
        log::error!("Failed to cleanup unused binary {browser}:{version}: {e}");
      } else {
        // After removing the binary, also remove the empty version folder
        if let Err(e) = self.remove_empty_version_folder(&browser, &version) {
          log::error!("Failed to remove empty version folder for {browser}:{version}: {e}");
        }
        cleaned_up.push(format!("{browser} {version}"));
        log::info!("Successfully removed unused binary: {browser} {version}");
      }
    }

    if cleaned_up.is_empty() {
      log::info!("No unused binaries found to clean up");
    } else {
      log::info!("Cleaned up {} unused binaries", cleaned_up.len());
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
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    use crate::browser::{create_browser, BrowserType};
    let mut cleaned_up = Vec::new();
    let binaries_dir = self.profile_manager.get_binaries_dir();

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
            log::info!("Removed stale registry entry for {browser_str} {version}");
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
  fn comprehensive_cleanup(
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
    let regular_cleanup =
      self.cleanup_unused_binaries_internal(active_profiles, running_profiles)?;
    cleanup_results.extend(regular_cleanup);

    // Verify and cleanup stale entries
    let stale_cleanup = self.verify_and_cleanup_stale_entries()?;
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
          log::info!("Removed empty version folder: {}", version_dir.display());

          // Also check if the browser folder is now empty and remove it too
          let browser_dir = binaries_dir.join(browser);
          if browser_dir.exists() && browser_dir.is_dir() {
            if let Ok(mut browser_entries) = fs::read_dir(&browser_dir) {
              if browser_entries.next().is_none() {
                fs::remove_dir(&browser_dir)?;
                log::info!("Removed empty browser folder: {}", browser_dir.display());
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
          log::error!(
            "Failed to remove empty version folder {}: {e}",
            version_path.display()
          );
        } else {
          cleaned_up.push(format!(
            "Removed empty version folder: {browser_name}/{version_name}"
          ));
          log::info!("Removed empty version folder: {}", version_path.display());
        }
      }

      // If browser directory is now empty, remove it too
      if !has_non_empty_versions {
        if let Ok(mut entries) = fs::read_dir(&browser_path) {
          if entries.next().is_none() {
            if let Err(e) = fs::remove_dir(&browser_path) {
              log::error!(
                "Failed to remove empty browser folder {}: {e}",
                browser_path.display()
              );
            } else {
              cleaned_up.push(format!("Removed empty browser folder: {browser_name}"));
              log::info!("Removed empty browser folder: {}", browser_path.display());
            }
          }
        }
      }
    }

    Ok(cleaned_up)
  }

  /// Consolidate browser versions - keep only the latest version per browser
  pub fn consolidate_browser_versions(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Starting browser version consolidation...");

    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let binaries_dir = self.profile_manager.get_binaries_dir();
    let mut consolidated = Vec::new();

    // Group profiles by browser
    let mut browser_profiles: std::collections::HashMap<String, Vec<&BrowserProfile>> =
      std::collections::HashMap::new();
    for profile in &profiles {
      browser_profiles
        .entry(profile.browser.clone())
        .or_default()
        .push(profile);
    }

    for (browser_name, browser_profiles) in browser_profiles.iter() {
      // Find the latest version among all profiles for this browser that actually exists on disk
      let mut available_versions: Vec<String> = Vec::new();

      for profile in browser_profiles {
        // Only consider versions that actually exist on disk
        let browser_type = match crate::browser::BrowserType::from_str(browser_name) {
          Ok(bt) => bt,
          Err(_) => continue,
        };
        let browser = crate::browser::create_browser(browser_type.clone());

        if browser.is_version_downloaded(&profile.version, &binaries_dir) {
          available_versions.push(profile.version.clone());
        } else {
          log::info!(
            "Profile '{}' references version {} that doesn't exist on disk",
            profile.name,
            profile.version
          );
        }
      }

      if available_versions.is_empty() {
        log::info!("No available versions found for {browser_name}, skipping consolidation");
        continue;
      }

      // Sort available versions to find the latest
      available_versions.sort_by(|a, b| {
        // Sort versions using semantic versioning logic
        crate::api_client::compare_versions(b, a)
      });

      let latest_version = &available_versions[0];
      log::info!("Latest available version for {browser_name}: {latest_version}");

      // Check which profiles need to be updated to the latest version
      let mut profiles_to_update = Vec::new();
      let mut older_versions_to_remove = std::collections::HashSet::<String>::new();

      for profile in browser_profiles {
        if profile.version != *latest_version {
          // Only update if profile is not currently running
          if profile.process_id.is_none() {
            profiles_to_update.push(profile);
            older_versions_to_remove.insert(profile.version.clone());
          } else {
            log::info!(
              "Skipping version update for running profile: {} ({})",
              profile.name,
              profile.version
            );
          }
        }

        // Update profiles to latest version
        for profile in &profiles_to_update {
          match self.profile_manager.update_profile_version(
            app_handle,
            &profile.id.to_string(),
            latest_version,
          ) {
            Ok(_) => {
              consolidated.push(format!(
                "Updated profile '{}' from {} to {}",
                profile.name, profile.version, latest_version
              ));
            }
            Err(e) => {
              log::error!("Failed to update profile '{}': {}", profile.name, e);
            }
          }
        }

        // Remove older version binaries that are no longer needed
        for old_version in &older_versions_to_remove {
          log::info!("Consolidating: removing old version {browser_name} {old_version}");
          match self.cleanup_failed_download(browser_name, old_version) {
            Ok(_) => {
              consolidated.push(format!("Removed old version: {browser_name} {old_version}"));
              log::info!("Successfully removed old version: {browser_name} {old_version}");
            }
            Err(e) => {
              log::error!("Failed to cleanup old version {browser_name} {old_version}: {e}");
            }
          }
        }
      }
    }

    // Save registry after consolidation
    self
      .save()
      .map_err(|e| format!("Failed to save registry after consolidation: {e}"))?;

    log::info!(
      "Browser version consolidation completed: {} actions taken",
      consolidated.len()
    );
    Ok(consolidated)
  }

  /// Check if browser binaries exist for all profiles and return missing binaries
  pub async fn check_missing_binaries(
    &self,
  ) -> Result<Vec<(String, String, String)>, Box<dyn std::error::Error + Send + Sync>> {
    use crate::browser::{create_browser, BrowserType};
    // Get all profiles
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    let mut missing_binaries = Vec::new();

    for profile in profiles {
      let browser_type = match BrowserType::from_str(&profile.browser) {
        Ok(bt) => bt,
        Err(_) => {
          log::info!(
            "Warning: Invalid browser type '{}' for profile '{}'",
            profile.browser,
            profile.name
          );
          continue;
        }
      };

      let browser = create_browser(browser_type.clone());

      // Get binaries directory
      let binaries_dir = if let Some(base_dirs) = directories::BaseDirs::new() {
        let mut path = base_dirs.data_local_dir().to_path_buf();
        path.push(if cfg!(debug_assertions) {
          "DonutBrowserDev"
        } else {
          "DonutBrowser"
        });
        path.push("binaries");
        path
      } else {
        return Err("Failed to get base directories".into());
      };

      log::info!(
        "binaries_dir: {binaries_dir:?} for profile: {}",
        profile.name
      );

      // Check if the version is downloaded
      if !browser.is_version_downloaded(&profile.version, &binaries_dir) {
        missing_binaries.push((profile.name, profile.browser, profile.version));
      }
    }

    Ok(missing_binaries)
  }

  /// Automatically download missing binaries for all profiles
  pub async fn ensure_all_binaries_exist(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // First, clean up any stale registry entries
    if let Ok(cleaned_up) = self.verify_and_cleanup_stale_entries() {
      if !cleaned_up.is_empty() {
        log::info!(
          "Cleaned up {} stale registry entries: {}",
          cleaned_up.len(),
          cleaned_up.join(", ")
        );
      }
    }

    // Consolidate browser versions - keep only latest version per browser
    if let Ok(consolidated) = self.consolidate_browser_versions(app_handle) {
      if !consolidated.is_empty() {
        log::info!("Version consolidation results:");
        for action in &consolidated {
          log::info!("  {action}");
        }
      }
    }

    let missing_binaries = self.check_missing_binaries().await?;
    let mut downloaded = Vec::new();

    for (profile_name, browser, version) in missing_binaries {
      log::info!("Downloading missing binary for profile '{profile_name}': {browser} {version}");

      match crate::downloader::download_browser(
        app_handle.clone(),
        browser.clone(),
        version.clone(),
      )
      .await
      {
        Ok(_) => {
          downloaded.push(format!(
            "{browser} {version} (for profile '{profile_name}')"
          ));

          // After successful download, update profiles that use this browser to the new version
          match self
            .update_profiles_to_version(app_handle, &browser, &version)
            .await
          {
            Ok(updated_profiles) => {
              if !updated_profiles.is_empty() {
                log::info!(
                  "Successfully updated {} profiles to version {}:",
                  updated_profiles.len(),
                  version
                );
                for update_msg in updated_profiles {
                  log::info!("  {update_msg}");
                }
              }
            }
            Err(e) => {
              log::error!("CRITICAL: Failed to update profiles to version {version}: {e}");
              log::error!("This may cause profile version inconsistencies and cleanup issues");
            }
          }
        }
        Err(e) => {
          log::error!("Failed to download {browser} {version} for profile '{profile_name}': {e}");
        }
      }
    }

    // Check if GeoIP database is missing for Camoufox profiles
    if self.geoip_downloader.check_missing_geoip_database()? {
      log::info!("GeoIP database is missing for Camoufox profiles, downloading...");

      match self
        .geoip_downloader
        .download_geoip_database(app_handle)
        .await
      {
        Ok(_) => {
          downloaded.push("GeoIP database for Camoufox".to_string());
          log::info!("GeoIP database downloaded successfully");
        }
        Err(e) => {
          log::error!("Failed to download GeoIP database: {e}");
          // Don't fail the entire operation if GeoIP download fails
        }
      }
    }

    Ok(downloaded)
  }

  /// Update all profiles using a specific browser to a new version
  async fn update_profiles_to_version(
    &self,
    app_handle: &tauri::AppHandle,
    browser: &str,
    version: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let mut updated_profiles = Vec::new();

    for profile in profiles {
      if profile.browser == browser && profile.version != version {
        // Check if profile is currently running
        if profile.process_id.is_some() {
          log::info!(
            "Skipping version update for running profile: {} ({})",
            profile.name,
            profile.version
          );
          continue;
        }

        // Update the profile version
        match self.profile_manager.update_profile_version(
          app_handle,
          &profile.id.to_string(),
          version,
        ) {
          Ok(_) => {
            updated_profiles.push(format!(
              "Updated profile '{}' from {} to {}",
              profile.name, profile.version, version
            ));
            log::info!(
              "Successfully updated profile '{}' to version {}",
              profile.name,
              version
            );

            // Save registry after each profile update to ensure consistency
            if let Err(e) = self.save() {
              log::warn!("Warning: Failed to save registry after profile update: {e}");
            }
          }
          Err(e) => {
            log::error!("Failed to update profile '{}': {}", profile.name, e);
          }
        }
      }
    }

    Ok(updated_profiles)
  }

  /// Cleanup unused binaries based on active and running profiles
  pub fn cleanup_unused_binaries(
    &self,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // Load current profiles using injected ProfileManager
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    // Get active browser versions (all profiles)
    let active_versions = self.get_active_browser_versions(&profiles);

    // Get running browser versions (only running profiles)
    let running_versions = self.get_running_browser_versions(&profiles);

    // Get binaries directory from profile manager
    let binaries_dir = self.profile_manager.get_binaries_dir();

    // Use comprehensive cleanup that syncs registry with disk and removes unused binaries
    let cleaned_up =
      self.comprehensive_cleanup(&binaries_dir, &active_versions, &running_versions)?;

    // Registry is already saved by comprehensive_cleanup
    Ok(cleaned_up)
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref DOWNLOADED_BROWSERS_REGISTRY: DownloadedBrowsersRegistry = {
    let registry = DownloadedBrowsersRegistry::new();
    if let Err(e) = registry.load() {
      log::warn!("Warning: Failed to load downloaded browsers registry: {e}");
    }
    registry
  };
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_registry_creation() {
    // Create a mock profile manager for testing
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

    assert!(registry.is_browser_registered("firefox", "139.0"));
    assert!(!registry.is_browser_registered("firefox", "140.0"));
    assert!(!registry.is_browser_registered("chrome", "139.0"));
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

    // Should NOT be registered until verification completes
    assert!(
      !registry.is_browser_registered("firefox", "139.0"),
      "Browser should NOT be registered after marking as started (only after verification)"
    );

    // Mark as completed (after verification)
    registry
      .mark_download_completed("firefox", "139.0", PathBuf::from("/test/path"))
      .expect("Failed to mark download as completed");

    // Should now be registered
    assert!(
      registry.is_browser_registered("firefox", "139.0"),
      "Browser should be registered after verification completes"
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
      registry.is_browser_registered("firefox", "139.0"),
      "Browser should be registered after adding"
    );

    let removed = registry.remove_browser("firefox", "139.0");
    assert!(
      removed.is_some(),
      "Remove operation should return the removed browser info"
    );
    assert!(
      !registry.is_browser_registered("firefox", "139.0"),
      "Browser should not be registered after removal"
    );
  }

  #[test]
  fn test_twilight_download() {
    let registry = DownloadedBrowsersRegistry::new();

    // Mark twilight download started
    registry.mark_download_started("zen", "twilight", PathBuf::from("/test/zen-twilight"));

    // Should NOT be registered until verification completes
    assert!(
      !registry.is_browser_registered("zen", "twilight"),
      "Zen twilight version should NOT be registered until verification completes"
    );

    // Mark as completed (after verification)
    registry
      .mark_download_completed("zen", "twilight", PathBuf::from("/test/zen-twilight"))
      .expect("Failed to mark twilight download as completed");

    // Now it should be registered
    assert!(
      registry.is_browser_registered("zen", "twilight"),
      "Zen twilight version should be registered after verification completes"
    );
  }

  #[test]
  fn test_is_browser_registered_vs_downloaded() {
    let registry = DownloadedBrowsersRegistry::new();
    let info = DownloadedBrowserInfo {
      browser: "firefox".to_string(),
      version: "139.0".to_string(),
      file_path: PathBuf::from("/test/path"),
    };

    // Add browser to registry
    registry.add_browser(info);

    // Should be registered (in-memory check)
    assert!(
      registry.is_browser_registered("firefox", "139.0"),
      "Browser should be registered after adding to registry"
    );

    // is_browser_downloaded should return false in test environment because files don't exist
    // This tests the difference between registered (in registry) vs downloaded (files exist)
    assert!(
      !registry.is_browser_downloaded("firefox", "139.0"),
      "Browser should not be considered downloaded when files don't exist on disk"
    );
  }
}

#[tauri::command]
pub fn get_downloaded_browser_versions(browser_str: String) -> Result<Vec<String>, String> {
  let registry = DownloadedBrowsersRegistry::instance();
  Ok(registry.get_downloaded_versions(&browser_str))
}

#[tauri::command]
pub fn is_browser_downloaded(browser_str: String, version: String) -> bool {
  let registry = DownloadedBrowsersRegistry::instance();
  registry.is_browser_downloaded(&browser_str, &version)
}

#[tauri::command]
pub async fn check_missing_binaries() -> Result<Vec<(String, String, String)>, String> {
  let registry = DownloadedBrowsersRegistry::instance();
  registry
    .check_missing_binaries()
    .await
    .map_err(|e| format!("Failed to check missing binaries: {e}"))
}

#[tauri::command]
pub async fn ensure_all_binaries_exist(
  app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
  let registry = DownloadedBrowsersRegistry::instance();
  registry
    .ensure_all_binaries_exist(&app_handle)
    .await
    .map_err(|e| format!("Failed to ensure all binaries exist: {e}"))
}

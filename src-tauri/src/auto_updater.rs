use crate::api_client::is_browser_version_nightly;
use crate::browser_runner::{BrowserProfile, BrowserRunner};
use crate::browser_version_service::{BrowserVersionInfo, BrowserVersionService};
use crate::settings_manager::SettingsManager;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use tauri::Emitter;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateNotification {
  pub id: String,
  pub browser: String,
  pub current_version: String,
  pub new_version: String,
  pub affected_profiles: Vec<String>,
  pub is_stable_update: bool,
  pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AutoUpdateState {
  pub pending_updates: Vec<UpdateNotification>,
  pub disabled_browsers: HashSet<String>, // browsers disabled during update
  #[serde(default)]
  pub auto_update_downloads: HashSet<String>, // track auto-update downloads for toast suppression
  pub last_check_timestamp: u64,
}

pub struct AutoUpdater {
  version_service: BrowserVersionService,
  browser_runner: BrowserRunner,
  settings_manager: SettingsManager,
}

impl AutoUpdater {
  pub fn new() -> Self {
    Self {
      version_service: BrowserVersionService::new(),
      browser_runner: BrowserRunner::new(),
      settings_manager: SettingsManager::new(),
    }
  }

  /// Check for updates for all profiles
  pub async fn check_for_updates(
    &self,
  ) -> Result<Vec<UpdateNotification>, Box<dyn std::error::Error + Send + Sync>> {
    let mut notifications = Vec::new();
    let mut browser_versions: HashMap<String, Vec<BrowserVersionInfo>> = HashMap::new();

    // Group profiles by browser
    let profiles = self
      .browser_runner
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    let mut browser_profiles: HashMap<String, Vec<BrowserProfile>> = HashMap::new();

    for profile in profiles {
      // Only check supported browsers
      if !self
        .version_service
        .is_browser_supported(&profile.browser)
        .unwrap_or(false)
      {
        continue;
      }

      browser_profiles
        .entry(profile.browser.clone())
        .or_default()
        .push(profile);
    }

    for (browser, profiles) in browser_profiles {
      // Get cached versions first, then try to fetch if needed
      let versions = if let Some(cached) = self
        .version_service
        .get_cached_browser_versions_detailed(&browser)
      {
        cached
      } else if self.version_service.should_update_cache(&browser) {
        // Try to fetch fresh versions
        match self
          .version_service
          .fetch_browser_versions_detailed(&browser, false)
          .await
        {
          Ok(versions) => versions,
          Err(_) => continue, // Skip this browser if fetch fails
        }
      } else {
        continue; // No cached versions and cache doesn't need update
      };

      browser_versions.insert(browser.clone(), versions.clone());

      // Check each profile for updates
      for profile in profiles {
        if let Some(update) = self.check_profile_update(&profile, &versions)? {
          // Apply chromium threshold logic
          if browser == "chromium" {
            // For chromium, only show notifications if there are 200+ new versions
            let current_version = &profile.version.parse::<u32>().unwrap();
            let new_version = &update.new_version.parse::<u32>().unwrap();

            let result = new_version - current_version;
            println!(
              "Current version: {current_version}, New version: {new_version}, Result: {result}"
            );
            if result > 200 {
              notifications.push(update);
            } else {
              println!(
                "Skipping chromium update notification: only {result} new versions (need 50+)"
              );
            }
          } else {
            notifications.push(update);
          }
        }
      }
    }

    Ok(notifications)
  }

  pub async fn check_for_updates_with_progress(&self, app_handle: &tauri::AppHandle) {
    println!("Starting auto-update check with progress...");

    // Check for browser updates and trigger auto-downloads
    match self.check_for_updates().await {
      Ok(update_notifications) => {
        if !update_notifications.is_empty() {
          println!(
            "Found {} browser updates to auto-download",
            update_notifications.len()
          );

          // Trigger automatic downloads for each update
          for notification in update_notifications {
            println!(
              "Auto-downloading {} version {}",
              notification.browser, notification.new_version
            );

            // Clone app_handle for the async task
            let app_handle_clone = app_handle.clone();
            let browser = notification.browser.clone();
            let new_version = notification.new_version.clone();
            let notification_id = notification.id.clone();
            let affected_profiles = notification.affected_profiles.clone();

            // Spawn async task to handle the download and auto-update
            tokio::spawn(async move {
              // First, check if browser already exists
              match crate::browser_runner::is_browser_downloaded(
                browser.clone(),
                new_version.clone(),
              ) {
                true => {
                  println!("Browser {browser} {new_version} already downloaded, proceeding to auto-update profiles");

                  // Browser already exists, go straight to profile update
                  match crate::auto_updater::complete_browser_update_with_auto_update(
                    browser.clone(),
                    new_version.clone(),
                  )
                  .await
                  {
                    Ok(updated_profiles) => {
                      println!(
                        "Auto-update completed for {} profiles: {:?}",
                        updated_profiles.len(),
                        updated_profiles
                      );
                    }
                    Err(e) => {
                      eprintln!("Failed to complete auto-update for {browser}: {e}");
                    }
                  }
                }
                false => {
                  println!("Downloading browser {browser} version {new_version}...");

                  // Emit the auto-update event to trigger frontend handling
                  let auto_update_event = serde_json::json!({
                    "browser": browser,
                    "new_version": new_version,
                    "notification_id": notification_id,
                    "affected_profiles": affected_profiles
                  });

                  if let Err(e) =
                    app_handle_clone.emit("browser-auto-update-available", &auto_update_event)
                  {
                    eprintln!("Failed to emit auto-update event for {browser}: {e}");
                  } else {
                    println!("Emitted auto-update event for {browser}");
                  }
                }
              }
            });
          }
        } else {
          println!("No browser updates needed");
        }
      }
      Err(e) => {
        eprintln!("Failed to check for browser updates: {e}");
      }
    }
  }

  /// Check if a specific profile has an available update
  fn check_profile_update(
    &self,
    profile: &BrowserProfile,
    available_versions: &[BrowserVersionInfo],
  ) -> Result<Option<UpdateNotification>, Box<dyn std::error::Error + Send + Sync>> {
    let current_version = &profile.version;
    let is_current_nightly = is_browser_version_nightly(&profile.browser, current_version, None);

    // Find the best available update
    let best_update = available_versions
      .iter()
      .filter(|v| {
        // Only consider versions newer than current
        self.is_version_newer(&v.version, current_version)
          && is_browser_version_nightly(&profile.browser, &v.version, None) == is_current_nightly
      })
      .max_by(|a, b| self.compare_versions(&a.version, &b.version));

    if let Some(update_version) = best_update {
      let notification = UpdateNotification {
        id: format!(
          "{}_{}_to_{}",
          profile.browser, current_version, update_version.version
        ),
        browser: profile.browser.clone(),
        current_version: current_version.clone(),
        new_version: update_version.version.clone(),
        affected_profiles: vec![profile.name.clone()],
        is_stable_update: !update_version.is_prerelease,
        timestamp: std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap()
          .as_secs(),
      };
      Ok(Some(notification))
    } else {
      Ok(None)
    }
  }

  /// Group update notifications by browser and version
  pub fn group_update_notifications(
    &self,
    notifications: Vec<UpdateNotification>,
  ) -> Vec<UpdateNotification> {
    let mut grouped: HashMap<String, UpdateNotification> = HashMap::new();

    for notification in notifications {
      let key = format!("{}_{}", notification.browser, notification.new_version);

      if let Some(existing) = grouped.get_mut(&key) {
        // Merge affected profiles
        existing
          .affected_profiles
          .extend(notification.affected_profiles);
        existing.affected_profiles.sort();
        existing.affected_profiles.dedup();
      } else {
        grouped.insert(key, notification);
      }
    }

    let mut result: Vec<UpdateNotification> = grouped.into_values().collect();

    // Sort by priority: stable updates first, then by timestamp
    result.sort_by(|a, b| match (a.is_stable_update, b.is_stable_update) {
      (true, false) => std::cmp::Ordering::Less,
      (false, true) => std::cmp::Ordering::Greater,
      _ => b.timestamp.cmp(&a.timestamp),
    });

    result
  }

  /// Automatically update all affected profile versions after browser download
  pub async fn auto_update_profile_versions(
    &self,
    browser: &str,
    new_version: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let profiles = self
      .browser_runner
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let mut updated_profiles = Vec::new();

    // Find all profiles for this browser that should be updated
    for profile in profiles {
      if profile.browser == browser {
        // Check if profile is currently running
        if profile.process_id.is_some() {
          continue; // Skip running profiles
        }

        // Check if this is an update (newer version)
        if self.is_version_newer(new_version, &profile.version) {
          // Update the profile version
          match self
            .browser_runner
            .update_profile_version(&profile.name, new_version)
          {
            Ok(_) => {
              updated_profiles.push(profile.name);
            }
            Err(e) => {
              eprintln!("Failed to update profile {}: {}", profile.name, e);
            }
          }
        }
      }
    }

    Ok(updated_profiles)
  }

  /// Complete browser update process with auto-update of profile versions
  pub async fn complete_browser_update_with_auto_update(
    &self,
    browser: &str,
    new_version: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // Auto-update profile versions first
    let updated_profiles = self
      .auto_update_profile_versions(browser, new_version)
      .await?;

    // Remove browser from disabled list and clean up auto-update tracking
    let mut state = self.load_auto_update_state()?;
    state.disabled_browsers.remove(browser);
    let download_key = format!("{browser}-{new_version}");
    state.auto_update_downloads.remove(&download_key);
    self.save_auto_update_state(&state)?;

    // Always perform cleanup after auto-update - don't fail the update if cleanup fails
    if let Err(e) = self.cleanup_unused_binaries_internal() {
      eprintln!("Warning: Failed to cleanup unused binaries after auto-update: {e}");
    }

    Ok(updated_profiles)
  }

  /// Internal method to cleanup unused binaries (used by auto-cleanup)
  fn cleanup_unused_binaries_internal(
    &self,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // Load current profiles
    let profiles = self
      .browser_runner
      .list_profiles()
      .map_err(|e| format!("Failed to load profiles: {e}"))?;

    // Load registry
    let mut registry = crate::downloaded_browsers::DownloadedBrowsersRegistry::load()
      .map_err(|e| format!("Failed to load browser registry: {e}"))?;

    // Get active browser versions
    let active_versions = registry.get_active_browser_versions(&profiles);

    // Cleanup unused binaries
    let cleaned_up = registry
      .cleanup_unused_binaries(&active_versions)
      .map_err(|e| format!("Failed to cleanup unused binaries: {e}"))?;

    // Save updated registry
    registry
      .save()
      .map_err(|e| format!("Failed to save registry: {e}"))?;

    Ok(cleaned_up)
  }

  /// Check if browser is disabled due to ongoing update
  pub fn is_browser_disabled(
    &self,
    browser: &str,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let state = self.load_auto_update_state()?;
    Ok(state.disabled_browsers.contains(browser))
  }

  /// Dismiss update notification
  pub fn dismiss_update_notification(
    &self,
    notification_id: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut state = self.load_auto_update_state()?;
    state.pending_updates.retain(|n| n.id != notification_id);
    self.save_auto_update_state(&state)?;
    Ok(())
  }

  fn is_version_newer(&self, version1: &str, version2: &str) -> bool {
    // Use the proper VersionComponent comparison from api_client.rs
    let version_a = crate::api_client::VersionComponent::parse(version1);
    let version_b = crate::api_client::VersionComponent::parse(version2);
    version_a > version_b
  }

  fn compare_versions(&self, version1: &str, version2: &str) -> std::cmp::Ordering {
    // Use the proper VersionComponent comparison from api_client.rs
    let version_a = crate::api_client::VersionComponent::parse(version1);
    let version_b = crate::api_client::VersionComponent::parse(version2);
    version_a.cmp(&version_b)
  }

  fn get_auto_update_state_file(&self) -> PathBuf {
    self
      .settings_manager
      .get_settings_dir()
      .join("auto_update_state.json")
  }

  fn load_auto_update_state(
    &self,
  ) -> Result<AutoUpdateState, Box<dyn std::error::Error + Send + Sync>> {
    let state_file = self.get_auto_update_state_file();

    if !state_file.exists() {
      return Ok(AutoUpdateState::default());
    }

    let content = fs::read_to_string(state_file)?;
    let state: AutoUpdateState = serde_json::from_str(&content)?;
    Ok(state)
  }

  fn save_auto_update_state(
    &self,
    state: &AutoUpdateState,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let settings_dir = self.settings_manager.get_settings_dir();
    std::fs::create_dir_all(&settings_dir)?;

    let state_file = self.get_auto_update_state_file();
    let json = serde_json::to_string_pretty(state)?;
    fs::write(state_file, json)?;

    Ok(())
  }
}

// Tauri commands

#[tauri::command]
pub async fn check_for_browser_updates() -> Result<Vec<UpdateNotification>, String> {
  let updater = AutoUpdater::new();
  let notifications = updater
    .check_for_updates()
    .await
    .map_err(|e| format!("Failed to check for updates: {e}"))?;
  let grouped = updater.group_update_notifications(notifications);
  Ok(grouped)
}

#[tauri::command]
pub async fn is_browser_disabled_for_update(browser: String) -> Result<bool, String> {
  let updater = AutoUpdater::new();
  updater
    .is_browser_disabled(&browser)
    .map_err(|e| format!("Failed to check browser status: {e}"))
}

#[tauri::command]
pub async fn dismiss_update_notification(notification_id: String) -> Result<(), String> {
  let updater = AutoUpdater::new();
  updater
    .dismiss_update_notification(&notification_id)
    .map_err(|e| format!("Failed to dismiss notification: {e}"))
}

#[tauri::command]
pub async fn complete_browser_update_with_auto_update(
  browser: String,
  new_version: String,
) -> Result<Vec<String>, String> {
  let updater = AutoUpdater::new();
  updater
    .complete_browser_update_with_auto_update(&browser, &new_version)
    .await
    .map_err(|e| format!("Failed to complete browser update: {e}"))
}

#[tauri::command]
pub async fn check_for_updates_with_progress(app_handle: tauri::AppHandle) {
  let updater = AutoUpdater::new();
  updater.check_for_updates_with_progress(&app_handle).await;
}

#[cfg(test)]
mod tests {
  use super::*;

  fn create_test_profile(name: &str, browser: &str, version: &str) -> BrowserProfile {
    BrowserProfile {
      id: uuid::Uuid::new_v4(),
      name: name.to_string(),
      browser: browser.to_string(),
      version: version.to_string(),
      process_id: None,
      proxy_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      camoufox_config: None,
      group_id: None,
    }
  }

  fn create_test_version_info(version: &str, is_prerelease: bool) -> BrowserVersionInfo {
    BrowserVersionInfo {
      version: version.to_string(),
      is_prerelease,
      date: "2024-01-01".to_string(),
    }
  }

  #[test]
  fn test_compare_versions() {
    let updater = AutoUpdater::new();

    assert_eq!(
      updater.compare_versions("1.0.0", "1.0.0"),
      std::cmp::Ordering::Equal
    );
    assert_eq!(
      updater.compare_versions("1.0.1", "1.0.0"),
      std::cmp::Ordering::Greater
    );
    assert_eq!(
      updater.compare_versions("1.0.0", "1.0.1"),
      std::cmp::Ordering::Less
    );
    assert_eq!(
      updater.compare_versions("2.0.0", "1.9.9"),
      std::cmp::Ordering::Greater
    );
    assert_eq!(
      updater.compare_versions("1.10.0", "1.9.0"),
      std::cmp::Ordering::Greater
    );
  }

  #[test]
  fn test_is_version_newer() {
    let updater = AutoUpdater::new();

    assert!(updater.is_version_newer("1.0.1", "1.0.0"));
    assert!(updater.is_version_newer("2.0.0", "1.9.9"));
    assert!(!updater.is_version_newer("1.0.0", "1.0.1"));
    assert!(!updater.is_version_newer("1.0.0", "1.0.0"));
  }

  #[test]
  fn test_camoufox_beta_version_comparison() {
    let updater = AutoUpdater::new();

    // Test the exact user-reported scenario: 135.0.1beta24 vs 135.0beta22
    assert!(
      updater.is_version_newer("135.0.1beta24", "135.0beta22"),
      "135.0.1beta24 should be newer than 135.0beta22"
    );

    assert_eq!(
      updater.compare_versions("135.0.1beta24", "135.0beta22"),
      std::cmp::Ordering::Greater,
      "135.0.1beta24 should compare as greater than 135.0beta22"
    );

    // Test other camoufox beta version combinations
    assert!(
      updater.is_version_newer("135.0.5beta24", "135.0.5beta22"),
      "135.0.5beta24 should be newer than 135.0.5beta22"
    );

    assert!(
      updater.is_version_newer("135.0.1beta1", "135.0beta1"),
      "135.0.1beta1 should be newer than 135.0beta1 due to patch version"
    );

    // Test that older versions are not considered newer
    assert!(
      !updater.is_version_newer("135.0beta22", "135.0.1beta24"),
      "135.0beta22 should NOT be newer than 135.0.1beta24"
    );
  }

  #[test]
  fn test_beta_version_ordering_comprehensive() {
    let updater = AutoUpdater::new();

    // Test various beta version patterns that could appear in camoufox
    let test_cases = vec![
      ("135.0.1beta24", "135.0beta22", true),   // User reported case
      ("135.0.5beta24", "135.0.5beta22", true), // Same patch, different beta
      ("135.1beta1", "135.0beta99", true),      // Higher minor beats beta number
      ("136.0beta1", "135.9.9beta99", true),    // Higher major beats everything
      ("135.0.1beta1", "135.0beta1", true),     // Patch version matters
      ("135.0beta22", "135.0.1beta24", false),  // Reverse of user case
    ];

    for (newer, older, should_be_newer) in test_cases {
      let result = updater.is_version_newer(newer, older);
      assert_eq!(
        result,
        should_be_newer,
        "Expected {} {} {} but got {}",
        newer,
        if should_be_newer { ">" } else { "<=" },
        older,
        if result { "true" } else { "false" }
      );
    }
  }

  #[test]
  fn test_check_profile_update_stable_to_stable() {
    let updater = AutoUpdater::new();
    let profile = create_test_profile("test", "firefox", "1.0.0");
    let versions = vec![
      create_test_version_info("1.0.1", false), // stable, newer
      create_test_version_info("1.1.0-alpha", true), // alpha, should be ignored
      create_test_version_info("0.9.0", false), // stable, older
    ];

    let result = updater.check_profile_update(&profile, &versions).unwrap();
    assert!(result.is_some());

    let update = result.unwrap();
    assert_eq!(update.new_version, "1.0.1");
    assert!(update.is_stable_update);
  }

  #[test]
  fn test_check_profile_update_alpha_to_alpha() {
    let updater = AutoUpdater::new();
    let profile = create_test_profile("test", "firefox", "1.0.0-alpha");
    let versions = vec![
      create_test_version_info("1.0.1", false), // stable, should be included
      create_test_version_info("1.1.0-alpha", true), // alpha, newer
      create_test_version_info("0.9.0-alpha", true), // alpha, older
    ];

    let result = updater.check_profile_update(&profile, &versions).unwrap();
    assert!(result.is_some());

    let update = result.unwrap();
    // Should pick the newest version (alpha user can upgrade to stable or newer alpha)
    assert_eq!(update.new_version, "1.1.0-alpha");
    assert!(!update.is_stable_update);
  }

  #[test]
  fn test_check_profile_update_no_update_available() {
    let updater = AutoUpdater::new();
    let profile = create_test_profile("test", "firefox", "1.0.0");
    let versions = vec![
      create_test_version_info("0.9.0", false), // older
      create_test_version_info("1.0.0", false), // same version
    ];

    let result = updater.check_profile_update(&profile, &versions).unwrap();
    assert!(result.is_none());
  }

  #[test]
  fn test_group_update_notifications() {
    let updater = AutoUpdater::new();
    let notifications = vec![
      UpdateNotification {
        id: "firefox_1.0.0_to_1.1.0_profile1".to_string(),
        browser: "firefox".to_string(),
        current_version: "1.0.0".to_string(),
        new_version: "1.1.0".to_string(),
        affected_profiles: vec!["profile1".to_string()],
        is_stable_update: true,
        timestamp: 1000,
      },
      UpdateNotification {
        id: "firefox_1.0.0_to_1.1.0_profile2".to_string(),
        browser: "firefox".to_string(),
        current_version: "1.0.0".to_string(),
        new_version: "1.1.0".to_string(),
        affected_profiles: vec!["profile2".to_string()],
        is_stable_update: true,
        timestamp: 1001,
      },
      UpdateNotification {
        id: "chrome_1.0.0_to_1.1.0-alpha".to_string(),
        browser: "chrome".to_string(),
        current_version: "1.0.0".to_string(),
        new_version: "1.1.0-alpha".to_string(),
        affected_profiles: vec!["profile3".to_string()],
        is_stable_update: false,
        timestamp: 1002,
      },
    ];

    let grouped = updater.group_update_notifications(notifications);

    assert_eq!(grouped.len(), 2);

    // Find the Firefox notification
    let firefox_notification = grouped.iter().find(|n| n.browser == "firefox").unwrap();
    assert_eq!(firefox_notification.affected_profiles.len(), 2);
    assert!(firefox_notification
      .affected_profiles
      .contains(&"profile1".to_string()));
    assert!(firefox_notification
      .affected_profiles
      .contains(&"profile2".to_string()));

    // Stable updates should come first
    assert!(grouped[0].is_stable_update);
  }

  #[test]
  fn test_auto_update_state_persistence() {
    use std::sync::Once;
    use tempfile::TempDir;

    static INIT: Once = Once::new();
    INIT.call_once(|| {
      // Initialize any required static data
    });

    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();

    // Create a mock settings manager that uses the temp directory
    struct TestSettingsManager {
      settings_dir: std::path::PathBuf,
    }

    impl TestSettingsManager {
      fn new(settings_dir: std::path::PathBuf) -> Self {
        Self { settings_dir }
      }

      fn get_settings_dir(&self) -> std::path::PathBuf {
        self.settings_dir.clone()
      }
    }

    let test_settings_manager = TestSettingsManager::new(temp_dir.path().to_path_buf());

    let mut state = AutoUpdateState::default();
    state.disabled_browsers.insert("firefox".to_string());
    state
      .auto_update_downloads
      .insert("firefox-1.1.0".to_string());
    state.pending_updates.push(UpdateNotification {
      id: "test".to_string(),
      browser: "firefox".to_string(),
      current_version: "1.0.0".to_string(),
      new_version: "1.1.0".to_string(),
      affected_profiles: vec!["profile1".to_string()],
      is_stable_update: true,
      timestamp: 1000,
    });

    // Test save and load
    let state_file = test_settings_manager
      .get_settings_dir()
      .join("auto_update_state.json");
    std::fs::create_dir_all(test_settings_manager.get_settings_dir()).unwrap();
    let json = serde_json::to_string_pretty(&state).unwrap();
    std::fs::write(&state_file, json).unwrap();

    // Load state
    let content = std::fs::read_to_string(&state_file).unwrap();
    let loaded_state: AutoUpdateState = serde_json::from_str(&content).unwrap();

    assert_eq!(loaded_state.disabled_browsers.len(), 1);
    assert!(loaded_state.disabled_browsers.contains("firefox"));
    assert_eq!(loaded_state.auto_update_downloads.len(), 1);
    assert!(loaded_state.auto_update_downloads.contains("firefox-1.1.0"));
    assert_eq!(loaded_state.pending_updates.len(), 1);
    assert_eq!(loaded_state.pending_updates[0].id, "test");
  }

  #[tokio::test]
  async fn test_browser_disable_enable_cycle() {
    use tempfile::TempDir;

    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();

    // Create a mock settings manager that uses the temp directory
    struct TestSettingsManager {
      settings_dir: std::path::PathBuf,
    }

    impl TestSettingsManager {
      fn new(settings_dir: std::path::PathBuf) -> Self {
        Self { settings_dir }
      }

      fn get_settings_dir(&self) -> std::path::PathBuf {
        self.settings_dir.clone()
      }
    }

    let test_settings_manager = TestSettingsManager::new(temp_dir.path().to_path_buf());

    // Test browser disable/enable cycle with manual state management
    let state_file = test_settings_manager
      .get_settings_dir()
      .join("auto_update_state.json");
    std::fs::create_dir_all(test_settings_manager.get_settings_dir()).unwrap();

    // Initially not disabled (empty state file means default state)
    let state = AutoUpdateState::default();
    assert!(!state.disabled_browsers.contains("firefox"));

    // Start update (should disable)
    let mut state = AutoUpdateState::default();
    state.disabled_browsers.insert("firefox".to_string());
    state
      .auto_update_downloads
      .insert("firefox-1.1.0".to_string());
    let json = serde_json::to_string_pretty(&state).unwrap();
    std::fs::write(&state_file, json).unwrap();

    // Check that it's disabled
    let content = std::fs::read_to_string(&state_file).unwrap();
    let loaded_state: AutoUpdateState = serde_json::from_str(&content).unwrap();
    assert!(loaded_state.disabled_browsers.contains("firefox"));
    assert!(loaded_state.auto_update_downloads.contains("firefox-1.1.0"));

    // Complete update (should enable)
    let mut state = loaded_state;
    state.disabled_browsers.remove("firefox");
    state.auto_update_downloads.remove("firefox-1.1.0");
    let json = serde_json::to_string_pretty(&state).unwrap();
    std::fs::write(&state_file, json).unwrap();

    // Check that it's enabled again
    let content = std::fs::read_to_string(&state_file).unwrap();
    let final_state: AutoUpdateState = serde_json::from_str(&content).unwrap();
    assert!(!final_state.disabled_browsers.contains("firefox"));
    assert!(!final_state.auto_update_downloads.contains("firefox-1.1.0"));
  }

  #[test]
  fn test_dismiss_update_notification() {
    use tempfile::TempDir;

    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();

    // Create a mock settings manager that uses the temp directory
    struct TestSettingsManager {
      settings_dir: std::path::PathBuf,
    }

    impl TestSettingsManager {
      fn new(settings_dir: std::path::PathBuf) -> Self {
        Self { settings_dir }
      }

      fn get_settings_dir(&self) -> std::path::PathBuf {
        self.settings_dir.clone()
      }
    }

    let test_settings_manager = TestSettingsManager::new(temp_dir.path().to_path_buf());

    let mut state = AutoUpdateState::default();
    state.pending_updates.push(UpdateNotification {
      id: "test_notification".to_string(),
      browser: "firefox".to_string(),
      current_version: "1.0.0".to_string(),
      new_version: "1.1.0".to_string(),
      affected_profiles: vec!["profile1".to_string()],
      is_stable_update: true,
      timestamp: 1000,
    });

    // Save initial state
    let state_file = test_settings_manager
      .get_settings_dir()
      .join("auto_update_state.json");
    std::fs::create_dir_all(test_settings_manager.get_settings_dir()).unwrap();
    let json = serde_json::to_string_pretty(&state).unwrap();
    std::fs::write(&state_file, json).unwrap();

    // Dismiss notification (remove from pending updates)
    state
      .pending_updates
      .retain(|n| n.id != "test_notification");
    let json = serde_json::to_string_pretty(&state).unwrap();
    std::fs::write(&state_file, json).unwrap();

    // Check that it's removed
    let content = std::fs::read_to_string(&state_file).unwrap();
    let loaded_state: AutoUpdateState = serde_json::from_str(&content).unwrap();
    assert_eq!(loaded_state.pending_updates.len(), 0);
  }
}

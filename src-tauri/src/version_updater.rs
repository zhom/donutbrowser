use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::time::interval;

use crate::auto_updater::AutoUpdater;
use crate::browser_version_manager::BrowserVersionManager;
use crate::events;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VersionUpdateProgress {
  pub current_browser: String,
  pub total_browsers: usize,
  pub completed_browsers: usize,
  pub new_versions_found: usize,
  pub browser_new_versions: usize, // New versions found for current browser
  pub status: String,              // "updating", "completed", "error"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackgroundUpdateResult {
  pub browser: String,
  pub new_versions_count: usize,
  pub total_versions_count: usize,
  pub updated_successfully: bool,
  pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BackgroundUpdateState {
  last_update_time: u64,
  update_interval_hours: u64,
}

impl Default for BackgroundUpdateState {
  fn default() -> Self {
    Self {
      last_update_time: 0,
      update_interval_hours: 12,
    }
  }
}

/// Extension of auto_updater.rs for background updates
pub struct VersionUpdater {
  browser_version_manager: &'static BrowserVersionManager,
  auto_updater: &'static AutoUpdater,
  app_handle: Option<tauri::AppHandle>,
}

impl VersionUpdater {
  pub fn new() -> Self {
    Self {
      browser_version_manager: BrowserVersionManager::instance(),
      auto_updater: AutoUpdater::instance(),
      app_handle: None,
    }
  }

  pub fn set_app_handle(&mut self, app_handle: tauri::AppHandle) {
    self.app_handle = Some(app_handle);
  }

  fn get_cache_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let base_dirs = BaseDirs::new().ok_or("Failed to get base directories")?;
    let app_name = if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    };
    let cache_dir = base_dirs.cache_dir().join(app_name).join("version_cache");
    fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir)
  }

  pub(crate) fn get_background_update_state_file() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cache_dir = Self::get_cache_dir()?;
    Ok(cache_dir.join("background_update_state.json"))
  }

  pub(crate) fn load_background_update_state() -> BackgroundUpdateState {
    let state_file = match Self::get_background_update_state_file() {
      Ok(file) => file,
      Err(_) => return BackgroundUpdateState::default(),
    };

    if !state_file.exists() {
      return BackgroundUpdateState::default();
    }

    let content = match fs::read_to_string(&state_file) {
      Ok(content) => content,
      Err(_) => return BackgroundUpdateState::default(),
    };

    serde_json::from_str(&content).unwrap_or_default()
  }

  pub(crate) fn save_background_update_state(
    state: &BackgroundUpdateState,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let state_file = Self::get_background_update_state_file()?;
    let content = serde_json::to_string_pretty(state)?;
    fs::write(&state_file, content)?;
    Ok(())
  }

  fn get_current_timestamp() -> u64 {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs()
  }

  fn should_run_background_update() -> bool {
    let state = Self::load_background_update_state();
    let current_time = Self::get_current_timestamp();
    let elapsed_secs = current_time.saturating_sub(state.last_update_time);
    let update_interval_secs = state.update_interval_hours * 60 * 60;

    // Run update if:
    // 1. Never updated before (last_update_time == 0)
    // 2. More than 3 hours have passed since last update
    let should_update = state.last_update_time == 0 || elapsed_secs >= update_interval_secs;

    if should_update {
      log::debug!(
        "Background update needed: last_update={}, elapsed={}h, required={}h",
        state.last_update_time,
        elapsed_secs / 3600,
        state.update_interval_hours
      );
    } else {
      log::debug!(
        "Background update not needed: last_update={}, elapsed={}h, required={}h",
        state.last_update_time,
        elapsed_secs / 3600,
        state.update_interval_hours
      );
    }

    should_update
  }

  pub async fn check_and_run_startup_update(
    &self,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Only run if an update is actually needed
    if !Self::should_run_background_update() {
      log::debug!("No startup version update needed");
      return Ok(());
    }

    if let Some(ref app_handle) = self.app_handle {
      log::info!("Running startup version update...");

      match self.update_all_browser_versions(app_handle).await {
        Ok(_) => {
          // Update the persistent state after successful update
          let state = BackgroundUpdateState {
            last_update_time: Self::get_current_timestamp(),
            update_interval_hours: 3,
          };

          if let Err(e) = Self::save_background_update_state(&state) {
            log::error!("Failed to save background update state: {e}");
          } else {
            log::info!("Startup version update completed successfully");
          }
        }
        Err(e) => {
          log::error!("Startup version update failed: {e}");
          return Err(e);
        }
      }
    } else {
      return Err("App handle not available for startup update".into());
    }

    Ok(())
  }

  pub async fn start_background_updates(
    &self,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
      "Starting background version update service (checking every 5 minutes for 3-hour intervals)"
    );

    // Run initial startup check
    if let Err(e) = self.check_and_run_startup_update().await {
      eprintln!("Startup version update failed: {e}");
    }

    Ok(())
  }

  pub async fn run_background_task() {
    let mut update_interval = interval(Duration::from_secs(5 * 60)); // Check every 5 minutes
    update_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
      update_interval.tick().await;

      // Check if we should run an update based on persistent state
      if !Self::should_run_background_update() {
        continue;
      }

      println!("Starting background version update...");

      // Get the updater instance for this update cycle
      let updater = get_version_updater();
      let result = {
        let updater_guard = updater.lock().await;
        if let Some(ref app_handle) = updater_guard.app_handle {
          updater_guard.update_all_browser_versions(app_handle).await
        } else {
          Err("App handle not available for background update".into())
        }
      }; // Release the lock here

      match result {
        Ok(_) => {
          // Update the persistent state after successful update
          let state = BackgroundUpdateState {
            last_update_time: Self::get_current_timestamp(),
            update_interval_hours: 3,
          };

          if let Err(e) = Self::save_background_update_state(&state) {
            eprintln!("Failed to save background update state: {e}");
          } else {
            println!("Background version update completed successfully");
          }
        }
        Err(e) => {
          eprintln!("Background version update failed: {e}");

          // Try to emit error event if we have an app handle
          let updater_guard = updater.lock().await;
          if let Some(ref _app_handle) = updater_guard.app_handle {
            let progress = VersionUpdateProgress {
              current_browser: "".to_string(),
              total_browsers: 0,
              completed_browsers: 0,
              new_versions_found: 0,
              browser_new_versions: 0,
              status: "error".to_string(),
            };
            let _ = events::emit("version-update-progress", &progress);
          }
        }
      }
    }
  }

  async fn update_all_browser_versions(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<Vec<BackgroundUpdateResult>, Box<dyn std::error::Error + Send + Sync>> {
    let supported_browsers = self.browser_version_manager.get_supported_browsers();
    let total_browsers = supported_browsers.len();
    let mut results = Vec::new();
    let mut total_new_versions = 0;

    // Emit initial progress
    let initial_progress = VersionUpdateProgress {
      current_browser: String::new(),
      total_browsers,
      completed_browsers: 0,
      new_versions_found: 0,
      browser_new_versions: 0,
      status: "updating".to_string(),
    };

    if let Err(e) = events::emit("version-update-progress", &initial_progress) {
      log::error!("Failed to emit initial progress: {e}");
    }

    for (index, browser) in supported_browsers.iter().enumerate() {
      log::debug!("Updating browser versions for: {browser}");

      // Emit progress update for current browser
      let progress = VersionUpdateProgress {
        current_browser: browser.clone(),
        total_browsers,
        completed_browsers: index,
        new_versions_found: total_new_versions,
        browser_new_versions: 0,
        status: "updating".to_string(),
      };

      if let Err(e) = events::emit("version-update-progress", &progress) {
        log::error!("Failed to emit progress for {browser}: {e}");
      }

      match self.update_browser_versions(browser).await {
        Ok(new_versions_count) => {
          results.push(BackgroundUpdateResult {
            browser: browser.clone(),
            new_versions_count,
            total_versions_count: 0, // We don't track total for background updates
            updated_successfully: true,
            error: None,
          });

          total_new_versions += new_versions_count;

          // Emit progress update with new versions found
          let progress = VersionUpdateProgress {
            current_browser: browser.clone(),
            total_browsers,
            completed_browsers: index,
            new_versions_found: total_new_versions,
            browser_new_versions: new_versions_count,
            status: "updating".to_string(),
          };

          if let Err(e) = events::emit("version-update-progress", &progress) {
            log::error!("Failed to emit progress with versions for {browser}: {e}");
          }
        }
        Err(e) => {
          results.push(BackgroundUpdateResult {
            browser: browser.clone(),
            new_versions_count: 0,
            total_versions_count: 0,
            updated_successfully: false,
            error: Some(e.to_string()),
          });
        }
      }
    }

    // Emit completion
    let final_progress = VersionUpdateProgress {
      current_browser: String::new(),
      total_browsers,
      completed_browsers: total_browsers,
      new_versions_found: total_new_versions,
      browser_new_versions: 0,
      status: "completed".to_string(),
    };

    if let Err(e) = events::emit("version-update-progress", &final_progress) {
      eprintln!("Failed to emit completion progress: {e}");
    }

    // After all version updates are complete, trigger auto-update check
    if total_new_versions > 0 {
      println!(
        "Found {total_new_versions} new versions across all browsers. Checking for auto-updates..."
      );

      // Trigger auto-update check which will automatically download browsers
      self
        .auto_updater
        .check_for_updates_with_progress(app_handle)
        .await;
    } else {
      println!("No new versions found, skipping auto-update check");
    }

    Ok(results)
  }

  async fn update_browser_versions(
    &self,
    browser: &str,
  ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    self
      .browser_version_manager
      .update_browser_versions_incrementally(browser)
      .await
  }

  pub async fn trigger_manual_update(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<Vec<BackgroundUpdateResult>, Box<dyn std::error::Error + Send + Sync>> {
    let results = self.update_all_browser_versions(app_handle).await?;

    // Update the persistent state after successful manual update
    let state = BackgroundUpdateState {
      last_update_time: Self::get_current_timestamp(),
      update_interval_hours: 3,
    };

    if let Err(e) = Self::save_background_update_state(&state) {
      log::error!("Failed to save background update state after manual update: {e}");
    }

    Ok(results)
  }

  pub async fn get_last_update_time(&self) -> Option<u64> {
    let state = Self::load_background_update_state();
    if state.last_update_time == 0 {
      None
    } else {
      Some(state.last_update_time)
    }
  }

  pub async fn get_time_until_next_update(&self) -> u64 {
    let state = Self::load_background_update_state();
    let current_time = Self::get_current_timestamp();

    if state.last_update_time == 0 {
      0 // No previous update, should update now
    } else {
      let elapsed = current_time.saturating_sub(state.last_update_time);
      let update_interval_secs = state.update_interval_hours * 60 * 60;

      update_interval_secs.saturating_sub(elapsed)
    }
  }
}

// Global instance
static VERSION_UPDATER: OnceLock<Arc<Mutex<VersionUpdater>>> = OnceLock::new();

pub fn get_version_updater() -> Arc<Mutex<VersionUpdater>> {
  VERSION_UPDATER
    .get_or_init(|| Arc::new(Mutex::new(VersionUpdater::new())))
    .clone()
}

#[tauri::command]
pub async fn trigger_manual_version_update(
  app_handle: tauri::AppHandle,
) -> Result<Vec<BackgroundUpdateResult>, String> {
  let updater = get_version_updater();
  let updater_guard = updater.lock().await;

  updater_guard
    .trigger_manual_update(&app_handle)
    .await
    .map_err(|e| format!("Failed to trigger manual update: {e}"))
}

#[tauri::command]
pub async fn get_version_update_status() -> Result<(Option<u64>, u64), String> {
  let updater = get_version_updater();
  let updater_guard = updater.lock().await;

  let last_update = updater_guard.get_last_update_time().await;
  let time_until_next = updater_guard.get_time_until_next_update().await;

  Ok((last_update, time_until_next))
}

#[tauri::command]
pub async fn clear_all_version_cache_and_refetch(
  app_handle: tauri::AppHandle,
) -> Result<(), String> {
  let api_client = crate::api_client::ApiClient::instance();
  let version_updater = VersionUpdater::new();

  // Clear all cache first
  api_client
    .clear_all_cache()
    .map_err(|e| format!("Failed to clear version cache: {e}"))?;

  // Disable all browsers during the update process
  let supported_browsers = version_updater
    .browser_version_manager
    .get_supported_browsers();

  // Load current state and disable all browsers
  let mut state = version_updater
    .auto_updater
    .load_auto_update_state()
    .map_err(|e| format!("Failed to load auto update state: {e}"))?;
  for browser in &supported_browsers {
    state.disabled_browsers.insert(browser.clone());
  }
  version_updater
    .auto_updater
    .save_auto_update_state(&state)
    .map_err(|e| format!("Failed to save auto update state: {e}"))?;

  let updater = get_version_updater();
  let updater_guard = updater.lock().await;

  let result = updater_guard
    .trigger_manual_update(&app_handle)
    .await
    .map_err(|e| format!("Failed to trigger version update: {e}"));

  // Re-enable all browsers after the update completes (regardless of success/failure)
  let mut final_state = version_updater
    .auto_updater
    .load_auto_update_state()
    .unwrap_or_default();
  for browser in &supported_browsers {
    final_state.disabled_browsers.remove(browser);
  }
  if let Err(e) = version_updater
    .auto_updater
    .save_auto_update_state(&final_state)
  {
    log::warn!("Failed to re-enable browsers after cache clear: {e}");
  }

  result?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serial_test::serial;
  use std::env;
  use tempfile::TempDir;

  fn setup_test_env() -> TempDir {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    env::set_var("HOME", temp_dir.path());
    temp_dir
  }

  fn cleanup_state_file() {
    if let Ok(state_file) = VersionUpdater::get_background_update_state_file() {
      let _ = fs::remove_file(&state_file);
    }
  }

  #[test]
  #[serial]
  fn test_background_update_state_persistence() {
    let _temp_dir = setup_test_env();

    // Clean up any existing state file first
    if let Ok(state_file) = VersionUpdater::get_background_update_state_file() {
      let _ = fs::remove_file(&state_file);
    }

    // Create a test state
    let test_state = BackgroundUpdateState {
      last_update_time: 1609459200, // 2021-01-01 00:00:00 UTC
      update_interval_hours: 3,
    };

    // Save the state
    let save_result = VersionUpdater::save_background_update_state(&test_state);
    assert!(save_result.is_ok(), "Should save state successfully");

    // Verify file was created
    let state_file = VersionUpdater::get_background_update_state_file().unwrap();
    assert!(state_file.exists(), "State file should exist after saving");

    // Read the file directly to verify contents
    let file_content = fs::read_to_string(&state_file).expect("Should read state file");
    let file_state: BackgroundUpdateState =
      serde_json::from_str(&file_content).expect("Should parse state file");

    // Verify the file contents match what we saved
    assert_eq!(
      file_state.last_update_time, test_state.last_update_time,
      "File last_update_time should match. Expected: {}, Got: {}",
      test_state.last_update_time, file_state.last_update_time
    );
    assert_eq!(
      file_state.update_interval_hours, test_state.update_interval_hours,
      "File update_interval_hours should match"
    );

    // Load the state back using the method
    let loaded_state = VersionUpdater::load_background_update_state();

    // Verify the values match
    assert_eq!(
      loaded_state.last_update_time, test_state.last_update_time,
      "Loaded last_update_time should match. Expected: {}, Got: {}",
      test_state.last_update_time, loaded_state.last_update_time
    );
    assert_eq!(
      loaded_state.update_interval_hours, test_state.update_interval_hours,
      "Loaded update_interval_hours should match"
    );

    // Clean up
    cleanup_state_file();
  }

  #[test]
  #[serial]
  fn test_should_run_background_update_logic() {
    let _temp_dir = setup_test_env();

    // Clean up any existing state file first
    cleanup_state_file();

    let current_time = VersionUpdater::get_current_timestamp();

    // Test with recent update (should not update)
    let recent_state = BackgroundUpdateState {
      last_update_time: current_time - 60, // 1 minute ago
      update_interval_hours: 3,
    };

    // Save and test recent state
    let save_result = VersionUpdater::save_background_update_state(&recent_state);
    assert!(save_result.is_ok(), "Should save recent state successfully");

    let should_update_recent = VersionUpdater::should_run_background_update();
    assert!(
      !should_update_recent,
      "Should not update when last update was recent"
    );

    // Test with old update (should update)
    let old_state = BackgroundUpdateState {
      last_update_time: current_time - (4 * 60 * 60), // 4 hours ago
      update_interval_hours: 3,
    };

    // Save and test old state
    let save_result = VersionUpdater::save_background_update_state(&old_state);
    assert!(save_result.is_ok(), "Should save old state successfully");

    let should_update_old = VersionUpdater::should_run_background_update();
    assert!(should_update_old, "Should update when last update was old");

    // Test with never updated (should update)
    let never_updated_state = BackgroundUpdateState {
      last_update_time: 0,
      update_interval_hours: 3,
    };

    let save_result = VersionUpdater::save_background_update_state(&never_updated_state);
    assert!(
      save_result.is_ok(),
      "Should save never updated state successfully"
    );

    let should_update_never = VersionUpdater::should_run_background_update();
    assert!(
      should_update_never,
      "Should update when never updated before"
    );

    // Clean up
    cleanup_state_file();
  }

  #[test]
  fn test_version_updater_creation() {
    let updater = VersionUpdater::new();

    // Should have valid references to services
    assert!(
      !std::ptr::eq(
        updater.browser_version_manager as *const _,
        std::ptr::null()
      ),
      "Version service should not be null"
    );
    assert!(
      !std::ptr::eq(updater.auto_updater as *const _, std::ptr::null()),
      "Auto updater should not be null"
    );
    assert!(
      updater.app_handle.is_none(),
      "App handle should initially be None"
    );
  }

  #[test]
  fn test_get_current_timestamp() {
    let timestamp1 = VersionUpdater::get_current_timestamp();

    // Should be a reasonable timestamp (after year 2020)
    assert!(
      timestamp1 > 1577836800,
      "Timestamp should be after 2020-01-01"
    ); // 2020-01-01 00:00:00 UTC

    // Should be before year 2100
    assert!(
      timestamp1 < 4102444800,
      "Timestamp should be before 2100-01-01"
    ); // 2100-01-01 00:00:00 UTC

    // Wait a tiny bit and check it increases
    std::thread::sleep(std::time::Duration::from_millis(1));
    let timestamp2 = VersionUpdater::get_current_timestamp();
    assert!(timestamp2 >= timestamp1, "Timestamp should not decrease");
  }

  #[test]
  fn test_get_version_updater_singleton() {
    let updater1 = get_version_updater();
    let updater2 = get_version_updater();

    // Should return the same Arc instance
    assert!(
      Arc::ptr_eq(&updater1, &updater2),
      "Should return same singleton instance"
    );
  }
}

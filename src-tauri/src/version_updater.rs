use crate::browser_version_service::BrowserVersionService;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tokio::sync::Mutex;
use tokio::time::{interval, Interval};

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
struct BackgroundUpdateState {
  last_update_time: u64,
  update_interval_hours: u64,
}

impl Default for BackgroundUpdateState {
  fn default() -> Self {
    Self {
      last_update_time: 0,
      update_interval_hours: 3,
    }
  }
}

pub struct VersionUpdater {
  version_service: BrowserVersionService,
  app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
  update_interval: Interval,
}

impl VersionUpdater {
  pub fn new() -> Self {
    let mut update_interval = interval(Duration::from_secs(5 * 60)); // Check every 5 minutes
    update_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    Self {
      version_service: BrowserVersionService::new(),
      app_handle: Arc::new(Mutex::new(None)),
      update_interval,
    }
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

  fn get_background_update_state_file() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cache_dir = Self::get_cache_dir()?;
    Ok(cache_dir.join("background_update_state.json"))
  }

  fn load_background_update_state() -> BackgroundUpdateState {
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

  fn save_background_update_state(
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
      println!(
        "Background update needed: last_update={}, elapsed={}h, required={}h",
        state.last_update_time,
        elapsed_secs / 3600,
        state.update_interval_hours
      );
    } else {
      println!(
        "Background update not needed: last_update={}, elapsed={}h, required={}h",
        state.last_update_time,
        elapsed_secs / 3600,
        state.update_interval_hours
      );
    }

    should_update
  }

  pub async fn set_app_handle(&self, app_handle: tauri::AppHandle) {
    let mut handle = self.app_handle.lock().await;
    *handle = Some(app_handle);
  }

  pub async fn check_and_run_startup_update(
    &self,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Only run if an update is actually needed
    if !Self::should_run_background_update() {
      println!("No startup version update needed");
      return Ok(());
    }

    let app_handle = {
      let handle_guard = self.app_handle.lock().await;
      handle_guard.clone()
    };

    if let Some(handle) = app_handle {
      println!("Running startup version update...");

      match self.update_all_browser_versions(&handle).await {
        Ok(_) => {
          // Update the persistent state after successful update
          let state = BackgroundUpdateState {
            last_update_time: Self::get_current_timestamp(),
            update_interval_hours: 3,
          };

          if let Err(e) = Self::save_background_update_state(&state) {
            eprintln!("Failed to save background update state: {e}");
          } else {
            println!("Startup version update completed successfully");
          }
        }
        Err(e) => {
          eprintln!("Startup version update failed: {e}");
          return Err(e);
        }
      }
    } else {
      return Err("App handle not available for startup update".into());
    }

    Ok(())
  }

  pub async fn start_background_updates(&mut self) {
    println!(
      "Starting background version update service (checking every 5 minutes for 3-hour intervals)"
    );

    // Run initial startup check
    if let Err(e) = self.check_and_run_startup_update().await {
      eprintln!("Startup version update failed: {e}");
    }

    loop {
      self.update_interval.tick().await;

      // Check if we should run an update based on persistent state
      if !Self::should_run_background_update() {
        continue;
      }

      // Check if we have an app handle
      let app_handle = {
        let handle_guard = self.app_handle.lock().await;
        handle_guard.clone()
      };

      if let Some(handle) = app_handle {
        println!("Starting background version update...");

        match self.update_all_browser_versions(&handle).await {
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

            // Emit error event
            let progress = VersionUpdateProgress {
              current_browser: "".to_string(),
              total_browsers: 0,
              completed_browsers: 0,
              new_versions_found: 0,
              browser_new_versions: 0,
              status: "error".to_string(),
            };
            let _ = handle.emit("version-update-progress", &progress);
          }
        }
      } else {
        println!("App handle not available, skipping background update");
      }
    }
  }

  async fn update_all_browser_versions(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<Vec<BackgroundUpdateResult>, Box<dyn std::error::Error + Send + Sync>> {
    println!("Starting background version update for all browsers");

    let browsers = [
      "firefox",
      "firefox-developer",
      "mullvad-browser",
      "zen",
      "brave",
      "chromium",
      "tor-browser",
    ];

    let total_browsers = browsers.len();
    let mut results = Vec::new();
    let mut total_new_versions = 0;

    // Emit start event
    let progress = VersionUpdateProgress {
      current_browser: "".to_string(),
      total_browsers,
      completed_browsers: 0,
      new_versions_found: 0,
      browser_new_versions: 0,
      status: "updating".to_string(),
    };
    let _ = app_handle.emit("version-update-progress", &progress);

    for (index, browser) in browsers.iter().enumerate() {
      // Check if individual browser cache is expired before updating
      if !self.version_service.should_update_cache(browser) {
        println!("Skipping {browser} - cache is still fresh");

        let browser_result = BackgroundUpdateResult {
          browser: browser.to_string(),
          new_versions_count: 0,
          total_versions_count: 0,
          updated_successfully: true,
          error: None,
        };
        results.push(browser_result);
        continue;
      }

      println!("Updating versions for browser: {browser}");

      // Emit progress for current browser
      let progress = VersionUpdateProgress {
        current_browser: browser.to_string(),
        total_browsers,
        completed_browsers: index,
        new_versions_found: total_new_versions,
        browser_new_versions: 0,
        status: "updating".to_string(),
      };
      let _ = app_handle.emit("version-update-progress", &progress);

      let result = self.update_browser_versions(browser).await;

      match result {
        Ok(new_count) => {
          total_new_versions += new_count;
          let browser_result = BackgroundUpdateResult {
            browser: browser.to_string(),
            new_versions_count: new_count,
            total_versions_count: 0, // We'll update this if needed
            updated_successfully: true,
            error: None,
          };
          results.push(browser_result);

          println!("Found {new_count} new versions for {browser}");
        }
        Err(e) => {
          eprintln!("Failed to update versions for {browser}: {e}");
          let browser_result = BackgroundUpdateResult {
            browser: browser.to_string(),
            new_versions_count: 0,
            total_versions_count: 0,
            updated_successfully: false,
            error: Some(e.to_string()),
          };
          results.push(browser_result);
        }
      }

      // Small delay between browsers to avoid overwhelming APIs
      tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // Emit completion event
    let progress = VersionUpdateProgress {
      current_browser: "".to_string(),
      total_browsers,
      completed_browsers: total_browsers,
      new_versions_found: total_new_versions,
      browser_new_versions: 0,
      status: "completed".to_string(),
    };
    let _ = app_handle.emit("version-update-progress", &progress);

    println!("Background version update completed. Found {total_new_versions} new versions total");

    Ok(results)
  }

  async fn update_browser_versions(
    &self,
    browser: &str,
  ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    self
      .version_service
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
      eprintln!("Failed to save background update state after manual update: {e}");
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

#[cfg(test)]
mod tests {
  use super::*;

  // Helper function to create a unique test state file
  fn get_test_state_file(test_name: &str) -> PathBuf {
    let cache_dir = VersionUpdater::get_cache_dir().unwrap();
    cache_dir.join(format!("test_{test_name}_state.json"))
  }

  fn save_test_state(
    test_name: &str,
    state: &BackgroundUpdateState,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let state_file = get_test_state_file(test_name);
    let content = serde_json::to_string_pretty(state)?;
    fs::write(&state_file, content)?;
    Ok(())
  }

  fn load_test_state(test_name: &str) -> BackgroundUpdateState {
    let state_file = get_test_state_file(test_name);

    if !state_file.exists() {
      return BackgroundUpdateState::default();
    }

    let content = match fs::read_to_string(&state_file) {
      Ok(content) => content,
      Err(_) => return BackgroundUpdateState::default(),
    };

    serde_json::from_str(&content).unwrap_or_default()
  }

  #[test]
  fn test_background_update_state_persistence() {
    let test_name = "persistence";

    // Create a test state
    let test_state = BackgroundUpdateState {
      last_update_time: 1609459200, // 2021-01-01 00:00:00 UTC
      update_interval_hours: 3,
    };

    // Save the state
    save_test_state(test_name, &test_state).unwrap();

    // Load the state back
    let loaded_state = load_test_state(test_name);

    // Verify the values match
    assert_eq!(loaded_state.last_update_time, test_state.last_update_time);
    assert_eq!(
      loaded_state.update_interval_hours,
      test_state.update_interval_hours
    );

    // Clean up
    let _ = fs::remove_file(get_test_state_file(test_name));
  }

  #[test]
  fn test_should_run_background_update_logic() {
    // Note: This test uses the shared state file, so results may vary
    // depending on previous test runs. This is expected behavior.

    // Test with recent update (should not update)
    let recent_state = BackgroundUpdateState {
      last_update_time: VersionUpdater::get_current_timestamp() - 60, // 1 minute ago
      update_interval_hours: 3,
    };
    VersionUpdater::save_background_update_state(&recent_state).unwrap();
    assert!(!VersionUpdater::should_run_background_update());

    // Test with old update (should update)
    let old_state = BackgroundUpdateState {
      last_update_time: VersionUpdater::get_current_timestamp() - (4 * 60 * 60), // 4 hours ago
      update_interval_hours: 3,
    };
    VersionUpdater::save_background_update_state(&old_state).unwrap();
    assert!(VersionUpdater::should_run_background_update());
  }

  #[test]
  fn test_cache_dir_creation() {
    // This should not panic and should create the directory if it doesn't exist
    let cache_dir = VersionUpdater::get_cache_dir().unwrap();
    assert!(cache_dir.exists());
    assert!(cache_dir.is_dir());
  }
}

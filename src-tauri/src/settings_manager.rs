use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::fs::{self, create_dir_all};
use std::path::PathBuf;

use crate::api_client::ApiClient;
use crate::version_updater;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TableSortingSettings {
  pub column: String,    // Column to sort by: "name", "browser", "status"
  pub direction: String, // "asc" or "desc"
}

impl Default for TableSortingSettings {
  fn default() -> Self {
    Self {
      column: "name".to_string(),
      direction: "asc".to_string(),
    }
  }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
  #[serde(default)]
  pub set_as_default_browser: bool,
  #[serde(default)]
  pub show_settings_on_startup: bool,
  #[serde(default = "default_theme")]
  pub theme: String, // "light", "dark", or "system"
}

fn default_theme() -> String {
  "system".to_string()
}

impl Default for AppSettings {
  fn default() -> Self {
    Self {
      set_as_default_browser: false,
      show_settings_on_startup: true,
      theme: "system".to_string(),
    }
  }
}

pub struct SettingsManager {
  base_dirs: BaseDirs,
}

impl SettingsManager {
  pub fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
    }
  }

  pub fn get_settings_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("settings");
    path
  }

  pub fn get_settings_file(&self) -> PathBuf {
    self.get_settings_dir().join("app_settings.json")
  }

  pub fn get_table_sorting_file(&self) -> PathBuf {
    self.get_settings_dir().join("table_sorting.json")
  }

  pub fn load_settings(&self) -> Result<AppSettings, Box<dyn std::error::Error>> {
    let settings_file = self.get_settings_file();

    if !settings_file.exists() {
      // Return default settings if file doesn't exist
      return Ok(AppSettings::default());
    }

    let content = fs::read_to_string(&settings_file)?;

    // Parse the settings file - serde will use default values for missing fields
    match serde_json::from_str::<AppSettings>(&content) {
      Ok(settings) => {
        // Save the settings back to ensure any missing fields are written with defaults
        if let Err(e) = self.save_settings(&settings) {
          eprintln!("Warning: Failed to update settings file with defaults: {e}");
        }
        Ok(settings)
      }
      Err(e) => {
        eprintln!("Warning: Failed to parse settings file, using defaults: {e}");
        let default_settings = AppSettings::default();

        // Try to save default settings to fix the corrupted file
        if let Err(save_error) = self.save_settings(&default_settings) {
          eprintln!("Warning: Failed to save default settings: {save_error}");
        }

        Ok(default_settings)
      }
    }
  }

  pub fn save_settings(&self, settings: &AppSettings) -> Result<(), Box<dyn std::error::Error>> {
    let settings_dir = self.get_settings_dir();
    create_dir_all(&settings_dir)?;

    let settings_file = self.get_settings_file();
    let json = serde_json::to_string_pretty(settings)?;
    fs::write(settings_file, json)?;

    Ok(())
  }

  pub fn load_table_sorting(&self) -> Result<TableSortingSettings, Box<dyn std::error::Error>> {
    let sorting_file = self.get_table_sorting_file();

    if !sorting_file.exists() {
      // Return default sorting if file doesn't exist
      return Ok(TableSortingSettings::default());
    }

    let content = fs::read_to_string(sorting_file)?;
    let sorting: TableSortingSettings = serde_json::from_str(&content)?;
    Ok(sorting)
  }

  pub fn save_table_sorting(
    &self,
    sorting: &TableSortingSettings,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let settings_dir = self.get_settings_dir();
    create_dir_all(&settings_dir)?;

    let sorting_file = self.get_table_sorting_file();
    let json = serde_json::to_string_pretty(sorting)?;
    fs::write(sorting_file, json)?;

    Ok(())
  }

  pub fn should_show_settings_on_startup(&self) -> Result<bool, Box<dyn std::error::Error>> {
    let settings = self.load_settings()?;

    // Show prompt if:
    // 1. User wants to see the prompt
    // 2. Donut Browser is not set as default
    // 3. User hasn't explicitly disabled the default browser setting
    Ok(settings.show_settings_on_startup && !settings.set_as_default_browser)
  }
}

#[tauri::command]
pub async fn get_app_settings() -> Result<AppSettings, String> {
  let manager = SettingsManager::new();
  manager
    .load_settings()
    .map_err(|e| format!("Failed to load settings: {e}"))
}

#[tauri::command]
pub async fn save_app_settings(settings: AppSettings) -> Result<(), String> {
  let manager = SettingsManager::new();
  manager
    .save_settings(&settings)
    .map_err(|e| format!("Failed to save settings: {e}"))
}

#[tauri::command]
pub async fn should_show_settings_on_startup() -> Result<bool, String> {
  let manager = SettingsManager::new();
  manager
    .should_show_settings_on_startup()
    .map_err(|e| format!("Failed to check prompt setting: {e}"))
}

#[tauri::command]
pub async fn get_table_sorting_settings() -> Result<TableSortingSettings, String> {
  let manager = SettingsManager::new();
  manager
    .load_table_sorting()
    .map_err(|e| format!("Failed to load table sorting settings: {e}"))
}

#[tauri::command]
pub async fn save_table_sorting_settings(sorting: TableSortingSettings) -> Result<(), String> {
  let manager = SettingsManager::new();
  manager
    .save_table_sorting(&sorting)
    .map_err(|e| format!("Failed to save table sorting settings: {e}"))
}

#[tauri::command]
pub async fn clear_all_version_cache_and_refetch(
  app_handle: tauri::AppHandle,
) -> Result<(), String> {
  let api_client = ApiClient::new();

  // Clear all cache first
  api_client
    .clear_all_cache()
    .map_err(|e| format!("Failed to clear version cache: {e}"))?;

  let updater = version_updater::get_version_updater();
  let updater_guard = updater.lock().await;

  updater_guard
    .trigger_manual_update(&app_handle)
    .await
    .map_err(|e| format!("Failed to trigger version update: {e}"))?;

  Ok(())
}

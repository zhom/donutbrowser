// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

// Store pending URLs that need to be handled when the window is ready
static PENDING_URLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

mod api_client;
mod app_auto_updater;
mod auto_updater;
mod browser;
mod browser_runner;
mod browser_version_service;
mod default_browser;
mod download;
mod downloaded_browsers;
mod extraction;
mod proxy_manager;
mod settings_manager;
mod version_updater;

extern crate lazy_static;

use browser_runner::{
  check_browser_exists, check_browser_status, create_browser_profile, create_browser_profile_new,
  delete_profile, download_browser, fetch_browser_versions, fetch_browser_versions_cached_first,
  fetch_browser_versions_detailed, fetch_browser_versions_with_count,
  fetch_browser_versions_with_count_cached_first, get_cached_browser_versions_detailed,
  get_downloaded_browser_versions, get_saved_mullvad_releases, get_supported_browsers,
  is_browser_downloaded, kill_browser_profile, launch_browser_profile, list_browser_profiles,
  rename_profile, should_update_browser_cache, update_profile_proxy, update_profile_version,
};

use settings_manager::{
  disable_default_browser_prompt, get_app_settings, get_table_sorting_settings, save_app_settings,
  save_table_sorting_settings, should_show_settings_on_startup,
};

use default_browser::{
  is_default_browser, open_url_with_profile, set_as_default_browser, smart_open_url,
};

use version_updater::{
  check_version_update_needed, force_version_update_check, get_version_update_status,
  get_version_updater, trigger_manual_version_update,
};

use auto_updater::{
  check_for_browser_updates, complete_browser_update, complete_browser_update_with_auto_update,
  dismiss_update_notification, is_auto_update_download, is_browser_disabled_for_update,
  mark_auto_update_download, remove_auto_update_download, start_browser_update,
};

use app_auto_updater::{
  check_for_app_updates, check_for_app_updates_manual, download_and_install_app_update,
  get_app_version_info,
};

#[tauri::command]
fn greet() -> String {
  let now = SystemTime::now();
  let epoch_ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis();
  format!("Hello world from Rust! Current epoch: {epoch_ms}")
}

#[tauri::command]
async fn handle_url_open(app: tauri::AppHandle, url: String) -> Result<(), String> {
  println!("handle_url_open called with URL: {url}");

  // Check if the main window exists and is ready
  if let Some(window) = app.get_webview_window("main") {
    if window.is_visible().unwrap_or(false) {
      // Window is visible, emit event directly
      println!("Main window is visible, emitting show-profile-selector event");
      app
        .emit("show-profile-selector", url.clone())
        .map_err(|e| format!("Failed to emit URL open event: {e}"))?;
      let _ = window.show();
      let _ = window.set_focus();
    } else {
      // Window not visible yet - add to pending URLs
      println!("Main window not visible, adding URL to pending list");
      let mut pending = PENDING_URLS.lock().unwrap();
      pending.push(url);
    }
  } else {
    // Window doesn't exist yet - add to pending URLs
    println!("Main window doesn't exist, adding URL to pending list");
    let mut pending = PENDING_URLS.lock().unwrap();
    pending.push(url);
  }

  Ok(())
}

#[tauri::command]
async fn check_and_handle_startup_url(app_handle: tauri::AppHandle) -> Result<bool, String> {
  let pending_urls = {
    let mut pending = PENDING_URLS.lock().unwrap();
    let urls = pending.clone();
    pending.clear(); // Clear after getting them
    urls
  };

  if !pending_urls.is_empty() {
    println!(
      "Handling {} pending URLs from frontend request",
      pending_urls.len()
    );

    for url in pending_urls {
      println!("Emitting show-profile-selector event for URL: {url}");
      if let Err(e) = app_handle.emit("show-profile-selector", url.clone()) {
        eprintln!("Failed to emit URL event: {e}");
        return Err(format!("Failed to emit URL event: {e}"));
      }
    }

    return Ok(true);
  }

  Ok(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_opener::init())
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_deep_link::init())
    .setup(|app| {
      // Set up deep link handler
      let handle = app.handle().clone();

      #[cfg(any(windows, target_os = "linux"))]
      {
        // For Windows and Linux, register all deep links at runtime for development
        app.deep_link().register_all()?;
      }

      // Handle deep links - this works for both scenarios:
      // 1. App is running and URL is opened
      // 2. App is not running and URL causes app to launch
      app.deep_link().on_open_url({
        let handle = handle.clone();
        move |event| {
          let urls = event.urls();
          for url in urls {
            let url_string = url.to_string();
            println!("Deep link received: {url_string}");

            // Clone the handle for each async task
            let handle_clone = handle.clone();

            // Handle the URL asynchronously
            tauri::async_runtime::spawn(async move {
              if let Err(e) = handle_url_open(handle_clone, url_string.clone()).await {
                eprintln!("Failed to handle deep link URL: {e}");
              }
            });
          }
        }
      });

      // Initialize and start background version updater
      let app_handle = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        let version_updater = get_version_updater();
        let mut updater_guard = version_updater.lock().await;

        // Set the app handle
        updater_guard.set_app_handle(app_handle).await;

        // Start the background updates
        updater_guard.start_background_updates().await;
      });

      // Check for app updates at startup
      let app_handle_update = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        // Add a small delay to ensure the app is fully loaded
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        println!("Starting app update check at startup...");
        let updater = app_auto_updater::AppAutoUpdater::new();
        match updater.check_for_updates().await {
          Ok(Some(update_info)) => {
            println!(
              "App update available: {} -> {}",
              update_info.current_version, update_info.new_version
            );
            // Emit update available event to the frontend
            if let Err(e) = app_handle_update.emit("app-update-available", &update_info) {
              eprintln!("Failed to emit app update event: {e}");
            } else {
              println!("App update event emitted successfully");
            }
          }
          Ok(None) => {
            println!("No app updates available");
          }
          Err(e) => {
            eprintln!("Failed to check for app updates: {e}");
          }
        }
      });

      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      greet,
      get_supported_browsers,
      download_browser,
      delete_profile,
      is_browser_downloaded,
      check_browser_exists,
      create_browser_profile_new,
      create_browser_profile,
      list_browser_profiles,
      launch_browser_profile,
      fetch_browser_versions,
      fetch_browser_versions_detailed,
      fetch_browser_versions_with_count,
      fetch_browser_versions_cached_first,
      fetch_browser_versions_with_count_cached_first,
      get_cached_browser_versions_detailed,
      should_update_browser_cache,
      get_downloaded_browser_versions,
      get_saved_mullvad_releases,
      update_profile_proxy,
      update_profile_version,
      check_browser_status,
      kill_browser_profile,
      rename_profile,
      get_app_settings,
      save_app_settings,
      should_show_settings_on_startup,
      disable_default_browser_prompt,
      get_table_sorting_settings,
      save_table_sorting_settings,
      is_default_browser,
      open_url_with_profile,
      set_as_default_browser,
      smart_open_url,
      handle_url_open,
      check_and_handle_startup_url,
      trigger_manual_version_update,
      get_version_update_status,
      check_version_update_needed,
      force_version_update_check,
      check_for_browser_updates,
      start_browser_update,
      complete_browser_update,
      is_browser_disabled_for_update,
      dismiss_update_notification,
      complete_browser_update_with_auto_update,
      mark_auto_update_download,
      remove_auto_update_download,
      is_auto_update_download,
      check_for_app_updates,
      check_for_app_updates_manual,
      download_and_install_app_update,
      get_app_version_info,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

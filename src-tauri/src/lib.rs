// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::env;
use std::sync::Mutex;
use tauri::{Emitter, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
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
mod profile_importer;
mod proxy_manager;
mod settings_manager;
mod theme_detector;
mod version_updater;

extern crate lazy_static;

use browser_runner::{
  check_browser_exists, check_browser_status, check_missing_binaries, create_browser_profile_new,
  delete_profile, download_browser, ensure_all_binaries_exist, fetch_browser_versions_cached_first,
  fetch_browser_versions_with_count, fetch_browser_versions_with_count_cached_first,
  get_browser_release_types, get_downloaded_browser_versions, get_supported_browsers,
  is_browser_supported_on_platform, kill_browser_profile, launch_browser_profile,
  list_browser_profiles, rename_profile, update_profile_proxy, update_profile_version,
};

use settings_manager::{
  clear_all_version_cache_and_refetch, get_app_settings, get_table_sorting_settings,
  save_app_settings, save_table_sorting_settings, should_show_settings_on_startup,
};

use default_browser::{
  is_default_browser, open_url_with_profile, set_as_default_browser, smart_open_url,
};

use version_updater::{
  get_version_update_status, get_version_updater, trigger_manual_version_update,
};

use auto_updater::{
  check_for_browser_updates, complete_browser_update_with_auto_update, dismiss_update_notification,
  is_browser_disabled_for_update,
};

use app_auto_updater::{
  check_for_app_updates, check_for_app_updates_manual, download_and_install_app_update,
};

use profile_importer::{detect_existing_profiles, import_browser_profile};

use theme_detector::get_system_theme;

// Trait to extend WebviewWindow with transparent titlebar functionality
pub trait WindowExt {
  #[cfg(target_os = "macos")]
  fn set_transparent_titlebar(&self, transparent: bool) -> Result<(), String>;
}

impl<R: Runtime> WindowExt for WebviewWindow<R> {
  #[cfg(target_os = "macos")]
  fn set_transparent_titlebar(&self, transparent: bool) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSWindow, NSWindowStyleMask, NSWindowTitleVisibility};

    unsafe {
      let ns_window: Retained<NSWindow> =
        Retained::retain(self.ns_window().unwrap().cast()).unwrap();

      if transparent {
        // Hide the title text
        ns_window.setTitleVisibility(NSWindowTitleVisibility(2)); // NSWindowTitleHidden

        // Make titlebar transparent
        ns_window.setTitlebarAppearsTransparent(true);

        // Set full size content view
        let current_mask = ns_window.styleMask();
        let new_mask = NSWindowStyleMask(current_mask.0 | (1 << 15)); // NSFullSizeContentViewWindowMask
        ns_window.setStyleMask(new_mask);
      } else {
        // Show the title text
        ns_window.setTitleVisibility(NSWindowTitleVisibility(0)); // NSWindowTitleVisible

        // Make titlebar opaque
        ns_window.setTitlebarAppearsTransparent(false);

        // Remove full size content view
        let current_mask = ns_window.styleMask();
        let new_mask = NSWindowStyleMask(current_mask.0 & !(1 << 15));
        ns_window.setStyleMask(new_mask);
      }
    }

    Ok(())
  }
}

#[tauri::command]
async fn handle_url_open(app: tauri::AppHandle, url: String) -> Result<(), String> {
  println!("handle_url_open called with URL: {url}");

  // Check if the main window exists and is ready
  if let Some(window) = app.get_webview_window("main") {
    println!("Main window exists");

    // Try to show and focus the window first
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.unminimize();

    app
      .emit("show-profile-selector", url.clone())
      .map_err(|e| format!("Failed to emit URL open event: {e}"))?;
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
  println!("check_and_handle_startup_url called");

  let pending_urls = {
    let mut pending = PENDING_URLS.lock().unwrap();
    let urls = pending.clone();
    pending.clear(); // Clear after getting them
    urls
  };

  println!("Found {} pending URLs", pending_urls.len());

  if !pending_urls.is_empty() {
    println!(
      "Handling {} pending URLs from frontend request",
      pending_urls.len()
    );

    // Ensure the main window is visible and focused
    if let Some(window) = app_handle.get_webview_window("main") {
      let _ = window.show();
      let _ = window.set_focus();
      let _ = window.unminimize();

      // Give the window a moment to become visible
      tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

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
  let args: Vec<String> = env::args().collect();
  let startup_url = args.iter().find(|arg| arg.starts_with("http")).cloned();

  if let Some(url) = startup_url.clone() {
    println!("Found startup URL in command line: {url}");
    let mut pending = PENDING_URLS.lock().unwrap();
    pending.push(url.clone());
  }

  tauri::Builder::default()
    .plugin(tauri_plugin_single_instance::init(|_, args, _cwd| {
      println!("Single instance triggered with args: {args:?}");
    }))
    .plugin(tauri_plugin_deep_link::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_opener::init())
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_macos_permissions::init())
    .setup(|app| {
      // Create the main window programmatically
      #[allow(unused_variables)]
      let win_builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Donut Browser")
        .inner_size(900.0, 600.0)
        .resizable(false)
        .fullscreen(false)
        .center()
        .focused(true)
        .visible(true);

      #[allow(unused_variables)]
      let window = win_builder.build().unwrap();

      // Set transparent titlebar for macOS
      #[cfg(target_os = "macos")]
      {
        if let Err(e) = window.set_transparent_titlebar(true) {
          eprintln!("Failed to set transparent titlebar: {e}");
        }
      }

      // Set up deep link handler
      let handle = app.handle().clone();

      #[cfg(any(windows, target_os = "linux"))]
      {
        // For Windows and Linux, register all deep links at runtime for development
        if let Err(e) = app.deep_link().register_all() {
          eprintln!("Failed to register deep links: {e}");
        }
      }

      #[cfg(target_os = "macos")]
      {
        // On macOS, try to register deep links for development builds
        if let Err(e) = app.deep_link().register_all() {
          eprintln!(
            "Note: Deep link registration failed on macOS (this is normal for production): {e}"
          );
        }
      }

      app.deep_link().on_open_url({
        let handle = handle.clone();
        move |event| {
          let urls = event.urls();
          println!("Deep link event received with {} URLs", urls.len());

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

      if let Some(startup_url) = startup_url {
        let handle_clone = handle.clone();
        tauri::async_runtime::spawn(async move {
          println!("Processing startup URL from command line: {startup_url}");
          if let Err(e) = handle_url_open(handle_clone, startup_url.clone()).await {
            eprintln!("Failed to handle startup URL: {e}");
          }
        });
      }

      // Initialize and start background version updater
      let app_handle = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        let version_updater = get_version_updater();

        // Set the app handle
        {
          let mut updater_guard = version_updater.lock().await;
          updater_guard.set_app_handle(app_handle);
        }

        // Run startup check without holding the lock
        {
          let updater_guard = version_updater.lock().await;
          if let Err(e) = updater_guard.start_background_updates().await {
            eprintln!("Failed to start background updates: {e}");
          }
        }
      });

      // Start the background update task separately
      tauri::async_runtime::spawn(async move {
        version_updater::VersionUpdater::run_background_task().await;
      });

      let app_handle_auto_updater = app.handle().clone();

      // Start the auto-update check task separately
      tauri::async_runtime::spawn(async move {
        auto_updater::check_for_updates_with_progress(app_handle_auto_updater).await;
      });

      let app_handle_update = app.handle().clone();
      tauri::async_runtime::spawn(async move {
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
      get_supported_browsers,
      is_browser_supported_on_platform,
      download_browser,
      delete_profile,
      check_browser_exists,
      create_browser_profile_new,
      list_browser_profiles,
      launch_browser_profile,
      fetch_browser_versions_with_count,
      fetch_browser_versions_cached_first,
      fetch_browser_versions_with_count_cached_first,
      get_downloaded_browser_versions,
      get_browser_release_types,
      update_profile_proxy,
      update_profile_version,
      check_browser_status,
      kill_browser_profile,
      rename_profile,
      get_app_settings,
      save_app_settings,
      should_show_settings_on_startup,
      get_table_sorting_settings,
      save_table_sorting_settings,
      clear_all_version_cache_and_refetch,
      is_default_browser,
      open_url_with_profile,
      set_as_default_browser,
      smart_open_url,
      check_and_handle_startup_url,
      trigger_manual_version_update,
      get_version_update_status,
      check_for_browser_updates,
      is_browser_disabled_for_update,
      dismiss_update_notification,
      complete_browser_update_with_auto_update,
      check_for_app_updates,
      check_for_app_updates_manual,
      download_and_install_app_update,
      get_system_theme,
      detect_existing_profiles,
      import_browser_profile,
      check_missing_binaries,
      ensure_all_binaries_exist,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
  use std::fs;

  #[test]
  fn test_no_unused_tauri_commands() {
    check_unused_commands(false); // Run in strict mode for CI
  }

  #[test]
  fn test_unused_tauri_commands_detailed() {
    check_unused_commands(true); // Run in verbose mode for development
  }

  fn check_unused_commands(verbose: bool) {
    // Extract command names from the generate_handler! macro in this file
    let lib_rs_content = fs::read_to_string("src/lib.rs").expect("Failed to read lib.rs");
    let commands = extract_tauri_commands(&lib_rs_content);

    // Get all frontend files
    let frontend_files = get_frontend_files("../src");

    // Check which commands are actually used
    let mut unused_commands = Vec::new();
    let mut used_commands = Vec::new();

    for command in &commands {
      let mut is_used = false;

      for file_content in &frontend_files {
        // More comprehensive search for command usage
        if is_command_used(file_content, command) {
          is_used = true;
          break;
        }
      }

      if is_used {
        used_commands.push(command.clone());
        if verbose {
          println!("✅ {command}");
        }
      } else {
        unused_commands.push(command.clone());
        if verbose {
          println!("❌ {command} (UNUSED)");
        }
      }
    }

    if verbose {
      println!("\n📊 Summary:");
      println!("  ✅ Used commands: {}", used_commands.len());
      println!("  ❌ Unused commands: {}", unused_commands.len());
    }

    if !unused_commands.is_empty() {
      let message = format!(
        "Found {} unused Tauri commands: {}\n\nThese commands are exported in generate_handler! but not used in the frontend.\nConsider removing them or add them to the allowlist if they're used elsewhere.\n\nRun `pnpm check-unused-commands` for detailed analysis.",
        unused_commands.len(),
        unused_commands.join(", ")
      );

      if verbose {
        println!("\n🚨 {message}");
      } else {
        panic!("{}", message);
      }
    } else if verbose {
      println!("\n🎉 All exported commands are being used!");
    } else {
      println!(
        "✅ All {} exported Tauri commands are being used in the frontend",
        commands.len()
      );
    }
  }

  fn is_command_used(content: &str, command: &str) -> bool {
    // Check various patterns for invoke usage
    let patterns = vec![
      format!("invoke<{}>(\"{}\"", "", command), // invoke<Type>("command"
      format!("invoke(\"{}\"", command),         // invoke("command"
      format!("invoke<{}>(\"{}\",", "", command), // invoke<Type>("command",
      format!("invoke(\"{}\",", command),        // invoke("command",
      format!("\"{}\"", command),                // Just the command name in quotes
    ];

    for pattern in patterns {
      if content.contains(&pattern) {
        return true;
      }
    }

    // Also check for the command name appearing after "invoke" within a reasonable distance
    if let Some(invoke_pos) = content.find("invoke") {
      let after_invoke = &content[invoke_pos..];
      if let Some(cmd_pos) = after_invoke.find(&format!("\"{command}\"")) {
        // If the command appears within 100 characters of "invoke", consider it used
        if cmd_pos < 100 {
          return true;
        }
      }
    }

    false
  }

  fn extract_tauri_commands(content: &str) -> Vec<String> {
    let mut commands = Vec::new();

    // Find the generate_handler! macro
    if let Some(start) = content.find("tauri::generate_handler![") {
      if let Some(end) = content[start..].find("])") {
        let handler_content = &content[start + 25..start + end]; // Skip "tauri::generate_handler!["

        // Extract command names
        for line in handler_content.lines() {
          let line = line.trim();
          if !line.is_empty() && !line.starts_with("//") {
            // Remove trailing comma and whitespace
            let command = line.trim_end_matches(',').trim();
            if !command.is_empty() {
              commands.push(command.to_string());
            }
          }
        }
      }
    }

    commands
  }

  fn get_frontend_files(src_dir: &str) -> Vec<String> {
    let mut files_content = Vec::new();

    if let Ok(entries) = fs::read_dir(src_dir) {
      for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
          // Recursively read subdirectories
          let subdir_files = get_frontend_files(&path.to_string_lossy());
          files_content.extend(subdir_files);
        } else if let Some(extension) = path.extension() {
          if matches!(
            extension.to_str(),
            Some("ts") | Some("tsx") | Some("js") | Some("jsx")
          ) {
            if let Ok(content) = fs::read_to_string(&path) {
              files_content.push(content);
            }
          }
        }
      }
    }

    files_content
  }
}

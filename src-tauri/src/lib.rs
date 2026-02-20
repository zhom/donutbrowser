// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::env;
use std::sync::Mutex;
use tauri::{Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_log::{Target, TargetKind};

// Store pending URLs that need to be handled when the window is ready
static PENDING_URLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

mod api_client;
mod api_server;
mod app_auto_updater;
mod auto_updater;
mod browser;
mod browser_runner;
mod browser_version_manager;
pub mod camoufox;
mod camoufox_manager;
mod default_browser;
mod downloaded_browsers_registry;
mod downloader;
mod extraction;
mod geoip_downloader;
mod group_manager;
mod ip_utils;
mod platform_browser;
mod profile;
mod profile_importer;
mod proxy_manager;
pub mod proxy_runner;
pub mod proxy_server;
pub mod proxy_storage;
mod settings_manager;
pub mod sync;
pub mod traffic_stats;
mod wayfern_manager;
mod wayfern_terms;
// mod theme_detector; // removed: theme detection handled in webview via CSS prefers-color-scheme
pub mod cloud_auth;
mod commercial_license;
mod cookie_manager;
pub mod daemon;
pub mod daemon_client;
mod daemon_spawn;
pub mod daemon_ws;
pub mod events;
mod mcp_server;
mod tag_manager;
mod version_updater;
pub mod vpn;
pub mod vpn_worker_runner;
pub mod vpn_worker_storage;

use browser_runner::{
  check_browser_exists, kill_browser_profile, launch_browser_profile, open_url_with_profile,
};

use profile::manager::{
  check_browser_status, clone_profile, create_browser_profile_new, delete_profile,
  list_browser_profiles, rename_profile, update_camoufox_config, update_profile_note,
  update_profile_proxy, update_profile_tags, update_profile_vpn, update_wayfern_config,
};

use browser_version_manager::{
  fetch_browser_versions_cached_first, fetch_browser_versions_with_count,
  fetch_browser_versions_with_count_cached_first, get_supported_browsers,
  is_browser_supported_on_platform,
};

use downloaded_browsers_registry::{
  check_missing_binaries, ensure_active_browsers_downloaded, ensure_all_binaries_exist,
  get_downloaded_browser_versions,
};

use downloader::{cancel_download, download_browser};

use settings_manager::{
  decline_launch_on_login, enable_launch_on_login, get_app_settings, get_sync_settings,
  get_system_language, get_table_sorting_settings, save_app_settings, save_sync_settings,
  save_table_sorting_settings, should_show_launch_on_login_prompt,
};

use sync::{
  is_group_in_use_by_synced_profile, is_proxy_in_use_by_synced_profile,
  is_vpn_in_use_by_synced_profile, request_profile_sync, set_group_sync_enabled,
  set_profile_sync_enabled, set_proxy_sync_enabled, set_vpn_sync_enabled,
};

use tag_manager::get_all_tags;

use default_browser::{is_default_browser, set_as_default_browser};

use version_updater::{
  clear_all_version_cache_and_refetch, get_version_update_status, get_version_updater,
  trigger_manual_version_update,
};

use auto_updater::{
  check_for_browser_updates, complete_browser_update_with_auto_update, dismiss_update_notification,
};

use app_auto_updater::{
  check_for_app_updates, check_for_app_updates_manual, download_and_prepare_app_update,
  restart_application,
};

use profile_importer::{detect_existing_profiles, import_browser_profile};

use group_manager::{
  assign_profiles_to_group, create_profile_group, delete_profile_group, delete_selected_profiles,
  get_groups_with_profile_counts, get_profile_groups, update_profile_group,
};

use geoip_downloader::{check_missing_geoip_database, GeoIPDownloader};

use browser_version_manager::get_browser_release_types;

use api_server::{get_api_server_status, start_api_server, stop_api_server};

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
  log::info!("handle_url_open called with URL: {url}");

  // Check if the main window exists and is ready
  if let Some(window) = app.get_webview_window("main") {
    log::debug!("Main window exists");

    // Try to show and focus the window first
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.unminimize();

    events::emit("show-profile-selector", url.clone())
      .map_err(|e| format!("Failed to emit URL open event: {e}"))?;
  } else {
    // Window doesn't exist yet - add to pending URLs
    log::debug!("Main window doesn't exist, adding URL to pending list");
    let mut pending = PENDING_URLS.lock().unwrap();
    pending.push(url);
  }

  Ok(())
}

#[tauri::command]
async fn create_stored_proxy(
  app_handle: tauri::AppHandle,
  name: String,
  proxy_settings: crate::browser::ProxySettings,
) -> Result<crate::proxy_manager::StoredProxy, String> {
  crate::proxy_manager::PROXY_MANAGER
    .create_stored_proxy(&app_handle, name, proxy_settings)
    .map_err(|e| format!("Failed to create stored proxy: {e}"))
}

#[tauri::command]
async fn get_stored_proxies() -> Result<Vec<crate::proxy_manager::StoredProxy>, String> {
  Ok(crate::proxy_manager::PROXY_MANAGER.get_stored_proxies())
}

#[tauri::command]
async fn update_stored_proxy(
  app_handle: tauri::AppHandle,
  proxy_id: String,
  name: Option<String>,
  proxy_settings: Option<crate::browser::ProxySettings>,
) -> Result<crate::proxy_manager::StoredProxy, String> {
  crate::proxy_manager::PROXY_MANAGER
    .update_stored_proxy(&app_handle, &proxy_id, name, proxy_settings)
    .map_err(|e| format!("Failed to update stored proxy: {e}"))
}

#[tauri::command]
async fn delete_stored_proxy(app_handle: tauri::AppHandle, proxy_id: String) -> Result<(), String> {
  crate::proxy_manager::PROXY_MANAGER
    .delete_stored_proxy(&app_handle, &proxy_id)
    .map_err(|e| format!("Failed to delete stored proxy: {e}"))
}

#[tauri::command]
async fn check_proxy_validity(
  proxy_id: String,
  proxy_settings: crate::browser::ProxySettings,
) -> Result<crate::proxy_manager::ProxyCheckResult, String> {
  crate::proxy_manager::PROXY_MANAGER
    .check_proxy_validity(&proxy_id, &proxy_settings)
    .await
}

#[tauri::command]
fn get_cached_proxy_check(proxy_id: String) -> Option<crate::proxy_manager::ProxyCheckResult> {
  crate::proxy_manager::PROXY_MANAGER.get_cached_proxy_check(&proxy_id)
}

#[tauri::command]
fn export_proxies(format: String) -> Result<String, String> {
  match format.as_str() {
    "json" => crate::proxy_manager::PROXY_MANAGER.export_proxies_json(),
    "txt" => Ok(crate::proxy_manager::PROXY_MANAGER.export_proxies_txt()),
    _ => Err(format!("Unsupported export format: {format}")),
  }
}

#[tauri::command]
async fn import_proxies_json(
  app_handle: tauri::AppHandle,
  content: String,
) -> Result<crate::proxy_manager::ProxyImportResult, String> {
  crate::proxy_manager::PROXY_MANAGER
    .import_proxies_json(&app_handle, &content)
    .map_err(|e| format!("Failed to import proxies: {e}"))
}

#[tauri::command]
fn parse_txt_proxies(content: String) -> Vec<crate::proxy_manager::ProxyParseResult> {
  crate::proxy_manager::ProxyManager::parse_txt_proxies(&content)
}

#[tauri::command]
async fn import_proxies_from_parsed(
  app_handle: tauri::AppHandle,
  parsed_proxies: Vec<crate::proxy_manager::ParsedProxyLine>,
  name_prefix: Option<String>,
) -> Result<crate::proxy_manager::ProxyImportResult, String> {
  crate::proxy_manager::PROXY_MANAGER
    .import_proxies_from_parsed(&app_handle, parsed_proxies, name_prefix)
    .map_err(|e| format!("Failed to import proxies: {e}"))
}

#[tauri::command]
fn read_profile_cookies(profile_id: String) -> Result<cookie_manager::CookieReadResult, String> {
  cookie_manager::CookieManager::read_cookies(&profile_id)
}

#[tauri::command]
async fn copy_profile_cookies(
  app_handle: tauri::AppHandle,
  request: cookie_manager::CookieCopyRequest,
) -> Result<Vec<cookie_manager::CookieCopyResult>, String> {
  cookie_manager::CookieManager::copy_cookies(&app_handle, request).await
}

#[tauri::command]
fn check_wayfern_terms_accepted() -> bool {
  wayfern_terms::WayfernTermsManager::instance().is_terms_accepted()
}

#[tauri::command]
fn check_wayfern_downloaded() -> bool {
  wayfern_terms::WayfernTermsManager::instance().is_wayfern_downloaded()
}

#[tauri::command]
async fn accept_wayfern_terms() -> Result<(), String> {
  wayfern_terms::WayfernTermsManager::instance()
    .accept_terms()
    .await
}

#[tauri::command]
async fn get_commercial_trial_status(
  app_handle: tauri::AppHandle,
) -> Result<commercial_license::TrialStatus, String> {
  commercial_license::CommercialLicenseManager::instance()
    .get_trial_status(&app_handle)
    .await
}

#[tauri::command]
async fn acknowledge_trial_expiration(app_handle: tauri::AppHandle) -> Result<(), String> {
  commercial_license::CommercialLicenseManager::instance()
    .acknowledge_expiration(&app_handle)
    .await
}

#[tauri::command]
fn has_acknowledged_trial_expiration(app_handle: tauri::AppHandle) -> Result<bool, String> {
  commercial_license::CommercialLicenseManager::instance().has_acknowledged(&app_handle)
}

#[tauri::command]
async fn start_mcp_server(app_handle: tauri::AppHandle) -> Result<u16, String> {
  mcp_server::McpServer::instance().start(app_handle).await
}

#[tauri::command]
async fn stop_mcp_server() -> Result<(), String> {
  mcp_server::McpServer::instance().stop().await
}

#[tauri::command]
fn get_mcp_server_status() -> bool {
  mcp_server::McpServer::instance().is_running()
}

#[derive(serde::Serialize)]
struct McpConfig {
  port: u16,
  token: String,
  config_json: String,
}

#[tauri::command]
async fn get_mcp_config(app_handle: tauri::AppHandle) -> Result<Option<McpConfig>, String> {
  let mcp_server = mcp_server::McpServer::instance();
  if !mcp_server.is_running() {
    return Ok(None);
  }

  let port = mcp_server
    .get_port()
    .ok_or("MCP server port not available")?;

  let settings_manager = settings_manager::SettingsManager::instance();
  let token = settings_manager
    .get_mcp_token(&app_handle)
    .await
    .map_err(|e| format!("Failed to get MCP token: {e}"))?
    .ok_or("MCP token not found")?;

  let config_json = serde_json::json!({
    "mcpServers": {
      "donut-browser": {
        "url": format!("http://127.0.0.1:{}/mcp", port),
        "headers": {
          "Authorization": format!("Bearer {}", token)
        }
      }
    }
  })
  .to_string();

  Ok(Some(McpConfig {
    port,
    token,
    config_json,
  }))
}

#[tauri::command]
async fn is_geoip_database_available() -> Result<bool, String> {
  Ok(GeoIPDownloader::is_geoip_database_available())
}

#[tauri::command]
async fn get_all_traffic_snapshots() -> Result<Vec<crate::traffic_stats::TrafficSnapshot>, String> {
  // Use real-time snapshots that merge in-memory data with disk data
  Ok(crate::traffic_stats::get_all_traffic_snapshots_realtime())
}

#[tauri::command]
async fn clear_all_traffic_stats() -> Result<(), String> {
  crate::traffic_stats::clear_all_traffic_stats()
    .map_err(|e| format!("Failed to clear traffic stats: {e}"))
}

#[tauri::command]
async fn get_traffic_stats_for_period(
  profile_id: String,
  seconds: u64,
) -> Result<Option<crate::traffic_stats::FilteredTrafficStats>, String> {
  Ok(crate::traffic_stats::get_traffic_stats_for_period(
    &profile_id,
    seconds,
  ))
}

#[tauri::command]
async fn download_geoip_database(app_handle: tauri::AppHandle) -> Result<(), String> {
  let downloader = GeoIPDownloader::instance();
  downloader
    .download_geoip_database(&app_handle)
    .await
    .map_err(|e| format!("Failed to download GeoIP database: {e}"))
}

// VPN commands
#[tauri::command]
async fn import_vpn_config(
  content: String,
  filename: String,
  name: Option<String>,
) -> Result<vpn::VpnImportResult, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  match storage.import_config(&content, &filename, name.clone()) {
    Ok(config) => Ok(vpn::VpnImportResult {
      success: true,
      vpn_id: Some(config.id),
      vpn_type: Some(config.vpn_type),
      name: config.name,
      error: None,
    }),
    Err(e) => Ok(vpn::VpnImportResult {
      success: false,
      vpn_id: None,
      vpn_type: None,
      name: name.unwrap_or_else(|| filename.clone()),
      error: Some(e.to_string()),
    }),
  }
}

#[tauri::command]
async fn list_vpn_configs() -> Result<Vec<vpn::VpnConfig>, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  storage
    .list_configs()
    .map_err(|e| format!("Failed to list VPN configs: {e}"))
}

#[tauri::command]
async fn get_vpn_config(vpn_id: String) -> Result<vpn::VpnConfig, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  storage
    .load_config(&vpn_id)
    .map_err(|e| format!("Failed to load VPN config: {e}"))
}

#[tauri::command]
async fn delete_vpn_config(app_handle: tauri::AppHandle, vpn_id: String) -> Result<(), String> {
  // First disconnect if connected (stop VPN worker)
  let _ = vpn_worker_runner::stop_vpn_worker_by_vpn_id(&vpn_id).await;

  // Check if sync was enabled before deleting
  let was_sync_enabled = {
    let storage = vpn::VPN_STORAGE
      .lock()
      .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;
    storage
      .load_config(&vpn_id)
      .map(|c| c.sync_enabled)
      .unwrap_or(false)
  };

  // Delete from storage
  {
    let storage = vpn::VPN_STORAGE
      .lock()
      .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

    storage
      .delete_config(&vpn_id)
      .map_err(|e| format!("Failed to delete VPN config: {e}"))?;
  }

  // If sync was enabled, also delete from remote
  if was_sync_enabled {
    let vpn_id_clone = vpn_id.clone();
    let app_handle_clone = app_handle.clone();
    tauri::async_runtime::spawn(async move {
      match sync::SyncEngine::create_from_settings(&app_handle_clone).await {
        Ok(engine) => {
          if let Err(e) = engine.delete_vpn(&vpn_id_clone).await {
            log::warn!("Failed to delete VPN {} from sync: {}", vpn_id_clone, e);
          } else {
            log::info!("VPN {} deleted from sync storage", vpn_id_clone);
          }
        }
        Err(e) => {
          log::debug!("Sync not configured, skipping remote VPN deletion: {}", e);
        }
      }
    });
  }

  let _ = events::emit("vpn-configs-changed", ());

  Ok(())
}

#[tauri::command]
async fn create_vpn_config_manual(
  name: String,
  vpn_type: vpn::VpnType,
  config_data: String,
) -> Result<vpn::VpnConfig, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  storage
    .create_config_manual(&name, vpn_type, &config_data)
    .map_err(|e| format!("Failed to create VPN config: {e}"))
}

#[tauri::command]
async fn update_vpn_config(vpn_id: String, name: String) -> Result<vpn::VpnConfig, String> {
  let storage = vpn::VPN_STORAGE
    .lock()
    .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;

  storage
    .update_config_name(&vpn_id, &name)
    .map_err(|e| format!("Failed to update VPN config: {e}"))
}

#[tauri::command]
async fn check_vpn_validity(
  vpn_id: String,
) -> Result<crate::proxy_manager::ProxyCheckResult, String> {
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs();

  // Start a temporary VPN worker to send real traffic
  let vpn_worker = vpn_worker_runner::start_vpn_worker(&vpn_id)
    .await
    .map_err(|e| format!("Failed to start VPN worker: {e}"))?;

  let socks_url = format!("socks5://127.0.0.1:{}", vpn_worker.local_port.unwrap_or(0));

  // Fetch public IP through the VPN SOCKS5 proxy
  let result = match ip_utils::fetch_public_ip(Some(&socks_url)).await {
    Ok(ip) => {
      let (city, country, country_code) =
        crate::proxy_manager::ProxyManager::get_ip_geolocation(&ip)
          .await
          .unwrap_or_default();

      crate::proxy_manager::ProxyCheckResult {
        ip,
        city,
        country,
        country_code,
        timestamp: now,
        is_valid: true,
      }
    }
    Err(e) => {
      log::warn!("VPN check failed to fetch public IP: {e}");
      crate::proxy_manager::ProxyCheckResult {
        ip: String::new(),
        city: None,
        country: None,
        country_code: None,
        timestamp: now,
        is_valid: false,
      }
    }
  };

  // Stop the temporary VPN worker
  let _ = vpn_worker_runner::stop_vpn_worker(&vpn_worker.id).await;

  Ok(result)
}

#[tauri::command]
async fn connect_vpn(vpn_id: String) -> Result<(), String> {
  // Start VPN worker process (detached, survives GUI shutdown)
  vpn_worker_runner::start_vpn_worker(&vpn_id)
    .await
    .map_err(|e| format!("Failed to connect VPN: {e}"))?;

  // Update last_used timestamp
  {
    let storage = vpn::VPN_STORAGE
      .lock()
      .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;
    let _ = storage.update_last_used(&vpn_id);
  }

  Ok(())
}

#[tauri::command]
async fn disconnect_vpn(vpn_id: String) -> Result<(), String> {
  vpn_worker_runner::stop_vpn_worker_by_vpn_id(&vpn_id)
    .await
    .map_err(|e| format!("Failed to disconnect VPN: {e}"))?;
  Ok(())
}

#[tauri::command]
async fn get_vpn_status(vpn_id: String) -> Result<vpn::VpnStatus, String> {
  use crate::proxy_storage::is_process_running;

  if let Some(worker) = vpn_worker_storage::find_vpn_worker_by_vpn_id(&vpn_id) {
    let connected = worker.pid.map(is_process_running).unwrap_or(false);
    Ok(vpn::VpnStatus {
      connected,
      vpn_id,
      connected_at: None,
      bytes_sent: None,
      bytes_received: None,
      last_handshake: None,
    })
  } else {
    Ok(vpn::VpnStatus {
      connected: false,
      vpn_id,
      connected_at: None,
      bytes_sent: None,
      bytes_received: None,
      last_handshake: None,
    })
  }
}

#[tauri::command]
async fn list_active_vpn_connections() -> Result<Vec<vpn::VpnStatus>, String> {
  use crate::proxy_storage::is_process_running;

  let workers = vpn_worker_storage::list_vpn_worker_configs();
  Ok(
    workers
      .into_iter()
      .filter(|w| w.pid.map(is_process_running).unwrap_or(false))
      .map(|w| vpn::VpnStatus {
        connected: true,
        vpn_id: w.vpn_id,
        connected_at: None,
        bytes_sent: None,
        bytes_received: None,
        last_handshake: None,
      })
      .collect(),
  )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let args: Vec<String> = env::args().collect();
  let startup_url = args.iter().find(|arg| arg.starts_with("http")).cloned();

  if let Some(url) = startup_url.clone() {
    log::info!("Found startup URL in command line: {url}");
    let mut pending = PENDING_URLS.lock().unwrap();
    pending.push(url.clone());
  }

  // Configure logging plugin with separate logs for dev and production
  let log_file_name = if cfg!(debug_assertions) {
    "DonutBrowserDev"
  } else {
    "DonutBrowser"
  };

  tauri::Builder::default()
    .plugin(
      tauri_plugin_log::Builder::new()
        .clear_targets() // Clear default targets to avoid duplicates
        .target(Target::new(TargetKind::Stdout))
        .target(Target::new(TargetKind::Webview))
        .target(Target::new(TargetKind::LogDir {
          file_name: Some(log_file_name.to_string()),
        }))
        .max_file_size(100_000) // 100KB
        .level(log::LevelFilter::Info)
        .format(|out, message, record| {
          use chrono::Local;
          let now = Local::now();
          let timestamp = format!(
            "{}.{:03}",
            now.format("%Y-%m-%d %H:%M:%S"),
            now.timestamp_subsec_millis()
          );
          out.finish(format_args!(
            "[{}][{}][{}] {}",
            timestamp,
            record.target(),
            record.level(),
            message
          ))
        })
        .build(),
    )
    .plugin(tauri_plugin_single_instance::init(|_, args, _cwd| {
      log::info!("Single instance triggered with args: {args:?}");
    }))
    .plugin(tauri_plugin_deep_link::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_opener::init())
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_macos_permissions::init())
    .setup(|app| {
      // Start the daemon for tray icon
      if let Err(e) = daemon_spawn::ensure_daemon_running() {
        log::warn!("Failed to start daemon: {e}");
      }

      // Monitor daemon health - quit GUI if daemon dies
      let app_handle_daemon = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        // Give the daemon time to fully start
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
          interval.tick().await;

          let is_running = tokio::task::spawn_blocking(daemon_spawn::is_daemon_running)
            .await
            .unwrap_or(false);

          if !is_running {
            log::warn!("Daemon is no longer running, quitting GUI");
            app_handle_daemon.exit(0);
            break;
          }
        }
      });

      // Create the main window programmatically
      #[allow(unused_variables)]
      let win_builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Donut Browser")
        .inner_size(800.0, 500.0)
        .resizable(false)
        .fullscreen(false)
        .center()
        .focused(true)
        .visible(true);

      #[cfg(target_os = "windows")]
      let win_builder = win_builder.decorations(false);

      #[allow(unused_variables)]
      let window = win_builder.build().unwrap();

      // Set transparent titlebar for macOS
      #[cfg(target_os = "macos")]
      {
        if let Err(e) = window.set_transparent_titlebar(true) {
          log::warn!("Failed to set transparent titlebar: {e}");
        }
      }

      // Set up deep link handler
      let handle = app.handle().clone();

      // Initialize the global event emitter for the events module
      let emitter = std::sync::Arc::new(events::TauriEmitter::new(handle.clone()));
      if let Err(e) = events::set_global_emitter(emitter) {
        log::warn!("Failed to set global event emitter: {e}");
      }

      #[cfg(windows)]
      {
        // For Windows, register all deep links at runtime
        if let Err(e) = app.deep_link().register_all() {
          log::warn!("Failed to register deep links: {e}");
        }
      }

      #[cfg(target_os = "macos")]
      {
        // On macOS, try to register deep links for development builds
        if let Err(e) = app.deep_link().register_all() {
          log::debug!(
            "Note: Deep link registration failed on macOS (this is normal for production): {e}"
          );
        }
      }

      app.deep_link().on_open_url({
        let handle = handle.clone();
        move |event| {
          let urls = event.urls();
          log::info!("Deep link event received with {} URLs", urls.len());

          for url in urls {
            let url_string = url.to_string();
            log::info!("Deep link received: {url_string}");

            // Clone the handle for each async task
            let handle_clone = handle.clone();

            // Handle the URL asynchronously
            tauri::async_runtime::spawn(async move {
              if let Err(e) = handle_url_open(handle_clone, url_string.clone()).await {
                log::error!("Failed to handle deep link URL: {e}");
              }
            });
          }
        }
      });

      if let Some(startup_url) = startup_url {
        let handle_clone = handle.clone();
        tauri::async_runtime::spawn(async move {
          log::info!("Processing startup URL from command line: {startup_url}");
          if let Err(e) = handle_url_open(handle_clone, startup_url.clone()).await {
            log::error!("Failed to handle startup URL: {e}");
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
            log::error!("Failed to start background updates: {e}");
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

      // Handle any pending URLs that were received before the window was ready
      let handle_pending = handle.clone();
      tauri::async_runtime::spawn(async move {
        // Wait a bit for the window to be fully ready
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        let pending_urls = {
          let mut pending = PENDING_URLS.lock().unwrap();
          let urls = pending.clone();
          pending.clear();
          urls
        };

        for url in pending_urls {
          log::info!("Processing pending URL: {url}");
          if let Err(e) = handle_url_open(handle_pending.clone(), url).await {
            log::error!("Failed to handle pending URL: {e}");
          }
        }
      });

      // Start periodic cleanup task for unused binaries
      // Only runs when sync is not in progress to avoid deleting browsers
      // that might be needed for profiles being synced from the cloud
      tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(43200)); // Every 12 hours

        loop {
          interval.tick().await;

          // Check if sync is in progress before running cleanup
          if let Some(scheduler) = sync::get_global_scheduler() {
            if scheduler.is_sync_in_progress().await {
              log::debug!("Skipping cleanup: sync is in progress");
              continue;
            }
          }

          let registry =
            crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
          if let Err(e) = registry.cleanup_unused_binaries() {
            log::error!("Periodic cleanup failed: {e}");
          } else {
            log::debug!("Periodic cleanup completed successfully");
          }
        }
      });

      let _app_handle_update = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        log::info!("Starting app update check at startup...");
        let updater = app_auto_updater::AppAutoUpdater::instance();
        match updater.check_for_updates().await {
          Ok(Some(update_info)) => {
            log::info!(
              "App update available: {} -> {}",
              update_info.current_version,
              update_info.new_version
            );
            // Emit update available event to the frontend
            if let Err(e) = events::emit("app-update-available", &update_info) {
              log::error!("Failed to emit app update event: {e}");
            } else {
              log::debug!("App update event emitted successfully");
            }
          }
          Ok(None) => {
            log::debug!("No app updates available");
          }
          Err(e) => {
            log::error!("Failed to check for app updates: {e}");
          }
        }
      });

      // Start Camoufox cleanup task
      let _app_handle_cleanup = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        let camoufox_manager = crate::camoufox_manager::CamoufoxManager::instance();
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

        loop {
          interval.tick().await;

          match camoufox_manager.cleanup_dead_instances().await {
            Ok(_) => {
              // Cleanup completed silently
            }
            Err(e) => {
              log::error!("Error during Camoufox cleanup: {e}");
            }
          }
        }
      });

      // Check and download GeoIP database at startup if needed
      let app_handle_geoip = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        // Wait a bit for the app to fully initialize
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let geoip_downloader = crate::geoip_downloader::GeoIPDownloader::instance();
        match geoip_downloader.check_missing_geoip_database() {
          Ok(true) => {
            log::info!(
              "GeoIP database is missing for Camoufox profiles, downloading at startup..."
            );
            let geoip_downloader = GeoIPDownloader::instance();
            if let Err(e) = geoip_downloader
              .download_geoip_database(&app_handle_geoip)
              .await
            {
              log::error!("Failed to download GeoIP database at startup: {e}");
            } else {
              log::info!("GeoIP database downloaded successfully at startup");
            }
          }
          Ok(false) => {
            // No Camoufox profiles or GeoIP database already available
          }
          Err(e) => {
            log::error!("Failed to check GeoIP database status at startup: {e}");
          }
        }
      });

      // Start proxy cleanup task for dead browser processes
      let app_handle_proxy_cleanup = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));

        loop {
          interval.tick().await;

          match crate::proxy_manager::PROXY_MANAGER
            .cleanup_dead_proxies(app_handle_proxy_cleanup.clone())
            .await
          {
            Ok(dead_pids) => {
              if !dead_pids.is_empty() {
                log::info!(
                  "Cleaned up proxies for {} dead browser processes",
                  dead_pids.len()
                );
              }
            }
            Err(e) => {
              log::error!("Error during proxy cleanup: {e}");
            }
          }
        }
      });

      // Periodically broadcast browser running status to the frontend
      let app_handle_status = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_running_states: std::collections::HashMap<String, bool> =
          std::collections::HashMap::new();

        loop {
          interval.tick().await;

          let runner = crate::browser_runner::BrowserRunner::instance();
          // If listing profiles fails, skip this tick
          let profiles = match runner.profile_manager.list_profiles() {
            Ok(p) => p,
            Err(e) => {
              log::warn!("Failed to list profiles in status checker: {e}");
              continue;
            }
          };

          for profile in profiles {
            // Check browser status and track changes
            match runner
              .check_browser_status(app_handle_status.clone(), &profile)
              .await
            {
              Ok(is_running) => {
                let profile_id = profile.id.to_string();
                let last_state = last_running_states
                  .get(&profile_id)
                  .copied()
                  .unwrap_or(false);

                // Only emit event if state actually changed
                if last_state != is_running {
                  log::debug!(
                    "Status checker detected change for profile {}: {} -> {}",
                    profile.name,
                    last_state,
                    is_running
                  );

                  #[derive(serde::Serialize)]
                  struct RunningChangedPayload {
                    id: String,
                    is_running: bool,
                  }

                  let payload = RunningChangedPayload {
                    id: profile_id.clone(),
                    is_running,
                  };

                  if let Err(e) = events::emit("profile-running-changed", &payload) {
                    log::warn!("Failed to emit profile running changed event: {e}");
                  } else {
                    log::debug!(
                      "Status checker emitted profile-running-changed event for {}: running={}",
                      profile.name,
                      is_running
                    );
                  }

                  last_running_states.insert(profile_id, is_running);
                } else {
                  // Update the state even if unchanged to ensure we have it tracked
                  last_running_states.insert(profile_id, is_running);
                }
              }
              Err(e) => {
                log::warn!("Status check failed for profile {}: {}", profile.name, e);
                continue;
              }
            }
          }
        }
      });

      // Nodecar warm-up is now triggered from the frontend to allow UI blocking overlay

      // Start API server if enabled in settings
      let app_handle_api = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        match crate::settings_manager::get_app_settings(app_handle_api.clone()).await {
          Ok(settings) => {
            if settings.api_enabled {
              log::info!("API is enabled in settings, starting API server...");
              match crate::api_server::start_api_server_internal(settings.api_port, &app_handle_api)
                .await
              {
                Ok(port) => {
                  log::info!("API server started successfully on port {port}");
                  // Emit success toast to frontend
                  if let Err(e) = events::emit(
                    "show-toast",
                    crate::api_server::ToastPayload {
                      message: "API server started successfully".to_string(),
                      variant: "success".to_string(),
                      title: "Local API Started".to_string(),
                      description: Some(format!("API server running on port {port}")),
                    },
                  ) {
                    log::error!("Failed to emit API start toast: {e}");
                  }
                }
                Err(e) => {
                  log::error!("Failed to start API server at startup: {e}");
                  // Emit error toast to frontend
                  if let Err(toast_err) = events::emit(
                    "show-toast",
                    crate::api_server::ToastPayload {
                      message: "Failed to start API server".to_string(),
                      variant: "error".to_string(),
                      title: "Failed to Start Local API".to_string(),
                      description: Some(format!("Error: {e}")),
                    },
                  ) {
                    log::error!("Failed to emit API error toast: {toast_err}");
                  }
                }
              }
            }
          }
          Err(e) => {
            log::error!("Failed to load app settings for API startup: {e}");
          }
        }
      });

      // Start sync subscription and scheduler if configured
      let app_handle_sync = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        use std::sync::Arc;

        let mut subscription_manager = sync::SubscriptionManager::new();
        let work_rx = subscription_manager.take_work_receiver();

        if let Err(e) = subscription_manager.start(app_handle_sync.clone()).await {
          log::warn!("Failed to start sync subscription: {e}");
        }

        if let Some(work_rx) = work_rx {
          let scheduler = Arc::new(sync::SyncScheduler::new());

          // Set the global scheduler so commands can access it
          sync::set_global_scheduler(scheduler.clone());

          // Start initial sync for all enabled profiles
          scheduler.sync_all_enabled_profiles(&app_handle_sync).await;

          // Check for missing synced profiles (deleted locally but exist remotely)
          match sync::SyncEngine::create_from_settings(&app_handle_sync).await {
            Ok(engine) => {
              if let Err(e) = engine
                .check_for_missing_synced_profiles(&app_handle_sync)
                .await
              {
                log::warn!("Failed to check for missing profiles: {}", e);
              }
            }
            Err(e) => {
              log::debug!("Sync not configured, skipping missing profile check: {}", e);
            }
          }

          scheduler
            .clone()
            .start(app_handle_sync.clone(), work_rx)
            .await;
          log::info!("Sync scheduler started");
        }
      });

      // Start cloud auth background refresh loop
      let app_handle_cloud = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        // On startup, refresh sync token and proxy if cloud auth is active.
        // api_call_with_retry handles 401/refresh internally — no direct
        // refresh_access_token call needed.
        if cloud_auth::CLOUD_AUTH.is_logged_in().await {
          if let Err(e) = cloud_auth::CLOUD_AUTH.get_or_refresh_sync_token().await {
            log::warn!("Failed to refresh cloud sync token on startup: {e}");
          }
          cloud_auth::CLOUD_AUTH.sync_cloud_proxy().await;
        }
        cloud_auth::CloudAuthManager::start_sync_token_refresh_loop(app_handle_cloud).await;
      });

      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      get_supported_browsers,
      is_browser_supported_on_platform,
      download_browser,
      cancel_download,
      delete_profile,
      clone_profile,
      check_browser_exists,
      create_browser_profile_new,
      list_browser_profiles,
      launch_browser_profile,
      fetch_browser_versions_with_count,
      fetch_browser_versions_cached_first,
      fetch_browser_versions_with_count_cached_first,
      get_downloaded_browser_versions,
      get_all_tags,
      get_browser_release_types,
      update_profile_proxy,
      update_profile_vpn,
      update_profile_tags,
      update_profile_note,
      check_browser_status,
      kill_browser_profile,
      rename_profile,
      get_app_settings,
      save_app_settings,
      should_show_launch_on_login_prompt,
      enable_launch_on_login,
      decline_launch_on_login,
      get_table_sorting_settings,
      save_table_sorting_settings,
      get_system_language,
      clear_all_version_cache_and_refetch,
      is_default_browser,
      open_url_with_profile,
      set_as_default_browser,
      trigger_manual_version_update,
      get_version_update_status,
      check_for_browser_updates,
      dismiss_update_notification,
      complete_browser_update_with_auto_update,
      check_for_app_updates,
      check_for_app_updates_manual,
      download_and_prepare_app_update,
      restart_application,
      detect_existing_profiles,
      import_browser_profile,
      check_missing_binaries,
      check_missing_geoip_database,
      ensure_all_binaries_exist,
      ensure_active_browsers_downloaded,
      create_stored_proxy,
      get_stored_proxies,
      update_stored_proxy,
      delete_stored_proxy,
      check_proxy_validity,
      get_cached_proxy_check,
      export_proxies,
      import_proxies_json,
      parse_txt_proxies,
      import_proxies_from_parsed,
      update_camoufox_config,
      update_wayfern_config,
      get_profile_groups,
      get_groups_with_profile_counts,
      create_profile_group,
      update_profile_group,
      delete_profile_group,
      assign_profiles_to_group,
      delete_selected_profiles,
      is_geoip_database_available,
      download_geoip_database,
      start_api_server,
      stop_api_server,
      get_api_server_status,
      get_all_traffic_snapshots,
      clear_all_traffic_stats,
      get_traffic_stats_for_period,
      get_sync_settings,
      save_sync_settings,
      set_profile_sync_enabled,
      request_profile_sync,
      set_proxy_sync_enabled,
      set_group_sync_enabled,
      is_proxy_in_use_by_synced_profile,
      is_group_in_use_by_synced_profile,
      set_vpn_sync_enabled,
      is_vpn_in_use_by_synced_profile,
      read_profile_cookies,
      copy_profile_cookies,
      check_wayfern_terms_accepted,
      check_wayfern_downloaded,
      accept_wayfern_terms,
      get_commercial_trial_status,
      acknowledge_trial_expiration,
      has_acknowledged_trial_expiration,
      start_mcp_server,
      stop_mcp_server,
      get_mcp_server_status,
      get_mcp_config,
      // VPN commands
      import_vpn_config,
      list_vpn_configs,
      get_vpn_config,
      delete_vpn_config,
      create_vpn_config_manual,
      update_vpn_config,
      check_vpn_validity,
      connect_vpn,
      disconnect_vpn,
      get_vpn_status,
      list_active_vpn_connections,
      // Cloud auth commands
      cloud_auth::cloud_request_otp,
      cloud_auth::cloud_verify_otp,
      cloud_auth::cloud_get_user,
      cloud_auth::cloud_refresh_profile,
      cloud_auth::cloud_logout,
      cloud_auth::cloud_get_proxy_usage,
      cloud_auth::cloud_get_countries,
      cloud_auth::cloud_get_states,
      cloud_auth::cloud_get_cities,
      cloud_auth::create_cloud_location_proxy,
      cloud_auth::restart_sync_service
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
    // Commands that are intentionally not used in the frontend
    // but are used via MCP server or other programmatic APIs
    let mcp_only_commands = [
      "connect_vpn",
      "disconnect_vpn",
      "get_vpn_status",
      "get_vpn_config",
      "list_active_vpn_connections",
    ];

    // Extract command names from the generate_handler! macro in this file
    let lib_rs_content = fs::read_to_string("src/lib.rs").expect("Failed to read lib.rs");
    let commands = extract_tauri_commands(&lib_rs_content);

    // Get all frontend files
    let frontend_files = get_frontend_files("../src");

    // Check which commands are actually used
    let mut unused_commands = Vec::new();
    let mut used_commands = Vec::new();

    for command in &commands {
      // Skip commands that are intentionally MCP-only
      if mcp_only_commands.contains(&command.as_str()) {
        used_commands.push(command.clone());
        if verbose {
          println!("✅ {command} (MCP-only)");
        }
        continue;
      }

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
              // Strip module prefix (e.g., "cloud_auth::cloud_request_otp" -> "cloud_request_otp")
              let command = command.rsplit("::").next().unwrap_or(command);
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

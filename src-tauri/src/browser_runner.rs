use crate::browser::ProxySettings;
use crate::cloud_auth::CLOUD_AUTH;
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::events;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::PROXY_MANAGER;
use crate::wayfern_manager::{WayfernConfig, WayfernManager};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct BrowserRunner {
  pub profile_manager: &'static ProfileManager,
  pub downloaded_browsers_registry: &'static DownloadedBrowsersRegistry,
  auto_updater: &'static crate::auto_updater::AutoUpdater,
  wayfern_manager: &'static WayfernManager,
}

impl BrowserRunner {
  fn new() -> Self {
    Self {
      profile_manager: ProfileManager::instance(),
      downloaded_browsers_registry: DownloadedBrowsersRegistry::instance(),
      auto_updater: crate::auto_updater::AutoUpdater::instance(),
      wayfern_manager: WayfernManager::instance(),
    }
  }

  pub fn instance() -> &'static BrowserRunner {
    &BROWSER_RUNNER
  }

  pub fn get_binaries_dir(&self) -> PathBuf {
    crate::app_dirs::binaries_dir()
  }

  /// Resolve the DNS blocklist level to a cached file path.
  /// If a level is set but the cache is missing, fetches on demand (blocks until done).
  async fn resolve_blocklist_file(
    profile: &crate::profile::BrowserProfile,
  ) -> Result<Option<String>, String> {
    let Some(ref level_str) = profile.dns_blocklist else {
      return Ok(None);
    };
    let Some(level) = crate::dns_blocklist::BlocklistLevel::parse_level(level_str) else {
      return Ok(None);
    };
    if level == crate::dns_blocklist::BlocklistLevel::None {
      return Ok(None);
    }
    let path = crate::dns_blocklist::BlocklistManager::ensure_cached(level)
      .await
      .map_err(|e| format!("Failed to fetch DNS blocklist: {e}"))?;
    Ok(Some(path.to_string_lossy().to_string()))
  }

  /// Refresh cloud proxy credentials if the profile uses a cloud or cloud-derived proxy,
  /// then resolve the proxy settings with profile-specific sid for sticky sessions.
  async fn resolve_proxy_with_refresh(
    &self,
    proxy_id: Option<&String>,
    profile_id: Option<&str>,
  ) -> Result<Option<ProxySettings>, String> {
    let proxy_id = match proxy_id {
      Some(id) => id,
      None => return Ok(None),
    };

    if PROXY_MANAGER.is_cloud_or_derived(proxy_id) {
      log::info!("Refreshing cloud proxy credentials before launch for proxy {proxy_id}");
      CLOUD_AUTH.sync_cloud_proxy().await;
    }
    // For cloud-derived proxies, inject profile-specific sid for sticky sessions
    if let Some(pid) = profile_id {
      if PROXY_MANAGER.is_cloud_or_derived(proxy_id) {
        return Ok(PROXY_MANAGER.resolve_proxy_for_profile(proxy_id, pid));
      }
    }
    Ok(PROXY_MANAGER.get_proxy_settings_by_id(proxy_id))
  }

  fn fire_launch_hook(profile: &BrowserProfile) {
    let Some(raw_url) = profile.launch_hook.as_deref() else {
      return;
    };
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
      return;
    }

    let parsed = match url::Url::parse(trimmed) {
      Ok(u) => u,
      Err(e) => {
        log::warn!(
          "Skipping launch hook for profile {} (ID: {}): invalid URL: {e}",
          profile.name,
          profile.id
        );
        return;
      }
    };

    if !matches!(parsed.scheme(), "http" | "https") {
      log::warn!(
        "Skipping launch hook for profile {} (ID: {}): URL must be http or https",
        profile.name,
        profile.id
      );
      return;
    }

    let url = parsed.to_string();
    let profile_name = profile.name.clone();
    let profile_id = profile.id.to_string();

    log::info!("Firing launch hook GET {url} for profile {profile_name} (ID: {profile_id})");

    tokio::spawn(async move {
      let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
      {
        Ok(c) => c,
        Err(e) => {
          log::warn!("Launch hook client build failed for {url}: {e}");
          return;
        }
      };

      match client.get(&url).send().await {
        Ok(resp) => {
          log::info!(
            "Launch hook {url} for profile {profile_name} returned status {}",
            resp.status()
          );
        }
        Err(e) => {
          log::warn!("Launch hook {url} for profile {profile_name} failed: {e}");
        }
      }
    });
  }

  async fn resolve_launch_proxy(
    &self,
    profile: &BrowserProfile,
  ) -> Result<Option<ProxySettings>, String> {
    Self::fire_launch_hook(profile);

    self
      .resolve_proxy_with_refresh(profile.proxy_id.as_ref(), Some(&profile.id.to_string()))
      .await
  }

  /// Get the executable path for a browser profile
  /// This is a common helper to eliminate code duplication across the codebase
  pub fn get_browser_executable_path(
    &self,
    profile: &BrowserProfile,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Create browser instance to get executable path
    let browser_type = crate::browser::BrowserType::from_str(&profile.browser)
      .map_err(|e| format!("Invalid browser type: {e}"))?;
    let browser = crate::browser::create_browser(browser_type);

    // Construct browser directory path: binaries/<browser>/<version>/
    let mut browser_dir = self.get_binaries_dir();
    browser_dir.push(&profile.browser);
    browser_dir.push(&profile.version);

    // Get platform-specific executable path
    browser
      .get_executable_path(&browser_dir)
      .map_err(|e| format!("Failed to get executable path for {}: {e}", profile.browser).into())
  }

  pub async fn launch_browser(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    local_proxy_settings: Option<&ProxySettings>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    self
      .launch_browser_internal(app_handle, profile, url, local_proxy_settings, None, false)
      .await
  }

  async fn launch_browser_internal(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    _local_proxy_settings: Option<&ProxySettings>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Handle Wayfern profiles using WayfernManager
    if profile.browser == "wayfern" {
      // Get or create wayfern config
      let mut wayfern_config = profile.wayfern_config.clone().unwrap_or_else(|| {
        log::info!(
          "No wayfern config found for profile {}, using default",
          profile.name
        );
        WayfernConfig::default()
      });

      // Always start a local proxy for Wayfern (for traffic monitoring and geoip support)
      let mut upstream_proxy = self
        .resolve_launch_proxy(profile)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;

      // If profile has a VPN instead of proxy, start VPN worker and use it as upstream
      if upstream_proxy.is_none() {
        if let Some(ref vpn_id) = profile.vpn_id {
          match crate::vpn_worker_runner::start_vpn_worker(vpn_id).await {
            Ok(vpn_worker) => {
              if let Some(port) = vpn_worker.local_port {
                upstream_proxy = Some(ProxySettings {
                  proxy_type: "socks5".to_string(),
                  host: "127.0.0.1".to_string(),
                  port,
                  username: None,
                  password: None,
                });
                log::info!("VPN worker started for Wayfern profile on port {}", port);
              }
            }
            Err(e) => {
              return Err(format!("Failed to start VPN worker: {e}").into());
            }
          }
        }
      }

      log::info!(
        "Starting local proxy for Wayfern profile: {} (upstream: {})",
        profile.name,
        upstream_proxy
          .as_ref()
          .map(|p| format!("{}:{}", p.host, p.port))
          .unwrap_or_else(|| "DIRECT".to_string())
      );

      // Start the proxy and get local proxy settings
      // If proxy startup fails, DO NOT launch Wayfern - it requires local proxy
      let profile_id_str = profile.id.to_string();
      let blocklist_file = Self::resolve_blocklist_file(profile).await?;
      let local_proxy = PROXY_MANAGER
        .start_proxy(
          app_handle.clone(),
          upstream_proxy.as_ref(),
          0, // Use 0 as temporary PID, will be updated later
          Some(&profile_id_str),
          profile.proxy_bypass_rules.clone(),
          blocklist_file,
          // Wayfern (Chromium) uses a local SOCKS5 proxy so QUIC and WebRTC
          // UDP can be routed through it (via SOCKS5 UDP ASSOCIATE) without
          // leaking the real IP, rather than being forced direct as they
          // would be over an HTTP CONNECT proxy.
          "socks5",
        )
        .await
        .map_err(|e| {
          let error_msg = format!("Failed to start local proxy for Wayfern: {e}");
          log::error!("{}", error_msg);
          error_msg
        })?;

      // Format proxy URL for wayfern - use SOCKS5 for the local proxy so
      // Chromium proxies UDP (QUIC/WebRTC), not just TCP.
      let proxy_url = format!("socks5://{}:{}", local_proxy.host, local_proxy.port);

      // Set proxy in wayfern config
      wayfern_config.proxy = Some(proxy_url);

      log::info!(
        "Configured local proxy for Wayfern: {:?}",
        wayfern_config.proxy
      );

      // Check if we need to generate a new fingerprint on every launch
      let mut updated_profile = profile.clone();
      if wayfern_config.randomize_fingerprint_on_launch == Some(true) {
        log::info!(
          "Generating random fingerprint for Wayfern profile: {}",
          profile.name
        );

        // Create a config copy without the existing fingerprint to force generation of a new one
        let mut config_for_generation = wayfern_config.clone();
        config_for_generation.fingerprint = None;

        // Generate a new fingerprint
        let (new_fingerprint, geolocation_applied) = self
          .wayfern_manager
          .generate_fingerprint_config(&app_handle, profile, &config_for_generation)
          .await
          .map_err(|e| format!("Failed to generate random fingerprint: {e}"))?;

        log::info!(
          "New fingerprint generated, length: {} chars",
          new_fingerprint.len()
        );

        // Update the config with the new fingerprint for launching
        wayfern_config.fingerprint = Some(new_fingerprint.clone());

        // Save the updated fingerprint to the profile so it persists.
        let mut updated_wayfern_config = updated_profile.wayfern_config.clone().unwrap_or_default();
        updated_wayfern_config.fingerprint = Some(new_fingerprint);
        // Preserve the randomize flag so it persists across launches
        updated_wayfern_config.randomize_fingerprint_on_launch = Some(true);
        // Preserve the OS setting so it's used for future fingerprint generation
        if wayfern_config.os.is_some() {
          updated_wayfern_config.os = wayfern_config.os.clone();
        }
        // The fresh fingerprint's location matches the current routing; record
        // its signature so launches keep it in sync with the non-randomize
        // path. Only when geolocation actually applied — otherwise leave it
        // unset so the refresh path can repair the location if the user later
        // turns randomize off.
        updated_wayfern_config.geo_proxy_signature = if geolocation_applied {
          Some(crate::wayfern_manager::WayfernManager::geo_signature(
            upstream_proxy.as_ref(),
            profile.vpn_id.as_deref(),
            wayfern_config.geoip.as_ref(),
          ))
        } else {
          None
        };
        updated_profile.wayfern_config = Some(updated_wayfern_config.clone());

        log::info!(
          "Updated profile wayfern_config with new fingerprint for profile: {}, fingerprint length: {}",
          profile.name,
          updated_wayfern_config.fingerprint.as_ref().map(|f| f.len()).unwrap_or(0)
        );
      } else {
        // Safety net: the stored fingerprint's timezone and geolocation were
        // computed for whatever proxy was set when the fingerprint was
        // generated. If the profile's proxy or VPN has changed since (the
        // common case being a user who forgot to set a proxy at creation and
        // added one afterwards), that location data is stale and the user would
        // see the wrong timezone on first launch. When the routing signature no
        // longer matches, refresh just the location fields of the stored
        // fingerprint through the current proxy. Wayfern only; the randomize
        // path above already regenerates the whole fingerprint each launch.
        let current_geo_sig = crate::wayfern_manager::WayfernManager::geo_signature(
          upstream_proxy.as_ref(),
          profile.vpn_id.as_deref(),
          wayfern_config.geoip.as_ref(),
        );
        let geo_enabled = !matches!(
          wayfern_config.geoip.as_ref(),
          Some(serde_json::Value::Bool(false))
        );
        if geo_enabled
          && wayfern_config.geo_proxy_signature.as_deref() != Some(current_geo_sig.as_str())
        {
          if let Some(stored_fp) = wayfern_config.fingerprint.clone() {
            log::info!(
              "Routing changed for Wayfern profile {} since its fingerprint was generated (was {:?}, now {}); refreshing timezone and geolocation",
              profile.name,
              wayfern_config.geo_proxy_signature,
              current_geo_sig
            );
            match crate::wayfern_manager::WayfernManager::refresh_fingerprint_geolocation(
              &stored_fp,
              wayfern_config.proxy.as_deref(),
              wayfern_config.geoip.as_ref(),
            )
            .await
            {
              Some(refreshed) => {
                // Use the refreshed fingerprint for this launch...
                wayfern_config.fingerprint = Some(refreshed.clone());
                wayfern_config.geo_proxy_signature = Some(current_geo_sig.clone());
                // ...and persist it so the corrected location sticks and we do
                // not refresh again on the next launch with the same proxy.
                let mut cfg = updated_profile.wayfern_config.clone().unwrap_or_default();
                cfg.fingerprint = Some(refreshed);
                cfg.geo_proxy_signature = Some(current_geo_sig);
                updated_profile.wayfern_config = Some(cfg);
              }
              None => {
                log::warn!(
                  "Could not refresh geolocation for Wayfern profile {} (proxy unreachable?); launching with existing location and will retry next launch",
                  profile.name
                );
              }
            }
          }
        }
      }

      // Create ephemeral dir for ephemeral or password-protected profiles
      if profile.password_protected {
        crate::profile::password::prepare_for_launch(profile)
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
      } else if profile.ephemeral {
        crate::ephemeral_dirs::create_ephemeral_dir(&profile.id.to_string())
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
      }

      // Launch Wayfern browser
      log::info!("Launching Wayfern for profile: {}", profile.name);

      // Get profile path for Wayfern
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path =
        crate::ephemeral_dirs::get_effective_profile_path(&updated_profile, &profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy().to_string();

      // Install extensions if an extension group is assigned
      let mut extension_paths = Vec::new();
      if updated_profile.extension_group_id.is_some() {
        let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
        match mgr.install_extensions_for_profile(&updated_profile, &profile_data_path) {
          Ok(paths) => {
            if !paths.is_empty() {
              log::info!(
                "Prepared {} Chromium extensions for profile: {}",
                paths.len(),
                updated_profile.name
              );
            }
            extension_paths = paths;
          }
          Err(e) => {
            log::warn!("Failed to install extensions for Wayfern profile: {e}");
          }
        }
      }

      // Get proxy URL from config
      let proxy_url = wayfern_config.proxy.as_deref();

      let wayfern_result = self
        .wayfern_manager
        .launch_wayfern(
          &app_handle,
          &updated_profile,
          &profile_path_str,
          &wayfern_config,
          url.as_deref(),
          proxy_url,
          profile.ephemeral,
          &extension_paths,
          remote_debugging_port,
          headless,
        )
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to launch Wayfern: {e}").into()
        })?;

      // Get the process ID from launch result
      let process_id = wayfern_result.processId.unwrap_or(0);
      log::info!("Wayfern launched successfully with PID: {process_id}");

      // Wayfern.setFingerprint echoes back the fingerprint the browser actually
      // applied, which may be UPGRADED from the stored one (e.g. when the
      // stored fingerprint targets an older browser version). Persist it so the
      // next launch starts from the upgraded value — saved below via
      // save_process_info(&updated_profile).
      if let Some(used_fp) = wayfern_result.used_fingerprint.clone() {
        let mut cfg = updated_profile.wayfern_config.clone().unwrap_or_default();
        if cfg.fingerprint.as_deref() != Some(used_fp.as_str()) {
          log::info!(
            "Persisting upgraded fingerprint from Wayfern.setFingerprint for profile: {} (len {})",
            profile.name,
            used_fp.len()
          );
          cfg.fingerprint = Some(used_fp);
          updated_profile.wayfern_config = Some(cfg);
        }
      }

      // Update profile with the process info
      updated_profile.process_id = Some(process_id);
      updated_profile.last_launch = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());

      // Update the proxy manager with the correct PID
      if let Err(e) = PROXY_MANAGER.update_proxy_pid(0, process_id) {
        log::warn!("Warning: Failed to update proxy PID mapping: {e}");
      } else {
        log::info!("Updated proxy PID mapping from temp (0) to actual PID: {process_id}");
      }

      // Persist the real browser PID so the detached proxy worker self-reaps
      // when this browser dies, even after the GUI exits/restarts.
      PROXY_MANAGER.set_browser_pid_for_profile(&updated_profile.id.to_string(), process_id);

      // Save the updated profile
      log::info!(
        "Saving profile {} with wayfern_config fingerprint length: {}",
        updated_profile.name,
        updated_profile
          .wayfern_config
          .as_ref()
          .and_then(|c| c.fingerprint.as_ref())
          .map(|f| f.len())
          .unwrap_or(0)
      );
      self.save_process_info(&updated_profile)?;
      let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
        let _ = tm.rebuild_from_profiles(&self.profile_manager.list_profiles().unwrap_or_default());
      });
      log::info!(
        "Successfully saved profile with process info: {}",
        updated_profile.name
      );

      // Emit profiles-changed to trigger frontend to reload profiles from disk
      if let Err(e) = events::emit_empty("profiles-changed") {
        log::warn!("Warning: Failed to emit profiles-changed event: {e}");
      }

      log::info!(
        "Emitting profile events for successful Wayfern launch: {}",
        updated_profile.name
      );

      // Emit profile update event to frontend
      if let Err(e) = events::emit("profile-updated", &updated_profile) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }

      // Emit minimal running changed event to frontend
      #[derive(Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }

      let payload = RunningChangedPayload {
        id: updated_profile.id.to_string(),
        is_running: updated_profile.process_id.is_some(),
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      } else {
        log::info!(
          "Successfully emitted profile-running-changed event for Wayfern {}: running={}",
          updated_profile.name,
          payload.is_running
        );
      }

      return Ok(updated_profile);
    }

    Err(format!("Unsupported browser type: {}", profile.browser).into())
  }

  pub async fn open_url_in_existing_browser(
    &self,
    _app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: &str,
    _internal_proxy_settings: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle Wayfern profiles using WayfernManager
    if profile.browser == "wayfern" {
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path =
        crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      // Check if the process is running
      match self
        .wayfern_manager
        .find_wayfern_by_profile(&profile_path_str)
        .await
      {
        Some(_wayfern_process) => {
          log::info!(
            "Opening URL in existing Wayfern process for profile: {} (ID: {})",
            profile.name,
            profile.id
          );

          // Use CDP to open URL in a new tab
          self
            .wayfern_manager
            .open_url_in_tab(&profile_path_str, url)
            .await?;
          return Ok(());
        }
        None => {
          return Err("Wayfern browser is not running".into());
        }
      }
    }

    Err(format!("Unsupported browser type: {}", profile.browser).into())
  }

  pub async fn launch_browser_with_debugging(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Wayfern starts (and PID-reconciles) its own local proxy
    // inside `launch_browser_internal`, so we hand it None here rather than
    // staging a second, orphaned proxy worker.
    self
      .launch_browser_internal(
        app_handle,
        profile,
        url,
        None,
        remote_debugging_port,
        headless,
      )
      .await
  }

  pub async fn launch_or_open_url(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    internal_proxy_settings: Option<&ProxySettings>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    log::info!(
      "launch_or_open_url called for profile: {} (ID: {})",
      profile.name,
      profile.id
    );

    // Get the most up-to-date profile data
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles in launch_or_open_url: {e}"))?;
    let updated_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| profile.clone());

    log::info!(
      "Checking browser status for profile: {} (ID: {})",
      updated_profile.name,
      updated_profile.id
    );

    // Check if browser is already running
    let is_running = self
      .check_browser_status(app_handle.clone(), &updated_profile)
      .await
      .map_err(|e| format!("Failed to check browser status: {e}"))?;

    // Get the updated profile again after status check (PID might have been updated)
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles after status check: {e}"))?;
    let final_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| updated_profile.clone());

    log::info!(
      "Browser status check - Profile: {} (ID: {}), Running: {}, URL: {:?}, PID: {:?}",
      final_profile.name,
      final_profile.id,
      is_running,
      url,
      final_profile.process_id
    );

    if is_running && url.is_some() {
      // Browser is running and we have a URL to open
      if let Some(url_ref) = url.as_ref() {
        log::info!("Opening URL in existing browser: {url_ref}");

        match self
          .open_url_in_existing_browser(
            app_handle.clone(),
            &final_profile,
            url_ref,
            internal_proxy_settings,
          )
          .await
        {
          Ok(()) => {
            log::info!("Successfully opened URL in existing browser");
            Ok(final_profile)
          }
          Err(e) => {
            log::info!("Failed to open URL in existing browser: {e}");

            // Fall back to launching a new instance
            log::info!(
              "Falling back to new instance for browser: {}",
              final_profile.browser
            );
            // Fallback to launching a new instance for other browsers
            self
              .launch_browser_internal(
                app_handle.clone(),
                &final_profile,
                url,
                internal_proxy_settings,
                None,
                false,
              )
              .await
          }
        }
      } else {
        // This case shouldn't happen since we checked is_some() above, but handle it gracefully
        log::info!("URL was unexpectedly None, launching new browser instance");
        self
          .launch_browser(
            app_handle.clone(),
            &final_profile,
            url,
            internal_proxy_settings,
          )
          .await
      }
    } else {
      // Browser is not running or no URL provided, launch new instance
      if !is_running {
        log::info!("Launching new browser instance - browser not running");
      } else {
        log::info!("Launching new browser instance - no URL provided");
      }
      self
        .launch_browser_internal(
          app_handle.clone(),
          &final_profile,
          url,
          internal_proxy_settings,
          None,
          false,
        )
        .await
    }
  }

  fn save_process_info(
    &self,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use the regular save_profile method which handles the UUID structure
    self.profile_manager.save_profile(profile).map_err(|e| {
      let error_string = e.to_string();
      Box::new(std::io::Error::other(error_string)) as Box<dyn std::error::Error + Send + Sync>
    })
  }

  pub async fn check_browser_status(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    self
      .profile_manager
      .check_browser_status(app_handle, profile)
      .await
  }

  pub async fn kill_browser_process(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle Wayfern profiles using WayfernManager
    if profile.browser == "wayfern" {
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path =
        crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      log::info!(
        "Attempting to kill Wayfern process for profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      // Stop the proxy associated with this profile first
      let profile_id_str = profile.id.to_string();
      if let Err(e) = PROXY_MANAGER
        .stop_proxy_by_profile_id(app_handle.clone(), &profile_id_str)
        .await
      {
        log::warn!(
          "Warning: Failed to stop proxy for profile {}: {e}",
          profile_id_str
        );
      }

      let mut process_actually_stopped = false;
      match self
        .wayfern_manager
        .find_wayfern_by_profile(&profile_path_str)
        .await
      {
        Some(wayfern_process) => {
          log::info!(
            "Found Wayfern process: {} (PID: {:?})",
            wayfern_process.id,
            wayfern_process.processId
          );

          match self.wayfern_manager.stop_wayfern(&wayfern_process.id).await {
            Ok(_) => {
              if let Some(pid) = wayfern_process.processId {
                // Verify the process actually died by checking after a short delay
                use tokio::time::{sleep, Duration};
                sleep(Duration::from_millis(500)).await;

                use sysinfo::{Pid, System};
                let system = System::new_all();
                process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();

                if process_actually_stopped {
                  log::info!(
                    "Successfully stopped Wayfern process: {} (PID: {:?}) - verified process is dead",
                    wayfern_process.id,
                    pid
                  );
                } else {
                  log::warn!(
                    "Wayfern stop command returned success but process {} (PID: {:?}) is still running - forcing kill",
                    wayfern_process.id,
                    pid
                  );
                  // Force kill the process
                  #[cfg(target_os = "macos")]
                  {
                    use crate::platform_browser;
                    if let Err(e) = platform_browser::macos::kill_browser_process_impl(
                      pid,
                      Some(&profile_path_str),
                    )
                    .await
                    {
                      log::error!("Failed to force kill Wayfern process {}: {}", pid, e);
                    } else {
                      sleep(Duration::from_millis(500)).await;
                      let system = System::new_all();
                      process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                      if process_actually_stopped {
                        log::info!(
                          "Successfully force killed Wayfern process {} (PID: {:?})",
                          wayfern_process.id,
                          pid
                        );
                      }
                    }
                  }
                  #[cfg(target_os = "linux")]
                  {
                    use crate::platform_browser;
                    if let Err(e) = platform_browser::linux::kill_browser_process_impl(
                      pid,
                      Some(&profile_path_str),
                    )
                    .await
                    {
                      log::error!("Failed to force kill Wayfern process {}: {}", pid, e);
                    } else {
                      sleep(Duration::from_millis(500)).await;
                      let system = System::new_all();
                      process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                      if process_actually_stopped {
                        log::info!(
                          "Successfully force killed Wayfern process {} (PID: {:?})",
                          wayfern_process.id,
                          pid
                        );
                      }
                    }
                  }
                  #[cfg(target_os = "windows")]
                  {
                    use crate::platform_browser;
                    if let Err(e) = platform_browser::windows::kill_browser_process_impl(pid).await
                    {
                      log::error!("Failed to force kill Wayfern process {}: {}", pid, e);
                    } else {
                      sleep(Duration::from_millis(500)).await;
                      let system = System::new_all();
                      process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                      if process_actually_stopped {
                        log::info!(
                          "Successfully force killed Wayfern process {} (PID: {:?})",
                          wayfern_process.id,
                          pid
                        );
                      }
                    }
                  }
                }
              } else {
                process_actually_stopped = true;
              }
            }
            Err(e) => {
              log::error!(
                "Error stopping Wayfern process {}: {}",
                wayfern_process.id,
                e
              );
              // Try to force kill if we have a PID
              if let Some(pid) = wayfern_process.processId {
                log::info!(
                  "Attempting force kill after stop_wayfern error for PID: {}",
                  pid
                );
                #[cfg(target_os = "macos")]
                {
                  use crate::platform_browser;
                  if let Err(kill_err) =
                    platform_browser::macos::kill_browser_process_impl(pid, Some(&profile_path_str))
                      .await
                  {
                    log::error!("Failed to force kill Wayfern process {}: {}", pid, kill_err);
                  } else {
                    use tokio::time::{sleep, Duration};
                    sleep(Duration::from_millis(500)).await;
                    use sysinfo::{Pid, System};
                    let system = System::new_all();
                    process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                  }
                }
                #[cfg(target_os = "linux")]
                {
                  use crate::platform_browser;
                  if let Err(kill_err) =
                    platform_browser::linux::kill_browser_process_impl(pid, Some(&profile_path_str))
                      .await
                  {
                    log::error!("Failed to force kill Wayfern process {}: {}", pid, kill_err);
                  } else {
                    use tokio::time::{sleep, Duration};
                    sleep(Duration::from_millis(500)).await;
                    use sysinfo::{Pid, System};
                    let system = System::new_all();
                    process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                  }
                }
                #[cfg(target_os = "windows")]
                {
                  use crate::platform_browser;
                  if let Err(kill_err) =
                    platform_browser::windows::kill_browser_process_impl(pid).await
                  {
                    log::error!("Failed to force kill Wayfern process {}: {}", pid, kill_err);
                  } else {
                    use tokio::time::{sleep, Duration};
                    sleep(Duration::from_millis(500)).await;
                    use sysinfo::{Pid, System};
                    let system = System::new_all();
                    process_actually_stopped = system.process(Pid::from(pid as usize)).is_none();
                  }
                }
              }
            }
          }
        }
        None => {
          log::info!(
            "No running Wayfern process found for profile: {} (ID: {})",
            profile.name,
            profile.id
          );
          process_actually_stopped = true;
        }
      }

      // If process wasn't confirmed stopped, return an error
      if !process_actually_stopped {
        log::error!(
          "Failed to stop Wayfern process for profile: {} (ID: {}) - process may still be running",
          profile.name,
          profile.id
        );
        return Err(
          format!(
            "Failed to stop Wayfern process for profile {} - process may still be running",
            profile.name
          )
          .into(),
        );
      }

      // Clear the process ID from the profile and save immediately so that
      // subsequent calls to update_profile_version (which re-reads from disk)
      // see the cleared process_id.
      let mut updated_profile = profile.clone();
      updated_profile.process_id = None;
      self
        .save_process_info(&updated_profile)
        .map_err(|e| format!("Failed to update profile: {e}"))?;

      // Check for pending updates and apply them
      if let Ok(Some(pending_update)) = self
        .auto_updater
        .get_pending_update(&profile.browser, &profile.version)
      {
        log::info!(
          "Found pending update for Wayfern profile {}: {} -> {}",
          profile.name,
          profile.version,
          pending_update.new_version
        );

        match self.profile_manager.update_profile_version(
          &app_handle,
          &profile.id.to_string(),
          &pending_update.new_version,
        ) {
          Ok(updated_profile_after_update) => {
            log::info!(
              "Successfully updated Wayfern profile {} from version {} to {}",
              profile.name,
              profile.version,
              pending_update.new_version
            );
            updated_profile = updated_profile_after_update;

            if let Err(e) = self
              .auto_updater
              .dismiss_update_notification(&pending_update.id)
            {
              log::warn!("Warning: Failed to dismiss pending update notification: {e}");
            }
          }
          Err(e) => {
            log::error!(
              "Failed to apply pending update for Wayfern profile {}: {}",
              profile.name,
              e
            );
          }
        }
      }

      // If no pending update was applied, check if a newer installed version exists
      if updated_profile.version == profile.version {
        if let Some(p) = self
          .auto_updater
          .update_profile_to_latest_installed(&app_handle, &updated_profile)
        {
          updated_profile = p;
        }
      }

      log::info!(
        "Emitting profile events for successful Wayfern kill: {}",
        updated_profile.name
      );

      // Emit profile update event to frontend
      if let Err(e) = events::emit("profile-updated", &updated_profile) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }

      // Emit minimal running changed event
      #[derive(Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      let payload = RunningChangedPayload {
        id: updated_profile.id.to_string(),
        is_running: false,
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      } else {
        log::info!(
          "Successfully emitted profile-running-changed event for Wayfern {}: running={}",
          updated_profile.name,
          payload.is_running
        );
      }

      if profile.password_protected {
        // Await the re-encryption so the queued sync (released later by
        // `mark_profile_stopped` in `kill_browser`) sees fresh ciphertext on
        // disk instead of the previous snapshot.
        crate::profile::password::complete_after_quit_and_wait(profile).await;
      } else if profile.ephemeral {
        crate::ephemeral_dirs::remove_ephemeral_dir(&profile.id.to_string());
      }

      log::info!(
        "Wayfern process cleanup completed for profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      // Consolidate browser versions after stopping a browser
      if let Ok(consolidated) = self
        .downloaded_browsers_registry
        .consolidate_browser_versions(&app_handle)
      {
        if !consolidated.is_empty() {
          log::info!("Post-stop version consolidation results:");
          for action in &consolidated {
            log::info!("  {action}");
          }
        }
      }

      return Ok(());
    }

    Err(
      format!(
        "Unsupported browser '{}' for profile '{}' — only Wayfern is supported",
        profile.browser, profile.name
      )
      .into(),
    )
  }

  pub async fn open_url_with_profile(
    &self,
    app_handle: tauri::AppHandle,
    profile_id: String,
    url: String,
  ) -> Result<(), String> {
    // Get the profile by name
    let profiles = self
      .profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    let profile = profiles
      .into_iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| format!("Profile '{profile_id}' not found"))?;

    if profile.is_cross_os() {
      return Err(format!(
        "Cannot open URL with profile '{}': this profile was created on {} and cannot be used on a different operating system",
        profile.name,
        profile.host_os.as_deref().unwrap_or("another OS"),
      ));
    }

    log::info!("Opening URL '{url}' with profile '{profile_id}'");

    // Use launch_or_open_url which handles both launching new instances and opening in existing ones
    self
      .launch_or_open_url(app_handle, &profile, Some(url.clone()), None)
      .await
      .map_err(|e| {
        log::info!("Failed to open URL with profile '{profile_id}': {e}");
        format!("Failed to open URL with profile: {e}")
      })?;

    log::info!("Successfully opened URL '{url}' with profile '{profile_id}'");
    Ok(())
  }
}

#[tauri::command]
pub async fn launch_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
) -> Result<BrowserProfile, String> {
  launch_browser_profile_impl(app_handle, profile, url, None, false, false).await
}

pub async fn launch_browser_profile_impl(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
  remote_debugging_port: Option<u16>,
  headless: bool,
  force_new: bool,
) -> Result<BrowserProfile, String> {
  log::info!(
    "Launch request received for profile: {} (ID: {})",
    profile.name,
    profile.id
  );

  if profile.is_cross_os() {
    return Err(format!(
      "Cannot launch profile '{}': this profile was created on {} and cannot be launched on a different operating system",
      profile.name,
      profile.host_os.as_deref().unwrap_or("another OS"),
    ));
  }

  // Team lock check: if profile is sync-enabled and user is on a team, acquire lock
  crate::team_lock::acquire_team_lock_if_needed(&profile).await?;

  // Notify sync scheduler that profile is now running and queue sync for when it stops
  if let Some(scheduler) = crate::sync::get_global_scheduler() {
    let pid = profile.id.to_string();
    scheduler.mark_profile_running(&pid).await;
    if profile.is_sync_enabled() {
      scheduler.queue_profile_sync(pid).await;
    }
  }

  let browser_runner = BrowserRunner::instance();

  // Resolve the most up-to-date profile from disk by ID to avoid using stale proxy_id/browser state
  let profile_for_launch = match browser_runner
    .profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))
  {
    Ok(profiles) => profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| profile.clone()),
    Err(e) => {
      return Err(e);
    }
  };

  log::info!(
    "Resolved profile for launch: {} (ID: {})",
    profile_for_launch.name,
    profile_for_launch.id
  );

  log::info!(
    "Starting browser launch for profile: {} (ID: {})",
    profile_for_launch.name,
    profile_for_launch.id
  );

  // Launch browser or open URL in existing instance. Wayfern starts its
  // own local proxy inside `launch_browser_internal`; other browser types
  // are rejected there, so no proxy needs to be staged here.
  //
  // `force_new` callers (API/MCP) always start a fresh instance with the
  // requested debug port and headless mode, bypassing the "open URL in the
  // existing window" path which would otherwise ignore both.
  let launch_result = if force_new {
    browser_runner
      .launch_browser_with_debugging(
        app_handle.clone(),
        &profile_for_launch,
        url,
        remote_debugging_port,
        headless,
      )
      .await
  } else {
    browser_runner
      .launch_or_open_url(app_handle.clone(), &profile_for_launch, url, None)
      .await
  };
  let updated_profile = launch_result.map_err(|e| {
    log::info!("Browser launch failed for profile: {}, error: {}", profile_for_launch.name, e);

    // Emit a failure event to clear loading states in the frontend
    #[derive(serde::Serialize)]
    struct RunningChangedPayload {
      id: String,
      is_running: bool,
    }
    let payload = RunningChangedPayload {
      id: profile_for_launch.id.to_string(),
      is_running: false,
    };

    if let Err(e) = events::emit("profile-running-changed", &payload) {
      log::warn!("Warning: Failed to emit profile running changed event: {e}");
    }

    // Check if this is an architecture compatibility issue
    if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
      if io_error.kind() == std::io::ErrorKind::Other && io_error.to_string().contains("Exec format error") {
        return format!("Failed to launch browser: Executable format error. This browser version is not compatible with your system architecture ({}). Please try a different browser or version that supports your platform.", std::env::consts::ARCH);
      }
    }
    format!("Failed to launch browser or open URL: {e}")
  })?;

  log::info!(
    "Browser launch completed for profile: {} (ID: {})",
    updated_profile.name,
    updated_profile.id
  );

  // Now update the proxy with the correct PID if we have one
  if let Some(actual_pid) = updated_profile.process_id {
    // Update the proxy manager with the correct PID (we always started with temp pid 1)
    let _ = PROXY_MANAGER.update_proxy_pid(1u32, actual_pid);
  }

  Ok(updated_profile)
}

#[tauri::command]
pub fn check_browser_exists(browser_str: String, version: String) -> bool {
  // This is an alias for is_browser_downloaded to provide clearer semantics for auto-updates
  let runner = BrowserRunner::instance();
  runner
    .downloaded_browsers_registry
    .is_browser_downloaded(&browser_str, &version)
}

#[tauri::command]
pub async fn kill_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
) -> Result<(), String> {
  log::info!(
    "Kill request received for profile: {} (ID: {})",
    profile.name,
    profile.id
  );

  let browser_runner = BrowserRunner::instance();

  match browser_runner
    .kill_browser_process(app_handle.clone(), &profile)
    .await
  {
    Ok(()) => {
      log::info!(
        "Successfully killed browser profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      // Release team lock if applicable
      crate::team_lock::release_team_lock_if_needed(&profile).await;

      // Notify sync scheduler that profile stopped (sync was queued at launch)
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        scheduler
          .mark_profile_stopped(&profile.id.to_string())
          .await;
      }

      // Auto-update non-running profiles and cleanup unused binaries
      let browser_for_update = profile.browser.clone();
      let app_handle_for_update = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        let registry = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
        let mut versions = registry.get_downloaded_versions(&browser_for_update);
        if !versions.is_empty() {
          versions.sort_by(|a, b| crate::api_client::compare_versions(b, a));
          let latest_version = &versions[0];

          let auto_updater = crate::auto_updater::AutoUpdater::instance();
          match auto_updater
            .auto_update_profile_versions(
              &app_handle_for_update,
              &browser_for_update,
              latest_version,
            )
            .await
          {
            Ok(updated) => {
              if !updated.is_empty() {
                log::info!(
                  "Auto-updated {} profiles after stop: {:?}",
                  updated.len(),
                  updated
                );
              }
            }
            Err(e) => {
              log::error!("Failed to auto-update profile versions after stop: {e}");
            }
          }
        }

        match registry.cleanup_unused_binaries() {
          Ok(cleaned) => {
            if !cleaned.is_empty() {
              log::info!("Cleaned up unused binaries after stop: {:?}", cleaned);
            }
          }
          Err(e) => {
            log::error!("Failed to cleanup unused binaries after stop: {e}");
          }
        }
      });

      Ok(())
    }
    Err(e) => {
      log::info!("Failed to kill browser profile {}: {}", profile.name, e);

      // Emit a failure event to clear loading states in the frontend
      #[derive(serde::Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      // On kill failure, we assume the process is still running
      let payload = RunningChangedPayload {
        id: profile.id.to_string(),
        is_running: true,
      };

      if let Err(e) = events::emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      }

      Err(format!("Failed to kill browser: {e}"))
    }
  }
}

#[tauri::command]
pub async fn open_url_with_profile(
  app_handle: tauri::AppHandle,
  profile_id: String,
  url: String,
) -> Result<(), String> {
  let browser_runner = BrowserRunner::instance();
  browser_runner
    .open_url_with_profile(app_handle, profile_id, url)
    .await
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_RUNNER: BrowserRunner = BrowserRunner::new();
}

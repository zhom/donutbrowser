use crate::browser::{create_browser, BrowserType, ProxySettings};
use crate::camoufox_manager::{CamoufoxConfig, CamoufoxManager};
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::platform_browser;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::PROXY_MANAGER;
use directories::BaseDirs;
use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::System;
use tauri::Emitter;
pub struct BrowserRunner {
  base_dirs: BaseDirs,
  pub profile_manager: &'static ProfileManager,
  pub downloaded_browsers_registry: &'static DownloadedBrowsersRegistry,
  auto_updater: &'static crate::auto_updater::AutoUpdater,
  camoufox_manager: &'static CamoufoxManager,
}

impl BrowserRunner {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      profile_manager: ProfileManager::instance(),
      downloaded_browsers_registry: DownloadedBrowsersRegistry::instance(),
      auto_updater: crate::auto_updater::AutoUpdater::instance(),
      camoufox_manager: CamoufoxManager::instance(),
    }
  }

  pub fn instance() -> &'static BrowserRunner {
    &BROWSER_RUNNER
  }

  // Helper function to check if a process matches TOR/Mullvad browser
  fn is_tor_or_mullvad_browser(
    &self,
    exe_name: &str,
    cmd: &[std::ffi::OsString],
    browser_type: &str,
  ) -> bool {
    #[cfg(target_os = "macos")]
    return platform_browser::macos::is_tor_or_mullvad_browser(exe_name, cmd, browser_type);

    #[cfg(target_os = "windows")]
    return platform_browser::windows::is_tor_or_mullvad_browser(exe_name, cmd, browser_type);

    #[cfg(target_os = "linux")]
    return platform_browser::linux::is_tor_or_mullvad_browser(exe_name, cmd, browser_type);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
      let _ = (exe_name, cmd, browser_type);
      false
    }
  }

  pub fn get_binaries_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("binaries");
    path
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
    local_proxy_settings: Option<&ProxySettings>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Check if browser is disabled due to ongoing update
    if self.auto_updater.is_browser_disabled(&profile.browser)? {
      return Err(
        format!(
          "{} is currently being updated. Please wait for the update to complete.",
          profile.browser
        )
        .into(),
      );
    }

    // Handle camoufox profiles using nodecar launcher
    if profile.browser == "camoufox" {
      // Get or create camoufox config
      let mut camoufox_config = profile.camoufox_config.clone().unwrap_or_else(|| {
        log::info!(
          "No camoufox config found for profile {}, using default",
          profile.name
        );
        CamoufoxConfig::default()
      });

      // Always start a local proxy for Camoufox (for traffic monitoring and geoip support)
      let upstream_proxy = profile
        .proxy_id
        .as_ref()
        .and_then(|id| PROXY_MANAGER.get_proxy_settings_by_id(id));

      log::info!(
        "Starting local proxy for Camoufox profile: {} (upstream: {})",
        profile.name,
        upstream_proxy
          .as_ref()
          .map(|p| format!("{}:{}", p.host, p.port))
          .unwrap_or_else(|| "DIRECT".to_string())
      );

      // Start the proxy and get local proxy settings
      // If proxy startup fails, DO NOT launch Camoufox - it requires local proxy
      let local_proxy = PROXY_MANAGER
        .start_proxy(
          app_handle.clone(),
          upstream_proxy.as_ref(),
          0, // Use 0 as temporary PID, will be updated later
          Some(&profile.name),
        )
        .await
        .map_err(|e| {
          let error_msg = format!("Failed to start local proxy for Camoufox: {e}");
          log::error!("{}", error_msg);
          error_msg
        })?;

      // Format proxy URL for camoufox - always use HTTP for the local proxy
      let proxy_url = format!("http://{}:{}", local_proxy.host, local_proxy.port);

      // Set proxy in camoufox config
      camoufox_config.proxy = Some(proxy_url);

      // Ensure geoip is always enabled for proper geolocation spoofing
      if camoufox_config.geoip.is_none() {
        camoufox_config.geoip = Some(serde_json::Value::Bool(true));
      }

      log::info!(
        "Configured local proxy for Camoufox: {:?}, geoip: {:?}",
        camoufox_config.proxy,
        camoufox_config.geoip
      );

      // Check if we need to generate a new fingerprint on every launch
      let mut updated_profile = profile.clone();
      if camoufox_config.randomize_fingerprint_on_launch == Some(true) {
        log::info!(
          "Generating random fingerprint for Camoufox profile: {}",
          profile.name
        );

        // Create a config copy without the existing fingerprint to force generation of a new one
        let mut config_for_generation = camoufox_config.clone();
        config_for_generation.fingerprint = None;

        // Generate a new fingerprint
        let new_fingerprint = self
          .camoufox_manager
          .generate_fingerprint_config(&app_handle, profile, &config_for_generation)
          .await
          .map_err(|e| format!("Failed to generate random fingerprint: {e}"))?;

        log::info!(
          "New fingerprint generated, length: {} chars",
          new_fingerprint.len()
        );

        // Update the config with the new fingerprint for launching
        camoufox_config.fingerprint = Some(new_fingerprint.clone());

        // Save the updated fingerprint to the profile so it persists
        // We need to preserve all existing config fields and only update the fingerprint
        let mut updated_camoufox_config =
          updated_profile.camoufox_config.clone().unwrap_or_default();
        updated_camoufox_config.fingerprint = Some(new_fingerprint);
        // Preserve the randomize flag so it persists across launches
        updated_camoufox_config.randomize_fingerprint_on_launch = Some(true);
        // Preserve the OS setting so it's used for future fingerprint generation
        if camoufox_config.os.is_some() {
          updated_camoufox_config.os = camoufox_config.os.clone();
        }
        updated_profile.camoufox_config = Some(updated_camoufox_config.clone());

        log::info!(
          "Updated profile camoufox_config with new fingerprint for profile: {}, fingerprint length: {}",
          profile.name,
          updated_camoufox_config.fingerprint.as_ref().map(|f| f.len()).unwrap_or(0)
        );
      }

      // Use the nodecar camoufox launcher
      log::info!(
        "Launching Camoufox via nodecar for profile: {}",
        profile.name
      );
      let camoufox_result = self
        .camoufox_manager
        .launch_camoufox_profile_nodecar(
          app_handle.clone(),
          updated_profile.clone(),
          camoufox_config,
          url,
        )
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to launch camoufox via nodecar: {e}").into()
        })?;

      // For server-based Camoufox, we use the process_id
      let process_id = camoufox_result.processId.unwrap_or(0);
      log::info!("Camoufox launched successfully with PID: {process_id}");

      // Update profile with the process info from camoufox result
      updated_profile.process_id = Some(process_id);
      updated_profile.last_launch = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());

      // Update the proxy manager with the correct PID
      if let Err(e) = PROXY_MANAGER.update_proxy_pid(0, process_id) {
        log::warn!("Warning: Failed to update proxy PID mapping: {e}");
      } else {
        log::info!("Updated proxy PID mapping from temp (0) to actual PID: {process_id}");
      }

      // Save the updated profile (includes new fingerprint if randomize is enabled)
      log::info!(
        "Saving profile {} with camoufox_config fingerprint length: {}",
        updated_profile.name,
        updated_profile
          .camoufox_config
          .as_ref()
          .and_then(|c| c.fingerprint.as_ref())
          .map(|f| f.len())
          .unwrap_or(0)
      );
      self.save_process_info(&updated_profile)?;
      // Ensure tag suggestions include any tags from this profile
      let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
        let _ = tm.rebuild_from_profiles(&self.profile_manager.list_profiles().unwrap_or_default());
      });
      log::info!(
        "Successfully saved profile with process info: {}",
        updated_profile.name
      );

      // Emit profiles-changed to trigger frontend to reload profiles from disk
      // This ensures the UI displays the newly generated fingerprint
      if let Err(e) = app_handle.emit("profiles-changed", ()) {
        log::warn!("Warning: Failed to emit profiles-changed event: {e}");
      }

      log::info!(
        "Emitting profile events for successful Camoufox launch: {}",
        updated_profile.name
      );

      // Emit profile update event to frontend
      if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }

      // Emit minimal running changed event to frontend with a small delay
      #[derive(Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }

      let payload = RunningChangedPayload {
        id: updated_profile.id.to_string(),
        is_running: updated_profile.process_id.is_some(),
      };

      if let Err(e) = app_handle.emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      } else {
        log::info!(
          "Successfully emitted profile-running-changed event for Camoufox {}: running={}",
          updated_profile.name,
          payload.is_running
        );
      }

      return Ok(updated_profile);
    }

    // Create browser instance
    let browser_type = BrowserType::from_str(&profile.browser)
      .map_err(|_| format!("Invalid browser type: {}", profile.browser))?;
    let browser = create_browser(browser_type.clone());

    // Get executable path using common helper
    let executable_path = self
      .get_browser_executable_path(profile)
      .expect("Failed to get executable path");

    log::info!("Executable path: {executable_path:?}");

    // Prepare the executable (set permissions, etc.)
    if let Err(e) = browser.prepare_executable(&executable_path) {
      log::warn!("Warning: Failed to prepare executable: {e}");
      // Continue anyway, the error might not be critical
    }

    // Get stored proxy settings for later use (removed as we handle this in proxy startup)
    let _stored_proxy_settings = profile
      .proxy_id
      .as_ref()
      .and_then(|id| PROXY_MANAGER.get_proxy_settings_by_id(id));

    // Use provided local proxy for Chromium-based browsers launch arguments
    let proxy_for_launch_args: Option<&ProxySettings> = local_proxy_settings;

    // Get profile data path and launch arguments
    let profiles_dir = self.profile_manager.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    let browser_args = browser
      .create_launch_args(
        &profile_data_path.to_string_lossy(),
        proxy_for_launch_args,
        url,
        remote_debugging_port,
        headless,
      )
      .expect("Failed to create launch arguments");

    // Launch browser using platform-specific method
    let child = {
      #[cfg(target_os = "macos")]
      {
        platform_browser::macos::launch_browser_process(&executable_path, &browser_args).await?
      }

      #[cfg(target_os = "windows")]
      {
        platform_browser::windows::launch_browser_process(&executable_path, &browser_args).await?
      }

      #[cfg(target_os = "linux")]
      {
        platform_browser::linux::launch_browser_process(&executable_path, &browser_args).await?
      }

      #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
      {
        return Err("Unsupported platform for browser launching".into());
      }
    };

    let launcher_pid = child.id();

    log::info!(
      "Launched browser with launcher PID: {} for profile: {} (ID: {})",
      launcher_pid,
      profile.name,
      profile.id
    );

    // For TOR and Mullvad browsers, we need to find the actual browser process
    // because they use launcher scripts that spawn the real browser process
    let mut actual_pid = launcher_pid;

    if matches!(
      browser_type,
      BrowserType::TorBrowser | BrowserType::MullvadBrowser
    ) {
      // Wait a moment for the actual browser process to start
      tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;

      // Find the actual browser process
      let system = System::new_all();
      for (pid, process) in system.processes() {
        let process_name = process.name().to_str().unwrap_or("");
        let process_cmd = process.cmd();
        let pid_u32 = pid.as_u32();

        // Skip if this is the launcher process itself
        if pid_u32 == launcher_pid {
          continue;
        }

        if self.is_tor_or_mullvad_browser(process_name, process_cmd, &profile.browser) {
          log::info!(
            "Found actual {} browser process: PID {} ({})",
            profile.browser,
            pid_u32,
            process_name
          );
          actual_pid = pid_u32;
          break;
        }
      }
    }

    // On macOS, when launching via `open -a`, the child PID is the `open` helper.
    // Resolve and store the actual browser PID for all browser types.
    #[cfg(target_os = "macos")]
    {
      // Give the browser a moment to start
      tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

      let system = System::new_all();
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path = profile.get_profile_data_path(&profiles_dir);
      let profile_data_path_str = profile_data_path.to_string_lossy();

      for (pid, process) in system.processes() {
        let cmd = process.cmd();
        if cmd.is_empty() {
          continue;
        }

        // Determine if this process matches the intended browser type
        let exe_name_lower = process.name().to_string_lossy().to_lowercase();
        let is_correct_browser = match profile.browser.as_str() {
          "firefox" => {
            exe_name_lower.contains("firefox")
              && !exe_name_lower.contains("developer")
              && !exe_name_lower.contains("tor")
              && !exe_name_lower.contains("mullvad")
              && !exe_name_lower.contains("camoufox")
          }
          "firefox-developer" => {
            // More flexible detection for Firefox Developer Edition
            (exe_name_lower.contains("firefox") && exe_name_lower.contains("developer"))
              || (exe_name_lower.contains("firefox")
                && cmd.iter().any(|arg| {
                  let arg_str = arg.to_str().unwrap_or("");
                  arg_str.contains("Developer")
                    || arg_str.contains("developer")
                    || arg_str.contains("FirefoxDeveloperEdition")
                    || arg_str.contains("firefox-developer")
                }))
              || exe_name_lower == "firefox" // Firefox Developer might just show as "firefox"
          }
          "mullvad-browser" => {
            self.is_tor_or_mullvad_browser(&exe_name_lower, cmd, "mullvad-browser")
          }
          "tor-browser" => self.is_tor_or_mullvad_browser(&exe_name_lower, cmd, "tor-browser"),
          "zen" => exe_name_lower.contains("zen"),
          "chromium" => exe_name_lower.contains("chromium") || exe_name_lower.contains("chrome"),
          "brave" => exe_name_lower.contains("brave") || exe_name_lower.contains("Brave"),
          _ => false,
        };

        if !is_correct_browser {
          continue;
        }

        // Check for profile path match
        let profile_path_match = if matches!(
          profile.browser.as_str(),
          "firefox" | "firefox-developer" | "tor-browser" | "mullvad-browser" | "zen"
        ) {
          // Firefox-based browsers: look for -profile argument followed by path
          let mut found_profile_arg = false;
          for (i, arg) in cmd.iter().enumerate() {
            if let Some(arg_str) = arg.to_str() {
              if arg_str == "-profile" && i + 1 < cmd.len() {
                if let Some(next_arg) = cmd.get(i + 1).and_then(|a| a.to_str()) {
                  if next_arg == profile_data_path_str {
                    found_profile_arg = true;
                    break;
                  }
                }
              }
              // Also check for combined -profile=path format
              if arg_str == format!("-profile={profile_data_path_str}") {
                found_profile_arg = true;
                break;
              }
              // Check if the argument is the profile path directly
              if arg_str == profile_data_path_str {
                found_profile_arg = true;
                break;
              }
            }
          }
          found_profile_arg
        } else {
          // Chromium-based browsers: look for --user-data-dir argument
          cmd.iter().any(|s| {
            if let Some(arg) = s.to_str() {
              arg == format!("--user-data-dir={profile_data_path_str}")
                || arg == profile_data_path_str
            } else {
              false
            }
          })
        };

        if profile_path_match {
          let pid_u32 = pid.as_u32();
          if pid_u32 != launcher_pid {
            actual_pid = pid_u32;
            break;
          }
        }
      }
    }

    // Update profile with process info and save
    let mut updated_profile = profile.clone();
    updated_profile.process_id = Some(actual_pid);
    updated_profile.last_launch = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());

    self.save_process_info(&updated_profile)?;
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.profile_manager.list_profiles().unwrap_or_default());
    });

    // Apply proxy settings if needed (for Firefox-based browsers)
    if profile.proxy_id.is_some()
      && matches!(
        browser_type,
        BrowserType::Firefox
          | BrowserType::FirefoxDeveloper
          | BrowserType::Zen
          | BrowserType::TorBrowser
          | BrowserType::MullvadBrowser
      )
    {
      // Proxy settings for Firefox-based browsers are applied via user.js file
      // which is already handled in the profile creation process
    }

    log::info!(
      "Emitting profile events for successful launch: {} (ID: {})",
      updated_profile.name,
      updated_profile.id
    );

    // Emit profile update event to frontend
    if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
      log::warn!("Warning: Failed to emit profile update event: {e}");
    }

    // Emit minimal running changed event to frontend with a small delay to ensure UI consistency
    #[derive(Serialize)]
    struct RunningChangedPayload {
      id: String,
      is_running: bool,
    }
    let payload = RunningChangedPayload {
      id: updated_profile.id.to_string(),
      is_running: updated_profile.process_id.is_some(),
    };

    if let Err(e) = app_handle.emit("profile-running-changed", &payload) {
      log::warn!("Warning: Failed to emit profile running changed event: {e}");
    } else {
      log::info!(
        "Successfully emitted profile-running-changed event for {}: running={}",
        updated_profile.name,
        payload.is_running
      );
    }

    Ok(updated_profile)
  }

  pub async fn open_url_in_existing_browser(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: &str,
    _internal_proxy_settings: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle camoufox profiles using nodecar launcher
    if profile.browser == "camoufox" {
      // Get the profile path based on the UUID
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path = profile.get_profile_data_path(&profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      // Check if the process is running
      match self
        .camoufox_manager
        .find_camoufox_by_profile(&profile_path_str)
        .await
      {
        Ok(Some(_camoufox_process)) => {
          log::info!(
            "Opening URL in existing Camoufox process for profile: {} (ID: {})",
            profile.name,
            profile.id
          );

          // For Camoufox, we need to launch a new instance with the URL since it doesn't support remote commands
          // This is a limitation of Camoufox's architecture
          return Err("Camoufox doesn't support opening URLs in existing instances. Please close the browser and launch again with the URL.".into());
        }
        Ok(None) => {
          return Err("Camoufox browser is not running".into());
        }
        Err(e) => {
          return Err(format!("Error checking Camoufox process: {e}").into());
        }
      }
    }

    // Use the comprehensive browser status check for non-camoufox browsers
    let is_running = self
      .check_browser_status(app_handle.clone(), profile)
      .await?;

    if !is_running {
      return Err("Browser is not running".into());
    }

    // Get the updated profile with current PID
    let profiles = self
      .profile_manager
      .list_profiles()
      .expect("Failed to list profiles");
    let updated_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| profile.clone());

    // Ensure we have a valid process ID
    if updated_profile.process_id.is_none() {
      return Err("No valid process ID found for the browser".into());
    }

    let browser_type = BrowserType::from_str(&updated_profile.browser)
      .map_err(|_| format!("Invalid browser type: {}", updated_profile.browser))?;

    // Get browser directory for all platforms - path structure: binaries/<browser>/<version>/
    let mut browser_dir = self.get_binaries_dir();
    browser_dir.push(&updated_profile.browser);
    browser_dir.push(&updated_profile.version);

    match browser_type {
      BrowserType::Firefox | BrowserType::FirefoxDeveloper | BrowserType::Zen => {
        #[cfg(target_os = "macos")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::macos::open_url_in_existing_browser_firefox_like(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "windows")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::windows::open_url_in_existing_browser_firefox_like(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "linux")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::linux::open_url_in_existing_browser_firefox_like(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        return Err("Unsupported platform".into());
      }
      BrowserType::MullvadBrowser | BrowserType::TorBrowser => {
        #[cfg(target_os = "macos")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::macos::open_url_in_existing_browser_tor_mullvad(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "windows")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::windows::open_url_in_existing_browser_tor_mullvad(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "linux")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::linux::open_url_in_existing_browser_tor_mullvad(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        return Err("Unsupported platform".into());
      }
      BrowserType::Chromium | BrowserType::Brave => {
        #[cfg(target_os = "macos")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::macos::open_url_in_existing_browser_chromium(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "windows")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::windows::open_url_in_existing_browser_chromium(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(target_os = "linux")]
        {
          let profiles_dir = self.profile_manager.get_profiles_dir();
          return platform_browser::linux::open_url_in_existing_browser_chromium(
            &updated_profile,
            url,
            browser_type,
            &browser_dir,
            &profiles_dir,
          )
          .await;
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        return Err("Unsupported platform".into());
      }
      BrowserType::Camoufox => {
        // This should never be reached due to the early return above, but handle it just in case
        Err("Camoufox URL opening should be handled in the early return above".into())
      }
    }
  }

  pub async fn launch_browser_with_debugging(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    remote_debugging_port: Option<u16>,
    headless: bool,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Always start a local proxy for API launches
    // Determine upstream proxy if configured; otherwise use DIRECT
    let upstream_proxy = profile
      .proxy_id
      .as_ref()
      .and_then(|id| PROXY_MANAGER.get_proxy_settings_by_id(id));

    // Use a temporary PID (1) to start the proxy, we'll update it after browser launch
    let temp_pid = 1u32;

    // Start local proxy - if this fails, DO NOT launch browser
    let internal_proxy = PROXY_MANAGER
      .start_proxy(
        app_handle.clone(),
        upstream_proxy.as_ref(),
        temp_pid,
        Some(&profile.name),
      )
      .await
      .map_err(|e| {
        let error_msg = format!("Failed to start local proxy: {e}");
        log::error!("{}", error_msg);
        error_msg
      })?;

    let internal_proxy_settings = Some(internal_proxy.clone());

    // Configure Firefox profiles to use local proxy
    {
      // For Firefox-based browsers, apply PAC/user.js to point to the local proxy
      if matches!(
        profile.browser.as_str(),
        "firefox" | "firefox-developer" | "zen" | "tor-browser" | "mullvad-browser"
      ) {
        let profiles_dir = self.profile_manager.get_profiles_dir();
        let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");

        // Provide a dummy upstream (ignored when internal proxy is provided)
        let dummy_upstream = ProxySettings {
          proxy_type: "http".to_string(),
          host: "127.0.0.1".to_string(),
          port: internal_proxy.port,
          username: None,
          password: None,
        };

        self
          .profile_manager
          .apply_proxy_settings_to_profile(&profile_path, &dummy_upstream, Some(&internal_proxy))
          .map_err(|e| format!("Failed to update profile proxy: {e}"))?;
      }
    }

    let result = self
      .launch_browser_internal(
        app_handle.clone(),
        profile,
        url,
        internal_proxy_settings.as_ref(),
        remote_debugging_port,
        headless,
      )
      .await;

    // Update proxy with correct PID if launch succeeded
    if let Ok(ref updated_profile) = result {
      if let Some(actual_pid) = updated_profile.process_id {
        let _ = PROXY_MANAGER.update_proxy_pid(temp_pid, actual_pid);
      }
    }

    result
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

        // For TOR/Mullvad browsers, add extra verification
        if matches!(
          final_profile.browser.as_str(),
          "tor-browser" | "mullvad-browser"
        ) {
          log::info!("TOR/Mullvad browser detected - ensuring we have correct PID");
          if final_profile.process_id.is_none() {
            log::info!(
              "ERROR: No PID found for running TOR/Mullvad browser - this should not happen"
            );
            return Err("No PID found for running browser".into());
          }
        }
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

            // For Mullvad and Tor browsers, don't fall back to new instance since they use -no-remote
            // and can't have multiple instances with the same profile
            match final_profile.browser.as_str() {
              "mullvad-browser" | "tor-browser" => {
                Err(format!("Failed to open URL in existing {} browser. Cannot launch new instance due to profile conflict: {}", final_profile.browser, e).into())
              }
              _ => {
                log::info!("Falling back to new instance for browser: {}", final_profile.browser);
                // Fallback to launching a new instance for other browsers
                self.launch_browser_internal(app_handle.clone(), &final_profile, url, internal_proxy_settings, None, false).await
              }
            }
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
    // Handle camoufox profiles using nodecar launcher
    if profile.browser == "camoufox" {
      // Search by profile path to find the running Camoufox instance
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path = profile.get_profile_data_path(&profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      log::info!(
        "Attempting to kill Camoufox process for profile: {} (ID: {})",
        profile.name,
        profile.id
      );

      match self
        .camoufox_manager
        .find_camoufox_by_profile(&profile_path_str)
        .await
      {
        Ok(Some(camoufox_process)) => {
          log::info!(
            "Found Camoufox process: {} (PID: {:?})",
            camoufox_process.id,
            camoufox_process.processId
          );

          match self
            .camoufox_manager
            .stop_camoufox(&app_handle, &camoufox_process.id)
            .await
          {
            Ok(stopped) => {
              if stopped {
                log::info!(
                  "Successfully stopped Camoufox process: {} (PID: {:?})",
                  camoufox_process.id,
                  camoufox_process.processId
                );
              } else {
                log::info!(
                  "Failed to stop Camoufox process: {} (PID: {:?})",
                  camoufox_process.id,
                  camoufox_process.processId
                );
              }
            }
            Err(e) => {
              log::info!(
                "Error stopping Camoufox process {}: {}",
                camoufox_process.id,
                e
              );
            }
          }
        }
        Ok(None) => {
          log::info!(
            "No running Camoufox process found for profile: {} (ID: {})",
            profile.name,
            profile.id
          );
        }
        Err(e) => {
          log::info!(
            "Error finding Camoufox process for profile {}: {}",
            profile.name,
            e
          );
        }
      }

      // Clear the process ID from the profile
      let mut updated_profile = profile.clone();
      updated_profile.process_id = None;

      // Check for pending updates and apply them for Camoufox profiles too
      if let Ok(Some(pending_update)) = self
        .auto_updater
        .get_pending_update(&profile.browser, &profile.version)
      {
        log::info!(
          "Found pending update for Camoufox profile {}: {} -> {}",
          profile.name,
          profile.version,
          pending_update.new_version
        );

        // Update the profile to the new version
        match self.profile_manager.update_profile_version(
          &app_handle,
          &profile.id.to_string(),
          &pending_update.new_version,
        ) {
          Ok(updated_profile_after_update) => {
            log::info!(
              "Successfully updated Camoufox profile {} from version {} to {}",
              profile.name,
              profile.version,
              pending_update.new_version
            );
            updated_profile = updated_profile_after_update;

            // Remove the pending update from the auto updater state
            if let Err(e) = self
              .auto_updater
              .dismiss_update_notification(&pending_update.id)
            {
              log::warn!("Warning: Failed to dismiss pending update notification: {e}");
            }
          }
          Err(e) => {
            log::error!(
              "Failed to apply pending update for Camoufox profile {}: {}",
              profile.name,
              e
            );
            // Continue with the original profile update (just clearing process_id)
          }
        }
      }

      self
        .save_process_info(&updated_profile)
        .map_err(|e| format!("Failed to update profile: {e}"))?;

      log::info!(
        "Emitting profile events for successful Camoufox kill: {}",
        updated_profile.name
      );

      // Emit profile update event to frontend
      if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }

      // Emit minimal running changed event to frontend immediately
      #[derive(Serialize)]
      struct RunningChangedPayload {
        id: String,
        is_running: bool,
      }
      let payload = RunningChangedPayload {
        id: updated_profile.id.to_string(),
        is_running: false, // Explicitly set to false since we just killed it
      };

      if let Err(e) = app_handle.emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      } else {
        log::info!(
          "Successfully emitted profile-running-changed event for Camoufox {}: running={}",
          updated_profile.name,
          payload.is_running
        );
      }

      log::info!(
        "Camoufox process cleanup completed for profile: {} (ID: {})",
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

    // For non-camoufox browsers, use the existing logic
    let pid = if let Some(pid) = profile.process_id {
      // First verify the stored PID is still valid and belongs to our profile
      let system = System::new_all();
      if let Some(process) = system.process(sysinfo::Pid::from(pid as usize)) {
        let cmd = process.cmd();
        let exe_name = process.name().to_string_lossy();

        // Verify this process is actually our browser
        let is_correct_browser = match profile.browser.as_str() {
          "firefox" => {
            exe_name.contains("firefox")
              && !exe_name.contains("developer")
              && !exe_name.contains("tor")
              && !exe_name.contains("mullvad")
              && !exe_name.contains("camoufox")
          }
          "firefox-developer" => {
            // More flexible detection for Firefox Developer Edition
            (exe_name.contains("firefox") && exe_name.contains("developer"))
              || (exe_name.contains("firefox")
                && cmd.iter().any(|arg| {
                  let arg_str = arg.to_str().unwrap_or("");
                  arg_str.contains("Developer")
                    || arg_str.contains("developer")
                    || arg_str.contains("FirefoxDeveloperEdition")
                    || arg_str.contains("firefox-developer")
                }))
              || exe_name == "firefox" // Firefox Developer might just show as "firefox"
          }
          "mullvad-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "mullvad-browser"),
          "tor-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "tor-browser"),
          "zen" => exe_name.contains("zen"),
          "chromium" => exe_name.contains("chromium") || exe_name.contains("chrome"),
          "brave" => exe_name.contains("brave") || exe_name.contains("Brave"),
          _ => false,
        };

        if is_correct_browser {
          // Verify profile path match
          let profiles_dir = self.profile_manager.get_profiles_dir();
          let profile_data_path = profile.get_profile_data_path(&profiles_dir);
          let profile_data_path_str = profile_data_path.to_string_lossy();

          let profile_path_match = if matches!(
            profile.browser.as_str(),
            "firefox" | "firefox-developer" | "tor-browser" | "mullvad-browser" | "zen"
          ) {
            // Firefox-based browsers: look for -profile argument followed by path
            let mut found_profile_arg = false;
            for (i, arg) in cmd.iter().enumerate() {
              if let Some(arg_str) = arg.to_str() {
                if arg_str == "-profile" && i + 1 < cmd.len() {
                  if let Some(next_arg) = cmd.get(i + 1).and_then(|a| a.to_str()) {
                    if next_arg == profile_data_path_str {
                      found_profile_arg = true;
                      break;
                    }
                  }
                }
                // Also check for combined -profile=path format
                if arg_str == format!("-profile={profile_data_path_str}") {
                  found_profile_arg = true;
                  break;
                }
                // Check if the argument is the profile path directly
                if arg_str == profile_data_path_str {
                  found_profile_arg = true;
                  break;
                }
              }
            }
            found_profile_arg
          } else {
            // Chromium-based browsers: look for --user-data-dir argument
            cmd.iter().any(|s| {
              if let Some(arg) = s.to_str() {
                arg == format!("--user-data-dir={profile_data_path_str}")
                  || arg == profile_data_path_str
              } else {
                false
              }
            })
          };

          if profile_path_match {
            log::info!(
              "Verified stored PID {} is valid for profile {} (ID: {})",
              pid,
              profile.name,
              profile.id
            );
            pid
          } else {
            log::info!("Stored PID {} doesn't match profile path for {} (ID: {}), searching for correct process", pid, profile.name, profile.id);
            // Fall through to search for correct process
            self.find_browser_process_by_profile(profile)?
          }
        } else {
          log::info!("Stored PID {} doesn't match browser type for {} (ID: {}), searching for correct process", pid, profile.name, profile.id);
          // Fall through to search for correct process
          self.find_browser_process_by_profile(profile)?
        }
      } else {
        log::info!(
          "Stored PID {} is no longer valid for profile {} (ID: {}), searching for correct process",
          pid,
          profile.name,
          profile.id
        );
        // Fall through to search for correct process
        self.find_browser_process_by_profile(profile)?
      }
    } else {
      // No stored PID, search for the process
      self.find_browser_process_by_profile(profile)?
    };

    log::info!("Attempting to kill browser process with PID: {pid}");

    // Stop any associated proxy first
    if let Err(e) = PROXY_MANAGER.stop_proxy(app_handle.clone(), pid).await {
      log::warn!("Warning: Failed to stop proxy for PID {pid}: {e}");
    }

    #[cfg(target_os = "macos")]
    {
      let profiles_dir = self.profile_manager.get_profiles_dir();
      let profile_data_path = profile.get_profile_data_path(&profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy().to_string();
      platform_browser::macos::kill_browser_process_impl(pid, Some(&profile_path_str)).await?;
    }

    #[cfg(target_os = "windows")]
    platform_browser::windows::kill_browser_process_impl(pid).await?;

    #[cfg(target_os = "linux")]
    platform_browser::linux::kill_browser_process_impl(pid).await?;

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("Unsupported platform".into());

    let system = System::new_all();
    if system.process(sysinfo::Pid::from(pid as usize)).is_some() {
      log::error!(
        "Browser process {} is still running after kill attempt for profile: {} (ID: {})",
        pid,
        profile.name,
        profile.id
      );
      return Err(
        format!(
          "Browser process {} is still running after kill attempt",
          pid
        )
        .into(),
      );
    }

    log::info!(
      "Verified browser process {} is terminated for profile: {} (ID: {})",
      pid,
      profile.name,
      profile.id
    );

    // Clear the process ID from the profile
    let mut updated_profile = profile.clone();
    updated_profile.process_id = None;

    // Check for pending updates and apply them
    if let Ok(Some(pending_update)) = self
      .auto_updater
      .get_pending_update(&profile.browser, &profile.version)
    {
      log::info!(
        "Found pending update for profile {}: {} -> {}",
        profile.name,
        profile.version,
        pending_update.new_version
      );

      // Update the profile to the new version
      match self.profile_manager.update_profile_version(
        &app_handle,
        &profile.id.to_string(),
        &pending_update.new_version,
      ) {
        Ok(updated_profile_after_update) => {
          log::info!(
            "Successfully updated profile {} from version {} to {}",
            profile.name,
            profile.version,
            pending_update.new_version
          );
          updated_profile = updated_profile_after_update;

          // Remove the pending update from the auto updater state
          if let Err(e) = self
            .auto_updater
            .dismiss_update_notification(&pending_update.id)
          {
            log::warn!("Warning: Failed to dismiss pending update notification: {e}");
          }
        }
        Err(e) => {
          log::error!(
            "Failed to apply pending update for profile {}: {}",
            profile.name,
            e
          );
          // Continue with the original profile update (just clearing process_id)
        }
      }
    }

    self
      .save_process_info(&updated_profile)
      .map_err(|e| format!("Failed to update profile: {e}"))?;

    log::info!(
      "Emitting profile events for successful kill: {}",
      updated_profile.name
    );

    // Emit profile update event to frontend
    if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
      log::warn!("Warning: Failed to emit profile update event: {e}");
    }

    // Emit minimal running changed event to frontend immediately
    #[derive(Serialize)]
    struct RunningChangedPayload {
      id: String,
      is_running: bool,
    }
    let payload = RunningChangedPayload {
      id: updated_profile.id.to_string(),
      is_running: false, // Explicitly set to false since we just killed it
    };

    if let Err(e) = app_handle.emit("profile-running-changed", &payload) {
      log::warn!("Warning: Failed to emit profile running changed event: {e}");
    } else {
      log::info!(
        "Successfully emitted profile-running-changed event for {}: running={}",
        updated_profile.name,
        payload.is_running
      );
    }

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

    Ok(())
  }

  /// Helper method to find browser process by profile path
  fn find_browser_process_by_profile(
    &self,
    profile: &BrowserProfile,
  ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let system = System::new_all();
    let profiles_dir = self.profile_manager.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    let profile_data_path_str = profile_data_path.to_string_lossy();

    log::info!(
      "Searching for {} browser process with profile path: {}",
      profile.browser,
      profile_data_path_str
    );

    for (pid, process) in system.processes() {
      let cmd = process.cmd();
      if cmd.is_empty() {
        continue;
      }

      // Check if this is the right browser executable first
      let exe_name = process.name().to_string_lossy().to_lowercase();
      let is_correct_browser = match profile.browser.as_str() {
        "firefox" => {
          exe_name.contains("firefox")
            && !exe_name.contains("developer")
            && !exe_name.contains("tor")
            && !exe_name.contains("mullvad")
            && !exe_name.contains("camoufox")
        }
        "firefox-developer" => {
          // More flexible detection for Firefox Developer Edition
          (exe_name.contains("firefox") && exe_name.contains("developer"))
            || (exe_name.contains("firefox")
              && cmd.iter().any(|arg| {
                let arg_str = arg.to_str().unwrap_or("");
                arg_str.contains("Developer")
                  || arg_str.contains("developer")
                  || arg_str.contains("FirefoxDeveloperEdition")
                  || arg_str.contains("firefox-developer")
              }))
            || exe_name == "firefox" // Firefox Developer might just show as "firefox"
        }
        "mullvad-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "mullvad-browser"),
        "tor-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "tor-browser"),
        "zen" => exe_name.contains("zen"),
        "chromium" => exe_name.contains("chromium") || exe_name.contains("chrome"),
        "brave" => exe_name.contains("brave") || exe_name.contains("Brave"),
        _ => false,
      };

      if !is_correct_browser {
        continue;
      }

      // Check for profile path match with improved logic
      let profile_path_match = if matches!(
        profile.browser.as_str(),
        "firefox" | "firefox-developer" | "tor-browser" | "mullvad-browser" | "zen"
      ) {
        // Firefox-based browsers: look for -profile argument followed by path
        let mut found_profile_arg = false;
        for (i, arg) in cmd.iter().enumerate() {
          if let Some(arg_str) = arg.to_str() {
            if arg_str == "-profile" && i + 1 < cmd.len() {
              if let Some(next_arg) = cmd.get(i + 1).and_then(|a| a.to_str()) {
                if next_arg == profile_data_path_str {
                  found_profile_arg = true;
                  break;
                }
              }
            }
            // Also check for combined -profile=path format
            if arg_str == format!("-profile={profile_data_path_str}") {
              found_profile_arg = true;
              break;
            }
            // Check if the argument is the profile path directly
            if arg_str == profile_data_path_str {
              found_profile_arg = true;
              break;
            }
          }
        }
        found_profile_arg
      } else {
        // Chromium-based browsers: look for --user-data-dir argument
        cmd.iter().any(|s| {
          if let Some(arg) = s.to_str() {
            arg == format!("--user-data-dir={profile_data_path_str}")
              || arg == profile_data_path_str
          } else {
            false
          }
        })
      };

      if profile_path_match {
        let pid_u32 = pid.as_u32();
        log::info!(
          "Found matching {} browser process with PID: {} for profile: {} (ID: {})",
          profile.browser,
          pid_u32,
          profile.name,
          profile.id
        );
        return Ok(pid_u32);
      }
    }

    Err(
      format!(
        "No running {} browser process found for profile: {} (ID: {})",
        profile.browser, profile.name, profile.id
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
  log::info!(
    "Launch request received for profile: {} (ID: {})",
    profile.name,
    profile.id
  );

  let browser_runner = BrowserRunner::instance();

  // Store the internal proxy settings for passing to launch_browser
  let mut internal_proxy_settings: Option<ProxySettings> = None;

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

  // Always start a local proxy before launching (non-Camoufox handled here; Camoufox has its own flow)
  // This ensures all traffic goes through the local proxy for monitoring and future features
  if profile.browser != "camoufox" {
    // Determine upstream proxy if configured; otherwise use DIRECT (no upstream)
    let upstream_proxy = profile_for_launch
      .proxy_id
      .as_ref()
      .and_then(|id| PROXY_MANAGER.get_proxy_settings_by_id(id));

    // Use a temporary PID (1) to start the proxy, we'll update it after browser launch
    let temp_pid = 1u32;

    // Always start a local proxy, even if there's no upstream proxy
    // This allows for traffic monitoring and future features
    match PROXY_MANAGER
      .start_proxy(
        app_handle.clone(),
        upstream_proxy.as_ref(),
        temp_pid,
        Some(&profile.name),
      )
      .await
    {
      Ok(internal_proxy) => {
        // Use internal proxy for subsequent launch
        internal_proxy_settings = Some(internal_proxy.clone());

        // For Firefox-based browsers, always apply PAC/user.js to point to the local proxy
        if matches!(
          profile_for_launch.browser.as_str(),
          "firefox" | "firefox-developer" | "zen" | "tor-browser" | "mullvad-browser"
        ) {
          let profiles_dir = browser_runner.profile_manager.get_profiles_dir();
          let profile_path = profiles_dir
            .join(profile_for_launch.id.to_string())
            .join("profile");

          // Provide a dummy upstream (ignored when internal proxy is provided)
          let dummy_upstream = ProxySettings {
            proxy_type: "http".to_string(),
            host: "127.0.0.1".to_string(),
            port: internal_proxy.port,
            username: None,
            password: None,
          };

          browser_runner
            .profile_manager
            .apply_proxy_settings_to_profile(&profile_path, &dummy_upstream, Some(&internal_proxy))
            .map_err(|e| format!("Failed to update profile proxy: {e}"))?;
        }

        log::info!(
          "Local proxy prepared for profile: {} on port: {} (upstream: {})",
          profile_for_launch.name,
          internal_proxy.port,
          upstream_proxy
            .as_ref()
            .map(|p| format!("{}:{}", p.host, p.port))
            .unwrap_or_else(|| "DIRECT".to_string())
        );
      }
      Err(e) => {
        let error_msg = format!("Failed to start local proxy: {e}");
        log::error!("{}", error_msg);
        // DO NOT launch browser if proxy startup fails - all browsers must use local proxy
        return Err(error_msg);
      }
    }
  }

  log::info!(
    "Starting browser launch for profile: {} (ID: {})",
    profile_for_launch.name,
    profile_for_launch.id
  );

  // Launch browser or open URL in existing instance
  let updated_profile = browser_runner.launch_or_open_url(app_handle.clone(), &profile_for_launch, url, internal_proxy_settings.as_ref()).await.map_err(|e| {
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

    if let Err(e) = app_handle.emit("profile-running-changed", &payload) {
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
    // Update the proxy manager with the correct PID (we always started with temp pid 1 for non-Camoufox)
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

      if let Err(e) = app_handle.emit("profile-running-changed", &payload) {
        log::warn!("Warning: Failed to emit profile running changed event: {e}");
      }

      Err(format!("Failed to kill browser: {e}"))
    }
  }
}

pub async fn launch_browser_profile_with_debugging(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
  remote_debugging_port: Option<u16>,
  headless: bool,
) -> Result<BrowserProfile, String> {
  let browser_runner = BrowserRunner::instance();
  browser_runner
    .launch_browser_with_debugging(app_handle, &profile, url, remote_debugging_port, headless)
    .await
    .map_err(|e| format!("Failed to launch browser with debugging: {e}"))
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

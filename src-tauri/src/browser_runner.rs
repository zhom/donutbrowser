use crate::platform_browser;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::PROXY_MANAGER;
use directories::BaseDirs;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::System;
use tauri::Emitter;

use crate::browser::{create_browser, BrowserType, ProxySettings};
use crate::browser_version_manager::{
  BrowserVersionInfo, BrowserVersionManager, BrowserVersionsResult,
};
use crate::camoufox::CamoufoxConfig;
use crate::download::DownloadProgress;
use crate::downloaded_browsers::DownloadedBrowsersRegistry;

// Global state to track currently downloading browser-version pairs
lazy_static::lazy_static! {
  static ref DOWNLOADING_BROWSERS: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
}

pub struct BrowserRunner {
  base_dirs: BaseDirs,
}

impl BrowserRunner {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
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

  pub fn get_profiles_dir(&self) -> PathBuf {
    let profile_manager = ProfileManager::instance();
    profile_manager.get_profiles_dir()
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

  /// Internal method to cleanup unused binaries (used by auto-cleanup)
  pub fn cleanup_unused_binaries_internal(
    &self,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // Load current profiles
    let profiles = self
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    // Get registry instance
    let registry = crate::downloaded_browsers::DownloadedBrowsersRegistry::instance();

    // Get active browser versions (all profiles)
    let active_versions = registry.get_active_browser_versions(&profiles);

    // Get running browser versions (only running profiles)
    let running_versions = registry.get_running_browser_versions(&profiles);

    // Get binaries directory
    let binaries_dir = self.get_binaries_dir();

    // Use comprehensive cleanup that syncs registry with disk and removes unused binaries
    let cleaned_up =
      registry.comprehensive_cleanup(&binaries_dir, &active_versions, &running_versions)?;

    // Registry is already saved by comprehensive_cleanup
    Ok(cleaned_up)
  }

  fn apply_proxy_settings_to_profile(
    &self,
    profile_data_path: &Path,
    proxy: &ProxySettings,
    internal_proxy: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profile_manager = ProfileManager::instance();
    profile_manager.apply_proxy_settings_to_profile(profile_data_path, proxy, internal_proxy)
  }

  pub fn save_profile(&self, profile: &BrowserProfile) -> Result<(), Box<dyn std::error::Error>> {
    let profile_manager = ProfileManager::instance();
    let result = profile_manager.save_profile(profile);
    // Update tag suggestions after any save
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });
    result
  }

  pub fn list_profiles(&self) -> Result<Vec<BrowserProfile>, Box<dyn std::error::Error>> {
    let profile_manager = ProfileManager::instance();

    profile_manager.list_profiles()
  }

  pub async fn launch_browser(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    local_proxy_settings: Option<&ProxySettings>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Check if browser is disabled due to ongoing update
    let auto_updater = crate::auto_updater::AutoUpdater::instance();
    if auto_updater.is_browser_disabled(&profile.browser)? {
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
        println!(
          "No camoufox config found for profile {}, using default",
          profile.name
        );
        crate::camoufox::CamoufoxConfig::default()
      });

      // Always start a local proxy for Camoufox (for traffic monitoring and geoip support)
      let upstream_proxy = profile
        .proxy_id
        .as_ref()
        .and_then(|id| PROXY_MANAGER.get_proxy_settings_by_id(id));

      println!(
        "Starting local proxy for Camoufox profile: {} (upstream: {})",
        profile.name,
        upstream_proxy
          .as_ref()
          .map(|p| format!("{}:{}", p.host, p.port))
          .unwrap_or_else(|| "DIRECT".to_string())
      );

      // Start the proxy and get local proxy settings
      let local_proxy = PROXY_MANAGER
        .start_proxy(
          app_handle.clone(),
          upstream_proxy.as_ref(),
          0, // Use 0 as temporary PID, will be updated later
          Some(&profile.name),
        )
        .await
        .map_err(|e| format!("Failed to start local proxy for Camoufox: {e}"))?;

      // Format proxy URL for camoufox - always use HTTP for the local proxy
      let proxy_url = format!("http://{}:{}", local_proxy.host, local_proxy.port);

      // Set proxy in camoufox config
      camoufox_config.proxy = Some(proxy_url);

      // Ensure geoip is always enabled for proper geolocation spoofing
      if camoufox_config.geoip.is_none() {
        camoufox_config.geoip = Some(serde_json::Value::Bool(true));
      }

      println!(
        "Configured local proxy for Camoufox: {:?}, geoip: {:?}",
        camoufox_config.proxy, camoufox_config.geoip
      );

      // Use the nodecar camoufox launcher
      println!(
        "Launching Camoufox via nodecar for profile: {}",
        profile.name
      );
      let camoufox_launcher = crate::camoufox::CamoufoxNodecarLauncher::instance();
      let camoufox_result = camoufox_launcher
        .launch_camoufox_profile_nodecar(app_handle.clone(), profile.clone(), camoufox_config, url)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to launch camoufox via nodecar: {e}").into()
        })?;

      // For server-based Camoufox, we use the process_id
      let process_id = camoufox_result.processId.unwrap_or(0);
      println!("Camoufox launched successfully with PID: {process_id}");

      // Update profile with the process info from camoufox result
      let mut updated_profile = profile.clone();
      updated_profile.process_id = Some(process_id);
      updated_profile.last_launch = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());

      // Update the proxy manager with the correct PID
      if let Err(e) = PROXY_MANAGER.update_proxy_pid(0, process_id) {
        println!("Warning: Failed to update proxy PID mapping: {e}");
      } else {
        println!("Updated proxy PID mapping from temp (0) to actual PID: {process_id}");
      }

      // Save the updated profile
      self.save_process_info(&updated_profile)?;
      // Ensure tag suggestions include any tags from this profile
      let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
        let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
      });
      println!(
        "Updated profile with process info: {}",
        updated_profile.name
      );

      println!(
        "Emitting profile events for successful Camoufox launch: {}",
        updated_profile.name
      );

      // Emit profile update event to frontend
      if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
        println!("Warning: Failed to emit profile update event: {e}");
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
        println!("Warning: Failed to emit profile running changed event: {e}");
      } else {
        println!(
          "Successfully emitted profile-running-changed event for Camoufox {}: running={}",
          updated_profile.name, payload.is_running
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

    println!("Executable path: {executable_path:?}");

    // Prepare the executable (set permissions, etc.)
    if let Err(e) = browser.prepare_executable(&executable_path) {
      println!("Warning: Failed to prepare executable: {e}");
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
    let profiles_dir = self.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    let browser_args = browser
      .create_launch_args(
        &profile_data_path.to_string_lossy(),
        proxy_for_launch_args,
        url,
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

    println!(
      "Launched browser with launcher PID: {} for profile: {} (ID: {})",
      launcher_pid, profile.name, profile.id
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
          println!(
            "Found actual {} browser process: PID {} ({})",
            profile.browser, pid_u32, process_name
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
      let profiles_dir = self.get_profiles_dir();
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
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
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

    println!(
      "Emitting profile events for successful launch: {} (ID: {})",
      updated_profile.name, updated_profile.id
    );

    // Emit profile update event to frontend
    if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
      println!("Warning: Failed to emit profile update event: {e}");
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
      println!("Warning: Failed to emit profile running changed event: {e}");
    } else {
      println!(
        "Successfully emitted profile-running-changed event for {}: running={}",
        updated_profile.name, payload.is_running
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
      let camoufox_launcher = crate::camoufox::CamoufoxNodecarLauncher::instance();

      // Get the profile path based on the UUID
      let profiles_dir = self.get_profiles_dir();
      let profile_data_path = profile.get_profile_data_path(&profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      // Check if the process is running
      match camoufox_launcher
        .find_camoufox_by_profile(&profile_path_str)
        .await
      {
        Ok(Some(_camoufox_process)) => {
          println!(
            "Opening URL in existing Camoufox process for profile: {} (ID: {})",
            profile.name, profile.id
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
    let profiles = self.list_profiles().expect("Failed to list profiles");
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
          let profiles_dir = self.get_profiles_dir();
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
          let profiles_dir = self.get_profiles_dir();
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
          let profiles_dir = self.get_profiles_dir();
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
          let profiles_dir = self.get_profiles_dir();
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
          let profiles_dir = self.get_profiles_dir();
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
          let profiles_dir = self.get_profiles_dir();
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
          let profiles_dir = self.get_profiles_dir();
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
          let profiles_dir = self.get_profiles_dir();
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
          let profiles_dir = self.get_profiles_dir();
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

  pub async fn launch_or_open_url(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
    url: Option<String>,
    internal_proxy_settings: Option<&ProxySettings>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    println!(
      "launch_or_open_url called for profile: {} (ID: {})",
      profile.name, profile.id
    );

    // Get the most up-to-date profile data
    let profiles = self
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles in launch_or_open_url: {e}"))?;
    let updated_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| profile.clone());

    println!(
      "Checking browser status for profile: {} (ID: {})",
      updated_profile.name, updated_profile.id
    );

    // Check if browser is already running
    let is_running = self
      .check_browser_status(app_handle.clone(), &updated_profile)
      .await
      .map_err(|e| format!("Failed to check browser status: {e}"))?;

    // Get the updated profile again after status check (PID might have been updated)
    let profiles = self
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles after status check: {e}"))?;
    let final_profile = profiles
      .into_iter()
      .find(|p| p.id == profile.id)
      .unwrap_or_else(|| updated_profile.clone());

    println!(
      "Browser status check - Profile: {} (ID: {}), Running: {}, URL: {:?}, PID: {:?}",
      final_profile.name, final_profile.id, is_running, url, final_profile.process_id
    );

    if is_running && url.is_some() {
      // Browser is running and we have a URL to open
      if let Some(url_ref) = url.as_ref() {
        println!("Opening URL in existing browser: {url_ref}");

        // For TOR/Mullvad browsers, add extra verification
        if matches!(
          final_profile.browser.as_str(),
          "tor-browser" | "mullvad-browser"
        ) {
          println!("TOR/Mullvad browser detected - ensuring we have correct PID");
          if final_profile.process_id.is_none() {
            println!(
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
            println!("Successfully opened URL in existing browser");
            Ok(final_profile)
          }
          Err(e) => {
            println!("Failed to open URL in existing browser: {e}");

            // For Mullvad and Tor browsers, don't fall back to new instance since they use -no-remote
            // and can't have multiple instances with the same profile
            match final_profile.browser.as_str() {
              "mullvad-browser" | "tor-browser" => {
                Err(format!("Failed to open URL in existing {} browser. Cannot launch new instance due to profile conflict: {}", final_profile.browser, e).into())
              }
              _ => {
                println!("Falling back to new instance for browser: {}", final_profile.browser);
                // Fallback to launching a new instance for other browsers
                self.launch_browser(app_handle.clone(), &final_profile, url, internal_proxy_settings).await
              }
            }
          }
        }
      } else {
        // This case shouldn't happen since we checked is_some() above, but handle it gracefully
        println!("URL was unexpectedly None, launching new browser instance");
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
        println!("Launching new browser instance - browser not running");
      } else {
        println!("Launching new browser instance - no URL provided");
      }
      self
        .launch_browser(
          app_handle.clone(),
          &final_profile,
          url,
          internal_proxy_settings,
        )
        .await
    }
  }

  fn save_process_info(
    &self,
    profile: &BrowserProfile,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use the regular save_profile method which handles the UUID structure
    self.save_profile(profile).map_err(|e| {
      let error_string = e.to_string();
      Box::new(std::io::Error::other(error_string)) as Box<dyn std::error::Error + Send + Sync>
    })
  }

  pub fn delete_profile(&self, profile_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let profile_manager = ProfileManager::instance();
    profile_manager.delete_profile(profile_id)?;

    // Always perform cleanup after profile deletion to remove unused binaries
    if let Err(e) = self.cleanup_unused_binaries_internal() {
      println!("Warning: Failed to cleanup unused binaries: {e}");
    }

    // Rebuild tags after deletion
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    Ok(())
  }

  pub async fn check_browser_status(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let profile_manager = ProfileManager::instance();
    profile_manager
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
      let camoufox_launcher = crate::camoufox::CamoufoxNodecarLauncher::instance();

      // Search by profile path to find the running Camoufox instance
      let profiles_dir = self.get_profiles_dir();
      let profile_data_path = profile.get_profile_data_path(&profiles_dir);
      let profile_path_str = profile_data_path.to_string_lossy();

      println!(
        "Attempting to kill Camoufox process for profile: {} (ID: {})",
        profile.name, profile.id
      );

      match camoufox_launcher
        .find_camoufox_by_profile(&profile_path_str)
        .await
      {
        Ok(Some(camoufox_process)) => {
          println!(
            "Found Camoufox process: {} (PID: {:?})",
            camoufox_process.id, camoufox_process.processId
          );

          match camoufox_launcher
            .stop_camoufox(&app_handle, &camoufox_process.id)
            .await
          {
            Ok(stopped) => {
              if stopped {
                println!(
                  "Successfully stopped Camoufox process: {} (PID: {:?})",
                  camoufox_process.id, camoufox_process.processId
                );
              } else {
                println!(
                  "Failed to stop Camoufox process: {} (PID: {:?})",
                  camoufox_process.id, camoufox_process.processId
                );
              }
            }
            Err(e) => {
              println!(
                "Error stopping Camoufox process {}: {}",
                camoufox_process.id, e
              );
            }
          }
        }
        Ok(None) => {
          println!(
            "No running Camoufox process found for profile: {} (ID: {})",
            profile.name, profile.id
          );
        }
        Err(e) => {
          println!(
            "Error finding Camoufox process for profile {}: {}",
            profile.name, e
          );
        }
      }

      // Clear the process ID from the profile
      let mut updated_profile = profile.clone();
      updated_profile.process_id = None;
      self
        .save_process_info(&updated_profile)
        .map_err(|e| format!("Failed to update profile: {e}"))?;

      println!(
        "Emitting profile events for successful Camoufox kill: {}",
        updated_profile.name
      );

      // Emit profile update event to frontend
      if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
        println!("Warning: Failed to emit profile update event: {e}");
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
        println!("Warning: Failed to emit profile running changed event: {e}");
      } else {
        println!(
          "Successfully emitted profile-running-changed event for Camoufox {}: running={}",
          updated_profile.name, payload.is_running
        );
      }

      println!(
        "Camoufox process cleanup completed for profile: {} (ID: {})",
        profile.name, profile.id
      );
      return Ok(());
    }

    // For non-camoufox browsers, use the existing logic
    let pid = if let Some(pid) = profile.process_id {
      // First verify the stored PID is still valid and belongs to our profile
      let system = System::new_all();
      if let Some(process) = system.process(sysinfo::Pid::from(pid as usize)) {
        let cmd = process.cmd();
        let exe_name = process.name().to_string_lossy().to_lowercase();

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
          let profiles_dir = self.get_profiles_dir();
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
            println!(
              "Verified stored PID {} is valid for profile {} (ID: {})",
              pid, profile.name, profile.id
            );
            pid
          } else {
            println!("Stored PID {} doesn't match profile path for {} (ID: {}), searching for correct process", pid, profile.name, profile.id);
            // Fall through to search for correct process
            self.find_browser_process_by_profile(profile)?
          }
        } else {
          println!("Stored PID {} doesn't match browser type for {} (ID: {}), searching for correct process", pid, profile.name, profile.id);
          // Fall through to search for correct process
          self.find_browser_process_by_profile(profile)?
        }
      } else {
        println!(
          "Stored PID {} is no longer valid for profile {} (ID: {}), searching for correct process",
          pid, profile.name, profile.id
        );
        // Fall through to search for correct process
        self.find_browser_process_by_profile(profile)?
      }
    } else {
      // No stored PID, search for the process
      self.find_browser_process_by_profile(profile)?
    };

    println!("Attempting to kill browser process with PID: {pid}");

    // Stop any associated proxy first
    if let Err(e) = PROXY_MANAGER.stop_proxy(app_handle.clone(), pid).await {
      println!("Warning: Failed to stop proxy for PID {pid}: {e}");
    }

    // Kill the process using platform-specific implementation
    #[cfg(target_os = "macos")]
    platform_browser::macos::kill_browser_process_impl(pid).await?;

    #[cfg(target_os = "windows")]
    platform_browser::windows::kill_browser_process_impl(pid).await?;

    #[cfg(target_os = "linux")]
    platform_browser::linux::kill_browser_process_impl(pid).await?;

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("Unsupported platform".into());

    // Clear the process ID from the profile
    let mut updated_profile = profile.clone();
    updated_profile.process_id = None;
    self
      .save_process_info(&updated_profile)
      .map_err(|e| format!("Failed to update profile: {e}"))?;

    println!(
      "Emitting profile events for successful kill: {}",
      updated_profile.name
    );

    // Emit profile update event to frontend
    if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
      println!("Warning: Failed to emit profile update event: {e}");
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
      println!("Warning: Failed to emit profile running changed event: {e}");
    } else {
      println!(
        "Successfully emitted profile-running-changed event for {}: running={}",
        updated_profile.name, payload.is_running
      );
    }

    Ok(())
  }

  /// Helper method to find browser process by profile path
  fn find_browser_process_by_profile(
    &self,
    profile: &BrowserProfile,
  ) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let system = System::new_all();
    let profiles_dir = self.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    let profile_data_path_str = profile_data_path.to_string_lossy();

    println!(
      "Searching for {} browser process with profile path: {}",
      profile.browser, profile_data_path_str
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
        println!(
          "Found matching {} browser process with PID: {} for profile: {} (ID: {})",
          profile.browser, pid_u32, profile.name, profile.id
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

  /// Check if browser binaries exist for all profiles and return missing binaries
  pub async fn check_missing_binaries(
    &self,
  ) -> Result<Vec<(String, String, String)>, Box<dyn std::error::Error + Send + Sync>> {
    // Get all profiles
    let profiles = self
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    let mut missing_binaries = Vec::new();

    for profile in profiles {
      let browser_type = match BrowserType::from_str(&profile.browser) {
        Ok(bt) => bt,
        Err(_) => {
          println!(
            "Warning: Invalid browser type '{}' for profile '{}'",
            profile.browser, profile.name
          );
          continue;
        }
      };

      let browser = create_browser(browser_type.clone());
      let binaries_dir = self.get_binaries_dir();
      println!(
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

  /// Check if GeoIP database is missing for Camoufox profiles
  pub fn check_missing_geoip_database(
    &self,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Get all profiles
    let profiles = self
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    // Check if there are any Camoufox profiles
    let has_camoufox_profiles = profiles.iter().any(|profile| profile.browser == "camoufox");

    if has_camoufox_profiles {
      // Check if GeoIP database is available
      use crate::geoip_downloader::GeoIPDownloader;
      return Ok(!GeoIPDownloader::is_geoip_database_available());
    }

    Ok(false)
  }

  /// Automatically download missing binaries for all profiles
  pub async fn ensure_all_binaries_exist(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    // First, clean up any stale registry entries
    let registry = DownloadedBrowsersRegistry::instance();
    if let Ok(cleaned_up) = registry.verify_and_cleanup_stale_entries(self) {
      if !cleaned_up.is_empty() {
        println!(
          "Cleaned up {} stale registry entries: {}",
          cleaned_up.len(),
          cleaned_up.join(", ")
        );
      }
    }

    let missing_binaries = self.check_missing_binaries().await?;
    let mut downloaded = Vec::new();

    for (profile_name, browser, version) in missing_binaries {
      println!("Downloading missing binary for profile '{profile_name}': {browser} {version}");

      match self
        .download_browser_impl(app_handle.clone(), browser.clone(), version.clone())
        .await
      {
        Ok(_) => {
          downloaded.push(format!(
            "{browser} {version} (for profile '{profile_name}')"
          ));
        }
        Err(e) => {
          eprintln!("Failed to download {browser} {version} for profile '{profile_name}': {e}");
        }
      }
    }

    // Check if GeoIP database is missing for Camoufox profiles
    if self.check_missing_geoip_database()? {
      println!("GeoIP database is missing for Camoufox profiles, downloading...");

      use crate::geoip_downloader::GeoIPDownloader;

      let geoip_downloader = GeoIPDownloader::instance();

      match geoip_downloader.download_geoip_database(app_handle).await {
        Ok(_) => {
          downloaded.push("GeoIP database for Camoufox".to_string());
          println!("GeoIP database downloaded successfully");
        }
        Err(e) => {
          eprintln!("Failed to download GeoIP database: {e}");
          // Don't fail the entire operation if GeoIP download fails
        }
      }
    }

    Ok(downloaded)
  }

  pub async fn download_browser_impl(
    &self,
    app_handle: tauri::AppHandle,
    browser_str: String,
    version: String,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Check if this browser-version pair is already being downloaded
    let download_key = format!("{browser_str}-{version}");
    {
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      if downloading.contains(&download_key) {
        return Err(format!("Browser '{browser_str}' version '{version}' is already being downloaded. Please wait for the current download to complete.").into());
      }
      // Mark this browser-version pair as being downloaded
      downloading.insert(download_key.clone());
    }

    let browser_type =
      BrowserType::from_str(&browser_str).map_err(|e| format!("Invalid browser type: {e}"))?;
    let browser = create_browser(browser_type.clone());

    // Get registry instance and check if already downloaded
    let registry = DownloadedBrowsersRegistry::instance();

    // Check if registry thinks it's downloaded, but also verify files actually exist
    if registry.is_browser_downloaded(&browser_str, &version) {
      let binaries_dir = self.get_binaries_dir();
      let actually_exists = browser.is_version_downloaded(&version, &binaries_dir);

      if actually_exists {
        return Ok(version);
      } else {
        // Registry says it's downloaded but files don't exist - clean up registry
        println!("Registry indicates {browser_str} {version} is downloaded, but files are missing. Cleaning up registry entry.");
        registry.remove_browser(&browser_str, &version);
        registry
          .save()
          .map_err(|e| format!("Failed to save cleaned registry: {e}"))?;
      }
    }

    // Check if browser is supported on current platform before attempting download
    let version_service = BrowserVersionManager::instance();

    if !version_service
      .is_browser_supported(&browser_str)
      .unwrap_or(false)
    {
      return Err(
        format!(
          "Browser '{}' is not supported on your platform ({} {}). Supported browsers: {}",
          browser_str,
          std::env::consts::OS,
          std::env::consts::ARCH,
          version_service.get_supported_browsers().join(", ")
        )
        .into(),
      );
    }

    let download_info = version_service
      .get_download_info(&browser_str, &version)
      .map_err(|e| format!("Failed to get download info: {e}"))?;

    // Create browser directory
    let mut browser_dir = self.get_binaries_dir();
    browser_dir.push(&browser_str);
    browser_dir.push(&version);

    create_dir_all(&browser_dir).map_err(|e| format!("Failed to create browser directory: {e}"))?;

    // Mark download as started in registry
    registry.mark_download_started(&browser_str, &version, browser_dir.clone());
    registry
      .save()
      .map_err(|e| format!("Failed to save registry: {e}"))?;

    // Use the download module
    let downloader = crate::download::Downloader::instance();
    // Attempt to download the archive. If the download fails but an archive with the
    // expected filename already exists (manual download), continue using that file.
    let download_path: PathBuf = match downloader
      .download_browser(
        &app_handle,
        browser_type.clone(),
        &version,
        &download_info,
        &browser_dir,
      )
      .await
    {
      Ok(path) => path,
      Err(e) => {
        // Do NOT continue with extraction on failed downloads. Partial files may exist but are invalid.
        // Clean registry entry and stop here so the UI can show a single, clear error.
        let _ = registry.remove_browser(&browser_str, &version);
        let _ = registry.save();
        let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
        downloading.remove(&download_key);
        return Err(format!("Failed to download browser: {e}").into());
      }
    };

    // Use the extraction module
    if download_info.is_archive {
      let extractor = crate::extraction::Extractor::instance();
      match extractor
        .extract_browser(
          &app_handle,
          browser_type.clone(),
          &version,
          &download_path,
          &browser_dir,
        )
        .await
      {
        Ok(_) => {
          // Do not remove the archive here. We keep it until verification succeeds.
        }
        Err(e) => {
          // Do not remove the archive or extracted files. Just drop the registry entry
          // so it won't be reported as downloaded.
          let _ = registry.remove_browser(&browser_str, &version);
          let _ = registry.save();
          // Remove browser-version pair from downloading set on error
          {
            let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
            downloading.remove(&download_key);
          }
          return Err(format!("Failed to extract browser: {e}").into());
        }
      }

      // Give filesystem a moment to settle after extraction
      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Emit verification progress
    let progress = DownloadProgress {
      browser: browser_str.clone(),
      version: version.clone(),
      downloaded_bytes: 0,
      total_bytes: None,
      percentage: 100.0,
      speed_bytes_per_sec: 0.0,
      eta_seconds: None,
      stage: "verifying".to_string(),
    };
    let _ = app_handle.emit("download-progress", &progress);

    // Verify the browser was downloaded correctly
    println!("Verifying download for browser: {browser_str}, version: {version}");

    // Use the browser's own verification method
    let binaries_dir = self.get_binaries_dir();
    if !browser.is_version_downloaded(&version, &binaries_dir) {
      // Provide detailed error information for debugging
      let browser_dir = binaries_dir.join(&browser_str).join(&version);
      let mut error_details = format!(
        "Browser download completed but verification failed for {} {}. Expected directory: {}",
        browser_str,
        version,
        browser_dir.display()
      );

      // List what files actually exist
      if browser_dir.exists() {
        error_details.push_str("\nFiles found in directory:");
        if let Ok(entries) = std::fs::read_dir(&browser_dir) {
          for entry in entries.flatten() {
            let path = entry.path();
            let file_type = if path.is_dir() { "DIR" } else { "FILE" };
            error_details.push_str(&format!("\n  {} {}", file_type, path.display()));
          }
        } else {
          error_details.push_str("\n  (Could not read directory contents)");
        }
      } else {
        error_details.push_str("\nDirectory does not exist!");
      }

      // For Camoufox on Linux, provide specific expected files
      if browser_str == "camoufox" && cfg!(target_os = "linux") {
        let camoufox_subdir = browser_dir.join("camoufox");
        error_details.push_str("\nExpected Camoufox executable locations:");
        error_details.push_str(&format!("\n  {}/camoufox-bin", camoufox_subdir.display()));
        error_details.push_str(&format!("\n  {}/camoufox", camoufox_subdir.display()));

        if camoufox_subdir.exists() {
          error_details.push_str(&format!(
            "\nCamoufox subdirectory exists: {}",
            camoufox_subdir.display()
          ));
          if let Ok(entries) = std::fs::read_dir(&camoufox_subdir) {
            error_details.push_str("\nFiles in camoufox subdirectory:");
            for entry in entries.flatten() {
              let path = entry.path();
              let file_type = if path.is_dir() { "DIR" } else { "FILE" };
              error_details.push_str(&format!("\n  {} {}", file_type, path.display()));
            }
          }
        } else {
          error_details.push_str(&format!(
            "\nCamoufox subdirectory does not exist: {}",
            camoufox_subdir.display()
          ));
        }
      }

      // Do not delete files on verification failure; keep archive for manual retry.
      let _ = registry.remove_browser(&browser_str, &version);
      let _ = registry.save();
      // Remove browser-version pair from downloading set on verification failure
      {
        let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
        downloading.remove(&download_key);
      }
      return Err(error_details.into());
    }

    // Mark completion in registry. If it fails (e.g., rare race during cleanup), log but continue.
    if let Err(e) = registry.mark_download_completed(&browser_str, &version) {
      eprintln!("Warning: Could not mark {browser_str} {version} as completed in registry: {e}");
    }
    registry
      .save()
      .map_err(|e| format!("Failed to save registry: {e}"))?;

    // Now that verification succeeded, remove the archive file if it exists
    if download_info.is_archive {
      let archive_path = browser_dir.join(&download_info.filename);
      if archive_path.exists() {
        if let Err(e) = std::fs::remove_file(&archive_path) {
          println!("Warning: Could not delete archive file after verification: {e}");
        }
      }
    }

    // If this is Camoufox, automatically download GeoIP database
    if browser_str == "camoufox" {
      use crate::geoip_downloader::GeoIPDownloader;

      // Check if GeoIP database is already available
      if !GeoIPDownloader::is_geoip_database_available() {
        println!("Downloading GeoIP database for Camoufox...");

        let geoip_downloader = GeoIPDownloader::instance();

        match geoip_downloader.download_geoip_database(&app_handle).await {
          Ok(_) => {
            println!("GeoIP database downloaded successfully");
          }
          Err(e) => {
            eprintln!("Failed to download GeoIP database: {e}");
            // Don't fail the browser download if GeoIP download fails
          }
        }
      } else {
        println!("GeoIP database already available");
      }
    }

    // Emit completion
    let progress = DownloadProgress {
      browser: browser_str.clone(),
      version: version.clone(),
      downloaded_bytes: 0,
      total_bytes: None,
      percentage: 100.0,
      speed_bytes_per_sec: 0.0,
      eta_seconds: Some(0.0),
      stage: "completed".to_string(),
    };
    let _ = app_handle.emit("download-progress", &progress);

    // Remove browser-version pair from downloading set
    {
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      downloading.remove(&download_key);
    }

    Ok(version)
  }

  /// Check if a browser version is downloaded
  pub fn is_browser_downloaded(&self, browser_str: &str, version: &str) -> bool {
    // Always check if files actually exist on disk
    let browser_type = match BrowserType::from_str(browser_str) {
      Ok(bt) => bt,
      Err(_) => {
        println!("Invalid browser type: {browser_str}");
        return false;
      }
    };
    let browser = create_browser(browser_type.clone());
    let binaries_dir = self.get_binaries_dir();
    let files_exist = browser.is_version_downloaded(version, &binaries_dir);

    // If files don't exist but registry thinks they do, clean up the registry
    if !files_exist {
      let registry = DownloadedBrowsersRegistry::instance();
      if registry.is_browser_downloaded(browser_str, version) {
        println!("Cleaning up stale registry entry for {browser_str} {version}");
        registry.remove_browser(browser_str, version);
        let _ = registry.save(); // Don't fail if save fails, just log
      }
    }

    files_exist
  }

  pub fn get_all_tags(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let tag_manager = crate::tag_manager::TAG_MANAGER.lock().unwrap();
    tag_manager.get_all_tags()
  }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_browser_profile_with_group(
  app_handle: tauri::AppHandle,
  name: String,
  browser: String,
  version: String,
  release_type: String,
  proxy_id: Option<String>,
  camoufox_config: Option<CamoufoxConfig>,
  group_id: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .create_profile_with_group(
      &app_handle,
      &name,
      &browser,
      &version,
      &release_type,
      proxy_id,
      camoufox_config,
      group_id,
    )
    .await
    .map_err(|e| format!("Failed to create profile: {e}"))
}

#[tauri::command]
pub fn list_browser_profiles() -> Result<Vec<BrowserProfile>, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))
}

#[tauri::command]
pub async fn launch_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
  url: Option<String>,
) -> Result<BrowserProfile, String> {
  println!(
    "Launch request received for profile: {} (ID: {})",
    profile.name, profile.id
  );

  let browser_runner = BrowserRunner::instance();

  // Store the internal proxy settings for passing to launch_browser
  let mut internal_proxy_settings: Option<ProxySettings> = None;

  // Resolve the most up-to-date profile from disk by ID to avoid using stale proxy_id/browser state
  let profile_for_launch = match browser_runner
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

  println!(
    "Resolved profile for launch: {} (ID: {})",
    profile_for_launch.name, profile_for_launch.id
  );

  // Always start a local proxy before launching (non-Camoufox handled here; Camoufox has its own flow)
  if profile.browser != "camoufox" {
    // Determine upstream proxy if configured; otherwise use DIRECT
    let upstream_proxy = profile_for_launch
      .proxy_id
      .as_ref()
      .and_then(|id| PROXY_MANAGER.get_proxy_settings_by_id(id));

    // Use a temporary PID (1) to start the proxy, we'll update it after browser launch
    let temp_pid = 1u32;

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

        // For Firefox-based browsers, apply PAC/user.js to point to the local proxy
        if matches!(
          profile_for_launch.browser.as_str(),
          "firefox" | "firefox-developer" | "zen" | "tor-browser" | "mullvad-browser"
        ) {
          let profiles_dir = browser_runner.get_profiles_dir();
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
            .apply_proxy_settings_to_profile(&profile_path, &dummy_upstream, Some(&internal_proxy))
            .map_err(|e| format!("Failed to update profile proxy: {e}"))?;
        }

        println!(
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
        eprintln!("Failed to start local proxy (will launch without it): {e}");
      }
    }
  }

  println!(
    "Starting browser launch for profile: {} (ID: {})",
    profile_for_launch.name, profile_for_launch.id
  );

  // Launch browser or open URL in existing instance
  let updated_profile = browser_runner.launch_or_open_url(app_handle.clone(), &profile_for_launch, url, internal_proxy_settings.as_ref()).await.map_err(|e| {
    println!("Browser launch failed for profile: {}, error: {}", profile_for_launch.name, e);

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
      println!("Warning: Failed to emit profile running changed event: {e}");
    }

    // Check if this is an architecture compatibility issue
    if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
      if io_error.kind() == std::io::ErrorKind::Other && io_error.to_string().contains("Exec format error") {
        return format!("Failed to launch browser: Executable format error. This browser version is not compatible with your system architecture ({}). Please try a different browser or version that supports your platform.", std::env::consts::ARCH);
      }
    }
    format!("Failed to launch browser or open URL: {e}")
  })?;

  println!(
    "Browser launch completed for profile: {} (ID: {})",
    updated_profile.name, updated_profile.id
  );

  // Now update the proxy with the correct PID if we have one
  if let Some(actual_pid) = updated_profile.process_id {
    // Update the proxy manager with the correct PID (we always started with temp pid 1 for non-Camoufox)
    let _ = PROXY_MANAGER.update_proxy_pid(1u32, actual_pid);
  }

  Ok(updated_profile)
}

#[tauri::command]
pub async fn update_profile_proxy(
  app_handle: tauri::AppHandle,
  profile_name: String,
  proxy_id: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_proxy(app_handle, &profile_name, proxy_id)
    .await
    .map_err(|e| format!("Failed to update profile: {e}"))
}

#[tauri::command]
pub fn update_profile_tags(
  profile_name: String,
  tags: Vec<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_tags(&profile_name, tags)
    .map_err(|e| format!("Failed to update profile tags: {e}"))
}

#[tauri::command]
pub async fn check_browser_status(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
) -> Result<bool, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .check_browser_status(app_handle, &profile)
    .await
    .map_err(|e| format!("Failed to check browser status: {e}"))
}

#[tauri::command]
pub fn rename_profile(
  _app_handle: tauri::AppHandle,
  old_id: &str,
  new_name: &str,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .rename_profile(old_id, new_name)
    .map_err(|e| format!("Failed to rename profile: {e}"))
}

#[tauri::command]
pub fn delete_profile(_app_handle: tauri::AppHandle, profile_id: String) -> Result<(), String> {
  let browser_runner = BrowserRunner::instance();
  browser_runner
    .delete_profile(profile_id.as_str())
    .map_err(|e| format!("Failed to delete profile: {e}"))
}

#[tauri::command]
pub fn get_supported_browsers() -> Result<Vec<String>, String> {
  let service = BrowserVersionManager::instance();
  Ok(service.get_supported_browsers())
}

#[tauri::command]
pub fn is_browser_supported_on_platform(browser_str: String) -> Result<bool, String> {
  let service = BrowserVersionManager::instance();
  service
    .is_browser_supported(&browser_str)
    .map_err(|e| format!("Failed to check browser support: {e}"))
}

#[tauri::command]
pub async fn fetch_browser_versions_cached_first(
  browser_str: String,
) -> Result<Vec<BrowserVersionInfo>, String> {
  let service = BrowserVersionManager::instance();

  // Get cached versions immediately if available
  if let Some(cached_versions) = service.get_cached_browser_versions_detailed(&browser_str) {
    // Check if we should update cache in background
    if service.should_update_cache(&browser_str) {
      // Start background update but return cached data immediately
      let service_clone = BrowserVersionManager::instance();
      let browser_str_clone = browser_str.clone();
      tokio::spawn(async move {
        if let Err(e) = service_clone
          .fetch_browser_versions_detailed(&browser_str_clone, false)
          .await
        {
          eprintln!("Background version update failed for {browser_str_clone}: {e}");
        }
      });
    }
    Ok(cached_versions)
  } else {
    // No cache available, fetch fresh
    service
      .fetch_browser_versions_detailed(&browser_str, false)
      .await
      .map_err(|e| format!("Failed to fetch detailed browser versions: {e}"))
  }
}

#[tauri::command]
pub async fn fetch_browser_versions_with_count_cached_first(
  browser_str: String,
) -> Result<BrowserVersionsResult, String> {
  let service = BrowserVersionManager::instance();

  // Get cached versions immediately if available
  if let Some(cached_versions) = service.get_cached_browser_versions(&browser_str) {
    // Check if we should update cache in background
    if service.should_update_cache(&browser_str) {
      // Start background update but return cached data immediately
      let service_clone = BrowserVersionManager::instance();
      let browser_str_clone = browser_str.clone();
      tokio::spawn(async move {
        if let Err(e) = service_clone
          .fetch_browser_versions_with_count(&browser_str_clone, false)
          .await
        {
          eprintln!("Background version update failed for {browser_str_clone}: {e}");
        }
      });
    }

    // Return cached data in the expected format
    Ok(BrowserVersionsResult {
      versions: cached_versions.clone(),
      new_versions_count: None, // No new versions when returning cached data
      total_versions_count: cached_versions.len(),
    })
  } else {
    // No cache available, fetch fresh
    service
      .fetch_browser_versions_with_count(&browser_str, false)
      .await
      .map_err(|e| format!("Failed to fetch browser versions: {e}"))
  }
}

#[tauri::command]
pub async fn download_browser(
  app_handle: tauri::AppHandle,
  browser_str: String,
  version: String,
) -> Result<String, String> {
  let browser_runner = BrowserRunner::instance();
  browser_runner
    .download_browser_impl(app_handle, browser_str, version)
    .await
    .map_err(|e| format!("Failed to download browser: {e}"))
}

#[tauri::command]
pub fn is_browser_downloaded(browser_str: String, version: String) -> bool {
  let browser_runner = BrowserRunner::instance();
  browser_runner.is_browser_downloaded(&browser_str, &version)
}

#[tauri::command]
pub fn get_all_tags() -> Result<Vec<String>, String> {
  let browser_runner = BrowserRunner::instance();
  browser_runner
    .get_all_tags()
    .map_err(|e| format!("Failed to get tags: {e}"))
}

#[tauri::command]
pub fn check_browser_exists(browser_str: String, version: String) -> bool {
  // This is an alias for is_browser_downloaded to provide clearer semantics for auto-updates
  is_browser_downloaded(browser_str, version)
}

#[tauri::command]
pub async fn kill_browser_profile(
  app_handle: tauri::AppHandle,
  profile: BrowserProfile,
) -> Result<(), String> {
  println!(
    "Kill request received for profile: {} (ID: {})",
    profile.name, profile.id
  );

  let browser_runner = BrowserRunner::instance();

  match browser_runner
    .kill_browser_process(app_handle.clone(), &profile)
    .await
  {
    Ok(()) => {
      println!(
        "Successfully killed browser profile: {} (ID: {})",
        profile.name, profile.id
      );
      Ok(())
    }
    Err(e) => {
      println!("Failed to kill browser profile {}: {}", profile.name, e);

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
        println!("Warning: Failed to emit profile running changed event: {e}");
      }

      Err(format!("Failed to kill browser: {e}"))
    }
  }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_browser_profile_new(
  app_handle: tauri::AppHandle,
  name: String,
  browser_str: String,
  version: String,
  release_type: String,
  proxy_id: Option<String>,
  camoufox_config: Option<CamoufoxConfig>,
  group_id: Option<String>,
) -> Result<BrowserProfile, String> {
  let browser_type =
    BrowserType::from_str(&browser_str).map_err(|e| format!("Invalid browser type: {e}"))?;
  create_browser_profile_with_group(
    app_handle,
    name,
    browser_type.as_str().to_string(),
    version,
    release_type,
    proxy_id,
    camoufox_config,
    group_id,
  )
  .await
}

#[tauri::command]
pub async fn update_camoufox_config(
  app_handle: tauri::AppHandle,
  profile_id: String,
  config: CamoufoxConfig,
) -> Result<(), String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_camoufox_config(app_handle, &profile_id, config)
    .await
    .map_err(|e| format!("Failed to update Camoufox config: {e}"))
}

#[tauri::command]
pub async fn fetch_browser_versions_with_count(
  browser_str: String,
) -> Result<BrowserVersionsResult, String> {
  let service = BrowserVersionManager::instance();
  service
    .fetch_browser_versions_with_count(&browser_str, false)
    .await
    .map_err(|e| format!("Failed to fetch browser versions: {e}"))
}

#[tauri::command]
pub fn get_downloaded_browser_versions(browser_str: String) -> Result<Vec<String>, String> {
  let registry = DownloadedBrowsersRegistry::instance();
  Ok(registry.get_downloaded_versions(&browser_str))
}

#[tauri::command]
pub async fn check_missing_binaries() -> Result<Vec<(String, String, String)>, String> {
  let browser_runner = BrowserRunner::instance();
  browser_runner
    .check_missing_binaries()
    .await
    .map_err(|e| format!("Failed to check missing binaries: {e}"))
}

#[tauri::command]
pub async fn check_missing_geoip_database() -> Result<bool, String> {
  let browser_runner = BrowserRunner::instance();
  browser_runner
    .check_missing_geoip_database()
    .map_err(|e| format!("Failed to check missing GeoIP database: {e}"))
}

#[tauri::command]
pub async fn ensure_all_binaries_exist(
  app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
  let browser_runner = BrowserRunner::instance();
  browser_runner
    .ensure_all_binaries_exist(&app_handle)
    .await
    .map_err(|e| format!("Failed to ensure all binaries exist: {e}"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn create_test_browser_runner() -> (&'static BrowserRunner, TempDir) {
    let temp_dir = TempDir::new().unwrap();

    // Mock the base directories by setting environment variables
    std::env::set_var("HOME", temp_dir.path());

    let browser_runner = BrowserRunner::instance();
    (browser_runner, temp_dir)
  }

  #[test]
  fn test_get_binaries_dir() {
    let (runner, _temp_dir) = create_test_browser_runner();
    let binaries_dir = runner.get_binaries_dir();

    assert!(binaries_dir.to_string_lossy().contains("DonutBrowser"));
    assert!(binaries_dir.to_string_lossy().contains("binaries"));
  }

  #[test]
  fn test_get_profiles_dir() {
    let (runner, _temp_dir) = create_test_browser_runner();
    let profiles_dir = runner.get_profiles_dir();

    assert!(profiles_dir.to_string_lossy().contains("DonutBrowser"));
    assert!(profiles_dir.to_string_lossy().contains("profiles"));
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_RUNNER: BrowserRunner = BrowserRunner::new();
}

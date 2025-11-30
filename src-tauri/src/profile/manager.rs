use crate::api_client::is_browser_version_nightly;
use crate::browser::{create_browser, BrowserType, ProxySettings};
use crate::camoufox_manager::CamoufoxConfig;
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::profile::types::BrowserProfile;
use crate::proxy_manager::PROXY_MANAGER;
use directories::BaseDirs;
use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};
use sysinfo::{Pid, System};
use tauri::Emitter;

pub struct ProfileManager {
  base_dirs: BaseDirs,
  camoufox_manager: &'static crate::camoufox_manager::CamoufoxManager,
}

impl ProfileManager {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      camoufox_manager: crate::camoufox_manager::CamoufoxManager::instance(),
    }
  }

  pub fn instance() -> &'static ProfileManager {
    &PROFILE_MANAGER
  }

  pub fn get_profiles_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("profiles");
    path
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

  #[allow(clippy::too_many_arguments)]
  pub async fn create_profile_with_group(
    &self,
    app_handle: &tauri::AppHandle,
    name: &str,
    browser: &str,
    version: &str,
    release_type: &str,
    proxy_id: Option<String>,
    camoufox_config: Option<CamoufoxConfig>,
    group_id: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    log::info!("Attempting to create profile: {name}");

    // Check if a profile with this name already exists (case insensitive)
    let existing_profiles = self.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == name.to_lowercase())
    {
      return Err(format!("Profile with name '{name}' already exists").into());
    }

    // Generate a new UUID for this profile
    let profile_id = uuid::Uuid::new_v4();
    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile_id.to_string());
    let profile_data_dir = profile_uuid_dir.join("profile");
    let profile_file = profile_uuid_dir.join("metadata.json");

    // Create profile directory with UUID and profile subdirectory
    create_dir_all(&profile_uuid_dir)?;
    create_dir_all(&profile_data_dir)?;

    // For Camoufox profiles, generate fingerprint during creation
    let final_camoufox_config = if browser == "camoufox" {
      let mut config = camoufox_config.unwrap_or_else(|| {
        log::info!("Creating default Camoufox config for profile: {name}");
        crate::camoufox_manager::CamoufoxConfig::default()
      });

      // Always ensure executable_path is set to the user's binary location
      if config.executable_path.is_none() {
        let mut browser_dir = self.get_binaries_dir();
        browser_dir.push(browser);
        browser_dir.push(version);

        #[cfg(target_os = "macos")]
        let binary_path = browser_dir
          .join("Camoufox.app")
          .join("Contents")
          .join("MacOS")
          .join("camoufox");

        #[cfg(target_os = "windows")]
        let binary_path = browser_dir.join("camoufox.exe");

        #[cfg(target_os = "linux")]
        let binary_path = browser_dir.join("camoufox");

        config.executable_path = Some(binary_path.to_string_lossy().to_string());
        log::info!("Set Camoufox executable path: {:?}", config.executable_path);
      }

      // Pass upstream proxy information to config for fingerprint generation
      if let Some(proxy_id_ref) = &proxy_id {
        if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_ref) {
          // For fingerprint generation, pass upstream proxy directly with credentials if present
          let proxy_url = if let (Some(username), Some(password)) =
            (&proxy_settings.username, &proxy_settings.password)
          {
            format!(
              "{}://{}:{}@{}:{}",
              proxy_settings.proxy_type.to_lowercase(),
              username,
              password,
              proxy_settings.host,
              proxy_settings.port
            )
          } else {
            format!(
              "{}://{}:{}",
              proxy_settings.proxy_type.to_lowercase(),
              proxy_settings.host,
              proxy_settings.port
            )
          };
          config.proxy = Some(proxy_url);
          log::info!(
            "Using upstream proxy for Camoufox fingerprint generation: {}://{}:{}",
            proxy_settings.proxy_type.to_lowercase(),
            proxy_settings.host,
            proxy_settings.port
          );
        }
      }

      // Generate fingerprint if not already provided
      if config.fingerprint.is_none() {
        log::info!("Generating fingerprint for Camoufox profile: {name}");

        // Use the camoufox launcher to generate the config

        // Create a temporary profile for fingerprint generation
        let temp_profile = BrowserProfile {
          id: uuid::Uuid::new_v4(),
          name: name.to_string(),
          browser: browser.to_string(),
          version: version.to_string(),
          proxy_id: proxy_id.clone(),
          process_id: None,
          last_launch: None,
          release_type: release_type.to_string(),
          camoufox_config: None,
          group_id: group_id.clone(),
          tags: Vec::new(),
          note: None,
        };

        match self
          .camoufox_manager
          .generate_fingerprint_config(app_handle, &temp_profile, &config)
          .await
        {
          Ok(generated_fingerprint) => {
            config.fingerprint = Some(generated_fingerprint);
            log::info!("Successfully generated fingerprint for profile: {name}");
          }
          Err(e) => {
            return Err(
              format!("Failed to generate fingerprint for Camoufox profile '{name}': {e}").into(),
            );
          }
        }
      } else {
        log::info!("Using provided fingerprint for Camoufox profile: {name}");
      }

      // Clear the proxy from config after fingerprint generation
      // Browser launch should always use local proxy, never direct to upstream
      config.proxy = None;

      Some(config)
    } else {
      camoufox_config.clone()
    };

    let profile = BrowserProfile {
      id: profile_id,
      name: name.to_string(),
      browser: browser.to_string(),
      version: version.to_string(),
      proxy_id: proxy_id.clone(),
      process_id: None,
      last_launch: None,
      release_type: release_type.to_string(),
      camoufox_config: final_camoufox_config,
      group_id: group_id.clone(),
      tags: Vec::new(),
      note: None,
    };

    // Save profile info
    self.save_profile(&profile)?;

    // Verify the profile was saved correctly
    if !profile_file.exists() {
      return Err(format!("Failed to create profile file for '{name}'").into());
    }

    log::info!("Profile '{name}' created successfully with ID: {profile_id}");

    // Create user.js with common Firefox preferences and apply proxy settings if provided
    if let Some(proxy_id_ref) = &proxy_id {
      if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_ref) {
        self.apply_proxy_settings_to_profile(&profile_data_dir, &proxy_settings, None)?;
      } else {
        // Proxy ID provided but not found, disable proxy
        self.disable_proxy_settings_in_profile(&profile_data_dir)?;
      }
    } else {
      // Create user.js with common Firefox preferences but no proxy
      self.disable_proxy_settings_in_profile(&profile_data_dir)?;
    }

    // Emit profile creation event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn save_profile(&self, profile: &BrowserProfile) -> Result<(), Box<dyn std::error::Error>> {
    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile.id.to_string());
    let profile_file = profile_uuid_dir.join("metadata.json");

    // Ensure the UUID directory exists
    create_dir_all(&profile_uuid_dir)?;

    let json = serde_json::to_string_pretty(profile)?;
    fs::write(profile_file, json)?;

    // Update tag suggestions after any save
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    Ok(())
  }

  pub fn list_profiles(&self) -> Result<Vec<BrowserProfile>, Box<dyn std::error::Error>> {
    let profiles_dir = self.get_profiles_dir();
    if !profiles_dir.exists() {
      return Ok(vec![]);
    }

    let mut profiles = Vec::new();
    for entry in fs::read_dir(profiles_dir)? {
      let entry = entry?;
      let path = entry.path();

      // Look for UUID directories containing metadata.json
      if path.is_dir() {
        let metadata_file = path.join("metadata.json");
        if metadata_file.exists() {
          let content = fs::read_to_string(metadata_file)?;
          let profile: BrowserProfile = serde_json::from_str(&content)?;
          profiles.push(profile);
        }
      }
    }

    Ok(profiles)
  }

  pub fn rename_profile(
    &self,
    app_handle: &tauri::AppHandle,
    profile_id: &str,
    new_name: &str,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Check if new name already exists (case insensitive)
    let existing_profiles = self.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == new_name.to_lowercase())
    {
      return Err(format!("Profile with name '{new_name}' already exists").into());
    }

    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let mut profile = existing_profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Update profile name (no need to move directories since we use UUID)
    profile.name = new_name.to_string();

    // Save profile with new name
    self.save_profile(&profile)?;

    // Keep tag suggestions up to date after name change (rebuild from all profiles)
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    // Emit profile rename event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn delete_profile(
    &self,
    app_handle: &tauri::AppHandle,
    profile_id: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Attempting to delete profile with ID: {profile_id}");

    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Check if browser is running
    if profile.process_id.is_some() {
      return Err(
        "Cannot delete profile while browser is running. Please stop the browser first.".into(),
      );
    }

    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile.id.to_string());

    // Delete the entire UUID directory (contains both metadata.json and profile data)
    if profile_uuid_dir.exists() {
      log::info!("Deleting profile directory: {}", profile_uuid_dir.display());
      fs::remove_dir_all(&profile_uuid_dir)?;
      log::info!("Profile directory deleted successfully");
    }

    // Verify deletion was successful
    if profile_uuid_dir.exists() {
      return Err(format!("Failed to completely delete profile '{}'", profile.name).into());
    }

    log::info!(
      "Profile '{}' (ID: {}) deleted successfully",
      profile.name,
      profile_id
    );

    // Rebuild tag suggestions after deletion
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    // Always perform cleanup after profile deletion to remove unused binaries
    if let Err(e) = DownloadedBrowsersRegistry::instance().cleanup_unused_binaries() {
      log::warn!("Warning: Failed to cleanup unused binaries after profile deletion: {e}");
    }

    // Emit profile deletion event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(())
  }

  pub fn update_profile_version(
    &self,
    app_handle: &tauri::AppHandle,
    profile_id: &str,
    version: &str,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Check if the browser is currently running
    if profile.process_id.is_some() {
      return Err(
        "Cannot update version while browser is running. Please stop the browser first.".into(),
      );
    }

    // Verify the new version is downloaded
    let browser_type = BrowserType::from_str(&profile.browser)
      .map_err(|_| format!("Invalid browser type: {}", profile.browser))?;
    let browser = create_browser(browser_type.clone());
    let binaries_dir = self.get_binaries_dir();

    if !browser.is_version_downloaded(version, &binaries_dir) {
      return Err(format!("Browser version {version} is not downloaded").into());
    }

    // Update version
    profile.version = version.to_string();

    // Update the release_type based on the version and browser
    profile.release_type = if is_browser_version_nightly(&profile.browser, version, None) {
      "nightly".to_string()
    } else {
      "stable".to_string()
    };

    // Save the updated profile
    self.save_profile(&profile)?;

    // Emit profile update event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn assign_profiles_to_group(
    &self,
    app_handle: &tauri::AppHandle,
    profile_ids: Vec<String>,
    group_id: Option<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;

    for profile_id in profile_ids {
      let profile_uuid = uuid::Uuid::parse_str(&profile_id)
        .map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
      let mut profile = profiles
        .iter()
        .find(|p| p.id == profile_uuid)
        .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?
        .clone();

      // Check if browser is running
      if profile.process_id.is_some() {
        return Err(format!(
          "Cannot modify group for profile '{}' while browser is running. Please stop the browser first.", profile.name
        ).into());
      }

      profile.group_id = group_id.clone();
      self.save_profile(&profile)?;
    }

    // Rebuild tag suggestions after group changes just in case
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    // Emit profile group assignment event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(())
  }

  pub fn update_profile_tags(
    &self,
    app_handle: &tauri::AppHandle,
    profile_id: &str,
    tags: Vec<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<String> = Vec::with_capacity(tags.len());
    for t in tags.into_iter() {
      if seen.insert(t.clone()) {
        deduped.push(t);
      }
    }
    profile.tags = deduped;

    // Save profile
    self.save_profile(&profile)?;

    // Update global tag suggestions from all profiles
    let _ = crate::tag_manager::TAG_MANAGER.lock().map(|tm| {
      let _ = tm.rebuild_from_profiles(&self.list_profiles().unwrap_or_default());
    });

    // Emit profile tags update event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn update_profile_note(
    &self,
    app_handle: &tauri::AppHandle,
    profile_id: &str,
    note: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Find the profile by ID
    let profile_uuid =
      uuid::Uuid::parse_str(profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

    // Update note (trim whitespace, set to None if empty)
    profile.note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());

    // Save profile
    self.save_profile(&profile)?;

    // Emit profile note update event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub fn delete_multiple_profiles(
    &self,
    app_handle: &tauri::AppHandle,
    profile_ids: Vec<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;

    for profile_id in profile_ids {
      let profile_uuid = uuid::Uuid::parse_str(&profile_id)
        .map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
      let profile = profiles
        .iter()
        .find(|p| p.id == profile_uuid)
        .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

      // Check if browser is running
      if profile.process_id.is_some() {
        return Err(
          format!(
            "Cannot delete profile '{}' while browser is running. Please stop the browser first.",
            profile.name
          )
          .into(),
        );
      }

      // Delete the profile
      let profiles_dir = self.get_profiles_dir();
      let profile_uuid_dir = profiles_dir.join(profile.id.to_string());

      if profile_uuid_dir.exists() {
        std::fs::remove_dir_all(&profile_uuid_dir)?;
      }
    }

    // Emit profile deletion event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(())
  }

  pub async fn update_camoufox_config(
    &self,
    app_handle: tauri::AppHandle,
    profile_id: &str,
    config: CamoufoxConfig,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Find the profile by ID
    let profile_uuid = uuid::Uuid::parse_str(profile_id).map_err(
      |_| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Invalid profile ID: {profile_id}").into()
      },
    )?;
    let profiles =
      self
        .list_profiles()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to list profiles: {e}").into()
        })?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Profile with ID '{profile_id}' not found").into()
      })?;

    // Check if the browser is currently running using the comprehensive status check
    let is_running = self
      .check_browser_status(app_handle.clone(), &profile)
      .await?;

    if is_running {
      return Err(
        "Cannot update Camoufox configuration while browser is running. Please stop the browser first.".into(),
      );
    }

    // Update the Camoufox configuration
    profile.camoufox_config = Some(config);

    // Save the updated profile
    self
      .save_profile(&profile)
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Failed to save profile: {e}").into()
      })?;

    log::info!(
      "Camoufox configuration updated for profile '{}' (ID: {}).",
      profile.name,
      profile_id
    );

    // Emit profile config update event
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(())
  }

  pub async fn update_profile_proxy(
    &self,
    app_handle: tauri::AppHandle,
    profile_id: &str,
    proxy_id: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Find the profile by ID
    let profile_uuid = uuid::Uuid::parse_str(profile_id).map_err(
      |_| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Invalid profile ID: {profile_id}").into()
      },
    )?;
    let profiles =
      self
        .list_profiles()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to list profiles: {e}").into()
        })?;

    let mut profile = profiles
      .into_iter()
      .find(|p| p.id == profile_uuid)
      .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Profile with ID '{profile_id}' not found").into()
      })?;

    // Update proxy settings
    profile.proxy_id = proxy_id.clone();

    // Save the updated profile
    self
      .save_profile(&profile)
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Failed to save profile: {e}").into()
      })?;

    // Update on-disk browser profile config immediately
    if let Some(proxy_id_ref) = &proxy_id {
      if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_ref) {
        let profiles_dir = self.get_profiles_dir();
        let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
        self
          .apply_proxy_settings_to_profile(&profile_path, &proxy_settings, None)
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to apply proxy settings: {e}").into()
          })?;
      } else {
        // Proxy ID provided but proxy not found, disable proxy
        let profiles_dir = self.get_profiles_dir();
        let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
        self
          .disable_proxy_settings_in_profile(&profile_path)
          .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to disable proxy settings: {e}").into()
          })?;
      }
    } else {
      // No proxy ID provided, disable proxy
      let profiles_dir = self.get_profiles_dir();
      let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
      self
        .disable_proxy_settings_in_profile(&profile_path)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to disable proxy settings: {e}").into()
        })?;
    }

    // Emit profile update event so frontend UIs can refresh immediately (e.g. proxy manager)
    if let Err(e) = app_handle.emit("profile-updated", &profile) {
      log::warn!("Warning: Failed to emit profile update event: {e}");
    }

    // Emit general profiles changed event for profile list updates
    if let Err(e) = app_handle.emit("profiles-changed", ()) {
      log::warn!("Warning: Failed to emit profiles-changed event: {e}");
    }

    Ok(profile)
  }

  pub async fn check_browser_status(
    &self,
    app_handle: tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Handle Camoufox profiles using nodecar-based status checking
    if profile.browser == "camoufox" {
      return self
        .check_camoufox_status_via_nodecar(&app_handle, profile)
        .await;
    }

    // For non-camoufox browsers, use the existing PID-based logic
    let inner_profile = profile.clone();
    let system = System::new_all();
    let mut is_running = false;
    let mut found_pid: Option<u32> = None;

    // First check if the stored PID is still valid
    if let Some(pid) = profile.process_id {
      if let Some(process) = system.process(Pid::from(pid as usize)) {
        let cmd = process.cmd();
        // Verify this process is actually our browser with the correct profile
        let profiles_dir = self.get_profiles_dir();
        let profile_data_path = profile.get_profile_data_path(&profiles_dir);
        let profile_data_path_str = profile_data_path.to_string_lossy();
        let profile_path_match = cmd.iter().any(|s| {
          let arg = s.to_str().unwrap_or("");
          // For Firefox-based browsers, check for exact profile path match
          if profile.browser == "tor-browser"
            || profile.browser == "firefox"
            || profile.browser == "firefox-developer"
            || profile.browser == "mullvad-browser"
            || profile.browser == "zen"
          {
            arg == profile_data_path_str
              || arg == format!("-profile={profile_data_path_str}")
              || (arg == "-profile"
                && cmd
                  .iter()
                  .any(|s2| s2.to_str().unwrap_or("") == profile_data_path_str))
          } else {
            // For Chromium-based browsers, check for user-data-dir
            arg.contains(&format!("--user-data-dir={profile_data_path_str}"))
              || arg == profile_data_path_str
          }
        });

        if profile_path_match {
          is_running = true;
          found_pid = Some(pid);
          // Found existing browser process
        }
      }
    }

    // If we didn't find the browser with the stored PID, search all processes
    if !is_running {
      for (pid, process) in system.processes() {
        let cmd = process.cmd();
        if cmd.len() >= 2 {
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
            "firefox-developer" => exe_name.contains("firefox") && exe_name.contains("developer"),
            "mullvad-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "mullvad-browser"),
            "tor-browser" => self.is_tor_or_mullvad_browser(&exe_name, cmd, "tor-browser"),
            "zen" => exe_name.contains("zen"),
            "chromium" => exe_name.contains("chromium"),
            "brave" => exe_name.contains("brave"),
            // Camoufox is handled via nodecar, not PID-based checking
            _ => false,
          };

          if !is_correct_browser {
            continue;
          }

          // Check for profile path match
          let profiles_dir = self.get_profiles_dir();
          let profile_data_path = profile.get_profile_data_path(&profiles_dir);
          let profile_data_path_str = profile_data_path.to_string_lossy();
          let profile_path_match = cmd.iter().any(|s| {
            let arg = s.to_str().unwrap_or("");
            // For Firefox-based browsers, check for exact profile path match
            if profile.browser == "camoufox" {
              // Camoufox uses user_data_dir like Chromium browsers
              arg.contains(&format!("--user-data-dir={profile_data_path_str}"))
                || arg == profile_data_path_str
            } else if profile.browser == "tor-browser"
              || profile.browser == "firefox"
              || profile.browser == "firefox-developer"
              || profile.browser == "mullvad-browser"
              || profile.browser == "zen"
            {
              arg == profile_data_path_str
                || arg == format!("-profile={profile_data_path_str}")
                || (arg == "-profile"
                  && cmd
                    .iter()
                    .any(|s2| s2.to_str().unwrap_or("") == profile_data_path_str))
            } else {
              // For Chromium-based browsers, check for user-data-dir
              arg.contains(&format!("--user-data-dir={profile_data_path_str}"))
                || arg == profile_data_path_str
            }
          });

          if profile_path_match {
            // Found a matching process
            found_pid = Some(pid.as_u32());
            is_running = true;
            log::info!(
              "Found browser process with PID: {} for profile: {}",
              pid.as_u32(),
              profile.name
            );
            break;
          }
        }
      }
    }

    // Only persist status changes if the profile metadata still exists on disk
    let profiles_dir = self.get_profiles_dir();
    let profile_uuid_dir = profiles_dir.join(profile.id.to_string());
    let metadata_file = profile_uuid_dir.join("metadata.json");
    let metadata_exists = metadata_file.exists();

    if metadata_exists {
      // Load the latest profile from disk to avoid overwriting fields like proxy_id
      let latest_profile: BrowserProfile = match std::fs::read_to_string(&metadata_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
      {
        Some(p) => p,
        None => inner_profile.clone(),
      };

      let previous_pid = latest_profile.process_id;
      let mut merged = latest_profile.clone();

      if let Some(pid) = found_pid {
        if merged.process_id != Some(pid) {
          merged.process_id = Some(pid);
          if let Err(e) = self.save_profile(&merged) {
            log::warn!("Warning: Failed to update profile with new PID: {e}");
          }
        }
      } else if merged.process_id.is_some() {
        // Clear the PID if no process found
        merged.process_id = None;
        if let Err(e) = self.save_profile(&merged) {
          log::warn!("Warning: Failed to clear profile PID: {e}");
        }

        // Stop any associated proxy immediately when the browser stops
        if let Some(old_pid) = previous_pid {
          let _ = crate::proxy_manager::PROXY_MANAGER
            .stop_proxy(app_handle.clone(), old_pid)
            .await;
        }
      }

      // Emit profile update event to frontend
      if let Err(e) = app_handle.emit("profile-updated", &merged) {
        log::warn!("Warning: Failed to emit profile update event: {e}");
      }
    }

    Ok(is_running)
  }

  // Check Camoufox status using nodecar-based approach
  async fn check_camoufox_status_via_nodecar(
    &self,
    app_handle: &tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let launcher = self.camoufox_manager;
    let profiles_dir = self.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    let profile_path_str = profile_data_path.to_string_lossy();

    // Check if there's a running Camoufox instance for this profile
    match launcher.find_camoufox_by_profile(&profile_path_str).await {
      Ok(Some(camoufox_process)) => {
        // Found a running instance, update profile with process info if changed
        let profiles_dir = self.get_profiles_dir();
        let profile_uuid_dir = profiles_dir.join(profile.id.to_string());
        let metadata_file = profile_uuid_dir.join("metadata.json");
        let metadata_exists = metadata_file.exists();

        if metadata_exists {
          // Load latest to avoid overwriting other fields
          let mut latest: BrowserProfile = match std::fs::read_to_string(&metadata_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
          {
            Some(p) => p,
            None => profile.clone(),
          };

          if latest.process_id != camoufox_process.processId {
            latest.process_id = camoufox_process.processId;
            if let Err(e) = self.save_profile(&latest) {
              log::warn!("Warning: Failed to update Camoufox profile with process info: {e}");
            }

            // Emit profile update event to frontend
            if let Err(e) = app_handle.emit("profile-updated", &latest) {
              log::warn!("Warning: Failed to emit profile update event: {e}");
            }

            log::info!(
              "Camoufox process has started for profile '{}' with PID: {:?}",
              profile.name,
              camoufox_process.processId
            );
          }
        }
        Ok(true)
      }
      Ok(None) => {
        // No running instance found, clear process ID if set and stop proxy
        let profiles_dir = self.get_profiles_dir();
        let profile_uuid_dir = profiles_dir.join(profile.id.to_string());
        let metadata_file = profile_uuid_dir.join("metadata.json");
        let metadata_exists = metadata_file.exists();

        if metadata_exists {
          let mut latest: BrowserProfile = match std::fs::read_to_string(&metadata_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
          {
            Some(p) => p,
            None => profile.clone(),
          };

          if let Some(old_pid) = latest.process_id {
            latest.process_id = None;
            if let Err(e) = self.save_profile(&latest) {
              log::warn!("Warning: Failed to clear Camoufox profile process info: {e}");
            }

            // Stop any proxy tied to this old PID immediately
            let _ = crate::proxy_manager::PROXY_MANAGER
              .stop_proxy(app_handle.clone(), old_pid)
              .await;

            // Emit profile update event to frontend
            if let Err(e) = app_handle.emit("profile-updated", &latest) {
              log::warn!("Warning: Failed to emit profile update event: {e}");
            }
          }
        }
        Ok(false)
      }
      Err(e) => {
        // Error checking status, assume not running and clear process ID
        log::warn!("Warning: Failed to check Camoufox status via nodecar: {e}");
        let profiles_dir = self.get_profiles_dir();
        let profile_uuid_dir = profiles_dir.join(profile.id.to_string());
        let metadata_file = profile_uuid_dir.join("metadata.json");
        let metadata_exists = metadata_file.exists();

        if metadata_exists {
          let mut latest: BrowserProfile = match std::fs::read_to_string(&metadata_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
          {
            Some(p) => p,
            None => profile.clone(),
          };

          if let Some(old_pid) = latest.process_id {
            latest.process_id = None;
            if let Err(e2) = self.save_profile(&latest) {
              log::warn!(
                "Warning: Failed to clear Camoufox profile process info after error: {e2}"
              );
            }

            // Best-effort stop of proxy tied to old PID
            let _ = crate::proxy_manager::PROXY_MANAGER
              .stop_proxy(app_handle.clone(), old_pid)
              .await;

            // Emit profile update event to frontend
            if let Err(e3) = app_handle.emit("profile-updated", &latest) {
              log::warn!("Warning: Failed to emit profile update event: {e3}");
            }
          }
        }
        Ok(false)
      }
    }
  }

  // Helper function to check if a process matches TOR/Mullvad browser
  fn is_tor_or_mullvad_browser(
    &self,
    exe_name: &str,
    cmd: &[std::ffi::OsString],
    browser_type: &str,
  ) -> bool {
    #[cfg(target_os = "macos")]
    return crate::platform_browser::macos::is_tor_or_mullvad_browser(exe_name, cmd, browser_type);

    #[cfg(target_os = "windows")]
    return crate::platform_browser::windows::is_tor_or_mullvad_browser(
      exe_name,
      cmd,
      browser_type,
    );

    #[cfg(target_os = "linux")]
    return crate::platform_browser::linux::is_tor_or_mullvad_browser(exe_name, cmd, browser_type);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
      let _ = (exe_name, cmd, browser_type);
      false
    }
  }

  fn get_common_firefox_preferences(&self) -> Vec<String> {
    vec![
      // Disable default browser check
      "user_pref(\"browser.shell.checkDefaultBrowser\", false);".to_string(),
      "user_pref(\"browser.shell.skipDefaultBrowserCheckOnFirstRun\", true);".to_string(),
      "user_pref(\"browser.preferences.moreFromMozilla\", false);".to_string(),
      "user_pref(\"services.sync.prefs.sync.browser.startup.upgradeDialog.enabled\", false);"
        .to_string(),
      // Disable welcome / first-run screens
      "user_pref(\"browser.aboutwelcome.enabled\", false);".to_string(),
      "user_pref(\"browser.startup.homepage_override.mstone\", \"ignore\");".to_string(),
      "user_pref(\"startup.homepage_welcome_url\", \"\");".to_string(),
      "user_pref(\"startup.homepage_welcome_url.additional\", \"\");".to_string(),
      "user_pref(\"startup.homepage_override_url\", \"\");".to_string(),
      // Keep extension updates enabled
      "user_pref(\"extensions.update.enabled\", true);".to_string(),
      "user_pref(\"extensions.update.autoUpdateDefault\", true);".to_string(),
      // Completely disable browser update checking
      "user_pref(\"app.update.enabled\", false);".to_string(),
      "user_pref(\"app.update.auto\", false);".to_string(),
      "user_pref(\"app.update.mode\", 0);".to_string(),
      "user_pref(\"app.update.service.enabled\", false);".to_string(),
      "user_pref(\"app.update.staging.enabled\", false);".to_string(),
      "user_pref(\"app.update.silent\", true);".to_string(),
      "user_pref(\"app.update.disabledForTesting\", true);".to_string(),
      // Prevent update URL access entirely
      "user_pref(\"app.update.url\", \"\");".to_string(),
      "user_pref(\"app.update.url.manual\", \"\");".to_string(),
      "user_pref(\"app.update.url.details\", \"\");".to_string(),
      // Disable update timing/scheduling
      "user_pref(\"app.update.timerFirstInterval\", 999999999);".to_string(),
      "user_pref(\"app.update.interval\", 999999999);".to_string(),
      "user_pref(\"app.update.background.interval\", 999999999);".to_string(),
      "user_pref(\"app.update.idletime\", 999999999);".to_string(),
      "user_pref(\"app.update.promptWaitTime\", 999999999);".to_string(),
      // Disable update attempts
      "user_pref(\"app.update.download.maxAttempts\", 0);".to_string(),
      "user_pref(\"app.update.elevate.maxAttempts\", 0);".to_string(),
      "user_pref(\"app.update.checkInstallTime\", false);".to_string(),
      // Suppress update UI/prompts/notifications
      "user_pref(\"app.update.doorhanger\", false);".to_string(),
      "user_pref(\"app.update.badge\", false);".to_string(),
      "user_pref(\"app.update.notifyDuringDownload\", false);".to_string(),
      "user_pref(\"app.update.background.scheduling.enabled\", false);".to_string(),
      "user_pref(\"app.update.background.enabled\", false);".to_string(),
      // Disable BITS (Windows Background Intelligent Transfer Service) updates
      "user_pref(\"app.update.BITS.enabled\", false);".to_string(),
      // Disable language pack updates
      "user_pref(\"app.update.langpack.enabled\", false);".to_string(),
      // Suppress upgrade dialogs on startup
      "user_pref(\"browser.startup.upgradeDialog.enabled\", false);".to_string(),
      // Disable update ping telemetry
      "user_pref(\"toolkit.telemetry.updatePing.enabled\", false);".to_string(),
      // Zen browser specific - disable welcome screen and updates
      "user_pref(\"zen.welcome-screen.seen\", true);".to_string(),
      "user_pref(\"zen.updates.enabled\", false);".to_string(),
      "user_pref(\"zen.updates.check-for-updates\", false);".to_string(),
      // Additional first-run suppressions
      "user_pref(\"app.normandy.first_run\", false);".to_string(),
      "user_pref(\"trailhead.firstrun.didSeeAboutWelcome\", true);".to_string(),
      "user_pref(\"datareporting.policy.dataSubmissionPolicyBypassNotification\", true);"
        .to_string(),
      "user_pref(\"toolkit.telemetry.reportingpolicy.firstRun\", false);".to_string(),
      // Disable quit confirmation dialogs
      "user_pref(\"browser.warnOnQuit\", false);".to_string(),
      "user_pref(\"browser.showQuitWarning\", false);".to_string(),
      "user_pref(\"browser.tabs.warnOnClose\", false);".to_string(),
      "user_pref(\"browser.tabs.warnOnCloseOtherTabs\", false);".to_string(),
      "user_pref(\"browser.sessionstore.warnOnQuit\", false);".to_string(),
    ]
  }

  pub fn apply_proxy_settings_to_profile(
    &self,
    profile_data_path: &Path,
    proxy: &ProxySettings,
    internal_proxy: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let user_js_path = profile_data_path.join("user.js");
    let prefs_js_path = profile_data_path.join("prefs.js");

    // Remove prefs.js if it exists to ensure Firefox reads user.js instead
    // Firefox may cache proxy settings in prefs.js, so we need to clear it
    if prefs_js_path.exists() {
      log::info!("Removing prefs.js to ensure Firefox reads updated user.js settings");
      let _ = fs::remove_file(&prefs_js_path);
    }

    let mut preferences = Vec::new();

    // Get the UUID directory (parent of profile data directory)
    let uuid_dir = profile_data_path
      .parent()
      .ok_or("Invalid profile path - cannot find UUID directory")?;

    // Add common Firefox preferences (like disabling default browser check)
    preferences.extend(self.get_common_firefox_preferences());

    // Use embedded PAC template instead of reading from file
    const PAC_TEMPLATE: &str = r#"function FindProxyForURL(url, host) {
  return "{{proxy_url}}";
}"#;

    // Format proxy URL based on type and whether we have an internal proxy
    let proxy_url = if let Some(internal) = internal_proxy {
      // Use internal proxy (local proxy) as the primary proxy
      // This is the local proxy that forwards to the upstream proxy
      log::info!(
        "Applying local proxy settings to Firefox profile: {}:{}",
        internal.host,
        internal.port
      );
      format!("HTTP {}:{}", internal.host, internal.port)
    } else {
      // Use user-configured proxy directly (upstream proxy)
      log::info!(
        "Applying upstream proxy settings to Firefox profile: {}:{} ({})",
        proxy.host,
        proxy.port,
        proxy.proxy_type
      );
      match proxy.proxy_type.as_str() {
        "http" => format!("HTTP {}:{}", proxy.host, proxy.port),
        "https" => format!("HTTPS {}:{}", proxy.host, proxy.port),
        "socks4" => format!("SOCKS4 {}:{}", proxy.host, proxy.port),
        "socks5" => format!("SOCKS5 {}:{}", proxy.host, proxy.port),
        _ => return Err(format!("Unsupported proxy type: {}", proxy.proxy_type).into()),
      }
    };

    // Replace placeholders in PAC file
    let pac_content = PAC_TEMPLATE
      .replace("{{proxy_url}}", &proxy_url)
      .replace("{{proxy_credentials}}", ""); // Credentials are now handled by the PAC file

    // Save PAC file in UUID directory
    let pac_path = uuid_dir.join("proxy.pac");
    log::info!(
      "Creating PAC file at: {} with proxy: {}",
      pac_path.display(),
      proxy_url
    );
    fs::write(&pac_path, &pac_content)?;
    log::info!(
      "Created PAC file at: {} with content: {}",
      pac_path.display(),
      pac_content
    );

    // Configure Firefox to use the PAC file
    // Convert path to absolute and properly format for file:// URL
    let pac_path_absolute = pac_path.canonicalize().unwrap_or_else(|_| pac_path.clone());
    let pac_url = if cfg!(windows) {
      // Windows: file:///C:/path/to/file.pac
      format!(
        "file:///{}",
        pac_path_absolute.to_string_lossy().replace('\\', "/")
      )
    } else {
      // Unix/macOS: file:///absolute/path/to/file.pac (three slashes for absolute path)
      format!("file://{}", pac_path_absolute.to_string_lossy())
    };

    log::info!("PAC file path (absolute): {}", pac_path_absolute.display());
    log::info!("PAC file URL for Firefox: {}", pac_url);

    preferences.extend([
      "user_pref(\"network.proxy.type\", 2);".to_string(),
      format!(
        "user_pref(\"network.proxy.autoconfig_url\", \"{}\");",
        pac_url
      ),
      "user_pref(\"network.proxy.failover_direct\", false);".to_string(),
      "user_pref(\"network.proxy.socks_remote_dns\", true);".to_string(),
      "user_pref(\"network.proxy.no_proxies_on\", \"\");".to_string(),
      "user_pref(\"signon.autologin.proxy\", true);".to_string(),
      "user_pref(\"network.proxy.share_proxy_settings\", false);".to_string(),
      "user_pref(\"network.automatic-ntlm-auth.allow-proxies\", false);".to_string(),
      "user_pref(\"network.auth-use-sspi\", false);".to_string(),
    ]);

    // Write settings to user.js file
    let user_js_content = preferences.join("\n");
    fs::write(user_js_path, &user_js_content)?;
    log::info!("Updated user.js with proxy settings. PAC URL: {}", pac_url);
    if let Some(internal) = internal_proxy {
      log::info!(
        "Firefox will use LOCAL proxy: {}:{} (which forwards to upstream)",
        internal.host,
        internal.port
      );
    } else {
      log::info!(
        "Firefox will use UPSTREAM proxy directly: {}:{}",
        proxy.host,
        proxy.port
      );
    }

    Ok(())
  }

  pub fn disable_proxy_settings_in_profile(
    &self,
    profile_data_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let user_js_path = profile_data_path.join("user.js");
    let mut preferences = Vec::new();

    // Get the UUID directory (parent of profile data directory)
    let uuid_dir = profile_data_path
      .parent()
      .ok_or("Invalid profile path - cannot find UUID directory")?;

    // Add common Firefox preferences (like disabling default browser check)
    preferences.extend(self.get_common_firefox_preferences());

    preferences.push("user_pref(\"network.proxy.type\", 0);".to_string());
    preferences.push("user_pref(\"network.proxy.failover_direct\", true);".to_string());

    // Create a direct proxy PAC file in UUID directory
    let pac_content = "function FindProxyForURL(url, host) { return 'DIRECT'; }";
    let pac_path = uuid_dir.join("proxy.pac");
    fs::write(&pac_path, pac_content)?;
    preferences.push(format!(
      "user_pref(\"network.proxy.autoconfig_url\", \"file://{}\");",
      pac_path.to_string_lossy()
    ));

    fs::write(user_js_path, preferences.join("\n"))?;

    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use tempfile::TempDir;

  fn create_test_profile_manager() -> (&'static ProfileManager, TempDir) {
    let temp_dir = TempDir::new().unwrap();

    // Mock the base directories by setting environment variables
    std::env::set_var("HOME", temp_dir.path());

    let profile_manager = ProfileManager::instance();
    (profile_manager, temp_dir)
  }

  #[test]
  fn test_profile_manager_creation() {
    let (_manager, _temp_dir) = create_test_profile_manager();
    // If we get here without panicking, the test passes
  }

  #[test]
  fn test_get_profiles_dir() {
    let (manager, _temp_dir) = create_test_profile_manager();
    let profiles_dir = manager.get_profiles_dir();

    assert!(
      profiles_dir.to_string_lossy().contains("DonutBrowser"),
      "Profiles dir should contain DonutBrowser"
    );
    assert!(
      profiles_dir.to_string_lossy().contains("profiles"),
      "Profiles dir should contain profiles"
    );
  }

  #[test]
  fn test_get_common_firefox_preferences() {
    let (manager, _temp_dir) = create_test_profile_manager();

    let prefs = manager.get_common_firefox_preferences();
    assert!(!prefs.is_empty(), "Should return non-empty preferences");

    // Check for some expected preferences
    let prefs_string = prefs.join("\n");
    assert!(
      prefs_string.contains("browser.shell.checkDefaultBrowser"),
      "Should contain default browser check preference"
    );
    assert!(
      prefs_string.contains("app.update.enabled"),
      "Should contain update preference"
    );
  }

  #[test]
  fn test_get_binaries_dir() {
    let (manager, _temp_dir) = create_test_profile_manager();

    let binaries_dir = manager.get_binaries_dir();
    let path_str = binaries_dir.to_string_lossy();

    assert!(
      path_str.contains("DonutBrowser"),
      "Binaries dir should contain DonutBrowser"
    );
    assert!(
      path_str.contains("binaries"),
      "Binaries dir should contain binaries"
    );
  }

  #[test]
  fn test_disable_proxy_settings_in_profile() {
    let (manager, temp_dir) = create_test_profile_manager();

    // Create a test profile directory
    let profile_dir = temp_dir.path().join("test_profile");
    fs::create_dir_all(&profile_dir).expect("Should create profile directory");

    let result = manager.disable_proxy_settings_in_profile(&profile_dir);
    assert!(result.is_ok(), "Should successfully disable proxy settings");

    // Check that user.js was created
    let user_js_path = profile_dir.join("user.js");
    assert!(user_js_path.exists(), "user.js should be created");

    let content = fs::read_to_string(&user_js_path).expect("Should read user.js");
    assert!(
      content.contains("network.proxy.type"),
      "Should contain proxy type setting"
    );
    assert!(
      content.contains("0"),
      "Should set proxy type to 0 (no proxy)"
    );
  }

  #[test]
  fn test_apply_proxy_settings_to_profile() {
    let (manager, temp_dir) = create_test_profile_manager();

    // Create a test profile directory structure
    let uuid_dir = temp_dir.path().join("test_uuid");
    let profile_dir = uuid_dir.join("profile");
    fs::create_dir_all(&profile_dir).expect("Should create profile directory");

    let proxy_settings = ProxySettings {
      proxy_type: "http".to_string(),
      host: "proxy.example.com".to_string(),
      port: 8080,
      username: Some("user".to_string()),
      password: Some("pass".to_string()),
    };

    let result = manager.apply_proxy_settings_to_profile(&profile_dir, &proxy_settings, None);
    assert!(result.is_ok(), "Should successfully apply proxy settings");

    // Check that user.js was created
    let user_js_path = profile_dir.join("user.js");
    assert!(user_js_path.exists(), "user.js should be created");

    let content = fs::read_to_string(&user_js_path).expect("Should read user.js");
    assert!(
      content.contains("network.proxy.type"),
      "Should contain proxy type setting"
    );
    assert!(content.contains("2"), "Should set proxy type to 2 (PAC)");

    // Check that PAC file was created
    let pac_path = uuid_dir.join("proxy.pac");
    assert!(pac_path.exists(), "proxy.pac should be created");

    let pac_content = fs::read_to_string(&pac_path).expect("Should read proxy.pac");
    assert!(
      pac_content.contains("FindProxyForURL"),
      "PAC file should contain FindProxyForURL function"
    );
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
pub async fn update_profile_proxy(
  app_handle: tauri::AppHandle,
  profile_id: String,
  proxy_id: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_proxy(app_handle, &profile_id, proxy_id)
    .await
    .map_err(|e| format!("Failed to update profile: {e}"))
}

#[tauri::command]
pub fn update_profile_tags(
  app_handle: tauri::AppHandle,
  profile_id: String,
  tags: Vec<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_tags(&app_handle, &profile_id, tags)
    .map_err(|e| format!("Failed to update profile tags: {e}"))
}

#[tauri::command]
pub fn update_profile_note(
  app_handle: tauri::AppHandle,
  profile_id: String,
  note: Option<String>,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .update_profile_note(&app_handle, &profile_id, note)
    .map_err(|e| format!("Failed to update profile note: {e}"))
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
  app_handle: tauri::AppHandle,
  profile_id: String,
  new_name: String,
) -> Result<BrowserProfile, String> {
  let profile_manager = ProfileManager::instance();
  profile_manager
    .rename_profile(&app_handle, &profile_id, &new_name)
    .map_err(|e| format!("Failed to rename profile: {e}"))
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

// Global singleton instance
#[tauri::command]
pub fn delete_profile(app_handle: tauri::AppHandle, profile_id: String) -> Result<(), String> {
  ProfileManager::instance()
    .delete_profile(&app_handle, &profile_id)
    .map_err(|e| format!("Failed to delete profile: {e}"))
}

lazy_static::lazy_static! {
  static ref PROFILE_MANAGER: ProfileManager = ProfileManager::new();
}

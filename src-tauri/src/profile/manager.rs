use crate::browser::{create_browser, BrowserType, ProxySettings};
use crate::camoufox::CamoufoxConfig;
use crate::profile::types::BrowserProfile;
use crate::proxy_manager::PROXY_MANAGER;
use directories::BaseDirs;
use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};
use sysinfo::{Pid, System};
use tauri::Emitter;

pub struct ProfileManager {
  base_dirs: BaseDirs,
}

impl ProfileManager {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
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

  #[allow(clippy::too_many_arguments)]
  pub async fn create_profile(
    &self,
    app_handle: &tauri::AppHandle,
    name: &str,
    browser: &str,
    version: &str,
    release_type: &str,
    proxy_id: Option<String>,
    camoufox_config: Option<CamoufoxConfig>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    self
      .create_profile_with_group(
        app_handle,
        name,
        browser,
        version,
        release_type,
        proxy_id,
        camoufox_config,
        None,
      )
      .await
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
    println!("Attempting to create profile: {name}");

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
        println!("Creating default Camoufox config for profile: {name}");
        crate::camoufox::CamoufoxConfig::default()
      });

      // Pass upstream proxy information to config for fingerprint generation
      if let Some(proxy_id_ref) = &proxy_id {
        if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_ref) {
          // For fingerprint generation, pass upstream proxy directly with credentials if present
          let proxy_url = if let (Some(username), Some(password)) = (&proxy_settings.username, &proxy_settings.password) {
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
          println!(
            "Using upstream proxy for Camoufox fingerprint generation: {}://{}:{}",
            proxy_settings.proxy_type.to_lowercase(),
            proxy_settings.host,
            proxy_settings.port
          );
        }
      }

      // Generate fingerprint if not already provided
      if config.fingerprint.is_none() {
        println!("Generating fingerprint for Camoufox profile: {name}");

        // Use the camoufox launcher to generate the config
        let camoufox_launcher = crate::camoufox::CamoufoxNodecarLauncher::instance();
        match camoufox_launcher
          .generate_fingerprint_config(app_handle, &config)
          .await
        {
          Ok(generated_fingerprint) => {
            config.fingerprint = Some(generated_fingerprint);
            println!("Successfully generated fingerprint for profile: {name}");
          }
          Err(e) => {
            return Err(format!("Failed to generate fingerprint for Camoufox profile '{name}': {e}").into());
          }
        }
      } else {
        println!("Using provided fingerprint for Camoufox profile: {name}");
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
    };

    // Save profile info
    self.save_profile(&profile)?;

    // Verify the profile was saved correctly
    if !profile_file.exists() {
      return Err(format!("Failed to create profile file for '{name}'").into());
    }

    println!("Profile '{name}' created successfully with ID: {profile_id}");

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
    old_name: &str,
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

    // Find the profile by old name
    let mut profile = existing_profiles
      .into_iter()
      .find(|p| p.name == old_name)
      .ok_or_else(|| format!("Profile '{old_name}' not found"))?;

    // Update profile name (no need to move directories since we use UUID)
    profile.name = new_name.to_string();

    // Save profile with new name
    self.save_profile(&profile)?;

    Ok(profile)
  }

  pub fn delete_profile(&self, profile_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Attempting to delete profile: {profile_name}");

    // Find the profile by name
    let profiles = self.list_profiles()?;
    let profile = profiles
      .into_iter()
      .find(|p| p.name == profile_name)
      .ok_or_else(|| format!("Profile '{profile_name}' not found"))?;

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
      println!("Deleting profile directory: {}", profile_uuid_dir.display());
      fs::remove_dir_all(&profile_uuid_dir)?;
      println!("Profile directory deleted successfully");
    }

    // Verify deletion was successful
    if profile_uuid_dir.exists() {
      return Err(format!("Failed to completely delete profile '{profile_name}'").into());
    }

    println!("Profile '{profile_name}' deleted successfully");

    Ok(())
  }

  pub fn update_profile_version(
    &self,
    profile_name: &str,
    version: &str,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    // Find the profile by name
    let profiles = self.list_profiles()?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.name == profile_name)
      .ok_or_else(|| format!("Profile {profile_name} not found"))?;

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
    profile.release_type =
      if crate::api_client::is_browser_version_nightly(&profile.browser, version, None) {
        "nightly".to_string()
      } else {
        "stable".to_string()
      };

    // Save the updated profile
    self.save_profile(&profile)?;

    Ok(profile)
  }

  pub fn assign_profiles_to_group(
    &self,
    profile_names: Vec<String>,
    group_id: Option<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;

    for profile_name in profile_names {
      let mut profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .ok_or_else(|| format!("Profile '{profile_name}' not found"))?
        .clone();

      // Check if browser is running
      if profile.process_id.is_some() {
        return Err(format!(
          "Cannot modify group for profile '{profile_name}' while browser is running. Please stop the browser first."
        ).into());
      }

      profile.group_id = group_id.clone();
      self.save_profile(&profile)?;
    }

    Ok(())
  }

  pub fn delete_multiple_profiles(
    &self,
    profile_names: Vec<String>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = self.list_profiles()?;

    for profile_name in profile_names {
      let profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .ok_or_else(|| format!("Profile '{profile_name}' not found"))?;

      // Check if browser is running
      if profile.process_id.is_some() {
        return Err(
          format!(
            "Cannot delete profile '{profile_name}' while browser is running. Please stop the browser first."
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

    Ok(())
  }

  pub async fn update_camoufox_config(
    &self,
    app_handle: tauri::AppHandle,
    profile_name: &str,
    config: CamoufoxConfig,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Find the profile by name
    let profiles =
      self
        .list_profiles()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to list profiles: {e}").into()
        })?;
    let mut profile = profiles
      .into_iter()
      .find(|p| p.name == profile_name)
      .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Profile {profile_name} not found").into()
      })?;

    // Check if the browser is currently running using the comprehensive status check
    let is_running = self.check_browser_status(app_handle, &profile).await?;

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

    println!("Camoufox configuration updated for profile '{profile_name}'.");

    Ok(())
  }

  pub async fn update_profile_proxy(
    &self,
    app_handle: tauri::AppHandle,
    profile_name: &str,
    proxy_id: Option<String>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error + Send + Sync>> {
    // Find the profile by name
    let profiles =
      self
        .list_profiles()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
          format!("Failed to list profiles: {e}").into()
        })?;

    let mut profile = profiles
      .into_iter()
      .find(|p| p.name == profile_name)
      .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Profile {profile_name} not found").into()
      })?;

    // Check if browser is running to manage proxy accordingly
    let browser_is_running = profile.process_id.is_some()
      && self
        .check_browser_status(app_handle.clone(), &profile)
        .await?;

    // If browser is running, stop existing proxy
    if browser_is_running && profile.proxy_id.is_some() {
      if let Some(pid) = profile.process_id {
        let _ = PROXY_MANAGER.stop_proxy(app_handle.clone(), pid).await;
      }
    }

    // Update proxy settings
    profile.proxy_id = proxy_id.clone();

    // Save the updated profile
    self
      .save_profile(&profile)
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        format!("Failed to save profile: {e}").into()
      })?;

    // Handle proxy startup/configuration
    if let Some(proxy_id_ref) = &proxy_id {
      if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_ref) {
        if browser_is_running {
          // Browser is running and proxy is enabled, start new proxy
          if let Some(pid) = profile.process_id {
            match PROXY_MANAGER
              .start_proxy(
                app_handle.clone(),
                Some(&proxy_settings),
                pid,
                Some(profile_name),
              )
              .await
            {
              Ok(internal_proxy_settings) => {
                let profiles_dir = self.get_profiles_dir();
                let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");

                // Apply the proxy settings with the internal proxy to the profile directory
                self
                  .apply_proxy_settings_to_profile(
                    &profile_path,
                    &proxy_settings,
                    Some(&internal_proxy_settings),
                  )
                  .map_err(|e| format!("Failed to update profile proxy: {e}"))?;

                println!("Successfully started proxy for profile: {}", profile.name);
              }
              Err(e) => {
                eprintln!("Failed to start proxy: {e}");
                // Apply proxy settings without internal proxy
                let profiles_dir = self.get_profiles_dir();
                let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
                self
                  .apply_proxy_settings_to_profile(&profile_path, &proxy_settings, None)
                  .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("Failed to apply proxy settings: {e}").into()
                  })?;
              }
            }
          } else {
            // No PID available, apply proxy settings without internal proxy
            let profiles_dir = self.get_profiles_dir();
            let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
            self
              .apply_proxy_settings_to_profile(&profile_path, &proxy_settings, None)
              .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("Failed to apply proxy settings: {e}").into()
              })?;
          }
        } else {
          // Proxy disabled or browser not running, just apply settings
          let profiles_dir = self.get_profiles_dir();
          let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
          self
            .apply_proxy_settings_to_profile(&profile_path, &proxy_settings, None)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
              format!("Failed to apply proxy settings: {e}").into()
            })?;
        }
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
    let mut inner_profile = profile.clone();
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
          println!(
            "Found existing browser process with PID: {} for profile: {}",
            pid, profile.name
          );
        } else {
          println!("PID {pid} exists but doesn't match our profile path exactly, searching for correct process...");
        }
      } else {
        println!("Stored PID {pid} no longer exists, searching for browser process...");
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
            println!(
              "Found browser process with PID: {} for profile: {}",
              pid.as_u32(),
              profile.name
            );
            break;
          }
        }
      }
    }

    // Update the process ID if we found a different one
    if let Some(pid) = found_pid {
      if inner_profile.process_id != Some(pid) {
        inner_profile.process_id = Some(pid);
        if let Err(e) = self.save_profile(&inner_profile) {
          println!("Warning: Failed to update profile with new PID: {e}");
        }
      }
    } else if inner_profile.process_id.is_some() {
      // Clear the PID if no process found
      inner_profile.process_id = None;
      if let Err(e) = self.save_profile(&inner_profile) {
        println!("Warning: Failed to clear profile PID: {e}");
      }
    }

    // Emit profile update event to frontend
    if let Err(e) = app_handle.emit("profile-updated", &inner_profile) {
      println!("Warning: Failed to emit profile update event: {e}");
    }

    Ok(is_running)
  }

  // Check Camoufox status using nodecar-based approach
  async fn check_camoufox_status_via_nodecar(
    &self,
    app_handle: &tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    use crate::camoufox::CamoufoxNodecarLauncher;

    let launcher = CamoufoxNodecarLauncher::instance();
    let profiles_dir = self.get_profiles_dir();
    let profile_data_path = profile.get_profile_data_path(&profiles_dir);
    let profile_path_str = profile_data_path.to_string_lossy();

    // Check if there's a running Camoufox instance for this profile
    match launcher.find_camoufox_by_profile(&profile_path_str).await {
      Ok(Some(camoufox_process)) => {
        // Found a running instance, update profile with process info
        let mut updated_profile = profile.clone();
        updated_profile.process_id = camoufox_process.processId;
        if let Err(e) = self.save_profile(&updated_profile) {
          println!("Warning: Failed to update Camoufox profile with process info: {e}");
        }

        // Emit profile update event to frontend
        if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
          println!("Warning: Failed to emit profile update event: {e}");
        }

        println!(
          "Camoufox profile '{}' is running with PID: {:?}",
          profile.name, camoufox_process.processId
        );
        Ok(true)
      }
      Ok(None) => {
        // No running instance found, clear process ID if set
        if profile.process_id.is_some() {
          let mut updated_profile = profile.clone();
          updated_profile.process_id = None;
          if let Err(e) = self.save_profile(&updated_profile) {
            println!("Warning: Failed to clear Camoufox profile process info: {e}");
          }

          // Emit profile update event to frontend
          if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
            println!("Warning: Failed to emit profile update event: {e}");
          }
        }
        println!("Camoufox profile '{}' is not running", profile.name);
        Ok(false)
      }
      Err(e) => {
        // Error checking status, assume not running and clear process ID
        println!("Warning: Failed to check Camoufox status via nodecar: {e}");
        if profile.process_id.is_some() {
          let mut updated_profile = profile.clone();
          updated_profile.process_id = None;
          if let Err(e) = self.save_profile(&updated_profile) {
            println!("Warning: Failed to clear Camoufox profile process info after error: {e}");
          }

          // Emit profile update event to frontend
          if let Err(e) = app_handle.emit("profile-updated", &updated_profile) {
            println!("Warning: Failed to emit profile update event: {e}");
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

  fn get_binaries_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("binaries");
    path
  }

  fn get_common_firefox_preferences(&self) -> Vec<String> {
    vec![
      // Disable default browser updates
      "user_pref(\"browser.shell.checkDefaultBrowser\", false);".to_string(),
      "user_pref(\"browser.shell.skipDefaultBrowserCheckOnFirstRun\", true);".to_string(),
      "user_pref(\"browser.preferences.moreFromMozilla\", false);".to_string(),
      "user_pref(\"services.sync.prefs.sync.browser.startup.upgradeDialog.enabled\", false);"
        .to_string(),
      "user_pref(\"app.update.enabled\", false);".to_string(),
      "user_pref(\"app.update.staging.enabled\", false);".to_string(),
      "user_pref(\"app.update.timerFirstInterval\", -1);".to_string(),
      "user_pref(\"app.update.download.maxAttempts\", 0);".to_string(),
      "user_pref(\"app.update.elevate.maxAttempts\", 0);".to_string(),
      "user_pref(\"app.update.disabledForTesting\", true);".to_string(),
      "user_pref(\"app.update.auto\", false);".to_string(),
      "user_pref(\"app.update.mode\", 0);".to_string(),
      "user_pref(\"app.update.promptWaitTime\", -1);".to_string(),
      "user_pref(\"app.update.service.enabled\", false);".to_string(),
      "user_pref(\"app.update.silent\", true);".to_string(),
      "user_pref(\"app.update.checkInstallTime\", false);".to_string(),
      "user_pref(\"app.update.interval\", -1);".to_string(),
      "user_pref(\"app.update.background.interval\", -1);".to_string(),
      "user_pref(\"app.update.idletime\", -1);".to_string(),
    ]
  }

  pub fn apply_proxy_settings_to_profile(
    &self,
    profile_data_path: &Path,
    proxy: &ProxySettings,
    internal_proxy: Option<&ProxySettings>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let user_js_path = profile_data_path.join("user.js");
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
      // Use internal proxy as the primary proxy
      format!("HTTP {}:{}", internal.host, internal.port)
    } else {
      // Use user-configured proxy directly
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
    fs::write(&pac_path, pac_content)?;

    // Configure Firefox to use the PAC file
    preferences.extend([
      "user_pref(\"network.proxy.type\", 2);".to_string(),
      format!(
        "user_pref(\"network.proxy.autoconfig_url\", \"file://{}\");",
        pac_path.to_string_lossy()
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
    fs::write(user_js_path, preferences.join("\n"))?;

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

    assert!(profiles_dir.to_string_lossy().contains("DonutBrowser"));
    assert!(profiles_dir.to_string_lossy().contains("profiles"));
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref PROFILE_MANAGER: ProfileManager = ProfileManager::new();
}

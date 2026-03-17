use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};

use crate::camoufox_manager::CamoufoxConfig;
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::profile::types::{get_host_os, BrowserProfile, SyncMode};
use crate::profile::ProfileManager;
use crate::proxy_manager::PROXY_MANAGER;
use crate::wayfern_manager::WayfernConfig;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DetectedProfile {
  pub browser: String,
  pub mapped_browser: String,
  pub name: String,
  pub path: String,
  pub description: String,
}

fn map_browser_type(browser: &str) -> &str {
  match browser {
    "firefox" | "firefox-developer" | "zen" => "camoufox",
    "chromium" | "brave" => "wayfern",
    "camoufox" => "camoufox",
    "wayfern" => "wayfern",
    _ => "wayfern",
  }
}

pub struct ProfileImporter {
  base_dirs: BaseDirs,
  downloaded_browsers_registry: &'static DownloadedBrowsersRegistry,
  profile_manager: &'static ProfileManager,
  camoufox_manager: &'static crate::camoufox_manager::CamoufoxManager,
  wayfern_manager: &'static crate::wayfern_manager::WayfernManager,
}

impl ProfileImporter {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      downloaded_browsers_registry: DownloadedBrowsersRegistry::instance(),
      profile_manager: ProfileManager::instance(),
      camoufox_manager: crate::camoufox_manager::CamoufoxManager::instance(),
      wayfern_manager: crate::wayfern_manager::WayfernManager::instance(),
    }
  }

  pub fn instance() -> &'static ProfileImporter {
    &PROFILE_IMPORTER
  }

  pub fn detect_existing_profiles(
    &self,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut detected_profiles = Vec::new();

    detected_profiles.extend(self.detect_firefox_profiles()?);
    detected_profiles.extend(self.detect_chrome_profiles()?);
    detected_profiles.extend(self.detect_brave_profiles()?);
    detected_profiles.extend(self.detect_firefox_developer_profiles()?);
    detected_profiles.extend(self.detect_chromium_profiles()?);
    detected_profiles.extend(self.detect_zen_browser_profiles()?);

    let mut seen_paths = HashSet::new();
    let unique_profiles: Vec<DetectedProfile> = detected_profiles
      .into_iter()
      .filter(|profile| seen_paths.insert(profile.path.clone()))
      .collect();

    Ok(unique_profiles)
  }

  fn detect_firefox_profiles(&self) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let firefox_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/Firefox/Profiles");
      profiles.extend(self.scan_firefox_profiles_dir(&firefox_dir, "firefox")?);
    }

    #[cfg(target_os = "windows")]
    {
      let app_data = self.base_dirs.data_dir();
      let firefox_dir = app_data.join("Mozilla/Firefox/Profiles");
      profiles.extend(self.scan_firefox_profiles_dir(&firefox_dir, "firefox")?);

      let local_app_data = self.base_dirs.data_local_dir();
      let firefox_local_dir = local_app_data.join("Mozilla/Firefox/Profiles");
      if firefox_local_dir.exists() {
        profiles.extend(self.scan_firefox_profiles_dir(&firefox_local_dir, "firefox")?);
      }
    }

    #[cfg(target_os = "linux")]
    {
      let firefox_dir = self.base_dirs.home_dir().join(".mozilla/firefox");
      profiles.extend(self.scan_firefox_profiles_dir(&firefox_dir, "firefox")?);
    }

    Ok(profiles)
  }

  fn detect_firefox_developer_profiles(
    &self,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let firefox_dev_alt_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/Firefox Developer Edition/Profiles");

      if firefox_dev_alt_dir.exists() {
        profiles.extend(self.scan_firefox_profiles_dir(&firefox_dev_alt_dir, "firefox-developer")?);
      }
    }

    #[cfg(target_os = "windows")]
    {
      let app_data = self.base_dirs.data_dir();
      let firefox_dev_dir = app_data.join("Mozilla/Firefox Developer Edition/Profiles");
      if firefox_dev_dir.exists() {
        profiles.extend(self.scan_firefox_profiles_dir(&firefox_dev_dir, "firefox-developer")?);
      }
    }

    #[cfg(target_os = "linux")]
    {
      let firefox_dev_dir = self
        .base_dirs
        .home_dir()
        .join(".mozilla/firefox-dev-edition");
      if firefox_dev_dir.exists() {
        profiles.extend(self.scan_firefox_profiles_dir(&firefox_dev_dir, "firefox-developer")?);
      }
    }

    Ok(profiles)
  }

  fn detect_chrome_profiles(&self) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let chrome_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/Google/Chrome");
      profiles.extend(self.scan_chrome_profiles_dir(&chrome_dir, "chromium")?);
    }

    #[cfg(target_os = "windows")]
    {
      let local_app_data = self.base_dirs.data_local_dir();
      let chrome_dir = local_app_data.join("Google/Chrome/User Data");
      profiles.extend(self.scan_chrome_profiles_dir(&chrome_dir, "chromium")?);
    }

    #[cfg(target_os = "linux")]
    {
      let chrome_dir = self.base_dirs.home_dir().join(".config/google-chrome");
      profiles.extend(self.scan_chrome_profiles_dir(&chrome_dir, "chromium")?);
    }

    Ok(profiles)
  }

  fn detect_chromium_profiles(&self) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let chromium_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/Chromium");
      profiles.extend(self.scan_chrome_profiles_dir(&chromium_dir, "chromium")?);
    }

    #[cfg(target_os = "windows")]
    {
      let local_app_data = self.base_dirs.data_local_dir();
      let chromium_dir = local_app_data.join("Chromium/User Data");
      profiles.extend(self.scan_chrome_profiles_dir(&chromium_dir, "chromium")?);
    }

    #[cfg(target_os = "linux")]
    {
      let chromium_dir = self.base_dirs.home_dir().join(".config/chromium");
      profiles.extend(self.scan_chrome_profiles_dir(&chromium_dir, "chromium")?);
    }

    Ok(profiles)
  }

  fn detect_brave_profiles(&self) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let brave_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/BraveSoftware/Brave-Browser");
      profiles.extend(self.scan_chrome_profiles_dir(&brave_dir, "brave")?);
    }

    #[cfg(target_os = "windows")]
    {
      let local_app_data = self.base_dirs.data_local_dir();
      let brave_dir = local_app_data.join("BraveSoftware/Brave-Browser/User Data");
      profiles.extend(self.scan_chrome_profiles_dir(&brave_dir, "brave")?);
    }

    #[cfg(target_os = "linux")]
    {
      let brave_dir = self
        .base_dirs
        .home_dir()
        .join(".config/BraveSoftware/Brave-Browser");
      profiles.extend(self.scan_chrome_profiles_dir(&brave_dir, "brave")?);
    }

    Ok(profiles)
  }

  fn detect_zen_browser_profiles(
    &self,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let zen_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/Zen/Profiles");
      profiles.extend(self.scan_firefox_profiles_dir(&zen_dir, "zen")?);
    }

    #[cfg(target_os = "windows")]
    {
      let app_data = self.base_dirs.data_dir();
      let zen_dir = app_data.join("Zen/Profiles");
      profiles.extend(self.scan_firefox_profiles_dir(&zen_dir, "zen")?);
    }

    #[cfg(target_os = "linux")]
    {
      let zen_dir = self.base_dirs.home_dir().join(".zen");
      profiles.extend(self.scan_firefox_profiles_dir(&zen_dir, "zen")?);
    }

    Ok(profiles)
  }

  fn scan_firefox_profiles_dir(
    &self,
    profiles_dir: &Path,
    browser_type: &str,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    if !profiles_dir.exists() {
      return Ok(profiles);
    }

    let profiles_ini = profiles_dir
      .parent()
      .unwrap_or(profiles_dir)
      .join("profiles.ini");
    if profiles_ini.exists() {
      if let Ok(content) = fs::read_to_string(&profiles_ini) {
        profiles.extend(self.parse_firefox_profiles_ini(&content, profiles_dir, browser_type)?);
      }
    }

    if let Ok(entries) = fs::read_dir(profiles_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
          let prefs_file = path.join("prefs.js");
          if prefs_file.exists() {
            let profile_name = path
              .file_name()
              .and_then(|n| n.to_str())
              .unwrap_or("Unknown Profile");

            let already_added = profiles.iter().any(|p| p.path == path.to_string_lossy());
            if !already_added {
              profiles.push(DetectedProfile {
                browser: browser_type.to_string(),
                mapped_browser: map_browser_type(browser_type).to_string(),
                name: format!(
                  "{} Profile - {}",
                  self.get_browser_display_name(browser_type),
                  profile_name
                ),
                path: path.to_string_lossy().to_string(),
                description: format!("Profile folder: {profile_name}"),
              });
            }
          }
        }
      }
    }

    Ok(profiles)
  }

  fn parse_firefox_profiles_ini(
    &self,
    content: &str,
    profiles_dir: &Path,
    browser_type: &str,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();
    let mut current_section = String::new();
    let mut profile_name = String::new();
    let mut profile_path = String::new();
    let mut is_relative = true;

    for line in content.lines() {
      let line = line.trim();

      if line.starts_with('[') && line.ends_with(']') {
        if !current_section.is_empty()
          && current_section.starts_with("Profile")
          && !profile_path.is_empty()
        {
          let full_path = if is_relative {
            profiles_dir.join(&profile_path)
          } else {
            PathBuf::from(&profile_path)
          };

          if full_path.exists() {
            let display_name = if profile_name.is_empty() {
              format!("{} Profile", self.get_browser_display_name(browser_type))
            } else {
              format!(
                "{} - {}",
                self.get_browser_display_name(browser_type),
                profile_name
              )
            };

            profiles.push(DetectedProfile {
              browser: browser_type.to_string(),
              mapped_browser: map_browser_type(browser_type).to_string(),
              name: display_name,
              path: full_path.to_string_lossy().to_string(),
              description: format!("Profile: {profile_name}"),
            });
          }
        }

        current_section = line[1..line.len() - 1].to_string();
        profile_name.clear();
        profile_path.clear();
        is_relative = true;
      } else if line.contains('=') {
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() == 2 {
          let key = parts[0].trim();
          let value = parts[1].trim();

          match key {
            "Name" => profile_name = value.to_string(),
            "Path" => profile_path = value.to_string(),
            "IsRelative" => is_relative = value == "1",
            _ => {}
          }
        }
      }
    }

    if !current_section.is_empty()
      && current_section.starts_with("Profile")
      && !profile_path.is_empty()
    {
      let full_path = if is_relative {
        profiles_dir.join(&profile_path)
      } else {
        PathBuf::from(&profile_path)
      };

      if full_path.exists() {
        let display_name = if profile_name.is_empty() {
          format!("{} Profile", self.get_browser_display_name(browser_type))
        } else {
          format!(
            "{} - {}",
            self.get_browser_display_name(browser_type),
            profile_name
          )
        };

        profiles.push(DetectedProfile {
          browser: browser_type.to_string(),
          mapped_browser: map_browser_type(browser_type).to_string(),
          name: display_name,
          path: full_path.to_string_lossy().to_string(),
          description: format!("Profile: {profile_name}"),
        });
      }
    }

    Ok(profiles)
  }

  fn scan_chrome_profiles_dir(
    &self,
    browser_dir: &Path,
    browser_type: &str,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    if !browser_dir.exists() {
      return Ok(profiles);
    }

    let default_profile = browser_dir.join("Default");
    if default_profile.exists() && default_profile.join("Preferences").exists() {
      profiles.push(DetectedProfile {
        browser: browser_type.to_string(),
        mapped_browser: map_browser_type(browser_type).to_string(),
        name: format!(
          "{} - Default Profile",
          self.get_browser_display_name(browser_type)
        ),
        path: default_profile.to_string_lossy().to_string(),
        description: "Default profile".to_string(),
      });
    }

    if let Ok(entries) = fs::read_dir(browser_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
          let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

          if dir_name.starts_with("Profile ") && path.join("Preferences").exists() {
            let profile_number = &dir_name[8..];
            profiles.push(DetectedProfile {
              browser: browser_type.to_string(),
              mapped_browser: map_browser_type(browser_type).to_string(),
              name: format!(
                "{} - Profile {}",
                self.get_browser_display_name(browser_type),
                profile_number
              ),
              path: path.to_string_lossy().to_string(),
              description: format!("Profile {profile_number}"),
            });
          }
        }
      }
    }

    Ok(profiles)
  }

  fn get_browser_display_name(&self, browser_type: &str) -> &str {
    match browser_type {
      "firefox" => "Firefox",
      "firefox-developer" => "Firefox Developer",
      "chromium" => "Chrome/Chromium",
      "brave" => "Brave",
      "zen" => "Zen Browser",
      "camoufox" => "Camoufox",
      "wayfern" => "Wayfern",
      _ => "Unknown Browser",
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub async fn import_profile(
    &self,
    app_handle: &tauri::AppHandle,
    source_path: &str,
    browser_type: &str,
    new_profile_name: &str,
    proxy_id: Option<String>,
    camoufox_config: Option<CamoufoxConfig>,
    wayfern_config: Option<WayfernConfig>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let source_path = Path::new(source_path);
    if !source_path.exists() {
      return Err("Source profile path does not exist".into());
    }

    let mapped = map_browser_type(browser_type);

    if let Some(ref pid) = proxy_id {
      if PROXY_MANAGER.is_cloud_or_derived(pid) || pid == crate::proxy_manager::CLOUD_PROXY_ID {
        crate::cloud_auth::CLOUD_AUTH.sync_cloud_proxy().await;
      }
    }

    let existing_profiles = self.profile_manager.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == new_profile_name.to_lowercase())
    {
      return Err(format!("Profile with name '{new_profile_name}' already exists").into());
    }

    let profile_id = uuid::Uuid::new_v4();
    let profiles_dir = self.profile_manager.get_profiles_dir();
    let new_profile_uuid_dir = profiles_dir.join(profile_id.to_string());
    let new_profile_data_dir = new_profile_uuid_dir.join("profile");

    create_dir_all(&new_profile_uuid_dir)?;
    create_dir_all(&new_profile_data_dir)?;

    Self::copy_directory_recursive(source_path, &new_profile_data_dir)?;

    let version = self.get_default_version_for_browser(mapped)?;

    let final_camoufox_config = if mapped == "camoufox" {
      let mut config = camoufox_config.unwrap_or_default();

      if config.executable_path.is_none() {
        let mut browser_dir = self.profile_manager.get_binaries_dir();
        browser_dir.push(mapped);
        browser_dir.push(&version);

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
      }

      if let Some(ref proxy_id_val) = proxy_id {
        if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_val) {
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
        }
      }

      if config.fingerprint.is_none() {
        let temp_profile = BrowserProfile {
          id: uuid::Uuid::new_v4(),
          name: new_profile_name.to_string(),
          browser: mapped.to_string(),
          version: version.clone(),
          proxy_id: proxy_id.clone(),
          vpn_id: None,
          process_id: None,
          last_launch: None,
          release_type: "stable".to_string(),
          camoufox_config: None,
          wayfern_config: None,
          group_id: None,
          tags: Vec::new(),
          note: None,
          sync_mode: SyncMode::Disabled,
          encryption_salt: None,
          last_sync: None,
          host_os: None,
          ephemeral: false,
          extension_group_id: None,
          proxy_bypass_rules: Vec::new(),
          created_by_id: None,
          created_by_email: None,
        };

        match self
          .camoufox_manager
          .generate_fingerprint_config(app_handle, &temp_profile, &config)
          .await
        {
          Ok(fp) => config.fingerprint = Some(fp),
          Err(e) => {
            return Err(
              format!(
                "Failed to generate fingerprint for imported profile '{new_profile_name}': {e}"
              )
              .into(),
            );
          }
        }
      }

      config.proxy = None;
      Some(config)
    } else {
      None
    };

    let final_wayfern_config = if mapped == "wayfern" {
      let mut config = wayfern_config.unwrap_or_default();

      if config.executable_path.is_none() {
        let mut browser_dir = self.profile_manager.get_binaries_dir();
        browser_dir.push(mapped);
        browser_dir.push(&version);

        #[cfg(target_os = "macos")]
        let binary_path = browser_dir
          .join("Chromium.app")
          .join("Contents")
          .join("MacOS")
          .join("Chromium");

        #[cfg(target_os = "windows")]
        let binary_path = browser_dir.join("chrome.exe");

        #[cfg(target_os = "linux")]
        let binary_path = browser_dir.join("chrome");

        config.executable_path = Some(binary_path.to_string_lossy().to_string());
      }

      if let Some(ref proxy_id_val) = proxy_id {
        if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_val) {
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
        }
      }

      if config.fingerprint.is_none() {
        let temp_profile = BrowserProfile {
          id: uuid::Uuid::new_v4(),
          name: new_profile_name.to_string(),
          browser: mapped.to_string(),
          version: version.clone(),
          proxy_id: proxy_id.clone(),
          vpn_id: None,
          process_id: None,
          last_launch: None,
          release_type: "stable".to_string(),
          camoufox_config: None,
          wayfern_config: None,
          group_id: None,
          tags: Vec::new(),
          note: None,
          sync_mode: SyncMode::Disabled,
          encryption_salt: None,
          last_sync: None,
          host_os: None,
          ephemeral: false,
          extension_group_id: None,
          proxy_bypass_rules: Vec::new(),
          created_by_id: None,
          created_by_email: None,
        };

        match self
          .wayfern_manager
          .generate_fingerprint_config(app_handle, &temp_profile, &config)
          .await
        {
          Ok(fp) => config.fingerprint = Some(fp),
          Err(e) => {
            return Err(
              format!(
                "Failed to generate fingerprint for imported profile '{new_profile_name}': {e}"
              )
              .into(),
            );
          }
        }
      }

      config.proxy = None;
      Some(config)
    } else {
      None
    };

    let profile = BrowserProfile {
      id: profile_id,
      name: new_profile_name.to_string(),
      browser: mapped.to_string(),
      version,
      proxy_id,
      vpn_id: None,
      process_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      camoufox_config: final_camoufox_config,
      wayfern_config: final_wayfern_config,
      group_id: None,
      tags: Vec::new(),
      note: None,
      sync_mode: SyncMode::Disabled,
      encryption_salt: None,
      last_sync: None,
      host_os: Some(get_host_os()),
      ephemeral: false,
      extension_group_id: None,
      proxy_bypass_rules: Vec::new(),
      created_by_id: None,
      created_by_email: None,
    };

    self.profile_manager.save_profile(&profile)?;

    log::info!(
      "Successfully imported profile '{}' from '{}'",
      new_profile_name,
      source_path.display()
    );

    Ok(())
  }

  fn get_default_version_for_browser(
    &self,
    browser_type: &str,
  ) -> Result<String, Box<dyn std::error::Error>> {
    let downloaded_versions = self
      .downloaded_browsers_registry
      .get_downloaded_versions(browser_type);

    if let Some(version) = downloaded_versions.first() {
      return Ok(version.clone());
    }

    Err(
      format!(
        "No downloaded versions found for browser '{}'. Please download a version of {} first before importing profiles.",
        browser_type,
        self.get_browser_display_name(browser_type)
      )
      .into(),
    )
  }

  pub fn copy_directory_recursive(
    source: &Path,
    destination: &Path,
  ) -> Result<(), Box<dyn std::error::Error>> {
    if !destination.exists() {
      create_dir_all(destination)?;
    }

    for entry in fs::read_dir(source)? {
      let entry = entry?;
      let source_path = entry.path();
      let dest_path = destination.join(entry.file_name());

      if source_path.is_dir() {
        Self::copy_directory_recursive(&source_path, &dest_path)?;
      } else {
        fs::copy(&source_path, &dest_path)?;
      }
    }

    Ok(())
  }
}

#[tauri::command]
pub async fn detect_existing_profiles() -> Result<Vec<DetectedProfile>, String> {
  let importer = ProfileImporter::instance();
  importer
    .detect_existing_profiles()
    .map_err(|e| format!("Failed to detect existing profiles: {e}"))
}

#[tauri::command]
pub async fn import_browser_profile(
  app_handle: tauri::AppHandle,
  source_path: String,
  browser_type: String,
  new_profile_name: String,
  proxy_id: Option<String>,
  camoufox_config: Option<CamoufoxConfig>,
  wayfern_config: Option<WayfernConfig>,
) -> Result<(), String> {
  let fingerprint_os = camoufox_config
    .as_ref()
    .and_then(|c| c.os.as_deref())
    .or_else(|| wayfern_config.as_ref().and_then(|c| c.os.as_deref()));

  if !crate::cloud_auth::CLOUD_AUTH
    .is_fingerprint_os_allowed(fingerprint_os)
    .await
  {
    return Err("Fingerprint OS spoofing requires an active Pro subscription".to_string());
  }

  let importer = ProfileImporter::instance();
  importer
    .import_profile(
      &app_handle,
      &source_path,
      &browser_type,
      &new_profile_name,
      proxy_id,
      camoufox_config,
      wayfern_config,
    )
    .await
    .map_err(|e| format!("Failed to import profile: {e}"))
}

lazy_static::lazy_static! {
  static ref PROFILE_IMPORTER: ProfileImporter = ProfileImporter::new();
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::env;
  use tempfile::TempDir;

  fn create_test_profile_importer() -> (ProfileImporter, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    env::set_var("HOME", temp_dir.path());
    let importer = ProfileImporter::new();
    (importer, temp_dir)
  }

  #[test]
  fn test_profile_importer_creation() {
    let (_importer, _temp_dir) = create_test_profile_importer();
  }

  #[test]
  fn test_get_browser_display_name() {
    let (importer, _temp_dir) = create_test_profile_importer();

    assert_eq!(importer.get_browser_display_name("firefox"), "Firefox");
    assert_eq!(
      importer.get_browser_display_name("firefox-developer"),
      "Firefox Developer"
    );
    assert_eq!(
      importer.get_browser_display_name("chromium"),
      "Chrome/Chromium"
    );
    assert_eq!(importer.get_browser_display_name("brave"), "Brave");
    assert_eq!(importer.get_browser_display_name("zen"), "Zen Browser");
    assert_eq!(
      importer.get_browser_display_name("unknown"),
      "Unknown Browser"
    );
  }

  #[test]
  fn test_map_browser_type() {
    assert_eq!(map_browser_type("firefox"), "camoufox");
    assert_eq!(map_browser_type("firefox-developer"), "camoufox");
    assert_eq!(map_browser_type("zen"), "camoufox");
    assert_eq!(map_browser_type("chromium"), "wayfern");
    assert_eq!(map_browser_type("brave"), "wayfern");
    assert_eq!(map_browser_type("camoufox"), "camoufox");
    assert_eq!(map_browser_type("wayfern"), "wayfern");
    assert_eq!(map_browser_type("something_else"), "wayfern");
  }

  #[test]
  fn test_detect_existing_profiles_no_panic() {
    let (importer, _temp_dir) = create_test_profile_importer();

    let result = importer.detect_existing_profiles();
    assert!(result.is_ok(), "detect_existing_profiles should not fail");
    let _profiles = result.unwrap();
  }

  #[test]
  fn test_scan_firefox_profiles_dir_nonexistent() {
    let (importer, temp_dir) = create_test_profile_importer();

    let nonexistent_dir = temp_dir.path().join("nonexistent");
    let result = importer.scan_firefox_profiles_dir(&nonexistent_dir, "firefox");

    assert!(
      result.is_ok(),
      "Should handle nonexistent directory gracefully"
    );
    let profiles = result.unwrap();
    assert!(
      profiles.is_empty(),
      "Should return empty vector for nonexistent directory"
    );
  }

  #[test]
  fn test_scan_chrome_profiles_dir_nonexistent() {
    let (importer, temp_dir) = create_test_profile_importer();

    let nonexistent_dir = temp_dir.path().join("nonexistent");
    let result = importer.scan_chrome_profiles_dir(&nonexistent_dir, "chromium");

    assert!(
      result.is_ok(),
      "Should handle nonexistent directory gracefully"
    );
    let profiles = result.unwrap();
    assert!(
      profiles.is_empty(),
      "Should return empty vector for nonexistent directory"
    );
  }

  #[test]
  fn test_parse_firefox_profiles_ini_empty() {
    let (importer, _temp_dir) = create_test_profile_importer();

    let empty_content = "";
    let profiles_dir = Path::new("/tmp");
    let result = importer.parse_firefox_profiles_ini(empty_content, profiles_dir, "firefox");

    assert!(result.is_ok(), "Should handle empty profiles.ini");
    let profiles = result.unwrap();
    assert!(
      profiles.is_empty(),
      "Should return empty vector for empty content"
    );
  }

  #[test]
  fn test_parse_firefox_profiles_ini_valid() {
    let (importer, temp_dir) = create_test_profile_importer();

    let profiles_dir = temp_dir.path().join("profiles");
    let profile_dir = profiles_dir.join("test.profile");
    fs::create_dir_all(&profile_dir).expect("Should create profile directory");

    let prefs_file = profile_dir.join("prefs.js");
    fs::write(&prefs_file, "// Firefox preferences").expect("Should create prefs.js");

    let profiles_ini_content = r#"
[Profile0]
Name=Test Profile
IsRelative=1
Path=test.profile
"#;

    let result =
      importer.parse_firefox_profiles_ini(profiles_ini_content, &profiles_dir, "firefox");

    assert!(result.is_ok(), "Should parse valid profiles.ini");
    let profiles = result.unwrap();
    assert_eq!(profiles.len(), 1, "Should find one profile");
    assert_eq!(profiles[0].name, "Firefox - Test Profile");
    assert_eq!(profiles[0].browser, "firefox");
    assert_eq!(profiles[0].mapped_browser, "camoufox");
  }

  #[test]
  fn test_copy_directory_recursive() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    let source_dir = temp_dir.path().join("source");
    let source_subdir = source_dir.join("subdir");
    fs::create_dir_all(&source_subdir).expect("Should create source directories");

    let source_file1 = source_dir.join("file1.txt");
    let source_file2 = source_subdir.join("file2.txt");
    fs::write(&source_file1, "content1").expect("Should create file1");
    fs::write(&source_file2, "content2").expect("Should create file2");

    let dest_dir = temp_dir.path().join("dest");

    let result = ProfileImporter::copy_directory_recursive(&source_dir, &dest_dir);
    assert!(result.is_ok(), "Should copy directory successfully");

    let dest_file1 = dest_dir.join("file1.txt");
    let dest_file2 = dest_dir.join("subdir").join("file2.txt");

    assert!(dest_file1.exists(), "file1.txt should be copied");
    assert!(dest_file2.exists(), "file2.txt should be copied");

    let content1 = fs::read_to_string(&dest_file1).expect("Should read file1");
    let content2 = fs::read_to_string(&dest_file2).expect("Should read file2");

    assert_eq!(content1, "content1", "file1 content should match");
    assert_eq!(content2, "content2", "file2 content should match");
  }

  #[test]
  fn test_get_default_version_for_browser_no_versions() {
    let (importer, _temp_dir) = create_test_profile_importer();

    // Use a browser name that is guaranteed to have no downloaded versions,
    // since the global registry singleton may contain real data from the system.
    let result = importer.get_default_version_for_browser("nonexistent_browser_xyz");
    assert!(
      result.is_err(),
      "Should fail when no versions are available"
    );

    let error_msg = result.unwrap_err().to_string();
    assert!(
      error_msg.contains("No downloaded versions found"),
      "Error should mention no versions found"
    );
  }
}

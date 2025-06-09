use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};

use crate::browser::BrowserType;
use crate::browser_runner::BrowserRunner;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DetectedProfile {
  pub browser: String,
  pub name: String,
  pub path: String,
  pub description: String,
}

pub struct ProfileImporter {
  base_dirs: BaseDirs,
  browser_runner: BrowserRunner,
}

impl ProfileImporter {
  pub fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      browser_runner: BrowserRunner::new(),
    }
  }

  /// Detect existing browser profiles on the system
  pub fn detect_existing_profiles(
    &self,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut detected_profiles = Vec::new();

    // Detect Firefox profiles
    detected_profiles.extend(self.detect_firefox_profiles()?);

    // Detect Chrome profiles
    detected_profiles.extend(self.detect_chrome_profiles()?);

    // Detect Brave profiles
    detected_profiles.extend(self.detect_brave_profiles()?);

    // Detect Firefox Developer Edition profiles
    detected_profiles.extend(self.detect_firefox_developer_profiles()?);

    // Detect Chromium profiles
    detected_profiles.extend(self.detect_chromium_profiles()?);

    // Detect Mullvad Browser profiles
    detected_profiles.extend(self.detect_mullvad_browser_profiles()?);

    // Detect Zen Browser profiles
    detected_profiles.extend(self.detect_zen_browser_profiles()?);

    // Detect TOR Browser profiles
    detected_profiles.extend(self.detect_tor_browser_profiles()?);

    // Remove duplicates based on path
    let mut seen_paths = HashSet::new();
    let unique_profiles: Vec<DetectedProfile> = detected_profiles
      .into_iter()
      .filter(|profile| seen_paths.insert(profile.path.clone()))
      .collect();

    Ok(unique_profiles)
  }

  /// Detect Firefox profiles
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
      // Primary location in AppData\Roaming
      let app_data = self.base_dirs.data_dir();
      let firefox_dir = app_data.join("Mozilla/Firefox/Profiles");
      profiles.extend(self.scan_firefox_profiles_dir(&firefox_dir, "firefox")?);

      // Also check AppData\Local for portable installations
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

  /// Detect Firefox Developer Edition profiles
  fn detect_firefox_developer_profiles(
    &self,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      // Firefox Developer Edition on macOS uses separate profile directories
      let firefox_dev_alt_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/Firefox Developer Edition/Profiles");

      // Only scan the dedicated dev edition directory if it exists, otherwise skip to avoid duplicates
      if firefox_dev_alt_dir.exists() {
        profiles.extend(self.scan_firefox_profiles_dir(&firefox_dev_alt_dir, "firefox-developer")?);
      }
    }

    #[cfg(target_os = "windows")]
    {
      let app_data = self.base_dirs.data_dir();
      // Firefox Developer Edition on Windows typically uses separate directories
      let firefox_dev_dir = app_data.join("Mozilla/Firefox Developer Edition/Profiles");
      if firefox_dev_dir.exists() {
        profiles.extend(self.scan_firefox_profiles_dir(&firefox_dev_dir, "firefox-developer")?);
      }
    }

    #[cfg(target_os = "linux")]
    {
      // Firefox Developer Edition on Linux uses separate directories
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

  /// Detect Chrome profiles
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

  /// Detect Chromium profiles
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

  /// Detect Brave profiles
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

  /// Detect Mullvad Browser profiles
  fn detect_mullvad_browser_profiles(
    &self,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let mullvad_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/MullvadBrowser/Profiles");
      profiles.extend(self.scan_firefox_profiles_dir(&mullvad_dir, "mullvad-browser")?);
    }

    #[cfg(target_os = "windows")]
    {
      // Primary location in AppData\Roaming
      let app_data = self.base_dirs.data_dir();
      let mullvad_dir = app_data.join("MullvadBrowser/Profiles");
      profiles.extend(self.scan_firefox_profiles_dir(&mullvad_dir, "mullvad-browser")?);

      // Also check common installation locations
      let local_app_data = self.base_dirs.data_local_dir();
      let mullvad_local_dir = local_app_data.join("MullvadBrowser/Profiles");
      if mullvad_local_dir.exists() {
        profiles.extend(self.scan_firefox_profiles_dir(&mullvad_local_dir, "mullvad-browser")?);
      }
    }

    #[cfg(target_os = "linux")]
    {
      let mullvad_dir = self.base_dirs.home_dir().join(".mullvad-browser");
      profiles.extend(self.scan_firefox_profiles_dir(&mullvad_dir, "mullvad-browser")?);
    }

    Ok(profiles)
  }

  /// Detect Zen Browser profiles
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

  /// Detect TOR Browser profiles
  fn detect_tor_browser_profiles(
    &self,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      // TOR Browser on macOS is typically in Applications
      let tor_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/TorBrowser-Data/Browser/profile.default");

      if tor_dir.exists() {
        profiles.push(DetectedProfile {
          browser: "tor-browser".to_string(),
          name: "TOR Browser - Default Profile".to_string(),
          path: tor_dir.to_string_lossy().to_string(),
          description: "Default TOR Browser profile".to_string(),
        });
      }
    }

    #[cfg(target_os = "windows")]
    {
      // Check common TOR Browser installation locations on Windows
      let possible_paths = [
        // Default installation in user directory
        (
          "Desktop",
          "Desktop/Tor Browser/Browser/TorBrowser/Data/Browser/profile.default",
        ),
        // AppData locations
        (
          "AppData/Roaming",
          "TorBrowser/Browser/TorBrowser/Data/Browser/profile.default",
        ),
        (
          "AppData/Local",
          "TorBrowser/Browser/TorBrowser/Data/Browser/profile.default",
        ),
      ];

      let home_dir = self.base_dirs.home_dir();

      for (location_name, relative_path) in &possible_paths {
        let tor_dir = home_dir.join(relative_path);
        if tor_dir.exists() {
          profiles.push(DetectedProfile {
            browser: "tor-browser".to_string(),
            name: format!("TOR Browser - {} Profile", location_name),
            path: tor_dir.to_string_lossy().to_string(),
            description: format!("TOR Browser profile from {}", location_name),
          });
        }
      }

      // Also check AppData directories if available
      let app_data = self.base_dirs.data_dir();
      let tor_app_data =
        app_data.join("TorBrowser/Browser/TorBrowser/Data/Browser/profile.default");
      if tor_app_data.exists() {
        profiles.push(DetectedProfile {
          browser: "tor-browser".to_string(),
          name: "TOR Browser - AppData Profile".to_string(),
          path: tor_app_data.to_string_lossy().to_string(),
          description: "TOR Browser profile from AppData".to_string(),
        });
      }
    }

    #[cfg(target_os = "linux")]
    {
      // Common TOR Browser locations on Linux
      let possible_paths = [
        ".local/share/torbrowser/tbb/x86_64/tor-browser_en-US/Browser/TorBrowser/Data/Browser/profile.default",
        "tor-browser_en-US/Browser/TorBrowser/Data/Browser/profile.default",
        ".tor-browser/Browser/TorBrowser/Data/Browser/profile.default",
        "Downloads/tor-browser_en-US/Browser/TorBrowser/Data/Browser/profile.default",
      ];

      let home_dir = self.base_dirs.home_dir();

      for relative_path in &possible_paths {
        let tor_dir = home_dir.join(relative_path);
        if tor_dir.exists() {
          profiles.push(DetectedProfile {
            browser: "tor-browser".to_string(),
            name: "TOR Browser - Default Profile".to_string(),
            path: tor_dir.to_string_lossy().to_string(),
            description: "TOR Browser profile".to_string(),
          });
          break; // Only add the first one found to avoid duplicates
        }
      }
    }

    Ok(profiles)
  }

  /// Scan Firefox-style profiles directory
  fn scan_firefox_profiles_dir(
    &self,
    profiles_dir: &Path,
    browser_type: &str,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    if !profiles_dir.exists() {
      return Ok(profiles);
    }

    // Read profiles.ini file if it exists
    let profiles_ini = profiles_dir
      .parent()
      .unwrap_or(profiles_dir)
      .join("profiles.ini");
    if profiles_ini.exists() {
      if let Ok(content) = fs::read_to_string(&profiles_ini) {
        profiles.extend(self.parse_firefox_profiles_ini(&content, profiles_dir, browser_type)?);
      }
    }

    // Also scan directory for any profile folders not in profiles.ini
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

            // Check if this profile was already found in profiles.ini
            let already_added = profiles.iter().any(|p| p.path == path.to_string_lossy());
            if !already_added {
              profiles.push(DetectedProfile {
                browser: browser_type.to_string(),
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

  /// Parse Firefox profiles.ini file
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
        // Save previous profile if complete
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
              name: display_name,
              path: full_path.to_string_lossy().to_string(),
              description: format!("Profile: {profile_name}"),
            });
          }
        }

        // Start new section
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

    // Handle last profile
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
          name: display_name,
          path: full_path.to_string_lossy().to_string(),
          description: format!("Profile: {profile_name}"),
        });
      }
    }

    Ok(profiles)
  }

  /// Scan Chrome-style profiles directory
  fn scan_chrome_profiles_dir(
    &self,
    browser_dir: &Path,
    browser_type: &str,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    if !browser_dir.exists() {
      return Ok(profiles);
    }

    // Check for Default profile
    let default_profile = browser_dir.join("Default");
    if default_profile.exists() && default_profile.join("Preferences").exists() {
      profiles.push(DetectedProfile {
        browser: browser_type.to_string(),
        name: format!(
          "{} - Default Profile",
          self.get_browser_display_name(browser_type)
        ),
        path: default_profile.to_string_lossy().to_string(),
        description: "Default profile".to_string(),
      });
    }

    // Check for Profile X directories
    if let Ok(entries) = fs::read_dir(browser_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
          let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

          if dir_name.starts_with("Profile ") && path.join("Preferences").exists() {
            let profile_number = &dir_name[8..]; // Remove "Profile " prefix
            profiles.push(DetectedProfile {
              browser: browser_type.to_string(),
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

  /// Get browser display name
  fn get_browser_display_name(&self, browser_type: &str) -> &str {
    match browser_type {
      "firefox" => "Firefox",
      "firefox-developer" => "Firefox Developer",
      "chromium" => "Chrome/Chromium",
      "brave" => "Brave",
      "mullvad-browser" => "Mullvad Browser",
      "zen" => "Zen Browser",
      "tor-browser" => "Tor Browser",
      _ => "Unknown Browser",
    }
  }

  /// Import a profile from an existing browser profile
  pub fn import_profile(
    &self,
    source_path: &str,
    browser_type: &str,
    new_profile_name: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    // Validate that source path exists
    let source_path = Path::new(source_path);
    if !source_path.exists() {
      return Err("Source profile path does not exist".into());
    }

    // Validate browser type
    let _browser_type = BrowserType::from_str(browser_type)
      .map_err(|_| format!("Invalid browser type: {browser_type}"))?;

    // Check if a profile with this name already exists
    let existing_profiles = self.browser_runner.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == new_profile_name.to_lowercase())
    {
      return Err(format!("Profile with name '{new_profile_name}' already exists").into());
    }

    // Create the new profile directory
    let snake_case_name = new_profile_name.to_lowercase().replace(' ', "_");
    let profiles_dir = self.browser_runner.get_profiles_dir();
    let new_profile_path = profiles_dir.join(&snake_case_name);

    create_dir_all(&new_profile_path)?;

    // Copy all files from source to destination
    Self::copy_directory_recursive(source_path, &new_profile_path)?;

    // Create the profile metadata without overwriting the imported data
    // We need to find a suitable version for this browser type
    let available_versions = self.get_default_version_for_browser(browser_type)?;

    let profile = crate::browser_runner::BrowserProfile {
      name: new_profile_name.to_string(),
      browser: browser_type.to_string(),
      version: available_versions,
      profile_path: new_profile_path.to_string_lossy().to_string(),
      proxy: None,
      process_id: None,
      last_launch: None,
    };

    // Save the profile metadata
    self.browser_runner.save_profile(&profile)?;

    println!(
      "Successfully imported profile '{}' from '{}'",
      new_profile_name,
      source_path.display()
    );

    Ok(())
  }

  /// Get a default version for a browser type
  fn get_default_version_for_browser(
    &self,
    browser_type: &str,
  ) -> Result<String, Box<dyn std::error::Error>> {
    // Try to get a downloaded version first, fallback to a reasonable default
    let registry =
      crate::downloaded_browsers::DownloadedBrowsersRegistry::load().unwrap_or_default();
    let downloaded_versions = registry.get_downloaded_versions(browser_type);

    if let Some(version) = downloaded_versions.first() {
      return Ok(version.clone());
    }

    // If no downloaded versions, return a sensible default
    match browser_type {
      "firefox" => Ok("latest".to_string()),
      "firefox-developer" => Ok("latest".to_string()),
      "chromium" => Ok("latest".to_string()),
      "brave" => Ok("latest".to_string()),
      "zen" => Ok("latest".to_string()),
      "mullvad-browser" => Ok("13.5.16".to_string()), // Mullvad Browser common version
      "tor-browser" => Ok("latest".to_string()),
      _ => Ok("latest".to_string()),
    }
  }

  /// Recursively copy directory contents
  fn copy_directory_recursive(
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

// Tauri commands
#[tauri::command]
pub async fn detect_existing_profiles() -> Result<Vec<DetectedProfile>, String> {
  let importer = ProfileImporter::new();
  importer
    .detect_existing_profiles()
    .map_err(|e| format!("Failed to detect existing profiles: {e}"))
}

#[tauri::command]
pub async fn import_browser_profile(
  source_path: String,
  browser_type: String,
  new_profile_name: String,
) -> Result<(), String> {
  let importer = ProfileImporter::new();
  importer
    .import_profile(&source_path, &browser_type, &new_profile_name)
    .map_err(|e| format!("Failed to import profile: {e}"))
}

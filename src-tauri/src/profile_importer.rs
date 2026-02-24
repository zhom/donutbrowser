use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};

use crate::browser::BrowserType;
use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::profile::ProfileManager;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DetectedProfile {
  pub browser: String,
  pub name: String,
  pub path: String,
  pub description: String,
}

pub struct ProfileImporter {
  base_dirs: BaseDirs,
  downloaded_browsers_registry: &'static DownloadedBrowsersRegistry,
  profile_manager: &'static ProfileManager,
}

impl ProfileImporter {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      downloaded_browsers_registry: DownloadedBrowsersRegistry::instance(),
      profile_manager: ProfileManager::instance(),
    }
  }

  pub fn instance() -> &'static ProfileImporter {
    &PROFILE_IMPORTER
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

    // Detect Zen Browser profiles
    detected_profiles.extend(self.detect_zen_browser_profiles()?);

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
      "zen" => "Zen Browser",
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
    let existing_profiles = self.profile_manager.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == new_profile_name.to_lowercase())
    {
      return Err(format!("Profile with name '{new_profile_name}' already exists").into());
    }

    // Generate UUID for new profile and create the directory structure
    let profile_id = uuid::Uuid::new_v4();
    let profiles_dir = self.profile_manager.get_profiles_dir();
    let new_profile_uuid_dir = profiles_dir.join(profile_id.to_string());
    let new_profile_data_dir = new_profile_uuid_dir.join("profile");

    create_dir_all(&new_profile_uuid_dir)?;
    create_dir_all(&new_profile_data_dir)?;

    // Copy all files from source to destination profile subdirectory
    Self::copy_directory_recursive(source_path, &new_profile_data_dir)?;

    // Create the profile metadata without overwriting the imported data
    // We need to find a suitable version for this browser type
    let available_versions = self.get_default_version_for_browser(browser_type)?;

    let profile = crate::profile::BrowserProfile {
      id: profile_id,
      name: new_profile_name.to_string(),
      browser: browser_type.to_string(),
      version: available_versions,
      proxy_id: None,
      vpn_id: None,
      process_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      camoufox_config: None,
      wayfern_config: None,
      group_id: None,
      tags: Vec::new(),
      note: None,
      sync_mode: crate::profile::types::SyncMode::Disabled,
      encryption_salt: None,
      last_sync: None,
      host_os: Some(crate::profile::types::get_host_os()),
      ephemeral: false,
    };

    // Save the profile metadata
    self.profile_manager.save_profile(&profile)?;

    log::info!(
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
    // Check if any version of the browser is downloaded
    let downloaded_versions = self
      .downloaded_browsers_registry
      .get_downloaded_versions(browser_type);

    if let Some(version) = downloaded_versions.first() {
      return Ok(version.clone());
    }

    // If no downloaded versions found, return an error
    Err(format!(
      "No downloaded versions found for browser '{}'. Please download a version of {} first before importing profiles.",
      browser_type,
      self.get_browser_display_name(browser_type)
    ).into())
  }

  /// Recursively copy directory contents
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

// Tauri commands
#[tauri::command]
pub async fn detect_existing_profiles() -> Result<Vec<DetectedProfile>, String> {
  let importer = ProfileImporter::instance();
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
  let importer = ProfileImporter::instance();
  importer
    .import_profile(&source_path, &browser_type, &new_profile_name)
    .map_err(|e| format!("Failed to import profile: {e}"))
}

// Global singleton instance
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

    // Set up a temporary home directory for testing
    env::set_var("HOME", temp_dir.path());

    let importer = ProfileImporter::new();
    (importer, temp_dir)
  }

  #[test]
  fn test_profile_importer_creation() {
    let (_importer, _temp_dir) = create_test_profile_importer();
    // Test passes if no panic occurs
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
  fn test_detect_existing_profiles_no_panic() {
    let (importer, _temp_dir) = create_test_profile_importer();

    // This should not panic even if no browser profiles exist
    let result = importer.detect_existing_profiles();
    assert!(result.is_ok(), "detect_existing_profiles should not fail");

    let _profiles = result.unwrap();
    // We can't assert specific profiles since they depend on the system
    // but we can verify the result is a valid Vec
    // We can't assert specific profiles since they depend on the system
    // but we can verify the result is a valid Vec (length check is always true for Vec, but shows intent)
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

    // Create a mock profile directory
    let profiles_dir = temp_dir.path().join("profiles");
    let profile_dir = profiles_dir.join("test.profile");
    fs::create_dir_all(&profile_dir).expect("Should create profile directory");

    // Create a prefs.js file to make it look like a valid profile
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
  }

  #[test]
  fn test_copy_directory_recursive() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create source directory structure
    let source_dir = temp_dir.path().join("source");
    let source_subdir = source_dir.join("subdir");
    fs::create_dir_all(&source_subdir).expect("Should create source directories");

    // Create some test files
    let source_file1 = source_dir.join("file1.txt");
    let source_file2 = source_subdir.join("file2.txt");
    fs::write(&source_file1, "content1").expect("Should create file1");
    fs::write(&source_file2, "content2").expect("Should create file2");

    // Create destination directory
    let dest_dir = temp_dir.path().join("dest");

    // Copy recursively
    let result = ProfileImporter::copy_directory_recursive(&source_dir, &dest_dir);
    assert!(result.is_ok(), "Should copy directory successfully");

    // Verify files were copied
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

    // This should fail since no versions are downloaded in test environment
    let result = importer.get_default_version_for_browser("firefox");
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

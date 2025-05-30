use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Emitter;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppReleaseAsset {
  pub name: String,
  pub browser_download_url: String,
  pub size: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppRelease {
  pub tag_name: String,
  pub name: String,
  pub body: String,
  pub published_at: String,
  pub prerelease: bool,
  pub assets: Vec<AppReleaseAsset>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppUpdateInfo {
  pub current_version: String,
  pub new_version: String,
  pub release_notes: String,
  pub download_url: String,
  pub is_nightly: bool,
  pub published_at: String,
}

pub struct AppAutoUpdater {
  client: Client,
}

impl AppAutoUpdater {
  pub fn new() -> Self {
    Self {
      client: Client::new(),
    }
  }

  /// Check if running a nightly build based on environment variable
  pub fn is_nightly_build() -> bool {
    // If STABLE_RELEASE env var is set at compile time, it's a stable build
    // Otherwise, it's a nightly build
    option_env!("STABLE_RELEASE").is_none()
  }

  /// Get current app version from Cargo.toml
  pub fn get_current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
  }

  /// Check for app updates
  pub async fn check_for_updates(
    &self,
  ) -> Result<Option<AppUpdateInfo>, Box<dyn std::error::Error + Send + Sync>> {
    let current_version = Self::get_current_version();
    let is_nightly = Self::is_nightly_build();

    println!("Checking for updates - Current version: {current_version}, Is nightly: {is_nightly}");

    let releases = self.fetch_app_releases().await?;

    // Filter releases based on build type
    let filtered_releases: Vec<&AppRelease> = if is_nightly {
      // For nightly builds, look for nightly releases
      releases
        .iter()
        .filter(|release| release.tag_name.starts_with("nightly-"))
        .collect()
    } else {
      // For stable builds, look for stable releases (semver format)
      releases
        .iter()
        .filter(|release| {
          release.tag_name.starts_with('v') && !release.tag_name.starts_with("nightly-")
        })
        .collect()
    };

    if filtered_releases.is_empty() {
      println!("No releases found for build type");
      return Ok(None);
    }

    // Get the latest release
    let latest_release = filtered_releases[0];

    // Check if we need to update
    if self.should_update(&current_version, &latest_release.tag_name, is_nightly) {
      // Find the appropriate asset for current platform
      if let Some(download_url) = self.get_download_url_for_platform(&latest_release.assets) {
        let update_info = AppUpdateInfo {
          current_version,
          new_version: latest_release.tag_name.clone(),
          release_notes: latest_release.body.clone(),
          download_url,
          is_nightly,
          published_at: latest_release.published_at.clone(),
        };

        return Ok(Some(update_info));
      }
    }

    Ok(None)
  }

  /// Fetch app releases from GitHub
  async fn fetch_app_releases(
    &self,
  ) -> Result<Vec<AppRelease>, Box<dyn std::error::Error + Send + Sync>> {
    let url = "https://api.github.com/repos/zhom/donutbrowser/releases";
    let response = self
      .client
      .get(url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(format!("GitHub API request failed: {}", response.status()).into());
    }

    let releases: Vec<AppRelease> = response.json().await?;
    Ok(releases)
  }

  /// Determine if an update should be performed
  fn should_update(&self, current_version: &str, new_version: &str, is_nightly: bool) -> bool {
    if is_nightly {
      // For nightly builds, always update if there's a newer nightly
      // Compare the commit hashes (assuming format: nightly-<commit_hash>)
      if let (Some(current_hash), Some(new_hash)) = (
        current_version.strip_prefix("nightly-"),
        new_version.strip_prefix("nightly-"),
      ) {
        return new_hash != current_hash;
      }
      // If current version doesn't have nightly prefix, it's an upgrade from stable to nightly
      !current_version.starts_with("nightly-")
    } else {
      // For stable builds, use semantic versioning comparison
      self.is_version_newer(new_version, current_version)
    }
  }

  /// Compare semantic versions (returns true if version1 > version2)
  fn is_version_newer(&self, version1: &str, version2: &str) -> bool {
    let v1 = self.parse_semver(version1);
    let v2 = self.parse_semver(version2);
    v1 > v2
  }

  /// Parse semantic version string into comparable tuple
  fn parse_semver(&self, version: &str) -> (u32, u32, u32) {
    let clean_version = version.trim_start_matches('v');
    let parts: Vec<&str> = clean_version.split('.').collect();

    let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    (major, minor, patch)
  }

  /// Get the appropriate download URL for the current platform
  fn get_download_url_for_platform(&self, assets: &[AppReleaseAsset]) -> Option<String> {
    let arch = if cfg!(target_arch = "aarch64") {
      "aarch64"
    } else {
      "x64"
    };

    // Look for macOS DMG with the appropriate architecture
    for asset in assets {
      if asset.name.contains(".dmg") && asset.name.contains(arch) {
        return Some(asset.browser_download_url.clone());
      }
    }

    // Fallback: look for any macOS DMG
    for asset in assets {
      if asset.name.contains(".dmg") {
        return Some(asset.browser_download_url.clone());
      }
    }

    None
  }

  /// Download and install app update
  pub async fn download_and_install_update(
    &self,
    app_handle: &tauri::AppHandle,
    update_info: &AppUpdateInfo,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Create temporary directory for download
    let temp_dir = std::env::temp_dir().join("donut_app_update");
    fs::create_dir_all(&temp_dir)?;

    // Extract filename from URL
    let filename = update_info
      .download_url
      .split('/')
      .next_back()
      .unwrap_or("update.dmg")
      .to_string();

    // Emit download start event
    let _ = app_handle.emit("app-update-progress", "Downloading update...");

    // Download the update
    let download_path = self
      .download_update(&update_info.download_url, &temp_dir, &filename)
      .await?;

    // Emit extraction start event
    let _ = app_handle.emit("app-update-progress", "Preparing update...");

    // Extract the update
    let extracted_app_path = self.extract_update(&download_path, &temp_dir).await?;

    // Emit installation start event
    let _ = app_handle.emit("app-update-progress", "Installing update...");

    // Install the update (overwrite current app)
    self.install_update(&extracted_app_path).await?;

    // Clean up temporary files
    let _ = fs::remove_dir_all(&temp_dir);

    // Emit completion event
    let _ = app_handle.emit("app-update-progress", "Update completed. Restarting...");

    // Restart the application
    self.restart_application().await?;

    Ok(())
  }

  /// Download the update file
  async fn download_update(
    &self,
    download_url: &str,
    dest_dir: &Path,
    filename: &str,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let file_path = dest_dir.join(filename);

    let response = self
      .client
      .get(download_url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(format!("Download failed with status: {}", response.status()).into());
    }

    let mut file = fs::File::create(&file_path)?;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
      let chunk = chunk?;
      file.write_all(&chunk)?;
    }

    Ok(file_path)
  }

  /// Extract the update (DMG on macOS)
  async fn extract_update(
    &self,
    dmg_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // For DMG files on macOS, we need to mount and copy the .app
    let mount_point = dest_dir.join("mount");
    fs::create_dir_all(&mount_point)?;

    // Mount the DMG
    let output = Command::new("hdiutil")
      .args([
        "attach",
        "-nobrowse",
        "-mountpoint",
        mount_point.to_str().unwrap(),
        dmg_path.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to mount DMG: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Find the .app in the mount point
    let app_entry = fs::read_dir(&mount_point)?
      .filter_map(Result::ok)
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "app"))
      .ok_or("No .app found in DMG")?;

    let app_path = dest_dir.join("extracted_app");
    if app_path.exists() {
      fs::remove_dir_all(&app_path)?;
    }

    // Copy the .app to extraction directory
    let output = Command::new("cp")
      .args([
        "-R",
        app_entry.path().to_str().unwrap(),
        app_path.to_str().unwrap(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(
        format!(
          "Failed to copy app: {}",
          String::from_utf8_lossy(&output.stderr)
        )
        .into(),
      );
    }

    // Unmount the DMG
    let _ = Command::new("hdiutil")
      .args(["detach", mount_point.to_str().unwrap()])
      .output();

    Ok(app_path)
  }

  /// Install the update by replacing the current app
  async fn install_update(
    &self,
    new_app_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get the current application bundle path
    let current_app_path = self.get_current_app_path()?;

    // Create a backup of the current app
    let backup_path = current_app_path.with_extension("app.backup");
    if backup_path.exists() {
      fs::remove_dir_all(&backup_path)?;
    }

    // Move current app to backup
    fs::rename(&current_app_path, &backup_path)?;

    // Move new app to current location
    fs::rename(new_app_path, &current_app_path)?;

    // Remove quarantine attributes from the new app
    let _ = Command::new("xattr")
      .args([
        "-dr",
        "com.apple.quarantine",
        current_app_path.to_str().unwrap(),
      ])
      .output();

    let _ = Command::new("xattr")
      .args(["-cr", current_app_path.to_str().unwrap()])
      .output();

    // Clean up backup after successful installation
    let _ = fs::remove_dir_all(&backup_path);

    Ok(())
  }

  /// Get the current application bundle path
  fn get_current_app_path(&self) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Get the current executable path
    let exe_path = std::env::current_exe()?;

    // Navigate up to find the .app bundle
    let mut current = exe_path.as_path();
    while let Some(parent) = current.parent() {
      if parent.extension().is_some_and(|ext| ext == "app") {
        return Ok(parent.to_path_buf());
      }
      current = parent;
    }

    Err("Could not find application bundle".into())
  }

  /// Restart the application
  async fn restart_application(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app_path = self.get_current_app_path()?;

    // Use open command to restart the app
    let _ = Command::new("open")
      .args([app_path.to_str().unwrap()])
      .spawn()?;

    // Exit current process after a short delay
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    std::process::exit(0);
  }
}

// Tauri commands

#[tauri::command]
pub async fn check_for_app_updates() -> Result<Option<AppUpdateInfo>, String> {
  let updater = AppAutoUpdater::new();
  updater
    .check_for_updates()
    .await
    .map_err(|e| format!("Failed to check for app updates: {e}"))
}

#[tauri::command]
pub async fn download_and_install_app_update(
  app_handle: tauri::AppHandle,
  update_info: AppUpdateInfo,
) -> Result<(), String> {
  let updater = AppAutoUpdater::new();
  updater
    .download_and_install_update(&app_handle, &update_info)
    .await
    .map_err(|e| format!("Failed to install app update: {e}"))
}

#[tauri::command]
pub fn get_app_version_info() -> Result<(String, bool), String> {
  Ok((
    AppAutoUpdater::get_current_version(),
    AppAutoUpdater::is_nightly_build(),
  ))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_is_nightly_build() {
    // This will depend on whether STABLE_RELEASE is set during test compilation
    let is_nightly = AppAutoUpdater::is_nightly_build();
    println!("Is nightly build: {is_nightly}");
  }

  #[test]
  fn test_version_comparison() {
    let updater = AppAutoUpdater::new();

    // Test semantic version comparison
    assert!(updater.is_version_newer("v1.1.0", "v1.0.0"));
    assert!(updater.is_version_newer("v2.0.0", "v1.9.9"));
    assert!(updater.is_version_newer("v1.0.1", "v1.0.0"));
    assert!(!updater.is_version_newer("v1.0.0", "v1.0.0"));
    assert!(!updater.is_version_newer("v1.0.0", "v1.0.1"));
  }

  #[test]
  fn test_parse_semver() {
    let updater = AppAutoUpdater::new();

    assert_eq!(updater.parse_semver("v1.2.3"), (1, 2, 3));
    assert_eq!(updater.parse_semver("1.2.3"), (1, 2, 3));
    assert_eq!(updater.parse_semver("v2.0.0"), (2, 0, 0));
    assert_eq!(updater.parse_semver("0.1.0"), (0, 1, 0));
  }

  #[test]
  fn test_should_update_stable() {
    let updater = AppAutoUpdater::new();

    // Stable version updates
    assert!(updater.should_update("v1.0.0", "v1.1.0", false));
    assert!(updater.should_update("v1.0.0", "v2.0.0", false));
    assert!(!updater.should_update("v1.1.0", "v1.0.0", false));
    assert!(!updater.should_update("v1.0.0", "v1.0.0", false));
  }

  #[test]
  fn test_should_update_nightly() {
    let updater = AppAutoUpdater::new();

    // Nightly version updates
    assert!(updater.should_update("nightly-abc123", "nightly-def456", true));
    assert!(!updater.should_update("nightly-abc123", "nightly-abc123", true));

    // Upgrade from stable to nightly
    assert!(updater.should_update("v1.0.0", "nightly-abc123", true));
  }

  #[test]
  fn test_get_download_url_for_platform() {
    let updater = AppAutoUpdater::new();

    let assets = vec![
      AppReleaseAsset {
        name: "Donut.Browser_0.1.0_x64.dmg".to_string(),
        browser_download_url: "https://example.com/x64.dmg".to_string(),
        size: 12345,
      },
      AppReleaseAsset {
        name: "Donut.Browser_0.1.0_aarch64.dmg".to_string(),
        browser_download_url: "https://example.com/aarch64.dmg".to_string(),
        size: 12345,
      },
    ];

    let url = updater.get_download_url_for_platform(&assets);
    assert!(url.is_some());

    // The exact URL depends on the target architecture
    let url = url.unwrap();
    assert!(url.contains(".dmg"));
  }
}

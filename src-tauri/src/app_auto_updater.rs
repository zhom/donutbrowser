/*!
# App Auto Updater

This module provides comprehensive self-update functionality for the Donut Browser application
across multiple operating systems and installation methods.

## Supported Platforms

### macOS
- **Format**: DMG files
- **Installation**: Replaces the .app bundle in place
- **Architecture**: Supports both x64 and aarch64 (Apple Silicon)

### Windows
- **Formats**: MSI (preferred), EXE, ZIP
- **Installation**:
  - MSI: Silent installation using msiexec
  - EXE: Silent installation with multiple fallback flags (NSIS, Inno Setup)
  - ZIP: Binary replacement
- **Architecture**: Supports both x64 and x86_64

### Linux
- **Formats**: DEB, RPM, AppImage, TAR.GZ
- **Installation Methods**:
  - **DEB**: Uses dpkg or apt with pkexec for privilege escalation
  - **RPM**: Uses rpm, dnf, yum, or zypper with pkexec
  - **AppImage**: Direct replacement or installation to ~/.local/bin
  - **TAR.GZ**: Binary extraction and replacement
- **Architecture**: Supports x64, x86_64, amd64, aarch64, arm64

## Linux Installation Detection

The updater automatically detects how the application was installed:
- **AppImage**: Detected via APPIMAGE environment variable
- **Package Manager**: Detected by executable location and package queries
- **Manual**: Detected by location in user directories
- **System**: Detected by location in system directories

## Update Process

1. **Check**: Fetches releases from GitHub API
2. **Filter**: Filters releases based on build type (stable vs nightly)
3. **Compare**: Compares versions using semantic versioning or commit hashes
4. **Download**: Downloads appropriate asset with progress tracking
5. **Extract**: Extracts or prepares installer based on format
6. **Install**: Installs using platform-appropriate method
7. **Restart**: Restarts application after successful installation

## Error Handling

- Comprehensive error messages for each platform
- Fallback mechanisms for different package managers
- Backup creation before installation
- Cleanup of temporary files
- Graceful handling of permission issues

## Testing

Includes comprehensive unit tests for:
- Version comparison logic
- Platform detection
- Asset selection
- Installation method detection (Linux)
- File format support
*/

use crate::events;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
enum LinuxInstallationMethod {
  Deb,      // Installed via DEB package
  Rpm,      // Installed via RPM package
  AppImage, // Running from AppImage
  Manual,   // Manually installed (e.g., extracted tarball)
  Unknown,  // Cannot determine
}

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
  pub manual_update_required: bool,
  pub release_page_url: Option<String>,
}

pub struct AppAutoUpdater {
  client: Client,
  extractor: &'static crate::extraction::Extractor,
}

impl AppAutoUpdater {
  fn new() -> Self {
    Self {
      client: Client::new(),
      extractor: crate::extraction::Extractor::instance(),
    }
  }

  pub fn instance() -> &'static AppAutoUpdater {
    &APP_AUTO_UPDATER
  }

  /// Check if running a nightly build based on environment variable
  pub fn is_nightly_build() -> bool {
    // If STABLE_RELEASE env var is set at compile time, it's a stable build
    if option_env!("STABLE_RELEASE").is_some() {
      return false;
    }

    // Also check if the current version starts with "nightly-"
    let current_version = Self::get_current_version();
    if current_version.starts_with("nightly-") {
      return true;
    }

    // If STABLE_RELEASE is not set and version doesn't start with "nightly-",
    // it's still considered a nightly build (dev builds, main branch builds, etc.)
    true
  }

  /// Get current app version from build-time injection
  pub fn get_current_version() -> String {
    // Use build-time injected version instead of CARGO_PKG_VERSION
    env!("BUILD_VERSION").to_string()
  }

  /// Check for app updates
  pub async fn check_for_updates(
    &self,
  ) -> Result<Option<AppUpdateInfo>, Box<dyn std::error::Error + Send + Sync>> {
    let current_version = Self::get_current_version();
    let is_nightly = Self::is_nightly_build();

    log::info!("=== App Update Check ===");
    log::info!("Current version: {current_version}");
    log::info!("Is nightly build: {is_nightly}");
    log::info!("STABLE_RELEASE env: {:?}", option_env!("STABLE_RELEASE"));

    let releases = self.fetch_app_releases().await?;
    log::info!("Fetched {} releases from GitHub", releases.len());

    // Filter releases based on build type
    let filtered_releases: Vec<&AppRelease> = if is_nightly {
      // For nightly builds, look for nightly releases
      let nightly_releases: Vec<&AppRelease> = releases
        .iter()
        .filter(|release| release.tag_name.starts_with("nightly-"))
        .collect();
      log::info!("Found {} nightly releases", nightly_releases.len());
      nightly_releases
    } else {
      // For stable builds, look for stable releases (semver format)
      let stable_releases: Vec<&AppRelease> = releases
        .iter()
        .filter(|release| release.tag_name.starts_with('v'))
        .collect();
      log::info!("Found {} stable releases", stable_releases.len());
      stable_releases
    };

    if filtered_releases.is_empty() {
      log::info!("No releases found for build type (nightly: {is_nightly})");
      return Ok(None);
    }

    // Get the latest release
    let latest_release = filtered_releases[0];
    log::info!(
      "Latest release: {} ({})",
      latest_release.tag_name,
      latest_release.name
    );

    // Check if we need to update
    if self.should_update(&current_version, &latest_release.tag_name, is_nightly) {
      log::info!("Update available!");

      // Build the release page URL
      let release_page_url = format!(
        "https://github.com/zhom/donutbrowser/releases/tag/{}",
        latest_release.tag_name
      );

      // Find the appropriate asset for current platform
      let download_url = self.get_download_url_for_platform(&latest_release.assets);

      // On Linux, we show the update notification even if auto-update is disabled
      // Users can manually download from the release page
      #[cfg(target_os = "linux")]
      {
        let manual_update_required = download_url.is_none();
        let update_info = AppUpdateInfo {
          current_version,
          new_version: latest_release.tag_name.clone(),
          release_notes: latest_release.body.clone(),
          download_url: download_url.unwrap_or_else(|| release_page_url.clone()),
          is_nightly,
          published_at: latest_release.published_at.clone(),
          manual_update_required,
          release_page_url: Some(release_page_url),
        };

        log::info!(
          "Update info prepared: {} -> {} (manual_update_required: {})",
          update_info.current_version,
          update_info.new_version,
          update_info.manual_update_required
        );
        return Ok(Some(update_info));
      }

      #[cfg(not(target_os = "linux"))]
      {
        if let Some(url) = download_url {
          let update_info = AppUpdateInfo {
            current_version,
            new_version: latest_release.tag_name.clone(),
            release_notes: latest_release.body.clone(),
            download_url: url,
            is_nightly,
            published_at: latest_release.published_at.clone(),
            manual_update_required: false,
            release_page_url: Some(release_page_url),
          };

          log::info!(
            "Update info prepared: {} -> {}",
            update_info.current_version,
            update_info.new_version
          );
          return Ok(Some(update_info));
        } else {
          log::info!("No suitable download asset found for current platform");
        }
      }
    } else {
      log::info!("No update needed");
    }

    Ok(None)
  }

  /// Fetch app releases from GitHub
  async fn fetch_app_releases(
    &self,
  ) -> Result<Vec<AppRelease>, Box<dyn std::error::Error + Send + Sync>> {
    let url = "https://api.github.com/repos/zhom/donutbrowser/releases?per_page=100";
    let response = self
      .client
      .get(url)
      .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
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
    if current_version.starts_with("dev-") {
      return false;
    }

    log::info!(
      "Comparing versions: current={current_version}, new={new_version}, is_nightly={is_nightly}"
    );

    if is_nightly {
      // For nightly builds, always update if there's a newer nightly
      if let (Some(current_hash), Some(new_hash)) = (
        current_version.strip_prefix("nightly-"),
        new_version.strip_prefix("nightly-"),
      ) {
        // Different commit hashes mean we should update
        let should_update = new_hash != current_hash;
        log::info!("Nightly comparison: current_hash={current_hash}, new_hash={new_hash}, should_update={should_update}");
        return should_update;
      }

      // If current version doesn't have nightly prefix but we're in nightly mode,
      // this could be a dev build or stable build upgrading to nightly
      if !current_version.starts_with("nightly-") {
        log::info!("Upgrading from non-nightly to nightly: {new_version}");
        return true;
      }
    } else {
      // For stable builds, use semantic versioning comparison
      let should_update = self.is_version_newer(new_version, current_version);
      log::info!("Stable comparison: {new_version} > {current_version} = {should_update}");
      return should_update;
    }

    false
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

  /// Detect if we're running from an AppImage
  #[cfg(target_os = "linux")]
  fn is_running_from_appimage(&self) -> bool {
    // Check APPIMAGE environment variable first
    if std::env::var("APPIMAGE").is_ok() {
      return true;
    }

    // Check if current executable path looks like an AppImage
    if let Ok(exe_path) = std::env::current_exe() {
      if let Some(file_name) = exe_path.file_name().and_then(|n| n.to_str()) {
        if file_name.to_lowercase().contains("appimage") {
          return true;
        }
      }

      // Check if the executable is in a temporary mount point (typical for AppImages)
      if let Some(path_str) = exe_path.to_str() {
        if path_str.contains("/tmp/.mount_") || path_str.contains("/tmp/appimage") {
          return true;
        }
      }
    }

    false
  }

  /// Detect how the application was installed on Linux
  #[cfg(target_os = "linux")]
  fn detect_linux_installation_method(&self) -> LinuxInstallationMethod {
    // First check if we're running from an AppImage
    if self.is_running_from_appimage() {
      return LinuxInstallationMethod::AppImage;
    }

    // Get current executable path
    let exe_path = match std::env::current_exe() {
      Ok(path) => path,
      Err(_) => return LinuxInstallationMethod::Unknown,
    };

    let exe_path_str = exe_path.to_string_lossy();
    log::info!("Detecting installation method for: {exe_path_str}");

    // Check if installed via package manager by querying package databases
    if let Some(exe_name) = exe_path.file_name().and_then(|n| n.to_str()) {
      // Try to find the package that owns this file

      // Check DEB systems (dpkg)
      if let Ok(output) = Command::new("dpkg").args(["-S", &exe_path_str]).output() {
        if output.status.success() {
          let stdout = String::from_utf8_lossy(&output.stdout);
          if !stdout.trim().is_empty() && !stdout.contains("no path found") {
            log::info!("Found DEB package owning the executable");
            return LinuxInstallationMethod::Deb;
          }
        }
      }

      // Check RPM systems (rpm)
      if let Ok(output) = Command::new("rpm").args(["-qf", &exe_path_str]).output() {
        if output.status.success() {
          let stdout = String::from_utf8_lossy(&output.stdout);
          if !stdout.trim().is_empty() && !stdout.contains("not owned") {
            log::info!("Found RPM package owning the executable");
            return LinuxInstallationMethod::Rpm;
          }
        }
      }

      // Alternative RPM check with different systems
      for rpm_cmd in &["dnf", "yum", "zypper"] {
        if let Ok(output) = Command::new(rpm_cmd)
          .args(["provides", &exe_path_str])
          .output()
        {
          if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.trim().is_empty() && stdout.contains(exe_name) {
              log::info!("Found RPM package via {rpm_cmd}");
              return LinuxInstallationMethod::Rpm;
            }
          }
        }
      }
    }

    // Check installation location to infer method
    if exe_path_str.starts_with("/usr/bin/") || exe_path_str.starts_with("/usr/local/bin/") {
      // Likely installed via package manager or system-wide installation
      log::info!("Executable in system directory, assuming package installation");

      // Try to determine which package system is available
      if Command::new("dpkg").arg("--version").output().is_ok() {
        return LinuxInstallationMethod::Deb;
      } else if Command::new("rpm").arg("--version").output().is_ok() {
        return LinuxInstallationMethod::Rpm;
      }

      return LinuxInstallationMethod::Manual;
    } else if exe_path_str.contains("/.local/") || exe_path_str.starts_with("/home/") {
      // User-local installation
      log::info!("Executable in user directory, assuming manual installation");
      return LinuxInstallationMethod::Manual;
    }

    log::info!("Could not determine installation method");
    LinuxInstallationMethod::Unknown
  }

  /// Get the appropriate download URL for the current platform
  fn get_download_url_for_platform(&self, assets: &[AppReleaseAsset]) -> Option<String> {
    let arch = if cfg!(target_arch = "aarch64") {
      "aarch64"
    } else if cfg!(target_arch = "x86_64") {
      "x64"
    } else {
      "unknown"
    };

    log::info!("Looking for platform-specific asset for arch: {arch}");

    #[cfg(target_os = "linux")]
    {
      // If we're running from an AppImage, disable auto-updates for safety
      if self.is_running_from_appimage() {
        log::info!("Running from AppImage - auto-updates disabled for safety");
        return None;
      }
    }

    #[cfg(target_os = "macos")]
    {
      self.get_macos_download_url(assets, arch)
    }

    #[cfg(target_os = "windows")]
    {
      self.get_windows_download_url(assets, arch)
    }

    #[cfg(target_os = "linux")]
    {
      self.get_linux_download_url(assets, arch)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
      log::info!("Unsupported platform for auto-update");
      None
    }
  }

  #[cfg(target_os = "macos")]
  fn get_macos_download_url(&self, assets: &[AppReleaseAsset], arch: &str) -> Option<String> {
    // Look for exact architecture match in DMG
    for asset in assets {
      if asset.name.contains(".dmg")
        && (asset.name.contains(&format!("_{arch}.dmg"))
          || asset.name.contains(&format!("-{arch}.dmg"))
          || asset.name.contains(&format!("_{arch}_"))
          || asset.name.contains(&format!("-{arch}-")))
      {
        log::info!("Found exact architecture match: {}", asset.name);
        return Some(asset.browser_download_url.clone());
      }
    }

    // Look for x86_64 variations if we're looking for x64
    if arch == "x64" {
      for asset in assets {
        if asset.name.contains(".dmg")
          && (asset.name.contains("x86_64") || asset.name.contains("x86-64"))
        {
          log::info!("Found x86_64 variant: {}", asset.name);
          return Some(asset.browser_download_url.clone());
        }
      }
    }

    // Look for arm64 variations if we're looking for aarch64
    if arch == "aarch64" {
      for asset in assets {
        if asset.name.contains(".dmg")
          && (asset.name.contains("arm64") || asset.name.contains("aarch64"))
        {
          log::info!("Found arm64 variant: {}", asset.name);
          return Some(asset.browser_download_url.clone());
        }
      }
    }

    // Fallback to any macOS DMG
    for asset in assets {
      if asset.name.contains(".dmg")
        && (asset.name.to_lowercase().contains("macos")
          || asset.name.to_lowercase().contains("darwin")
          || !asset.name.contains(".app.tar.gz"))
      {
        log::info!("Found fallback DMG: {}", asset.name);
        return Some(asset.browser_download_url.clone());
      }
    }

    None
  }

  #[cfg(target_os = "windows")]
  fn get_windows_download_url(&self, assets: &[AppReleaseAsset], arch: &str) -> Option<String> {
    // Priority order: MSI > EXE > ZIP
    let extensions = ["msi", "exe", "zip"];

    for ext in &extensions {
      // Look for exact architecture match
      for asset in assets {
        if asset.name.to_lowercase().ends_with(&format!(".{ext}"))
          && (asset.name.contains(&format!("_{arch}.{ext}"))
            || asset.name.contains(&format!("-{arch}.{ext}"))
            || asset.name.contains(&format!("_{arch}_"))
            || asset.name.contains(&format!("-{arch}-")))
        {
          log::info!("Found Windows {ext} with exact arch match: {}", asset.name);
          return Some(asset.browser_download_url.clone());
        }
      }

      // Look for x86_64 variations if we're looking for x64
      if arch == "x64" {
        for asset in assets {
          if asset.name.to_lowercase().ends_with(&format!(".{ext}"))
            && (asset.name.contains("x86_64") || asset.name.contains("x86-64"))
          {
            log::info!("Found Windows {ext} with x86_64 variant: {}", asset.name);
            return Some(asset.browser_download_url.clone());
          }
        }
      }

      // Fallback to any Windows file of this type
      for asset in assets {
        if asset.name.to_lowercase().ends_with(&format!(".{ext}"))
          && (asset.name.to_lowercase().contains("windows")
            || asset.name.to_lowercase().contains("win32")
            || asset.name.to_lowercase().contains("win64"))
        {
          log::info!("Found Windows {ext} fallback: {}", asset.name);
          return Some(asset.browser_download_url.clone());
        }
      }
    }

    None
  }

  #[cfg(target_os = "linux")]
  fn get_linux_download_url(&self, assets: &[AppReleaseAsset], arch: &str) -> Option<String> {
    // Detect installation method to prioritize appropriate formats
    let installation_method = self.detect_linux_installation_method();
    log::info!("Detected Linux installation method: {installation_method:?}");

    // Priority order based on installation method
    let extensions = match installation_method {
      LinuxInstallationMethod::Deb => vec!["deb", "tar.gz"],
      LinuxInstallationMethod::Rpm => vec!["rpm", "tar.gz"],
      LinuxInstallationMethod::AppImage => {
        // AppImages should not auto-update for safety
        log::info!("AppImage installation detected - auto-updates disabled");
        return None;
      }
      LinuxInstallationMethod::Manual | LinuxInstallationMethod::Unknown => {
        vec!["deb", "rpm", "tar.gz"]
      }
    };

    for ext in &extensions {
      // Look for exact architecture match
      for asset in assets {
        let asset_name_lower = asset.name.to_lowercase();
        if asset_name_lower.ends_with(&format!(".{ext}"))
          && (asset.name.contains(&format!("_{arch}.{ext}"))
            || asset.name.contains(&format!("-{arch}.{ext}"))
            || asset.name.contains(&format!("_{arch}_"))
            || asset.name.contains(&format!("-{arch}-")))
        {
          log::info!("Found Linux {ext} with exact arch match: {}", asset.name);
          return Some(asset.browser_download_url.clone());
        }
      }

      // Look for x86_64 variations if we're looking for x64
      if arch == "x64" {
        for asset in assets {
          let asset_name_lower = asset.name.to_lowercase();
          if asset_name_lower.ends_with(&format!(".{ext}"))
            && (asset.name.contains("x86_64")
              || asset.name.contains("x86-64")
              || asset.name.contains("amd64"))
          {
            log::info!("Found Linux {ext} with x86_64 variant: {}", asset.name);
            return Some(asset.browser_download_url.clone());
          }
        }
      }

      // Look for arm64 variations if we're looking for aarch64
      if arch == "aarch64" {
        for asset in assets {
          let asset_name_lower = asset.name.to_lowercase();
          if asset_name_lower.ends_with(&format!(".{ext}"))
            && (asset.name.contains("arm64") || asset.name.contains("aarch64"))
          {
            log::info!("Found Linux {ext} with arm64 variant: {}", asset.name);
            return Some(asset.browser_download_url.clone());
          }
        }
      }

      // Fallback to any Linux file of this type
      for asset in assets {
        let asset_name_lower = asset.name.to_lowercase();
        if asset_name_lower.ends_with(&format!(".{ext}"))
          && (asset_name_lower.contains("linux")
            || asset_name_lower.contains("ubuntu")
            || asset_name_lower.contains("debian"))
        {
          log::info!("Found Linux {ext} fallback: {}", asset.name);
          return Some(asset.browser_download_url.clone());
        }
      }
    }

    None
  }

  /// Download the update file without progress tracking (silent download)
  async fn download_update_silent(
    &self,
    download_url: &str,
    dest_dir: &Path,
    filename: &str,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let file_path = dest_dir.join(filename);

    let response = self
      .client
      .get(download_url)
      .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(format!("Download failed with status: {}", response.status()).into());
    }

    let total_size = response.content_length().unwrap_or(0);
    log::info!("Silent download size: {} bytes", total_size);
    let mut file = fs::File::create(&file_path)?;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
      let chunk = chunk?;
      file.write_all(&chunk)?;
    }

    log::info!("Silent download completed: {}", file_path.display());
    Ok(file_path)
  }

  /// Download and prepare app update (silent download + install + notify)
  pub async fn download_and_prepare_update(
    &self,
    _app_handle: &tauri::AppHandle,
    update_info: &AppUpdateInfo,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Starting background update download and install");

    let temp_dir = std::env::temp_dir().join("donut_app_update");
    fs::create_dir_all(&temp_dir)?;

    let filename = update_info
      .download_url
      .split('/')
      .next_back()
      .unwrap_or("update.dmg")
      .to_string();

    log::info!("Downloading update from: {}", update_info.download_url);

    let download_path = self
      .download_update_silent(&update_info.download_url, &temp_dir, &filename)
      .await?;

    log::info!("Extracting update...");
    let extracted_app_path = self.extract_update(&download_path, &temp_dir).await?;

    log::info!("Installing update (overwriting binary)...");
    self.install_update(&extracted_app_path).await?;

    log::info!("Cleaning up temporary files...");
    let _ = fs::remove_dir_all(&temp_dir);

    log::info!("Update installed successfully, emitting app-update-ready event");

    let _ = events::emit("app-update-ready", update_info.new_version.clone());

    Ok(())
  }

  /// Extract the update using the extraction module
  async fn extract_update(
    &self,
    archive_path: &Path,
    dest_dir: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let file_name = archive_path
      .file_name()
      .and_then(|name| name.to_str())
      .unwrap_or("");

    // Handle compound extensions like .tar.gz
    if file_name.ends_with(".tar.gz") {
      return self.extractor.extract_tar_gz(archive_path, dest_dir).await;
    }

    let extension = archive_path
      .extension()
      .and_then(|ext| ext.to_str())
      .unwrap_or("");

    match extension {
      "dmg" => {
        #[cfg(target_os = "macos")]
        {
          self.extractor.extract_dmg(archive_path, dest_dir).await
        }
        #[cfg(not(target_os = "macos"))]
        {
          Err("DMG extraction is only supported on macOS".into())
        }
      }
      "msi" => {
        #[cfg(target_os = "windows")]
        {
          // For MSI files on Windows, we need to run the installer
          // MSI files can't be extracted like archives, they need to be executed
          // Return the path to the MSI file itself for installation
          Ok(archive_path.to_path_buf())
        }
        #[cfg(not(target_os = "windows"))]
        {
          Err("MSI installation is only supported on Windows".into())
        }
      }
      "exe" => {
        #[cfg(target_os = "windows")]
        {
          // For exe installers on Windows, return the path for execution
          Ok(archive_path.to_path_buf())
        }
        #[cfg(not(target_os = "windows"))]
        {
          Err("EXE installation is only supported on Windows".into())
        }
      }
      "deb" => {
        #[cfg(target_os = "linux")]
        {
          // For DEB files on Linux, return the path for installation
          Ok(archive_path.to_path_buf())
        }
        #[cfg(not(target_os = "linux"))]
        {
          Err("DEB installation is only supported on Linux".into())
        }
      }
      "rpm" => {
        #[cfg(target_os = "linux")]
        {
          // For RPM files on Linux, return the path for installation
          Ok(archive_path.to_path_buf())
        }
        #[cfg(not(target_os = "linux"))]
        {
          Err("RPM installation is only supported on Linux".into())
        }
      }
      "appimage" => {
        #[cfg(target_os = "linux")]
        {
          // For AppImage files, return the path for installation
          Ok(archive_path.to_path_buf())
        }
        #[cfg(not(target_os = "linux"))]
        {
          Err("AppImage installation is only supported on Linux".into())
        }
      }
      "zip" => self.extractor.extract_zip(archive_path, dest_dir).await,
      _ => Err(format!("Unsupported archive format: {extension}").into()),
    }
  }

  /// Install the update by replacing the current app
  async fn install_update(
    &self,
    #[allow(unused_variables)] installer_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_os = "macos")]
    {
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
      fs::rename(installer_path, &current_app_path)?;

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

      // Clean up old "Donut Browser.app" if it exists (from before the project rename)
      if let Some(parent_dir) = current_app_path.parent() {
        let old_app_path = parent_dir.join("Donut Browser.app");
        if old_app_path.exists() && old_app_path != current_app_path {
          log::info!(
            "Removing old 'Donut Browser.app' from: {}",
            old_app_path.display()
          );
          if let Err(e) = fs::remove_dir_all(&old_app_path) {
            log::warn!("Warning: Failed to remove old 'Donut Browser.app': {e}");
          } else {
            log::info!("Successfully removed old 'Donut Browser.app'");
          }
        }
      }

      Ok(())
    }

    #[cfg(target_os = "windows")]
    {
      let extension = installer_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

      log::info!("Installing Windows update with extension: {extension}");

      match extension {
        "msi" => {
          // Install MSI silently with enhanced error handling
          log::info!("Running MSI installer: {}", installer_path.display());

          let mut cmd = Command::new("msiexec");
          cmd.args([
            "/i",
            installer_path.to_str().unwrap(),
            "/quiet",
            "/norestart",
            "REBOOT=ReallySuppress",
            "/l*v", // Enable verbose logging
            &format!("{}.log", installer_path.to_str().unwrap()),
          ]);

          let output = cmd.output()?;

          if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            // Try to read the log file for more details
            let log_path = format!("{}.log", installer_path.to_str().unwrap());
            let log_content = fs::read_to_string(&log_path).unwrap_or_default();

            log::info!("MSI installation failed with exit code: {exit_code}");
            log::info!("Error output: {error_msg}");
            if !log_content.is_empty() {
              log::info!(
                "Log file content (last 500 chars): {}",
                &log_content
                  .chars()
                  .rev()
                  .take(500)
                  .collect::<String>()
                  .chars()
                  .rev()
                  .collect::<String>()
              );
            }

            return Err(
              format!("MSI installation failed (exit code {exit_code}): {error_msg}").into(),
            );
          }

          log::info!("MSI installation completed successfully");
        }
        "exe" => {
          // Run exe installer silently with multiple fallback options
          log::info!("Running EXE installer: {}", installer_path.display());

          // Try NSIS silent flag first (most common for Tauri)
          let mut success = false;
          let mut last_error = String::new();

          // NSIS installer flags (used by Tauri)
          let nsis_args = vec![
            vec!["/S"],                                             // Standard NSIS silent flag
            vec!["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"], // Inno Setup flags
            vec!["/quiet"],                                         // Generic quiet flag
            vec!["/silent"],                                        // Alternative silent flag
          ];

          for args in nsis_args {
            log::info!("Trying installer with args: {:?}", args);
            let output = Command::new(installer_path).args(&args).output();

            match output {
              Ok(output) if output.status.success() => {
                log::info!(
                  "EXE installation completed successfully with args: {:?}",
                  args
                );
                success = true;
                break;
              }
              Ok(output) => {
                let error_msg = String::from_utf8_lossy(&output.stderr);
                last_error = format!(
                  "Exit code {}: {}",
                  output.status.code().unwrap_or(-1),
                  error_msg
                );
                log::info!("Installer failed with args {:?}: {}", args, last_error);
              }
              Err(e) => {
                last_error = format!("Failed to execute installer: {e}");
                log::info!(
                  "Failed to execute installer with args {:?}: {}",
                  args,
                  last_error
                );
              }
            }
          }

          if !success {
            return Err(
              format!(
                "EXE installation failed after trying multiple methods. Last error: {last_error}"
              )
              .into(),
            );
          }
        }
        "zip" => {
          // Handle ZIP files by extracting and replacing the current executable
          log::info!("Handling ZIP update: {}", installer_path.display());

          let temp_extract_dir = installer_path.parent().unwrap().join("extracted");
          fs::create_dir_all(&temp_extract_dir)?;

          // Extract ZIP file
          let extracted_path = self
            .extractor
            .extract_zip(installer_path, &temp_extract_dir)
            .await?;

          // Find the executable in the extracted files
          let current_exe = self.get_current_app_path()?;
          let current_exe_name = current_exe.file_name().unwrap();

          // Look for the new executable
          let new_exe_path =
            if extracted_path.is_file() && extracted_path.file_name() == Some(current_exe_name) {
              extracted_path
            } else {
              // Search in extracted directory
              let mut found_exe = None;
              if let Ok(entries) = fs::read_dir(&extracted_path) {
                for entry in entries.flatten() {
                  let path = entry.path();
                  if path.file_name() == Some(current_exe_name) {
                    found_exe = Some(path);
                    break;
                  }
                }
              }
              found_exe.ok_or("Could not find executable in ZIP file")?
            };

          // Create backup of current executable
          let backup_path = current_exe.with_extension("exe.backup");
          if backup_path.exists() {
            fs::remove_file(&backup_path)?;
          }
          fs::copy(&current_exe, &backup_path)?;

          // Replace current executable
          fs::copy(&new_exe_path, &current_exe)?;

          // Clean up
          let _ = fs::remove_dir_all(&temp_extract_dir);

          log::info!("ZIP update completed successfully");
        }
        _ => {
          return Err(format!("Unsupported installer format: {extension}").into());
        }
      }

      Ok(())
    }

    #[cfg(target_os = "linux")]
    {
      let file_name = installer_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");

      log::info!("Installing Linux update: {}", installer_path.display());

      // Handle compound extensions like .tar.gz
      if file_name.ends_with(".tar.gz") {
        return self.install_linux_tarball(installer_path).await;
      }

      let extension = installer_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

      match extension {
        "deb" => self.install_linux_deb(installer_path).await,
        "rpm" => self.install_linux_rpm(installer_path).await,
        "appimage" => self.install_linux_appimage(installer_path).await,
        _ => Err(format!("Unsupported Linux installer format: {extension}").into()),
      }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
      Err("Auto-update installation not supported on this platform".into())
    }
  }

  /// Install Linux DEB package
  #[cfg(target_os = "linux")]
  async fn install_linux_deb(
    &self,
    deb_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Installing DEB package: {}", deb_path.display());

    // Try different package managers in order of preference
    let package_managers = [
      ("dpkg", vec!["-i", deb_path.to_str().unwrap()]),
      ("apt", vec!["install", "-y", deb_path.to_str().unwrap()]),
    ];

    let mut last_error = String::new();

    for (manager, args) in &package_managers {
      // Check if package manager exists
      if Command::new("which").arg(manager).output().is_ok() {
        log::info!("Trying to install with {manager}");

        let output = Command::new("pkexec").arg(manager).args(args).output();

        match output {
          Ok(output) if output.status.success() => {
            log::info!("DEB installation completed successfully with {manager}");
            return Ok(());
          }
          Ok(output) => {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            last_error = format!("{manager} failed: {error_msg}");
            log::info!("Installation failed with {manager}: {error_msg}");
          }
          Err(e) => {
            last_error = format!("Failed to execute {manager}: {e}");
            log::info!("Failed to execute {manager}: {e}");
          }
        }
      }
    }

    Err(format!("DEB installation failed. Last error: {last_error}").into())
  }

  /// Install Linux RPM package
  #[cfg(target_os = "linux")]
  async fn install_linux_rpm(
    &self,
    rpm_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Installing RPM package: {}", rpm_path.display());

    // Try different package managers in order of preference
    let package_managers = [
      ("rpm", vec!["-Uvh", rpm_path.to_str().unwrap()]),
      ("dnf", vec!["install", "-y", rpm_path.to_str().unwrap()]),
      ("yum", vec!["install", "-y", rpm_path.to_str().unwrap()]),
      ("zypper", vec!["install", "-y", rpm_path.to_str().unwrap()]),
    ];

    let mut last_error = String::new();

    for (manager, args) in &package_managers {
      // Check if package manager exists
      if Command::new("which").arg(manager).output().is_ok() {
        log::info!("Trying to install with {manager}");

        let output = Command::new("pkexec").arg(manager).args(args).output();

        match output {
          Ok(output) if output.status.success() => {
            log::info!("RPM installation completed successfully with {manager}");
            return Ok(());
          }
          Ok(output) => {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            last_error = format!("{manager} failed: {error_msg}");
            log::info!("Installation failed with {manager}: {error_msg}");
          }
          Err(e) => {
            last_error = format!("Failed to execute {manager}: {e}");
            log::info!("Failed to execute {manager}: {e}");
          }
        }
      }
    }

    Err(format!("RPM installation failed. Last error: {last_error}").into())
  }

  /// Install Linux AppImage
  #[cfg(target_os = "linux")]
  async fn install_linux_appimage(
    &self,
    appimage_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Installing AppImage: {}", appimage_path.display());

    // This function should not be called for AppImages since we disable auto-updates for them
    // But if it somehow gets called, we'll handle it safely

    if !self.is_running_from_appimage() {
      return Err("AppImage installation attempted but not running from AppImage".into());
    }

    let current_exe = self.get_current_app_path()?;

    // Detect if we're running from an AppImage using multiple methods
    let current_appimage = if let Ok(appimage_env) = std::env::var("APPIMAGE") {
      PathBuf::from(appimage_env)
    } else {
      // Fallback: use current executable path
      current_exe.clone()
    };

    // Create backup
    let backup_path = current_appimage.with_extension("appimage.backup");
    if backup_path.exists() {
      fs::remove_file(&backup_path)?;
    }
    fs::copy(&current_appimage, &backup_path)?;

    // Make new AppImage executable
    let _ = Command::new("chmod")
      .args(["+x", appimage_path.to_str().unwrap()])
      .output();

    // Replace the AppImage
    fs::copy(appimage_path, &current_appimage)?;

    log::info!("AppImage replacement completed successfully");
    Ok(())
  }

  /// Install Linux tarball
  #[cfg(target_os = "linux")]
  async fn install_linux_tarball(
    &self,
    tarball_path: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Installing tarball: {}", tarball_path.display());

    let current_exe = self.get_current_app_path()?;
    let temp_extract_dir = tarball_path.parent().unwrap().join("extracted");
    fs::create_dir_all(&temp_extract_dir)?;

    // Extract tarball
    let extracted_path = self
      .extractor
      .extract_tar_gz(tarball_path, &temp_extract_dir)
      .await?;

    // Find the executable in the extracted files
    let current_exe_name = current_exe.file_name().unwrap();
    let new_exe_path =
      if extracted_path.is_file() && extracted_path.file_name() == Some(current_exe_name) {
        extracted_path
      } else {
        // Search in extracted directory
        let mut found_exe = None;
        if let Ok(entries) = fs::read_dir(&extracted_path) {
          for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name() == Some(current_exe_name) {
              found_exe = Some(path);
              break;
            }
            // Also check subdirectories
            if path.is_dir() {
              if let Ok(sub_entries) = fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten() {
                  let sub_path = sub_entry.path();
                  if sub_path.file_name() == Some(current_exe_name) {
                    found_exe = Some(sub_path);
                    break;
                  }
                }
              }
            }
          }
        }
        found_exe.ok_or("Could not find executable in tarball")?
      };

    // Create backup of current executable
    let backup_path = current_exe.with_extension("backup");
    if backup_path.exists() {
      fs::remove_file(&backup_path)?;
    }
    fs::copy(&current_exe, &backup_path)?;

    // Replace current executable
    fs::copy(&new_exe_path, &current_exe)?;

    // Make sure it's executable
    let _ = Command::new("chmod")
      .args(["+x", current_exe.to_str().unwrap()])
      .output();

    // Clean up
    let _ = fs::remove_dir_all(&temp_extract_dir);

    log::info!("Tarball installation completed successfully");
    Ok(())
  }

  /// Get the current application bundle path
  fn get_current_app_path(&self) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_os = "macos")]
    {
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

    #[cfg(target_os = "windows")]
    {
      // On Windows, just return the current executable path
      std::env::current_exe().map_err(|e| e.into())
    }

    #[cfg(target_os = "linux")]
    {
      // On Linux, return the current executable path
      std::env::current_exe().map_err(|e| e.into())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
      Err("Platform not supported".into())
    }
  }

  /// Restart the application
  async fn restart_application(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(target_os = "macos")]
    {
      let app_path = self.get_current_app_path()?;
      let current_pid = std::process::id();

      // Create a temporary restart script
      let temp_dir = std::env::temp_dir();
      let script_path = temp_dir.join("donut_restart.sh");

      // Create the restart script content
      let script_content = format!(
        r#"#!/bin/bash
# Wait for the current process to exit
while kill -0 {} 2>/dev/null; do
  sleep 0.5
done

# Wait a bit more to ensure clean exit
sleep 1

# Start the new application
open "{}"

# Clean up this script
rm "{}"
"#,
        current_pid,
        app_path.to_str().unwrap(),
        script_path.to_str().unwrap()
      );

      // Write the script to file
      fs::write(&script_path, script_content)?;

      // Make the script executable
      let _ = Command::new("chmod")
        .args(["+x", script_path.to_str().unwrap()])
        .output();

      // Execute the restart script in the background
      let mut cmd = Command::new("bash");
      cmd.arg(script_path.to_str().unwrap());

      // Detach the process completely
      use std::os::unix::process::CommandExt;
      cmd.process_group(0);

      let _child = cmd.spawn()?;

      // Give the script a moment to start
      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

      // Exit the current process
      std::process::exit(0);
    }

    #[cfg(target_os = "windows")]
    {
      let app_path = self.get_current_app_path()?;
      let current_pid = std::process::id();

      // Create a temporary restart batch script
      let temp_dir = std::env::temp_dir();
      let script_path = temp_dir.join("donut_restart.bat");

      // Create the restart script content
      let script_content = format!(
        r#"@echo off
rem Wait for the current process to exit
:wait_loop
tasklist /fi "PID eq {}" >nul 2>&1
if %errorlevel% equ 0 (
    timeout /t 1 /nobreak >nul
    goto wait_loop
)

rem Wait a bit more to ensure clean exit
timeout /t 2 /nobreak >nul

rem Start the new application
start "" "{}"

rem Clean up this script
del "%~f0"
"#,
        current_pid,
        app_path.to_str().unwrap()
      );

      // Write the script to file
      fs::write(&script_path, script_content)?;

      // Execute the restart script in the background
      let mut cmd = Command::new("cmd");
      cmd.args(["/C", script_path.to_str().unwrap()]);

      // Start the process detached
      let _child = cmd.spawn()?;

      // Give the script a moment to start
      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

      // Exit the current process
      std::process::exit(0);
    }

    #[cfg(target_os = "linux")]
    {
      let app_path = self.get_current_app_path()?;
      let current_pid = std::process::id();

      // Create a temporary restart script
      let temp_dir = std::env::temp_dir();
      let script_path = temp_dir.join("donut_restart.sh");

      // Create the restart script content
      let script_content = format!(
        r#"#!/bin/bash
# Wait for the current process to exit
while kill -0 {} 2>/dev/null; do
  sleep 0.5
done

# Wait a bit more to ensure clean exit
sleep 1

# Start the new application
"{}" &

# Clean up this script
rm "{}"
"#,
        current_pid,
        app_path.to_str().unwrap(),
        script_path.to_str().unwrap()
      );

      // Write the script to file
      fs::write(&script_path, script_content)?;

      // Make the script executable
      let _ = Command::new("chmod")
        .args(["+x", script_path.to_str().unwrap()])
        .output();

      // Execute the restart script in the background
      let mut cmd = Command::new("bash");
      cmd.arg(script_path.to_str().unwrap());

      // Detach the process completely
      use std::os::unix::process::CommandExt;
      cmd.process_group(0);

      let _child = cmd.spawn()?;

      // Give the script a moment to start
      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

      // Exit the current process
      std::process::exit(0);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
      Err("Application restart not supported on this platform".into())
    }
  }
}

// Tauri commands

#[tauri::command]
pub async fn check_for_app_updates() -> Result<Option<AppUpdateInfo>, String> {
  let updater = AppAutoUpdater::instance();
  updater
    .check_for_updates()
    .await
    .map_err(|e| format!("Failed to check for app updates: {e}"))
}

#[tauri::command]
pub async fn download_and_prepare_app_update(
  app_handle: tauri::AppHandle,
  update_info: AppUpdateInfo,
) -> Result<(), String> {
  let updater = AppAutoUpdater::instance();
  updater
    .download_and_prepare_update(&app_handle, &update_info)
    .await
    .map_err(|e| format!("Failed to download and prepare app update: {e}"))
}

#[tauri::command]
pub async fn restart_application() -> Result<(), String> {
  let updater = AppAutoUpdater::instance();
  updater
    .restart_application()
    .await
    .map_err(|e| format!("Failed to restart application: {e}"))
}

#[tauri::command]
pub async fn check_for_app_updates_manual() -> Result<Option<AppUpdateInfo>, String> {
  log::info!("Manual app update check triggered");
  let updater = AppAutoUpdater::instance();
  updater
    .check_for_updates()
    .await
    .map_err(|e| format!("Failed to check for app updates: {e}"))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_is_nightly_build() {
    // This will depend on whether STABLE_RELEASE is set during test compilation
    let is_nightly = AppAutoUpdater::is_nightly_build();
    log::info!("Is nightly build: {is_nightly}");

    // The result should be true for test builds since STABLE_RELEASE is not set
    // unless the test is run in a stable release environment
    assert!(is_nightly || option_env!("STABLE_RELEASE").is_some());
  }

  #[test]
  fn test_version_comparison() {
    let updater = AppAutoUpdater::instance();

    // Test semantic version comparison
    assert!(updater.is_version_newer("v1.1.0", "v1.0.0"));
    assert!(updater.is_version_newer("v2.0.0", "v1.9.9"));
    assert!(updater.is_version_newer("v1.0.1", "v1.0.0"));
    assert!(!updater.is_version_newer("v1.0.0", "v1.0.0"));
    assert!(!updater.is_version_newer("v1.0.0", "v1.0.1"));
  }

  #[test]
  fn test_parse_semver() {
    let updater = AppAutoUpdater::instance();

    assert_eq!(updater.parse_semver("v1.2.3"), (1, 2, 3));
    assert_eq!(updater.parse_semver("1.2.3"), (1, 2, 3));
    assert_eq!(updater.parse_semver("v2.0.0"), (2, 0, 0));
    assert_eq!(updater.parse_semver("0.1.0"), (0, 1, 0));
  }

  #[test]
  fn test_should_update_stable() {
    let updater = AppAutoUpdater::instance();

    // Stable version updates
    assert!(updater.should_update("v1.0.0", "v1.1.0", false));
    assert!(updater.should_update("v1.0.0", "v2.0.0", false));
    assert!(!updater.should_update("v1.1.0", "v1.0.0", false));
    assert!(!updater.should_update("v1.0.0", "v1.0.0", false));
  }

  #[test]
  fn test_should_update_nightly() {
    let updater = AppAutoUpdater::instance();

    // Nightly version updates
    assert!(updater.should_update("nightly-abc123", "nightly-def456", true));
    assert!(!updater.should_update("nightly-abc123", "nightly-abc123", true));

    // Upgrade from stable to nightly
    assert!(updater.should_update("v1.0.0", "nightly-abc123", true));

    // Don't upgrade dev, ever
    assert!(!updater.should_update("dev-0.1.0", "nightly-xyz987", false));
    assert!(!updater.should_update("dev-0.1.0", "nightly-xyz987", true));
    assert!(!updater.should_update("dev-0.1.0", "v1.2.3", false));
  }

  #[test]
  fn test_should_update_edge_cases() {
    let updater = AppAutoUpdater::instance();

    // Test with different nightly formats
    assert!(updater.should_update("nightly-abc123", "nightly-def456", true));
    assert!(!updater.should_update("nightly-abc123", "nightly-abc123", true));

    // Test stable version edge cases
    assert!(updater.should_update("v0.9.9", "v1.0.0", false));
    assert!(!updater.should_update("v1.0.0", "v0.9.9", false));
    assert!(!updater.should_update("v1.0.0", "v1.0.0", false));

    // Test version without 'v' prefix
    assert!(updater.should_update("0.9.9", "v1.0.0", false));
    assert!(updater.should_update("v0.9.9", "1.0.0", false));
  }

  #[test]
  fn test_extract_update_uses_extractor() {
    // This test verifies that the extract_update method properly uses the Extractor
    // We can't run the actual extraction in unit tests without real DMG files,
    // but we can verify the method signature and basic logic
    let updater = AppAutoUpdater::instance();

    // Test that unsupported formats would be rejected
    let temp_dir = std::env::temp_dir();
    let unsupported_file = temp_dir.join("test.rar");

    // Create a mock runtime to test the logic
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    // This would fail because .rar is not supported, which proves
    // our method is using the Extractor logic
    let result = rt.block_on(async { updater.extract_update(&unsupported_file, &temp_dir).await });

    // Should fail with unsupported format error
    assert!(result.is_err(), "Unsupported format should return error");
    let error_msg = result.expect_err("Should have error").to_string();
    assert!(
      error_msg.contains("Unsupported archive format: rar")
        || error_msg.contains("unknown")
        || error_msg.contains("unsupported"),
      "Error should mention unsupported format, got: {error_msg}"
    );
  }

  #[test]
  fn test_platform_specific_download_urls() {
    let updater = AppAutoUpdater::instance();

    // Create comprehensive assets for all platforms
    let all_assets = vec![
      // macOS assets
      AppReleaseAsset {
        name: "Donut.Browser_0.1.0_aarch64.dmg".to_string(),
        browser_download_url: "https://example.com/aarch64.dmg".to_string(),
        size: 12345,
      },
      AppReleaseAsset {
        name: "Donut.Browser_0.1.0_x64.dmg".to_string(),
        browser_download_url: "https://example.com/x64.dmg".to_string(),
        size: 12345,
      },
      // Windows assets
      AppReleaseAsset {
        name: "Donut.Browser_0.1.0_x64.msi".to_string(),
        browser_download_url: "https://example.com/x64.msi".to_string(),
        size: 12345,
      },
      AppReleaseAsset {
        name: "Donut.Browser_0.1.0_x64.exe".to_string(),
        browser_download_url: "https://example.com/x64.exe".to_string(),
        size: 12345,
      },
      // Linux assets
      AppReleaseAsset {
        name: "donutbrowser_0.1.0_amd64.deb".to_string(),
        browser_download_url: "https://example.com/amd64.deb".to_string(),
        size: 12345,
      },
      AppReleaseAsset {
        name: "donutbrowser-0.1.0-1.x86_64.rpm".to_string(),
        browser_download_url: "https://example.com/x86_64.rpm".to_string(),
        size: 12345,
      },
      AppReleaseAsset {
        name: "Donut.Browser-0.1.0-x86_64.AppImage".to_string(),
        browser_download_url: "https://example.com/x86_64.AppImage".to_string(),
        size: 12345,
      },
    ];

    // Test that the method returns a URL for the current platform
    let url = updater.get_download_url_for_platform(&all_assets);
    assert!(
      url.is_some(),
      "Should find a suitable download URL for current platform"
    );

    // Test platform-specific behavior
    #[cfg(target_os = "macos")]
    {
      let url = url.unwrap();
      assert!(url.contains(".dmg"), "macOS should prefer DMG files");
    }

    #[cfg(target_os = "windows")]
    {
      let url = url.unwrap();
      assert!(
        url.contains(".msi") || url.contains(".exe") || url.contains(".zip"),
        "Windows should prefer MSI, EXE, or ZIP files"
      );
    }

    #[cfg(target_os = "linux")]
    {
      let url = url.unwrap();
      assert!(
        url.contains(".deb")
          || url.contains(".rpm")
          || url.contains(".appimage")
          || url.contains(".tar.gz"),
        "Linux should prefer DEB, RPM, AppImage, or TAR.GZ files"
      );
    }
  }

  #[test]
  fn test_supported_file_extensions() {
    let updater = AppAutoUpdater::instance();
    let temp_dir = std::env::temp_dir();
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Test that all supported extensions are handled
    let supported_extensions = ["dmg", "msi", "exe", "deb", "rpm", "appimage", "zip"];

    for ext in &supported_extensions {
      let test_file = temp_dir.join(format!("test.{ext}"));
      let result = rt.block_on(async { updater.extract_update(&test_file, &temp_dir).await });

      // The result should either succeed or fail with a platform-specific error,
      // but not with "Unsupported archive format"
      if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
          !error_msg.contains("Unsupported archive format"),
          "Extension {ext} should be supported but got: {error_msg}"
        );
      }
    }

    // Test tar.gz compound extension
    let tar_gz_file = temp_dir.join("test.tar.gz");
    let result = rt.block_on(async { updater.extract_update(&tar_gz_file, &temp_dir).await });

    if let Err(e) = result {
      let error_msg = e.to_string();
      assert!(
        !error_msg.contains("Unsupported archive format"),
        "tar.gz should be supported but got: {error_msg}"
      );
    }
  }

  #[cfg(target_os = "linux")]
  #[test]
  fn test_appimage_detection() {
    let updater = AppAutoUpdater::instance();

    // Test that AppImage detection works with various scenarios
    // Note: These tests can't fully simulate AppImage environment without actual AppImage

    // Test that the method exists and doesn't panic
    let _is_appimage = updater.is_running_from_appimage();

    // Test installation method detection
    let _method = updater.detect_linux_installation_method();
  }

  #[cfg(target_os = "linux")]
  #[test]
  fn test_appimage_auto_update_disabled() {
    let updater = AppAutoUpdater::instance();

    // Create mock assets including AppImage
    let assets = vec![
      AppReleaseAsset {
        name: "donutbrowser_0.1.0_amd64.deb".to_string(),
        browser_download_url: "https://example.com/amd64.deb".to_string(),
        size: 12345,
      },
      AppReleaseAsset {
        name: "Donut.Browser-0.1.0-x86_64.AppImage".to_string(),
        browser_download_url: "https://example.com/x86_64.AppImage".to_string(),
        size: 12345,
      },
    ];

    // If we're running from AppImage, should return None (disabled)
    // If not, should return a suitable download URL
    let url = updater.get_download_url_for_platform(&assets);

    // The test should pass regardless of whether we're in AppImage or not
    // If in AppImage: url should be None
    // If not in AppImage: url should be Some(...)
    if updater.is_running_from_appimage() {
      assert!(
        url.is_none(),
        "Auto-updates should be disabled for AppImages"
      );
    } else {
      // Should find a suitable non-AppImage download
      if let Some(url_str) = url {
        assert!(
          !url_str.contains("AppImage"),
          "Should not select AppImage when not running from AppImage"
        );
      }
    }
  }

  #[test]
  fn test_appimage_detection_logic() {
    let updater = AppAutoUpdater::instance();

    // Test that the get_download_url_for_platform method properly handles AppImage detection
    // This test can run on all platforms

    // Create comprehensive assets for all platforms including AppImage
    let all_assets = vec![
      // macOS assets
      AppReleaseAsset {
        name: "Donut.Browser_0.1.0_aarch64.dmg".to_string(),
        browser_download_url: "https://example.com/aarch64.dmg".to_string(),
        size: 12345,
      },
      // Windows assets
      AppReleaseAsset {
        name: "Donut.Browser_0.1.0_x64.msi".to_string(),
        browser_download_url: "https://example.com/x64.msi".to_string(),
        size: 12345,
      },
      // Linux assets
      AppReleaseAsset {
        name: "donutbrowser_0.1.0_amd64.deb".to_string(),
        browser_download_url: "https://example.com/amd64.deb".to_string(),
        size: 12345,
      },
      AppReleaseAsset {
        name: "Donut.Browser-0.1.0-x86_64.AppImage".to_string(),
        browser_download_url: "https://example.com/x86_64.AppImage".to_string(),
        size: 12345,
      },
    ];

    // Test that the method returns a URL for the current platform
    let url = updater.get_download_url_for_platform(&all_assets);

    // On non-Linux platforms, should always return a URL
    #[cfg(not(target_os = "linux"))]
    {
      assert!(
        url.is_some(),
        "Should find a suitable download URL for non-Linux platforms"
      );
      let url_str = url.unwrap();
      assert!(
        !url_str.contains("AppImage"),
        "Non-Linux platforms should not get AppImage downloads"
      );
    }

    // On Linux platforms, behavior depends on AppImage detection
    #[cfg(target_os = "linux")]
    {
      // The URL might be None if AppImage is detected, or Some if not
      // This is expected behavior based on our implementation
      if let Some(url_str) = url {
        // If we get a URL, it should not be an AppImage
        assert!(
          !url_str.contains("AppImage"),
          "Should not select AppImage format"
        );
      }
      // If url is None, it means AppImage was detected and auto-updates are disabled
    }
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref APP_AUTO_UPDATER: AppAutoUpdater = AppAutoUpdater::new();
}

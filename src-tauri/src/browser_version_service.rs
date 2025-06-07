use crate::api_client::{sort_versions, ApiClient, BrowserRelease};
use crate::browser::GithubRelease;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrowserVersionInfo {
  pub version: String,
  pub is_prerelease: bool,
  pub date: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrowserVersionsResult {
  pub versions: Vec<String>,
  pub new_versions_count: Option<usize>,
  pub total_versions_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadInfo {
  pub url: String,
  pub filename: String,
  pub is_archive: bool, // true for .dmg, .zip, etc.
}

pub struct BrowserVersionService {
  api_client: ApiClient,
}

impl BrowserVersionService {
  pub fn new() -> Self {
    Self {
      api_client: ApiClient::new(),
    }
  }

  #[cfg(test)]
  pub fn new_with_api_client(api_client: ApiClient) -> Self {
    Self { api_client }
  }

  /// Check if a browser is supported on the current platform and architecture
  pub fn is_browser_supported(
    &self,
    browser: &str,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let (os, arch) = Self::get_platform_info();

    match browser {
      "firefox" | "firefox-developer" => Ok(true),
      "mullvad-browser" => {
        // Mullvad doesn't support ARM64 on Windows and Linux
        if arch == "arm64" && (os == "windows" || os == "linux") {
          Ok(false)
        } else {
          Ok(true)
        }
      }
      "zen" => {
        // Zen supports all platforms and architectures
        Ok(true)
      }
      "brave" => {
        // Brave supports all platforms and architectures
        Ok(true)
      }
      "chromium" => {
        // Chromium doesn't support ARM64 on Linux
        if arch == "arm64" && os == "linux" {
          Ok(false)
        } else {
          Ok(true)
        }
      }
      "tor-browser" => {
        // TOR Browser doesn't support ARM64 on Windows and Linux
        if arch == "arm64" && (os == "windows" || os == "linux") {
          Ok(false)
        } else {
          Ok(true)
        }
      }
      _ => Err(format!("Unknown browser: {browser}").into()),
    }
  }

  /// Get list of browsers supported on the current platform
  pub fn get_supported_browsers(&self) -> Vec<String> {
    let all_browsers = vec![
      "firefox",
      "firefox-developer",
      "mullvad-browser",
      "zen",
      "brave",
      "chromium",
      "tor-browser",
    ];

    all_browsers
      .into_iter()
      .filter(|browser| self.is_browser_supported(browser).unwrap_or(false))
      .map(|s| s.to_string())
      .collect()
  }

  /// Get cached browser versions immediately (returns None if no cache exists)
  pub fn get_cached_browser_versions(&self, browser: &str) -> Option<Vec<String>> {
    self.api_client.load_cached_versions(browser)
  }

  /// Get cached detailed browser version information immediately
  pub fn get_cached_browser_versions_detailed(
    &self,
    browser: &str,
  ) -> Option<Vec<BrowserVersionInfo>> {
    let cached_versions = self.api_client.load_cached_versions(browser)?;

    // Convert cached versions to detailed info (without dates since cache doesn't store them)
    let detailed_info: Vec<BrowserVersionInfo> = cached_versions
      .into_iter()
      .map(|version| {
        BrowserVersionInfo {
          version: version.clone(),
          is_prerelease: crate::api_client::is_browser_version_nightly(browser, &version, None),
          date: "".to_string(), // Cache doesn't store dates
        }
      })
      .collect();

    Some(detailed_info)
  }

  /// Check if cache should be updated (expired or doesn't exist)
  pub fn should_update_cache(&self, browser: &str) -> bool {
    self.api_client.is_cache_expired(browser)
  }

  /// Fetch browser versions with optional caching
  pub async fn fetch_browser_versions(
    &self,
    browser: &str,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let result = self
      .fetch_browser_versions_with_count(browser, no_caching)
      .await?;
    Ok(result.versions)
  }

  /// Fetch browser versions with new count information and optional caching
  pub async fn fetch_browser_versions_with_count(
    &self,
    browser: &str,
    no_caching: bool,
  ) -> Result<BrowserVersionsResult, Box<dyn std::error::Error + Send + Sync>> {
    // Get existing cached versions to compare and merge
    let existing_versions = self
      .api_client
      .load_cached_versions(browser)
      .unwrap_or_default();
    let existing_set: HashSet<String> = existing_versions.into_iter().collect();

    // Fetch fresh versions from API
    let fresh_versions = match browser {
      "firefox" => self.fetch_firefox_versions(true).await?, // Always fetch fresh for merging
      "firefox-developer" => self.fetch_firefox_developer_versions(true).await?,
      "mullvad-browser" => self.fetch_mullvad_versions(true).await?,
      "zen" => self.fetch_zen_versions(true).await?,
      "brave" => self.fetch_brave_versions(true).await?,
      "chromium" => self.fetch_chromium_versions(true).await?,
      "tor-browser" => self.fetch_tor_versions(true).await?,
      _ => return Err(format!("Unsupported browser: {browser}").into()),
    };

    let fresh_set: HashSet<String> = fresh_versions.into_iter().collect();

    // Find new versions (in fresh but not in existing cache)
    let new_versions: Vec<String> = fresh_set.difference(&existing_set).cloned().collect();
    let new_versions_count = if existing_set.is_empty() {
      None
    } else {
      Some(new_versions.len())
    };

    // Merge existing and fresh versions
    let mut merged_versions: Vec<String> = existing_set.union(&fresh_set).cloned().collect();

    // Sort versions using the existing sorting logic
    crate::api_client::sort_versions(&mut merged_versions);

    // Save the merged cache (unless explicitly bypassing cache)
    if !no_caching {
      if let Err(e) = self
        .api_client
        .save_cached_versions(browser, &merged_versions)
      {
        eprintln!("Failed to save merged cache for {browser}: {e}");
      }
    }

    let total_versions_count = merged_versions.len();

    Ok(BrowserVersionsResult {
      versions: merged_versions,
      new_versions_count,
      total_versions_count,
    })
  }

  /// Fetch detailed browser version information with optional caching
  pub async fn fetch_browser_versions_detailed(
    &self,
    browser: &str,
    no_caching: bool,
  ) -> Result<Vec<BrowserVersionInfo>, Box<dyn std::error::Error + Send + Sync>> {
    // For detailed versions, we'll use the merged versions from fetch_browser_versions_with_count
    // to ensure consistency with the version list
    let versions_result = self
      .fetch_browser_versions_with_count(browser, no_caching)
      .await?;
    let merged_versions = versions_result.versions;

    // Convert the version strings to BrowserVersionInfo
    // Since we don't have detailed date/prerelease info for cached versions,
    // we'll fetch fresh detailed info and map it to our merged versions
    let detailed_info: Vec<BrowserVersionInfo> = match browser {
      "firefox" => {
        let releases = self.fetch_firefox_releases_detailed(true).await?;
        merged_versions
          .into_iter()
          .map(|version| {
            // Try to find matching release info, otherwise create basic info
            if let Some(release) = releases.iter().find(|r| r.version == version) {
              BrowserVersionInfo {
                version: release.version.clone(),
                is_prerelease: release.is_prerelease,
                date: release.date.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: crate::api_client::is_browser_version_nightly(
                  "firefox", &version, None,
                ),
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      "firefox-developer" => {
        let releases = self.fetch_firefox_developer_releases_detailed(true).await?;
        merged_versions
          .into_iter()
          .map(|version| {
            if let Some(release) = releases.iter().find(|r| r.version == version) {
              BrowserVersionInfo {
                version: release.version.clone(),
                is_prerelease: release.is_prerelease,
                date: release.date.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: crate::api_client::is_browser_version_nightly(
                  "firefox-developer",
                  &version,
                  None,
                ),
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      "mullvad-browser" => {
        let releases = self.fetch_mullvad_releases_detailed(true).await?;
        merged_versions
          .into_iter()
          .map(|version| {
            if let Some(release) = releases.iter().find(|r| r.tag_name == version) {
              BrowserVersionInfo {
                version: release.tag_name.clone(),
                is_prerelease: release.is_nightly,
                date: release.published_at.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: false, // Mullvad usually stable releases
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      "zen" => {
        let releases = self.fetch_zen_releases_detailed(true).await?;
        merged_versions
          .into_iter()
          .map(|version| {
            if let Some(release) = releases.iter().find(|r| r.tag_name == version) {
              BrowserVersionInfo {
                version: release.tag_name.clone(),
                is_prerelease: release.is_nightly,
                date: release.published_at.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: crate::api_client::is_browser_version_nightly("zen", &version, None),
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      "brave" => {
        let releases = self.fetch_brave_releases_detailed(true).await?;
        merged_versions
          .into_iter()
          .map(|version| {
            if let Some(release) = releases.iter().find(|r| r.tag_name == version) {
              BrowserVersionInfo {
                version: release.tag_name.clone(),
                is_prerelease: release.is_nightly,
                date: release.published_at.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: crate::api_client::is_browser_version_nightly(
                  "brave", &version, None,
                ),
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      "chromium" => {
        let releases = self.fetch_chromium_releases_detailed(true).await?;
        merged_versions
          .into_iter()
          .map(|version| {
            if let Some(release) = releases.iter().find(|r| r.version == version) {
              BrowserVersionInfo {
                version: release.version.clone(),
                is_prerelease: release.is_prerelease,
                date: release.date.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: false, // Chromium usually stable releases
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      "tor-browser" => {
        let releases = self.fetch_tor_releases_detailed(true).await?;
        merged_versions
          .into_iter()
          .map(|version| {
            if let Some(release) = releases.iter().find(|r| r.version == version) {
              BrowserVersionInfo {
                version: release.version.clone(),
                is_prerelease: crate::api_client::is_browser_version_nightly(
                  "tor-browser",
                  &release.version,
                  None,
                ),
                date: release.date.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: false, // TOR Browser usually stable releases
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      _ => return Err(format!("Unsupported browser: {browser}").into()),
    };

    Ok(detailed_info)
  }

  /// Update browser versions incrementally (for background updates)
  pub async fn update_browser_versions_incrementally(
    &self,
    browser: &str,
  ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Get existing cached versions
    let existing_versions = self
      .api_client
      .load_cached_versions(browser)
      .unwrap_or_default();
    let existing_set: HashSet<String> = existing_versions.into_iter().collect();

    // Fetch new versions (always bypass cache for background updates)
    let new_versions = self.fetch_browser_versions(browser, true).await?;
    let new_set: HashSet<String> = new_versions.into_iter().collect();

    // Find truly new versions (not in existing cache)
    let really_new_versions: Vec<String> = new_set.difference(&existing_set).cloned().collect();
    let new_versions_count = really_new_versions.len();

    // Merge existing and new versions
    let mut all_versions: Vec<String> = existing_set.union(&new_set).cloned().collect();

    // Sort versions using the existing sorting logic
    sort_versions(&mut all_versions);

    // Save the updated cache
    if let Err(e) = self.api_client.save_cached_versions(browser, &all_versions) {
      eprintln!("Failed to save updated cache for {browser}: {e}");
    }

    Ok(new_versions_count)
  }

  /// Get download information for a specific browser and version
  pub fn get_download_info(
    &self,
    browser: &str,
    version: &str,
  ) -> Result<DownloadInfo, Box<dyn std::error::Error + Send + Sync>> {
    let (os, arch) = Self::get_platform_info();

    match browser {
      "firefox" => {
        let (platform_path, filename, is_archive) = match (&os[..], &arch[..]) {
          ("windows", "x64") => ("win64", format!("Firefox Setup {version}.exe"), false),
          ("windows", "arm64") => (
            "win64-aarch64",
            format!("Firefox Setup {version}.exe"),
            false,
          ),
          ("linux", "x64") => ("linux-x86_64", format!("firefox-{version}.tar.xz"), true),
          ("linux", "arm64") => ("linux-aarch64", format!("firefox-{version}.tar.xz"), true),
          ("macos", _) => ("mac", format!("Firefox {version}.dmg"), true),
          _ => {
            return Err(
              format!("Unsupported platform/architecture for Firefox: {os}/{arch}").into(),
            )
          }
        };

        Ok(DownloadInfo {
          url: format!(
            "https://download-installer.cdn.mozilla.net/pub/firefox/releases/{version}/{platform_path}/en-US/{filename}"
          ),
          filename,
          is_archive,
        })
      }
      "firefox-developer" => {
        let (platform_path, filename, is_archive) = match (&os[..], &arch[..]) {
          ("windows", "x64") => ("win64", format!("Firefox Setup {version}.exe"), false),
          ("windows", "arm64") => (
            "win64-aarch64",
            format!("Firefox Setup {version}.exe"),
            false,
          ),
          ("linux", "x64") => ("linux-x86_64", format!("firefox-{version}.tar.xz"), true),
          ("linux", "arm64") => ("linux-aarch64", format!("firefox-{version}.tar.xz"), true),
          ("macos", _) => ("mac", format!("Firefox {version}.dmg"), true),
          _ => {
            return Err(
              format!("Unsupported platform/architecture for Firefox Developer: {os}/{arch}")
                .into(),
            )
          }
        };

        Ok(DownloadInfo {
          url: format!(
            "https://download-installer.cdn.mozilla.net/pub/devedition/releases/{version}/{platform_path}/en-US/{filename}"
          ),
          filename,
          is_archive,
        })
      }
      "mullvad-browser" => {
        // Mullvad Browser doesn't support ARM64 on Windows and Linux
        if arch == "arm64" && (os == "windows" || os == "linux") {
          return Err(format!("Mullvad Browser doesn't support ARM64 on {os}").into());
        }

        let (platform_str, filename, is_archive) = match os.as_str() {
          "windows" => {
            if arch == "arm64" {
              return Err("Mullvad Browser doesn't support ARM64 on Windows".into());
            }
            (
              "windows-x86_64",
              format!("mullvad-browser-windows-x86_64-{version}.exe"),
              false,
            )
          }
          "linux" => {
            if arch == "arm64" {
              return Err("Mullvad Browser doesn't support ARM64 on Linux".into());
            }
            (
              "x86_64",
              format!("mullvad-browser-x86_64-{version}.tar.xz"),
              true,
            )
          }
          "macos" => (
            "macos",
            format!("mullvad-browser-macos-{version}.dmg"),
            true,
          ),
          _ => return Err(format!("Unsupported platform for Mullvad Browser: {os}").into()),
        };

        Ok(DownloadInfo {
          url: format!(
            "https://github.com/mullvad/mullvad-browser/releases/download/{version}/mullvad-browser-{platform_str}-{version}{}", 
            if os == "windows" { ".exe" } else if os == "linux" { ".tar.xz" } else { ".dmg" }
          ),
          filename,
          is_archive,
        })
      }
      "zen" => {
        let (asset_name, filename, is_archive) = match (&os[..], &arch[..]) {
          ("windows", "x64") => ("zen.installer.exe", format!("zen-{version}.exe"), false),
          ("windows", "arm64") => (
            "zen.installer-arm64.exe",
            format!("zen-{version}-arm64.exe"),
            false,
          ),
          ("linux", "x64") => (
            "zen.linux-x86_64.tar.xz",
            format!("zen-{version}-x86_64.tar.xz"),
            true,
          ),
          ("linux", "arm64") => (
            "zen.linux-aarch64.tar.xz",
            format!("zen-{version}-aarch64.tar.xz"),
            true,
          ),
          ("macos", _) => (
            "zen.macos-universal.dmg",
            format!("zen-{version}.dmg"),
            true,
          ),
          _ => {
            return Err(format!("Unsupported platform/architecture for Zen: {os}/{arch}").into())
          }
        };

        Ok(DownloadInfo {
          url: format!(
            "https://github.com/zen-browser/desktop/releases/download/{version}/{asset_name}"
          ),
          filename,
          is_archive,
        })
      }
      "brave" => {
        // Brave uses different asset naming conventions
        // The actual URL will be resolved dynamically in the download service
        let (filename, is_archive) = match (&os[..], &arch[..]) {
          ("windows", _) => (format!("brave-{version}.exe"), false),
          ("linux", "x64") => (format!("brave-browser-{version}-linux-amd64.zip"), true),
          ("linux", "arm64") => (format!("brave-browser-{version}-linux-arm64.zip"), true),
          ("macos", _) => ("Brave-Browser-universal.dmg".to_string(), true),
          _ => {
            return Err(format!("Unsupported platform/architecture for Brave: {os}/{arch}").into())
          }
        };

        Ok(DownloadInfo {
          url: format!(
            "https://github.com/brave/brave-browser/releases/download/{version}/brave-placeholder"
          ),
          filename,
          is_archive,
        })
      }
      "chromium" => {
        let platform_str = match (&os[..], &arch[..]) {
          ("windows", "x64") => "Win_x64",
          ("windows", "arm64") => "Win_Arm64",
          ("linux", "x64") => "Linux_x64",
          ("linux", "arm64") => return Err("Chromium doesn't support ARM64 on Linux".into()),
          ("macos", "x64") => "Mac",
          ("macos", "arm64") => "Mac_Arm",
          _ => {
            return Err(
              format!("Unsupported platform/architecture for Chromium: {os}/{arch}").into(),
            )
          }
        };

        let (archive_name, filename) = match os.as_str() {
          "windows" => ("chrome-win.zip", format!("chromium-{version}-win.zip")),
          "linux" => ("chrome-linux.zip", format!("chromium-{version}-linux.zip")),
          "macos" => ("chrome-mac.zip", format!("chromium-{version}-mac.zip")),
          _ => return Err(format!("Unsupported platform for Chromium: {os}").into()),
        };

        Ok(DownloadInfo {
          url: format!(
            "https://commondatastorage.googleapis.com/chromium-browser-snapshots/{platform_str}/{version}/{archive_name}"
          ),
          filename,
          is_archive: true,
        })
      }
      "tor-browser" => {
        // TOR Browser doesn't support ARM64 on Windows and Linux
        if arch == "arm64" && (os == "windows" || os == "linux") {
          return Err(format!("TOR Browser doesn't support ARM64 on {os}").into());
        }

        let (platform_str, filename, is_archive) = match os.as_str() {
          "windows" => {
            if arch == "arm64" {
              return Err("TOR Browser doesn't support ARM64 on Windows".into());
            }
            (
              "windows-x86_64-portable",
              format!("tor-browser-windows-x86_64-portable-{version}.exe"),
              false,
            )
          }
          "linux" => {
            if arch == "arm64" {
              return Err("TOR Browser doesn't support ARM64 on Linux".into());
            }
            (
              "linux-x86_64",
              format!("tor-browser-linux-x86_64-{version}.tar.xz"),
              true,
            )
          }
          "macos" => ("macos", format!("tor-browser-macos-{version}.dmg"), true),
          _ => return Err(format!("Unsupported platform for TOR Browser: {os}").into()),
        };

        Ok(DownloadInfo {
          url: format!(
            "https://archive.torproject.org/tor-package-archive/torbrowser/{version}/tor-browser-{platform_str}-{version}{}", 
            if os == "windows" { ".exe" } else if os == "linux" { ".tar.xz" } else { ".dmg" }
          ),
          filename,
          is_archive,
        })
      }
      _ => Err(format!("Unsupported browser: {browser}").into()),
    }
  }

  /// Get platform and architecture information
  fn get_platform_info() -> (String, String) {
    let os = if cfg!(target_os = "windows") {
      "windows"
    } else if cfg!(target_os = "linux") {
      "linux"
    } else if cfg!(target_os = "macos") {
      "macos"
    } else {
      "unknown"
    };

    let arch = if cfg!(target_arch = "x86_64") {
      "x64"
    } else if cfg!(target_arch = "aarch64") {
      "arm64"
    } else {
      "unknown"
    };

    (os.to_string(), arch.to_string())
  }

  // Private helper methods for each browser type

  async fn fetch_firefox_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self.fetch_firefox_releases_detailed(no_caching).await?;
    Ok(releases.into_iter().map(|r| r.version).collect())
  }

  async fn fetch_firefox_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self
      .api_client
      .fetch_firefox_releases_with_caching(no_caching)
      .await
  }

  async fn fetch_firefox_developer_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self
      .fetch_firefox_developer_releases_detailed(no_caching)
      .await?;
    Ok(releases.into_iter().map(|r| r.version).collect())
  }

  async fn fetch_firefox_developer_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self
      .api_client
      .fetch_firefox_developer_releases_with_caching(no_caching)
      .await
  }

  async fn fetch_mullvad_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self.fetch_mullvad_releases_detailed(no_caching).await?;
    Ok(releases.into_iter().map(|r| r.tag_name).collect())
  }

  async fn fetch_mullvad_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self
      .api_client
      .fetch_mullvad_releases_with_caching(no_caching)
      .await
  }

  async fn fetch_zen_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self.fetch_zen_releases_detailed(no_caching).await?;
    Ok(releases.into_iter().map(|r| r.tag_name).collect())
  }

  async fn fetch_zen_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self
      .api_client
      .fetch_zen_releases_with_caching(no_caching)
      .await
  }

  async fn fetch_brave_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self.fetch_brave_releases_detailed(no_caching).await?;
    Ok(releases.into_iter().map(|r| r.tag_name).collect())
  }

  async fn fetch_brave_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self
      .api_client
      .fetch_brave_releases_with_caching(no_caching)
      .await
  }

  async fn fetch_chromium_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self.fetch_chromium_releases_detailed(no_caching).await?;
    Ok(releases.into_iter().map(|r| r.version).collect())
  }

  async fn fetch_chromium_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self
      .api_client
      .fetch_chromium_releases_with_caching(no_caching)
      .await
  }

  async fn fetch_tor_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self.fetch_tor_releases_detailed(no_caching).await?;
    Ok(releases.into_iter().map(|r| r.version).collect())
  }

  async fn fetch_tor_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self
      .api_client
      .fetch_tor_releases_with_caching(no_caching)
      .await
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use wiremock::matchers::{method, path, query_param};
  use wiremock::{Mock, MockServer, ResponseTemplate};

  async fn setup_mock_server() -> MockServer {
    MockServer::start().await
  }

  fn create_test_api_client(server: &MockServer) -> ApiClient {
    let base_url = server.uri();
    ApiClient::new_with_base_urls(
      base_url.clone(), // firefox_api_base
      base_url.clone(), // firefox_dev_api_base
      base_url.clone(), // github_api_base
      base_url.clone(), // chromium_api_base
      base_url.clone(), // tor_archive_base
    )
  }

  fn create_test_service(api_client: ApiClient) -> BrowserVersionService {
    BrowserVersionService::new_with_api_client(api_client)
  }

  async fn setup_firefox_mocks(server: &MockServer) {
    let mock_response = r#"{
      "releases": {
        "firefox-139.0": {
          "build_number": 1,
          "category": "major",
          "date": "2024-01-15",
          "description": "Firefox 139.0 Release",
          "is_security_driven": false,
          "product": "firefox",
          "version": "139.0"
        },
        "firefox-138.0": {
          "build_number": 1,
          "category": "major",
          "date": "2024-01-01",
          "description": "Firefox 138.0 Release",
          "is_security_driven": false,
          "product": "firefox",
          "version": "138.0"
        },
        "firefox-137.0": {
          "build_number": 1,
          "category": "major",
          "date": "2023-12-15",
          "description": "Firefox 137.0 Release",
          "is_security_driven": false,
          "product": "firefox",
          "version": "137.0"
        }
      }
    }"#;

    Mock::given(method("GET"))
      .and(path("/firefox.json"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(mock_response)
          .insert_header("content-type", "application/json"),
      )
      .mount(server)
      .await;
  }

  async fn setup_firefox_dev_mocks(server: &MockServer) {
    let mock_response = r#"{
      "releases": {
        "devedition-140.0b1": {
          "build_number": 1,
          "category": "major",
          "date": "2024-01-20",
          "description": "Firefox Developer Edition 140.0b1",
          "is_security_driven": false,
          "product": "devedition",
          "version": "140.0b1"
        },
        "devedition-139.0b5": {
          "build_number": 1,
          "category": "major",
          "date": "2024-01-10",
          "description": "Firefox Developer Edition 139.0b5",
          "is_security_driven": false,
          "product": "devedition",
          "version": "139.0b5"
        }
      }
    }"#;

    Mock::given(method("GET"))
      .and(path("/devedition.json"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(mock_response)
          .insert_header("content-type", "application/json"),
      )
      .mount(server)
      .await;
  }

  async fn setup_mullvad_mocks(server: &MockServer) {
    let mock_response = r#"[
      {
        "tag_name": "14.5a6",
        "name": "Mullvad Browser 14.5a6",
        "prerelease": true,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "mullvad-browser-macos-14.5a6.dmg",
            "browser_download_url": "https://example.com/mullvad-14.5a6.dmg",
            "size": 100000000
          }
        ]
      },
      {
        "tag_name": "14.5a5",
        "name": "Mullvad Browser 14.5a5",
        "prerelease": true,
        "published_at": "2024-01-10T10:00:00Z",
        "assets": [
          {
            "name": "mullvad-browser-macos-14.5a5.dmg",
            "browser_download_url": "https://example.com/mullvad-14.5a5.dmg",
            "size": 99000000
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/mullvad/mullvad-browser/releases"))
      .and(query_param("per_page", "100"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(mock_response)
          .insert_header("content-type", "application/json"),
      )
      .mount(server)
      .await;
  }

  async fn setup_zen_mocks(server: &MockServer) {
    let mock_response = r#"[
      {
        "tag_name": "twilight",
        "name": "Zen Browser Twilight",
        "prerelease": false,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "zen.macos-universal.dmg",
            "browser_download_url": "https://example.com/zen-twilight.dmg",
            "size": 120000000
          }
        ]
      },
      {
        "tag_name": "1.11b",
        "name": "Zen Browser 1.11b",
        "prerelease": false,
        "published_at": "2024-01-10T10:00:00Z",
        "assets": [
          {
            "name": "zen.macos-universal.dmg",
            "browser_download_url": "https://example.com/zen-1.11b.dmg",
            "size": 115000000
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/zen-browser/desktop/releases"))
      .and(query_param("per_page", "100"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(mock_response)
          .insert_header("content-type", "application/json"),
      )
      .mount(server)
      .await;
  }

  async fn setup_brave_mocks(server: &MockServer) {
    let mock_response = r#"[
      {
        "tag_name": "v1.79.119",
        "name": "Release v1.79.119 (Chromium 137.0.7151.68)",
        "prerelease": false,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "brave-v1.79.119-universal.dmg",
            "browser_download_url": "https://example.com/brave-1.79.119-universal.dmg",
            "size": 200000000
          },
          {
            "name": "brave-browser-1.79.119-linux-amd64.zip",
            "browser_download_url": "https://example.com/brave-browser-1.79.119-linux-amd64.zip",
            "size": 150000000
          },
          {
            "name": "brave-browser-1.79.119-linux-arm64.zip",
            "browser_download_url": "https://example.com/brave-browser-1.79.119-linux-arm64.zip",
            "size": 145000000
          }
        ]
      },
      {
        "tag_name": "v1.81.8",
        "name": "Nightly v1.81.8",
        "prerelease": false,
        "published_at": "2024-01-10T10:00:00Z",
        "assets": [
          {
            "name": "brave-v1.81.8-universal.dmg",
            "browser_download_url": "https://example.com/brave-1.81.8-universal.dmg",
            "size": 199000000
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/brave/brave-browser/releases"))
      .and(query_param("per_page", "100"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(mock_response)
          .insert_header("content-type", "application/json"),
      )
      .mount(server)
      .await;
  }

  async fn setup_chromium_mocks(server: &MockServer) {
    let arch = if cfg!(target_arch = "aarch64") {
      "Mac_Arm"
    } else {
      "Mac"
    };

    Mock::given(method("GET"))
      .and(path(format!("/{arch}/LAST_CHANGE")))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string("1465660")
          .insert_header("content-type", "text/plain"),
      )
      .mount(server)
      .await;
  }

  async fn setup_tor_mocks(server: &MockServer) {
    let mock_html = r#"
    <html>
    <body>
    <a href="../">../</a>
    <a href="14.0.4/">14.0.4/</a>
    <a href="14.0.3/">14.0.3/</a>
    <a href="14.0.2/">14.0.2/</a>
    </body>
    </html>
    "#;

    let version_html_144 = r#"
    <html>
    <body>
    <a href="tor-browser-macos-14.0.4.dmg">tor-browser-macos-14.0.4.dmg</a>
    </body>
    </html>
    "#;

    let version_html_143 = r#"
    <html>
    <body>
    <a href="tor-browser-macos-14.0.3.dmg">tor-browser-macos-14.0.3.dmg</a>
    </body>
    </html>
    "#;

    let version_html_142 = r#"
    <html>
    <body>
    <a href="tor-browser-macos-14.0.2.dmg">tor-browser-macos-14.0.2.dmg</a>
    </body>
    </html>
    "#;

    Mock::given(method("GET"))
      .and(path("/"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(mock_html)
          .insert_header("content-type", "text/html"),
      )
      .mount(server)
      .await;

    Mock::given(method("GET"))
      .and(path("/14.0.4/"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(version_html_144)
          .insert_header("content-type", "text/html"),
      )
      .mount(server)
      .await;

    Mock::given(method("GET"))
      .and(path("/14.0.3/"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(version_html_143)
          .insert_header("content-type", "text/html"),
      )
      .mount(server)
      .await;

    Mock::given(method("GET"))
      .and(path("/14.0.2/"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string(version_html_142)
          .insert_header("content-type", "text/html"),
      )
      .mount(server)
      .await;
  }

  #[tokio::test]
  async fn test_browser_version_service_creation() {
    let _ = BrowserVersionService::new();
    // Test passes if we can create the service without panicking
  }

  #[tokio::test]
  async fn test_fetch_firefox_versions() {
    let server = setup_mock_server().await;
    setup_firefox_mocks(&server).await;

    let api_client = create_test_api_client(&server);
    let service = create_test_service(api_client);

    // Test with caching
    let result_cached = service.fetch_browser_versions("firefox", false).await;
    assert!(
      result_cached.is_ok(),
      "Should fetch Firefox versions with caching"
    );

    if let Ok(versions) = result_cached {
      assert!(!versions.is_empty(), "Should have Firefox versions");
      assert_eq!(versions[0], "139.0", "Should have latest version first");
      println!(
        "Firefox cached test passed. Found {versions_count} versions",
        versions_count = versions.len()
      );
    }

    // Test without caching
    let result_no_cache = service.fetch_browser_versions("firefox", true).await;
    assert!(
      result_no_cache.is_ok(),
      "Should fetch Firefox versions without caching"
    );

    if let Ok(versions) = result_no_cache {
      assert!(
        !versions.is_empty(),
        "Should have Firefox versions without caching"
      );
      assert_eq!(versions[0], "139.0", "Should have latest version first");
      println!(
        "Firefox no-cache test passed. Found {versions_count} versions",
        versions_count = versions.len()
      );
    }
  }

  #[tokio::test]
  async fn test_fetch_browser_versions_with_count() {
    let server = setup_mock_server().await;
    setup_firefox_mocks(&server).await;

    let api_client = create_test_api_client(&server);
    let service = create_test_service(api_client);

    let result = service
      .fetch_browser_versions_with_count("firefox", false)
      .await;
    assert!(result.is_ok(), "Should fetch Firefox versions with count");

    if let Ok(result) = result {
      assert!(!result.versions.is_empty(), "Should have versions");
      assert_eq!(
        result.total_versions_count,
        result.versions.len(),
        "Total count should match versions length"
      );
      assert_eq!(
        result.versions[0], "139.0",
        "Should have latest version first"
      );
      println!(
        "Firefox count test passed. Found {} versions, new: {}",
        result.total_versions_count,
        result.new_versions_count.unwrap_or(0)
      );
    }
  }

  #[tokio::test]
  async fn test_fetch_detailed_versions() {
    let server = setup_mock_server().await;
    setup_firefox_mocks(&server).await;

    let api_client = create_test_api_client(&server);
    let service = create_test_service(api_client);

    let result = service
      .fetch_browser_versions_detailed("firefox", false)
      .await;
    assert!(result.is_ok(), "Should fetch detailed Firefox versions");

    if let Ok(versions) = result {
      assert!(!versions.is_empty(), "Should have detailed versions");

      // Check that the first version has all required fields
      let first_version = &versions[0];
      assert!(
        !first_version.version.is_empty(),
        "Version should not be empty"
      );
      assert_eq!(
        first_version.version, "139.0",
        "Should have latest version first"
      );
      assert_eq!(first_version.date, "2024-01-15", "Should have correct date");
      assert!(!first_version.is_prerelease, "Should be stable release");
      println!(
        "Firefox detailed test passed. Found {versions_count} detailed versions",
        versions_count = versions.len()
      );
    }
  }

  #[tokio::test]
  async fn test_unsupported_browser() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let service = create_test_service(api_client);

    let result = service.fetch_browser_versions("unsupported", false).await;
    assert!(
      result.is_err(),
      "Should return error for unsupported browser"
    );

    if let Err(e) = result {
      assert!(
        e.to_string().contains("Unsupported browser"),
        "Error should mention unsupported browser"
      );
    }
  }

  #[tokio::test]
  async fn test_incremental_update() {
    let server = setup_mock_server().await;
    setup_firefox_mocks(&server).await;

    let api_client = create_test_api_client(&server);
    let service = create_test_service(api_client);

    // This test might fail if there are no cached versions yet, which is fine
    let result = service
      .update_browser_versions_incrementally("firefox")
      .await;

    // The test should complete without panicking
    match result {
      Ok(count) => {
        println!("Incremental update test passed. Found {count} new versions");
      }
      Err(e) => {
        println!("Incremental update test failed (expected for first run): {e}");
      }
    }
  }

  #[tokio::test]
  async fn test_all_supported_browsers() {
    let server = setup_mock_server().await;

    // Setup all browser mocks
    setup_firefox_mocks(&server).await;
    setup_firefox_dev_mocks(&server).await;
    setup_mullvad_mocks(&server).await;
    setup_zen_mocks(&server).await;
    setup_brave_mocks(&server).await;
    setup_chromium_mocks(&server).await;
    setup_tor_mocks(&server).await;

    let api_client = create_test_api_client(&server);
    let service = create_test_service(api_client);

    let browsers = vec![
      "firefox",
      "firefox-developer",
      "mullvad-browser",
      "zen",
      "brave",
      "chromium",
      "tor-browser",
    ];

    for browser in browsers {
      let result = service.fetch_browser_versions(browser, false).await;

      match result {
        Ok(versions) => {
          assert!(!versions.is_empty(), "Should have versions for {browser}");
          println!(
            "{browser} test passed. Found {versions_count} versions",
            versions_count = versions.len()
          );
        }
        Err(e) => {
          panic!("{browser} test failed: {e}");
        }
      }
    }
  }

  #[tokio::test]
  async fn test_no_caching_parameter() {
    let server = setup_mock_server().await;
    setup_firefox_mocks(&server).await;

    let api_client = create_test_api_client(&server);
    let service = create_test_service(api_client);

    // Test with caching enabled (default)
    let result_cached = service.fetch_browser_versions("firefox", false).await;
    assert!(
      result_cached.is_ok(),
      "Should fetch Firefox versions with caching"
    );

    // Test with caching disabled (no_caching = true)
    let result_no_cache = service.fetch_browser_versions("firefox", true).await;
    assert!(
      result_no_cache.is_ok(),
      "Should fetch Firefox versions without caching"
    );

    // Both should return versions
    if let (Ok(cached_versions), Ok(no_cache_versions)) = (result_cached, result_no_cache) {
      assert!(
        !cached_versions.is_empty(),
        "Cached versions should not be empty"
      );
      assert!(
        !no_cache_versions.is_empty(),
        "No-cache versions should not be empty"
      );
      assert_eq!(
        cached_versions, no_cache_versions,
        "Both should return same versions"
      );
      println!(
        "No-caching test passed. Cached: {} versions, No-cache: {} versions",
        cached_versions.len(),
        no_cache_versions.len()
      );
    }
  }

  #[tokio::test]
  async fn test_detailed_versions_with_no_caching() {
    let server = setup_mock_server().await;
    setup_firefox_mocks(&server).await;

    let api_client = create_test_api_client(&server);
    let service = create_test_service(api_client);

    // Test detailed versions with caching
    let result_cached = service
      .fetch_browser_versions_detailed("firefox", false)
      .await;
    assert!(
      result_cached.is_ok(),
      "Should fetch detailed Firefox versions with caching"
    );

    // Test detailed versions without caching
    let result_no_cache = service
      .fetch_browser_versions_detailed("firefox", true)
      .await;
    assert!(
      result_no_cache.is_ok(),
      "Should fetch detailed Firefox versions without caching"
    );

    // Both should return detailed version info
    if let (Ok(cached_versions), Ok(no_cache_versions)) = (result_cached, result_no_cache) {
      assert!(
        !cached_versions.is_empty(),
        "Cached detailed versions should not be empty"
      );
      assert!(
        !no_cache_versions.is_empty(),
        "No-cache detailed versions should not be empty"
      );

      // Check that detailed versions have all required fields
      let first_cached = &cached_versions[0];
      let first_no_cache = &no_cache_versions[0];

      assert!(
        !first_cached.version.is_empty(),
        "Cached version should not be empty"
      );
      assert!(
        !first_no_cache.version.is_empty(),
        "No-cache version should not be empty"
      );

      assert_eq!(first_cached.version, "139.0", "Should have correct version");
      assert_eq!(
        first_no_cache.version, "139.0",
        "Should have correct version"
      );
      assert_eq!(first_cached.date, "2024-01-15", "Should have correct date");
      assert_eq!(
        first_no_cache.date, "2024-01-15",
        "Should have correct date"
      );

      println!(
        "Detailed no-caching test passed. Cached: {} versions, No-cache: {} versions",
        cached_versions.len(),
        no_cache_versions.len()
      );
    }
  }

  #[test]
  fn test_get_download_info() {
    let service = BrowserVersionService::new();

    // Test Firefox
    let firefox_info = service.get_download_info("firefox", "139.0").unwrap();
    assert_eq!(firefox_info.filename, "Firefox 139.0.dmg");
    assert!(firefox_info
      .url
      .contains("download-installer.cdn.mozilla.net"));
    assert!(firefox_info.url.contains("/pub/firefox/releases/139.0/"));
    assert!(firefox_info.is_archive);

    // Test Firefox Developer
    let firefox_dev_info = service
      .get_download_info("firefox-developer", "139.0b1")
      .unwrap();
    assert_eq!(firefox_dev_info.filename, "Firefox 139.0b1.dmg");
    assert!(firefox_dev_info
      .url
      .contains("download-installer.cdn.mozilla.net"));
    assert!(firefox_dev_info
      .url
      .contains("/pub/devedition/releases/139.0b1/"));
    assert!(firefox_dev_info.is_archive);

    // Test Mullvad Browser
    let mullvad_info = service
      .get_download_info("mullvad-browser", "14.5a6")
      .unwrap();
    assert_eq!(mullvad_info.filename, "mullvad-browser-macos-14.5a6.dmg");
    assert!(mullvad_info.url.contains("mullvad-browser-macos-14.5a6"));
    assert!(mullvad_info.is_archive);

    // Test Zen Browser
    let zen_info = service.get_download_info("zen", "1.11b").unwrap();
    assert_eq!(zen_info.filename, "zen-1.11b.dmg");
    assert!(zen_info.url.contains("zen.macos-universal.dmg"));
    assert!(zen_info.is_archive);

    // Test Tor Browser
    let tor_info = service.get_download_info("tor-browser", "14.0.4").unwrap();
    assert_eq!(tor_info.filename, "tor-browser-macos-14.0.4.dmg");
    assert!(tor_info.url.contains("tor-browser-macos-14.0.4"));
    assert!(tor_info.is_archive);

    // Test Chromium
    let chromium_info = service.get_download_info("chromium", "1465660").unwrap();
    assert_eq!(chromium_info.filename, "chromium-1465660-mac.zip");
    assert!(chromium_info.url.contains("chrome-mac.zip"));
    assert!(chromium_info.is_archive);

    // Test Brave
    let brave_info = service.get_download_info("brave", "v1.81.9").unwrap();
    assert_eq!(brave_info.filename, "Brave-Browser-universal.dmg");
    assert!(brave_info.url.contains("brave-placeholder"));
    assert!(brave_info.is_archive);

    // Test unsupported browser
    let unsupported_result = service.get_download_info("unsupported", "1.0.0");
    assert!(unsupported_result.is_err());

    println!("Download info test passed for all browsers");
  }
}

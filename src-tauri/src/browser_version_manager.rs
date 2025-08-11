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
pub struct BrowserReleaseTypes {
  pub stable: Option<String>,
  pub nightly: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadInfo {
  pub url: String,
  pub filename: String,
  pub is_archive: bool, // true for .dmg, .zip, etc.
}

pub struct BrowserVersionManager {
  api_client: &'static ApiClient,
}

impl BrowserVersionManager {
  fn new() -> Self {
    Self {
      api_client: ApiClient::instance(),
    }
  }

  pub fn instance() -> &'static BrowserVersionManager {
    &BROWSER_VERSION_SERVICE
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
      "camoufox" => {
        // Camoufox supports all platforms and architectures according to the JS code
        Ok(true)
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
      "camoufox",
    ];

    all_browsers
      .into_iter()
      .filter(|browser| self.is_browser_supported(browser).unwrap_or(false))
      .map(|s| s.to_string())
      .collect()
  }

  /// Get cached browser versions immediately (returns None if no cache exists)
  pub fn get_cached_browser_versions(&self, browser: &str) -> Option<Vec<String>> {
    if browser == "brave" {
      return ApiClient::instance()
        .get_cached_github_releases("brave")
        .map(|releases| releases.into_iter().map(|r| r.tag_name).collect());
    }

    self
      .api_client
      .load_cached_versions(browser)
      .map(|releases| releases.into_iter().map(|r| r.version).collect())
  }

  /// Get cached detailed browser version information immediately
  pub fn get_cached_browser_versions_detailed(
    &self,
    browser: &str,
  ) -> Option<Vec<BrowserVersionInfo>> {
    if browser == "brave" {
      if let Some(releases) = ApiClient::instance().get_cached_github_releases("brave") {
        let detailed_info: Vec<BrowserVersionInfo> = releases
          .into_iter()
          .map(|r| BrowserVersionInfo {
            version: r.tag_name,
            is_prerelease: r.is_nightly,
            date: r.published_at,
          })
          .collect();
        return Some(detailed_info);
      }
    }

    let cached_releases = self.api_client.load_cached_versions(browser)?;

    // Convert cached versions to detailed info (without dates since cache doesn't store them)
    let detailed_info: Vec<BrowserVersionInfo> = cached_releases
      .into_iter()
      .map(|r| BrowserVersionInfo {
        version: r.version,
        is_prerelease: r.is_prerelease,
        date: r.date,
      })
      .collect();

    Some(detailed_info)
  }

  /// Check if cache should be updated (expired or doesn't exist)
  pub fn should_update_cache(&self, browser: &str) -> bool {
    self.api_client.is_cache_expired(browser)
  }

  /// Get latest stable and nightly versions for a browser (cached first)
  pub async fn get_browser_release_types(
    &self,
    browser: &str,
  ) -> Result<BrowserReleaseTypes, Box<dyn std::error::Error + Send + Sync>> {
    // Try to get from cache first
    if let Some(cached_versions) = self.get_cached_browser_versions_detailed(browser) {
      let latest_stable = cached_versions
        .iter()
        .find(|v| !v.is_prerelease)
        .map(|v| v.version.clone());

      let latest_nightly = cached_versions
        .iter()
        .find(|v| v.is_prerelease)
        .map(|v| v.version.clone());

      return Ok(BrowserReleaseTypes {
        stable: latest_stable,
        nightly: latest_nightly,
      });
    }

    let detailed_versions = self.fetch_browser_versions_detailed(browser, false).await?;

    let latest_stable = detailed_versions
      .iter()
      .find(|v| !v.is_prerelease)
      .map(|v| v.version.clone());

    let latest_nightly = detailed_versions
      .iter()
      .find(|v| v.is_prerelease)
      .map(|v| v.version.clone());

    Ok(BrowserReleaseTypes {
      stable: latest_stable,
      nightly: latest_nightly,
    })
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
    let existing_set: HashSet<String> = existing_versions.into_iter().map(|r| r.version).collect();

    // Fetch fresh versions from API
    let fresh_versions = match browser {
      "firefox" => self.fetch_firefox_versions(true).await?, // Always fetch fresh for merging
      "firefox-developer" => self.fetch_firefox_developer_versions(true).await?,
      "mullvad-browser" => self.fetch_mullvad_versions(true).await?,
      "zen" => self.fetch_zen_versions(true).await?,
      "brave" => self.fetch_brave_versions(true).await?,
      "chromium" => self.fetch_chromium_versions(true).await?,
      "tor-browser" => self.fetch_tor_versions(true).await?,
      "camoufox" => self.fetch_camoufox_versions(true).await?,
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
    if !no_caching && browser != "brave" {
      let merged_releases: Vec<BrowserRelease> = merged_versions
        .iter()
        .map(|v| BrowserRelease {
          version: v.clone(),
          date: "".to_string(),
          is_prerelease: crate::api_client::is_browser_version_nightly(browser, v, None),
        })
        .collect();
      if let Err(e) = self
        .api_client
        .save_cached_versions(browser, &merged_releases)
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
          // Filter out twilight releases at the detailed level too
          .filter(|version| version.to_lowercase() != "twilight")
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
      "camoufox" => {
        let releases = self.fetch_camoufox_releases_detailed(true).await?;
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
                is_prerelease: false, // Camoufox usually stable releases
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      _ => {
        return Err(format!("Unsupported browser: {browser}").into());
      }
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
    let existing_set: HashSet<String> = existing_versions.into_iter().map(|r| r.version).collect();

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
    let releases: Vec<BrowserRelease> = all_versions
      .iter()
      .map(|v| BrowserRelease {
        version: v.clone(),
        date: "".to_string(),
        is_prerelease: crate::api_client::is_browser_version_nightly(browser, v, None),
      })
      .collect();
    if let Err(e) = self.api_client.save_cached_versions(browser, &releases) {
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
            "https://github.com/brave/brave-browser/releases/download/{version}/{filename}"
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
      "camoufox" => {
        // Camoufox downloads from GitHub releases with pattern: camoufox-{version}-{release}-{os}.{arch}.zip
        let (os_name, arch_name) = match (&os[..], &arch[..]) {
          ("windows", "x64") => ("win", "x86_64"),
          ("windows", "arm64") => ("win", "arm64"),
          ("linux", "x64") => ("lin", "x86_64"),
          ("linux", "arm64") => ("lin", "arm64"),
          ("macos", "x64") => ("mac", "x86_64"),
          ("macos", "arm64") => ("mac", "arm64"),
          _ => {
            return Err(
              format!("Unsupported platform/architecture for Camoufox: {os}/{arch}").into(),
            )
          }
        };

        // Note: We provide a placeholder URL here since Camoufox requires dynamic resolution
        // The actual URL will be resolved in download.rs resolve_download_url
        Ok(DownloadInfo {
          url: format!(
            "https://github.com/daijro/camoufox/releases/download/{version}/camoufox-{{version}}-{{release}}-{os_name}.{arch_name}.zip"
          ),
          filename: format!("camoufox-{version}-{os_name}.{arch_name}.zip"),
          is_archive: true,
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
    Ok(
      releases
        .into_iter()
        .filter(|r| r.tag_name.to_lowercase() != "twilight")
        .map(|r| r.tag_name)
        .collect(),
    )
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
    // Persist a lightweight versions cache with accurate prerelease info for Brave
    let converted: Vec<BrowserRelease> = releases
      .iter()
      .map(|r| BrowserRelease {
        version: r.tag_name.clone(),
        date: r.published_at.clone(),
        is_prerelease: r.is_nightly,
      })
      .collect();
    // Always save so that other callers without release_name can classify correctly
    if let Err(e) = self.api_client.save_cached_versions("brave", &converted) {
      eprintln!("Failed to persist Brave versions cache: {e}");
    }

    Ok(releases.into_iter().map(|r| r.tag_name).collect())
  }

  async fn fetch_brave_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self
      .api_client
      .fetch_brave_releases_with_caching(no_caching)
      .await?;

    // Save a parallel versions cache for Brave with accurate prerelease flags
    let converted: Vec<BrowserRelease> = releases
      .iter()
      .map(|r| BrowserRelease {
        version: r.tag_name.clone(),
        date: r.published_at.clone(),
        is_prerelease: r.is_nightly,
      })
      .collect();
    if let Err(e) = self.api_client.save_cached_versions("brave", &converted) {
      eprintln!("Failed to persist Brave versions cache: {e}");
    }

    Ok(releases)
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

  async fn fetch_camoufox_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let releases = self.fetch_camoufox_releases_detailed(no_caching).await?;
    Ok(releases.into_iter().map(|r| r.tag_name).collect())
  }

  async fn fetch_camoufox_releases_detailed(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self
      .api_client
      .fetch_camoufox_releases_with_caching(no_caching)
      .await
  }
}

#[tauri::command]
pub async fn get_browser_release_types(
  browser_str: String,
) -> Result<crate::browser_version_manager::BrowserReleaseTypes, String> {
  let service = BrowserVersionManager::instance();
  service
    .get_browser_release_types(&browser_str)
    .await
    .map_err(|e| format!("Failed to get release types: {e}"))
}

#[cfg(test)]
mod tests {
  use super::*;

  use wiremock::MockServer;

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

  fn create_test_service(_api_client: ApiClient) -> &'static BrowserVersionManager {
    BrowserVersionManager::instance()
  }

  #[tokio::test]
  async fn test_browser_version_manager_creation() {
    let _ = BrowserVersionManager::instance();
    // Test passes if we can create the service without panicking
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

  #[test]
  fn test_get_download_info() {
    let service = BrowserVersionManager::instance();

    // Test Firefox - platform-specific expectations
    let firefox_info = service.get_download_info("firefox", "139.0").unwrap();

    #[cfg(target_os = "macos")]
    {
      assert_eq!(firefox_info.filename, "Firefox 139.0.dmg");
      assert!(firefox_info.is_archive);
    }

    #[cfg(target_os = "linux")]
    {
      assert_eq!(firefox_info.filename, "firefox-139.0.tar.xz");
      assert!(firefox_info.is_archive);
    }

    #[cfg(target_os = "windows")]
    {
      assert_eq!(firefox_info.filename, "Firefox Setup 139.0.exe");
      assert!(!firefox_info.is_archive);
    }

    assert!(firefox_info
      .url
      .contains("download-installer.cdn.mozilla.net"));
    assert!(firefox_info.url.contains("/pub/firefox/releases/139.0/"));

    // Test Firefox Developer
    let firefox_dev_info = service
      .get_download_info("firefox-developer", "139.0b1")
      .unwrap();

    #[cfg(target_os = "macos")]
    {
      assert_eq!(firefox_dev_info.filename, "Firefox 139.0b1.dmg");
      assert!(firefox_dev_info.is_archive);
    }

    #[cfg(target_os = "linux")]
    {
      assert_eq!(firefox_dev_info.filename, "firefox-139.0b1.tar.xz");
      assert!(firefox_dev_info.is_archive);
    }

    #[cfg(target_os = "windows")]
    {
      assert_eq!(firefox_dev_info.filename, "Firefox Setup 139.0b1.exe");
      assert!(!firefox_dev_info.is_archive);
    }

    assert!(firefox_dev_info
      .url
      .contains("download-installer.cdn.mozilla.net"));
    assert!(firefox_dev_info
      .url
      .contains("/pub/devedition/releases/139.0b1/"));

    // Test Mullvad Browser
    let mullvad_info = service
      .get_download_info("mullvad-browser", "14.5a6")
      .unwrap();

    #[cfg(target_os = "macos")]
    {
      assert_eq!(mullvad_info.filename, "mullvad-browser-macos-14.5a6.dmg");
      assert!(mullvad_info.url.contains("mullvad-browser-macos-14.5a6"));
      assert!(mullvad_info.is_archive);
    }

    #[cfg(target_os = "linux")]
    {
      assert_eq!(
        mullvad_info.filename,
        "mullvad-browser-x86_64-14.5a6.tar.xz"
      );
      assert!(mullvad_info.url.contains("mullvad-browser-x86_64-14.5a6"));
      assert!(mullvad_info.is_archive);
    }

    #[cfg(target_os = "windows")]
    {
      assert_eq!(
        mullvad_info.filename,
        "mullvad-browser-windows-x86_64-14.5a6.exe"
      );
      assert!(mullvad_info
        .url
        .contains("mullvad-browser-windows-x86_64-14.5a6"));
      assert!(!mullvad_info.is_archive);
    }

    // Test Zen Browser
    let zen_info = service.get_download_info("zen", "1.11b").unwrap();

    #[cfg(target_os = "macos")]
    {
      assert_eq!(zen_info.filename, "zen-1.11b.dmg");
      assert!(zen_info.url.contains("zen.macos-universal.dmg"));
      assert!(zen_info.is_archive);
    }

    #[cfg(target_os = "linux")]
    {
      assert_eq!(zen_info.filename, "zen-1.11b-x86_64.tar.xz");
      assert!(zen_info.url.contains("zen.linux-x86_64.tar.xz"));
      assert!(zen_info.is_archive);
    }

    #[cfg(target_os = "windows")]
    {
      assert_eq!(zen_info.filename, "zen-1.11b.exe");
      assert!(zen_info.url.contains("zen.installer.exe"));
      assert!(!zen_info.is_archive);
    }

    // Test Tor Browser
    let tor_info = service.get_download_info("tor-browser", "14.0.4").unwrap();

    #[cfg(target_os = "macos")]
    {
      assert_eq!(tor_info.filename, "tor-browser-macos-14.0.4.dmg");
      assert!(tor_info.url.contains("tor-browser-macos-14.0.4"));
      assert!(tor_info.is_archive);
    }

    #[cfg(target_os = "linux")]
    {
      assert_eq!(tor_info.filename, "tor-browser-linux-x86_64-14.0.4.tar.xz");
      assert!(tor_info.url.contains("tor-browser-linux-x86_64-14.0.4"));
      assert!(tor_info.is_archive);
    }

    #[cfg(target_os = "windows")]
    {
      assert_eq!(
        tor_info.filename,
        "tor-browser-windows-x86_64-portable-14.0.4.exe"
      );
      assert!(tor_info
        .url
        .contains("tor-browser-windows-x86_64-portable-14.0.4"));
      assert!(!tor_info.is_archive);
    }

    // Test Chromium
    let chromium_info = service.get_download_info("chromium", "1465660").unwrap();

    #[cfg(target_os = "macos")]
    {
      assert_eq!(chromium_info.filename, "chromium-1465660-mac.zip");
      assert!(chromium_info.url.contains("chrome-mac.zip"));
    }

    #[cfg(target_os = "linux")]
    {
      assert_eq!(chromium_info.filename, "chromium-1465660-linux.zip");
      assert!(chromium_info.url.contains("chrome-linux.zip"));
    }

    #[cfg(target_os = "windows")]
    {
      assert_eq!(chromium_info.filename, "chromium-1465660-win.zip");
      assert!(chromium_info.url.contains("chrome-win.zip"));
    }

    assert!(chromium_info.is_archive);

    // Test Brave - Note: Brave uses dynamic URL resolution, so get_download_info provides a template URL
    let brave_info = service.get_download_info("brave", "v1.81.9").unwrap();

    #[cfg(target_os = "macos")]
    {
      assert_eq!(brave_info.filename, "Brave-Browser-universal.dmg");
      assert_eq!(brave_info.url, "https://github.com/brave/brave-browser/releases/download/v1.81.9/Brave-Browser-universal.dmg");
      assert!(brave_info.is_archive);
    }

    #[cfg(target_os = "linux")]
    {
      assert_eq!(brave_info.filename, "brave-browser-v1.81.9-linux-amd64.zip");
      assert_eq!(brave_info.url, "https://github.com/brave/brave-browser/releases/download/v1.81.9/brave-browser-v1.81.9-linux-amd64.zip");
      assert!(brave_info.is_archive);
    }

    #[cfg(target_os = "windows")]
    {
      assert_eq!(brave_info.filename, "brave-v1.81.9.exe");
      assert_eq!(
        brave_info.url,
        "https://github.com/brave/brave-browser/releases/download/v1.81.9/brave-v1.81.9.exe"
      );
      assert!(!brave_info.is_archive);
    }

    // Test unsupported browser
    let unsupported_result = service.get_download_info("unsupported", "1.0.0");
    assert!(unsupported_result.is_err());

    println!("Download info test passed for all browsers");
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_VERSION_SERVICE: BrowserVersionManager = BrowserVersionManager::new();
}

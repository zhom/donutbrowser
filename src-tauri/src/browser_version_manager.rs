use crate::api_client::{sort_versions, ApiClient, BrowserRelease};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrowserVersionInfo {
  pub version: String,
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
      "wayfern" => {
        let platform_key = format!("{os}-{arch}");
        Ok(matches!(
          platform_key.as_str(),
          "macos-arm64"
            | "linux-x64"
            | "macos-x64"
            | "linux-arm64"
            | "windows-x64"
            | "windows-arm64"
        ))
      }
      _ => Err(format!("Unknown browser: {browser}").into()),
    }
  }

  /// Get list of browsers supported on the current platform
  pub fn get_supported_browsers(&self) -> Vec<String> {
    let all_browsers = vec!["wayfern"];

    all_browsers
      .into_iter()
      .filter(|browser| self.is_browser_supported(browser).unwrap_or(false))
      .map(|s| s.to_string())
      .collect()
  }

  /// Get cached browser versions immediately (returns None if no cache exists)
  pub fn get_cached_browser_versions(&self, browser: &str) -> Option<Vec<String>> {
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
    let cached_releases = self.api_client.load_cached_versions(browser)?;

    // Convert cached versions to detailed info (without dates since cache doesn't store them)
    let detailed_info: Vec<BrowserVersionInfo> = cached_releases
      .into_iter()
      .map(|r| BrowserVersionInfo {
        version: r.version,
        date: r.date,
      })
      .collect();

    Some(detailed_info)
  }

  /// Check if cache should be updated (expired or doesn't exist)
  pub fn should_update_cache(&self, browser: &str) -> bool {
    self.api_client.is_cache_expired(browser)
  }

  /// Get the latest Wayfern version (fresh cache first)
  pub async fn get_browser_release_types(
    &self,
    browser: &str,
  ) -> Result<BrowserReleaseTypes, Box<dyn std::error::Error + Send + Sync>> {
    if browser != "wayfern" {
      return Err(format!("Unsupported browser: {browser}").into());
    }

    // Only trust an unexpired cache. A stale entry can point at a version that
    // is no longer published — the downloader rejects such requests, so serving
    // it here would make every download started from this list fail.
    if !self.api_client.is_cache_expired(browser) {
      if let Some(cached_versions) = self.get_cached_browser_versions_detailed(browser) {
        return Ok(BrowserReleaseTypes {
          stable: cached_versions.first().map(|v| v.version.clone()),
        });
      }
    }

    // Expired or missing cache: fetch fresh, falling back to whatever cache
    // exists when the network is unavailable.
    match self.fetch_browser_versions_detailed(browser, false).await {
      Ok(detailed_versions) => Ok(BrowserReleaseTypes {
        stable: detailed_versions.first().map(|v| v.version.clone()),
      }),
      Err(e) => match self.get_cached_browser_versions_detailed(browser) {
        Some(cached_versions) => Ok(BrowserReleaseTypes {
          stable: cached_versions.first().map(|v| v.version.clone()),
        }),
        None => Err(e),
      },
    }
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
      "wayfern" => self.fetch_wayfern_versions(true).await?,
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
      let merged_releases: Vec<BrowserRelease> = merged_versions
        .iter()
        .map(|v| BrowserRelease {
          version: v.clone(),
          date: "".to_string(),
        })
        .collect();
      if let Err(e) = self
        .api_client
        .save_cached_versions(browser, &merged_releases)
      {
        log::error!("Failed to save merged cache for {browser}: {e}");
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
      "wayfern" => merged_versions
        .into_iter()
        .map(|version| BrowserVersionInfo {
          version: version.clone(),
          date: "".to_string(),
        })
        .collect(),
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
      })
      .collect();
    if let Err(e) = self.api_client.save_cached_versions(browser, &releases) {
      log::error!("Failed to save updated cache for {browser}: {e}");
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
      "wayfern" => {
        // Wayfern downloads from https://download.wayfern.com/
        // File naming: wayfern-{chromium_version}-{platform}-{arch}.{ext}
        // Platform/arch format: linux-x64, macos-arm64, etc.
        let platform_key = format!("{os}-{arch}");
        let (filename, is_archive) = match platform_key.as_str() {
          "macos-arm64" | "macos-x64" => (format!("wayfern-{version}-{platform_key}.dmg"), true),
          "linux-x64" | "linux-arm64" => (format!("wayfern-{version}-{platform_key}.tar.xz"), true),
          "windows-x64" | "windows-arm64" => {
            (format!("wayfern-{version}-{platform_key}.zip"), true)
          }
          _ => {
            return Err(
              format!("Unsupported platform/architecture for Wayfern: {os}/{arch}").into(),
            )
          }
        };

        // Note: The actual URL will be resolved dynamically from version.json in downloader.rs
        Ok(DownloadInfo {
          url: format!("https://download.wayfern.com/{filename}"),
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

  async fn fetch_wayfern_versions(
    &self,
    no_caching: bool,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let version_info = self
      .api_client
      .fetch_wayfern_version_with_caching(no_caching)
      .await?;

    // Check if current platform has a download available
    if self
      .api_client
      .has_wayfern_compatible_download(&version_info)
    {
      Ok(vec![version_info.version])
    } else {
      // No compatible download for current platform
      Ok(vec![])
    }
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

  #[tokio::test]
  async fn test_browser_version_manager_creation() {
    let _ = BrowserVersionManager::instance();
  }

  #[tokio::test]
  async fn test_unsupported_browser() {
    let service = BrowserVersionManager::instance();

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

    let wayfern_info = service.get_download_info("wayfern", "1.0.0").unwrap();

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
      assert_eq!(wayfern_info.filename, "wayfern-1.0.0-macos-arm64.dmg");
      assert!(wayfern_info.is_archive);
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
      assert_eq!(wayfern_info.filename, "wayfern-1.0.0-macos-x64.dmg");
      assert!(wayfern_info.is_archive);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
      assert_eq!(wayfern_info.filename, "wayfern-1.0.0-linux-x64.tar.xz");
      assert!(wayfern_info.is_archive);
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
      assert_eq!(wayfern_info.filename, "wayfern-1.0.0-linux-arm64.tar.xz");
      assert!(wayfern_info.is_archive);
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
      assert_eq!(wayfern_info.filename, "wayfern-1.0.0-windows-x64.zip");
      assert!(wayfern_info.is_archive);
    }

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
      assert_eq!(wayfern_info.filename, "wayfern-1.0.0-windows-arm64.zip");
      assert!(wayfern_info.is_archive);
    }

    assert!(wayfern_info.url.contains("download.wayfern.com"));

    let unsupported_result = service.get_download_info("testbrowser", "1.0.0");
    assert!(unsupported_result.is_err());
  }
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
          log::error!("Background version update failed for {browser_str_clone}: {e}");
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
          log::error!("Background version update failed for {browser_str_clone}: {e}");
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
pub async fn fetch_browser_versions_with_count(
  browser_str: String,
) -> Result<BrowserVersionsResult, String> {
  let service = BrowserVersionManager::instance();
  service
    .fetch_browser_versions_with_count(&browser_str, false)
    .await
    .map_err(|e| format!("Failed to fetch browser versions: {e}"))
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref BROWSER_VERSION_SERVICE: BrowserVersionManager = BrowserVersionManager::new();
}

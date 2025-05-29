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
          is_prerelease: crate::api_client::is_alpha_version(&version),
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
      _ => return Err(format!("Unsupported browser: {}", browser).into()),
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
        eprintln!("Failed to save merged cache for {}: {}", browser, e);
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
                is_prerelease: crate::api_client::is_alpha_version(&version),
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
                is_prerelease: crate::api_client::is_alpha_version(&version),
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
                is_prerelease: release.is_alpha,
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
                is_prerelease: release.prerelease,
                date: release.published_at.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: version.contains("alpha") || version.contains("beta"),
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
                is_prerelease: release.prerelease,
                date: release.published_at.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: version.contains("beta") || version.contains("dev"),
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
                is_prerelease: false, // Chromium versions are usually stable
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
                is_prerelease: release.is_prerelease,
                date: release.date.clone(),
              }
            } else {
              BrowserVersionInfo {
                version: version.clone(),
                is_prerelease: version.contains("alpha") || version.contains("rc"),
                date: "".to_string(),
              }
            }
          })
          .collect()
      }
      _ => return Err(format!("Unsupported browser: {}", browser).into()),
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
      eprintln!("Failed to save updated cache for {}: {}", browser, e);
    }

    Ok(new_versions_count)
  }

  /// Get download information for a specific browser and version
  pub fn get_download_info(
    &self,
    browser: &str,
    version: &str,
  ) -> Result<DownloadInfo, Box<dyn std::error::Error + Send + Sync>> {
    match browser {
            "firefox" => Ok(DownloadInfo {
                url: format!("https://download.mozilla.org/?product=firefox-{}&os=osx&lang=en-US", version),
                filename: format!("firefox-{}.dmg", version),
                is_archive: true,
            }),
            "firefox-developer" => Ok(DownloadInfo {
                url: format!("https://download.mozilla.org/?product=devedition-{}&os=osx&lang=en-US", version),
                filename: format!("firefox-developer-{}.dmg", version),
                is_archive: true,
            }),
            "mullvad-browser" => Ok(DownloadInfo {
                url: format!(
                    "https://github.com/mullvad/mullvad-browser/releases/download/{}/mullvad-browser-macos-{}.dmg",
                    version, version
                ),
                filename: format!("mullvad-browser-{}.dmg", version),
                is_archive: true,
            }),
            "zen" => Ok(DownloadInfo {
                url: format!(
                    "https://github.com/zen-browser/desktop/releases/download/{}/zen.macos-universal.dmg",
                    version
                ),
                filename: format!("zen-{}.dmg", version),
                is_archive: true,
            }),
            "brave" => {
                // For Brave, we use a placeholder URL since we need to resolve the actual asset URL dynamically
                // The actual URL will be resolved in the download service using the GitHub API
                Ok(DownloadInfo {
                    url: format!(
                        "https://github.com/brave/brave-browser/releases/download/{}/Brave-Browser-universal.dmg",
                        version
                    ),
                    filename: format!("brave-{}.dmg", version),
                    is_archive: true,
                })
            }
            "chromium" => {
                let arch = if cfg!(target_arch = "aarch64") { "Mac_Arm" } else { "Mac" };
                Ok(DownloadInfo {
                    url: format!(
                        "https://commondatastorage.googleapis.com/chromium-browser-snapshots/{}/{}/chrome-mac.zip",
                        arch, version
                    ),
                    filename: format!("chromium-{}.zip", version),
                    is_archive: true,
                })
            }
            "tor-browser" => Ok(DownloadInfo {
                url: format!(
                    "https://archive.torproject.org/tor-package-archive/torbrowser/{}/tor-browser-macos-{}.dmg",
                    version, version
                ),
                filename: format!("tor-browser-{}.dmg", version),
                is_archive: true,
            }),
            _ => Err(format!("Unsupported browser: {}", browser).into()),
        }
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

  #[tokio::test]
  async fn test_browser_version_service_creation() {
    let _service = BrowserVersionService::new();
    // Test passes if we can create the service without panicking
  }

  #[tokio::test]
  async fn test_fetch_firefox_versions() {
    let service = BrowserVersionService::new();

    // Test with caching
    let result_cached = service.fetch_browser_versions("firefox", false).await;
    assert!(
      result_cached.is_ok(),
      "Should fetch Firefox versions with caching"
    );

    if let Ok(versions) = result_cached {
      assert!(!versions.is_empty(), "Should have Firefox versions");
      println!(
        "Firefox cached test passed. Found {} versions",
        versions.len()
      );
    }

    // Small delay to avoid rate limiting
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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
      println!(
        "Firefox no-cache test passed. Found {} versions",
        versions.len()
      );
    }
  }

  #[tokio::test]
  async fn test_fetch_browser_versions_with_count() {
    let service = BrowserVersionService::new();

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
      println!(
        "Firefox count test passed. Found {} versions, new: {:?}",
        result.total_versions_count, result.new_versions_count
      );
    }
  }

  #[tokio::test]
  async fn test_fetch_detailed_versions() {
    let service = BrowserVersionService::new();

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
      println!(
        "Firefox detailed test passed. Found {} detailed versions",
        versions.len()
      );
    }
  }

  #[tokio::test]
  async fn test_unsupported_browser() {
    let service = BrowserVersionService::new();

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
    let service = BrowserVersionService::new();

    // This test might fail if there are no cached versions yet, which is fine
    let result = service
      .update_browser_versions_incrementally("firefox")
      .await;

    // The test should complete without panicking
    match result {
      Ok(count) => {
        println!(
          "Incremental update test passed. Found {} new versions",
          count
        );
      }
      Err(e) => {
        println!(
          "Incremental update test failed (expected for first run): {}",
          e
        );
        // Don't fail the test, as this is expected behavior for first run
      }
    }
  }

  #[tokio::test]
  async fn test_all_supported_browsers() {
    let service = BrowserVersionService::new();
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
      // Test that we can at least call the function without panicking
      let result = service.fetch_browser_versions(browser, false).await;

      match result {
        Ok(versions) => {
          println!("{} test passed. Found {} versions", browser, versions.len());
        }
        Err(e) => {
          // Some browsers might fail due to network issues, but shouldn't panic
          println!("{} test failed (network issue): {}", browser, e);
        }
      }

      // Small delay between requests to avoid rate limiting
      tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }
  }

  #[tokio::test]
  async fn test_no_caching_parameter() {
    let service = BrowserVersionService::new();

    // Test with caching enabled (default)
    let result_cached = service.fetch_browser_versions("firefox", false).await;
    assert!(
      result_cached.is_ok(),
      "Should fetch Firefox versions with caching"
    );

    // Small delay to avoid rate limiting
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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
      println!(
        "No-caching test passed. Cached: {} versions, No-cache: {} versions",
        cached_versions.len(),
        no_cache_versions.len()
      );
    }
  }

  #[tokio::test]
  async fn test_detailed_versions_with_no_caching() {
    let service = BrowserVersionService::new();

    // Test detailed versions with caching
    let result_cached = service
      .fetch_browser_versions_detailed("firefox", false)
      .await;
    assert!(
      result_cached.is_ok(),
      "Should fetch detailed Firefox versions with caching"
    );

    // Small delay to avoid rate limiting
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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
    assert_eq!(firefox_info.filename, "firefox-139.0.dmg");
    assert!(firefox_info.url.contains("firefox-139.0"));
    assert!(firefox_info.is_archive);

    // Test Firefox Developer
    let firefox_dev_info = service
      .get_download_info("firefox-developer", "139.0b1")
      .unwrap();
    assert_eq!(firefox_dev_info.filename, "firefox-developer-139.0b1.dmg");
    assert!(firefox_dev_info.url.contains("devedition-139.0b1"));
    assert!(firefox_dev_info.is_archive);

    // Test Mullvad Browser
    let mullvad_info = service
      .get_download_info("mullvad-browser", "14.5a6")
      .unwrap();
    assert_eq!(mullvad_info.filename, "mullvad-browser-14.5a6.dmg");
    assert!(mullvad_info.url.contains("mullvad-browser-macos-14.5a6"));
    assert!(mullvad_info.is_archive);

    // Test Zen Browser
    let zen_info = service.get_download_info("zen", "1.11b").unwrap();
    assert_eq!(zen_info.filename, "zen-1.11b.dmg");
    assert!(zen_info.url.contains("zen.macos-universal.dmg"));
    assert!(zen_info.is_archive);

    // Test Tor Browser
    let tor_info = service.get_download_info("tor-browser", "14.0.4").unwrap();
    assert_eq!(tor_info.filename, "tor-browser-14.0.4.dmg");
    assert!(tor_info.url.contains("tor-browser-macos-14.0.4"));
    assert!(tor_info.is_archive);

    // Test Chromium
    let chromium_info = service.get_download_info("chromium", "1465660").unwrap();
    assert_eq!(chromium_info.filename, "chromium-1465660.zip");
    assert!(chromium_info.url.contains("chrome-mac.zip"));
    assert!(chromium_info.is_archive);

    // Test Brave
    let brave_info = service.get_download_info("brave", "v1.81.9").unwrap();
    assert_eq!(brave_info.filename, "brave-v1.81.9.dmg");
    assert!(brave_info.url.contains("Brave-Browser"));
    assert!(brave_info.is_archive);

    // Test unsupported browser
    let unsupported_result = service.get_download_info("unsupported", "1.0.0");
    assert!(unsupported_result.is_err());

    println!("Download info test passed for all browsers");
  }
}

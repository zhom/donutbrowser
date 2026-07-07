use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionComponent {
  pub major: u32,
  pub minor: u32,
  pub patch: u32,
  pub build: u32,
}

impl VersionComponent {
  pub fn parse(version: &str) -> Self {
    let version = version.trim();
    let version = if version.starts_with('v') || version.starts_with('V') {
      &version[1..]
    } else {
      version
    };

    let numeric_part = Self::numeric_prefix(version);

    let parts: Vec<u32> = numeric_part
      .split('.')
      .filter_map(|part| part.parse().ok())
      .collect();

    VersionComponent {
      major: parts.first().copied().unwrap_or(0),
      minor: parts.get(1).copied().unwrap_or(0),
      patch: parts.get(2).copied().unwrap_or(0),
      build: parts.get(3).copied().unwrap_or(0),
    }
  }

  fn numeric_prefix(version: &str) -> String {
    let version = version.to_lowercase();
    for (i, ch) in version.char_indices() {
      if ch.is_alphabetic() && i > 0 {
        return version[..i].to_string();
      }
    }
    version
  }
}

impl PartialOrd for VersionComponent {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for VersionComponent {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    (self.major, self.minor, self.patch, self.build).cmp(&(
      other.major,
      other.minor,
      other.patch,
      other.build,
    ))
  }
}

pub fn sort_versions(versions: &mut [String]) {
  versions.sort_by(|a, b| {
    let version_a = VersionComponent::parse(a);
    let version_b = VersionComponent::parse(b);
    version_b.cmp(&version_a)
  });
}

pub fn compare_versions(version1: &str, version2: &str) -> std::cmp::Ordering {
  let version_a = VersionComponent::parse(version1);
  let version_b = VersionComponent::parse(version2);
  version_a.cmp(&version_b)
}

pub fn is_version_newer(version1: &str, version2: &str) -> bool {
  let version_a = VersionComponent::parse(version1);
  let version_b = VersionComponent::parse(version2);
  version_a > version_b
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrowserRelease {
  pub version: String,
  pub date: String,
}

/// Wayfern version info from https://donutbrowser.com/wayfern.json
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WayfernVersionInfo {
  pub version: String,
  pub downloads: std::collections::HashMap<String, Option<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedVersionData {
  releases: Vec<BrowserRelease>,
  timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedWayfernData {
  version_info: WayfernVersionInfo,
  timestamp: u64,
}

pub struct ApiClient {
  client: Client,
}

impl ApiClient {
  pub fn new() -> Self {
    let client = Client::builder()
      .timeout(std::time::Duration::from_secs(30))
      .build()
      .unwrap_or_else(|_| Client::new());

    Self { client }
  }

  pub fn instance() -> &'static ApiClient {
    &API_CLIENT
  }

  fn get_cache_dir() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = crate::app_dirs::cache_dir().join("version_cache");
    fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir)
  }

  fn get_current_timestamp() -> u64 {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs()
  }

  fn is_cache_valid(timestamp: u64) -> bool {
    let current_time = Self::get_current_timestamp();
    let cache_duration = 10 * 60;
    current_time - timestamp < cache_duration
  }

  pub fn load_cached_versions(&self, browser: &str) -> Option<Vec<BrowserRelease>> {
    let cache_dir = Self::get_cache_dir().ok()?;
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    if !cache_file.exists() {
      return None;
    }

    let content = fs::read_to_string(&cache_file).ok()?;
    if let Ok(cached) = serde_json::from_str::<CachedVersionData>(&content) {
      log::info!("Using cached versions for {browser}");
      return Some(cached.releases);
    }

    if let Ok(legacy_versions) = serde_json::from_str::<Vec<String>>(&content) {
      log::info!("Using legacy cached versions for {browser}; upgrading in-memory");
      let releases: Vec<BrowserRelease> = legacy_versions
        .into_iter()
        .map(|version| BrowserRelease {
          version,
          date: "".to_string(),
        })
        .collect();
      return Some(releases);
    }

    None
  }

  pub fn is_cache_expired(&self, browser: &str) -> bool {
    let cache_dir = match Self::get_cache_dir() {
      Ok(dir) => dir,
      Err(_) => return true,
    };
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    if !cache_file.exists() {
      return true;
    }

    let content = match fs::read_to_string(&cache_file) {
      Ok(content) => content,
      Err(_) => return true,
    };

    let cached_data: CachedVersionData = match serde_json::from_str(&content) {
      Ok(data) => data,
      Err(_) => return true,
    };

    !Self::is_cache_valid(cached_data.timestamp)
  }

  pub fn save_cached_versions(
    &self,
    browser: &str,
    releases: &[BrowserRelease],
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = Self::get_cache_dir()?;
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    let cached_data = CachedVersionData {
      releases: releases.to_vec(),
      timestamp: Self::get_current_timestamp(),
    };

    let content = serde_json::to_string_pretty(&cached_data)?;
    fs::write(&cache_file, content)?;
    log::info!("Cached {} versions for {}", releases.len(), browser);
    Ok(())
  }

  fn load_cached_wayfern_version(&self) -> Option<WayfernVersionInfo> {
    let cache_dir = Self::get_cache_dir().ok()?;
    let cache_file = cache_dir.join("wayfern_version.json");

    if !cache_file.exists() {
      return None;
    }

    let content = fs::read_to_string(&cache_file).ok()?;
    let cached_data: CachedWayfernData = serde_json::from_str(&content).ok()?;

    Some(cached_data.version_info)
  }

  fn save_cached_wayfern_version(
    &self,
    version_info: &WayfernVersionInfo,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = Self::get_cache_dir()?;
    let cache_file = cache_dir.join("wayfern_version.json");

    let cached_data = CachedWayfernData {
      version_info: version_info.clone(),
      timestamp: Self::get_current_timestamp(),
    };

    let content = serde_json::to_string_pretty(&cached_data)?;
    fs::write(&cache_file, content)?;
    log::info!("Cached Wayfern version: {}", version_info.version);
    Ok(())
  }

  /// Fetch Wayfern version info from https://donutbrowser.com/wayfern.json
  pub async fn fetch_wayfern_version_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<WayfernVersionInfo, Box<dyn std::error::Error + Send + Sync>> {
    if !no_caching {
      if let Some(cached_version) = self.load_cached_wayfern_version() {
        log::info!("Using cached Wayfern version: {}", cached_version.version);
        return Ok(cached_version);
      }
    }

    log::info!("Fetching Wayfern version from https://donutbrowser.com/wayfern.json");
    let url = "https://donutbrowser.com/wayfern.json";

    let mut last_err = None;
    let mut version_info: Option<WayfernVersionInfo> = None;

    for attempt in 1..=3 {
      match self
        .client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
        .send()
        .await
      {
        Ok(response) => {
          if !response.status().is_success() {
            last_err = Some(format!("HTTP {}", response.status()));
          } else {
            match response.json::<WayfernVersionInfo>().await {
              Ok(info) => {
                version_info = Some(info);
                break;
              }
              Err(e) => last_err = Some(format!("Failed to parse response: {e}")),
            }
          }
        }
        Err(e) => {
          log::warn!("Wayfern fetch attempt {attempt}/3 failed: {e}");
          last_err = Some(e.to_string());
        }
      }

      if attempt < 3 {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
      }
    }

    let version_info = version_info.ok_or_else(|| {
      format!(
        "Failed to fetch Wayfern version after 3 attempts: {}",
        last_err.unwrap_or_default()
      )
    })?;
    log::info!("Fetched Wayfern version: {}", version_info.version);

    if !no_caching {
      if let Err(e) = self.save_cached_wayfern_version(&version_info) {
        log::error!("Failed to cache Wayfern version: {e}");
      }
    }

    Ok(version_info)
  }

  /// Get the download URL for Wayfern based on current platform
  pub fn get_wayfern_download_url(&self, version_info: &WayfernVersionInfo) -> Option<String> {
    let (os, arch) = Self::get_platform_info();
    let platform_key = format!("{os}-{arch}");

    version_info
      .downloads
      .get(&platform_key)
      .and_then(|url| url.clone())
  }

  /// Check if Wayfern has a compatible download for current platform
  pub fn has_wayfern_compatible_download(&self, version_info: &WayfernVersionInfo) -> bool {
    self.get_wayfern_download_url(version_info).is_some()
  }

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

  pub fn clear_all_cache(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = Self::get_cache_dir()?;

    if cache_dir.exists() {
      for entry in fs::read_dir(&cache_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
          fs::remove_file(&path)?;
          log::info!("Removed cache file: {path:?}");
        }
      }
      log::info!("All version cache cleared successfully");
    }

    Ok(())
  }
}

lazy_static::lazy_static! {
  static ref API_CLIENT: ApiClient = ApiClient::new();
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_version_parsing() {
    let v1 = VersionComponent::parse("1.2.3");
    assert_eq!(v1.major, 1);
    assert_eq!(v1.minor, 2);
    assert_eq!(v1.patch, 3);

    let v2 = VersionComponent::parse("138.0.7204.50");
    assert_eq!(v2.major, 138);
    assert_eq!(v2.minor, 0);
    assert_eq!(v2.patch, 7204);
    assert_eq!(v2.build, 50);

    let v3 = VersionComponent::parse("137.0b5");
    assert_eq!(v3.major, 137);
    assert_eq!(v3.minor, 0);
    assert_eq!(v3.patch, 0);
  }

  #[test]
  fn test_version_comparison() {
    assert!(VersionComponent::parse("1.2.4") > VersionComponent::parse("1.2.3"));
    assert!(VersionComponent::parse("2.0.0") > VersionComponent::parse("1.9.9"));
    assert!(VersionComponent::parse("138.0.7204.50") > VersionComponent::parse("138.0.7204.49"));
  }

  #[test]
  fn test_version_sorting() {
    let mut versions = vec![
      "138.0.7204.50".to_string(),
      "138.0.7204.49".to_string(),
      "139.0.7204.1".to_string(),
      "137.0.7204.99".to_string(),
    ];

    sort_versions(&mut versions);

    assert_eq!(versions[0], "139.0.7204.1");
    assert_eq!(versions[1], "138.0.7204.50");
    assert_eq!(versions[2], "138.0.7204.49");
    assert_eq!(versions[3], "137.0.7204.99");
  }
}

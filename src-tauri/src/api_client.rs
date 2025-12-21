use directories::BaseDirs;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::browser::GithubRelease;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionComponent {
  pub major: u32,
  pub minor: u32,
  pub patch: u32,
  pub pre_release: Option<PreRelease>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreRelease {
  pub kind: PreReleaseKind,
  pub number: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PreReleaseKind {
  Alpha,
  Beta,
  RC,
  Dev,
  Pre,
}

impl VersionComponent {
  pub fn parse(version: &str) -> Self {
    let version = version.trim();
    // Normalize common tag prefixes like 'v1.2.3' -> '1.2.3'
    let version = if version.starts_with('v') || version.starts_with('V') {
      &version[1..]
    } else {
      version
    };

    // Handle special case for Zen Browser twilight releases
    if version.to_lowercase() == "twilight" {
      // Pure twilight release without base version
      return VersionComponent {
        major: 999, // High major version to indicate it's a rolling release
        minor: 0,
        patch: 0,
        pre_release: Some(PreRelease {
          kind: PreReleaseKind::Alpha,
          number: Some(999), // High number to indicate it's a rolling release
        }),
      };
    }

    // Split version into numeric and pre-release parts
    let (numeric_part, pre_release_part) = Self::split_version(version);

    // Parse numeric parts (major.minor.patch)
    let parts: Vec<u32> = numeric_part
      .split('.')
      .filter_map(|part| part.parse().ok())
      .collect();

    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);
    let patch = parts.get(2).copied().unwrap_or(0);

    // Parse pre-release part
    let pre_release = pre_release_part
      .as_deref()
      .and_then(Self::parse_pre_release);

    VersionComponent {
      major,
      minor,
      patch,
      pre_release,
    }
  }

  fn split_version(version: &str) -> (String, Option<String>) {
    let version = version.to_lowercase();

    // Look for pre-release indicators
    for (i, ch) in version.char_indices() {
      if ch.is_alphabetic() && i > 0 {
        // Check if this is a pre-release indicator
        let remaining = &version[i..];
        if remaining.starts_with('a')
          || remaining.starts_with('b')
          || remaining.starts_with("alpha")
          || remaining.starts_with("beta")
          || remaining.starts_with("rc")
          || remaining.starts_with("dev")
          || remaining.starts_with("pre")
        {
          return (version[..i].to_string(), Some(remaining.to_string()));
        }
      }
    }

    (version, None)
  }

  fn parse_pre_release(pre_release: &str) -> Option<PreRelease> {
    let pre_release = pre_release.trim().to_lowercase();

    if pre_release.is_empty() {
      return None;
    }

    // Extract kind and number
    let (kind, number) = if let Some(stripped) = pre_release.strip_prefix("alpha") {
      (PreReleaseKind::Alpha, Self::extract_number(stripped))
    } else if let Some(stripped) = pre_release.strip_prefix("beta") {
      (PreReleaseKind::Beta, Self::extract_number(stripped))
    } else if let Some(stripped) = pre_release.strip_prefix("rc") {
      (PreReleaseKind::RC, Self::extract_number(stripped))
    } else if let Some(stripped) = pre_release.strip_prefix("dev") {
      (PreReleaseKind::Dev, Self::extract_number(stripped))
    } else if let Some(stripped) = pre_release.strip_prefix("pre") {
      (PreReleaseKind::Pre, Self::extract_number(stripped))
    } else if let Some(stripped) = pre_release.strip_prefix('a') {
      (PreReleaseKind::Alpha, Self::extract_number(stripped))
    } else if let Some(stripped) = pre_release.strip_prefix('b') {
      (PreReleaseKind::Beta, Self::extract_number(stripped))
    } else {
      return None;
    };

    Some(PreRelease { kind, number })
  }

  fn extract_number(s: &str) -> Option<u32> {
    let numeric_part: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    numeric_part.parse().ok()
  }
}

impl PartialOrd for VersionComponent {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for VersionComponent {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    // Check for twilight versions
    let self_is_twilight = self
      .pre_release
      .as_ref()
      .map(|pr| pr.kind == PreReleaseKind::Alpha && pr.number == Some(999))
      .unwrap_or(false);
    let other_is_twilight = other
      .pre_release
      .as_ref()
      .map(|pr| pr.kind == PreReleaseKind::Alpha && pr.number == Some(999))
      .unwrap_or(false);

    // If one is twilight and the other isn't, twilight always has priority
    if self_is_twilight && !other_is_twilight {
      return Ordering::Greater; // twilight > non-twilight
    }
    if !self_is_twilight && other_is_twilight {
      return Ordering::Less; // non-twilight < twilight
    }

    // Both are twilight or both are not twilight - use normal comparison
    match (self_is_twilight, other_is_twilight) {
      (true, true) => {
        // Both are twilight, compare by base version
        return (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch));
      }
      (false, false) => {
        // Neither is twilight, continue with normal comparison
      }
      _ => unreachable!(), // Already handled above
    }

    // Compare major.minor.patch first
    match (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)) {
      Ordering::Equal => {
        // If numeric parts are equal, compare pre-release
        match (&self.pre_release, &other.pre_release) {
          (None, None) => Ordering::Equal,
          (None, Some(_)) => Ordering::Greater, // Stable > pre-release
          (Some(_), None) => Ordering::Less,    // Pre-release < stable
          (Some(a), Some(b)) => {
            // Compare pre-release kinds first
            match a.kind.cmp(&b.kind) {
              Ordering::Equal => {
                // Same kind, compare numbers
                match (&a.number, &b.number) {
                  (None, None) => Ordering::Equal,
                  (None, Some(_)) => Ordering::Less,
                  (Some(_), None) => Ordering::Greater,
                  (Some(a_num), Some(b_num)) => a_num.cmp(b_num),
                }
              }
              other => other,
            }
          }
        }
      }
      other => other,
    }
  }
}

// Helper function to sort versions properly
pub fn sort_versions(versions: &mut [String]) {
  versions.sort_by(|a, b| {
    let version_a = VersionComponent::parse(a);
    let version_b = VersionComponent::parse(b);
    version_b.cmp(&version_a) // Descending order (newest first)
  });
}

// Helper function to compare two versions
pub fn compare_versions(version1: &str, version2: &str) -> std::cmp::Ordering {
  let version_a = VersionComponent::parse(version1);
  let version_b = VersionComponent::parse(version2);
  version_a.cmp(&version_b)
}

pub fn is_version_newer(version1: &str, version2: &str) -> bool {
  // Use the proper VersionComponent comparison from api_client.rs
  let version_a = VersionComponent::parse(version1);
  let version_b = VersionComponent::parse(version2);
  version_a > version_b
}

// Helper function to sort GitHub releases
pub fn sort_github_releases(releases: &mut [GithubRelease]) {
  releases.sort_by(|a, b| {
    // Normalize tags like "v1.81.9" -> "1.81.9" for correct ordering
    let tag_a = a.tag_name.trim_start_matches('v');
    let tag_b = b.tag_name.trim_start_matches('v');
    let version_a = VersionComponent::parse(tag_a);
    let version_b = VersionComponent::parse(tag_b);
    version_b.cmp(&version_a) // Descending order (newest first)
  });
}

pub fn is_nightly_version(version: &str) -> bool {
  let version_comp = VersionComponent::parse(version);
  version_comp.pre_release.is_some()
}

/// Centralized function to determine if a browser version/release is nightly/prerelease
/// This is the single source of truth for nightly detection across the entire codebase
pub fn is_browser_version_nightly(
  browser: &str,
  version: &str,
  release_name: Option<&str>,
) -> bool {
  match browser {
    "zen" => {
      // For Zen Browser, only "twilight" is considered nightly
      version.to_lowercase() == "twilight"
    }
    "brave" => {
      // For Brave Browser, only releases whose name starts with "Release" (case-insensitive) are stable.
      if let Some(name) = release_name {
        let normalized = name.trim_start().to_ascii_lowercase();
        return !normalized.starts_with("release");
      }

      // Fallback: try cached GitHub releases
      if let Some(releases) = ApiClient::instance().get_cached_github_releases("brave") {
        if let Some(found) = releases.iter().find(|r| r.tag_name == version) {
          let normalized = found.name.trim_start().to_ascii_lowercase();
          return !normalized.starts_with("release");
        }
      }

      // Last resort: when no name available, treat as nightly (non-Release)
      true
    }
    "firefox-developer" => {
      // For Firefox Developer Edition, always treat as nightly/prerelease
      // This ensures consistent behavior regardless of cache state or API response parsing
      true
    }
    "firefox" => {
      // For Firefox, use the category from the API response to determine stability
      // This will be handled in the API parsing, so this fallback is for cached versions
      is_nightly_version(version)
    }
    "chromium" => {
      // Chromium builds are generally stable snapshots
      false
    }
    "camoufox" => {
      // For Camoufox, beta versions are actually the stable releases
      false
    }
    _ => {
      // Default fallback
      is_nightly_version(version)
    }
  }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FirefoxRelease {
  pub build_number: u32,
  pub category: String,
  pub date: String,
  pub description: Option<String>,
  pub is_security_driven: bool,
  pub product: String,
  pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FirefoxApiResponse {
  pub releases: HashMap<String, FirefoxRelease>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BrowserRelease {
  pub version: String,
  pub date: String,
  pub is_prerelease: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedVersionData {
  releases: Vec<BrowserRelease>,
  timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedGithubData {
  releases: Vec<GithubRelease>,
  timestamp: u64,
}

pub struct ApiClient {
  client: Client,
  firefox_api_base: String,
  firefox_dev_api_base: String,
  github_api_base: String,
  chromium_api_base: String,
}

impl ApiClient {
  pub fn new() -> Self {
    let client = Client::builder()
      .timeout(std::time::Duration::from_secs(30))
      .build()
      .unwrap_or_else(|_| Client::new());

    Self {
      client,
      firefox_api_base: "https://product-details.mozilla.org/1.0".to_string(),
      firefox_dev_api_base: "https://product-details.mozilla.org/1.0".to_string(),
      github_api_base: "https://api.github.com".to_string(),
      chromium_api_base: "https://commondatastorage.googleapis.com/chromium-browser-snapshots"
        .to_string(),
    }
  }

  async fn fetch_github_releases_multiple_pages(
    &self,
    base_releases_url: &str,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    let mut all_releases: Vec<GithubRelease> = Vec::new();

    // For now, only fetch 1 page
    for page in 1..=1 {
      let url = format!("{base_releases_url}?per_page=100&page={page}");
      let response = self
        .client
        .get(&url)
        .header(
          "User-Agent",
          "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        )
        .send()
        .await?;

      if !response.status().is_success() {
        // If the first page fails, propagate error; otherwise stop pagination
        if page == 1 {
          return Err(
            format!(
              "GitHub API returned status for page {}: {}",
              page,
              response.status()
            )
            .into(),
          );
        } else {
          break;
        }
      }

      let text = response.text().await?;
      let mut page_releases: Vec<GithubRelease> = serde_json::from_str(&text).map_err(|e| {
        log::error!("Failed to parse GitHub API response (page {page}): {e}");
        log::error!(
          "Response text (first 500 chars): {}",
          if text.len() > 500 {
            &text[..500]
          } else {
            &text
          }
        );
        format!("Failed to parse GitHub API response: {e}")
      })?;

      if page_releases.is_empty() {
        break;
      }

      all_releases.append(&mut page_releases);
    }

    Ok(all_releases)
  }

  pub fn instance() -> &'static ApiClient {
    &API_CLIENT
  }

  #[cfg(test)]
  pub fn new_with_base_urls(
    firefox_api_base: String,
    firefox_dev_api_base: String,
    github_api_base: String,
    chromium_api_base: String,
  ) -> Self {
    Self {
      client: Client::new(),
      firefox_api_base,
      firefox_dev_api_base,
      github_api_base,
      chromium_api_base,
    }
  }

  fn get_cache_dir() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let base_dirs = BaseDirs::new().ok_or("Failed to get base directories")?;
    let app_name = if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    };
    let cache_dir = base_dirs.cache_dir().join(app_name).join("version_cache");
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
    let cache_duration = 10 * 60; // 10 minutes in seconds
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
      // Always return cached releases regardless of age - they're always valid
      log::info!("Using cached versions for {browser}");
      return Some(cached.releases);
    }

    // Backward compatibility: legacy caches stored just an array of version strings
    if let Ok(legacy_versions) = serde_json::from_str::<Vec<String>>(&content) {
      log::info!("Using legacy cached versions for {browser}; upgrading in-memory");
      let releases: Vec<BrowserRelease> = legacy_versions
        .into_iter()
        .map(|version| BrowserRelease {
          is_prerelease: is_browser_version_nightly(browser, &version, None),
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
      Err(_) => return true, // If we can't get cache dir, consider expired
    };
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    if !cache_file.exists() {
      return true; // No cache file means expired
    }

    let content = match fs::read_to_string(&cache_file) {
      Ok(content) => content,
      Err(_) => return true, // Can't read cache, consider expired
    };

    let cached_data: CachedVersionData = match serde_json::from_str(&content) {
      Ok(data) => data,
      Err(_) => return true, // Can't parse cache, consider expired
    };

    // Check if cache is older than 10 minutes
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

  fn load_cached_github_releases(&self, browser: &str) -> Option<Vec<GithubRelease>> {
    let cache_dir = Self::get_cache_dir().ok()?;
    let cache_file = cache_dir.join(format!("{browser}_github.json"));

    if !cache_file.exists() {
      return None;
    }

    let content = fs::read_to_string(&cache_file).ok()?;
    let cached_data: CachedGithubData = serde_json::from_str(&content).ok()?;

    // Always use cached GitHub releases - cache never expires, only gets updated with new versions
    Some(cached_data.releases)
  }

  /// Public accessor for cached GitHub releases (used by other modules for classification)
  pub fn get_cached_github_releases(&self, browser: &str) -> Option<Vec<GithubRelease>> {
    self.load_cached_github_releases(browser)
  }

  fn save_cached_github_releases(
    &self,
    browser: &str,
    releases: &[GithubRelease],
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = Self::get_cache_dir()?;
    let cache_file = cache_dir.join(format!("{browser}_github.json"));

    let cached_data = CachedGithubData {
      releases: releases.to_vec(),
      timestamp: Self::get_current_timestamp(),
    };

    let content = serde_json::to_string_pretty(&cached_data)?;
    fs::write(&cache_file, content)?;
    log::info!("Cached {} GitHub releases for {}", releases.len(), browser);
    Ok(())
  }

  pub async fn fetch_firefox_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_releases) = self.load_cached_versions("firefox") {
        return Ok(cached_releases);
      }
    }

    log::info!("Fetching Firefox releases from Mozilla API...");
    let url = format!("{}/firefox.json", self.firefox_api_base);

    let response = self
      .client
      .get(url)
      .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(format!("Failed to fetch Firefox versions: {}", response.status()).into());
    }

    let firefox_response: FirefoxApiResponse = response.json().await?;

    // Extract releases and filter for stable versions
    let mut releases: Vec<BrowserRelease> = firefox_response
      .releases
      .into_iter()
      .filter_map(|(key, release)| {
        // Only include releases that start with "firefox-" and have proper version format
        if key.starts_with("firefox-") && !release.version.is_empty() {
          let is_stable = matches!(release.category.as_str(), "major" | "stability");
          Some(BrowserRelease {
            version: release.version.clone(),
            date: release.date,
            is_prerelease: !is_stable,
          })
        } else {
          None
        }
      })
      .collect();

    // Sort by version number in descending order (newest first)
    releases.sort_by(|a, b| {
      let version_a = VersionComponent::parse(&a.version);
      let version_b = VersionComponent::parse(&b.version);
      version_b.cmp(&version_a)
    });

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_versions("firefox", &releases) {
        log::error!("Failed to cache Firefox versions: {e}");
      }
    }

    Ok(releases)
  }

  pub async fn fetch_firefox_developer_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_releases) = self.load_cached_versions("firefox-developer") {
        return Ok(cached_releases);
      }
    }

    log::info!("Fetching Firefox Developer Edition releases from Mozilla API...");
    let url = format!("{}/devedition.json", self.firefox_dev_api_base);

    let response = self
      .client
      .get(&url)
      .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
      .send()
      .await?;

    if !response.status().is_success() {
      let error_msg = format!(
        "Failed to fetch Firefox Developer Edition versions: {} - URL: {}",
        response.status(),
        url
      );
      log::error!("{error_msg}");
      return Err(error_msg.into());
    }

    let firefox_response: FirefoxApiResponse = response.json().await?;

    // Extract releases and filter for developer edition versions
    let mut releases: Vec<BrowserRelease> = firefox_response
      .releases
      .into_iter()
      .filter_map(|(key, release)| {
        // Only include releases that start with "devedition-" and have proper version format
        if key.starts_with("devedition-") && !release.version.is_empty() {
          let is_stable = matches!(release.category.as_str(), "major" | "stability");
          Some(BrowserRelease {
            version: release.version.clone(),
            date: release.date,
            is_prerelease: !is_stable,
          })
        } else {
          None
        }
      })
      .collect();

    // Sort by version number in descending order (newest first)
    releases.sort_by(|a, b| {
      let version_a = VersionComponent::parse(&a.version);
      let version_b = VersionComponent::parse(&b.version);
      version_b.cmp(&version_a)
    });

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_versions("firefox-developer", &releases) {
        log::error!("Failed to cache Firefox Developer versions: {e}");
      }
    }

    Ok(releases)
  }

  pub async fn fetch_zen_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_releases) = self.load_cached_github_releases("zen") {
        return Ok(cached_releases);
      }
    }

    log::info!("Fetching Zen releases from GitHub API");
    let base_url = format!(
      "{}/repos/zen-browser/desktop/releases",
      self.github_api_base
    );
    let mut releases: Vec<GithubRelease> =
      self.fetch_github_releases_multiple_pages(&base_url).await?;

    // Check for twilight updates and mark alpha releases
    for release in &mut releases {
      // Use browser-specific alpha detection for Zen Browser - only "twilight" is nightly
      release.is_nightly =
        is_browser_version_nightly("zen", &release.tag_name, Some(&release.name));

      // Check for twilight update if this is a twilight release
      if release.tag_name.to_lowercase() == "twilight" {
        if let Ok(has_update) = self.check_twilight_update(release).await {
          if has_update {
            log::info!(
              "Detected update for Zen twilight release: {}",
              release.tag_name
            );
          }
        }
      }
    }

    // Sort releases using the new version sorting system
    sort_github_releases(&mut releases);

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_github_releases("zen", &releases) {
        log::error!("Failed to cache Zen releases: {e}");
      }
    }

    Ok(releases)
  }

  pub async fn fetch_brave_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_releases) = self.load_cached_github_releases("brave") {
        return Ok(cached_releases);
      }
    }

    log::info!("Fetching Brave releases from GitHub API");
    let base_url = format!(
      "{}/repos/brave/brave-browser/releases",
      self.github_api_base
    );
    let releases: Vec<GithubRelease> = self.fetch_github_releases_multiple_pages(&base_url).await?;

    // Get platform info to filter appropriate releases
    let (os, _) = Self::get_platform_info();

    // Filter releases that have assets compatible with the current platform
    let mut filtered_releases: Vec<GithubRelease> = releases
      .into_iter()
      .filter_map(|mut release| {
        // Check if this release has compatible assets for the current platform
        let has_compatible_asset = Self::has_compatible_brave_asset(&release.assets, &os);

        if has_compatible_asset {
          // Use the centralized nightly detection function
          release.is_nightly =
            is_browser_version_nightly("brave", &release.tag_name, Some(&release.name));
          Some(release)
        } else {
          None
        }
      })
      .collect();

    // Sort releases using the new version sorting system
    sort_github_releases(&mut filtered_releases);

    if let Err(e) = self.save_cached_github_releases("brave", &filtered_releases) {
      log::error!("Failed to cache Brave releases: {e}");
    }

    Ok(filtered_releases)
  }

  /// Check if a Brave release has compatible assets for the given platform and architecture
  fn has_compatible_camoufox_asset(
    &self,
    assets: &[crate::browser::GithubAsset],
    os: &str,
    arch: &str,
  ) -> bool {
    let (os_name, arch_name) = match (os, arch) {
      ("windows", "x64") => ("win", "x86_64"),
      ("windows", "arm64") => ("win", "arm64"),
      ("linux", "x64") => ("lin", "x86_64"),
      ("linux", "arm64") => ("lin", "arm64"),
      ("macos", "x64") => ("mac", "x86_64"),
      ("macos", "arm64") => ("mac", "arm64"),
      _ => return false,
    };

    // Look for assets matching the pattern: camoufox-{version}-{release}-{os}.{arch}.zip
    // Use ends_with for precise matching to avoid false positives
    let pattern = format!(".{os_name}.{arch_name}.zip");
    assets.iter().any(|asset| {
      let name = asset.name.to_lowercase();
      name.starts_with("camoufox-") && name.ends_with(&pattern)
    })
  }

  fn has_compatible_brave_asset(assets: &[crate::browser::GithubAsset], os: &str) -> bool {
    match os {
      "windows" => {
        // For Windows, look for standalone setup EXE (not the auto-updater one)
        assets.iter().any(|asset| {
          let name = asset.name.to_lowercase();
          name.contains("standalone") && name.ends_with(".exe") && !name.contains("silent")
        }) || assets.iter().any(|asset| asset.name.ends_with(".exe"))
      }
      "macos" => {
        // For macOS, prefer universal DMG
        assets.iter().any(|asset| {
          let name = asset.name.to_lowercase();
          name.contains("universal") && name.ends_with(".dmg")
        }) || assets.iter().any(|asset| asset.name.ends_with(".dmg"))
      }
      "linux" => {
        if assets.iter().any(|asset| {
          let name = asset.name.to_lowercase();
          name.contains("lin")
        }) {
          return true;
        }

        false
      }
      _ => false,
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

  pub async fn fetch_chromium_latest_version(
    &self,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Use platform-aware URL for Chromium to match download URL generation
    let (os, arch) = Self::get_platform_info();
    let platform_str = match (&os[..], &arch[..]) {
      ("windows", "x64") => "Win_x64",
      ("windows", "arm64") => "Win_Arm64",
      ("linux", "x64") => "Linux_x64",
      ("linux", "arm64") => return Err("Chromium doesn't support ARM64 on Linux".into()),
      ("macos", "x64") => "Mac",
      ("macos", "arm64") => "Mac_Arm",
      _ => {
        return Err(format!("Unsupported platform/architecture for Chromium: {os}/{arch}").into())
      }
    };
    let url = format!("{}/{platform_str}/LAST_CHANGE", self.chromium_api_base);
    let version = self
      .client
      .get(&url)
      .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36")
      .send()
      .await?
      .text()
      .await?
      .trim()
      .to_string();

    Ok(version)
  }

  pub async fn fetch_chromium_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_releases) = self.load_cached_versions("chromium") {
        return Ok(cached_releases);
      }
    }

    log::info!("Fetching Chromium releases...");

    // Get the latest version first
    let latest_version = self.fetch_chromium_latest_version().await?;
    let latest_num: u32 = latest_version.parse().unwrap_or(0);

    // Generate a list of recent versions (last 20 builds, going back by 1000 each time)
    let mut versions = Vec::new();
    for i in 0..20 {
      let version_num = latest_num.saturating_sub(i * 1000);
      if version_num > 0 {
        versions.push(version_num.to_string());
      }
    }

    // Convert to BrowserRelease objects
    let releases: Vec<BrowserRelease> = versions
      .into_iter()
      .map(|version| BrowserRelease {
        version: version.clone(),
        date: "".to_string(),
        is_prerelease: false,
      })
      .collect();

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_versions("chromium", &releases) {
        log::error!("Failed to cache Chromium versions: {e}");
      }
    }

    Ok(releases)
  }

  pub async fn fetch_camoufox_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_releases) = self.load_cached_github_releases("camoufox") {
        log::info!(
          "Using cached Camoufox releases, count: {}",
          cached_releases.len()
        );
        return Ok(cached_releases);
      }
    }

    log::info!("Fetching Camoufox releases from GitHub API");
    let base_url = format!("{}/repos/daijro/camoufox/releases", self.github_api_base);
    let releases: Vec<GithubRelease> = self.fetch_github_releases_multiple_pages(&base_url).await?;

    log::info!(
      "Fetched {} total Camoufox releases from GitHub",
      releases.len()
    );

    // Get platform info to filter appropriate releases
    let (os, arch) = Self::get_platform_info();
    log::info!("Filtering for platform: {os}/{arch}");

    // Filter releases that have assets compatible with the current platform
    let mut compatible_releases: Vec<GithubRelease> = releases
      .into_iter()
      .enumerate()
      .filter_map(|(i, release)| {
        let has_compatible = self.has_compatible_camoufox_asset(&release.assets, &os, &arch);
        if !has_compatible {
          log::info!(
            "Release {} ({}) has no compatible assets for {}/{}",
            i,
            release.tag_name,
            os,
            arch
          );
          log::info!(
            "  Available assets: {:?}",
            release.assets.iter().map(|a| &a.name).collect::<Vec<_>>()
          );
        }
        if has_compatible {
          Some(release)
        } else {
          None
        }
      })
      .collect();

    log::info!(
      "After platform filtering: {} compatible releases",
      compatible_releases.len()
    );

    // Sort by version (latest first) with debugging
    log::info!(
      "Before sorting: {:?}",
      compatible_releases
        .iter()
        .map(|r| &r.tag_name)
        .take(10)
        .collect::<Vec<_>>()
    );
    sort_github_releases(&mut compatible_releases);
    log::info!(
      "After sorting: {:?}",
      compatible_releases
        .iter()
        .map(|r| &r.tag_name)
        .take(10)
        .collect::<Vec<_>>()
    );

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_github_releases("camoufox", &compatible_releases) {
        log::error!("Failed to cache Camoufox releases: {e}");
      } else {
        log::info!("Cached {} Camoufox releases", compatible_releases.len());
      }
    }

    Ok(compatible_releases)
  }

  /// Check if a Zen twilight release has been updated by comparing file size
  pub async fn check_twilight_update(
    &self,
    release: &GithubRelease,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if release.tag_name.to_lowercase() != "twilight" {
      return Ok(false); // Not a twilight release
    }

    // Find the macOS universal DMG asset
    let asset = release
      .assets
      .iter()
      .find(|asset| asset.name == "zen.macos-universal.dmg")
      .ok_or("No macOS universal asset found for twilight release")?;

    // Check if we have cached file size information
    let cache_dir = Self::get_cache_dir()?;
    let twilight_cache_file = cache_dir.join("zen_twilight_info.json");

    #[derive(serde::Serialize, serde::Deserialize)]
    struct TwilightInfo {
      file_size: u64,
      last_updated: u64,
    }

    let current_info = TwilightInfo {
      file_size: asset.size,
      last_updated: Self::get_current_timestamp(),
    };

    if !twilight_cache_file.exists() {
      // No cache exists, save current info and return true (new)
      let content = serde_json::to_string_pretty(&current_info)?;
      fs::write(&twilight_cache_file, content)?;
      return Ok(true);
    }

    let cached_content = fs::read_to_string(&twilight_cache_file)?;
    let cached_info: TwilightInfo = serde_json::from_str(&cached_content)?;

    // Check if file size has changed
    if cached_info.file_size != current_info.file_size {
      // File size changed, update cache and return true
      let content = serde_json::to_string_pretty(&current_info)?;
      fs::write(&twilight_cache_file, content)?;
      log::info!(
        "Zen twilight release updated: file size changed from {} to {}",
        cached_info.file_size,
        current_info.file_size
      );
      return Ok(true);
    }

    Ok(false) // No update detected
  }

  pub fn clear_all_cache(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache_dir = Self::get_cache_dir()?;

    if cache_dir.exists() {
      // Remove all cache files
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

// Global singleton instance
lazy_static::lazy_static! {
  static ref API_CLIENT: ApiClient = ApiClient::new();
}

#[cfg(test)]
mod tests {
  use super::*;
  use wiremock::matchers::{method, path, query_param};
  use wiremock::{Mock, MockServer, ResponseTemplate};

  async fn setup_mock_server() -> MockServer {
    MockServer::start().await
  }

  fn create_test_client(server: &MockServer) -> ApiClient {
    let base_url = server.uri();
    ApiClient::new_with_base_urls(
      base_url.clone(), // firefox_api_base
      base_url.clone(), // firefox_dev_api_base
      base_url.clone(), // github_api_base
      base_url.clone(), // chromium_api_base
    )
  }

  #[test]
  fn test_version_parsing() {
    // Test basic version parsing
    let v1 = VersionComponent::parse("1.2.3");
    assert_eq!(v1.major, 1);
    assert_eq!(v1.minor, 2);
    assert_eq!(v1.patch, 3);
    assert!(v1.pre_release.is_none());

    // Test alpha version
    let v2 = VersionComponent::parse("1.2.3a1");
    assert_eq!(v2.major, 1);
    assert_eq!(v2.minor, 2);
    assert_eq!(v2.patch, 3);
    assert!(v2.pre_release.is_some());
    let pre = v2.pre_release.unwrap();
    assert_eq!(pre.kind, PreReleaseKind::Alpha);
    assert_eq!(pre.number, Some(1));

    // Test beta version
    let v3 = VersionComponent::parse("137.0b5");
    assert_eq!(v3.major, 137);
    assert_eq!(v3.minor, 0);
    assert_eq!(v3.patch, 0);
    assert!(v3.pre_release.is_some());
    let pre = v3.pre_release.unwrap();
    assert_eq!(pre.kind, PreReleaseKind::Beta);
    assert_eq!(pre.number, Some(5));

    // Test twilight version (Zen Browser)
    let v4 = VersionComponent::parse("twilight");
    assert_eq!(v4.major, 999);
    assert_eq!(v4.minor, 0);
    assert_eq!(v4.patch, 0);
    assert!(v4.pre_release.is_some());
    let pre = v4.pre_release.unwrap();
    assert_eq!(pre.kind, PreReleaseKind::Alpha);
    assert_eq!(pre.number, Some(999));
  }

  #[test]
  fn test_version_comparison() {
    // Test basic version comparison
    let v1 = VersionComponent::parse("1.2.3");
    let v2 = VersionComponent::parse("1.2.4");
    assert!(v2 > v1);

    // Test major version difference
    let v3 = VersionComponent::parse("2.0.0");
    let v4 = VersionComponent::parse("1.9.9");
    assert!(v3 > v4);

    // Test stable vs pre-release
    let v5 = VersionComponent::parse("1.2.3");
    let v6 = VersionComponent::parse("1.2.3b1");
    assert!(v5 > v6); // Stable > beta

    // Test different pre-release types
    let v7 = VersionComponent::parse("1.2.3a1");
    let v8 = VersionComponent::parse("1.2.3b1");
    assert!(v8 > v7); // Beta > alpha

    // Test pre-release numbers
    let v9 = VersionComponent::parse("137.0b4");
    let v10 = VersionComponent::parse("137.0b5");
    assert!(v10 > v9); // b5 > b4

    // Test twilight version (should have highest priority)
    let v11 = VersionComponent::parse("twilight");
    let v12 = VersionComponent::parse("1.0.0");
    assert!(v11 > v12); // twilight > stable due to high major version

    // Test twilight vs other pre-releases
    let v13 = VersionComponent::parse("twilight");
    let v14 = VersionComponent::parse("1.0.0a1");
    assert!(v13 > v14); // twilight > a1 due to high major version
  }

  #[test]
  fn test_version_sorting() {
    let mut versions = vec![
      "1.9.9b".to_string(),
      "1.12.6b".to_string(),
      "1.10.0".to_string(),
      "137.0b4".to_string(),
      "137.0b5".to_string(),
      "137.0".to_string(),
      "twilight".to_string(),
      "2.0.0a1".to_string(),
    ];

    sort_versions(&mut versions);

    // Expected order with twilight priority: twilight first due to high major version (999), then normal semantic versioning
    assert_eq!(versions[0], "twilight");
    assert_eq!(versions[1], "137.0");
    assert_eq!(versions[2], "137.0b5");
    assert_eq!(versions[3], "137.0b4");
    assert_eq!(versions[4], "2.0.0a1");
    assert_eq!(versions[5], "1.12.6b");
    assert_eq!(versions[6], "1.10.0");
    assert_eq!(versions[7], "1.9.9b");
  }

  #[test]
  fn test_sort_versions_comprehensive() {
    let mut versions = vec![
      "1.0.0".to_string(),
      "1.0.1".to_string(),
      "1.1.0".to_string(),
      "2.0.0a1".to_string(),
      "2.0.0b1".to_string(),
      "2.0.0rc1".to_string(),
      "2.0.0".to_string(),
      "10.0.0".to_string(),
      "twilight".to_string(),
    ];

    sort_versions(&mut versions);

    // Expected order with twilight priority: twilight first due to high major version (999), then normal semantic versioning
    assert_eq!(versions[0], "twilight");
    assert_eq!(versions[1], "10.0.0");
    assert_eq!(versions[2], "2.0.0");
    assert_eq!(versions[3], "2.0.0rc1");
    assert_eq!(versions[4], "2.0.0b1");
    assert_eq!(versions[5], "2.0.0a1");
  }

  #[tokio::test]
  async fn test_firefox_api() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

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
      .mount(&server)
      .await;

    let result = client.fetch_firefox_releases_with_caching(true).await;

    if let Err(e) = &result {
      log::info!("Firefox API test error: {e}");
    }
    assert!(result.is_ok());
    let releases = result.unwrap();
    assert!(!releases.is_empty());
    assert_eq!(releases[0].version, "139.0");
  }

  #[tokio::test]
  async fn test_firefox_developer_api() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

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
      .mount(&server)
      .await;

    let result = client
      .fetch_firefox_developer_releases_with_caching(true)
      .await;

    if let Err(e) = &result {
      log::info!("Firefox Developer API test error: {e}");
    }
    assert!(result.is_ok());
    let releases = result.unwrap();
    assert!(!releases.is_empty());
    assert_eq!(releases[0].version, "140.0b1");
  }

  #[tokio::test]
  async fn test_zen_api() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

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
      .mount(&server)
      .await;

    let result = client.fetch_zen_releases_with_caching(true).await;

    assert!(result.is_ok());
    let releases = result.unwrap();
    assert!(!releases.is_empty());
    assert_eq!(releases[0].tag_name, "twilight");
  }

  #[tokio::test]
  async fn test_brave_api() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

    let mock_response = r#"[
      {
        "tag_name": "v1.81.9",
        "name": "Release v1.81.9 (Chromium 137.0.7151.104)",
        "prerelease": false,
        "published_at": "2024-01-15T10:00:00Z",
        "draft": false,
        "assets": [
          {
            "name": "brave-v1.81.9-universal.dmg",
            "browser_download_url": "https://example.com/brave-1.81.9-universal.dmg",
            "size": 200000000
          },
          {
            "name": "brave-browser-1.81.9-linux-amd64.zip",
            "browser_download_url": "https://example.com/brave-1.81.9-linux-amd64.zip",
            "size": 180000000
          },
          {
            "name": "BraveBrowserStandaloneSetup.exe",
            "browser_download_url": "https://example.com/brave-1.81.9-setup.exe",
            "size": 150000000
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
      .mount(&server)
      .await;

    let result = client.fetch_brave_releases_with_caching(true).await;

    if let Err(e) = &result {
      log::info!("Brave API test error: {e}");
    }
    assert!(result.is_ok());
    let releases = result.unwrap();
    assert!(!releases.is_empty());
    assert_eq!(releases[0].tag_name, "v1.81.9");
    assert!(!releases[0].is_nightly); // "Release v1.81.9 (Chromium 137.0.7151.104)" starts with "Release" so it should be stable
  }

  #[tokio::test]
  async fn test_chromium_api() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

    let (os, arch) = ApiClient::get_platform_info();
    let platform_str = match (&os[..], &arch[..]) {
      ("windows", "x64") => "Win_x64",
      ("windows", "arm64") => "Win_Arm64",
      ("linux", "x64") => "Linux_x64",
      ("linux", "arm64") => return,
      ("macos", "x64") => "Mac",
      ("macos", "arm64") => "Mac_Arm",
      _ => return,
    };

    Mock::given(method("GET"))
      .and(path(format!("/{platform_str}/LAST_CHANGE")))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string("1465660")
          .insert_header("content-type", "text/plain"),
      )
      .mount(&server)
      .await;

    let result = client.fetch_chromium_latest_version().await;

    assert!(result.is_ok());
    let version = result.unwrap();
    assert_eq!(version, "1465660");
  }

  #[tokio::test]
  async fn test_chromium_releases_with_caching() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

    let (os, arch) = ApiClient::get_platform_info();
    let platform_str = match (&os[..], &arch[..]) {
      ("windows", "x64") => "Win_x64",
      ("windows", "arm64") => "Win_Arm64",
      ("linux", "x64") => "Linux_x64",
      ("linux", "arm64") => return,
      ("macos", "x64") => "Mac",
      ("macos", "arm64") => "Mac_Arm",
      _ => return,
    };

    Mock::given(method("GET"))
      .and(path(format!("/{platform_str}/LAST_CHANGE")))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string("1465660")
          .insert_header("content-type", "text/plain"),
      )
      .mount(&server)
      .await;

    let result = client.fetch_chromium_releases_with_caching(true).await;

    assert!(result.is_ok());
    let releases = result.unwrap();
    assert!(!releases.is_empty());
    assert_eq!(releases[0].version, "1465660");
    assert!(!releases[0].is_prerelease);
  }

  #[test]
  fn test_is_nightly_version() {
    assert!(is_nightly_version("1.2.3a1"));
    assert!(is_nightly_version("137.0b5"));
    assert!(is_nightly_version("140.0rc1"));
    assert!(!is_nightly_version("139.0"));
    assert!(!is_nightly_version("1.2.3"));
  }

  #[test]
  fn test_is_zen_nightly_version() {
    // Only "twilight" should be considered nightly for Zen Browser
    assert!(is_browser_version_nightly("zen", "twilight", None));
    assert!(is_browser_version_nightly("zen", "TWILIGHT", None)); // Case insensitive

    // Versions with "b" should NOT be considered nightly for Zen Browser
    assert!(!is_browser_version_nightly("zen", "1.12.8b", None));
    assert!(!is_browser_version_nightly("zen", "1.0.0b1", None));
    assert!(!is_browser_version_nightly("zen", "2.0.0", None));
  }

  #[tokio::test]
  async fn test_error_handling_404() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

    Mock::given(method("GET"))
      .and(path("/firefox.json"))
      .respond_with(ResponseTemplate::new(404))
      .mount(&server)
      .await;

    let result = client.fetch_firefox_releases_with_caching(true).await;
    assert!(result.is_err());
  }

  #[tokio::test]
  async fn test_error_handling_invalid_json() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

    Mock::given(method("GET"))
      .and(path("/firefox.json"))
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_string("invalid json")
          .insert_header("content-type", "application/json"),
      )
      .mount(&server)
      .await;

    let result = client.fetch_firefox_releases_with_caching(true).await;
    assert!(result.is_err());
  }

  #[tokio::test]
  async fn test_github_api_rate_limit() {
    let server = setup_mock_server().await;
    let client = create_test_client(&server);

    Mock::given(method("GET"))
      .and(path("/repos/zen-browser/desktop/releases"))
      .and(query_param("per_page", "100"))
      .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "60"))
      .mount(&server)
      .await;

    let result = client.fetch_zen_releases_with_caching(true).await;
    assert!(result.is_err());
  }

  #[test]
  fn test_camoufox_beta_version_parsing() {
    // Test specific Camoufox beta versions that are causing issues
    let v22 = VersionComponent::parse("135.0.5beta22");
    let v24 = VersionComponent::parse("135.0.5beta24");

    log::info!("v22: {v22:?}");
    log::info!("v24: {v24:?}");

    // v24 should be greater than v22
    assert!(
      v24 > v22,
      "135.0.5beta24 should be greater than 135.0.5beta22"
    );

    // Test other beta version combinations
    let v1 = VersionComponent::parse("135.0.5beta1");
    let v2 = VersionComponent::parse("135.0.5beta2");
    assert!(v2 > v1, "135.0.5beta2 should be greater than 135.0.5beta1");

    // Test sorting of multiple versions
    let mut versions = vec![
      "135.0.5beta22".to_string(),
      "135.0.5beta24".to_string(),
      "135.0.5beta23".to_string(),
      "135.0.5beta21".to_string(),
    ];

    sort_versions(&mut versions);

    log::info!("Sorted versions: {versions:?}");

    // Should be sorted from newest to oldest
    assert_eq!(versions[0], "135.0.5beta24");
    assert_eq!(versions[1], "135.0.5beta23");
    assert_eq!(versions[2], "135.0.5beta22");
    assert_eq!(versions[3], "135.0.5beta21");
  }

  #[test]
  fn test_camoufox_user_reported_versions() {
    // Test the exact versions reported by the user: 135.0.1beta24 vs 135.0beta22
    let v22 = VersionComponent::parse("135.0beta22");
    let v24 = VersionComponent::parse("135.0.1beta24");

    log::info!("User reported v22: {v22:?}");
    log::info!("User reported v24: {v24:?}");

    // 135.0.1beta24 should be greater than 135.0beta22 (newer patch version)
    assert!(
      v24 > v22,
      "135.0.1beta24 should be greater than 135.0beta22, but got: v24={v24:?} vs v22={v22:?}"
    );

    // Test sorting of the exact user-reported versions
    let mut versions = vec!["135.0beta22".to_string(), "135.0.1beta24".to_string()];

    sort_versions(&mut versions);

    log::info!("User reported sorted versions: {versions:?}");

    // Should be sorted from newest to oldest
    assert_eq!(
      versions[0], "135.0.1beta24",
      "135.0.1beta24 should be first (newest)"
    );
    assert_eq!(
      versions[1], "135.0beta22",
      "135.0beta22 should be second (older)"
    );
  }

  #[test]
  fn test_camoufox_version_classification() {
    // Test that Camoufox beta versions are now correctly classified as stable (not nightly)
    assert!(
      !is_browser_version_nightly("camoufox", "135.0beta22", None),
      "135.0beta22 should be classified as stable for Camoufox"
    );
    assert!(
      !is_browser_version_nightly("camoufox", "135.0.1beta24", None),
      "135.0.1beta24 should be classified as stable for Camoufox"
    );

    // Test with release names too - beta releases should be stable
    assert!(
      !is_browser_version_nightly("camoufox", "135.0beta22", Some("Release Beta 22")),
      "Release with 'Beta' in name should be classified as stable for Camoufox"
    );

    // Test that stable versions are not classified as nightly
    assert!(
      !is_browser_version_nightly("camoufox", "135.0", None),
      "135.0 should be classified as stable"
    );
    assert!(
      !is_browser_version_nightly("camoufox", "135.0.1", None),
      "135.0.1 should be classified as stable"
    );

    // Test alpha and RC versions are still considered nightly
    assert!(
      !is_browser_version_nightly("camoufox", "136.0alpha1", None),
      "136.0alpha1 should not be classified as nightly/prerelease"
    );
    assert!(
      !is_browser_version_nightly("camoufox", "136.0rc1", None),
      "136.0rc1 should not be classified as nightly/prerelease"
    );
  }
}

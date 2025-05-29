use directories::BaseDirs;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::browser::GithubRelease;

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionComponent {
  major: u32,
  minor: u32,
  patch: u32,
  pre_release: Option<PreRelease>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreRelease {
  kind: PreReleaseKind,
  number: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PreReleaseKind {
  Alpha,
  Beta,
  RC,
  Dev,
  Pre,
}

impl VersionComponent {
  fn parse(version: &str) -> Self {
    let version = version.trim();

    // Handle special case for Zen Browser twilight releases
    if version.to_lowercase().contains("twilight") {
      return VersionComponent {
        major: u32::MAX,
        minor: u32::MAX,
        patch: u32::MAX,
        pre_release: None,
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

// Helper function to sort GitHub releases
pub fn sort_github_releases(releases: &mut [GithubRelease]) {
  releases.sort_by(|a, b| {
    let version_a = VersionComponent::parse(&a.tag_name);
    let version_b = VersionComponent::parse(&b.tag_name);
    version_b.cmp(&version_a) // Descending order (newest first)
  });
}

pub fn is_alpha_version(version: &str) -> bool {
  let version_comp = VersionComponent::parse(version);
  version_comp.pre_release.is_some()
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
  pub download_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedVersionData {
  versions: Vec<String>,
  timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedGithubData {
  releases: Vec<GithubRelease>,
  timestamp: u64,
}

pub struct ApiClient {
  client: Client,
}

impl ApiClient {
  pub fn new() -> Self {
    Self {
      client: Client::new(),
    }
  }

  fn get_cache_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
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

  pub fn load_cached_versions(&self, browser: &str) -> Option<Vec<String>> {
    let cache_dir = Self::get_cache_dir().ok()?;
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    if !cache_file.exists() {
      return None;
    }

    let content = fs::read_to_string(&cache_file).ok()?;
    let cached_data: CachedVersionData = serde_json::from_str(&content).ok()?;

    // Always return cached versions regardless of age - they're always valid
    println!("Using cached versions for {browser}");
    Some(cached_data.versions)
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
    versions: &[String],
  ) -> Result<(), Box<dyn std::error::Error>> {
    let cache_dir = Self::get_cache_dir()?;
    let cache_file = cache_dir.join(format!("{browser}_versions.json"));

    let cached_data = CachedVersionData {
      versions: versions.to_vec(),
      timestamp: Self::get_current_timestamp(),
    };

    let content = serde_json::to_string_pretty(&cached_data)?;
    fs::write(&cache_file, content)?;
    println!("Cached {} versions for {}", versions.len(), browser);
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
    println!("Using cached GitHub releases for {browser}");
    Some(cached_data.releases)
  }

  fn save_cached_github_releases(
    &self,
    browser: &str,
    releases: &[GithubRelease],
  ) -> Result<(), Box<dyn std::error::Error>> {
    let cache_dir = Self::get_cache_dir()?;
    let cache_file = cache_dir.join(format!("{browser}_github.json"));

    let cached_data = CachedGithubData {
      releases: releases.to_vec(),
      timestamp: Self::get_current_timestamp(),
    };

    let content = serde_json::to_string_pretty(&cached_data)?;
    fs::write(&cache_file, content)?;
    println!("Cached {} GitHub releases for {}", releases.len(), browser);
    Ok(())
  }

  pub async fn fetch_firefox_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_versions) = self.load_cached_versions("firefox") {
        return Ok(
          cached_versions
            .into_iter()
            .map(|version| {
              BrowserRelease {
                version: version.clone(),
                date: "".to_string(), // Cache doesn't store dates
                is_prerelease: is_alpha_version(&version),
                download_url: Some(format!(
                  "https://download.mozilla.org/?product=firefox-{version}&os=osx&lang=en-US"
                )),
              }
            })
            .collect(),
        );
      }
    }

    println!("Fetching Firefox releases from Mozilla API...");
    let url = "https://product-details.mozilla.org/1.0/firefox.json";

    let response = self
      .client
      .get(url)
      .header("User-Agent", "donutbrowser")
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
            download_url: Some(format!(
              "https://download.mozilla.org/?product=firefox-{}&os=osx&lang=en-US",
              release.version
            )),
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

    // Extract versions for caching
    let versions: Vec<String> = releases.iter().map(|r| r.version.clone()).collect();

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_versions("firefox", &versions) {
        eprintln!("Failed to cache Firefox versions: {e}");
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
      if let Some(cached_versions) = self.load_cached_versions("firefox-developer") {
        return Ok(
          cached_versions
            .into_iter()
            .map(|version| {
              BrowserRelease {
                version: version.clone(),
                date: "".to_string(), // Cache doesn't store dates
                is_prerelease: is_alpha_version(&version),
                download_url: Some(format!(
                  "https://download.mozilla.org/?product=devedition-{version}&os=osx&lang=en-US"
                )),
              }
            })
            .collect(),
        );
      }
    }

    println!("Fetching Firefox Developer Edition releases from Mozilla API...");
    let url = "https://product-details.mozilla.org/1.0/devedition.json";

    let response = self
      .client
      .get(url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(
        format!(
          "Failed to fetch Firefox Developer Edition versions: {}",
          response.status()
        )
        .into(),
      );
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
            download_url: Some(format!(
              "https://download.mozilla.org/?product=devedition-{}&os=osx&lang=en-US",
              release.version
            )),
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

    // Extract versions for caching
    let versions: Vec<String> = releases.iter().map(|r| r.version.clone()).collect();

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_versions("firefox-developer", &versions) {
        eprintln!("Failed to cache Firefox Developer versions: {e}");
      }
    }

    Ok(releases)
  }

  pub async fn fetch_mullvad_releases(
    &self,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self.fetch_mullvad_releases_with_caching(false).await
  }

  pub async fn fetch_mullvad_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_releases) = self.load_cached_github_releases("mullvad") {
        return Ok(cached_releases);
      }
    }

    println!("Fetching Mullvad releases from GitHub API...");
    let url = "https://api.github.com/repos/mullvad/mullvad-browser/releases";
    let releases = self
      .client
      .get(url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?
      .json::<Vec<GithubRelease>>()
      .await?;

    let mut releases: Vec<GithubRelease> = releases
      .into_iter()
      .map(|mut release| {
        release.is_alpha = release.prerelease;
        release
      })
      .collect();

    // Sort releases using the new version sorting system
    sort_github_releases(&mut releases);

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_github_releases("mullvad", &releases) {
        eprintln!("Failed to cache Mullvad releases: {e}");
      }
    }

    Ok(releases)
  }

  pub async fn fetch_zen_releases(
    &self,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self.fetch_zen_releases_with_caching(false).await
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

    println!("Fetching Zen releases from GitHub API...");
    let url = "https://api.github.com/repos/zen-browser/desktop/releases";
    let mut releases = self
      .client
      .get(url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?
      .json::<Vec<GithubRelease>>()
      .await?;

    // Sort releases using the new version sorting system (twilight releases will be at top)
    sort_github_releases(&mut releases);

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_github_releases("zen", &releases) {
        eprintln!("Failed to cache Zen releases: {e}");
      }
    }

    Ok(releases)
  }

  pub async fn fetch_brave_releases(
    &self,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    self.fetch_brave_releases_with_caching(false).await
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

    println!("Fetching Brave releases from GitHub API...");
    let url = "https://api.github.com/repos/brave/brave-browser/releases";
    let releases = self
      .client
      .get(url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?
      .json::<Vec<GithubRelease>>()
      .await?;

    // Filter releases that have universal macOS DMG assets
    let mut filtered_releases: Vec<GithubRelease> = releases
      .into_iter()
      .filter_map(|mut release| {
        // Check if this release has a universal DMG asset
        let has_universal_dmg = release
          .assets
          .iter()
          .any(|asset| asset.name.contains(".dmg") && asset.name.contains("universal"));

        if has_universal_dmg {
          // Set is_alpha based on the release name
          // Nightly releases contain "Nightly", stable contain "Release"
          release.is_alpha = release.name.to_lowercase().contains("nightly");
          Some(release)
        } else {
          None
        }
      })
      .collect();

    // Sort releases using the new version sorting system
    sort_github_releases(&mut filtered_releases);

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_github_releases("brave", &filtered_releases) {
        eprintln!("Failed to cache Brave releases: {e}");
      }
    }

    Ok(filtered_releases)
  }

  pub async fn fetch_chromium_latest_version(
    &self,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Use architecture-aware URL for Chromium
    let arch = if cfg!(target_arch = "aarch64") {
      "Mac_Arm"
    } else {
      "Mac"
    };
    let url = format!(
      "https://commondatastorage.googleapis.com/chromium-browser-snapshots/{arch}/LAST_CHANGE"
    );
    let version = self
      .client
      .get(&url)
      .header("User-Agent", "donutbrowser")
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
      if let Some(cached_versions) = self.load_cached_versions("chromium") {
        return Ok(
          cached_versions
            .into_iter()
            .map(|version| {
              BrowserRelease {
                version: version.clone(),
                date: "".to_string(), // Cache doesn't store dates
                is_prerelease: false, // Chromium versions are generally stable builds
                download_url: None,
              }
            })
            .collect(),
        );
      }
    }

    println!("Fetching Chromium releases...");

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

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_versions("chromium", &versions) {
        eprintln!("Failed to cache Chromium versions: {e}");
      }
    }

    Ok(
      versions
        .into_iter()
        .map(|version| BrowserRelease {
          version: version.clone(),
          date: "".to_string(),
          is_prerelease: false,
          download_url: None,
        })
        .collect(),
    )
  }

  pub async fn fetch_tor_releases_with_caching(
    &self,
    no_caching: bool,
  ) -> Result<Vec<BrowserRelease>, Box<dyn std::error::Error + Send + Sync>> {
    // Check cache first (unless bypassing)
    if !no_caching {
      if let Some(cached_versions) = self.load_cached_versions("tor-browser") {
        return Ok(cached_versions.into_iter().map(|version| {
          BrowserRelease {
            version: version.clone(),
            date: "".to_string(), // Cache doesn't store dates
            is_prerelease: false, // Assume all archived versions are stable
            download_url: Some(format!(
              "https://archive.torproject.org/tor-package-archive/torbrowser/{version}/tor-browser-macos-{version}.dmg"
            )),
          }
        }).collect());
      }
    }

    println!("Fetching TOR releases from archive...");
    let url = "https://archive.torproject.org/tor-package-archive/torbrowser/";
    let html = self
      .client
      .get(url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?
      .text()
      .await?;

    // Parse HTML to extract version directories
    let mut version_candidates = Vec::new();

    // Look for directory links in the HTML
    for line in html.lines() {
      if line.contains("<a href=\"") && line.contains("/\">") {
        // Extract the directory name from the href attribute
        if let Some(start) = line.find("<a href=\"") {
          let start = start + 9; // Length of "<a href=\""
          if let Some(end) = line[start..].find("/\">") {
            let version = &line[start..start + end];

            // Skip parent directory and non-version entries
            if version != ".."
              && !version.is_empty()
              && version.chars().next().unwrap_or('a').is_ascii_digit()
            {
              version_candidates.push(version.to_string());
            }
          }
        }
      }
    }

    // Sort version candidates using the new version sorting system
    sort_versions(&mut version_candidates);

    // Only check the first 10 versions to avoid being too slow
    let mut version_strings = Vec::new();
    for version in version_candidates.into_iter().take(10) {
      // Check if this version has a macOS DMG file
      if let Ok(has_macos) = self.check_tor_version_has_macos(&version).await {
        if has_macos {
          version_strings.push(version);
        }
      }

      // Add a small delay to avoid overwhelming the server
      tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Cache the results (unless bypassing cache)
    if !no_caching {
      if let Err(e) = self.save_cached_versions("tor-browser", &version_strings) {
        eprintln!("Failed to cache TOR versions: {e}");
      }
    }

    Ok(version_strings.into_iter().map(|version| {
      BrowserRelease {
        version: version.clone(),
        date: "".to_string(), // TOR archive doesn't provide structured dates
        is_prerelease: false, // Assume all archived versions are stable
        download_url: Some(format!(
          "https://archive.torproject.org/tor-package-archive/torbrowser/{version}/tor-browser-macos-{version}.dmg"
        )),
      }
    }).collect())
  }

  async fn check_tor_version_has_macos(
    &self,
    version: &str,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("https://archive.torproject.org/tor-package-archive/torbrowser/{version}/");
    let html = self
      .client
      .get(&url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?
      .text()
      .await?;

    // Check if there's a macOS DMG file in this version directory
    Ok(html.contains("tor-browser-macos-") && html.contains(".dmg"))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
    let v4 = VersionComponent::parse("1.0.0-twilight");
    assert_eq!(v4.major, u32::MAX);
    assert_eq!(v4.minor, u32::MAX);
    assert_eq!(v4.patch, u32::MAX);
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

    // Test twilight version (should be highest)
    let v11 = VersionComponent::parse("1.0.0-twilight");
    let v12 = VersionComponent::parse("999.999.999");
    assert!(v11 > v12);
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
      "1.0.0-twilight".to_string(),
      "2.0.0a1".to_string(),
    ];

    sort_versions(&mut versions);

    // Expected order: twilight, 137.0, 137.0b5, 137.0b4, 2.0.0a1, 1.12.6b, 1.10.0, 1.9.9b
    assert_eq!(versions[0], "1.0.0-twilight");
    assert_eq!(versions[1], "137.0");
    assert_eq!(versions[2], "137.0b5");
    assert_eq!(versions[3], "137.0b4");
    assert_eq!(versions[4], "2.0.0a1");
    assert_eq!(versions[5], "1.12.6b");
    assert_eq!(versions[6], "1.10.0");
    assert_eq!(versions[7], "1.9.9b");
  }

  #[tokio::test]
  async fn test_firefox_api() {
    let client = ApiClient::new();
    let result = client.fetch_firefox_releases_with_caching(false).await;

    match result {
      Ok(releases) => {
        assert!(!releases.is_empty(), "Should have Firefox releases");

        // Check that releases have required fields
        let first_release = &releases[0];
        assert!(
          !first_release.version.is_empty(),
          "Version should not be empty"
        );
        assert!(
          first_release.download_url.is_some(),
          "Should have download URL"
        );

        println!("Firefox API test passed. Found {} releases", releases.len());
        println!("Latest version: {}", releases[0].version);
      }
      Err(e) => {
        println!("Firefox API test failed: {e}");
        panic!("Firefox API should work");
      }
    }
  }

  #[tokio::test]
  async fn test_firefox_developer_api() {
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await; // Rate limiting

    let client = ApiClient::new();
    let result = client
      .fetch_firefox_developer_releases_with_caching(false)
      .await;

    match result {
      Ok(releases) => {
        assert!(
          !releases.is_empty(),
          "Should have Firefox Developer releases"
        );

        let first_release = &releases[0];
        assert!(
          !first_release.version.is_empty(),
          "Version should not be empty"
        );
        assert!(
          first_release.download_url.is_some(),
          "Should have download URL"
        );

        println!(
          "Firefox Developer API test passed. Found {} releases",
          releases.len()
        );
        println!("Latest version: {}", releases[0].version);
      }
      Err(e) => {
        println!("Firefox Developer API test failed: {e}");
        panic!("Firefox Developer API should work");
      }
    }
  }

  #[tokio::test]
  async fn test_mullvad_api() {
    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await; // Rate limiting

    let client = ApiClient::new();
    let result = client.fetch_mullvad_releases().await;

    match result {
      Ok(releases) => {
        assert!(!releases.is_empty(), "Should have Mullvad releases");

        let first_release = &releases[0];
        assert!(
          !first_release.tag_name.is_empty(),
          "Tag name should not be empty"
        );

        println!("Mullvad API test passed. Found {} releases", releases.len());
        println!("Latest version: {}", releases[0].tag_name);
      }
      Err(e) => {
        println!("Mullvad API test failed: {e}");
        panic!("Mullvad API should work");
      }
    }
  }

  #[tokio::test]
  async fn test_zen_api() {
    tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await; // Rate limiting

    let client = ApiClient::new();
    let result = client.fetch_zen_releases().await;

    match result {
      Ok(releases) => {
        assert!(!releases.is_empty(), "Should have Zen releases");

        let first_release = &releases[0];
        assert!(
          !first_release.tag_name.is_empty(),
          "Tag name should not be empty"
        );

        println!("Zen API test passed. Found {} releases", releases.len());
        println!("Latest version: {}", releases[0].tag_name);
      }
      Err(e) => {
        println!("Zen API test failed: {e}");
        panic!("Zen API should work");
      }
    }
  }

  #[tokio::test]
  async fn test_brave_api() {
    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await; // Rate limiting

    let client = ApiClient::new();
    let result = client.fetch_brave_releases().await;

    match result {
      Ok(releases) => {
        // Note: Brave might not always have macOS releases, so we don't assert non-empty
        println!(
          "Brave API test passed. Found {} releases with macOS assets",
          releases.len()
        );
        if !releases.is_empty() {
          println!("Latest version: {}", releases[0].tag_name);
        }
      }
      Err(e) => {
        println!("Brave API test failed: {e}");
        panic!("Brave API should work");
      }
    }
  }

  #[tokio::test]
  async fn test_chromium_api() {
    tokio::time::sleep(tokio::time::Duration::from_millis(2500)).await; // Rate limiting

    let client = ApiClient::new();
    let result = client.fetch_chromium_latest_version().await;

    match result {
      Ok(version) => {
        assert!(!version.is_empty(), "Version should not be empty");
        assert!(
          version.chars().all(|c| c.is_ascii_digit()),
          "Version should be numeric"
        );

        println!("Chromium API test passed. Latest version: {version}");
      }
      Err(e) => {
        println!("Chromium API test failed: {e}");
        panic!("Chromium API should work");
      }
    }
  }

  #[tokio::test]
  async fn test_tor_api() {
    tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await; // Rate limiting

    let client = ApiClient::new();

    // Use a timeout for this test since TOR API can be slow
    let timeout_duration = tokio::time::Duration::from_secs(30);
    let result = tokio::time::timeout(
      timeout_duration,
      client.fetch_tor_releases_with_caching(false),
    )
    .await;

    match result {
      Ok(Ok(releases)) => {
        assert!(!releases.is_empty(), "Should have TOR releases");

        let first_release = &releases[0];
        assert!(
          !first_release.version.is_empty(),
          "Version should not be empty"
        );
        assert!(
          first_release.download_url.is_some(),
          "Should have download URL"
        );

        println!("TOR API test passed. Found {} releases", releases.len());
        println!("Latest version: {}", releases[0].version);
      }
      Ok(Err(e)) => {
        println!("TOR API test failed: {e}");
        // Don't panic for TOR API since it can be unreliable
        println!("TOR API test skipped due to network issues");
      }
      Err(_) => {
        println!("TOR API test timed out after 30 seconds");
        // Don't panic for timeout, just skip
        println!("TOR API test skipped due to timeout");
      }
    }
  }

  #[tokio::test]
  async fn test_tor_version_check() {
    tokio::time::sleep(tokio::time::Duration::from_millis(3500)).await; // Rate limiting

    let client = ApiClient::new();
    let result = client.check_tor_version_has_macos("14.0.4").await;

    match result {
      Ok(has_macos) => {
        assert!(has_macos, "Version 14.0.4 should have macOS support");
        println!("TOR version check test passed. Version 14.0.4 has macOS: {has_macos}");
      }
      Err(e) => {
        println!("TOR version check test failed: {e}");
        panic!("TOR version check should work");
      }
    }
  }
}

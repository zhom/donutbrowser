use crate::browser::GithubRelease;
use directories::BaseDirs;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Emitter;
use tokio::fs;
use tokio::io::AsyncWriteExt;

const MMDB_REPO: &str = "P3TERX/GeoLite.mmdb";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoIPDownloadProgress {
  pub stage: String, // "downloading", "extracting", "completed"
  pub percentage: f64,
  pub message: String,
}

pub struct GeoIPDownloader {
  client: Client,
}

impl GeoIPDownloader {
  pub fn new() -> Self {
    Self {
      client: Client::new(),
    }
  }

  /// Create a new downloader with custom client (for testing)
  #[cfg(test)]
  pub fn new_with_client(client: Client) -> Self {
    Self { client }
  }

  fn get_cache_dir() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let base_dirs = BaseDirs::new().ok_or("Failed to determine base directories")?;

    #[cfg(target_os = "windows")]
    let cache_dir = base_dirs
      .data_local_dir()
      .join("camoufox")
      .join("camoufox")
      .join("Cache");

    #[cfg(target_os = "macos")]
    let cache_dir = base_dirs.cache_dir().join("camoufox");

    #[cfg(target_os = "linux")]
    let cache_dir = base_dirs.cache_dir().join("camoufox");

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let cache_dir = base_dirs.cache_dir().join("camoufox");

    Ok(cache_dir)
  }

  fn get_mmdb_file_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Self::get_cache_dir()?.join("GeoLite2-City.mmdb"))
  }

  pub fn is_geoip_database_available() -> bool {
    if let Ok(mmdb_path) = Self::get_mmdb_file_path() {
      mmdb_path.exists()
    } else {
      false
    }
  }

  fn find_city_mmdb_asset(&self, release: &GithubRelease) -> Option<String> {
    for asset in &release.assets {
      if asset.name.ends_with("-City.mmdb") {
        return Some(asset.browser_download_url.clone());
      }
    }
    None
  }

  pub async fn download_geoip_database(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Emit initial progress
    let _ = app_handle.emit(
      "geoip-download-progress",
      GeoIPDownloadProgress {
        stage: "downloading".to_string(),
        percentage: 0.0,
        message: "Starting GeoIP database download".to_string(),
      },
    );

    // Fetch latest release from GitHub
    let releases = self.fetch_geoip_releases().await?;
    let latest_release = releases.first().ok_or("No GeoIP database releases found")?;

    let download_url = self
      .find_city_mmdb_asset(latest_release)
      .ok_or("No compatible GeoIP database asset found")?;

    // Create cache directory
    let cache_dir = Self::get_cache_dir()?;
    fs::create_dir_all(&cache_dir).await?;

    let mmdb_path = Self::get_mmdb_file_path()?;

    // Download the file
    let response = self.client.get(&download_url).send().await?;

    if !response.status().is_success() {
      return Err(
        format!(
          "Failed to download GeoIP database: HTTP {}",
          response.status()
        )
        .into(),
      );
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut downloaded = 0;
    let mut file = fs::File::create(&mmdb_path).await?;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
      let chunk = chunk?;
      downloaded += chunk.len() as u64;
      file.write_all(&chunk).await?;

      if total_size > 0 {
        let percentage = (downloaded as f64 / total_size as f64) * 100.0;
        let _ = app_handle.emit(
          "geoip-download-progress",
          GeoIPDownloadProgress {
            stage: "downloading".to_string(),
            percentage,
            message: format!("Downloaded {downloaded} / {total_size} bytes"),
          },
        );
      }
    }

    file.flush().await?;

    // Emit completion
    let _ = app_handle.emit(
      "geoip-download-progress",
      GeoIPDownloadProgress {
        stage: "completed".to_string(),
        percentage: 100.0,
        message: "GeoIP database download completed".to_string(),
      },
    );

    Ok(())
  }

  async fn fetch_geoip_releases(
    &self,
  ) -> Result<Vec<GithubRelease>, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("https://api.github.com/repos/{MMDB_REPO}/releases");
    let response = self
      .client
      .get(&url)
      .header("User-Agent", "Mozilla/5.0 (compatible; donutbrowser)")
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(format!("Failed to fetch releases: HTTP {}", response.status()).into());
    }

    let releases: Vec<GithubRelease> = response.json().await?;
    Ok(releases)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::browser::GithubRelease;
  use wiremock::matchers::{method, path};
  use wiremock::{Mock, MockServer, ResponseTemplate};

  fn create_mock_release() -> GithubRelease {
    GithubRelease {
      tag_name: "v1.0.0".to_string(),
      name: "Test Release".to_string(),
      body: Some("Test release body".to_string()),
      published_at: "2023-01-01T00:00:00Z".to_string(),
      created_at: Some("2023-01-01T00:00:00Z".to_string()),
      html_url: Some("https://example.com/release".to_string()),
      tarball_url: Some("https://example.com/tarball".to_string()),
      zipball_url: Some("https://example.com/zipball".to_string()),
      draft: false,
      prerelease: false,
      is_nightly: false,
      id: Some(1),
      node_id: Some("test_node_id".to_string()),
      target_commitish: None,
      assets: vec![crate::browser::GithubAsset {
        id: Some(1),
        node_id: Some("test_asset_node_id".to_string()),
        name: "GeoLite2-City.mmdb".to_string(),
        label: None,
        content_type: Some("application/octet-stream".to_string()),
        state: Some("uploaded".to_string()),
        size: 1024,
        download_count: Some(0),
        created_at: Some("2023-01-01T00:00:00Z".to_string()),
        updated_at: Some("2023-01-01T00:00:00Z".to_string()),
        browser_download_url: "https://example.com/GeoLite2-City.mmdb".to_string(),
      }],
    }
  }

  #[tokio::test]
  async fn test_fetch_geoip_releases_success() {
    let mock_server = MockServer::start().await;
    let releases = vec![create_mock_release()];

    Mock::given(method("GET"))
      .and(path(format!("/repos/{MMDB_REPO}/releases")))
      .respond_with(ResponseTemplate::new(200).set_body_json(&releases))
      .mount(&mock_server)
      .await;

    let client = Client::builder()
      .build()
      .expect("Failed to create HTTP client");

    let downloader = GeoIPDownloader::new_with_client(client);

    // Override the URL for testing
    let url = format!("{}/repos/{}/releases", mock_server.uri(), MMDB_REPO);
    let response = downloader
      .client
      .get(&url)
      .header("User-Agent", "Mozilla/5.0 (compatible; donutbrowser)")
      .send()
      .await
      .expect("Request should succeed");

    assert!(response.status().is_success());

    let fetched_releases: Vec<GithubRelease> = response.json().await.expect("Should parse JSON");
    assert_eq!(fetched_releases.len(), 1);
    assert_eq!(fetched_releases[0].tag_name, "v1.0.0");
  }

  #[tokio::test]
  async fn test_find_city_mmdb_asset() {
    let downloader = GeoIPDownloader::new();
    let release = create_mock_release();

    let asset_url = downloader.find_city_mmdb_asset(&release);
    assert!(asset_url.is_some());
    assert_eq!(asset_url.unwrap(), "https://example.com/GeoLite2-City.mmdb");
  }

  #[tokio::test]
  async fn test_find_city_mmdb_asset_not_found() {
    let downloader = GeoIPDownloader::new();
    let mut release = create_mock_release();
    release.assets[0].name = "wrong-file.txt".to_string();

    let asset_url = downloader.find_city_mmdb_asset(&release);
    assert!(asset_url.is_none());
  }

  #[test]
  fn test_get_cache_dir() {
    let cache_dir = GeoIPDownloader::get_cache_dir();
    assert!(cache_dir.is_ok());

    let path = cache_dir.unwrap();
    assert!(path.to_string_lossy().contains("camoufox"));
  }

  #[test]
  fn test_get_mmdb_file_path() {
    let mmdb_path = GeoIPDownloader::get_mmdb_file_path();
    assert!(mmdb_path.is_ok());

    let path = mmdb_path.unwrap();
    assert!(path.to_string_lossy().ends_with("GeoLite2-City.mmdb"));
  }

  #[test]
  fn test_is_geoip_database_available() {
    // This test will return false unless the database actually exists
    // In a real environment, this would check the actual file system
    let is_available = GeoIPDownloader::is_geoip_database_available();
    // We can't assert a specific value since it depends on the system state
    // But we can verify the function doesn't panic
    println!("GeoIP database available: {is_available}");
  }
}

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

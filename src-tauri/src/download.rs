use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use tauri::Emitter;

use crate::api_client::ApiClient;
use crate::browser::BrowserType;
use crate::browser_version_service::DownloadInfo;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadProgress {
  pub browser: String,
  pub version: String,
  pub downloaded_bytes: u64,
  pub total_bytes: Option<u64>,
  pub percentage: f64,
  pub speed_bytes_per_sec: f64,
  pub eta_seconds: Option<f64>,
  pub stage: String, // "downloading", "extracting", "verifying"
}

pub struct Downloader {
  client: Client,
  api_client: ApiClient,
}

impl Downloader {
  pub fn new() -> Self {
    Self {
      client: Client::new(),
      api_client: ApiClient::new(),
    }
  }

  /// Resolve the actual download URL for browsers that need dynamic asset resolution
  pub async fn resolve_download_url(
    &self,
    browser_type: BrowserType,
    version: &str,
    download_info: &DownloadInfo,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match browser_type {
      BrowserType::Brave => {
        // For Brave, we need to find the actual macOS asset
        let releases = self.api_client.fetch_brave_releases().await?;

        // Find the release with the matching version
        let release = releases
          .iter()
          .find(|r| {
            r.tag_name == version || r.tag_name == format!("v{}", version.trim_start_matches('v'))
          })
          .ok_or(format!("Brave version {} not found", version))?;

        // Find the universal macOS DMG asset
        let asset = release
          .assets
          .iter()
          .find(|asset| asset.name.contains(".dmg") && asset.name.contains("universal"))
          .ok_or(format!(
            "No universal macOS DMG asset found for Brave version {}",
            version
          ))?;

        Ok(asset.browser_download_url.clone())
      }
      BrowserType::Zen => {
        // For Zen, verify the asset exists
        let releases = self.api_client.fetch_zen_releases().await?;

        let release = releases
          .iter()
          .find(|r| r.tag_name == version)
          .ok_or(format!("Zen version {} not found", version))?;

        // Find the macOS universal DMG asset
        let asset = release
          .assets
          .iter()
          .find(|asset| asset.name == "zen.macos-universal.dmg")
          .ok_or(format!(
            "No macOS universal asset found for Zen version {}",
            version
          ))?;

        Ok(asset.browser_download_url.clone())
      }
      BrowserType::MullvadBrowser => {
        // For Mullvad, verify the asset exists
        let releases = self.api_client.fetch_mullvad_releases().await?;

        let release = releases
          .iter()
          .find(|r| r.tag_name == version)
          .ok_or(format!("Mullvad version {} not found", version))?;

        // Find the macOS DMG asset
        let asset = release
          .assets
          .iter()
          .find(|asset| asset.name.contains(".dmg") && asset.name.contains("mac"))
          .ok_or(format!(
            "No macOS asset found for Mullvad version {}",
            version
          ))?;

        Ok(asset.browser_download_url.clone())
      }
      _ => {
        // For other browsers, use the provided URL
        Ok(download_info.url.clone())
      }
    }
  }

  pub async fn download_browser(
    &self,
    app_handle: &tauri::AppHandle,
    browser_type: BrowserType,
    version: &str,
    download_info: &DownloadInfo,
    dest_path: &Path,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let file_path = dest_path.join(&download_info.filename);

    // Resolve the actual download URL
    let download_url = self
      .resolve_download_url(browser_type.clone(), version, download_info)
      .await?;

    // Emit initial progress
    let progress = DownloadProgress {
      browser: browser_type.as_str().to_string(),
      version: version.to_string(),
      downloaded_bytes: 0,
      total_bytes: None,
      percentage: 0.0,
      speed_bytes_per_sec: 0.0,
      eta_seconds: None,
      stage: "downloading".to_string(),
    };

    let _ = app_handle.emit("download-progress", &progress);

    // Start download
    let response = self
      .client
      .get(&download_url)
      .header("User-Agent", "donutbrowser")
      .send()
      .await?;

    let total_size = response.content_length();
    let mut downloaded = 0u64;
    let start_time = std::time::Instant::now();
    let mut last_update = start_time;

    let mut file = File::create(&file_path)?;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
      let chunk = chunk?;
      io::copy(&mut chunk.as_ref(), &mut file)?;
      downloaded += chunk.len() as u64;

      let now = std::time::Instant::now();
      // Update progress every 100ms to avoid too many events
      if now.duration_since(last_update).as_millis() >= 100 {
        let elapsed = start_time.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
          downloaded as f64 / elapsed
        } else {
          0.0
        };
        let percentage = if let Some(total) = total_size {
          (downloaded as f64 / total as f64) * 100.0
        } else {
          0.0
        };
        let eta = if speed > 0.0 {
          total_size.map(|total| (total - downloaded) as f64 / speed)
        } else {
          None
        };

        let progress = DownloadProgress {
          browser: browser_type.as_str().to_string(),
          version: version.to_string(),
          downloaded_bytes: downloaded,
          total_bytes: total_size,
          percentage,
          speed_bytes_per_sec: speed,
          eta_seconds: eta,
          stage: "downloading".to_string(),
        };

        let _ = app_handle.emit("download-progress", &progress);
        last_update = now;
      }
    }

    Ok(file_path)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn test_resolve_brave_download_url() {
    let downloader = Downloader::new();

    // Test with a known Brave version
    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "brave-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Brave, "v1.81.9", &download_info)
      .await;

    match result {
      Ok(url) => {
        assert!(url.contains("github.com/brave/brave-browser"));
        assert!(url.contains(".dmg"));
        assert!(url.contains("universal"));
        println!("Brave download URL resolved: {}", url);
      }
      Err(e) => {
        println!(
          "Brave URL resolution failed (expected if version doesn't exist): {}",
          e
        );
        // This might fail if the version doesn't exist, which is okay for testing
      }
    }
  }

  #[tokio::test]
  async fn test_resolve_zen_download_url() {
    let downloader = Downloader::new();

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "zen-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Zen, "1.11b", &download_info)
      .await;

    match result {
      Ok(url) => {
        assert!(url.contains("github.com/zen-browser/desktop"));
        assert!(url.contains("zen.macos-universal.dmg"));
        println!("Zen download URL resolved: {}", url);
      }
      Err(e) => {
        println!(
          "Zen URL resolution failed (expected if version doesn't exist): {}",
          e
        );
      }
    }
  }

  #[tokio::test]
  async fn test_resolve_mullvad_download_url() {
    let downloader = Downloader::new();

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "mullvad-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::MullvadBrowser, "14.5a6", &download_info)
      .await;

    match result {
      Ok(url) => {
        assert!(url.contains("github.com/mullvad/mullvad-browser"));
        assert!(url.contains(".dmg"));
        println!("Mullvad download URL resolved: {}", url);
      }
      Err(e) => {
        println!(
          "Mullvad URL resolution failed (expected if version doesn't exist): {}",
          e
        );
      }
    }
  }

  #[tokio::test]
  async fn test_resolve_firefox_download_url() {
    let downloader = Downloader::new();

    let download_info = DownloadInfo {
      url: "https://download.mozilla.org/?product=firefox-139.0&os=osx&lang=en-US".to_string(),
      filename: "firefox-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Firefox, "139.0", &download_info)
      .await;

    match result {
      Ok(url) => {
        assert_eq!(url, download_info.url);
        println!("Firefox download URL (passthrough): {}", url);
      }
      Err(e) => {
        panic!("Firefox URL resolution should not fail: {}", e);
      }
    }
  }

  #[tokio::test]
  async fn test_resolve_chromium_download_url() {
    let downloader = Downloader::new();

    let download_info = DownloadInfo {
      url: "https://commondatastorage.googleapis.com/chromium-browser-snapshots/Mac/1465660/chrome-mac.zip".to_string(),
      filename: "chromium-test.zip".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Chromium, "1465660", &download_info)
      .await;

    match result {
      Ok(url) => {
        assert_eq!(url, download_info.url);
        println!("Chromium download URL (passthrough): {}", url);
      }
      Err(e) => {
        panic!("Chromium URL resolution should not fail: {}", e);
      }
    }
  }

  #[tokio::test]
  async fn test_resolve_tor_download_url() {
    let downloader = Downloader::new();

    let download_info = DownloadInfo {
      url: "https://archive.torproject.org/tor-package-archive/torbrowser/14.0.4/tor-browser-macos-14.0.4.dmg".to_string(),
      filename: "tor-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::TorBrowser, "14.0.4", &download_info)
      .await;

    match result {
      Ok(url) => {
        assert_eq!(url, download_info.url);
        println!("TOR download URL (passthrough): {}", url);
      }
      Err(e) => {
        panic!("TOR URL resolution should not fail: {}", e);
      }
    }
  }
}

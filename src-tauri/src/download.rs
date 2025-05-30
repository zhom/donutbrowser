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

  #[cfg(test)]
  pub fn new_with_api_client(api_client: ApiClient) -> Self {
    Self {
      client: Client::new(),
      api_client,
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
        let releases = self.api_client.fetch_brave_releases_with_caching(true).await?;

        // Find the release with the matching version
        let release = releases
          .iter()
          .find(|r| {
            r.tag_name == version || r.tag_name == format!("v{}", version.trim_start_matches('v'))
          })
          .ok_or(format!("Brave version {version} not found"))?;

        // Find the universal macOS DMG asset
        let asset = release
          .assets
          .iter()
          .find(|asset| asset.name.contains(".dmg") && asset.name.contains("universal"))
          .ok_or(format!(
            "No universal macOS DMG asset found for Brave version {version}"
          ))?;

        Ok(asset.browser_download_url.clone())
      }
      BrowserType::Zen => {
        // For Zen, verify the asset exists
        let releases = self.api_client.fetch_zen_releases_with_caching(true).await?;

        let release = releases
          .iter()
          .find(|r| r.tag_name == version)
          .ok_or(format!("Zen version {version} not found"))?;

        // Find the macOS universal DMG asset
        let asset = release
          .assets
          .iter()
          .find(|asset| asset.name == "zen.macos-universal.dmg")
          .ok_or(format!(
            "No macOS universal asset found for Zen version {version}"
          ))?;

        Ok(asset.browser_download_url.clone())
      }
      BrowserType::MullvadBrowser => {
        // For Mullvad, verify the asset exists
        let releases = self.api_client.fetch_mullvad_releases_with_caching(true).await?;

        let release = releases
          .iter()
          .find(|r| r.tag_name == version)
          .ok_or(format!("Mullvad version {version} not found"))?;

        // Find the macOS DMG asset
        let asset = release
          .assets
          .iter()
          .find(|asset| asset.name.contains(".dmg") && asset.name.contains("mac"))
          .ok_or(format!(
            "No macOS asset found for Mullvad version {version}"
          ))?;

        Ok(asset.browser_download_url.clone())
      }
      _ => {
        // For other browsers, use the provided URL
        Ok(download_info.url.clone())
      }
    }
  }

  pub async fn download_browser<R: tauri::Runtime>(
    &self,
    app_handle: &tauri::AppHandle<R>,
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

    // Check if the response is successful
    if !response.status().is_success() {
      return Err(format!("Download failed with status: {}", response.status()).into());
    }

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
  use crate::api_client::ApiClient;
  use crate::browser::BrowserType;
  use crate::browser_version_service::DownloadInfo;

  use wiremock::{MockServer, Mock, ResponseTemplate};
  use wiremock::matchers::{method, path, header};
  use tempfile::TempDir;

  async fn setup_mock_server() -> MockServer {
    MockServer::start().await
  }

  fn create_test_api_client(server: &MockServer) -> ApiClient {
    let base_url = server.uri();
    ApiClient::new_with_base_urls(
      base_url.clone(),    // firefox_api_base
      base_url.clone(),    // firefox_dev_api_base
      base_url.clone(),    // github_api_base
      base_url.clone(),    // chromium_api_base
      base_url.clone(),    // tor_archive_base
      base_url.clone(),    // mozilla_download_base
    )
  }

  #[tokio::test]
  async fn test_resolve_brave_download_url() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let mock_response = r#"[
      {
        "tag_name": "v1.81.9",
        "name": "Brave Release 1.81.9",
        "prerelease": false,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "brave-v1.81.9-universal.dmg",
            "browser_download_url": "https://example.com/brave-1.81.9-universal.dmg"
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/brave/brave-browser/releases"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_string(mock_response)
        .insert_header("content-type", "application/json"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "brave-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Brave, "v1.81.9", &download_info)
      .await;

    assert!(result.is_ok());
    let url = result.unwrap();
    assert_eq!(url, "https://example.com/brave-1.81.9-universal.dmg");
  }

  #[tokio::test]
  async fn test_resolve_zen_download_url() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let mock_response = r#"[
      {
        "tag_name": "1.11b",
        "name": "Zen Browser 1.11b",
        "prerelease": false,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "zen.macos-universal.dmg",
            "browser_download_url": "https://example.com/zen-1.11b-universal.dmg"
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/zen-browser/desktop/releases"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_string(mock_response)
        .insert_header("content-type", "application/json"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "zen-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Zen, "1.11b", &download_info)
      .await;

    assert!(result.is_ok());
    let url = result.unwrap();
    assert_eq!(url, "https://example.com/zen-1.11b-universal.dmg");
  }

  #[tokio::test]
  async fn test_resolve_mullvad_download_url() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let mock_response = r#"[
      {
        "tag_name": "14.5a6",
        "name": "Mullvad Browser 14.5a6",
        "prerelease": true,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "mullvad-browser-macos-14.5a6.dmg",
            "browser_download_url": "https://example.com/mullvad-14.5a6.dmg"
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/mullvad/mullvad-browser/releases"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_string(mock_response)
        .insert_header("content-type", "application/json"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "mullvad-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::MullvadBrowser, "14.5a6", &download_info)
      .await;

    assert!(result.is_ok());
    let url = result.unwrap();
    assert_eq!(url, "https://example.com/mullvad-14.5a6.dmg");
  }

  #[tokio::test]
  async fn test_resolve_firefox_download_url() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let download_info = DownloadInfo {
      url: "https://download.mozilla.org/?product=firefox-139.0&os=osx&lang=en-US".to_string(),
      filename: "firefox-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Firefox, "139.0", &download_info)
      .await;

    assert!(result.is_ok());
    let url = result.unwrap();
    assert_eq!(url, download_info.url);
  }

  #[tokio::test]
  async fn test_resolve_chromium_download_url() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let download_info = DownloadInfo {
      url: "https://commondatastorage.googleapis.com/chromium-browser-snapshots/Mac/1465660/chrome-mac.zip".to_string(),
      filename: "chromium-test.zip".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Chromium, "1465660", &download_info)
      .await;

    assert!(result.is_ok());
    let url = result.unwrap();
    assert_eq!(url, download_info.url);
  }

  #[tokio::test]
  async fn test_resolve_tor_download_url() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let download_info = DownloadInfo {
      url: "https://archive.torproject.org/tor-package-archive/torbrowser/14.0.4/tor-browser-macos-14.0.4.dmg".to_string(),
      filename: "tor-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::TorBrowser, "14.0.4", &download_info)
      .await;

    assert!(result.is_ok());
    let url = result.unwrap();
    assert_eq!(url, download_info.url);
  }

  #[tokio::test]
  async fn test_resolve_brave_version_not_found() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let mock_response = r#"[
      {
        "tag_name": "v1.81.8",
        "name": "Brave Release 1.81.8",
        "prerelease": false,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "brave-v1.81.8-universal.dmg",
            "browser_download_url": "https://example.com/brave-1.81.8-universal.dmg"
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/brave/brave-browser/releases"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_string(mock_response)
        .insert_header("content-type", "application/json"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "brave-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Brave, "v1.81.9", &download_info)
      .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Brave version v1.81.9 not found"));
  }

  #[tokio::test]
  async fn test_resolve_zen_asset_not_found() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let mock_response = r#"[
      {
        "tag_name": "1.11b",
        "name": "Zen Browser 1.11b",
        "prerelease": false,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "zen.linux-universal.tar.bz2",
            "browser_download_url": "https://example.com/zen-1.11b-linux.tar.bz2"
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/zen-browser/desktop/releases"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_string(mock_response)
        .insert_header("content-type", "application/json"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "zen-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::Zen, "1.11b", &download_info)
      .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No macOS universal asset found"));
  }

  #[tokio::test]
  async fn test_download_browser_with_progress() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);
    
    // Create a temporary directory for the test
    let temp_dir = TempDir::new().unwrap();
    let dest_path = temp_dir.path();

    // Create test file content (simulating a small download)
    let test_content = b"This is a test file content for download simulation";
    
    // Mock the download endpoint
    Mock::given(method("GET"))
      .and(path("/test-download"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_bytes(test_content)
        .insert_header("content-length", test_content.len().to_string())
        .insert_header("content-type", "application/octet-stream"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: format!("{}/test-download", server.uri()),
      filename: "test-file.dmg".to_string(),
      is_archive: true,
    };

    // Create a mock app handle for testing
    let app = tauri::test::mock_app();
    let app_handle = app.handle().clone();

    let result = downloader
      .download_browser(
        &app_handle,
        BrowserType::Firefox,
        "139.0",
        &download_info,
        dest_path,
      )
      .await;

    assert!(result.is_ok());
    let downloaded_file = result.unwrap();
    assert!(downloaded_file.exists());
    
    // Verify file content
    let downloaded_content = std::fs::read(&downloaded_file).unwrap();
    assert_eq!(downloaded_content, test_content);
  }

  #[tokio::test]
  async fn test_download_browser_network_error() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);
    
    let temp_dir = TempDir::new().unwrap();
    let dest_path = temp_dir.path();

    // Mock a 404 response
    Mock::given(method("GET"))
      .and(path("/missing-file"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(404))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: format!("{}/missing-file", server.uri()),
      filename: "missing-file.dmg".to_string(),
      is_archive: true,
    };

    let app = tauri::test::mock_app();
    let app_handle = app.handle().clone();

    let result = downloader
      .download_browser(
        &app_handle,
        BrowserType::Firefox,
        "139.0",
        &download_info,
        dest_path,
      )
      .await;

    assert!(result.is_err());
  }

  #[tokio::test]
  async fn test_resolve_mullvad_asset_not_found() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let mock_response = r#"[
      {
        "tag_name": "14.5a6",
        "name": "Mullvad Browser 14.5a6",
        "prerelease": true,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "mullvad-browser-linux-14.5a6.tar.xz",
            "browser_download_url": "https://example.com/mullvad-14.5a6.tar.xz"
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/mullvad/mullvad-browser/releases"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_string(mock_response)
        .insert_header("content-type", "application/json"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "mullvad-test.dmg".to_string(),
      is_archive: true,
    };

    let result = downloader
      .resolve_download_url(BrowserType::MullvadBrowser, "14.5a6", &download_info)
      .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No macOS asset found"));
  }

  #[tokio::test]
  async fn test_brave_version_with_v_prefix() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);

    let mock_response = r#"[
      {
        "tag_name": "v1.81.9",
        "name": "Brave Release 1.81.9",
        "prerelease": false,
        "published_at": "2024-01-15T10:00:00Z",
        "assets": [
          {
            "name": "brave-v1.81.9-universal.dmg",
            "browser_download_url": "https://example.com/brave-1.81.9-universal.dmg"
          }
        ]
      }
    ]"#;

    Mock::given(method("GET"))
      .and(path("/repos/brave/brave-browser/releases"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_string(mock_response)
        .insert_header("content-type", "application/json"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: "placeholder".to_string(),
      filename: "brave-test.dmg".to_string(),
      is_archive: true,
    };

    // Test with version without v prefix
    let result = downloader
      .resolve_download_url(BrowserType::Brave, "1.81.9", &download_info)
      .await;

    assert!(result.is_ok());
    let url = result.unwrap();
    assert_eq!(url, "https://example.com/brave-1.81.9-universal.dmg");
  }

  #[tokio::test]
  async fn test_download_browser_chunked_response() {
    let server = setup_mock_server().await;
    let api_client = create_test_api_client(&server);
    let downloader = Downloader::new_with_api_client(api_client);
    
    let temp_dir = TempDir::new().unwrap();
    let dest_path = temp_dir.path();

    // Create larger test content to simulate chunked transfer
    let test_content = vec![42u8; 1024]; // 1KB of data
    
    Mock::given(method("GET"))
      .and(path("/chunked-download"))
      .and(header("user-agent", "donutbrowser"))
      .respond_with(ResponseTemplate::new(200)
        .set_body_bytes(test_content.clone())
        .insert_header("content-length", test_content.len().to_string())
        .insert_header("content-type", "application/octet-stream"))
      .mount(&server)
      .await;

    let download_info = DownloadInfo {
      url: format!("{}/chunked-download", server.uri()),
      filename: "chunked-file.dmg".to_string(),
      is_archive: true,
    };

    let app = tauri::test::mock_app();
    let app_handle = app.handle().clone();

    let result = downloader
      .download_browser(
        &app_handle,
        BrowserType::Chromium,
        "1465660",
        &download_info,
        dest_path,
      )
      .await;

    assert!(result.is_ok());
    let downloaded_file = result.unwrap();
    assert!(downloaded_file.exists());
    
    let downloaded_content = std::fs::read(&downloaded_file).unwrap();
    assert_eq!(downloaded_content.len(), test_content.len());
  }
}

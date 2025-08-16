use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use tauri::Emitter;

use crate::api_client::ApiClient;
use crate::browser::BrowserType;
use crate::browser_version_manager::DownloadInfo;

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
  api_client: &'static ApiClient,
}

impl Downloader {
  fn new() -> Self {
    Self {
      client: Client::new(),
      api_client: ApiClient::instance(),
    }
  }

  pub fn instance() -> &'static Downloader {
    &DOWNLOADER
  }

  #[cfg(test)]
  pub fn new_with_api_client(_api_client: ApiClient) -> Self {
    Self {
      client: Client::new(),
      api_client: ApiClient::instance(),
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
        // For Brave, we need to find the actual platform-specific asset
        let releases = self
          .api_client
          .fetch_brave_releases_with_caching(true)
          .await?;

        // Find the release with the matching version
        let release = releases
          .iter()
          .find(|r| {
            r.tag_name == version || r.tag_name == format!("v{}", version.trim_start_matches('v'))
          })
          .ok_or(format!("Brave version {version} not found"))?;

        // Get platform and architecture info
        let (os, arch) = Self::get_platform_info();

        // Find the appropriate asset based on platform and architecture
        let asset_url = self
          .find_brave_asset(&release.assets, &os, &arch)
          .ok_or(format!(
            "No compatible asset found for Brave version {version} on {os}/{arch}"
          ))?;

        Ok(asset_url)
      }
      BrowserType::Zen => {
        // For Zen, verify the asset exists and handle different naming patterns
        let releases = match self.api_client.fetch_zen_releases_with_caching(true).await {
          Ok(releases) => releases,
          Err(e) => {
            eprintln!("Failed to fetch Zen releases: {e}");
            return Err(format!("Failed to fetch Zen releases from GitHub API: {e}. This might be due to GitHub API rate limiting or network issues. Please try again later.").into());
          }
        };

        let release = releases
          .iter()
          .find(|r| r.tag_name == version)
          .ok_or_else(|| {
            format!(
              "Zen version {} not found. Available versions: {}",
              version,
              releases
                .iter()
                .take(5)
                .map(|r| r.tag_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
            )
          })?;

        // Get platform and architecture info
        let (os, arch) = Self::get_platform_info();

        // Find the appropriate asset
        let asset_url = self
          .find_zen_asset(&release.assets, &os, &arch)
          .ok_or_else(|| {
            let available_assets: Vec<&str> =
              release.assets.iter().map(|a| a.name.as_str()).collect();
            format!(
              "No compatible asset found for Zen version {} on {}/{}. Available assets: {}",
              version,
              os,
              arch,
              available_assets.join(", ")
            )
          })?;

        Ok(asset_url)
      }
      BrowserType::MullvadBrowser => {
        // For Mullvad, verify the asset exists
        let releases = self
          .api_client
          .fetch_mullvad_releases_with_caching(true)
          .await?;

        let release = releases
          .iter()
          .find(|r| r.tag_name == version)
          .ok_or(format!("Mullvad version {version} not found"))?;

        // Get platform and architecture info
        let (os, arch) = Self::get_platform_info();

        // Find the appropriate asset
        let asset_url = self
          .find_mullvad_asset(&release.assets, &os, &arch)
          .ok_or(format!(
            "No compatible asset found for Mullvad version {version} on {os}/{arch}"
          ))?;

        Ok(asset_url)
      }
      BrowserType::Camoufox => {
        // For Camoufox, verify the asset exists and find the correct download URL
        let releases = self
          .api_client
          .fetch_camoufox_releases_with_caching(true)
          .await?;

        let release = releases
          .iter()
          .find(|r| r.tag_name == version)
          .ok_or(format!("Camoufox version {version} not found"))?;

        // Get platform and architecture info
        let (os, arch) = Self::get_platform_info();

        // Find the appropriate asset
        let asset_url = self
          .find_camoufox_asset(&release.assets, &os, &arch)
          .ok_or(format!(
            "No compatible asset found for Camoufox version {version} on {os}/{arch}"
          ))?;

        Ok(asset_url)
      }
      _ => {
        // For other browsers, use the provided URL
        Ok(download_info.url.clone())
      }
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

  /// Find the appropriate Brave asset for the current platform and architecture
  fn find_brave_asset(
    &self,
    assets: &[crate::browser::GithubAsset],
    os: &str,
    arch: &str,
  ) -> Option<String> {
    // Brave asset naming patterns:
    // Windows: BraveBrowserStandaloneNightlySetup.exe, BraveBrowserStandaloneSilentNightlySetup.exe
    // macOS: Brave-Browser-Nightly-universal.dmg, Brave-Browser-Nightly-universal.pkg
    // Linux: brave-browser-1.79.119-linux-arm64.zip, brave-browser-1.79.119-linux-amd64.zip

    let asset = match os {
      "windows" => {
        // For Windows, look for standalone setup EXE (not the auto-updater one)
        assets
          .iter()
          .find(|asset| {
            let name = asset.name.to_lowercase();
            name.contains("standalone") && name.ends_with(".exe") && !name.contains("silent")
          })
          .or_else(|| {
            // Fallback to any EXE if standalone not found
            assets.iter().find(|asset| asset.name.ends_with(".exe"))
          })
      }
      "macos" => {
        // For macOS, prefer universal DMG
        assets
          .iter()
          .find(|asset| {
            let name = asset.name.to_lowercase();
            name.contains("universal") && name.ends_with(".dmg")
          })
          .or_else(|| {
            // Fallback to any DMG
            assets.iter().find(|asset| asset.name.ends_with(".dmg"))
          })
      }
      "linux" => {
        // For Linux, be strict about architecture matching - same logic as has_compatible_brave_asset
        let arch_pattern = if arch == "arm64" { "arm64" } else { "amd64" };

        assets.iter().find(|asset| {
          let name = asset.name.to_lowercase();
          name.contains("linux") && name.contains(arch_pattern) && name.ends_with(".zip")
        })
      }
      _ => None,
    };

    asset.map(|a| a.browser_download_url.clone())
  }

  /// Find the appropriate Zen asset for the current platform and architecture
  fn find_zen_asset(
    &self,
    assets: &[crate::browser::GithubAsset],
    os: &str,
    arch: &str,
  ) -> Option<String> {
    // Zen asset naming patterns:
    // Windows: zen.installer.exe, zen.installer-arm64.exe
    // macOS: zen.macos-universal.dmg
    // Linux: zen.linux-x86_64.tar.xz, zen.linux-aarch64.tar.xz, zen-x86_64.AppImage, zen-aarch64.AppImage

    let asset = match (os, arch) {
      ("windows", "x64") => assets
        .iter()
        .find(|asset| asset.name == "zen.installer.exe"),
      ("windows", "arm64") => assets
        .iter()
        .find(|asset| asset.name == "zen.installer-arm64.exe"),
      ("macos", _) => assets
        .iter()
        .find(|asset| asset.name == "zen.macos-universal.dmg"),
      ("linux", "x64") => {
        // Prefer tar.xz, fallback to AppImage
        assets
          .iter()
          .find(|asset| asset.name == "zen.linux-x86_64.tar.xz")
          .or_else(|| {
            assets
              .iter()
              .find(|asset| asset.name == "zen-x86_64.AppImage")
          })
      }
      ("linux", "arm64") => {
        // Prefer tar.xz, fallback to AppImage
        assets
          .iter()
          .find(|asset| asset.name == "zen.linux-aarch64.tar.xz")
          .or_else(|| {
            assets
              .iter()
              .find(|asset| asset.name == "zen-aarch64.AppImage")
          })
      }
      _ => None,
    };

    asset.map(|a| a.browser_download_url.clone())
  }

  /// Find the appropriate Mullvad asset for the current platform and architecture
  fn find_mullvad_asset(
    &self,
    assets: &[crate::browser::GithubAsset],
    os: &str,
    arch: &str,
  ) -> Option<String> {
    // Mullvad asset naming patterns:
    // Windows: mullvad-browser-windows-x86_64-VERSION.exe
    // macOS: mullvad-browser-macos-VERSION.dmg
    // Linux: mullvad-browser-x86_64-VERSION.tar.xz

    let asset = match (os, arch) {
      ("windows", "x64") => assets.iter().find(|asset| {
        asset.name.contains("windows")
          && asset.name.contains("x86_64")
          && asset.name.ends_with(".exe")
      }),
      ("windows", "arm64") => {
        // Mullvad doesn't support ARM64 on Windows
        None
      }
      ("macos", _) => assets
        .iter()
        .find(|asset| asset.name.contains("macos") && asset.name.ends_with(".dmg")),
      ("linux", "x64") => assets.iter().find(|asset| {
        asset.name.contains("x86_64")
          && asset.name.ends_with(".tar.xz")
          && !asset.name.contains("windows")
      }),
      ("linux", "arm64") => {
        // Mullvad doesn't support ARM64 on Linux
        None
      }
      _ => None,
    };

    asset.map(|a| a.browser_download_url.clone())
  }

  /// Find the appropriate Camoufox asset for the current platform and architecture
  fn find_camoufox_asset(
    &self,
    assets: &[crate::browser::GithubAsset],
    os: &str,
    arch: &str,
  ) -> Option<String> {
    // Camoufox asset naming pattern: camoufox-{version}-{release}-{os}.{arch}.zip
    let (os_name, arch_name) = match (os, arch) {
      ("windows", "x64") => ("win", "x86_64"),
      ("windows", "arm64") => ("win", "arm64"),
      ("linux", "x64") => ("lin", "x86_64"),
      ("linux", "arm64") => ("lin", "arm64"),
      ("macos", "x64") => ("mac", "x86_64"),
      ("macos", "arm64") => ("mac", "arm64"),
      _ => return None,
    };

    // Look for assets matching the pattern
    let asset = assets.iter().find(|asset| {
      let name = asset.name.to_lowercase();
      name.starts_with("camoufox-")
        && name.contains(&format!("-{os_name}.{arch_name}.zip"))
        && name.ends_with(".zip")
    });

    asset.map(|a| a.browser_download_url.clone())
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

    // Check if this is a twilight release for special handling
    let is_twilight =
      browser_type == BrowserType::Zen && version.to_lowercase().contains("twilight");

    // Determine if we have a partial file to resume
    let mut existing_size: u64 = 0;
    if let Ok(meta) = std::fs::metadata(&file_path) {
      existing_size = meta.len();
    }

    // Build request, add Range only if we have bytes. If the server responds with 416 (Range Not
    // Satisfiable), delete the partial file and retry once without the Range header.
    let response = {
      let mut request = self
        .client
        .get(&download_url)
        .header(
          "User-Agent",
          "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        );

      if existing_size > 0 {
        request = request.header("Range", format!("bytes={existing_size}-"));
      }

      let first = request.send().await?;

      if first.status().as_u16() == 416 && existing_size > 0 {
        // Partial file on disk is not acceptable to the server — remove it and retry from scratch
        let _ = std::fs::remove_file(&file_path);
        existing_size = 0;

        let retry = self
          .client
          .get(&download_url)
          .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
          )
          .send()
          .await?;
        retry
      } else {
        first
      }
    };

    // Check if the response is successful (200 OK or 206 Partial Content)
    if !(response.status().is_success() || response.status().as_u16() == 206) {
      return Err(format!("Download failed with status: {}", response.status()).into());
    }

    // Determine total size
    let mut total_size = response.content_length();

    // If resuming (206) and Content-Range is present, parse total
    if response.status().as_u16() == 206 {
      if let Some(content_range) = response.headers().get(reqwest::header::CONTENT_RANGE) {
        if let Ok(cr) = content_range.to_str() {
          // Format: bytes start-end/total
          if let Some((_, total_str)) = cr.split('/').collect::<Vec<_>>().split_first() {
            if let Some(total_str) = total_str.first() {
              if let Ok(total) = total_str.parse::<u64>() {
                total_size = Some(total);
              }
            }
          }
        }
      } else if let Some(len) = response.headers().get(reqwest::header::CONTENT_LENGTH) {
        // Fallback: total = existing + incoming length
        if let Ok(len_str) = len.to_str() {
          if let Ok(incoming) = len_str.parse::<u64>() {
            total_size = Some(existing_size + incoming);
          }
        }
      }
    } else if existing_size > 0 && response.status().is_success() {
      // Server ignored range or we asked from 0; if 200 and existing file has content, start fresh
      // Truncate existing file so we don't append duplicate bytes
      let _ = std::fs::remove_file(&file_path);
      existing_size = 0;
    }

    let mut downloaded = existing_size;
    let start_time = std::time::Instant::now();
    let mut last_update = start_time;

    // Emit initial progress AFTER we've established total size and resume state
    let initial_percentage = if let Some(total) = total_size {
      if total > 0 {
        (existing_size as f64 / total as f64) * 100.0
      } else {
        0.0
      }
    } else {
      0.0
    };

    let initial_stage = if is_twilight {
      "downloading (twilight rolling release)".to_string()
    } else {
      "downloading".to_string()
    };

    let progress = DownloadProgress {
      browser: browser_type.as_str().to_string(),
      version: version.to_string(),
      downloaded_bytes: existing_size,
      total_bytes: total_size,
      percentage: initial_percentage,
      speed_bytes_per_sec: 0.0,
      eta_seconds: None,
      stage: initial_stage,
    };

    let _ = app_handle.emit("download-progress", &progress);

    // Open file in append mode (resuming) or create new
    use std::fs::OpenOptions;
    let mut file = OpenOptions::new()
      .create(true)
      .append(true)
      .open(&file_path)?;
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
        // Compute speed based only on bytes downloaded in this session to avoid inflated values when resuming
        let downloaded_since_start = downloaded.saturating_sub(existing_size);
        let speed = if elapsed > 0.0 {
          downloaded_since_start as f64 / elapsed
        } else {
          0.0
        };
        let percentage = if let Some(total) = total_size {
          if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
          } else {
            0.0
          }
        } else {
          0.0
        };
        let eta = if speed > 0.0 {
          total_size.map(|total| (total - downloaded) as f64 / speed)
        } else {
          None
        };

        let stage_description = if is_twilight {
          "downloading (twilight rolling release)".to_string()
        } else {
          "downloading".to_string()
        };

        let progress = DownloadProgress {
          browser: browser_type.as_str().to_string(),
          version: version.to_string(),
          downloaded_bytes: downloaded,
          total_bytes: total_size,
          percentage,
          speed_bytes_per_sec: speed,
          eta_seconds: eta,
          stage: stage_description,
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
  use crate::browser_version_manager::DownloadInfo;

  use tempfile::TempDir;
  use wiremock::matchers::{method, path};
  use wiremock::{Mock, MockServer, ResponseTemplate};

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
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_bytes(test_content)
          .insert_header("content-length", test_content.len().to_string())
          .insert_header("content-type", "application/octet-stream"),
      )
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
      .respond_with(
        ResponseTemplate::new(200)
          .set_body_bytes(test_content.clone())
          .insert_header("content-length", test_content.len().to_string())
          .insert_header("content-type", "application/octet-stream"),
      )
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

// Global singleton instance
lazy_static::lazy_static! {
  static ref DOWNLOADER: Downloader = Downloader::new();
}

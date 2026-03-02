use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::api_client::ApiClient;
use crate::browser::{create_browser, BrowserType};
use crate::browser_version_manager::DownloadInfo;
use crate::events;

// Global state to track currently downloading browser-version pairs
lazy_static::lazy_static! {
  static ref DOWNLOADING_BROWSERS: std::sync::Arc<Mutex<std::collections::HashSet<String>>> =
    std::sync::Arc::new(Mutex::new(std::collections::HashSet::new()));
  static ref DOWNLOAD_CANCELLATION_TOKENS: std::sync::Arc<Mutex<std::collections::HashMap<String, CancellationToken>>> =
    std::sync::Arc::new(Mutex::new(std::collections::HashMap::new()));
}

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
  registry: &'static crate::downloaded_browsers_registry::DownloadedBrowsersRegistry,
  version_service: &'static crate::browser_version_manager::BrowserVersionManager,
  extractor: &'static crate::extraction::Extractor,
  geoip_downloader: &'static crate::geoip_downloader::GeoIPDownloader,
}

impl Downloader {
  fn new() -> Self {
    Self {
      client: Client::new(),
      api_client: ApiClient::instance(),
      registry: crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance(),
      version_service: crate::browser_version_manager::BrowserVersionManager::instance(),
      extractor: crate::extraction::Extractor::instance(),
      geoip_downloader: crate::geoip_downloader::GeoIPDownloader::instance(),
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
      registry: crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance(),
      version_service: crate::browser_version_manager::BrowserVersionManager::instance(),
      extractor: crate::extraction::Extractor::instance(),
      geoip_downloader: crate::geoip_downloader::GeoIPDownloader::instance(),
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
            log::error!("Failed to fetch Zen releases: {e}");
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
      BrowserType::Camoufox => {
        // For Camoufox, verify the asset exists and find the correct download URL
        let releases = self
          .api_client
          .fetch_camoufox_releases_with_caching(true)
          .await?;

        let release = releases
          .iter()
          .find(|r| r.tag_name == version)
          .or_else(|| {
            log::info!("Camoufox: requested version {version} not found, using latest available");
            releases.first()
          })
          .ok_or("No Camoufox releases found".to_string())?;

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
      BrowserType::Wayfern => {
        // For Wayfern, get the download URL from version.json
        let version_info = self
          .api_client
          .fetch_wayfern_version_with_caching(true)
          .await?;

        if version_info.version != version {
          log::info!(
            "Wayfern: requested version {version}, using available version {}",
            version_info.version
          );
        }

        // Get the download URL for current platform
        let download_url = self
          .api_client
          .get_wayfern_download_url(&version_info)
          .ok_or_else(|| {
            let (os, arch) = Self::get_platform_info();
            format!(
              "No compatible download found for Wayfern on {os}/{arch}. Available platforms: {}",
              version_info
                .downloads
                .iter()
                .filter_map(|(k, v)| if v.is_some() { Some(k.as_str()) } else { None })
                .collect::<Vec<_>>()
                .join(", ")
            )
          })?;

        Ok(download_url)
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

  /// Find the appropriate Camoufox asset for the current platform and architecture
  fn find_camoufox_asset(
    &self,
    assets: &[crate::browser::GithubAsset],
    os: &str,
    arch: &str,
  ) -> Option<String> {
    // Camoufox asset naming pattern: camoufox-{version}-beta.{number}-{os}.{arch}.zip
    // Example: camoufox-135.0.1-beta.24-lin.x86_64.zip
    let (os_name, arch_name) = match (os, arch) {
      ("windows", "x64") => ("win", "x86_64"),
      ("windows", "arm64") => ("win", "arm64"),
      ("linux", "x64") => ("lin", "x86_64"),
      ("linux", "arm64") => ("lin", "arm64"),
      ("macos", "x64") => ("mac", "x86_64"),
      ("macos", "arm64") => ("mac", "arm64"),
      _ => return None,
    };

    // Use ends_with for precise matching to avoid false positives
    // The separator before OS is a dash: -lin.x86_64.zip, -mac.arm64.zip, etc.
    let pattern = format!("-{os_name}.{arch_name}.zip");
    let asset = assets.iter().find(|asset| {
      let name = asset.name.to_lowercase();
      name.starts_with("camoufox-") && name.ends_with(&pattern)
    });

    if let Some(asset) = asset {
      log::info!(
        "Selected Camoufox asset for {}/{}: {}",
        os,
        arch,
        asset.name
      );
      Some(asset.browser_download_url.clone())
    } else {
      log::warn!(
        "No matching Camoufox asset found for {}/{} with pattern '{}'. Available assets: {:?}",
        os,
        arch,
        pattern,
        assets.iter().map(|a| &a.name).collect::<Vec<_>>()
      );
      None
    }
  }

  /// Ensure version.json exists in the Camoufox installation directory.
  /// Creates the file if it doesn't exist, using the version from the tag name.
  async fn ensure_camoufox_version_json(
    &self,
    browser_dir: &Path,
    version: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // The browser_dir is typically: binaries/camoufox/<version>/
    // Find the executable directory within it
    let version_json_locations = vec![
      browser_dir.join("version.json"),
      browser_dir.join("camoufox").join("version.json"),
    ];

    // Check if version.json already exists in any expected location
    for location in &version_json_locations {
      if location.exists() {
        log::info!("version.json already exists at: {}", location.display());
        return Ok(());
      }
    }

    // Parse the Firefox version from the Camoufox version tag
    // Format: "135.0.1-beta.24" -> Firefox version is "135.0.1" (or just "135.0")
    let firefox_version = version.split('-').next().unwrap_or(version);

    // Create version.json in the browser directory
    let version_json_path = browser_dir.join("version.json");
    let version_data = serde_json::json!({
      "version": firefox_version
    });

    let version_json_str = serde_json::to_string_pretty(&version_data)?;
    tokio::fs::write(&version_json_path, version_json_str).await?;

    log::info!(
      "Created version.json at {} with Firefox version: {}",
      version_json_path.display(),
      firefox_version
    );

    Ok(())
  }

  fn configure_camoufox_search_engine(
    &self,
    browser_dir: &Path,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    configure_camoufox_search_engine(browser_dir)
  }

  pub async fn download_browser<R: tauri::Runtime>(
    &self,
    _app_handle: &tauri::AppHandle<R>,
    browser_type: BrowserType,
    version: &str,
    download_info: &DownloadInfo,
    dest_path: &Path,
    cancel_token: Option<&CancellationToken>,
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

    let _ = events::emit("download-progress", &progress);

    // Open file in append mode (resuming) or create new
    use std::fs::OpenOptions;
    let mut file = OpenOptions::new()
      .create(true)
      .append(true)
      .open(&file_path)?;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
      if let Some(token) = cancel_token {
        if token.is_cancelled() {
          drop(file);
          let _ = std::fs::remove_file(&file_path);
          return Err("Download cancelled".into());
        }
      }
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

        let _ = events::emit("download-progress", &progress);
        last_update = now;
      }
    }

    Ok(file_path)
  }

  /// Download a browser binary, verify it, and register it in the downloaded browsers registry
  pub async fn download_browser_full(
    &self,
    app_handle: &tauri::AppHandle,
    browser_str: String,
    version: String,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Only check Wayfern terms if Wayfern is already downloaded
    let terms_manager = crate::wayfern_terms::WayfernTermsManager::instance();
    if terms_manager.is_wayfern_downloaded() && !terms_manager.is_terms_accepted() {
      return Err("Please accept Wayfern Terms and Conditions before downloading browsers".into());
    }

    // For Wayfern/Camoufox, resolve the actual available version from the API
    let version = if browser_str == "wayfern" {
      match self
        .api_client
        .fetch_wayfern_version_with_caching(true)
        .await
      {
        Ok(info) if info.version != version => {
          log::info!(
            "Wayfern: requested {version}, using available {}",
            info.version
          );
          info.version
        }
        _ => version,
      }
    } else if browser_str == "camoufox" {
      match self
        .api_client
        .fetch_camoufox_releases_with_caching(true)
        .await
      {
        Ok(releases) if !releases.is_empty() && releases[0].tag_name != version => {
          log::info!(
            "Camoufox: requested {version}, using available {}",
            releases[0].tag_name
          );
          releases[0].tag_name.clone()
        }
        _ => version,
      }
    } else {
      version
    };

    // Check if this browser-version pair is already being downloaded
    let download_key = format!("{browser_str}-{version}");
    let cancel_token = {
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      if downloading.contains(&download_key) {
        return Err(format!("Browser '{browser_str}' version '{version}' is already being downloaded. Please wait for the current download to complete.").into());
      }
      // Mark this browser-version pair as being downloaded
      downloading.insert(download_key.clone());

      let token = CancellationToken::new();
      let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
      tokens.insert(download_key.clone(), token.clone());
      token
    };

    let browser_type =
      BrowserType::from_str(&browser_str).map_err(|e| format!("Invalid browser type: {e}"))?;
    let browser = create_browser(browser_type.clone());

    // Use injected registry instance

    let binaries_dir = crate::app_dirs::binaries_dir();

    // Check if registry thinks it's downloaded, but also verify files actually exist
    if self.registry.is_browser_downloaded(&browser_str, &version) {
      let actually_exists = browser.is_version_downloaded(&version, &binaries_dir);

      if actually_exists {
        // Remove from downloading set since it's already downloaded
        let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
        downloading.remove(&download_key);
        drop(downloading);
        let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
        tokens.remove(&download_key);
        return Ok(version);
      } else {
        // Registry says it's downloaded but files don't exist - clean up registry
        log::info!("Registry indicates {browser_str} {version} is downloaded, but files are missing. Cleaning up registry entry.");
        self.registry.remove_browser(&browser_str, &version);
        self
          .registry
          .save()
          .map_err(|e| format!("Failed to save cleaned registry: {e}"))?;
      }
    }

    // Check if browser is supported on current platform before attempting download
    if !self
      .version_service
      .is_browser_supported(&browser_str)
      .unwrap_or(false)
    {
      // Remove from downloading set on error
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      downloading.remove(&download_key);
      drop(downloading);
      let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
      tokens.remove(&download_key);
      return Err(
        format!(
          "Browser '{}' is not supported on your platform ({} {}). Supported browsers: {}",
          browser_str,
          std::env::consts::OS,
          std::env::consts::ARCH,
          self.version_service.get_supported_browsers().join(", ")
        )
        .into(),
      );
    }

    let download_info = self
      .version_service
      .get_download_info(&browser_str, &version)
      .map_err(|e| format!("Failed to get download info: {e}"))?;

    // Create browser directory
    let mut browser_dir = binaries_dir.clone();
    browser_dir.push(&browser_str);
    browser_dir.push(&version);

    std::fs::create_dir_all(&browser_dir)
      .map_err(|e| format!("Failed to create browser directory: {e}"))?;

    // Mark download as started (but don't add to registry yet)
    self
      .registry
      .mark_download_started(&browser_str, &version, browser_dir.clone());

    // Attempt to download the archive. If the download fails but an archive with the
    // expected filename already exists (manual download), continue using that file.
    let download_path: PathBuf = match self
      .download_browser(
        app_handle,
        browser_type.clone(),
        &version,
        &download_info,
        &browser_dir,
        Some(&cancel_token),
      )
      .await
    {
      Ok(path) => path,
      Err(e) => {
        // Do NOT continue with extraction on failed downloads. Partial files may exist but are invalid.
        // Clean registry entry and stop here so the UI can show a single, clear error.
        let _ = self.registry.remove_browser(&browser_str, &version);
        let _ = self.registry.save();
        let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
        downloading.remove(&download_key);
        drop(downloading);
        let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
        tokens.remove(&download_key);

        // Emit cancelled stage if the download was cancelled by user
        if cancel_token.is_cancelled() {
          let progress = DownloadProgress {
            browser: browser_str.clone(),
            version: version.clone(),
            downloaded_bytes: 0,
            total_bytes: None,
            percentage: 0.0,
            speed_bytes_per_sec: 0.0,
            eta_seconds: None,
            stage: "cancelled".to_string(),
          };
          let _ = events::emit("download-progress", &progress);
        }

        return Err(format!("Failed to download browser: {e}").into());
      }
    };

    // Use the extraction module
    if download_info.is_archive {
      match self
        .extractor
        .extract_browser(
          app_handle,
          browser_type.clone(),
          &version,
          &download_path,
          &browser_dir,
        )
        .await
      {
        Ok(_) => {
          // Do not remove the archive here. We keep it until verification succeeds.
        }
        Err(e) => {
          // Do not remove the archive or extracted files. Just drop the registry entry
          // so it won't be reported as downloaded.
          let _ = self.registry.remove_browser(&browser_str, &version);
          let _ = self.registry.save();
          // Remove browser-version pair from downloading set on error
          {
            let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
            downloading.remove(&download_key);
          }
          {
            let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
            tokens.remove(&download_key);
          }
          return Err(format!("Failed to extract browser: {e}").into());
        }
      }

      // Give filesystem a moment to settle after extraction
      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Emit verification progress
    let progress = DownloadProgress {
      browser: browser_str.clone(),
      version: version.clone(),
      downloaded_bytes: 0,
      total_bytes: None,
      percentage: 100.0,
      speed_bytes_per_sec: 0.0,
      eta_seconds: None,
      stage: "verifying".to_string(),
    };
    let _ = events::emit("download-progress", &progress);

    // Verify the browser was downloaded correctly
    log::info!("Verifying download for browser: {browser_str}, version: {version}");

    // Use the browser's own verification method
    if !browser.is_version_downloaded(&version, &binaries_dir) {
      // Provide detailed error information for debugging
      let browser_dir = binaries_dir.join(&browser_str).join(&version);
      let mut error_details = format!(
        "Browser download completed but verification failed for {} {}. Expected directory: {}",
        browser_str,
        version,
        browser_dir.display()
      );

      // List what files actually exist
      if browser_dir.exists() {
        error_details.push_str("\nFiles found in directory:");
        if let Ok(entries) = std::fs::read_dir(&browser_dir) {
          for entry in entries.flatten() {
            let path = entry.path();
            let file_type = if path.is_dir() { "DIR" } else { "FILE" };
            error_details.push_str(&format!("\n  {} {}", file_type, path.display()));
          }
        } else {
          error_details.push_str("\n  (Could not read directory contents)");
        }
      } else {
        error_details.push_str("\nDirectory does not exist!");
      }

      // For Camoufox on Linux, provide specific expected files
      if browser_str == "camoufox" && cfg!(target_os = "linux") {
        let camoufox_subdir = browser_dir.join("camoufox");
        error_details.push_str("\nExpected Camoufox executable locations:");
        error_details.push_str(&format!("\n  {}/camoufox-bin", camoufox_subdir.display()));
        error_details.push_str(&format!("\n  {}/camoufox", camoufox_subdir.display()));

        if camoufox_subdir.exists() {
          error_details.push_str(&format!(
            "\nCamoufox subdirectory exists: {}",
            camoufox_subdir.display()
          ));
          if let Ok(entries) = std::fs::read_dir(&camoufox_subdir) {
            error_details.push_str("\nFiles in camoufox subdirectory:");
            for entry in entries.flatten() {
              let path = entry.path();
              let file_type = if path.is_dir() { "DIR" } else { "FILE" };
              error_details.push_str(&format!("\n  {} {}", file_type, path.display()));
            }
          }
        } else {
          error_details.push_str(&format!(
            "\nCamoufox subdirectory does not exist: {}",
            camoufox_subdir.display()
          ));
        }
      }

      // Do not delete files on verification failure; keep archive for manual retry.
      let _ = self.registry.remove_browser(&browser_str, &version);
      let _ = self.registry.save();
      // Remove browser-version pair from downloading set on verification failure
      {
        let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
        downloading.remove(&download_key);
      }
      {
        let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
        tokens.remove(&download_key);
      }
      return Err(error_details.into());
    }

    // Mark completion in registry - only now add to registry after verification
    if let Err(e) =
      self
        .registry
        .mark_download_completed(&browser_str, &version, browser_dir.clone())
    {
      log::warn!("Warning: Could not mark {browser_str} {version} as completed in registry: {e}");
    }
    self
      .registry
      .save()
      .map_err(|e| format!("Failed to save registry: {e}"))?;

    // Now that verification succeeded, remove the archive file if it exists
    if download_info.is_archive {
      let archive_path = browser_dir.join(&download_info.filename);
      if archive_path.exists() {
        if let Err(e) = std::fs::remove_file(&archive_path) {
          log::warn!("Warning: Could not delete archive file after verification: {e}");
        }
      }
    }

    // If this is Camoufox, automatically download GeoIP database and create version.json
    if browser_str == "camoufox" {
      // Check if GeoIP database is already available
      if !crate::geoip_downloader::GeoIPDownloader::is_geoip_database_available() {
        log::info!("Downloading GeoIP database for Camoufox...");

        match self
          .geoip_downloader
          .download_geoip_database(app_handle)
          .await
        {
          Ok(_) => {
            log::info!("GeoIP database downloaded successfully");
          }
          Err(e) => {
            log::error!("Failed to download GeoIP database: {e}");
            // Don't fail the browser download if GeoIP download fails
          }
        }
      } else {
        log::info!("GeoIP database already available");
      }

      // Create version.json if it doesn't exist
      if let Err(e) = self
        .ensure_camoufox_version_json(&browser_dir, &version)
        .await
      {
        log::warn!("Failed to create version.json for Camoufox: {e}");
      }

      if let Err(e) = self.configure_camoufox_search_engine(&browser_dir) {
        log::warn!("Failed to configure Camoufox search engine: {e}");
      }
    }

    // Emit completion
    let progress = DownloadProgress {
      browser: browser_str.clone(),
      version: version.clone(),
      downloaded_bytes: 0,
      total_bytes: None,
      percentage: 100.0,
      speed_bytes_per_sec: 0.0,
      eta_seconds: Some(0.0),
      stage: "completed".to_string(),
    };
    let _ = events::emit("download-progress", &progress);

    // Remove browser-version pair from downloading set and cancel token
    {
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      downloading.remove(&download_key);
    }
    {
      let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
      tokens.remove(&download_key);
    }

    // Auto-update non-running profiles to the new version and cleanup unused binaries
    {
      let browser_for_update = browser_str.clone();
      let version_for_update = version.clone();
      let app_handle_for_update = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        let auto_updater = crate::auto_updater::AutoUpdater::instance();
        match auto_updater
          .auto_update_profile_versions(
            &app_handle_for_update,
            &browser_for_update,
            &version_for_update,
          )
          .await
        {
          Ok(updated) => {
            if !updated.is_empty() {
              log::info!(
                "Auto-updated {} profiles to {} {}: {:?}",
                updated.len(),
                browser_for_update,
                version_for_update,
                updated
              );
            }
          }
          Err(e) => {
            log::error!("Failed to auto-update profile versions: {e}");
          }
        }

        let registry = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
        match registry.cleanup_unused_binaries() {
          Ok(cleaned) => {
            if !cleaned.is_empty() {
              log::info!("Cleaned up unused binaries after download: {:?}", cleaned);
            }
          }
          Err(e) => {
            log::error!("Failed to cleanup unused binaries: {e}");
          }
        }
      });
    }

    Ok(version)
  }
}

#[tauri::command]
pub async fn download_browser(
  app_handle: tauri::AppHandle,
  browser_str: String,
  version: String,
) -> Result<String, String> {
  let downloader = Downloader::instance();
  downloader
    .download_browser_full(&app_handle, browser_str, version)
    .await
    .map_err(|e| format!("Failed to download browser: {e}"))
}

#[tauri::command]
pub async fn cancel_download(browser_str: String, version: String) -> Result<(), String> {
  let download_key = format!("{browser_str}-{version}");
  let token = {
    let tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
    tokens.get(&download_key).cloned()
  };

  if let Some(token) = token {
    token.cancel();
    Ok(())
  } else {
    Err(format!(
      "No active download found for {browser_str} {version}"
    ))
  }
}

/// Find all candidate `distribution/` directories inside the Camoufox browser dir.
/// On macOS: `<browser_dir>/<app>.app/Contents/Resources/distribution/`
/// On Linux: `<browser_dir>/camoufox/distribution/`
/// On Windows: `<browser_dir>/distribution/`
/// Also includes `<browser_dir>/distribution/` as a fallback for all platforms.
fn find_camoufox_distribution_dirs(browser_dir: &Path) -> Vec<std::path::PathBuf> {
  let mut dirs = Vec::new();

  #[cfg(target_os = "macos")]
  {
    if let Ok(entries) = std::fs::read_dir(browser_dir) {
      for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "app") {
          dirs.push(
            entry
              .path()
              .join("Contents")
              .join("Resources")
              .join("distribution"),
          );
        }
      }
    }
  }

  #[cfg(target_os = "linux")]
  {
    let camoufox_subdir = browser_dir.join("camoufox").join("distribution");
    dirs.push(camoufox_subdir);
  }

  // Fallback for all platforms
  dirs.push(browser_dir.join("distribution"));

  dirs
}

/// Set DuckDuckGo as the default search engine in Camoufox.
/// Creates or updates distribution/policies.json with a proper DuckDuckGo engine definition.
/// Called both at download time and at launch time to cover existing installations.
pub fn configure_camoufox_search_engine(
  browser_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let distribution_dirs = find_camoufox_distribution_dirs(browser_dir);

  // Find an existing policies.json, or pick the first candidate dir to create one
  let (policies_path, mut policies) = {
    let mut found = None;
    for dir in &distribution_dirs {
      let path = dir.join("policies.json");
      if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
          if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            found = Some((path, val));
            break;
          }
        }
      }
    }
    match found {
      Some(f) => f,
      None => {
        // Pick the first candidate directory that exists (or can be created)
        let target_dir = distribution_dirs
          .iter()
          .find(|d| d.parent().is_some_and(|p| p.exists()))
          .or(distribution_dirs.first())
          .ok_or("No suitable distribution directory found")?;
        std::fs::create_dir_all(target_dir)?;
        (
          target_dir.join("policies.json"),
          serde_json::json!({"policies": {}}),
        )
      }
    }
  };

  // Check if already configured
  let has_ddg_default = policies
    .get("policies")
    .and_then(|p| p.get("SearchEngines"))
    .and_then(|se| se.get("Default"))
    .and_then(|d| d.as_str())
    == Some("DuckDuckGo");

  let has_ddg_engine = policies
    .get("policies")
    .and_then(|p| p.get("SearchEngines"))
    .and_then(|se| se.get("Add"))
    .and_then(|a| a.as_array())
    .is_some_and(|arr| {
      arr
        .iter()
        .any(|e| e.get("Name").and_then(|n| n.as_str()) == Some("DuckDuckGo"))
    });

  if has_ddg_default && has_ddg_engine {
    return Ok(());
  }

  let ddg_engine = serde_json::json!({
    "Name": "DuckDuckGo",
    "URLTemplate": "https://duckduckgo.com/?q={searchTerms}",
    "SuggestURLTemplate": "https://duckduckgo.com/ac/?q={searchTerms}&type=list",
    "Method": "GET",
    "IconURL": "https://duckduckgo.com/favicon.ico",
    "Alias": "ddg"
  });

  // Ensure policies.SearchEngines exists
  let policies_obj = policies
    .as_object_mut()
    .ok_or("Invalid policies.json")?
    .entry("policies")
    .or_insert(serde_json::json!({}));
  let se = policies_obj
    .as_object_mut()
    .ok_or("Invalid policies object")?
    .entry("SearchEngines")
    .or_insert(serde_json::json!({}));

  if let Some(se_obj) = se.as_object_mut() {
    // Set DuckDuckGo as default
    se_obj.insert(
      "Default".to_string(),
      serde_json::Value::String("DuckDuckGo".to_string()),
    );

    // Add DuckDuckGo engine definition if not present
    let add_arr = se_obj
      .entry("Add")
      .or_insert(serde_json::json!([]))
      .as_array_mut()
      .ok_or("SearchEngines.Add is not an array")?;

    // Remove fake "None" engine
    add_arr.retain(|entry| entry.get("Name").and_then(|n| n.as_str()) != Some("None"));

    // Add DuckDuckGo if not already present
    if !add_arr
      .iter()
      .any(|e| e.get("Name").and_then(|n| n.as_str()) == Some("DuckDuckGo"))
    {
      add_arr.push(ddg_engine);
    }

    // Ensure DuckDuckGo is not in the Remove list
    if let Some(remove_arr) = se_obj.get_mut("Remove").and_then(|r| r.as_array_mut()) {
      remove_arr.retain(|v| v.as_str() != Some("DuckDuckGo"));
    }
  }

  let updated = serde_json::to_string_pretty(&policies)?;
  std::fs::write(&policies_path, updated)?;
  log::info!(
    "Configured DuckDuckGo search engine in {}",
    policies_path.display()
  );

  Ok(())
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
        None,
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
        None,
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
        None,
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

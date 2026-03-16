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
  pub fn new_for_test() -> Self {
    Self {
      client: Client::new(),
      api_client: ApiClient::instance(),
      registry: crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance(),
      version_service: crate::browser_version_manager::BrowserVersionManager::instance(),
      extractor: crate::extraction::Extractor::instance(),
      geoip_downloader: crate::geoip_downloader::GeoIPDownloader::instance(),
    }
  }

  #[cfg(test)]
  pub async fn download_file(
    &self,
    download_url: &str,
    dest_path: &Path,
    filename: &str,
  ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let file_path = dest_path.join(filename);

    let response = self
      .client
      .get(download_url)
      .header(
        "User-Agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
      )
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(format!("Download failed with status: {}", response.status()).into());
    }

    let mut file = std::fs::OpenOptions::new()
      .create(true)
      .truncate(true)
      .write(true)
      .open(&file_path)?;

    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
      let chunk = chunk?;
      io::copy(&mut chunk.as_ref(), &mut file)?;
    }

    Ok(file_path)
  }

  /// Resolve the actual download URL for browsers that need dynamic asset resolution
  pub async fn resolve_download_url(
    &self,
    browser_type: BrowserType,
    version: &str,
    _download_info: &DownloadInfo,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match browser_type {
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

    // Check existing file size — if it matches the expected size, skip download
    let mut existing_size: u64 = 0;
    if let Ok(meta) = std::fs::metadata(&file_path) {
      existing_size = meta.len();
    }

    // Do a HEAD request to get the expected file size for skip/resume decisions
    let head_response = self
      .client
      .head(&download_url)
      .header(
        "User-Agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
      )
      .send()
      .await
      .ok();

    let expected_size = head_response.as_ref().and_then(|r| r.content_length());

    // If existing file matches expected size, skip download entirely
    if existing_size > 0 {
      if let Some(expected) = expected_size {
        if existing_size == expected {
          log::info!(
            "Archive {} already exists with correct size ({} bytes), skipping download",
            file_path.display(),
            existing_size
          );
          return Ok(file_path);
        }
      }
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

    let initial_stage = "downloading".to_string();

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

        let stage_description = "downloading".to_string();

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
          log::error!("Extraction failed for {browser_str} {version}: {e}");

          // Delete the corrupt/invalid archive so a fresh download happens next time
          if download_path.exists() {
            log::info!("Deleting corrupt archive: {}", download_path.display());
            let _ = std::fs::remove_file(&download_path);
          }

          let _ = self.registry.remove_browser(&browser_str, &version);
          let _ = self.registry.save();
          {
            let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
            downloading.remove(&download_key);
          }
          {
            let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
            tokens.remove(&download_key);
          }

          // Emit error stage so the UI shows a toast
          let progress = DownloadProgress {
            browser: browser_str.clone(),
            version: version.clone(),
            downloaded_bytes: 0,
            total_bytes: None,
            percentage: 0.0,
            speed_bytes_per_sec: 0.0,
            eta_seconds: None,
            stage: "error".to_string(),
          };
          let _ = events::emit("download-progress", &progress);

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

    // Auto-update non-running profiles to the latest installed version and cleanup unused binaries
    {
      let app_handle_for_update = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        let auto_updater = crate::auto_updater::AutoUpdater::instance();
        match auto_updater.update_profiles_to_latest_installed(&app_handle_for_update) {
          Ok(updated) => {
            if !updated.is_empty() {
              log::info!(
                "Auto-updated {} profiles to latest installed versions: {:?}",
                updated.len(),
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

#[cfg(test)]
mod tests {
  use super::*;

  use tempfile::TempDir;
  use wiremock::matchers::{method, path};
  use wiremock::{Mock, MockServer, ResponseTemplate};

  #[tokio::test]
  async fn test_download_file_with_progress() {
    let server = MockServer::start().await;
    let downloader = Downloader::new_for_test();

    let temp_dir = TempDir::new().unwrap();
    let dest_path = temp_dir.path();

    let test_content = b"This is a test file content for download simulation";

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

    let download_url = format!("{}/test-download", server.uri());

    let result = downloader
      .download_file(&download_url, dest_path, "test-file.dmg")
      .await;

    assert!(result.is_ok());
    let downloaded_file = result.unwrap();
    assert!(downloaded_file.exists());

    let downloaded_content = std::fs::read(&downloaded_file).unwrap();
    assert_eq!(downloaded_content, test_content);
  }

  #[tokio::test]
  async fn test_download_file_network_error() {
    let server = MockServer::start().await;
    let downloader = Downloader::new_for_test();

    let temp_dir = TempDir::new().unwrap();
    let dest_path = temp_dir.path();

    Mock::given(method("GET"))
      .and(path("/missing-file"))
      .respond_with(ResponseTemplate::new(404))
      .mount(&server)
      .await;

    let download_url = format!("{}/missing-file", server.uri());

    let result = downloader
      .download_file(&download_url, dest_path, "missing-file.dmg")
      .await;

    assert!(result.is_err());
  }

  #[tokio::test]
  async fn test_download_file_chunked_response() {
    let server = MockServer::start().await;
    let downloader = Downloader::new_for_test();

    let temp_dir = TempDir::new().unwrap();
    let dest_path = temp_dir.path();

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

    let download_url = format!("{}/chunked-download", server.uri());

    let result = downloader
      .download_file(&download_url, dest_path, "chunked-file.dmg")
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

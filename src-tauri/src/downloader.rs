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

// Maximum time to wait for the next chunk of a streaming download before treating
// the connection as stalled. Converts an indefinite hang into a terminal error so
// the UI can surface it and the caller can move on / retry.
const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

// Global state to track currently downloading browser-version pairs
lazy_static::lazy_static! {
  static ref DOWNLOADING_BROWSERS: std::sync::Arc<Mutex<std::collections::HashSet<String>>> =
    std::sync::Arc::new(Mutex::new(std::collections::HashSet::new()));
  static ref DOWNLOAD_CANCELLATION_TOKENS: std::sync::Arc<Mutex<std::collections::HashMap<String, CancellationToken>>> =
    std::sync::Arc::new(Mutex::new(std::collections::HashMap::new()));
}

/// Clears a browser-version pair from the in-flight download maps on every
/// exit path of `download_browser_full`. A leaked key would permanently report
/// "already being downloaded" for that version until app restart.
struct InFlightDownload(String);

impl Drop for InFlightDownload {
  fn drop(&mut self) {
    if let Ok(mut downloading) = DOWNLOADING_BROWSERS.lock() {
      downloading.remove(&self.0);
    }
    if let Ok(mut tokens) = DOWNLOAD_CANCELLATION_TOKENS.lock() {
      tokens.remove(&self.0);
    }
  }
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
}

impl Downloader {
  fn new() -> Self {
    Self {
      client: Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        // Per-read idle timeout: if the connection stalls mid-stream with no bytes
        // for this long, the read fails instead of hanging forever. This is the
        // transport-level guard; the streaming loop also wraps each read in an
        // explicit tokio timeout as defense-in-depth.
        .read_timeout(STREAM_IDLE_TIMEOUT)
        .build()
        .unwrap_or_else(|_| Client::new()),
      api_client: ApiClient::instance(),
      registry: crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance(),
      version_service: crate::browser_version_manager::BrowserVersionManager::instance(),
      extractor: crate::extraction::Extractor::instance(),
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
      return Err(
        format!(
          "Download failed with HTTP status {}",
          response.status().as_u16()
        )
        .into(),
      );
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
      BrowserType::Wayfern => {
        // For Wayfern, get the download URL from version.json
        let version_info = self
          .api_client
          .fetch_wayfern_version_with_caching(true)
          .await?;

        // Never substitute: downloading the current build into the requested
        // version's directory would register a mislabeled install.
        if version_info.version != version {
          return Err(
            serde_json::json!({
              "code": "WAYFERN_VERSION_NOT_AVAILABLE",
              "params": { "requested": version, "current": version_info.version }
            })
            .to_string()
            .into(),
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
    log::info!(
      "Resolving download URL for {} {}",
      browser_type.as_str(),
      version
    );
    let download_url = self
      .resolve_download_url(browser_type.clone(), version, download_info)
      .await?;
    log::info!("Download URL resolved: {}", download_url);

    // In-session resume: a large (~1GB) download over a flaky connection can
    // drop mid-stream. Rather than surfacing the first stall/chunk error as a
    // terminal failure (which forces the user to re-click and risks the CDN
    // answering 200 = full restart), re-issue a ranged GET and keep appending to
    // the same partial file. `existing_size` is re-read from disk each pass so
    // the Range offset always matches the bytes already flushed.
    let max_send_retries = 5u32;
    let max_stream_restarts = 5u32;
    let mut stream_restarts = 0u32;
    let start_time = std::time::Instant::now();

    use futures_util::StreamExt;
    use std::fs::OpenOptions;
    use std::io::Write;

    loop {
      // Determine how much of a partial file we already have on disk.
      let mut existing_size: u64 = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

      // Build request with retry logic for transient connect/timeout errors.
      let mut response: Option<reqwest::Response> = None;
      for attempt in 0..=max_send_retries {
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

        log::info!("Sending download request (attempt {})...", attempt + 1);
        match request.send().await {
          Ok(resp) => {
            log::info!(
              "Download response received: status={}, content-length={:?}",
              resp.status(),
              resp.content_length()
            );
            if resp.status().as_u16() == 416 && existing_size > 0 {
              // The requested range is past the end of the object. Parse
              // `Content-Range: bytes */total`: if the partial already covers the
              // whole object it is complete (keep it); otherwise it is corrupt or
              // oversized, so discard and restart from scratch.
              let server_total = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.rsplit('/').next())
                .and_then(|t| t.trim().parse::<u64>().ok());
              let partial_is_complete = match server_total {
                Some(total) => existing_size >= total,
                None => true,
              };
              if partial_is_complete {
                log::info!(
                  "Archive {} already complete ({} bytes), skipping download",
                  file_path.display(),
                  existing_size
                );
                return Ok(file_path);
              }
              let _ = std::fs::remove_file(&file_path);
              existing_size = 0;
              log::warn!("Download returned 416 with an incomplete partial, restarting from 0");
              continue;
            }
            response = Some(resp);
            break;
          }
          Err(e) => {
            let is_retryable = e.is_connect() || e.is_timeout() || e.is_request();
            if is_retryable && attempt < max_send_retries {
              let delay = 2u64.pow(attempt.min(4));
              log::warn!(
                "Download attempt {} failed ({}), retrying in {}s...",
                attempt + 1,
                e,
                delay
              );
              tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            } else {
              return Err(format!("Download failed after {} attempts: {}", attempt + 1, e).into());
            }
          }
        }
      }
      let response = response.ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
        "Download failed: no response received".into()
      })?;

      // Check if the response is successful (200 OK or 206 Partial Content)
      if !(response.status().is_success() || response.status().as_u16() == 206) {
        return Err(
          format!(
            "Download failed with HTTP status {}",
            response.status().as_u16()
          )
          .into(),
        );
      }

      // Determine total size
      let mut total_size = response.content_length();

      // If resuming (206) and Content-Range is present, parse total
      if response.status().as_u16() == 206 {
        if let Some(content_range) = response.headers().get(reqwest::header::CONTENT_RANGE) {
          if let Ok(cr) = content_range.to_str() {
            // Format: bytes start-end/total
            if let Some(total) = cr
              .rsplit('/')
              .next()
              .and_then(|t| t.trim().parse::<u64>().ok())
            {
              total_size = Some(total);
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

      // If the existing file already matches the total size, skip the download
      if existing_size > 0 {
        if let Some(total) = total_size {
          if existing_size >= total {
            log::info!(
              "Archive {} already complete ({} bytes), skipping download",
              file_path.display(),
              existing_size
            );
            return Ok(file_path);
          }
        }
      }

      let mut downloaded = existing_size;
      let mut last_update = std::time::Instant::now();

      // Emit initial progress AFTER we've established total size and resume state
      let initial_percentage = match total_size {
        Some(total) if total > 0 => (existing_size as f64 / total as f64) * 100.0,
        _ => 0.0,
      };
      let _ = events::emit(
        "download-progress",
        &DownloadProgress {
          browser: browser_type.as_str().to_string(),
          version: version.to_string(),
          downloaded_bytes: existing_size,
          total_bytes: total_size,
          percentage: initial_percentage,
          speed_bytes_per_sec: 0.0,
          eta_seconds: None,
          stage: "downloading".to_string(),
        },
      );

      // Open file in append mode (resuming) or create new.
      // Wrap in BufWriter with a large buffer to reduce the number of disk writes,
      // which dramatically improves download speed on Windows (NTFS + Defender overhead).
      let raw_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)?;
      let mut file = io::BufWriter::with_capacity(8 * 1024 * 1024, raw_file);
      let mut stream = response.bytes_stream();

      // On a mid-stream failure (idle stall or chunk error) we record it here and,
      // after flushing what we have, decide whether to resume or give up.
      let mut retryable_stream_err: Option<String> = None;

      loop {
        // Wrap each read in an idle timeout so a stalled connection (no bytes flowing)
        // surfaces as a retryable error instead of awaiting forever.
        let next = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await {
          Ok(item) => item,
          Err(_) => {
            retryable_stream_err = Some(format!(
              "Download stalled: no data received for {}s",
              STREAM_IDLE_TIMEOUT.as_secs()
            ));
            break;
          }
        };
        let Some(chunk) = next else {
          break;
        };
        if let Some(token) = cancel_token {
          if token.is_cancelled() {
            let _ = file.flush();
            drop(file);
            let _ = std::fs::remove_file(&file_path);
            return Err("Download cancelled".into());
          }
        }
        let chunk = match chunk {
          Ok(c) => c,
          Err(e) => {
            retryable_stream_err = Some(format!("Download chunk error: {e}"));
            break;
          }
        };
        file.write_all(&chunk)?;
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
          let percentage = match total_size {
            Some(total) if total > 0 => (downloaded as f64 / total as f64) * 100.0,
            _ => 0.0,
          };
          let eta = if speed > 0.0 {
            total_size.map(|total| total.saturating_sub(downloaded) as f64 / speed)
          } else {
            None
          };

          let _ = events::emit(
            "download-progress",
            &DownloadProgress {
              browser: browser_type.as_str().to_string(),
              version: version.to_string(),
              downloaded_bytes: downloaded,
              total_bytes: total_size,
              percentage,
              speed_bytes_per_sec: speed,
              eta_seconds: eta,
              stage: "downloading".to_string(),
            },
          );
          last_update = now;
        }
      }

      // Always flush what we have so a resume (this pass or a later run) starts
      // from the correct on-disk offset.
      file.flush()?;
      drop(file);

      let Some(err) = retryable_stream_err else {
        return Ok(file_path);
      };

      // Re-check cancellation before scheduling a retry.
      if let Some(token) = cancel_token {
        if token.is_cancelled() {
          let _ = std::fs::remove_file(&file_path);
          return Err("Download cancelled".into());
        }
      }
      if stream_restarts >= max_stream_restarts {
        // Keep the partial on disk so a later run (or app restart) can resume.
        return Err(err.into());
      }
      stream_restarts += 1;
      let delay = 2u64.pow(stream_restarts.min(4));
      log::warn!(
        "{} — resuming from {} bytes (restart {}/{}) in {}s",
        err,
        std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0),
        stream_restarts,
        max_stream_restarts,
        delay
      );
      tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
    }
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

    // Validate the browser type before touching the in-flight maps so a bad
    // request can't leave state behind.
    let browser_type =
      BrowserType::from_str(&browser_str).map_err(|e| format!("Invalid browser type: {e}"))?;
    let browser = create_browser(browser_type.clone());

    // For Wayfern, only the currently published version can be fetched.
    // Requesting any other not-yet-downloaded version is an error — silently
    // substituting the latest would install a version the caller never asked
    // for while the response still echoes the requested one. The fetch must
    // succeed too: proceeding unverified would let resolve_download_url fetch
    // the current build into the requested version's directory (a mislabeled
    // install), and that URL resolution needs the same endpoint anyway.
    if browser_str == "wayfern" && !self.registry.is_browser_downloaded(&browser_str, &version) {
      let info = self
        .api_client
        .fetch_wayfern_version_with_caching(true)
        .await
        .map_err(|e| format!("Failed to determine the current Wayfern version: {e}"))?;
      if info.version != version {
        return Err(
          serde_json::json!({
            "code": "WAYFERN_VERSION_NOT_AVAILABLE",
            "params": { "requested": version, "current": info.version }
          })
          .to_string()
          .into(),
        );
      }
    }

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
    // Cleared on drop, whatever exit path this function takes.
    let _in_flight = InFlightDownload(download_key.clone());

    let binaries_dir = crate::app_dirs::binaries_dir();

    // Check if registry thinks it's downloaded, but also verify files actually exist
    if self.registry.is_browser_downloaded(&browser_str, &version) {
      let actually_exists = browser.is_version_downloaded(&version, &binaries_dir);

      if actually_exists {
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

        // Emit a terminal stage so the UI stops spinning. A user cancellation maps to
        // "cancelled"; any other failure (network error, stall timeout, bad status)
        // maps to "error" so the frontend can show a concrete error toast.
        let stage = if cancel_token.is_cancelled() {
          "cancelled"
        } else {
          "error"
        };
        let progress = DownloadProgress {
          browser: browser_str.clone(),
          version: version.clone(),
          downloaded_bytes: 0,
          total_bytes: None,
          percentage: 0.0,
          speed_bytes_per_sec: 0.0,
          eta_seconds: None,
          stage: stage.to_string(),
        };
        let _ = events::emit("download-progress", &progress);

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

      // Do not delete files on verification failure; keep archive for manual retry.
      let _ = self.registry.remove_browser(&browser_str, &version);
      let _ = self.registry.save();

      // Emit a terminal error stage so the UI shows an error instead of spinning.
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

/// Check if a specific browser-version pair is currently being downloaded
pub fn is_downloading(browser: &str, version: &str) -> bool {
  let download_key = format!("{browser}-{version}");
  let downloading = DOWNLOADING_BROWSERS.lock().unwrap();
  downloading.contains(&download_key)
}

/// Clear all in-progress download bookkeeping for a browser.
///
/// Used as a last-resort cleanup when a download future is abandoned (e.g. dropped
/// by an outer timeout) before its own error path could run. Matches by the
/// `"{browser}-"` key prefix rather than an exact version so no stuck key is left
/// behind even when the caller doesn't know which version was actually in flight.
pub fn clear_download_state_for_browser(browser: &str) {
  let prefix = format!("{browser}-");
  {
    let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
    downloading.retain(|key| !key.starts_with(&prefix));
  }
  {
    let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
    tokens.retain(|key, _| !key.starts_with(&prefix));
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
    .map_err(|e| crate::wrap_backend_error(e, "Failed to download browser"))
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

  #[test]
  fn test_clear_download_state_for_browser_removes_stuck_keys() {
    // Simulate a download future that was abandoned without running its own cleanup,
    // leaving stuck bookkeeping for a version that differs from the requested one.
    let key = "wayfern-1.2.3-resolved".to_string();
    {
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      downloading.insert(key.clone());
    }
    {
      let mut tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
      tokens.insert(key.clone(), CancellationToken::new());
    }

    // A different browser's in-progress state must be left untouched.
    let other = "chromium-9.9.9".to_string();
    {
      let mut downloading = DOWNLOADING_BROWSERS.lock().unwrap();
      downloading.insert(other.clone());
    }

    clear_download_state_for_browser("wayfern");

    assert!(
      !is_downloading("wayfern", "1.2.3-resolved"),
      "stuck wayfern key should be cleared even when version differs from request"
    );
    {
      let tokens = DOWNLOAD_CANCELLATION_TOKENS.lock().unwrap();
      assert!(
        !tokens.contains_key(&key),
        "stuck wayfern cancellation token should be cleared"
      );
    }
    assert!(
      is_downloading("chromium", "9.9.9"),
      "unrelated browser's download state must be preserved"
    );

    // Cleanup so we don't leak global state into other tests.
    clear_download_state_for_browser("wayfern");
    clear_download_state_for_browser("chromium");
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref DOWNLOADER: Downloader = Downloader::new();
}

use crate::browser::{GithubAsset, GithubRelease};
use crate::events;
use crate::profile::manager::ProfileManager;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::fs;
use tokio::io::AsyncWriteExt;

const MMDB_REPO: &str = "P3TERX/GeoLite.mmdb";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoIPDownloadProgress {
  pub stage: String, // "downloading", "extracting", "completed"
  pub percentage: f64,
  pub message: String,
  // Extra fields to mirror browser download progress payload
  pub downloaded_bytes: Option<u64>,
  pub total_bytes: Option<u64>,
  pub speed_bytes_per_sec: Option<f64>,
  pub eta_seconds: Option<f64>,
}

static DOWNLOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

pub struct GeoIPDownloader {
  client: Client,
}

/// How the files are checked before they replace the ones in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseKind {
  City,
  Asn,
}

impl DatabaseKind {
  /// What the file's own metadata names it, e.g. `GeoLite2-City`.
  fn type_suffix(self) -> &'static str {
    match self {
      Self::City => "City",
      Self::Asn => "ASN",
    }
  }
}

/// A finished download that is not the database it claims to be: cut short,
/// an error page, or another file. It never replaces the file in use.
fn verify_database(
  path: &std::path::Path,
  kind: DatabaseKind,
  expected_size: u64,
) -> Result<(), String> {
  let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
  if expected_size > 0 && size != expected_size {
    return Err(format!(
      "size {size} differs from the published {expected_size}"
    ));
  }
  let reader = maxminddb::Reader::open_readfile(path).map_err(|e| e.to_string())?;
  let database_type = &reader.metadata().database_type;
  if !database_type.ends_with(kind.type_suffix()) {
    return Err(format!("the file is a {database_type} database"));
  }
  Ok(())
}

impl GeoIPDownloader {
  fn new() -> Self {
    // A stalled connection must not hold the one download slot forever. The
    // overall limit leaves room for a 60 MB file on a slow link.
    let client = Client::builder()
      .connect_timeout(std::time::Duration::from_secs(30))
      .timeout(std::time::Duration::from_secs(30 * 60))
      .build()
      .unwrap_or_else(|_| Client::new());
    Self { client }
  }

  pub fn instance() -> &'static GeoIPDownloader {
    &GEOIP_DOWNLOADER
  }

  /// Create a new downloader with custom client (for testing)
  #[cfg(test)]
  pub fn new_with_client(client: Client) -> Self {
    Self { client }
  }

  fn get_cache_dir() -> PathBuf {
    crate::app_dirs::cache_dir()
  }

  pub fn get_mmdb_file_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Self::get_cache_dir().join("GeoLite2-City.mmdb"))
  }

  /// Where the autonomous-system database lives. It is the only MaxMind file
  /// that carries an organisation for an address, which is what a proxy check
  /// reports as the exit's ISP; the city database has no such field. Same
  /// release, same publisher, fetched by the same code — it is simply a second
  /// asset off the download the city database already comes from.
  pub fn get_asn_mmdb_file_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Self::get_cache_dir().join("GeoLite2-ASN.mmdb"))
  }

  fn get_release_path() -> PathBuf {
    crate::app_dirs::cache_dir().join("geoip_release")
  }

  /// The release the files in place came from, so an unchanged release is not
  /// downloaded again.
  fn installed_release() -> Option<String> {
    std::fs::read_to_string(Self::get_release_path())
      .ok()
      .map(|tag| tag.trim().to_string())
      .filter(|tag| !tag.is_empty())
  }

  fn asn_missing() -> bool {
    Self::get_asn_mmdb_file_path().map_or(true, |path| !path.exists())
  }

  pub fn is_geoip_database_available() -> bool {
    if let Ok(mmdb_path) = Self::get_mmdb_file_path() {
      mmdb_path.exists()
    } else {
      false
    }
  }

  fn get_timestamp_path() -> PathBuf {
    crate::app_dirs::cache_dir().join("geoip_last_download")
  }

  fn is_geoip_stale() -> bool {
    let timestamp_path = Self::get_timestamp_path();
    let Ok(content) = std::fs::read_to_string(&timestamp_path) else {
      return true;
    };
    let Ok(timestamp) = content.trim().parse::<u64>() else {
      return true;
    };
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    const SEVEN_DAYS: u64 = 7 * 24 * 60 * 60;
    now.saturating_sub(timestamp) > SEVEN_DAYS
  }

  /// Check if GeoIP database is missing or stale for Wayfern fingerprint geo.
  pub fn check_missing_geoip_database(
    &self,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let needs_geoip = profiles.iter().any(|profile| profile.browser == "wayfern");

    if needs_geoip {
      return Ok(
        !Self::is_geoip_database_available() || Self::is_geoip_stale() || Self::asn_missing(),
      );
    }

    Ok(false)
  }

  fn find_mmdb_asset<'a>(release: &'a GithubRelease, suffix: &str) -> Option<&'a GithubAsset> {
    release
      .assets
      .iter()
      .find(|asset| asset.name.ends_with(suffix))
  }

  /// The newest published release that has a City database.
  fn pick_release(releases: &[GithubRelease]) -> Option<&GithubRelease> {
    releases
      .iter()
      .filter(|release| !release.prerelease)
      .find(|release| Self::find_mmdb_asset(release, "-City.mmdb").is_some())
  }

  fn write_timestamp() {
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    let path = Self::get_timestamp_path();
    if let Some(parent) = path.parent() {
      let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, now.to_string());
  }

  pub async fn download_geoip_database(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if DOWNLOAD_IN_PROGRESS
      .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
      .is_err()
    {
      log::debug!("GeoIP download already running");
      return Ok(());
    }
    let result = self.download_geoip_database_inner(_app_handle).await;
    DOWNLOAD_IN_PROGRESS.store(false, Ordering::SeqCst);
    result
  }

  async fn download_geoip_database_inner(
    &self,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Emit initial progress
    let _ = events::emit(
      "geoip-download-progress",
      GeoIPDownloadProgress {
        stage: "downloading".to_string(),
        percentage: 0.0,
        message: "Starting GeoIP database download".to_string(),
        downloaded_bytes: Some(0),
        total_bytes: None,
        speed_bytes_per_sec: Some(0.0),
        eta_seconds: None,
      },
    );

    #[cfg(feature = "e2e")]
    let fixture_url = std::env::var("DONUT_E2E_GEOIP_DOWNLOAD_URL")
      .ok()
      .filter(|url| !url.is_empty());
    #[cfg(not(feature = "e2e"))]
    let fixture_url: Option<String> = None;

    #[cfg(feature = "e2e")]
    let asn_fixture_url = std::env::var("DONUT_E2E_GEOIP_ASN_DOWNLOAD_URL")
      .ok()
      .filter(|url| !url.is_empty());
    #[cfg(not(feature = "e2e"))]
    let asn_fixture_url: Option<String> = None;

    // The ASN asset comes off the same release as the city one, so the release
    // is kept rather than looked up twice.
    let (download_url, city_size, asn_url, asn_size, release_tag) = if let Some(url) = fixture_url {
      (url, 0, asn_fixture_url, 0, None)
    } else {
      let releases = self.fetch_geoip_releases().await?;
      let release =
        Self::pick_release(&releases).ok_or("No compatible GeoIP database release found")?;
      let city = Self::find_mmdb_asset(release, "-City.mmdb")
        .ok_or("No compatible GeoIP database asset found")?;
      let asn = Self::find_mmdb_asset(release, "-ASN.mmdb");
      (
        city.browser_download_url.clone(),
        city.size,
        asn.map(|asset| asset.browser_download_url.clone()),
        asn.map_or(0, |asset| asset.size),
        Some(release.tag_name.clone()),
      )
    };

    // The same release as the files in place: nothing to fetch but a missing
    // ASN file. A 60 MB file is not downloaded again to learn that.
    let city_current = release_tag.is_some()
      && release_tag == Self::installed_release()
      && Self::is_geoip_database_available();
    if city_current {
      Self::write_timestamp();
      if let (true, Some(url)) = (Self::asn_missing(), asn_url.as_deref()) {
        if let Err(e) = self.download_asn_database(url, asn_size).await {
          log::warn!(
            "GeoIP ASN database download failed err=\"{}\"",
            crate::log_redaction::text(&e.to_string())
          );
        }
      }
      log::debug!("GeoIP database already current, download skipped");
      let _ = events::emit(
        "geoip-download-progress",
        GeoIPDownloadProgress {
          stage: "completed".to_string(),
          percentage: 100.0,
          message: "GeoIP database is current".to_string(),
          downloaded_bytes: None,
          total_bytes: None,
          speed_bytes_per_sec: None,
          eta_seconds: None,
        },
      );
      return Ok(());
    }

    // Create cache directory
    let cache_dir = Self::get_cache_dir();
    fs::create_dir_all(&cache_dir).await?;

    let mmdb_path = Self::get_mmdb_file_path()?;

    // Always download to a temp file first, then atomically rename.
    // This prevents corruption if the app is closed mid-download.
    let temp_path = mmdb_path.with_extension("mmdb.downloading");

    // Remove any leftover temp file from a previous interrupted download
    let _ = fs::remove_file(&temp_path).await;

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
    let mut downloaded: u64 = 0;
    let mut file = fs::File::create(&temp_path).await?;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    use std::time::Instant;
    let start_time = Instant::now();
    let mut last_update = Instant::now();
    while let Some(chunk) = stream.next().await {
      let chunk = chunk?;
      downloaded += chunk.len() as u64;
      file.write_all(&chunk).await?;

      let now = Instant::now();
      if now.duration_since(last_update).as_millis() >= 100 {
        let elapsed = start_time.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
          downloaded as f64 / elapsed
        } else {
          0.0
        };
        let percentage = if total_size > 0 {
          (downloaded as f64 / total_size as f64) * 100.0
        } else {
          0.0
        };
        let eta = if speed > 0.0 && total_size > 0 {
          Some((total_size.saturating_sub(downloaded)) as f64 / speed)
        } else {
          None
        };

        let _ = events::emit(
          "geoip-download-progress",
          GeoIPDownloadProgress {
            stage: "downloading".to_string(),
            percentage,
            message: format!("Downloaded {downloaded} / {total_size} bytes"),
            downloaded_bytes: Some(downloaded),
            total_bytes: Some(total_size),
            speed_bytes_per_sec: Some(speed),
            eta_seconds: eta,
          },
        );
        last_update = now;
      }
    }

    file.flush().await?;
    drop(file);

    if let Err(reason) = verify_database(&temp_path, DatabaseKind::City, city_size) {
      let _ = fs::remove_file(&temp_path).await;
      return Err(format!("Downloaded GeoIP database failed verification: {reason}").into());
    }

    // Atomically replace the old database with the new one
    fs::rename(&temp_path, &mmdb_path).await?;
    Self::write_timestamp();
    if let Some(tag) = &release_tag {
      let _ = fs::write(Self::get_release_path(), tag).await;
    }
    log::info!(
      "GeoIP database updated release={} bytes={downloaded}",
      release_tag.as_deref().unwrap_or("fixture")
    );

    // The autonomous-system database, best effort. It only feeds the exit
    // organisation a proxy check reports, so a failure here must never fail
    // the download that fingerprint geolocation actually depends on.
    if let Some(url) = asn_url {
      if let Err(e) = self.download_asn_database(&url, asn_size).await {
        log::warn!(
          "GeoIP ASN database download failed err=\"{}\"",
          crate::log_redaction::text(&e.to_string())
        );
      }
    }

    // Emit completion
    let _ = events::emit(
      "geoip-download-progress",
      GeoIPDownloadProgress {
        stage: "completed".to_string(),
        percentage: 100.0,
        message: "GeoIP database download completed".to_string(),
        downloaded_bytes: Some(downloaded),
        total_bytes: Some(total_size),
        speed_bytes_per_sec: Some(0.0),
        eta_seconds: Some(0.0),
      },
    );

    Ok(())
  }

  /// Fetch the ASN database to a temp file and rename it into place, so a
  /// half-written file is never left where a lookup would read it.
  async fn download_asn_database(
    &self,
    url: &str,
    expected_size: u64,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = Self::get_asn_mmdb_file_path()?;
    let temp_path = path.with_extension("mmdb.downloading");
    let _ = fs::remove_file(&temp_path).await;

    let response = self.client.get(url).send().await?;
    if !response.status().is_success() {
      return Err(format!("HTTP {}", response.status()).into());
    }

    let mut file = fs::File::create(&temp_path).await?;
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
      file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    drop(file);

    if let Err(reason) = verify_database(&temp_path, DatabaseKind::Asn, expected_size) {
      let _ = fs::remove_file(&temp_path).await;
      return Err(format!("failed verification: {reason}").into());
    }
    fs::rename(&temp_path, &path).await?;
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

#[tauri::command]
pub fn check_missing_geoip_database() -> Result<bool, String> {
  let geoip_downloader = GeoIPDownloader::instance();
  geoip_downloader
    .check_missing_geoip_database()
    .map_err(|e| format!("Failed to check missing GeoIP database: {e}"))
}

// Global singleton instance
static GEOIP_DOWNLOADER: std::sync::LazyLock<GeoIPDownloader> =
  std::sync::LazyLock::new(GeoIPDownloader::new);

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

  #[test]
  fn test_find_city_mmdb_asset() {
    let release = create_mock_release();
    let asset = GeoIPDownloader::find_mmdb_asset(&release, "-City.mmdb");
    assert_eq!(
      asset.map(|asset| asset.browser_download_url.as_str()),
      Some("https://example.com/GeoLite2-City.mmdb")
    );
  }

  #[test]
  fn test_find_city_mmdb_asset_not_found() {
    let mut release = create_mock_release();
    release.assets[0].name = "wrong-file.txt".to_string();
    assert!(GeoIPDownloader::find_mmdb_asset(&release, "-City.mmdb").is_none());
  }

  #[test]
  fn a_prerelease_or_a_release_without_a_city_file_is_skipped() {
    let mut prerelease = create_mock_release();
    prerelease.tag_name = "pre".to_string();
    prerelease.prerelease = true;
    let mut empty = create_mock_release();
    empty.tag_name = "empty".to_string();
    empty.assets.clear();
    let releases = vec![prerelease, empty, create_mock_release()];
    assert_eq!(
      GeoIPDownloader::pick_release(&releases).map(|r| r.tag_name.as_str()),
      Some("v1.0.0")
    );
  }

  #[test]
  fn a_file_that_is_not_the_database_never_replaces_the_one_in_use() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("City.mmdb.downloading");
    std::fs::write(&page, b"<html>rate limited</html>").unwrap();
    assert!(verify_database(&page, DatabaseKind::City, 0).is_err());
    assert!(verify_database(&page, DatabaseKind::City, 25).is_err());
    assert!(verify_database(&page, DatabaseKind::City, 999)
      .unwrap_err()
      .contains("differs"));
  }

  #[test]
  fn test_get_cache_dir() {
    let cache_dir = GeoIPDownloader::get_cache_dir();
    assert!(!cache_dir.as_os_str().is_empty());
  }

  #[test]
  fn test_get_mmdb_file_path() {
    let mmdb_path = GeoIPDownloader::get_mmdb_file_path();
    assert!(mmdb_path.is_ok());

    let path = mmdb_path.unwrap();
    assert!(path.to_string_lossy().ends_with("GeoLite2-City.mmdb"));
  }

  #[test]
  fn test_is_geoip_stale() {
    let tmp = tempfile::TempDir::new().unwrap();
    let _guard = crate::app_dirs::set_test_cache_dir(tmp.path().to_path_buf());

    // No timestamp file → stale
    assert!(GeoIPDownloader::is_geoip_stale());

    let timestamp_path = GeoIPDownloader::get_timestamp_path();
    std::fs::create_dir_all(timestamp_path.parent().unwrap()).unwrap();

    // Recent timestamp → not stale
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs();
    std::fs::write(&timestamp_path, now.to_string()).unwrap();
    assert!(!GeoIPDownloader::is_geoip_stale());

    // 8 days ago → stale
    let eight_days_ago = now - 8 * 24 * 60 * 60;
    std::fs::write(&timestamp_path, eight_days_ago.to_string()).unwrap();
    assert!(GeoIPDownloader::is_geoip_stale());
  }

  #[test]
  fn test_is_geoip_database_available() {
    // Test that the function works correctly regardless of file system state.
    // This test only verifies the function doesn't panic and returns a boolean.
    // We cannot reliably compare is_available to file existence due to potential
    // race conditions with other tests that modify environment variables.
    let _is_available = GeoIPDownloader::is_geoip_database_available();

    // Verify the function logic by checking if the path resolution works
    let mmdb_path_result = GeoIPDownloader::get_mmdb_file_path();
    assert!(
      mmdb_path_result.is_ok(),
      "Should be able to get MMDB file path"
    );

    let mmdb_path = mmdb_path_result.unwrap();
    assert!(
      mmdb_path.to_string_lossy().ends_with("GeoLite2-City.mmdb"),
      "Path should end with expected filename"
    );
  }
}

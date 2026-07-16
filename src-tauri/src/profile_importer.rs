use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, create_dir_all, File};
use std::io;
use std::path::{Path, PathBuf};

use crate::downloaded_browsers_registry::DownloadedBrowsersRegistry;
use crate::events;
use crate::profile::types::{get_host_os, BrowserProfile, SyncMode};
use crate::profile::ProfileManager;
use crate::proxy_manager::PROXY_MANAGER;
use crate::wayfern_manager::WayfernConfig;

/// Prefix for temp directories that hold extracted profile archives. Cleanup
/// refuses to delete anything outside the system temp dir with this prefix.
const IMPORT_SCRATCH_PREFIX: &str = "donutbrowser-profile-import-";

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub struct DetectedProfile {
  pub browser: String,
  pub mapped_browser: String,
  pub name: String,
  pub path: String,
  pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub struct ImportProfileItem {
  pub source_path: String,
  #[serde(default = "default_import_browser_type")]
  pub browser_type: String,
  pub new_profile_name: String,
  #[serde(default)]
  pub proxy_id: Option<String>,
}

fn default_import_browser_type() -> String {
  "chromium".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DuplicateStrategy {
  /// Skip items whose requested name is already taken.
  Skip,
  /// Auto-suffix the requested name (`Name (2)`, `Name (3)`, …) until free.
  #[default]
  Rename,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub struct ProfileImportItemResult {
  /// Final profile name (after any duplicate-rename).
  pub name: String,
  pub source_path: String,
  /// "imported" | "skipped" | "failed"
  pub status: String,
  pub profile_id: Option<String>,
  /// Structured `{"code": …}` error string when status is "failed".
  pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub struct ProfileImportBatchResult {
  pub imported_count: usize,
  pub skipped_count: usize,
  pub failed_count: usize,
  pub results: Vec<ProfileImportItemResult>,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub struct ArchiveScanResult {
  /// Temp directory the archive was extracted into. Pass back to
  /// `cleanup_profile_import_scratch` once the import is done.
  pub extracted_dir: String,
  pub profiles: Vec<DetectedProfile>,
}

#[derive(Debug, Serialize, Clone)]
struct ProfileImportProgress {
  total: usize,
  completed: usize,
  index: usize,
  name: String,
  /// "importing" | "imported" | "skipped" | "failed"
  status: String,
}

fn map_browser_type(_browser: &str) -> &str {
  // Every import source maps to Wayfern — the only launchable engine.
  "wayfern"
}

/// Convert an importer error into the structured `{"code": …}` string the
/// frontend translates. Errors that are already structured pass through.
pub fn error_to_code_string(e: Box<dyn std::error::Error>) -> String {
  let msg = e.to_string();
  if msg.starts_with('{') {
    msg
  } else {
    serde_json::json!({ "code": "INTERNAL_ERROR", "params": { "detail": msg } }).to_string()
  }
}

/// Resolve a requested profile name against the set of taken (lowercased)
/// names by appending ` (2)`, ` (3)`, … . The chosen name is added to `taken`.
fn resolve_duplicate_name(requested: &str, taken: &mut HashSet<String>) -> String {
  if taken.insert(requested.to_lowercase()) {
    return requested.to_string();
  }
  let mut n = 2usize;
  loop {
    let candidate = format!("{requested} ({n})");
    if taken.insert(candidate.to_lowercase()) {
      return candidate;
    }
    n += 1;
  }
}

fn emit_import_progress(total: usize, completed: usize, index: usize, name: &str, status: &str) {
  let _ = events::emit(
    "profile-import-progress",
    &ProfileImportProgress {
      total,
      completed,
      index,
      name: name.to_string(),
      status: status.to_string(),
    },
  );
}

pub struct ProfileImporter {
  base_dirs: BaseDirs,
  downloaded_browsers_registry: &'static DownloadedBrowsersRegistry,
  profile_manager: &'static ProfileManager,
  wayfern_manager: &'static crate::wayfern_manager::WayfernManager,
}

impl ProfileImporter {
  fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      downloaded_browsers_registry: DownloadedBrowsersRegistry::instance(),
      profile_manager: ProfileManager::instance(),
      wayfern_manager: crate::wayfern_manager::WayfernManager::instance(),
    }
  }

  pub fn instance() -> &'static ProfileImporter {
    &PROFILE_IMPORTER
  }

  pub fn detect_existing_profiles(
    &self,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut detected_profiles = Vec::new();

    // Only Chromium-based sources (mapping to Wayfern) are detected. Gecko-family
    // sources mapped to Camoufox, which was removed, so they can no longer be
    // imported.
    detected_profiles.extend(self.detect_chrome_profiles()?);
    detected_profiles.extend(self.detect_brave_profiles()?);
    detected_profiles.extend(self.detect_chromium_profiles()?);

    let mut seen_paths = HashSet::new();
    let unique_profiles: Vec<DetectedProfile> = detected_profiles
      .into_iter()
      .filter(|profile| seen_paths.insert(profile.path.clone()))
      .collect();

    Ok(unique_profiles)
  }

  /// Scan an arbitrary folder for importable Chromium-family profiles.
  /// Handles three shapes: the folder itself is a profile (has `Preferences`),
  /// the folder is a user-data dir (`Default` / `Profile N` children), or the
  /// folder holds one profile directory per child (exported/migrated layouts,
  /// including one nested user-data dir per child).
  pub fn scan_folder(
    &self,
    folder: &Path,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    if !folder.exists() || !folder.is_dir() {
      return Err(
        serde_json::json!({ "code": "IMPORT_SOURCE_NOT_FOUND" })
          .to_string()
          .into(),
      );
    }

    if folder.join("Preferences").exists() {
      let name = folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Imported profile")
        .to_string();
      return Ok(vec![DetectedProfile {
        browser: "chromium".to_string(),
        mapped_browser: map_browser_type("chromium").to_string(),
        name,
        path: folder.to_string_lossy().to_string(),
        description: "Chromium profile".to_string(),
      }]);
    }

    let mut profiles = self.scan_chrome_profiles_dir(folder, "chromium")?;
    let mut seen: HashSet<String> = profiles.iter().map(|p| p.path.clone()).collect();

    if let Ok(entries) = fs::read_dir(folder) {
      for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
          continue;
        }
        let dir_name = path
          .file_name()
          .and_then(|n| n.to_str())
          .unwrap_or("")
          .to_string();

        if path.join("Preferences").exists() {
          let path_str = path.to_string_lossy().to_string();
          if seen.insert(path_str.clone()) {
            profiles.push(DetectedProfile {
              browser: "chromium".to_string(),
              mapped_browser: map_browser_type("chromium").to_string(),
              name: dir_name,
              path: path_str,
              description: "Chromium profile".to_string(),
            });
          }
        } else {
          for nested in self.scan_chrome_profiles_dir(&path, "chromium")? {
            if seen.insert(nested.path.clone()) {
              profiles.push(DetectedProfile {
                name: format!("{} - {}", dir_name, nested.description),
                ..nested
              });
            }
          }
        }
      }
    }

    Ok(profiles)
  }

  /// Extract a ZIP archive into a scratch temp dir and scan it for profiles.
  pub async fn extract_archive_and_scan(
    &self,
    archive_path: &str,
  ) -> Result<ArchiveScanResult, Box<dyn std::error::Error>> {
    let path = Path::new(archive_path);
    if !path.exists() {
      return Err(
        serde_json::json!({ "code": "IMPORT_SOURCE_NOT_FOUND" })
          .to_string()
          .into(),
      );
    }

    let extension = path
      .extension()
      .and_then(|e| e.to_str())
      .unwrap_or("")
      .to_lowercase();
    if extension != "zip" {
      return Err(
        serde_json::json!({ "code": "UNSUPPORTED_ARCHIVE_FORMAT" })
          .to_string()
          .into(),
      );
    }

    let dest =
      std::env::temp_dir().join(format!("{IMPORT_SCRATCH_PREFIX}{}", uuid::Uuid::new_v4()));
    let archive = path.to_path_buf();
    let dest_clone = dest.clone();
    tokio::task::spawn_blocking(move || Self::extract_zip_archive(&archive, &dest_clone))
      .await
      .map_err(|e| format!("Archive extraction task failed: {e}"))?
      .map_err(|e| {
        let _ = fs::remove_dir_all(&dest);
        serde_json::json!({ "code": "ARCHIVE_EXTRACTION_FAILED", "params": { "detail": e } })
          .to_string()
      })?;

    let profiles = self.scan_folder(&dest)?;
    Ok(ArchiveScanResult {
      extracted_dir: dest.to_string_lossy().to_string(),
      profiles,
    })
  }

  fn extract_zip_archive(zip_path: &Path, dest: &Path) -> Result<(), String> {
    create_dir_all(dest).map_err(|e| e.to_string())?;
    let file = File::open(zip_path)
      .map_err(|e| format!("Failed to open ZIP file {}: {}", zip_path.display(), e))?;
    let mut archive = zip::ZipArchive::new(io::BufReader::new(file))
      .map_err(|e| format!("Failed to read ZIP archive {}: {}", zip_path.display(), e))?;

    for i in 0..archive.len() {
      let mut entry = archive
        .by_index(i)
        .map_err(|e| format!("Failed to read ZIP entry at index {i}: {e}"))?;

      // enclosed_name prevents path traversal via ../ entries.
      let enclosed = entry
        .enclosed_name()
        .ok_or_else(|| format!("ZIP contains an invalid entry path: {}", entry.name()))?;
      let outpath = dest.join(enclosed);

      if entry.is_dir() {
        create_dir_all(&outpath).map_err(|e| e.to_string())?;
      } else {
        if let Some(parent) = outpath.parent() {
          create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut outfile = File::create(&outpath)
          .map_err(|e| format!("Failed to create file {}: {}", outpath.display(), e))?;
        io::copy(&mut entry, &mut outfile)
          .map_err(|e| format!("Failed to extract file {}: {}", outpath.display(), e))?;
      }
    }

    Ok(())
  }

  /// Remove a scratch dir created by `extract_archive_and_scan`. Refuses paths
  /// outside the system temp dir or without the import-scratch prefix.
  pub fn cleanup_scratch_dir(extracted_dir: &str) -> Result<(), String> {
    let path = PathBuf::from(extracted_dir);
    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !path.starts_with(std::env::temp_dir()) || !dir_name.starts_with(IMPORT_SCRATCH_PREFIX) {
      return Err(
        serde_json::json!({ "code": "INTERNAL_ERROR", "params": { "detail": "Refusing to remove a non-scratch directory" } })
          .to_string(),
      );
    }
    if path.exists() {
      fs::remove_dir_all(&path).map_err(|e| {
        serde_json::json!({ "code": "INTERNAL_ERROR", "params": { "detail": e.to_string() } })
          .to_string()
      })?;
    }
    Ok(())
  }

  fn detect_chrome_profiles(&self) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let chrome_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/Google/Chrome");
      profiles.extend(self.scan_chrome_profiles_dir(&chrome_dir, "chromium")?);
    }

    #[cfg(target_os = "windows")]
    {
      let local_app_data = self.base_dirs.data_local_dir();
      let chrome_dir = local_app_data.join("Google/Chrome/User Data");
      profiles.extend(self.scan_chrome_profiles_dir(&chrome_dir, "chromium")?);
    }

    #[cfg(target_os = "linux")]
    {
      let chrome_dir = self.base_dirs.home_dir().join(".config/google-chrome");
      profiles.extend(self.scan_chrome_profiles_dir(&chrome_dir, "chromium")?);
    }

    Ok(profiles)
  }

  fn detect_chromium_profiles(&self) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let chromium_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/Chromium");
      profiles.extend(self.scan_chrome_profiles_dir(&chromium_dir, "chromium")?);
    }

    #[cfg(target_os = "windows")]
    {
      let local_app_data = self.base_dirs.data_local_dir();
      let chromium_dir = local_app_data.join("Chromium/User Data");
      profiles.extend(self.scan_chrome_profiles_dir(&chromium_dir, "chromium")?);
    }

    #[cfg(target_os = "linux")]
    {
      let chromium_dir = self.base_dirs.home_dir().join(".config/chromium");
      profiles.extend(self.scan_chrome_profiles_dir(&chromium_dir, "chromium")?);
    }

    Ok(profiles)
  }

  fn detect_brave_profiles(&self) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    #[cfg(target_os = "macos")]
    {
      let brave_dir = self
        .base_dirs
        .home_dir()
        .join("Library/Application Support/BraveSoftware/Brave-Browser");
      profiles.extend(self.scan_chrome_profiles_dir(&brave_dir, "brave")?);
    }

    #[cfg(target_os = "windows")]
    {
      let local_app_data = self.base_dirs.data_local_dir();
      let brave_dir = local_app_data.join("BraveSoftware/Brave-Browser/User Data");
      profiles.extend(self.scan_chrome_profiles_dir(&brave_dir, "brave")?);
    }

    #[cfg(target_os = "linux")]
    {
      let brave_dir = self
        .base_dirs
        .home_dir()
        .join(".config/BraveSoftware/Brave-Browser");
      profiles.extend(self.scan_chrome_profiles_dir(&brave_dir, "brave")?);
    }

    Ok(profiles)
  }

  fn scan_chrome_profiles_dir(
    &self,
    browser_dir: &Path,
    browser_type: &str,
  ) -> Result<Vec<DetectedProfile>, Box<dyn std::error::Error>> {
    let mut profiles = Vec::new();

    if !browser_dir.exists() {
      return Ok(profiles);
    }

    let default_profile = browser_dir.join("Default");
    if default_profile.exists() && default_profile.join("Preferences").exists() {
      profiles.push(DetectedProfile {
        browser: browser_type.to_string(),
        mapped_browser: map_browser_type(browser_type).to_string(),
        name: format!(
          "{} - Default Profile",
          self.get_browser_display_name(browser_type)
        ),
        path: default_profile.to_string_lossy().to_string(),
        description: "Default profile".to_string(),
      });
    }

    if let Ok(entries) = fs::read_dir(browser_dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
          let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

          if dir_name.starts_with("Profile ") && path.join("Preferences").exists() {
            let profile_number = &dir_name[8..];
            profiles.push(DetectedProfile {
              browser: browser_type.to_string(),
              mapped_browser: map_browser_type(browser_type).to_string(),
              name: format!(
                "{} - Profile {}",
                self.get_browser_display_name(browser_type),
                profile_number
              ),
              path: path.to_string_lossy().to_string(),
              description: format!("Profile {profile_number}"),
            });
          }
        }
      }
    }

    Ok(profiles)
  }

  fn get_browser_display_name(&self, browser_type: &str) -> &str {
    match browser_type {
      "chromium" => "Chrome/Chromium",
      "brave" => "Brave",
      "zen" => "Zen Browser",

      "wayfern" => "Wayfern",
      _ => "Unknown Browser",
    }
  }

  /// Import a batch of profiles. Items are isolated: one failure doesn't stop
  /// the rest. Emits `profile-import-progress` events around each item.
  pub async fn import_profiles(
    &self,
    app_handle: &tauri::AppHandle,
    items: Vec<ImportProfileItem>,
    group_id: Option<String>,
    duplicate_strategy: DuplicateStrategy,
    wayfern_config: Option<WayfernConfig>,
  ) -> Result<ProfileImportBatchResult, Box<dyn std::error::Error>> {
    if items.is_empty() {
      return Err(
        serde_json::json!({ "code": "IMPORT_NO_ITEMS" })
          .to_string()
          .into(),
      );
    }

    if let Some(ref gid) = group_id {
      let groups = crate::group_manager::GroupManager::new().get_all_groups()?;
      if !groups.iter().any(|g| &g.id == gid) {
        return Err(
          serde_json::json!({ "code": "GROUP_NOT_FOUND" })
            .to_string()
            .into(),
        );
      }
    }

    let mut taken_names: HashSet<String> = self
      .profile_manager
      .list_profiles()?
      .iter()
      .map(|p| p.name.to_lowercase())
      .collect();

    let total = items.len();
    let mut results = Vec::with_capacity(total);
    let mut imported_count = 0usize;
    let mut skipped_count = 0usize;
    let mut failed_count = 0usize;
    let mut completed = 0usize;

    for (index, item) in items.into_iter().enumerate() {
      let requested = item.new_profile_name.trim().to_string();
      if requested.is_empty() {
        failed_count += 1;
        completed += 1;
        emit_import_progress(total, completed, index, &item.source_path, "failed");
        results.push(ProfileImportItemResult {
          name: requested,
          source_path: item.source_path,
          status: "failed".to_string(),
          profile_id: None,
          error: Some(serde_json::json!({ "code": "NAME_CANNOT_BE_EMPTY" }).to_string()),
        });
        continue;
      }

      let final_name = if taken_names.contains(&requested.to_lowercase()) {
        match duplicate_strategy {
          DuplicateStrategy::Skip => {
            skipped_count += 1;
            completed += 1;
            emit_import_progress(total, completed, index, &requested, "skipped");
            results.push(ProfileImportItemResult {
              name: requested,
              source_path: item.source_path,
              status: "skipped".to_string(),
              profile_id: None,
              error: None,
            });
            continue;
          }
          DuplicateStrategy::Rename => resolve_duplicate_name(&requested, &mut taken_names),
        }
      } else {
        taken_names.insert(requested.to_lowercase());
        requested
      };

      emit_import_progress(total, completed, index, &final_name, "importing");

      match self
        .import_profile(
          app_handle,
          &item.source_path,
          &item.browser_type,
          &final_name,
          item.proxy_id.clone(),
          group_id.clone(),
          wayfern_config.clone(),
        )
        .await
      {
        Ok(profile) => {
          imported_count += 1;
          completed += 1;
          emit_import_progress(total, completed, index, &final_name, "imported");
          let _ = events::emit_empty("profiles-changed");
          results.push(ProfileImportItemResult {
            name: final_name,
            source_path: item.source_path,
            status: "imported".to_string(),
            profile_id: Some(profile.id.to_string()),
            error: None,
          });
        }
        Err(e) => {
          failed_count += 1;
          completed += 1;
          emit_import_progress(total, completed, index, &final_name, "failed");
          // The name was reserved but the import failed — free it again.
          taken_names.remove(&final_name.to_lowercase());
          results.push(ProfileImportItemResult {
            name: final_name,
            source_path: item.source_path,
            status: "failed".to_string(),
            profile_id: None,
            error: Some(error_to_code_string(e)),
          });
        }
      }
    }

    Ok(ProfileImportBatchResult {
      imported_count,
      skipped_count,
      failed_count,
      results,
    })
  }

  #[allow(clippy::too_many_arguments)]
  pub async fn import_profile(
    &self,
    app_handle: &tauri::AppHandle,
    source_path: &str,
    browser_type: &str,
    new_profile_name: &str,
    proxy_id: Option<String>,
    group_id: Option<String>,
    wayfern_config: Option<WayfernConfig>,
  ) -> Result<BrowserProfile, Box<dyn std::error::Error>> {
    let source_path = Path::new(source_path);
    if !source_path.exists() {
      return Err(
        serde_json::json!({ "code": "IMPORT_SOURCE_NOT_FOUND" })
          .to_string()
          .into(),
      );
    }

    let mapped = map_browser_type(browser_type);

    if let Some(ref pid) = proxy_id {
      if PROXY_MANAGER.is_cloud_or_derived(pid) || pid == crate::proxy_manager::CLOUD_PROXY_ID {
        crate::cloud_auth::CLOUD_AUTH.sync_cloud_proxy().await;
      }
    }

    let existing_profiles = self.profile_manager.list_profiles()?;
    if existing_profiles
      .iter()
      .any(|p| p.name.to_lowercase() == new_profile_name.to_lowercase())
    {
      return Err(
        serde_json::json!({ "code": "PROFILE_NAME_EXISTS", "params": { "name": new_profile_name } })
          .to_string()
          .into(),
      );
    }

    let profile_id = uuid::Uuid::new_v4();
    let profiles_dir = self.profile_manager.get_profiles_dir();
    let new_profile_uuid_dir = profiles_dir.join(profile_id.to_string());
    let new_profile_data_dir = new_profile_uuid_dir.join("profile");

    create_dir_all(&new_profile_uuid_dir)?;
    create_dir_all(&new_profile_data_dir)?;

    // Profile dirs can be multiple GB — keep the copy off the async runtime.
    let copy_source = source_path.to_path_buf();
    let copy_dest = new_profile_data_dir.clone();
    let copy_result = tokio::task::spawn_blocking(move || {
      Self::copy_directory_recursive(&copy_source, &copy_dest).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Profile copy task failed: {e}"))?;
    if let Err(e) = copy_result {
      let _ = fs::remove_dir_all(&new_profile_uuid_dir);
      return Err(
        serde_json::json!({ "code": "INTERNAL_ERROR", "params": { "detail": e } })
          .to_string()
          .into(),
      );
    }

    let version = match self.get_default_version_for_browser(mapped) {
      Ok(version) => version,
      Err(e) => {
        let _ = fs::remove_dir_all(&new_profile_uuid_dir);
        return Err(e);
      }
    };

    let final_wayfern_config = if mapped == "wayfern" {
      let mut config = wayfern_config.unwrap_or_default();

      if let Some(ref proxy_id_val) = proxy_id {
        if let Some(proxy_settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id_val) {
          let proxy_url = if let (Some(username), Some(password)) =
            (&proxy_settings.username, &proxy_settings.password)
          {
            format!(
              "{}://{}:{}@{}:{}",
              proxy_settings.proxy_type.to_lowercase(),
              username,
              password,
              proxy_settings.host,
              proxy_settings.port
            )
          } else {
            format!(
              "{}://{}:{}",
              proxy_settings.proxy_type.to_lowercase(),
              proxy_settings.host,
              proxy_settings.port
            )
          };
          config.proxy = Some(proxy_url);
        }
      }

      if config.fingerprint.is_none() {
        let temp_profile = BrowserProfile {
          id: uuid::Uuid::new_v4(),
          name: new_profile_name.to_string(),
          browser: mapped.to_string(),
          version: version.clone(),
          proxy_id: proxy_id.clone(),
          vpn_id: None,
          launch_hook: None,
          process_id: None,
          last_launch: None,
          release_type: "stable".to_string(),
          wayfern_config: None,
          group_id: None,
          tags: Vec::new(),
          note: None,
          window_color: None,
          sync_mode: SyncMode::Disabled,
          encryption_salt: None,
          last_sync: None,
          host_os: None,
          ephemeral: false,
          extension_group_id: None,
          proxy_bypass_rules: Vec::new(),
          created_by_id: None,
          created_by_email: None,
          dns_blocklist: None,
          password_protected: false,
          created_at: None,
          updated_at: None,
        };

        match self
          .wayfern_manager
          .generate_fingerprint_config(app_handle, &temp_profile, &config)
          .await
        {
          // geo_proxy_signature is intentionally left unset here: the first
          // launch's signature-mismatch refresh verifies the location either way.
          Ok((fp, _geolocation_applied)) => config.fingerprint = Some(fp),
          Err(e) => {
            let _ = fs::remove_dir_all(&new_profile_uuid_dir);
            return Err(
              serde_json::json!({
                "code": "INTERNAL_ERROR",
                "params": { "detail": format!("Failed to generate fingerprint for imported profile '{new_profile_name}': {e}") }
              })
              .to_string()
              .into(),
            );
          }
        }
      }

      config.proxy = None;
      Some(config)
    } else {
      None
    };

    let profile = BrowserProfile {
      id: profile_id,
      name: new_profile_name.to_string(),
      browser: mapped.to_string(),
      version,
      proxy_id,
      vpn_id: None,
      launch_hook: None,
      process_id: None,
      last_launch: None,
      release_type: "stable".to_string(),
      wayfern_config: final_wayfern_config,
      group_id,
      tags: Vec::new(),
      note: None,
      window_color: None,
      sync_mode: SyncMode::Disabled,
      encryption_salt: None,
      last_sync: None,
      host_os: Some(get_host_os()),
      ephemeral: false,
      extension_group_id: None,
      proxy_bypass_rules: Vec::new(),
      created_by_id: None,
      created_by_email: None,
      dns_blocklist: None,
      password_protected: false,
      created_at: Some(
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .map(|d| d.as_secs())
          .unwrap_or(0),
      ),
      updated_at: Some(crate::proxy_manager::now_secs()),
    };

    self.profile_manager.save_profile(&profile)?;

    log::info!(
      "Successfully imported profile '{}' from '{}'",
      new_profile_name,
      source_path.display()
    );

    Ok(profile)
  }

  fn get_default_version_for_browser(
    &self,
    browser_type: &str,
  ) -> Result<String, Box<dyn std::error::Error>> {
    let downloaded_versions = self
      .downloaded_browsers_registry
      .get_downloaded_versions(browser_type);

    if let Some(version) = downloaded_versions.first() {
      return Ok(version.clone());
    }

    Err(
      serde_json::json!({
        "code": "BROWSER_NOT_DOWNLOADED",
        "params": { "browser": self.get_browser_display_name(browser_type) }
      })
      .to_string()
      .into(),
    )
  }

  pub fn copy_directory_recursive(
    source: &Path,
    destination: &Path,
  ) -> Result<(), Box<dyn std::error::Error>> {
    if !destination.exists() {
      create_dir_all(destination)?;
    }

    for entry in fs::read_dir(source)? {
      let entry = entry?;
      let source_path = entry.path();
      let dest_path = destination.join(entry.file_name());

      if source_path.is_dir() {
        Self::copy_directory_recursive(&source_path, &dest_path)?;
      } else {
        fs::copy(&source_path, &dest_path)?;
      }
    }

    Ok(())
  }
}

#[tauri::command]
pub async fn detect_existing_profiles() -> Result<Vec<DetectedProfile>, String> {
  let importer = ProfileImporter::instance();
  importer
    .detect_existing_profiles()
    .map_err(error_to_code_string)
}

#[tauri::command]
pub async fn scan_folder_for_profiles(folder_path: String) -> Result<Vec<DetectedProfile>, String> {
  let importer = ProfileImporter::instance();
  importer
    .scan_folder(Path::new(&folder_path))
    .map_err(error_to_code_string)
}

#[tauri::command]
pub async fn scan_profile_archive(archive_path: String) -> Result<ArchiveScanResult, String> {
  let importer = ProfileImporter::instance();
  importer
    .extract_archive_and_scan(&archive_path)
    .await
    .map_err(error_to_code_string)
}

#[tauri::command]
pub async fn cleanup_profile_import_scratch(extracted_dir: String) -> Result<(), String> {
  ProfileImporter::cleanup_scratch_dir(&extracted_dir)
}

#[tauri::command]
pub async fn import_browser_profiles(
  app_handle: tauri::AppHandle,
  items: Vec<ImportProfileItem>,
  group_id: Option<String>,
  duplicate_strategy: Option<DuplicateStrategy>,
  wayfern_config: Option<WayfernConfig>,
) -> Result<ProfileImportBatchResult, String> {
  let fingerprint_os = wayfern_config.as_ref().and_then(|c| c.os.as_deref());

  if !crate::cloud_auth::CLOUD_AUTH
    .is_fingerprint_os_allowed(fingerprint_os)
    .await
  {
    return Err(serde_json::json!({ "code": "FINGERPRINT_REQUIRES_PRO" }).to_string());
  }

  let importer = ProfileImporter::instance();
  importer
    .import_profiles(
      &app_handle,
      items,
      group_id,
      duplicate_strategy.unwrap_or_default(),
      wayfern_config,
    )
    .await
    .map_err(error_to_code_string)
}

lazy_static::lazy_static! {
  static ref PROFILE_IMPORTER: ProfileImporter = ProfileImporter::new();
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::env;
  use tempfile::TempDir;

  fn create_test_profile_importer() -> (ProfileImporter, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    env::set_var("HOME", temp_dir.path());
    let importer = ProfileImporter::new();
    (importer, temp_dir)
  }

  #[test]
  fn test_profile_importer_creation() {
    let (_importer, _temp_dir) = create_test_profile_importer();
  }

  #[test]
  fn test_get_browser_display_name() {
    let (importer, _temp_dir) = create_test_profile_importer();

    assert_eq!(
      importer.get_browser_display_name("chromium"),
      "Chrome/Chromium"
    );
    assert_eq!(importer.get_browser_display_name("brave"), "Brave");
    assert_eq!(importer.get_browser_display_name("zen"), "Zen Browser");
    assert_eq!(
      importer.get_browser_display_name("unknown"),
      "Unknown Browser"
    );
  }

  #[test]
  fn test_map_browser_type() {
    assert_eq!(map_browser_type("chromium"), "wayfern");
    assert_eq!(map_browser_type("brave"), "wayfern");
    assert_eq!(map_browser_type("camoufox"), "wayfern");
    assert_eq!(map_browser_type("wayfern"), "wayfern");
    assert_eq!(map_browser_type("something_else"), "wayfern");
  }

  #[test]
  fn test_detect_existing_profiles_no_panic() {
    let (importer, _temp_dir) = create_test_profile_importer();

    let result = importer.detect_existing_profiles();
    assert!(result.is_ok(), "detect_existing_profiles should not fail");
    let _profiles = result.unwrap();
  }

  #[test]
  fn test_scan_chrome_profiles_dir_nonexistent() {
    let (importer, temp_dir) = create_test_profile_importer();

    let nonexistent_dir = temp_dir.path().join("nonexistent");
    let result = importer.scan_chrome_profiles_dir(&nonexistent_dir, "chromium");

    assert!(
      result.is_ok(),
      "Should handle nonexistent directory gracefully"
    );
    let profiles = result.unwrap();
    assert!(
      profiles.is_empty(),
      "Should return empty vector for nonexistent directory"
    );
  }

  #[test]
  fn test_copy_directory_recursive() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    let source_dir = temp_dir.path().join("source");
    let source_subdir = source_dir.join("subdir");
    fs::create_dir_all(&source_subdir).expect("Should create source directories");

    let source_file1 = source_dir.join("file1.txt");
    let source_file2 = source_subdir.join("file2.txt");
    fs::write(&source_file1, "content1").expect("Should create file1");
    fs::write(&source_file2, "content2").expect("Should create file2");

    let dest_dir = temp_dir.path().join("dest");

    let result = ProfileImporter::copy_directory_recursive(&source_dir, &dest_dir);
    assert!(result.is_ok(), "Should copy directory successfully");

    let dest_file1 = dest_dir.join("file1.txt");
    let dest_file2 = dest_dir.join("subdir").join("file2.txt");

    assert!(dest_file1.exists(), "file1.txt should be copied");
    assert!(dest_file2.exists(), "file2.txt should be copied");

    let content1 = fs::read_to_string(&dest_file1).expect("Should read file1");
    let content2 = fs::read_to_string(&dest_file2).expect("Should read file2");

    assert_eq!(content1, "content1", "file1 content should match");
    assert_eq!(content2, "content2", "file2 content should match");
  }

  #[test]
  fn test_get_default_version_for_browser_no_versions() {
    let (importer, _temp_dir) = create_test_profile_importer();

    // Use a browser name that is guaranteed to have no downloaded versions,
    // since the global registry singleton may contain real data from the system.
    let result = importer.get_default_version_for_browser("nonexistent_browser_xyz");
    assert!(
      result.is_err(),
      "Should fail when no versions are available"
    );

    let error_msg = result.unwrap_err().to_string();
    assert!(
      error_msg.contains("BROWSER_NOT_DOWNLOADED"),
      "Error should carry the BROWSER_NOT_DOWNLOADED code"
    );
  }

  #[test]
  fn test_resolve_duplicate_name() {
    let mut taken: HashSet<String> = ["existing".to_string()].into_iter().collect();

    assert_eq!(resolve_duplicate_name("Fresh", &mut taken), "Fresh");
    // Case-insensitive collision gets a suffix.
    assert_eq!(
      resolve_duplicate_name("Existing", &mut taken),
      "Existing (2)"
    );
    assert_eq!(
      resolve_duplicate_name("Existing", &mut taken),
      "Existing (3)"
    );
    // The fresh name reserved above now collides too.
    assert_eq!(resolve_duplicate_name("fresh", &mut taken), "fresh (2)");
  }

  #[test]
  fn test_scan_folder_single_profile() {
    let (importer, temp_dir) = create_test_profile_importer();

    let profile_dir = temp_dir.path().join("my-profile");
    fs::create_dir_all(&profile_dir).unwrap();
    fs::write(profile_dir.join("Preferences"), "{}").unwrap();

    let profiles = importer.scan_folder(&profile_dir).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "my-profile");
    assert_eq!(profiles[0].mapped_browser, "wayfern");
  }

  #[test]
  fn test_scan_folder_user_data_dir_and_children() {
    let (importer, temp_dir) = create_test_profile_importer();

    let root = temp_dir.path().join("exported");
    // User-data-dir shape.
    fs::create_dir_all(root.join("Default")).unwrap();
    fs::write(root.join("Default/Preferences"), "{}").unwrap();
    fs::create_dir_all(root.join("Profile 2")).unwrap();
    fs::write(root.join("Profile 2/Preferences"), "{}").unwrap();
    // Free-form exported profile dir.
    fs::create_dir_all(root.join("account-a")).unwrap();
    fs::write(root.join("account-a/Preferences"), "{}").unwrap();
    // Nested user-data-dir one level down.
    fs::create_dir_all(root.join("old-chrome/Default")).unwrap();
    fs::write(root.join("old-chrome/Default/Preferences"), "{}").unwrap();
    // Noise: dir without Preferences anywhere.
    fs::create_dir_all(root.join("random")).unwrap();

    let profiles = importer.scan_folder(&root).unwrap();
    let mut names: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
    names.sort();
    assert_eq!(profiles.len(), 4, "should find 4 profiles: {names:?}");
    assert!(names.contains(&"account-a".to_string()));
    assert!(names.iter().any(|n| n.contains("old-chrome")));
  }

  #[test]
  fn test_scan_folder_missing() {
    let (importer, temp_dir) = create_test_profile_importer();

    let result = importer.scan_folder(&temp_dir.path().join("nope"));
    assert!(result.is_err());
    assert!(result
      .unwrap_err()
      .to_string()
      .contains("IMPORT_SOURCE_NOT_FOUND"));
  }

  #[test]
  fn test_cleanup_scratch_dir_refuses_foreign_paths() {
    let temp_dir = TempDir::new().unwrap();
    let foreign = temp_dir.path().join("not-scratch");
    fs::create_dir_all(&foreign).unwrap();

    let result = ProfileImporter::cleanup_scratch_dir(&foreign.to_string_lossy());
    assert!(
      result.is_err(),
      "must refuse dirs without the scratch prefix"
    );
    assert!(foreign.exists());

    let scratch = std::env::temp_dir().join(format!("{IMPORT_SCRATCH_PREFIX}test-cleanup"));
    fs::create_dir_all(&scratch).unwrap();
    ProfileImporter::cleanup_scratch_dir(&scratch.to_string_lossy()).unwrap();
    assert!(!scratch.exists(), "scratch dir should be removed");
  }
}

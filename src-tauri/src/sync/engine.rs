use super::client::SyncClient;
use super::encryption;
use super::manifest::{compute_diff, generate_manifest, get_cache_path, HashCache, SyncManifest};
use super::types::*;
use crate::events;
use crate::profile::types::{BrowserProfile, SyncMode};
use crate::profile::ProfileManager;
use crate::settings_manager::SettingsManager;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex as TokioMutex, Semaphore};

/// Upload/download concurrency limit
const SYNC_CONCURRENCY: usize = 32;

/// Max retries for individual file uploads/downloads
const MAX_FILE_RETRIES: u32 = 3;

/// Critical file patterns — if any of these fail to upload/download, the sync is aborted.
const CRITICAL_FILE_PATTERNS: &[&str] = &[
  "Cookies",
  "Login Data",
  "Local Storage",
  "Local State",
  "Preferences",
  "Secure Preferences",
  "Web Data",
  "Extension Cookies",
  // Firefox/Camoufox equivalents
  "cookies.sqlite",
  "key4.db",
  "logins.json",
  "cert9.db",
  "places.sqlite",
  "formhistory.sqlite",
  "permissions.sqlite",
  "prefs.js",
  "storage.sqlite",
];

fn is_critical_file(path: &str) -> bool {
  CRITICAL_FILE_PATTERNS
    .iter()
    .any(|pattern| path.contains(pattern))
}

/// Checkpoint all SQLite WAL files in a profile directory.
///
/// When a browser crashes or is killed, SQLite WAL files may contain
/// uncommitted data (e.g. cookies, login data). Since WAL files are
/// excluded from sync, we must checkpoint them into the main database
/// files before generating the manifest to avoid data loss.
fn checkpoint_sqlite_wal_files(profile_dir: &Path) {
  fn find_wal_files(dir: &Path, wal_files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
      return;
    };
    for entry in entries.flatten() {
      let path = entry.path();
      if path.is_dir() {
        find_wal_files(&path, wal_files);
      } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.ends_with("-wal") {
          wal_files.push(path);
        }
      }
    }
  }

  let mut wal_files = Vec::new();
  find_wal_files(profile_dir, &mut wal_files);

  for wal_path in &wal_files {
    // Only checkpoint non-empty WAL files
    let is_non_empty = fs::metadata(wal_path).map(|m| m.len() > 0).unwrap_or(false);
    if !is_non_empty {
      continue;
    }

    // Derive the main database path by stripping the "-wal" suffix
    let db_path_str = wal_path.to_string_lossy();
    let db_path = PathBuf::from(db_path_str.strip_suffix("-wal").unwrap());

    if !db_path.exists() {
      continue;
    }

    match rusqlite::Connection::open(&db_path) {
      Ok(conn) => match conn.pragma_update(None, "wal_checkpoint", "TRUNCATE") {
        Ok(_) => {
          log::info!(
            "Checkpointed WAL for: {}",
            db_path.file_name().unwrap_or_default().to_string_lossy()
          );
        }
        Err(e) => {
          log::warn!("Failed to checkpoint WAL for {}: {}", db_path.display(), e);
        }
      },
      Err(e) => {
        log::warn!(
          "Failed to open DB for WAL checkpoint {}: {}",
          db_path.display(),
          e
        );
      }
    }
  }
}

/// Resume state persisted to disk so interrupted syncs can continue
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SyncResumeState {
  profile_id: String,
  direction: String,
  started_at: String,
  completed_files: HashSet<String>,
}

impl SyncResumeState {
  fn path(profile_dir: &Path) -> std::path::PathBuf {
    profile_dir.join(".donut-sync").join("resume-state.json")
  }

  fn load(profile_dir: &Path) -> Option<Self> {
    let path = Self::path(profile_dir);
    let content = fs::read_to_string(&path).ok()?;
    let state: Self = serde_json::from_str(&content).ok()?;
    // Discard if older than 12 hours (presigned URLs expire in 1h but files may still be there)
    if let Ok(started) = DateTime::parse_from_rfc3339(&state.started_at) {
      let age = Utc::now() - started.with_timezone(&Utc);
      if age.num_hours() > 12 {
        let _ = fs::remove_file(&path);
        return None;
      }
    }
    Some(state)
  }

  fn save(&self, profile_dir: &Path) -> SyncResult<()> {
    let path = Self::path(profile_dir);
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)
        .map_err(|e| SyncError::IoError(format!("Failed to create resume state dir: {e}")))?;
    }
    let json = serde_json::to_string(self).map_err(|e| {
      SyncError::SerializationError(format!("Failed to serialize resume state: {e}"))
    })?;
    fs::write(&path, json)
      .map_err(|e| SyncError::IoError(format!("Failed to write resume state: {e}")))?;
    Ok(())
  }

  fn delete(profile_dir: &Path) {
    let path = Self::path(profile_dir);
    let _ = fs::remove_file(&path);
  }
}

/// Tracks live sync progress and emits throttled events to the frontend
struct SyncProgressTracker {
  profile_id: String,
  profile_name: String,
  phase: String,
  total_files: u64,
  total_bytes: u64,
  completed_files: AtomicU64,
  completed_bytes: AtomicU64,
  failed_count: AtomicU64,
  start_time: Instant,
  last_emit: TokioMutex<Instant>,
}

impl SyncProgressTracker {
  fn new(
    profile_id: String,
    profile_name: String,
    phase: &str,
    total_files: u64,
    total_bytes: u64,
  ) -> Self {
    Self {
      profile_id,
      profile_name,
      phase: phase.to_string(),
      total_files,
      total_bytes,
      completed_files: AtomicU64::new(0),
      completed_bytes: AtomicU64::new(0),
      failed_count: AtomicU64::new(0),
      start_time: Instant::now(),
      last_emit: TokioMutex::new(Instant::now() - std::time::Duration::from_secs(1)),
    }
  }

  fn record_success(&self, bytes: u64) {
    self.completed_files.fetch_add(1, Ordering::Relaxed);
    self.completed_bytes.fetch_add(bytes, Ordering::Relaxed);
    self.maybe_emit();
  }

  fn record_failure(&self) {
    self.completed_files.fetch_add(1, Ordering::Relaxed);
    self.failed_count.fetch_add(1, Ordering::Relaxed);
    self.maybe_emit();
  }

  fn maybe_emit(&self) {
    let Ok(mut last) = self.last_emit.try_lock() else {
      return;
    };
    if last.elapsed().as_millis() < 250 {
      return;
    }
    *last = Instant::now();
    self.emit_progress();
  }

  fn emit_final(&self) {
    self.emit_progress();
  }

  fn emit_progress(&self) {
    let completed_bytes = self.completed_bytes.load(Ordering::Relaxed);
    let elapsed = self.start_time.elapsed().as_secs_f64().max(0.1);
    let speed = (completed_bytes as f64 / elapsed) as u64;
    let remaining_bytes = self.total_bytes.saturating_sub(completed_bytes);
    let eta = if speed > 0 {
      remaining_bytes / speed
    } else {
      0
    };

    let _ = events::emit(
      "profile-sync-progress",
      serde_json::json!({
        "profile_id": self.profile_id,
        "profile_name": self.profile_name,
        "phase": self.phase,
        "completed_files": self.completed_files.load(Ordering::Relaxed),
        "total_files": self.total_files,
        "completed_bytes": completed_bytes,
        "total_bytes": self.total_bytes,
        "speed_bytes_per_sec": speed,
        "eta_seconds": eta,
        "failed_count": self.failed_count.load(Ordering::Relaxed),
      }),
    );
  }
}

/// Check if sync is configured (cloud or self-hosted)
pub fn is_sync_configured() -> bool {
  if crate::cloud_auth::CLOUD_AUTH.has_active_paid_subscription_sync() {
    return true;
  }
  let manager = SettingsManager::instance();
  if let Ok(settings) = manager.load_settings() {
    return settings.sync_server_url.is_some();
  }
  false
}

pub struct SyncEngine {
  client: SyncClient,
}

impl SyncEngine {
  pub fn new(server_url: String, token: String) -> Self {
    Self {
      client: SyncClient::new(server_url, token),
    }
  }

  pub async fn create_from_settings(app_handle: &tauri::AppHandle) -> Result<Self, String> {
    // Cloud auth takes priority
    if crate::cloud_auth::CLOUD_AUTH.is_logged_in().await {
      let url = crate::cloud_auth::CLOUD_SYNC_URL.to_string();
      let token = crate::cloud_auth::CLOUD_AUTH
        .get_or_refresh_sync_token()
        .await
        .map_err(|e| format!("Failed to get cloud sync token: {e}"))?
        .ok_or_else(|| "Cloud sync token not available".to_string())?;
      return Ok(Self::new(url, token));
    }

    // Fall back to self-hosted settings
    let manager = SettingsManager::instance();
    let settings = manager
      .load_settings()
      .map_err(|e| format!("Failed to load settings: {e}"))?;

    let server_url = settings
      .sync_server_url
      .ok_or_else(|| "Sync server URL not configured".to_string())?;

    let token = manager
      .get_sync_token(app_handle)
      .await
      .map_err(|e| format!("Failed to get sync token: {e}"))?
      .ok_or_else(|| "Sync token not configured".to_string())?;

    Ok(Self::new(server_url, token))
  }

  /// Get the key prefix for team profiles. Returns empty string for personal profiles.
  async fn get_team_key_prefix(profile: &BrowserProfile) -> String {
    if profile.created_by_id.is_some() {
      if let Some(auth) = crate::cloud_auth::CLOUD_AUTH.get_user().await {
        if let Some(team_id) = &auth.user.team_id {
          return format!("teams/{}/", team_id);
        }
      }
    }
    String::new()
  }

  /// Check if this is a self-hosted sync (no cloud login).
  async fn is_self_hosted_sync() -> bool {
    !crate::cloud_auth::CLOUD_AUTH.is_logged_in().await
  }

  pub async fn sync_profile(
    &self,
    app_handle: &tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> SyncResult<()> {
    if profile.is_cross_os() {
      log::info!(
        "Cross-OS profile: {} ({}) — syncing metadata only",
        profile.name,
        profile.id
      );
      return self.sync_cross_os_metadata(app_handle, profile).await;
    }

    // Skip team profiles for self-hosted sync
    if Self::is_self_hosted_sync().await && profile.created_by_id.is_some() {
      log::info!(
        "Skipping team profile for self-hosted sync: {} ({})",
        profile.name,
        profile.id
      );
      return Ok(());
    }

    // Skip if profile is currently running locally
    if profile.process_id.is_some() {
      log::info!(
        "Skipping sync for running profile: {} ({})",
        profile.name,
        profile.id
      );
      return Ok(());
    }

    // Skip if profile is locked by another team member
    if crate::team_lock::TEAM_LOCK
      .is_locked_by_another(&profile.id.to_string())
      .await
    {
      log::info!(
        "Skipping sync for profile locked by another team member: {} ({})",
        profile.name,
        profile.id
      );
      return Ok(());
    }

    // Derive encryption key if encrypted sync
    let encryption_key = if profile.is_encrypted_sync() {
      let password = encryption::load_e2e_password()
        .map_err(|e| SyncError::InvalidData(format!("Failed to load E2E password: {e}")))?
        .ok_or_else(|| {
          let _ = events::emit("profile-sync-e2e-password-required", ());
          SyncError::InvalidData("E2E password not set".to_string())
        })?;
      let salt = profile.encryption_salt.as_deref().ok_or_else(|| {
        SyncError::InvalidData("Encryption salt missing on encrypted profile".to_string())
      })?;
      let key = encryption::derive_profile_key(&password, salt)
        .map_err(|e| SyncError::InvalidData(format!("Key derivation failed: {e}")))?;
      Some(key)
    } else {
      None
    };

    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profile_dir = profiles_dir.join(profile.id.to_string());
    let profile_id = profile.id.to_string();

    // Determine team key prefix for team profiles
    let key_prefix = Self::get_team_key_prefix(profile).await;

    log::info!(
      "Starting delta sync for profile: {} ({}){}",
      profile.name,
      profile_id,
      if key_prefix.is_empty() {
        String::new()
      } else {
        format!(" [team prefix: {}]", key_prefix)
      }
    );

    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "profile_name": profile.name,
        "status": "syncing"
      }),
    );

    // Ensure profile directory exists
    fs::create_dir_all(&profile_dir).map_err(|e| {
      SyncError::IoError(format!(
        "Failed to create profile directory {}: {e}",
        profile_dir.display()
      ))
    })?;

    // Checkpoint any SQLite WAL files to ensure all data is in the main DB
    // before we generate the manifest (WAL files are excluded from sync)
    checkpoint_sqlite_wal_files(&profile_dir);

    // Load or create hash cache
    let cache_path = get_cache_path(&profile_dir);
    let mut hash_cache = HashCache::load(&cache_path);

    // Generate local manifest
    let local_manifest = generate_manifest(&profile_id, &profile_dir, &mut hash_cache)?;

    let total_size: u64 = local_manifest.files.iter().map(|f| f.size).sum();
    let has_cookies = local_manifest
      .files
      .iter()
      .any(|f| f.path.contains("Cookies") || f.path.contains("cookies"));
    let has_local_state = local_manifest
      .files
      .iter()
      .any(|f| f.path.contains("Local State"));
    log::info!(
      "Profile {} manifest: {} files, {} bytes total, cookies={}, local_state={}",
      profile_id,
      local_manifest.files.len(),
      total_size,
      has_cookies,
      has_local_state
    );

    // Save the hash cache for future runs
    hash_cache.save(&cache_path)?;

    // Try to download remote manifest
    let remote_manifest_key = format!("{}profiles/{}/manifest.json", key_prefix, profile_id);
    let remote_manifest = self
      .download_manifest(&remote_manifest_key, encryption_key.as_ref())
      .await?;

    // Compute diff
    let diff = compute_diff(&local_manifest, remote_manifest.as_ref());

    if diff.is_empty() {
      log::info!("Profile {} is already in sync", profile_id);
      let _ = events::emit(
        "profile-sync-status",
        serde_json::json!({
          "profile_id": profile_id,
          "profile_name": profile.name,
          "status": "synced"
        }),
      );
      return Ok(());
    }

    let upload_bytes: u64 = diff.files_to_upload.iter().map(|f| f.size).sum();
    let download_bytes: u64 = diff.files_to_download.iter().map(|f| f.size).sum();
    let total_files = diff.files_to_upload.len()
      + diff.files_to_download.len()
      + diff.files_to_delete_local.len()
      + diff.files_to_delete_remote.len();

    log::info!(
      "Profile {} diff: {} to upload, {} to download, {} to delete local, {} to delete remote",
      profile_id,
      diff.files_to_upload.len(),
      diff.files_to_download.len(),
      diff.files_to_delete_local.len(),
      diff.files_to_delete_remote.len()
    );

    let _ = events::emit(
      "profile-sync-progress",
      serde_json::json!({
        "profile_id": profile_id,
        "profile_name": profile.name,
        "phase": "started",
        "total_files": total_files,
        "total_bytes": upload_bytes + download_bytes
      }),
    );

    // Perform uploads
    if !diff.files_to_upload.is_empty() {
      self
        .upload_profile_files(
          app_handle,
          &profile_id,
          &profile.name,
          &profile_dir,
          &diff.files_to_upload,
          encryption_key.as_ref(),
          &key_prefix,
        )
        .await?;
    }

    // Perform downloads
    if !diff.files_to_download.is_empty() {
      self
        .download_profile_files(
          app_handle,
          &profile_id,
          &profile.name,
          &profile_dir,
          &diff.files_to_download,
          encryption_key.as_ref(),
          &key_prefix,
        )
        .await?;
    }

    // Delete local files that don't exist remotely (when remote is newer)
    for path in &diff.files_to_delete_local {
      let file_path = profile_dir.join(path);
      if file_path.exists() {
        let _ = fs::remove_file(&file_path);
        log::debug!("Deleted local file: {}", path);
      }
    }

    // Delete remote files that don't exist locally (when local is newer)
    for path in &diff.files_to_delete_remote {
      let remote_key = format!("{}profiles/{}/files/{}", key_prefix, profile_id, path);
      let _ = self.client.delete(&remote_key, None).await;
      log::debug!("Deleted remote file: {}", path);
    }

    // Upload metadata.json (sanitized profile)
    self
      .upload_profile_metadata(&profile_id, profile, &key_prefix)
      .await?;

    // If we recovered from an empty local state (downloaded everything from remote),
    // regenerate the manifest from the actual files now on disk so we don't
    // overwrite the remote manifest with an empty one.
    let final_manifest = if local_manifest.files.is_empty() && !diff.files_to_download.is_empty() {
      let mut new_cache = HashCache::load(&cache_path);
      let mut regenerated = generate_manifest(&profile_id, &profile_dir, &mut new_cache)?;
      new_cache.save(&cache_path)?;
      regenerated.encrypted = encryption_key.is_some();
      regenerated
    } else {
      let mut m = local_manifest;
      m.encrypted = encryption_key.is_some();
      m
    };

    // Upload manifest.json last for atomicity
    self
      .upload_manifest(
        &profile_id,
        &final_manifest,
        encryption_key.as_ref(),
        &key_prefix,
      )
      .await?;

    // Sync completed successfully — clean up resume state
    SyncResumeState::delete(&profile_dir);

    // Sync associated proxy, group, and VPN
    if let Some(proxy_id) = &profile.proxy_id {
      let _ = self.sync_proxy(proxy_id, Some(app_handle)).await;
    }
    if let Some(group_id) = &profile.group_id {
      let _ = self.sync_group(group_id, Some(app_handle)).await;
    }
    if let Some(vpn_id) = &profile.vpn_id {
      let _ = self.sync_vpn(vpn_id, Some(app_handle)).await;
    }

    // Download remote metadata and merge changes (name, tags, notes, etc.)
    let remote_metadata_key = format!("{}profiles/{}/metadata.json", key_prefix, profile_id);
    if let Ok(remote_meta) = self.download_profile_metadata(&remote_metadata_key).await {
      let mut updated_profile = profile.clone();
      // Merge fields that can be changed on other devices
      updated_profile.name = remote_meta.name;
      updated_profile.tags = remote_meta.tags;
      updated_profile.note = remote_meta.note;
      updated_profile.proxy_id = remote_meta.proxy_id;
      updated_profile.vpn_id = remote_meta.vpn_id;
      updated_profile.group_id = remote_meta.group_id;
      updated_profile.last_sync = Some(
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap()
          .as_secs(),
      );
      let _ = profile_manager.save_profile(&updated_profile);
    } else {
      // Fallback: just update last_sync
      let mut updated_profile = profile.clone();
      updated_profile.last_sync = Some(
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap()
          .as_secs(),
      );
      let _ = profile_manager.save_profile(&updated_profile);
    }
    let _ = events::emit("profiles-changed", ());

    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "profile_name": profile.name,
        "status": "synced"
      }),
    );

    log::info!("Profile {} synced successfully", profile_id);
    Ok(())
  }

  async fn download_manifest(
    &self,
    key: &str,
    encryption_key: Option<&[u8; 32]>,
  ) -> SyncResult<Option<SyncManifest>> {
    let stat = self.client.stat(key).await?;
    if !stat.exists {
      return Ok(None);
    }

    let presign = self.client.presign_download(key).await?;
    let data = self.client.download_bytes(&presign.url).await?;

    // Try parsing as plaintext JSON first (unencrypted or backwards-compatible)
    if let Ok(manifest) = serde_json::from_slice::<SyncManifest>(&data) {
      return Ok(Some(manifest));
    }

    // If plaintext parse failed and we have an encryption key, try decrypting
    if let Some(key) = encryption_key {
      let decrypted = encryption::decrypt_bytes(key, &data)
        .map_err(|e| SyncError::InvalidData(format!("Failed to decrypt manifest: {e}")))?;
      let manifest: SyncManifest = serde_json::from_slice(&decrypted).map_err(|e| {
        SyncError::SerializationError(format!("Failed to parse decrypted manifest: {e}"))
      })?;
      return Ok(Some(manifest));
    }

    Err(SyncError::SerializationError(
      "Failed to parse manifest (not valid JSON and no encryption key available)".to_string(),
    ))
  }

  async fn upload_manifest(
    &self,
    profile_id: &str,
    manifest: &SyncManifest,
    encryption_key: Option<&[u8; 32]>,
    key_prefix: &str,
  ) -> SyncResult<()> {
    let json = serde_json::to_string_pretty(manifest)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize manifest: {e}")))?;

    let upload_data = if let Some(key) = encryption_key {
      encryption::encrypt_bytes(key, json.as_bytes())
        .map_err(|e| SyncError::InvalidData(format!("Failed to encrypt manifest: {e}")))?
    } else {
      json.into_bytes()
    };

    let content_type = if encryption_key.is_some() {
      "application/octet-stream"
    } else {
      "application/json"
    };

    let remote_key = format!("{}profiles/{}/manifest.json", key_prefix, profile_id);
    let presign = self
      .client
      .presign_upload(&remote_key, Some(content_type))
      .await?;

    self
      .client
      .upload_bytes(&presign.url, &upload_data, Some(content_type))
      .await?;

    Ok(())
  }

  async fn download_profile_metadata(&self, key: &str) -> SyncResult<BrowserProfile> {
    let stat = self.client.stat(key).await?;
    if !stat.exists {
      return Err(SyncError::InvalidData(
        "Remote metadata not found".to_string(),
      ));
    }

    let presign = self.client.presign_download(key).await?;
    let data = self.client.download_bytes(&presign.url).await?;
    let profile: BrowserProfile = serde_json::from_slice(&data)
      .map_err(|e| SyncError::SerializationError(format!("Failed to parse metadata: {e}")))?;

    Ok(profile)
  }

  /// Sync only metadata for cross-OS profiles (tags, notes, proxies, groups).
  /// No browser files are synced.
  async fn sync_cross_os_metadata(
    &self,
    app_handle: &tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> SyncResult<()> {
    let profile_id = profile.id.to_string();
    let key_prefix = Self::get_team_key_prefix(profile).await;
    let profile_manager = ProfileManager::instance();

    // Upload our metadata
    self
      .upload_profile_metadata(&profile_id, profile, &key_prefix)
      .await?;

    // Download remote metadata and merge if remote has changes
    let remote_metadata_key = format!("{}profiles/{}/metadata.json", key_prefix, profile_id);
    if let Ok(remote_meta) = self.download_profile_metadata(&remote_metadata_key).await {
      let mut updated = profile.clone();
      updated.name = remote_meta.name;
      updated.tags = remote_meta.tags;
      updated.note = remote_meta.note;
      updated.proxy_id = remote_meta.proxy_id;
      updated.vpn_id = remote_meta.vpn_id;
      updated.group_id = remote_meta.group_id;
      updated.last_sync = Some(
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap()
          .as_secs(),
      );
      let _ = profile_manager.save_profile(&updated);
    }

    // Sync associated entities
    if let Some(proxy_id) = &profile.proxy_id {
      let _ = self.sync_proxy(proxy_id, Some(app_handle)).await;
    }
    if let Some(group_id) = &profile.group_id {
      let _ = self.sync_group(group_id, Some(app_handle)).await;
    }

    let _ = events::emit("profiles-changed", ());
    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "profile_name": profile.name,
        "status": "synced"
      }),
    );

    log::info!("Cross-OS profile {} metadata synced", profile_id);
    Ok(())
  }

  async fn upload_profile_metadata(
    &self,
    profile_id: &str,
    profile: &BrowserProfile,
    key_prefix: &str,
  ) -> SyncResult<()> {
    let mut sanitized = profile.clone();
    sanitized.process_id = None;
    sanitized.last_launch = None;

    let json = serde_json::to_string_pretty(&sanitized)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize profile: {e}")))?;

    let remote_key = format!("{}profiles/{}/metadata.json", key_prefix, profile_id);
    let presign = self
      .client
      .presign_upload(&remote_key, Some("application/json"))
      .await?;

    self
      .client
      .upload_bytes(&presign.url, json.as_bytes(), Some("application/json"))
      .await?;

    Ok(())
  }

  #[allow(clippy::too_many_arguments)]
  async fn upload_profile_files(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    profile_name: &str,
    profile_dir: &Path,
    files: &[super::manifest::ManifestFileEntry],
    encryption_key: Option<&[u8; 32]>,
    key_prefix: &str,
  ) -> SyncResult<()> {
    if files.is_empty() {
      return Ok(());
    }

    // Load resume state to skip already-uploaded files
    let mut resume_state = SyncResumeState::load(profile_dir)
      .filter(|s| s.profile_id == profile_id && s.direction == "upload");

    let already_done: HashSet<String> = resume_state
      .as_ref()
      .map(|s| s.completed_files.clone())
      .unwrap_or_default();

    let files_to_process: Vec<_> = files
      .iter()
      .filter(|f| !already_done.contains(&f.path))
      .collect();
    let skipped = files.len() - files_to_process.len();

    if skipped > 0 {
      log::info!(
        "Resume: skipping {} already-uploaded files, processing {} remaining for profile {}",
        skipped,
        files_to_process.len(),
        profile_id
      );
    }

    log::info!(
      "Uploading {} files for profile {}",
      files_to_process.len(),
      profile_id
    );

    if files_to_process.is_empty() {
      return Ok(());
    }

    // Initialize resume state if not resuming
    if resume_state.is_none() {
      resume_state = Some(SyncResumeState {
        profile_id: profile_id.to_string(),
        direction: "upload".to_string(),
        started_at: Utc::now().to_rfc3339(),
        completed_files: HashSet::new(),
      });
    }
    let resume_state = Arc::new(TokioMutex::new(resume_state.unwrap()));

    // Get batch presigned URLs
    let items: Vec<(String, Option<String>)> = files_to_process
      .iter()
      .map(|f| {
        let key = format!("{}profiles/{}/files/{}", key_prefix, profile_id, f.path);
        let content_type = mime_guess::from_path(&f.path)
          .first()
          .map(|m| m.to_string());
        (key, content_type)
      })
      .collect();

    let batch_response = self.client.presign_upload_batch(items).await?;

    // Build URL map
    let url_map: HashMap<String, String> = batch_response
      .items
      .into_iter()
      .map(|item| (item.key, item.url))
      .collect();

    let total_bytes: u64 = files.iter().map(|f| f.size).sum();
    let already_bytes: u64 = files
      .iter()
      .filter(|f| already_done.contains(&f.path))
      .map(|f| f.size)
      .sum();

    let tracker = Arc::new(SyncProgressTracker::new(
      profile_id.to_string(),
      profile_name.to_string(),
      "uploading",
      files.len() as u64,
      total_bytes,
    ));
    // Pre-populate tracker with resumed progress
    tracker
      .completed_files
      .store(skipped as u64, Ordering::Relaxed);
    tracker
      .completed_bytes
      .store(already_bytes, Ordering::Relaxed);
    tracker.emit_final();

    let semaphore = Arc::new(Semaphore::new(SYNC_CONCURRENCY));
    let client = self.client.clone();
    let profile_dir = profile_dir.to_path_buf();
    let profile_id_owned = profile_id.to_string();
    let enc_key = encryption_key.copied();

    type FileResult = Result<String, (String, String, bool)>;
    let mut handles: Vec<tokio::task::JoinHandle<FileResult>> = Vec::new();

    // Counter for batching resume state saves
    let save_counter = Arc::new(AtomicU64::new(0));

    for file in &files_to_process {
      let sem = semaphore.clone();
      let file_path = profile_dir.join(&file.path);
      let relative_path = file.path.clone();
      let file_size = file.size;
      let remote_key = format!(
        "{}profiles/{}/files/{}",
        key_prefix, profile_id_owned, file.path
      );
      let url = url_map.get(&remote_key).cloned();
      let critical = is_critical_file(&file.path);

      if url.is_none() {
        log::warn!("No presigned URL for {}", remote_key);
        if critical {
          return Err(SyncError::NetworkError(format!(
            "No presigned URL for critical file: {}",
            file.path
          )));
        }
        continue;
      }

      let url = url.unwrap();
      let client = client.clone();
      let tracker = tracker.clone();
      let resume_state = resume_state.clone();
      let save_counter = save_counter.clone();
      let profile_dir_clone = profile_dir.clone();
      let content_type = mime_guess::from_path(&file.path)
        .first()
        .map(|m| m.to_string());

      handles.push(tokio::spawn(async move {
        let _permit = sem.acquire().await.unwrap();

        let data = match fs::read(&file_path) {
          Ok(d) => d,
          Err(e) if e.kind() == std::io::ErrorKind::NotFound && !critical => {
            log::debug!("File disappeared, skipping: {}", file_path.display());
            tracker.record_success(0);
            return Ok(relative_path);
          }
          Err(e) => {
            let msg = format!("Failed to read {}: {}", file_path.display(), e);
            log::warn!("{}", msg);
            tracker.record_failure();
            return Err((relative_path, msg, critical));
          }
        };

        let upload_data = if let Some(ref key) = enc_key {
          match encryption::encrypt_bytes(key, &data) {
            Ok(encrypted) => encrypted,
            Err(e) => {
              let msg = format!("Failed to encrypt {}: {}", file_path.display(), e);
              log::warn!("{}", msg);
              tracker.record_failure();
              return Err((relative_path, msg, critical));
            }
          }
        } else {
          data
        };

        // Retry loop for network uploads
        let mut last_err = String::new();
        for attempt in 0..MAX_FILE_RETRIES {
          match client
            .upload_bytes(&url, &upload_data, content_type.as_deref())
            .await
          {
            Ok(()) => {
              tracker.record_success(file_size);

              // Record in resume state, save periodically
              {
                let mut state = resume_state.lock().await;
                state.completed_files.insert(relative_path.clone());
                let count = save_counter.fetch_add(1, Ordering::Relaxed);
                if count.is_multiple_of(50) {
                  let _ = state.save(&profile_dir_clone);
                }
              }

              return Ok(relative_path);
            }
            Err(e) => {
              last_err = format!("{}", e);
              if attempt < MAX_FILE_RETRIES - 1 {
                log::debug!(
                  "Retry {}/{} for {}: {}",
                  attempt + 1,
                  MAX_FILE_RETRIES,
                  relative_path,
                  last_err
                );
                tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1)))
                  .await;
              }
            }
          }
        }

        let msg = format!(
          "Failed to upload {} after {} retries: {}",
          relative_path, MAX_FILE_RETRIES, last_err
        );
        log::warn!("{}", msg);
        tracker.record_failure();
        Err((relative_path, msg, critical))
      }));
    }

    // Collect results
    let mut critical_failures = Vec::new();
    let mut non_critical_failures = Vec::new();

    for handle in handles {
      match handle.await {
        Ok(Ok(_)) => {}
        Ok(Err((path, msg, true))) => critical_failures.push((path, msg)),
        Ok(Err((path, msg, false))) => non_critical_failures.push((path, msg)),
        Err(e) => {
          log::warn!("Upload task panicked: {}", e);
        }
      }
    }

    // Final resume state save
    {
      let state = resume_state.lock().await;
      let _ = state.save(&profile_dir);
    }

    tracker.emit_final();

    if !non_critical_failures.is_empty() {
      log::warn!(
        "Upload completed with {} non-critical failures for profile {}",
        non_critical_failures.len(),
        profile_id_owned
      );
    }

    if !critical_failures.is_empty() {
      let file_list: Vec<&str> = critical_failures.iter().map(|(p, _)| p.as_str()).collect();
      return Err(SyncError::IoError(format!(
        "Critical files failed to upload: {}. Sync aborted to prevent data loss.",
        file_list.join(", ")
      )));
    }

    Ok(())
  }

  #[allow(clippy::too_many_arguments)]
  async fn download_profile_files(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    profile_name: &str,
    profile_dir: &Path,
    files: &[super::manifest::ManifestFileEntry],
    encryption_key: Option<&[u8; 32]>,
    key_prefix: &str,
  ) -> SyncResult<()> {
    if files.is_empty() {
      return Ok(());
    }

    // Load resume state to skip already-downloaded files
    let mut resume_state = SyncResumeState::load(profile_dir)
      .filter(|s| s.profile_id == profile_id && s.direction == "download");

    let already_done: HashSet<String> = resume_state
      .as_ref()
      .map(|s| s.completed_files.clone())
      .unwrap_or_default();

    let files_to_process: Vec<_> = files
      .iter()
      .filter(|f| !already_done.contains(&f.path))
      .collect();
    let skipped = files.len() - files_to_process.len();

    if skipped > 0 {
      log::info!(
        "Resume: skipping {} already-downloaded files, processing {} remaining for profile {}",
        skipped,
        files_to_process.len(),
        profile_id
      );
    }

    log::info!(
      "Downloading {} files for profile {}",
      files_to_process.len(),
      profile_id
    );

    if files_to_process.is_empty() {
      return Ok(());
    }

    // Initialize resume state if not resuming
    if resume_state.is_none() {
      resume_state = Some(SyncResumeState {
        profile_id: profile_id.to_string(),
        direction: "download".to_string(),
        started_at: Utc::now().to_rfc3339(),
        completed_files: HashSet::new(),
      });
    }
    let resume_state = Arc::new(TokioMutex::new(resume_state.unwrap()));

    // Get batch presigned URLs
    let keys: Vec<String> = files_to_process
      .iter()
      .map(|f| format!("{}profiles/{}/files/{}", key_prefix, profile_id, f.path))
      .collect();

    let batch_response = self.client.presign_download_batch(keys).await?;

    // Build URL map
    let url_map: HashMap<String, String> = batch_response
      .items
      .into_iter()
      .map(|item| (item.key, item.url))
      .collect();

    let total_bytes: u64 = files.iter().map(|f| f.size).sum();
    let already_bytes: u64 = files
      .iter()
      .filter(|f| already_done.contains(&f.path))
      .map(|f| f.size)
      .sum();

    let tracker = Arc::new(SyncProgressTracker::new(
      profile_id.to_string(),
      profile_name.to_string(),
      "downloading",
      files.len() as u64,
      total_bytes,
    ));
    tracker
      .completed_files
      .store(skipped as u64, Ordering::Relaxed);
    tracker
      .completed_bytes
      .store(already_bytes, Ordering::Relaxed);
    tracker.emit_final();

    let semaphore = Arc::new(Semaphore::new(SYNC_CONCURRENCY));
    let client = self.client.clone();
    let profile_dir = profile_dir.to_path_buf();
    let profile_id_owned = profile_id.to_string();
    let enc_key = encryption_key.copied();

    type FileResult = Result<String, (String, String, bool)>;
    let mut handles: Vec<tokio::task::JoinHandle<FileResult>> = Vec::new();

    let save_counter = Arc::new(AtomicU64::new(0));

    for file in &files_to_process {
      let sem = semaphore.clone();
      let file_path = profile_dir.join(&file.path);
      let relative_path = file.path.clone();
      let file_size = file.size;
      let remote_key = format!(
        "{}profiles/{}/files/{}",
        key_prefix, profile_id_owned, file.path
      );
      let url = url_map.get(&remote_key).cloned();
      let critical = is_critical_file(&file.path);

      if url.is_none() {
        log::warn!("No presigned URL for {}", remote_key);
        if critical {
          return Err(SyncError::NetworkError(format!(
            "No presigned URL for critical file: {}",
            file.path
          )));
        }
        continue;
      }

      let url = url.unwrap();
      let client = client.clone();
      let tracker = tracker.clone();
      let resume_state = resume_state.clone();
      let save_counter = save_counter.clone();
      let profile_dir_clone = profile_dir.clone();

      handles.push(tokio::spawn(async move {
        let _permit = sem.acquire().await.unwrap();

        // Retry loop for network downloads
        let mut last_err = String::new();
        for attempt in 0..MAX_FILE_RETRIES {
          match client.download_bytes(&url).await {
            Ok(data) => {
              let write_data = if let Some(ref key) = enc_key {
                match encryption::decrypt_bytes(key, &data) {
                  Ok(decrypted) => decrypted,
                  Err(e) => {
                    let msg = format!("Failed to decrypt {}: {}", relative_path, e);
                    log::warn!("{}", msg);
                    tracker.record_failure();
                    return Err((relative_path, msg, critical));
                  }
                }
              } else {
                data
              };

              if let Some(parent) = file_path.parent() {
                let _ = fs::create_dir_all(parent);
              }
              if let Err(e) = fs::write(&file_path, &write_data) {
                let msg = format!("Failed to write {}: {}", file_path.display(), e);
                log::warn!("{}", msg);
                tracker.record_failure();
                return Err((relative_path, msg, critical));
              }

              tracker.record_success(file_size);

              {
                let mut state = resume_state.lock().await;
                state.completed_files.insert(relative_path.clone());
                let count = save_counter.fetch_add(1, Ordering::Relaxed);
                if count.is_multiple_of(50) {
                  let _ = state.save(&profile_dir_clone);
                }
              }

              return Ok(relative_path);
            }
            Err(e) => {
              last_err = format!("{}", e);
              if attempt < MAX_FILE_RETRIES - 1 {
                log::debug!(
                  "Retry {}/{} for {}: {}",
                  attempt + 1,
                  MAX_FILE_RETRIES,
                  relative_path,
                  last_err
                );
                tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1)))
                  .await;
              }
            }
          }
        }

        let msg = format!(
          "Failed to download {} after {} retries: {}",
          relative_path, MAX_FILE_RETRIES, last_err
        );
        log::warn!("{}", msg);
        tracker.record_failure();
        Err((relative_path, msg, critical))
      }));
    }

    let mut critical_failures = Vec::new();
    let mut non_critical_failures = Vec::new();

    for handle in handles {
      match handle.await {
        Ok(Ok(_)) => {}
        Ok(Err((path, msg, true))) => critical_failures.push((path, msg)),
        Ok(Err((path, msg, false))) => non_critical_failures.push((path, msg)),
        Err(e) => {
          log::warn!("Download task panicked: {}", e);
        }
      }
    }

    // Final resume state save
    {
      let state = resume_state.lock().await;
      let _ = state.save(&profile_dir);
    }

    tracker.emit_final();

    if !non_critical_failures.is_empty() {
      log::warn!(
        "Download completed with {} non-critical failures for profile {}",
        non_critical_failures.len(),
        profile_id_owned
      );
    }

    if !critical_failures.is_empty() {
      let file_list: Vec<&str> = critical_failures.iter().map(|(p, _)| p.as_str()).collect();
      return Err(SyncError::IoError(format!(
        "Critical files failed to download: {}. Sync aborted to prevent data loss.",
        file_list.join(", ")
      )));
    }

    Ok(())
  }

  async fn sync_proxy(
    &self,
    proxy_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let proxy_manager = &crate::proxy_manager::PROXY_MANAGER;
    let proxies = proxy_manager.get_stored_proxies();
    let local_proxy = proxies.iter().find(|p| p.id == proxy_id).cloned();

    let remote_key = format!("proxies/{}.json", proxy_id);
    let stat = self.client.stat(&remote_key).await?;

    match (local_proxy, stat.exists) {
      (Some(proxy), true) => {
        // Both exist - compare timestamps
        let local_updated = proxy.last_sync.unwrap_or(0);
        let remote_updated: DateTime<Utc> = stat
          .last_modified
          .as_ref()
          .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
          .map(|dt| dt.with_timezone(&Utc))
          .unwrap_or_else(Utc::now);
        let remote_ts = remote_updated.timestamp() as u64;

        if remote_ts > local_updated {
          // Remote is newer - download
          self.download_proxy(proxy_id, app_handle).await?;
        } else if local_updated > remote_ts {
          // Local is newer - upload
          self.upload_proxy(&proxy).await?;
        }
      }
      (Some(proxy), false) => {
        // Only local exists - upload
        self.upload_proxy(&proxy).await?;
      }
      (None, true) => {
        // Only remote exists - download
        self.download_proxy(proxy_id, app_handle).await?;
      }
      (None, false) => {
        // Neither exists - nothing to do
        log::debug!("Proxy {} not found locally or remotely", proxy_id);
      }
    }

    Ok(())
  }

  async fn upload_proxy(&self, proxy: &crate::proxy_manager::StoredProxy) -> SyncResult<()> {
    let mut updated_proxy = proxy.clone();
    updated_proxy.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );

    let json = serde_json::to_string_pretty(&updated_proxy)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize proxy: {e}")))?;

    let remote_key = format!("proxies/{}.json", proxy.id);
    let presign = self
      .client
      .presign_upload(&remote_key, Some("application/json"))
      .await?;
    self
      .client
      .upload_bytes(&presign.url, json.as_bytes(), Some("application/json"))
      .await?;

    // Update local proxy with new last_sync
    let proxy_manager = &crate::proxy_manager::PROXY_MANAGER;
    let proxy_file = proxy_manager.get_proxy_file_path(&proxy.id);
    fs::write(&proxy_file, &json).map_err(|e| {
      SyncError::IoError(format!(
        "Failed to update proxy file {}: {e}",
        proxy_file.display()
      ))
    })?;

    log::info!("Proxy {} uploaded", proxy.id);
    Ok(())
  }

  async fn download_proxy(
    &self,
    proxy_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let remote_key = format!("proxies/{}.json", proxy_id);
    let presign = self.client.presign_download(&remote_key).await?;
    let data = self.client.download_bytes(&presign.url).await?;

    let mut proxy: crate::proxy_manager::StoredProxy = serde_json::from_slice(&data)
      .map_err(|e| SyncError::SerializationError(format!("Failed to parse proxy JSON: {e}")))?;

    proxy.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );

    let proxy_manager = &crate::proxy_manager::PROXY_MANAGER;
    let proxy_file = proxy_manager.get_proxy_file_path(&proxy.id);
    if let Some(parent) = proxy_file.parent() {
      fs::create_dir_all(parent).map_err(|e| {
        SyncError::IoError(format!(
          "Failed to create proxy directory {}: {e}",
          parent.display()
        ))
      })?;
    }

    let json = serde_json::to_string_pretty(&proxy)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize proxy: {e}")))?;
    fs::write(&proxy_file, &json).map_err(|e| {
      SyncError::IoError(format!(
        "Failed to write proxy file {}: {e}",
        proxy_file.display()
      ))
    })?;

    // Emit event for UI update
    if let Some(_handle) = app_handle {
      let _ = events::emit("stored-proxies-changed", ());
      let _ = events::emit(
        "proxy-sync-status",
        serde_json::json!({
          "id": proxy_id,
          "status": "synced"
        }),
      );
    }

    log::info!("Proxy {} downloaded", proxy_id);
    Ok(())
  }

  async fn sync_group(
    &self,
    group_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let local_group = {
      let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
      let groups = group_manager.get_all_groups().unwrap_or_default();
      groups.into_iter().find(|g| g.id == group_id)
    };

    let remote_key = format!("groups/{}.json", group_id);
    let stat = self.client.stat(&remote_key).await?;

    match (local_group, stat.exists) {
      (Some(group), true) => {
        // Both exist - compare timestamps
        let local_updated = group.last_sync.unwrap_or(0);
        let remote_updated: DateTime<Utc> = stat
          .last_modified
          .as_ref()
          .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
          .map(|dt| dt.with_timezone(&Utc))
          .unwrap_or_else(Utc::now);
        let remote_ts = remote_updated.timestamp() as u64;

        if remote_ts > local_updated {
          // Remote is newer - download
          self.download_group(group_id, app_handle).await?;
        } else if local_updated > remote_ts {
          // Local is newer - upload
          self.upload_group(&group).await?;
        }
      }
      (Some(group), false) => {
        // Only local exists - upload
        self.upload_group(&group).await?;
      }
      (None, true) => {
        // Only remote exists - download
        self.download_group(group_id, app_handle).await?;
      }
      (None, false) => {
        // Neither exists - nothing to do
        log::debug!("Group {} not found locally or remotely", group_id);
      }
    }

    Ok(())
  }

  async fn upload_group(&self, group: &crate::group_manager::ProfileGroup) -> SyncResult<()> {
    let mut updated_group = group.clone();
    updated_group.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );

    let json = serde_json::to_string_pretty(&updated_group)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize group: {e}")))?;

    let remote_key = format!("groups/{}.json", group.id);
    let presign = self
      .client
      .presign_upload(&remote_key, Some("application/json"))
      .await?;
    self
      .client
      .upload_bytes(&presign.url, json.as_bytes(), Some("application/json"))
      .await?;

    // Update local group with new last_sync
    {
      let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
      if let Err(e) = group_manager.update_group_internal(&updated_group) {
        log::warn!("Failed to update group last_sync: {}", e);
      }
    }

    log::info!("Group {} uploaded", group.id);
    Ok(())
  }

  async fn download_group(
    &self,
    group_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let remote_key = format!("groups/{}.json", group_id);
    let presign = self.client.presign_download(&remote_key).await?;
    let data = self.client.download_bytes(&presign.url).await?;

    let mut group: crate::group_manager::ProfileGroup = serde_json::from_slice(&data)
      .map_err(|e| SyncError::SerializationError(format!("Failed to parse group JSON: {e}")))?;

    group.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );

    // Save or update local group
    {
      let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
      if let Err(e) = group_manager.upsert_group_internal(&group) {
        log::warn!("Failed to save downloaded group: {}", e);
      }
    }

    // Emit event for UI update
    if let Some(_handle) = app_handle {
      let _ = events::emit("groups-changed", ());
      let _ = events::emit(
        "group-sync-status",
        serde_json::json!({
          "id": group_id,
          "status": "synced"
        }),
      );
    }

    log::info!("Group {} downloaded", group_id);
    Ok(())
  }

  pub async fn sync_proxy_by_id(&self, proxy_id: &str) -> SyncResult<()> {
    self.sync_proxy(proxy_id, None).await
  }

  pub async fn sync_proxy_by_id_with_handle(
    &self,
    proxy_id: &str,
    app_handle: &tauri::AppHandle,
  ) -> SyncResult<()> {
    self.sync_proxy(proxy_id, Some(app_handle)).await
  }

  pub async fn sync_group_by_id(&self, group_id: &str) -> SyncResult<()> {
    self.sync_group(group_id, None).await
  }

  pub async fn sync_group_by_id_with_handle(
    &self,
    group_id: &str,
    app_handle: &tauri::AppHandle,
  ) -> SyncResult<()> {
    self.sync_group(group_id, Some(app_handle)).await
  }

  pub async fn delete_profile(&self, profile_id: &str) -> SyncResult<()> {
    let prefix = format!("profiles/{}/", profile_id);
    let tombstone_key = format!("tombstones/profiles/{}.json", profile_id);

    let result = self
      .client
      .delete_prefix(&prefix, Some(&tombstone_key))
      .await?;

    log::info!(
      "Profile {} deleted from sync ({} objects removed)",
      profile_id,
      result.deleted_count
    );

    // Also delete from team path if user is on a team
    if let Some(auth) = crate::cloud_auth::CLOUD_AUTH.get_user().await {
      if let Some(team_id) = &auth.user.team_id {
        let team_prefix = format!("teams/{}/profiles/{}/", team_id, profile_id);
        let team_tombstone = format!("teams/{}/tombstones/profiles/{}.json", team_id, profile_id);
        let team_result = self
          .client
          .delete_prefix(&team_prefix, Some(&team_tombstone))
          .await?;
        if team_result.deleted_count > 0 {
          log::info!(
            "Profile {} deleted from team sync ({} objects removed)",
            profile_id,
            team_result.deleted_count
          );
        }
      }
    }

    Ok(())
  }

  pub async fn delete_proxy(&self, proxy_id: &str) -> SyncResult<()> {
    let remote_key = format!("proxies/{}.json", proxy_id);
    let tombstone_key = format!("tombstones/proxies/{}.json", proxy_id);

    self
      .client
      .delete(&remote_key, Some(&tombstone_key))
      .await?;

    log::info!("Proxy {} deleted from sync", proxy_id);
    Ok(())
  }

  pub async fn delete_group(&self, group_id: &str) -> SyncResult<()> {
    let remote_key = format!("groups/{}.json", group_id);
    let tombstone_key = format!("tombstones/groups/{}.json", group_id);

    self
      .client
      .delete(&remote_key, Some(&tombstone_key))
      .await?;

    log::info!("Group {} deleted from sync", group_id);
    Ok(())
  }

  async fn sync_vpn(&self, vpn_id: &str, app_handle: Option<&tauri::AppHandle>) -> SyncResult<()> {
    let local_vpn = {
      let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
      storage.load_config(vpn_id).ok()
    };

    let remote_key = format!("vpns/{}.json", vpn_id);
    let stat = self.client.stat(&remote_key).await?;

    match (local_vpn, stat.exists) {
      (Some(vpn), true) => {
        let local_updated = vpn.last_sync.unwrap_or(0);
        let remote_updated: DateTime<Utc> = stat
          .last_modified
          .as_ref()
          .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
          .map(|dt| dt.with_timezone(&Utc))
          .unwrap_or_else(Utc::now);
        let remote_ts = remote_updated.timestamp() as u64;

        if remote_ts > local_updated {
          self.download_vpn(vpn_id, app_handle).await?;
        } else if local_updated > remote_ts {
          self.upload_vpn(&vpn).await?;
        }
      }
      (Some(vpn), false) => {
        self.upload_vpn(&vpn).await?;
      }
      (None, true) => {
        self.download_vpn(vpn_id, app_handle).await?;
      }
      (None, false) => {
        log::debug!("VPN {} not found locally or remotely", vpn_id);
      }
    }

    Ok(())
  }

  async fn upload_vpn(&self, vpn: &crate::vpn::VpnConfig) -> SyncResult<()> {
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs();

    let mut updated_vpn = vpn.clone();
    updated_vpn.last_sync = Some(now);

    let json = serde_json::to_string_pretty(&updated_vpn)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize VPN: {e}")))?;

    let remote_key = format!("vpns/{}.json", vpn.id);
    let presign = self
      .client
      .presign_upload(&remote_key, Some("application/json"))
      .await?;
    self
      .client
      .upload_bytes(&presign.url, json.as_bytes(), Some("application/json"))
      .await?;

    // Update local VPN with new last_sync
    {
      let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
      if let Err(e) = storage.update_sync_fields(&vpn.id, vpn.sync_enabled, Some(now)) {
        log::warn!("Failed to update VPN last_sync: {}", e);
      }
    }

    log::info!("VPN {} uploaded", vpn.id);
    Ok(())
  }

  async fn download_vpn(
    &self,
    vpn_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let remote_key = format!("vpns/{}.json", vpn_id);
    let presign = self.client.presign_download(&remote_key).await?;
    let data = self.client.download_bytes(&presign.url).await?;

    let mut vpn: crate::vpn::VpnConfig = serde_json::from_slice(&data)
      .map_err(|e| SyncError::SerializationError(format!("Failed to parse VPN JSON: {e}")))?;

    vpn.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );
    vpn.sync_enabled = true;

    // Save via VPN storage (handles encryption)
    {
      let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
      if let Err(e) = storage.save_config(&vpn) {
        log::warn!("Failed to save downloaded VPN: {}", e);
      }
    }

    // Emit event for UI update
    if let Some(_handle) = app_handle {
      let _ = events::emit("vpn-configs-changed", ());
      let _ = events::emit(
        "vpn-sync-status",
        serde_json::json!({
          "id": vpn_id,
          "status": "synced"
        }),
      );
    }

    log::info!("VPN {} downloaded", vpn_id);
    Ok(())
  }

  pub async fn sync_vpn_by_id_with_handle(
    &self,
    vpn_id: &str,
    app_handle: &tauri::AppHandle,
  ) -> SyncResult<()> {
    self.sync_vpn(vpn_id, Some(app_handle)).await
  }

  pub async fn delete_vpn(&self, vpn_id: &str) -> SyncResult<()> {
    let remote_key = format!("vpns/{}.json", vpn_id);
    let tombstone_key = format!("tombstones/vpns/{}.json", vpn_id);

    self
      .client
      .delete(&remote_key, Some(&tombstone_key))
      .await?;

    log::info!("VPN {} deleted from sync", vpn_id);
    Ok(())
  }

  // Extension sync

  async fn sync_extension(
    &self,
    ext_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let local_ext = {
      let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      manager.get_extension(ext_id).ok()
    };

    let remote_key = format!("extensions/{}.json", ext_id);
    let stat = self.client.stat(&remote_key).await?;

    match (local_ext, stat.exists) {
      (Some(ext), true) => {
        let local_updated = ext.last_sync.unwrap_or(0);
        let remote_updated: DateTime<Utc> = stat
          .last_modified
          .as_ref()
          .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
          .map(|dt| dt.with_timezone(&Utc))
          .unwrap_or_else(Utc::now);
        let remote_ts = remote_updated.timestamp() as u64;

        if remote_ts > local_updated {
          self.download_extension(ext_id, app_handle).await?;
        } else if local_updated > remote_ts {
          self.upload_extension(&ext).await?;
        }
      }
      (Some(ext), false) => {
        self.upload_extension(&ext).await?;
      }
      (None, true) => {
        self.download_extension(ext_id, app_handle).await?;
      }
      (None, false) => {
        log::debug!("Extension {} not found locally or remotely", ext_id);
      }
    }

    Ok(())
  }

  async fn upload_extension(&self, ext: &crate::extension_manager::Extension) -> SyncResult<()> {
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs();

    let mut updated_ext = ext.clone();
    updated_ext.last_sync = Some(now);

    let json = serde_json::to_string_pretty(&updated_ext)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize extension: {e}")))?;

    let remote_key = format!("extensions/{}.json", ext.id);
    let presign = self
      .client
      .presign_upload(&remote_key, Some("application/json"))
      .await?;
    self
      .client
      .upload_bytes(&presign.url, json.as_bytes(), Some("application/json"))
      .await?;

    // Also upload the extension file data
    let file_path = {
      let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      let file_dir = manager.get_file_dir_public(&ext.id);
      file_dir.join(&ext.file_name)
    };

    if file_path.exists() {
      let file_data = fs::read(&file_path).map_err(|e| {
        SyncError::IoError(format!(
          "Failed to read extension file {}: {e}",
          file_path.display()
        ))
      })?;

      let file_remote_key = format!("extensions/{}/file/{}", ext.id, ext.file_name);
      let file_presign = self
        .client
        .presign_upload(&file_remote_key, Some("application/octet-stream"))
        .await?;
      self
        .client
        .upload_bytes(
          &file_presign.url,
          &file_data,
          Some("application/octet-stream"),
        )
        .await?;
    }

    // Update local extension with new last_sync
    {
      let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      if let Err(e) = manager.update_extension_internal(&updated_ext) {
        log::warn!("Failed to update extension last_sync: {}", e);
      }
    }

    log::info!("Extension {} uploaded", ext.id);
    Ok(())
  }

  async fn download_extension(
    &self,
    ext_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let remote_key = format!("extensions/{}.json", ext_id);
    let presign = self.client.presign_download(&remote_key).await?;
    let data = self.client.download_bytes(&presign.url).await?;

    let mut ext: crate::extension_manager::Extension = serde_json::from_slice(&data)
      .map_err(|e| SyncError::SerializationError(format!("Failed to parse extension JSON: {e}")))?;

    ext.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );
    ext.sync_enabled = true;

    // Download the extension file
    let file_remote_key = format!("extensions/{}/file/{}", ext.id, ext.file_name);
    let file_stat = self.client.stat(&file_remote_key).await?;
    if file_stat.exists {
      let file_presign = self.client.presign_download(&file_remote_key).await?;
      let file_data = self.client.download_bytes(&file_presign.url).await?;

      let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      let file_dir = manager.get_file_dir_public(&ext.id);
      drop(manager);

      fs::create_dir_all(&file_dir).map_err(|e| {
        SyncError::IoError(format!(
          "Failed to create extension file dir {}: {e}",
          file_dir.display()
        ))
      })?;
      let file_path = file_dir.join(&ext.file_name);
      fs::write(&file_path, &file_data).map_err(|e| {
        SyncError::IoError(format!(
          "Failed to write extension file {}: {e}",
          file_path.display()
        ))
      })?;
    }

    // Save or update local extension
    {
      let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      if let Err(e) = manager.upsert_extension_internal(&ext) {
        log::warn!("Failed to save downloaded extension: {}", e);
      }
    }

    if let Some(_handle) = app_handle {
      let _ = events::emit("extensions-changed", ());
    }

    log::info!("Extension {} downloaded", ext_id);
    Ok(())
  }

  pub async fn sync_extension_by_id_with_handle(
    &self,
    ext_id: &str,
    app_handle: &tauri::AppHandle,
  ) -> SyncResult<()> {
    self.sync_extension(ext_id, Some(app_handle)).await
  }

  pub async fn delete_extension(&self, ext_id: &str) -> SyncResult<()> {
    let remote_key = format!("extensions/{}.json", ext_id);
    let file_prefix = format!("extensions/{}/file/", ext_id);
    let tombstone_key = format!("tombstones/extensions/{}.json", ext_id);

    // Delete metadata
    self
      .client
      .delete(&remote_key, Some(&tombstone_key))
      .await?;

    // Delete file data
    let _ = self.client.delete_prefix(&file_prefix, None).await;

    log::info!("Extension {} deleted from sync", ext_id);
    Ok(())
  }

  // Extension group sync

  async fn sync_extension_group(
    &self,
    group_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let local_group = {
      let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      manager.get_group(group_id).ok()
    };

    let remote_key = format!("extension_groups/{}.json", group_id);
    let stat = self.client.stat(&remote_key).await?;

    match (local_group, stat.exists) {
      (Some(group), true) => {
        let local_updated = group.last_sync.unwrap_or(0);
        let remote_updated: DateTime<Utc> = stat
          .last_modified
          .as_ref()
          .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
          .map(|dt| dt.with_timezone(&Utc))
          .unwrap_or_else(Utc::now);
        let remote_ts = remote_updated.timestamp() as u64;

        if remote_ts > local_updated {
          self.download_extension_group(group_id, app_handle).await?;
        } else if local_updated > remote_ts {
          self.upload_extension_group(&group).await?;
        }
      }
      (Some(group), false) => {
        self.upload_extension_group(&group).await?;
      }
      (None, true) => {
        self.download_extension_group(group_id, app_handle).await?;
      }
      (None, false) => {
        log::debug!("Extension group {} not found locally or remotely", group_id);
      }
    }

    Ok(())
  }

  async fn upload_extension_group(
    &self,
    group: &crate::extension_manager::ExtensionGroup,
  ) -> SyncResult<()> {
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs();

    let mut updated_group = group.clone();
    updated_group.last_sync = Some(now);

    let json = serde_json::to_string_pretty(&updated_group).map_err(|e| {
      SyncError::SerializationError(format!("Failed to serialize extension group: {e}"))
    })?;

    let remote_key = format!("extension_groups/{}.json", group.id);
    let presign = self
      .client
      .presign_upload(&remote_key, Some("application/json"))
      .await?;
    self
      .client
      .upload_bytes(&presign.url, json.as_bytes(), Some("application/json"))
      .await?;

    // Update local group with new last_sync
    {
      let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      if let Err(e) = manager.update_group_internal(&updated_group) {
        log::warn!("Failed to update extension group last_sync: {}", e);
      }
    }

    log::info!("Extension group {} uploaded", group.id);
    Ok(())
  }

  async fn download_extension_group(
    &self,
    group_id: &str,
    app_handle: Option<&tauri::AppHandle>,
  ) -> SyncResult<()> {
    let remote_key = format!("extension_groups/{}.json", group_id);
    let presign = self.client.presign_download(&remote_key).await?;
    let data = self.client.download_bytes(&presign.url).await?;

    let mut group: crate::extension_manager::ExtensionGroup = serde_json::from_slice(&data)
      .map_err(|e| {
        SyncError::SerializationError(format!("Failed to parse extension group JSON: {e}"))
      })?;

    group.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );
    group.sync_enabled = true;

    // Save or update local group
    {
      let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      if let Err(e) = manager.upsert_group_internal(&group) {
        log::warn!("Failed to save downloaded extension group: {}", e);
      }
    }

    if let Some(_handle) = app_handle {
      let _ = events::emit("extensions-changed", ());
    }

    log::info!("Extension group {} downloaded", group_id);
    Ok(())
  }

  pub async fn sync_extension_group_by_id_with_handle(
    &self,
    group_id: &str,
    app_handle: &tauri::AppHandle,
  ) -> SyncResult<()> {
    self.sync_extension_group(group_id, Some(app_handle)).await
  }

  pub async fn delete_extension_group(&self, group_id: &str) -> SyncResult<()> {
    let remote_key = format!("extension_groups/{}.json", group_id);
    let tombstone_key = format!("tombstones/extension_groups/{}.json", group_id);

    self
      .client
      .delete(&remote_key, Some(&tombstone_key))
      .await?;

    log::info!("Extension group {} deleted from sync", group_id);
    Ok(())
  }

  /// Download a profile from S3 if it exists remotely but not locally
  pub async fn download_profile_if_missing(
    &self,
    app_handle: &tauri::AppHandle,
    profile_id: &str,
    key_prefix: &str,
  ) -> SyncResult<bool> {
    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profile_dir = profiles_dir.join(profile_id);

    // Check if profile exists locally
    let profile_uuid = uuid::Uuid::parse_str(profile_id)
      .map_err(|_| SyncError::InvalidData(format!("Invalid profile ID format: {}", profile_id)))?;

    let profiles = profile_manager
      .list_profiles()
      .map_err(|e| SyncError::IoError(format!("Failed to list profiles: {e}")))?;

    let exists_locally = profiles.iter().any(|p| p.id == profile_uuid);

    if exists_locally {
      log::debug!("Profile {} exists locally, skipping download", profile_id);
      return Ok(false);
    }

    // Check if profile exists remotely
    let manifest_key = format!("{}profiles/{}/manifest.json", key_prefix, profile_id);
    let stat = self.client.stat(&manifest_key).await?;

    if !stat.exists {
      log::debug!("Profile {} does not exist remotely, skipping", profile_id);
      return Ok(false);
    }

    log::info!(
      "Profile {} exists remotely but not locally, downloading...",
      profile_id
    );

    // Download metadata.json first to get profile info
    let metadata_key = format!("{}profiles/{}/metadata.json", key_prefix, profile_id);
    let metadata_stat = self.client.stat(&metadata_key).await?;

    if !metadata_stat.exists {
      log::warn!(
        "Profile {} manifest exists but metadata.json missing, skipping",
        profile_id
      );
      return Ok(false);
    }

    let metadata_presign = self.client.presign_download(&metadata_key).await?;
    let metadata_data = self.client.download_bytes(&metadata_presign.url).await?;
    let mut profile: BrowserProfile = serde_json::from_slice(&metadata_data)
      .map_err(|e| SyncError::SerializationError(format!("Failed to parse metadata: {e}")))?;

    // Cross-OS profile: save metadata only, skip manifest + file downloads
    if profile.is_cross_os() {
      log::info!(
        "Profile {} is cross-OS (host_os={:?}), downloading metadata only",
        profile_id,
        profile.host_os
      );

      fs::create_dir_all(&profile_dir).map_err(|e| {
        SyncError::IoError(format!(
          "Failed to create profile directory {}: {e}",
          profile_dir.display()
        ))
      })?;

      if profile.sync_mode == SyncMode::Disabled {
        profile.sync_mode = SyncMode::Regular;
      }
      profile.last_sync = Some(
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap()
          .as_secs(),
      );

      profile_manager
        .save_profile(&profile)
        .map_err(|e| SyncError::IoError(format!("Failed to save cross-OS profile: {e}")))?;

      let _ = events::emit("profiles-changed", ());
      let _ = events::emit(
        "profile-sync-status",
        serde_json::json!({
          "profile_id": profile_id,
          "profile_name": profile.name,
          "status": "synced"
        }),
      );

      log::info!(
        "Cross-OS profile {} metadata downloaded successfully",
        profile_id
      );
      return Ok(true);
    }

    // Derive encryption key before downloading manifest if profile uses encrypted sync.
    // The manifest itself may be encrypted (new behavior) or plaintext (backwards compat).
    let encryption_key = if profile.is_encrypted_sync() {
      let password = encryption::load_e2e_password()
        .map_err(|e| SyncError::InvalidData(format!("Failed to load E2E password: {e}")))?
        .ok_or_else(|| {
          let _ = events::emit("profile-sync-e2e-password-required", ());
          SyncError::InvalidData(
            "Remote profile is encrypted but no E2E password is set".to_string(),
          )
        })?;
      let salt = profile.encryption_salt.as_deref().ok_or_else(|| {
        SyncError::InvalidData("Encryption salt missing on encrypted profile".to_string())
      })?;
      let key = encryption::derive_profile_key(&password, salt)
        .map_err(|e| SyncError::InvalidData(format!("Key derivation failed: {e}")))?;
      Some(key)
    } else {
      None
    };

    // Download manifest (may be encrypted for e2e profiles)
    let manifest = self
      .download_manifest(&manifest_key, encryption_key.as_ref())
      .await?;
    let Some(manifest) = manifest else {
      return Err(SyncError::InvalidData(
        "Remote manifest not found".to_string(),
      ));
    };

    // Ensure profile directory exists
    fs::create_dir_all(&profile_dir).map_err(|e| {
      SyncError::IoError(format!(
        "Failed to create profile directory {}: {e}",
        profile_dir.display()
      ))
    })?;

    // Download all files from manifest
    let total_size: u64 = manifest.files.iter().map(|f| f.size).sum();
    log::info!(
      "Profile {} recovery: downloading {} files ({} bytes total)",
      profile_id,
      manifest.files.len(),
      total_size
    );
    for file in &manifest.files {
      log::info!(
        "  -> {} ({} bytes, hash: {})",
        file.path,
        file.size,
        file.hash
      );
    }
    if !manifest.files.is_empty() {
      self
        .download_profile_files(
          app_handle,
          profile_id,
          &profile.name,
          &profile_dir,
          &manifest.files,
          encryption_key.as_ref(),
          key_prefix,
        )
        .await?;
    }

    // Verify critical files after download
    let os_crypt_key_path = profile_dir.join("profile").join("os_crypt_key");
    let cookies_path = profile_dir.join("profile").join("Default").join("Cookies");
    if os_crypt_key_path.exists() {
      let key_data = fs::read(&os_crypt_key_path).unwrap_or_default();
      log::info!(
        "Profile {} sync: os_crypt_key present ({} bytes, sha256: {:x})",
        profile_id,
        key_data.len(),
        {
          use std::hash::{Hash, Hasher};
          let mut h = std::collections::hash_map::DefaultHasher::new();
          key_data.hash(&mut h);
          h.finish()
        }
      );
    } else {
      log::warn!(
        "Profile {} sync: os_crypt_key NOT FOUND after download",
        profile_id
      );
    }
    if cookies_path.exists() {
      let cookies_meta = fs::metadata(&cookies_path).unwrap_or_else(|_| fs::metadata(".").unwrap());
      log::info!(
        "Profile {} sync: Cookies present ({} bytes)",
        profile_id,
        cookies_meta.len()
      );
    } else {
      log::warn!(
        "Profile {} sync: Cookies NOT FOUND after download",
        profile_id
      );
    }

    // Set sync mode and save profile
    if profile.sync_mode == SyncMode::Disabled {
      profile.sync_mode = if manifest.encrypted {
        SyncMode::Encrypted
      } else {
        SyncMode::Regular
      };
    }
    profile.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );

    profile_manager
      .save_profile(&profile)
      .map_err(|e| SyncError::IoError(format!("Failed to save downloaded profile: {e}")))?;

    let _ = events::emit("profiles-changed", ());
    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "profile_name": profile.name,
        "status": "synced"
      }),
    );

    log::info!("Profile {} downloaded successfully", profile_id);
    Ok(true)
  }

  /// Check for profiles that exist remotely but not locally and download them
  pub async fn check_for_missing_synced_profiles(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> SyncResult<Vec<String>> {
    log::info!("Checking for missing synced profiles...");

    // List all personal profiles from S3 (paginated)
    let all_objects = self.client.list_all("profiles/").await?;

    let mut downloaded: Vec<String> = Vec::new();

    // Extract unique profile IDs with their key prefix
    let mut profiles_to_check: HashMap<String, String> = HashMap::new();
    for obj in all_objects {
      if obj.key.starts_with("profiles/") && obj.key.ends_with("/manifest.json") {
        if let Some(profile_id) = obj
          .key
          .strip_prefix("profiles/")
          .and_then(|s| s.strip_suffix("/manifest.json"))
        {
          profiles_to_check.insert(profile_id.to_string(), String::new());
        }
      }
    }

    // Also list team profiles if user is on a team
    if let Some(auth) = crate::cloud_auth::CLOUD_AUTH.get_user().await {
      if let Some(team_id) = &auth.user.team_id {
        let team_prefix = format!("teams/{}/", team_id);
        let team_list_key = format!("{}profiles/", team_prefix);
        if let Ok(team_objects) = self.client.list_all(&team_list_key).await {
          for obj in team_objects {
            if obj.key.starts_with("profiles/") && obj.key.ends_with("/manifest.json") {
              if let Some(profile_id) = obj
                .key
                .strip_prefix("profiles/")
                .and_then(|s| s.strip_suffix("/manifest.json"))
              {
                profiles_to_check.insert(profile_id.to_string(), team_prefix.clone());
              }
            }
          }
        }
      }
    }

    log::info!(
      "Found {} profiles in remote storage, checking for missing ones...",
      profiles_to_check.len()
    );

    // For each remote profile, check if it exists locally and download if missing
    for (profile_id, key_prefix) in &profiles_to_check {
      match self
        .download_profile_if_missing(app_handle, profile_id, key_prefix)
        .await
      {
        Ok(true) => {
          downloaded.push(profile_id.clone());
        }
        Ok(false) => {
          // Profile exists locally or doesn't exist remotely, skip
        }
        Err(e) => {
          log::warn!("Failed to check/download profile {}: {}", profile_id, e);
        }
      }
    }

    if !downloaded.is_empty() {
      log::info!(
        "Downloaded {} missing profiles: {:?}",
        downloaded.len(),
        downloaded
      );
    } else {
      log::info!("No missing profiles found");
    }

    // Delete local synced profiles that have a remote tombstone (deleted on another device)
    {
      let profile_manager = ProfileManager::instance();
      let local_synced: Vec<(String, Option<String>)> = profile_manager
        .list_profiles()
        .unwrap_or_default()
        .iter()
        .filter(|p| p.is_sync_enabled())
        .map(|p| (p.id.to_string(), p.created_by_id.clone()))
        .collect();

      let team_prefix = if let Some(auth) = crate::cloud_auth::CLOUD_AUTH.get_user().await {
        auth.user.team_id.map(|tid| format!("teams/{}/", tid))
      } else {
        None
      };

      for (pid, created_by_id) in &local_synced {
        // Check personal tombstone
        let personal_tombstone = format!("tombstones/profiles/{}.json", pid);
        let has_personal_tombstone = matches!(
          self.client.stat(&personal_tombstone).await,
          Ok(stat) if stat.exists
        );

        // Check team tombstone
        let has_team_tombstone = if let (Some(tp), Some(_)) = (&team_prefix, created_by_id) {
          let team_tombstone = format!("{}tombstones/profiles/{}.json", tp, pid);
          matches!(
            self.client.stat(&team_tombstone).await,
            Ok(stat) if stat.exists
          )
        } else {
          false
        };

        if has_personal_tombstone || has_team_tombstone {
          log::info!(
            "Profile {} has remote tombstone, deleting locally (deleted on another device)",
            pid
          );
          if let Err(e) = profile_manager.delete_profile_local_only(pid) {
            log::warn!("Failed to delete tombstoned profile {}: {}", pid, e);
          }
        }
      }
    }

    // Refresh metadata for local cross-OS profiles (propagate renames, tags, notes from originating device)
    let profile_manager = ProfileManager::instance();
    // Collect cross-OS profiles before async operations to avoid holding non-Send Result across await
    let cross_os_profiles: Vec<(String, SyncMode, Option<String>)> = profile_manager
      .list_profiles()
      .unwrap_or_default()
      .iter()
      .filter(|p| p.is_cross_os() && p.is_sync_enabled())
      .map(|p| (p.id.to_string(), p.sync_mode, p.created_by_id.clone()))
      .collect();

    if !cross_os_profiles.is_empty() {
      let team_prefix = if let Some(auth) = crate::cloud_auth::CLOUD_AUTH.get_user().await {
        auth.user.team_id.map(|tid| format!("teams/{}/", tid))
      } else {
        None
      };

      for (pid, sync_mode, created_by_id) in &cross_os_profiles {
        let kp = if created_by_id.is_some() {
          team_prefix.as_deref().unwrap_or("")
        } else {
          ""
        };
        let metadata_key = format!("{}profiles/{}/metadata.json", kp, pid);
        match self.client.stat(&metadata_key).await {
          Ok(stat) if stat.exists => match self.client.presign_download(&metadata_key).await {
            Ok(presign) => match self.client.download_bytes(&presign.url).await {
              Ok(data) => {
                if let Ok(mut remote_profile) = serde_json::from_slice::<BrowserProfile>(&data) {
                  remote_profile.sync_mode = *sync_mode;
                  remote_profile.last_sync = Some(
                    std::time::SystemTime::now()
                      .duration_since(std::time::UNIX_EPOCH)
                      .unwrap()
                      .as_secs(),
                  );
                  if let Err(e) = profile_manager.save_profile(&remote_profile) {
                    log::warn!("Failed to refresh cross-OS profile {} metadata: {}", pid, e);
                  } else {
                    log::debug!("Refreshed cross-OS profile {} metadata", pid);
                  }
                }
              }
              Err(e) => {
                log::warn!(
                  "Failed to download cross-OS profile {} metadata: {}",
                  pid,
                  e
                );
              }
            },
            Err(e) => {
              log::warn!("Failed to presign cross-OS profile {} metadata: {}", pid, e);
            }
          },
          _ => {}
        }
      }
      let _ = events::emit("profiles-changed", ());
    }

    Ok(downloaded)
  }

  /// Check for remote entities (proxies, groups, VPNs) not present locally and download them
  pub async fn check_for_missing_synced_entities(
    &self,
    app_handle: &tauri::AppHandle,
  ) -> SyncResult<()> {
    log::info!("Checking for missing synced entities...");

    // Check for remote proxies not present locally
    let remote_proxies = self.client.list("proxies/").await?;
    for obj in &remote_proxies.objects {
      if let Some(proxy_id) = obj
        .key
        .strip_prefix("proxies/")
        .and_then(|s| s.strip_suffix(".json"))
      {
        let exists_locally = crate::proxy_manager::PROXY_MANAGER
          .get_stored_proxies()
          .iter()
          .any(|p| p.id == proxy_id);
        if !exists_locally {
          let tombstone_key = format!("tombstones/proxies/{}.json", proxy_id);
          if let Ok(stat) = self.client.stat(&tombstone_key).await {
            if stat.exists {
              continue;
            }
          }
          log::info!(
            "Proxy {} exists remotely but not locally, downloading...",
            proxy_id
          );
          if let Err(e) = self.download_proxy(proxy_id, Some(app_handle)).await {
            log::warn!("Failed to download missing proxy {}: {}", proxy_id, e);
          }
        }
      }
    }

    // Check for remote groups not present locally
    let remote_groups = self.client.list("groups/").await?;
    for obj in &remote_groups.objects {
      if let Some(group_id) = obj
        .key
        .strip_prefix("groups/")
        .and_then(|s| s.strip_suffix(".json"))
      {
        let exists_locally = {
          let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
          group_manager
            .get_all_groups()
            .unwrap_or_default()
            .iter()
            .any(|g| g.id == group_id)
        };
        if !exists_locally {
          let tombstone_key = format!("tombstones/groups/{}.json", group_id);
          if let Ok(stat) = self.client.stat(&tombstone_key).await {
            if stat.exists {
              continue;
            }
          }
          log::info!(
            "Group {} exists remotely but not locally, downloading...",
            group_id
          );
          if let Err(e) = self.download_group(group_id, Some(app_handle)).await {
            log::warn!("Failed to download missing group {}: {}", group_id, e);
          }
        }
      }
    }

    // Check for remote VPNs not present locally
    let remote_vpns = self.client.list("vpns/").await?;
    for obj in &remote_vpns.objects {
      if let Some(vpn_id) = obj
        .key
        .strip_prefix("vpns/")
        .and_then(|s| s.strip_suffix(".json"))
      {
        let exists_locally = {
          let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
          storage.load_config(vpn_id).is_ok()
        };
        if !exists_locally {
          let tombstone_key = format!("tombstones/vpns/{}.json", vpn_id);
          if let Ok(stat) = self.client.stat(&tombstone_key).await {
            if stat.exists {
              continue;
            }
          }
          log::info!(
            "VPN {} exists remotely but not locally, downloading...",
            vpn_id
          );
          if let Err(e) = self.download_vpn(vpn_id, Some(app_handle)).await {
            log::warn!("Failed to download missing VPN {}: {}", vpn_id, e);
          }
        }
      }
    }

    log::info!("Missing synced entities check complete");
    Ok(())
  }
}

/// Check if proxy is used by any synced profile
pub fn is_proxy_used_by_synced_profile(proxy_id: &str) -> bool {
  let profile_manager = ProfileManager::instance();
  if let Ok(profiles) = profile_manager.list_profiles() {
    profiles
      .iter()
      .any(|p| p.is_sync_enabled() && p.proxy_id.as_deref() == Some(proxy_id))
  } else {
    false
  }
}

/// Check if group is used by any synced profile
pub fn is_group_used_by_synced_profile(group_id: &str) -> bool {
  let profile_manager = ProfileManager::instance();
  if let Ok(profiles) = profile_manager.list_profiles() {
    profiles
      .iter()
      .any(|p| p.is_sync_enabled() && p.group_id.as_deref() == Some(group_id))
  } else {
    false
  }
}

/// Enable sync for proxy if not already enabled
pub async fn enable_proxy_sync_if_needed(
  proxy_id: &str,
  _app_handle: &tauri::AppHandle,
) -> Result<(), String> {
  let proxy_manager = &crate::proxy_manager::PROXY_MANAGER;
  let proxies = proxy_manager.get_stored_proxies();
  let proxy = proxies
    .iter()
    .find(|p| p.id == proxy_id)
    .ok_or_else(|| format!("Proxy with ID '{proxy_id}' not found"))?;

  if !proxy.sync_enabled {
    let mut updated_proxy = proxy.clone();
    updated_proxy.sync_enabled = true;

    let proxy_file = proxy_manager.get_proxy_file_path(&proxy.id);
    let json = serde_json::to_string_pretty(&updated_proxy)
      .map_err(|e| format!("Failed to serialize proxy: {e}"))?;
    std::fs::write(&proxy_file, &json)
      .map_err(|e| format!("Failed to update proxy file {}: {e}", proxy_file.display()))?;

    let _ = events::emit("stored-proxies-changed", ());
    log::info!("Auto-enabled sync for proxy {}", proxy_id);
  }

  Ok(())
}

/// Check if VPN is used by any synced profile
pub fn is_vpn_used_by_synced_profile(vpn_id: &str) -> bool {
  let profile_manager = ProfileManager::instance();
  if let Ok(profiles) = profile_manager.list_profiles() {
    profiles
      .iter()
      .any(|p| p.is_sync_enabled() && p.vpn_id.as_deref() == Some(vpn_id))
  } else {
    false
  }
}

/// Enable sync for VPN if not already enabled
pub async fn enable_vpn_sync_if_needed(
  vpn_id: &str,
  _app_handle: &tauri::AppHandle,
) -> Result<(), String> {
  let vpn = {
    let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
    storage
      .load_config(vpn_id)
      .map_err(|e| format!("VPN with ID '{vpn_id}' not found: {e}"))?
  };

  if !vpn.sync_enabled {
    let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
    storage
      .update_sync_fields(vpn_id, true, None)
      .map_err(|e| format!("Failed to enable VPN sync: {e}"))?;

    let _ = events::emit("vpn-configs-changed", ());
    log::info!("Auto-enabled sync for VPN {}", vpn_id);
  }

  Ok(())
}

/// Enable sync for group if not already enabled
pub async fn enable_group_sync_if_needed(
  group_id: &str,
  _app_handle: &tauri::AppHandle,
) -> Result<(), String> {
  let group = {
    let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
    let groups = group_manager.get_all_groups().unwrap_or_default();
    groups
      .iter()
      .find(|g| g.id == group_id)
      .ok_or_else(|| format!("Group with ID '{group_id}' not found"))?
      .clone()
  };

  if !group.sync_enabled {
    let mut updated_group = group.clone();
    updated_group.sync_enabled = true;

    {
      let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
      if let Err(e) = group_manager.update_group_internal(&updated_group) {
        return Err(format!("Failed to update group: {e}"));
      }
    }

    let _ = events::emit("groups-changed", ());
    log::info!("Auto-enabled sync for group {}", group_id);
  }

  Ok(())
}

#[tauri::command]
pub async fn set_profile_sync_mode(
  app_handle: tauri::AppHandle,
  profile_id: String,
  sync_mode: String,
) -> Result<(), String> {
  let new_mode = match sync_mode.as_str() {
    "Disabled" => SyncMode::Disabled,
    "Regular" => SyncMode::Regular,
    "Encrypted" => SyncMode::Encrypted,
    _ => return Err(format!("Invalid sync mode: {sync_mode}")),
  };

  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?;

  let profile_uuid =
    uuid::Uuid::parse_str(&profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
  let mut profile = profiles
    .into_iter()
    .find(|p| p.id == profile_uuid)
    .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

  if profile.is_cross_os() {
    return Err("Cannot modify sync settings for a cross-OS profile".to_string());
  }

  if profile.ephemeral {
    return Err("Cannot enable sync for an ephemeral profile".to_string());
  }

  let old_mode = profile.sync_mode;
  let enabling = new_mode != SyncMode::Disabled;

  if enabling {
    let cloud_logged_in = crate::cloud_auth::CLOUD_AUTH.is_logged_in().await;

    if !cloud_logged_in {
      let manager = SettingsManager::instance();
      let settings = manager
        .load_settings()
        .map_err(|e| format!("Failed to load settings: {e}"))?;

      if settings.sync_server_url.is_none() {
        let _ = events::emit(
          "profile-sync-status",
          serde_json::json!({
            "profile_id": profile_id,
            "profile_name": profile.name,
            "status": "error",
            "error": "Sync server not configured. Please configure sync settings first."
          }),
        );
        return Err(
          "Sync server not configured. Please configure sync settings first.".to_string(),
        );
      }

      let token = manager.get_sync_token(&app_handle).await.ok().flatten();
      if token.is_none() {
        let _ = events::emit(
          "profile-sync-status",
          serde_json::json!({
            "profile_id": profile_id,
            "profile_name": profile.name,
            "status": "error",
            "error": "Sync token not configured. Please configure sync settings first."
          }),
        );
        return Err("Sync token not configured. Please configure sync settings first.".to_string());
      }
    }
  }

  // If switching to Encrypted, verify eligibility, password, and generate salt
  if new_mode == SyncMode::Encrypted {
    // Only pro users and team owners can enable encryption
    if let Some(state) = crate::cloud_auth::CLOUD_AUTH.get_user().await {
      if state.user.plan == "team" && state.user.team_role.as_deref() != Some("owner") {
        return Err("Profile encryption is available for Pro users and team owners.".to_string());
      }
    }

    if !encryption::has_e2e_password() {
      return Err("E2E password not set. Please set a password in Settings first.".to_string());
    }
    if profile.encryption_salt.is_none() {
      profile.encryption_salt = Some(encryption::generate_salt());
    }
  }

  // If switching between Regular<->Encrypted, delete remote manifest to force full re-upload
  let mode_switched = old_mode != SyncMode::Disabled && enabling && old_mode != new_mode;
  if mode_switched {
    if let Ok(engine) = SyncEngine::create_from_settings(&app_handle).await {
      let key_prefix = SyncEngine::get_team_key_prefix(&profile).await;
      let manifest_key = format!("{}profiles/{}/manifest.json", key_prefix, profile_id);
      let _ = engine.client.delete(&manifest_key, None).await;
      log::info!(
        "Deleted remote manifest for profile {} due to sync mode change ({:?} -> {:?})",
        profile_id,
        old_mode,
        new_mode
      );
    }
  }

  profile.sync_mode = new_mode;

  profile_manager
    .save_profile(&profile)
    .map_err(|e| format!("Failed to save profile: {e}"))?;

  let _ = events::emit("profiles-changed", ());

  if enabling {
    let is_running = profile.process_id.is_some();

    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "profile_name": profile.name,
        "status": if is_running { "waiting" } else { "syncing" }
      }),
    );

    if let Some(scheduler) = super::get_global_scheduler() {
      scheduler
        .queue_profile_sync_immediate(profile_id.clone())
        .await;

      if let Some(ref proxy_id) = profile.proxy_id {
        if let Err(e) = enable_proxy_sync_if_needed(proxy_id, &app_handle).await {
          log::warn!("Failed to enable sync for proxy {}: {}", proxy_id, e);
        } else {
          scheduler.queue_proxy_sync(proxy_id.clone()).await;
        }
      }
      if let Some(ref group_id) = profile.group_id {
        if let Err(e) = enable_group_sync_if_needed(group_id, &app_handle).await {
          log::warn!("Failed to enable sync for group {}: {}", group_id, e);
        } else {
          scheduler.queue_group_sync(group_id.clone()).await;
        }
      }
      if let Some(ref vpn_id) = profile.vpn_id {
        if let Err(e) = enable_vpn_sync_if_needed(vpn_id, &app_handle).await {
          log::warn!("Failed to enable sync for VPN {}: {}", vpn_id, e);
        } else {
          scheduler.queue_vpn_sync(vpn_id.clone()).await;
        }
      }
    } else {
      log::warn!("Scheduler not initialized, sync will not start");
    }
  } else {
    // Delete remote data when disabling sync
    if old_mode != SyncMode::Disabled {
      let profile_id_clone = profile_id.clone();
      let app_handle_clone = app_handle.clone();
      tokio::spawn(async move {
        match SyncEngine::create_from_settings(&app_handle_clone).await {
          Ok(engine) => {
            if let Err(e) = engine.delete_profile(&profile_id_clone).await {
              log::warn!(
                "Failed to delete profile {} from sync: {}",
                profile_id_clone,
                e
              );
            } else {
              log::info!("Profile {} deleted from sync service", profile_id_clone);
            }
          }
          Err(e) => {
            log::debug!("Sync not configured, skipping remote deletion: {}", e);
          }
        }
      });
    }

    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "profile_name": profile.name,
        "status": "disabled"
      }),
    );
  }

  if crate::cloud_auth::CLOUD_AUTH.is_logged_in().await {
    let sync_count = profile_manager
      .list_profiles()
      .map(|profiles| profiles.iter().filter(|p| p.is_sync_enabled()).count())
      .unwrap_or(0);

    tokio::spawn(async move {
      if let Err(e) = crate::cloud_auth::CLOUD_AUTH
        .report_sync_profile_count(sync_count as i64)
        .await
      {
        log::warn!("Failed to report sync profile count: {e}");
      }
    });
  }

  Ok(())
}

#[tauri::command]
pub async fn request_profile_sync(
  _app_handle: tauri::AppHandle,
  profile_id: String,
) -> Result<(), String> {
  // Validate profile exists and sync is enabled
  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?;

  let profile_uuid =
    uuid::Uuid::parse_str(&profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
  let profile = profiles
    .into_iter()
    .find(|p| p.id == profile_uuid)
    .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

  if !profile.is_sync_enabled() {
    return Err("Sync is not enabled for this profile".to_string());
  }

  // Queue sync via scheduler
  if let Some(scheduler) = super::get_global_scheduler() {
    let is_running = profile.process_id.is_some();
    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "profile_name": profile.name,
        "status": if is_running { "waiting" } else { "syncing" }
      }),
    );

    scheduler.queue_profile_sync_immediate(profile_id).await;
    Ok(())
  } else {
    Err("Sync scheduler not initialized".to_string())
  }
}

#[tauri::command]
pub async fn sync_profile(app_handle: tauri::AppHandle, profile_id: String) -> Result<(), String> {
  trigger_sync_for_profile(app_handle, profile_id).await
}

pub async fn trigger_sync_for_profile(
  app_handle: tauri::AppHandle,
  profile_id: String,
) -> Result<(), String> {
  let engine = SyncEngine::create_from_settings(&app_handle)
    .await
    .map_err(|e| format!("Failed to create sync engine: {e}"))?;

  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?;

  let profile_uuid =
    uuid::Uuid::parse_str(&profile_id).map_err(|_| format!("Invalid profile ID: {profile_id}"))?;
  let profile = profiles
    .into_iter()
    .find(|p| p.id == profile_uuid)
    .ok_or_else(|| format!("Profile with ID '{profile_id}' not found"))?;

  engine
    .sync_profile(&app_handle, &profile)
    .await
    .map_err(|e| format!("Sync failed: {e}"))?;

  Ok(())
}

#[tauri::command]
pub async fn set_proxy_sync_enabled(
  app_handle: tauri::AppHandle,
  proxy_id: String,
  enabled: bool,
) -> Result<(), String> {
  let proxy_manager = &crate::proxy_manager::PROXY_MANAGER;
  let proxies = proxy_manager.get_stored_proxies();
  let proxy = proxies
    .iter()
    .find(|p| p.id == proxy_id)
    .ok_or_else(|| format!("Proxy with ID '{proxy_id}' not found"))?;

  // Block modifying sync for cloud-managed proxies
  if proxy.is_cloud_managed {
    return Err("Cannot modify sync for a cloud-managed proxy".to_string());
  }

  // If disabling, check if proxy is used by any synced profile
  if !enabled && is_proxy_used_by_synced_profile(&proxy_id) {
    return Err("Sync cannot be disabled while this proxy is used by synced profiles".to_string());
  }

  // If enabling, check that sync settings are configured
  if enabled {
    let cloud_logged_in = crate::cloud_auth::CLOUD_AUTH.is_logged_in().await;

    if !cloud_logged_in {
      let manager = SettingsManager::instance();
      let settings = manager
        .load_settings()
        .map_err(|e| format!("Failed to load settings: {e}"))?;

      if settings.sync_server_url.is_none() {
        return Err(
          "Sync server not configured. Please configure sync settings first.".to_string(),
        );
      }

      let token = manager.get_sync_token(&app_handle).await.ok().flatten();
      if token.is_none() {
        return Err("Sync token not configured. Please configure sync settings first.".to_string());
      }
    }
  }

  let mut updated_proxy = proxy.clone();
  updated_proxy.sync_enabled = enabled;

  if !enabled {
    updated_proxy.last_sync = None;
  }

  let proxy_file = proxy_manager.get_proxy_file_path(&proxy.id);
  let json = serde_json::to_string_pretty(&updated_proxy)
    .map_err(|e| format!("Failed to serialize proxy: {e}"))?;
  std::fs::write(&proxy_file, &json)
    .map_err(|e| format!("Failed to update proxy file {}: {e}", proxy_file.display()))?;

  let _ = events::emit("stored-proxies-changed", ());

  if enabled {
    let _ = events::emit(
      "proxy-sync-status",
      serde_json::json!({
        "id": proxy_id,
        "status": "syncing"
      }),
    );

    if let Some(scheduler) = super::get_global_scheduler() {
      scheduler.queue_proxy_sync(proxy_id).await;
    }
  } else {
    let _ = events::emit(
      "proxy-sync-status",
      serde_json::json!({
        "id": proxy_id,
        "status": "disabled"
      }),
    );
  }

  Ok(())
}

#[tauri::command]
pub async fn set_group_sync_enabled(
  app_handle: tauri::AppHandle,
  group_id: String,
  enabled: bool,
) -> Result<(), String> {
  let group = {
    let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
    let groups = group_manager.get_all_groups().unwrap_or_default();
    groups
      .iter()
      .find(|g| g.id == group_id)
      .ok_or_else(|| format!("Group with ID '{group_id}' not found"))?
      .clone()
  };

  // If disabling, check if group is used by any synced profile
  if !enabled && is_group_used_by_synced_profile(&group_id) {
    return Err("Sync cannot be disabled while this group is used by synced profiles".to_string());
  }

  // If enabling, check that sync settings are configured
  if enabled {
    let cloud_logged_in = crate::cloud_auth::CLOUD_AUTH.is_logged_in().await;

    if !cloud_logged_in {
      let manager = SettingsManager::instance();
      let settings = manager
        .load_settings()
        .map_err(|e| format!("Failed to load settings: {e}"))?;

      if settings.sync_server_url.is_none() {
        return Err(
          "Sync server not configured. Please configure sync settings first.".to_string(),
        );
      }

      let token = manager.get_sync_token(&app_handle).await.ok().flatten();
      if token.is_none() {
        return Err("Sync token not configured. Please configure sync settings first.".to_string());
      }
    }
  }

  let mut updated_group = group.clone();
  updated_group.sync_enabled = enabled;

  if !enabled {
    updated_group.last_sync = None;
  }

  {
    let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
    if let Err(e) = group_manager.update_group_internal(&updated_group) {
      return Err(format!("Failed to update group: {e}"));
    }
  }

  let _ = events::emit("groups-changed", ());

  if enabled {
    let _ = events::emit(
      "group-sync-status",
      serde_json::json!({
        "id": group_id,
        "status": "syncing"
      }),
    );

    if let Some(scheduler) = super::get_global_scheduler() {
      scheduler.queue_group_sync(group_id).await;
    }
  } else {
    let _ = events::emit(
      "group-sync-status",
      serde_json::json!({
        "id": group_id,
        "status": "disabled"
      }),
    );
  }

  Ok(())
}

#[tauri::command]
pub fn is_proxy_in_use_by_synced_profile(proxy_id: String) -> bool {
  is_proxy_used_by_synced_profile(&proxy_id)
}

#[tauri::command]
pub fn is_group_in_use_by_synced_profile(group_id: String) -> bool {
  is_group_used_by_synced_profile(&group_id)
}

#[tauri::command]
pub async fn set_vpn_sync_enabled(
  app_handle: tauri::AppHandle,
  vpn_id: String,
  enabled: bool,
) -> Result<(), String> {
  let vpn = {
    let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
    storage
      .load_config(&vpn_id)
      .map_err(|e| format!("VPN with ID '{vpn_id}' not found: {e}"))?
  };

  // If disabling, check if VPN is used by any synced profile
  if !enabled && is_vpn_used_by_synced_profile(&vpn_id) {
    return Err("Sync cannot be disabled while this VPN is used by synced profiles".to_string());
  }

  // If enabling, check that sync settings are configured
  if enabled {
    let cloud_logged_in = crate::cloud_auth::CLOUD_AUTH.is_logged_in().await;

    if !cloud_logged_in {
      let manager = SettingsManager::instance();
      let settings = manager
        .load_settings()
        .map_err(|e| format!("Failed to load settings: {e}"))?;

      if settings.sync_server_url.is_none() {
        return Err(
          "Sync server not configured. Please configure sync settings first.".to_string(),
        );
      }

      let token = manager.get_sync_token(&app_handle).await.ok().flatten();
      if token.is_none() {
        return Err("Sync token not configured. Please configure sync settings first.".to_string());
      }
    }
  }

  let last_sync = if enabled { vpn.last_sync } else { None };

  {
    let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
    storage
      .update_sync_fields(&vpn_id, enabled, last_sync)
      .map_err(|e| format!("Failed to update VPN sync: {e}"))?;
  }

  let _ = events::emit("vpn-configs-changed", ());

  if enabled {
    let _ = events::emit(
      "vpn-sync-status",
      serde_json::json!({
        "id": vpn_id,
        "status": "syncing"
      }),
    );

    if let Some(scheduler) = super::get_global_scheduler() {
      scheduler.queue_vpn_sync(vpn_id).await;
    }
  } else {
    let _ = events::emit(
      "vpn-sync-status",
      serde_json::json!({
        "id": vpn_id,
        "status": "disabled"
      }),
    );
  }

  Ok(())
}

#[tauri::command]
pub fn is_vpn_in_use_by_synced_profile(vpn_id: String) -> bool {
  is_vpn_used_by_synced_profile(&vpn_id)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UnsyncedEntityCounts {
  pub proxies: usize,
  pub groups: usize,
  pub vpns: usize,
  pub extensions: usize,
  pub extension_groups: usize,
}

#[tauri::command]
pub fn get_unsynced_entity_counts() -> Result<UnsyncedEntityCounts, String> {
  let proxy_count = {
    let proxies = crate::proxy_manager::PROXY_MANAGER.get_stored_proxies();
    proxies
      .iter()
      .filter(|p| !p.sync_enabled && !p.is_cloud_managed)
      .count()
  };

  let group_count = {
    let gm = crate::group_manager::GROUP_MANAGER.lock().unwrap();
    let groups = gm
      .get_all_groups()
      .map_err(|e| format!("Failed to get groups: {e}"))?;
    groups.iter().filter(|g| !g.sync_enabled).count()
  };

  let vpn_count = {
    let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
    let configs = storage
      .list_configs()
      .map_err(|e| format!("Failed to list VPN configs: {e}"))?;
    configs.iter().filter(|c| !c.sync_enabled).count()
  };

  let extension_count = {
    let em = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let exts = em
      .list_extensions()
      .map_err(|e| format!("Failed to list extensions: {e}"))?;
    exts.iter().filter(|e| !e.sync_enabled).count()
  };

  let extension_group_count = {
    let em = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let groups = em
      .list_groups()
      .map_err(|e| format!("Failed to list extension groups: {e}"))?;
    groups.iter().filter(|g| !g.sync_enabled).count()
  };

  Ok(UnsyncedEntityCounts {
    proxies: proxy_count,
    groups: group_count,
    vpns: vpn_count,
    extensions: extension_count,
    extension_groups: extension_group_count,
  })
}

#[tauri::command]
pub async fn enable_sync_for_all_entities(app_handle: tauri::AppHandle) -> Result<(), String> {
  // Enable sync for all unsynced proxies
  {
    let proxies = crate::proxy_manager::PROXY_MANAGER.get_stored_proxies();
    for proxy in &proxies {
      if !proxy.sync_enabled && !proxy.is_cloud_managed {
        if let Err(e) = set_proxy_sync_enabled(app_handle.clone(), proxy.id.clone(), true).await {
          log::warn!("Failed to enable sync for proxy {}: {e}", proxy.id);
        }
      }
    }
  }

  // Enable sync for all unsynced groups
  {
    let groups = {
      let gm = crate::group_manager::GROUP_MANAGER.lock().unwrap();
      gm.get_all_groups()
        .map_err(|e| format!("Failed to get groups: {e}"))?
    };
    for group in &groups {
      if !group.sync_enabled {
        if let Err(e) = set_group_sync_enabled(app_handle.clone(), group.id.clone(), true).await {
          log::warn!("Failed to enable sync for group {}: {e}", group.id);
        }
      }
    }
  }

  // Enable sync for all unsynced VPNs
  {
    let configs = {
      let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
      storage
        .list_configs()
        .map_err(|e| format!("Failed to list VPN configs: {e}"))?
    };
    for config in &configs {
      if !config.sync_enabled {
        if let Err(e) = set_vpn_sync_enabled(app_handle.clone(), config.id.clone(), true).await {
          log::warn!("Failed to enable sync for VPN {}: {e}", config.id);
        }
      }
    }
  }

  // Enable sync for all unsynced extensions
  {
    let exts = {
      let em = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      em.list_extensions()
        .map_err(|e| format!("Failed to list extensions: {e}"))?
    };
    for ext in &exts {
      if !ext.sync_enabled {
        if let Err(e) = set_extension_sync_enabled(app_handle.clone(), ext.id.clone(), true).await {
          log::warn!("Failed to enable sync for extension {}: {e}", ext.id);
        }
      }
    }
  }

  // Enable sync for all unsynced extension groups
  {
    let groups = {
      let em = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      em.list_groups()
        .map_err(|e| format!("Failed to list extension groups: {e}"))?
    };
    for group in &groups {
      if !group.sync_enabled {
        if let Err(e) =
          set_extension_group_sync_enabled(app_handle.clone(), group.id.clone(), true).await
        {
          log::warn!(
            "Failed to enable sync for extension group {}: {e}",
            group.id
          );
        }
      }
    }
  }

  Ok(())
}

#[tauri::command]
pub async fn set_extension_sync_enabled(
  app_handle: tauri::AppHandle,
  extension_id: String,
  enabled: bool,
) -> Result<(), String> {
  let ext = {
    let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    manager
      .get_extension(&extension_id)
      .map_err(|e| format!("Extension with ID '{extension_id}' not found: {e}"))?
  };

  if enabled {
    let cloud_logged_in = crate::cloud_auth::CLOUD_AUTH.is_logged_in().await;
    if !cloud_logged_in {
      let manager = SettingsManager::instance();
      let settings = manager
        .load_settings()
        .map_err(|e| format!("Failed to load settings: {e}"))?;
      if settings.sync_server_url.is_none() {
        return Err(
          "Sync server not configured. Please configure sync settings first.".to_string(),
        );
      }
      let token = manager.get_sync_token(&app_handle).await.ok().flatten();
      if token.is_none() {
        return Err("Sync token not configured. Please configure sync settings first.".to_string());
      }
    }
  }

  let mut updated_ext = ext;
  updated_ext.sync_enabled = enabled;
  if !enabled {
    updated_ext.last_sync = None;
  }

  {
    let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    manager
      .update_extension_internal(&updated_ext)
      .map_err(|e| format!("Failed to update extension sync: {e}"))?;
  }

  let _ = events::emit("extensions-changed", ());

  if enabled {
    if let Some(scheduler) = super::get_global_scheduler() {
      scheduler.queue_extension_sync(extension_id).await;
    }
  }

  Ok(())
}

#[tauri::command]
pub async fn set_extension_group_sync_enabled(
  app_handle: tauri::AppHandle,
  extension_group_id: String,
  enabled: bool,
) -> Result<(), String> {
  let group = {
    let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    manager
      .get_group(&extension_group_id)
      .map_err(|e| format!("Extension group with ID '{extension_group_id}' not found: {e}"))?
  };

  if enabled {
    let cloud_logged_in = crate::cloud_auth::CLOUD_AUTH.is_logged_in().await;
    if !cloud_logged_in {
      let manager = SettingsManager::instance();
      let settings = manager
        .load_settings()
        .map_err(|e| format!("Failed to load settings: {e}"))?;
      if settings.sync_server_url.is_none() {
        return Err(
          "Sync server not configured. Please configure sync settings first.".to_string(),
        );
      }
      let token = manager.get_sync_token(&app_handle).await.ok().flatten();
      if token.is_none() {
        return Err("Sync token not configured. Please configure sync settings first.".to_string());
      }
    }
  }

  let mut updated_group = group;
  updated_group.sync_enabled = enabled;
  if !enabled {
    updated_group.last_sync = None;
  }

  {
    let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    manager
      .update_group_internal(&updated_group)
      .map_err(|e| format!("Failed to update extension group sync: {e}"))?;
  }

  let _ = events::emit("extensions-changed", ());

  if enabled {
    if let Some(scheduler) = super::get_global_scheduler() {
      scheduler
        .queue_extension_group_sync(extension_group_id)
        .await;
    }
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_checkpoint_sqlite_wal_files() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create a SQLite database in WAL mode and insert data.
    // Use std::mem::forget to prevent the connection destructor from running,
    // which simulates a browser crash where WAL is not checkpointed.
    {
      let conn = rusqlite::Connection::open(&db_path).unwrap();
      conn.pragma_update(None, "journal_mode", "WAL").unwrap();
      conn.pragma_update(None, "wal_autocheckpoint", "0").unwrap();
      conn
        .execute(
          "CREATE TABLE cookies (id INTEGER PRIMARY KEY, value TEXT)",
          [],
        )
        .unwrap();
      conn
        .execute(
          "INSERT INTO cookies (value) VALUES ('session_token_123')",
          [],
        )
        .unwrap();
      // Leak the connection to prevent auto-checkpoint on drop
      std::mem::forget(conn);
    }

    // Verify WAL file exists and has data
    let wal_path = temp_dir.path().join("test.db-wal");
    assert!(wal_path.exists(), "WAL file should exist");
    let wal_size = fs::metadata(&wal_path).unwrap().len();
    assert!(wal_size > 0, "WAL file should be non-empty");

    // Run checkpoint
    checkpoint_sqlite_wal_files(temp_dir.path());

    // After checkpoint, WAL should be truncated (empty)
    let wal_size_after = fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
    assert_eq!(
      wal_size_after, 0,
      "WAL should be truncated after checkpoint"
    );

    // Verify data is still accessible from the main database
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let value: String = conn
      .query_row("SELECT value FROM cookies WHERE id = 1", [], |row| {
        row.get(0)
      })
      .unwrap();
    assert_eq!(value, "session_token_123");
  }

  #[test]
  fn test_checkpoint_handles_missing_db() {
    let temp_dir = tempfile::TempDir::new().unwrap();

    // Create a WAL file without a corresponding database
    let wal_path = temp_dir.path().join("missing.db-wal");
    fs::write(&wal_path, b"fake wal data").unwrap();

    // Should not panic
    checkpoint_sqlite_wal_files(temp_dir.path());
  }

  #[test]
  fn test_checkpoint_skips_empty_wal() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create a database and checkpoint immediately (WAL is empty)
    {
      let conn = rusqlite::Connection::open(&db_path).unwrap();
      conn.pragma_update(None, "journal_mode", "WAL").unwrap();
      conn
        .execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", [])
        .unwrap();
    }

    // Create an empty WAL file
    let wal_path = temp_dir.path().join("test.db-wal");
    fs::write(&wal_path, b"").unwrap();

    // Should skip empty WAL without error
    checkpoint_sqlite_wal_files(temp_dir.path());
  }

  #[test]
  fn test_checkpoint_nested_directories() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let nested_dir = temp_dir.path().join("profile").join("Default");
    fs::create_dir_all(&nested_dir).unwrap();

    let db_path = nested_dir.join("Cookies");

    // Create a database with WAL data, leak connection to simulate crash
    {
      let conn = rusqlite::Connection::open(&db_path).unwrap();
      conn.pragma_update(None, "journal_mode", "WAL").unwrap();
      conn.pragma_update(None, "wal_autocheckpoint", "0").unwrap();
      conn
        .execute(
          "CREATE TABLE cookies (host_key TEXT, name TEXT, value TEXT)",
          [],
        )
        .unwrap();
      conn
        .execute(
          "INSERT INTO cookies VALUES ('.example.com', 'session', 'abc')",
          [],
        )
        .unwrap();
      std::mem::forget(conn);
    }

    let wal_path = nested_dir.join("Cookies-wal");
    assert!(wal_path.exists());

    // Checkpoint from the top-level directory
    checkpoint_sqlite_wal_files(temp_dir.path());

    // Verify data is in the main database
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
      .query_row("SELECT COUNT(*) FROM cookies", [], |row| row.get(0))
      .unwrap();
    assert_eq!(count, 1);
  }
}

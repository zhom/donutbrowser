use super::client::SyncClient;
use super::manifest::{compute_diff, generate_manifest, get_cache_path, HashCache, SyncManifest};
use super::types::*;
use crate::events;
use crate::profile::types::BrowserProfile;
use crate::profile::ProfileManager;
use crate::settings_manager::SettingsManager;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Semaphore;

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

  pub async fn sync_profile(
    &self,
    app_handle: &tauri::AppHandle,
    profile: &BrowserProfile,
  ) -> SyncResult<()> {
    if profile.is_cross_os() {
      log::info!(
        "Skipping file sync for cross-OS profile: {} ({})",
        profile.name,
        profile.id
      );
      return Ok(());
    }

    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profile_dir = profiles_dir.join(profile.id.to_string());
    let profile_id = profile.id.to_string();

    log::info!(
      "Starting delta sync for profile: {} ({})",
      profile.name,
      profile_id
    );

    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
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
    let remote_manifest_key = format!("profiles/{}/manifest.json", profile_id);
    let remote_manifest = self.download_manifest(&remote_manifest_key).await?;

    // Compute diff
    let diff = compute_diff(&local_manifest, remote_manifest.as_ref());

    if diff.is_empty() {
      log::info!("Profile {} is already in sync", profile_id);
      let _ = events::emit(
        "profile-sync-status",
        serde_json::json!({
          "profile_id": profile_id,
          "status": "synced"
        }),
      );
      return Ok(());
    }

    log::info!(
      "Profile {} diff: {} to upload, {} to download, {} to delete local, {} to delete remote",
      profile_id,
      diff.files_to_upload.len(),
      diff.files_to_download.len(),
      diff.files_to_delete_local.len(),
      diff.files_to_delete_remote.len()
    );

    // Perform uploads
    if !diff.files_to_upload.is_empty() {
      self
        .upload_profile_files(app_handle, &profile_id, &profile_dir, &diff.files_to_upload)
        .await?;
    }

    // Perform downloads
    if !diff.files_to_download.is_empty() {
      self
        .download_profile_files(
          app_handle,
          &profile_id,
          &profile_dir,
          &diff.files_to_download,
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
      let remote_key = format!("profiles/{}/files/{}", profile_id, path);
      let _ = self.client.delete(&remote_key, None).await;
      log::debug!("Deleted remote file: {}", path);
    }

    // Upload metadata.json (sanitized profile)
    self.upload_profile_metadata(&profile_id, profile).await?;

    // Upload manifest.json last for atomicity
    self.upload_manifest(&profile_id, &local_manifest).await?;

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

    // Update profile last_sync
    let mut updated_profile = profile.clone();
    updated_profile.last_sync = Some(
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    );
    let _ = profile_manager.save_profile(&updated_profile);
    let _ = events::emit("profiles-changed", ());

    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "status": "synced"
      }),
    );

    log::info!("Profile {} synced successfully", profile_id);
    Ok(())
  }

  async fn download_manifest(&self, key: &str) -> SyncResult<Option<SyncManifest>> {
    let stat = self.client.stat(key).await?;
    if !stat.exists {
      return Ok(None);
    }

    let presign = self.client.presign_download(key).await?;
    let data = self.client.download_bytes(&presign.url).await?;

    let manifest: SyncManifest = serde_json::from_slice(&data)
      .map_err(|e| SyncError::SerializationError(format!("Failed to parse manifest: {e}")))?;

    Ok(Some(manifest))
  }

  async fn upload_manifest(&self, profile_id: &str, manifest: &SyncManifest) -> SyncResult<()> {
    let json = serde_json::to_string_pretty(manifest)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize manifest: {e}")))?;

    let remote_key = format!("profiles/{}/manifest.json", profile_id);
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

  async fn upload_profile_metadata(
    &self,
    profile_id: &str,
    profile: &BrowserProfile,
  ) -> SyncResult<()> {
    let mut sanitized = profile.clone();
    sanitized.process_id = None;
    sanitized.last_launch = None;

    let json = serde_json::to_string_pretty(&sanitized)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize profile: {e}")))?;

    let remote_key = format!("profiles/{}/metadata.json", profile_id);
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

  async fn upload_profile_files(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    profile_dir: &Path,
    files: &[super::manifest::ManifestFileEntry],
  ) -> SyncResult<()> {
    if files.is_empty() {
      return Ok(());
    }

    log::info!("Uploading {} files for profile {}", files.len(), profile_id);

    // Get batch presigned URLs
    let items: Vec<(String, Option<String>)> = files
      .iter()
      .map(|f| {
        let key = format!("profiles/{}/files/{}", profile_id, f.path);
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

    // Upload with bounded concurrency
    let semaphore = Arc::new(Semaphore::new(8));
    let client = self.client.clone();
    let profile_dir = profile_dir.to_path_buf();
    let profile_id = profile_id.to_string();

    let mut handles = Vec::new();

    for file in files {
      let sem = semaphore.clone();
      let file_path = profile_dir.join(&file.path);
      let remote_key = format!("profiles/{}/files/{}", profile_id, file.path);
      let url = url_map.get(&remote_key).cloned();

      if url.is_none() {
        log::warn!("No presigned URL for {}", remote_key);
        continue;
      }

      let url = url.unwrap();
      let client = client.clone();
      let content_type = mime_guess::from_path(&file.path)
        .first()
        .map(|m| m.to_string());

      handles.push(tokio::spawn(async move {
        let _permit = sem.acquire().await.unwrap();

        let data = match fs::read(&file_path) {
          Ok(d) => d,
          Err(e) => {
            log::warn!("Failed to read {}: {}", file_path.display(), e);
            return;
          }
        };

        if let Err(e) = client
          .upload_bytes(&url, &data, content_type.as_deref())
          .await
        {
          log::warn!("Failed to upload {}: {}", file_path.display(), e);
        }
      }));
    }

    for handle in handles {
      let _ = handle.await;
    }

    let _ = events::emit(
      "profile-sync-progress",
      serde_json::json!({
        "profile_id": profile_id,
        "phase": "upload",
        "done": files.len(),
        "total": files.len()
      }),
    );

    Ok(())
  }

  async fn download_profile_files(
    &self,
    _app_handle: &tauri::AppHandle,
    profile_id: &str,
    profile_dir: &Path,
    files: &[super::manifest::ManifestFileEntry],
  ) -> SyncResult<()> {
    if files.is_empty() {
      return Ok(());
    }

    log::info!(
      "Downloading {} files for profile {}",
      files.len(),
      profile_id
    );

    // Get batch presigned URLs
    let keys: Vec<String> = files
      .iter()
      .map(|f| format!("profiles/{}/files/{}", profile_id, f.path))
      .collect();

    let batch_response = self.client.presign_download_batch(keys).await?;

    // Build URL map
    let url_map: HashMap<String, String> = batch_response
      .items
      .into_iter()
      .map(|item| (item.key, item.url))
      .collect();

    // Download with bounded concurrency
    let semaphore = Arc::new(Semaphore::new(8));
    let client = self.client.clone();
    let profile_dir = profile_dir.to_path_buf();
    let profile_id = profile_id.to_string();

    let mut handles = Vec::new();

    for file in files {
      let sem = semaphore.clone();
      let file_path = profile_dir.join(&file.path);
      let remote_key = format!("profiles/{}/files/{}", profile_id, file.path);
      let url = url_map.get(&remote_key).cloned();

      if url.is_none() {
        log::warn!("No presigned URL for {}", remote_key);
        continue;
      }

      let url = url.unwrap();
      let client = client.clone();

      handles.push(tokio::spawn(async move {
        let _permit = sem.acquire().await.unwrap();

        match client.download_bytes(&url).await {
          Ok(data) => {
            if let Some(parent) = file_path.parent() {
              let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::write(&file_path, &data) {
              log::warn!("Failed to write {}: {}", file_path.display(), e);
            }
          }
          Err(e) => {
            log::warn!("Failed to download {}: {}", remote_key, e);
          }
        }
      }));
    }

    for handle in handles {
      let _ = handle.await;
    }

    let _ = events::emit(
      "profile-sync-progress",
      serde_json::json!({
        "profile_id": profile_id,
        "phase": "download",
        "done": files.len(),
        "total": files.len()
      }),
    );

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

  /// Download a profile from S3 if it exists remotely but not locally
  pub async fn download_profile_if_missing(
    &self,
    app_handle: &tauri::AppHandle,
    profile_id: &str,
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
    let manifest_key = format!("profiles/{}/manifest.json", profile_id);
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
    let metadata_key = format!("profiles/{}/metadata.json", profile_id);
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

      profile.sync_enabled = true;
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
          "status": "synced"
        }),
      );

      log::info!(
        "Cross-OS profile {} metadata downloaded successfully",
        profile_id
      );
      return Ok(true);
    }

    // Download manifest
    let manifest = self.download_manifest(&manifest_key).await?;
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
        .download_profile_files(app_handle, profile_id, &profile_dir, &manifest.files)
        .await?;
    }

    // Set sync enabled and save profile
    profile.sync_enabled = true;
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

    // List all profiles from S3
    let list_response = self.client.list("profiles/").await?;

    let mut downloaded: Vec<String> = Vec::new();

    // Extract unique profile IDs from the list
    let mut profile_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for obj in list_response.objects {
      if obj.key.starts_with("profiles/") && obj.key.ends_with("/manifest.json") {
        if let Some(profile_id) = obj
          .key
          .strip_prefix("profiles/")
          .and_then(|s| s.strip_suffix("/manifest.json"))
        {
          profile_ids.insert(profile_id.to_string());
        }
      }
    }

    log::info!(
      "Found {} profiles in remote storage, checking for missing ones...",
      profile_ids.len()
    );

    // For each remote profile, check if it exists locally and download if missing
    for profile_id in profile_ids {
      match self
        .download_profile_if_missing(app_handle, &profile_id)
        .await
      {
        Ok(true) => {
          downloaded.push(profile_id);
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

    // Refresh metadata for local cross-OS profiles (propagate renames, tags, notes from originating device)
    let profile_manager = ProfileManager::instance();
    // Collect cross-OS profiles before async operations to avoid holding non-Send Result across await
    let cross_os_profiles: Vec<(String, bool)> = profile_manager
      .list_profiles()
      .unwrap_or_default()
      .iter()
      .filter(|p| p.is_cross_os() && p.sync_enabled)
      .map(|p| (p.id.to_string(), p.sync_enabled))
      .collect();

    if !cross_os_profiles.is_empty() {
      for (pid, sync_enabled) in &cross_os_profiles {
        let metadata_key = format!("profiles/{}/metadata.json", pid);
        match self.client.stat(&metadata_key).await {
          Ok(stat) if stat.exists => match self.client.presign_download(&metadata_key).await {
            Ok(presign) => match self.client.download_bytes(&presign.url).await {
              Ok(data) => {
                if let Ok(mut remote_profile) = serde_json::from_slice::<BrowserProfile>(&data) {
                  remote_profile.sync_enabled = *sync_enabled;
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
}

/// Check if proxy is used by any synced profile
pub fn is_proxy_used_by_synced_profile(proxy_id: &str) -> bool {
  let profile_manager = ProfileManager::instance();
  if let Ok(profiles) = profile_manager.list_profiles() {
    profiles
      .iter()
      .any(|p| p.sync_enabled && p.proxy_id.as_deref() == Some(proxy_id))
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
      .any(|p| p.sync_enabled && p.group_id.as_deref() == Some(group_id))
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
      .any(|p| p.sync_enabled && p.vpn_id.as_deref() == Some(vpn_id))
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
pub async fn set_profile_sync_enabled(
  app_handle: tauri::AppHandle,
  profile_id: String,
  enabled: bool,
) -> Result<(), String> {
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

  // If enabling, first check that sync settings are configured
  if enabled {
    // Cloud auth provides sync settings dynamically — skip local checks
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
            "status": "error",
            "error": "Sync token not configured. Please configure sync settings first."
          }),
        );
        return Err("Sync token not configured. Please configure sync settings first.".to_string());
      }
    }
  }

  profile.sync_enabled = enabled;

  profile_manager
    .save_profile(&profile)
    .map_err(|e| format!("Failed to save profile: {e}"))?;

  let _ = events::emit("profiles-changed", ());

  if enabled {
    // Check if profile is running to determine status
    let is_running = profile.process_id.is_some();

    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "status": if is_running { "waiting" } else { "syncing" }
      }),
    );

    // Queue sync via scheduler (not direct sync)
    if let Some(scheduler) = super::get_global_scheduler() {
      scheduler
        .queue_profile_sync_immediate(profile_id.clone())
        .await;

      // Auto-enable sync for proxy and group if they exist
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
    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
        "status": "disabled"
      }),
    );
  }

  // Report updated sync-enabled profile count to the cloud backend
  if crate::cloud_auth::CLOUD_AUTH.is_logged_in().await {
    let sync_count = profile_manager
      .list_profiles()
      .map(|profiles| profiles.iter().filter(|p| p.sync_enabled).count())
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

  if !profile.sync_enabled {
    return Err("Sync is not enabled for this profile".to_string());
  }

  // Queue sync via scheduler
  if let Some(scheduler) = super::get_global_scheduler() {
    let is_running = profile.process_id.is_some();
    let _ = events::emit(
      "profile-sync-status",
      serde_json::json!({
        "profile_id": profile_id,
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

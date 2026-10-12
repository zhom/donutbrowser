//! Restore points: copies of a synced profile's files that the sync server
//! keeps beside the live ones. One is made a day after a sync that changed
//! something, and on demand; restoring one puts its files back and makes every
//! device take them.
//!
//! The copies are made inside the store (no byte travels through this
//! computer), and the server keeps the newest three of each kind.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::engine::SyncEngine;
use super::manifest::DiffBias;
use super::types::{SyncError, SyncResult};
use crate::profile::{BrowserProfile, ProfileManager};

/// How long after one automatic restore point the next may be made.
const AUTO_EVERY_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestorePoint {
  pub id: String,
  pub created_at: String,
  /// `auto` (daily) or `manual`.
  pub kind: String,
  pub files: u64,
  pub bytes: u64,
}

/// When each profile last got an automatic restore point, kept on this
/// computer only.
static LAST_AUTO: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn state_path() -> std::path::PathBuf {
  crate::app_dirs::data_dir().join("restore_points.json")
}

fn last_auto(profile_id: &str) -> u64 {
  let Ok(mut guard) = LAST_AUTO.lock() else {
    return 0;
  };
  let map = guard.get_or_insert_with(|| {
    std::fs::read_to_string(state_path())
      .ok()
      .and_then(|text| serde_json::from_str(&text).ok())
      .unwrap_or_default()
  });
  map.get(profile_id).copied().unwrap_or(0)
}

fn note_auto(profile_id: &str, at: u64) {
  let Ok(mut guard) = LAST_AUTO.lock() else {
    return;
  };
  let map = guard.get_or_insert_with(HashMap::new);
  map.insert(profile_id.to_string(), at);
  if let Ok(text) = serde_json::to_string(map) {
    let _ = std::fs::write(state_path(), text);
  }
}

/// Whether an automatic restore point is due for a profile last given one at
/// `last`, at `now`.
pub fn auto_due(last: u64, now: u64) -> bool {
  now.saturating_sub(last) >= AUTO_EVERY_SECS
}

fn code(code: &str) -> String {
  crate::backend_error(code)
}

/// A sync failure as the code the app shows: the server's own code when it
/// sent one, `RESTORE_POINTS_UNSUPPORTED` for a server too old to know the
/// routes.
fn failure(error: SyncError) -> String {
  let text = error.to_string();
  if let Some(start) = text.find("{\"") {
    if let Ok(body) = serde_json::from_str::<serde_json::Value>(&text[start..]) {
      if let Some(found) = body
        .get("code")
        .or_else(|| body.pointer("/message/code"))
        .and_then(serde_json::Value::as_str)
      {
        return code(found);
      }
    }
  }
  if text.contains("(404") {
    return code("RESTORE_POINTS_UNSUPPORTED");
  }
  crate::backend_error_with_detail("RESTORE_POINT_FAILED", text)
}

impl SyncEngine {
  async fn profile_root(profile: &BrowserProfile) -> String {
    format!(
      "{}profiles/{}/",
      Self::get_team_key_prefix(profile).await,
      profile.id
    )
  }

  pub async fn list_restore_points(
    &self,
    profile: &BrowserProfile,
  ) -> SyncResult<Vec<RestorePoint>> {
    let root = Self::profile_root(profile).await;
    self.client.snapshots_list(&root).await
  }

  pub async fn create_restore_point(
    &self,
    profile: &BrowserProfile,
    kind: &str,
  ) -> SyncResult<RestorePoint> {
    let root = Self::profile_root(profile).await;
    let point = self.client.snapshots_create(&root, kind).await?;
    log::info!(
      "Restore point created profile={} kind={kind} files={} bytes={}",
      profile.id,
      point.files,
      point.bytes
    );
    Ok(point)
  }

  pub async fn delete_restore_point(&self, profile: &BrowserProfile, id: &str) -> SyncResult<()> {
    let root = Self::profile_root(profile).await;
    self.client.snapshots_delete(&root, id).await
  }

  /// Put restore point `id` back: its files replace the live ones on the
  /// server, its manifest goes up stamped as the newest, and this computer
  /// takes the result as it would after a remote session.
  pub async fn restore_point(
    &self,
    app_handle: &tauri::AppHandle,
    profile: &BrowserProfile,
    id: &str,
  ) -> SyncResult<()> {
    let root = Self::profile_root(profile).await;
    let key_prefix = Self::get_team_key_prefix(profile).await;
    let encryption_key = Self::profile_encryption_key(profile)?;
    let snapshot_manifest = format!("snapshots/{root}{id}/manifest.json");
    let mut manifest = self
      .download_manifest(&snapshot_manifest, encryption_key.as_ref())
      .await?
      .ok_or_else(|| SyncError::InvalidData("restore point has no manifest".to_string()))?;
    self.client.snapshots_restore_files(&root, id).await?;
    manifest.updated_at = chrono::Utc::now().to_rfc3339();
    self
      .upload_manifest(
        &profile.id.to_string(),
        &manifest,
        encryption_key.as_ref(),
        &key_prefix,
      )
      .await?;
    self
      .sync_profile_with_bias(app_handle, profile, DiffBias::Restore)
      .await?;
    log::info!(
      "Restore point restored profile={} files={}",
      profile.id,
      manifest.files.len()
    );
    Ok(())
  }

  /// After a sync that changed something, the day's automatic restore point,
  /// in the background.
  pub(super) fn maybe_auto_restore_point(&self, profile: &BrowserProfile) {
    if self.read_only || !profile.is_sync_enabled() {
      return;
    }
    let profile_id = profile.id.to_string();
    let now = crate::proxy_manager::now_secs();
    if !auto_due(last_auto(&profile_id), now) {
      return;
    }
    note_auto(&profile_id, now);
    let engine = self.clone();
    let profile = profile.clone();
    tauri::async_runtime::spawn(async move {
      if let Err(e) = engine.create_restore_point(&profile, "auto").await {
        log::warn!(
          "Automatic restore point failed profile={} err=\"{e}\"",
          profile.id
        );
      }
    });
  }
}

/// The profile, and an engine for it, when restore points can serve it.
async fn engine_for(
  app_handle: &tauri::AppHandle,
  profile_id: &str,
) -> Result<(SyncEngine, BrowserProfile), String> {
  let profile = ProfileManager::instance()
    .list_profiles()
    .map_err(|e| crate::wrap_backend_error(e, "Failed to list profiles"))?
    .into_iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or_else(|| code("PROFILE_NOT_FOUND"))?;
  if !profile.is_sync_enabled() {
    return Err(code("RESTORE_POINTS_NEED_SYNC"));
  }
  let engine = SyncEngine::create_from_settings(app_handle)
    .await
    .map_err(|_| code("RESTORE_POINTS_NEED_SYNC"))?;
  Ok((engine, profile))
}

#[tauri::command]
pub async fn list_restore_points(
  app_handle: tauri::AppHandle,
  profile_id: String,
) -> Result<Vec<RestorePoint>, String> {
  let (engine, profile) = engine_for(&app_handle, &profile_id).await?;
  engine.list_restore_points(&profile).await.map_err(failure)
}

#[tauri::command]
pub async fn create_restore_point(
  app_handle: tauri::AppHandle,
  profile_id: String,
) -> Result<RestorePoint, String> {
  let (engine, profile) = engine_for(&app_handle, &profile_id).await?;
  engine
    .create_restore_point(&profile, "manual")
    .await
    .map_err(failure)
}

#[tauri::command]
pub async fn delete_restore_point(
  app_handle: tauri::AppHandle,
  profile_id: String,
  id: String,
) -> Result<(), String> {
  let (engine, profile) = engine_for(&app_handle, &profile_id).await?;
  engine
    .delete_restore_point(&profile, &id)
    .await
    .map_err(failure)?;
  log::info!("Restore point deleted profile={profile_id}");
  Ok(())
}

/// Restore `id` into a closed profile. The data it replaces is kept first, as
/// a manual restore point, so a restore can itself be undone.
#[tauri::command]
pub async fn restore_restore_point(
  app_handle: tauri::AppHandle,
  profile_id: String,
  id: String,
) -> Result<(), String> {
  let (engine, profile) = engine_for(&app_handle, &profile_id).await?;
  if crate::browser_runner::browser_is_running_for(&profile_id) {
    return Err(code("PROFILE_RUNNING"));
  }
  engine
    .create_restore_point(&profile, "manual")
    .await
    .map_err(failure)?;
  engine
    .restore_point(&app_handle, &profile, &id)
    .await
    .map_err(failure)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_automatic_point_is_made_once_a_day() {
    assert!(auto_due(0, AUTO_EVERY_SECS));
    assert!(!auto_due(1_000, 1_000 + AUTO_EVERY_SECS - 1));
    assert!(auto_due(1_000, 1_000 + AUTO_EVERY_SECS));
  }

  #[test]
  fn a_server_code_reaches_the_app_and_an_old_server_says_so() {
    let refused = SyncError::AuthError(
      "(400 Bad Request) {\"message\":{\"code\":\"SNAPSHOT_NOTHING_TO_KEEP\"},\"statusCode\":400}"
        .to_string(),
    );
    assert!(failure(refused).contains("SNAPSHOT_NOTHING_TO_KEEP"));
    let read_only =
      SyncError::AuthError("(403 Forbidden) {\"code\":\"TEAM_ROLE_READ_ONLY\"}".to_string());
    assert!(failure(read_only).contains("TEAM_ROLE_READ_ONLY"));
    let old = SyncError::AuthError("(404 Not Found) Cannot POST".to_string());
    assert!(failure(old).contains("RESTORE_POINTS_UNSUPPORTED"));
  }
}

use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::cloud_auth::{CloudAuthManager, CLOUD_API_URL, CLOUD_AUTH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileLockInfo {
  #[serde(rename = "profileId")]
  pub profile_id: String,
  #[serde(rename = "lockedBy")]
  pub locked_by: String,
  #[serde(rename = "lockedByEmail")]
  pub locked_by_email: String,
  #[serde(rename = "lockedAt")]
  pub locked_at: String,
  #[serde(rename = "expiresAt", default)]
  pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AcquireLockResponse {
  success: bool,
  #[serde(rename = "lockedBy")]
  locked_by: Option<String>,
  #[serde(rename = "lockedByEmail")]
  locked_by_email: Option<String>,
}

pub struct TeamLockManager {
  locks: RwLock<HashMap<String, ProfileLockInfo>>,
  heartbeat_handle: Mutex<Option<JoinHandle<()>>>,
  connected_team_id: Mutex<Option<String>>,
}

lazy_static! {
  pub static ref TEAM_LOCK: TeamLockManager = TeamLockManager::new();
}

impl TeamLockManager {
  fn new() -> Self {
    Self {
      locks: RwLock::new(HashMap::new()),
      heartbeat_handle: Mutex::new(None),
      connected_team_id: Mutex::new(None),
    }
  }

  pub async fn connect(&self, team_id: &str) {
    log::info!("Connecting team lock manager for team: {team_id}");

    {
      let mut tid = self.connected_team_id.lock().await;
      *tid = Some(team_id.to_string());
    }

    if let Err(e) = self.fetch_initial_locks(team_id).await {
      log::warn!("Failed to fetch initial locks: {e}");
    }

    self.start_heartbeat_loop().await;
  }

  pub async fn disconnect(&self) {
    log::info!("Disconnecting team lock manager");

    {
      let mut handle = self.heartbeat_handle.lock().await;
      if let Some(h) = handle.take() {
        h.abort();
      }
    }

    {
      let mut locks = self.locks.write().await;
      locks.clear();
    }

    {
      let mut tid = self.connected_team_id.lock().await;
      *tid = None;
    }
  }

  pub async fn acquire_lock(&self, profile_id: &str) -> Result<(), String> {
    let team_id = self.get_team_id().await?;
    let client = Client::new();

    let access_token =
      CloudAuthManager::load_access_token()?.ok_or_else(|| "Not logged in".to_string())?;

    let url = format!("{CLOUD_API_URL}/api/teams/{team_id}/locks");
    let response = client
      .post(&url)
      .header("Authorization", format!("Bearer {access_token}"))
      .json(&serde_json::json!({ "profileId": profile_id }))
      .send()
      .await
      .map_err(|e| format!("Failed to acquire lock: {e}"))?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(format!("Lock acquisition failed ({status}): {body}"));
    }

    let result: AcquireLockResponse = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse lock response: {e}"))?;

    if !result.success {
      let email = result
        .locked_by_email
        .unwrap_or_else(|| "another user".to_string());
      return Err(format!("Profile is in use by {email}"));
    }

    // Update local cache
    if let Some(user) = CLOUD_AUTH.get_user().await {
      let mut locks = self.locks.write().await;
      locks.insert(
        profile_id.to_string(),
        ProfileLockInfo {
          profile_id: profile_id.to_string(),
          locked_by: user.user.id.clone(),
          locked_by_email: user.user.email.clone(),
          locked_at: chrono::Utc::now().to_rfc3339(),
          expires_at: None,
        },
      );
    }

    let _ = crate::events::emit(
      "team-lock-acquired",
      serde_json::json!({ "profileId": profile_id }),
    );

    Ok(())
  }

  pub async fn release_lock(&self, profile_id: &str) -> Result<(), String> {
    let team_id = self.get_team_id().await?;
    let client = Client::new();

    let access_token =
      CloudAuthManager::load_access_token()?.ok_or_else(|| "Not logged in".to_string())?;

    let url = format!("{CLOUD_API_URL}/api/teams/{team_id}/locks/{profile_id}");
    let _ = client
      .delete(&url)
      .header("Authorization", format!("Bearer {access_token}"))
      .send()
      .await;

    {
      let mut locks = self.locks.write().await;
      locks.remove(profile_id);
    }

    let _ = crate::events::emit(
      "team-lock-released",
      serde_json::json!({ "profileId": profile_id }),
    );

    Ok(())
  }

  pub async fn get_locks(&self) -> Vec<ProfileLockInfo> {
    let locks = self.locks.read().await;
    locks.values().cloned().collect()
  }

  pub async fn get_lock_status(&self, profile_id: &str) -> Option<ProfileLockInfo> {
    let locks = self.locks.read().await;
    locks.get(profile_id).cloned()
  }

  pub async fn is_locked_by_another(&self, profile_id: &str) -> bool {
    let locks = self.locks.read().await;
    if let Some(lock) = locks.get(profile_id) {
      if let Some(user) = CLOUD_AUTH.get_user().await {
        return lock.locked_by != user.user.id;
      }
    }
    false
  }

  async fn fetch_initial_locks(&self, team_id: &str) -> Result<(), String> {
    let client = Client::new();
    let access_token =
      CloudAuthManager::load_access_token()?.ok_or_else(|| "Not logged in".to_string())?;

    let url = format!("{CLOUD_API_URL}/api/teams/{team_id}/locks");
    let response = client
      .get(&url)
      .header("Authorization", format!("Bearer {access_token}"))
      .send()
      .await
      .map_err(|e| format!("Failed to fetch locks: {e}"))?;

    if !response.status().is_success() {
      return Err("Failed to fetch locks".to_string());
    }

    let lock_list: Vec<ProfileLockInfo> = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse locks: {e}"))?;

    let mut locks = self.locks.write().await;
    locks.clear();
    for lock in lock_list {
      locks.insert(lock.profile_id.clone(), lock);
    }

    Ok(())
  }

  async fn start_heartbeat_loop(&self) {
    let mut handle = self.heartbeat_handle.lock().await;
    if let Some(h) = handle.take() {
      h.abort();
    }

    let h = tokio::spawn(async move {
      loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;

        let team_id = match TEAM_LOCK.get_team_id().await {
          Ok(id) => id,
          Err(_) => break,
        };

        let held_locks: Vec<String> = {
          let locks = TEAM_LOCK.locks.read().await;
          if let Some(user) = CLOUD_AUTH.get_user().await {
            locks
              .values()
              .filter(|l| l.locked_by == user.user.id)
              .map(|l| l.profile_id.clone())
              .collect()
          } else {
            vec![]
          }
        };

        for profile_id in held_locks {
          let client = Client::new();
          if let Ok(Some(token)) = CloudAuthManager::load_access_token() {
            let url = format!("{CLOUD_API_URL}/api/teams/{team_id}/locks/{profile_id}/heartbeat");
            let _ = client
              .post(&url)
              .header("Authorization", format!("Bearer {token}"))
              .send()
              .await;
          }
        }

        // Refresh lock state from server
        if let Err(e) = TEAM_LOCK.fetch_initial_locks(&team_id).await {
          log::debug!("Failed to refresh locks: {e}");
        }
      }
    });

    *handle = Some(h);
  }

  async fn get_team_id(&self) -> Result<String, String> {
    let tid = self.connected_team_id.lock().await;
    tid
      .clone()
      .ok_or_else(|| "Not connected to a team".to_string())
  }
}

/// Acquire team lock if profile is sync-enabled and user is on a team.
/// Returns Ok(()) if lock acquired or not applicable, Err with message if locked by another.
pub async fn acquire_team_lock_if_needed(
  profile: &crate::profile::BrowserProfile,
) -> Result<(), String> {
  if !profile.is_sync_enabled() {
    return Ok(());
  }
  if !CLOUD_AUTH.is_on_team_plan().await {
    return Ok(());
  }

  if TEAM_LOCK
    .is_locked_by_another(&profile.id.to_string())
    .await
  {
    if let Some(lock) = TEAM_LOCK.get_lock_status(&profile.id.to_string()).await {
      return Err(format!("Profile is in use by {}", lock.locked_by_email));
    }
    return Err("Profile is in use by another team member".to_string());
  }

  TEAM_LOCK.acquire_lock(&profile.id.to_string()).await
}

/// Release team lock if profile is sync-enabled and user is on a team.
/// Logs warnings on failure but does not return errors.
pub async fn release_team_lock_if_needed(profile: &crate::profile::BrowserProfile) {
  if !profile.is_sync_enabled() {
    return;
  }
  if !CLOUD_AUTH.is_on_team_plan().await {
    return;
  }

  if let Err(e) = TEAM_LOCK.release_lock(&profile.id.to_string()).await {
    log::warn!(
      "Failed to release team lock for profile {}: {e}",
      profile.id
    );
  }
}

// --- Tauri commands ---

#[tauri::command]
pub async fn get_team_locks() -> Result<Vec<ProfileLockInfo>, String> {
  Ok(TEAM_LOCK.get_locks().await)
}

#[tauri::command]
pub async fn get_team_lock_status(profile_id: String) -> Result<Option<ProfileLockInfo>, String> {
  Ok(TEAM_LOCK.get_lock_status(&profile_id).await)
}

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::cloud_auth::{AuthorizedError, CLOUD_API_URL, CLOUD_AUTH};

pub const LOCK_CHANGED_EVENT: &str = "profile-lock-changed";
pub const SHUTDOWN_RELEASE_TIMEOUT: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const RELEASE_RETRY_DELAYS: [Duration; 4] = [
  Duration::from_secs(1),
  Duration::from_secs(2),
  Duration::from_secs(4),
  Duration::from_secs(8),
];

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

#[derive(Debug, Deserialize)]
struct HeartbeatResponse {
  #[serde(default)]
  refreshed: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Renewal {
  Refreshed,
  Lost,
  Failed,
}

#[derive(Debug, PartialEq, Eq)]
enum ReleaseStep {
  Done,
  Retry,
  GiveUp,
}

fn renewal_outcome(status: u16, body: &str) -> Renewal {
  if !(200..300).contains(&status) {
    return Renewal::Failed;
  }
  match serde_json::from_str::<HeartbeatResponse>(body) {
    Ok(reply) if reply.refreshed => Renewal::Refreshed,
    Ok(_) => Renewal::Lost,
    Err(_) => Renewal::Failed,
  }
}

fn release_step(status: u16) -> ReleaseStep {
  match status {
    200..=299 | 404 => ReleaseStep::Done,
    408 | 500..=599 => ReleaseStep::Retry,
    _ => ReleaseStep::GiveUp,
  }
}

fn release_retry_delay(attempt: usize) -> Option<Duration> {
  RELEASE_RETRY_DELAYS.get(attempt).copied()
}

fn lock_signature(locks: &HashMap<String, ProfileLockInfo>) -> BTreeSet<(String, String)> {
  locks
    .values()
    .map(|lock| (lock.profile_id.clone(), lock.locked_by.clone()))
    .collect()
}

fn emit_lock_change(profile_id: Option<&str>, action: &str) {
  let _ = crate::events::emit(
    LOCK_CHANGED_EVENT,
    serde_json::json!({ "profileId": profile_id, "action": action }),
  );
}

fn lock_url(profile_id: &str) -> String {
  format!("{CLOUD_API_URL}/api/profile-locks/{profile_id}")
}

async fn release_attempt(profile_id: &str) -> Option<Duration> {
  let url = lock_url(profile_id);
  match CLOUD_AUTH
    .authorized_request(|client, token| client.delete(&url).bearer_auth(token))
    .await
  {
    Ok(response) => match release_step(response.status().as_u16()) {
      ReleaseStep::Done => None,
      ReleaseStep::Retry => Some(Duration::ZERO),
      ReleaseStep::GiveUp => {
        log::warn!(
          "Profile lock release refused profile={profile_id} status={}",
          response.status().as_u16()
        );
        None
      }
    },
    Err(AuthorizedError::SignedOut) => None,
    Err(AuthorizedError::RateLimited(wait)) => Some(wait),
    Err(AuthorizedError::Transport(e)) => {
      log::debug!("Profile lock release attempt failed profile={profile_id} err=\"{e}\"");
      Some(Duration::ZERO)
    }
  }
}

async fn release_with_retry(profile_id: String, mut pending: Option<Duration>) {
  let mut attempt = 0;
  while let Some(wait) = pending {
    let Some(delay) = release_retry_delay(attempt) else {
      log::warn!("Profile lock release gave up after retries profile={profile_id}");
      return;
    };
    attempt += 1;
    tokio::time::sleep(delay.max(wait)).await;
    if PROFILE_LOCK.holds(&profile_id).await {
      return;
    }
    pending = release_attempt(&profile_id).await;
  }
}

pub struct ProfileLockManager {
  locks: RwLock<HashMap<String, ProfileLockInfo>>,
  held: RwLock<HashSet<String>>,
  heartbeat_handle: Mutex<Option<JoinHandle<()>>>,
  connected: Mutex<bool>,
  paused_until: std::sync::Mutex<Option<Instant>>,
}

pub static PROFILE_LOCK: std::sync::LazyLock<ProfileLockManager> =
  std::sync::LazyLock::new(ProfileLockManager::new);

// Keep backward compatibility alias
pub use PROFILE_LOCK as TEAM_LOCK;

impl ProfileLockManager {
  fn new() -> Self {
    Self {
      locks: RwLock::new(HashMap::new()),
      held: RwLock::new(HashSet::new()),
      heartbeat_handle: Mutex::new(None),
      connected: Mutex::new(false),
      paused_until: std::sync::Mutex::new(None),
    }
  }

  pub async fn connect(&self) {
    {
      let mut c = self.connected.lock().await;
      *c = true;
    }

    match self.fetch_locks().await {
      Ok(_) => log::info!("Profile lock manager connected"),
      Err(e) => log::warn!("Profile lock manager connected, initial lock fetch failed err=\"{e}\""),
    }

    self.start_heartbeat_loop().await;
  }

  pub async fn disconnect(&self) {
    log::info!("Profile lock manager disconnected");

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

    self.held.write().await.clear();

    {
      let mut c = self.connected.lock().await;
      *c = false;
    }
  }

  pub async fn is_connected(&self) -> bool {
    *self.connected.lock().await
  }

  pub async fn holds(&self, profile_id: &str) -> bool {
    self.held.read().await.contains(profile_id)
  }

  async fn held_ids(&self) -> Vec<String> {
    self.held.read().await.iter().cloned().collect()
  }

  async fn forget(&self, profile_id: &str) {
    self.held.write().await.remove(profile_id);
    self.locks.write().await.remove(profile_id);
  }

  fn pause_for(&self, wait: Duration) {
    *self
      .paused_until
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now() + wait);
  }

  fn is_paused(&self) -> bool {
    self
      .paused_until
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
      .is_some_and(|until| Instant::now() < until)
  }

  fn note_failure(&self, error: &AuthorizedError) {
    if let AuthorizedError::RateLimited(wait) = error {
      self.pause_for(*wait);
    }
  }

  pub async fn acquire_lock(&self, profile_id: &str) -> Result<(), String> {
    let url = lock_url(profile_id);
    let response = CLOUD_AUTH
      .authorized_request(|client, token| client.post(&url).bearer_auth(token))
      .await
      .map_err(|e| {
        self.note_failure(&e);
        log::warn!("Profile lock not acquired profile={profile_id} err=\"{e}\"");
        crate::backend_error("PROFILE_LOCK_UNAVAILABLE")
      })?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      log::warn!(
        "Profile lock not acquired profile={profile_id} status={} err=\"{body}\"",
        status.as_u16()
      );
      return Err(crate::backend_error("PROFILE_LOCK_UNAVAILABLE"));
    }

    let result: AcquireLockResponse = response.json().await.map_err(|e| {
      log::warn!("Profile lock response unparsable profile={profile_id} err=\"{e}\"");
      crate::backend_error("PROFILE_LOCK_UNAVAILABLE")
    })?;

    if !result.success {
      return Err(lock_conflict_error(
        profile_id,
        result.locked_by.as_deref(),
        result.locked_by_email.as_deref(),
      ));
    }

    self.held.write().await.insert(profile_id.to_string());

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

    emit_lock_change(Some(profile_id), "acquired");

    Ok(())
  }

  pub async fn release_lock(&self, profile_id: &str) {
    self.forget(profile_id).await;
    emit_lock_change(Some(profile_id), "released");

    let pending = release_attempt(profile_id).await;
    if pending.is_some() {
      let profile_id = profile_id.to_string();
      tauri::async_runtime::spawn(release_with_retry(profile_id, pending));
    }
  }

  pub async fn release_all_held_within(&self, limit: Duration) {
    let held: Vec<String> = self.held.write().await.drain().collect();
    if held.is_empty() {
      return;
    }
    {
      let mut locks = self.locks.write().await;
      for profile_id in &held {
        locks.remove(profile_id);
      }
    }
    for profile_id in &held {
      emit_lock_change(Some(profile_id), "released");
    }

    let releases = held.into_iter().map(|profile_id| async move {
      let pending = release_attempt(&profile_id).await;
      release_with_retry(profile_id, pending).await;
    });
    if tokio::time::timeout(limit, futures_util::future::join_all(releases))
      .await
      .is_err()
    {
      log::warn!(
        "Profile lock release timed out limit_ms={}",
        limit.as_millis()
      );
    }
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

  pub(crate) async fn fetch_locks(&self) -> Result<(), String> {
    let url = format!("{CLOUD_API_URL}/api/profile-locks");
    let response = CLOUD_AUTH
      .authorized_request(|client, token| client.get(&url).bearer_auth(token))
      .await
      .map_err(|e| {
        self.note_failure(&e);
        format!("Failed to fetch locks: {e}")
      })?;

    if !response.status().is_success() {
      return Err("Failed to fetch locks".to_string());
    }

    let lock_list: Vec<ProfileLockInfo> = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse locks: {e}"))?;

    if self.replace_display_locks(lock_list).await {
      emit_lock_change(None, "refreshed");
    }

    Ok(())
  }

  async fn replace_display_locks(&self, lock_list: Vec<ProfileLockInfo>) -> bool {
    let mut locks = self.locks.write().await;
    let before = lock_signature(&locks);
    locks.clear();
    for lock in lock_list {
      locks.insert(lock.profile_id.clone(), lock);
    }
    lock_signature(&locks) != before
  }

  async fn renew(&self, profile_id: &str) -> Result<Renewal, AuthorizedError> {
    let url = format!("{}/heartbeat", lock_url(profile_id));
    let response = CLOUD_AUTH
      .authorized_request(|client, token| client.post(&url).bearer_auth(token))
      .await?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    Ok(renewal_outcome(status, &body))
  }

  async fn recover_lost_lock(&self, profile_id: &str) {
    if !self.holds(profile_id).await {
      return;
    }
    if crate::browser_runner::browser_is_running_for(profile_id) {
      log::warn!("Profile lock lost while running, taking it again profile={profile_id}");
      if let Err(e) = self.acquire_lock(profile_id).await {
        log::warn!("Profile lock not taken again profile={profile_id} err=\"{e}\"");
        self.forget(profile_id).await;
        emit_lock_change(Some(profile_id), "released");
      }
    } else {
      self.forget(profile_id).await;
      emit_lock_change(Some(profile_id), "released");
    }
  }

  async fn start_heartbeat_loop(&self) {
    let mut handle = self.heartbeat_handle.lock().await;
    if let Some(h) = handle.take() {
      h.abort();
    }

    let h = tokio::spawn(async move {
      loop {
        tokio::time::sleep(HEARTBEAT_INTERVAL).await;

        if !PROFILE_LOCK.is_connected().await {
          break;
        }

        if PROFILE_LOCK.is_paused() {
          continue;
        }

        // Send heartbeat for each held lock
        let held_locks = PROFILE_LOCK.held_ids().await;
        let mut signed_out = false;

        for profile_id in held_locks {
          match PROFILE_LOCK.renew(&profile_id).await {
            Ok(Renewal::Refreshed) => {}
            Ok(Renewal::Lost) => PROFILE_LOCK.recover_lost_lock(&profile_id).await,
            Ok(Renewal::Failed) => {
              log::debug!("Profile lock heartbeat not accepted profile={profile_id}");
            }
            Err(AuthorizedError::SignedOut) => {
              signed_out = true;
              break;
            }
            Err(e) => {
              PROFILE_LOCK.note_failure(&e);
              log::debug!("Profile lock heartbeat failed profile={profile_id} err=\"{e}\"");
              if PROFILE_LOCK.is_paused() {
                break;
              }
            }
          }
        }

        if signed_out || PROFILE_LOCK.is_paused() {
          continue;
        }

        // Refresh lock state from server
        if let Err(e) = PROFILE_LOCK.fetch_locks().await {
          log::debug!("Profile lock refresh failed err=\"{e}\"");
        }
      }
    });

    *handle = Some(h);
  }
}

/// Separator the cloud API puts between a user id and a non-desktop holder's
/// sub-identity. Must match the server's holder format exactly.
///
/// A remote VM session takes the lock under `<user id>:vm:<session id>` so it
/// contends with this desktop instead of silently sharing its lock. That makes
/// the holder string the one place a client can tell "a teammate has this open"
/// apart from "this is my own profile, running remotely" — two refusals that
/// need completely different words.
const VM_HOLDER_SEPARATOR: &str = ":vm:";

/// The `{"code":…}` for a lock this caller could not take.
fn lock_conflict_error(
  profile_id: &str,
  holder: Option<&str>,
  holder_email: Option<&str>,
) -> String {
  if holder.is_some_and(|id| id.contains(VM_HOLDER_SEPARATOR)) {
    // The user's own remote session. Saying "in use by you@example.com" here,
    // which is what the raw backend message did, reads as a bug.
    log::info!("Profile lock held by a remote session profile={profile_id}");
    return crate::backend_error("PROFILE_RUNNING_REMOTELY");
  }
  match holder_email {
    Some(email) if !email.is_empty() => serde_json::json!({
      "code": "PROFILE_LOCKED_BY_MEMBER",
      "params": { "email": email }
    })
    .to_string(),
    _ => crate::backend_error("PROFILE_LOCKED_ELSEWHERE"),
  }
}

/// Acquire profile lock if profile is sync-enabled and user has a paid subscription.
/// Returns whether a lock was actually taken, so a caller that unwinds a failed
/// launch releases only what it acquired. Releasing unconditionally would drop
/// a lock a REST handler up the stack still owns.
pub async fn acquire_team_lock_if_needed(
  profile: &crate::profile::BrowserProfile,
) -> Result<bool, String> {
  if !profile.is_sync_enabled() {
    return Ok(false);
  }
  if !CLOUD_AUTH.has_active_paid_subscription().await {
    return Ok(false);
  }

  // Ensure lock manager is connected
  if !PROFILE_LOCK.is_connected().await {
    PROFILE_LOCK.connect().await;
  }

  if PROFILE_LOCK
    .is_locked_by_another(&profile.id.to_string())
    .await
  {
    let held = PROFILE_LOCK.get_lock_status(&profile.id.to_string()).await;
    return Err(lock_conflict_error(
      &profile.id.to_string(),
      held.as_ref().map(|lock| lock.locked_by.as_str()),
      held.as_ref().map(|lock| lock.locked_by_email.as_str()),
    ));
  }

  PROFILE_LOCK
    .acquire_lock(&profile.id.to_string())
    .await
    .map(|()| true)
}

/// Release profile lock if profile is sync-enabled and user has a paid subscription.
pub async fn release_team_lock_if_needed(profile: &crate::profile::BrowserProfile) {
  let profile_id = profile.id.to_string();
  if !PROFILE_LOCK.holds(&profile_id).await {
    return;
  }

  PROFILE_LOCK.release_lock(&profile_id).await;
}

// --- Tauri commands ---

#[tauri::command]
pub async fn get_team_locks() -> Result<Vec<ProfileLockInfo>, String> {
  Ok(PROFILE_LOCK.get_locks().await)
}

#[tauri::command]
pub async fn get_team_lock_status(profile_id: String) -> Result<Option<ProfileLockInfo>, String> {
  Ok(PROFILE_LOCK.get_lock_status(&profile_id).await)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_users_own_remote_session_is_not_reported_as_a_teammate() {
    // The holder for a remote session is `<user id>:vm:<session id>` and it
    // carries the OWNER's email, so the previous message read "Profile is in use
    // by you@example.com" — the user's own address, about their own profile.
    let err = lock_conflict_error(
      "p1",
      Some("11111111-2222-3333-4444-555555555555:vm:run-remote:p1:abc"),
      Some("owner@example.com"),
    );
    assert_eq!(err, r#"{"code":"PROFILE_RUNNING_REMOTELY"}"#);
    assert!(!err.contains("owner@example.com"));
  }

  #[test]
  fn a_teammates_lock_names_them_through_a_translatable_code() {
    let err = lock_conflict_error("p1", Some("other-user-id"), Some("mate@example.com"));
    let json: serde_json::Value = serde_json::from_str(&err).expect("a code envelope");
    assert_eq!(json["code"], "PROFILE_LOCKED_BY_MEMBER");
    assert_eq!(json["params"]["email"], "mate@example.com");
  }

  fn server_lock(profile_id: &str, holder: &str) -> ProfileLockInfo {
    ProfileLockInfo {
      profile_id: profile_id.to_string(),
      locked_by: holder.to_string(),
      locked_by_email: "owner@example.com".to_string(),
      locked_at: "2026-09-27T00:00:00Z".to_string(),
      expires_at: None,
    }
  }

  #[tokio::test]
  async fn listing_the_server_locks_never_makes_them_renewal_targets() {
    let manager = ProfileLockManager::new();
    manager.held.write().await.insert("taken-here".to_string());

    let changed = manager
      .replace_display_locks(vec![
        server_lock("left-by-a-crash", "user-1"),
        server_lock("taken-here", "user-1"),
      ])
      .await;
    assert!(changed);
    assert_eq!(manager.held_ids().await, vec!["taken-here".to_string()]);
    assert_eq!(manager.get_locks().await.len(), 2);

    manager.replace_display_locks(vec![]).await;
    assert!(manager.get_locks().await.is_empty());
    assert!(manager.holds("taken-here").await);
    assert!(!manager.holds("left-by-a-crash").await);
  }

  #[tokio::test]
  async fn an_unchanged_listing_is_not_reported_as_a_change() {
    let manager = ProfileLockManager::new();
    assert!(
      manager
        .replace_display_locks(vec![server_lock("p1", "user-1")])
        .await
    );
    assert!(
      !manager
        .replace_display_locks(vec![server_lock("p1", "user-1")])
        .await
    );
    assert!(
      manager
        .replace_display_locks(vec![server_lock("p1", "user-2")])
        .await
    );
  }

  #[tokio::test]
  async fn disconnecting_forgets_every_lock_taken_here() {
    let manager = ProfileLockManager::new();
    manager.held.write().await.insert("p1".to_string());
    manager
      .replace_display_locks(vec![server_lock("p1", "user-1")])
      .await;
    manager.disconnect().await;
    assert!(manager.held_ids().await.is_empty());
    assert!(manager.get_locks().await.is_empty());
    assert!(!manager.is_connected().await);
  }

  #[tokio::test]
  async fn releasing_with_nothing_held_returns_at_once() {
    let manager = ProfileLockManager::new();
    let started = std::time::Instant::now();
    manager
      .release_all_held_within(SHUTDOWN_RELEASE_TIMEOUT)
      .await;
    assert!(started.elapsed() < Duration::from_millis(500));
  }

  #[test]
  fn a_renewal_that_did_not_refresh_means_the_lock_is_gone() {
    assert_eq!(
      renewal_outcome(201, r#"{"refreshed":true}"#),
      Renewal::Refreshed
    );
    assert_eq!(
      renewal_outcome(200, r#"{"refreshed":false}"#),
      Renewal::Lost
    );
    assert_eq!(renewal_outcome(201, "{}"), Renewal::Lost);
    assert_eq!(
      renewal_outcome(502, r#"{"refreshed":false}"#),
      Renewal::Failed
    );
    assert_eq!(renewal_outcome(200, "<html>"), Renewal::Failed);
  }

  #[test]
  fn a_release_is_retried_only_when_the_server_may_still_accept_it() {
    assert_eq!(release_step(200), ReleaseStep::Done);
    assert_eq!(release_step(404), ReleaseStep::Done);
    assert_eq!(release_step(500), ReleaseStep::Retry);
    assert_eq!(release_step(503), ReleaseStep::Retry);
    assert_eq!(release_step(408), ReleaseStep::Retry);
    assert_eq!(release_step(403), ReleaseStep::GiveUp);
    assert_eq!(release_step(400), ReleaseStep::GiveUp);
  }

  #[test]
  fn release_retries_back_off_and_then_stop() {
    let delays: Vec<Duration> = (0..).map_while(release_retry_delay).collect();
    assert_eq!(delays.len(), 4);
    assert!(delays.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(release_retry_delay(4), None);
  }

  #[test]
  fn the_frontend_listens_to_the_one_event_rust_emits() {
    let hook = include_str!("../../src/hooks/use-team-locks.ts");
    assert!(hook.contains(&format!("\"{LOCK_CHANGED_EVENT}\"")));
    assert!(!hook.contains("team-lock-acquired"));
    assert!(!hook.contains("team-lock-released"));
    let source = include_str!("team_lock.rs");
    let runtime = &source[..source.find("#[cfg(test)]").unwrap_or(source.len())];
    assert_eq!(runtime.matches("crate::events::emit(").count(), 1);
    assert_eq!(runtime.matches("LOCK_CHANGED_EVENT,").count(), 1);
  }

  #[test]
  fn releasing_depends_on_what_this_process_took_and_not_on_the_plan() {
    let source = include_str!("team_lock.rs");
    let release = source
      .split("pub async fn release_team_lock_if_needed(")
      .nth(1)
      .expect("release_team_lock_if_needed must exist");
    let body = &release[..release.find("\n}").unwrap_or(release.len())];
    assert!(body.contains("PROFILE_LOCK.holds("));
    assert!(!body.contains("has_active_paid_subscription"));
    assert!(!body.contains("is_sync_enabled"));
  }

  #[test]
  fn the_app_releases_what_it_holds_when_it_exits() {
    let lib = include_str!("lib.rs");
    let exit = lib
      .split("if let tauri::RunEvent::Exit = _event {")
      .nth(1)
      .expect("the exit handler must exist");
    let body = &exit[..exit.find("\n      }").unwrap_or(exit.len())];
    assert!(body.contains("release_all_held_within(team_lock::SHUTDOWN_RELEASE_TIMEOUT)"));
    assert_eq!(SHUTDOWN_RELEASE_TIMEOUT, Duration::from_secs(2));
  }

  #[test]
  fn a_lock_with_no_identifiable_holder_still_produces_a_code() {
    // Raw English here is what reaches a Russian user untranslated.
    for holder in [None, Some("")] {
      let err = lock_conflict_error("p1", holder, None);
      assert_eq!(err, r#"{"code":"PROFILE_LOCKED_ELSEWHERE"}"#);
    }
  }
}

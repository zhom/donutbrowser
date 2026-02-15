use super::engine::SyncEngine;
use super::subscription::SyncWorkItem;
use crate::events;
use crate::profile::ProfileManager;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::sleep;

static GLOBAL_SCHEDULER: std::sync::Mutex<Option<Arc<SyncScheduler>>> = std::sync::Mutex::new(None);

pub fn get_global_scheduler() -> Option<Arc<SyncScheduler>> {
  GLOBAL_SCHEDULER.lock().ok().and_then(|g| g.clone())
}

pub fn set_global_scheduler(scheduler: Arc<SyncScheduler>) {
  if let Ok(mut g) = GLOBAL_SCHEDULER.lock() {
    *g = Some(scheduler);
  }
}

#[derive(Debug, Clone)]
struct ProfileStopTime {
  #[allow(dead_code)]
  stopped_at: Instant,
  queued: bool,
}

pub struct SyncScheduler {
  running: Arc<AtomicBool>,
  pending_profiles: Arc<Mutex<HashMap<String, ProfileStopTime>>>,
  pending_proxies: Arc<Mutex<HashSet<String>>>,
  pending_groups: Arc<Mutex<HashSet<String>>>,
  pending_tombstones: Arc<Mutex<Vec<(String, String)>>>,
  running_profiles: Arc<Mutex<HashSet<String>>>,
  in_flight_profiles: Arc<Mutex<HashSet<String>>>,
}

impl Default for SyncScheduler {
  fn default() -> Self {
    Self::new()
  }
}

impl SyncScheduler {
  pub fn new() -> Self {
    Self {
      running: Arc::new(AtomicBool::new(false)),
      pending_profiles: Arc::new(Mutex::new(HashMap::new())),
      pending_proxies: Arc::new(Mutex::new(HashSet::new())),
      pending_groups: Arc::new(Mutex::new(HashSet::new())),
      pending_tombstones: Arc::new(Mutex::new(Vec::new())),
      running_profiles: Arc::new(Mutex::new(HashSet::new())),
      in_flight_profiles: Arc::new(Mutex::new(HashSet::new())),
    }
  }

  pub fn is_running(&self) -> bool {
    self.running.load(Ordering::SeqCst)
  }

  pub fn stop(&self) {
    self.running.store(false, Ordering::SeqCst);
  }

  /// Check if any sync operation is currently in progress
  pub async fn is_sync_in_progress(&self) -> bool {
    let in_flight = self.in_flight_profiles.lock().await;
    if !in_flight.is_empty() {
      return true;
    }
    drop(in_flight);

    let pending_profiles = self.pending_profiles.lock().await;
    if !pending_profiles.is_empty() {
      return true;
    }
    drop(pending_profiles);

    let pending_proxies = self.pending_proxies.lock().await;
    if !pending_proxies.is_empty() {
      return true;
    }
    drop(pending_proxies);

    let pending_groups = self.pending_groups.lock().await;
    if !pending_groups.is_empty() {
      return true;
    }
    drop(pending_groups);

    let pending_tombstones = self.pending_tombstones.lock().await;
    if !pending_tombstones.is_empty() {
      return true;
    }

    false
  }

  pub async fn mark_profile_running(&self, profile_id: &str) {
    let mut running = self.running_profiles.lock().await;
    running.insert(profile_id.to_string());
    log::debug!("Marked profile {} as running", profile_id);
  }

  pub async fn mark_profile_stopped(&self, profile_id: &str) {
    let mut running = self.running_profiles.lock().await;
    running.remove(profile_id);
    log::debug!("Marked profile {} as stopped", profile_id);

    let mut pending = self.pending_profiles.lock().await;
    if pending.contains_key(profile_id) {
      // Set stopped_at to past so it syncs immediately
      pending.insert(
        profile_id.to_string(),
        ProfileStopTime {
          stopped_at: Instant::now() - Duration::from_secs(3),
          queued: true,
        },
      );
      log::debug!(
        "Profile {} has pending sync, will execute immediately",
        profile_id
      );
    }
  }

  pub async fn is_profile_running(&self, profile_id: &str) -> bool {
    // First check our internal tracking
    let running = self.running_profiles.lock().await;
    if running.contains(profile_id) {
      return true;
    }
    drop(running);

    // Also check the actual profile state from ProfileManager
    let profile_manager = ProfileManager::instance();
    if let Ok(profiles) = profile_manager.list_profiles() {
      if let Some(profile) = profiles.iter().find(|p| p.id.to_string() == profile_id) {
        return profile.process_id.is_some();
      }
    }

    false
  }

  pub async fn queue_profile_sync(&self, profile_id: String) {
    self.queue_profile_sync_internal(profile_id).await;
  }

  pub async fn queue_profile_sync_immediate(&self, profile_id: String) {
    self.queue_profile_sync_internal(profile_id).await;
  }

  async fn queue_profile_sync_internal(&self, profile_id: String) {
    let is_running = self.is_profile_running(&profile_id).await;
    let mut pending = self.pending_profiles.lock().await;

    if is_running {
      // Profile is running - queue for after it stops
      pending.insert(
        profile_id.clone(),
        ProfileStopTime {
          stopped_at: Instant::now(),
          queued: true,
        },
      );
      log::debug!(
        "Profile {} is running, queued sync for after stop",
        profile_id
      );
    } else {
      // Profile is not running - sync immediately (set stopped_at to past)
      pending.insert(
        profile_id.clone(),
        ProfileStopTime {
          stopped_at: Instant::now() - Duration::from_secs(3),
          queued: true,
        },
      );
      log::debug!("Profile {} queued for immediate sync", profile_id);
    }
  }

  pub async fn queue_proxy_sync(&self, proxy_id: String) {
    let mut pending = self.pending_proxies.lock().await;
    pending.insert(proxy_id);
  }

  pub async fn queue_group_sync(&self, group_id: String) {
    let mut pending = self.pending_groups.lock().await;
    pending.insert(group_id);
  }

  pub async fn queue_tombstone(&self, entity_type: String, entity_id: String) {
    let mut pending = self.pending_tombstones.lock().await;
    if !pending
      .iter()
      .any(|(t, i)| t == &entity_type && i == &entity_id)
    {
      pending.push((entity_type, entity_id));
    }
  }

  pub async fn sync_all_enabled_profiles(&self, _app_handle: &tauri::AppHandle) {
    log::info!("Starting initial sync for all enabled profiles...");

    let profiles = {
      let profile_manager = ProfileManager::instance();
      match profile_manager.list_profiles() {
        Ok(p) => p,
        Err(e) => {
          log::error!("Failed to list profiles for initial sync: {e}");
          return;
        }
      }
    };

    let sync_enabled_profiles: Vec<_> = profiles.into_iter().filter(|p| p.sync_enabled).collect();

    if sync_enabled_profiles.is_empty() {
      log::debug!("No sync-enabled profiles found");
      return;
    }

    log::info!(
      "Found {} sync-enabled profiles, queueing for sync",
      sync_enabled_profiles.len()
    );

    for profile in sync_enabled_profiles {
      let profile_id = profile.id.to_string();
      let is_running = profile.process_id.is_some();

      // Emit initial status
      let _ = events::emit(
        "profile-sync-status",
        serde_json::json!({
          "profile_id": profile_id,
          "status": if is_running { "waiting" } else { "syncing" }
        }),
      );

      // Queue for immediate sync (or wait if running)
      self.queue_profile_sync_immediate(profile_id).await;
    }
  }

  pub async fn start(
    self: Arc<Self>,
    app_handle: tauri::AppHandle,
    mut work_rx: mpsc::UnboundedReceiver<SyncWorkItem>,
  ) {
    if self.running.swap(true, Ordering::SeqCst) {
      return;
    }

    let scheduler = self.clone();
    let app_handle_clone = app_handle.clone();

    tokio::spawn(async move {
      while scheduler.running.load(Ordering::SeqCst) {
        tokio::select! {
          Some(work_item) = work_rx.recv() => {
            match work_item {
              SyncWorkItem::Profile(id) => scheduler.queue_profile_sync(id).await,
              SyncWorkItem::Proxy(id) => scheduler.queue_proxy_sync(id).await,
              SyncWorkItem::Group(id) => scheduler.queue_group_sync(id).await,
              SyncWorkItem::Tombstone(entity_type, entity_id) => {
                scheduler.queue_tombstone(entity_type, entity_id).await
              }
            }
          }
          _ = sleep(Duration::from_millis(500)) => {
            scheduler.process_pending(&app_handle_clone).await;
          }
        }
      }

      log::info!("Sync scheduler stopped");
    });
  }

  async fn process_pending(&self, app_handle: &tauri::AppHandle) {
    self.process_pending_profiles(app_handle).await;
    self.process_pending_proxies(app_handle).await;
    self.process_pending_groups(app_handle).await;
    self.process_pending_tombstones(app_handle).await;
  }

  async fn process_pending_profiles(&self, app_handle: &tauri::AppHandle) {
    let profiles_to_sync: Vec<String> = {
      let mut pending = self.pending_profiles.lock().await;
      let running = self.running_profiles.lock().await;
      let in_flight = self.in_flight_profiles.lock().await;

      // Sync immediately if not running and not in-flight (no delay check)
      let ready: Vec<String> = pending
        .iter()
        .filter(|(id, stop_time)| {
          !running.contains(*id) && !in_flight.contains(*id) && stop_time.queued
        })
        .map(|(id, _)| id.clone())
        .collect();

      for id in &ready {
        pending.remove(id);
      }

      ready
    };

    for profile_id in profiles_to_sync {
      // Mark as in-flight to prevent duplicate syncs
      {
        let mut in_flight = self.in_flight_profiles.lock().await;
        if in_flight.contains(&profile_id) {
          log::debug!("Profile {} already in-flight, skipping", profile_id);
          continue;
        }
        in_flight.insert(profile_id.clone());
      }

      log::info!("Executing queued sync for profile {}", profile_id);
      let _ = events::emit(
        "profile-sync-status",
        serde_json::json!({
          "profile_id": profile_id,
          "status": "syncing"
        }),
      );

      let profile_to_sync = {
        let profile_manager = ProfileManager::instance();
        profile_manager.list_profiles().ok().and_then(|profiles| {
          profiles
            .into_iter()
            .find(|p| p.id.to_string() == profile_id && p.sync_enabled)
        })
      };

      let Some(profile) = profile_to_sync else {
        // Remove from in-flight
        let mut in_flight = self.in_flight_profiles.lock().await;
        in_flight.remove(&profile_id);
        continue;
      };

      let result = match SyncEngine::create_from_settings(app_handle).await {
        Ok(engine) => engine.sync_profile(app_handle, &profile).await,
        Err(e) => {
          log::error!("Failed to create sync engine: {}", e);
          Err(super::types::SyncError::NotConfigured)
        }
      };

      // Remove from in-flight and check if sync just completed
      let sync_just_completed = {
        let mut in_flight = self.in_flight_profiles.lock().await;
        in_flight.remove(&profile_id);
        // If this was the last in-flight profile and there are no pending profiles, sync just completed
        in_flight.is_empty()
          && self.pending_profiles.lock().await.is_empty()
          && self.pending_proxies.lock().await.is_empty()
          && self.pending_groups.lock().await.is_empty()
      };

      match result {
        Ok(()) => {
          log::info!("Profile {} synced successfully", profile_id);
          let _ = events::emit(
            "profile-sync-status",
            serde_json::json!({
              "profile_id": profile_id,
              "status": "synced"
            }),
          );
        }
        Err(e) => {
          log::error!("Failed to sync profile {}: {}", profile_id, e);
          let _ = events::emit(
            "profile-sync-status",
            serde_json::json!({
              "profile_id": profile_id,
              "status": "error",
              "error": e.to_string()
            }),
          );
        }
      }

      // Trigger cleanup after sync completes if this was the last profile
      if sync_just_completed {
        log::debug!("All profile syncs completed, triggering cleanup");
        let registry = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
        if let Err(e) = registry.cleanup_unused_binaries() {
          log::warn!("Cleanup after sync failed: {e}");
        } else {
          log::debug!("Cleanup after sync completed successfully");
        }
      }
    }
  }

  async fn process_pending_proxies(&self, app_handle: &tauri::AppHandle) {
    let proxies_to_sync: Vec<String> = {
      let mut pending = self.pending_proxies.lock().await;
      let list: Vec<String> = pending.drain().collect();
      list
    };

    if proxies_to_sync.is_empty() {
      return;
    }

    match SyncEngine::create_from_settings(app_handle).await {
      Ok(engine) => {
        for proxy_id in proxies_to_sync {
          log::info!("Syncing proxy {}", proxy_id);
          let _ = events::emit(
            "proxy-sync-status",
            serde_json::json!({
              "id": proxy_id,
              "status": "syncing"
            }),
          );
          match engine
            .sync_proxy_by_id_with_handle(&proxy_id, app_handle)
            .await
          {
            Ok(()) => {
              let _ = events::emit(
                "proxy-sync-status",
                serde_json::json!({
                  "id": proxy_id,
                  "status": "synced"
                }),
              );
            }
            Err(e) => {
              log::error!("Failed to sync proxy {}: {}", proxy_id, e);
              let _ = events::emit(
                "proxy-sync-status",
                serde_json::json!({
                  "id": proxy_id,
                  "status": "error"
                }),
              );
            }
          }
        }

        // Check if all sync work is complete after proxies finish
        if !self.is_sync_in_progress().await {
          log::debug!("All syncs completed after proxy sync, triggering cleanup");
          let registry =
            crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
          if let Err(e) = registry.cleanup_unused_binaries() {
            log::warn!("Cleanup after sync failed: {e}");
          } else {
            log::debug!("Cleanup after sync completed successfully");
          }
        }
      }
      Err(e) => {
        log::error!("Failed to create sync engine: {}", e);
      }
    }
  }

  async fn process_pending_groups(&self, app_handle: &tauri::AppHandle) {
    let groups_to_sync: Vec<String> = {
      let mut pending = self.pending_groups.lock().await;
      let list: Vec<String> = pending.drain().collect();
      list
    };

    if groups_to_sync.is_empty() {
      return;
    }

    match SyncEngine::create_from_settings(app_handle).await {
      Ok(engine) => {
        for group_id in groups_to_sync {
          log::info!("Syncing group {}", group_id);
          let _ = events::emit(
            "group-sync-status",
            serde_json::json!({
              "id": group_id,
              "status": "syncing"
            }),
          );
          match engine
            .sync_group_by_id_with_handle(&group_id, app_handle)
            .await
          {
            Ok(()) => {
              let _ = events::emit(
                "group-sync-status",
                serde_json::json!({
                  "id": group_id,
                  "status": "synced"
                }),
              );
            }
            Err(e) => {
              log::error!("Failed to sync group {}: {}", group_id, e);
              let _ = events::emit(
                "group-sync-status",
                serde_json::json!({
                  "id": group_id,
                  "status": "error"
                }),
              );
            }
          }
        }

        // Check if all sync work is complete after groups finish
        if !self.is_sync_in_progress().await {
          log::debug!("All syncs completed after group sync, triggering cleanup");
          let registry =
            crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
          if let Err(e) = registry.cleanup_unused_binaries() {
            log::warn!("Cleanup after sync failed: {e}");
          } else {
            log::debug!("Cleanup after sync completed successfully");
          }
        }
      }
      Err(e) => {
        log::error!("Failed to create sync engine: {}", e);
      }
    }
  }

  async fn process_pending_tombstones(&self, app_handle: &tauri::AppHandle) {
    let tombstones: Vec<(String, String)> = {
      let mut pending = self.pending_tombstones.lock().await;
      std::mem::take(&mut *pending)
    };

    if tombstones.is_empty() {
      return;
    }

    for (entity_type, entity_id) in tombstones {
      log::info!("Processing tombstone for {} {}", entity_type, entity_id);
      match entity_type.as_str() {
        "profile" => {
          let exists_locally = {
            let profile_manager = ProfileManager::instance();
            if let Ok(profiles) = profile_manager.list_profiles() {
              let profile_uuid = uuid::Uuid::parse_str(&entity_id).ok();
              profile_uuid
                .as_ref()
                .map(|uuid| profiles.iter().any(|p| p.id == *uuid))
                .unwrap_or(false)
            } else {
              false
            }
          };

          if exists_locally {
            // Profile exists locally but was deleted remotely - delete locally
            log::info!(
              "Profile {} exists locally, deleting due to remote tombstone",
              entity_id
            );
            // Note: We don't actually delete here to avoid data loss.
            // The user should be notified or we could add a confirmation step.
            // For now, just log it.
          } else {
            // Profile doesn't exist locally - check if it still exists remotely
            // (tombstone might have been created but profile files still exist)
            // Try to download it
            match SyncEngine::create_from_settings(app_handle).await {
              Ok(engine) => {
                if let Ok(true) = engine
                  .download_profile_if_missing(app_handle, &entity_id)
                  .await
                {
                  log::info!(
                    "Downloaded missing profile {} from remote storage",
                    entity_id
                  );
                }
              }
              Err(e) => {
                log::debug!("Sync not configured, skipping profile download: {}", e);
              }
            }
          }
        }
        "proxy" => {
          log::debug!(
            "Proxy tombstone for {} - local deletion not implemented",
            entity_id
          );
        }
        "group" => {
          log::debug!(
            "Group tombstone for {} - local deletion not implemented",
            entity_id
          );
        }
        _ => {}
      }
    }
  }
}

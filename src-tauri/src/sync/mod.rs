mod client;
pub mod encryption;
mod engine;
pub mod manifest;
pub mod preflight;
pub mod scheduler;
pub mod subscription;
pub mod types;

pub use client::SyncClient;
pub use encryption::{
  check_has_e2e_password, delete_e2e_password, set_e2e_password, verify_e2e_password,
};
pub use engine::{
  cancel_profile_sync, enable_extension_group_sync_if_needed, enable_group_sync_if_needed,
  enable_proxy_sync_if_needed, enable_sync_for_all_entities, enable_vpn_sync_if_needed,
  get_unsynced_entity_counts, is_group_in_use_by_synced_profile, is_group_used_by_synced_profile,
  is_proxy_in_use_by_synced_profile, is_proxy_used_by_synced_profile, is_sync_configured,
  is_vpn_in_use_by_synced_profile, is_vpn_used_by_synced_profile,
  pull_profile_after_remote_session, request_profile_sync, rollover_encryption_for_all_entities,
  set_extension_group_sync_enabled, set_extension_sync_enabled, set_group_sync_enabled,
  set_profile_sync_mode, set_proxy_sync_enabled, set_vpn_sync_enabled, sync_profile,
  trigger_sync_for_profile, ProfileSyncOutcome, SyncEngine,
};
pub use manifest::{
  compute_diff, compute_diff_with_bias, generate_manifest, DiffBias, HashCache, ManifestDiff,
  SyncManifest,
};
pub use preflight::{check_sync_server, check_sync_server_connection, SyncServerCheck};
pub use scheduler::{get_global_scheduler, set_global_scheduler, SyncScheduler};
pub use subscription::{SubscriptionManager, SyncWorkItem};
pub use types::{SyncError, SyncResult};

/// The live subscription, held so it can be stopped.
///
/// It used to be a local inside whichever task built the pipeline. Dropping a
/// `SubscriptionManager` does not end its work: `SyncSubscription::start`
/// spawns a task holding clones of the running flag and the work sender, so the
/// task outlived the handle and nothing could reach it. Every restart added one
/// more live SSE connection, each with its own poll loop on the server, and
/// disconnecting left an authenticated stream open to a server the user had
/// just removed.
static GLOBAL_SUBSCRIPTION: std::sync::Mutex<Option<SubscriptionManager>> =
  std::sync::Mutex::new(None);

/// Held for the whole of `start_pipeline`, so only one pipeline is ever being
/// assembled at a time.
static PIPELINE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Retire the running pipeline, both halves of it.
pub fn stop_pipeline() {
  if let Some(scheduler) = get_global_scheduler() {
    scheduler.stop();
  }
  if let Ok(mut guard) = GLOBAL_SUBSCRIPTION.lock() {
    if let Some(subscription) = guard.as_mut() {
      subscription.stop();
    }
    *guard = None;
  }
}

/// Build and start the sync pipeline. Safe to call again to restart it.
///
/// Startup and `restart_sync_service` each held their own copy of this, and the
/// copies had drifted. The restart copy stopped the old scheduler first and then
/// returned early if the subscription failed to start, so it left the
/// application holding a scheduler whose task had already exited. Everything
/// queued afterwards went into `pending_profiles` and was never drained, and
/// sync was silently dead until the app was restarted. One function cannot
/// drift from itself.
pub async fn start_pipeline(app_handle: tauri::AppHandle) {
  // Two restarts arriving together would otherwise interleave: the second
  // retires what the first has not published yet, then both start, and one
  // scheduler is left ticking with nothing able to reach it. Building the
  // pipeline is rare and already awaits the network, so serialising it costs
  // nothing worth measuring.
  let _building = PIPELINE_LOCK.lock().await;

  stop_pipeline();

  let mut subscription_manager = SubscriptionManager::new();
  let Some(work_rx) = subscription_manager.take_work_receiver() else {
    log::error!("Sync pipeline has no work receiver; not starting");
    return;
  };

  // A subscription failure costs live updates from other devices. It does not
  // stop this device syncing its own changes on the timer, so carry on. The
  // restart path used to give up here, which turned a token hiccup into sync
  // being dead until the next launch.
  if let Err(e) = subscription_manager.start(app_handle.clone()).await {
    log::warn!("Failed to start sync subscription, continuing without live updates: {e}");
  }
  if let Ok(mut guard) = GLOBAL_SUBSCRIPTION.lock() {
    *guard = Some(subscription_manager);
  }

  let scheduler = std::sync::Arc::new(SyncScheduler::new());
  // Published before the loop starts, because the checks below await the
  // network and anything queued in the meantime has to land in this scheduler.
  // `stop()` marks it cancelled, so a restart arriving during that window still
  // retires it and `start` below becomes a no-op.
  set_global_scheduler(scheduler.clone());

  scheduler.sync_all_enabled_profiles(&app_handle).await;

  match SyncEngine::create_from_settings(&app_handle).await {
    Ok(engine) => {
      if let Err(e) = engine.check_for_missing_synced_profiles(&app_handle).await {
        log::warn!("Failed to check for missing profiles: {e}");
      }
      if let Err(e) = engine.check_for_missing_synced_entities(&app_handle).await {
        log::warn!("Failed to check for missing entities: {e}");
      }
    }
    Err(e) => {
      log::warn!("Sync not configured, skipping missing profile check: {e}");
    }
  }

  if scheduler.clone().start(app_handle, work_rx).await {
    log::info!("Sync scheduler started");
  }
}

/// Queue a profile sync if the profile has sync enabled. No-op otherwise.
///
/// Called from profile metadata update paths so a rename / tag edit / proxy
/// reassignment shows up on other devices without waiting for the next
/// scheduled tick. Spawns the async queue call so this helper is callable
/// from both sync and async contexts.
pub fn queue_profile_sync_if_eligible(profile: &crate::profile::BrowserProfile) {
  if !profile.is_sync_enabled() {
    return;
  }
  let profile_id = profile.id.to_string();
  tauri::async_runtime::spawn(async move {
    if let Some(scheduler) = get_global_scheduler() {
      scheduler.queue_profile_sync(profile_id).await;
    }
  });
}

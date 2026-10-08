use super::engine::SyncEngine;
use super::subscription::SyncWorkItem;
use crate::events;
use crate::log_redaction::Plain;
use crate::log_streak::Streak;
use crate::profile::ProfileManager;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::Mutex;

static GLOBAL_SCHEDULER: std::sync::Mutex<Option<Arc<SyncScheduler>>> = std::sync::Mutex::new(None);

static SYNC_ENGINE: Streak = Streak::new(module_path!(), "Sync engine setup");

/// The last error per `kind=id`, so an entity that keeps failing the same way
/// logs once, not on every pass.
static FAILING: LazyLock<std::sync::Mutex<HashMap<String, String>>> =
  LazyLock::new(Default::default);

/// Tombstones already reported this run. The server replays every tombstone
/// to each new SSE connection.
static NOTED_TOMBSTONES: LazyLock<std::sync::Mutex<HashSet<String>>> =
  LazyLock::new(Default::default);

fn log_sync_failure(kind: &str, id: &str, err: impl Display) {
  let err = err.to_string();
  let repeated = FAILING
    .lock()
    .map(|mut failing| failing.insert(format!("{kind}={id}"), err.clone()).as_ref() == Some(&err))
    .unwrap_or(false);
  if repeated {
    log::debug!("Sync failed again {kind}={id} err=\"{err}\"");
  } else {
    log::error!("Sync failed {kind}={id} err=\"{err}\"");
  }
}

fn log_sync_success(kind: &str, id: &str) {
  let recovered = FAILING
    .lock()
    .map(|mut failing| failing.remove(&format!("{kind}={id}")).is_some())
    .unwrap_or(false);
  if recovered {
    log::info!("Sync recovered {kind}={id}");
  }
}

fn first_tombstone_notice(entity_id: &str) -> bool {
  NOTED_TOMBSTONES
    .lock()
    .map(|mut noted| noted.insert(entity_id.to_string()))
    .unwrap_or(true)
}

pub fn get_global_scheduler() -> Option<Arc<SyncScheduler>> {
  GLOBAL_SCHEDULER.lock().ok().and_then(|g| g.clone())
}

pub fn set_global_scheduler(scheduler: Arc<SyncScheduler>) {
  if let Ok(mut g) = GLOBAL_SCHEDULER.lock() {
    *g = Some(scheduler);
  }
}

/// What `start` should do, given the flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartDecision {
  /// Nothing is running and nothing retired it. Spawn the loop.
  Start,
  /// A loop is already ticking on this scheduler.
  AlreadyRunning,
  /// `stop` was called on it, possibly before it ever ran.
  Retired,
}

#[derive(Debug, Clone)]
struct ProfileStopTime {
  #[allow(dead_code)]
  stopped_at: Instant,
  queued: bool,
}

pub struct SyncScheduler {
  running: Arc<AtomicBool>,
  /// Set by `stop()` and never cleared. A scheduler is one-shot.
  ///
  /// The pipeline publishes a scheduler before it starts its loop, because work
  /// queued during the network checks in between has to land somewhere. That
  /// left a window where `stop()` cleared a `running` flag that was still
  /// false, so it did nothing, and the scheduler then started anyway and ticked
  /// forever with no way to reach it. `running` cannot express "retired before
  /// it ever ran", so this does.
  cancelled: Arc<AtomicBool>,
  pending_profiles: Arc<Mutex<HashMap<String, ProfileStopTime>>>,
  pending_proxies: Arc<Mutex<HashSet<String>>>,
  pending_groups: Arc<Mutex<HashSet<String>>>,
  pending_vpns: Arc<Mutex<HashSet<String>>>,
  pending_extensions: Arc<Mutex<HashSet<String>>>,
  pending_extension_groups: Arc<Mutex<HashSet<String>>>,
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
      cancelled: Arc::new(AtomicBool::new(false)),
      pending_profiles: Arc::new(Mutex::new(HashMap::new())),
      pending_proxies: Arc::new(Mutex::new(HashSet::new())),
      pending_groups: Arc::new(Mutex::new(HashSet::new())),
      pending_vpns: Arc::new(Mutex::new(HashSet::new())),
      pending_extensions: Arc::new(Mutex::new(HashSet::new())),
      pending_extension_groups: Arc::new(Mutex::new(HashSet::new())),
      pending_tombstones: Arc::new(Mutex::new(Vec::new())),
      running_profiles: Arc::new(Mutex::new(HashSet::new())),
      in_flight_profiles: Arc::new(Mutex::new(HashSet::new())),
    }
  }

  pub fn is_running(&self) -> bool {
    self.running.load(Ordering::SeqCst)
  }

  /// Retire this scheduler for good.
  ///
  /// Order matters: mark it cancelled before clearing `running`, so a `start()`
  /// racing this call cannot slip between the two and begin ticking.
  pub fn stop(&self) {
    self.cancelled.store(true, Ordering::SeqCst);
    self.running.store(false, Ordering::SeqCst);
  }

  /// Whether this specific profile is mid-sync or queued to sync.
  ///
  /// A remote host materialises the profile by pulling the synced manifest, so
  /// launching one while the upload is still running hands it a torn snapshot:
  /// the manifest is written last, but a launch that races a *queued* sync can
  /// still pull files that are about to be replaced. Either way the browser
  /// comes up on a profile that never existed on this machine.
  ///
  /// Deliberately per-profile rather than the global
  /// {@link Self::is_sync_in_progress}: an unrelated profile uploading 80 MB
  /// must not block launching this one.
  pub async fn is_profile_sync_in_progress(&self, profile_id: &str) -> bool {
    if self.in_flight_profiles.lock().await.contains(profile_id) {
      return true;
    }
    self.pending_profiles.lock().await.contains_key(profile_id)
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

    let pending_vpns = self.pending_vpns.lock().await;
    if !pending_vpns.is_empty() {
      return true;
    }
    drop(pending_vpns);

    let pending_extensions = self.pending_extensions.lock().await;
    if !pending_extensions.is_empty() {
      return true;
    }
    drop(pending_extensions);

    let pending_extension_groups = self.pending_extension_groups.lock().await;
    if !pending_extension_groups.is_empty() {
      return true;
    }
    drop(pending_extension_groups);

    let pending_tombstones = self.pending_tombstones.lock().await;
    if !pending_tombstones.is_empty() {
      return true;
    }

    false
  }

  pub async fn mark_profile_running(&self, profile_id: &str) {
    let mut running = self.running_profiles.lock().await;
    running.insert(profile_id.to_string());
    log::debug!("Profile marked running profile={profile_id}");
  }

  pub async fn mark_profile_stopped(&self, profile_id: &str) {
    let mut running = self.running_profiles.lock().await;
    running.remove(profile_id);
    log::debug!("Profile marked stopped profile={profile_id}");

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
      log::debug!("Profile pending sync released profile={profile_id}");
    }
  }

  pub async fn is_profile_running(&self, profile_id: &str) -> bool {
    // Check our internal tracking (authoritative — immediately updated by mark_profile_stopped)
    let running = self.running_profiles.lock().await;
    if running.contains(profile_id) {
      return true;
    }
    drop(running);

    // Check if locked by another device (profile in use remotely)
    if crate::team_lock::PROFILE_LOCK
      .is_locked_by_another(profile_id)
      .await
    {
      log::debug!("Profile locked on another device, treated as running profile={profile_id}");
      return true;
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
        "Profile sync queued until stop profile={}",
        Plain(&profile_id)
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
      log::debug!("Profile sync queued profile={}", Plain(&profile_id));
    }
  }

  pub async fn queue_proxy_sync(&self, proxy_id: String) {
    let mut pending = self.pending_proxies.lock().await;
    pending.insert(proxy_id);
  }

  pub async fn queue_vpn_sync(&self, vpn_id: String) {
    let mut pending = self.pending_vpns.lock().await;
    pending.insert(vpn_id);
  }

  pub async fn queue_group_sync(&self, group_id: String) {
    let mut pending = self.pending_groups.lock().await;
    pending.insert(group_id);
  }

  pub async fn queue_extension_sync(&self, extension_id: String) {
    let mut pending = self.pending_extensions.lock().await;
    pending.insert(extension_id);
  }

  pub async fn queue_extension_group_sync(&self, extension_group_id: String) {
    let mut pending = self.pending_extension_groups.lock().await;
    pending.insert(extension_group_id);
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
    let profiles = {
      let profile_manager = ProfileManager::instance();
      match profile_manager.list_profiles() {
        Ok(p) => p,
        Err(e) => {
          log::error!("Initial sync not queued: profile list failed err=\"{e}\"");
          return;
        }
      }
    };

    let sync_enabled_profiles: Vec<_> = profiles
      .into_iter()
      .filter(|p| p.is_sync_enabled())
      .collect();

    if sync_enabled_profiles.is_empty() {
      log::debug!("Initial sync skipped: no sync-enabled profiles");
      return;
    }

    log::info!(
      "Initial sync queued profiles={}",
      sync_enabled_profiles.len()
    );

    for profile in sync_enabled_profiles {
      let profile_id = profile.id.to_string();
      let is_running = profile.process_id.is_some();
      let is_team_locked = crate::team_lock::TEAM_LOCK
        .is_locked_by_another(&profile_id)
        .await;
      let should_wait = is_running || is_team_locked;

      // Track running state in the scheduler
      if is_running {
        self.mark_profile_running(&profile_id).await;
      }

      if should_wait {
        log::debug!(
          "Initial sync deferred profile={profile_id} reason={}",
          if is_running { "running" } else { "team_locked" }
        );
      }

      // Emit initial status
      let _ = events::emit(
        "profile-sync-status",
        serde_json::json!({
          "profile_id": profile_id,
          "status": if should_wait { "waiting" } else { "syncing" }
        }),
      );

      // Queue for sync — running profiles will be deferred by the scheduler
      self.queue_profile_sync_immediate(profile_id).await;
    }
  }

  /// The decision `start` makes before it spawns anything.
  ///
  /// Split out so it can be tested. `start` needs a `tauri::AppHandle`, which a
  /// unit test cannot build, and the retirement rule is the part worth pinning
  /// down. The `running` check stays a `swap` so two concurrent starts cannot
  /// both win.
  fn claim_start_slot(&self) -> StartDecision {
    if self.cancelled.load(Ordering::SeqCst) {
      return StartDecision::Retired;
    }
    if self.running.swap(true, Ordering::SeqCst) {
      return StartDecision::AlreadyRunning;
    }
    StartDecision::Start
  }

  /// Begin ticking. Returns whether a loop was actually started, so the caller
  /// can log the truth instead of assuming.
  pub async fn start(
    self: Arc<Self>,
    app_handle: tauri::AppHandle,
    mut work_rx: mpsc::UnboundedReceiver<SyncWorkItem>,
  ) -> bool {
    match self.claim_start_slot() {
      StartDecision::Retired => {
        // Retired while the pipeline was still assembling it. Starting now
        // would leave a task nothing can stop, because the handle in the global
        // has already been replaced.
        log::info!("Sync scheduler not started: retired before start");
        return false;
      }
      StartDecision::AlreadyRunning => {
        log::warn!("Sync scheduler already running, second start ignored");
        return false;
      }
      StartDecision::Start => {}
    }

    let scheduler = self.clone();
    let app_handle_clone = app_handle.clone();

    tokio::spawn(async move {
      // A fresh `sleep` inside the `select!` restarts from zero on every
      // iteration, so a steady stream of work items kept resetting it and
      // `process_pending` never ran: queued profiles sat there for as long as
      // the stream lasted. An interval keeps its own schedule regardless of how
      // often the other arm fires. `Delay` rather than `Burst` so a slow
      // `process_pending` does not come back to a pile of missed ticks and run
      // itself back to back.
      let mut ticker = tokio::time::interval(Duration::from_millis(2000));
      ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
      // The first tick of an interval resolves immediately. The old shape
      // always waited 2000 ms before its first pass, so consume it here and
      // keep that behaviour.
      ticker.tick().await;

      // Once the senders are gone `recv()` resolves instantly and forever, so
      // the arm has to be disabled or the loop spins hot on a dead channel.
      let mut work_channel_open = true;

      while scheduler.running.load(Ordering::SeqCst) {
        tokio::select! {
          received = work_rx.recv(), if work_channel_open => {
            match received {
              Some(work_item) => match work_item {
                SyncWorkItem::Profile(id) => {
                  if crate::remote_handoff::pending_session_for(&id).is_some() {
                    crate::remote_handoff::schedule_pull(app_handle_clone.clone(), id);
                  } else {
                    scheduler.queue_profile_sync(id).await;
                  }
                }
                SyncWorkItem::Proxy(id) => scheduler.queue_proxy_sync(id).await,
                SyncWorkItem::Group(id) => scheduler.queue_group_sync(id).await,
                SyncWorkItem::Vpn(id) => scheduler.queue_vpn_sync(id).await,
                SyncWorkItem::Extension(id) => scheduler.queue_extension_sync(id).await,
                SyncWorkItem::ExtensionGroup(id) => scheduler.queue_extension_group_sync(id).await,
                SyncWorkItem::Tombstone(entity_type, entity_id) => {
                  scheduler.queue_tombstone(entity_type, entity_id).await
                }
              },
              None => {
                // The subscription is gone, so no more live updates from other
                // devices. Local changes and the timer still work, so keep
                // ticking rather than ending the scheduler.
                log::warn!("Sync work channel closed, timer only from now");
                work_channel_open = false;
              }
            }
          }
          _ = ticker.tick() => {
            scheduler.process_pending(&app_handle_clone).await;
          }
        }
      }

      log::info!("Sync scheduler stopped");
    });

    true
  }

  async fn process_pending(&self, app_handle: &tauri::AppHandle) {
    // Deletions first. A queued sync for an entity another device deleted would
    // otherwise re-upload it from the local copy this tick is about to remove.
    self.process_pending_tombstones(app_handle).await;
    self.process_pending_profiles(app_handle).await;
    self.process_pending_proxies(app_handle).await;
    self.process_pending_groups(app_handle).await;
    self.process_pending_vpns(app_handle).await;
    self.process_pending_extensions(app_handle).await;
    self.process_pending_extension_groups(app_handle).await;
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

    // Mark all profiles as in-flight and filter out duplicates
    let mut to_sync = Vec::new();
    for profile_id in profiles_to_sync {
      let mut in_flight = self.in_flight_profiles.lock().await;
      if in_flight.contains(&profile_id) {
        log::debug!("Profile sync already in flight profile={profile_id}");
        continue;
      }
      in_flight.insert(profile_id.clone());
      to_sync.push(profile_id);
    }

    // Sync all profiles in parallel
    let mut sync_set = tokio::task::JoinSet::new();
    for profile_id in to_sync {
      let app = app_handle.clone();
      let in_flight = self.in_flight_profiles.clone();
      sync_set.spawn(async move {
        let started = Instant::now();
        log::debug!("Profile sync started profile={profile_id}");
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
              .find(|p| p.id.to_string() == profile_id && p.is_sync_enabled())
          })
        };

        let Some(profile) = profile_to_sync else {
          let mut inf = in_flight.lock().await;
          inf.remove(&profile_id);
          return;
        };

        let result = match SyncEngine::create_from_settings(&app).await {
          Ok(engine) => {
            SYNC_ENGINE.succeeded();
            engine
              .sync_profile_with_bias(&app, &profile, super::DiffBias::Auto)
              .await
          }
          Err(e) => {
            SYNC_ENGINE.failed(e);
            Err(super::types::SyncError::NotConfigured)
          }
        };

        {
          let mut inf = in_flight.lock().await;
          inf.remove(&profile_id);
        }

        match result {
          Ok(super::ProfileSyncOutcome::Completed) => {
            log_sync_success("profile", &profile_id);
            log::debug!(
              "Profile sync completed profile={profile_id} elapsed_ms={}",
              started.elapsed().as_millis()
            );
            let _ = events::emit(
              "profile-sync-status",
              serde_json::json!({
                "profile_id": profile_id,
                "status": "synced"
              }),
            );
          }
          Ok(super::ProfileSyncOutcome::Skipped(reason)) => {
            log::debug!("Profile sync skipped profile={profile_id} reason=\"{reason}\"");
          }
          Err(e) => {
            // Both are logged where they happen.
            if matches!(
              e,
              super::types::SyncError::Cancelled | super::types::SyncError::NotConfigured
            ) {
              log::debug!("Profile sync ended profile={profile_id} err=\"{e}\"");
            } else {
              log_sync_failure("profile", &profile_id, &e);
            }
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
      });
    }

    // Wait for all parallel syncs to finish (only if we actually spawned any)
    if !sync_set.is_empty() {
      while let Some(result) = sync_set.join_next().await {
        if let Err(e) = result {
          log::error!("Profile sync task panicked err=\"{e}\"");
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
        SYNC_ENGINE.succeeded();
        for proxy_id in proxies_to_sync {
          log::debug!("Sync started proxy={proxy_id}");
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
              log_sync_success("proxy", &proxy_id);
              let _ = events::emit(
                "proxy-sync-status",
                serde_json::json!({
                  "id": proxy_id,
                  "status": "synced"
                }),
              );
            }
            Err(e) => {
              log_sync_failure("proxy", &proxy_id, &e);
              let _ = events::emit(
                "proxy-sync-status",
                serde_json::json!({
                  "id": proxy_id,
                  "status": "error",
                  "error": e.to_string()
                }),
              );
            }
          }
        }

        // Check if all sync work is complete after proxies finish
      }
      Err(e) => SYNC_ENGINE.failed(e),
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
        SYNC_ENGINE.succeeded();
        for group_id in groups_to_sync {
          log::debug!("Sync started group={group_id}");
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
              log_sync_success("group", &group_id);
              let _ = events::emit(
                "group-sync-status",
                serde_json::json!({
                  "id": group_id,
                  "status": "synced"
                }),
              );
            }
            Err(e) => {
              log_sync_failure("group", &group_id, &e);
              let _ = events::emit(
                "group-sync-status",
                serde_json::json!({
                  "id": group_id,
                  "status": "error",
                  "error": e.to_string()
                }),
              );
            }
          }
        }

        // Check if all sync work is complete after groups finish
      }
      Err(e) => SYNC_ENGINE.failed(e),
    }
  }

  async fn process_pending_vpns(&self, app_handle: &tauri::AppHandle) {
    let vpns_to_sync: Vec<String> = {
      let mut pending = self.pending_vpns.lock().await;
      let list: Vec<String> = pending.drain().collect();
      list
    };

    if vpns_to_sync.is_empty() {
      return;
    }

    match SyncEngine::create_from_settings(app_handle).await {
      Ok(engine) => {
        SYNC_ENGINE.succeeded();
        for vpn_id in vpns_to_sync {
          log::debug!("Sync started vpn={vpn_id}");
          let _ = events::emit(
            "vpn-sync-status",
            serde_json::json!({
              "id": vpn_id,
              "status": "syncing"
            }),
          );
          match engine.sync_vpn_by_id_with_handle(&vpn_id, app_handle).await {
            Ok(()) => {
              log_sync_success("vpn", &vpn_id);
              let _ = events::emit(
                "vpn-sync-status",
                serde_json::json!({
                  "id": vpn_id,
                  "status": "synced"
                }),
              );
            }
            Err(e) => {
              log_sync_failure("vpn", &vpn_id, &e);
              let _ = events::emit(
                "vpn-sync-status",
                serde_json::json!({
                  "id": vpn_id,
                  "status": "error",
                  "error": e.to_string()
                }),
              );
            }
          }
        }
      }
      Err(e) => SYNC_ENGINE.failed(e),
    }
  }

  async fn process_pending_extensions(&self, app_handle: &tauri::AppHandle) {
    let extensions_to_sync: Vec<String> = {
      let mut pending = self.pending_extensions.lock().await;
      let list: Vec<String> = pending.drain().collect();
      list
    };

    if extensions_to_sync.is_empty() {
      return;
    }

    match SyncEngine::create_from_settings(app_handle).await {
      Ok(engine) => {
        SYNC_ENGINE.succeeded();
        for ext_id in extensions_to_sync {
          log::debug!("Sync started extension={ext_id}");
          let _ = events::emit(
            "extension-sync-status",
            serde_json::json!({ "id": ext_id, "status": "syncing" }),
          );
          if let Err(e) = engine
            .sync_extension_by_id_with_handle(&ext_id, app_handle)
            .await
          {
            log_sync_failure("extension", &ext_id, &e);
            let _ = events::emit(
              "extension-sync-status",
              serde_json::json!({ "id": ext_id, "status": "error" }),
            );
          } else {
            log_sync_success("extension", &ext_id);
            let _ = events::emit(
              "extension-sync-status",
              serde_json::json!({ "id": ext_id, "status": "synced" }),
            );
          }
        }
      }
      Err(e) => SYNC_ENGINE.failed(e),
    }
  }

  async fn process_pending_extension_groups(&self, app_handle: &tauri::AppHandle) {
    let groups_to_sync: Vec<String> = {
      let mut pending = self.pending_extension_groups.lock().await;
      let list: Vec<String> = pending.drain().collect();
      list
    };

    if groups_to_sync.is_empty() {
      return;
    }

    match SyncEngine::create_from_settings(app_handle).await {
      Ok(engine) => {
        SYNC_ENGINE.succeeded();
        for group_id in groups_to_sync {
          log::debug!("Sync started extension_group={group_id}");
          let _ = events::emit(
            "extension-sync-status",
            serde_json::json!({ "id": group_id, "status": "syncing" }),
          );
          if let Err(e) = engine
            .sync_extension_group_by_id_with_handle(&group_id, app_handle)
            .await
          {
            log_sync_failure("extension_group", &group_id, &e);
            let _ = events::emit(
              "extension-sync-status",
              serde_json::json!({ "id": group_id, "status": "error" }),
            );
          } else {
            log_sync_success("extension_group", &group_id);
            let _ = events::emit(
              "extension-sync-status",
              serde_json::json!({ "id": group_id, "status": "synced" }),
            );
          }
        }
      }
      Err(e) => SYNC_ENGINE.failed(e),
    }
  }

  /// Forget a queued config sync for an entity whose deletion is being applied
  /// this tick, so the drain that follows cannot re-upload it.
  async fn drop_pending_config_sync(&self, entity_type: &str, entity_id: &str) {
    match entity_type {
      "proxy" => {
        self.pending_proxies.lock().await.remove(entity_id);
      }
      "group" => {
        self.pending_groups.lock().await.remove(entity_id);
      }
      "vpn" => {
        self.pending_vpns.lock().await.remove(entity_id);
      }
      "extension" => {
        self.pending_extensions.lock().await.remove(entity_id);
      }
      "extension_group" => {
        self.pending_extension_groups.lock().await.remove(entity_id);
      }
      _ => {}
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

    for (entity_type, entity_id) in &tombstones {
      self.drop_pending_config_sync(entity_type, entity_id).await;
    }

    for (entity_type, entity_id) in tombstones {
      log::debug!("Tombstone received {entity_type}={entity_id}");
      match entity_type.as_str() {
        "profile" => {
          let profile_manager = ProfileManager::instance();
          let found = uuid::Uuid::parse_str(&entity_id).ok().and_then(|uuid| {
            profile_manager
              .list_profiles()
              .ok()?
              .into_iter()
              .find(|p| p.id == uuid)
          });

          let local = match found {
            Some(profile) if profile.is_sync_enabled() => profile,
            Some(_) => {
              if first_tombstone_notice(&entity_id) {
                log::info!("Tombstone ignored, sync off locally profile={entity_id}");
              }
              continue;
            }
            None => {
              log::debug!("Tombstone ignored, profile not on this device profile={entity_id}");
              continue;
            }
          };

          // The event can be stale. A restore from the trash, or sync switched
          // off and on again, clears the tombstone after it was written, and
          // the profile it named is live again. Only a tombstone that is still
          // in the cloud erases the local copy.
          let still_tombstoned = match SyncEngine::create_from_settings(app_handle).await {
            Ok(engine) => {
              SYNC_ENGINE.succeeded();
              engine.profile_tombstone_exists(&local).await
            }
            Err(e) => {
              SYNC_ENGINE.failed(e);
              continue;
            }
          };
          if !still_tombstoned {
            log::info!("Tombstone stale, kept local profile profile={entity_id}");
            continue;
          }

          match profile_manager.delete_profile_local_only(&entity_id) {
            Ok(_) => log::info!("Profile deleted remotely, removed locally profile={entity_id}"),
            Err(e) => log::warn!(
              "Remotely deleted profile not removed locally profile={entity_id} err=\"{e}\""
            ),
          }
        }
        "proxy" => {
          let proxy_manager = &crate::proxy_manager::PROXY_MANAGER;
          let proxies = proxy_manager.get_stored_proxies();
          if let Some(proxy) = proxies.iter().find(|p| p.id == entity_id) {
            if proxy.sync_enabled {
              log::info!("Proxy deleted remotely, removed locally proxy={entity_id}");
              let proxy_file = proxy_manager.get_proxy_file_path(&entity_id);
              if proxy_file.exists() {
                let _ = std::fs::remove_file(&proxy_file);
              }
              proxy_manager.remove_from_memory(&entity_id);
              let _ = events::emit("stored-proxies-changed", ());
            }
          }
        }
        "group" => {
          let group_manager = crate::group_manager::GROUP_MANAGER.lock().unwrap();
          let groups = group_manager.get_all_groups().unwrap_or_default();
          if let Some(group) = groups.iter().find(|g| g.id == entity_id) {
            if group.sync_enabled {
              log::info!("Group deleted remotely, removed locally group={entity_id}");
              let _ = group_manager.delete_group_internal(&entity_id);
              let _ = events::emit("groups-changed", ());
            }
          }
        }
        "vpn" => {
          let storage = crate::vpn::VPN_STORAGE.lock().unwrap();
          if let Ok(vpn) = storage.load_config(&entity_id) {
            if vpn.sync_enabled {
              log::info!("VPN deleted remotely, removed locally vpn={entity_id}");
              let _ = storage.delete_config(&entity_id);
              let _ = events::emit("vpn-configs-changed", ());
            }
          }
        }
        "extension" => {
          let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
          if let Ok(ext) = manager.get_extension(&entity_id) {
            if ext.sync_enabled {
              log::info!("Extension deleted remotely, removed locally extension={entity_id}");
              let _ = manager.delete_extension_internal(&entity_id);
              let _ = events::emit("extensions-changed", ());
            }
          }
        }
        "extension_group" => {
          let manager = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
          if let Ok(group) = manager.get_group(&entity_id) {
            if group.sync_enabled {
              log::info!(
                "Extension group deleted remotely, removed locally extension_group={entity_id}"
              );
              let _ = manager.delete_group_internal(&entity_id);
              let _ = events::emit("extensions-changed", ());
            }
          }
        }
        _ => {}
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_fresh_scheduler_starts_once() {
    let scheduler = SyncScheduler::new();
    assert_eq!(scheduler.claim_start_slot(), StartDecision::Start);
    assert!(scheduler.is_running());
    assert_eq!(
      scheduler.claim_start_slot(),
      StartDecision::AlreadyRunning,
      "a second start must not spawn a second loop on the same scheduler"
    );
  }

  #[test]
  fn a_scheduler_retired_before_it_ran_never_starts() {
    // The pipeline publishes a scheduler, then awaits two network checks, then
    // starts the loop. A restart landing in that window calls `stop()` on a
    // scheduler that has not started yet. `running` was already false, so the
    // old `stop()` did nothing at all, the loop started afterwards, and it
    // ticked forever with the global already pointing elsewhere.
    let scheduler = SyncScheduler::new();
    assert!(!scheduler.is_running());

    scheduler.stop();

    assert_eq!(
      scheduler.claim_start_slot(),
      StartDecision::Retired,
      "a scheduler stopped before starting must stay stopped"
    );
    assert!(
      !scheduler.is_running(),
      "refusing to start must not leave the running flag set"
    );
  }

  #[test]
  fn stopping_a_running_scheduler_retires_it_for_good() {
    let scheduler = SyncScheduler::new();
    assert_eq!(scheduler.claim_start_slot(), StartDecision::Start);

    scheduler.stop();
    assert!(!scheduler.is_running());

    // A scheduler is one-shot. Restarting the pipeline builds a new one, so a
    // retired instance coming back to life could only ever be a duplicate.
    assert_eq!(scheduler.claim_start_slot(), StartDecision::Retired);
  }

  #[tokio::test]
  async fn test_drop_pending_config_sync_removes_only_the_deleted_entity() {
    let scheduler = SyncScheduler::new();
    scheduler.queue_proxy_sync("proxy-1".to_string()).await;
    scheduler.queue_group_sync("group-1".to_string()).await;
    assert!(scheduler.is_sync_in_progress().await);

    // A tombstone drops that entity's queued sync, otherwise the drain that
    // follows re-uploads the copy this tick is about to delete. Every other
    // queued entity is left alone.
    scheduler.drop_pending_config_sync("proxy", "proxy-1").await;
    assert!(scheduler.pending_proxies.lock().await.is_empty());
    assert!(scheduler.pending_groups.lock().await.contains("group-1"));

    scheduler.drop_pending_config_sync("group", "group-1").await;
    assert!(!scheduler.is_sync_in_progress().await);
  }
}

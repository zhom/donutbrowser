//! What a remote session owes this machine, and the gate that collects it.
//!
//! A profile that runs on the leased fleet is written by the host, not here.
//! The host pushes it back to cloud storage when the session ends, and until
//! this machine has pulled that push, the local profile directory is a stale
//! copy of something that has moved on.
//!
//! Opening that stale copy is not a cosmetic problem, it is destructive. The
//! local browser writes, every local mtime jumps past the host's push, and the
//! next ordinary sync therefore reads local as the newer side: it uploads the
//! pre-session files and puts everything the host wrote into
//! `files_to_delete_remote`. A night of cookie warming is deleted with no error
//! anywhere. Nothing in the manifest can prevent this, because by then the local
//! clock genuinely IS later.
//!
//! So the gate is here instead, and it is deliberately a LOCAL, per-machine
//! fact rather than a synced one. "This computer has not yet pulled" is true of
//! one computer at a time; putting it in the profile's synced metadata would let
//! a second device that had already pulled clear it for a first device that had
//! not.
//!
//! Two states, and the difference matters to the user:
//!
//! - [`HandoffState::Running`]: a session is live on the fleet. The profile lock
//!   is held server-side, so a launch would be refused anyway; this makes the
//!   refusal instant and legible instead of a round trip and a raw string.
//! - [`HandoffState::PendingSync`]: the session is over, the lock is released,
//!   and the work is sitting in cloud storage. This is the window that used to
//!   be wide open.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Duration;

/// Emitted whenever the set of gated profiles changes.
pub const EVENT_REMOTE_HANDOFF: &str = "remote-handoff-changed";

/// Attempts at pulling a finished session's work before giving up for now.
///
/// The entry survives a failure, so "giving up" only means this burst stops;
/// the next stream event, app start or manual sync tries again. What the retries
/// buy is the common case: the profile lock is released server-side a moment
/// before this machine's cached copy of it expires, and a single attempt would
/// hit `Skipped("profile is locked elsewhere")` and leave the user blocked for
/// no reason.
const PULL_ATTEMPTS: u32 = 5;

/// Delay before the second pull attempt. Doubles, capped by [`PULL_RETRY_MAX`].
const PULL_RETRY_BASE: Duration = Duration::from_secs(2);

/// Ceiling on the pull backoff. Above the 30s profile-lock refresh, so a run of
/// attempts is guaranteed to span at least one refresh of the lock cache.
const PULL_RETRY_MAX: Duration = Duration::from_secs(45);

/// Where a profile stands with respect to the fleet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffState {
  /// A session is live on the fleet right now.
  Running,
  /// A session has finished and its work has not been pulled down yet.
  PendingSync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HandoffEntry {
  session_id: String,
  state: HandoffState,
  /// When this entry last changed, unix seconds. Diagnostics only; the gate
  /// never expires on its own, because an entry that timed out would reopen
  /// exactly the window it exists to close.
  observed_at: u64,
}

type Store = HashMap<String, HandoffEntry>;

static STORE: RwLock<Option<Store>> = RwLock::new(None);

fn store_path() -> std::path::PathBuf {
  crate::app_dirs::settings_dir().join("remote_handoff.json")
}

fn load_from_disk() -> Store {
  let path = store_path();
  let Ok(bytes) = std::fs::read(&path) else {
    return Store::new();
  };
  match serde_json::from_slice::<Store>(&bytes) {
    Ok(store) => store,
    Err(e) => {
      // Losing the file means losing the gate, so say so loudly rather than
      // starting empty and quietly permitting a launch over pending work.
      log::error!(
        "Could not read {}: {e}. Profiles with unsynced remote work will not be gated until \
         the next session event.",
        path.display()
      );
      Store::new()
    }
  }
}

fn persist(store: &Store) {
  let path = store_path();
  if let Some(parent) = path.parent() {
    if let Err(e) = std::fs::create_dir_all(parent) {
      log::warn!("Could not create {}: {e}", parent.display());
      return;
    }
  }
  match serde_json::to_vec_pretty(store) {
    Ok(bytes) => {
      if let Err(e) = crate::app_dirs::write_owner_only(&path, &bytes) {
        log::warn!("Could not write {}: {e}", path.display());
      }
    }
    Err(e) => log::warn!("Could not encode the remote handoff store: {e}"),
  }
}

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
  let mut guard = STORE
    .write()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  let store = guard.get_or_insert_with(load_from_disk);
  f(store)
}

/// Apply a mutation, and persist plus announce it only if it changed anything.
fn mutate(f: impl FnOnce(&mut Store) -> bool) {
  let changed = with_store(|store| {
    let changed = f(store);
    if changed {
      persist(store);
    }
    changed
  });
  if changed {
    announce();
  }
}

fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

fn announce() {
  let _ = crate::events::emit(EVENT_REMOTE_HANDOFF, states());
}

/// Every gated profile, for the UI and for one-shot reads.
pub fn states() -> HashMap<String, HandoffState> {
  with_store(|store| {
    store
      .iter()
      .map(|(profile_id, entry)| (profile_id.clone(), entry.state))
      .collect()
  })
}

/// Where this profile stands, if it is gated at all.
pub fn state_for(profile_id: &str) -> Option<HandoffState> {
  with_store(|store| store.get(profile_id).map(|entry| entry.state))
}

/// The session currently holding this profile on the fleet, if any.
///
/// Answers for a `provisioning` session too, which the drivable-session index
/// deliberately does not. Stopping a session that has not finished coming up is
/// the single most common thing a user does after starting one by mistake, and
/// an index built for "where do I attach a CDP client" cannot serve it.
pub fn running_session_for_profile(profile_id: &str) -> Option<String> {
  with_store(|store| {
    store
      .get(profile_id)
      .filter(|entry| entry.state == HandoffState::Running)
      .map(|entry| entry.session_id.clone())
  })
}

/// Which profile a session belongs to, as this machine last recorded it.
///
/// The backend's stop reply carries a session id and a duration but no profile,
/// and the caller that pressed stop needs to know whose work to pull. Reading it
/// back from the gate avoids a second round trip for something already known.
pub fn profile_for_session(session_id: &str) -> Option<String> {
  with_store(|store| {
    store
      .iter()
      .find(|(_, entry)| entry.session_id == session_id)
      .map(|(profile_id, _)| profile_id.clone())
  })
}

/// Record that a session is live on the fleet for this profile.
///
/// Written to disk immediately, and this is the point of the whole store: if the
/// app is closed while a session runs, nothing on restart would otherwise
/// distinguish "this profile is fine" from "a host has been writing to this
/// profile for the last hour".
pub fn note_running(profile_id: &str, session_id: &str) {
  mutate(|store| {
    let entry = store.get(profile_id);
    if entry
      .is_some_and(|held| held.state == HandoffState::Running && held.session_id == session_id)
    {
      return false;
    }
    store.insert(
      profile_id.to_string(),
      HandoffEntry {
        session_id: session_id.to_string(),
        state: HandoffState::Running,
        observed_at: now_secs(),
      },
    );
    true
  });
}

/// Record that a session has finished and its work is waiting in cloud storage.
///
/// Returns whether this call is the one that moved the profile into
/// `PendingSync`, so the caller starts exactly one pull for a transition that
/// the stream may well deliver more than once.
pub fn note_ended(profile_id: &str, session_id: &str) -> bool {
  let mut transitioned = false;
  mutate(|store| {
    // Only a session this machine was watching can hand work over to it.
    //
    // No entry means one of two things and both say "do nothing": the pull for
    // this session already completed and cleared the gate, or this machine
    // never held the profile. The backend's listing returns closed sessions
    // alongside live ones, so the snapshot on every reconnect replays each
    // finished session — treating those as fresh handoffs would gate a
    // perfectly current profile on every app start, and keep it blocked for as
    // long as the machine happened to be offline.
    let Some(entry) = store.get(profile_id) else {
      return false;
    };
    // A late `closed` for a session that has already been replaced by a newer
    // one must not mark the newer one's profile as finished.
    if entry.session_id != session_id {
      return false;
    }
    if entry.state == HandoffState::PendingSync {
      return false;
    }
    transitioned = true;
    store.insert(
      profile_id.to_string(),
      HandoffEntry {
        session_id: session_id.to_string(),
        state: HandoffState::PendingSync,
        observed_at: now_secs(),
      },
    );
    true
  });
  transitioned
}

/// Drop the gate. Called only after a pull has actually completed.
pub fn clear(profile_id: &str) {
  mutate(|store| store.remove(profile_id).is_some());
}

/// Bring stored `Running` entries back in line with what the backend reports.
///
/// The stream is how a transition normally arrives, and it cannot deliver one
/// that happened while the app was shut. Any profile this machine last saw
/// running, whose session the backend no longer reports as live, finished
/// without being observed — and its work is sitting in cloud storage unpulled.
/// Returns the profiles that just moved into `PendingSync`.
pub fn reconcile(live_session_ids: &std::collections::HashSet<String>) -> Vec<String> {
  let mut ended = Vec::new();
  mutate(|store| {
    let stale: Vec<(String, String)> = store
      .iter()
      .filter(|(_, entry)| entry.state == HandoffState::Running)
      .filter(|(_, entry)| !live_session_ids.contains(&entry.session_id))
      .map(|(profile_id, entry)| (profile_id.clone(), entry.session_id.clone()))
      .collect();
    for (profile_id, session_id) in stale {
      log::info!(
        "Remote session {session_id} for profile {profile_id} ended while this machine was not \
         watching; its work is still in cloud storage"
      );
      store.insert(
        profile_id.clone(),
        HandoffEntry {
          session_id,
          state: HandoffState::PendingSync,
          observed_at: now_secs(),
        },
      );
      ended.push(profile_id);
    }
    !ended.is_empty()
  });
  ended
}

/// Refuse a local launch that would run over unsynced remote work.
///
/// Returns the `{"code":…}` string a Tauri command and the REST layer both
/// surface. Every local launch path calls this: the two that did not are how a
/// profile could be opened locally while a host was still writing to it.
pub fn ensure_local_launch_allowed(profile_id: &str) -> Result<(), String> {
  match state_for(profile_id) {
    None => Ok(()),
    Some(HandoffState::Running) => Err(crate::backend_error("PROFILE_RUNNING_REMOTELY")),
    Some(HandoffState::PendingSync) => Err(crate::backend_error("PROFILE_REMOTE_SYNC_PENDING")),
  }
}

/// Restart the pull for every profile still waiting on one.
///
/// A pull can fail for as long as the machine is offline, and its retries are
/// bounded, so without this a profile could stay blocked from launching until
/// the user found the manual sync button. Called whenever the app has a cloud
/// session again, which is exactly when a previously impossible pull becomes
/// possible.
pub fn resume_pending_pulls(app_handle: &tauri::AppHandle) {
  let pending: Vec<String> = with_store(|store| {
    store
      .iter()
      .filter(|(_, entry)| entry.state == HandoffState::PendingSync)
      .map(|(profile_id, _)| profile_id.clone())
      .collect()
  });
  for profile_id in pending {
    log::info!("Resuming the post-session pull for profile {profile_id}");
    schedule_pull(app_handle.clone(), profile_id);
  }
}

/// Pull one profile's finished session down, then lift its gate.
///
/// Spawned rather than awaited by its callers: a stream frame and a stop button
/// must not block on a transfer that can take minutes. The gate stays up for the
/// whole attempt, so there is no window in which the user can open the stale
/// copy while this is in flight.
pub fn schedule_pull(app_handle: tauri::AppHandle, profile_id: String) {
  tauri::async_runtime::spawn(async move {
    for attempt in 0..PULL_ATTEMPTS {
      if state_for(&profile_id) != Some(HandoffState::PendingSync) {
        // A new session started, or another pull got there first.
        return;
      }
      if attempt > 0 {
        let delay = PULL_RETRY_BASE
          .saturating_mul(1u32 << (attempt - 1).min(16))
          .min(PULL_RETRY_MAX);
        tokio::time::sleep(delay).await;
      }

      match crate::sync::pull_profile_after_remote_session(&app_handle, &profile_id).await {
        Ok(outcome) if outcome.is_completed() => {
          log::info!("Pulled remote session work for profile {profile_id}");
          clear(&profile_id);
          return;
        }
        Ok(crate::sync::ProfileSyncOutcome::Skipped(reason)) => {
          log::info!("Post-session pull for profile {profile_id} did nothing ({reason}); retrying");
        }
        Ok(_) => unreachable!("is_completed covers every completed outcome"),
        Err(e) => {
          log::warn!("Post-session pull for profile {profile_id} failed: {e}");
        }
      }
    }

    log::warn!(
      "Could not pull remote session work for profile {profile_id} yet; it stays blocked from \
       launching locally until the pull succeeds"
    );
  });
}

/// Serialises every test that can reach [`STORE`], wherever it lives.
///
/// `remote_session`'s tests drive session transitions through `note_running`
/// and `note_ended`, so they mutate this module's global store too — with the
/// same `p1`/`p2` fixture ids. Two mutexes meant the two groups could interleave
/// and clobber each other, which showed up as an intermittent failure in the
/// suite guarding a data-loss bug.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the store lock and start from an empty store. Callers must hold the
/// returned guard for the whole test.
#[cfg(test)]
pub(crate) fn lock_for_test() -> std::sync::MutexGuard<'static, ()> {
  let lock = TEST_LOCK
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  *STORE
    .write()
    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Store::new());
  lock
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashSet;

  /// Point the store at a scratch directory and start it empty.
  ///
  /// Everything returned must outlive the test body: dropping the guard
  /// restores the real data directory, and a test that let it drop early would
  /// write a gate file into the developer's own app data. `TEST_DATA_DIR` is
  /// thread-local but [`STORE`] is process-global, so [`lock_for_test`] is what
  /// keeps two tests from sharing one store while pointing at different
  /// directories.
  fn isolated() -> (
    tempfile::TempDir,
    crate::app_dirs::TestDirGuard,
    std::sync::MutexGuard<'static, ()>,
  ) {
    let lock = lock_for_test();
    let dir = tempfile::TempDir::new().expect("a scratch directory");
    let guard = crate::app_dirs::set_test_data_dir(dir.path().to_path_buf());
    // Re-taken after the data dir is redirected, so nothing loads from the
    // real one.
    *STORE
      .write()
      .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Store::new());
    (dir, guard, lock)
  }

  #[test]
  fn a_live_session_blocks_a_local_launch() {
    let _iso = isolated();
    note_running("p1", "s1");
    let err = ensure_local_launch_allowed("p1").expect_err("a live session must block a launch");
    assert!(err.contains("PROFILE_RUNNING_REMOTELY"));
  }

  #[test]
  fn a_finished_session_still_blocks_until_the_work_is_pulled() {
    // The whole point. The profile lock is released the moment the session
    // closes, so without this the user can open the stale copy and the next
    // sync deletes everything the host wrote.
    let _iso = isolated();
    note_running("p1", "s1");
    assert!(note_ended("p1", "s1"));
    let err = ensure_local_launch_allowed("p1").expect_err("pending work must block a launch");
    assert!(err.contains("PROFILE_REMOTE_SYNC_PENDING"));

    clear("p1");
    assert!(ensure_local_launch_allowed("p1").is_ok());
  }

  #[test]
  fn an_ungated_profile_is_not_blocked() {
    let _iso = isolated();
    note_running("p1", "s1");
    assert!(ensure_local_launch_allowed("p2").is_ok());
  }

  #[test]
  fn the_end_transition_is_reported_once_however_often_the_frame_arrives() {
    // The stream re-delivers a snapshot on every reconnect, and `closed` can
    // arrive alongside it. Starting a pull per frame would run several
    // concurrent transfers of the same profile.
    let _iso = isolated();
    note_running("p1", "s1");
    assert!(note_ended("p1", "s1"));
    assert!(!note_ended("p1", "s1"));
    assert!(!note_ended("p1", "s1"));
  }

  #[test]
  fn a_closed_session_this_machine_never_watched_does_not_gate_anything() {
    // `listForUser` returns closed sessions next to live ones, so the snapshot
    // on every reconnect replays every session that ever finished. Treating
    // those as fresh handoffs would block the Run button on a perfectly current
    // profile at each app start, and block it indefinitely while offline.
    let _iso = isolated();
    assert!(!note_ended("p1", "s-finished-last-week"));
    assert_eq!(state_for("p1"), None);
    assert!(ensure_local_launch_allowed("p1").is_ok());
  }

  #[test]
  fn a_pulled_profile_is_not_re_gated_by_a_replayed_close() {
    // Same frame, one step later: the pull completed and cleared the gate. The
    // next reconnect must not put it back.
    let _iso = isolated();
    note_running("p1", "s1");
    note_ended("p1", "s1");
    clear("p1");
    assert!(!note_ended("p1", "s1"));
    assert!(ensure_local_launch_allowed("p1").is_ok());
  }

  #[test]
  fn a_late_close_for_a_replaced_session_does_not_gate_the_new_one() {
    // Session s1 finished and was pulled; s2 is now live on the same profile. A
    // straggling `closed` for s1 must not declare s2's profile finished, or the
    // gate lifts while a host is still writing.
    let _iso = isolated();
    note_running("p1", "s2");
    assert!(!note_ended("p1", "s1"));
    assert_eq!(state_for("p1"), Some(HandoffState::Running));
  }

  #[test]
  fn a_session_that_ended_while_the_app_was_shut_is_recovered() {
    // Nothing streams a transition to a process that is not running. Without
    // this the profile reads as still-running for ever and can never be
    // launched again, and its work is never pulled.
    let _iso = isolated();
    note_running("p1", "s1");
    let live: HashSet<String> = HashSet::new();
    assert_eq!(reconcile(&live), vec!["p1".to_string()]);
    assert_eq!(state_for("p1"), Some(HandoffState::PendingSync));
  }

  #[test]
  fn reconcile_leaves_a_session_that_is_genuinely_still_live() {
    let _iso = isolated();
    note_running("p1", "s1");
    let live: HashSet<String> = ["s1".to_string()].into_iter().collect();
    assert!(reconcile(&live).is_empty());
    assert_eq!(state_for("p1"), Some(HandoffState::Running));
  }

  #[test]
  fn reconcile_does_not_reopen_a_pending_profile() {
    // `PendingSync` is not a session state and no listing will ever contain it.
    // Re-deriving it from the snapshot would report the same handoff as new on
    // every reconnect and start a pull each time.
    let _iso = isolated();
    note_running("p1", "s1");
    note_ended("p1", "s1");
    let live: HashSet<String> = HashSet::new();
    assert!(reconcile(&live).is_empty());
  }

  #[test]
  fn the_gate_survives_a_restart() {
    // Held on disk precisely because the dangerous window outlives the process:
    // an app killed mid-session comes back with no memory of it.
    let (_dir, _guard, _lock) = isolated();
    note_running("p1", "s1");
    note_ended("p1", "s1");

    *STORE
      .write()
      .unwrap_or_else(std::sync::PoisonError::into_inner) = None;

    assert_eq!(state_for("p1"), Some(HandoffState::PendingSync));
  }
}

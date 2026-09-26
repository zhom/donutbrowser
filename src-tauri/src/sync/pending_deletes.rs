//! Cloud deletes of synced profiles that this device still owes.
//!
//! Deleting a synced profile removes it from the cloud in the background. A
//! delete that cannot reach the server (offline, server down) is recorded here
//! and retried by the next reconcile. Without the record, that reconcile finds
//! the profile still in the cloud and downloads it back.
//!
//! The running delete is kept as well, so a restore of the same profile can
//! wait for it. The server writes the tombstone only after it has removed every
//! file, and a tombstone that lands after the restore turned sync back on
//! erases the restored profile.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

const FILE_NAME: &str = "pending_profile_deletes.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDelete {
  pub profile_id: String,
  /// Unix seconds. A cloud copy changed after this wins over the delete.
  pub deleted_at: u64,
}

static FILE_LOCK: Mutex<()> = Mutex::new(());

static RUNNING: LazyLock<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>> =
  LazyLock::new(Default::default);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
  mutex
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn file() -> PathBuf {
  crate::app_dirs::data_dir().join(FILE_NAME)
}

fn read() -> Vec<PendingDelete> {
  let Ok(content) = std::fs::read_to_string(file()) else {
    return Vec::new();
  };
  serde_json::from_str(&content).unwrap_or_else(|e| {
    log::warn!("Ignoring unreadable {FILE_NAME}: {e}");
    Vec::new()
  })
}

fn write(entries: &[PendingDelete]) {
  let path = file();
  if entries.is_empty() {
    if let Err(e) = std::fs::remove_file(&path) {
      if e.kind() != std::io::ErrorKind::NotFound {
        log::warn!("Could not remove {}: {e}", path.display());
      }
    }
    return;
  }
  let result = serde_json::to_vec_pretty(entries)
    .map_err(std::io::Error::other)
    .and_then(|json| {
      if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
      }
      let tmp = path.with_extension("json.tmp");
      std::fs::write(&tmp, json)?;
      std::fs::rename(&tmp, &path)
    });
  if let Err(e) = result {
    log::warn!("Could not write {}: {e}", path.display());
  }
}

pub fn list() -> Vec<PendingDelete> {
  let _guard = lock(&FILE_LOCK);
  read()
}

/// Owe the cloud a delete of `profile_id`, replacing an older record.
pub fn record(profile_id: &str, deleted_at: u64) {
  let _guard = lock(&FILE_LOCK);
  let mut entries = read();
  entries.retain(|entry| entry.profile_id != profile_id);
  entries.push(PendingDelete {
    profile_id: profile_id.to_string(),
    deleted_at,
  });
  write(&entries);
}

/// The delete landed, or it no longer applies.
pub fn forget(profile_id: &str) {
  let _guard = lock(&FILE_LOCK);
  let mut entries = read();
  let before = entries.len();
  entries.retain(|entry| entry.profile_id != profile_id);
  if entries.len() != before {
    write(&entries);
  }
}

/// Keep the task running the cloud delete of `profile_id`.
pub fn track(profile_id: &str, handle: tauri::async_runtime::JoinHandle<()>) {
  let mut running = lock(&RUNNING);
  running.retain(|_, task| !task.inner().is_finished());
  running.insert(profile_id.to_string(), handle);
}

/// Wait for a cloud delete of `profile_id` started in this session to end.
pub async fn wait_for(profile_id: &str) {
  let handle = lock(&RUNNING).remove(profile_id);
  if let Some(handle) = handle {
    if let Err(e) = handle.await {
      log::warn!("The cloud delete of profile {profile_id} ended abnormally: {e}");
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn records_replace_and_forget_by_profile() {
    let dir = tempfile::TempDir::new().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(dir.path().to_path_buf());

    assert!(list().is_empty());
    record("a", 10);
    record("b", 20);
    record("a", 30);
    assert_eq!(
      list(),
      vec![
        PendingDelete {
          profile_id: "b".to_string(),
          deleted_at: 20
        },
        PendingDelete {
          profile_id: "a".to_string(),
          deleted_at: 30
        },
      ]
    );

    forget("missing");
    forget("b");
    assert_eq!(list().len(), 1);
    forget("a");
    assert!(list().is_empty());
    assert!(!file().exists(), "an empty record leaves no file behind");
  }

  #[tokio::test]
  async fn wait_for_returns_once_the_delete_has_ended() {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = done.clone();
    let handle = tauri::async_runtime::spawn(async move {
      let _ = rx.await;
      flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    track("waited-profile", handle);
    tx.send(()).unwrap();
    wait_for("waited-profile").await;
    assert!(done.load(std::sync::atomic::Ordering::SeqCst));
    // A second wait finds nothing and returns at once.
    wait_for("waited-profile").await;
  }
}

//! Small config entities that sync whole, with no per-item switch: profile
//! statuses and profile templates.
//!
//! Each kind is one JSON file on this device and one object per entity in the
//! cloud at `<folder>/<id>.json`. Conflicts resolve on `updated_at`, last write
//! wins, as for every other config entity. A delete leaves the tombstone
//! `tombstones/<folder>/<id>.json`, and a local copy older than its tombstone
//! is removed rather than uploaded again.
//!
//! They sync whenever sync is set up: they hold no secrets, they are what
//! profiles refer to by id, and a team shares them through the owner's space.

use super::engine::SyncEngine;
use super::types::{SyncError, SyncResult};
use crate::events;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigKind {
  Status,
  Template,
}

impl ConfigKind {
  pub const ALL: [ConfigKind; 2] = [ConfigKind::Status, ConfigKind::Template];

  /// The cloud key folder, also the tombstone folder.
  pub fn folder(self) -> &'static str {
    match self {
      ConfigKind::Status => "statuses",
      ConfigKind::Template => "templates",
    }
  }

  /// The entity type a live tombstone event carries.
  pub fn tombstone_type(self) -> &'static str {
    match self {
      ConfigKind::Status => "status",
      ConfigKind::Template => "template",
    }
  }

  pub fn from_tombstone_type(entity_type: &str) -> Option<Self> {
    Self::ALL
      .into_iter()
      .find(|kind| kind.tombstone_type() == entity_type)
  }

  pub fn from_folder(folder: &str) -> Option<Self> {
    Self::ALL.into_iter().find(|kind| kind.folder() == folder)
  }

  /// Emitted whenever the local set of this kind changes.
  pub fn changed_event(self) -> &'static str {
    match self {
      ConfigKind::Status => "profile-statuses-changed",
      ConfigKind::Template => "profile-templates-changed",
    }
  }
}

pub trait ConfigEntity: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
  const KIND: ConfigKind;
  fn id(&self) -> &str;
  /// Unix seconds of the last user edit.
  fn updated_at(&self) -> u64;
  fn store() -> &'static ConfigStore<Self>;
}

#[derive(Serialize, Deserialize)]
#[serde(bound = "T: Serialize + DeserializeOwned")]
struct StoreFile<T> {
  #[serde(default = "Vec::new")]
  items: Vec<T>,
}

/// Every entity of one kind, in one JSON file in the data directory.
pub struct ConfigStore<T> {
  file_name: &'static str,
  lock: Mutex<()>,
  _kind: PhantomData<fn() -> T>,
}

impl<T: ConfigEntity> ConfigStore<T> {
  pub const fn new(file_name: &'static str) -> Self {
    Self {
      file_name,
      lock: Mutex::new(()),
      _kind: PhantomData,
    }
  }

  fn path(&self) -> std::path::PathBuf {
    crate::app_dirs::data_subdir().join(self.file_name)
  }

  fn read(&self) -> Result<Vec<T>, String> {
    let path = self.path();
    match std::fs::read_to_string(&path) {
      Ok(content) => serde_json::from_str::<StoreFile<T>>(&content)
        .map(|file| file.items)
        .map_err(|e| format!("{} is unreadable: {e}", self.file_name)),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
      Err(e) => Err(format!("{} is unreadable: {e}", self.file_name)),
    }
  }

  fn write(&self, items: Vec<T>) -> Result<(), String> {
    let path = self.path();
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&StoreFile { items }).map_err(|e| e.to_string())?;
    crate::app_dirs::write_owner_only(&path, json.as_bytes()).map_err(|e| e.to_string())
  }

  pub fn list(&self) -> Vec<T> {
    let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
    self.read().unwrap_or_else(|e| {
      log::warn!("Config store read failed err=\"{e}\"");
      Vec::new()
    })
  }

  pub fn get(&self, id: &str) -> Option<T> {
    self.list().into_iter().find(|item| item.id() == id)
  }

  /// Read, change and write back under one lock, so two edits cannot lose
  /// each other.
  pub fn mutate<R>(
    &self,
    change: impl FnOnce(&mut Vec<T>) -> Result<R, String>,
  ) -> Result<R, String> {
    let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut items = self.read()?;
    let result = change(&mut items)?;
    self.write(items)?;
    Ok(result)
  }

  fn put(&self, item: T) -> Result<(), String> {
    self.mutate(|items| {
      match items.iter_mut().find(|existing| existing.id() == item.id()) {
        Some(existing) => *existing = item,
        None => items.push(item),
      }
      Ok(())
    })
  }

  fn remove(&self, id: &str) -> Result<bool, String> {
    self.mutate(|items| {
      let before = items.len();
      items.retain(|item| item.id() != id);
      Ok(items.len() != before)
    })
  }
}

fn notify(kind: ConfigKind) {
  if let Err(e) = events::emit_empty(kind.changed_event()) {
    log::warn!(
      "Event emit failed event={} err=\"{e}\"",
      kind.changed_event()
    );
  }
}

/// After a local create or edit: tell the UI, and queue the upload.
pub fn changed_locally<T: ConfigEntity>(id: &str) {
  notify(T::KIND);
  if !crate::sync::is_sync_configured() {
    return;
  }
  if let Some(scheduler) = crate::sync::get_global_scheduler() {
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
      scheduler.queue_config_sync(T::KIND, id).await;
    });
  }
}

/// After a local delete: tell the UI, and queue the remote delete.
pub fn deleted_locally<T: ConfigEntity>(id: &str) {
  notify(T::KIND);
  if !crate::sync::is_sync_configured() {
    return;
  }
  if let Some(scheduler) = crate::sync::get_global_scheduler() {
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
      scheduler.queue_config_delete(T::KIND, id).await;
    });
  }
}

/// Every local entity id of every kind, for the upload pass at sync start.
pub fn all_local_ids() -> Vec<(ConfigKind, String)> {
  fn ids<T: ConfigEntity>() -> Vec<(ConfigKind, String)> {
    T::store()
      .list()
      .iter()
      .map(|item| (T::KIND, item.id().to_string()))
      .collect()
  }
  let mut all = ids::<crate::profile_statuses::ProfileStatus>();
  all.extend(ids::<crate::profile_templates::ProfileTemplate>());
  all
}

/// Apply a deletion another device made.
pub fn apply_remote_delete(kind: ConfigKind, id: &str) {
  fn remove<T: ConfigEntity>(id: &str) {
    match T::store().remove(id) {
      Ok(true) => {
        log::info!(
          "{} deleted remotely, removed locally id={id}",
          T::KIND.tombstone_type()
        );
        notify(T::KIND);
      }
      Ok(false) => {}
      Err(e) => log::warn!(
        "Remote {} delete not applied id={id} err=\"{e}\"",
        T::KIND.tombstone_type()
      ),
    }
  }
  match kind {
    ConfigKind::Status => remove::<crate::profile_statuses::ProfileStatus>(id),
    ConfigKind::Template => remove::<crate::profile_templates::ProfileTemplate>(id),
  }
}

pub async fn sync_one(engine: &SyncEngine, kind: ConfigKind, id: &str) -> SyncResult<()> {
  match kind {
    ConfigKind::Status => {
      engine
        .sync_config::<crate::profile_statuses::ProfileStatus>(id)
        .await
    }
    ConfigKind::Template => {
      engine
        .sync_config::<crate::profile_templates::ProfileTemplate>(id)
        .await
    }
  }
}

pub async fn fetch_missing(engine: &SyncEngine) -> SyncResult<()> {
  engine
    .fetch_missing_configs::<crate::profile_statuses::ProfileStatus>()
    .await?;
  engine
    .fetch_missing_configs::<crate::profile_templates::ProfileTemplate>()
    .await
}

impl SyncEngine {
  pub async fn sync_config<T: ConfigEntity>(&self, id: &str) -> SyncResult<()> {
    let kind = T::KIND;
    let local = T::store().get(id);
    let local_updated_at = local.as_ref().map_or(0, T::updated_at);
    if self
      .tombstone_blocks(kind.folder(), id, local_updated_at)
      .await?
    {
      // Deleted elsewhere after the last edit here. Uploading it again would
      // undo that delete on every device.
      if local.is_some() {
        apply_remote_delete(kind, id);
      }
      return Ok(());
    }

    let remote_key = format!("{}/{id}.json", kind.folder());
    let stat = self.client.stat(&remote_key).await?;
    match (local, stat.exists) {
      (Some(item), true) => {
        let remote_updated_at = self.remote_updated_at(&stat, &remote_key).await;
        if self.remote_wins(local_updated_at, remote_updated_at) {
          self.download_config::<T>(id).await?;
        } else if local_updated_at > remote_updated_at {
          self.upload_config(&item).await?;
        }
      }
      (Some(item), false) => self.upload_config(&item).await?,
      (None, true) => self.download_config::<T>(id).await?,
      (None, false) => {}
    }
    Ok(())
  }

  async fn upload_config<T: ConfigEntity>(&self, item: &T) -> SyncResult<()> {
    let json = serde_json::to_string_pretty(item)
      .map_err(|e| SyncError::SerializationError(e.to_string()))?;
    let remote_key = format!("{}/{}.json", T::KIND.folder(), item.id());
    self
      .upload_config_json(&remote_key, &json, item.updated_at())
      .await?;
    log::info!("{} uploaded id={}", T::KIND.tombstone_type(), item.id());
    Ok(())
  }

  async fn download_config<T: ConfigEntity>(&self, id: &str) -> SyncResult<()> {
    let remote_key = format!("{}/{id}.json", T::KIND.folder());
    let presign = self.client.presign_download(&remote_key).await?;
    let raw = self.client.download_bytes(&presign.url).await?;
    let data = super::encryption::maybe_unseal_after_download(&raw)
      .map_err(|e| SyncError::InvalidData(format!("Failed to unseal {remote_key}: {e}")))?;
    let item: T = serde_json::from_slice(&data)
      .map_err(|e| SyncError::SerializationError(format!("Failed to parse {remote_key}: {e}")))?;
    // The object's own id must be the one its key names, or a crafted object
    // could overwrite a different local entity.
    if item.id() != id {
      return Err(SyncError::InvalidData(format!(
        "{remote_key} holds a different id"
      )));
    }
    T::store().put(item).map_err(SyncError::InvalidData)?;
    notify(T::KIND);
    log::info!("{} downloaded id={id}", T::KIND.tombstone_type());
    Ok(())
  }

  pub async fn delete_config(&self, kind: ConfigKind, id: &str) -> SyncResult<()> {
    let folder = kind.folder();
    if self.skips_remote_write(&format!("key={folder}/{id}.json")) {
      return Ok(());
    }
    self
      .client
      .delete(
        &format!("{folder}/{id}.json"),
        Some(&format!("tombstones/{folder}/{id}.json")),
      )
      .await?;
    log::info!("{} deleted from cloud id={id}", kind.tombstone_type());
    Ok(())
  }

  /// Download every entity of this kind the cloud has and this device lacks.
  pub async fn fetch_missing_configs<T: ConfigEntity>(&self) -> SyncResult<()> {
    let folder = T::KIND.folder();
    let prefix = format!("{folder}/");
    let local: std::collections::HashSet<String> = T::store()
      .list()
      .iter()
      .map(|item| item.id().to_string())
      .collect();
    let remote = self.client.list(&prefix).await?;
    for object in &remote.objects {
      let Some(id) = object
        .key
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(".json"))
        .filter(|id| crate::app_dirs::is_plain_path_part(id))
      else {
        continue;
      };
      if local.contains(id) || self.tombstone_blocks(folder, id, 0).await.unwrap_or(true) {
        continue;
      }
      if let Err(e) = self.download_config::<T>(id).await {
        log::warn!(
          "Missing {} download failed id={id} err=\"{e}\"",
          T::KIND.tombstone_type()
        );
      }
    }
    Ok(())
  }
}

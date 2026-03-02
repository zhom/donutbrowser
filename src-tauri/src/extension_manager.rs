use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::events;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extension {
  pub id: String,
  pub name: String,
  pub file_name: String,
  pub file_type: String,
  pub browser_compatibility: Vec<String>,
  pub created_at: u64,
  pub updated_at: u64,
  #[serde(default)]
  pub sync_enabled: bool,
  #[serde(default)]
  pub last_sync: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionGroup {
  pub id: String,
  pub name: String,
  pub extension_ids: Vec<String>,
  pub created_at: u64,
  pub updated_at: u64,
  #[serde(default)]
  pub sync_enabled: bool,
  #[serde(default)]
  pub last_sync: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExtensionGroupsData {
  groups: Vec<ExtensionGroup>,
}

fn now_secs() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

fn extensions_base_dir() -> PathBuf {
  crate::app_dirs::extensions_dir()
}

fn extension_groups_file() -> PathBuf {
  crate::app_dirs::data_subdir().join("extension_groups.json")
}

fn determine_browser_compatibility(file_type: &str) -> Vec<String> {
  match file_type {
    "xpi" => vec!["firefox".to_string()],
    "crx" => vec!["chromium".to_string()],
    "zip" => vec!["chromium".to_string(), "firefox".to_string()],
    _ => vec![],
  }
}

fn get_file_type(file_name: &str) -> Option<String> {
  let ext = file_name.rsplit('.').next()?.to_lowercase();
  match ext.as_str() {
    "xpi" | "crx" | "zip" => Some(ext),
    _ => None,
  }
}

pub struct ExtensionManager;

impl ExtensionManager {
  pub fn new() -> Self {
    Self
  }

  fn get_extension_dir(&self, ext_id: &str) -> PathBuf {
    extensions_base_dir().join(ext_id)
  }

  fn get_metadata_path(&self, ext_id: &str) -> PathBuf {
    self.get_extension_dir(ext_id).join("metadata.json")
  }

  fn get_file_dir(&self, ext_id: &str) -> PathBuf {
    self.get_extension_dir(ext_id).join("file")
  }

  pub fn get_file_dir_public(&self, ext_id: &str) -> PathBuf {
    self.get_file_dir(ext_id)
  }

  // Extension CRUD

  pub fn add_extension(
    &self,
    name: String,
    file_name: String,
    file_data: Vec<u8>,
  ) -> Result<Extension, Box<dyn std::error::Error>> {
    let file_type =
      get_file_type(&file_name).ok_or_else(|| format!("Unsupported file type: {file_name}"))?;

    let browser_compatibility = determine_browser_compatibility(&file_type);
    let now = now_secs();

    let ext = Extension {
      id: uuid::Uuid::new_v4().to_string(),
      name,
      file_name: file_name.clone(),
      file_type,
      browser_compatibility,
      created_at: now,
      updated_at: now,
      sync_enabled: crate::sync::is_sync_configured(),
      last_sync: None,
    };

    let file_dir = self.get_file_dir(&ext.id);
    fs::create_dir_all(&file_dir)?;
    fs::write(file_dir.join(&file_name), &file_data)?;

    let metadata_path = self.get_metadata_path(&ext.id);
    let json = serde_json::to_string_pretty(&ext)?;
    fs::write(metadata_path, json)?;

    if let Err(e) = events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }

    if ext.sync_enabled {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        let id = ext.id.clone();
        tauri::async_runtime::spawn(async move {
          scheduler.queue_extension_sync(id).await;
        });
      }
    }

    Ok(ext)
  }

  pub fn get_extension(&self, id: &str) -> Result<Extension, Box<dyn std::error::Error>> {
    let metadata_path = self.get_metadata_path(id);
    if !metadata_path.exists() {
      return Err(format!("Extension with id '{id}' not found").into());
    }
    let content = fs::read_to_string(metadata_path)?;
    let ext: Extension = serde_json::from_str(&content)?;
    Ok(ext)
  }

  pub fn list_extensions(&self) -> Result<Vec<Extension>, Box<dyn std::error::Error>> {
    let base = extensions_base_dir();
    if !base.exists() {
      return Ok(Vec::new());
    }

    let mut extensions = Vec::new();
    for entry in fs::read_dir(base)? {
      let entry = entry?;
      if entry.file_type()?.is_dir() {
        let metadata_path = entry.path().join("metadata.json");
        if metadata_path.exists() {
          let content = fs::read_to_string(&metadata_path)?;
          if let Ok(ext) = serde_json::from_str::<Extension>(&content) {
            extensions.push(ext);
          }
        }
      }
    }

    extensions.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(extensions)
  }

  pub fn update_extension(
    &self,
    id: &str,
    name: Option<String>,
    file_name: Option<String>,
    file_data: Option<Vec<u8>>,
  ) -> Result<Extension, Box<dyn std::error::Error>> {
    let mut ext = self.get_extension(id)?;

    if let Some(new_name) = name {
      ext.name = new_name;
    }

    if let (Some(new_file_name), Some(data)) = (file_name, file_data) {
      let new_file_type = get_file_type(&new_file_name)
        .ok_or_else(|| format!("Unsupported file type: {new_file_name}"))?;

      // Remove old file
      let file_dir = self.get_file_dir(id);
      if file_dir.exists() {
        fs::remove_dir_all(&file_dir)?;
      }
      fs::create_dir_all(&file_dir)?;
      fs::write(file_dir.join(&new_file_name), &data)?;

      ext.file_name = new_file_name;
      ext.file_type = new_file_type.clone();
      ext.browser_compatibility = determine_browser_compatibility(&new_file_type);
    }

    ext.updated_at = now_secs();

    let metadata_path = self.get_metadata_path(id);
    let json = serde_json::to_string_pretty(&ext)?;
    fs::write(metadata_path, json)?;

    if let Err(e) = events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }

    if ext.sync_enabled {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        let eid = ext.id.clone();
        tauri::async_runtime::spawn(async move {
          scheduler.queue_extension_sync(eid).await;
        });
      }
    }

    Ok(ext)
  }

  pub fn delete_extension(
    &self,
    app_handle: &tauri::AppHandle,
    id: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let ext = self.get_extension(id)?;
    let ext_dir = self.get_extension_dir(id);
    if ext_dir.exists() {
      fs::remove_dir_all(&ext_dir)?;
    }

    // Remove from all groups
    let mut groups_data = self.load_groups_data()?;
    for group in &mut groups_data.groups {
      group.extension_ids.retain(|eid| eid != id);
    }
    self.save_groups_data(&groups_data)?;

    if let Err(e) = events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }

    if ext.sync_enabled {
      let ext_id = id.to_string();
      let app_handle_clone = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        match crate::sync::SyncEngine::create_from_settings(&app_handle_clone).await {
          Ok(engine) => {
            if let Err(e) = engine.delete_extension(&ext_id).await {
              log::warn!("Failed to delete extension {} from sync: {}", ext_id, e);
            }
          }
          Err(e) => {
            log::debug!("Sync not configured, skipping remote deletion: {}", e);
          }
        }
      });
    }

    Ok(())
  }

  // Extension Group CRUD

  fn load_groups_data(&self) -> Result<ExtensionGroupsData, Box<dyn std::error::Error>> {
    let path = extension_groups_file();
    if !path.exists() {
      return Ok(ExtensionGroupsData { groups: Vec::new() });
    }
    let content = fs::read_to_string(path)?;
    let data: ExtensionGroupsData = serde_json::from_str(&content)?;
    Ok(data)
  }

  fn save_groups_data(&self, data: &ExtensionGroupsData) -> Result<(), Box<dyn std::error::Error>> {
    let path = extension_groups_file();
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(data)?;
    fs::write(path, json)?;
    Ok(())
  }

  pub fn create_group(&self, name: String) -> Result<ExtensionGroup, Box<dyn std::error::Error>> {
    let mut data = self.load_groups_data()?;

    if data.groups.iter().any(|g| g.name == name) {
      return Err(format!("Extension group with name '{name}' already exists").into());
    }

    let now = now_secs();
    let group = ExtensionGroup {
      id: uuid::Uuid::new_v4().to_string(),
      name,
      extension_ids: Vec::new(),
      created_at: now,
      updated_at: now,
      sync_enabled: crate::sync::is_sync_configured(),
      last_sync: None,
    };

    data.groups.push(group.clone());
    self.save_groups_data(&data)?;

    if let Err(e) = events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }

    if group.sync_enabled {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        let id = group.id.clone();
        tauri::async_runtime::spawn(async move {
          scheduler.queue_extension_group_sync(id).await;
        });
      }
    }

    Ok(group)
  }

  pub fn get_group(&self, id: &str) -> Result<ExtensionGroup, Box<dyn std::error::Error>> {
    let data = self.load_groups_data()?;
    data
      .groups
      .into_iter()
      .find(|g| g.id == id)
      .ok_or_else(|| format!("Extension group with id '{id}' not found").into())
  }

  pub fn list_groups(&self) -> Result<Vec<ExtensionGroup>, Box<dyn std::error::Error>> {
    let data = self.load_groups_data()?;
    Ok(data.groups)
  }

  pub fn update_group(
    &self,
    id: &str,
    name: Option<String>,
    extension_ids: Option<Vec<String>>,
  ) -> Result<ExtensionGroup, Box<dyn std::error::Error>> {
    let mut data = self.load_groups_data()?;

    if let Some(ref new_name) = name {
      if data
        .groups
        .iter()
        .any(|g| g.name == *new_name && g.id != id)
      {
        return Err(format!("Extension group with name '{new_name}' already exists").into());
      }
    }

    let group = data
      .groups
      .iter_mut()
      .find(|g| g.id == id)
      .ok_or_else(|| format!("Extension group with id '{id}' not found"))?;

    if let Some(new_name) = name {
      group.name = new_name;
    }
    if let Some(new_ids) = extension_ids {
      group.extension_ids = new_ids;
    }
    group.updated_at = now_secs();

    let updated = group.clone();
    self.save_groups_data(&data)?;

    if let Err(e) = events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }

    if updated.sync_enabled {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        let gid = updated.id.clone();
        tauri::async_runtime::spawn(async move {
          scheduler.queue_extension_group_sync(gid).await;
        });
      }
    }

    Ok(updated)
  }

  pub fn delete_group(
    &self,
    app_handle: &tauri::AppHandle,
    id: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let mut data = self.load_groups_data()?;

    let was_sync_enabled = data
      .groups
      .iter()
      .find(|g| g.id == id)
      .map(|g| g.sync_enabled)
      .unwrap_or(false);

    let initial_len = data.groups.len();
    data.groups.retain(|g| g.id != id);
    if data.groups.len() == initial_len {
      return Err(format!("Extension group with id '{id}' not found").into());
    }
    self.save_groups_data(&data)?;

    // Clear extension_group_id from profiles that used this group
    let profile_manager = crate::profile::ProfileManager::instance();
    if let Ok(profiles) = profile_manager.list_profiles() {
      for mut p in profiles {
        if p.extension_group_id.as_deref() == Some(id) {
          p.extension_group_id = None;
          let _ = profile_manager.save_profile(&p);
        }
      }
    }

    if was_sync_enabled {
      let group_id_owned = id.to_string();
      let app_handle_clone = app_handle.clone();
      tauri::async_runtime::spawn(async move {
        match crate::sync::SyncEngine::create_from_settings(&app_handle_clone).await {
          Ok(engine) => {
            if let Err(e) = engine.delete_extension_group(&group_id_owned).await {
              log::warn!(
                "Failed to delete extension group {} from sync: {}",
                group_id_owned,
                e
              );
            }
          }
          Err(e) => {
            log::debug!("Sync not configured, skipping remote deletion: {}", e);
          }
        }
      });
    }

    if let Err(e) = events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }

    Ok(())
  }

  pub fn add_extension_to_group(
    &self,
    group_id: &str,
    extension_id: &str,
  ) -> Result<ExtensionGroup, Box<dyn std::error::Error>> {
    // Verify extension exists
    let _ = self.get_extension(extension_id)?;

    let mut data = self.load_groups_data()?;
    let group = data
      .groups
      .iter_mut()
      .find(|g| g.id == group_id)
      .ok_or_else(|| format!("Extension group with id '{group_id}' not found"))?;

    if !group.extension_ids.contains(&extension_id.to_string()) {
      group.extension_ids.push(extension_id.to_string());
      group.updated_at = now_secs();
    }

    let updated = group.clone();
    self.save_groups_data(&data)?;

    if let Err(e) = events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }

    if updated.sync_enabled {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        let gid = updated.id.clone();
        tauri::async_runtime::spawn(async move {
          scheduler.queue_extension_group_sync(gid).await;
        });
      }
    }

    Ok(updated)
  }

  pub fn remove_extension_from_group(
    &self,
    group_id: &str,
    extension_id: &str,
  ) -> Result<ExtensionGroup, Box<dyn std::error::Error>> {
    let mut data = self.load_groups_data()?;
    let group = data
      .groups
      .iter_mut()
      .find(|g| g.id == group_id)
      .ok_or_else(|| format!("Extension group with id '{group_id}' not found"))?;

    group.extension_ids.retain(|eid| eid != extension_id);
    group.updated_at = now_secs();

    let updated = group.clone();
    self.save_groups_data(&data)?;

    if let Err(e) = events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }

    if updated.sync_enabled {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        let gid = updated.id.clone();
        tauri::async_runtime::spawn(async move {
          scheduler.queue_extension_group_sync(gid).await;
        });
      }
    }

    Ok(updated)
  }

  // Sync helpers

  pub fn update_extension_internal(
    &self,
    ext: &Extension,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let metadata_path = self.get_metadata_path(&ext.id);
    if let Some(parent) = metadata_path.parent() {
      fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(ext)?;
    fs::write(metadata_path, json)?;
    Ok(())
  }

  pub fn upsert_extension_internal(
    &self,
    ext: &Extension,
  ) -> Result<(), Box<dyn std::error::Error>> {
    self.update_extension_internal(ext)
  }

  pub fn delete_extension_internal(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ext_dir = self.get_extension_dir(id);
    if ext_dir.exists() {
      fs::remove_dir_all(&ext_dir)?;
    }
    // Remove from all groups
    let mut groups_data = self.load_groups_data()?;
    for group in &mut groups_data.groups {
      group.extension_ids.retain(|eid| eid != id);
    }
    self.save_groups_data(&groups_data)?;
    Ok(())
  }

  pub fn update_group_internal(
    &self,
    group: &ExtensionGroup,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let mut data = self.load_groups_data()?;
    if let Some(existing) = data.groups.iter_mut().find(|g| g.id == group.id) {
      existing.name = group.name.clone();
      existing.extension_ids = group.extension_ids.clone();
      existing.sync_enabled = group.sync_enabled;
      existing.last_sync = group.last_sync;
      existing.updated_at = group.updated_at;
      self.save_groups_data(&data)?;
    }
    Ok(())
  }

  pub fn upsert_group_internal(
    &self,
    group: &ExtensionGroup,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let mut data = self.load_groups_data()?;
    if let Some(existing) = data.groups.iter_mut().find(|g| g.id == group.id) {
      existing.name = group.name.clone();
      existing.extension_ids = group.extension_ids.clone();
      existing.sync_enabled = group.sync_enabled;
      existing.last_sync = group.last_sync;
      existing.updated_at = group.updated_at;
    } else {
      data.groups.push(group.clone());
    }
    self.save_groups_data(&data)?;
    Ok(())
  }

  pub fn delete_group_internal(&self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut data = self.load_groups_data()?;
    data.groups.retain(|g| g.id != id);
    self.save_groups_data(&data)?;
    Ok(())
  }

  // Compatibility validation

  pub fn validate_group_compatibility(
    &self,
    group_id: &str,
    browser: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let group = self.get_group(group_id)?;
    let browser_type = match browser {
      "camoufox" | "firefox" | "firefox-developer" | "zen" => "firefox",
      "wayfern" | "chromium" | "brave" => "chromium",
      _ => return Err(format!("Extensions are not supported for browser '{browser}'").into()),
    };

    for ext_id in &group.extension_ids {
      let ext = self.get_extension(ext_id)?;
      if !ext
        .browser_compatibility
        .contains(&browser_type.to_string())
      {
        return Err(
          format!(
            "Extension '{}' ({}) is not compatible with {} browsers",
            ext.name, ext.file_type, browser_type
          )
          .into(),
        );
      }
    }

    Ok(())
  }

  // Launch-time installation

  pub fn install_extensions_for_profile(
    &self,
    profile: &crate::profile::BrowserProfile,
    profile_data_path: &std::path::Path,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let group_id = match &profile.extension_group_id {
      Some(id) => id,
      None => return Ok(Vec::new()),
    };

    let group = self.get_group(group_id)?;
    if group.extension_ids.is_empty() {
      return Ok(Vec::new());
    }

    let browser_type = match profile.browser.as_str() {
      "camoufox" | "firefox" | "firefox-developer" | "zen" => "firefox",
      "wayfern" | "chromium" | "brave" => "chromium",
      _ => return Ok(Vec::new()),
    };

    let mut extension_paths = Vec::new();

    match browser_type {
      "firefox" => {
        let extensions_dir = profile_data_path.join("extensions");
        // Clear existing extensions
        if extensions_dir.exists() {
          fs::remove_dir_all(&extensions_dir)?;
        }
        fs::create_dir_all(&extensions_dir)?;

        for ext_id in &group.extension_ids {
          if let Ok(ext) = self.get_extension(ext_id) {
            if !ext.browser_compatibility.contains(&"firefox".to_string()) {
              continue;
            }
            let src_file = self.get_file_dir(ext_id).join(&ext.file_name);
            if src_file.exists() {
              // Firefox expects .xpi files in extensions dir
              let dest_name = if ext.file_type == "zip" {
                format!(
                  "{}.xpi",
                  ext
                    .file_name
                    .rsplit('.')
                    .next_back()
                    .unwrap_or(&ext.file_name)
                )
              } else {
                ext.file_name.clone()
              };
              let dest = extensions_dir.join(&dest_name);
              fs::copy(&src_file, &dest)?;
              extension_paths.push(dest.to_string_lossy().to_string());
            }
          }
        }
      }
      "chromium" => {
        // For Chromium, unpack extensions and return paths for --load-extension
        let unpacked_base = extensions_base_dir().join("unpacked");
        if unpacked_base.exists() {
          fs::remove_dir_all(&unpacked_base)?;
        }
        fs::create_dir_all(&unpacked_base)?;

        for ext_id in &group.extension_ids {
          if let Ok(ext) = self.get_extension(ext_id) {
            if !ext.browser_compatibility.contains(&"chromium".to_string()) {
              continue;
            }
            let src_file = self.get_file_dir(ext_id).join(&ext.file_name);
            if src_file.exists() {
              let unpack_dir = unpacked_base.join(ext_id);
              fs::create_dir_all(&unpack_dir)?;

              // Extract .crx or .zip
              match Self::unpack_extension(&src_file, &unpack_dir) {
                Ok(()) => {
                  extension_paths.push(unpack_dir.to_string_lossy().to_string());
                }
                Err(e) => {
                  log::warn!("Failed to unpack extension '{}': {}", ext.name, e);
                }
              }
            }
          }
        }
      }
      _ => {}
    }

    Ok(extension_paths)
  }

  fn unpack_extension(
    src: &std::path::Path,
    dest: &std::path::Path,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let data = fs::read(src)?;
    let mut archive = match zip::ZipArchive::new(std::io::Cursor::new(data.as_slice())) {
      Ok(a) => a,
      Err(e) => {
        // CRX files have a header before the ZIP data — try skipping the CRX header
        if let Some(zip_start) = Self::find_zip_start(&data) {
          zip::ZipArchive::new(std::io::Cursor::new(&data[zip_start..]))
            .map_err(|e2| format!("Failed to open CRX as zip after header skip: {e2}"))?
        } else {
          return Err(format!("Failed to open as zip: {e}").into());
        }
      }
    };
    for i in 0..archive.len() {
      let mut file = archive.by_index(i)?;
      let out_path = dest.join(file.mangled_name());

      if file.is_dir() {
        fs::create_dir_all(&out_path)?;
      } else {
        if let Some(parent) = out_path.parent() {
          fs::create_dir_all(parent)?;
        }
        let mut out_file = fs::File::create(&out_path)?;
        std::io::copy(&mut file, &mut out_file)?;
      }
    }

    Ok(())
  }

  fn find_zip_start(data: &[u8]) -> Option<usize> {
    // ZIP local file header magic: PK\x03\x04
    let magic = [0x50, 0x4B, 0x03, 0x04];
    data.windows(4).position(|window| window == magic)
  }
}

// Global instance
lazy_static::lazy_static! {
  pub static ref EXTENSION_MANAGER: Mutex<ExtensionManager> = Mutex::new(ExtensionManager::new());
}

// Tauri commands

#[tauri::command]
pub async fn list_extensions() -> Result<Vec<Extension>, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .list_extensions()
    .map_err(|e| format!("Failed to list extensions: {e}"))
}

#[tauri::command]
pub async fn add_extension(
  name: String,
  file_name: String,
  file_data: Vec<u8>,
) -> Result<Extension, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .add_extension(name, file_name, file_data)
    .map_err(|e| format!("Failed to add extension: {e}"))
}

#[tauri::command]
pub async fn update_extension(
  extension_id: String,
  name: Option<String>,
  file_name: Option<String>,
  file_data: Option<Vec<u8>>,
) -> Result<Extension, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .update_extension(&extension_id, name, file_name, file_data)
    .map_err(|e| format!("Failed to update extension: {e}"))
}

#[tauri::command]
pub async fn delete_extension(
  app_handle: tauri::AppHandle,
  extension_id: String,
) -> Result<(), String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .delete_extension(&app_handle, &extension_id)
    .map_err(|e| format!("Failed to delete extension: {e}"))
}

#[tauri::command]
pub async fn list_extension_groups() -> Result<Vec<ExtensionGroup>, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .list_groups()
    .map_err(|e| format!("Failed to list extension groups: {e}"))
}

#[tauri::command]
pub async fn create_extension_group(name: String) -> Result<ExtensionGroup, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .create_group(name)
    .map_err(|e| format!("Failed to create extension group: {e}"))
}

#[tauri::command]
pub async fn update_extension_group(
  group_id: String,
  name: Option<String>,
  extension_ids: Option<Vec<String>>,
) -> Result<ExtensionGroup, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .update_group(&group_id, name, extension_ids)
    .map_err(|e| format!("Failed to update extension group: {e}"))
}

#[tauri::command]
pub async fn delete_extension_group(
  app_handle: tauri::AppHandle,
  group_id: String,
) -> Result<(), String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .delete_group(&app_handle, &group_id)
    .map_err(|e| format!("Failed to delete extension group: {e}"))
}

#[tauri::command]
pub async fn add_extension_to_group(
  group_id: String,
  extension_id: String,
) -> Result<ExtensionGroup, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .add_extension_to_group(&group_id, &extension_id)
    .map_err(|e| format!("Failed to add extension to group: {e}"))
}

#[tauri::command]
pub async fn remove_extension_from_group(
  group_id: String,
  extension_id: String,
) -> Result<ExtensionGroup, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }
  let mgr = EXTENSION_MANAGER.lock().unwrap();
  mgr
    .remove_extension_from_group(&group_id, &extension_id)
    .map_err(|e| format!("Failed to remove extension from group: {e}"))
}

#[tauri::command]
pub async fn assign_extension_group_to_profile(
  profile_id: String,
  extension_group_id: Option<String>,
) -> Result<crate::profile::BrowserProfile, String> {
  if !crate::cloud_auth::CLOUD_AUTH
    .has_active_paid_subscription()
    .await
  {
    return Err("Extension management requires an active Pro subscription".to_string());
  }

  // Validate compatibility if assigning a group
  if let Some(ref group_id) = extension_group_id {
    let profile_manager = crate::profile::ProfileManager::instance();
    let profiles = profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| format!("Profile '{profile_id}' not found"))?;

    let mgr = EXTENSION_MANAGER.lock().unwrap();
    mgr
      .validate_group_compatibility(group_id, &profile.browser)
      .map_err(|e| format!("{e}"))?;
  }

  let profile_manager = crate::profile::ProfileManager::instance();
  profile_manager
    .update_profile_extension_group(&profile_id, extension_group_id)
    .map_err(|e| format!("Failed to assign extension group: {e}"))
}

#[tauri::command]
pub async fn get_extension_group_for_profile(
  profile_id: String,
) -> Result<Option<ExtensionGroup>, String> {
  let profile_manager = crate::profile::ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?;
  let profile = profiles
    .iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or_else(|| format!("Profile '{profile_id}' not found"))?;

  match &profile.extension_group_id {
    Some(group_id) => {
      let mgr = EXTENSION_MANAGER.lock().unwrap();
      match mgr.get_group(group_id) {
        Ok(group) => Ok(Some(group)),
        Err(_) => Ok(None),
      }
    }
    None => Ok(None),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_get_file_type() {
    assert_eq!(get_file_type("ublock.xpi"), Some("xpi".to_string()));
    assert_eq!(get_file_type("ext.crx"), Some("crx".to_string()));
    assert_eq!(get_file_type("ext.zip"), Some("zip".to_string()));
    assert_eq!(get_file_type("readme.txt"), None);
    assert_eq!(get_file_type("noext"), None);
  }

  #[test]
  fn test_determine_browser_compatibility() {
    assert_eq!(
      determine_browser_compatibility("xpi"),
      vec!["firefox".to_string()]
    );
    assert_eq!(
      determine_browser_compatibility("crx"),
      vec!["chromium".to_string()]
    );
    assert_eq!(
      determine_browser_compatibility("zip"),
      vec!["chromium".to_string(), "firefox".to_string()]
    );
  }

  #[test]
  fn test_extension_manager_crud() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());

    let mgr = ExtensionManager::new();

    // List empty
    let exts = mgr.list_extensions().unwrap();
    assert!(exts.is_empty());

    // Add
    let ext = mgr
      .add_extension(
        "Test Ext".to_string(),
        "test.xpi".to_string(),
        vec![0, 1, 2, 3],
      )
      .unwrap();
    assert_eq!(ext.name, "Test Ext");
    assert_eq!(ext.file_type, "xpi");
    assert_eq!(ext.browser_compatibility, vec!["firefox".to_string()]);

    // Get
    let fetched = mgr.get_extension(&ext.id).unwrap();
    assert_eq!(fetched.name, "Test Ext");

    // List
    let exts = mgr.list_extensions().unwrap();
    assert_eq!(exts.len(), 1);

    // Update name
    let updated = mgr
      .update_extension(&ext.id, Some("Updated".to_string()), None, None)
      .unwrap();
    assert_eq!(updated.name, "Updated");

    // Delete
    mgr.delete_extension_internal(&ext.id).unwrap();
    let exts = mgr.list_extensions().unwrap();
    assert!(exts.is_empty());
  }

  #[test]
  fn test_extension_group_crud() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());

    let mgr = ExtensionManager::new();

    // Create group
    let group = mgr.create_group("My Group".to_string()).unwrap();
    assert_eq!(group.name, "My Group");
    assert!(group.extension_ids.is_empty());

    // List groups
    let groups = mgr.list_groups().unwrap();
    assert_eq!(groups.len(), 1);

    // Add extension
    let ext = mgr
      .add_extension(
        "Test Ext".to_string(),
        "test.xpi".to_string(),
        vec![0, 1, 2, 3],
      )
      .unwrap();

    // Add to group
    let updated = mgr.add_extension_to_group(&group.id, &ext.id).unwrap();
    assert_eq!(updated.extension_ids.len(), 1);

    // Remove from group
    let updated = mgr.remove_extension_from_group(&group.id, &ext.id).unwrap();
    assert!(updated.extension_ids.is_empty());

    // Duplicate name check
    let err = mgr.create_group("My Group".to_string());
    assert!(err.is_err());
  }

  #[test]
  fn test_validate_group_compatibility() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());

    let mgr = ExtensionManager::new();

    let ext = mgr
      .add_extension(
        "Firefox Ext".to_string(),
        "test.xpi".to_string(),
        vec![0, 1, 2, 3],
      )
      .unwrap();

    let group = mgr.create_group("Firefox Group".to_string()).unwrap();
    mgr.add_extension_to_group(&group.id, &ext.id).unwrap();

    // Compatible with camoufox (firefox-based)
    assert!(mgr
      .validate_group_compatibility(&group.id, "camoufox")
      .is_ok());

    // Incompatible with wayfern (chromium-based)
    assert!(mgr
      .validate_group_compatibility(&group.id, "wayfern")
      .is_err());
  }

  #[test]
  fn test_find_zip_start() {
    let data = vec![0x00, 0x00, 0x50, 0x4B, 0x03, 0x04, 0xFF];
    assert_eq!(ExtensionManager::find_zip_start(&data), Some(2));

    let data = vec![0x50, 0x4B, 0x03, 0x04, 0xFF];
    assert_eq!(ExtensionManager::find_zip_start(&data), Some(0));

    let data = vec![0x00, 0x00, 0x00];
    assert_eq!(ExtensionManager::find_zip_start(&data), None);
  }

  #[test]
  fn test_delete_extension_removes_from_groups() {
    let tmp = tempfile::tempdir().unwrap();
    let _guard = crate::app_dirs::set_test_data_dir(tmp.path().to_path_buf());

    let mgr = ExtensionManager::new();

    let ext = mgr
      .add_extension("Test".to_string(), "test.xpi".to_string(), vec![0, 1, 2, 3])
      .unwrap();

    let group = mgr.create_group("G1".to_string()).unwrap();
    mgr.add_extension_to_group(&group.id, &ext.id).unwrap();

    // Delete extension should remove from group
    mgr.delete_extension_internal(&ext.id).unwrap();

    let updated_group = mgr.get_group(&group.id).unwrap();
    assert!(updated_group.extension_ids.is_empty());
  }
}

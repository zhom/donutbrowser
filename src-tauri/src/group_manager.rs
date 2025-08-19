use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileGroup {
  pub id: String,
  pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupWithCount {
  pub id: String,
  pub name: String,
  pub count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct GroupsData {
  groups: Vec<ProfileGroup>,
}

pub struct GroupManager {
  base_dirs: BaseDirs,
  data_dir_override: Option<PathBuf>,
}

impl GroupManager {
  pub fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      data_dir_override: std::env::var("DONUTBROWSER_DATA_DIR")
        .ok()
        .map(PathBuf::from),
    }
  }

  // Helper for tests to override data directory without global env var
  #[allow(dead_code)]
  pub fn with_data_dir_override(dir: &Path) -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      data_dir_override: Some(dir.to_path_buf()),
    }
  }

  fn get_groups_file_path(&self) -> PathBuf {
    if let Some(dir) = &self.data_dir_override {
      let mut override_path = dir.clone();
      // Ensure the directory exists before returning the path
      let _ = fs::create_dir_all(&override_path);
      override_path.push("groups.json");
      return override_path;
    }

    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("data");
    path.push("groups.json");
    path
  }

  fn load_groups_data(&self) -> Result<GroupsData, Box<dyn std::error::Error>> {
    let groups_file = self.get_groups_file_path();

    if !groups_file.exists() {
      return Ok(GroupsData { groups: Vec::new() });
    }

    let content = fs::read_to_string(groups_file)?;
    let groups_data: GroupsData = serde_json::from_str(&content)?;
    Ok(groups_data)
  }

  fn save_groups_data(&self, groups_data: &GroupsData) -> Result<(), Box<dyn std::error::Error>> {
    let groups_file = self.get_groups_file_path();

    // Ensure the parent directory exists
    if let Some(parent) = groups_file.parent() {
      fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(groups_data)?;
    fs::write(groups_file, json)?;
    Ok(())
  }

  pub fn get_all_groups(&self) -> Result<Vec<ProfileGroup>, Box<dyn std::error::Error>> {
    let groups_data = self.load_groups_data()?;
    Ok(groups_data.groups)
  }

  pub fn create_group(
    &self,
    app_handle: &tauri::AppHandle,
    name: String,
  ) -> Result<ProfileGroup, Box<dyn std::error::Error>> {
    let mut groups_data = self.load_groups_data()?;

    // Check if group with this name already exists
    if groups_data.groups.iter().any(|g| g.name == name) {
      return Err(format!("Group with name '{name}' already exists").into());
    }

    let group = ProfileGroup {
      id: uuid::Uuid::new_v4().to_string(),
      name,
    };

    groups_data.groups.push(group.clone());
    self.save_groups_data(&groups_data)?;

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("groups-changed", ()) {
      eprintln!("Failed to emit groups-changed event: {e}");
    }

    Ok(group)
  }

  pub fn update_group(
    &self,
    app_handle: &tauri::AppHandle,
    id: String,
    name: String,
  ) -> Result<ProfileGroup, Box<dyn std::error::Error>> {
    let mut groups_data = self.load_groups_data()?;

    // Check if another group with this name already exists
    if groups_data
      .groups
      .iter()
      .any(|g| g.name == name && g.id != id)
    {
      return Err(format!("Group with name '{name}' already exists").into());
    }

    let group = groups_data
      .groups
      .iter_mut()
      .find(|g| g.id == id)
      .ok_or_else(|| format!("Group with id '{id}' not found"))?;

    group.name = name;
    let updated_group = group.clone();

    self.save_groups_data(&groups_data)?;

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("groups-changed", ()) {
      eprintln!("Failed to emit groups-changed event: {e}");
    }

    Ok(updated_group)
  }

  pub fn delete_group(
    &self,
    app_handle: &tauri::AppHandle,
    id: String,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let mut groups_data = self.load_groups_data()?;

    let initial_len = groups_data.groups.len();
    groups_data.groups.retain(|g| g.id != id);

    if groups_data.groups.len() == initial_len {
      return Err(format!("Group with id '{id}' not found").into());
    }

    self.save_groups_data(&groups_data)?;

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("groups-changed", ()) {
      eprintln!("Failed to emit groups-changed event: {e}");
    }

    Ok(())
  }

  pub fn get_groups_with_profile_counts(
    &self,
    profiles: &[crate::profile::BrowserProfile],
  ) -> Result<Vec<GroupWithCount>, Box<dyn std::error::Error>> {
    let groups = self.get_all_groups()?;
    let mut group_counts = HashMap::new();

    // Count profiles in each group
    for profile in profiles {
      if let Some(group_id) = &profile.group_id {
        *group_counts.entry(group_id.clone()).or_insert(0) += 1;
      }
    }

    // Create result including all groups (even those with 0 count)
    let mut result = Vec::new();
    for group in groups {
      let count = group_counts.get(&group.id).copied().unwrap_or(0);
      result.push(GroupWithCount {
        id: group.id,
        name: group.name,
        count,
      });
    }

    // Add default group count (profiles without group_id), always include even if 0
    let default_count = profiles.iter().filter(|p| p.group_id.is_none()).count();
    let default_group = GroupWithCount {
      id: "default".to_string(),
      name: "Default".to_string(),
      count: default_count,
    };
    // Insert at the beginning for consistent ordering with UI expectations
    result.insert(0, default_group);

    Ok(result)
  }
}

// Global instance
lazy_static::lazy_static! {
  pub static ref GROUP_MANAGER: Mutex<GroupManager> = Mutex::new(GroupManager::new());
}

// Helper function to get groups with counts
pub fn get_groups_with_counts(profiles: &[crate::profile::BrowserProfile]) -> Vec<GroupWithCount> {
  let group_manager = GROUP_MANAGER.lock().unwrap();
  group_manager
    .get_groups_with_profile_counts(profiles)
    .unwrap_or_default()
}

// Tauri commands
#[tauri::command]
pub async fn get_profile_groups() -> Result<Vec<ProfileGroup>, String> {
  let group_manager = GROUP_MANAGER.lock().unwrap();
  group_manager
    .get_all_groups()
    .map_err(|e| format!("Failed to get profile groups: {e}"))
}

#[tauri::command]
pub async fn get_groups_with_profile_counts() -> Result<Vec<GroupWithCount>, String> {
  let profile_manager = crate::profile::ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?;
  Ok(get_groups_with_counts(&profiles))
}

#[tauri::command]
pub async fn create_profile_group(
  app_handle: tauri::AppHandle,
  name: String,
) -> Result<ProfileGroup, String> {
  let group_manager = GROUP_MANAGER.lock().unwrap();
  group_manager
    .create_group(&app_handle, name)
    .map_err(|e| format!("Failed to create group: {e}"))
}

#[tauri::command]
pub async fn update_profile_group(
  app_handle: tauri::AppHandle,
  group_id: String,
  name: String,
) -> Result<ProfileGroup, String> {
  let group_manager = GROUP_MANAGER.lock().unwrap();
  group_manager
    .update_group(&app_handle, group_id, name)
    .map_err(|e| format!("Failed to update group: {e}"))
}

#[tauri::command]
pub async fn delete_profile_group(
  app_handle: tauri::AppHandle,
  group_id: String,
) -> Result<(), String> {
  let group_manager = GROUP_MANAGER.lock().unwrap();
  group_manager
    .delete_group(&app_handle, group_id)
    .map_err(|e| format!("Failed to delete group: {e}"))
}

#[tauri::command]
pub async fn assign_profiles_to_group(
  app_handle: tauri::AppHandle,
  profile_ids: Vec<String>,
  group_id: Option<String>,
) -> Result<(), String> {
  let profile_manager = crate::profile::ProfileManager::instance();
  profile_manager
    .assign_profiles_to_group(&app_handle, profile_ids, group_id)
    .map_err(|e| format!("Failed to assign profiles to group: {e}"))
}

#[tauri::command]
pub async fn delete_selected_profiles(
  app_handle: tauri::AppHandle,
  profile_ids: Vec<String>,
) -> Result<(), String> {
  let profile_manager = crate::profile::ProfileManager::instance();
  profile_manager
    .delete_multiple_profiles(&app_handle, profile_ids)
    .map_err(|e| format!("Failed to delete profiles: {e}"))
}

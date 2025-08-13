use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

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

  pub fn create_group(&self, name: String) -> Result<ProfileGroup, Box<dyn std::error::Error>> {
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

    Ok(group)
  }

  pub fn update_group(
    &self,
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
    Ok(updated_group)
  }

  pub fn delete_group(&self, id: String) -> Result<(), Box<dyn std::error::Error>> {
    let mut groups_data = self.load_groups_data()?;

    let initial_len = groups_data.groups.len();
    groups_data.groups.retain(|g| g.id != id);

    if groups_data.groups.len() == initial_len {
      return Err(format!("Group with id '{id}' not found").into());
    }

    self.save_groups_data(&groups_data)?;
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::env;
  use tempfile::TempDir;

  fn create_test_group_manager() -> (GroupManager, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Set up a temporary home directory for testing
    env::set_var("HOME", temp_dir.path());

    // Use per-test isolated data directory without relying on global env vars
    let data_override = temp_dir.path().join("donutbrowser_test_data");
    let manager = GroupManager::with_data_dir_override(&data_override);
    (manager, temp_dir)
  }

  #[test]
  fn test_group_manager_creation() {
    let (_manager, _temp_dir) = create_test_group_manager();
    // Test passes if no panic occurs
  }

  #[test]
  fn test_create_and_get_groups() {
    let (manager, _temp_dir) = create_test_group_manager();

    // Initially should have no groups
    let groups = manager
      .get_all_groups()
      .expect("Should be able to get groups");
    assert!(groups.is_empty(), "Should start with no groups");

    // Create a group
    let group_name = "Test Group".to_string();
    let created_group = manager
      .create_group(group_name.clone())
      .expect("Should create group successfully");

    assert_eq!(
      created_group.name, group_name,
      "Created group should have correct name"
    );
    assert!(
      !created_group.id.is_empty(),
      "Created group should have an ID"
    );

    // Verify group was saved
    let groups = manager
      .get_all_groups()
      .expect("Should be able to get groups");
    assert_eq!(groups.len(), 1, "Should have one group");
    assert_eq!(
      groups[0].name, group_name,
      "Retrieved group should have correct name"
    );
    assert_eq!(
      groups[0].id, created_group.id,
      "Retrieved group should have correct ID"
    );
  }

  #[test]
  fn test_create_duplicate_group_fails() {
    let (manager, _temp_dir) = create_test_group_manager();

    let group_name = "Duplicate Group".to_string();

    // Create first group
    let _first_group = manager
      .create_group(group_name.clone())
      .expect("Should create first group");

    // Try to create duplicate group
    let result = manager.create_group(group_name.clone());
    assert!(result.is_err(), "Should fail to create duplicate group");

    let error_msg = result.unwrap_err().to_string();
    assert!(
      error_msg.contains("already exists"),
      "Error should mention group already exists"
    );
  }

  #[test]
  fn test_update_group() {
    let (manager, _temp_dir) = create_test_group_manager();

    // Create a group
    let original_name = "Original Name".to_string();
    let created_group = manager
      .create_group(original_name)
      .expect("Should create group");

    // Update the group
    let new_name = "Updated Name".to_string();
    let updated_group = manager
      .update_group(created_group.id.clone(), new_name.clone())
      .expect("Should update group successfully");

    assert_eq!(
      updated_group.name, new_name,
      "Updated group should have new name"
    );
    assert_eq!(
      updated_group.id, created_group.id,
      "Updated group should keep same ID"
    );

    // Verify update was persisted
    let groups = manager.get_all_groups().expect("Should get groups");
    assert_eq!(groups.len(), 1, "Should still have one group");
    assert_eq!(
      groups[0].name, new_name,
      "Persisted group should have updated name"
    );
  }

  #[test]
  fn test_update_nonexistent_group_fails() {
    let (manager, _temp_dir) = create_test_group_manager();

    let result = manager.update_group("nonexistent-id".to_string(), "New Name".to_string());
    assert!(result.is_err(), "Should fail to update nonexistent group");

    let error_msg = result.unwrap_err().to_string();
    assert!(
      error_msg.contains("not found"),
      "Error should mention group not found"
    );
  }

  #[test]
  fn test_delete_group() {
    let (manager, _temp_dir) = create_test_group_manager();

    // Create a group
    let group_name = "To Delete".to_string();
    let created_group = manager
      .create_group(group_name)
      .expect("Should create group");

    // Verify group exists
    let groups = manager.get_all_groups().expect("Should get groups");
    assert_eq!(groups.len(), 1, "Should have one group");

    // Delete the group
    manager
      .delete_group(created_group.id)
      .expect("Should delete group successfully");

    // Verify group was deleted
    let groups = manager.get_all_groups().expect("Should get groups");
    assert!(groups.is_empty(), "Should have no groups after deletion");
  }

  #[test]
  fn test_delete_nonexistent_group_fails() {
    let (manager, _temp_dir) = create_test_group_manager();

    let result = manager.delete_group("nonexistent-id".to_string());
    assert!(result.is_err(), "Should fail to delete nonexistent group");

    let error_msg = result.unwrap_err().to_string();
    assert!(
      error_msg.contains("not found"),
      "Error should mention group not found"
    );
  }

  #[test]
  fn test_get_groups_with_profile_counts() {
    let (manager, _temp_dir) = create_test_group_manager();

    // Create test groups
    let group1 = manager
      .create_group("Group 1".to_string())
      .expect("Should create group 1");
    let _group2 = manager
      .create_group("Group 2".to_string())
      .expect("Should create group 2");

    // Create mock profiles
    let profiles = vec![
      crate::profile::BrowserProfile {
        id: uuid::Uuid::new_v4(),
        name: "Profile 1".to_string(),
        browser: "firefox".to_string(),
        version: "1.0".to_string(),
        proxy_id: None,
        process_id: None,
        last_launch: None,
        release_type: "stable".to_string(),
        camoufox_config: None,
        group_id: Some(group1.id.clone()),
      },
      crate::profile::BrowserProfile {
        id: uuid::Uuid::new_v4(),
        name: "Profile 2".to_string(),
        browser: "firefox".to_string(),
        version: "1.0".to_string(),
        proxy_id: None,
        process_id: None,
        last_launch: None,
        release_type: "stable".to_string(),
        camoufox_config: None,
        group_id: Some(group1.id.clone()),
      },
      crate::profile::BrowserProfile {
        id: uuid::Uuid::new_v4(),
        name: "Profile 3".to_string(),
        browser: "firefox".to_string(),
        version: "1.0".to_string(),
        proxy_id: None,
        process_id: None,
        last_launch: None,
        release_type: "stable".to_string(),
        camoufox_config: None,
        group_id: None, // Default group
      },
    ];

    let groups_with_counts = manager
      .get_groups_with_profile_counts(&profiles)
      .expect("Should get groups with counts");

    // Should have default group + group1 + group2 (group2 has 0 profiles but should still appear)
    assert_eq!(
      groups_with_counts.len(),
      3,
      "Should include all groups, even those with 0 profiles"
    );

    // Check default group
    let default_group = groups_with_counts
      .iter()
      .find(|g| g.id == "default")
      .expect("Should have default group");
    assert_eq!(
      default_group.count, 1,
      "Default group should have 1 profile"
    );

    // Check group1
    let group1_with_count = groups_with_counts
      .iter()
      .find(|g| g.id == group1.id)
      .expect("Should have group1");
    assert_eq!(group1_with_count.count, 2, "Group1 should have 2 profiles");

    // Check that group2 exists with 0 profiles
    let group2_with_count = groups_with_counts
      .iter()
      .find(|g| g.name == "Group 2")
      .expect("Should have group2 present even with 0 profiles");
    assert_eq!(group2_with_count.count, 0, "Group2 should have 0 profiles");
  }
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
pub async fn create_profile_group(name: String) -> Result<ProfileGroup, String> {
  let group_manager = GROUP_MANAGER.lock().unwrap();
  group_manager
    .create_group(name)
    .map_err(|e| format!("Failed to create group: {e}"))
}

#[tauri::command]
pub async fn update_profile_group(group_id: String, name: String) -> Result<ProfileGroup, String> {
  let group_manager = GROUP_MANAGER.lock().unwrap();
  group_manager
    .update_group(group_id, name)
    .map_err(|e| format!("Failed to update group: {e}"))
}

#[tauri::command]
pub async fn delete_profile_group(group_id: String) -> Result<(), String> {
  let group_manager = GROUP_MANAGER.lock().unwrap();
  group_manager
    .delete_group(group_id)
    .map_err(|e| format!("Failed to delete group: {e}"))
}

#[tauri::command]
pub async fn assign_profiles_to_group(
  profile_names: Vec<String>,
  group_id: Option<String>,
) -> Result<(), String> {
  let profile_manager = crate::profile::ProfileManager::instance();
  profile_manager
    .assign_profiles_to_group(profile_names, group_id)
    .map_err(|e| format!("Failed to assign profiles to group: {e}"))
}

#[tauri::command]
pub async fn delete_selected_profiles(profile_names: Vec<String>) -> Result<(), String> {
  let profile_manager = crate::profile::ProfileManager::instance();
  profile_manager
    .delete_multiple_profiles(profile_names)
    .map_err(|e| format!("Failed to delete profiles: {e}"))
}

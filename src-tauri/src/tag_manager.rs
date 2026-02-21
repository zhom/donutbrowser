use crate::profile::BrowserProfile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct TagsData {
  tags: Vec<String>,
}

pub struct TagManager;

impl TagManager {
  pub fn new() -> Self {
    Self
  }

  fn get_tags_file_path(&self) -> std::path::PathBuf {
    crate::app_dirs::data_subdir().join("tags.json")
  }

  fn load_tags_data(&self) -> Result<TagsData, Box<dyn std::error::Error>> {
    let file_path = self.get_tags_file_path();
    if !file_path.exists() {
      return Ok(TagsData::default());
    }
    let content = fs::read_to_string(file_path)?;
    let data: TagsData = serde_json::from_str(&content)?;
    Ok(data)
  }

  fn save_tags_data(&self, data: &TagsData) -> Result<(), Box<dyn std::error::Error>> {
    let file_path = self.get_tags_file_path();
    if let Some(parent) = file_path.parent() {
      fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(data)?;
    fs::write(file_path, json)?;
    Ok(())
  }

  pub fn get_all_tags(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut all = self.load_tags_data()?.tags;
    // Ensure deterministic order
    all.sort();
    all.dedup();
    Ok(all)
  }

  pub fn rebuild_from_profiles(
    &self,
    profiles: &[BrowserProfile],
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Build a set of all tags currently used by any profile
    let mut set: BTreeSet<String> = BTreeSet::new();
    for profile in profiles {
      for tag in &profile.tags {
        // Store exactly as provided (no normalization) to preserve characters
        set.insert(tag.clone());
      }
    }
    let combined: Vec<String> = set.into_iter().collect();
    self.save_tags_data(&TagsData {
      tags: combined.clone(),
    })?;
    Ok(combined)
  }
}

#[tauri::command]
pub fn get_all_tags() -> Result<Vec<String>, String> {
  let tag_manager = crate::tag_manager::TAG_MANAGER.lock().unwrap();
  tag_manager
    .get_all_tags()
    .map_err(|e| format!("Failed to get tags: {e}"))
}

lazy_static::lazy_static! {
  pub static ref TAG_MANAGER: std::sync::Mutex<TagManager> = std::sync::Mutex::new(TagManager::new());
}

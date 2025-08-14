use crate::profile::BrowserProfile;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct TagsData {
  tags: Vec<String>,
}

pub struct TagManager {
  base_dirs: BaseDirs,
  data_dir_override: Option<PathBuf>,
}

impl TagManager {
  pub fn new() -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      data_dir_override: std::env::var("DONUTBROWSER_DATA_DIR")
        .ok()
        .map(PathBuf::from),
    }
  }

  #[allow(dead_code)]
  pub fn with_data_dir_override(dir: &Path) -> Self {
    Self {
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
      data_dir_override: Some(dir.to_path_buf()),
    }
  }

  fn get_tags_file_path(&self) -> PathBuf {
    if let Some(dir) = &self.data_dir_override {
      let mut override_path = dir.clone();
      let _ = fs::create_dir_all(&override_path);
      override_path.push("tags.json");
      return override_path;
    }

    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("data");
    path.push("tags.json");
    path
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

lazy_static::lazy_static! {
  pub static ref TAG_MANAGER: std::sync::Mutex<TagManager> = std::sync::Mutex::new(TagManager::new());
}

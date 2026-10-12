//! Profile statuses: a named, colored label per profile for where its account
//! stands (active, warming up, banned, ...). The person defines the set; a
//! profile points at one by id.

use crate::sync::config_entities::{self, ConfigEntity, ConfigKind, ConfigStore};
use serde::{Deserialize, Serialize};

const MAX_STATUSES: usize = 100;
const MAX_NAME_CHARS: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileStatus {
  pub id: String,
  pub name: String,
  /// `#RRGGBB`. A data color the person picked, not a theme color.
  pub color: String,
  /// Order in lists and menus: the order the statuses were created in.
  #[serde(default)]
  pub position: u64,
  #[serde(default)]
  pub updated_at: u64,
}

static STORE: ConfigStore<ProfileStatus> = ConfigStore::new("profile_statuses.json");

impl ConfigEntity for ProfileStatus {
  const KIND: ConfigKind = ConfigKind::Status;
  fn id(&self) -> &str {
    &self.id
  }
  fn updated_at(&self) -> u64 {
    self.updated_at
  }
  fn store() -> &'static ConfigStore<Self> {
    &STORE
  }
}

fn code(code: &str) -> String {
  crate::backend_error(code)
}

fn checked_name(name: &str) -> Result<String, String> {
  let name = name.trim();
  if name.is_empty() {
    return Err(code("NAME_CANNOT_BE_EMPTY"));
  }
  if name.chars().count() > MAX_NAME_CHARS {
    return Err(code("PROFILE_STATUS_NAME_TOO_LONG"));
  }
  Ok(name.to_string())
}

fn checked_color(color: &str) -> Result<String, String> {
  let color = color.trim();
  let valid =
    color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit());
  if valid {
    Ok(color.to_ascii_lowercase())
  } else {
    Err(code("PROFILE_STATUS_COLOR_INVALID"))
  }
}

pub fn list() -> Vec<ProfileStatus> {
  let mut statuses = STORE.list();
  statuses.sort_by(|a, b| {
    a.position
      .cmp(&b.position)
      .then_with(|| a.name.cmp(&b.name))
  });
  statuses
}

pub fn exists(status_id: &str) -> bool {
  STORE.get(status_id).is_some()
}

pub fn create(name: &str, color: &str) -> Result<ProfileStatus, String> {
  let name = checked_name(name)?;
  let color = checked_color(color)?;
  let status = STORE.mutate(|statuses| {
    if statuses.len() >= MAX_STATUSES {
      return Err(code("PROFILE_STATUS_LIMIT_REACHED"));
    }
    if statuses.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
      return Err(code("PROFILE_STATUS_EXISTS"));
    }
    let status = ProfileStatus {
      id: uuid::Uuid::new_v4().to_string(),
      name,
      color,
      position: statuses.iter().map(|s| s.position + 1).max().unwrap_or(0),
      updated_at: crate::proxy_manager::now_secs(),
    };
    statuses.push(status.clone());
    Ok(status)
  })?;
  config_entities::changed_locally::<ProfileStatus>(&status.id);
  crate::team_activity::record("status.create", &status.id, Some(&status.name), None);
  Ok(status)
}

pub fn update(status_id: &str, name: &str, color: &str) -> Result<ProfileStatus, String> {
  let name = checked_name(name)?;
  let color = checked_color(color)?;
  let status = STORE.mutate(|statuses| {
    if statuses
      .iter()
      .any(|s| s.id != status_id && s.name.eq_ignore_ascii_case(&name))
    {
      return Err(code("PROFILE_STATUS_EXISTS"));
    }
    let status = statuses
      .iter_mut()
      .find(|s| s.id == status_id)
      .ok_or_else(|| code("PROFILE_STATUS_NOT_FOUND"))?;
    if status.name != name || status.color != color {
      status.name = name;
      status.color = color;
      status.updated_at = crate::proxy_manager::now_secs();
    }
    Ok(status.clone())
  })?;
  config_entities::changed_locally::<ProfileStatus>(&status.id);
  crate::team_activity::record("status.update", &status.id, Some(&status.name), None);
  Ok(status)
}

/// Deletes the status and takes it off every profile that had it.
pub fn delete(status_id: &str) -> Result<(), String> {
  let removed = STORE.mutate(|statuses| {
    let before = statuses.len();
    statuses.retain(|s| s.id != status_id);
    Ok(statuses.len() != before)
  })?;
  if !removed {
    return Err(code("PROFILE_STATUS_NOT_FOUND"));
  }
  crate::profile::ProfileManager::instance()
    .clear_status_references(status_id)
    .map_err(|e| crate::wrap_backend_error(e, "Failed to clear the status from profiles"))?;
  config_entities::deleted_locally::<ProfileStatus>(status_id);
  crate::team_activity::record("status.delete", status_id, None, None);
  Ok(())
}

#[tauri::command]
pub fn list_profile_statuses() -> Vec<ProfileStatus> {
  list()
}

#[tauri::command]
pub fn create_profile_status(name: String, color: String) -> Result<ProfileStatus, String> {
  create(&name, &color)
}

#[tauri::command]
pub fn update_profile_status(
  status_id: String,
  name: String,
  color: String,
) -> Result<ProfileStatus, String> {
  update(&status_id, &name, &color)
}

#[tauri::command]
pub fn delete_profile_status(status_id: String) -> Result<(), String> {
  delete(&status_id)
}

/// Give every listed profile one status, or none.
#[tauri::command]
pub fn set_profiles_status(
  profile_ids: Vec<String>,
  status_id: Option<String>,
) -> Result<(), String> {
  let status_id = status_id.filter(|id| !id.is_empty());
  if let Some(id) = &status_id {
    if !exists(id) {
      return Err(code("PROFILE_STATUS_NOT_FOUND"));
    }
  }
  crate::profile::ProfileManager::instance()
    .set_profiles_status(&profile_ids, status_id)
    .map_err(|e| crate::wrap_backend_error(e, "Failed to set the profile status"))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_are_trimmed_and_bounded() {
    assert_eq!(checked_name("  Warming up ").unwrap(), "Warming up");
    assert!(checked_name("   ")
      .unwrap_err()
      .contains("NAME_CANNOT_BE_EMPTY"));
    assert!(checked_name(&"x".repeat(41))
      .unwrap_err()
      .contains("PROFILE_STATUS_NAME_TOO_LONG"));
    assert_eq!(checked_name(&"é".repeat(40)).unwrap().chars().count(), 40);
  }

  #[test]
  fn statuses_keep_their_order_and_refuse_duplicate_names() {
    let dir = tempfile::tempdir().unwrap();
    let _data = crate::app_dirs::set_test_data_dir(dir.path().to_path_buf());

    let active = create("Active", "#22AA44").unwrap();
    let banned = create("Banned", "#cc2222").unwrap();
    assert_eq!(active.color, "#22aa44");
    assert!(create(" active ", "#000000")
      .unwrap_err()
      .contains("PROFILE_STATUS_EXISTS"));
    assert_eq!(
      list().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
      ["Active", "Banned"]
    );

    let renamed = update(&banned.id, "Suspended", "#cc2222").unwrap();
    assert_eq!(renamed.name, "Suspended");
    assert!(renamed.updated_at >= banned.updated_at);
    assert!(update(&banned.id, "Active", "#cc2222")
      .unwrap_err()
      .contains("PROFILE_STATUS_EXISTS"));
    assert!(update("missing", "X", "#000000")
      .unwrap_err()
      .contains("PROFILE_STATUS_NOT_FOUND"));

    delete(&active.id).unwrap();
    assert!(!exists(&active.id));
    assert!(delete(&active.id)
      .unwrap_err()
      .contains("PROFILE_STATUS_NOT_FOUND"));
    assert_eq!(list().len(), 1);
  }

  #[test]
  fn colors_must_be_six_digit_hex() {
    assert_eq!(checked_color("#A1B2C3").unwrap(), "#a1b2c3");
    for bad in ["", "#fff", "a1b2c3", "#a1b2c", "#a1b2c3d", "#zzzzzz", "red"] {
      assert!(checked_color(bad).is_err(), "{bad:?}");
    }
  }
}

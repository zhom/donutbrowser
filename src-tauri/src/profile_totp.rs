//! The 2FA keys of the accounts a profile signs in to.
//!
//! Device-only on purpose: one file per profile, sealed with this
//! installation's vault key. The keys are never synced, never part of a
//! profile export, and never reachable over REST or MCP. Only the codes leave
//! this module, and only to the app's own window.

use crate::totp::{TotpCode, TotpKey};
use serde::{Deserialize, Serialize};

const HEADER: &[u8; 5] = b"DBTFA";
const MAX_ENTRIES: usize = 50;
const MAX_LABEL_CHARS: usize = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
  id: String,
  label: String,
  key: TotpKey,
}

/// What the window may know about an entry: everything but the secret.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TotpEntry {
  pub id: String,
  pub label: String,
  pub issuer: Option<String>,
  pub account: Option<String>,
  pub digits: u32,
  pub period: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TotpEntryCode {
  pub id: String,
  #[serde(flatten)]
  pub code: TotpCode,
}

fn magic() -> [u8; 6] {
  let mut magic = [0u8; 6];
  magic[..5].copy_from_slice(HEADER);
  magic[5] = 2;
  magic
}

fn file_for(profile_id: &str) -> Result<std::path::PathBuf, String> {
  // A UUID, so a crafted id can never name a path outside the folder.
  let id =
    uuid::Uuid::parse_str(profile_id).map_err(|_| crate::backend_error("INVALID_PROFILE_ID"))?;
  Ok(
    crate::app_dirs::data_subdir()
      .join("totp")
      .join(format!("{id}.dat")),
  )
}

fn read(profile_id: &str) -> Result<Vec<StoredEntry>, String> {
  match crate::vault::open(&file_for(profile_id)?, &magic())? {
    None => Ok(Vec::new()),
    Some(json) => serde_json::from_str(&json).map_err(|e| format!("2FA keys are unreadable: {e}")),
  }
}

fn write(profile_id: &str, entries: &[StoredEntry]) -> Result<(), String> {
  let file = file_for(profile_id)?;
  if entries.is_empty() {
    return match std::fs::remove_file(&file) {
      Ok(()) => Ok(()),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
      Err(e) => Err(e.to_string()),
    };
  }
  let json = serde_json::to_string(entries).map_err(|e| e.to_string())?;
  crate::vault::seal(&file, &magic(), &json)?;
  crate::app_dirs::restrict_to_owner(&file);
  Ok(())
}

fn view(entry: &StoredEntry) -> TotpEntry {
  TotpEntry {
    id: entry.id.clone(),
    label: entry.label.clone(),
    issuer: entry.key.issuer.clone(),
    account: entry.key.account.clone(),
    digits: entry.key.digits,
    period: entry.key.period,
  }
}

fn ensure_profile(profile_id: &str) -> Result<(), String> {
  let exists = crate::profile::ProfileManager::instance()
    .list_profiles()
    .map_err(|e| crate::wrap_backend_error(e, "Failed to list profiles"))?
    .iter()
    .any(|profile| profile.id.to_string() == profile_id);
  if exists {
    Ok(())
  } else {
    Err(crate::backend_error("PROFILE_NOT_FOUND"))
  }
}

pub fn list(profile_id: &str) -> Result<Vec<TotpEntry>, String> {
  Ok(read(profile_id)?.iter().map(view).collect())
}

pub fn add(profile_id: &str, label: &str, key: &str) -> Result<TotpEntry, String> {
  ensure_profile(profile_id)?;
  let key = TotpKey::parse(key).map_err(|_| crate::backend_error("TOTP_KEY_INVALID"))?;
  let label = label.trim();
  let label = if label.is_empty() {
    key
      .issuer
      .clone()
      .or_else(|| key.account.clone())
      .unwrap_or_default()
  } else {
    label.to_string()
  };
  if label.is_empty() {
    return Err(crate::backend_error("NAME_CANNOT_BE_EMPTY"));
  }
  if label.chars().count() > MAX_LABEL_CHARS {
    return Err(crate::backend_error("TOTP_LABEL_TOO_LONG"));
  }
  let mut entries = read(profile_id)?;
  if entries.len() >= MAX_ENTRIES {
    return Err(crate::backend_error("TOTP_LIMIT_REACHED"));
  }
  let entry = StoredEntry {
    id: uuid::Uuid::new_v4().to_string(),
    label,
    key,
  };
  entries.push(entry.clone());
  write(profile_id, &entries)?;
  log::info!("2FA key added profile={profile_id} keys={}", entries.len());
  Ok(view(&entry))
}

pub fn remove(profile_id: &str, entry_id: &str) -> Result<(), String> {
  let mut entries = read(profile_id)?;
  let before = entries.len();
  entries.retain(|entry| entry.id != entry_id);
  if entries.len() == before {
    return Err(crate::backend_error("TOTP_ENTRY_NOT_FOUND"));
  }
  write(profile_id, &entries)?;
  log::info!(
    "2FA key removed profile={profile_id} keys={}",
    entries.len()
  );
  Ok(())
}

pub fn codes(profile_id: &str) -> Result<Vec<TotpEntryCode>, String> {
  Ok(
    read(profile_id)?
      .iter()
      .map(|entry| TotpEntryCode {
        id: entry.id.clone(),
        code: entry.key.code_now(),
      })
      .collect(),
  )
}

/// With the profile itself: its accounts' keys must not outlive it.
pub fn forget_profile(profile_id: &str) {
  if let Ok(file) = file_for(profile_id) {
    match std::fs::remove_file(&file) {
      Ok(()) => log::info!("2FA keys erased profile={profile_id}"),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
      Err(e) => log::warn!("2FA keys not erased profile={profile_id} err=\"{e}\""),
    }
  }
}

#[tauri::command]
pub fn list_profile_totp(profile_id: String) -> Result<Vec<TotpEntry>, String> {
  list(&profile_id)
}

#[tauri::command]
pub fn add_profile_totp(
  profile_id: String,
  label: String,
  key: String,
) -> Result<TotpEntry, String> {
  add(&profile_id, &label, &key)
}

#[tauri::command]
pub fn delete_profile_totp(profile_id: String, entry_id: String) -> Result<(), String> {
  remove(&profile_id, &entry_id)
}

#[tauri::command]
pub fn get_profile_totp_codes(profile_id: String) -> Result<Vec<TotpEntryCode>, String> {
  codes(&profile_id)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn keys_are_sealed_on_disk_and_only_codes_come_back() {
    let dir = tempfile::tempdir().unwrap();
    let _data = crate::app_dirs::set_test_data_dir(dir.path().to_path_buf());
    let profile = uuid::Uuid::new_v4().to_string();
    let key = "otpauth://totp/Shop:alice?secret=JBSWY3DPEHPK3PXP&issuer=Shop";
    let entry = StoredEntry {
      id: "e1".to_string(),
      label: "Shop".to_string(),
      key: TotpKey::parse(key).unwrap(),
    };
    write(&profile, std::slice::from_ref(&entry)).unwrap();

    let raw = std::fs::read(file_for(&profile).unwrap()).unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(
      !text.contains("JBSWY3DPEHPK3PXP"),
      "the secret must not be readable on disk"
    );
    assert!(!text.contains("alice"));

    let listed = list(&profile).unwrap();
    assert_eq!(listed[0].label, "Shop");
    assert_eq!(listed[0].account.as_deref(), Some("alice"));
    assert!(!serde_json::to_string(&listed)
      .unwrap()
      .contains("JBSWY3DPEHPK3PXP"));
    let code = codes(&profile).unwrap();
    assert_eq!(code[0].code.code.len(), 6);

    remove(&profile, "e1").unwrap();
    assert!(
      !file_for(&profile).unwrap().exists(),
      "the last key takes the file with it"
    );
    assert!(remove(&profile, "e1")
      .unwrap_err()
      .contains("TOTP_ENTRY_NOT_FOUND"));
  }

  #[test]
  fn a_profile_id_must_be_a_uuid() {
    assert!(file_for("../../etc/passwd")
      .unwrap_err()
      .contains("INVALID_PROFILE_ID"));
  }
}

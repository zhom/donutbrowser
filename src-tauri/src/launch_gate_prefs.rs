//! Persisted "I know, launch it anyway" acknowledgements for the launch gate.
//!
//! Deliberately NOT synced. An acknowledgement is a statement about this
//! machine's operator ("I understand this profile's exit disagrees with its
//! fingerprint"), not a property of the profile. Syncing it would let one
//! teammate disarm another's gate, and writing it into profile metadata would
//! bump `updated_at` and make a local dismissal look like a remote edit.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::profile::types::BrowserProfile;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FingerprintAck {
  /// Hash of the fingerprint that was acknowledged.
  pub fingerprint_hash: String,
  /// Exit endpoint identity it was acknowledged against.
  pub exit_identity: String,
  pub acked_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaunchGatePrefs {
  #[serde(default)]
  pub fingerprint_acks: HashMap<String, FingerprintAck>,
  /// Profile id -> acknowledged extension keys.
  #[serde(default)]
  pub vpn_extension_acks: HashMap<String, Vec<String>>,
}

lazy_static::lazy_static! {
  /// Serializes read-modify-write so two concurrent acknowledgements in a bulk
  /// run cannot clobber each other.
  static ref PREFS_LOCK: Mutex<()> = Mutex::new(());
}

fn prefs_file() -> PathBuf {
  crate::app_dirs::data_subdir().join("launch_gate_prefs.json")
}

pub fn load() -> LaunchGatePrefs {
  let Ok(content) = std::fs::read_to_string(prefs_file()) else {
    return LaunchGatePrefs::default();
  };
  serde_json::from_str(&content).unwrap_or_else(|e| {
    log::warn!("Failed to parse launch gate prefs, ignoring them: {e}");
    LaunchGatePrefs::default()
  })
}

fn save(prefs: &LaunchGatePrefs) {
  let path = prefs_file();
  if let Some(parent) = path.parent() {
    if let Err(e) = std::fs::create_dir_all(parent) {
      log::warn!("Failed to create launch gate prefs dir: {e}");
      return;
    }
  }
  match serde_json::to_string_pretty(prefs) {
    Ok(json) => {
      if let Err(e) = std::fs::write(&path, json) {
        log::warn!("Failed to write launch gate prefs: {e}");
      }
    }
    Err(e) => log::warn!("Failed to serialize launch gate prefs: {e}"),
  }
}

fn update(mutate: impl FnOnce(&mut LaunchGatePrefs)) {
  let _guard = PREFS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
  let mut prefs = load();
  mutate(&mut prefs);
  save(&prefs);
}

/// Stable digest of a profile's stored fingerprint, so an acknowledgement stops
/// applying the moment the fingerprint is regenerated or matched to a new exit.
pub fn fingerprint_hash(profile: &BrowserProfile) -> String {
  use sha2::{Digest, Sha256};
  let fingerprint = profile
    .wayfern_config
    .as_ref()
    .and_then(|c| c.fingerprint.as_deref())
    .unwrap_or("");
  let mut hasher = Sha256::new();
  hasher.update(fingerprint.as_bytes());
  hasher
    .finalize()
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect()
}

/// Record that the user accepted this exact (fingerprint, exit) mismatch.
pub fn ack_fingerprint(profile: &BrowserProfile, exit_identity: &str) {
  let ack = FingerprintAck {
    fingerprint_hash: fingerprint_hash(profile),
    exit_identity: exit_identity.to_string(),
    acked_at: crate::proxy_manager::now_secs(),
  };
  let profile_id = profile.id.to_string();
  update(|prefs| {
    prefs.fingerprint_acks.insert(profile_id, ack);
  });
}

/// Whether the user already accepted the mismatch this profile currently has.
///
/// Bound to both the fingerprint and the exit endpoint on purpose: the old
/// per-profile "don't warn again" flag never expired, so one dismissal left a
/// profile unprotected forever, including after its proxy was swapped for one
/// in a different country.
pub fn fingerprint_ack_matches(profile: &BrowserProfile, exit_identity: &str) -> bool {
  let prefs = load();
  prefs
    .fingerprint_acks
    .get(&profile.id.to_string())
    .is_some_and(|ack| {
      ack.fingerprint_hash == fingerprint_hash(profile) && ack.exit_identity == exit_identity
    })
}

pub fn ack_extensions(profile_id: &str, keys: &[String]) {
  if keys.is_empty() {
    return;
  }
  let profile_id = profile_id.to_string();
  let keys = keys.to_vec();
  update(|prefs| {
    let entry = prefs.vpn_extension_acks.entry(profile_id).or_default();
    for key in keys {
      if !entry.contains(&key) {
        entry.push(key);
      }
    }
  });
}

/// True when every one of these extensions has already been acknowledged for
/// this profile. Installing a *different* VPN extension later re-warns, because
/// its key is not in the acknowledged set.
pub fn extensions_acked(profile_id: &str, keys: &[String]) -> bool {
  if keys.is_empty() {
    return true;
  }
  let prefs = load();
  let Some(acked) = prefs.vpn_extension_acks.get(profile_id) else {
    return false;
  };
  keys.iter().all(|k| acked.contains(k))
}

/// Drop everything remembered for a profile, for use when it is deleted.
pub fn forget_profile(profile_id: &str) {
  let profile_id = profile_id.to_string();
  update(|prefs| {
    prefs.fingerprint_acks.remove(&profile_id);
    prefs.vpn_extension_acks.remove(&profile_id);
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn profile_with(fingerprint: &str) -> BrowserProfile {
    let mut profile = BrowserProfile {
      id: uuid::Uuid::new_v4(),
      browser: "wayfern".into(),
      ..Default::default()
    };
    profile.wayfern_config = Some(crate::wayfern_manager::WayfernConfig {
      fingerprint: Some(fingerprint.to_string()),
      ..Default::default()
    });
    profile
  }

  #[test]
  fn fingerprint_hash_changes_with_the_fingerprint() {
    let a = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let b = profile_with(r#"{"timezone":"America/New_York"}"#);
    assert_ne!(fingerprint_hash(&a), fingerprint_hash(&b));
  }

  #[test]
  fn fingerprint_hash_is_stable_for_the_same_fingerprint() {
    let a = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let b = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    assert_eq!(fingerprint_hash(&a), fingerprint_hash(&b));
  }

  #[test]
  fn a_profile_without_a_fingerprint_still_hashes() {
    let profile = BrowserProfile {
      id: uuid::Uuid::new_v4(),
      ..Default::default()
    };
    assert!(!fingerprint_hash(&profile).is_empty());
  }

  #[test]
  fn acks_round_trip_and_rearm_on_change() {
    let _guard = crate::app_dirs::set_test_data_dir(tempfile::tempdir().expect("tempdir").keep());

    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    assert!(!fingerprint_ack_matches(&profile, "http://gw:1"));

    ack_fingerprint(&profile, "http://gw:1");
    assert!(fingerprint_ack_matches(&profile, "http://gw:1"));

    // Swapping the proxy re-arms the gate: the mismatch the user accepted is
    // not the mismatch they now have.
    assert!(!fingerprint_ack_matches(&profile, "http://other:2"));

    // Regenerating the fingerprint re-arms it too.
    let regenerated = profile_with(r#"{"timezone":"America/New_York"}"#);
    let mut same_id = regenerated.clone();
    same_id.id = profile.id;
    assert!(!fingerprint_ack_matches(&same_id, "http://gw:1"));
  }

  #[test]
  fn extension_acks_are_per_key() {
    let _guard = crate::app_dirs::set_test_data_dir(tempfile::tempdir().expect("tempdir").keep());

    let profile_id = uuid::Uuid::new_v4().to_string();
    let nord = vec!["crx:aaaa".to_string()];
    let other = vec!["crx:bbbb".to_string()];

    assert!(!extensions_acked(&profile_id, &nord));
    ack_extensions(&profile_id, &nord);
    assert!(extensions_acked(&profile_id, &nord));

    // A different extension installed later must warn again.
    assert!(!extensions_acked(&profile_id, &other));
    assert!(!extensions_acked(
      &profile_id,
      &[nord[0].clone(), other[0].clone()]
    ));

    // Nothing to acknowledge is trivially acknowledged, so an empty scan never
    // opens the dialog.
    assert!(extensions_acked(&profile_id, &[]));
  }

  #[test]
  fn forgetting_a_profile_clears_both_kinds_of_ack() {
    let _guard = crate::app_dirs::set_test_data_dir(tempfile::tempdir().expect("tempdir").keep());

    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let profile_id = profile.id.to_string();
    ack_fingerprint(&profile, "http://gw:1");
    ack_extensions(&profile_id, &["crx:aaaa".to_string()]);

    forget_profile(&profile_id);

    assert!(!fingerprint_ack_matches(&profile, "http://gw:1"));
    assert!(!extensions_acked(&profile_id, &["crx:aaaa".to_string()]));
  }

  #[test]
  fn corrupt_prefs_file_is_ignored_rather_than_fatal() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let _guard = crate::app_dirs::set_test_data_dir(dir.clone());
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(
      dir.join("data").join("launch_gate_prefs.json"),
      "{ not json",
    )
    .unwrap();

    // Must not panic, and must fail closed (nothing acknowledged).
    let prefs = load();
    assert!(prefs.fingerprint_acks.is_empty());
  }
}

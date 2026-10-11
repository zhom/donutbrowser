use crate::wayfern_manager::WayfernConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum SyncStatus {
  #[default]
  Disabled,
  Syncing,
  Synced,
  Error,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncMode {
  #[default]
  Disabled,
  Regular,
  Encrypted,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct BrowserProfile {
  pub id: uuid::Uuid,
  pub name: String,
  pub browser: String,
  pub version: String,
  #[serde(default)]
  pub proxy_id: Option<String>, // Reference to stored proxy
  #[serde(default)]
  pub vpn_id: Option<String>, // Reference to stored VPN config
  #[serde(default)]
  pub launch_hook: Option<String>,
  #[serde(default)]
  pub process_id: Option<u32>,
  #[serde(default)]
  pub last_launch: Option<u64>,
  #[serde(default = "default_release_type")]
  pub release_type: String,
  #[serde(default)]
  pub wayfern_config: Option<WayfernConfig>, // Wayfern configuration
  #[serde(default)]
  pub group_id: Option<String>, // Reference to profile group
  #[serde(default)]
  pub tags: Vec<String>, // Free-form tags
  #[serde(default)]
  pub note: Option<String>, // User note
  #[serde(default)]
  pub window_color: Option<String>, // Per-profile window frame color "#RRGGBB"; auto-derived from the id when unset
  #[serde(default)]
  pub sync_mode: SyncMode,
  #[serde(default)]
  pub encryption_salt: Option<String>,
  #[serde(default)]
  pub last_sync: Option<u64>, // Timestamp of last successful sync (epoch seconds)
  #[serde(default)]
  pub host_os: Option<String>, // OS where profile was created ("macos", "windows", "linux")
  #[serde(default)]
  pub ephemeral: bool,
  /// A profile that exists for one automation run. REST and MCP create it,
  /// the browser stopping destroys it, and a startup sweep destroys any that
  /// outlived a crash. Always ephemeral as well, so nothing it browses ever
  /// reaches real disk. Never trashed: a disposable profile has nothing to
  /// restore, and keeping one would defeat the point of asking for it.
  #[serde(default)]
  pub temporary: bool,
  #[serde(default)]
  pub extension_group_id: Option<String>,
  #[serde(default)]
  pub proxy_bypass_rules: Vec<String>,
  #[serde(default)]
  pub created_by_id: Option<String>,
  #[serde(default)]
  pub created_by_email: Option<String>,
  #[serde(default)]
  pub dns_blocklist: Option<String>,
  /// The DNS server this profile resolves names with, through its own proxy
  /// or VPN (see `profile_dns`). `None` lets the proxy resolve them.
  #[serde(default)]
  pub dns_server: Option<String>,
  /// The profile's status (see `profile_statuses`), by id.
  #[serde(default)]
  pub status_id: Option<String>,
  /// Pages a launch opens, in order. An explicit launch URL replaces them.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub start_urls: Vec<String>,
  /// True when the on-disk profile dir is encrypted with a per-profile password.
  /// Decryption goes to a RAM-backed ephemeral dir, never to disk.
  #[serde(default)]
  pub password_protected: bool,
  /// Wipe browsing data (except extensions and bookmarks) when the browser
  /// exits. Ignored for ephemeral and password-protected profiles.
  #[serde(default)]
  pub clear_on_close: bool,
  /// Profile creation timestamp (epoch seconds, UTC). `None` for legacy
  /// profiles that pre-date this field — those are treated as ancient by
  /// any staleness check.
  #[serde(default)]
  pub created_at: Option<u64>,
  /// Unix seconds of the last meaningful metadata edit (name, tags, note,
  /// proxy/vpn/group/extension assignment, launch hook, bypass rules, dns).
  /// Source of truth for metadata sync conflict resolution (last-write-wins);
  /// NOT bumped by browser-file changes, which sync via the file manifest.
  #[serde(default)]
  pub updated_at: Option<u64>,
}

pub fn default_release_type() -> String {
  "stable".to_string()
}

pub fn get_host_os() -> String {
  if cfg!(target_os = "macos") {
    "macos".to_string()
  } else if cfg!(target_os = "windows") {
    "windows".to_string()
  } else {
    "linux".to_string()
  }
}

/// Whether a value is one `get_host_os` can actually return.
///
/// A fingerprint OS is a wider set than a host OS: `"android"` and `"ios"` are
/// valid fingerprints but no machine ever reports them as its host. Storing one
/// in `host_os` records a creating machine that cannot exist, which every
/// reader of `host_os` then treats as foreign.
pub fn is_host_os(value: &str) -> bool {
  matches!(value, "macos" | "windows" | "linux")
}

impl BrowserProfile {
  /// Get the path to the profile data directory (profiles/{uuid}/profile)
  pub fn get_profile_data_path(&self, profiles_dir: &Path) -> PathBuf {
    profiles_dir.join(self.id.to_string()).join("profile")
  }

  /// Resolve the OS this profile was created on. Checks `host_os` first,
  /// then falls back to the fingerprint config's `os` field (for profiles
  /// created before `host_os` was introduced or synced without it).
  pub fn resolved_os(&self) -> Option<&str> {
    self
      .host_os
      .as_deref()
      .or_else(|| self.wayfern_config.as_ref().and_then(|c| c.os.as_deref()))
  }

  /// Whether this profile's directory runs on every supported OS, whichever OS
  /// wrote it. True for Wayfern; other browsers stay on the OS that created
  /// their profile.
  pub fn runs_on_any_os(&self) -> bool {
    self.browser == "wayfern"
  }

  /// Returns true when the profile cannot run here because a browser bound to
  /// its OS created it on a different OS. A Wayfern profile is never cross-OS:
  /// it syncs its data and launches on every OS.
  pub fn is_cross_os(&self) -> bool {
    if self.runs_on_any_os() {
      return false;
    }
    match self.resolved_os() {
      Some(os) => os != get_host_os(),
      None => false,
    }
  }

  /// The OS a fingerprint without an explicit `os` describes: the machine that
  /// created the profile, or this machine when that is not recorded. Such a
  /// fingerprint was generated for its creating machine, so it keeps that OS
  /// when the profile moves to another one.
  pub fn native_os(&self) -> String {
    self
      .host_os
      .as_deref()
      .filter(|os| is_host_os(os))
      .map(str::to_string)
      .unwrap_or_else(get_host_os)
  }

  /// Returns true if sync is enabled (either Regular or Encrypted mode).
  pub fn is_sync_enabled(&self) -> bool {
    self.sync_mode != SyncMode::Disabled
  }

  /// Returns true if sync uses E2E encryption.
  pub fn is_encrypted_sync(&self) -> bool {
    self.sync_mode == SyncMode::Encrypted
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn host_os_is_always_a_valid_host_os() {
    // The invariant the host_os backfill guard rests on: whatever this machine
    // reports must satisfy is_host_os, on every platform.
    assert!(is_host_os(&get_host_os()));
  }

  #[test]
  fn mobile_fingerprint_targets_are_not_host_operating_systems() {
    // Backfilling host_os from a fingerprint OS used to store these, and since
    // get_host_os can never return them, is_cross_os stayed true forever and
    // the profile could not be launched on the machine that created it.
    for os in ["macos", "windows", "linux"] {
      assert!(is_host_os(os), "{os} must count as a host OS");
    }
    for os in ["android", "ios", "", "Windows", "chromeos"] {
      assert!(!is_host_os(os), "{os} must not be stored as a host OS");
    }
  }

  fn other_host_os() -> &'static str {
    if get_host_os() == "windows" {
      "macos"
    } else {
      "windows"
    }
  }

  fn profile_from(browser: &str, host_os: Option<&str>) -> BrowserProfile {
    BrowserProfile {
      browser: browser.to_string(),
      host_os: host_os.map(str::to_string),
      ..BrowserProfile::default()
    }
  }

  #[test]
  fn a_wayfern_profile_from_another_os_is_not_cross_os() {
    for os in ["macos", "windows", "linux"] {
      let profile = profile_from("wayfern", Some(os));
      assert!(profile.runs_on_any_os());
      assert!(
        !profile.is_cross_os(),
        "a Wayfern profile created on {os} must run here"
      );
    }
    // A claimed OS with no recorded creating machine is not a reason either.
    let mut claimed = profile_from("wayfern", None);
    claimed.wayfern_config = Some(WayfernConfig {
      os: Some(other_host_os().to_string()),
      ..WayfernConfig::default()
    });
    assert!(!claimed.is_cross_os());
  }

  #[test]
  fn a_profile_of_another_browser_stays_on_its_own_os() {
    let foreign = profile_from("camoufox", Some(other_host_os()));
    assert!(!foreign.runs_on_any_os());
    assert!(foreign.is_cross_os());
    assert!(!profile_from("camoufox", Some(&get_host_os())).is_cross_os());
    assert!(!profile_from("camoufox", None).is_cross_os());
  }

  #[test]
  fn native_os_is_the_creating_machine_not_this_one() {
    let other = other_host_os();
    assert_eq!(profile_from("wayfern", Some(other)).native_os(), other);
    assert_eq!(profile_from("wayfern", None).native_os(), get_host_os());
    // A fingerprint OS stored by mistake is not a machine.
    assert_eq!(
      profile_from("wayfern", Some("android")).native_os(),
      get_host_os()
    );
  }
}

//! Detects VPN/proxy browser extensions present in a profile.
//!
//! An extension holding Chromium's `proxy` permission can override the proxy
//! Donut passes on the command line, so the browser's real exit stops being the
//! one Donut measured and generated the fingerprint against. That produces
//! exactly the geo/timezone/language mismatch the fingerprint exists to avoid,
//! except Donut cannot observe it from the outside — hence a launch-time
//! warning rather than a measurement.
//!
//! That permission is a capability, not an identity. Chromium exposes no
//! read-only variant of it, so a download manager replicating the browser's
//! route for its own transfers declares exactly what a VPN hijacking it
//! declares. The two are reported as different things — see `rules::classify`.
//!
//! Two sources, deliberately both: Donut-managed extensions live in the app's
//! own store and are handed to Chromium via `--load-extension` from *outside*
//! the profile directory, while extensions the user installed from the Web
//! Store live *inside* it. Neither set appears in the other.

mod browser_scan;
mod rules;

// `message_placeholder_key`/`lookup_message` are shared with
// `extension_manager`, which resolves the same placeholders out of a zip.
use rules::{classify, manifest_str, signal_labels, signals_from_manifest, vpn_keyword_hit};
pub use rules::{lookup_message, message_placeholder_key, DetectedVpnExtension};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Instant;

use crate::profile::types::BrowserProfile;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionScan {
  pub extensions: Vec<DetectedVpnExtension>,
  /// `scanned` | `partial` | `encrypted` | `ephemeral` | `missing`.
  ///
  /// Reported honestly so the dialog can say the scan was incomplete rather
  /// than implying a clean profile it never managed to read.
  pub scan_state: String,
}

/// Donut-managed extensions, reached through the profile's extension group.
///
/// Read live from the stored archive rather than from the metadata cached on
/// `Extension`, so replacing an extension's file cannot leave a stale verdict
/// behind. N is the group size — typically a handful.
fn scan_donut_extensions(profile: &BrowserProfile, out: &mut Vec<DetectedVpnExtension>) {
  let Some(group_id) = &profile.extension_group_id else {
    return;
  };
  let Ok(manager) = crate::extension_manager::EXTENSION_MANAGER.lock() else {
    log::warn!("VPN extension scan: extension manager lock poisoned, skipping managed extensions");
    return;
  };
  let Ok(group) = manager.get_group(group_id) else {
    return;
  };

  for ext_id in &group.extension_ids {
    let Ok(ext) = manager.get_extension(ext_id) else {
      continue;
    };
    let path = manager.get_file_dir_public(ext_id).join(&ext.file_name);
    let Ok(data) = std::fs::read(&path) else {
      continue;
    };
    let Some(manifest) =
      crate::extension_manager::read_manifest_from_archive(&data, &ext.file_type)
    else {
      continue;
    };

    let raw_name = manifest_str(&manifest, "name").unwrap_or_else(|| ext.name.clone());
    let name =
      crate::extension_manager::resolve_archive_i18n(&data, &ext.file_type, &manifest, &raw_name)
        .unwrap_or_else(|| {
          // An unresolvable placeholder is not a name — fall back to the one
          // the extension carries in Donut.
          if message_placeholder_key(&raw_name).is_some() {
            ext.name.clone()
          } else {
            raw_name.clone()
          }
        });
    let description = manifest_str(&manifest, "description").and_then(|d| {
      crate::extension_manager::resolve_archive_i18n(&data, &ext.file_type, &manifest, &d).or(
        if message_placeholder_key(&d).is_some() {
          None
        } else {
          Some(d)
        },
      )
    });

    let signals = signals_from_manifest(&manifest);
    let keyword = vpn_keyword_hit(&name, description.as_deref());
    // A Donut-managed extension is stored under Donut's own uuid, not the Web
    // Store id the known-VPN list is keyed on, so it is classified on what its
    // manifest says about itself.
    let Some(confidence) = classify(None, &signals, keyword) else {
      continue;
    };

    out.push(DetectedVpnExtension {
      key: format!("donut:{ext_id}"),
      name,
      version: manifest_str(&manifest, "version").or_else(|| ext.version.clone()),
      source: "donut".to_string(),
      confidence: confidence.to_string(),
      proxy_control: signals.proxy_permission,
      signals: signal_labels(None, &signals, keyword),
    });
  }
}

/// Scan a profile for VPN/proxy extensions from both sources.
///
/// Never fails: an unreadable profile reports whatever it could see plus a
/// `scan_state` explaining why the picture is incomplete.
pub fn scan_profile(profile: &BrowserProfile) -> ExtensionScan {
  let started = Instant::now();
  let mut extensions = Vec::new();

  scan_donut_extensions(profile, &mut extensions);

  let profiles_dir = crate::app_dirs::profiles_dir();
  let user_data_dir = crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);

  // `get_effective_profile_path` only returns the decrypted RAM copy while the
  // profile is unlocked; locked, it falls back to the on-disk directory, which
  // exists but is ciphertext. Walking that finds nothing — so the state has to
  // be decided on whether we actually got a readable copy, not on the path
  // existing, or a locked profile reports as verified-clean.
  let has_plaintext_dir = !(profile.password_protected || profile.ephemeral)
    || crate::ephemeral_dirs::get_ephemeral_dir(&profile.id.to_string()).is_some();

  let scan_state = if !has_plaintext_dir {
    if profile.password_protected {
      "encrypted"
    } else {
      "ephemeral"
    }
  } else if !user_data_dir.is_dir() {
    // Never launched, so there is no profile directory to inspect yet.
    "missing"
  } else if browser_scan::scan_browser_extensions(&user_data_dir, &mut extensions, started) {
    "scanned"
  } else {
    "partial"
  };

  // Collapse only exact duplicates of the same extension. `key` is the real
  // identity (`donut:<uuid>` / `crx:<id>`); name+version is not, and two
  // distinct extensions sharing a display name would silently fold into one,
  // hiding a real detection behind an unrelated namesake.
  let mut seen = HashSet::new();
  extensions.retain(|e| seen.insert(e.key.clone()));

  ExtensionScan {
    extensions,
    scan_state: scan_state.to_string(),
  }
}

/// True when at least one extension holds the `proxy` permission outright, so
/// it can redirect the browser's traffic without asking for anything further.
///
/// Informational: it tells the user an exit measurement may describe a route
/// the browser will not take. It deliberately does not relax the gate — a
/// measurement that might be wrong is a reason for more scrutiny, not less,
/// and this signal is true for every download manager on the machine.
pub fn has_proxy_control(scan: &ExtensionScan) -> bool {
  scan.extensions.iter().any(|e| e.proxy_control)
}

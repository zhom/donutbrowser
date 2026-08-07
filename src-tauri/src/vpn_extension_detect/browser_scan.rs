//! Enumerates extensions the user installed from inside the browser, by
//! walking the Chromium profile directory on disk.
//!
//! Depends only on `std`, `serde_json`, the sibling `rules` module, and the
//! shared profile-directory-name predicate, so the directory-layout handling —
//! the riskiest part of detection — stays testable without app state.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::rules::{
  classify, keyword_hit, lookup_message, manifest_str, message_placeholder_key, signal_labels,
  signals_from_manifest, version_dir_sort_key, DetectedVpnExtension,
};

/// Upper bound on extension directories walked per profile. A launch must not
/// stall behind a pathological profile; whatever was found is still reported,
/// flagged `partial`.
const MAX_EXTENSION_DIRS: usize = 300;
/// Manifests are a few KiB. Anything past this is not a manifest we can use.
const MAX_MANIFEST_BYTES: u64 = 512 * 1024;
/// Wall-clock ceiling for the whole scan. This runs on the launch path.
const SCAN_DEADLINE: Duration = Duration::from_millis(750);
/// Chromium preference files are larger than a manifest but still bounded; a
/// pathological one must not be parsed while the user waits for a browser.
const MAX_PREFERENCES_BYTES: u64 = 32 * 1024 * 1024;

/// Chromium profile directories to search inside a user-data dir.
///
/// Two layouts are real here: Donut launches Wayfern with only
/// `--user-data-dir`, so Chromium uses `Default/`; but an imported profile is
/// copied in as the profile directory itself, putting `Extensions/` at the
/// root. Checking only one layout misses every profile of the other kind.
fn candidate_profile_dirs(user_data_dir: &Path) -> Vec<PathBuf> {
  let mut dirs = Vec::new();

  if user_data_dir.join("Preferences").exists() || user_data_dir.join("Extensions").is_dir() {
    dirs.push(user_data_dir.to_path_buf());
  }

  if let Ok(entries) = std::fs::read_dir(user_data_dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if !path.is_dir() {
        continue;
      }
      let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        continue;
      };
      if crate::profile::clear_on_close::is_profile_dir_name(name)
        || path.join("Preferences").exists()
      {
        dirs.push(path);
      }
    }
  }

  dirs.sort();
  dirs.dedup();
  dirs
}

fn read_json_file(path: &Path, max_bytes: Option<u64>) -> Option<serde_json::Value> {
  let metadata = std::fs::metadata(path).ok()?;
  if !metadata.is_file() {
    return None;
  }
  if max_bytes.is_some_and(|cap| metadata.len() > cap) {
    return None;
  }
  serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Chromium locale directory names are `[A-Za-z0-9_-]` (`en`, `en_GB`,
/// `zh_CN`). Anything else in a manifest we did not write is untrusted input
/// being joined into a filesystem path, so it is refused rather than sanitized
/// — this runs on the launch path against extensions the user may have
/// sideloaded.
fn is_safe_locale_name(name: &str) -> bool {
  !name.is_empty()
    && name.len() <= 32
    && name
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn resolve_dir_i18n(
  version_dir: &Path,
  manifest: &serde_json::Value,
  value: &str,
) -> Option<String> {
  let key = message_placeholder_key(value)?;
  let default_locale = manifest.get("default_locale")?.as_str()?;
  if !is_safe_locale_name(default_locale) {
    log::warn!("Ignoring extension with a suspicious default_locale: {default_locale:?}");
    return None;
  }
  let messages = read_json_file(
    &version_dir
      .join("_locales")
      .join(default_locale)
      .join("messages.json"),
    Some(MAX_MANIFEST_BYTES),
  )?;
  lookup_message(&messages, &key)
}

/// What the profile's preference files say about installed extensions.
///
/// Read-only on purpose: `Secure Preferences` is MAC-protected, and rewriting
/// it invalidates the signature, which disables every extension in the profile.
#[derive(Default)]
struct PreferenceExtensions {
  /// Explicitly disabled. A disabled extension cannot touch the proxy, so
  /// warning about it would be a false alarm.
  disabled: HashSet<String>,
  /// Unpacked ("Load unpacked" / developer mode) extensions, which live
  /// OUTSIDE `Extensions/` and are therefore invisible to the directory walk.
  /// Sideloading is exactly how someone gets a VPN extension in without the
  /// Web Store, so missing these would leave the obvious hole open.
  unpacked: Vec<(String, PathBuf)>,
}

fn preference_extensions(profile_dir: &Path) -> PreferenceExtensions {
  let mut out = PreferenceExtensions::default();
  for file in ["Secure Preferences", "Preferences"] {
    // Larger than a manifest, but still capped: this is parsed while the user
    // waits for a browser to start.
    let Some(prefs) = read_json_file(&profile_dir.join(file), Some(MAX_PREFERENCES_BYTES)) else {
      continue;
    };
    let Some(settings) = prefs
      .get("extensions")
      .and_then(|e| e.get("settings"))
      .and_then(|s| s.as_object())
    else {
      continue;
    };
    for (id, entry) in settings {
      // Chromium's Extension::State: 0 = disabled.
      if entry.get("state").and_then(serde_json::Value::as_i64) == Some(0) {
        out.disabled.insert(id.clone());
        continue;
      }
      // A packed extension's `path` is relative to Extensions/; an unpacked
      // one records an absolute path elsewhere on disk.
      if let Some(path) = entry.get("path").and_then(|p| p.as_str()) {
        let path = PathBuf::from(path);
        if path.is_absolute() {
          out.unpacked.push((id.clone(), path));
        }
      }
    }
  }
  out
}

/// Walk a user-data dir for VPN/proxy extensions.
///
/// Returns false when the walk was cut short by a cap or the deadline, so the
/// caller can report the scan as incomplete rather than clean.
pub(super) fn scan_browser_extensions(
  user_data_dir: &Path,
  out: &mut Vec<DetectedVpnExtension>,
  started: Instant,
) -> bool {
  let mut walked = 0usize;

  for profile_dir in candidate_profile_dirs(user_data_dir) {
    // Read preferences first: unpacked extensions live outside Extensions/, so
    // a profile that has only sideloaded ones has no Extensions/ dir at all and
    // must not be skipped before they are considered.
    if started.elapsed() > SCAN_DEADLINE {
      return false;
    }
    let prefs = preference_extensions(&profile_dir);
    let disabled = &prefs.disabled;

    for (crx_id, unpacked_dir) in &prefs.unpacked {
      if walked >= MAX_EXTENSION_DIRS || started.elapsed() > SCAN_DEADLINE {
        return false;
      }
      walked += 1;
      if let Some(found) = detect_in_version_dir(crx_id, unpacked_dir) {
        out.push(found);
      }
    }

    let Ok(entries) = std::fs::read_dir(profile_dir.join("Extensions")) else {
      continue;
    };

    for entry in entries.flatten() {
      if walked >= MAX_EXTENSION_DIRS || started.elapsed() > SCAN_DEADLINE {
        return false;
      }
      walked += 1;

      let ext_dir = entry.path();
      if !ext_dir.is_dir() {
        continue;
      }
      let Some(crx_id) = ext_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
      else {
        continue;
      };
      if disabled.contains(&crx_id) {
        continue;
      }

      // Several versions can coexist on disk; Chromium runs the highest.
      let Ok(version_entries) = std::fs::read_dir(&ext_dir) else {
        continue;
      };
      let mut versions: Vec<PathBuf> = version_entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
      versions.sort_by_key(|p| {
        p.file_name()
          .and_then(|n| n.to_str())
          .map(version_dir_sort_key)
          .unwrap_or_default()
      });
      let Some(version_dir) = versions.last() else {
        continue;
      };

      if let Some(found) = detect_in_version_dir(&crx_id, version_dir) {
        out.push(found);
      }
    }
  }

  true
}

/// Classify the extension whose unpacked files live in `version_dir`.
/// Shared by the packed walk and the unpacked (developer-mode) entries.
fn detect_in_version_dir(crx_id: &str, version_dir: &Path) -> Option<DetectedVpnExtension> {
  let manifest = read_json_file(&version_dir.join("manifest.json"), Some(MAX_MANIFEST_BYTES))?;

  let raw_name = manifest_str(&manifest, "name").unwrap_or_else(|| crx_id.to_string());
  let name = resolve_dir_i18n(version_dir, &manifest, &raw_name).unwrap_or_else(|| {
    if message_placeholder_key(&raw_name).is_some() {
      crx_id.to_string()
    } else {
      raw_name.clone()
    }
  });
  let description = manifest_str(&manifest, "description").and_then(|d| {
    resolve_dir_i18n(version_dir, &manifest, &d).or(if message_placeholder_key(&d).is_some() {
      None
    } else {
      Some(d)
    })
  });

  let signals = signals_from_manifest(&manifest);
  let keyword = keyword_hit(&name, description.as_deref());
  let confidence = classify(&signals, keyword)?;

  Some(DetectedVpnExtension {
    key: format!("crx:{crx_id}"),
    name,
    version: manifest_str(&manifest, "version"),
    source: "browser".to_string(),
    confidence: confidence.to_string(),
    signals: signal_labels(&signals, keyword),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;

  fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
  }

  /// Build a Chromium `Preferences` blob for one extension.
  ///
  /// Serialized rather than string-interpolated on purpose: a Windows path is
  /// `C:\Users\...`, and pasting it into a JSON string literal produces invalid
  /// escape sequences, so the file silently fails to parse and every assertion
  /// about it passes for the wrong reason.
  fn preferences_json(crx_id: &str, state: i64, path: &Path) -> String {
    serde_json::json!({
      "extensions": {
        "settings": {
          crx_id: { "state": state, "path": path.to_string_lossy() }
        }
      }
    })
    .to_string()
  }

  const VPN_MANIFEST: &str = r#"{"name":"Turbo VPN","version":"2.1.0","permissions":["proxy"]}"#;
  const CRX_ID: &str = "abcdefghijklmnopabcdefghijklmnop";

  #[test]
  fn candidate_dirs_accepts_the_default_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("Default").join("Preferences"), "{}");
    assert_eq!(candidate_profile_dirs(root), vec![root.join("Default")]);
  }

  #[test]
  fn candidate_dirs_accepts_the_imported_root_layout() {
    // profile_importer copies a Chromium profile dir in as the root, so
    // Preferences and Extensions/ sit at the top level.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("Preferences"), "{}");
    assert_eq!(candidate_profile_dirs(root), vec![root.to_path_buf()]);
  }

  #[test]
  fn candidate_dirs_finds_named_profile_dirs_without_preferences() {
    // Chromium writes Preferences lazily, so a populated Default/ can lack it.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Profile 2").join("Extensions")).unwrap();
    assert_eq!(candidate_profile_dirs(root), vec![root.join("Profile 2")]);
  }

  #[test]
  fn scan_finds_a_vpn_extension_in_the_default_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
      &root
        .join("Default")
        .join("Extensions")
        .join(CRX_ID)
        .join("2.1.0_0")
        .join("manifest.json"),
      VPN_MANIFEST,
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].key, format!("crx:{CRX_ID}"));
    assert_eq!(out[0].name, "Turbo VPN");
    assert_eq!(out[0].confidence, "confirmed");
    assert_eq!(out[0].source, "browser");
  }

  #[test]
  fn scan_finds_a_vpn_extension_in_the_imported_root_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("Preferences"), "{}");
    write(
      &root
        .join("Extensions")
        .join(CRX_ID)
        .join("2.1.0_0")
        .join("manifest.json"),
      VPN_MANIFEST,
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert_eq!(out.len(), 1);
  }

  #[test]
  fn scan_reads_the_highest_version_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let ext = root.join("Default").join("Extensions").join(CRX_ID);
    // Lexicographically "1.9.0_0" > "1.10.0_0"; numerically it is not.
    write(
      &ext.join("1.9.0_0").join("manifest.json"),
      r#"{"name":"Old","version":"1.9.0","permissions":["storage"]}"#,
    );
    write(
      &ext.join("1.10.0_0").join("manifest.json"),
      r#"{"name":"New VPN","version":"1.10.0","permissions":["proxy"]}"#,
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].version.as_deref(), Some("1.10.0"));
  }

  #[test]
  fn scan_skips_extensions_chromium_has_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let profile = root.join("Default");
    write(
      &profile
        .join("Extensions")
        .join(CRX_ID)
        .join("2.1.0_0")
        .join("manifest.json"),
      VPN_MANIFEST,
    );
    write(
      &profile.join("Secure Preferences"),
      &format!(r#"{{"extensions":{{"settings":{{"{CRX_ID}":{{"state":0}}}}}}}}"#),
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert!(
      out.is_empty(),
      "a disabled extension cannot touch the proxy"
    );
  }

  #[test]
  fn scan_keeps_enabled_extensions_listed_in_preferences() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let profile = root.join("Default");
    write(
      &profile
        .join("Extensions")
        .join(CRX_ID)
        .join("2.1.0_0")
        .join("manifest.json"),
      VPN_MANIFEST,
    );
    write(
      &profile.join("Secure Preferences"),
      &format!(r#"{{"extensions":{{"settings":{{"{CRX_ID}":{{"state":1}}}}}}}}"#),
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert_eq!(out.len(), 1);
  }

  #[test]
  fn scan_resolves_a_localized_extension_name() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let version_dir = root
      .join("Default")
      .join("Extensions")
      .join(CRX_ID)
      .join("2.1.0_0");
    write(
      &version_dir.join("manifest.json"),
      r#"{"name":"__MSG_appName__","version":"2.1.0","default_locale":"en","permissions":["proxy"]}"#,
    );
    write(
      &version_dir
        .join("_locales")
        .join("en")
        .join("messages.json"),
      r#"{"appName":{"message":"Nord VPN"}}"#,
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert_eq!(out[0].name, "Nord VPN");
  }

  #[test]
  fn scan_falls_back_to_the_crx_id_when_a_placeholder_cannot_be_resolved() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
      &root
        .join("Default")
        .join("Extensions")
        .join(CRX_ID)
        .join("2.1.0_0")
        .join("manifest.json"),
      r#"{"name":"__MSG_appName__","version":"2.1.0","permissions":["proxy"]}"#,
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert_eq!(out[0].name, CRX_ID, "never show a raw __MSG_ placeholder");
  }

  #[test]
  fn scan_ignores_an_ordinary_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
      &root
        .join("Default")
        .join("Extensions")
        .join(CRX_ID)
        .join("1.0.0_0")
        .join("manifest.json"),
      r#"{"name":"Dark Reader","version":"1.0.0","permissions":["storage","activeTab"]}"#,
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert!(out.is_empty());
  }

  #[test]
  fn scan_reports_incomplete_when_the_deadline_has_passed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
      &root
        .join("Default")
        .join("Extensions")
        .join(CRX_ID)
        .join("2.1.0_0")
        .join("manifest.json"),
      VPN_MANIFEST,
    );

    let mut out = Vec::new();
    // A start time already past the deadline stands in for a slow disk.
    let expired = Instant::now() - SCAN_DEADLINE - Duration::from_millis(10);
    assert!(!scan_browser_extensions(root, &mut out, expired));
  }

  #[test]
  fn scan_survives_a_malformed_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
      &root
        .join("Default")
        .join("Extensions")
        .join(CRX_ID)
        .join("2.1.0_0")
        .join("manifest.json"),
      "{ not json",
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert!(out.is_empty());
  }

  #[test]
  fn preferences_json_escapes_windows_style_paths() {
    // The Windows CI failure this guards: a raw `C:\Users\...` pasted into a
    // JSON string literal is invalid (`\U` is not an escape), so Preferences
    // failed to parse, `preference_extensions` returned nothing, and the
    // unpacked extension silently vanished — on Linux the same test passed
    // because POSIX paths contain no backslashes.
    let json = preferences_json(CRX_ID, 1, Path::new(r"C:\Users\runner\ext\my-vpn"));
    let parsed: serde_json::Value =
      serde_json::from_str(&json).expect("Preferences must be valid JSON on every platform");
    assert_eq!(
      parsed["extensions"]["settings"][CRX_ID]["path"],
      r"C:\Users\runner\ext\my-vpn"
    );
  }

  #[test]
  fn scan_finds_an_unpacked_developer_mode_extension() {
    // Sideloading via "Load unpacked" is exactly how a VPN extension gets in
    // without the Web Store, and those files live outside Extensions/.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let unpacked = root.join("somewhere-else").join("my-vpn");
    write(&unpacked.join("manifest.json"), VPN_MANIFEST);
    write(
      &root.join("Default").join("Preferences"),
      &preferences_json(CRX_ID, 1, &unpacked),
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert_eq!(out.len(), 1, "unpacked extensions must not be invisible");
    assert_eq!(out[0].key, format!("crx:{CRX_ID}"));
    assert_eq!(out[0].confidence, "confirmed");
  }

  #[test]
  fn scan_ignores_a_disabled_unpacked_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let unpacked = root.join("somewhere-else").join("my-vpn");
    write(&unpacked.join("manifest.json"), VPN_MANIFEST);
    write(
      &root.join("Default").join("Preferences"),
      &preferences_json(CRX_ID, 0, &unpacked),
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert!(out.is_empty());
  }

  #[test]
  fn packed_relative_paths_are_not_treated_as_unpacked() {
    // A packed extension records a path relative to Extensions/; following it
    // as if absolute would read the wrong place (or nothing).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
      &root.join("Default").join("Preferences"),
      &preferences_json(CRX_ID, 1, Path::new(&format!("{CRX_ID}/2.1.0_0"))),
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    assert!(out.is_empty());
  }

  #[test]
  fn a_traversing_default_locale_is_refused() {
    // `default_locale` comes from a manifest we did not write. Joined naively
    // it reads any file the user can read, on the launch path.
    assert!(!is_safe_locale_name("../../../../etc"));
    assert!(!is_safe_locale_name("..\\..\\windows"));
    assert!(!is_safe_locale_name("/etc/passwd"));
    assert!(!is_safe_locale_name(""));
    assert!(is_safe_locale_name("en"));
    assert!(is_safe_locale_name("en_GB"));
    assert!(is_safe_locale_name("zh-CN"));
  }

  #[test]
  fn a_localized_name_with_a_traversing_locale_is_not_resolved() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let version_dir = root
      .join("Default")
      .join("Extensions")
      .join(CRX_ID)
      .join("2.1.0_0");
    write(
      &version_dir.join("manifest.json"),
      r#"{"name":"__MSG_appName__","version":"2.1.0","default_locale":"../../../../etc","permissions":["proxy"]}"#,
    );

    let mut out = Vec::new();
    assert!(scan_browser_extensions(root, &mut out, Instant::now()));
    // Still detected (the proxy permission is what matters), but the name
    // falls back rather than the traversal being followed.
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].name, CRX_ID);
  }

  #[test]
  fn scan_tolerates_a_missing_user_data_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    assert!(scan_browser_extensions(
      &tmp.path().join("nope"),
      &mut out,
      Instant::now()
    ));
    assert!(out.is_empty());
  }
}

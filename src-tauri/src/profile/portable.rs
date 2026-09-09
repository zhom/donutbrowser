//! Moving a profile between machines.
//!
//! An export is one zip: a manifest, the profile's configuration, and
//! optionally its browser data directory. An import creates a NEW profile from
//! it, with a fresh id, so importing an export twice gives two profiles rather
//! than a conflict or a silent overwrite.
//!
//! What deliberately does NOT travel:
//! - the process id, the last-launch time and the cloud-sync bookkeeping,
//!   which describe the machine that exported, not the profile;
//! - the caches, which Chromium rebuilds and which are most of the bytes;
//! - the browser binary, which the importing machine downloads for itself;
//! - a password-protected profile's data, because its at-rest key belongs to
//!   the exporting machine's keychain and the bytes would be unreadable
//!   anywhere else. Its configuration exports, its data does not, and the
//!   export says so rather than shipping an archive nobody can open.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::profile::types::BrowserProfile;

/// Bumped when the archive layout changes in a way an older build cannot read.
const FORMAT_VERSION: u32 = 1;
const MANIFEST_ENTRY: &str = "manifest.json";
const PROFILE_ENTRY: &str = "profile.json";
const DATA_PREFIX: &str = "data/";
/// A profile directory is browsing history, cookies and extension state. Past
/// this size an export is almost certainly a mistake (a cache directory that
/// escaped the prune, say), and writing gigabytes to a user's Downloads folder
/// without saying why is worse than refusing.
const MAX_DATA_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// What one archive says about itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortableManifest {
  pub format_version: u32,
  /// The app that wrote it, for a bug report.
  pub exported_by: String,
  pub exported_at: u64,
  /// The profile's name at export time. The id is deliberately absent: an
  /// import mints a new one, and carrying the old id invites a caller to
  /// "restore" over a live profile.
  pub profile_name: String,
  pub browser: String,
  pub version: String,
  /// Whether `data/` is present. False for a configuration-only export and for
  /// a password-protected profile, whose bytes cannot travel.
  pub includes_data: bool,
  /// Why the data is absent, when it is.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub data_omitted_reason: Option<String>,
}

/// What an import found in an archive, before it creates anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortablePreview {
  pub manifest: PortableManifest,
  /// The proxy the exporting machine had assigned, by name, when it had one.
  /// An import never links a proxy by id: ids are local to a machine.
  pub proxy_name: Option<String>,
  pub group_name: Option<String>,
  pub tags: Vec<String>,
}

fn err(context: &str, detail: impl std::fmt::Display) -> String {
  crate::backend_error_with_detail("PROFILE_EXPORT_FAILED", format!("{context}: {detail}"))
}

fn import_err(detail: impl std::fmt::Display) -> String {
  crate::backend_error_with_detail("PROFILE_IMPORT_FAILED", detail.to_string())
}

/// The configuration an export carries: the profile as stored, minus
/// everything that describes this machine or this moment.
pub fn exportable_config(profile: &BrowserProfile) -> serde_json::Value {
  let mut value = serde_json::to_value(profile).unwrap_or(serde_json::Value::Null);
  if let Some(object) = value.as_object_mut() {
    for machine_local in [
      "id",
      "process_id",
      "last_launch",
      "last_sync",
      "encryption_salt",
      "created_by_id",
      "created_by_email",
      "proxy_id",
      "vpn_id",
      "group_id",
      "extension_group_id",
      "temporary",
    ] {
      object.remove(machine_local);
    }
    // A password-protected profile's data cannot travel, so the flag must not
    // either: an imported profile with the flag set and no key is unopenable.
    object.insert("password_protected".to_string(), serde_json::json!(false));
    object.insert("sync_mode".to_string(), serde_json::json!("Disabled"));
  }
  value
}

/// Every file under `dir`, relative to it, skipping the cache directories and
/// the launcher's own per-launch documents.
fn collect_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
  fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| err("could not read the profile directory", e))?;
    for entry in entries.flatten() {
      let path = entry.path();
      let Ok(relative) = path.strip_prefix(root) else {
        continue;
      };
      let relative = relative.to_string_lossy().replace('\\', "/");
      if is_excluded(&relative) {
        continue;
      }
      let file_type = entry
        .file_type()
        .map_err(|e| err("could not stat a file", e))?;
      if file_type.is_symlink() {
        // A symlink in an archive is either useless on the other machine or a
        // way out of the extraction directory. Neither travels.
        continue;
      }
      if file_type.is_dir() {
        walk(root, &path, out)?;
      } else if file_type.is_file() {
        out.push((relative, path));
      }
    }
    Ok(())
  }

  let mut files = Vec::new();
  walk(dir, dir, &mut files)?;
  files.sort_by(|a, b| a.0.cmp(&b.0));
  Ok(files)
}

/// Whether a path inside the profile directory is left out of an export.
pub fn is_excluded(relative: &str) -> bool {
  const CACHE_SEGMENTS: [&str; 10] = [
    "Cache",
    "Code Cache",
    "GPUCache",
    "GrShaderCache",
    "ShaderCache",
    "DawnCache",
    "DawnGraphiteCache",
    "GraphiteDawnCache",
    "CacheStorage",
    "ScriptCache",
  ];
  const LAUNCH_FILES: [&str; 3] = [
    "wayfern-identity.json",
    "wayfern-persona.json",
    "window-icon.png",
  ];
  const SINGLETONS: [&str; 3] = ["SingletonLock", "SingletonSocket", "SingletonCookie"];

  let segments: Vec<&str> = relative.split('/').collect();
  if segments
    .iter()
    .any(|segment| CACHE_SEGMENTS.contains(segment))
  {
    return true;
  }
  let Some(name) = segments.last() else {
    return true;
  };
  LAUNCH_FILES.contains(name) || SINGLETONS.contains(name) || name.ends_with(".tmp")
}

/// Write an export archive for `profile` to `destination`.
///
/// `data_dir` is the profile's browser directory; `include_data` false writes
/// a configuration-only archive, which is the small one worth emailing.
pub fn export_to(
  profile: &BrowserProfile,
  data_dir: &Path,
  destination: &Path,
  include_data: bool,
  proxy_name: Option<String>,
  group_name: Option<String>,
) -> Result<PortableManifest, String> {
  let data_omitted_reason = if !include_data {
    Some("the export was asked for without the browser data".to_string())
  } else if profile.password_protected {
    Some(
      "the profile is password protected, and its data is encrypted with a key held by the exporting machine".to_string(),
    )
  } else if !data_dir.is_dir() {
    Some("the profile has no browser data yet".to_string())
  } else {
    None
  };
  let carries_data = data_omitted_reason.is_none();

  let manifest = PortableManifest {
    format_version: FORMAT_VERSION,
    exported_by: format!("Donut Browser {}", env!("CARGO_PKG_VERSION")),
    exported_at: crate::proxy_manager::now_secs(),
    profile_name: profile.name.clone(),
    browser: profile.browser.clone(),
    version: profile.version.clone(),
    includes_data: carries_data,
    data_omitted_reason,
  };

  let mut preview = serde_json::to_value(&manifest).map_err(|e| err("manifest", e))?;
  if let Some(object) = preview.as_object_mut() {
    object.insert("proxy_name".to_string(), serde_json::json!(proxy_name));
    object.insert("group_name".to_string(), serde_json::json!(group_name));
  }

  if let Some(parent) = destination.parent() {
    fs::create_dir_all(parent).map_err(|e| err("could not create the destination folder", e))?;
  }
  let file = fs::File::create(destination).map_err(|e| err("could not create the archive", e))?;
  let mut writer = zip::ZipWriter::new(file);
  let options: zip::write::FileOptions<'_, ()> =
    zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

  writer
    .start_file(MANIFEST_ENTRY, options)
    .map_err(|e| err("manifest", e))?;
  writer
    .write_all(
      serde_json::to_string_pretty(&preview)
        .map_err(|e| err("manifest", e))?
        .as_bytes(),
    )
    .map_err(|e| err("manifest", e))?;

  writer
    .start_file(PROFILE_ENTRY, options)
    .map_err(|e| err("profile", e))?;
  writer
    .write_all(
      serde_json::to_string_pretty(&exportable_config(profile))
        .map_err(|e| err("profile", e))?
        .as_bytes(),
    )
    .map_err(|e| err("profile", e))?;

  if carries_data {
    let mut written = 0u64;
    for (relative, absolute) in collect_files(data_dir)? {
      let data = match fs::read(&absolute) {
        Ok(data) => data,
        // A browser file can vanish between the walk and the read; that is not
        // a reason to fail an export of everything else.
        Err(e) => {
          log::warn!("Skipping {} in the export: {e}", absolute.display());
          continue;
        }
      };
      written = written.saturating_add(data.len() as u64);
      if written > MAX_DATA_BYTES {
        return Err(crate::backend_error("PROFILE_EXPORT_TOO_LARGE"));
      }
      writer
        .start_file(format!("{DATA_PREFIX}{relative}"), options)
        .map_err(|e| err(&relative, e))?;
      writer.write_all(&data).map_err(|e| err(&relative, e))?;
    }
  }

  writer
    .finish()
    .map_err(|e| err("could not finish the archive", e))?;
  Ok(manifest)
}

fn open_archive(path: &Path) -> Result<zip::ZipArchive<fs::File>, String> {
  let file = fs::File::open(path).map_err(|e| import_err(format!("could not open it: {e}")))?;
  zip::ZipArchive::new(file).map_err(|e| import_err(format!("it is not a readable archive: {e}")))
}

fn read_entry<R: Read + Seek>(
  archive: &mut zip::ZipArchive<R>,
  name: &str,
) -> Result<String, String> {
  let mut entry = archive
    .by_name(name)
    .map_err(|_| import_err(format!("the archive has no {name}")))?;
  let mut body = String::new();
  entry
    .read_to_string(&mut body)
    .map_err(|e| import_err(format!("{name} could not be read: {e}")))?;
  Ok(body)
}

/// What an archive holds, without creating anything.
pub fn preview(path: &Path) -> Result<PortablePreview, String> {
  let mut archive = open_archive(path)?;
  let manifest_json = read_entry(&mut archive, MANIFEST_ENTRY)?;
  let manifest: PortableManifest = serde_json::from_str(&manifest_json)
    .map_err(|e| import_err(format!("its manifest is malformed: {e}")))?;
  if manifest.format_version > FORMAT_VERSION {
    return Err(crate::backend_error_with_detail(
      "PROFILE_IMPORT_TOO_NEW",
      manifest.format_version.to_string(),
    ));
  }
  let extra: serde_json::Value = serde_json::from_str(&manifest_json).unwrap_or_default();
  let profile_json = read_entry(&mut archive, PROFILE_ENTRY)?;
  let stored: serde_json::Value = serde_json::from_str(&profile_json)
    .map_err(|e| import_err(format!("its profile is malformed: {e}")))?;

  Ok(PortablePreview {
    manifest,
    proxy_name: extra["proxy_name"].as_str().map(str::to_string),
    group_name: extra["group_name"].as_str().map(str::to_string),
    tags: stored["tags"]
      .as_array()
      .map(|tags| {
        tags
          .iter()
          .filter_map(|tag| tag.as_str().map(str::to_string))
          .collect()
      })
      .unwrap_or_default(),
  })
}

/// The profile an import should create: the archive's configuration under a
/// fresh id and the given name, with nothing carried over from the exporting
/// machine.
pub fn imported_profile(
  archive_profile: &serde_json::Value,
  name: &str,
) -> Result<BrowserProfile, String> {
  let mut value = archive_profile.clone();
  let object = value
    .as_object_mut()
    .ok_or_else(|| import_err("its profile is not an object"))?;
  object.insert(
    "id".to_string(),
    serde_json::json!(uuid::Uuid::new_v4().to_string()),
  );
  object.insert("name".to_string(), serde_json::json!(name));
  object.insert("process_id".to_string(), serde_json::Value::Null);
  object.insert("last_launch".to_string(), serde_json::Value::Null);
  object.insert("last_sync".to_string(), serde_json::Value::Null);
  object.insert("encryption_salt".to_string(), serde_json::Value::Null);
  object.insert("password_protected".to_string(), serde_json::json!(false));
  object.insert("temporary".to_string(), serde_json::json!(false));
  object.insert("sync_mode".to_string(), serde_json::json!("Disabled"));
  object.insert(
    "host_os".to_string(),
    serde_json::json!(crate::profile::types::get_host_os()),
  );
  object.insert(
    "created_at".to_string(),
    serde_json::json!(crate::proxy_manager::now_secs()),
  );
  object.insert(
    "updated_at".to_string(),
    serde_json::json!(crate::proxy_manager::now_secs()),
  );
  serde_json::from_value(value).map_err(|e| import_err(format!("its profile is unusable: {e}")))
}

/// Extract the archive's `data/` into `data_dir`.
///
/// Every entry is checked to land inside `data_dir`: an archive is untrusted
/// input, and `../` in a name is how an extraction writes over a user's files.
pub fn extract_data(path: &Path, data_dir: &Path) -> Result<usize, String> {
  let mut archive = open_archive(path)?;
  fs::create_dir_all(data_dir)
    .map_err(|e| import_err(format!("could not create the profile directory: {e}")))?;
  let root = data_dir
    .canonicalize()
    .map_err(|e| import_err(format!("could not resolve the profile directory: {e}")))?;

  let mut restored = 0;
  for index in 0..archive.len() {
    let mut entry = archive
      .by_index(index)
      .map_err(|e| import_err(format!("could not read entry {index}: {e}")))?;
    if entry.is_dir() {
      continue;
    }
    let Some(name) = entry.enclosed_name() else {
      return Err(crate::backend_error("PROFILE_IMPORT_UNSAFE_ARCHIVE"));
    };
    let name = name.to_string_lossy().replace('\\', "/");
    let Some(relative) = name.strip_prefix(DATA_PREFIX) else {
      continue;
    };
    if relative.is_empty() || is_excluded(relative) {
      continue;
    }
    let destination = root.join(relative);
    if !destination.starts_with(&root) {
      return Err(crate::backend_error("PROFILE_IMPORT_UNSAFE_ARCHIVE"));
    }
    if let Some(parent) = destination.parent() {
      fs::create_dir_all(parent)
        .map_err(|e| import_err(format!("could not create {}: {e}", parent.display())))?;
    }
    let mut file = fs::File::create(&destination)
      .map_err(|e| import_err(format!("could not write {}: {e}", destination.display())))?;
    std::io::copy(&mut entry, &mut file)
      .map_err(|e| import_err(format!("could not write {}: {e}", destination.display())))?;
    restored += 1;
  }
  Ok(restored)
}

/// Pick a name no live profile carries: the archive's own when it is free,
/// otherwise `name (imported)`, `name (imported 2)`, and so on.
pub fn unique_imported_name(name: &str, taken: &[String]) -> String {
  let normalized: Vec<String> = taken.iter().map(|n| n.trim().to_lowercase()).collect();
  let is_taken = |candidate: &str| normalized.contains(&candidate.trim().to_lowercase());
  if !is_taken(name) {
    return name.to_string();
  }
  let mut attempt = 1u32;
  loop {
    let candidate = if attempt == 1 {
      format!("{name} (imported)")
    } else {
      format!("{name} (imported {attempt})")
    };
    if !is_taken(&candidate) {
      return candidate;
    }
    attempt += 1;
  }
}

/// Write an export of `profile_id` to `destination`.
#[tauri::command]
pub async fn export_profile(
  profile_id: String,
  destination: String,
  include_data: Option<bool>,
) -> Result<PortableManifest, String> {
  let manager = crate::profile::ProfileManager::instance();
  let profile = manager
    .list_profiles()
    .map_err(|e| err("could not read the profiles", e))?
    .into_iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or_else(|| crate::backend_error("PROFILE_NOT_FOUND"))?;
  // An export reads the whole profile directory; a browser writing to it at
  // the same time produces an archive of half-written databases.
  if profile
    .process_id
    .is_some_and(crate::proxy_storage::is_process_running)
  {
    return Err(crate::backend_error("PROFILE_RUNNING"));
  }

  let data_dir = manager
    .get_profiles_dir()
    .join(profile.id.to_string())
    .join("profile");
  let proxy_name = profile.proxy_id.as_deref().and_then(|id| {
    crate::proxy_manager::PROXY_MANAGER
      .get_stored_proxies()
      .into_iter()
      .find(|proxy| proxy.id == id)
      .map(|proxy| proxy.name)
  });
  let group_name = profile.group_id.as_deref().and_then(|id| {
    let manager = crate::group_manager::GROUP_MANAGER
      .lock()
      .unwrap_or_else(|poisoned| poisoned.into_inner());
    manager
      .get_all_groups()
      .ok()?
      .into_iter()
      .find(|group| group.id == id)
      .map(|group| group.name)
  });

  export_to(
    &profile,
    &data_dir,
    Path::new(&destination),
    include_data.unwrap_or(true),
    proxy_name,
    group_name,
  )
}

/// What an archive holds, so the user can decide before anything is created.
#[tauri::command]
pub fn preview_profile_archive(path: String) -> Result<PortablePreview, String> {
  preview(Path::new(&path))
}

/// Create a profile from an archive.
#[tauri::command]
pub async fn import_profile_archive(
  path: String,
  name: Option<String>,
) -> Result<BrowserProfile, String> {
  let archive_path = Path::new(&path);
  let details = preview(archive_path)?;
  let manager = crate::profile::ProfileManager::instance();
  let existing = manager
    .list_profiles()
    .map_err(|e| import_err(format!("could not read the profiles: {e}")))?;
  let taken: Vec<String> = existing.iter().map(|p| p.name.clone()).collect();
  let wanted = name
    .as_deref()
    .map(str::trim)
    .filter(|n| !n.is_empty())
    .unwrap_or(&details.manifest.profile_name);
  if wanted.is_empty() {
    return Err(crate::backend_error("NAME_CANNOT_BE_EMPTY"));
  }

  let mut archive = open_archive(archive_path)?;
  let stored: serde_json::Value =
    serde_json::from_str(&read_entry(&mut archive, PROFILE_ENTRY)?)
      .map_err(|e| import_err(format!("its profile is malformed: {e}")))?;
  drop(archive);

  let profile = imported_profile(&stored, &unique_imported_name(wanted, &taken))?;
  let data_dir = manager
    .get_profiles_dir()
    .join(profile.id.to_string())
    .join("profile");
  fs::create_dir_all(&data_dir)
    .map_err(|e| import_err(format!("could not create the profile directory: {e}")))?;

  if details.manifest.includes_data {
    if let Err(e) = extract_data(archive_path, &data_dir) {
      // Nothing half-imported is left behind: the profile was never saved, so
      // removing its directory removes every trace of the attempt.
      let _ = fs::remove_dir_all(data_dir.parent().unwrap_or(&data_dir));
      return Err(e);
    }
  }

  manager.save_profile(&profile).map_err(|e| {
    let _ = fs::remove_dir_all(data_dir.parent().unwrap_or(&data_dir));
    import_err(format!("could not save it: {e}"))
  })?;
  let _ = crate::events::emit("profiles-changed", serde_json::json!({}));
  Ok(profile)
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn sample() -> BrowserProfile {
    BrowserProfile {
      id: uuid::Uuid::new_v4(),
      name: "Shop".to_string(),
      browser: "wayfern".to_string(),
      version: "152.0.7977.64".to_string(),
      proxy_id: Some("proxy-1".to_string()),
      group_id: Some("group-1".to_string()),
      tags: vec!["eu".to_string()],
      process_id: Some(4242),
      last_launch: Some(1000),
      created_by_email: Some("someone@example.com".to_string()),
      ..BrowserProfile::default()
    }
  }

  fn seed_data(dir: &Path) {
    fs::create_dir_all(dir.join("Default/Network")).unwrap();
    fs::write(dir.join("Default/Network/Cookies"), b"cookie-db").unwrap();
    fs::write(dir.join("Local State"), b"{}").unwrap();
    fs::create_dir_all(dir.join("Default/Cache")).unwrap();
    fs::write(dir.join("Default/Cache/data_0"), vec![0u8; 4096]).unwrap();
    fs::write(dir.join("wayfern-identity.json"), b"{}").unwrap();
    fs::write(dir.join("SingletonLock"), b"lock").unwrap();
  }

  fn entries(path: &Path) -> Vec<String> {
    let mut archive = open_archive(path).unwrap();
    (0..archive.len())
      .map(|i| archive.by_index(i).unwrap().name().to_string())
      .collect()
  }

  #[test]
  fn an_export_carries_the_profile_and_its_data_but_not_the_machine() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("profile");
    seed_data(&data);
    let archive = root.path().join("shop.donutprofile");
    let profile = sample();

    let manifest = export_to(
      &profile,
      &data,
      &archive,
      true,
      Some("Residential EU".to_string()),
      Some("Clients".to_string()),
    )
    .unwrap();
    assert!(manifest.includes_data);
    assert_eq!(manifest.profile_name, "Shop");

    let names = entries(&archive);
    assert!(names.contains(&"manifest.json".to_string()));
    assert!(names.contains(&"profile.json".to_string()));
    assert!(names.contains(&"data/Default/Network/Cookies".to_string()));
    assert!(
      !names.iter().any(|n| n.contains("Cache")),
      "caches are rebuilt by the browser and must not travel: {names:?}"
    );
    assert!(
      !names.iter().any(|n| n.contains("wayfern-identity.json")),
      "the launcher rewrites its own documents every launch: {names:?}"
    );
    assert!(!names.iter().any(|n| n.contains("SingletonLock")));

    let preview = preview(&archive).unwrap();
    assert_eq!(preview.proxy_name.as_deref(), Some("Residential EU"));
    assert_eq!(preview.group_name.as_deref(), Some("Clients"));
    assert_eq!(preview.tags, vec!["eu".to_string()]);
  }

  #[test]
  fn the_exported_configuration_drops_what_belongs_to_this_machine() {
    let config = exportable_config(&sample());
    for gone in [
      "id",
      "process_id",
      "last_launch",
      "proxy_id",
      "group_id",
      "created_by_email",
      "encryption_salt",
    ] {
      assert!(config.get(gone).is_none(), "{gone} must not travel");
    }
    assert_eq!(config["password_protected"], serde_json::json!(false));
    assert_eq!(config["sync_mode"], serde_json::json!("Disabled"));
    assert_eq!(config["version"], serde_json::json!("152.0.7977.64"));
  }

  #[test]
  fn a_password_protected_profile_exports_its_configuration_and_says_why_not_its_data() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("profile");
    seed_data(&data);
    let archive = root.path().join("locked.donutprofile");
    let mut profile = sample();
    profile.password_protected = true;

    let manifest = export_to(&profile, &data, &archive, true, None, None).unwrap();
    assert!(!manifest.includes_data);
    assert!(manifest
      .data_omitted_reason
      .as_deref()
      .unwrap()
      .contains("password protected"));
    assert!(!entries(&archive).iter().any(|n| n.starts_with("data/")));
  }

  #[test]
  fn an_import_is_a_new_profile_that_owes_nothing_to_the_exporter() {
    let source = sample();
    let config = exportable_config(&source);
    let imported = imported_profile(&config, "Shop (imported)").unwrap();

    assert_ne!(imported.id, source.id);
    assert_eq!(imported.name, "Shop (imported)");
    assert_eq!(imported.version, source.version);
    assert_eq!(imported.tags, source.tags);
    assert_eq!(imported.process_id, None);
    assert_eq!(imported.proxy_id, None, "a proxy id is local to a machine");
    assert_eq!(imported.group_id, None);
    assert!(!imported.password_protected);
    assert!(!imported.temporary);
    assert!(imported.created_at.is_some());

    // Twice from one archive gives two profiles, not a conflict.
    let again = imported_profile(&config, "Shop (imported)").unwrap();
    assert_ne!(again.id, imported.id);
  }

  #[test]
  fn extraction_restores_the_data_and_refuses_to_escape_the_profile_directory() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("profile");
    seed_data(&data);
    let archive = root.path().join("shop.donutprofile");
    export_to(&sample(), &data, &archive, true, None, None).unwrap();

    let restored_dir = root.path().join("restored");
    let restored = extract_data(&archive, &restored_dir).unwrap();
    assert!(restored >= 2);
    assert_eq!(
      fs::read(restored_dir.join("Default/Network/Cookies")).unwrap(),
      b"cookie-db"
    );
    assert!(!restored_dir.join("Default/Cache/data_0").exists());

    // A hand-made archive with a traversing entry is refused outright.
    let hostile = root.path().join("hostile.donutprofile");
    {
      let file = fs::File::create(&hostile).unwrap();
      let mut writer = zip::ZipWriter::new(file);
      let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
      writer.start_file("manifest.json", options).unwrap();
      writer.write_all(b"{}").unwrap();
      writer.start_file("data/../../escaped", options).unwrap();
      writer.write_all(b"nope").unwrap();
      writer.finish().unwrap();
    }
    let target = root.path().join("target");
    assert!(extract_data(&hostile, &target)
      .unwrap_err()
      .contains("PROFILE_IMPORT_UNSAFE_ARCHIVE"));
    assert!(!root.path().join("escaped").exists());
  }

  #[test]
  fn an_archive_from_a_newer_build_is_refused_by_name() {
    let root = TempDir::new().unwrap();
    let archive = root.path().join("future.donutprofile");
    {
      let file = fs::File::create(&archive).unwrap();
      let mut writer = zip::ZipWriter::new(file);
      let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
      writer.start_file("manifest.json", options).unwrap();
      writer
        .write_all(
          serde_json::json!({
            "format_version": FORMAT_VERSION + 1,
            "exported_by": "Donut Browser 99.0.0",
            "exported_at": 1,
            "profile_name": "Future",
            "browser": "wayfern",
            "version": "999",
            "includes_data": false,
          })
          .to_string()
          .as_bytes(),
        )
        .unwrap();
      writer.finish().unwrap();
    }
    assert!(preview(&archive)
      .unwrap_err()
      .contains("PROFILE_IMPORT_TOO_NEW"));
  }

  #[test]
  fn a_file_that_is_not_an_archive_is_a_coded_error_not_a_panic() {
    let root = TempDir::new().unwrap();
    let bogus = root.path().join("notes.txt");
    fs::write(&bogus, b"just some text").unwrap();
    assert!(preview(&bogus)
      .unwrap_err()
      .contains("PROFILE_IMPORT_FAILED"));
  }
}

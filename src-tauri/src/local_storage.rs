//! A profile's Local Storage, read and written in the shape of Playwright's
//! `storageState`, so a signed-in session moves between Donut and automation
//! tools: `{ "cookies": [...], "origins": [{ "origin", "localStorage": [...] }] }`.
//!
//! Chromium keeps Local Storage in a LevelDB under
//! `Default/Local Storage/leveldb`:
//!
//! - `VERSION` is `1`;
//! - `META:<origin>` holds when the origin's data last changed and its size,
//!   `METAACCESS:<origin>` when it was last read (small protobufs);
//! - `_<origin>\0<key>` holds a value. A key and a value each start with a
//!   format byte: `0` for UTF-16LE, `1` for Latin-1.
//!
//! A read opens a copy of the folder, so it never changes the profile. A write
//! needs the browser closed: a running browser holds the database and would
//! lose the write or corrupt the files.

use crate::profile::{BrowserProfile, ProfileManager};
use rusty_leveldb::{CompressorId, LdbIterator, Options, WriteBatch, DB};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The most an import may carry, cookies included.
pub const MAX_IMPORT_BYTES: usize = 32 * 1024 * 1024;
/// Chromium's own limit per origin: five million UTF-16 code units.
const MAX_ORIGIN_BYTES: u64 = 10 * 1024 * 1024;
const MAX_ORIGINS: usize = 10_000;
/// Seconds from 1601-01-01 (Chromium's time origin) to the Unix epoch.
const CHROMIUM_EPOCH_OFFSET_SECS: i64 = 11_644_473_600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StorageItem {
  pub name: String,
  pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OriginStorage {
  /// `https://example.com`, the way a page's `origin` reads.
  pub origin: String,
  #[serde(rename = "localStorage", default)]
  pub local_storage: Vec<StorageItem>,
  /// The origin's IndexedDB databases, in Playwright's shape. Only present when
  /// an export asked for them or an import carries them.
  #[serde(rename = "indexedDB", default, skip_serializing_if = "Vec::is_empty")]
  #[schema(value_type = Vec<Object>)]
  pub indexed_db: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct LocalStorageStats {
  pub origins: usize,
  pub items: usize,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct StorageStateImport {
  pub origins: usize,
  pub items: usize,
  pub cookies_imported: usize,
  pub cookies_replaced: usize,
  /// Codes of the cookies that could not be imported.
  pub cookie_errors: Vec<String>,
  /// IndexedDB records written.
  pub indexed_db_records: u64,
}

#[derive(Debug, Deserialize)]
struct StorageState {
  #[serde(default)]
  origins: Vec<OriginStorage>,
  #[serde(default)]
  cookies: Vec<serde_json::Value>,
}

fn code(code: &str) -> String {
  crate::backend_error(code)
}

fn database_dir(profile: &BrowserProfile) -> PathBuf {
  profile
    .get_profile_data_path(&ProfileManager::instance().get_profiles_dir())
    .join("Default")
    .join("Local Storage")
    .join("leveldb")
}

/// The profile, when its Local Storage can be read at all: an ephemeral
/// profile keeps none on disk, and a password-protected one keeps it sealed.
fn readable_profile(profile_id: &str) -> Result<BrowserProfile, String> {
  let profile = ProfileManager::instance()
    .list_profiles()
    .map_err(|e| crate::wrap_backend_error(e, "Failed to list profiles"))?
    .into_iter()
    .find(|profile| profile.id.to_string() == profile_id)
    .ok_or_else(|| code("PROFILE_NOT_FOUND"))?;
  if profile.ephemeral {
    return Err(code("LOCAL_STORAGE_EPHEMERAL"));
  }
  if profile.password_protected {
    return Err(code("LOCAL_STORAGE_PROFILE_PROTECTED"));
  }
  Ok(profile)
}

// --- The database format ----------------------------------------------------

fn decode_string(bytes: &[u8]) -> Option<String> {
  let (format, rest) = bytes.split_first()?;
  match format {
    0 => {
      if rest.len() % 2 != 0 {
        return None;
      }
      let units: Vec<u16> = rest
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .collect();
      String::from_utf16(&units).ok()
    }
    1 => Some(rest.iter().map(|&b| char::from(b)).collect()),
    _ => None,
  }
}

/// Latin-1 when every character fits, as Chromium writes it; else UTF-16LE.
fn encode_string(text: &str) -> Vec<u8> {
  if text.chars().all(|c| (c as u32) <= 0xFF) {
    let mut out = Vec::with_capacity(text.len() + 1);
    out.push(1);
    out.extend(text.chars().map(|c| c as u8));
    out
  } else {
    let mut out = Vec::with_capacity(text.len() * 2 + 1);
    out.push(0);
    for unit in text.encode_utf16() {
      out.extend_from_slice(&unit.to_le_bytes());
    }
    out
  }
}

fn data_prefix(origin: &str) -> Vec<u8> {
  let mut key = Vec::with_capacity(origin.len() + 2);
  key.push(b'_');
  key.extend_from_slice(origin.as_bytes());
  key.push(0);
  key
}

fn data_key(origin: &str, name: &str) -> Vec<u8> {
  let mut key = data_prefix(origin);
  key.extend(encode_string(name));
  key
}

/// `(origin, name)` from a data key, or `None` for any other entry.
fn parse_data_key(key: &[u8]) -> Option<(String, String)> {
  let rest = key.strip_prefix(b"_")?;
  let split = rest.iter().position(|&b| b == 0)?;
  let origin = std::str::from_utf8(&rest[..split]).ok()?.to_string();
  let name = decode_string(&rest[split + 1..])?;
  Some((origin, name))
}

fn varint(mut value: u64, out: &mut Vec<u8>) {
  while value >= 0x80 {
    out.push((value as u8) | 0x80);
    value >>= 7;
  }
  out.push(value as u8);
}

/// Microseconds since 1601-01-01, Chromium's `Time` internal value.
fn chromium_now() -> i64 {
  let unix_micros = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_micros() as i64);
  unix_micros + CHROMIUM_EPOCH_OFFSET_SECS * 1_000_000
}

/// `LocalStorageAreaWriteMetaData { last_modified = 1; size_bytes = 2; }`.
fn write_meta(last_modified: i64, size_bytes: u64) -> Vec<u8> {
  let mut out = vec![0x08];
  varint(last_modified as u64, &mut out);
  out.push(0x10);
  varint(size_bytes, &mut out);
  out
}

/// `LocalStorageAreaAccessMetaData { last_accessed = 1; }`.
fn access_meta(last_accessed: i64) -> Vec<u8> {
  let mut out = vec![0x08];
  varint(last_accessed as u64, &mut out);
  out
}

/// What an item weighs against the per-origin limit: UTF-16 code units, two
/// bytes each, the way Chromium counts.
fn item_bytes(name: &str, value: &str) -> u64 {
  ((name.encode_utf16().count() + value.encode_utf16().count()) * 2) as u64
}

fn open(dir: &Path) -> Result<DB, String> {
  let options = Options {
    compressor: rusty_leveldb::compressor::SnappyCompressor::ID,
    ..Options::default()
  };
  DB::open(dir, options).map_err(|e| e.to_string())
}

fn read_database(dir: &Path) -> Result<Vec<OriginStorage>, String> {
  let mut db = open(dir)?;
  let mut iter = db.new_iter().map_err(|e| e.to_string())?;
  let mut origins: Vec<OriginStorage> = Vec::new();
  iter.seek(b"_");
  while iter.valid() {
    let Some((key, value)) = iter.current() else {
      break;
    };
    if !key.starts_with(b"_") {
      break;
    }
    if let (Some((origin, name)), Some(value)) = (parse_data_key(&key), decode_string(&value)) {
      match origins.last_mut() {
        Some(last) if last.origin == origin => last.local_storage.push(StorageItem { name, value }),
        _ => origins.push(OriginStorage {
          origin,
          local_storage: vec![StorageItem { name, value }],
          indexed_db: Vec::new(),
        }),
      }
    }
    if !iter.advance() {
      break;
    }
  }
  drop(iter);
  let _ = db.close();
  Ok(origins)
}

/// Write `origins` into the database at `dir`, creating it when the profile
/// has none yet. An item replaces the one of the same name; the origin's
/// other items stay.
fn write_database(dir: &Path, origins: &[OriginStorage]) -> Result<(), String> {
  std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
  let mut db = open(dir)?;
  let now = chromium_now();
  if db.get(b"VERSION").is_none() {
    db.put(b"VERSION", b"1").map_err(|e| e.to_string())?;
  }
  for origin in origins {
    let mut batch = WriteBatch::default();
    for item in &origin.local_storage {
      batch.put(
        &data_key(&origin.origin, &item.name),
        &encode_string(&item.value),
      );
    }
    db.write(batch, false).map_err(|e| e.to_string())?;

    let prefix = data_prefix(&origin.origin);
    let mut size = 0u64;
    let mut iter = db.new_iter().map_err(|e| e.to_string())?;
    iter.seek(&prefix);
    while iter.valid() {
      let Some((key, value)) = iter.current() else {
        break;
      };
      if !key.starts_with(&prefix) {
        break;
      }
      if let (Some((_, name)), Some(value)) = (parse_data_key(&key), decode_string(&value)) {
        size += item_bytes(&name, &value);
      }
      if !iter.advance() {
        break;
      }
    }
    drop(iter);
    if size > MAX_ORIGIN_BYTES {
      return Err(
        serde_json::json!({
          "code": "LOCAL_STORAGE_ORIGIN_TOO_LARGE",
          "params": { "origin": origin.origin }
        })
        .to_string(),
      );
    }
    let meta_key = format!("META:{}", origin.origin);
    let access_key = format!("METAACCESS:{}", origin.origin);
    db.put(meta_key.as_bytes(), &write_meta(now, size))
      .map_err(|e| e.to_string())?;
    db.put(access_key.as_bytes(), &access_meta(now))
      .map_err(|e| e.to_string())?;
  }
  db.flush().map_err(|e| e.to_string())?;
  db.close().map_err(|e| e.to_string())
}

/// A copy of the folder to read from, without the browser's lock file.
fn copy_database(from: &Path) -> Result<Option<tempfile::TempDir>, String> {
  if !from.exists() {
    return Ok(None);
  }
  let copy = tempfile::tempdir().map_err(|e| e.to_string())?;
  for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
    let entry = entry.map_err(|e| e.to_string())?;
    let name = entry.file_name();
    if name == "LOCK" || !entry.path().is_file() {
      continue;
    }
    std::fs::copy(entry.path(), copy.path().join(&name)).map_err(|e| e.to_string())?;
  }
  Ok(Some(copy))
}

/// `https://example.com`, from whatever spelling of an origin was imported.
fn normalized_origin(raw: &str) -> Result<String, String> {
  let invalid = || {
    serde_json::json!({
      "code": "LOCAL_STORAGE_ORIGIN_INVALID",
      "params": { "origin": raw.chars().take(200).collect::<String>() }
    })
    .to_string()
  };
  let url = url::Url::parse(raw.trim()).map_err(|_| invalid())?;
  if !matches!(url.scheme(), "http" | "https") {
    return Err(invalid());
  }
  let origin = url.origin();
  if !origin.is_tuple() {
    return Err(invalid());
  }
  Ok(origin.ascii_serialization())
}

// --- What the app and the API call -----------------------------------------

pub fn read(profile_id: &str) -> Result<Vec<OriginStorage>, String> {
  let profile = readable_profile(profile_id)?;
  let Some(copy) = copy_database(&database_dir(&profile))? else {
    return Ok(Vec::new());
  };
  read_database(copy.path())
    .map_err(|e| crate::backend_error_with_detail("LOCAL_STORAGE_UNREADABLE", e))
}

pub fn stats(profile_id: &str) -> Result<LocalStorageStats, String> {
  let origins = read(profile_id)?;
  Ok(LocalStorageStats {
    origins: origins.len(),
    items: origins.iter().map(|o| o.local_storage.len()).sum(),
  })
}

/// Playwright's cookie shape from a stored cookie.
fn playwright_cookie(cookie: &crate::cookie_manager::UnifiedCookie) -> serde_json::Value {
  let same_site = match cookie.same_site {
    0 => "None",
    2 => "Strict",
    // Unspecified behaves as Lax in Chromium, so Lax is what it travels as.
    _ => "Lax",
  };
  serde_json::json!({
    "name": cookie.name,
    "value": cookie.value,
    "domain": cookie.domain,
    "path": cookie.path,
    "expires": if cookie.expires == 0 { -1 } else { cookie.expires },
    "httpOnly": cookie.is_http_only,
    "secure": cookie.is_secure,
    "sameSite": same_site,
  })
}

pub fn export_storage_state(profile_id: &str, include_cookies: bool) -> Result<String, String> {
  let origins = read(profile_id)?;
  let cookies: Vec<serde_json::Value> = if include_cookies {
    crate::cookie_manager::CookieManager::read_cookies(profile_id)
      .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?
      .domains
      .iter()
      .flat_map(|domain| domain.cookies.iter().map(playwright_cookie))
      .collect()
  } else {
    Vec::new()
  };
  let count: usize = origins.iter().map(|o| o.local_storage.len()).sum();
  log::info!(
    "Storage state exported profile={profile_id} origins={} items={count} cookies={}",
    origins.len(),
    cookies.len()
  );
  serde_json::to_string_pretty(&serde_json::json!({ "cookies": cookies, "origins": origins }))
    .map_err(|e| e.to_string())
}

/// Check and tidy what an import carries, before anything is written.
fn checked_origins(origins: Vec<OriginStorage>) -> Result<Vec<OriginStorage>, String> {
  if origins.len() > MAX_ORIGINS {
    return Err(code("LOCAL_STORAGE_TOO_MANY_ORIGINS"));
  }
  let mut merged: Vec<OriginStorage> = Vec::new();
  for origin in origins {
    let name = normalized_origin(&origin.origin)?;
    let size: u64 = origin
      .local_storage
      .iter()
      .map(|item| item_bytes(&item.name, &item.value))
      .sum();
    if size > MAX_ORIGIN_BYTES {
      return Err(
        serde_json::json!({
          "code": "LOCAL_STORAGE_ORIGIN_TOO_LARGE",
          "params": { "origin": name }
        })
        .to_string(),
      );
    }
    crate::indexed_db::checked_databases(&origin.indexed_db)?;
    match merged.iter_mut().find(|o| o.origin == name) {
      Some(existing) => {
        existing.local_storage.extend(origin.local_storage);
        existing.indexed_db.extend(origin.indexed_db);
      }
      None => merged.push(OriginStorage {
        origin: name,
        local_storage: origin.local_storage,
        indexed_db: origin.indexed_db,
      }),
    }
  }
  Ok(merged)
}

pub async fn import_storage_state(
  app_handle: &tauri::AppHandle,
  profile_id: &str,
  content: &str,
) -> Result<StorageStateImport, String> {
  if content.len() > MAX_IMPORT_BYTES {
    return Err(code("LOCAL_STORAGE_IMPORT_TOO_LARGE"));
  }
  let state: StorageState =
    serde_json::from_str(content).map_err(|_| code("STORAGE_STATE_INVALID"))?;
  let profile = readable_profile(profile_id)?;
  let running = ProfileManager::instance()
    .check_browser_status(app_handle.clone(), &profile)
    .await
    .unwrap_or(false);
  if running {
    return Err(code("LOCAL_STORAGE_BROWSER_RUNNING"));
  }
  if crate::remote_handoff::state_for(profile_id).is_some() {
    return Err(code("LOCAL_STORAGE_REMOTE_SESSION"));
  }
  let origins = checked_origins(state.origins)?;
  let items: usize = origins.iter().map(|o| o.local_storage.len()).sum();

  let (mut cookies_imported, mut cookies_replaced, mut cookie_errors) = (0, 0, Vec::new());
  if !state.cookies.is_empty() {
    let cookies = serde_json::json!({ "cookies": state.cookies }).to_string();
    let result =
      crate::cookie_manager::CookieManager::import_cookies(app_handle, profile_id, &cookies)
        .await?;
    cookies_imported = result.cookies_imported;
    cookies_replaced = result.cookies_replaced;
    cookie_errors = result.errors;
  }

  let with_items: Vec<OriginStorage> = origins
    .iter()
    .filter(|o| !o.local_storage.is_empty())
    .cloned()
    .collect();
  if !with_items.is_empty() {
    let dir = database_dir(&profile);
    let to_write = with_items;
    tauri::async_runtime::spawn_blocking(move || write_database(&dir, &to_write))
      .await
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?
      .map_err(|e| {
        if e.starts_with('{') {
          e
        } else {
          crate::backend_error_with_detail("LOCAL_STORAGE_UNWRITABLE", e)
        }
      })?;
  }
  let databases: Vec<(String, Vec<serde_json::Value>)> = origins
    .iter()
    .filter(|o| !o.indexed_db.is_empty())
    .map(|o| (o.origin.clone(), o.indexed_db.clone()))
    .collect();
  let indexed_db_records = if databases.is_empty() {
    0
  } else {
    crate::indexed_db::with_headless_browser(app_handle, &profile, move |session| {
      Box::pin(async move { crate::indexed_db::import(session, &databases).await })
    })
    .await?
  };
  log::info!(
    "Storage state imported profile={profile_id} origins={} items={items} cookies={cookies_imported} indexed_db_records={indexed_db_records}",
    origins.len()
  );
  Ok(StorageStateImport {
    origins: origins.len(),
    items,
    cookies_imported,
    cookies_replaced,
    cookie_errors,
    indexed_db_records,
  })
}

#[tauri::command]
pub async fn get_profile_local_storage_stats(
  profile_id: String,
) -> Result<LocalStorageStats, String> {
  tauri::async_runtime::spawn_blocking(move || stats(&profile_id))
    .await
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?
}

/// The storage state, with IndexedDB when asked for: that part needs the
/// browser, so the profile must be closed and is opened headless for a moment.
pub async fn export_storage_state_with(
  app_handle: &tauri::AppHandle,
  profile_id: &str,
  include_cookies: bool,
  include_indexed_db: bool,
) -> Result<String, String> {
  let id = profile_id.to_string();
  let content =
    tauri::async_runtime::spawn_blocking(move || export_storage_state(&id, include_cookies))
      .await
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))??;
  if !include_indexed_db {
    return Ok(content);
  }
  let profile = readable_profile(profile_id)?;
  let running = ProfileManager::instance()
    .check_browser_status(app_handle.clone(), &profile)
    .await
    .unwrap_or(false);
  if running {
    return Err(code("LOCAL_STORAGE_BROWSER_RUNNING"));
  }
  if crate::remote_handoff::state_for(profile_id).is_some() {
    return Err(code("LOCAL_STORAGE_REMOTE_SESSION"));
  }
  let origins = crate::indexed_db::origins_on_disk(
    &profile.get_profile_data_path(&ProfileManager::instance().get_profiles_dir()),
  );
  if origins.is_empty() {
    return Ok(content);
  }
  let exported = crate::indexed_db::with_headless_browser(app_handle, &profile, move |session| {
    Box::pin(async move { crate::indexed_db::export(session, &origins).await })
  })
  .await?;
  let mut state: serde_json::Value = serde_json::from_str(&content)
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;
  let mut listed: Vec<OriginStorage> =
    serde_json::from_value(state["origins"].take()).unwrap_or_default();
  let databases: usize = exported.origins.iter().map(|(_, dbs)| dbs.len()).sum();
  for (origin, dbs) in exported.origins {
    match listed.iter_mut().find(|o| o.origin == origin) {
      Some(existing) => existing.indexed_db = dbs,
      None => listed.push(OriginStorage {
        origin,
        local_storage: Vec::new(),
        indexed_db: dbs,
      }),
    }
  }
  state["origins"] = serde_json::to_value(&listed).map_err(|e| e.to_string())?;
  log::info!(
    "IndexedDB exported profile={profile_id} databases={databases} skipped={}",
    exported.skipped
  );
  serde_json::to_string_pretty(&state).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_profile_storage_state(
  app_handle: tauri::AppHandle,
  profile_id: String,
  include_cookies: bool,
  include_indexed_db: Option<bool>,
) -> Result<String, String> {
  export_storage_state_with(
    &app_handle,
    &profile_id,
    include_cookies,
    include_indexed_db.unwrap_or(false),
  )
  .await
}

#[tauri::command]
pub async fn import_profile_storage_state(
  app_handle: tauri::AppHandle,
  profile_id: String,
  content: String,
) -> Result<StorageStateImport, String> {
  import_storage_state(&app_handle, &profile_id, &content).await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn strings_round_trip_in_both_formats() {
    for text in ["plain", "", "ümlaut ÿ", "日本語 🍩"] {
      let encoded = encode_string(text);
      assert_eq!(decode_string(&encoded).as_deref(), Some(text));
    }
    assert_eq!(encode_string("ab")[0], 1, "Latin-1 when it fits");
    assert_eq!(encode_string("日")[0], 0, "UTF-16 when it does not");
    assert_eq!(decode_string(&[0, 1]), None, "an odd UTF-16 length");
    assert_eq!(decode_string(&[9, 1]), None);
  }

  #[test]
  fn a_data_key_names_its_origin_and_item() {
    let key = data_key("https://example.com", "token");
    assert!(key.starts_with(b"_https://example.com\0\x01"));
    assert_eq!(
      parse_data_key(&key),
      Some(("https://example.com".to_string(), "token".to_string()))
    );
    assert_eq!(parse_data_key(b"META:https://example.com"), None);
  }

  #[test]
  fn meta_is_the_protobuf_chromium_reads() {
    assert_eq!(write_meta(1, 300), vec![0x08, 0x01, 0x10, 0xAC, 0x02]);
    assert_eq!(access_meta(150), vec![0x08, 0x96, 0x01]);
  }

  #[test]
  fn a_written_database_reads_back_and_keeps_other_items() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("leveldb");
    let item = |name: &str, value: &str| StorageItem {
      name: name.to_string(),
      value: value.to_string(),
    };
    write_database(
      &path,
      &[OriginStorage {
        origin: "https://shop.example".to_string(),
        local_storage: vec![item("cart", "3 items"), item("lang", "日本語")],
        indexed_db: Vec::new(),
      }],
    )
    .unwrap();
    write_database(
      &path,
      &[
        OriginStorage {
          origin: "https://shop.example".to_string(),
          local_storage: vec![item("cart", "empty")],
          indexed_db: Vec::new(),
        },
        OriginStorage {
          origin: "https://mail.example".to_string(),
          local_storage: vec![item("session", "abc")],
          indexed_db: Vec::new(),
        },
      ],
    )
    .unwrap();
    let copy = copy_database(&path).unwrap().unwrap();
    let read = read_database(copy.path()).unwrap();
    assert_eq!(read.len(), 2);
    let shop = read
      .iter()
      .find(|o| o.origin == "https://shop.example")
      .unwrap();
    assert_eq!(
      shop.local_storage,
      vec![item("cart", "empty"), item("lang", "日本語")]
    );
    let mut db = open(&path).unwrap();
    assert_eq!(db.get(b"VERSION").as_deref(), Some(&b"1"[..]));
    assert!(db.get(b"META:https://mail.example").is_some());
  }

  #[test]
  fn imported_origins_are_checked_and_spelled_as_a_page_reads_them() {
    let origin = |name: &str| OriginStorage {
      origin: name.to_string(),
      local_storage: vec![StorageItem {
        name: "k".to_string(),
        value: "v".to_string(),
      }],
      indexed_db: Vec::new(),
    };
    let tidy = checked_origins(vec![
      origin("https://Example.com:443/path"),
      origin("https://example.com"),
    ])
    .unwrap();
    assert_eq!(tidy.len(), 1);
    assert_eq!(tidy[0].origin, "https://example.com");
    assert_eq!(tidy[0].local_storage.len(), 2);
    for bad in ["file:///etc", "javascript:alert(1)", "not a url"] {
      assert!(checked_origins(vec![origin(bad)])
        .unwrap_err()
        .contains("LOCAL_STORAGE_ORIGIN_INVALID"));
    }
    let huge = OriginStorage {
      origin: "https://big.example".to_string(),
      local_storage: vec![StorageItem {
        name: "k".to_string(),
        value: "x".repeat(6_000_000),
      }],
      indexed_db: Vec::new(),
    };
    assert!(checked_origins(vec![huge])
      .unwrap_err()
      .contains("LOCAL_STORAGE_ORIGIN_TOO_LARGE"));
  }
}

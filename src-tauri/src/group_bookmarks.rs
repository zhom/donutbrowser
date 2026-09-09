//! Bookmarks a profile group shares with every profile in it.
//!
//! A group owns an ordered list of `title` + `url` (+ optional folder). Before
//! a launch the list is written into the profile's Chromium `Bookmarks` file
//! inside ONE folder Donut owns. Everything else in that file — the bookmark
//! bar, the other-bookmarks tree, whatever the person browsing saved — is
//! parsed, left alone, and written back byte for byte.
//!
//! Chromium is unforgiving about this file: a structure it cannot decode is
//! dropped and the person loses every bookmark they had. So the writer never
//! regenerates the document. It parses the real JSON, replaces the children of
//! the one folder it owns, and re-serializes. A file that does not parse is
//! refused outright rather than overwritten, because a half-read file plus a
//! confident rewrite is exactly how bookmarks disappear.
//!
//! The write is idempotent by construction: the managed folder is found by its
//! marker, its previous position, ids, GUIDs and `date_added` stamps are
//! carried forward for entries that are still in the group, and a launch that
//! changes nothing does not touch the file at all.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::profile::types::BrowserProfile;

/// The folder Donut owns inside the bookmark bar. Users see this name.
pub const MANAGED_FOLDER_NAME: &str = "Donut Group Bookmarks";

/// Marker written into the folder's `meta_info`, so the folder is still
/// recognised after someone renames it. Chromium round-trips `meta_info`
/// verbatim and never includes it in the file checksum.
const MANAGED_MARKER_KEY: &str = "donut_managed_group_bookmarks";
const MANAGED_MARKER_VALUE: &str = "1";

/// The profile subdirectory Chromium reads when no `--profile-directory` is
/// passed. Donut never passes one.
const INITIAL_PROFILE_DIR: &str = "Default";

/// One bookmark shared by every profile in a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupBookmark {
  /// What the bookmark is called in the bar.
  pub title: String,
  /// An `http` or `https` address. Every other scheme is refused.
  pub url: String,
  /// Optional sub-folder inside the managed folder.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub folder: Option<String>,
}

/// Trim and check a bookmark list before it is stored on a group.
///
/// Same allowlist the browser-navigation surface uses, minus `about:blank`:
/// a bookmark to a blank tab is not a bookmark, and `file:`, `data:`,
/// `javascript:` and friends must never be written into a profile that an
/// automation client can then be told to open.
pub fn validate(bookmarks: Vec<GroupBookmark>) -> Result<Vec<GroupBookmark>, String> {
  let mut cleaned = Vec::with_capacity(bookmarks.len());
  for bookmark in bookmarks {
    let title = bookmark.title.trim().to_string();
    if title.is_empty() {
      return Err(json!({ "code": "NAME_CANNOT_BE_EMPTY" }).to_string());
    }
    let url = bookmark.url.trim().to_string();
    if !crate::mcp_server::is_navigable_url(&url) || url.eq_ignore_ascii_case("about:blank") {
      return Err(json!({ "code": "URL_SCHEME_NOT_ALLOWED" }).to_string());
    }
    let folder = bookmark
      .folder
      .map(|f| f.trim().to_string())
      .filter(|f| !f.is_empty());
    cleaned.push(GroupBookmark { title, url, folder });
  }
  Ok(cleaned)
}

/// Chromium timestamps are microseconds since 1601-01-01 UTC, as a decimal
/// string. 11644473600 seconds separate that epoch from the Unix one.
const WINDOWS_EPOCH_OFFSET_MICROS: u64 = 11_644_473_600_000_000;

fn chromium_now() -> String {
  let unix_micros = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_micros() as u64)
    .unwrap_or(0);
  (unix_micros + WINDOWS_EPOCH_OFFSET_MICROS).to_string()
}

/// The three permanent roots, in the order Chromium decodes them. The checksum
/// walks them in exactly this order, so it must not change.
const ROOT_KEYS: [&str; 3] = ["bookmark_bar", "other", "synced"];

fn permanent_root(id: &str, name: &str, now: &str) -> Value {
  json!({
    "children": [],
    "date_added": now,
    "date_modified": now,
    "guid": uuid::Uuid::new_v4().to_string(),
    "id": id,
    "name": name,
    "type": "folder",
  })
}

/// A minimal document Chromium accepts, used only when the profile has never
/// had a `Bookmarks` file.
fn empty_document() -> Value {
  let now = chromium_now();
  json!({
    "checksum": "",
    "roots": {
      "bookmark_bar": permanent_root("1", "Bookmarks bar", &now),
      "other": permanent_root("2", "Other bookmarks", &now),
      "synced": permanent_root("3", "Mobile bookmarks", &now),
    },
    "version": 1,
  })
}

fn is_managed_folder(node: &Value) -> bool {
  if node.get("type").and_then(Value::as_str) != Some("folder") {
    return false;
  }
  let marked = node
    .get("meta_info")
    .and_then(Value::as_object)
    .and_then(|m| m.get(MANAGED_MARKER_KEY))
    .and_then(Value::as_str)
    == Some(MANAGED_MARKER_VALUE);
  marked || node.get("name").and_then(Value::as_str) == Some(MANAGED_FOLDER_NAME)
}

/// Highest numeric `id` anywhere in the document, so new nodes never collide
/// with an existing one. A duplicate id makes Chromium renumber the whole tree
/// on load, which is harmless but rewrites a file nobody asked it to rewrite.
fn max_id_in_node(node: &Value, current: &mut u64) {
  if let Some(id) = node
    .get("id")
    .and_then(Value::as_str)
    .and_then(|s| s.parse::<u64>().ok())
  {
    *current = (*current).max(id);
  }
  if let Some(children) = node.get("children").and_then(Value::as_array) {
    for child in children {
      max_id_in_node(child, current);
    }
  }
}

fn max_id(document: &Value, current: &mut u64) {
  let Some(roots) = document.get("roots") else {
    return;
  };
  for key in ROOT_KEYS {
    if let Some(root) = roots.get(key) {
      max_id_in_node(root, current);
    }
  }
}

/// Identity of a bookmark inside the managed folder, used to carry an existing
/// node's id, GUID and creation stamp across a rewrite.
type BookmarkKey = (String, String, String);

fn bookmark_key(folder: Option<&str>, title: &str, url: &str) -> BookmarkKey {
  (
    folder.unwrap_or_default().to_string(),
    title.to_string(),
    url.to_string(),
  )
}

#[derive(Default)]
struct CarriedOver {
  urls: HashMap<BookmarkKey, Value>,
  folders: HashMap<String, Value>,
  root: Option<Value>,
}

/// Index the folder Donut owns so a rewrite can keep every stamp it can.
fn carry_over(existing: Option<&Value>) -> CarriedOver {
  let mut carried = CarriedOver {
    root: existing.cloned(),
    ..Default::default()
  };
  let Some(children) = existing
    .and_then(|n| n.get("children"))
    .and_then(Value::as_array)
  else {
    return carried;
  };
  for child in children {
    let name = child
      .get("name")
      .and_then(Value::as_str)
      .unwrap_or_default();
    match child.get("type").and_then(Value::as_str) {
      Some("folder") => {
        carried.folders.insert(name.to_string(), child.clone());
        if let Some(nested) = child.get("children").and_then(Value::as_array) {
          for leaf in nested {
            if leaf.get("type").and_then(Value::as_str) != Some("url") {
              continue;
            }
            let key = bookmark_key(
              Some(name),
              leaf.get("name").and_then(Value::as_str).unwrap_or_default(),
              leaf.get("url").and_then(Value::as_str).unwrap_or_default(),
            );
            carried.urls.insert(key, leaf.clone());
          }
        }
      }
      Some("url") => {
        let key = bookmark_key(
          None,
          child
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
          child.get("url").and_then(Value::as_str).unwrap_or_default(),
        );
        carried.urls.insert(key, child.clone());
      }
      _ => {}
    }
  }
  carried
}

fn stamp_of(previous: Option<&Value>, now: &str) -> String {
  previous
    .and_then(|n| n.get("date_added"))
    .and_then(Value::as_str)
    .unwrap_or(now)
    .to_string()
}

fn guid_of(previous: Option<&Value>) -> String {
  previous
    .and_then(|n| n.get("guid"))
    .and_then(Value::as_str)
    .map(str::to_string)
    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Hands a node the id it already had, so an unchanged group rebuilds to the
/// exact document that is already on disk and no relaunch rewrites the file.
/// A node with no history — or one whose id a duplicate entry already claimed —
/// gets a fresh number above everything in the document.
struct IdSource {
  next: u64,
  taken: std::collections::HashSet<String>,
}

impl IdSource {
  fn take(&mut self, previous: Option<&Value>) -> String {
    if let Some(id) = previous.and_then(|n| n.get("id")).and_then(Value::as_str) {
      if self.taken.insert(id.to_string()) {
        return id.to_string();
      }
    }
    loop {
      self.next += 1;
      let candidate = self.next.to_string();
      if self.taken.insert(candidate.clone()) {
        return candidate;
      }
    }
  }
}

fn url_node(
  bookmark: &GroupBookmark,
  previous: Option<&Value>,
  ids: &mut IdSource,
  now: &str,
) -> Value {
  json!({
    "date_added": stamp_of(previous, now),
    "guid": guid_of(previous),
    "id": ids.take(previous),
    "name": bookmark.title,
    "type": "url",
    "url": bookmark.url,
  })
}

/// Build the managed folder from the group's list, keeping the declared order.
/// A sub-folder appears at the position of its first member.
fn build_managed_folder(
  bookmarks: &[GroupBookmark],
  carried: &CarriedOver,
  ids: &mut IdSource,
  now: &str,
) -> Value {
  let mut children: Vec<Value> = Vec::new();
  let mut folder_slots: HashMap<String, usize> = HashMap::new();

  for bookmark in bookmarks {
    let previous = carried.urls.get(&bookmark_key(
      bookmark.folder.as_deref(),
      &bookmark.title,
      &bookmark.url,
    ));
    let leaf = url_node(bookmark, previous, ids, now);

    let Some(folder_name) = bookmark.folder.as_deref() else {
      children.push(leaf);
      continue;
    };

    if let Some(&slot) = folder_slots.get(folder_name) {
      if let Some(list) = children[slot]
        .get_mut("children")
        .and_then(Value::as_array_mut)
      {
        list.push(leaf);
      }
      continue;
    }

    let previous_folder = carried.folders.get(folder_name);
    let folder = json!({
      "children": [leaf],
      "date_added": stamp_of(previous_folder, now),
      "date_modified": stamp_of(previous_folder, now),
      "guid": guid_of(previous_folder),
      "id": ids.take(previous_folder),
      "name": folder_name,
      "type": "folder",
    });
    folder_slots.insert(folder_name.to_string(), children.len());
    children.push(folder);
  }

  json!({
    "children": children,
    "date_added": stamp_of(carried.root.as_ref(), now),
    "date_modified": now,
    "guid": guid_of(carried.root.as_ref()),
    "id": ids.take(carried.root.as_ref()),
    "meta_info": { MANAGED_MARKER_KEY: MANAGED_MARKER_VALUE },
    "name": MANAGED_FOLDER_NAME,
    "type": "folder",
  })
}

/// Two folder nodes describe the same bookmarks, ignoring the `date_modified`
/// stamp that moves on every write.
fn same_content(left: &Value, right: &Value) -> bool {
  let strip = |node: &Value| {
    let mut copy = node.clone();
    if let Some(object) = copy.as_object_mut() {
      object.remove("date_modified");
    }
    copy
  };
  strip(left) == strip(right)
}

fn roots_mut(document: &mut Value) -> Result<&mut Map<String, Value>, String> {
  if !document.is_object() {
    return Err("Bookmarks file is not a JSON object".to_string());
  }
  let now = chromium_now();
  let object = document
    .as_object_mut()
    .ok_or_else(|| "Bookmarks file is not a JSON object".to_string())?;
  object.entry("version").or_insert_with(|| json!(1));
  let roots = object.entry("roots").or_insert_with(|| json!({}));
  let roots = roots
    .as_object_mut()
    .ok_or_else(|| "Bookmarks roots is not a JSON object".to_string())?;
  for (key, id, name) in [
    ("bookmark_bar", "1", "Bookmarks bar"),
    ("other", "2", "Other bookmarks"),
    ("synced", "3", "Mobile bookmarks"),
  ] {
    roots
      .entry(key)
      .or_insert_with(|| permanent_root(id, name, &now));
  }
  Ok(roots)
}

/// Replace the managed folder's contents inside `user_data_dir`.
///
/// Returns whether the file was written. `Ok(false)` means the folder already
/// said exactly this, which is the normal answer for a relaunch.
pub fn apply_managed_folder(
  user_data_dir: &Path,
  bookmarks: &[GroupBookmark],
) -> Result<bool, String> {
  let profile_dir = user_data_dir.join(INITIAL_PROFILE_DIR);
  let file = profile_dir.join("Bookmarks");

  let mut document = match std::fs::read_to_string(&file) {
    Ok(raw) if raw.trim().is_empty() => empty_document(),
    Ok(raw) => serde_json::from_str::<Value>(&raw).map_err(|e| {
      // Never overwrite what could not be read: the file may still hold every
      // bookmark this person has, and Chromium keeps its own `Bookmarks.bak`.
      format!("Refusing to rewrite an unreadable Bookmarks file at {file:?}: {e}")
    })?,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
      if bookmarks.is_empty() {
        return Ok(false);
      }
      empty_document()
    }
    Err(e) => return Err(format!("Could not read {file:?}: {e}")),
  };

  let mut ids = IdSource {
    next: 0,
    taken: std::collections::HashSet::new(),
  };
  max_id(&document, &mut ids.next);

  // The managed folder is lifted out of the bar first, so the rest of the file
  // is never rebuilt: everything that comes back is the caller's own bytes.
  let (previous, previous_slot) = {
    let roots = roots_mut(&mut document)?;
    let bar = roots
      .get_mut("bookmark_bar")
      .and_then(Value::as_object_mut)
      .ok_or_else(|| "Bookmarks bar root is not a JSON object".to_string())?;
    let children = bar
      .entry("children")
      .or_insert_with(|| json!([]))
      .as_array_mut()
      .ok_or_else(|| "Bookmarks bar children is not an array".to_string())?;
    let slot = children.iter().position(is_managed_folder);
    let existing = slot.map(|at| children[at].clone());
    children.retain(|child| !is_managed_folder(child));
    (existing, slot)
  };

  if bookmarks.is_empty() {
    if previous.is_none() {
      return Ok(false);
    }
    let checksum = compute_checksum(&document);
    return write_document(&profile_dir, &file, &mut document, checksum);
  }

  let carried = carry_over(previous.as_ref());
  let folder = build_managed_folder(bookmarks, &carried, &mut ids, &chromium_now());

  // The relaunch case: nothing about the group changed, so nothing is written
  // and the file's mtime (and its sync manifest entry) stays put.
  let unchanged = previous
    .as_ref()
    .is_some_and(|existing| same_content(existing, &folder));
  if unchanged {
    return Ok(false);
  }

  {
    let roots = roots_mut(&mut document)?;
    let children = roots
      .get_mut("bookmark_bar")
      .and_then(|bar| bar.get_mut("children"))
      .and_then(Value::as_array_mut)
      .ok_or_else(|| "Bookmarks bar children is not an array".to_string())?;
    match previous_slot {
      Some(slot) if slot <= children.len() => children.insert(slot, folder),
      _ => children.push(folder),
    }
  }

  let checksum = compute_checksum(&document);
  write_document(&profile_dir, &file, &mut document, checksum)
}

fn write_document(
  profile_dir: &Path,
  file: &Path,
  document: &mut Value,
  checksum: String,
) -> Result<bool, String> {
  if let Some(object) = document.as_object_mut() {
    object.insert("checksum".to_string(), Value::String(checksum));
  }
  std::fs::create_dir_all(profile_dir)
    .map_err(|e| format!("Could not create {profile_dir:?}: {e}"))?;
  let serialized =
    serde_json::to_string(document).map_err(|e| format!("Could not serialize bookmarks: {e}"))?;
  // Rename over the real file so a crash mid-write cannot leave Chromium a
  // truncated document to discard.
  let temporary = file.with_extension("donut-tmp");
  std::fs::write(&temporary, serialized.as_bytes())
    .map_err(|e| format!("Could not write {temporary:?}: {e}"))?;
  std::fs::rename(&temporary, file).map_err(|e| {
    let _ = std::fs::remove_file(&temporary);
    format!("Could not replace {file:?}: {e}")
  })?;
  Ok(true)
}

/// Chromium's `BookmarkCodec` checksum: MD5 over a pre-order walk of the three
/// permanent roots, feeding each node's id, its title as UTF-16 code units,
/// its type, and, for a bookmark, its URL exactly as written.
///
/// A mismatch is not fatal — Chromium renumbers and re-saves — but a correct
/// one means the browser opens the file we wrote without rewriting it.
fn compute_checksum(document: &Value) -> String {
  let mut md5 = Md5::new();
  if let Some(roots) = document.get("roots") {
    for key in ROOT_KEYS {
      if let Some(root) = roots.get(key) {
        checksum_node(root, &mut md5);
      }
    }
  }
  md5.finish_hex()
}

fn checksum_node(node: &Value, md5: &mut Md5) {
  let id = node.get("id").and_then(Value::as_str).unwrap_or_default();
  let title = node.get("name").and_then(Value::as_str).unwrap_or_default();
  md5.update(id.as_bytes());
  for unit in title.encode_utf16() {
    md5.update(&unit.to_le_bytes());
  }
  if node.get("type").and_then(Value::as_str) == Some("url") {
    md5.update(b"url");
    md5.update(
      node
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .as_bytes(),
    );
    return;
  }
  md5.update(b"folder");
  if let Some(children) = node.get("children").and_then(Value::as_array) {
    for child in children {
      checksum_node(child, md5);
    }
  }
}

/// Resolve the group bookmarks a profile should carry, or `None` when it is in
/// no group.
fn bookmarks_for(profile: &BrowserProfile) -> Option<Vec<GroupBookmark>> {
  let group_id = profile.group_id.as_deref()?;
  let manager = crate::group_manager::GROUP_MANAGER.lock().ok()?;
  let groups = manager.get_all_groups().ok()?;
  groups
    .into_iter()
    .find(|g| g.id == group_id)
    .map(|g| g.bookmarks)
}

/// Whether this profile's `Bookmarks` file may be rewritten right now.
///
/// A running browser holds the file and rewrites it from memory on exit, so a
/// write underneath it is thrown away at best. An ephemeral or temporary
/// profile is destroyed when its run ends, so shared bookmarks have nothing to
/// persist into. A password-protected profile keeps its plaintext only in a
/// RAM-backed copy; the on-disk directory is ciphertext and a JSON document
/// dropped into it would be unreadable to the profile and corrupt to the tool
/// that decrypts it.
fn is_writable(profile: &BrowserProfile) -> bool {
  !profile.ephemeral
    && !profile.temporary
    && !profile.password_protected
    && !crate::profile::trash::is_running_locally(profile)
}

fn profile_user_data_dir(profile: &BrowserProfile) -> PathBuf {
  let profiles_dir = crate::profile::ProfileManager::instance().get_profiles_dir();
  profile.get_profile_data_path(&profiles_dir)
}

/// Bring one profile's managed folder up to date with its group.
///
/// `Ok(false)` means nothing needed writing: the profile is in no group, its
/// folder already says exactly this, or it is a kind of profile shared
/// bookmarks do not apply to.
pub fn sync_profile(profile: &BrowserProfile) -> Result<bool, String> {
  if !is_writable(profile) {
    return Ok(false);
  }
  let Some(bookmarks) = bookmarks_for(profile) else {
    return Ok(false);
  };
  apply_managed_folder(&profile_user_data_dir(profile), &bookmarks)
}

/// The pre-spawn write. Called once per real browser launch, before the
/// browser process exists and while the profile is provably not running.
pub fn sync_for_launch(profile: &BrowserProfile) {
  match sync_profile(profile) {
    Ok(true) => log::info!("Wrote group bookmarks into profile {}", profile.name),
    Ok(false) => {}
    // Never blocks a launch. The browser opening without today's shared
    // bookmarks is a smaller failure than the browser not opening.
    Err(e) => log::warn!(
      "Could not write group bookmarks for profile {}: {e}",
      profile.name
    ),
  }
}

/// Tauri command: write a profile's group bookmarks now, without launching.
///
/// The launch does this by itself; this is for pushing an edit out to profiles
/// that are sitting stopped. Returns whether the file changed.
#[tauri::command]
pub async fn apply_group_bookmarks_to_profile(profile_id: String) -> Result<bool, String> {
  let profiles = crate::profile::ProfileManager::instance()
    .list_profiles()
    .map_err(|e| e.to_string())?;
  let profile = profiles
    .into_iter()
    .find(|profile| profile.id.to_string() == profile_id)
    .ok_or_else(|| json!({ "code": "PROFILE_NOT_FOUND" }).to_string())?;
  if crate::profile::trash::is_running_locally(&profile) {
    return Err(json!({ "code": "PROFILE_RUNNING" }).to_string());
  }
  sync_profile(&profile)
}

/// Tauri command: read a group's bookmark list.
#[tauri::command]
pub async fn get_group_bookmarks(group_id: String) -> Result<Vec<GroupBookmark>, String> {
  let manager = crate::group_manager::GROUP_MANAGER
    .lock()
    .map_err(|_| json!({ "code": "INTERNAL_ERROR" }).to_string())?;
  let groups = manager.get_all_groups().map_err(|e| e.to_string())?;
  groups
    .into_iter()
    .find(|g| g.id == group_id)
    .map(|g| g.bookmarks)
    .ok_or_else(|| json!({ "code": "GROUP_NOT_FOUND" }).to_string())
}

/// Tauri command: replace a group's bookmark list.
#[tauri::command]
pub async fn set_group_bookmarks(
  app_handle: tauri::AppHandle,
  group_id: String,
  bookmarks: Vec<GroupBookmark>,
) -> Result<Vec<GroupBookmark>, String> {
  let cleaned = validate(bookmarks)?;
  let manager = crate::group_manager::GROUP_MANAGER
    .lock()
    .map_err(|_| json!({ "code": "INTERNAL_ERROR" }).to_string())?;
  manager
    .set_group_bookmarks(&app_handle, &group_id, cleaned.clone())
    .map_err(|e| e.to_string())?;
  Ok(cleaned)
}

// --- MD5, because Chromium's bookmark checksum is defined in terms of it ---

const MD5_SHIFTS: [u32; 64] = [
  7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14,
  20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6,
  10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_SINE: [u32; 64] = [
  0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
  0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
  0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
  0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
  0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
  0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
  0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
  0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

struct Md5 {
  state: [u32; 4],
  buffer: [u8; 64],
  buffered: usize,
  length: u64,
}

impl Md5 {
  fn new() -> Self {
    Self {
      state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
      buffer: [0; 64],
      buffered: 0,
      length: 0,
    }
  }

  fn update(&mut self, mut data: &[u8]) {
    self.length = self.length.wrapping_add(data.len() as u64);
    while !data.is_empty() {
      let take = (64 - self.buffered).min(data.len());
      self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
      self.buffered += take;
      data = &data[take..];
      if self.buffered == 64 {
        let block = self.buffer;
        self.compress(&block);
        self.buffered = 0;
      }
    }
  }

  fn compress(&mut self, block: &[u8; 64]) {
    let mut words = [0u32; 16];
    for (index, word) in words.iter_mut().enumerate() {
      let start = index * 4;
      *word = u32::from_le_bytes([
        block[start],
        block[start + 1],
        block[start + 2],
        block[start + 3],
      ]);
    }

    let [mut a, mut b, mut c, mut d] = self.state;
    for i in 0..64 {
      let (mixed, index) = match i / 16 {
        0 => ((b & c) | (!b & d), i),
        1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
        2 => (b ^ c ^ d, (3 * i + 5) % 16),
        _ => (c ^ (b | !d), (7 * i) % 16),
      };
      let rotated = mixed
        .wrapping_add(a)
        .wrapping_add(MD5_SINE[i])
        .wrapping_add(words[index])
        .rotate_left(MD5_SHIFTS[i]);
      a = d;
      d = c;
      c = b;
      b = b.wrapping_add(rotated);
    }

    self.state[0] = self.state[0].wrapping_add(a);
    self.state[1] = self.state[1].wrapping_add(b);
    self.state[2] = self.state[2].wrapping_add(c);
    self.state[3] = self.state[3].wrapping_add(d);
  }

  fn finish_hex(mut self) -> String {
    let bits = self.length.wrapping_mul(8);
    self.update(&[0x80]);
    // `update` counted the padding, so measure the tail from the buffer.
    while self.buffered != 56 {
      self.update(&[0]);
    }
    let block = {
      let mut block = self.buffer;
      block[56..].copy_from_slice(&bits.to_le_bytes());
      block
    };
    self.compress(&block);

    let mut hex = String::with_capacity(32);
    for word in self.state {
      for byte in word.to_le_bytes() {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
      }
    }
    hex
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  fn md5_hex(input: &[u8]) -> String {
    let mut md5 = Md5::new();
    md5.update(input);
    md5.finish_hex()
  }

  #[test]
  fn md5_matches_the_published_vectors() {
    // Chromium's checksum is only useful if this is really MD5.
    assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(
      md5_hex(b"message digest"),
      "f96b697d7cb7938d525a2f31aaf161d0"
    );
    assert_eq!(
      md5_hex(b"abcdefghijklmnopqrstuvwxyz"),
      "c3fcd3d76192e4007dfb496cca67e13b"
    );
    // Longer than one block, and longer than the 56-byte padding boundary.
    assert_eq!(
      md5_hex(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"),
      "57edf4a22be3c955ac49da2e2107b67a"
    );
  }

  fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "donut-group-bookmarks-{name}-{}",
      uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(dir.join(INITIAL_PROFILE_DIR)).unwrap();
    dir
  }

  fn bookmark(title: &str, url: &str) -> GroupBookmark {
    GroupBookmark {
      title: title.to_string(),
      url: url.to_string(),
      folder: None,
    }
  }

  fn read(dir: &Path) -> Value {
    let raw = std::fs::read_to_string(dir.join(INITIAL_PROFILE_DIR).join("Bookmarks")).unwrap();
    serde_json::from_str(&raw).unwrap()
  }

  fn bar_children(document: &Value) -> &Vec<Value> {
    document["roots"]["bookmark_bar"]["children"]
      .as_array()
      .unwrap()
  }

  fn managed(document: &Value) -> Vec<&Value> {
    bar_children(document)
      .iter()
      .filter(|child| is_managed_folder(child))
      .collect()
  }

  #[test]
  fn writes_a_valid_file_when_the_profile_has_none() {
    let dir = temp_dir("fresh");
    assert!(apply_managed_folder(&dir, &[bookmark("Docs", "https://docs.example")]).unwrap());

    let document = read(&dir);
    assert_eq!(document["version"], 1);
    for key in ROOT_KEYS {
      assert!(document["roots"][key].is_object(), "missing root {key}");
    }
    let folders = managed(&document);
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0]["name"], MANAGED_FOLDER_NAME);
    let entries = folders[0]["children"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["type"], "url");
    assert_eq!(entries[0]["url"], "https://docs.example");
    assert_eq!(entries[0]["name"], "Docs");
    // Chromium parses these as decimal strings, not numbers.
    assert!(entries[0]["date_added"]
      .as_str()
      .unwrap()
      .parse::<u64>()
      .is_ok());
    assert!(entries[0]["id"].as_str().unwrap().parse::<u64>().is_ok());
    std::fs::remove_dir_all(dir).ok();
  }

  #[test]
  fn no_file_and_no_bookmarks_writes_nothing() {
    let dir = temp_dir("nothing");
    assert!(!apply_managed_folder(&dir, &[]).unwrap());
    assert!(!dir.join(INITIAL_PROFILE_DIR).join("Bookmarks").exists());
    std::fs::remove_dir_all(dir).ok();
  }

  fn seed_user_file(dir: &Path) {
    let file = json!({
      "checksum": "deadbeef",
      "roots": {
        "bookmark_bar": {
          "children": [{
            "date_added": "13300000000000000",
            "guid": "11111111-1111-4111-8111-111111111111",
            "id": "7",
            "name": "My Bank",
            "type": "url",
            "url": "https://bank.example/"
          }],
          "date_added": "13300000000000000",
          "date_modified": "13300000000000000",
          "guid": "22222222-2222-4222-8222-222222222222",
          "id": "1",
          "name": "Bookmarks bar",
          "type": "folder"
        },
        "other": {
          "children": [{
            "date_added": "13300000000000000",
            "guid": "33333333-3333-4333-8333-333333333333",
            "id": "8",
            "name": "Recipes",
            "type": "url",
            "url": "https://recipes.example/"
          }],
          "date_added": "13300000000000000",
          "date_modified": "13300000000000000",
          "guid": "44444444-4444-4444-8444-444444444444",
          "id": "2",
          "name": "Other bookmarks",
          "type": "folder"
        },
        "synced": {
          "children": [],
          "date_added": "13300000000000000",
          "date_modified": "13300000000000000",
          "guid": "55555555-5555-4555-8555-555555555555",
          "id": "3",
          "name": "Mobile bookmarks",
          "type": "folder"
        }
      },
      "sync_metadata": "AAAA",
      "version": 1
    });
    std::fs::write(
      dir.join(INITIAL_PROFILE_DIR).join("Bookmarks"),
      serde_json::to_string_pretty(&file).unwrap(),
    )
    .unwrap();
  }

  #[test]
  fn a_users_own_bookmarks_survive_the_write() {
    let dir = temp_dir("preserve");
    seed_user_file(&dir);
    assert!(apply_managed_folder(&dir, &[bookmark("Wiki", "https://wiki.example")]).unwrap());

    let document = read(&dir);
    let bar = bar_children(&document);
    assert_eq!(bar.len(), 2);
    assert_eq!(bar[0]["name"], "My Bank");
    assert_eq!(bar[0]["id"], "7");
    assert_eq!(bar[0]["guid"], "11111111-1111-4111-8111-111111111111");
    assert_eq!(
      document["roots"]["other"]["children"][0]["name"], "Recipes",
      "the other-bookmarks tree must be untouched"
    );
    // Anything Chromium wrote that Donut does not understand has to come back.
    assert_eq!(document["sync_metadata"], "AAAA");
    assert_ne!(document["checksum"], "deadbeef");
    std::fs::remove_dir_all(dir).ok();
  }

  #[test]
  fn rewriting_the_same_list_changes_nothing() {
    let dir = temp_dir("idempotent");
    seed_user_file(&dir);
    let list = vec![
      bookmark("Wiki", "https://wiki.example"),
      GroupBookmark {
        title: "Ticket queue".to_string(),
        url: "https://tickets.example".to_string(),
        folder: Some("Internal".to_string()),
      },
    ];

    assert!(apply_managed_folder(&dir, &list).unwrap());
    let first = std::fs::read_to_string(dir.join(INITIAL_PROFILE_DIR).join("Bookmarks")).unwrap();

    // Second launch: same group, so the file must not be touched at all.
    assert!(!apply_managed_folder(&dir, &list).unwrap());
    let second = std::fs::read_to_string(dir.join(INITIAL_PROFILE_DIR).join("Bookmarks")).unwrap();
    assert_eq!(first, second);

    let document = read(&dir);
    assert_eq!(managed(&document).len(), 1, "the folder must not duplicate");
    std::fs::remove_dir_all(dir).ok();
  }

  #[test]
  fn removing_a_bookmark_from_the_group_removes_it_from_the_folder() {
    let dir = temp_dir("removal");
    seed_user_file(&dir);
    let full = vec![
      bookmark("Wiki", "https://wiki.example"),
      bookmark("Status", "https://status.example"),
    ];
    assert!(apply_managed_folder(&dir, &full).unwrap());
    assert_eq!(
      managed(&read(&dir))[0]["children"]
        .as_array()
        .unwrap()
        .len(),
      2
    );

    assert!(apply_managed_folder(&dir, &full[..1]).unwrap());
    let document = read(&dir);
    let entries = managed(&document)[0]["children"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "Wiki");
    // The stamp of a surviving entry is carried over, not reset.
    assert!(entries[0]["date_added"]
      .as_str()
      .unwrap()
      .parse::<u64>()
      .is_ok());

    // Emptying the group takes the whole folder away and leaves the user's own.
    assert!(apply_managed_folder(&dir, &[]).unwrap());
    let document = read(&dir);
    assert!(managed(&document).is_empty());
    assert_eq!(bar_children(&document).len(), 1);
    assert_eq!(bar_children(&document)[0]["name"], "My Bank");
    std::fs::remove_dir_all(dir).ok();
  }

  #[test]
  fn the_folder_keeps_its_place_in_the_bar() {
    let dir = temp_dir("position");
    seed_user_file(&dir);
    assert!(apply_managed_folder(&dir, &[bookmark("Wiki", "https://wiki.example")]).unwrap());

    // Someone drags the managed folder to the front of the bar.
    let mut document = read(&dir);
    let children = document["roots"]["bookmark_bar"]["children"]
      .as_array_mut()
      .unwrap();
    let folder = children.pop().unwrap();
    children.insert(0, folder);
    std::fs::write(
      dir.join(INITIAL_PROFILE_DIR).join("Bookmarks"),
      serde_json::to_string(&document).unwrap(),
    )
    .unwrap();

    assert!(apply_managed_folder(
      &dir,
      &[
        bookmark("Wiki", "https://wiki.example"),
        bookmark("Status", "https://status.example")
      ]
    )
    .unwrap());
    let document = read(&dir);
    assert!(is_managed_folder(&bar_children(&document)[0]));
    assert_eq!(bar_children(&document)[1]["name"], "My Bank");
    std::fs::remove_dir_all(dir).ok();
  }

  #[test]
  fn the_written_checksum_is_the_one_chromium_computes() {
    let dir = temp_dir("checksum");
    seed_user_file(&dir);
    apply_managed_folder(&dir, &[bookmark("Wiki", "https://wiki.example")]).unwrap();

    let document = read(&dir);
    let stored = document["checksum"].as_str().unwrap().to_string();
    assert_eq!(stored.len(), 32);
    // Recomputed from the file as read back: the value on disk describes the
    // tree on disk, which is the whole point of the field.
    assert_eq!(stored, compute_checksum(&document));

    // The digest is over the id/title/type/url stream, so touching a title
    // must move it.
    let mut tampered = document.clone();
    tampered["roots"]["bookmark_bar"]["children"][0]["name"] = json!("Renamed");
    assert_ne!(stored, compute_checksum(&tampered));
    std::fs::remove_dir_all(dir).ok();
  }

  #[test]
  fn an_unreadable_file_is_refused_rather_than_overwritten() {
    let dir = temp_dir("corrupt");
    let file = dir.join(INITIAL_PROFILE_DIR).join("Bookmarks");
    std::fs::write(&file, "{ this is not json").unwrap();
    let error = apply_managed_folder(&dir, &[bookmark("Wiki", "https://wiki.example")])
      .expect_err("a file that cannot be parsed must not be rewritten");
    assert!(error.contains("Refusing to rewrite"), "{error}");
    assert_eq!(
      std::fs::read_to_string(&file).unwrap(),
      "{ this is not json"
    );
    std::fs::remove_dir_all(dir).ok();
  }

  #[test]
  fn nested_folders_group_their_members_in_order() {
    let dir = temp_dir("folders");
    let list = vec![
      GroupBookmark {
        title: "Console".to_string(),
        url: "https://console.example".to_string(),
        folder: Some("Ops".to_string()),
      },
      bookmark("Home", "https://home.example"),
      GroupBookmark {
        title: "Runbook".to_string(),
        url: "https://runbook.example".to_string(),
        folder: Some("Ops".to_string()),
      },
    ];
    apply_managed_folder(&dir, &list).unwrap();

    let document = read(&dir);
    let entries = managed(&document)[0]["children"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["type"], "folder");
    assert_eq!(entries[0]["name"], "Ops");
    let ops = entries[0]["children"].as_array().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["name"], "Console");
    assert_eq!(ops[1]["name"], "Runbook");
    assert_eq!(entries[1]["name"], "Home");

    // Every id in the document is unique, or Chromium renumbers on load.
    let mut ids = Vec::new();
    fn collect(node: &Value, into: &mut Vec<String>) {
      if let Some(id) = node.get("id").and_then(Value::as_str) {
        into.push(id.to_string());
      }
      if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
          collect(child, into);
        }
      }
    }
    for key in ROOT_KEYS {
      collect(&document["roots"][key], &mut ids);
    }
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "duplicate bookmark ids: {ids:?}");
    std::fs::remove_dir_all(dir).ok();
  }

  #[test]
  fn only_http_and_https_urls_are_accepted() {
    assert!(validate(vec![bookmark("Ok", "https://example.com/x")]).is_ok());
    assert!(validate(vec![bookmark("Ok", "http://example.com")]).is_ok());

    for refused in [
      "file:///Users/someone/.ssh/id_rsa",
      "data:text/html,<script>1</script>",
      "javascript:alert(1)",
      "chrome://settings",
      "about:blank",
      "ftp://files.example",
      "",
    ] {
      let error =
        validate(vec![bookmark("Bad", refused)]).expect_err(&format!("{refused} must be refused"));
      assert!(
        error.contains("URL_SCHEME_NOT_ALLOWED"),
        "{refused}: {error}"
      );
    }

    let error = validate(vec![bookmark("   ", "https://example.com")])
      .expect_err("an empty title must be refused");
    assert!(error.contains("NAME_CANNOT_BE_EMPTY"), "{error}");
  }

  #[test]
  fn validation_trims_and_drops_an_empty_folder_name() {
    let cleaned = validate(vec![GroupBookmark {
      title: "  Docs  ".to_string(),
      url: "  https://docs.example  ".to_string(),
      folder: Some("   ".to_string()),
    }])
    .unwrap();
    assert_eq!(cleaned[0].title, "Docs");
    assert_eq!(cleaned[0].url, "https://docs.example");
    assert_eq!(cleaned[0].folder, None);
  }
}

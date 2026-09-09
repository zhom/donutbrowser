//! Moving Donut's data directory to another volume.
//!
//! Everything the app keeps lives under `app_dirs::data_dir()`: profiles,
//! downloaded browser binaries, settings, proxies, VPNs and extensions. A fleet
//! outgrows a small system disk long before it outgrows the machine, so the
//! directory has to be movable without hand-editing anything.
//!
//! The move is **copy, verify, then delete**, in that order and never any
//! other. A `rename` across volumes is not atomic and can leave half a profile
//! at each end; a delete before the copy is proven can lose the only copy of a
//! logged-in profile. The pointer that decides which directory the next start
//! uses is written only after verification passes, so a process killed at any
//! point still starts on a directory that is whole.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::Emitter;

/// Emitted while a move runs so the page can show real progress.
pub const MOVE_PROGRESS_EVENT: &str = "data-root-move-progress";

/// Sample files compared byte for byte after the copy, on top of the file count
/// and total size. Enough to catch a truncating or silently-failing filesystem
/// without re-reading tens of gigabytes.
const VERIFY_SAMPLE_SIZE: usize = 12;

/// How much of a sampled file is compared when it is too big to read whole.
/// The head and the tail together catch both a truncated write and a copy that
/// never started.
const SAMPLE_EDGE_BYTES: u64 = 1024 * 1024;
const SAMPLE_WHOLE_FILE_LIMIT: u64 = 4 * 1024 * 1024;

/// One move at a time. Two concurrent moves would interleave two copies into
/// one destination and then race to delete the same source.
static MOVE_RUNNING: AtomicBool = AtomicBool::new(false);

/// Set once a move succeeds. The running process keeps using the old directory
/// (every path was resolved at startup), so the page has to say so out loud.
static RESTART_REQUIRED: AtomicBool = AtomicBool::new(false);

fn code(code: &str) -> String {
  serde_json::json!({ "code": code }).to_string()
}

fn code_with(code: &str, params: serde_json::Value) -> String {
  serde_json::json!({ "code": code, "params": params }).to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataRootInfo {
  /// The directory this process is actually using.
  pub active_path: String,
  /// The directory recorded for the next start, when one was chosen.
  pub configured_path: Option<String>,
  /// Where the directory would resolve with nothing chosen.
  pub default_path: String,
  /// Bytes under `active_path`.
  pub size_bytes: u64,
  /// Regular files under `active_path`.
  pub file_count: u64,
  /// True when `DONUTBROWSER_DATA_DIR` decides the directory, so a choice made
  /// here would be recorded and then ignored.
  pub overridden_by_environment: bool,
  /// True once a move has completed in this process.
  pub restart_required: bool,
  /// The folder name a destination gets, so the page can show the full path it
  /// is about to move to before the user commits.
  pub app_directory_name: String,
  /// The recorded directory is not there right now, which is what an
  /// unplugged external drive looks like.
  ///
  /// The app deliberately keeps pointing at it rather than quietly starting
  /// empty somewhere else: plugging the drive back in has to restore
  /// everything, and a silent fallback is how a person concludes their
  /// profiles are gone.
  ///
  /// Never true straight after a move. The old directory is *supposed* to be
  /// gone then, and reporting that as a fault would tell somebody their move
  /// had broken something the moment it succeeded.
  pub active_path_missing: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MoveProgress {
  /// `scanning`, `copying`, `verifying`, `cleaning` or `done`.
  pub phase: String,
  pub copied_files: u64,
  pub total_files: u64,
  pub copied_bytes: u64,
  pub total_bytes: u64,
  pub destination: String,
}

/// What a walk of the source found. `bytes` counts regular files only:
/// directories and symlinks have a size that means nothing here and would make
/// the free-space estimate and the verification disagree across platforms.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct TreeScan {
  pub files: u64,
  pub bytes: u64,
  pub directories: u64,
  pub symlinks: u64,
}

/// The facts a refusal is decided from, gathered before anything is copied.
pub(crate) struct MovePreconditions<'a> {
  pub source: &'a Path,
  pub destination: &'a Path,
  pub browser_running: bool,
  pub sync_in_progress: bool,
  pub required_bytes: u64,
  pub available_bytes: u64,
}

/// Every refusal that can be decided without touching the disk.
///
/// Ordered by how much the user can do about it: a destination that is the
/// current directory, or sits inside it, is a mistake in the request itself;
/// a running browser or a live sync is a "not now"; space is last because it
/// is the only one that needs the source measured first.
pub(crate) fn check_move_preconditions(p: &MovePreconditions) -> Result<(), String> {
  if paths_equal(p.source, p.destination) {
    return Err(code("DATA_ROOT_SAME_AS_CURRENT"));
  }
  if is_inside(p.destination, p.source) {
    // Copying a directory into itself never terminates, and deleting the
    // source afterwards would delete the copy with it.
    return Err(code("DATA_ROOT_DESTINATION_INSIDE_SOURCE"));
  }
  if p.browser_running {
    return Err(code("DATA_ROOT_BROWSER_RUNNING"));
  }
  if p.sync_in_progress {
    return Err(code("DATA_ROOT_SYNC_IN_PROGRESS"));
  }
  if p.available_bytes < p.required_bytes {
    return Err(code_with(
      "DATA_ROOT_INSUFFICIENT_SPACE",
      serde_json::json!({
        "required": human_bytes(p.required_bytes),
        "available": human_bytes(p.available_bytes),
      }),
    ));
  }
  Ok(())
}

/// Bytes as a person reads them, for the one refusal that has to quote a size.
///
/// The unit symbols are the same in every language Donut ships, so the sentence
/// around them is translated and the figure is not. It is written here rather
/// than in the frontend because `backend-errors.ts` is loaded by a bare
/// `node --test` run and cannot import anything of ours.
pub(crate) fn human_bytes(bytes: u64) -> String {
  const KB: u64 = 1024;
  const MB: u64 = KB * 1024;
  const GB: u64 = MB * 1024;
  if bytes < KB {
    return format!("{bytes} B");
  }
  if bytes < MB {
    return format!("{:.1} KB", bytes as f64 / KB as f64);
  }
  if bytes < GB {
    return format!("{:.1} MB", bytes as f64 / MB as f64);
  }
  format!("{:.2} GB", bytes as f64 / GB as f64)
}

/// Compare two paths without needing either to exist. `canonicalize` is used
/// when it works (it resolves `..`, symlinks and case on macOS), and the
/// lexical form is the fallback for a destination that has not been created.
fn paths_equal(a: &Path, b: &Path) -> bool {
  match (a.canonicalize(), b.canonicalize()) {
    (Ok(a), Ok(b)) => a == b,
    _ => normalized(a) == normalized(b),
  }
}

/// True when `inner` is `outer` itself or sits below it.
pub(crate) fn is_inside(inner: &Path, outer: &Path) -> bool {
  let (inner, outer) = match (inner.canonicalize(), outer.canonicalize()) {
    (Ok(i), Ok(o)) => (i, o),
    _ => (normalized(inner), normalized(outer)),
  };
  inner.starts_with(&outer)
}

/// Lexical `..`/`.` removal, so `/a/b/../b/c` and `/a/b/c` compare equal even
/// when neither exists yet.
fn normalized(path: &Path) -> PathBuf {
  use std::path::Component;
  let mut out = PathBuf::new();
  for component in path.components() {
    match component {
      Component::CurDir => {}
      Component::ParentDir => {
        out.pop();
      }
      other => out.push(other.as_os_str()),
    }
  }
  out
}

/// Prove the destination can be written to before a single byte is copied.
/// Creating the directory is part of the probe: a parent that refuses `mkdir`
/// is exactly as unusable as one that refuses a write.
pub(crate) fn probe_writable(destination: &Path) -> Result<(), String> {
  if let Err(e) = std::fs::create_dir_all(destination) {
    log::warn!(
      "Cannot use {} as a data directory: {e}",
      destination.display()
    );
    return Err(code("DATA_ROOT_DESTINATION_NOT_WRITABLE"));
  }
  let probe = destination.join(".donut-write-probe");
  match std::fs::write(&probe, b"donut") {
    Ok(()) => {
      let _ = std::fs::remove_file(&probe);
      Ok(())
    }
    Err(e) => {
      log::warn!("Cannot write inside {}: {e}", destination.display());
      Err(code("DATA_ROOT_DESTINATION_NOT_WRITABLE"))
    }
  }
}

/// Refuse a destination that already holds files. Merging into somebody's
/// folder makes the count-and-size verification meaningless and makes the
/// delete that follows impossible to reason about.
pub(crate) fn ensure_empty(destination: &Path) -> Result<(), String> {
  let Ok(entries) = std::fs::read_dir(destination) else {
    return Ok(());
  };
  for entry in entries.flatten() {
    if entry.file_name() == ".donut-write-probe" {
      continue;
    }
    return Err(code("DATA_ROOT_DESTINATION_NOT_EMPTY"));
  }
  Ok(())
}

/// Walk a tree, counting regular files, their bytes, directories and symlinks.
///
/// Symlinks are counted but never followed: a link out of the data directory
/// would pull unrelated data into the copy, and a link back into it would loop.
pub(crate) fn scan_tree(root: &Path) -> std::io::Result<TreeScan> {
  let mut scan = TreeScan::default();
  let mut stack = vec![root.to_path_buf()];
  while let Some(dir) = stack.pop() {
    for entry in std::fs::read_dir(&dir)? {
      let entry = entry?;
      let path = entry.path();
      let meta = std::fs::symlink_metadata(&path)?;
      if meta.file_type().is_symlink() {
        scan.symlinks += 1;
      } else if meta.is_dir() {
        scan.directories += 1;
        stack.push(path);
      } else {
        scan.files += 1;
        scan.bytes += meta.len();
      }
    }
  }
  Ok(scan)
}

/// Free bytes on the volume holding `path`, found by the longest mount point
/// that is a prefix of it. `None` when no mount point matches, which is not a
/// reason to refuse a move: an unknown figure is not a small one.
pub(crate) fn available_space(path: &Path) -> Option<u64> {
  let disks = sysinfo::Disks::new_with_refreshed_list();
  let target = normalized(path);
  let mut best: Option<(usize, u64)> = None;
  for disk in disks.list() {
    let mount = disk.mount_point();
    if !target.starts_with(mount) {
      continue;
    }
    let depth = mount.components().count();
    if best.is_none_or(|(previous, _)| depth > previous) {
      best = Some((depth, disk.available_space()));
    }
  }
  best.map(|(_, free)| free)
}

/// Copy `source` into `destination`, reporting progress and collecting the
/// sample the verification re-reads.
fn copy_tree(
  source: &Path,
  destination: &Path,
  total: &TreeScan,
  report: &mut dyn FnMut(&str, u64, u64),
) -> std::io::Result<Vec<PathBuf>> {
  let stride = (total.files / VERIFY_SAMPLE_SIZE as u64).max(1);
  let mut samples: Vec<PathBuf> = Vec::new();
  let mut copied_files = 0u64;
  let mut copied_bytes = 0u64;
  let mut stack = vec![PathBuf::new()];

  while let Some(relative) = stack.pop() {
    let from = source.join(&relative);
    let to = destination.join(&relative);
    std::fs::create_dir_all(&to)?;
    for entry in std::fs::read_dir(&from)? {
      let entry = entry?;
      let name = entry.file_name();
      let child = relative.join(&name);
      let path = entry.path();
      let meta = std::fs::symlink_metadata(&path)?;
      if meta.file_type().is_symlink() {
        copy_symlink(&path, &destination.join(&child))?;
      } else if meta.is_dir() {
        stack.push(child);
      } else {
        std::fs::copy(&path, destination.join(&child))?;
        copied_files += 1;
        copied_bytes += meta.len();
        if copied_files.is_multiple_of(stride) && samples.len() < VERIFY_SAMPLE_SIZE {
          samples.push(child);
        }
        if copied_files.is_multiple_of(200) {
          report("copying", copied_files, copied_bytes);
        }
      }
    }
  }
  report("copying", copied_files, copied_bytes);
  Ok(samples)
}

/// Recreate a symlink at the destination rather than following it.
#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> std::io::Result<()> {
  let target = std::fs::read_link(from)?;
  if to.exists() {
    let _ = std::fs::remove_file(to);
  }
  std::os::unix::fs::symlink(target, to)
}

#[cfg(windows)]
fn copy_symlink(from: &Path, to: &Path) -> std::io::Result<()> {
  // Creating a symlink on Windows needs a privilege a normal user does not
  // have, so the link's contents are copied instead. The entry still exists at
  // the same path, which is what the verification checks.
  let metadata = std::fs::metadata(from)?;
  if metadata.is_dir() {
    std::fs::create_dir_all(to)
  } else {
    std::fs::copy(from, to).map(|_| ())
  }
}

/// Compare a copied file with its source. Whole files up to
/// `SAMPLE_WHOLE_FILE_LIMIT`; head and tail beyond that, so a 4 GB browser
/// archive is still checked at both ends without being read twice over.
fn same_contents(a: &Path, b: &Path) -> std::io::Result<bool> {
  use std::io::{Read, Seek, SeekFrom};
  let mut left = std::fs::File::open(a)?;
  let mut right = std::fs::File::open(b)?;
  let left_len = left.metadata()?.len();
  let right_len = right.metadata()?.len();
  if left_len != right_len {
    return Ok(false);
  }
  if left_len <= SAMPLE_WHOLE_FILE_LIMIT {
    let mut left_buf = Vec::new();
    let mut right_buf = Vec::new();
    left.read_to_end(&mut left_buf)?;
    right.read_to_end(&mut right_buf)?;
    return Ok(left_buf == right_buf);
  }
  let mut left_buf = vec![0u8; SAMPLE_EDGE_BYTES as usize];
  let mut right_buf = vec![0u8; SAMPLE_EDGE_BYTES as usize];
  for offset in [0, left_len - SAMPLE_EDGE_BYTES] {
    left.seek(SeekFrom::Start(offset))?;
    right.seek(SeekFrom::Start(offset))?;
    left.read_exact(&mut left_buf)?;
    right.read_exact(&mut right_buf)?;
    if left_buf != right_buf {
      return Ok(false);
    }
  }
  Ok(true)
}

/// Prove the copy is complete before anything is deleted.
///
/// The counts and the total size catch a lost or truncated file; re-reading the
/// sample catches a filesystem that reported a write it never made.
pub(crate) fn verify_copy(
  source: &Path,
  destination: &Path,
  expected: &TreeScan,
  samples: &[PathBuf],
) -> Result<(), String> {
  let copied = scan_tree(destination).map_err(|e| {
    log::error!("Could not read the copy at {}: {e}", destination.display());
    code_with(
      "DATA_ROOT_VERIFY_FAILED",
      serde_json::json!({ "detail": e.to_string() }),
    )
  })?;

  if copied.files != expected.files || copied.bytes != expected.bytes {
    log::error!(
      "The copy at {} does not match {}: {} files / {} bytes against {} files / {} bytes",
      destination.display(),
      source.display(),
      copied.files,
      copied.bytes,
      expected.files,
      expected.bytes
    );
    return Err(code_with(
      "DATA_ROOT_VERIFY_FAILED",
      serde_json::json!({
        "expectedFiles": expected.files.to_string(),
        "copiedFiles": copied.files.to_string(),
        "expectedBytes": expected.bytes.to_string(),
        "copiedBytes": copied.bytes.to_string(),
      }),
    ));
  }

  for relative in samples {
    let from = source.join(relative);
    let to = destination.join(relative);
    match same_contents(&from, &to) {
      Ok(true) => {}
      Ok(false) => {
        log::error!("{} did not copy faithfully", relative.display());
        return Err(code_with(
          "DATA_ROOT_VERIFY_FAILED",
          serde_json::json!({ "detail": relative.to_string_lossy() }),
        ));
      }
      Err(e) => {
        log::error!("Could not re-read {}: {e}", relative.display());
        return Err(code_with(
          "DATA_ROOT_VERIFY_FAILED",
          serde_json::json!({ "detail": e.to_string() }),
        ));
      }
    }
  }
  Ok(())
}

/// True when any profile still has a live process.
fn any_browser_running() -> bool {
  let Ok(profiles) = crate::profile::ProfileManager::instance().list_profiles() else {
    // Unreadable profiles means an unknown answer, and an unknown answer must
    // not clear the way for a move that deletes them.
    log::warn!("Could not list profiles before a data directory move; assuming one is running");
    return true;
  };
  profiles.into_iter().any(|profile| {
    profile
      .process_id
      .is_some_and(|pid| pid != 0 && crate::proxy_storage::is_process_running(pid))
  })
}

async fn sync_in_progress() -> bool {
  match crate::sync::get_global_scheduler() {
    Some(scheduler) => scheduler.is_sync_in_progress().await,
    None => false,
  }
}

fn info_now() -> DataRootInfo {
  let active = crate::app_dirs::data_dir();
  let scan = scan_tree(&active).unwrap_or_default();
  let restart_required = RESTART_REQUIRED.load(Ordering::SeqCst);
  DataRootInfo {
    active_path_missing: !restart_required && !active.is_dir(),
    active_path: active.to_string_lossy().to_string(),
    configured_path: crate::app_dirs::read_data_root_pointer(
      &crate::app_dirs::data_root_pointer_file(),
    )
    .map(|path| path.to_string_lossy().to_string()),
    default_path: crate::app_dirs::default_data_dir()
      .to_string_lossy()
      .to_string(),
    size_bytes: scan.bytes,
    file_count: scan.files,
    overridden_by_environment: crate::app_dirs::data_dir_forced_by_environment(),
    restart_required,
    app_directory_name: crate::app_dirs::app_name().to_string(),
  }
}

/// Release the one-move-at-a-time flag however the move ends.
struct MoveGuard;

impl Drop for MoveGuard {
  fn drop(&mut self) {
    MOVE_RUNNING.store(false, Ordering::SeqCst);
  }
}

async fn move_to(
  app_handle: tauri::AppHandle,
  destination: PathBuf,
) -> Result<DataRootInfo, String> {
  if MOVE_RUNNING.swap(true, Ordering::SeqCst) {
    return Err(code("DATA_ROOT_MOVE_IN_PROGRESS"));
  }
  let _guard = MoveGuard;

  if !destination.is_absolute() || destination.as_os_str().is_empty() {
    return Err(code("DATA_ROOT_DESTINATION_NOT_WRITABLE"));
  }

  // The one fact that has to be read from the async side; everything after it
  // is filesystem work.
  let sync_running = sync_in_progress().await;

  // Copying a fleet is minutes of blocking IO. Left on a runtime worker it
  // would stall every other task in the app — proxy workers, the sync
  // scheduler, the event loop that carries the progress this very move emits.
  match tokio::task::spawn_blocking(move || perform_move(app_handle, destination, sync_running))
    .await
  {
    Ok(result) => result,
    Err(e) => {
      log::error!("The data directory move task did not finish: {e}");
      Err(code_with(
        "DATA_ROOT_COPY_FAILED",
        serde_json::json!({ "detail": e.to_string() }),
      ))
    }
  }
}

/// The move itself, start to finish, on a blocking thread.
fn perform_move(
  app_handle: tauri::AppHandle,
  destination: PathBuf,
  sync_running: bool,
) -> Result<DataRootInfo, String> {
  let source = crate::app_dirs::data_dir();
  let emit = |phase: &str, copied_files: u64, copied_bytes: u64, total: &TreeScan| {
    let _ = app_handle.emit(
      MOVE_PROGRESS_EVENT,
      MoveProgress {
        phase: phase.to_string(),
        copied_files,
        total_files: total.files,
        copied_bytes,
        total_bytes: total.bytes,
        destination: destination.to_string_lossy().to_string(),
      },
    );
  };

  emit("scanning", 0, 0, &TreeScan::default());

  let total = scan_tree(&source).map_err(|e| {
    log::error!("Could not measure {}: {e}", source.display());
    code_with(
      "DATA_ROOT_COPY_FAILED",
      serde_json::json!({ "detail": e.to_string() }),
    )
  })?;

  // The refusals that need no disk write come first, so a destination that was
  // never going to be used is not created as a side effect of asking.
  check_move_preconditions(&MovePreconditions {
    source: &source,
    destination: &destination,
    browser_running: any_browser_running(),
    sync_in_progress: sync_running,
    required_bytes: total.bytes,
    // An unreadable volume is not a full one: skip the check rather than
    // refuse a move that would have worked.
    available_bytes: available_space(&destination).unwrap_or(u64::MAX),
  })?;

  probe_writable(&destination)?;
  ensure_empty(&destination)?;

  let mut report = |phase: &str, files: u64, bytes: u64| emit(phase, files, bytes, &total);

  let samples = copy_tree(&source, &destination, &total, &mut report).map_err(|e| {
    log::error!(
      "Copying {} to {} failed: {e}",
      source.display(),
      destination.display()
    );
    code_with(
      "DATA_ROOT_COPY_FAILED",
      serde_json::json!({ "detail": e.to_string() }),
    )
  })?;

  emit("verifying", total.files, total.bytes, &total);
  verify_copy(&source, &destination, &total, &samples)?;

  // Only now, with the copy proven whole, does the next start change where it
  // looks. A process killed before this line still starts on the old directory.
  crate::app_dirs::write_data_root_pointer(
    &crate::app_dirs::data_root_pointer_file(),
    &destination,
  )
  .map_err(|e| {
    log::error!("Could not record the new data directory: {e}");
    code_with(
      "DATA_ROOT_COPY_FAILED",
      serde_json::json!({ "detail": e.to_string() }),
    )
  })?;
  RESTART_REQUIRED.store(true, Ordering::SeqCst);

  emit("cleaning", total.files, total.bytes, &total);
  if let Err(e) = std::fs::remove_dir_all(&source) {
    // The move already succeeded: the pointer is written and the copy is
    // verified. A source that will not delete is leftover disk, not a failure.
    log::warn!(
      "Moved the data directory to {} but could not remove {}: {e}",
      destination.display(),
      source.display()
    );
  }

  emit("done", total.files, total.bytes, &total);
  log::info!(
    "Data directory moved to {} ({} files, {} bytes); it takes effect on the next start",
    destination.display(),
    total.files,
    total.bytes
  );
  Ok(info_now())
}

// --- Tauri commands ---

#[tauri::command]
pub async fn get_data_root_info() -> Result<DataRootInfo, String> {
  // Walking a fleet's worth of profiles is not work for the UI thread. A join
  // failure means the pool is gone, not that the answer is unknowable, so the
  // same read runs here rather than inventing a second, emptier answer.
  match tokio::task::spawn_blocking(info_now).await {
    Ok(info) => Ok(info),
    Err(e) => {
      log::error!("Reading the data directory off-thread failed: {e}");
      Ok(info_now())
    }
  }
}

#[tauri::command]
pub async fn move_data_root(
  app_handle: tauri::AppHandle,
  destination: String,
) -> Result<DataRootInfo, String> {
  move_to(app_handle, PathBuf::from(destination)).await
}

/// Forget a recorded directory so the next start uses the platform default
/// again.
///
/// It moves nothing. The page offers it only when the recorded directory is
/// not there — an external drive that is gone for good — because that is the
/// one case where pointing at it is worse than starting fresh.
#[tauri::command]
pub async fn clear_data_root_choice() -> Result<DataRootInfo, String> {
  crate::app_dirs::clear_data_root_pointer(&crate::app_dirs::data_root_pointer_file()).map_err(
    |e| {
      log::error!("Could not clear the data directory choice: {e}");
      code_with(
        "DATA_ROOT_COPY_FAILED",
        serde_json::json!({ "detail": e.to_string() }),
      )
    },
  )?;
  RESTART_REQUIRED.store(true, Ordering::SeqCst);
  Ok(info_now())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parsed_code(err: &str) -> String {
    serde_json::from_str::<serde_json::Value>(err)
      .expect("errors are JSON")
      .get("code")
      .and_then(|c| c.as_str())
      .expect("errors carry a code")
      .to_string()
  }

  fn baseline<'a>(source: &'a Path, destination: &'a Path) -> MovePreconditions<'a> {
    MovePreconditions {
      source,
      destination,
      browser_running: false,
      sync_in_progress: false,
      required_bytes: 100,
      available_bytes: 1000,
    }
  }

  fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
  }

  #[test]
  fn a_clean_request_is_allowed() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::create_dir_all(&source).unwrap();
    assert!(check_move_preconditions(&baseline(&source, &destination)).is_ok());
  }

  #[test]
  fn a_running_browser_stops_the_move() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::create_dir_all(&source).unwrap();
    let mut p = baseline(&source, &destination);
    p.browser_running = true;
    assert_eq!(
      parsed_code(&check_move_preconditions(&p).unwrap_err()),
      "DATA_ROOT_BROWSER_RUNNING"
    );
  }

  #[test]
  fn a_live_sync_stops_the_move() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::create_dir_all(&source).unwrap();
    let mut p = baseline(&source, &destination);
    p.sync_in_progress = true;
    assert_eq!(
      parsed_code(&check_move_preconditions(&p).unwrap_err()),
      "DATA_ROOT_SYNC_IN_PROGRESS"
    );
  }

  #[test]
  fn a_destination_inside_the_source_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("profiles")).unwrap();

    for inside in [
      source.join("profiles"),
      source.join("deep").join("nested"),
      source.join("profiles").join("..").join("binaries"),
    ] {
      assert_eq!(
        parsed_code(&check_move_preconditions(&baseline(&source, &inside)).unwrap_err()),
        "DATA_ROOT_DESTINATION_INSIDE_SOURCE",
        "{} is inside {}",
        inside.display(),
        source.display()
      );
    }

    // The source itself is its own refusal: nothing to copy, and the delete
    // that follows would take the only copy with it.
    assert_eq!(
      parsed_code(&check_move_preconditions(&baseline(&source, &source)).unwrap_err()),
      "DATA_ROOT_SAME_AS_CURRENT"
    );

    // A sibling that merely shares a prefix is not inside it.
    let sibling = temp.path().join("source-elsewhere");
    assert!(check_move_preconditions(&baseline(&source, &sibling)).is_ok());
  }

  #[test]
  fn a_destination_with_less_space_than_the_source_needs_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::create_dir_all(&source).unwrap();

    let mut p = baseline(&source, &destination);
    p.required_bytes = 8 * 1024 * 1024 * 1024;
    p.available_bytes = 1024 * 1024 * 1024;
    let err = check_move_preconditions(&p).unwrap_err();
    assert_eq!(parsed_code(&err), "DATA_ROOT_INSUFFICIENT_SPACE");
    let value: serde_json::Value = serde_json::from_str(&err).unwrap();
    // Written out, because the sentence is shown to a person.
    assert_eq!(value["params"]["required"], "8.00 GB");
    assert_eq!(value["params"]["available"], "1.00 GB");

    // Exactly enough is enough; one byte short is not.
    p.available_bytes = p.required_bytes;
    assert!(check_move_preconditions(&p).is_ok());
    p.available_bytes = p.required_bytes - 1;
    assert_eq!(
      parsed_code(&check_move_preconditions(&p).unwrap_err()),
      "DATA_ROOT_INSUFFICIENT_SPACE"
    );
  }

  #[cfg(unix)]
  #[test]
  fn an_unwritable_destination_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("read-only");
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();

    let destination = parent.join("DonutBrowser");
    let outcome = probe_writable(&destination);
    // Root ignores the mode bits, so the probe legitimately succeeds there and
    // there is nothing for this test to assert.
    if outcome.is_ok() {
      std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
      return;
    }
    assert_eq!(
      parsed_code(&outcome.unwrap_err()),
      "DATA_ROOT_DESTINATION_NOT_WRITABLE"
    );
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();

    // A writable destination passes, and the probe leaves nothing behind.
    let fine = temp.path().join("fine");
    assert!(probe_writable(&fine).is_ok());
    assert_eq!(std::fs::read_dir(&fine).unwrap().count(), 0);
  }

  #[test]
  fn a_destination_that_already_holds_files_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("destination");
    std::fs::create_dir_all(&destination).unwrap();
    assert!(ensure_empty(&destination).is_ok());
    write(&destination.join("someone-elses.txt"), b"hello");
    assert_eq!(
      parsed_code(&ensure_empty(&destination).unwrap_err()),
      "DATA_ROOT_DESTINATION_NOT_EMPTY"
    );
  }

  #[test]
  fn scanning_counts_files_bytes_and_directories_without_following_links() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    write(&root.join("a.txt"), b"12345");
    write(&root.join("nested").join("b.bin"), &[7u8; 40]);
    std::fs::create_dir_all(root.join("empty")).unwrap();

    let scan = scan_tree(&root).unwrap();
    assert_eq!(scan.files, 2);
    assert_eq!(scan.bytes, 45);
    assert_eq!(scan.directories, 2);
    assert_eq!(scan.symlinks, 0);

    #[cfg(unix)]
    {
      // A link out of the tree must not drag its target's bytes in.
      let outside = temp.path().join("outside.bin");
      std::fs::write(&outside, [1u8; 4096]).unwrap();
      std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
      let with_link = scan_tree(&root).unwrap();
      assert_eq!(with_link.files, 2);
      assert_eq!(with_link.bytes, 45);
      assert_eq!(with_link.symlinks, 1);
    }
  }

  #[test]
  fn verification_accepts_a_faithful_copy() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    write(&source.join("settings").join("app.json"), b"{\"a\":1}");
    write(
      &source.join("profiles").join("one").join("Cookies"),
      &[3u8; 900],
    );
    write(&source.join("binaries").join("browser"), &[9u8; 2048]);

    let total = scan_tree(&source).unwrap();
    let mut noop = |_: &str, _: u64, _: u64| {};
    let samples = copy_tree(&source, &destination, &total, &mut noop).unwrap();
    assert!(!samples.is_empty(), "a sample must be collected to verify");
    assert!(verify_copy(&source, &destination, &total, &samples).is_ok());
  }

  #[test]
  fn verification_rejects_a_copy_that_lost_a_file() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    write(&source.join("keep.txt"), b"kept");
    write(
      &source.join("profiles").join("one").join("Login Data"),
      &[5u8; 512],
    );

    let total = scan_tree(&source).unwrap();
    let mut noop = |_: &str, _: u64, _: u64| {};
    let samples = copy_tree(&source, &destination, &total, &mut noop).unwrap();

    // Exactly the failure a delete-before-verify would turn into data loss.
    std::fs::remove_file(destination.join("profiles").join("one").join("Login Data")).unwrap();
    assert_eq!(
      parsed_code(&verify_copy(&source, &destination, &total, &samples).unwrap_err()),
      "DATA_ROOT_VERIFY_FAILED"
    );
  }

  #[test]
  fn verification_rejects_a_copy_that_lost_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    write(
      &source.join("profiles").join("one").join("History"),
      &[4u8; 4096],
    );

    let total = scan_tree(&source).unwrap();
    let mut noop = |_: &str, _: u64, _: u64| {};
    let samples = copy_tree(&source, &destination, &total, &mut noop).unwrap();

    std::fs::write(
      destination.join("profiles").join("one").join("History"),
      [4u8; 2048],
    )
    .unwrap();
    assert_eq!(
      parsed_code(&verify_copy(&source, &destination, &total, &samples).unwrap_err()),
      "DATA_ROOT_VERIFY_FAILED"
    );
  }

  #[test]
  fn verification_rejects_a_file_whose_contents_changed() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    write(&source.join("only.bin"), &[1u8; 64]);

    let total = scan_tree(&source).unwrap();
    let mut noop = |_: &str, _: u64, _: u64| {};
    let samples = copy_tree(&source, &destination, &total, &mut noop).unwrap();
    assert_eq!(samples.len(), 1);

    // Same length, different bytes: only re-reading the sample catches this.
    std::fs::write(destination.join("only.bin"), [2u8; 64]).unwrap();
    assert_eq!(
      parsed_code(&verify_copy(&source, &destination, &total, &samples).unwrap_err()),
      "DATA_ROOT_VERIFY_FAILED"
    );
  }

  #[test]
  fn a_copy_carries_every_nested_file_and_empty_directory() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    for i in 0..40 {
      write(
        &source
          .join("profiles")
          .join(format!("p{i}"))
          .join("Cookies"),
        &[i as u8; 64],
      );
    }
    std::fs::create_dir_all(source.join("extensions")).unwrap();

    let total = scan_tree(&source).unwrap();
    let mut noop = |_: &str, _: u64, _: u64| {};
    let samples = copy_tree(&source, &destination, &total, &mut noop).unwrap();

    assert_eq!(samples.len(), VERIFY_SAMPLE_SIZE);
    assert_eq!(scan_tree(&destination).unwrap(), total);
    assert!(destination.join("extensions").is_dir());
    assert!(verify_copy(&source, &destination, &total, &samples).is_ok());
  }

  #[test]
  fn progress_reaches_the_full_count() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    for i in 0..5 {
      write(&source.join(format!("f{i}")), &[0u8; 10]);
    }
    let total = scan_tree(&source).unwrap();
    let mut seen: Vec<(String, u64, u64)> = Vec::new();
    let mut record = |phase: &str, files: u64, bytes: u64| {
      seen.push((phase.to_string(), files, bytes));
    };
    copy_tree(&source, &destination, &total, &mut record).unwrap();
    let last = seen.last().expect("progress is reported at least once");
    assert_eq!(last.0, "copying");
    assert_eq!(last.1, total.files);
    assert_eq!(last.2, total.bytes);
  }

  #[test]
  fn sizes_are_written_the_way_a_person_reads_them() {
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(999), "999 B");
    assert_eq!(human_bytes(1024), "1.0 KB");
    assert_eq!(human_bytes(1536), "1.5 KB");
    assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");
  }

  #[test]
  fn available_space_is_read_for_a_real_directory() {
    let temp = tempfile::tempdir().unwrap();
    // The figure itself depends on the machine; that it resolves at all is
    // what the refusal relies on.
    if let Some(free) = available_space(temp.path()) {
      assert!(free > 0);
    }
  }
}

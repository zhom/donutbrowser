use directories::BaseDirs;
use std::path::PathBuf;
use std::sync::OnceLock;

static BASE_DIRS: OnceLock<BaseDirs> = OnceLock::new();
static PORTABLE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

fn base_dirs() -> &'static BaseDirs {
  BASE_DIRS.get_or_init(|| BaseDirs::new().expect("Failed to get base directories"))
}

/// Returns the portable base directory if a `.portable` marker exists next to the executable.
fn portable_dir() -> Option<&'static PathBuf> {
  PORTABLE_DIR
    .get_or_init(|| {
      std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
        .filter(|dir| dir.join(".portable").exists())
    })
    .as_ref()
}

/// Returns true if the app is running in portable mode.
pub fn is_portable() -> bool {
  portable_dir().is_some()
}

/// Optional single-root override for all on-disk state. Set
/// `DONUTBROWSER_DATA_ROOT=/path` (e.g. a tmpfs mount) to relocate
/// data/cache/logs under `<root>/{data,cache,logs}` without touching the real
/// dev/prod directories. The more specific `DONUTBROWSER_DATA_DIR` /
/// `DONUTBROWSER_CACHE_DIR` overrides still take precedence over this.
fn data_root() -> Option<PathBuf> {
  std::env::var_os("DONUTBROWSER_DATA_ROOT")
    .filter(|v| !v.is_empty())
    .map(PathBuf::from)
}

/// Where logs go when something other than the platform default applies:
/// `<root>/logs` for `DONUTBROWSER_DATA_ROOT`, else `<exe dir>/logs` in
/// portable mode. `None` means the platform default app log dir.
///
/// Portable belongs here for the same reason `data_dir` and `cache_dir` honour
/// it: a portable install is expected to keep its state beside the executable.
/// Logs were the one thing still written to the host machine, which quietly
/// defeated that.
pub fn log_dir_override() -> Option<PathBuf> {
  log_dir_for(data_root(), portable_dir())
}

/// Split out from `log_dir_override` so the precedence is testable without a
/// real `.portable` marker sitting next to the test binary.
fn log_dir_for(root: Option<PathBuf>, portable: Option<&PathBuf>) -> Option<PathBuf> {
  if let Some(root) = root {
    return Some(root.join("logs"));
  }
  portable.map(|dir| dir.join("logs"))
}

/// File name `tauri-plugin-window-state` persists geometry under.
pub const WINDOW_STATE_FILENAME: &str = ".window-state.json";

/// File name of the pointer that records a data directory the user chose in
/// Settings.
pub const DATA_ROOT_POINTER_FILENAME: &str = "data-root.json";

static CUSTOM_DATA_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Where the pointer to a user-chosen data directory lives.
///
/// It must never sit inside `data_dir()` itself: a move deletes the old
/// directory once the copy verifies, which would take the pointer with it and
/// send the next start back to the platform default. Every branch below
/// therefore resolves OUTSIDE the data directory it points at.
///
/// - With `DONUTBROWSER_DATA_ROOT` set, `<root>/data-root.json`, a sibling of
///   `<root>/data`. An isolated run (the E2E harness) then keeps its own
///   pointer and can never read, or write, the real machine's.
/// - In portable mode, `<exe dir>/data-root.json`, beside `<exe dir>/data`, so
///   the choice travels with the install.
/// - Otherwise the platform preference directory, which is a different root
///   from `data_local_dir` on macOS, Linux and Windows alike.
pub fn data_root_pointer_file() -> PathBuf {
  data_root_pointer_file_for(
    data_root(),
    portable_dir(),
    base_dirs().preference_dir().join(app_name()),
  )
}

/// Split out from `data_root_pointer_file` so the precedence is testable
/// without a `.portable` marker or process-wide environment mutation.
fn data_root_pointer_file_for(
  root: Option<PathBuf>,
  portable: Option<&PathBuf>,
  preference_dir: PathBuf,
) -> PathBuf {
  if let Some(root) = root {
    return root.join(DATA_ROOT_POINTER_FILENAME);
  }
  if let Some(dir) = portable {
    return dir.join(DATA_ROOT_POINTER_FILENAME);
  }
  preference_dir.join(DATA_ROOT_POINTER_FILENAME)
}

/// Read a pointer file written by a previous "move data directory".
///
/// A missing, unreadable, malformed, empty or relative entry resolves to
/// `None`. Falling back to the platform default is always better than
/// resolving every profile, binary and setting to a path that cannot exist.
pub fn read_data_root_pointer(file: &std::path::Path) -> Option<PathBuf> {
  let content = std::fs::read_to_string(file).ok()?;
  let parsed: serde_json::Value = match serde_json::from_str(&content) {
    Ok(value) => value,
    Err(e) => {
      log::warn!(
        "Ignoring the data directory pointer at {}: it is not valid JSON ({e})",
        file.display()
      );
      return None;
    }
  };
  let path = PathBuf::from(parsed.get("path")?.as_str()?);
  if path.as_os_str().is_empty() || !path.is_absolute() {
    log::warn!(
      "Ignoring the data directory pointer at {}: {} is not an absolute path",
      file.display(),
      path.display()
    );
    return None;
  }
  Some(path)
}

/// Record a data directory for the next start. Written atomically, because a
/// truncated pointer read at startup would silently drop the user back onto
/// the platform default with an empty profile list.
pub fn write_data_root_pointer(
  file: &std::path::Path,
  path: &std::path::Path,
) -> std::io::Result<()> {
  if let Some(parent) = file.parent() {
    std::fs::create_dir_all(parent)?;
  }
  let body = serde_json::json!({ "path": path.to_string_lossy() }).to_string();
  let temp = file.with_extension("json.tmp");
  std::fs::write(&temp, body.as_bytes())?;
  std::fs::rename(&temp, file)
}

/// Forget a recorded data directory, returning the app to the default.
pub fn clear_data_root_pointer(file: &std::path::Path) -> std::io::Result<()> {
  match std::fs::remove_file(file) {
    Ok(()) => Ok(()),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
    Err(e) => Err(e),
  }
}

/// The data directory a previous move chose, read once per process.
///
/// Cached deliberately. Every open handle, cached path and loaded manager in a
/// running app points at the directory it started on, so a move must take
/// effect at the NEXT start and never mid-session.
pub fn custom_data_root() -> Option<&'static PathBuf> {
  CUSTOM_DATA_ROOT
    .get_or_init(|| read_data_root_pointer(&data_root_pointer_file()))
    .as_ref()
}

/// True when app state has been moved off the platform default location, by
/// portable mode, either directory override, or a data directory the user
/// chose in Settings.
fn state_is_relocated() -> bool {
  std::env::var_os("DONUTBROWSER_DATA_DIR").is_some_and(|v| !v.is_empty())
    || custom_data_root().is_some()
    || data_root().is_some()
    || portable_dir().is_some()
}

/// Absolute path the window-state file should live at, or `None` to leave the
/// plugin on its platform default.
///
/// `tauri-plugin-window-state` resolves its file as
/// `app_config_dir().join(filename)` and exposes no way to change the
/// directory, so the only lever is the file name. Handing it an ABSOLUTE path
/// works because `Path::join` discards the base when the argument is absolute,
/// which lands the file with the rest of our relocated state instead of on the
/// host machine. If a future plugin version sanitises the name to a bare file
/// component this silently reverts to the default directory, which is why the
/// first-run probe in `lib.rs` reads this same function rather than assuming.
///
/// A relocation that resolves to a relative path is rejected: see
/// `window_state_override_for`.
pub fn window_state_path_override() -> Option<PathBuf> {
  window_state_override_for(state_is_relocated(), data_dir())
}

/// Split out from `window_state_path_override` so the absolute-path rule is
/// testable without mutating process-wide environment variables.
///
/// A relative override is worse than no override: the plugin would resolve it
/// against `app_config_dir` and write into an intermediate directory it never
/// creates, so every save fails with ENOENT and is swallowed by the plugin's
/// fire-and-forget exit handler. Falling back to the platform default at least
/// persists geometry.
fn window_state_override_for(relocated: bool, data_dir: PathBuf) -> Option<PathBuf> {
  if !relocated {
    return None;
  }
  let path = data_dir.join(WINDOW_STATE_FILENAME);
  if !path.is_absolute() {
    log::warn!(
      "Ignoring relative window-state override {}: the plugin resolves its filename against app_config_dir, so geometry would never persist. Set DONUTBROWSER_DATA_DIR/DONUTBROWSER_DATA_ROOT to an absolute path.",
      path.display()
    );
    return None;
  }
  Some(path)
}

/// Where the window-state file actually is, override or not. Used for the
/// first-run probe, which must agree with whatever the plugin was configured
/// with or portable installs re-apply the default geometry on every launch.
pub fn window_state_path<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> Option<PathBuf> {
  if let Some(path) = window_state_path_override() {
    return Some(path);
  }
  use tauri::Manager;
  handle
    .path()
    .app_config_dir()
    .ok()
    .map(|dir| dir.join(WINDOW_STATE_FILENAME))
}

pub fn app_name() -> &'static str {
  if cfg!(debug_assertions) {
    "DonutBrowserDev"
  } else {
    "DonutBrowser"
  }
}

pub fn data_dir() -> PathBuf {
  #[cfg(test)]
  {
    if let Some(dir) = TEST_DATA_DIR.with(|cell| cell.borrow().clone()) {
      return dir;
    }
  }

  data_dir_for(
    std::env::var_os("DONUTBROWSER_DATA_DIR")
      .filter(|v| !v.is_empty())
      .map(PathBuf::from),
    custom_data_root(),
    data_root(),
    portable_dir(),
    base_dirs().data_local_dir().join(app_name()),
  )
}

/// The data directory resolution order, split out so it can be tested without
/// mutating process-wide environment variables.
///
/// `DONUTBROWSER_DATA_DIR` stays on top: it names an exact directory and is the
/// bluntest override there is. The directory the user picked in Settings comes
/// next, ahead of `DONUTBROWSER_DATA_ROOT` and portable mode, because both of
/// those are defaults for where state *would* live and an explicit choice
/// outranks a default. It cannot break an isolated run, because the pointer it
/// is read from lives under that same `DONUTBROWSER_DATA_ROOT`.
fn data_dir_for(
  env_data_dir: Option<PathBuf>,
  custom_root: Option<&PathBuf>,
  env_data_root: Option<PathBuf>,
  portable: Option<&PathBuf>,
  platform_default: PathBuf,
) -> PathBuf {
  if let Some(dir) = env_data_dir {
    return dir;
  }
  if let Some(dir) = custom_root {
    return dir.clone();
  }
  if let Some(root) = env_data_root {
    return root.join("data");
  }
  if let Some(dir) = portable {
    return dir.join("data");
  }
  platform_default
}

/// Where the data directory would resolve with no user choice recorded. Shown
/// in Settings so a person can see what they moved away from.
pub fn default_data_dir() -> PathBuf {
  data_dir_for(
    std::env::var_os("DONUTBROWSER_DATA_DIR")
      .filter(|v| !v.is_empty())
      .map(PathBuf::from),
    None,
    data_root(),
    portable_dir(),
    base_dirs().data_local_dir().join(app_name()),
  )
}

/// True when an environment override decides the data directory, so a
/// directory chosen in Settings would be recorded but not used.
pub fn data_dir_forced_by_environment() -> bool {
  std::env::var_os("DONUTBROWSER_DATA_DIR").is_some_and(|v| !v.is_empty())
}

pub fn cache_dir() -> PathBuf {
  #[cfg(test)]
  {
    if let Some(dir) = TEST_CACHE_DIR.with(|cell| cell.borrow().clone()) {
      return dir;
    }
  }

  if let Ok(dir) = std::env::var("DONUTBROWSER_CACHE_DIR") {
    return PathBuf::from(dir);
  }

  if let Some(root) = data_root() {
    return root.join("cache");
  }

  if let Some(dir) = portable_dir() {
    return dir.join("cache");
  }

  base_dirs().cache_dir().join(app_name())
}

pub fn profiles_dir() -> PathBuf {
  data_dir().join("profiles")
}

pub fn binaries_dir() -> PathBuf {
  data_dir().join("binaries")
}

pub fn data_subdir() -> PathBuf {
  data_dir().join("data")
}

pub fn settings_dir() -> PathBuf {
  data_dir().join("settings")
}

pub fn proxies_dir() -> PathBuf {
  data_dir().join("proxies")
}

pub fn proxy_workers_dir() -> PathBuf {
  cache_dir().join("proxy_workers")
}

pub fn vpn_dir() -> PathBuf {
  data_dir().join("vpn")
}

pub fn extensions_dir() -> PathBuf {
  data_dir().join("extensions")
}

pub fn dns_blocklist_dir() -> PathBuf {
  cache_dir().join("dns_blocklists")
}

/// Resolve the directory that tauri-plugin-log writes to. Mirrors the
/// `LogDir` target used in the plugin builder so the path matches what's
/// actually on disk for this OS.
pub fn log_dir<R: tauri::Runtime>(handle: &tauri::AppHandle<R>) -> PathBuf {
  if let Some(dir) = log_dir_override() {
    return dir;
  }
  use tauri::Manager;
  handle
    .path()
    .app_log_dir()
    .unwrap_or_else(|_| std::env::temp_dir())
}

#[cfg(test)]
thread_local! {
  static TEST_DATA_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
  static TEST_CACHE_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub struct TestDirGuard {
  kind: TestDirKind,
}

#[cfg(test)]
enum TestDirKind {
  Data,
  Cache,
}

#[cfg(test)]
impl Drop for TestDirGuard {
  fn drop(&mut self) {
    match self.kind {
      TestDirKind::Data => TEST_DATA_DIR.with(|cell| *cell.borrow_mut() = None),
      TestDirKind::Cache => TEST_CACHE_DIR.with(|cell| *cell.borrow_mut() = None),
    }
  }
}

#[cfg(test)]
pub fn set_test_data_dir(dir: PathBuf) -> TestDirGuard {
  TEST_DATA_DIR.with(|cell| *cell.borrow_mut() = Some(dir));
  TestDirGuard {
    kind: TestDirKind::Data,
  }
}

#[cfg(test)]
pub fn set_test_cache_dir(dir: PathBuf) -> TestDirGuard {
  TEST_CACHE_DIR.with(|cell| *cell.borrow_mut() = Some(dir));
  TestDirGuard {
    kind: TestDirKind::Cache,
  }
}

/// Restrict a just-written file to owner-only read/write (`0600`) on Unix so
/// other local users/processes can't read secret material (tokens, E2E
/// password, encrypted vault files). Best-effort: the write already succeeded,
/// so a permission failure is logged, not propagated. On Windows the per-user
/// profile ACL already restricts access, so this is a no-op there.
pub fn restrict_to_owner(path: &std::path::Path) {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
      log::warn!("Failed to restrict permissions on {}: {e}", path.display());
    }
  }
  #[cfg(not(unix))]
  {
    let _ = path;
  }
}

/// Write sensitive data without creating a wider-permission file first.
pub fn create_owner_only(path: &std::path::Path) -> std::io::Result<std::fs::File> {
  if path.exists() {
    restrict_to_owner(path);
  }
  let mut options = std::fs::OpenOptions::new();
  options.create(true).truncate(true).write(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
  }
  let file = options.open(path)?;
  restrict_to_owner(path);
  Ok(file)
}

pub fn write_owner_only(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
  use std::io::Write;
  let mut file = create_owner_only(path)?;
  file.write_all(content)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_app_name() {
    let name = app_name();
    assert!(
      name == "DonutBrowser" || name == "DonutBrowserDev",
      "app_name should be DonutBrowser or DonutBrowserDev, got: {name}"
    );
  }

  #[cfg(unix)]
  #[test]
  fn owner_only_writer_uses_private_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("secret.json");
    write_owner_only(&path, b"secret").unwrap();
    assert_eq!(
      std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
      0o600
    );
  }

  #[test]
  fn test_data_dir_returns_path() {
    let dir = data_dir();
    // Portable mode deliberately drops the app_name segment: state lives at
    // <exe dir>/data. The assertion only holds for the platform-default path.
    if is_portable() {
      assert!(dir.ends_with("data"));
    } else {
      assert!(
        dir.to_string_lossy().contains(app_name()),
        "data_dir should contain app_name"
      );
    }
  }

  #[test]
  fn test_cache_dir_returns_path() {
    let dir = cache_dir();
    if is_portable() {
      assert!(dir.ends_with("cache"));
    } else {
      assert!(
        dir.to_string_lossy().contains(app_name()),
        "cache_dir should contain app_name"
      );
    }
  }

  #[test]
  fn log_dir_follows_portable_mode_and_data_root() {
    let root = PathBuf::from("/tmp/donut-root");
    let portable = PathBuf::from("/tmp/donut-portable");

    // Neither: the platform default app log dir is used.
    assert_eq!(log_dir_for(None, None), None);

    // Portable alone keeps logs beside the executable rather than on the host.
    assert_eq!(
      log_dir_for(None, Some(&portable)),
      Some(portable.join("logs"))
    );

    // DONUTBROWSER_DATA_ROOT wins over portable, matching data_dir/cache_dir.
    assert_eq!(
      log_dir_for(Some(root.clone()), Some(&portable)),
      Some(root.join("logs"))
    );
    assert_eq!(
      log_dir_for(Some(root.clone()), None),
      Some(root.join("logs"))
    );
  }

  #[test]
  fn absolute_filename_escapes_the_plugin_base_dir() {
    // The whole window-state redirect rests on this std behaviour: joining an
    // absolute path discards the base. tauri-plugin-window-state does
    // `app_config_dir().join(filename)`, so an absolute "filename" relocates
    // the file. If this ever stops holding, the redirect silently stops too.
    let base = PathBuf::from("/Users/someone/Library/Application Support/com.donutbrowser");
    let absolute = PathBuf::from("/Volumes/Stick/Donut/data").join(WINDOW_STATE_FILENAME);
    assert_eq!(base.join(&absolute), absolute);
    assert!(!base.join(&absolute).starts_with(&base));
  }

  #[test]
  fn window_state_stays_at_the_platform_default_for_a_normal_install() {
    // A normal install must not be relocated: moving it would drop the window
    // geometry every existing user already has.
    if !state_is_relocated() {
      assert_eq!(window_state_path_override(), None);
    }
  }

  #[test]
  fn window_state_override_rejects_a_relative_data_dir() {
    // `DONUTBROWSER_DATA_ROOT=don-state` (or a relative DATA_DIR) would hand the
    // plugin a relative filename it resolves against app_config_dir, into a
    // directory nothing creates. Falling back to the default keeps geometry.
    assert_eq!(
      window_state_override_for(true, PathBuf::from("don-state/data")),
      None
    );
    assert_eq!(window_state_override_for(true, PathBuf::from("")), None);

    // temp_dir is absolute on every platform; a hard-coded "/tmp/..." is not
    // absolute on Windows, where these tests also run.
    let relocated = std::env::temp_dir().join("donut-relocated");
    assert_eq!(
      window_state_override_for(true, relocated.clone()),
      Some(relocated.join(WINDOW_STATE_FILENAME))
    );
    assert_eq!(window_state_override_for(false, relocated), None);
  }

  #[test]
  fn window_state_follows_a_relocated_data_dir() {
    let tmp = PathBuf::from("/tmp/donut-relocated");
    let _guard = set_test_data_dir(tmp.clone());
    // data_dir is overridden, so the file tracks it rather than app_config_dir.
    assert_eq!(
      data_dir().join(WINDOW_STATE_FILENAME),
      tmp.join(".window-state.json")
    );
  }

  #[test]
  fn portable_keeps_data_cache_and_logs_under_one_root() {
    // The three state directories must agree on where portable state lives, so
    // a portable install leaves nothing behind on the host.
    let portable = PathBuf::from("/tmp/donut-portable");
    assert_eq!(
      log_dir_for(None, Some(&portable)),
      Some(portable.join("logs"))
    );
    assert!(portable.join("data").starts_with(&portable));
    assert!(portable.join("cache").starts_with(&portable));
  }

  #[test]
  fn data_dir_resolution_order_puts_the_chosen_directory_under_the_exact_override() {
    let env_dir = PathBuf::from("/env/exact");
    let chosen = PathBuf::from("/Volumes/Big/DonutBrowser");
    let env_root = PathBuf::from("/env/root");
    let portable = PathBuf::from("/stick");
    let default = PathBuf::from("/home/user/.local/share/DonutBrowser");

    // DONUTBROWSER_DATA_DIR names an exact directory and outranks everything.
    assert_eq!(
      data_dir_for(
        Some(env_dir.clone()),
        Some(&chosen),
        Some(env_root.clone()),
        Some(&portable),
        default.clone(),
      ),
      env_dir
    );

    // The directory the user picked beats both defaults-for-where-state-lives.
    assert_eq!(
      data_dir_for(
        None,
        Some(&chosen),
        Some(env_root.clone()),
        Some(&portable),
        default.clone(),
      ),
      chosen
    );

    // With nothing chosen the existing order is untouched.
    assert_eq!(
      data_dir_for(
        None,
        None,
        Some(env_root.clone()),
        Some(&portable),
        default.clone(),
      ),
      env_root.join("data")
    );
    assert_eq!(
      data_dir_for(None, None, None, Some(&portable), default.clone()),
      portable.join("data")
    );
    assert_eq!(
      data_dir_for(None, None, None, None, default.clone()),
      default
    );
  }

  #[test]
  fn the_pointer_never_lives_inside_the_directory_it_points_at() {
    let root = PathBuf::from("/tmp/donut-root");
    let portable = PathBuf::from("/tmp/donut-portable");
    let preference = PathBuf::from("/home/user/.config/DonutBrowser");

    // With DONUTBROWSER_DATA_ROOT the data dir is <root>/data, so a sibling
    // file survives deleting it — and an isolated run reads only its own.
    let with_root =
      data_root_pointer_file_for(Some(root.clone()), Some(&portable), preference.clone());
    assert_eq!(with_root, root.join(DATA_ROOT_POINTER_FILENAME));
    assert!(!with_root.starts_with(root.join("data")));

    let with_portable = data_root_pointer_file_for(None, Some(&portable), preference.clone());
    assert_eq!(with_portable, portable.join(DATA_ROOT_POINTER_FILENAME));
    assert!(!with_portable.starts_with(portable.join("data")));

    assert_eq!(
      data_root_pointer_file_for(None, None, preference.clone()),
      preference.join(DATA_ROOT_POINTER_FILENAME)
    );
  }

  #[test]
  fn a_written_pointer_reads_back_and_a_broken_one_falls_back() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("nested").join(DATA_ROOT_POINTER_FILENAME);
    let target = std::env::temp_dir().join("donut-moved-root");

    assert_eq!(read_data_root_pointer(&file), None, "missing file");

    write_data_root_pointer(&file, &target).unwrap();
    assert_eq!(read_data_root_pointer(&file), Some(target.clone()));

    // A relative path would be resolved against whatever the working directory
    // happens to be, which is not a place app state can live.
    write_data_root_pointer(&file, std::path::Path::new("relative/root")).unwrap();
    assert_eq!(read_data_root_pointer(&file), None, "relative path");

    std::fs::write(&file, b"not json at all").unwrap();
    assert_eq!(read_data_root_pointer(&file), None, "malformed file");

    std::fs::write(&file, br#"{"other":"key"}"#).unwrap();
    assert_eq!(read_data_root_pointer(&file), None, "no path entry");

    write_data_root_pointer(&file, &target).unwrap();
    clear_data_root_pointer(&file).unwrap();
    assert_eq!(read_data_root_pointer(&file), None, "cleared");
    // Clearing an absent pointer is not an error; the caller has nothing to fix.
    clear_data_root_pointer(&file).unwrap();
  }

  #[test]
  fn test_subdirectory_helpers() {
    assert!(profiles_dir().ends_with("profiles"));
    assert!(binaries_dir().ends_with("binaries"));
    assert!(data_subdir().ends_with("data"));
    assert!(settings_dir().ends_with("settings"));
    assert!(proxies_dir().ends_with("proxies"));
    assert!(proxy_workers_dir().ends_with("proxy_workers"));
    assert!(vpn_dir().ends_with("vpn"));
    assert!(extensions_dir().ends_with("extensions"));
    assert!(dns_blocklist_dir().ends_with("dns_blocklists"));
  }

  #[test]
  fn test_set_test_data_dir() {
    let tmp = PathBuf::from("/tmp/test-donut-data");
    let _guard = set_test_data_dir(tmp.clone());
    assert_eq!(data_dir(), tmp);
    assert_eq!(profiles_dir(), tmp.join("profiles"));
    assert_eq!(binaries_dir(), tmp.join("binaries"));
  }

  #[test]
  fn test_set_test_cache_dir() {
    let tmp = PathBuf::from("/tmp/test-donut-cache");
    let _guard = set_test_cache_dir(tmp.clone());
    assert_eq!(cache_dir(), tmp);
  }

  #[test]
  fn test_guard_cleanup() {
    let original_data = data_dir();
    let original_cache = cache_dir();

    {
      let _guard = set_test_data_dir(PathBuf::from("/tmp/test-cleanup-data"));
      assert_eq!(data_dir(), PathBuf::from("/tmp/test-cleanup-data"));
    }
    assert_eq!(data_dir(), original_data);

    {
      let _guard = set_test_cache_dir(PathBuf::from("/tmp/test-cleanup-cache"));
      assert_eq!(cache_dir(), PathBuf::from("/tmp/test-cleanup-cache"));
    }
    assert_eq!(cache_dir(), original_cache);
  }
}

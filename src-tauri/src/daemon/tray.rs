use muda::{Menu, MenuItem};
use std::process::Command;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub fn load_icon() -> Icon {
  // On Windows, use the full-color icon so it renders well on dark taskbars.
  // On macOS/Linux, use the template icon (black with alpha) for system light/dark handling.
  #[cfg(target_os = "windows")]
  let icon_bytes = include_bytes!("../../icons/tray-icon-win-44.png");
  #[cfg(not(target_os = "windows"))]
  let icon_bytes = include_bytes!("../../icons/tray-icon-44.png");

  let image = image::load_from_memory(icon_bytes)
    .expect("Failed to load icon")
    .into_rgba8();

  let (width, height) = image.dimensions();
  let rgba = image.into_raw();

  Icon::from_rgba(rgba, width, height).expect("Failed to create icon")
}

pub struct TrayMenu {
  pub menu: Menu,
  pub quit_item: MenuItem,
}

impl Default for TrayMenu {
  fn default() -> Self {
    Self::new()
  }
}

impl TrayMenu {
  pub fn new() -> Self {
    let menu = Menu::new();

    let quit_item = MenuItem::new("Quit Donut Browser", true, None);

    menu.append(&quit_item).unwrap();

    Self { menu, quit_item }
  }
}

pub fn create_tray_icon(icon: Icon, menu: &Menu) -> TrayIcon {
  let builder = TrayIconBuilder::new()
    .with_icon(icon)
    .with_tooltip("Donut Browser")
    .with_menu(Box::new(menu.clone()));

  // On macOS, template icons are automatically colored by the system for light/dark mode
  #[cfg(target_os = "macos")]
  let builder = builder.with_icon_as_template(true);

  builder.build().expect("Failed to create tray icon")
}

/// Resolve the .app bundle path from the current daemon executable.
/// In production the daemon is at `Donut.app/Contents/MacOS/donut-daemon`.
#[cfg(target_os = "macos")]
fn get_app_bundle_path() -> Option<std::path::PathBuf> {
  let exe = std::env::current_exe().ok()?;
  let macos_dir = exe.parent()?;
  let contents_dir = macos_dir.parent()?;
  let app_dir = contents_dir.parent()?;
  if app_dir.extension().and_then(|e| e.to_str()) == Some("app") {
    Some(app_dir.to_path_buf())
  } else {
    None
  }
}

pub fn open_gui() {
  log::info!("Opening GUI...");

  // On macOS, use `open` WITHOUT `-n`. The daemon runs with Accessory
  // activation policy so macOS won't confuse it with the GUI process.
  // `open` will either activate the existing GUI or launch a new one.
  // Using `-n` would bypass the single-instance plugin entirely.
  #[cfg(target_os = "macos")]
  {
    // Use `open -n` to force launching a new process. Without `-n`, macOS
    // re-activates the daemon (the existing process from the bundle) instead
    // of launching the GUI binary. The single-instance Tauri plugin in the
    // GUI handles deduplication if a GUI instance is already running.
    if let Some(app_bundle) = get_app_bundle_path() {
      let _ = Command::new("open").args(["-n"]).arg(&app_bundle).spawn();
    } else {
      let _ = Command::new("open").args(["-n", "-a", "Donut"]).spawn();
    }
  }

  #[cfg(target_os = "windows")]
  {
    use std::path::PathBuf;

    if let Ok(current_exe) = std::env::current_exe() {
      if let Some(exe_dir) = current_exe.parent() {
        let app_path = exe_dir.join("donutbrowser.exe");
        if app_path.exists() {
          let _ = Command::new(app_path).spawn();
          return;
        }
      }
    }

    let paths = [
      dirs::data_local_dir().map(|p| p.join("Donut Browser").join("Donut Browser.exe")),
      Some(PathBuf::from(
        "C:\\Program Files\\Donut Browser\\Donut Browser.exe",
      )),
    ];

    for path in paths.iter().flatten() {
      if path.exists() {
        let _ = Command::new(path).spawn();
        return;
      }
    }
  }

  #[cfg(target_os = "linux")]
  {
    let _ = Command::new("donutbrowser").spawn();
  }
}

fn read_gui_pid() -> Option<u32> {
  let path = super::autostart::get_data_dir()?.join("daemon-state.json");
  let content = std::fs::read_to_string(path).ok()?;
  let val: serde_json::Value = serde_json::from_str(&content).ok()?;
  val.get("gui_pid")?.as_u64().map(|p| p as u32)
}

fn kill_gui_by_pid() -> bool {
  let Some(pid) = read_gui_pid() else {
    return false;
  };

  #[cfg(unix)]
  {
    let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    ret == 0
  }

  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    Command::new("taskkill")
      .args(["/PID", &pid.to_string(), "/F"])
      .creation_flags(CREATE_NO_WINDOW)
      .output()
      .map(|o| o.status.success())
      .unwrap_or(false)
  }

  #[cfg(not(any(unix, windows)))]
  {
    false
  }
}

pub fn quit_gui() {
  log::info!("[daemon] Quitting GUI...");

  if kill_gui_by_pid() {
    log::info!("[daemon] GUI killed by PID");
    return;
  }

  log::info!("[daemon] PID-based kill failed, falling back to name-based kill");

  #[cfg(target_os = "macos")]
  {
    // Use spawn() instead of output() to avoid blocking the event loop.
    // AppleScript has a ~2 minute default timeout that would freeze the tray icon.
    let _ = Command::new("osascript")
      .args(["-e", "tell application \"Donut\" to quit"])
      .spawn();
  }

  #[cfg(target_os = "windows")]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let _ = Command::new("taskkill")
      .args(["/IM", "Donut.exe", "/F"])
      .creation_flags(CREATE_NO_WINDOW)
      .spawn();
    let _ = Command::new("taskkill")
      .args(["/IM", "donutbrowser.exe", "/F"])
      .creation_flags(CREATE_NO_WINDOW)
      .spawn();
  }

  #[cfg(target_os = "linux")]
  {
    let _ = Command::new("pkill").args(["-x", "donutbrowser"]).spawn();
  }
}

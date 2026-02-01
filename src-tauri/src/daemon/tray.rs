use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

static GUI_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn load_icon() -> Icon {
  // Use the generated template icon (44x44 for retina, macOS standard menu bar size)
  // This is the donut logo converted to template format (black with alpha)
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
  pub open_item: MenuItem,
  pub running_profiles_submenu: Submenu,
  pub api_status_item: MenuItem,
  pub mcp_status_item: MenuItem,
  pub preferences_item: MenuItem,
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

    let open_item = MenuItem::new("Open Donut Browser", true, None);
    let running_profiles_submenu = Submenu::new("Running Profiles", true);
    let no_profiles_item = MenuItem::new("No running profiles", false, None);
    running_profiles_submenu.append(&no_profiles_item).unwrap();

    let separator1 = PredefinedMenuItem::separator();
    let api_status_item = MenuItem::new("API: Starting...", false, None);
    let mcp_status_item = MenuItem::new("MCP: Starting...", false, None);
    let separator2 = PredefinedMenuItem::separator();
    let preferences_item = MenuItem::new("Preferences...", true, None);
    let quit_item = MenuItem::new("Quit Donut Browser", true, None);

    menu.append(&open_item).unwrap();
    menu.append(&running_profiles_submenu).unwrap();
    menu.append(&separator1).unwrap();
    menu.append(&api_status_item).unwrap();
    menu.append(&mcp_status_item).unwrap();
    menu.append(&separator2).unwrap();
    menu.append(&preferences_item).unwrap();
    menu.append(&quit_item).unwrap();

    Self {
      menu,
      open_item,
      running_profiles_submenu,
      api_status_item,
      mcp_status_item,
      preferences_item,
      quit_item,
    }
  }

  pub fn update_api_status(&self, port: Option<u16>) {
    let text = match port {
      Some(p) => format!("API: Running on :{}", p),
      None => "API: Stopped".to_string(),
    };
    self.api_status_item.set_text(&text);
  }

  pub fn update_mcp_status(&self, running: bool) {
    let text = if running {
      "MCP: Running"
    } else {
      "MCP: Stopped"
    };
    self.mcp_status_item.set_text(text);
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

pub fn open_gui() {
  if GUI_RUNNING.load(Ordering::SeqCst) {
    log::info!("GUI already running, activating...");
    activate_gui();
    return;
  }

  log::info!("Opening GUI...");

  #[cfg(target_os = "macos")]
  {
    let _ = Command::new("open").arg("-a").arg("Donut Browser").spawn();
  }

  #[cfg(target_os = "windows")]
  {
    use std::path::PathBuf;

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

pub fn activate_gui() {
  #[cfg(target_os = "macos")]
  {
    let _ = Command::new("osascript")
      .args(["-e", "tell application \"Donut Browser\" to activate"])
      .spawn();
  }
}

pub fn set_gui_running(running: bool) {
  GUI_RUNNING.store(running, Ordering::SeqCst);
}

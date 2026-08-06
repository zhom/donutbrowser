//! Client-side window decorations on Linux.
//!
//! The app draws its own titlebar (as it already does on macOS and Windows), so
//! the window is built without server-side decorations. That means the app also
//! owns the window *controls*, and their side and order are a desktop-wide user
//! preference that differs between environments — GNOME defaults to
//! `:minimize,maximize,close` (all on the right), and a user who has moved them
//! to the left expects every app to follow.
//!
//! `GtkSettings::gtk-decoration-layout` is the one place every desktop
//! publishes that preference to GTK applications: GNOME mirrors
//! `org.gnome.desktop.wm.preferences button-layout` into it, and on KDE Plasma
//! `kde-gtk-config` mirrors KWin's decoration button configuration into it.
//! Reading this property therefore gets both environments right without any
//! desktop-specific branching.

use serde::Serialize;

/// Whether this window draws its own titlebar, and how.
#[derive(Debug, Clone, Serialize)]
pub struct WindowDecorations {
  /// True when the app owns the titlebar and must draw controls and resize
  /// edges. False means the platform still draws a real titlebar and the
  /// frontend must render nothing.
  pub client_side: bool,
  /// The desktop's button layout, e.g. `":minimize,maximize,close"`. Only
  /// meaningful when `client_side` is true.
  pub layout: Option<String>,
}

/// Whether to drop server-side decorations on this Linux session.
///
/// Enabled everywhere except KDE Plasma on Wayland, and overridable with
/// `DONUT_LINUX_CLIENT_DECORATIONS=1|0`.
///
/// The KDE/Wayland exclusion is deliberate and is about a failure mode, not a
/// preference. GTK3 speaks no `xdg-decoration`; when a window is built
/// undecorated, GTK does not mark it client-decorated, and on Wayland it
/// therefore *announces server-side decorations* to the compositor. mutter
/// ignores that (it never decorates Wayland toplevels), which is why GNOME
/// works. KWin honors it, so Plasma would be free to draw a Breeze titlebar
/// directly above the one the app draws — two titlebars, worse than the
/// feature is good. Whether it actually does depends on the decoration mode
/// KWin advertises, which could not be established from documentation and
/// cannot be tested from here, so this stays off until somebody can run it.
///
/// KDE on X11 is *not* excluded: there the request travels as `_MOTIF_WM_HINTS`,
/// which KWin has honored for as long as it has existed.
#[cfg(target_os = "linux")]
pub fn use_client_side_decorations() -> bool {
  if let Ok(value) = std::env::var("DONUT_LINUX_CLIENT_DECORATIONS") {
    let forced = matches!(value.trim(), "1" | "true" | "yes");
    log::info!("Client-side decorations forced to {forced} by DONUT_LINUX_CLIENT_DECORATIONS");
    return forced;
  }

  let env = |key: &str| std::env::var(key).unwrap_or_default().to_lowercase();

  // GDK_BACKEND is a comma-separated preference list ("wayland,x11"), and GDK
  // takes the FIRST entry it can open. Testing for a substring would read
  // "wayland,x11" as X11 and hand a Plasma Wayland session the undecorated
  // path this guard exists to withhold.
  let backend = env("GDK_BACKEND");
  let preferred = backend
    .split(',')
    .map(str::trim)
    .find(|value| !value.is_empty());
  let on_wayland = match preferred {
    Some("x11") => false,
    Some("wayland") => true,
    // Unset or something exotic: fall back to what the session advertises.
    _ => {
      !std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_default()
        .is_empty()
        && env("XDG_SESSION_TYPE") != "x11"
    }
  };

  let on_kde = env("XDG_CURRENT_DESKTOP").contains("kde")
    || env("XDG_SESSION_DESKTOP").contains("plasma")
    || env("DESKTOP_SESSION").contains("plasma")
    || !std::env::var("KDE_FULL_SESSION")
      .unwrap_or_default()
      .is_empty();

  if on_kde && on_wayland {
    log::info!(
      "Keeping server-side decorations: KWin on Wayland may draw its own titlebar over the \
       app's. Set DONUT_LINUX_CLIENT_DECORATIONS=1 to override."
    );
    return false;
  }
  true
}

#[cfg(target_os = "linux")]
mod imp {
  use std::sync::Mutex;

  lazy_static::lazy_static! {
    static ref LAYOUT: Mutex<Option<String>> = Mutex::new(None);
  }

  fn store(layout: Option<String>) {
    if let Ok(mut slot) = LAYOUT.lock() {
      *slot = layout;
    }
  }

  pub fn cached() -> Option<String> {
    LAYOUT.lock().ok().and_then(|slot| slot.clone())
  }

  /// Read the layout and subscribe to changes.
  ///
  /// MUST be called on the GTK main thread — `gtk::Settings::default()` panics
  /// elsewhere, and the notify subscription has to be attached from the thread
  /// owning the GTK main context. The app's `setup` hook already runs there.
  pub fn init<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use gtk::prelude::*;
    use tauri::Emitter;

    let Some(settings) = gtk::Settings::default() else {
      log::warn!("No GTK settings available; using the default decoration layout");
      return;
    };

    store(settings.gtk_decoration_layout().map(|v| v.to_string()));
    log::info!(
      "Window decoration layout: {}",
      cached().as_deref().unwrap_or("<unset>")
    );

    // The user can rearrange titlebar buttons while the app is running, and
    // every other application follows immediately. Note this never fires where
    // the value comes only from gtk-3.0/settings.ini (a KDE X11 box with no
    // xsettings daemon) — there it is simply static until restart.
    let handle = app.clone();
    settings.connect_gtk_decoration_layout_notify(move |settings| {
      let layout = settings.gtk_decoration_layout().map(|v| v.to_string());
      log::info!(
        "Window decoration layout changed to: {}",
        layout.as_deref().unwrap_or("<unset>")
      );
      store(layout.clone());
      if let Err(e) = handle.emit("window-decoration-layout-changed", layout) {
        log::warn!("Failed to emit window decoration layout change: {e}");
      }
    });
  }
}

#[cfg(not(target_os = "linux"))]
mod imp {
  pub fn init<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) {}
}

pub use imp::init;

/// How this window is decorated, and the desktop's button layout when the app
/// owns the titlebar.
#[tauri::command]
pub fn get_window_decoration_layout() -> WindowDecorations {
  #[cfg(target_os = "linux")]
  {
    let client_side = use_client_side_decorations();
    WindowDecorations {
      client_side,
      layout: if client_side { imp::cached() } else { None },
    }
  }
  // Every other platform keeps a real titlebar: macOS makes the native one
  // transparent, Windows draws its own controls on a fixed layout.
  #[cfg(not(target_os = "linux"))]
  {
    WindowDecorations {
      client_side: false,
      layout: None,
    }
  }
}

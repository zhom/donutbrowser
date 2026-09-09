use crate::browser_runner::BrowserRunner;
use crate::profile::BrowserProfile;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex as AsyncMutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WayfernConfig {
  /// LEGACY device payload, carried only by a profile whose browser has no
  /// identity API. Every other profile is rebuilt from `identity_id`, so this
  /// is read from older metadata and from a caller that supplies a whole
  /// device, and is never written once the profile has an identity.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub fingerprint: Option<String>,
  #[serde(default)]
  pub randomize_fingerprint_on_launch: Option<bool>,
  #[serde(default)]
  pub os: Option<String>,
  #[serde(default)]
  pub screen_max_width: Option<u32>,
  #[serde(default)]
  pub screen_max_height: Option<u32>,
  #[serde(default)]
  pub screen_min_width: Option<u32>,
  #[serde(default)]
  pub screen_min_height: Option<u32>,
  #[serde(default)]
  pub geoip: Option<serde_json::Value>, // For compatibility with shared config form
  #[serde(default)]
  pub block_images: Option<bool>, // For compatibility with shared config form
  /// LEGACY on/off switch kept for stored configs; `webrtc_mode` wins when
  /// both are present, and `Some(true)` alone reads as `block`.
  #[serde(default)]
  pub block_webrtc: Option<bool>,
  /// How WebRTC may reach the network: `auto` (real STUN only where the UDP
  /// it needs is carried by the route, else the exit-IP posture), `tcp_only`
  /// (one server-reflexive candidate on the proxy's exit IP), or `block` (no
  /// ICE candidates at all). `None` is `auto`.
  #[serde(default)]
  pub webrtc_mode: Option<String>,
  /// The user's edits to the profile's persona, as a JSON array of
  /// `{id,label,value}`. Everything not edited is derived from the profile's
  /// own seed, so this holds edits and nothing else. An empty value removes
  /// that row from what the browser offers.
  #[serde(default)]
  pub persona: Option<String>,
  /// A still (.png) or clip (.y4m/.mjpeg) the claimed camera serves, as an
  /// absolute path. `None` means the camera serves dark frames; the browser
  /// never falls back to the host device.
  #[serde(default)]
  pub camera_file: Option<String>,
  /// `x,y,width,height` in SOURCE pixels, cropped before the frame is scaled.
  #[serde(default)]
  pub camera_crop: Option<String>,
  /// Whether an interactive launch reopens the windows and tabs of the last
  /// session. `None` is the default, which is on. Automation, headless,
  /// ephemeral and clear-on-close launches never restore, whatever this says,
  /// and neither does a browser that cannot take the identity at launch: a
  /// restored tab loads before any post-launch `setIdentity`, so it would
  /// carry the host device for its whole lifetime.
  #[serde(default)]
  pub restore_session: Option<bool>,
  #[serde(default)]
  pub block_webgl: Option<bool>,
  #[serde(default, skip_serializing)]
  pub proxy: Option<String>,
  /// Stable signature of the proxy/VPN/geoip the fingerprint's location data
  /// (timezone, latitude/longitude, language) was last computed for. Compared
  /// on launch to detect that the routing changed since creation, so the
  /// location can be refreshed instead of showing stale data.
  #[serde(default)]
  pub geo_proxy_signature: Option<String>,
  /// Identity handle for this profile, when it has one. An identity-backed
  /// profile stores the id, its `location` and its `identity_overrides` and
  /// NOTHING else: the device is rebuilt from the id by the browser on every
  /// launch, so no fingerprint payload ever sits on disk to be copied.
  /// `None` means a legacy profile that still stores a whole payload in
  /// `fingerprint` and is applied with `Wayfern.setFingerprint`.
  #[serde(default)]
  pub identity_id: Option<String>,
  /// LEGACY, read only by `migrate_identity_config`: the derived device an
  /// older build snapshotted so the user's edits could be diffed out of the
  /// stored payload. Cleared by the migration and never serialized again, so
  /// a migrated profile carries no trace of it.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub identity_baseline: Option<String>,
  /// The user's own edits to an identity-backed device, as a JSON object of
  /// fingerprint fields. Sent verbatim as `setIdentity` overrides; everything
  /// not listed here comes from the identity. `None` means no edits.
  #[serde(default)]
  pub identity_overrides: Option<String>,
  /// The location the profile's exit resolves to (timezone, timezoneOffset,
  /// language, languages, latitude, longitude, accuracy) as a JSON object.
  /// It depends on the proxy, not on the identity, which is why it is the one
  /// piece of device state an identity-backed profile persists.
  #[serde(default)]
  pub location: Option<String>,
}

/// First Wayfern version that ships `createIdentity`/`setIdentity`/
/// `getIdentity`. Those commands are ADDITIVE: `setFingerprint`,
/// `getFingerprint` and `refreshFingerprint` are all still declared in the 151
/// protocol and still implemented, so this constant means "the identity API is
/// available here", never "the legacy commands are gone". A profile that
/// stores a whole device payload keeps being applied with `setFingerprint` on
/// 151, which is the only command that reproduces such a payload exactly.
///
/// Written as a full version rather than a major so it can be pinned to an
/// exact build if that is ever needed; missing components compare as zero, so
/// `"151"` means "any 151 or newer".
const IDENTITY_API_MIN_VERSION: &str = "151";

/// Whether `version` speaks the identity API.
///
/// `version` must be `BrowserProfile::version`, which is the field
/// `BrowserRunner::get_browser_executable_path` resolves the binary from, so it
/// is by construction the version that will actually launch. Read it at the
/// point of use and never cache it: `auto_updater` rewrites it when the browser
/// stops (`browser_runner.rs`, the pending-update block), so a value captured
/// before that point can describe a binary that is no longer on disk.
///
/// An unparsable version reads as 0.0.0.0 and therefore takes the legacy path,
/// which is the safe direction: the legacy commands exist on every Wayfern that
/// ever shipped, while `createIdentity` on an older build is an unknown method.
pub fn supports_identity_api(version: &str) -> bool {
  crate::api_client::compare_versions(version, IDENTITY_API_MIN_VERSION) != std::cmp::Ordering::Less
}

/// First Wayfern version that ships the 152 launch contract: the identity file
/// (`--wayfern-identity-file`), the `--wayfern-token` switch, WebRTC mode and
/// exit IP, persona/icon/camera/Widevine/entitlement-cache switches, the
/// `Vellum` input domain and the perception/locator/extraction commands.
/// Everything gated on it keeps its pre-152 path on any older browser.
const WAYFERN_152_MIN_VERSION: &str = "152";

/// Whether `version` speaks the Wayfern 152 launch and automation contract.
/// Same rule as [`supports_identity_api`]: pass `BrowserProfile::version`, read
/// it at the point of use, and an unparsable version takes the older path.
pub fn supports_wayfern_152(version: &str) -> bool {
  crate::api_client::compare_versions(version, WAYFERN_152_MIN_VERSION) != std::cmp::Ordering::Less
}

/// Where the launch-time identity document lives inside the profile directory.
/// Rewritten before every launch; the sync manifest excludes it.
pub const LAUNCH_IDENTITY_FILE: &str = "wayfern-identity.json";

/// What kind of launch this is, from the browser's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchKind {
  /// A person opened the profile from the app: the window is theirs, and the
  /// session they left is the one they expect to find again.
  Interactive,
  /// REST, MCP or a batch run drives the browser: it starts clean on the URL
  /// the caller named and never reopens what a person left behind.
  Automation,
}

/// Whether this launch reopens the last session, or why it does not.
///
/// `identity_at_launch` says the device is committed before the first
/// navigation (the 152 identity file), or that there is no device to commit.
/// Anything applied over CDP after the window opens loses the race with a
/// restored tab, which then carries the host device for its whole lifetime,
/// so such a launch starts on a fresh tab instead and the log says why.
pub fn session_restore_verdict(
  config: &WayfernConfig,
  kind: LaunchKind,
  headless: bool,
  ephemeral: bool,
  clear_on_close: bool,
  identity_at_launch: bool,
) -> Result<(), &'static str> {
  if kind == LaunchKind::Automation {
    return Err("an automation run starts clean");
  }
  if headless {
    return Err("a headless launch has no session to show");
  }
  if ephemeral {
    return Err("an ephemeral profile keeps nothing between launches");
  }
  if clear_on_close {
    return Err("the profile clears its data on close");
  }
  if config.randomize_fingerprint_on_launch == Some(true) {
    return Err("a device randomized on every launch has no session to continue");
  }
  if config.restore_session == Some(false) {
    return Err("the profile has session restore switched off");
  }
  if !identity_at_launch {
    return Err(
      "this browser applies the identity after the window opens, so a restored tab would load on the host device",
    );
  }
  Ok(())
}

/// The switches that shape the first window: session restore and the launch
/// identity. Kept apart from the rest of the command line so a test can pin
/// them per launch kind.
pub fn session_switches(restore_session: bool, identity_file: Option<&Path>) -> Vec<String> {
  let mut switches = Vec::new();
  if let Some(path) = identity_file {
    switches.push(format!("--wayfern-identity-file={}", path.display()));
  }
  if restore_session {
    switches.push("--restore-last-session".to_string());
  }
  switches
}

/// The WebRTC posture a 152 browser is launched with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebRtcMode {
  Auto,
  TcpOnly,
  Block,
}

impl WebRtcMode {
  pub fn parse(value: &str) -> Option<Self> {
    match value.trim().to_ascii_lowercase().as_str() {
      "auto" => Some(Self::Auto),
      "tcp_only" | "tcp-only" | "tcponly" => Some(Self::TcpOnly),
      "block" | "blocked" | "off" => Some(Self::Block),
      _ => None,
    }
  }

  /// The mode a stored config asks for. `webrtc_mode` wins; the legacy
  /// `block_webrtc: true` reads as `block`; anything else is `auto`. An
  /// unknown string is `auto` too, logged by the caller, never a launch error.
  pub fn from_config(config: &WayfernConfig) -> Self {
    if let Some(mode) = config.webrtc_mode.as_deref().and_then(Self::parse) {
      return mode;
    }
    if config.block_webrtc == Some(true) {
      return Self::Block;
    }
    Self::Auto
  }

  pub fn switch_value(self) -> &'static str {
    match self {
      Self::Auto => "auto",
      Self::TcpOnly => "tcp_only",
      Self::Block => "block",
    }
  }
}

/// The WebRTC switches for a launch: the mode always, and the exit IP whenever
/// one is known and the mode can use it. A value the browser will not accept
/// costs only the synthetic server-reflexive candidate that makes a TCP-only
/// route look natural, never a leak. Pre-152 browsers do not read these, and
/// get nothing.
pub fn webrtc_switches(version: &str, mode: WebRtcMode, exit_ip: Option<&str>) -> Vec<String> {
  if !supports_wayfern_152(version) {
    return Vec::new();
  }
  let mut switches = vec![format!("--wayfern-webrtc-mode={}", mode.switch_value())];
  if mode != WebRtcMode::Block {
    if let Some(ip) = exit_ip.map(str::trim).filter(|ip| !ip.is_empty()) {
      if let Ok(address) = ip.parse::<std::net::IpAddr>() {
        switches.push(format!("--wayfern-webrtc-exit-ip={address}"));
      }
    }
  }
  switches
}

/// Say so when a device claims a screen the host cannot show.
///
/// One function for both launch paths: an identity-backed profile learns its
/// screen from the running browser, a legacy one carries it on disk, and the
/// sentence is the same either way.
impl WayfernManager {
  fn warn_on_screen_over_host(device_json: &str, profile: &BrowserProfile, app_handle: &AppHandle) {
    if let Some((claimed_w, claimed_h, host_w, host_h)) =
      screen_claim_over_host(Some(device_json), host_screen_size(app_handle))
    {
      log::warn!(
        "Profile {} claims a {claimed_w}x{claimed_h} screen on a {host_w}x{host_h} display: its window can never fill the screen it reports, which a page can measure",
        profile.name
      );
    }
  }
}

/// The claimed screen a stored device presents, in CSS pixels.
fn claimed_screen(fingerprint_json: &str) -> Option<(u32, u32)> {
  let device = WayfernManager::fingerprint_object(fingerprint_json)?;
  let read = |key: &str| device.get(key).and_then(|v| v.as_u64()).map(|v| v as u32);
  Some((read("screenWidth")?, read("screenHeight")?))
}

/// How much bigger the claimed screen is than the display the browser will
/// actually open on, when it is bigger at all.
///
/// A device claiming a screen the host cannot show is visible from the page:
/// the window can never grow to the claimed size, so `outerWidth` stays below
/// `screen.width` however the user maximises it. Donut cannot fix the device
/// at launch without silently changing it, so it says so instead, on the
/// launch report and in the log.
pub fn screen_claim_over_host(
  fingerprint_json: Option<&str>,
  host: Option<(u32, u32)>,
) -> Option<(u32, u32, u32, u32)> {
  let (claimed_width, claimed_height) = claimed_screen(fingerprint_json?)?;
  let (host_width, host_height) = host?;
  if host_width == 0 || host_height == 0 {
    return None;
  }
  (claimed_width > host_width || claimed_height > host_height).then_some((
    claimed_width,
    claimed_height,
    host_width,
    host_height,
  ))
}

/// The primary display's size in CSS pixels, which is what a page reads.
pub fn host_screen_size(app_handle: &AppHandle) -> Option<(u32, u32)> {
  let monitor = app_handle.primary_monitor().ok().flatten()?;
  let scale = monitor.scale_factor();
  let size = monitor.size().to_logical::<f64>(scale);
  if size.width < 1.0 || size.height < 1.0 {
    return None;
  }
  Some((size.width as u32, size.height as u32))
}

/// Where a 152 browser keeps its entitlement cache: inside donut's own cache
/// root rather than the OS default, so it is removed with the app's data and
/// never shared between an e2e session and the real installation.
pub fn entitlement_cache_switch(version: &str, cache_root: &Path) -> Option<String> {
  if !supports_wayfern_152(version) {
    return None;
  }
  let dir = cache_root.join("wayfern-entitlements");
  if let Err(e) = std::fs::create_dir_all(&dir) {
    log::warn!(
      "Could not create the Wayfern entitlement cache at {}: {e}; the browser keeps its default",
      dir.display()
    );
    return None;
  }
  Some(format!("--wayfern-entitlement-cache-dir={}", dir.display()))
}

/// Fonts for the window badge, loaded from the system once per process. The
/// load walks every font directory, which is far too slow to repeat per launch.
fn badge_fonts() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
  static FONTS: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
    std::sync::OnceLock::new();
  FONTS
    .get_or_init(|| {
      let mut db = resvg::usvg::fontdb::Database::new();
      db.load_system_fonts();
      if let Some(family) = badge_sans_family(&db) {
        db.set_sans_serif_family(family);
      }
      std::sync::Arc::new(db)
    })
    .clone()
}

/// The family the badge's `sans-serif` resolves to.
///
/// The database names Arial for the generic family, which macOS and Windows
/// have and a Linux desktop usually does not: Ubuntu ships Noto, DejaVu,
/// Liberation and Ubuntu instead. An unresolved family draws no initial at
/// all, so the first family that is actually installed is chosen, and failing
/// every known name, any installed font at all.
fn badge_sans_family(db: &resvg::usvg::fontdb::Database) -> Option<String> {
  use resvg::usvg::fontdb::{Family, Query, Stretch, Style, Weight};
  const PREFERRED: [&str; 10] = [
    "Arial",
    "Helvetica Neue",
    "Helvetica",
    "Segoe UI",
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "Ubuntu",
    "Cantarell",
    "Roboto",
  ];
  let installed = |name: &str| {
    db.query(&Query {
      families: &[Family::Name(name)],
      weight: Weight::NORMAL,
      stretch: Stretch::Normal,
      style: Style::Normal,
    })
    .is_some()
  };
  PREFERRED
    .iter()
    .find(|name| installed(name))
    .map(|name| name.to_string())
    .or_else(|| {
      db.faces()
        .find_map(|face| face.families.first().map(|(name, _)| name.clone()))
    })
}

/// The first letter (or digit) of a profile name, upper-cased, for its badge.
pub fn badge_initial(name: &str) -> String {
  name
    .chars()
    .find(|c| c.is_alphanumeric())
    .map(|c| c.to_uppercase().collect())
    .unwrap_or_default()
}

/// Whether text on `color` (bare or `#`-prefixed RRGGBB) reads better dark.
fn badge_wants_dark_ink(color: &str) -> bool {
  let hex = color.trim().trim_start_matches('#');
  if hex.len() != 6 {
    return false;
  }
  let channel = |i: usize| {
    u8::from_str_radix(&hex[i..i + 2], 16)
      .map(|v| v as f64 / 255.0)
      .unwrap_or(0.0)
  };
  // Relative luminance, sRGB weights; 0.6 keeps white ink on every mid tone.
  0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4) > 0.6
}

/// Render the PNG a 152 browser shows as this profile's window, taskbar and
/// Dock icon: the profile's frame colour with its initial. `None` when the
/// badge cannot be rendered, in which case the browser keeps its stock icon.
pub fn render_profile_icon(name: &str, color: &str) -> Option<Vec<u8>> {
  use resvg::tiny_skia;
  let hex = color.trim().trim_start_matches('#');
  if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
    return None;
  }
  let initial = badge_initial(name);
  let ink = if badge_wants_dark_ink(hex) {
    "#1b1b1b"
  } else {
    "#ffffff"
  };
  let escaped = initial
    .replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;");
  let svg = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">
<rect x="16" y="16" width="224" height="224" rx="56" fill="#{hex}"/>
<text x="128" y="128" text-anchor="middle" dominant-baseline="central" font-family="sans-serif" font-weight="700" font-size="140" fill="{ink}">{escaped}</text>
</svg>"##
  );
  let options = resvg::usvg::Options {
    fontdb: badge_fonts(),
    ..Default::default()
  };
  let tree = resvg::usvg::Tree::from_str(&svg, &options).ok()?;
  let mut pixmap = tiny_skia::Pixmap::new(256, 256)?;
  resvg::render(
    &tree,
    tiny_skia::Transform::identity(),
    &mut pixmap.as_mut(),
  );
  pixmap.encode_png().ok()
}

/// Write the profile's window badge beside its data directory and return the
/// switch that hands it to a 152 browser. Older browsers get nothing.
pub fn profile_icon_switch(
  version: &str,
  profile_path: &str,
  name: &str,
  color: &str,
) -> Option<String> {
  if !supports_wayfern_152(version) {
    return None;
  }
  let png = render_profile_icon(name, color)?;
  let dir = Path::new(profile_path)
    .parent()
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from(profile_path));
  let path = dir.join("window-icon.png");
  if let Err(e) = std::fs::write(&path, png) {
    log::warn!(
      "Could not write the window badge for profile {name} at {}: {e}; the browser keeps its stock icon",
      path.display()
    );
    return None;
  }
  let path = path.canonicalize().unwrap_or(path);
  Some(format!("--wayfern-profile-icon={}", path.display()))
}

/// Write the profile's persona beside its data directory and return the switch
/// that hands it to a 152 browser. The document is DERIVED from `seed` with
/// the user's edits applied, so it is stable per profile and unique to it.
pub fn persona_switch(
  version: &str,
  profile_path: &str,
  seed: &str,
  edits: Option<&str>,
) -> Option<String> {
  if !supports_wayfern_152(version) {
    return None;
  }
  let edits: Vec<crate::wayfern_persona::PersonaField> = edits
    .map(str::trim)
    .filter(|edits| !edits.is_empty())
    .and_then(|edits| serde_json::from_str(edits).ok())
    .unwrap_or_default();
  let fields = crate::wayfern_persona::with_edits(seed, &edits);
  if fields.is_empty() {
    return None;
  }
  let path = Path::new(profile_path).join("wayfern-persona.json");
  let body = serde_json::to_vec(&crate::wayfern_persona::document(&fields)).ok()?;
  if let Err(e) = std::fs::write(&path, body) {
    log::warn!(
      "Could not write the persona at {}: {e}; the browser offers no fill entries",
      path.display()
    );
    return None;
  }
  crate::app_dirs::restrict_to_owner(&path);
  let path = path.canonicalize().unwrap_or(path);
  Some(format!("--wayfern-profile-persona={}", path.display()))
}

/// The camera switches for a launch. A file that is not there is not passed:
/// the browser would report it and serve dark frames anyway, and the log line
/// here names the profile, which its own does not.
pub fn camera_switches(version: &str, config: &WayfernConfig) -> Vec<String> {
  if !supports_wayfern_152(version) {
    return Vec::new();
  }
  let Some(file) = config
    .camera_file
    .as_deref()
    .map(str::trim)
    .filter(|file| !file.is_empty())
  else {
    return Vec::new();
  };
  if !Path::new(file).is_file() {
    log::warn!("Camera source {file} is missing; the claimed camera serves dark frames");
    return Vec::new();
  }
  let mut switches = vec![format!("--wayfern-camera-file={file}")];
  if let Some(crop) = config
    .camera_crop
    .as_deref()
    .map(str::trim)
    .filter(|crop| !crop.is_empty())
  {
    if valid_camera_crop(crop) {
      switches.push(format!("--wayfern-camera-crop={crop}"));
    } else {
      log::warn!(
        "Camera crop {crop:?} is not x,y,width,height in source pixels; using the whole frame"
      );
    }
  }
  switches
}

/// `x,y,width,height`, all non-negative integers, width and height non-zero.
fn valid_camera_crop(crop: &str) -> bool {
  let parts: Vec<&str> = crop.split(',').map(str::trim).collect();
  if parts.len() != 4 {
    return false;
  }
  let Ok(values) = parts
    .iter()
    .map(|part| part.parse::<u32>())
    .collect::<Result<Vec<_>, _>>()
  else {
    return false;
  };
  values[2] > 0 && values[3] > 0
}

/// The platform directory a component-updater CDM install uses, and the
/// library name inside it. `None` on a platform Widevine does not ship for.
fn widevine_platform() -> Option<(&'static str, &'static str)> {
  let arch = match std::env::consts::ARCH {
    "x86_64" => "x64",
    "aarch64" => "arm64",
    _ => return None,
  };
  let (os, library) = match std::env::consts::OS {
    "macos" => ("mac", "libwidevinecdm.dylib"),
    "windows" => ("win", "widevinecdm.dll"),
    "linux" => ("linux", "libwidevinecdm.so"),
    _ => return None,
  };
  Some((Box::leak(format!("{os}_{arch}").into_boxed_str()), library))
}

/// The Widevine switch for a launch, when a CDM has been provisioned into
/// `<data>/WidevineCdm` in the component-updater layout.
///
/// Donut does not download the CDM: its distribution is a licensing decision.
/// What this does is use one that is present, and say so when one is present
/// but unusable — the browser registers nothing from a broken directory, and a
/// silent fallback to the bundled path would hide that.
pub fn widevine_switch(version: &str, data_root: &Path) -> Option<String> {
  if !supports_wayfern_152(version) {
    return None;
  }
  let dir = data_root.join("WidevineCdm");
  if !dir.join("manifest.json").is_file() {
    return None;
  }
  match widevine_platform() {
    Some((platform, library)) => {
      let payload = dir.join("_platform_specific").join(platform).join(library);
      if !payload.is_file() {
        log::warn!(
          "Widevine is provisioned at {} but {} is missing; the browser will register no CDM",
          dir.display(),
          payload.display()
        );
      }
    }
    None => log::warn!(
      "Widevine does not ship for this platform; the CDM directory is passed as provisioned"
    ),
  }
  Some(format!("--wayfern-widevine-cdm-dir={}", dir.display()))
}

/// The last lines the browser wrote about Wayfern itself.
///
/// A refusal of the launch identity is logged by the browser and never fatal
/// to it (it keeps the device it would have used anyway), so the launcher has
/// to read the verdict off stderr to turn it into an error the user sees.
#[derive(Clone, Default)]
pub struct BrowserLogTap(Arc<std::sync::Mutex<VecDeque<String>>>);

impl BrowserLogTap {
  const CAPACITY: usize = 32;

  pub fn push(&self, line: String) {
    let mut lines = self.0.lock().unwrap_or_else(|e| e.into_inner());
    if lines.len() >= Self::CAPACITY {
      lines.pop_front();
    }
    lines.push_back(line);
  }

  pub fn lines(&self) -> Vec<String> {
    self
      .0
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .iter()
      .cloned()
      .collect()
  }

  /// The reason of the most recent launch-identity refusal, if any.
  pub fn identity_refusal(&self) -> Option<String> {
    self.lines().iter().rev().find_map(|line| {
      line
        .split_once("Wayfern launch identity refused: ")
        .map(|(_, reason)| reason.trim().to_string())
    })
  }

  /// Whether the browser reported the launch identity as applied.
  pub fn identity_applied(&self) -> bool {
    self
      .lines()
      .iter()
      .any(|line| line.contains("Wayfern launch identity applied"))
  }

  pub fn has_identity_verdict(&self) -> bool {
    self.identity_applied() || self.identity_refusal().is_some()
  }
}

/// Keep reading the browser's stderr for its lifetime, keeping only what it
/// says about Wayfern. Reading it all is what keeps the pipe from filling.
fn tap_browser_stderr(
  stderr: tokio::process::ChildStderr,
  tap: BrowserLogTap,
  profile_name: String,
) {
  tauri::async_runtime::spawn(async move {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
      if line.contains("Wayfern") || line.contains("wayfern_") {
        log::info!(
          "[browser {profile_name}] {}",
          crate::log_redaction::text(&line)
        );
        tap.push(line);
      }
    }
  });
}

/// How a browser process ended up stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
  /// It shut itself down after `Browser.close`, so its session files are
  /// complete and the next launch can restore them.
  Closed,
  /// It exited on a termination request.
  Terminated,
  /// It had to be killed outright.
  Killed,
  /// Nothing this function did stopped it.
  StillRunning,
}

impl std::fmt::Display for StopOutcome {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(match self {
      Self::Closed => "closed cleanly",
      Self::Terminated => "terminated",
      Self::Killed => "killed",
      Self::StillRunning => "still running",
    })
  }
}

/// Ask a process to exit: SIGTERM, which Chromium handles as a normal
/// shutdown, or `taskkill` without `/F`, which only reaches a process with a
/// window. The caller escalates when this is not enough.
fn terminate_process(pid: u32) {
  #[cfg(unix)]
  {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
  }
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let _ = std::process::Command::new("taskkill")
      .args(["/PID", &pid.to_string()])
      .creation_flags(CREATE_NO_WINDOW)
      .output();
  }
}

/// End a process without asking.
fn force_kill_process(pid: u32) {
  #[cfg(unix)]
  {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
  }
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let _ = std::process::Command::new("taskkill")
      .args(["/PID", &pid.to_string(), "/F"])
      .creation_flags(CREATE_NO_WINDOW)
      .output();
  }
}

/// Wait up to `limit` for `pid` to leave the process table.
async fn wait_for_exit(pid: u32, limit: Duration) -> bool {
  let started = std::time::Instant::now();
  loop {
    if !crate::proxy_storage::is_process_running(pid) {
      return true;
    }
    if started.elapsed() >= limit {
      return false;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
}

/// Fingerprint fields the browser takes as dedicated parameters rather than as
/// overrides. They describe the exit IP, so they travel through their own
/// channel instead of being duplicated into `overrides`.
const GEO_PARAM_KEYS: [&str; 4] = ["timezone", "language", "latitude", "longitude"];

/// Location fields `apply_geolocation` also writes locally, so the stored
/// fingerprint is complete before any browser runs. A value donut synthesised
/// itself is filtered out of the override diff; only a value the user actually
/// edited travels.
const GEO_DERIVED_KEYS: [&str; 2] = ["timezoneOffset", "languages"];

/// Fields the browser refuses in `overrides`. It rejects the entire call when
/// one appears, so they must never diff into the override set — that would be a
/// permanent, every-launch failure rather than a one-off.
const DERIVED_PROVENANCE_KEYS: [&str; 3] =
  ["webglProfileId", "mediaProfile", "deviceProfileApplied"];

/// Location fields donutbrowser owns end to end (`apply_geolocation` writes all
/// of them). Any of these the browser does not echo back after an identity is
/// applied is refilled from the stored fingerprint, so what donut persists
/// always carries the location the launch gate reads.
const LOCALE_CARRY_OVER_KEYS: [&str; 7] = [
  "timezone",
  "timezoneOffset",
  "language",
  "languages",
  "latitude",
  "longitude",
  "accuracy",
];

/// How the geolocation probe reaches this profile's exit.
///
/// Every variant either genuinely carries the traffic or refuses. There is no
/// "try it and see" arm on purpose: a probe that does not cross the profile's
/// upstream resolves THIS MACHINE'S address, and its location was then written
/// into the fingerprint as the exit's.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProbeRoute {
  /// `reqwest` proxies this scheme itself. Carries the rewritten URL, so
  /// `httpstls` is already the `https` reqwest understands.
  Reqwest(String),
  /// A temporary local `donut-proxy` worker dials this upstream. Carries the
  /// URL the WORKER should dial, which is not always the stored one.
  Worker(String),
  /// The upstream is VLESS, which no `donut-proxy` worker can speak. An
  /// Xray-core sidecar carries the VLESS hop and a `donut-proxy` worker fronts
  /// its loopback SOCKS5 endpoint, the same two-stage path `browser_runner`
  /// builds for a VLESS launch. Carries the VLESS URI.
  Xray(String),
  /// Nothing available here carries this upstream. The probe is skipped and
  /// the fingerprint keeps no location at all, which is the only honest
  /// outcome: a location that is neither the user's nor the exit's is worse
  /// than none.
  Unroutable,
}

/// A probe transport built for one fingerprint generation, plus the temporary
/// workers that have to be stopped once the probe is done.
#[derive(Default)]
struct ProbeTransport {
  /// The proxy URL to hand `reqwest`, or `None` when nothing could carry the
  /// probe and it must be skipped rather than sent unproxied.
  proxy: Option<String>,
  donut_worker_id: Option<String>,
  xray_worker_id: Option<String>,
}

impl ProbeTransport {
  /// Stop everything this transport started. The `donut-proxy` worker goes
  /// first because it is the one holding connections open through the Xray
  /// sidecar behind it.
  async fn shutdown(self) {
    if let Some(id) = self.donut_worker_id {
      let _ = crate::proxy_runner::stop_proxy_process(&id).await;
    }
    if let Some(id) = self.xray_worker_id {
      let _ = crate::xray_worker_runner::stop_xray_worker(&id).await;
    }
  }
}

/// A freshly generated device, plus its identity handle when the browser
/// supports identities.
pub struct GeneratedFingerprint {
  /// The device the browser produced, as a flat camelCase JSON object. For a
  /// LEGACY browser this is what `WayfernConfig::fingerprint` stores. For an
  /// identity-backed profile it is a VIEW for the caller to show once and
  /// discard: only `identity_id` and `location` are persisted.
  pub fingerprint: String,
  pub identity_id: Option<String>,
  /// `WayfernConfig::location` for the exit this device was generated
  /// against, or `None` when no location field was resolved.
  pub location: Option<String>,
  /// Whether fresh geolocation was resolved and applied. Callers must only
  /// stamp `geo_proxy_signature` when this is true.
  pub geolocation_applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct WayfernLaunchResult {
  pub id: String,
  #[serde(alias = "process_id")]
  pub processId: Option<u32>,
  #[serde(alias = "profile_path")]
  pub profilePath: Option<String>,
  pub url: Option<String>,
  pub cdp_port: Option<u16>,
}

struct WayfernInstance {
  id: String,
  process_id: Option<u32>,
  profile_path: Option<String>,
  url: Option<String>,
  cdp_port: Option<u16>,
  /// What the browser said about Wayfern on stderr, for diagnostics. Read
  /// through `browser_log_lines`, which the WebRTC and persona launch checks
  /// consult; nothing else needs it yet.
  #[allow(dead_code)]
  log_tap: BrowserLogTap,
}

struct WayfernManagerInner {
  instances: HashMap<String, WayfernInstance>,
}

pub struct WayfernManager {
  inner: Arc<AsyncMutex<WayfernManagerInner>>,
  http_client: Client,
}

#[derive(Debug, Deserialize)]
struct CdpTarget {
  #[serde(rename = "type")]
  target_type: String,
  #[serde(rename = "webSocketDebuggerUrl")]
  websocket_debugger_url: Option<String>,
}

impl WayfernManager {
  fn new() -> Self {
    Self {
      inner: Arc::new(AsyncMutex::new(WayfernManagerInner {
        instances: HashMap::new(),
      })),
      // CDP is always on loopback. Disable env/system proxies so a Windows
      // WinHTTP/IE proxy (or HTTP_PROXY) cannot intercept /json/version and
      // return 502 Bad Gateway while the browser is actually listening.
      http_client: Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .expect("Failed to build reqwest client for wayfern_manager"),
    }
  }

  pub fn instance() -> &'static WayfernManager {
    &WAYFERN_MANAGER
  }

  #[allow(dead_code)]
  pub fn get_profiles_dir(&self) -> PathBuf {
    crate::app_dirs::profiles_dir()
  }

  #[allow(dead_code)]
  fn get_binaries_dir(&self) -> PathBuf {
    crate::app_dirs::binaries_dir()
  }

  async fn find_free_port() -> Result<u16, Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
  }

  /// Normalize fingerprint data from Wayfern CDP format to our storage format.
  /// Wayfern returns fields like fonts, webglParameters as JSON strings which we keep as-is.
  fn normalize_fingerprint(fingerprint: serde_json::Value) -> serde_json::Value {
    // Our storage format matches what Wayfern returns:
    // - fonts, plugins, mimeTypes, voices are JSON strings
    // - webglParameters, webgl2Parameters, etc. are JSON strings
    // The form displays them as JSON text areas, so no conversion needed.
    fingerprint
  }

  /// Denormalize fingerprint data from our storage format to Wayfern CDP format.
  /// Wayfern expects certain fields as JSON strings.
  fn denormalize_fingerprint(fingerprint: serde_json::Value) -> serde_json::Value {
    // Our storage format matches what Wayfern expects:
    // - fonts, plugins, mimeTypes, voices are JSON strings
    // - webglParameters, webgl2Parameters, etc. are JSON strings
    // So no conversion is needed
    fingerprint
  }

  /// Derive the on-screen window size Chromium should open at, from the stored
  /// fingerprint. Applying a device over CDP only spoofs what the page
  /// *reports* for `windowOuterWidth`/`screenWidth`/etc.; it does not move or
  /// resize the real top-level window. Without `--window-size` the OS window keeps
  /// Chromium's default, so the visible window contradicts the reported
  /// dimensions — a detectable mismatch. We pass `--window-size` so the actual
  /// window matches the fingerprint.
  ///
  /// Keys are the camelCase fields Wayfern uses in its fingerprint
  /// (`windowOuterWidth`, `screenAvailWidth`, …) — NOT the dotted
  /// Preference order, matching how the fingerprint
  /// describes the window:
  /// 1. `windowOuterWidth` / `windowOuterHeight` — the real window size.
  /// 2. `screenAvailWidth` / `screenAvailHeight` — usable screen area.
  /// 3. `screenWidth` / `screenHeight` — full screen.
  ///
  /// Returns `None` when the fingerprint carries no usable dimensions, leaving
  /// Chromium's default untouched. The fingerprint JSON may be the bare object
  /// or the legacy `{ "fingerprint": {...} }` wrapper.
  fn window_size_from_fingerprint(fingerprint_json: &str) -> Option<(u32, u32)> {
    let obj = Self::fingerprint_object(fingerprint_json)?;

    // Accept both numeric and stringified numbers (Wayfern emits numbers, but a
    // CDP echo or older saved fingerprint may stringify them).
    let read = |key: &str| -> Option<u32> {
      let v = obj.get(key)?;
      v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        .filter(|n| *n > 0)
        .map(|n| n as u32)
    };
    let pair = |w: &str, h: &str| -> Option<(u32, u32)> { Some((read(w)?, read(h)?)) };

    pair("windowOuterWidth", "windowOuterHeight")
      .or_else(|| pair("screenAvailWidth", "screenAvailHeight"))
      .or_else(|| pair("screenWidth", "screenHeight"))
  }

  /// The fingerprint value a stored `WayfernConfig::fingerprint` string holds:
  /// the object itself, or the one nested in the legacy
  /// `{ "fingerprint": {...} }` wrapper some old profiles carry.
  ///
  /// The single place that shape is resolved. Everything that reads a stored
  /// fingerprint goes through this or through [`Self::fingerprint_object`], so
  /// no two readers can end up disagreeing about which shapes count.
  fn unwrap_stored_fingerprint(stored: &serde_json::Value) -> &serde_json::Value {
    stored.get("fingerprint").unwrap_or(stored)
  }

  /// Parse a stored fingerprint JSON into its object, tolerating the legacy
  /// `{ "fingerprint": {...} }` wrapper some old profiles carry.
  ///
  /// Shared with `fingerprint_consistency`, which reads the timezone and
  /// language it compares against the measured exit through this exact
  /// accessor. Two readers with their own idea of the stored shape is how the
  /// gate came to report "this profile declares no timezone" for a wrapped
  /// fingerprint whose launch presented the timezone nested one level down.
  pub fn fingerprint_object(
    fingerprint_json: &str,
  ) -> Option<serde_json::Map<String, serde_json::Value>> {
    let parsed: serde_json::Value = serde_json::from_str(fingerprint_json).ok()?;
    Self::unwrap_stored_fingerprint(&parsed)
      .as_object()
      .cloned()
  }

  /// The device this launch hands the browser, derived from what the profile
  /// stores. Pure, and the only place that payload is built, so what the
  /// browser is actually given can be asserted without one running.
  ///
  /// It never invents a field. A stored fingerprint that declares no timezone
  /// produces a payload with no timezone, and the engine keeps whatever it
  /// reports natively. The launcher used to insert `America/New_York` and
  /// offset 300 here, which put a US clock behind whatever exit the profile
  /// routed through, while `fingerprint_consistency`, reading the same stored
  /// fingerprint, told the user its timezone had never been compared. That is
  /// the one combination that must never happen: the app cannot claim it
  /// compared nothing while shipping a location it made up.
  fn launch_fingerprint_payload(fingerprint_json: &str) -> Result<serde_json::Value, String> {
    let stored: serde_json::Value = serde_json::from_str(fingerprint_json)
      .map_err(|e| format!("Failed to parse stored fingerprint JSON: {e}"))?;

    // Denormalize for Wayfern CDP (arrays/objects travel as JSON strings).
    let mut payload =
      Self::denormalize_fingerprint(Self::unwrap_stored_fingerprint(&stored).clone());

    // Normalize languages: a comma-separated string becomes the array the
    // browser expects.
    if let Some(obj) = payload.as_object_mut() {
      if let Some(serde_json::Value::String(s)) = obj.get("languages").cloned() {
        let arr: Vec<&str> = s.split(',').map(|l| l.trim()).collect();
        obj.insert("languages".to_string(), json!(arr));
      }
    }

    Ok(payload)
  }

  /// A stored JSON object field (`identity_overrides`, `location`), or an
  /// empty map when absent or unparsable.
  pub fn stored_object(json: Option<&str>) -> serde_json::Map<String, serde_json::Value> {
    json.and_then(Self::fingerprint_object).unwrap_or_default()
  }

  /// The exit-derived location fields a device object carries, in the shape
  /// `WayfernConfig::location` stores; `None` when it carries none.
  pub fn location_of(device: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let mut location = serde_json::Map::new();
    for key in LOCALE_CARRY_OVER_KEYS {
      if let Some(value) = device.get(key) {
        if !value.is_null() {
          location.insert(key.to_string(), value.clone());
        }
      }
    }
    if location.is_empty() {
      None
    } else {
      serde_json::to_string(&location).ok()
    }
  }

  /// Overrides from a WHOLE fingerprint an API or MCP caller supplied for an
  /// identity-backed profile: every field it names is taken as an explicit
  /// edit, except the provenance keys the browser refuses and the location
  /// keys, which travel through `location`.
  pub fn overrides_from_explicit_fingerprint(
    fingerprint: &serde_json::Map<String, serde_json::Value>,
  ) -> serde_json::Map<String, serde_json::Value> {
    let mut overrides = serde_json::Map::new();
    for (key, value) in fingerprint {
      if DERIVED_PROVENANCE_KEYS.contains(&key.as_str())
        || GEO_PARAM_KEYS.contains(&key.as_str())
        || LOCALE_CARRY_OVER_KEYS.contains(&key.as_str())
        || value.is_null()
      {
        continue;
      }
      overrides.insert(key.clone(), value.clone());
    }
    overrides
  }

  /// ONE-TIME MIGRATION to identity-only storage. A profile created by an
  /// earlier build stored the whole device in `fingerprint` beside its
  /// `identity_id`, with `identity_baseline` recording the derived view so the
  /// user's edits could be diffed out. This moves those edits into
  /// `identity_overrides`, the exit-derived fields into `location`, and drops
  /// the payload and the baseline. Returns whether anything changed.
  ///
  /// Without a baseline nothing can separate an edit from a derived value, so
  /// no override is recovered: pinning the whole device would defeat the
  /// identity, and the browser rebuilds every field from the id anyway.
  pub fn migrate_identity_config(config: &mut WayfernConfig) -> bool {
    if config.identity_id.is_none() {
      return false;
    }
    let Some(stored_json) = config.fingerprint.clone() else {
      if config.identity_baseline.is_some() {
        config.identity_baseline = None;
        return true;
      }
      return false;
    };
    let stored = Self::fingerprint_object(&stored_json).unwrap_or_default();
    let overrides = match config
      .identity_baseline
      .as_deref()
      .and_then(Self::fingerprint_object)
    {
      Some(baseline) => Self::identity_overrides(&stored, &baseline),
      None => serde_json::Map::new(),
    };
    if config.identity_overrides.is_none() && !overrides.is_empty() {
      config.identity_overrides = serde_json::to_string(&overrides).ok();
    }
    if config.location.is_none() {
      config.location = Self::location_of(&stored);
    }
    config.fingerprint = None;
    config.identity_baseline = None;
    true
  }

  /// The user's edits, recovered as the difference between the fingerprint the
  /// profile stores and the view the browser derived from the identity.
  ///
  /// The fingerprint form writes the whole edited object back over
  /// `WayfernConfig::fingerprint`, so the edits are not recorded anywhere on
  /// their own; the baseline is what makes them recoverable. Everything absent
  /// from the diff is supplied by the identity.
  ///
  /// Three exclusions, each for its own reason:
  ///
  /// - `GEO_PARAM_KEYS`, because `setIdentity` takes them as dedicated
  ///   parameters; sending them twice invites the two copies to disagree.
  /// - `DERIVED_PROVENANCE_KEYS`, because the browser rejects the whole call
  ///   when one appears. A stored payload carrying them would otherwise diff
  ///   every one into the override set and fail on every launch.
  /// - a `GEO_DERIVED_KEYS` value donut synthesised itself, because the
  ///   baseline is snapshotted BEFORE geolocation runs, so those two keys
  ///   always differ and would otherwise be pinned as user overrides for the
  ///   life of the profile. A value that is NOT what donut would have written
  ///   is a genuine edit and still travels.
  fn identity_overrides(
    current: &serde_json::Map<String, serde_json::Value>,
    baseline: &serde_json::Map<String, serde_json::Value>,
  ) -> serde_json::Map<String, serde_json::Value> {
    let synthesised = Self::donut_synthesised_geo_fields(current);
    let mut overrides = serde_json::Map::new();
    for (key, value) in current {
      if GEO_PARAM_KEYS.contains(&key.as_str()) || DERIVED_PROVENANCE_KEYS.contains(&key.as_str()) {
        continue;
      }
      if GEO_DERIVED_KEYS.contains(&key.as_str()) && synthesised.get(key) == Some(value) {
        continue;
      }
      if baseline.get(key) != Some(value) {
        overrides.insert(key.clone(), value.clone());
      }
    }
    overrides
  }

  /// What `apply_geolocation` would write into `GEO_DERIVED_KEYS` for the
  /// `timezone`/`language` this fingerprint already carries.
  ///
  /// Built by calling the same helpers the writer calls, so the two cannot
  /// compute a different answer for the same input. A key is absent when what
  /// it is derived from is missing or unparsable, which leaves the value
  /// looking like an edit — the conservative direction, since it only means an
  /// override travels that did not have to.
  fn donut_synthesised_geo_fields(
    fingerprint: &serde_json::Map<String, serde_json::Value>,
  ) -> serde_json::Map<String, serde_json::Value> {
    let mut derived = serde_json::Map::new();
    if let Some(minutes) = fingerprint
      .get("timezone")
      .and_then(|v| v.as_str())
      .and_then(Self::timezone_offset_minutes)
    {
      derived.insert("timezoneOffset".to_string(), json!(minutes));
    }
    if let Some(locale) = fingerprint.get("language").and_then(|v| v.as_str()) {
      derived.insert(
        "languages".to_string(),
        json!([locale, Self::base_language(locale)]),
      );
    }
    derived
  }

  /// The offset of `timezone` from UTC in minutes, in the sign convention
  /// `Date.prototype.getTimezoneOffset` uses (positive west of UTC), or `None`
  /// when the IANA name does not parse.
  ///
  /// It reads the offset AT THE CURRENT INSTANT, so a zone that observes DST
  /// answers differently either side of a transition. That is what a browser
  /// reports too, and it is why `apply_geolocation` and `identity_overrides`
  /// share this one implementation instead of each computing their own.
  fn timezone_offset_minutes(timezone: &str) -> Option<i32> {
    use chrono::Offset;
    let tz = timezone.parse::<chrono_tz::Tz>().ok()?;
    let offset_seconds = chrono::Utc::now()
      .with_timezone(&tz)
      .offset()
      .fix()
      .local_minus_utc();
    Some(-(offset_seconds / 60))
  }

  /// The bare language subtag of a BCP 47 tag (`de-DE` -> `de`), which is the
  /// second entry of the `languages` ladder donut builds. `Locale::language` is
  /// itself the first `-`-separated part of the tag, so this reproduces it.
  fn base_language(locale: &str) -> &str {
    locale.split('-').next().unwrap_or(locale)
  }

  /// The `setIdentity` geolocation parameters carried by a stored fingerprint.
  fn geo_params(
    fingerprint: &serde_json::Map<String, serde_json::Value>,
  ) -> serde_json::Map<String, serde_json::Value> {
    let mut params = serde_json::Map::new();
    for key in GEO_PARAM_KEYS {
      if let Some(value) = fingerprint.get(key) {
        if !value.is_null() {
          params.insert(key.to_string(), value.clone());
        }
      }
    }
    params
  }

  /// One of Wayfern's five `operatingSystem` names, or `None` for anything
  /// else. Unknown names are not guessed at: the caller treats `None` as "donut
  /// does not know what this profile claims" and lets the browser decide.
  fn normalize_os_name(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
      "windows" => Some("windows"),
      "macos" => Some("macos"),
      "linux" => Some("linux"),
      "android" => Some("android"),
      "ios" => Some("ios"),
      _ => None,
    }
  }

  /// The OS a `navigator.platform` value describes.
  ///
  /// Matches the browser's own platform-to-OS mapping, including the order of
  /// the tests: `Linux armv8l` and `aarch64` must read as android before the
  /// plain `Linux` test can claim them.
  fn os_from_platform(platform: &str) -> Option<&'static str> {
    if platform.contains("Win") {
      Some("windows")
    } else if platform.contains("Mac") {
      Some("macos")
    } else if platform.contains("iPhone") || platform.contains("iPad") {
      Some("ios")
    } else if platform.contains("Android")
      || platform.contains("Linux armv")
      || platform.contains("aarch64")
    {
      Some("android")
    } else if platform.contains("Linux") {
      Some("linux")
    } else {
      None
    }
  }

  /// The OS this profile claims, or `None` when nothing on it says so.
  ///
  /// `WayfernConfig::os` is authoritative because it is what generation was
  /// asked for; the stored fingerprint's `platform` is the fallback for
  /// profiles minted before that field existed. Deliberately conservative — an
  /// unrecognised value on either yields `None` rather than a guess, because
  /// the only caller uses this to REFUSE a launch.
  fn claimed_operating_system(
    config: &WayfernConfig,
    stored: Option<&serde_json::Map<String, serde_json::Value>>,
  ) -> Option<&'static str> {
    if let Some(os) = config.os.as_deref().and_then(Self::normalize_os_name) {
      return Some(os);
    }
    stored
      .and_then(|fp| fp.get("platform"))
      .and_then(|v| v.as_str())
      .and_then(Self::os_from_platform)
  }

  /// Translate a refused apply into a code the frontend can explain.
  ///
  /// CDP carries a message, not a machine-readable code, so matching the text
  /// is the only channel the browser has. The literals are the ones the browser
  /// emits when it refuses a cross-OS claim or a generation; if one is ever
  /// reworded this degrades to the generic code, which still carries the raw
  /// text for support, rather than breaking.
  fn apply_failure_error(detail: &str, claimed_os: Option<&str>) -> String {
    if detail.contains("Cross-OS fingerprinting requires") {
      return crate::backend_error_with_detail(
        "WAYFERN_CROSS_OS_REQUIRES_PLAN",
        claimed_os.unwrap_or("another operating system"),
      );
    }
    // BOTH refusal texts, because a profile may be on either browser version.
    // Older builds word the generation-limit refusal differently, and matching
    // only one wording leaves those users falling through to the generic
    // apply-failed message, losing the one piece of information that makes the
    // failure actionable.
    if detail.contains("generation limit reached") || detail.contains("Too many profiles") {
      return crate::backend_error("WAYFERN_GENERATION_LIMIT_REACHED");
    }
    crate::backend_error_with_detail("WAYFERN_FINGERPRINT_APPLY_FAILED", detail)
  }

  /// The document a 152 browser takes through `--wayfern-identity-file`, or
  /// `None` when this profile cannot be described that way: a legacy device
  /// payload (only `setFingerprint` reproduces one), no identity, no claimed
  /// operating system, or no timezone. The browser requires the timezone
  /// because it does not resolve the exit itself; donut holds the proxy and
  /// resolved it when the location was written.
  pub fn launch_identity_document(config: &WayfernConfig) -> Option<serde_json::Value> {
    if config.fingerprint.is_some() {
      return None;
    }
    let identity_id = config
      .identity_id
      .as_deref()
      .map(str::trim)
      .filter(|id| !id.is_empty())?;
    // The browser requires the operating system in the document. Over CDP an
    // omitted `operatingSystem` means the host, so the document says so
    // explicitly, and the profile's own claim wins when it has one.
    let host_os = crate::profile::types::get_host_os();
    let os = Self::claimed_operating_system(config, None).unwrap_or(host_os.as_str());
    let location = Self::stored_object(config.location.as_deref());
    let geo = Self::geo_params(&location);
    let timezone = geo
      .get("timezone")
      .and_then(|v| v.as_str())
      .filter(|tz| !tz.is_empty())?;

    let mut document = serde_json::Map::new();
    document.insert("identityId".to_string(), json!(identity_id));
    document.insert("operatingSystem".to_string(), json!(os));
    document.insert("timezone".to_string(), json!(timezone));
    if let Some(language) = geo
      .get("language")
      .and_then(|v| v.as_str())
      .filter(|l| !l.is_empty())
    {
      document.insert("language".to_string(), json!(language));
    }
    if let (Some(latitude), Some(longitude)) = (
      geo.get("latitude").and_then(|v| v.as_f64()),
      geo.get("longitude").and_then(|v| v.as_f64()),
    ) {
      document.insert("latitude".to_string(), json!(latitude));
      document.insert("longitude".to_string(), json!(longitude));
    }
    let overrides = Self::stored_object(config.identity_overrides.as_deref());
    if !overrides.is_empty() {
      document.insert(
        "overrides".to_string(),
        serde_json::Value::Object(overrides),
      );
    }
    Some(serde_json::Value::Object(document))
  }

  /// Write the launch identity into the profile directory and return the
  /// absolute path the browser is given. Private to the user on Unix: the
  /// document names the identity and the user's overrides.
  fn write_launch_identity(
    profile_path: &str,
    document: &serde_json::Value,
  ) -> Result<PathBuf, String> {
    let dir = PathBuf::from(profile_path);
    std::fs::create_dir_all(&dir)
      .map_err(|e| format!("could not create the profile directory for the identity file: {e}"))?;
    let path = dir.join(LAUNCH_IDENTITY_FILE);
    let body = serde_json::to_vec(document)
      .map_err(|e| format!("could not encode the identity document: {e}"))?;
    #[cfg(unix)]
    {
      use std::io::Write;
      use std::os::unix::fs::OpenOptionsExt;
      let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("could not write the identity file: {e}"))?;
      file
        .write_all(&body)
        .map_err(|e| format!("could not write the identity file: {e}"))?;
    }
    #[cfg(not(unix))]
    {
      std::fs::write(&path, &body)
        .map_err(|e| format!("could not write the identity file: {e}"))?;
    }
    let path = path.canonicalize().unwrap_or(path);
    Ok(path)
  }

  /// Whether the browser started on the identity the launcher handed it.
  ///
  /// The browser's own stderr verdict is authoritative when present: a
  /// refusal names its reason, an "applied" line settles it. Without one, the
  /// identity id the browser reports decides, and as a last resort the
  /// timezone, language and platform of the running device are compared with
  /// the document.
  fn launch_identity_verdict(
    document: &serde_json::Value,
    observed: Option<&serde_json::Value>,
    cdp_error: Option<&str>,
    tap: &BrowserLogTap,
  ) -> Result<&'static str, String> {
    if let Some(reason) = tap.identity_refusal() {
      return Err(reason);
    }
    let expected_id = document["identityId"].as_str().unwrap_or_default();
    let observed_id = observed.and_then(|o| o["identityId"].as_str());
    if !expected_id.is_empty() && observed_id == Some(expected_id) {
      return Ok("the browser reports the identity");
    }
    if tap.identity_applied() {
      return Ok("the browser logged the identity as applied");
    }
    let Some(observed) = observed else {
      return Err(match cdp_error {
        Some(error) => format!("Wayfern.getIdentity failed: {error}"),
        None => "the browser exposed no page target to verify the identity on".to_string(),
      });
    };
    let identity = &observed["identity"];
    let same = |key: &str| identity[key].as_str() == document[key].as_str();
    let platform = identity["platform"].as_str().unwrap_or_default();
    let os_matches = Self::os_from_platform(platform) == document["operatingSystem"].as_str();
    if same("timezone") && (document.get("language").is_none() || same("language")) && os_matches {
      return Ok("the running device matches the document's timezone, language and platform");
    }
    Err(format!(
      "the browser reports identity {} (timezone {}, language {}, platform {}) instead of {expected_id} ({}, {}, {})",
      observed_id.unwrap_or("none"),
      identity["timezone"].as_str().unwrap_or("unknown"),
      identity["language"].as_str().unwrap_or("unknown"),
      if platform.is_empty() { "unknown" } else { platform },
      document["timezone"].as_str().unwrap_or("unknown"),
      document["language"].as_str().unwrap_or("any"),
      document["operatingSystem"].as_str().unwrap_or("unknown"),
    ))
  }

  async fn wait_for_cdp_ready(
    &self,
    port: u16,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    // On first launch, macOS Gatekeeper verifies the binary which can take 30+ seconds.
    // Use a generous timeout (60s) to handle this.
    let max_attempts = 120;
    let delay = Duration::from_millis(500);

    let mut last_error: Option<String> = None;
    for attempt in 0..max_attempts {
      match self.http_client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
          log::info!("CDP ready on port {port} after {attempt} attempts");
          return Ok(());
        }
        Ok(resp) => {
          last_error = Some(format!("HTTP {} from {url}", resp.status()));
          tokio::time::sleep(delay).await;
        }
        Err(e) => {
          last_error = Some(format!("request failed: {e}"));
          tokio::time::sleep(delay).await;
        }
      }
    }

    let detail = last_error.unwrap_or_else(|| "no attempts completed".to_string());
    // Log at error level so we can diagnose Windows/AV/firewall-induced CDP hangs
    // in customer reports without needing them to reproduce in the moment.
    log::error!("CDP not ready after {max_attempts} attempts on port {port}: {detail}");
    Err(format!("CDP not ready after {max_attempts} attempts on port {port}: {detail}").into())
  }

  async fn get_cdp_targets(
    &self,
    port: u16,
  ) -> Result<Vec<CdpTarget>, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("http://127.0.0.1:{port}/json");
    let resp = self.http_client.get(&url).send().await?;
    let targets: Vec<CdpTarget> = resp.json().await?;
    Ok(targets)
  }

  async fn send_cdp_command(
    &self,
    ws_url: &str,
    method: &str,
    params: serde_json::Value,
  ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let (mut ws_stream, _) = connect_async(ws_url).await?;

    let command = json!({
      "id": 1,
      "method": method,
      "params": params
    });

    use futures_util::sink::SinkExt;
    use futures_util::stream::StreamExt;

    ws_stream
      .send(Message::Text(command.to_string().into()))
      .await?;

    while let Some(msg) = ws_stream.next().await {
      match msg? {
        Message::Text(text) => {
          let response: serde_json::Value = serde_json::from_str(text.as_str())?;
          if response.get("id") == Some(&json!(1)) {
            if let Some(error) = response.get("error") {
              return Err(format!("CDP error: {}", error).into());
            }
            return Ok(response.get("result").cloned().unwrap_or(json!({})));
          }
        }
        Message::Close(_) => break,
        _ => {}
      }
    }

    Err("No response received from CDP".into())
  }

  /// Stable signature describing what determines this profile's geolocation
  /// (timezone, latitude/longitude, language): the geoip mode first, then the
  /// VPN, the proxy, or a direct connection. Compared across creation and
  /// launch to detect a change. The VPN case keys off `vpn_id` rather than the
  /// per-launch local port, and the proxy case off type/host/port/username so
  /// that editing the proxy is also caught.
  pub fn geo_signature(
    proxy: Option<&crate::browser::ProxySettings>,
    vpn_id: Option<&str>,
    geoip: Option<&serde_json::Value>,
  ) -> String {
    // The "v2:" prefix invalidates every signature stamped before geolocation
    // failures stopped being stamped: those may describe fingerprints that
    // silently carry the host's location, so each pre-v2 profile gets one
    // launch-time refresh and is re-stamped in the current format.
    let base = match geoip {
      Some(serde_json::Value::Bool(false)) => "off".to_string(),
      Some(serde_json::Value::String(ip)) if !ip.is_empty() => format!("ip:{ip}"),
      _ => {
        if let Some(id) = vpn_id {
          format!("vpn:{id}")
        } else if let Some(p) = proxy {
          format!(
            "proxy:{}://{}@{}:{}",
            p.proxy_type.to_lowercase(),
            p.username.as_deref().unwrap_or(""),
            p.host,
            p.port
          )
        } else {
          "direct".to_string()
        }
      }
    };
    format!("v2:{base}")
  }

  /// Apply timezone/geolocation fields to a fingerprint object from the proxy's
  /// exit IP (or a fixed geoip IP). Mutates `fingerprint` in place. Returns true
  /// if fresh geolocation was fetched and applied, false if geolocation is
  /// disabled or could not be resolved (in which case only safe defaults are
  /// filled in). Shared by fingerprint generation and the launch-time refresh
  /// so both produce identical location data.
  async fn apply_geolocation(
    fingerprint: &mut serde_json::Value,
    proxy: Option<&str>,
    geoip: Option<&serde_json::Value>,
  ) -> bool {
    // Default to auto-detect; only an explicit `false` disables geolocation.
    let should_geolocate = !matches!(geoip, Some(serde_json::Value::Bool(false)));
    if !should_geolocate {
      return false;
    }

    let geo_result = async {
      let ip = match geoip {
        Some(serde_json::Value::String(ip_str)) => ip_str.clone(),
        _ => crate::ip_utils::fetch_public_ip(proxy)
          .await
          .map_err(|e| format!("Failed to fetch public IP: {e}"))?,
      };
      crate::geolocation::get_geolocation(&ip)
        .map_err(|e| format!("Failed to get geolocation for IP {ip}: {e}"))
    }
    .await;

    match geo_result {
      Ok(geo) => {
        if let Some(obj) = fingerprint.as_object_mut() {
          obj.insert("timezone".to_string(), json!(geo.timezone));
          // Both derived fields go through the same helpers `identity_overrides`
          // uses to recognise them, so a value written here can never look like
          // a user edit to the override diff.
          if let Some(offset_minutes) = Self::timezone_offset_minutes(&geo.timezone) {
            obj.insert("timezoneOffset".to_string(), json!(offset_minutes));
          }
          obj.insert("latitude".to_string(), json!(geo.latitude));
          obj.insert("longitude".to_string(), json!(geo.longitude));
          let locale_str = geo.locale.as_string();
          obj.insert("language".to_string(), json!(&locale_str));
          obj.insert(
            "languages".to_string(),
            json!([&locale_str, Self::base_language(&locale_str)]),
          );
        }
        log::info!(
          "Applied geolocation to Wayfern fingerprint: {} ({})",
          geo.locale.as_string(),
          geo.timezone
        );
        true
      }
      Err(e) => {
        // NOTHING is written here, deliberately. A failed probe used to fill in
        // America/New_York and offset 300, which made "we could not resolve the
        // exit" indistinguishable from "the exit is in New York": the profile
        // then presented a US location as its proxy's, in the one field the
        // consistency gate and the user both read as authoritative. A location
        // that is neither the user's nor the exit's is worse than no location,
        // so the fields are left ungenerated.
        //
        // Returning false is what makes that recoverable: the caller must not
        // stamp `geo_proxy_signature`, so the launch-time refresh sees a
        // signature mismatch and probes again through the local worker the
        // browser is about to use.
        log::warn!("Geolocation failed; leaving the fingerprint's location ungenerated: {e}");
        false
      }
    }
  }

  /// Refresh ONLY the location fields (timezone, offset, latitude/longitude,
  /// language) of an already-generated fingerprint to match the current proxy,
  /// leaving every other fingerprint field untouched. `proxy` is the local
  /// proxy URL the browser will use. Returns the updated fingerprint JSON on
  /// success, or None if geolocation is disabled or could not be resolved, in
  /// which case the caller keeps the existing fingerprint and retries on the
  /// next launch.
  pub async fn refresh_fingerprint_geolocation(
    fingerprint_json: &str,
    proxy: Option<&str>,
    geoip: Option<&serde_json::Value>,
  ) -> Option<String> {
    let mut fp: serde_json::Value = serde_json::from_str(fingerprint_json).ok()?;
    if Self::apply_geolocation(&mut fp, proxy, geoip).await {
      serde_json::to_string(&fp).ok()
    } else {
      None
    }
  }

  /// True when `url` is a socks proxy on a remote (non-loopback) host — the
  /// case where reqwest's SOCKS connector can't be trusted with the
  /// geolocation fetch. Loopback socks URLs are the app's own donut-proxy
  /// workers, whose single-segment replies don't trigger the connector bug.
  /// Upstreams the geolocation probe must reach through a local donut-proxy
  /// worker instead of handing to `reqwest`.
  ///
  /// Two groups. Remote SOCKS, which reqwest could proxy but which this code has
  /// always routed through a worker. And EVERY scheme reqwest cannot proxy,
  /// whatever its host, that group is the dangerous one: `Proxy::all` ACCEPTS
  /// such a URL, then matches nothing, so the probe went out from the user's
  /// REAL address and its geolocation was written into the profile fingerprint.
  /// Nothing failed and nothing was logged: exactly the exit-vs-fingerprint
  /// mismatch `fingerprint_consistency.rs` exists to catch, manufactured by the
  /// fingerprint generator itself. And `probe_url` skips `ss`/`vless` entirely,
  /// so the launch-time gate cannot catch it either.
  ///
  /// The loopback exemption applies ONLY to schemes reqwest can proxy. A
  /// loopback SOCKS upstream IS the local worker, so it needs no second one; a
  /// loopback `ss://127.0.0.1:8388` is still a scheme reqwest discards, and
  /// exempting it re-opened the whole leak.
  ///
  /// True here means only "reqwest must not be handed this". It does NOT mean a
  /// `donut-proxy` worker can carry it, reading it that way is what sent
  /// `vless://` to a worker that cannot speak VLESS, so the probe could never
  /// succeed. `worker_upstream_url` answers what a worker can actually dial,
  /// and `probe_route` puts the two together.
  fn needs_local_worker_for_probe(url: &str) -> bool {
    // Measured on the REWRITTEN url, because `httpstls://` becomes `https://`
    // before reqwest ever sees it and is proxyable from that point on.
    let rewritten = crate::proxy_storage::reqwest_upstream_url(url);
    let scheme = rewritten
      .split("://")
      .next()
      .unwrap_or_default()
      .to_ascii_lowercase();

    if !crate::proxy_storage::reqwest_can_proxy(&rewritten) {
      // No host exemption here: reqwest discards it wherever it points.
      return true;
    }

    scheme.starts_with("socks")
      && url::Url::parse(&rewritten)
        .ok()
        .and_then(|u| match u.host() {
          Some(url::Host::Ipv4(ip)) => Some(!ip.is_loopback()),
          Some(url::Host::Ipv6(ip)) => Some(!ip.is_loopback()),
          // socks is a non-special scheme, so the url crate keeps even
          // IP-literal hosts as Domain — parse them before comparing.
          Some(url::Host::Domain(domain)) => Some(
            domain != "localhost"
              && domain
                .parse::<std::net::IpAddr>()
                .map(|ip| !ip.is_loopback())
                .unwrap_or(true),
          ),
          None => None,
        })
        .unwrap_or(false)
  }

  /// The URL a `donut-proxy` worker can actually dial for this upstream, or
  /// `None` when no worker can carry it at all.
  ///
  /// The allow-list is the exact set `proxy_server::dial_upstream` matches on.
  /// Everything outside it reaches that function's `_` arm, so every request
  /// through the worker dies as "Unsupported upstream proxy scheme", a worker
  /// started on such a URL is not a fallback, it is a guaranteed failure. That
  /// is how `vless://` came to be routed here: `needs_local_worker_for_probe`
  /// answers "reqwest cannot proxy this", which was read as "a worker can", and
  /// the probe could then never succeed. VLESS is carried by an Xray-core
  /// sidecar instead (see `probe_route`), and anything else honestly has no
  /// transport here.
  ///
  /// `socks5h`/`socks4a` are the "resolve at the exit" spellings of `socks5`/
  /// `socks4`. The worker hands the target hostname to the SOCKS server rather
  /// than resolving it locally, which is exactly what the `h` asks for, so they
  /// are normalized to the spelling the worker matches instead of rejected.
  fn worker_upstream_url(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
      "http" | "https" | "httpstls" | "socks4" | "socks5" | "ss" | "shadowsocks" => {
        Some(format!("{scheme}://{rest}"))
      }
      "socks5h" => Some(format!("socks5://{rest}")),
      "socks4a" => Some(format!("socks4://{rest}")),
      _ => None,
    }
  }

  /// Decide how the geolocation probe for `url` reaches the exit.
  ///
  /// Preference order, and the reason for it: probe through something that
  /// genuinely carries the traffic, or do not probe at all. Every arm that
  /// cannot carry it resolves to `Unroutable`, which the caller turns into a
  /// skipped probe and an ungenerated location, never into a probe sent from
  /// this machine's own address, and never into a default location presented as
  /// the exit's.
  fn probe_route(url: &str) -> ProbeRoute {
    if !Self::needs_local_worker_for_probe(url) {
      let rewritten = crate::proxy_storage::reqwest_upstream_url(url);
      // Checked rather than assumed. `needs_local_worker_for_probe` already
      // returns true for everything reqwest cannot proxy, but handing reqwest a
      // URL it cannot match makes it send the request DIRECT with no error and
      // no log line, so the invariant is re-asserted where it matters.
      return if crate::proxy_storage::reqwest_can_proxy(&rewritten) {
        ProbeRoute::Reqwest(rewritten)
      } else {
        ProbeRoute::Unroutable
      };
    }

    // The worker gets the STORED url, never the reqwest rewrite: to
    // `donut-proxy`, `httpstls` means "TLS to the proxy, then CONNECT" while
    // `https` means a plaintext CONNECT, so handing it the reqwest spelling
    // would silently downgrade that hop.
    if let Some(upstream) = Self::worker_upstream_url(url) {
      return ProbeRoute::Worker(upstream);
    }

    if url
      .split_once("://")
      .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("vless"))
    {
      return match crate::xray::parse_vless_uri(url) {
        Ok(_) => ProbeRoute::Xray(url.to_string()),
        // A `vless://host:port` with no id, flow or security parameters is what
        // a stored VLESS proxy collapses to when it is rendered as
        // `type://host:port`. Xray cannot dial that, so there is nothing to
        // probe through and the launch-time refresh does the location instead -
        // by then the upstream is the loopback SOCKS5 endpoint of a real Xray
        // worker, which any transport here can carry.
        Err(_) => ProbeRoute::Unroutable,
      };
    }

    ProbeRoute::Unroutable
  }

  /// Whether the probe has to be skipped outright.
  ///
  /// True when the profile routes its traffic somewhere, the probe would
  /// actually leave this machine, and no transport was built to carry it.
  /// Probing anyway resolves this machine's own address and writes its location
  /// into the fingerprint as the exit's.
  ///
  /// `probe_leaves_this_machine` is false when the location comes from a pinned
  /// geoip IP or geolocation is switched off. `apply_geolocation` never touches
  /// the network through the proxy in either case, so a routed profile with a
  /// pinned IP must still get its location.
  fn must_skip_probe(
    routes_traffic: bool,
    probe_leaves_this_machine: bool,
    have_probe_proxy: bool,
  ) -> bool {
    routes_traffic && probe_leaves_this_machine && !have_probe_proxy
  }

  /// Build the transport the geolocation probe for `url` will use. Callers must
  /// `shutdown()` the result once the probe is done, on every path.
  async fn build_probe_transport(url: &str) -> ProbeTransport {
    match Self::probe_route(url) {
      ProbeRoute::Reqwest(proxy) => ProbeTransport {
        proxy: Some(proxy),
        ..Default::default()
      },
      ProbeRoute::Worker(upstream) => Self::front_with_local_worker(upstream, None).await,
      ProbeRoute::Xray(uri) => {
        // The error is flattened to a String on the spot. `start_xray_worker`
        // reports a bare `Box<dyn Error>`, which is not `Send`, and holding one
        // across the `front_with_local_worker` await below would make this
        // whole future non-`Send`, and with it every Tauri command and spawned
        // task that reaches fingerprint generation.
        let started = crate::xray_worker_runner::start_xray_worker(None, &uri)
          .await
          .map_err(|error| error.to_string());
        match started {
          Ok(worker) => {
            let upstream =
              crate::proxy_manager::ProxyManager::build_proxy_url(&worker.local_proxy_settings());
            Self::front_with_local_worker(upstream, Some(worker.id)).await
          }
          Err(e) => {
            log::warn!(
              "Could not start an Xray-core worker to carry the VLESS geolocation probe ({e}); skipping the probe rather than sending it unproxied"
            );
            ProbeTransport::default()
          }
        }
      }
      ProbeRoute::Unroutable => {
        log::warn!(
          "No transport here can carry the geolocation probe through this profile's upstream; skipping the probe rather than resolving this machine's own address"
        );
        ProbeTransport::default()
      }
    }
  }

  /// Put a temporary local `donut-proxy` worker in front of `upstream`, the
  /// same path the browser itself uses. `xray_worker_id` is threaded through so
  /// a sidecar started for a VLESS upstream is still stopped when the worker in
  /// front of it fails to start.
  async fn front_with_local_worker(
    upstream: String,
    xray_worker_id: Option<String>,
  ) -> ProbeTransport {
    match crate::proxy_runner::start_proxy_process(Some(upstream), None).await {
      Ok(worker) => ProbeTransport {
        proxy: Some(format!(
          "http://127.0.0.1:{}",
          worker.local_port.unwrap_or(0)
        )),
        donut_worker_id: Some(worker.id),
        xray_worker_id,
      },
      Err(e) => {
        // NOT the raw upstream. reqwest silently ignores a proxy URL it cannot
        // match, so handing it back here sent the probe from this machine's
        // real address. `None` means "no proxied probe available", and the
        // caller skips the probe entirely rather than making it unproxied.
        log::warn!(
          "Could not start local proxy worker for geolocation ({e}); skipping the probe rather than sending it unproxied"
        );
        ProbeTransport {
          proxy: None,
          donut_worker_id: None,
          xray_worker_id,
        }
      }
    }
  }

  /// Generate a device for `config` on a headless Wayfern.
  ///
  /// On a browser that ships the identity API this mints an identity and
  /// returns its handle alongside the fingerprint; on older browsers it returns
  /// the fingerprint alone. Which path runs is decided by `profile.version`,
  /// the same field the executable path is resolved from, so the generated
  /// device always matches the binary that ran.
  ///
  /// Callers must only stamp `geo_proxy_signature` when
  /// `geolocation_applied` is true: the device comes from a headless Wayfern
  /// launched without a proxy, so on failure it silently carries the HOST
  /// timezone/locale — stamping the signature then would tell the launch-time
  /// refresh the location is already correct for this proxy and permanently
  /// disable the one path that can repair it.
  pub async fn generate_fingerprint_config(
    &self,
    _app_handle: &AppHandle,
    profile: &BrowserProfile,
    config: &WayfernConfig,
  ) -> Result<GeneratedFingerprint, Box<dyn std::error::Error + Send + Sync>> {
    let executable_path = BrowserRunner::instance()
      .get_browser_executable_path(profile)
      .map_err(|e| format!("Failed to get Wayfern executable path: {e}"))?;

    let port = Self::find_free_port().await?;
    log::info!("Launching headless Wayfern on port {port} for fingerprint generation");

    let temp_profile_dir =
      std::env::temp_dir().join(format!("wayfern_fingerprint_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_profile_dir)?;

    let mut cmd = TokioCommand::new(&executable_path);
    cmd
      .arg("--headless=new")
      .arg(format!("--remote-debugging-port={port}"))
      .arg("--remote-debugging-address=127.0.0.1")
      .arg(format!("--user-data-dir={}", temp_profile_dir.display()))
      .arg("--no-first-run")
      .arg("--no-default-browser-check")
      .arg("--disable-background-mode")
      .arg("--use-mock-keychain")
      .arg("--password-store=basic")
      .arg("--disable-features=DialMediaRouteProvider");

    #[cfg(target_os = "linux")]
    cmd
      .arg("--no-sandbox")
      .arg("--disable-setuid-sandbox")
      .arg("--disable-dev-shm-usage");

    cmd.stdout(Stdio::null()).stderr(Stdio::piped());

    let child = cmd.spawn().map_err(|e| {
      // OS error 14001 = SxS / missing Visual C++ Redistributable
      let hint = if e.raw_os_error() == Some(14001) {
        ". This usually means the Visual C++ Redistributable is not installed. \
         Download it from https://aka.ms/vs/17/release/vc_redist.x64.exe"
      } else {
        ""
      };
      format!("Failed to spawn headless Wayfern: {e}{hint}")
    })?;
    let child_id = child.id();
    // Drain stderr for the browser's lifetime and keep what it says about
    // Wayfern: a generation browser that dies before CDP is up leaves its
    // reason there and nowhere else.
    let generation_log = BrowserLogTap::default();
    let mut child = child;
    if let Some(stderr) = child.stderr.take() {
      tap_browser_stderr(
        stderr,
        generation_log.clone(),
        format!("generation for {}", profile.name),
      );
    }

    let cleanup = || async {
      if let Some(id) = child_id {
        #[cfg(unix)]
        {
          use nix::sys::signal::{kill, Signal};
          use nix::unistd::Pid;
          let _ = kill(Pid::from_raw(id as i32), Signal::SIGTERM);
        }
        #[cfg(windows)]
        {
          use std::os::windows::process::CommandExt;
          const CREATE_NO_WINDOW: u32 = 0x08000000;
          let _ = std::process::Command::new("taskkill")
            .args(["/PID", &id.to_string(), "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        }
      }
      let _ = std::fs::remove_dir_all(&temp_profile_dir);
    };

    if let Err(e) = self.wait_for_cdp_ready(port).await {
      // Try to capture stderr from the failed process for diagnostics
      let stderr_output = if let Some(id) = child_id {
        // Check if process is still running
        let is_running = sysinfo::System::new_with_specifics(
          sysinfo::RefreshKind::nothing().with_processes(sysinfo::ProcessRefreshKind::nothing()),
        )
        .process(sysinfo::Pid::from(id as usize))
        .is_some();

        // The tap may still be a line behind the process's exit.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let said = generation_log.lines();
        let said = if said.is_empty() {
          String::from("it said nothing about Wayfern on stderr")
        } else {
          format!("its last Wayfern lines: {}", said.join(" | "))
        };
        if !is_running {
          format!("(process exited before CDP became ready; {said})")
        } else {
          format!("(process still running but not responding on CDP; {said})")
        }
      } else {
        String::new()
      };

      log::error!(
        "Fingerprint-generation Wayfern (headless, pid={child_id:?}) never became CDP-ready: {e}. {stderr_output}"
      );
      cleanup().await;
      return Err(e);
    }

    let targets = match self.get_cdp_targets(port).await {
      Ok(t) => t,
      Err(e) => {
        cleanup().await;
        return Err(e);
      }
    };

    let page_target = targets
      .iter()
      .find(|t| t.target_type == "page" && t.websocket_debugger_url.is_some());

    let ws_url = match page_target {
      Some(target) => target.websocket_debugger_url.as_ref().unwrap().clone(),
      None => {
        cleanup().await;
        return Err("No page target found for CDP".into());
      }
    };

    let host_os = crate::profile::types::get_host_os();
    let os = config.os.as_deref().unwrap_or(&host_os);

    // Include wayfern token if available (enables cross-OS fingerprinting for paid users)
    let wayfern_token = crate::cloud_auth::CLOUD_AUTH.get_wayfern_token().await;
    let mut generate_params = json!({ "operatingSystem": os });
    if let Some(ref token) = wayfern_token {
      generate_params
        .as_object_mut()
        .unwrap()
        .insert("wayfernToken".to_string(), json!(token));
    }

    let use_identity_api = supports_identity_api(&profile.version);

    // No geolocation override is passed here. Donut resolves the exit's
    // location itself, below, through the profile's own proxy, because the
    // browser cannot resolve it through an authenticated upstream.
    let generate_result = if use_identity_api {
      self
        .send_cdp_command(&ws_url, "Wayfern.createIdentity", generate_params)
        .await
    } else {
      match self
        .send_cdp_command(&ws_url, "Wayfern.refreshFingerprint", generate_params)
        .await
      {
        // The legacy pair is two commands: refresh mints the device, get reads
        // it back. Only the identity API returns the device from one call.
        Ok(_) => {
          self
            .send_cdp_command(&ws_url, "Wayfern.getFingerprint", json!({}))
            .await
        }
        Err(e) => Err(e),
      }
    };

    let (fingerprint, identity_id, geolocation_applied) = match generate_result {
      Ok(result) => {
        // createIdentity returns { identityId, identity }; getFingerprint
        // returns { fingerprint: {...} }. A bare object is tolerated so a
        // response-shape change does not lose the device outright; an identity
        // response missing identityId is rejected below instead, because a view
        // with no UUID behind it is not reproducible.
        let identity_id = result
          .get("identityId")
          .and_then(|v| v.as_str())
          .map(str::to_string);
        let fp = result
          .get("identity")
          .or_else(|| result.get("fingerprint"))
          .cloned()
          .unwrap_or(result);
        // Normalize the fingerprint: convert JSON string fields to proper types
        let mut normalized = Self::normalize_fingerprint(fp);

        // Build a transport that genuinely carries the probe through this
        // profile's upstream, or none at all. `probe_route` decides which:
        // reqwest where it can proxy the scheme itself, a temporary local
        // donut-proxy worker for the schemes that worker speaks (including
        // every remote SOCKS one, because reqwest's SOCKS connector corrupts
        // its parse buffer when a proxy splits a handshake reply across TCP
        // segments), an Xray-core sidecar behind such a worker for VLESS, and
        // nothing for an upstream none of them can speak.
        //
        // No transport is built when the probe would not leave this machine
        // anyway: `apply_geolocation` ignores `proxy` entirely when geolocation
        // is off or the location comes from a pinned geoip IP.
        let probe_leaves_this_machine = !matches!(
          config.geoip.as_ref(),
          Some(serde_json::Value::Bool(false)) | Some(serde_json::Value::String(_))
        );
        let transport = match config.proxy.as_deref() {
          Some(url) if probe_leaves_this_machine => Self::build_probe_transport(url).await,
          _ => ProbeTransport::default(),
        };

        // Apply timezone/geolocation for the proxy this fingerprint is being
        // generated against. Shared with the launch-time location refresh.
        // A profile that ROUTES ITS TRAFFIC must never probe from the real
        // address: `apply_geolocation` treats `None` as "no proxy configured",
        // which is right for a direct profile and an IP leak for a routed one.
        //
        // `config.proxy.is_some()` alone was not that predicate. A WireGuard
        // profile carries its route in `vpn_id` and reaches here with
        // `config.proxy == None`, so the guard never fired and the host's own
        // timezone, latitude/longitude and language were written into the
        // fingerprint as authoritative.
        let routes_traffic = config.proxy.is_some() || profile.vpn_id.is_some();
        let geolocation_applied = if Self::must_skip_probe(
          routes_traffic,
          probe_leaves_this_machine,
          transport.proxy.is_some(),
        ) {
          log::warn!(
            "Skipping the geolocation probe: this profile has an upstream but no proxied probe could be built, and probing directly would write this machine's own location into the fingerprint"
          );
          false
        } else {
          Self::apply_geolocation(
            &mut normalized,
            transport.proxy.as_deref(),
            config.geoip.as_ref(),
          )
          .await
        };

        transport.shutdown().await;

        (normalized, identity_id, geolocation_applied)
      }
      Err(e) => {
        cleanup().await;
        let what = if use_identity_api {
          "create identity"
        } else {
          "get fingerprint"
        };
        return Err(format!("Failed to {what}: {e}").into());
      }
    };

    cleanup().await;

    let fingerprint_json = serde_json::to_string(&fingerprint)
      .map_err(|e| format!("Failed to serialize fingerprint: {e}"))?;

    // Report the platform the engine actually produced alongside the one that
    // was asked for. Logging only the request made this line useless for
    // diagnosing a fingerprint that came back as something else.
    log::info!(
      "Generated Wayfern fingerprint for requested OS: {}, produced platform: {:?}, fields: {:?}",
      os,
      fingerprint.get("platform").and_then(|p| p.as_str()),
      fingerprint
        .as_object()
        .map(|o| o.keys().collect::<Vec<_>>())
    );

    // Log timezone/geolocation fields specifically for debugging
    if let Some(obj) = fingerprint.as_object() {
      log::info!(
        "Generated fingerprint - timezone: {:?}, timezoneOffset: {:?}, latitude: {:?}, longitude: {:?}, language: {:?}",
        obj.get("timezone"),
        obj.get("timezoneOffset"),
        obj.get("latitude"),
        obj.get("longitude"),
        obj.get("language")
      );
    }

    if use_identity_api && identity_id.is_none() {
      // Without the handle the stored fingerprint is not reproducible, which
      // would leave a profile that silently changes device on every launch.
      // The headless browser is already torn down by the `cleanup()` above.
      return Err("Wayfern.createIdentity returned no identityId".into());
    }

    Ok(GeneratedFingerprint {
      location: fingerprint.as_object().and_then(Self::location_of),
      fingerprint: fingerprint_json,
      identity_id,
      geolocation_applied,
    })
  }

  #[allow(clippy::too_many_arguments)]
  pub async fn launch_wayfern(
    &self,
    _app_handle: &AppHandle,
    profile: &BrowserProfile,
    profile_path: &str,
    config: &WayfernConfig,
    url: Option<&str>,
    proxy_url: Option<&str>,
    ephemeral: bool,
    extension_paths: &[String],
    remote_debugging_port: Option<u16>,
    headless: bool,
    kind: LaunchKind,
  ) -> Result<WayfernLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
    let executable_path = BrowserRunner::instance()
      .get_browser_executable_path(profile)
      .map_err(|e| format!("Failed to get Wayfern executable path: {e}"))?;

    let port = match remote_debugging_port {
      Some(p) => p,
      None => Self::find_free_port().await?,
    };
    log::info!("Launching Wayfern on CDP port {port} (detached)");

    // Diagnostic: verify critical profile files and test cookie decryption
    {
      let profile_path_buf = std::path::PathBuf::from(profile_path);
      let key_path = profile_path_buf.join("os_crypt_key");
      let cookies_path = {
        let network = profile_path_buf
          .join("Default")
          .join("Network")
          .join("Cookies");
        if network.exists() {
          network
        } else {
          profile_path_buf.join("Default").join("Cookies")
        }
      };

      if key_path.exists() {
        // Length only. The contents are the profile's encryption key, and this
        // log is the first thing a user attaches to a bug report.
        let key_len = std::fs::metadata(&key_path).map(|m| m.len()).unwrap_or(0);
        log::info!("Pre-launch: os_crypt_key present ({key_len} bytes)");
      } else {
        log::warn!("Pre-launch: os_crypt_key NOT FOUND");
      }

      if cookies_path.exists() {
        // Try to open Cookies DB and check if encrypted cookies can be decrypted
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
          &cookies_path,
          rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
          let cookie_count: i64 = conn
            .query_row(
              "SELECT COUNT(*) FROM cookies WHERE length(encrypted_value) > 0",
              [],
              |r| r.get(0),
            )
            .unwrap_or(0);
          let total_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM cookies", [], |r| r.get(0))
            .unwrap_or(0);
          log::info!(
            "Pre-launch: Cookies DB has {} total cookies, {} encrypted",
            total_count,
            cookie_count
          );

          // Try decrypting one cookie using the cookie_manager
          if let Some(encryption_key) =
            crate::cookie_manager::chrome_decrypt::get_encryption_key(&profile_path_buf)
          {
            if let Ok(mut stmt) = conn.prepare(
              "SELECT name, host_key, encrypted_value FROM cookies WHERE length(encrypted_value) > 0 LIMIT 1",
            ) {
              if let Ok(mut rows) = stmt.query([]) {
                if let Ok(Some(row)) = rows.next() {
                  let name: String = row.get(0).unwrap_or_default();
                  let host: String = row.get(1).unwrap_or_default();
                  let encrypted: Vec<u8> = row.get(2).unwrap_or_default();
                  let decrypted = crate::cookie_manager::chrome_decrypt::decrypt(
                    &encrypted,
                    &host,
                    &encryption_key,
                  );
                  match decrypted {
                    Some(val) => log::info!(
                      "Pre-launch: Cookie decryption SUCCEEDED for '{}' (host: {}, decrypted {} bytes)",
                      name, host, val.len()
                    ),
                    None => log::error!(
                      "Pre-launch: Cookie decryption FAILED for '{}' (host: {}, encrypted {} bytes)",
                      name, host, encrypted.len()
                    ),
                  }
                }
              }
            }
          } else {
            log::error!("Pre-launch: Failed to derive encryption key from os_crypt_key");
          }
        }
      } else {
        log::warn!("Pre-launch: Cookies NOT FOUND");
      }
    }

    let mut args = vec![
      format!("--remote-debugging-port={port}"),
      "--remote-debugging-address=127.0.0.1".to_string(),
      format!("--user-data-dir={profile_path}"),
      "--no-first-run".to_string(),
      "--no-default-browser-check".to_string(),
      "--disable-background-mode".to_string(),
      "--disable-component-update".to_string(),
      "--disable-background-timer-throttling".to_string(),
      "--crash-server-url=".to_string(),
      "--disable-updater".to_string(),
      "--hide-crash-restore-bubble".to_string(),
      // Release builds log nothing unless asked. Wayfern reports what it did
      // with the launch identity, WebRTC and the rest on stderr, and that is
      // the only channel that carries a refusal's reason.
      "--enable-logging=stderr".to_string(),
      "--log-level=0".to_string(),
      "--disable-infobars".to_string(),
      // Prefetch* / NoStatePrefetch: cross-site Speculation-Rules prefetch uses
      // an isolated NetworkContext that defaults to DIRECT egress (real host IP
      // leaks past the per-profile proxy). Disabling via a LAUNCH FLAG cannot be
      // re-enabled by an imported/synced network_prediction_options pref (which a
      // compile-time pref default could be).
      "--disable-features=DialMediaRouteProvider,DnsOverHttps,AsyncDns,Prefetch,PrefetchProxy,SpeculationRulesPrefetchFuture,NoStatePrefetch".to_string(),
      "--use-mock-keychain".to_string(),
      "--password-store=basic".to_string(),
    ];

    if headless {
      args.push("--headless=new".to_string());
    } else if let Some((w, h)) = config
      .fingerprint
      .as_deref()
      .and_then(Self::window_size_from_fingerprint)
    {
      // Size the real OS window to match the fingerprint so the visible window
      // agrees with the reported windowOuterWidth/screen dimensions. Anchor at
      // 0,0 so the window also fits within the spoofed screen origin. Skipped in
      // headless mode, where there is no on-screen window.
      log::info!("Sizing Wayfern window to fingerprint dimensions: {w}x{h}");
      args.push(format!("--window-size={w},{h}"));
      args.push("--window-position=0,0".to_string());
    }

    #[cfg(target_os = "linux")]
    {
      args.push("--no-sandbox".to_string());
      args.push("--disable-setuid-sandbox".to_string());
      args.push("--disable-dev-shm-usage".to_string());
    }

    if ephemeral {
      args.push("--disk-cache-size=1".to_string());
      args.push("--disable-breakpad".to_string());
      args.push("--disable-crash-reporter".to_string());
      args.push("--no-service-autorun".to_string());
      args.push("--disable-sync".to_string());
    }

    if !extension_paths.is_empty() {
      args.push(format!("--load-extension={}", extension_paths.join(",")));
    }

    // Per-profile window label + distinct frame color so concurrent profile
    // windows are easy to tell apart. The browser reads these switches and uses
    // them for the window title (label) and the frame colour. The label is the
    // profile name; the color is the user's window_color when set, otherwise
    // deterministically derived from the profile id so every profile still gets
    // a stable, distinct color.
    if !profile.name.is_empty() {
      args.push(format!("--wayfern-profile-label={}", profile.name));
    }
    // Profiles created before this feature have no stored color; persist the
    // id-derived one so the info dialog shows the same frame color the window
    // uses. It's deterministic per id, so no updated_at bump/sync is needed.
    if profile
      .window_color
      .as_deref()
      .map(str::trim)
      .unwrap_or("")
      .is_empty()
    {
      let mut backfilled = profile.clone();
      backfilled.window_color = Some(derive_profile_color(&backfilled.id));
      let _ = crate::profile::ProfileManager::instance().save_profile(&backfilled);
    }
    let profile_color = profile
      .window_color
      .clone()
      .filter(|c| !c.trim().is_empty())
      .unwrap_or_else(|| derive_profile_color(&profile.id));
    // Wayfern expects the frame color as bare RRGGBB hex, with no leading '#'
    // (the stored/user value may include one).
    let profile_color = profile_color.trim().trim_start_matches('#');
    args.push(format!("--wayfern-profile-color={profile_color}"));

    let mut wayfern_token = crate::cloud_auth::CLOUD_AUTH.get_wayfern_token().await;
    // Waiting is only meaningful for a plan a token can actually be minted for.
    // On "any active plan" this stalled every Solo launch by the full three
    // seconds waiting for a token the backend will never issue to them.
    if wayfern_token.is_none()
      && crate::cloud_auth::CLOUD_AUTH
        .is_entitled_to_wayfern_token()
        .await
    {
      // Brief wait for the background token fetch — when the API is healthy
      // the token usually lands in well under a second. If api.donutbrowser.com
      // is unreachable we don't want to gate the whole launch on it; the
      // browser still works without the token (cross-OS fingerprinting just
      // won't be enabled for this session, and the next launch will pick it
      // up once the token arrives).
      log::info!("Wayfern token not ready for paid user, waiting briefly...");
      for _ in 0..3 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        wayfern_token = crate::cloud_auth::CLOUD_AUTH.get_wayfern_token().await;
        if wayfern_token.is_some() {
          break;
        }
      }
      if wayfern_token.is_none() {
        log::warn!(
          "Wayfern token still unavailable after wait; launching without it (api.donutbrowser.com may be unreachable)"
        );
      }
    }

    // A cross-OS claim is authorized from the `wayfernToken` PARAMETER of
    // setIdentity/setFingerprint, so with no token in hand the apply is refused
    // and the window would sit there running the HOST device under a macOS or
    // Android profile. Refuse before spawning rather than opening a window we
    // are about to kill.
    //
    // "Cross-OS" is the browser's own test against the host OS, so `android`
    // and `ios` count on every desktop. Every browser version gates it the same
    // way; what changed is that the refusal is no longer swallowed as a log
    // line (see the apply loop below).
    //
    // Deliberately conservative — this only pre-empts when the claim is
    // certain. An unrecognised `os`, a platform that maps to nothing, and a
    // profile with no stored device all fall through and let the browser
    // decide, so a mistake here can only ever cost the clearer error message.
    if wayfern_token.is_none() {
      let stored_device = config
        .fingerprint
        .as_deref()
        .and_then(Self::fingerprint_object);
      if let Some(claimed) = Self::claimed_operating_system(config, stored_device.as_ref()) {
        let host_os = crate::profile::types::get_host_os();
        if claimed != host_os.as_str() {
          log::error!(
            "Refusing to launch profile {}: it claims {claimed} on a {host_os} host and no Wayfern token is available",
            profile.name
          );
          return Err(
            crate::backend_error_with_detail("WAYFERN_CROSS_OS_REQUIRES_PLAN", claimed).into(),
          );
        }
      }
    }

    if let Some(proxy) = proxy_url {
      // Map the local proxy scheme to the matching PAC directive. SOCKS5 lets
      // Chromium route UDP (QUIC/WebRTC) and resolve DNS through the proxy;
      // PROXY is HTTP CONNECT (TCP only). The host:port is the same either way.
      let (pac_directive, host_port) = if let Some(rest) = proxy.strip_prefix("socks5://") {
        ("SOCKS5", rest)
      } else {
        (
          "PROXY",
          proxy
            .trim_start_matches("http://")
            .trim_start_matches("https://"),
        )
      };
      let pac_data = format!(
        "data:application/x-ns-proxy-autoconfig,function FindProxyForURL(url,host){{return \"{pac_directive} {host_port}\";}}",
      );
      args.push(format!("--proxy-pac-url={pac_data}"));
      args.push("--dns-prefetch-disable".to_string());
    }

    // A 152 browser takes the identity on its command line and commits it
    // before the first navigation, which is what lets a restored tab load on
    // the profile's device rather than the host's. Older browsers, legacy
    // device payloads and a profile whose location carries no timezone keep
    // the post-launch CDP apply below.
    let launch_identity = if supports_wayfern_152(&profile.version) {
      Self::launch_identity_document(config)
    } else {
      None
    };
    if launch_identity.is_none()
      && supports_wayfern_152(&profile.version)
      && config.identity_id.is_some()
      && config.fingerprint.is_none()
    {
      log::warn!(
        "Profile {} has an identity but no resolved timezone (or no claimed operating system); applying it over CDP after launch instead of at startup",
        profile.name
      );
    }
    let identity_file = match &launch_identity {
      Some(document) => Some(
        Self::write_launch_identity(profile_path, document)
          .map_err(|e| crate::backend_error_with_detail("WAYFERN_IDENTITY_REFUSED", e))?,
      ),
      None => None,
    };
    let identity_at_launch =
      identity_file.is_some() || (config.identity_id.is_none() && config.fingerprint.is_none());
    let restore_session = match session_restore_verdict(
      config,
      kind,
      headless,
      ephemeral,
      profile.clear_on_close,
      identity_at_launch,
    ) {
      Ok(()) => true,
      Err(reason) => {
        log::info!(
          "Session restore is off for profile {}: {reason}",
          profile.name
        );
        false
      }
    };
    args.extend(session_switches(restore_session, identity_file.as_deref()));

    // WebRTC posture, plus the exit the launch gate measured for this route
    // (cache only: an automation launch never probes). A direct connection
    // has nothing cached and needs nothing: its real egress is already what
    // every HTTP request shows.
    let webrtc_mode = WebRtcMode::from_config(config);
    if let Some(unknown) = config
      .webrtc_mode
      .as_deref()
      .filter(|value| WebRtcMode::parse(value).is_none())
    {
      log::warn!(
        "Profile {} names an unknown WebRTC mode {unknown:?}; launching with auto",
        profile.name
      );
    }
    let exit_ip = crate::fingerprint_consistency::cached_exit_ip(profile);
    let webrtc = webrtc_switches(&profile.version, webrtc_mode, exit_ip.as_deref());
    if !webrtc.is_empty() {
      log::info!(
        "WebRTC for profile {}: mode {}, exit IP {}",
        profile.name,
        webrtc_mode.switch_value(),
        exit_ip.as_deref().unwrap_or("unknown")
      );
    }
    args.extend(webrtc);
    args.extend(entitlement_cache_switch(
      &profile.version,
      &crate::app_dirs::cache_dir(),
    ));
    args.extend(profile_icon_switch(
      &profile.version,
      profile_path,
      &profile.name,
      profile_color,
    ));
    // The persona is a property of the profile, so an identity-backed profile
    // seeds it from the identity and a legacy one from its id: either way the
    // same profile presents the same person on every launch.
    let persona_seed = config
      .identity_id
      .as_deref()
      .map(str::trim)
      .filter(|id| !id.is_empty())
      .map(str::to_string)
      .unwrap_or_else(|| profile.id.to_string());
    args.extend(persona_switch(
      &profile.version,
      profile_path,
      &persona_seed,
      config.persona.as_deref(),
    ));
    args.extend(camera_switches(&profile.version, config));
    args.extend(widevine_switch(
      &profile.version,
      &crate::app_dirs::data_dir(),
    ));
    if let Some(path) = &identity_file {
      log::info!(
        "Launch identity for profile {} written to {}",
        profile.name,
        path.display()
      );
    }

    let mut command = TokioCommand::new(&executable_path);
    command
      .args(&args)
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::piped());
    if let Some(ref token) = wayfern_token {
      command.env("WAYFERN_TOKEN", token);
      log::info!("Wayfern authorization configured for browser process");
    }

    let mut child = command
      .spawn()
      .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        let hint = if e.raw_os_error() == Some(14001) {
          ". This usually means the Visual C++ Redistributable is not installed. \
           Download it from https://aka.ms/vs/17/release/vc_redist.x64.exe"
        } else {
          ""
        };
        format!("Failed to spawn Wayfern: {e}{hint}").into()
      })?;
    let process_id = child.id();
    let log_tap = BrowserLogTap::default();
    if let Some(stderr) = child.stderr.take() {
      tap_browser_stderr(stderr, log_tap.clone(), profile.name.clone());
    }
    drop(child);

    self.wait_for_cdp_ready(port).await?;

    let targets = self.get_cdp_targets(port).await?;
    log::info!("Found {} CDP targets", targets.len());

    let page_targets: Vec<_> = targets.iter().filter(|t| t.target_type == "page").collect();
    log::info!("Found {} page targets", page_targets.len());

    // An identity-backed profile: the id, the user's overrides and the exit's
    // location are all the browser needs, and all the profile stores. The
    // device comes back in the response and is deliberately NOT persisted.
    let identity_only = supports_identity_api(&profile.version)
      && config.identity_id.is_some()
      && config.fingerprint.is_none();
    if let Some(document) = launch_identity.as_ref().filter(|_| identity_file.is_some()) {
      // The identity travelled on the command line. The browser never fails
      // its own launch over it (a refusal only logs), so the launcher checks,
      // and a browser running on the wrong device is closed rather than
      // handed to the user with the app still showing the profile's device.
      let mut observed: Option<serde_json::Value> = None;
      let mut last_error: Option<String> = None;
      for target in &page_targets {
        if let Some(ws_url) = &target.websocket_debugger_url {
          match self
            .send_cdp_command(ws_url, "Wayfern.getIdentity", json!({}))
            .await
          {
            Ok(result) => {
              observed = Some(result);
              break;
            }
            Err(e) => last_error = Some(e.to_string()),
          }
        }
      }
      // The browser wrote its verdict to stderr before it opened the
      // debugging port; the tap may still be a line behind the socket.
      if !log_tap.has_identity_verdict() {
        tokio::time::sleep(Duration::from_millis(300)).await;
      }
      match Self::launch_identity_verdict(
        document,
        observed.as_ref(),
        last_error.as_deref(),
        &log_tap,
      ) {
        Ok(confirmation) => {
          log::info!(
            "Launch identity confirmed for profile {}: {confirmation}",
            profile.name
          );
          // The only place an identity-backed profile's screen is known: the
          // device is derived by the browser, so nothing on disk carries it.
          if let Some(device) = observed
            .as_ref()
            .and_then(|value| value.get("identity"))
            .map(ToString::to_string)
          {
            Self::warn_on_screen_over_host(&device, profile, _app_handle);
          }
        }
        Err(reason) => {
          log::error!(
            "Killing Wayfern (pid {process_id:?}) for profile {}: the launch identity was not applied: {reason}",
            profile.name
          );
          if let Some(pid) = process_id {
            kill_browser_process(pid);
          }
          return Err(crate::backend_error_with_detail("WAYFERN_IDENTITY_REFUSED", reason).into());
        }
      }
    } else if identity_only {
      let identity_id = config.identity_id.clone().unwrap_or_default();
      let overrides = Self::stored_object(config.identity_overrides.as_deref());
      let location = Self::stored_object(config.location.as_deref());
      let wayfern_token = crate::cloud_auth::CLOUD_AUTH.get_wayfern_token().await;

      let mut params = serde_json::Map::new();
      params.insert("identityId".to_string(), json!(identity_id));
      // The claimed OS travels explicitly as well as inside the id, because an
      // older browser cannot read an id minted by a newer one and would rebuild
      // the HOST OS instead. Every release lets the explicit parameter win, so
      // this keeps one stored profile portable across them.
      if let Some(os) = config.os.as_deref().filter(|os| !os.is_empty()) {
        params.insert("operatingSystem".to_string(), json!(os));
      }
      if !overrides.is_empty() {
        params.insert(
          "overrides".to_string(),
          serde_json::Value::Object(overrides.clone()),
        );
      }
      // Location is a property of the exit, not of the identity, so it travels
      // in setIdentity's own parameters rather than as an override.
      params.extend(Self::geo_params(&location));
      if let Some(ref token) = wayfern_token {
        params.insert("wayfernToken".to_string(), json!(token));
      }
      log::info!(
        "Applying Wayfern identity {} with {} override(s): {:?}",
        identity_id,
        overrides.len(),
        overrides.keys().collect::<Vec<_>>()
      );

      let mut applied_ok = false;
      let mut last_apply_error: Option<String> = None;
      for target in &page_targets {
        if let Some(ws_url) = &target.websocket_debugger_url {
          match self
            .send_cdp_command(
              ws_url,
              "Wayfern.setIdentity",
              serde_json::Value::Object(params.clone()),
            )
            .await
          {
            Ok(_) => {
              applied_ok = true;
              log::info!("Successfully applied identity to page target");
            }
            Err(e) => {
              log::error!("Failed to apply identity to target: {e}");
              last_apply_error = Some(e.to_string());
            }
          }
        }
      }
      if !applied_ok {
        let detail = last_apply_error
          .unwrap_or_else(|| "the browser exposed no page target to apply it to".to_string());
        log::error!(
          "Killing Wayfern (pid {process_id:?}) for profile {}: the identity was never applied: {detail}",
          profile.name
        );
        if let Some(pid) = process_id {
          kill_browser_process(pid);
        }
        return Err(
          Self::apply_failure_error(&detail, Self::claimed_operating_system(config, None)).into(),
        );
      }
    } else if let Some(fingerprint_json) = &config.fingerprint {
      log::info!(
        "Applying fingerprint to Wayfern browser, fingerprint length: {} chars",
        fingerprint_json.len()
      );
      Self::warn_on_screen_over_host(fingerprint_json, profile, _app_handle);

      // Both stored shapes, the bare object and the legacy
      // `{"fingerprint": {...}}` wrapper, are resolved by the same accessor the
      // consistency gate reads, and nothing is defaulted in. A profile that
      // declares no timezone is launched with none, which is exactly what the
      // gate reports to the user.
      let fingerprint_for_cdp = Self::launch_fingerprint_payload(fingerprint_json)?;

      log::info!(
        "Fingerprint prepared for CDP command, fields: {:?}",
        fingerprint_for_cdp
          .as_object()
          .map(|o| o.keys().collect::<Vec<_>>())
      );

      // Log timezone and geolocation fields specifically for debugging
      if let Some(obj) = fingerprint_for_cdp.as_object() {
        log::info!(
          "Timezone/Geolocation fields - timezone: {:?}, timezoneOffset: {:?}, latitude: {:?}, longitude: {:?}, language: {:?}, languages: {:?}",
          obj.get("timezone"),
          obj.get("timezoneOffset"),
          obj.get("latitude"),
          obj.get("longitude"),
          obj.get("language"),
          obj.get("languages")
        );
      }

      // Include wayfern token if available (enables cross-OS fingerprinting for paid users)
      let wayfern_token = crate::cloud_auth::CLOUD_AUTH.get_wayfern_token().await;

      // The device as donut holds it, for the diagnostic below.
      let stored = fingerprint_for_cdp.as_object().cloned().unwrap_or_default();

      // `setFingerprint` is the only command that reproduces a whole payload
      // exactly, and on a browser without the identity API it is the only
      // command there is. A profile whose browser HAS that API never reaches
      // here: the launch path mints it an identity and drops the payload
      // first, so a stored device is never sent as a device again.
      let mut apply_params = fingerprint_for_cdp.clone();
      if let Some(ref token) = wayfern_token {
        if let Some(obj) = apply_params.as_object_mut() {
          obj.insert("wayfernToken".to_string(), json!(token));
        }
      }

      // An apply that never lands is the worst outcome this launch has: the
      // window opens on an unmanaged device while every surface in the app
      // still shows the profile's stored one. Track it so the launch can fail
      // instead of reporting success.
      let mut applied_ok = false;
      let mut last_apply_error: Option<String> = None;

      for target in &page_targets {
        if let Some(ws_url) = &target.websocket_debugger_url {
          log::info!("Applying fingerprint to page target");
          match self
            .send_cdp_command(ws_url, "Wayfern.setFingerprint", apply_params.clone())
            .await
          {
            Ok(_) => {
              applied_ok = true;
              log::info!("Successfully applied fingerprint to page target");
            }
            Err(e) => {
              log::error!("Failed to apply fingerprint to target: {e}");
              last_apply_error = Some(e.to_string());
            }
          }
        }
      }

      if !applied_ok {
        // Includes `page_targets` being empty: there was no target to send to,
        // so the device was never applied either. Kill the browser rather than
        // leave a window running a device the user did not choose and the app
        // does not know about.
        let detail = last_apply_error
          .unwrap_or_else(|| "the browser exposed no page target to apply it to".to_string());
        log::error!(
          "Killing Wayfern (pid {process_id:?}) for profile {}: the fingerprint was never applied: {detail}",
          profile.name
        );
        if let Some(pid) = process_id {
          kill_browser_process(pid);
        }
        return Err(
          Self::apply_failure_error(
            &detail,
            Self::claimed_operating_system(config, Some(&stored)),
          )
          .into(),
        );
      }
    } else {
      log::warn!("No fingerprint found in config, browser will use default fingerprint");
    }

    // Geolocation is handled internally by the browser binary.

    if let Some(url) = url {
      if restore_session {
        // The tabs the browser reopened are the user's; the URL gets a tab of
        // its own instead of replacing whichever one came first.
        log::info!("Opening the launch URL in a new tab beside the restored session");
        if let Err(e) = self.open_url_on_port(port, url).await {
          log::error!("Failed to open the launch URL in a new tab: {e}");
        }
      } else {
        log::info!("Navigating to URL via CDP");
        if let Some(target) = page_targets.first() {
          if let Some(ws_url) = &target.websocket_debugger_url {
            if let Err(e) = self
              .send_cdp_command(ws_url, "Page.navigate", json!({ "url": url }))
              .await
            {
              log::error!("Failed to navigate to URL: {e}");
            }
          }
        }
      }
    }

    for target in &page_targets {
      if let Some(ws_url) = &target.websocket_debugger_url {
        let _ = self
          .send_cdp_command(ws_url, "Emulation.clearDeviceMetricsOverride", json!({}))
          .await;
        let _ = self
          .send_cdp_command(
            ws_url,
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": false }),
          )
          .await;
        let _ = self
          .send_cdp_command(
            ws_url,
            "Emulation.setEmulatedMedia",
            json!({ "media": "", "features": [] }),
          )
          .await;
      }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let instance = WayfernInstance {
      id: id.clone(),
      process_id,
      profile_path: Some(profile_path.to_string()),
      url: url.map(|s| s.to_string()),
      cdp_port: Some(port),
      log_tap,
    };

    let mut inner = self.inner.lock().await;
    inner.instances.insert(id.clone(), instance);

    Ok(WayfernLaunchResult {
      id,
      processId: process_id,
      profilePath: Some(profile_path.to_string()),
      url: url.map(|s| s.to_string()),
      cdp_port: Some(port),
    })
  }

  pub async fn stop_wayfern(
    &self,
    id: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Taken out of the map first, and the lock dropped: a clean shutdown can
    // take seconds, and nothing else should wait on it.
    let instance = {
      let mut inner = self.inner.lock().await;
      inner.instances.remove(id)
    };

    if let Some(instance) = instance {
      log::info!("Cleaning up Wayfern instance {}", instance.id);
      if let Some(pid) = instance.process_id {
        let outcome = self.stop_browser_process(pid, instance.cdp_port).await;
        log::info!("Stopped Wayfern instance {id} (PID: {pid}): {outcome}");
      }
    }

    Ok(())
  }

  /// Stop a browser the way its session files need: ask it to close over
  /// CDP so Chromium runs its own shutdown (that is what writes the session
  /// and commits the cookie jar), then terminate, then kill. Each step gets a
  /// bounded wait, so a wedged browser still ends within seconds.
  pub async fn stop_browser_process(&self, pid: u32, cdp_port: Option<u16>) -> StopOutcome {
    if !crate::proxy_storage::is_process_running(pid) {
      return StopOutcome::Closed;
    }
    if let Some(port) = cdp_port {
      match tokio::time::timeout(Duration::from_secs(3), self.browser_close(port)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::warn!("Browser.close was not accepted on port {port}: {e}"),
        Err(_) => log::warn!("Browser.close timed out on port {port}"),
      }
      if wait_for_exit(pid, Duration::from_secs(5)).await {
        return StopOutcome::Closed;
      }
      log::warn!("Wayfern (PID {pid}) did not exit after Browser.close; terminating it");
    }
    terminate_process(pid);
    if wait_for_exit(pid, Duration::from_secs(5)).await {
      return StopOutcome::Terminated;
    }
    log::warn!("Wayfern (PID {pid}) ignored the termination request; killing it");
    force_kill_process(pid);
    if wait_for_exit(pid, Duration::from_secs(2)).await {
      return StopOutcome::Killed;
    }
    StopOutcome::StillRunning
  }

  /// Send `Browser.close` on the browser endpoint. The browser answers with an
  /// empty result, or simply drops the socket on its way out; both mean it
  /// agreed, and the caller watches the process rather than the reply.
  async fn browser_close(&self, port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let version: serde_json::Value = self
      .http_client
      .get(format!("http://127.0.0.1:{port}/json/version"))
      .send()
      .await?
      .json()
      .await?;
    let ws_url = version["webSocketDebuggerUrl"]
      .as_str()
      .ok_or("the browser reported no webSocketDebuggerUrl")?;
    match self
      .send_cdp_command(ws_url, "Browser.close", json!({}))
      .await
    {
      Ok(_) => Ok(()),
      Err(e) if e.to_string().contains("No response received") => Ok(()),
      Err(e) => Err(e),
    }
  }

  /// Opens a URL in a new tab for an existing Wayfern instance.
  pub async fn open_url_in_tab(
    &self,
    profile_path: &str,
    url: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let inner = self.inner.lock().await;
    let target_path = std::path::Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf());

    let port = inner
      .instances
      .values()
      .find(|i| {
        i.profile_path
          .as_deref()
          .map(|p| {
            std::path::Path::new(p)
              .canonicalize()
              .unwrap_or_else(|_| std::path::Path::new(p).to_path_buf())
              == target_path
          })
          .unwrap_or(false)
      })
      .and_then(|i| i.cdp_port)
      .ok_or("Wayfern instance (with CDP port) not found for profile")?;
    drop(inner);
    self.open_url_on_port(port, url).await
  }

  /// Open `url` in a new tab of the browser on `port`, through the CDP HTTP
  /// convenience endpoint, which needs no page target to exist yet.
  async fn open_url_on_port(
    &self,
    port: u16,
    url: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let new_tab_url = format!(
      "http://127.0.0.1:{port}/json/new?{}",
      urlencoding::encode(url)
    );
    let resp = self
      .http_client
      .put(&new_tab_url)
      .send()
      .await
      .map_err(|e| format!("Failed to open new tab: {e}"))?;
    if !resp.status().is_success() {
      return Err(format!("CDP /json/new returned HTTP {}", resp.status()).into());
    }

    log::info!("Opened URL in new tab via CDP");
    Ok(())
  }

  /// What the running browser for `profile_path` has said about Wayfern on
  /// stderr so far (launch identity, refusals), newest last.
  #[allow(dead_code)]
  pub async fn browser_log_lines(&self, profile_path: &str) -> Vec<String> {
    let inner = self.inner.lock().await;
    let target_path = std::path::Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf());
    inner
      .instances
      .values()
      .find(|instance| {
        instance.profile_path.as_deref().is_some_and(|path| {
          std::path::Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| std::path::Path::new(path).to_path_buf())
            == target_path
        })
      })
      .map(|instance| instance.log_tap.lines())
      .unwrap_or_default()
  }

  pub async fn get_cdp_port(&self, profile_path: &str) -> Option<u16> {
    let inner = self.inner.lock().await;
    let target_path = std::path::Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf());

    for instance in inner.instances.values() {
      if let Some(path) = &instance.profile_path {
        let instance_path = std::path::Path::new(path)
          .canonicalize()
          .unwrap_or_else(|_| std::path::Path::new(path).to_path_buf());
        if instance_path == target_path {
          return instance.cdp_port;
        }
      }
    }
    None
  }

  pub async fn find_wayfern_by_profile(&self, profile_path: &str) -> Option<WayfernLaunchResult> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};

    let mut inner = self.inner.lock().await;

    // Canonicalize the target path for comparison
    let target_path = std::path::Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf());

    // Find the instance with the matching profile path
    let mut found_id: Option<String> = None;
    for (id, instance) in &inner.instances {
      if let Some(path) = &instance.profile_path {
        let instance_path = std::path::Path::new(path)
          .canonicalize()
          .unwrap_or_else(|_| std::path::Path::new(path).to_path_buf());
        if instance_path == target_path {
          found_id = Some(id.clone());
          break;
        }
      }
    }

    // If we found an instance, verify the process is still running
    if let Some(id) = found_id {
      if let Some(instance) = inner.instances.get(&id) {
        if let Some(pid) = instance.process_id {
          let system = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
          );
          let sysinfo_pid = sysinfo::Pid::from_u32(pid);

          if system.process(sysinfo_pid).is_some() {
            return Some(WayfernLaunchResult {
              id: id.clone(),
              processId: instance.process_id,
              profilePath: instance.profile_path.clone(),
              url: instance.url.clone(),
              cdp_port: instance.cdp_port,
            });
          } else {
            log::info!(
              "Wayfern process {} for profile {} is no longer running, cleaning up",
              pid,
              profile_path
            );
            inner.instances.remove(&id);
            return None;
          }
        }
      }
    }

    // If not found in in-memory instances, scan system processes.
    // This handles the case where the GUI was restarted but Wayfern is still running.
    if let Some((pid, found_profile_path, cdp_port)) =
      Self::find_wayfern_process_by_profile(&target_path)
    {
      log::info!(
        "Found running Wayfern process (PID: {}) for profile path via system scan",
        pid
      );

      let instance_id = format!("recovered_{}", pid);
      inner.instances.insert(
        instance_id.clone(),
        WayfernInstance {
          id: instance_id.clone(),
          process_id: Some(pid),
          profile_path: Some(found_profile_path.clone()),
          url: None,
          cdp_port,
          log_tap: BrowserLogTap::default(),
        },
      );

      return Some(WayfernLaunchResult {
        id: instance_id,
        processId: Some(pid),
        profilePath: Some(found_profile_path),
        url: None,
        cdp_port,
      });
    }

    None
  }

  /// Scan system processes to find a Wayfern/Chromium process using a specific profile path
  fn find_wayfern_process_by_profile(
    target_path: &std::path::Path,
  ) -> Option<(u32, String, Option<u16>)> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};

    let system = System::new_with_specifics(
      RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );

    let target_path_str = target_path.to_string_lossy();

    for (pid, process) in system.processes() {
      let cmd = process.cmd();
      if cmd.is_empty() {
        continue;
      }

      let exe_name = process.name().to_string_lossy().to_lowercase();
      let is_chromium_like = exe_name.contains("wayfern")
        || exe_name.contains("chromium")
        || exe_name.contains("chrome");

      if !is_chromium_like {
        continue;
      }

      // Skip child processes (renderer, GPU, utility, zygote, etc.)
      // Only the main browser process lacks a --type= argument
      let is_child = cmd
        .iter()
        .any(|a| a.to_str().is_some_and(|s| s.starts_with("--type=")));
      if is_child {
        continue;
      }

      let mut matched = false;
      let mut cdp_port: Option<u16> = None;

      for arg in cmd.iter() {
        if let Some(arg_str) = arg.to_str() {
          if let Some(dir_val) = arg_str.strip_prefix("--user-data-dir=") {
            let cmd_path = std::path::Path::new(dir_val)
              .canonicalize()
              .unwrap_or_else(|_| std::path::Path::new(dir_val).to_path_buf());
            if cmd_path == target_path {
              matched = true;
            }
          }

          if let Some(port_val) = arg_str.strip_prefix("--remote-debugging-port=") {
            cdp_port = port_val.parse().ok();
          }
        }
      }

      if matched {
        return Some((pid.as_u32(), target_path_str.to_string(), cdp_port));
      }
    }

    None
  }

  #[allow(dead_code)]
  pub async fn launch_wayfern_profile(
    &self,
    app_handle: &AppHandle,
    profile: &BrowserProfile,
    config: &WayfernConfig,
    url: Option<&str>,
    proxy_url: Option<&str>,
  ) -> Result<WayfernLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
    let profiles_dir = self.get_profiles_dir();
    let profile_path = profiles_dir.join(profile.id.to_string()).join("profile");
    let profile_path_str = profile_path.to_string_lossy().to_string();

    std::fs::create_dir_all(&profile_path)?;

    if let Some(existing) = self.find_wayfern_by_profile(&profile_path_str).await {
      log::info!("Stopping existing Wayfern instance for profile");
      self.stop_wayfern(&existing.id).await?;
    }

    self
      .launch_wayfern(
        app_handle,
        profile,
        &profile_path_str,
        config,
        url,
        proxy_url,
        profile.ephemeral,
        &[],
        None,
        false,
        LaunchKind::Interactive,
      )
      .await
  }

  #[allow(dead_code)]
  pub async fn cleanup_dead_instances(&self) {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};

    let mut inner = self.inner.lock().await;
    let mut dead_ids = Vec::new();

    let system = System::new_with_specifics(
      RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );

    for (id, instance) in &inner.instances {
      if let Some(pid) = instance.process_id {
        let pid = sysinfo::Pid::from_u32(pid);
        if !system.processes().contains_key(&pid) {
          dead_ids.push(id.clone());
        }
      }
    }

    for id in dead_ids {
      log::info!("Cleaning up dead Wayfern instance: {id}");
      inner.instances.remove(&id);
    }
  }
}

/// Terminate a browser process by pid.
///
/// Shared with the launch path, which cannot go through `stop_wayfern`: an
/// instance is only registered in `inner.instances` at the very end of
/// `launch_wayfern`, so a launch that aborts part-way has a live process and no
/// id to stop it by.
fn kill_browser_process(pid: u32) {
  #[cfg(unix)]
  terminate_process(pid);
  #[cfg(windows)]
  force_kill_process(pid);
}

lazy_static::lazy_static! {
  static ref WAYFERN_MANAGER: WayfernManager = WayfernManager::new();
}

/// Deterministically derive a pleasant, distinct window frame color from a
/// profile id so concurrent profile windows are visually distinguishable even
/// when the user has not picked a custom color. Stable per profile (same id
/// always yields the same color). Returns "#RRGGBB".
pub fn derive_profile_color(id: &uuid::Uuid) -> String {
  // FNV-1a over the 16 id bytes -> hue in [0,360). The hue varies per profile
  // while saturation/lightness are fixed to a pastel band (see below).
  let mut h: u32 = 2166136261;
  for &b in id.as_bytes() {
    h = (h ^ u32::from(b)).wrapping_mul(16777619);
  }
  let hue = f64::from(h % 360);
  // Pastel: high lightness + soft saturation so windows stay easy to tell apart
  // without a garish frame.
  let (r, g, b) = hsl_to_rgb(hue, 0.6, 0.8);
  format!("#{r:02x}{g:02x}{b:02x}")
}

/// Convert HSL (h in [0,360), s/l in [0,1]) to 8-bit RGB.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
  let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
  let hp = h / 60.0;
  let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
  let (r1, g1, b1) = match hp as i32 {
    0 => (c, x, 0.0),
    1 => (x, c, 0.0),
    2 => (0.0, c, x),
    3 => (0.0, x, c),
    4 => (x, 0.0, c),
    _ => (c, 0.0, x),
  };
  let m = l - c / 2.0;
  let to_u8 = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
  (to_u8(r1), to_u8(g1), to_u8(b1))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn remote_socks_url_detection() {
    // Remote socks upstreams (the hyper-util-affected case) are detected...
    assert!(WayfernManager::needs_local_worker_for_probe(
      "socks5://user:pass@gw.dataimpulse.com:10000"
    ));
    assert!(WayfernManager::needs_local_worker_for_probe(
      "socks5://1.2.3.4:1080"
    ));
    assert!(WayfernManager::needs_local_worker_for_probe(
      "socks4://1.2.3.4:1080"
    ));

    // ...but the app's own loopback workers are not. socks is a non-special
    // URL scheme, so the IP literal parses as Host::Domain — the launch-time
    // randomize path depends on this returning false.
    assert!(!WayfernManager::needs_local_worker_for_probe(
      "socks5://127.0.0.1:24001"
    ));
    assert!(!WayfernManager::needs_local_worker_for_probe(
      "socks5://[::1]:24001"
    ));
    assert!(!WayfernManager::needs_local_worker_for_probe(
      "socks5://localhost:24001"
    ));

    // http/https reqwest proxies natively, so they stay on the direct path.
    assert!(!WayfernManager::needs_local_worker_for_probe(
      "http://gw.dataimpulse.com:10000"
    ));
    assert!(!WayfernManager::needs_local_worker_for_probe(
      "https://gw.dataimpulse.com:10000"
    ));
    // A hostless socks URL has nothing to route to; reqwest cannot use it, and
    // the worker path ends in a skipped probe rather than an unproxied one.
    assert!(!WayfernManager::needs_local_worker_for_probe("socks5://"));
    // An unparsable upstream asks for a worker ON PURPOSE. It used to answer
    // "no worker needed", which sent it down the arm that hands reqwest the raw
    // string, and reqwest answers garbage by silently going DIRECT. The worker
    // will fail to start for a malformed URL, and that failure now skips the
    // probe entirely, which is the outcome we want: no geolocation beats
    // geolocation taken from the user's own address.
    assert!(WayfernManager::needs_local_worker_for_probe("not a url"));

    // The schemes reqwest ACCEPTS and then silently does not proxy. Before
    // these were routed through a local worker, the fingerprint's geolocation
    // probe went out from the user's real address with nothing logged.
    // `httpstls` is NOT in this group: `reqwest_upstream_url` rewrites it to
    // `https://`, which reqwest proxies natively and over TLS, so the probe
    // genuinely goes through the proxy without spawning a worker. The rewrite
    // is what makes it safe, which is why the filter measures the REWRITTEN url
    // rather than the stored one.
    assert!(!WayfernManager::needs_local_worker_for_probe(
      "httpstls://user:pass@proxy.example.com:443"
    ));
    assert!(WayfernManager::needs_local_worker_for_probe(
      "ss://method:pass@1.2.3.4:8388"
    ));
    assert!(WayfernManager::needs_local_worker_for_probe(
      "vless://uuid@1.2.3.4:443"
    ));
    // Loopback is exempt ONLY for schemes reqwest can proxy. `httpstls` is
    // rewritten to `https`, so a loopback one genuinely needs no worker.
    assert!(!WayfernManager::needs_local_worker_for_probe(
      "httpstls://127.0.0.1:8443"
    ));
    // But a LOOPBACK ss/vless is still a scheme reqwest discards. Exempting
    // these re-opened the entire leak for anyone pointing at a locally
    // forwarded Shadowsocks or VLESS endpoint.
    for loopback in [
      "ss://cipher:pw@127.0.0.1:8388",
      "ss://cipher:pw@localhost:8388",
      "ss://cipher:pw@[::1]:8388",
      "shadowsocks://cipher:pw@127.0.0.1:8388",
      "vless://uuid@127.0.0.1:443",
    ] {
      assert!(
        WayfernManager::needs_local_worker_for_probe(loopback),
        "{loopback} must still go through a worker"
      );
    }
    // An ALLOW-list, so a scheme nobody anticipated cannot fall through.
    // `proxy_type` is free text via POST /v1/proxies, import_proxies_json and
    // the MCP tool, so this is reachable without a code change. Note this only
    // says "reqwest must not be handed it", whether anything here can carry it
    // is `probe_route`'s answer, tested below.
    for unknown in [
      "trojan://gw.example.com:443",
      "hysteria2://gw.example.com:443",
      "wireguard://gw.example.com:51820",
      "SS://cipher:pw@1.2.3.4:8388",
    ] {
      assert!(
        WayfernManager::needs_local_worker_for_probe(unknown),
        "{unknown} is not proxyable by reqwest and must not be handed to it"
      );
    }
    // socks5h is proxyable and remote, so it keeps the socks rule.
    assert!(WayfernManager::needs_local_worker_for_probe(
      "socks5h://1.2.3.4:1080"
    ));
    // http/https reqwest proxies natively, so they stay on the direct path.
    assert!(!WayfernManager::needs_local_worker_for_probe(
      "https://user:pass@proxy.example.com:8443"
    ));
  }

  /// A complete VLESS + XTLS Vision + REALITY URI, the only shape Donut takes.
  fn valid_vless_uri() -> String {
    "vless://6d6e21a1-4829-4d2b-bc7f-1b25707b61e4@vpn.example.com:443\
?security=reality&flow=xtls-rprx-vision&encryption=none&type=tcp&sni=a.com\
&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4&sid=00&fp=chrome"
      .to_string()
  }

  #[test]
  fn worker_upstream_url_mirrors_what_donut_proxy_can_actually_dial() {
    // This list is `proxy_server::dial_upstream`'s match arms. Anything outside
    // it reaches that function's `_` arm, and every request through the worker
    // fails as "Unsupported upstream proxy scheme".
    for carried in [
      "http://gw.example.com:8080",
      "https://gw.example.com:8080",
      "httpstls://user:pass@gw.example.com:443",
      "socks4://1.2.3.4:1080",
      "socks5://user:pass@gw.example.com:1080",
      "ss://aes-256-gcm:pw@1.2.3.4:8388",
      "shadowsocks://aes-256-gcm:pw@1.2.3.4:8388",
    ] {
      assert_eq!(
        WayfernManager::worker_upstream_url(carried).as_deref(),
        Some(carried),
        "{carried} is dialable by a donut-proxy worker and must pass through unchanged"
      );
    }

    // VLESS is the whole point of this defect: a worker started on it can never
    // complete a single request, so it must not be offered as a transport.
    assert_eq!(
      WayfernManager::worker_upstream_url(&valid_vless_uri()),
      None,
      "a donut-proxy worker cannot speak VLESS"
    );
    for unspeakable in [
      "vless://uuid@1.2.3.4:443",
      "trojan://gw.example.com:443",
      "hysteria2://gw.example.com:443",
      "wireguard://gw.example.com:51820",
      "not a url",
    ] {
      assert_eq!(
        WayfernManager::worker_upstream_url(unspeakable),
        None,
        "{unspeakable} is not dialable by a donut-proxy worker"
      );
    }

    // The "resolve at the exit" spellings mean the same thing to the worker,
    // which hands the target hostname to the SOCKS server rather than resolving
    // it here, but `dial_upstream` matches the scheme literally, so they have
    // to arrive spelled the way it matches.
    assert_eq!(
      WayfernManager::worker_upstream_url("socks5h://user:pass@gw.example.com:1080").as_deref(),
      Some("socks5://user:pass@gw.example.com:1080")
    );
    assert_eq!(
      WayfernManager::worker_upstream_url("socks4a://1.2.3.4:1080").as_deref(),
      Some("socks4://1.2.3.4:1080")
    );
    // An uppercase scheme is still the same scheme; `Url::parse` lowercases it
    // anyway, so normalizing here keeps the allow-list from rejecting it.
    assert_eq!(
      WayfernManager::worker_upstream_url("SS://aes-256-gcm:pw@1.2.3.4:8388").as_deref(),
      Some("ss://aes-256-gcm:pw@1.2.3.4:8388")
    );
  }

  #[test]
  fn vless_is_probed_through_xray_or_not_at_all_but_never_through_a_worker() {
    // A complete VLESS URI: an Xray-core sidecar carries the hop, exactly as
    // browser_runner does for a VLESS launch.
    let uri = valid_vless_uri();
    assert_eq!(
      WayfernManager::probe_route(&uri),
      ProbeRoute::Xray(uri.clone())
    );

    // A `vless://host:port` is what a stored VLESS proxy collapses to when it
    // is rendered as `type://host:port`: no id, no flow, no REALITY key. Xray
    // cannot dial it and neither can a worker, so the probe is skipped and the
    // location is left ungenerated rather than defaulted.
    for lossy in [
      "vless://vpn.example.com:443",
      "vless://uuid@vpn.example.com:443",
      // Right shape, unsupported transport, still nothing that can carry it.
      "vless://6d6e21a1-4829-4d2b-bc7f-1b25707b61e4@a.com:443?security=tls&type=ws",
    ] {
      assert_eq!(
        WayfernManager::probe_route(lossy),
        ProbeRoute::Unroutable,
        "{lossy} has no transport and must not start a worker"
      );
    }

    // And no VLESS shape may EVER resolve to a plain donut-proxy worker: that
    // worker answers every request with "Unsupported upstream proxy scheme",
    // which used to land as a default America/New_York in the fingerprint.
    for any_vless in [
      uri.as_str(),
      "vless://vpn.example.com:443",
      "vless://uuid@127.0.0.1:443",
      "VLESS://vpn.example.com:443",
    ] {
      assert!(
        !matches!(
          WayfernManager::probe_route(any_vless),
          ProbeRoute::Worker(_) | ProbeRoute::Reqwest(_)
        ),
        "{any_vless} must never be handed to a donut-proxy worker or to reqwest"
      );
    }
  }

  #[test]
  fn probe_route_sends_each_upstream_to_something_that_carries_it() {
    // reqwest proxies these itself; `httpstls` arrives already rewritten to the
    // `https` spelling reqwest understands.
    assert_eq!(
      WayfernManager::probe_route("http://gw.example.com:8080"),
      ProbeRoute::Reqwest("http://gw.example.com:8080".into())
    );
    assert_eq!(
      WayfernManager::probe_route("httpstls://user:pass@gw.example.com:443"),
      ProbeRoute::Reqwest("https://user:pass@gw.example.com:443".into())
    );
    // A loopback socks upstream IS the local worker the browser already uses,
    // so it needs no second one. This is the launch-time path.
    assert_eq!(
      WayfernManager::probe_route("socks5://127.0.0.1:24001"),
      ProbeRoute::Reqwest("socks5://127.0.0.1:24001".into())
    );

    // Remote SOCKS and Shadowsocks go through a worker, which speaks both.
    assert_eq!(
      WayfernManager::probe_route("socks5://user:pass@gw.example.com:1080"),
      ProbeRoute::Worker("socks5://user:pass@gw.example.com:1080".into())
    );
    assert_eq!(
      WayfernManager::probe_route("ss://aes-256-gcm:pw@1.2.3.4:8388"),
      ProbeRoute::Worker("ss://aes-256-gcm:pw@1.2.3.4:8388".into())
    );
    // The worker matches `socks5`, not `socks5h`, so the route carries the
    // spelling it dials rather than the stored one.
    assert_eq!(
      WayfernManager::probe_route("socks5h://1.2.3.4:1080"),
      ProbeRoute::Worker("socks5://1.2.3.4:1080".into())
    );

    // Schemes nothing here speaks. `proxy_type` is free text through the REST
    // API and MCP, so these are reachable without a code change, and each one
    // must end in a skipped probe rather than a fabricated location.
    for unroutable in [
      "trojan://gw.example.com:443",
      "hysteria2://gw.example.com:443",
      "wireguard://gw.example.com:51820",
      "not a url",
    ] {
      assert_eq!(
        WayfernManager::probe_route(unroutable),
        ProbeRoute::Unroutable,
        "{unroutable} has no transport that carries it"
      );
    }
  }

  #[test]
  fn a_pinned_geoip_ip_still_resolves_on_a_routed_profile() {
    // The probe is skipped only when it would actually leave this machine with
    // nothing to carry it. A routed profile whose location comes from a pinned
    // geoip IP resolves it locally, so it must not be skipped for want of a
    // transport it never needed.
    assert!(!WayfernManager::must_skip_probe(true, false, false));
    // Geolocation genuinely going out over a routed profile's upstream, with no
    // transport built: this is the case that must never probe.
    assert!(WayfernManager::must_skip_probe(true, true, false));
    // Transport built, or nothing routed: probe away.
    assert!(!WayfernManager::must_skip_probe(true, true, true));
    assert!(!WayfernManager::must_skip_probe(false, true, false));
  }

  #[tokio::test]
  async fn a_failed_geolocation_probe_leaves_the_location_ungenerated() {
    // A geoip string skips the network fetch entirely and fails in the local
    // MaxMind lookup, so this drives apply_geolocation's Err branch with no
    // network, no proxy and no worker.
    let mut fingerprint = json!({ "platform": "Win32" });
    let applied =
      WayfernManager::apply_geolocation(&mut fingerprint, None, Some(&json!("not-an-ip"))).await;

    assert!(!applied, "a failed lookup must not report success");
    let obj = fingerprint
      .as_object()
      .expect("fingerprint stays an object");
    for invented in [
      "timezone",
      "timezoneOffset",
      "latitude",
      "longitude",
      "language",
      "languages",
    ] {
      assert!(
        !obj.contains_key(invented),
        "a failed probe must not invent {invented}: {obj:?}"
      );
    }
    assert_eq!(obj.get("platform"), Some(&json!("Win32")));
  }

  #[tokio::test]
  async fn a_failed_geolocation_probe_does_not_overwrite_a_real_location() {
    // The other half: a fingerprint that already carries a resolved location
    // keeps it verbatim when a later probe fails.
    let mut fingerprint = json!({
      "timezone": "Europe/Berlin",
      "timezoneOffset": -60,
      "latitude": 52.52,
    });
    let applied =
      WayfernManager::apply_geolocation(&mut fingerprint, None, Some(&json!("not-an-ip"))).await;

    assert!(!applied);
    assert_eq!(fingerprint["timezone"], json!("Europe/Berlin"));
    assert_eq!(fingerprint["timezoneOffset"], json!(-60));
    assert_eq!(fingerprint["latitude"], json!(52.52));
  }

  fn obj(json: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str(json).expect("test fixture must be an object")
  }

  fn identity_config(timezone: Option<&str>) -> WayfernConfig {
    let mut location = serde_json::Map::new();
    if let Some(tz) = timezone {
      location.insert("timezone".into(), json!(tz));
      location.insert("timezoneOffset".into(), json!(60));
    }
    location.insert("language".into(), json!("de-DE"));
    location.insert("languages".into(), json!(["de-DE", "de"]));
    location.insert("latitude".into(), json!(52.52));
    location.insert("longitude".into(), json!(13.405));
    WayfernConfig {
      identity_id: Some("3fa85f64-5717-4562-b3fc-2c963f66afa6".into()),
      os: Some("windows".into()),
      location: Some(serde_json::Value::Object(location).to_string()),
      identity_overrides: Some(r#"{"hardwareConcurrency":8}"#.into()),
      ..Default::default()
    }
  }

  #[test]
  fn the_launch_identity_document_carries_exactly_what_the_browser_reads() {
    let document =
      WayfernManager::launch_identity_document(&identity_config(Some("Europe/Berlin")))
        .expect("a complete identity profile yields a document");
    assert_eq!(
      document,
      json!({
        "identityId": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
        "operatingSystem": "windows",
        "timezone": "Europe/Berlin",
        "language": "de-DE",
        "latitude": 52.52,
        "longitude": 13.405,
        "overrides": {"hardwareConcurrency": 8}
      })
    );
  }

  #[test]
  fn the_launch_identity_document_needs_a_timezone_and_an_identity() {
    // The browser refuses a document without a timezone, and does not resolve
    // one itself, so no document is written at all.
    assert!(WayfernManager::launch_identity_document(&identity_config(None)).is_none());
    // No claim means the host, which is what an omitted `operatingSystem`
    // means over CDP as well.
    let mut no_os = identity_config(Some("Europe/Berlin"));
    no_os.os = None;
    assert_eq!(
      WayfernManager::launch_identity_document(&no_os).unwrap()["operatingSystem"],
      json!(crate::profile::types::get_host_os())
    );
    let mut no_identity = identity_config(Some("Europe/Berlin"));
    no_identity.identity_id = Some("  ".into());
    assert!(WayfernManager::launch_identity_document(&no_identity).is_none());
    // A legacy device payload is only reproducible through setFingerprint.
    let mut legacy = identity_config(Some("Europe/Berlin"));
    legacy.fingerprint = Some("{}".into());
    assert!(WayfernManager::launch_identity_document(&legacy).is_none());
  }

  #[test]
  fn the_launch_identity_document_omits_what_the_location_lacks() {
    let mut config = identity_config(Some("Asia/Tokyo"));
    config.location = Some(r#"{"timezone":"Asia/Tokyo","latitude":35.6}"#.into());
    config.identity_overrides = None;
    let document = WayfernManager::launch_identity_document(&config).unwrap();
    assert_eq!(
      document,
      json!({
        "identityId": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
        "operatingSystem": "windows",
        "timezone": "Asia/Tokyo"
      }),
      "a lone latitude, an absent language and empty overrides must not travel"
    );
  }

  #[test]
  fn session_restore_is_only_for_a_person_on_a_device_committed_at_launch() {
    let config = WayfernConfig::default();
    let interactive = |c: &WayfernConfig| {
      session_restore_verdict(c, LaunchKind::Interactive, false, false, false, true)
    };
    assert_eq!(interactive(&config), Ok(()));
    assert!(
      session_restore_verdict(&config, LaunchKind::Automation, false, false, false, true).is_err()
    );
    assert!(
      session_restore_verdict(&config, LaunchKind::Interactive, true, false, false, true).is_err()
    );
    assert!(
      session_restore_verdict(&config, LaunchKind::Interactive, false, true, false, true).is_err()
    );
    assert!(
      session_restore_verdict(&config, LaunchKind::Interactive, false, false, true, true).is_err()
    );
    assert!(
      session_restore_verdict(&config, LaunchKind::Interactive, false, false, false, false)
        .is_err(),
      "a device applied after the window opens loses the race with a restored tab"
    );
    let off = WayfernConfig {
      restore_session: Some(false),
      ..Default::default()
    };
    assert!(interactive(&off).is_err());
    let randomized = WayfernConfig {
      randomize_fingerprint_on_launch: Some(true),
      ..Default::default()
    };
    assert!(interactive(&randomized).is_err());
    let explicit_on = WayfernConfig {
      restore_session: Some(true),
      ..Default::default()
    };
    assert_eq!(interactive(&explicit_on), Ok(()));
  }

  #[test]
  fn the_webrtc_mode_reads_the_new_field_then_the_legacy_flag() {
    assert_eq!(
      WebRtcMode::from_config(&WayfernConfig::default()),
      WebRtcMode::Auto
    );
    let legacy = WayfernConfig {
      block_webrtc: Some(true),
      ..Default::default()
    };
    assert_eq!(WebRtcMode::from_config(&legacy), WebRtcMode::Block);
    let both = WayfernConfig {
      block_webrtc: Some(true),
      webrtc_mode: Some("tcp_only".into()),
      ..Default::default()
    };
    assert_eq!(WebRtcMode::from_config(&both), WebRtcMode::TcpOnly);
    let unknown = WayfernConfig {
      webrtc_mode: Some("sideways".into()),
      ..Default::default()
    };
    assert_eq!(WebRtcMode::from_config(&unknown), WebRtcMode::Auto);
    assert_eq!(WebRtcMode::parse("TCP-ONLY"), Some(WebRtcMode::TcpOnly));
  }

  #[test]
  fn the_webrtc_switches_carry_the_exit_only_when_it_can_be_used() {
    assert!(webrtc_switches("151.0.7922.76", WebRtcMode::Block, Some("8.8.8.8")).is_empty());
    assert_eq!(
      webrtc_switches("152.0.7977.64", WebRtcMode::Block, Some("8.8.8.8")),
      vec!["--wayfern-webrtc-mode=block".to_string()]
    );
    assert_eq!(
      webrtc_switches("152.0.7977.64", WebRtcMode::TcpOnly, Some(" 8.8.8.8 ")),
      vec![
        "--wayfern-webrtc-mode=tcp_only".to_string(),
        "--wayfern-webrtc-exit-ip=8.8.8.8".to_string()
      ]
    );
    assert_eq!(
      webrtc_switches("152.0.7977.64", WebRtcMode::Auto, Some("not an ip")),
      vec!["--wayfern-webrtc-mode=auto".to_string()],
      "a literal the browser would refuse is not passed at all"
    );
    assert_eq!(
      webrtc_switches("152.0.7977.64", WebRtcMode::Auto, None),
      vec!["--wayfern-webrtc-mode=auto".to_string()]
    );
  }

  #[test]
  fn the_entitlement_cache_lives_under_the_app_cache_and_only_on_152() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(entitlement_cache_switch("151.0.7922.76", root.path()), None);
    let switch = entitlement_cache_switch("152.0.7977.64", root.path()).unwrap();
    let expected = root.path().join("wayfern-entitlements");
    assert_eq!(
      switch,
      format!("--wayfern-entitlement-cache-dir={}", expected.display())
    );
    assert!(
      expected.is_dir(),
      "the directory exists before the browser starts"
    );
  }

  #[test]
  fn the_window_badge_is_a_decodable_png_in_the_profile_colour() {
    assert_eq!(badge_initial("  donut shop"), "D");
    assert_eq!(badge_initial("42 things"), "4");
    assert_eq!(badge_initial("!!!"), "");
    assert!(badge_wants_dark_ink("#f5e6a0"));
    assert!(!badge_wants_dark_ink("#2b4c7e"));
    assert_eq!(render_profile_icon("Any", "not a colour"), None);

    let png = render_profile_icon("Donut", "#2b4c7e").expect("the badge renders");
    let image = image::load_from_memory(&png)
      .expect("a decodable PNG")
      .into_rgba8();
    assert_eq!((image.width(), image.height()), (256, 256));
    // Inside the rounded square, away from the initial: the profile colour.
    assert_eq!(image.get_pixel(40, 128).0, [0x2b, 0x4c, 0x7e, 0xff]);
    // The corners stay transparent so the badge reads as a tile, not a sheet.
    assert_eq!(image.get_pixel(2, 2).0[3], 0);
    // The initial is drawn in white ink somewhere in the middle third when the
    // machine has any font at all. Not sampled at the exact centre: that is
    // the counter of a "D", which stays the fill colour.
    if !badge_fonts().is_empty() {
      let ink = (80..176)
        .flat_map(|y| (80..176).map(move |x| (x, y)))
        .any(|(x, y)| {
          let p = image.get_pixel(x, y).0;
          p[0] > 0xc0 && p[1] > 0xc0 && p[2] > 0xc0
        });
      assert!(ink, "the initial must be drawn in the middle of the badge");
    }
  }

  #[test]
  fn the_profile_icon_switch_writes_the_badge_beside_the_data_dir() {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("profile");
    std::fs::create_dir_all(&data_dir).unwrap();
    let data_dir = data_dir.to_string_lossy().to_string();
    assert_eq!(
      profile_icon_switch("151.0.7922.76", &data_dir, "Donut", "ebb5ad"),
      None
    );
    let switch = profile_icon_switch("152.0.7977.64", &data_dir, "Donut", "ebb5ad").unwrap();
    let expected = root.path().join("window-icon.png").canonicalize().unwrap();
    assert_eq!(
      switch,
      format!("--wayfern-profile-icon={}", expected.display())
    );
    assert!(image::load_from_memory(&std::fs::read(expected).unwrap()).is_ok());
  }

  #[test]
  fn the_persona_document_is_written_per_profile_and_only_on_152() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().to_string_lossy().to_string();
    let seed = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    assert_eq!(persona_switch("151.0.7922.76", &dir, seed, None), None);
    let switch = persona_switch("152.0.7977.64", &dir, seed, None).unwrap();
    let path = root
      .path()
      .join("wayfern-persona.json")
      .canonicalize()
      .unwrap();
    assert_eq!(
      switch,
      format!("--wayfern-profile-persona={}", path.display())
    );
    let document: serde_json::Value =
      serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
      document["fields"].as_array().unwrap().len(),
      crate::wayfern_persona::FIELD_IDS.len()
    );

    // An edit lands, and a malformed edit blob is ignored rather than fatal.
    persona_switch(
      "152.0.7977.64",
      &dir,
      seed,
      Some(r#"[{"id":"email","label":"Email","value":"me@example.com"}]"#),
    )
    .unwrap();
    let edited: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(edited["fields"]
      .as_array()
      .unwrap()
      .iter()
      .any(|f| f["id"] == "email" && f["value"] == "me@example.com"));
    assert!(persona_switch("152.0.7977.64", &dir, seed, Some("not json")).is_some());
  }

  #[test]
  fn the_camera_switches_need_a_file_that_exists_and_a_sane_crop() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("frame.png");
    std::fs::write(&file, b"not really a png").unwrap();
    let path = file.to_string_lossy().to_string();
    let config = |file: Option<&str>, crop: Option<&str>| WayfernConfig {
      camera_file: file.map(str::to_string),
      camera_crop: crop.map(str::to_string),
      ..Default::default()
    };
    assert!(camera_switches("151.0.7922.76", &config(Some(&path), None)).is_empty());
    assert!(camera_switches("152.0.7977.64", &config(None, Some("0,0,10,10"))).is_empty());
    assert!(camera_switches("152.0.7977.64", &config(Some("/nope/frame.png"), None)).is_empty());
    assert_eq!(
      camera_switches("152.0.7977.64", &config(Some(&path), Some("0,0,640,480"))),
      vec![
        format!("--wayfern-camera-file={path}"),
        "--wayfern-camera-crop=0,0,640,480".to_string()
      ]
    );
    assert_eq!(
      camera_switches("152.0.7977.64", &config(Some(&path), Some("0,0,0,480"))),
      vec![format!("--wayfern-camera-file={path}")],
      "a crop with no area is dropped, the source is not"
    );
    assert!(!valid_camera_crop("1,2,3"));
    assert!(!valid_camera_crop("-1,0,10,10"));
    assert!(valid_camera_crop(" 1, 2, 30, 40 "));
  }

  #[test]
  fn widevine_travels_only_when_a_cdm_has_been_provisioned() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(widevine_switch("152.0.7977.64", root.path()), None);
    let dir = root.path().join("WidevineCdm");
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(
      widevine_switch("152.0.7977.64", root.path()),
      None,
      "an empty directory is not a provisioned CDM"
    );
    std::fs::write(dir.join("manifest.json"), b"{}").unwrap();
    assert_eq!(widevine_switch("151.0.7922.76", root.path()), None);
    assert_eq!(
      widevine_switch("152.0.7977.64", root.path()),
      Some(format!("--wayfern-widevine-cdm-dir={}", dir.display())),
      "a provisioned directory is passed even when its payload is missing, so the browser reports it"
    );
  }

  #[test]
  fn a_screen_claim_is_only_reported_when_it_is_bigger_than_the_display() {
    let device = |w: u32, h: u32| format!(r#"{{"screenWidth":{w},"screenHeight":{h}}}"#);
    assert_eq!(
      screen_claim_over_host(Some(&device(3840, 2160)), Some((2560, 1440))),
      Some((3840, 2160, 2560, 1440))
    );
    // Taller but not wider still cannot be shown.
    assert_eq!(
      screen_claim_over_host(Some(&device(1920, 2160)), Some((2560, 1440))),
      Some((1920, 2160, 2560, 1440))
    );
    assert_eq!(
      screen_claim_over_host(Some(&device(1920, 1080)), Some((2560, 1440))),
      None
    );
    assert_eq!(
      screen_claim_over_host(Some(&device(2560, 1440)), Some((2560, 1440))),
      None,
      "an exact fit is not a mismatch"
    );
    // Nothing to compare: no device, no host, a device with no screen, or a
    // display that reports nothing.
    assert_eq!(screen_claim_over_host(None, Some((2560, 1440))), None);
    assert_eq!(
      screen_claim_over_host(Some(&device(3840, 2160)), None),
      None
    );
    assert_eq!(
      screen_claim_over_host(Some(r#"{"platform":"MacIntel"}"#), Some((2560, 1440))),
      None
    );
    assert_eq!(
      screen_claim_over_host(Some(&device(3840, 2160)), Some((0, 0))),
      None
    );
  }

  #[test]
  fn the_session_switches_follow_the_verdict() {
    assert!(session_switches(false, None).is_empty());
    assert_eq!(
      session_switches(true, None),
      vec!["--restore-last-session".to_string()]
    );
    let file = Path::new("/tmp/p/wayfern-identity.json");
    assert_eq!(
      session_switches(true, Some(file)),
      vec![
        "--wayfern-identity-file=/tmp/p/wayfern-identity.json".to_string(),
        "--restore-last-session".to_string()
      ]
    );
  }

  #[test]
  fn the_identity_verdict_trusts_the_browser_log_first() {
    let document = json!({
      "identityId": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
      "operatingSystem": "macos",
      "timezone": "Europe/Berlin",
      "language": "de-DE"
    });
    let tap = BrowserLogTap::default();
    tap.push("[1:2:0908/173819.393301:ERROR:wayfern_launch_identity.cc(58)] Wayfern launch identity refused: the identity file carries no `timezone`".into());
    let observed = json!({"identityId": "3fa85f64-5717-4562-b3fc-2c963f66afa6", "identity": {}});
    assert_eq!(
      WayfernManager::launch_identity_verdict(&document, Some(&observed), None, &tap),
      Err("the identity file carries no `timezone`".to_string()),
      "a logged refusal wins even over a matching id"
    );

    let tap = BrowserLogTap::default();
    assert!(
      WayfernManager::launch_identity_verdict(&document, Some(&observed), None, &tap).is_ok()
    );

    // 152 reports no identityId for a launch identity; the applied line
    // settles it.
    let tap = BrowserLogTap::default();
    tap.push("[1:2:0908/173819.393301:INFO:wayfern_launch_identity.cc(300)] Wayfern launch identity applied before the first navigation: os=macos timezone=Europe/Berlin language=de-DE version=1".into());
    let anonymous = json!({"identity": {"timezone": "Europe/Berlin", "language": "de-DE", "platform": "MacIntel"}});
    assert!(
      WayfernManager::launch_identity_verdict(&document, Some(&anonymous), None, &tap).is_ok()
    );

    // No log line at all: the running device is compared with the document.
    let tap = BrowserLogTap::default();
    assert!(
      WayfernManager::launch_identity_verdict(&document, Some(&anonymous), None, &tap).is_ok()
    );
    let host = json!({"identity": {"timezone": "America/New_York", "language": "en-US", "platform": "MacIntel"}});
    let error =
      WayfernManager::launch_identity_verdict(&document, Some(&host), None, &tap).unwrap_err();
    assert!(error.contains("America/New_York"), "{error}");
    assert!(error.contains("Europe/Berlin"), "{error}");
    assert_eq!(
      WayfernManager::launch_identity_verdict(&document, None, Some("boom"), &tap),
      Err("Wayfern.getIdentity failed: boom".to_string())
    );
  }

  #[test]
  fn the_log_tap_keeps_the_newest_lines() {
    let tap = BrowserLogTap::default();
    for i in 0..40 {
      tap.push(format!("line {i}"));
    }
    let lines = tap.lines();
    assert_eq!(lines.len(), BrowserLogTap::CAPACITY);
    assert_eq!(lines.first().map(String::as_str), Some("line 8"));
    assert_eq!(lines.last().map(String::as_str), Some("line 39"));
    assert!(!tap.has_identity_verdict());
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn a_termination_request_ends_a_process_and_the_wait_notices() {
    let mut child = std::process::Command::new("sleep")
      .arg("30")
      .spawn()
      .expect("sleep spawns");
    let pid = child.id();
    assert!(!wait_for_exit(pid, Duration::from_millis(200)).await);
    terminate_process(pid);
    let _ = child.wait();
    assert!(wait_for_exit(pid, Duration::from_secs(5)).await);
  }

  #[test]
  fn identity_api_switch_follows_the_chromium_major() {
    // The profile version is the full Chromium version string.
    assert!(!supports_identity_api("150.0.7801.12"));
    assert!(supports_identity_api("151.0.7922.71"));
    assert!(supports_identity_api("152.0.1.0"));
    // A version we cannot parse must not be assumed to have the identity API:
    // calling createIdentity on a 150 binary is an unknown-command error,
    // whereas the legacy pair still exists on every version that ever shipped.
    assert!(!supports_identity_api(""));
    assert!(!supports_identity_api("not a version"));
    assert!(!supports_wayfern_152("151.0.7922.76"));
    assert!(supports_wayfern_152("152.0.7977.64"));
    assert!(!supports_wayfern_152("garbage"));
  }

  #[test]
  fn overrides_are_only_what_differs_from_the_derived_view() {
    let baseline = obj(r#"{"hardwareConcurrency": 8, "deviceMemory": 8, "platform": "Win32"}"#);
    let current = obj(r#"{"hardwareConcurrency": 16, "deviceMemory": 8, "platform": "Win32"}"#);

    let overrides = WayfernManager::identity_overrides(&current, &baseline);
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides.get("hardwareConcurrency"), Some(&json!(16)));
  }

  #[test]
  fn overrides_exclude_geolocation_and_include_added_keys() {
    let baseline = obj(r#"{"timezone": "America/New_York", "platform": "Win32"}"#);
    // Location is rewritten by donut on every geolocation refresh, and
    // setIdentity takes it as its own parameter, so it must never be sent twice.
    let current = obj(
      r#"{"timezone": "Europe/Berlin", "language": "de-DE", "platform": "Win32", "doNotTrack": "1"}"#,
    );

    let overrides = WayfernManager::identity_overrides(&current, &baseline);
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides.get("doNotTrack"), Some(&json!("1")));

    let geo = WayfernManager::geo_params(&current);
    assert_eq!(geo.get("timezone"), Some(&json!("Europe/Berlin")));
    assert_eq!(geo.get("language"), Some(&json!("de-DE")));
    assert!(geo.get("platform").is_none());
  }

  #[test]
  fn migration_moves_a_stored_payload_into_overrides_and_location() {
    let mut config = WayfernConfig {
      identity_id: Some("id-1".to_string()),
      identity_baseline: Some(r#"{"hardwareConcurrency": 8, "platform": "Win32"}"#.to_string()),
      fingerprint: Some(
        r#"{"hardwareConcurrency": 16, "platform": "Win32", "timezone": "Europe/Berlin"}"#
          .to_string(),
      ),
      ..Default::default()
    };

    assert!(WayfernManager::migrate_identity_config(&mut config));
    assert!(config.fingerprint.is_none());
    assert!(config.identity_baseline.is_none());

    let overrides = obj(config.identity_overrides.as_deref().unwrap());
    assert_eq!(overrides.get("hardwareConcurrency"), Some(&json!(16)));
    assert!(overrides.get("platform").is_none());
    // Location is the one piece of device state a migrated profile keeps: it
    // follows the exit, not the identity.
    let location = obj(config.location.as_deref().unwrap());
    assert_eq!(location.get("timezone"), Some(&json!("Europe/Berlin")));

    // Running again must change nothing, because a profile is migrated on
    // whichever launch reaches it first and every later launch repeats it.
    let after_first = serde_json::to_string(&config).unwrap();
    assert!(!WayfernManager::migrate_identity_config(&mut config));
    assert_eq!(serde_json::to_string(&config).unwrap(), after_first);
  }

  #[test]
  fn migration_is_a_no_op_for_an_already_identity_only_profile() {
    let mut config = WayfernConfig {
      identity_id: Some("id-1".to_string()),
      identity_overrides: Some(r#"{"doNotTrack":"1"}"#.to_string()),
      location: Some(r#"{"timezone":"Europe/Berlin"}"#.to_string()),
      ..Default::default()
    };

    assert!(!WayfernManager::migrate_identity_config(&mut config));
    assert!(config.fingerprint.is_none());
    assert_eq!(
      config.identity_overrides.as_deref(),
      Some(r#"{"doNotTrack":"1"}"#)
    );
    assert_eq!(
      config.location.as_deref(),
      Some(r#"{"timezone":"Europe/Berlin"}"#)
    );
  }

  #[test]
  fn migration_leaves_a_payload_only_profile_for_the_launch_path() {
    // A legacy profile has no identity for its edits to sit on, and only the
    // browser can mint one. The payload stays until the launch path replaces
    // it with a fresh identity, so the profile is never left with neither.
    let mut config = WayfernConfig {
      fingerprint: Some(r#"{"platform":"Win32"}"#.to_string()),
      ..Default::default()
    };

    assert!(!WayfernManager::migrate_identity_config(&mut config));
    assert_eq!(
      config.fingerprint.as_deref(),
      Some(r#"{"platform":"Win32"}"#)
    );
    assert!(config.identity_id.is_none());
    assert!(config.identity_overrides.is_none());
  }

  #[test]
  fn migration_has_nothing_to_do_for_a_config_with_neither() {
    let mut config = WayfernConfig::default();

    assert!(!WayfernManager::migrate_identity_config(&mut config));
    assert!(config.fingerprint.is_none());
    assert!(config.identity_id.is_none());
    assert!(config.identity_overrides.is_none());
    assert!(config.location.is_none());
  }

  #[test]
  fn migration_clears_a_baseline_left_behind_without_a_payload() {
    let mut config = WayfernConfig {
      identity_id: Some("id-1".to_string()),
      identity_baseline: Some(r#"{"platform":"Win32"}"#.to_string()),
      ..Default::default()
    };

    assert!(WayfernManager::migrate_identity_config(&mut config));
    assert!(config.identity_baseline.is_none());
    assert!(!WayfernManager::migrate_identity_config(&mut config));
  }

  #[test]
  fn a_migrated_config_writes_no_device_to_disk() {
    let mut config = WayfernConfig {
      identity_id: Some("id-1".to_string()),
      fingerprint: Some(r#"{"platform":"Win32","timezone":"Europe/Berlin"}"#.to_string()),
      ..Default::default()
    };

    assert!(WayfernManager::migrate_identity_config(&mut config));
    let written = serde_json::to_string(&config).unwrap();
    assert!(!written.contains("\"fingerprint\""));
    assert!(!written.contains("\"identity_baseline\""));
    assert!(written.contains("\"location\""));
  }

  #[test]
  fn overrides_drop_the_geo_fields_donut_synthesised_itself() {
    // The baseline is snapshotted BEFORE geolocation runs, so it never carries
    // these two. Without the filter they diff into the override set on the
    // first launch and stay pinned there for the life of the profile, which
    // permanently replaces the browser's realistic ladder with donut's.
    let baseline = obj(
      r#"{"platform": "Win32", "timezoneOffset": 0,
          "languages": ["de-DE", "de", "en-US", "en"]}"#,
    );
    // Built through the same helper the writer uses so the fixture cannot go
    // stale when Berlin changes its DST offset.
    let offset = WayfernManager::timezone_offset_minutes("Europe/Berlin").expect("known zone");
    let mut current = obj(
      r#"{"platform": "Win32", "timezone": "Europe/Berlin",
          "language": "de-DE", "languages": ["de-DE", "de"]}"#,
    );
    current.insert("timezoneOffset".to_string(), json!(offset));

    let overrides = WayfernManager::identity_overrides(&current, &baseline);
    assert!(overrides.is_empty(), "unexpected overrides: {overrides:?}");
  }

  #[test]
  fn a_user_edited_language_ladder_still_travels_as_an_override() {
    // Anything that is NOT what donut would have written is a real edit, so no
    // editing capability is lost by the filter above.
    let baseline = obj(r#"{"platform": "Win32", "languages": ["de-DE", "de"]}"#);
    let current = obj(
      r#"{"platform": "Win32", "timezone": "Europe/Berlin", "language": "de-DE",
          "languages": ["de-DE", "de", "en-US", "en"]}"#,
    );

    let overrides = WayfernManager::identity_overrides(&current, &baseline);
    assert_eq!(
      overrides.get("languages"),
      Some(&json!(["de-DE", "de", "en-US", "en"]))
    );
  }

  #[test]
  fn derived_provenance_never_travels_as_an_override() {
    // setIdentity rejects the whole call when one of these appears, so a stored
    // payload carrying them would fail on every launch rather than once.
    let baseline = obj(r#"{"platform": "Win32"}"#);
    let current = obj(
      r#"{"platform": "Win32", "webglProfileId": "webgl-abc",
          "mediaProfile": "media-abc", "deviceProfileApplied": true,
          "doNotTrack": "1"}"#,
    );

    let overrides = WayfernManager::identity_overrides(&current, &baseline);
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides.get("doNotTrack"), Some(&json!("1")));
  }

  #[test]
  fn the_claimed_os_comes_from_the_config_then_the_stored_platform() {
    let stored = obj(r#"{"platform": "MacIntel"}"#);

    // An explicit claim wins.
    let config = WayfernConfig {
      os: Some("android".to_string()),
      ..Default::default()
    };
    assert_eq!(
      WayfernManager::claimed_operating_system(&config, Some(&stored)),
      Some("android")
    );

    // Profiles minted before `os` existed fall back to the stored device.
    let config = WayfernConfig::default();
    assert_eq!(
      WayfernManager::claimed_operating_system(&config, Some(&stored)),
      Some("macos")
    );

    // Nothing to read, and an unrecognised claim, both stay unknown so the
    // launch is never refused on a guess.
    assert_eq!(
      WayfernManager::claimed_operating_system(&WayfernConfig::default(), None),
      None
    );
    let config = WayfernConfig {
      os: Some("freebsd".to_string()),
      ..Default::default()
    };
    assert_eq!(
      WayfernManager::claimed_operating_system(&config, None),
      None
    );
  }

  #[test]
  fn platform_strings_map_the_way_the_browser_maps_them() {
    // Mirrors the browser's own platform-to-OS mapping, including armv8l and
    // aarch64 reading as android rather than linux.
    assert_eq!(WayfernManager::os_from_platform("Win32"), Some("windows"));
    assert_eq!(WayfernManager::os_from_platform("MacIntel"), Some("macos"));
    assert_eq!(WayfernManager::os_from_platform("iPhone"), Some("ios"));
    assert_eq!(
      WayfernManager::os_from_platform("Linux armv8l"),
      Some("android")
    );
    assert_eq!(
      WayfernManager::os_from_platform("Linux aarch64"),
      Some("android")
    );
    assert_eq!(
      WayfernManager::os_from_platform("Linux x86_64"),
      Some("linux")
    );
    assert_eq!(WayfernManager::os_from_platform("Nintendo"), None);
  }

  #[test]
  fn a_refused_apply_is_translated_to_a_code_the_frontend_knows() {
    // The exact literals the browser emits.
    let cross_os = WayfernManager::apply_failure_error(
      "CDP error: Cross-OS fingerprinting requires a paid plan. Provide a wayfernToken parameter.",
      Some("macos"),
    );
    assert!(cross_os.contains("WAYFERN_CROSS_OS_REQUIRES_PLAN"));
    assert!(cross_os.contains("macos"));

    let quota = WayfernManager::apply_failure_error(
      "CDP error: Fingerprint generation limit reached for this account.",
      None,
    );
    assert!(quota.contains("WAYFERN_GENERATION_LIMIT_REACHED"));

    // Anything else keeps the raw text for support, under the generic code.
    let other = WayfernManager::apply_failure_error("CDP error: No response received", None);
    assert!(other.contains("WAYFERN_FINGERPRINT_APPLY_FAILED"));
    assert!(other.contains("No response received"));
  }

  #[test]
  fn the_launch_payload_never_invents_a_location() {
    // The regression, and the whole reason the gate is allowed to say "nothing
    // was compared": the launcher used to insert `America/New_York` and offset
    // 300 into any fingerprint that declared no timezone. The browser then
    // presented a US clock behind whatever exit the profile routed through,
    // while the app told the user the timezone had never been compared, the
    // exact mismatch `fingerprint_consistency` exists to surface, manufactured
    // by the launcher and then hidden by it.
    for stored in [
      r#"{"platform": "Win32"}"#,
      // The legacy wrapper takes the same path.
      r#"{"fingerprint": {"platform": "Win32"}}"#,
    ] {
      let payload =
        WayfernManager::launch_fingerprint_payload(stored).expect("a stored fingerprint parses");
      let obj = payload.as_object().expect("stays an object");
      for invented in ["timezone", "timezoneOffset", "latitude", "longitude"] {
        assert!(
          !obj.contains_key(invented),
          "the launch payload invented {invented} for {stored}: {obj:?}"
        );
      }
      assert_eq!(obj.get("platform"), Some(&json!("Win32")));
    }
  }

  #[test]
  fn the_launch_payload_unwraps_the_legacy_shape_and_keeps_a_declared_location() {
    let payload = WayfernManager::launch_fingerprint_payload(
      r#"{"fingerprint": {"timezone": "Europe/Berlin", "timezoneOffset": -60,
                          "languages": "de-DE, de"}}"#,
    )
    .expect("the legacy wrapper parses");

    assert_eq!(payload["timezone"], json!("Europe/Berlin"));
    assert_eq!(payload["timezoneOffset"], json!(-60));
    // A comma-separated ladder still becomes the array the browser expects.
    assert_eq!(payload["languages"], json!(["de-DE", "de"]));
    // The wrapper itself must never reach the browser.
    assert!(payload.get("fingerprint").is_none());
  }

  #[test]
  fn the_launch_payload_refuses_unparsable_json() {
    assert!(WayfernManager::launch_fingerprint_payload("not json").is_err());
  }

  #[test]
  fn window_size_prefers_outer_window_dimensions() {
    // Field names + values mirror a real Wayfern fingerprint (camelCase).
    let fp = r#"{"windowOuterWidth": 1268, "windowOuterHeight": 764,
                 "windowInnerWidth": 1253, "windowInnerHeight": 630,
                 "screenAvailWidth": 1280, "screenAvailHeight": 775,
                 "screenWidth": 1280, "screenHeight": 800}"#;
    assert_eq!(
      WayfernManager::window_size_from_fingerprint(fp),
      Some((1268, 764))
    );
  }

  #[test]
  fn window_size_falls_back_to_avail_then_full_screen() {
    let avail = r#"{"screenAvailWidth": 1280, "screenAvailHeight": 775,
                    "screenWidth": 1280, "screenHeight": 800}"#;
    assert_eq!(
      WayfernManager::window_size_from_fingerprint(avail),
      Some((1280, 775))
    );

    let full = r#"{"screenWidth": 2560, "screenHeight": 1440}"#;
    assert_eq!(
      WayfernManager::window_size_from_fingerprint(full),
      Some((2560, 1440))
    );
  }

  #[test]
  fn window_size_handles_wrapper_and_stringified_numbers() {
    let wrapped = r#"{"fingerprint": {"windowOuterWidth": "1366", "windowOuterHeight": "768"}}"#;
    assert_eq!(
      WayfernManager::window_size_from_fingerprint(wrapped),
      Some((1366, 768))
    );
  }

  #[test]
  fn window_size_none_when_missing_or_invalid() {
    // No dimensions at all.
    assert_eq!(
      WayfernManager::window_size_from_fingerprint(r#"{"userAgent": "x"}"#),
      None
    );
    // A width with no matching height is not a usable pair.
    assert_eq!(
      WayfernManager::window_size_from_fingerprint(r#"{"windowOuterWidth": 1268}"#),
      None
    );
    // Zero is rejected as a degenerate size.
    assert_eq!(
      WayfernManager::window_size_from_fingerprint(
        r#"{"windowOuterWidth": 0, "windowOuterHeight": 0}"#
      ),
      None
    );
    // Not valid JSON.
    assert_eq!(
      WayfernManager::window_size_from_fingerprint("not json"),
      None
    );
  }
}

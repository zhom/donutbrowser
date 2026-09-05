use crate::browser_runner::BrowserRunner;
use crate::profile::BrowserProfile;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex as AsyncMutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WayfernConfig {
  #[serde(default)]
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
  #[serde(default)]
  pub block_webrtc: Option<bool>,
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
  /// stored payload. Cleared by the migration; never written again.
  #[serde(default)]
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
/// exact build if the identity commands land part-way through the 151 line;
/// missing components compare as zero, so `"151"` means "any 151 or newer".
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
  /// The fingerprint the browser echoed back after applying it. It may differ
  /// from what was sent, so it is this value that gets persisted. Internal
  /// only — never sent to the frontend.
  #[serde(default, skip_serializing)]
  pub used_fingerprint: Option<String>,
  /// The refreshed baseline to persist alongside `used_fingerprint`. Keeping
  /// it in step is what stops an unedited field from being mistaken for a user
  /// edit on the next launch. Internal only.
  #[serde(default, skip_serializing)]
  pub used_identity_baseline: Option<String>,
}

struct WayfernInstance {
  id: String,
  process_id: Option<u32>,
  profile_path: Option<String>,
  url: Option<String>,
  cdp_port: Option<u16>,
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
    let parsed: serde_json::Value = serde_json::from_str(fingerprint_json).ok()?;
    let fp = parsed.get("fingerprint").unwrap_or(&parsed);
    let obj = fp.as_object()?;

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

  /// Parse a stored fingerprint JSON into its object, tolerating the legacy
  /// `{ "fingerprint": {...} }` wrapper some old profiles carry.
  pub fn fingerprint_object(
    fingerprint_json: &str,
  ) -> Option<serde_json::Map<String, serde_json::Value>> {
    let parsed: serde_json::Value = serde_json::from_str(fingerprint_json).ok()?;
    let fp = parsed.get("fingerprint").unwrap_or(&parsed);
    fp.as_object().cloned()
  }

  /// A stored JSON object field (`identity_overrides`, `location`), or an
  /// empty map when absent or unparsable.
  pub fn stored_object(json: Option<&str>) -> serde_json::Map<String, serde_json::Value> {
    json.and_then(Self::fingerprint_object).unwrap_or_default()
  }

  /// The exit-derived location fields a device object carries, in the shape
  /// `WayfernConfig::location` stores; `None` when it carries none.
  pub fn location_of(
    device: &serde_json::Map<String, serde_json::Value>,
  ) -> Option<String> {
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

  /// Fill in any location field the applied device does not already carry.
  ///
  /// `setIdentity` takes the location as its own parameters rather than inside
  /// the identity, so the view it echoes back may omit part of it — and donut's
  /// stored fingerprint must always carry the whole block, because the launch
  /// gate reads it before any browser is running and a stored device with no
  /// timezone turns the exit-vs-fingerprint check into a no-op.
  ///
  /// Only ABSENT fields are filled. Anything the browser did send back is its
  /// own and is kept: it re-roots the `languages` ladder onto the exit's
  /// language, which is a better answer than the two-entry list donut computes.
  fn carry_over_locale(
    from: &serde_json::Map<String, serde_json::Value>,
    into: &mut serde_json::Value,
  ) {
    let Some(target) = into.as_object_mut() else {
      return;
    };
    for key in LOCALE_CARRY_OVER_KEYS {
      if target.get(key).is_some_and(|v| !v.is_null()) {
        continue;
      }
      if let Some(value) = from.get(key) {
        target.insert(key.to_string(), value.clone());
      }
    }
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
  /// Mirrors `WayfernHandler::IsCrossOSFromPlatform`, including the order of
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
  /// is the only channel the browser has. The literals are the ones
  /// `WayfernHandler` emits from its cross-OS gate and its quota branch; if one
  /// is ever reworded this degrades to the generic code, which still carries
  /// the raw text for support, rather than breaking.
  fn apply_failure_error(detail: &str, claimed_os: Option<&str>) -> String {
    if detail.contains("Cross-OS fingerprinting requires") {
      return crate::backend_error_with_detail(
        "WAYFERN_CROSS_OS_REQUIRES_PLAN",
        claimed_os.unwrap_or("another operating system"),
      );
    }
    // BOTH refusal texts - this maps failures from either release. 151 emits
    // "Fingerprint generation limit reached for this account."; the shipped 150
    // browser emits "Too many profiles are being created." A 150 user would
    // otherwise fall through to the generic apply-failed message and lose the
    // one piece of information that makes the failure actionable.
    if detail.contains("generation limit reached") || detail.contains("Too many profiles") {
      return crate::backend_error("WAYFERN_GENERATION_LIMIT_REACHED");
    }
    crate::backend_error_with_detail("WAYFERN_FINGERPRINT_APPLY_FAILED", detail)
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
        log::warn!("Geolocation failed, using defaults: {e}");
        if let Some(obj) = fingerprint.as_object_mut() {
          if !obj.contains_key("timezone") {
            obj.insert("timezone".to_string(), json!("America/New_York"));
          }
          if !obj.contains_key("timezoneOffset") {
            obj.insert("timezoneOffset".to_string(), json!(300));
          }
        }
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
  fn is_remote_socks_url(url: &str) -> bool {
    url.starts_with("socks")
      && url::Url::parse(url)
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

        if !is_running {
          // Process exited — try to read its stderr
          String::from("(process exited before CDP became ready)")
        } else {
          String::from("(process still running but not responding on CDP)")
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
    // location itself, below, through the profile's own proxy — the browser's
    // C++ geo service cannot, because an authenticated upstream answers its
    // SimpleURLLoader requests with HTTP 407.
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

        // reqwest's SOCKS connector (hyper-util) corrupts its parse buffer
        // when a proxy splits a handshake reply across TCP segments, so a
        // socks upstream here can fail even though the proxy is healthy.
        // Route the geolocation lookup through a temporary local donut-proxy
        // worker — the same path the browser itself uses — and fall back to
        // the upstream URL only if the worker can't start. Two exclusions:
        // no worker when geolocation won't fetch through the proxy at all
        // (disabled, or a fixed geoip IP), and none for loopback socks URLs —
        // launch-time callers pass the already-running local worker's
        // socks5://127.0.0.1 URL, whose single-segment replies don't trigger
        // the bug, so chaining a second worker would only add latency.
        let needs_proxied_geo_fetch = !matches!(
          config.geoip.as_ref(),
          Some(serde_json::Value::Bool(false)) | Some(serde_json::Value::String(_))
        );
        let remote_socks_upstream = config
          .proxy
          .as_deref()
          .filter(|url| Self::is_remote_socks_url(url));
        let (geo_proxy, temp_worker_id) = match remote_socks_upstream {
          Some(url) if needs_proxied_geo_fetch => {
            match crate::proxy_runner::start_proxy_process(Some(url.to_string()), None)
              .await
              .map_err(|e| e.to_string())
            {
              Ok(worker) => {
                let local_url = format!("http://127.0.0.1:{}", worker.local_port.unwrap_or(0));
                (Some(local_url), Some(worker.id))
              }
              Err(e) => {
                log::warn!(
                  "Could not start local proxy worker for geolocation ({e}); using the socks upstream directly"
                );
                (config.proxy.clone(), None)
              }
            }
          }
          _ => (config.proxy.clone(), None),
        };

        // Apply timezone/geolocation for the proxy this fingerprint is being
        // generated against. Shared with the launch-time location refresh.
        let geolocation_applied =
          Self::apply_geolocation(&mut normalized, geo_proxy.as_deref(), config.geoip.as_ref())
            .await;

        if let Some(worker_id) = temp_worker_id {
          let _ = crate::proxy_runner::stop_proxy_process(&worker_id).await;
        }

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
      "--disable-session-crashed-bubble".to_string(),
      "--hide-crash-restore-bubble".to_string(),
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
    // windows are easy to tell apart. Wayfern reads these in
    // BrowserView::GetWindowTitle() (label) and BrowserFrameView::GetFrameColor()
    // (color). The label is the profile name; the color is the user's
    // window_color when set, otherwise deterministically derived from the
    // profile id so every profile still gets a stable, distinct color.
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
    // setIdentity/setFingerprint. The browser's gate does not consult the
    // WAYFERN_TOKEN env var this launch also sets, so with no token in hand the
    // apply is refused and the window would sit there running the HOST device
    // under a macOS or Android profile. Refuse before spawning rather than
    // opening a window we are about to kill.
    //
    // "Cross-OS" is the browser's own test (`WayfernHandler::IsCrossOS` against
    // `GetHostOperatingSystem`), so `android` and `ios` count on every desktop.
    //
    // Not a 151 regression: setFingerprint on 150 has the identical
    // parameter-only gate. What changed is that the refusal is no longer
    // swallowed as a log line (see the apply loop below).
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

    let mut command = TokioCommand::new(&executable_path);
    command
      .args(&args)
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null());
    if let Some(ref token) = wayfern_token {
      command.env("WAYFERN_TOKEN", token);
      log::info!("Wayfern authorization configured for browser process");
    }

    let child = command
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
    drop(child);

    self.wait_for_cdp_ready(port).await?;

    let targets = self.get_cdp_targets(port).await?;
    log::info!("Found {} CDP targets", targets.len());

    let page_targets: Vec<_> = targets.iter().filter(|t| t.target_type == "page").collect();
    log::info!("Found {} page targets", page_targets.len());

    // Apply fingerprint if configured
    let mut used_fingerprint: Option<String> = None;
    // Always None: nothing writes a baseline any more. The field stays on the
    // result for the one launch that still migrates a legacy identity profile.
    let used_identity_baseline: Option<String> = None;
    // An identity-backed profile: the id, the user's overrides and the exit's
    // location are all the browser needs, and all the profile stores. The
    // device comes back in the response and is deliberately NOT persisted.
    let identity_only = supports_identity_api(&profile.version)
      && config.identity_id.is_some()
      && config.fingerprint.is_none();
    if identity_only {
      let identity_id = config.identity_id.clone().unwrap_or_default();
      let overrides = Self::stored_object(config.identity_overrides.as_deref());
      let location = Self::stored_object(config.location.as_deref());
      let wayfern_token = crate::cloud_auth::CLOUD_AUTH.get_wayfern_token().await;

      let mut params = serde_json::Map::new();
      params.insert("identityId".to_string(), json!(identity_id));
      // The claimed OS travels explicitly as well as inside the id. A Wayfern
      // 152 id carries an epoch and a 16-bit check that a 151 browser's decoder
      // does not know; without this parameter 151 would read such an id as
      // untagged and rebuild the HOST OS. Both releases let the explicit
      // parameter win, so this keeps one stored profile portable across them.
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

      let stored_value: serde_json::Value = serde_json::from_str(fingerprint_json)
        .map_err(|e| format!("Failed to parse stored fingerprint JSON: {e}"))?;

      // The stored fingerprint should be the fingerprint object directly (after our fix in generate_fingerprint_config)
      // But for backwards compatibility, also handle the wrapped format
      let mut fingerprint = if stored_value.get("fingerprint").is_some() {
        // Old format: {"fingerprint": {...}} - extract the inner fingerprint
        stored_value.get("fingerprint").cloned().unwrap()
      } else {
        // New format: fingerprint object directly {...}
        stored_value.clone()
      };

      // Add default timezone if not present (for profiles created before timezone was added)
      if let Some(obj) = fingerprint.as_object_mut() {
        if !obj.contains_key("timezone") {
          obj.insert("timezone".to_string(), json!("America/New_York"));
          log::info!("Added default timezone to fingerprint");
        }
        if !obj.contains_key("timezoneOffset") {
          obj.insert("timezoneOffset".to_string(), json!(300));
          log::info!("Added default timezoneOffset to fingerprint");
        }
      }

      // Denormalize fingerprint for Wayfern CDP (convert arrays/objects to JSON strings)
      let mut fingerprint_for_cdp = Self::denormalize_fingerprint(fingerprint);

      // Normalize languages: if it's a comma-separated string, convert to array
      if let Some(obj) = fingerprint_for_cdp.as_object_mut() {
        if let Some(serde_json::Value::String(s)) = obj.get("languages").cloned() {
          let arr: Vec<&str> = s.split(',').map(|l| l.trim()).collect();
          obj.insert("languages".to_string(), json!(arr));
        }
      }

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

      // The device as donut holds it: the diff source for the overrides below,
      // and the fallback for any location field the echo does not return.
      let stored = fingerprint_for_cdp.as_object().cloned().unwrap_or_default();

      // Which command applies this profile's device. It is a property of the
      // PROFILE, not of the browser version, so a profile that stores a whole
      // payload keeps being applied with the payload command.
      //
      // `webglProfileId` is the discriminator: only a whole-payload profile
      // carries it, and the browser refuses it as an override. Sending it would
      // fail the call on every launch.
      let apply_by_identity = supports_identity_api(&profile.version)
        && config.identity_id.is_some()
        && stored.get("webglProfileId").is_none();

      // On the identity path only the user's own edits are sent; everything
      // else comes from the identity itself.
      let (apply_method, apply_params, _previous_baseline, overrides) =
        match config.identity_id.as_deref().filter(|_| apply_by_identity) {
          Some(identity_id) => {
            let previous_baseline = config
              .identity_baseline
              .as_deref()
              .and_then(Self::fingerprint_object)
              .unwrap_or_default();
            let overrides = Self::identity_overrides(&stored, &previous_baseline);

            let mut params = serde_json::Map::new();
            params.insert("identityId".to_string(), json!(identity_id));
            if !overrides.is_empty() {
              params.insert(
                "overrides".to_string(),
                serde_json::Value::Object(overrides.clone()),
              );
            }
            // Location is a property of the exit, not of the identity, so it
            // travels in setIdentity's own parameters rather than as an override.
            params.extend(Self::geo_params(&stored));
            if let Some(ref token) = wayfern_token {
              params.insert("wayfernToken".to_string(), json!(token));
            }

            log::info!(
              "Applying Wayfern identity {} with {} override(s): {:?}",
              identity_id,
              overrides.len(),
              overrides.keys().collect::<Vec<_>>()
            );

            (
              "Wayfern.setIdentity",
              serde_json::Value::Object(params),
              previous_baseline,
              overrides,
            )
          }
          None => {
            let mut params = fingerprint_for_cdp.clone();
            if let Some(ref token) = wayfern_token {
              if let Some(obj) = params.as_object_mut() {
                obj.insert("wayfernToken".to_string(), json!(token));
              }
            }
            (
              "Wayfern.setFingerprint",
              params,
              serde_json::Map::new(),
              serde_json::Map::new(),
            )
          }
        };

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
            .send_cdp_command(ws_url, apply_method, apply_params.clone())
            .await
          {
            Ok(result) => {
              // The device is on the target. Whether the ECHO parses is a
              // separate question — it only decides what we persist.
              applied_ok = true;
              log::info!("Successfully applied fingerprint to page target");
              // Both commands echo back the device the browser actually used,
              // which may differ from what we sent. Capture it once, from the
              // first target that succeeds, so the caller can persist it.
              if used_fingerprint.is_none() {
                // setIdentity wraps the object as { identity: {...} },
                // setFingerprint as { fingerprint: {...} }; tolerate a bare
                // object too.
                let applied = result
                  .get("identity")
                  .or_else(|| result.get("fingerprint"))
                  .cloned()
                  .unwrap_or(result);
                if let Some(applied_obj) = applied.as_object() {
                  let mut persisted = applied;
                  if apply_by_identity {
                    // The location travelled as setIdentity parameters rather
                    // than inside the identity, so make sure it survives into
                    // what we store. The launch gate and the pre-launch window
                    // sizing both read the stored fingerprint before any
                    // browser is running, and a stored device with no timezone
                    // silently turns the exit-vs-fingerprint check into a no-op.
                    Self::carry_over_locale(&stored, &mut persisted);
                  }
                  match serde_json::to_string(&Self::normalize_fingerprint(persisted)) {
                    Ok(s) => used_fingerprint = Some(s),
                    Err(e) => {
                      log::warn!("Failed to serialize used fingerprint: {e}")
                    }
                  }
                }
              }
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
    };

    let mut inner = self.inner.lock().await;
    inner.instances.insert(id.clone(), instance);

    Ok(WayfernLaunchResult {
      id,
      processId: process_id,
      profilePath: Some(profile_path.to_string()),
      url: url.map(|s| s.to_string()),
      cdp_port: Some(port),
      used_fingerprint,
      used_identity_baseline,
    })
  }

  pub async fn stop_wayfern(
    &self,
    id: &str,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut inner = self.inner.lock().await;

    if let Some(instance) = inner.instances.remove(id) {
      log::info!("Cleaning up Wayfern instance {}", instance.id);
      if let Some(pid) = instance.process_id {
        kill_browser_process(pid);
        log::info!("Stopped Wayfern instance {id} (PID: {pid})");
      }
    }

    Ok(())
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

    // Open the URL in a new tab via the CDP HTTP convenience endpoint.
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
              used_fingerprint: None,
              used_identity_baseline: None,
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
        },
      );

      return Some(WayfernLaunchResult {
        id: instance_id,
        processId: Some(pid),
        profilePath: Some(found_profile_path),
        url: None,
        cdp_port,
        used_fingerprint: None,
        used_identity_baseline: None,
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
      .args(["/PID", &pid.to_string(), "/F"])
      .creation_flags(CREATE_NO_WINDOW)
      .output();
  }
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
    assert!(WayfernManager::is_remote_socks_url(
      "socks5://user:pass@gw.dataimpulse.com:10000"
    ));
    assert!(WayfernManager::is_remote_socks_url("socks5://1.2.3.4:1080"));
    assert!(WayfernManager::is_remote_socks_url("socks4://1.2.3.4:1080"));

    // ...but the app's own loopback workers are not. socks is a non-special
    // URL scheme, so the IP literal parses as Host::Domain — the launch-time
    // randomize path depends on this returning false.
    assert!(!WayfernManager::is_remote_socks_url(
      "socks5://127.0.0.1:24001"
    ));
    assert!(!WayfernManager::is_remote_socks_url("socks5://[::1]:24001"));
    assert!(!WayfernManager::is_remote_socks_url(
      "socks5://localhost:24001"
    ));

    // Non-socks schemes and unparsable URLs never need the workaround.
    assert!(!WayfernManager::is_remote_socks_url(
      "http://gw.dataimpulse.com:10000"
    ));
    assert!(!WayfernManager::is_remote_socks_url(
      "https://gw.dataimpulse.com:10000"
    ));
    assert!(!WayfernManager::is_remote_socks_url("socks5://"));
    assert!(!WayfernManager::is_remote_socks_url("not a url"));
  }

  fn obj(json: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str(json).expect("test fixture must be an object")
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
  fn the_launch_echo_only_fills_location_the_browser_left_out() {
    // setIdentity carries the location in its own parameters, so the applied
    // view may not echo all of it back. The stored fingerprint has to keep it:
    // the launch gate reads `timezone` before any browser is running, and a
    // stored device without one turns that check into a no-op.
    let stored = obj(
      r#"{"timezone": "Europe/Berlin", "timezoneOffset": -60,
                         "language": "de-DE", "languages": ["de-DE", "de"]}"#,
    );
    // The browser returned its own, richer `languages` ladder and dropped the
    // rest.
    let mut applied = json!({"languages": ["de-DE", "de", "en-US", "en"]});

    WayfernManager::carry_over_locale(&stored, &mut applied);
    let applied = applied.as_object().unwrap();

    assert_eq!(applied.get("timezone"), Some(&json!("Europe/Berlin")));
    assert_eq!(applied.get("timezoneOffset"), Some(&json!(-60)));
    assert_eq!(applied.get("language"), Some(&json!("de-DE")));
    // What the browser DID return wins: it re-roots the ladder onto the exit's
    // language, which is a better answer than the two-entry list donut builds.
    assert_eq!(
      applied.get("languages"),
      Some(&json!(["de-DE", "de", "en-US", "en"]))
    );
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
    // Mirrors WayfernHandler::IsCrossOSFromPlatform, including armv8l/aarch64
    // reading as android rather than linux.
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
    // The exact literals WayfernHandler emits.
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

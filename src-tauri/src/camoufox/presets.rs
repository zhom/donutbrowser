//! Real-fingerprint preset support for Camoufox.
//!
//! Mirrors the preset-selection logic from
//! `pythonlib/camoufox/fingerprints.py` (`_select_presets_file`,
//! `load_presets`, `get_random_preset`, `from_preset`).
//!
//! Camoufox ships two bundled preset files:
//! - `fingerprint-presets-v135.json` — real fingerprints harvested from
//!   browsers running Firefox ≤148. The frozen "v135 line" — kept around
//!   so users who haven't upgraded their Camoufox binary keep getting
//!   consistent fingerprints.
//! - `fingerprint-presets-v150.json` — the *newer* bundle, refreshed by
//!   upstream as Camoufox tracks newer Firefox versions. This is the
//!   bundle every newer Camoufox release uses; we make no assumption that
//!   Firefox 150 is the ceiling.
//!
//! At launch we know the bundled Firefox version (see
//! `config::get_firefox_version`) and pick `v135` or `newer` accordingly.
//! The split point lives in `data::PRESETS_NEWER_MIN_FF` (currently 149)
//! and is the only number we hard-code — anything ≥ that gets the newer
//! bundle, regardless of how far Firefox itself has moved on.
//!
//! Falling back to Bayesian-network synthesis (the previous default) is
//! still possible when no preset matches the requested OS.

use rand::prelude::IndexedRandom;
use regex_lite::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::camoufox::data;

#[derive(Debug, Clone, Deserialize)]
pub struct Navigator {
  #[serde(rename = "userAgent")]
  pub user_agent: Option<String>,
  pub platform: Option<String>,
  #[serde(rename = "hardwareConcurrency")]
  pub hardware_concurrency: Option<u32>,
  #[serde(rename = "maxTouchPoints")]
  pub max_touch_points: Option<u32>,
  pub oscpu: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Screen {
  pub width: Option<u32>,
  pub height: Option<u32>,
  #[serde(rename = "colorDepth")]
  pub color_depth: Option<u32>,
  #[serde(rename = "availWidth")]
  pub avail_width: Option<u32>,
  #[serde(rename = "availHeight")]
  pub avail_height: Option<u32>,
  #[serde(rename = "devicePixelRatio")]
  pub device_pixel_ratio: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebGl {
  #[serde(rename = "unmaskedVendor")]
  pub unmasked_vendor: Option<String>,
  #[serde(rename = "unmaskedRenderer")]
  pub unmasked_renderer: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Preset {
  #[serde(default)]
  pub navigator: Option<Navigator>,
  #[serde(default)]
  pub screen: Option<Screen>,
  #[serde(default)]
  pub webgl: Option<WebGl>,
  #[serde(default)]
  pub timezone: Option<String>,
  #[serde(default)]
  pub fonts: Option<Vec<String>>,
  #[serde(rename = "speechVoices", default)]
  pub speech_voices: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PresetBundle {
  /// Bundle schema version — upstream writes this as a JSON integer (e.g.
  /// `1`), so we accept any JSON shape here and ignore it. Only the
  /// `presets` map matters at runtime.
  #[allow(dead_code)]
  #[serde(default)]
  pub version: Option<serde_json::Value>,
  #[serde(default)]
  pub presets: HashMap<String, Vec<Preset>>,
}

/// Which Camoufox release line the active binary belongs to. Determines
/// which preset bundle to load. The set is intentionally just two-valued:
/// the legacy v135 line and "everything newer" — upstream refreshes the
/// newer bundle as Firefox versions advance, but our routing logic stays
/// the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetLine {
  V135,
  Newer,
}

/// Pick the preset line that matches a Firefox major version, mirroring
/// `_select_presets_file` in the Python lib. Unknown / very old versions
/// fall back to the v135 bundle so the older Camoufox builds keep working.
pub fn preset_line_for(ff_version: Option<u32>) -> PresetLine {
  match ff_version {
    Some(v) if v >= data::PRESETS_NEWER_MIN_FF => PresetLine::Newer,
    _ => PresetLine::V135,
  }
}

/// Cache the parsed bundles forever — they're static, embedded data and
/// parsing the newer file twice would waste a few megs of CPU work on
/// every launch.
static V135_BUNDLE: OnceLock<Option<PresetBundle>> = OnceLock::new();
static NEWER_BUNDLE: OnceLock<Option<PresetBundle>> = OnceLock::new();

fn parse_bundle(json: &str) -> Option<PresetBundle> {
  match serde_json::from_str::<PresetBundle>(json) {
    Ok(b) => Some(b),
    Err(e) => {
      log::warn!("camoufox preset bundle failed to parse: {e}");
      None
    }
  }
}

pub fn load_presets(line: PresetLine) -> Option<&'static PresetBundle> {
  let slot = match line {
    PresetLine::V135 => &V135_BUNDLE,
    PresetLine::Newer => &NEWER_BUNDLE,
  };
  slot
    .get_or_init(|| match line {
      PresetLine::V135 => parse_bundle(data::FINGERPRINT_PRESETS_V135_JSON),
      PresetLine::Newer => parse_bundle(data::FINGERPRINT_PRESETS_NEWER_JSON),
    })
    .as_ref()
}

/// Normalize the OS string the rest of the codebase uses ("macos", "windows",
/// "linux") to the preset key. Returns `None` for OSes that don't have any
/// presets bundled.
fn normalize_os(os: &str) -> Option<&'static str> {
  match os {
    "windows" | "win" => Some("windows"),
    "macos" | "mac" | "darwin" => Some("macos"),
    "linux" | "lin" => Some("linux"),
    _ => None,
  }
}

/// Pick a random preset for the requested OS. `None` if there are no
/// presets bundled for that OS (which can happen in tests with reduced
/// fixtures, or if a new OS is added before its preset bundle ships).
pub fn get_random_preset(os: Option<&str>, ff_version: Option<u32>) -> Option<Preset> {
  let bundle = load_presets(preset_line_for(ff_version))?;

  let candidates: Vec<&Preset> = match os.and_then(normalize_os) {
    Some(os_key) => bundle.presets.get(os_key).map(|v| v.iter().collect())?,
    None => bundle.presets.values().flatten().collect(),
  };
  if candidates.is_empty() {
    return None;
  }
  candidates.choose(&mut rand::rng()).map(|p| (*p).clone())
}

/// Match python's `from_preset` — translate a real-fingerprint preset into
/// the CAMOU_CONFIG-style HashMap the rest of the launcher expects.
///
/// The caller is responsible for filling in fonts, voices, and the random
/// seeds; those are intentionally left out here so each call site can layer
/// its own RNG and font policy.
pub fn from_preset(preset: &Preset, ff_version: Option<u32>) -> HashMap<String, serde_json::Value> {
  let mut config: HashMap<String, serde_json::Value> = HashMap::new();

  if let Some(nav) = &preset.navigator {
    if let Some(ua) = &nav.user_agent {
      let ua = if let Some(v) = ff_version {
        rewrite_ua_firefox_version(ua, v)
      } else {
        ua.clone()
      };
      config.insert("navigator.userAgent".to_string(), serde_json::json!(ua));
    }
    if let Some(p) = &nav.platform {
      config.insert("navigator.platform".to_string(), serde_json::json!(p));
    }
    if let Some(hc) = nav.hardware_concurrency {
      config.insert(
        "navigator.hardwareConcurrency".to_string(),
        serde_json::json!(hc),
      );
    }
    if let Some(mtp) = nav.max_touch_points {
      config.insert(
        "navigator.maxTouchPoints".to_string(),
        serde_json::json!(mtp),
      );
    }
    // navigator.oscpu — explicit, or derived from the platform.
    let oscpu = nav.oscpu.clone().or_else(|| {
      nav.platform.as_deref().and_then(|plat| match plat {
        "MacIntel" => Some("Intel Mac OS X 10.15".to_string()),
        "Win32" => Some("Windows NT 10.0; Win64; x64".to_string()),
        p if p.to_ascii_lowercase().contains("linux") => Some("Linux x86_64".to_string()),
        _ => None,
      })
    });
    if let Some(o) = oscpu {
      config.insert("navigator.oscpu".to_string(), serde_json::json!(o));
    }
  }

  if let Some(s) = &preset.screen {
    if let Some(w) = s.width {
      config.insert("screen.width".to_string(), serde_json::json!(w));
    }
    if let Some(h) = s.height {
      config.insert("screen.height".to_string(), serde_json::json!(h));
    }
    if let Some(cd) = s.color_depth {
      config.insert("screen.colorDepth".to_string(), serde_json::json!(cd));
      config.insert("screen.pixelDepth".to_string(), serde_json::json!(cd));
    }
    if let Some(aw) = s.avail_width {
      config.insert("screen.availWidth".to_string(), serde_json::json!(aw));
    }
    if let Some(ah) = s.avail_height {
      config.insert("screen.availHeight".to_string(), serde_json::json!(ah));
    }
  }

  if let Some(w) = &preset.webgl {
    if let Some(v) = &w.unmasked_vendor {
      config.insert("webGl:vendor".to_string(), serde_json::json!(v));
    }
    if let Some(r) = &w.unmasked_renderer {
      config.insert("webGl:renderer".to_string(), serde_json::json!(r));
    }
  }

  if let Some(tz) = &preset.timezone {
    config.insert("timezone".to_string(), serde_json::json!(tz));
  }

  config
}

fn rewrite_ua_firefox_version(ua: &str, version: u32) -> String {
  let firefox_re = Regex::new(r"Firefox/\d+\.0").expect("static regex");
  let rv_re = Regex::new(r"rv:\d+\.0").expect("static regex");
  let first = firefox_re.replace_all(ua, format!("Firefox/{version}.0"));
  rv_re
    .replace_all(&first, format!("rv:{version}.0"))
    .into_owned()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn picks_v135_for_old_firefox() {
    assert_eq!(preset_line_for(Some(135)), PresetLine::V135);
    assert_eq!(preset_line_for(Some(148)), PresetLine::V135);
    assert_eq!(preset_line_for(None), PresetLine::V135);
  }

  #[test]
  fn picks_newer_for_anything_past_the_legacy_line() {
    // The threshold is data::PRESETS_NEWER_MIN_FF (currently 149).
    // Future Firefox versions all share the same bundle — there's
    // intentionally no per-version routing past v135.
    assert_eq!(preset_line_for(Some(149)), PresetLine::Newer);
    assert_eq!(preset_line_for(Some(150)), PresetLine::Newer);
    assert_eq!(preset_line_for(Some(160)), PresetLine::Newer);
    assert_eq!(preset_line_for(Some(200)), PresetLine::Newer);
  }

  #[test]
  fn both_bundles_parse_and_cover_all_platforms() {
    for (line, json) in [
      (PresetLine::V135, data::FINGERPRINT_PRESETS_V135_JSON),
      (PresetLine::Newer, data::FINGERPRINT_PRESETS_NEWER_JSON),
    ] {
      let bundle: PresetBundle =
        serde_json::from_str(json).unwrap_or_else(|e| panic!("bundle {line:?} parse error: {e}"));
      for os in ["macos", "windows", "linux"] {
        let presets = bundle.presets.get(os).unwrap_or_else(|| {
          panic!("bundle {line:?} is missing presets for {os}");
        });
        assert!(
          !presets.is_empty(),
          "bundle {line:?} has zero presets for {os}"
        );
      }
    }
  }

  #[test]
  fn random_preset_returns_for_each_os() {
    for os in ["macos", "windows", "linux"] {
      let preset = get_random_preset(Some(os), Some(150)).expect("preset");
      assert!(preset.navigator.is_some(), "navigator present for {os}");
    }
  }

  #[test]
  fn from_preset_rewrites_firefox_version() {
    let preset = Preset {
      navigator: Some(Navigator {
        user_agent: Some(
          "Mozilla/5.0 (X11; Linux x86_64; rv:135.0) Gecko/20100101 Firefox/135.0".to_string(),
        ),
        platform: Some("Linux x86_64".to_string()),
        hardware_concurrency: Some(8),
        max_touch_points: Some(0),
        oscpu: None,
      }),
      screen: None,
      webgl: None,
      timezone: None,
      fonts: None,
      speech_voices: None,
    };
    let config = from_preset(&preset, Some(150));
    let ua = config
      .get("navigator.userAgent")
      .and_then(|v| v.as_str())
      .unwrap();
    assert!(ua.contains("Firefox/150.0"), "got: {ua}");
    assert!(ua.contains("rv:150.0"), "got: {ua}");
    // oscpu derived from "Linux x86_64" platform
    assert_eq!(
      config
        .get("navigator.oscpu")
        .and_then(|v| v.as_str())
        .unwrap(),
      "Linux x86_64"
    );
  }

  #[test]
  fn from_preset_derives_oscpu_for_mac_and_win() {
    let mut preset = Preset {
      navigator: Some(Navigator {
        user_agent: None,
        platform: Some("MacIntel".to_string()),
        hardware_concurrency: None,
        max_touch_points: None,
        oscpu: None,
      }),
      screen: None,
      webgl: None,
      timezone: None,
      fonts: None,
      speech_voices: None,
    };
    assert_eq!(
      from_preset(&preset, None)
        .get("navigator.oscpu")
        .and_then(|v| v.as_str())
        .unwrap(),
      "Intel Mac OS X 10.15"
    );
    preset.navigator.as_mut().unwrap().platform = Some("Win32".to_string());
    assert_eq!(
      from_preset(&preset, None)
        .get("navigator.oscpu")
        .and_then(|v| v.as_str())
        .unwrap(),
      "Windows NT 10.0; Win64; x64"
    );
  }

  #[test]
  fn screen_color_depth_fills_both_keys() {
    let preset = Preset {
      navigator: None,
      screen: Some(Screen {
        width: Some(1920),
        height: Some(1080),
        color_depth: Some(24),
        avail_width: Some(1920),
        avail_height: Some(1050),
        device_pixel_ratio: Some(1.0),
      }),
      webgl: None,
      timezone: None,
      fonts: None,
      speech_voices: None,
    };
    let config = from_preset(&preset, None);
    assert_eq!(config.get("screen.colorDepth").unwrap(), 24);
    assert_eq!(config.get("screen.pixelDepth").unwrap(), 24);
    assert_eq!(config.get("screen.availWidth").unwrap(), 1920);
  }
}

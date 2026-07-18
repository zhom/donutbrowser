//! Launch-time consistency check: resolve the proxy's exit IP, geolocate it
//! with the bundled MaxMind database (the same source the fingerprint generator
//! uses), then compare its timezone and country against the profile
//! fingerprint's timezone and language. A mismatch (e.g. a US fingerprint
//! behind a German exit IP) is a strong anti-bot tell even though the real
//! device never leaks — so we warn the user after launch and offer to match the
//! fingerprint to the exit. Launches never rewrite the fingerprint silently, so
//! a real mismatch always surfaces here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::profile::types::BrowserProfile;
use crate::proxy_manager::PROXY_MANAGER;

/// Exit-node lookups are cached per proxy for this long. A stored proxy's exit
/// geolocation is stable enough that re-resolving the exit IP through the proxy
/// on every launch is wasteful.
const EXIT_CACHE_TTL_SECS: u64 = 30 * 60;

#[derive(Clone)]
struct CachedExit {
  fetched_at: u64,
  /// The proxy URL this exit was measured through. Editing a stored proxy keeps
  /// its id, so without this an entry outlives the endpoint it describes: the
  /// check would compare a re-generated fingerprint against the *old* exit and
  /// either warn about a correct profile or — worse — call a genuinely
  /// mismatched one consistent, which is exactly the tell it exists to catch.
  proxy_url: String,
  timezone: Option<String>,
  country_code: Option<String>,
  ip: Option<String>,
}

lazy_static::lazy_static! {
  static ref EXIT_CACHE: Mutex<HashMap<String, CachedExit>> = Mutex::new(HashMap::new());
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConsistencyResult {
  /// True when everything we could check lines up (or there was nothing to
  /// check — no proxy assigned).
  pub consistent: bool,
  /// True when we actually reached an exit node and compared something.
  pub checked: bool,
  pub exit_ip: Option<String>,
  pub exit_country_code: Option<String>,
  pub exit_timezone: Option<String>,
  pub fingerprint_timezone: Option<String>,
  pub fingerprint_language: Option<String>,
  /// One of "timezone", "language" — the dimensions that disagree.
  pub mismatches: Vec<String>,
}

impl ConsistencyResult {
  fn skip() -> Self {
    Self {
      consistent: true,
      checked: false,
      exit_ip: None,
      exit_country_code: None,
      exit_timezone: None,
      fingerprint_timezone: None,
      fingerprint_language: None,
      mismatches: Vec::new(),
    }
  }
}

/// URL for handing this proxy to reqwest, or None for upstreams reqwest can't
/// drive (Shadowsocks).
fn proxy_url(settings: &crate::browser::ProxySettings) -> Option<String> {
  match settings.proxy_type.to_lowercase().as_str() {
    "http" | "https" | "socks4" | "socks5" => Some(
      crate::proxy_manager::ProxyManager::build_proxy_url(settings),
    ),
    _ => None,
  }
}

/// Whether the fingerprint's language is plausible for the exit country.
///
/// Validated against the same CLDR data the fingerprint generator samples from
/// (`geolocation::LocaleSelector`), not a hand-written country->language table.
/// The generator picks a language at random weighted by CLDR speaker share, so
/// any table naming one "expected" language per country flags fingerprints
/// Donut itself produced — roughly 10% of US profiles legitimately get `es-US`
/// and ~23% of Canadian ones get `fr-CA`. `None` means the country has no CLDR
/// data and the check is skipped.
fn language_matches_country(cc: &str, language: &str) -> Option<bool> {
  crate::geolocation::locale_selector()?.region_speaks(cc, language)
}

/// Extract (timezone, language) from a profile's stored fingerprint JSON.
fn fingerprint_locale(profile: &BrowserProfile) -> (Option<String>, Option<String>) {
  let Some(config) = &profile.wayfern_config else {
    return (None, None);
  };
  let Some(fp_str) = &config.fingerprint else {
    return (None, None);
  };
  let Ok(fp) = serde_json::from_str::<serde_json::Value>(fp_str) else {
    return (None, None);
  };
  let timezone = fp
    .get("timezone")
    .and_then(|v| v.as_str())
    .map(str::to_string);
  let language = fp
    .get("language")
    .and_then(|v| v.as_str())
    .map(str::to_string);
  (timezone, language)
}

/// Run the check for a profile. No-ops (consistent, unchecked) when the
/// profile has no proxy or the exit node can't be reached.
pub async fn check_profile_consistency(
  profile: &BrowserProfile,
) -> Result<ConsistencyResult, String> {
  let Some(proxy_id) = &profile.proxy_id else {
    return Ok(ConsistencyResult::skip());
  };
  let Some(settings) = PROXY_MANAGER.get_proxy_settings_by_id(proxy_id) else {
    return Ok(ConsistencyResult::skip());
  };
  let Some(url) = proxy_url(&settings) else {
    return Ok(ConsistencyResult::skip());
  };

  let now = crate::proxy_manager::now_secs();

  // Serve a fresh cached exit lookup for this proxy if we have one, but only if
  // it was measured through the proxy's current endpoint and credentials.
  let cached = {
    let cache = EXIT_CACHE.lock().unwrap();
    cache
      .get(proxy_id)
      .filter(|c| c.proxy_url == url && now.saturating_sub(c.fetched_at) < EXIT_CACHE_TTL_SECS)
      .cloned()
  };

  let (exit_tz, exit_cc, exit_ip) = if let Some(c) = cached {
    (c.timezone, c.country_code, c.ip)
  } else {
    // Resolve the exit IP through the proxy, then geolocate it with the SAME
    // bundled MaxMind database the fingerprint generator (and the on-demand
    // match) use. Using one geo source everywhere means the check can never
    // disagree with what generation produced — a second source (e.g. ip-api)
    // routinely reports a different IANA zone for the same IP in multi-zone
    // countries, which would flag correctly-generated fingerprints and would
    // leave the "match to proxy" fix unable to satisfy the check.
    let exit_ip = crate::ip_utils::fetch_public_ip(Some(&url))
      .await
      .map_err(|e| format!("exit-node lookup failed: {e}"))?;
    match crate::geolocation::get_geolocation(&exit_ip) {
      Ok(geo) => {
        let tz = Some(geo.timezone);
        let cc = geo.locale.region.clone();
        let ip = Some(exit_ip);
        EXIT_CACHE.lock().unwrap().insert(
          proxy_id.clone(),
          CachedExit {
            fetched_at: now,
            proxy_url: url.clone(),
            timezone: tz.clone(),
            country_code: cc.clone(),
            ip: ip.clone(),
          },
        );
        (tz, cc, ip)
      }
      // Reached the exit but couldn't place it (database missing, or a private
      // exit IP). Skip rather than warn on an unknown location — the same
      // database gates fingerprint geo, so there's nothing to disagree with.
      Err(e) => {
        log::debug!("Consistency check: could not geolocate exit IP: {e}");
        return Ok(ConsistencyResult::skip());
      }
    }
  };

  let (fp_tz, fp_lang) = fingerprint_locale(profile);
  let mut mismatches = Vec::new();

  if let (Some(exit), Some(fp)) = (&exit_tz, &fp_tz) {
    if !exit.eq_ignore_ascii_case(fp) {
      mismatches.push("timezone".to_string());
    }
  }

  if let (Some(cc), Some(lang)) = (&exit_cc, &fp_lang) {
    if language_matches_country(cc, lang) == Some(false) {
      mismatches.push("language".to_string());
    }
  }

  Ok(ConsistencyResult {
    consistent: mismatches.is_empty(),
    checked: true,
    exit_ip,
    exit_country_code: exit_cc,
    exit_timezone: exit_tz,
    fingerprint_timezone: fp_tz,
    fingerprint_language: fp_lang,
    mismatches,
  })
}

#[tauri::command]
pub async fn check_profile_fingerprint_consistency(
  profile_id: String,
) -> Result<ConsistencyResult, String> {
  let profiles = crate::profile::ProfileManager::instance()
    .list_profiles()
    .map_err(|e| e.to_string())?;
  let profile = profiles
    .into_iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or_else(|| serde_json::json!({ "code": "PROFILE_NOT_FOUND" }).to_string())?;
  check_profile_consistency(&profile).await
}

/// Rewrite a profile's stored fingerprint so its geolocation (timezone,
/// language, coordinates) matches `exit_ip`, and persist it. This is the
/// on-demand resolution for the consistency warning: launches no longer rewrite
/// the fingerprint silently, so the user opts into matching it here.
///
/// `exit_ip` is the exit the consistency check already resolved, applied via
/// MaxMind directly — no second proxy round-trip — and it forces geolocation
/// even when the profile has geo spoofing disabled, since the user explicitly
/// asked to match. Takes effect on the next launch of the profile.
#[tauri::command]
pub async fn match_profile_fingerprint_to_exit(
  profile_id: String,
  exit_ip: String,
) -> Result<(), String> {
  let manager = crate::profile::ProfileManager::instance();
  let mut profile = manager
    .list_profiles()
    .map_err(|e| e.to_string())?
    .into_iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or_else(|| serde_json::json!({ "code": "PROFILE_NOT_FOUND" }).to_string())?;

  let mut config = profile
    .wayfern_config
    .clone()
    .filter(|c| c.fingerprint.is_some())
    .ok_or_else(|| serde_json::json!({ "code": "FINGERPRINT_MATCH_FAILED" }).to_string())?;
  let fingerprint = config.fingerprint.clone().unwrap();

  let geoip_override = serde_json::Value::String(exit_ip);
  let refreshed = crate::wayfern_manager::WayfernManager::refresh_fingerprint_geolocation(
    &fingerprint,
    None,
    Some(&geoip_override),
  )
  .await
  .ok_or_else(|| serde_json::json!({ "code": "FINGERPRINT_MATCH_FAILED" }).to_string())?;

  config.fingerprint = Some(refreshed);
  profile.wayfern_config = Some(config);
  manager.save_profile(&profile).map_err(|e| {
    serde_json::json!({ "code": "INTERNAL_ERROR", "params": { "detail": e.to_string() } })
      .to_string()
  })?;

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn language_check_accepts_the_main_language_of_the_country() {
    assert_eq!(language_matches_country("US", "en-US"), Some(true));
    assert_eq!(language_matches_country("de", "de-DE"), Some(true));
    assert_eq!(language_matches_country("BR", "pt-BR"), Some(true));
  }

  #[test]
  fn language_check_accepts_what_the_fingerprint_generator_emits() {
    // These are not the "expected" language for the country, but the generator
    // samples the CLDR distribution and produces them routinely — CLDR puts es
    // at 9.6% in the US and fr at 30% in Canada. Flagging them warns the user
    // about a fingerprint Donut itself created.
    assert_eq!(language_matches_country("US", "es-US"), Some(true));
    assert_eq!(language_matches_country("CA", "fr-CA"), Some(true));
    assert_eq!(language_matches_country("CA", "en-CA"), Some(true));
  }

  #[test]
  fn language_check_flags_us_fingerprint_behind_armenian_exit() {
    // The reported scenario: a US (en) fingerprint routed through an Armenian
    // exit. Armenia's CLDR lists hy/ku/az, never en, so this must flag.
    assert_eq!(language_matches_country("AM", "en-US"), Some(false));
    // And a fingerprint actually matched to Armenia must not flag.
    assert_eq!(language_matches_country("AM", "hy-AM"), Some(true));
  }

  #[test]
  fn language_check_still_flags_implausible_combinations() {
    // Nothing in CLDR associates these with the country, so they remain the
    // anti-bot tell the check exists to surface. (Japan lists ja/ryu/ko only.)
    assert_eq!(language_matches_country("JP", "pt-BR"), Some(false));
    assert_eq!(language_matches_country("JP", "de-DE"), Some(false));
    assert_eq!(language_matches_country("BR", "ru-RU"), Some(false));
  }

  #[test]
  fn language_check_tolerates_minority_languages_the_generator_can_emit() {
    // CLDR lists ja for Brazil (0.21%, the Japanese-Brazilian community) and de
    // for the US (0.47%), and the generator samples both. They are weak signals
    // but flagging them would contradict our own fingerprints, so the check
    // accepts anything CLDR lists at all. That is the deliberate ceiling on
    // this dimension — timezone, compared exactly, carries the real signal.
    assert_eq!(language_matches_country("BR", "ja-JP"), Some(true));
    assert_eq!(language_matches_country("US", "de-DE"), Some(true));
  }

  #[test]
  fn language_check_skips_countries_without_cldr_data() {
    // The old hand-written table returned None for ~180 countries and silently
    // skipped the check; only genuinely unknown territories should do that now.
    assert_eq!(language_matches_country("ZZ", "en-US"), None);
    // Countries the old table never covered are now checked.
    assert!(language_matches_country("ID", "id-ID").is_some());
    assert!(language_matches_country("CH", "de-CH").is_some());
  }

  #[test]
  fn proxy_url_percent_encodes_credentials_and_skips_shadowsocks() {
    let http = crate::browser::ProxySettings {
      proxy_type: "http".into(),
      host: "h".into(),
      port: 8080,
      username: Some("u".into()),
      password: Some("p".into()),
    };
    assert_eq!(proxy_url(&http).as_deref(), Some("http://u:p@h:8080"));

    // A password with URL-reserved characters must not break the authority —
    // unencoded, the `/` truncates the host and reqwest targets `u` instead.
    let reserved = crate::browser::ProxySettings {
      proxy_type: "http".into(),
      host: "gw.provider.io".into(),
      port: 8080,
      username: Some("user".into()),
      password: Some("ab/cd@ef".into()),
    };
    assert_eq!(
      proxy_url(&reserved).as_deref(),
      Some("http://user:ab%2Fcd%40ef@gw.provider.io:8080")
    );

    // Username-only proxies keep their auth.
    let user_only = crate::browser::ProxySettings {
      proxy_type: "socks5".into(),
      host: "h".into(),
      port: 1080,
      username: Some("justuser".into()),
      password: None,
    };
    assert_eq!(
      proxy_url(&user_only).as_deref(),
      Some("socks5://justuser@h:1080")
    );

    let ss = crate::browser::ProxySettings {
      proxy_type: "ss".into(),
      host: "h".into(),
      port: 8080,
      username: None,
      password: None,
    };
    assert_eq!(proxy_url(&ss), None);
  }
}

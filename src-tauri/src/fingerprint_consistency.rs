//! Measures a proxy's exit node and compares it to a profile's fingerprint.
//!
//! Resolve the exit IP through the upstream, geolocate it with the bundled
//! MaxMind database (the same source the fingerprint generator uses), then
//! compare its timezone and country against the fingerprint's timezone and
//! language. A mismatch (e.g. a US fingerprint behind a German exit IP) is a
//! strong anti-bot tell even though the real device never leaks.
//!
//! Every comparison has three outcomes, never two: the dimensions agree, they
//! disagree, or nothing was compared because the fingerprint declares no value
//! to compare against. That third state is reported, never folded into the
//! first, "we checked and it matches" and "we checked nothing" are different
//! claims, and only one of them has been earned.
//!
//! This module only measures. Deciding what a mismatch *means* for a launch —
//! block, warn, or ignore — belongs to `launch_gate`, which calls
//! `probe_and_check_consistency` before the browser is spawned. Launches never
//! rewrite the fingerprint silently, so a real mismatch always surfaces.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::profile::types::BrowserProfile;
use crate::proxy_manager::PROXY_MANAGER;

/// Exit-node lookups are cached per proxy for this long. A stored proxy's exit
/// geolocation is stable enough that re-resolving the exit IP through the proxy
/// on every launch is wasteful.
const EXIT_CACHE_TTL_SECS: u64 = 30 * 60;

/// Ceiling on a single exit probe. `fetch_public_ip` races six endpoints with
/// a 10s timeout each, which is fine for a background check but far longer
/// than a user will wait staring at a launch that has not started yet.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

#[derive(Clone)]
struct CachedExit {
  fetched_at: u64,
  /// The endpoint this exit was measured through. Editing a stored proxy keeps
  /// its id, so without this an entry outlives the endpoint it describes: the
  /// check would compare a re-generated fingerprint against the *old* exit and
  /// either warn about a correct profile or — worse — call a genuinely
  /// mismatched one consistent, which is exactly the tell it exists to catch.
  ///
  /// Never a loopback URL: the Xray/VPN workers a launch spins up get a fresh
  /// random port and credentials each time, so keying on those would miss on
  /// every relaunch and re-probe forever.
  identity: String,
  timezone: Option<String>,
  country_code: Option<String>,
  ip: Option<String>,
}

/// Identity of the exit a profile routes through, stable across worker
/// restarts.
///
/// `scope` keys the cache; `identity` detects that the endpoint behind that
/// key changed. Cloud-derived proxies inject a per-profile sticky-session id,
/// so two profiles sharing one stored proxy correctly get different identities
/// and never inherit each other's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitCacheKey {
  pub scope: String,
  pub identity: String,
}

/// Resolve the cache identity from the profile's *stored* configuration.
///
/// Deliberately not derived from the normalized upstream the launcher passes
/// to the probe: for VLESS and VPN that upstream is a loopback worker whose
/// port and credentials are regenerated per launch.
pub fn exit_cache_key(profile: &BrowserProfile) -> Option<ExitCacheKey> {
  if let Some(proxy_id) = &profile.proxy_id {
    let settings = PROXY_MANAGER
      .resolve_proxy_for_profile(proxy_id, &profile.id.to_string())
      .or_else(|| PROXY_MANAGER.get_proxy_settings_by_id(proxy_id))?;
    // build_proxy_url returns the VLESS URI verbatim for vless proxies, so one
    // call covers every transport.
    return Some(ExitCacheKey {
      scope: format!("proxy:{proxy_id}"),
      identity: crate::proxy_manager::ProxyManager::build_proxy_url(&settings),
    });
  }
  if let Some(vpn_id) = &profile.vpn_id {
    return Some(ExitCacheKey {
      scope: format!("vpn:{vpn_id}"),
      identity: vpn_id.clone(),
    });
  }
  None
}

lazy_static::lazy_static! {
  static ref EXIT_CACHE: Mutex<HashMap<String, CachedExit>> = Mutex::new(HashMap::new());
}

/// The dimensions an exit is compared on, in report order.
pub const CHECKED_DIMENSIONS: [&str; 2] = ["timezone", "language"];

/// The outcome of comparing a measured exit against a fingerprint.
///
/// Three states, deliberately distinct, because collapsing the third into the
/// first is how a launch came to report a match it never made:
///
/// * **agree**, `checked`, `consistent`, nothing in `unverified`;
/// * **disagree**, `checked`, not `consistent`, the offenders in `mismatches`;
/// * **not compared**, the dimension is named in `unverified`, and if nothing
///   at all could be compared then `checked` is false.
///
/// `consistent` alone never means "verified": it is also true when there was
/// nothing to compare. Read it together with `checked` and `unverified`, or
/// call [`ConsistencyResult::is_mismatch`] / [`ConsistencyResult::is_verified`].
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConsistencyResult {
  /// True when no dimension that was actually compared disagreed. Also true
  /// when nothing was compared at all, so this is a claim about what was
  /// measured, never a claim that anything was.
  pub consistent: bool,
  /// True when we reached an exit node **and** compared at least one dimension
  /// against it. False both when no exit was measured and when one was measured
  /// but the fingerprint declared nothing to compare it against.
  pub checked: bool,
  pub exit_ip: Option<String>,
  pub exit_country_code: Option<String>,
  pub exit_timezone: Option<String>,
  pub fingerprint_timezone: Option<String>,
  pub fingerprint_language: Option<String>,
  /// One of "timezone", "language" — the dimensions that disagree.
  pub mismatches: Vec<String>,
  /// One of "timezone", "language", dimensions the exit supplied a value for
  /// but that were never compared, because the fingerprint declares no value of
  /// its own (or, for language, because the exit country has no CLDR data).
  ///
  /// Not a mismatch: a launch is never blocked on one, because a fingerprint
  /// whose geolocation probe failed legitimately carries no location at all.
  /// `wayfern_manager::apply_geolocation` writes nothing rather than inventing
  /// `America/New_York`, and `wayfern_manager::launch_fingerprint_payload`
  /// forwards that absence to the browser rather than filling it back in, so
  /// "not compared" describes what the launch actually presents. But not a pass
  /// either, these dimensions are unverified and must never be reported to the
  /// user as agreeing.
  #[serde(default)]
  pub unverified: Vec<String>,
}

impl ConsistencyResult {
  pub fn skip() -> Self {
    Self {
      consistent: true,
      checked: false,
      exit_ip: None,
      exit_country_code: None,
      exit_timezone: None,
      fingerprint_timezone: None,
      fingerprint_language: None,
      mismatches: Vec::new(),
      unverified: Vec::new(),
    }
  }

  /// A positively measured disagreement, the only state that may stop a
  /// launch.
  pub fn is_mismatch(&self) -> bool {
    self.checked && !self.consistent
  }

  /// Every dimension the exit offered was compared and agreed. The only state
  /// that has earned the word "consistent" in front of a user.
  pub fn is_verified(&self) -> bool {
    self.checked && self.consistent && self.unverified.is_empty()
  }
}

/// Dimensions this profile can never be verified on, whatever exit it turns out
/// to use, because its stored fingerprint declares no value to compare.
///
/// Pure and local: no exit measurement, no I/O, so the pre-launch report can
/// state it before a single worker starts. A lower bound on what a real probe
/// will report as unverified, the exit's own country can also leave the
/// language uncomparable, and that is not knowable from here.
pub fn unverifiable_dimensions(profile: &BrowserProfile) -> Vec<String> {
  let (fp_tz, fp_lang) = fingerprint_locale(profile);
  let mut out = Vec::new();
  if fp_tz.is_none() {
    out.push("timezone".to_string());
  }
  if fp_lang.is_none() {
    out.push("language".to_string());
  }
  out
}

/// True when measuring the exit can still verify at least one dimension of this
/// profile's fingerprint. False means a probe would compare nothing, so telling
/// the user "Donut will check it while starting" would be a promise it cannot
/// keep.
pub fn can_verify_anything(profile: &BrowserProfile) -> bool {
  unverifiable_dimensions(profile).len() < CHECKED_DIMENSIONS.len()
}

/// Whether this upstream can carry a probe request at all.
///
/// Shadowsocks and anything else reqwest cannot dial directly is skipped
/// rather than guessed at.
fn probe_url(settings: &crate::browser::ProxySettings) -> Option<String> {
  match settings.proxy_type.to_lowercase().as_str() {
    "http" | "https" | "httpstls" | "socks4" | "socks5" => Some(
      crate::proxy_manager::ProxyManager::build_probe_proxy_url(settings),
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
/// data, so the language cannot be judged either way, the caller reports that
/// dimension as unverified rather than counting it as a match.
fn language_matches_country(cc: &str, language: &str) -> Option<bool> {
  crate::geolocation::locale_selector()?.region_speaks(cc, language)
}

/// Extract (timezone, language) from a profile's stored location, or from its
/// legacy fingerprint payload when it still stores one.
///
/// Read through `WayfernManager::fingerprint_object`, the same accessor the
/// launcher uses to build the device it hands the browser, so both stored
/// shapes, the bare object and the legacy `{ "fingerprint": {...} }` wrapper
/// old profiles carry, are read identically on both sides. Reading only the
/// top level here made a wrapped fingerprint report "declares no timezone",
/// which sent the check down the not-compared path on exactly the profiles old
/// enough to have the wrapper, while the launch presented the timezone nested
/// one level down. Sharing the accessor is what keeps the two from drifting
/// apart again.
fn fingerprint_locale(profile: &BrowserProfile) -> (Option<String>, Option<String>) {
  let Some(fp) = profile
    .wayfern_config
    .as_ref()
    .and_then(|config| config.location.as_deref().or(config.fingerprint.as_deref()))
    .and_then(crate::wayfern_manager::WayfernManager::fingerprint_object)
  else {
    return (None, None);
  };
  let read = |key: &str| fp.get(key).and_then(|v| v.as_str()).map(str::to_string);
  (read("timezone"), read("language"))
}

/// A mutex whose poison is not fatal.
///
/// A panic anywhere under this lock used to brick the check process-wide.
/// That was tolerable when a failed check only skipped a warning; now a launch
/// consults it, so a poisoned lock must degrade rather than propagate.
fn exit_cache() -> std::sync::MutexGuard<'static, HashMap<String, CachedExit>> {
  EXIT_CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Compare a measured exit against a profile's fingerprint. Pure — no I/O.
pub fn compare_exit_to_fingerprint(
  profile: &BrowserProfile,
  exit_timezone: Option<String>,
  exit_country_code: Option<String>,
  exit_ip: Option<String>,
) -> ConsistencyResult {
  let (fp_tz, fp_lang) = fingerprint_locale(profile);
  let mut mismatches = Vec::new();
  let mut unverified = Vec::new();
  let mut compared = 0usize;

  // Three outcomes per dimension, never two. An exit whose timezone the
  // fingerprint does not declare is NOT agreement: nothing was compared, and
  // folding that into "consistent" is a green light this check has not earned.
  // "Not compared" is also a claim about the launch, not just about this
  // function: it is only honest because the launcher hands the browser no
  // timezone either (`wayfern_manager::launch_fingerprint_payload`). If it ever
  // starts supplying one again, that value is what this must compare against -
  // reporting "nothing was compared" while a location ships is the one outcome
  // neither side may produce.
  if let Some(exit) = &exit_timezone {
    match &fp_tz {
      Some(fp) => {
        compared += 1;
        if !exit.eq_ignore_ascii_case(fp) {
          mismatches.push("timezone".to_string());
        }
      }
      None => unverified.push("timezone".to_string()),
    }
  }

  // Language has one extra way to be uncomparable: a country CLDR has no data
  // for answers `None`, which is no more a match than a missing fingerprint
  // language is.
  if let Some(cc) = &exit_country_code {
    match fp_lang
      .as_ref()
      .and_then(|lang| language_matches_country(cc, lang))
    {
      Some(plausible) => {
        compared += 1;
        if !plausible {
          mismatches.push("language".to_string());
        }
      }
      None => unverified.push("language".to_string()),
    }
  }

  ConsistencyResult {
    consistent: mismatches.is_empty(),
    checked: compared > 0,
    exit_ip,
    exit_country_code,
    exit_timezone,
    fingerprint_timezone: fp_tz,
    fingerprint_language: fp_lang,
    mismatches,
    unverified,
  }
}

/// Look up a still-valid cached exit for this profile.
fn cached_exit(key: &ExitCacheKey) -> Option<CachedExit> {
  let now = crate::proxy_manager::now_secs();
  exit_cache()
    .get(&key.scope)
    .filter(|c| {
      c.identity == key.identity && now.saturating_sub(c.fetched_at) < EXIT_CACHE_TTL_SECS
    })
    .cloned()
}

/// The exit IP the launch gate last measured for this profile's route, while
/// it is still fresh. Cache-only: the gate probes on an interactive launch and
/// an automation launch never probes, so a miss here is "unknown", not "direct".
pub fn cached_exit_ip(profile: &BrowserProfile) -> Option<String> {
  let key = exit_cache_key(profile)?;
  cached_exit(&key).and_then(|cached| cached.ip)
}

/// Cache-only check. Never performs I/O, so it is safe to call before a launch
/// and for every profile in a bulk run. Returns an unchecked result on a miss.
pub fn check_profile_consistency_cached(profile: &BrowserProfile) -> ConsistencyResult {
  let Some(key) = exit_cache_key(profile) else {
    return ConsistencyResult::skip();
  };
  let Some(cached) = cached_exit(&key) else {
    return ConsistencyResult::skip();
  };
  compare_exit_to_fingerprint(profile, cached.timezone, cached.country_code, cached.ip)
}

/// Drop any cached exit for this profile, so the next check re-measures.
pub fn invalidate_exit_cache(profile: &BrowserProfile) {
  if let Some(key) = exit_cache_key(profile) {
    exit_cache().remove(&key.scope);
  }
}

/// Measure the exit through an already-normalized upstream and compare it to
/// the fingerprint.
///
/// `upstream` is what the launcher will actually hand the browser — a loopback
/// worker for VLESS and VPN, the resolved endpoint for a stored proxy — so one
/// code path covers every transport. `None` means a genuine direct connection,
/// which has nothing to disagree with.
pub async fn probe_and_check_consistency(
  profile: &BrowserProfile,
  upstream: Option<&crate::browser::ProxySettings>,
  key: &ExitCacheKey,
) -> Result<ConsistencyResult, String> {
  if let Some(cached) = cached_exit(key) {
    return Ok(compare_exit_to_fingerprint(
      profile,
      cached.timezone,
      cached.country_code,
      cached.ip,
    ));
  }

  let Some(settings) = upstream else {
    return Ok(ConsistencyResult::skip());
  };
  let Some(url) = probe_url(settings) else {
    return Ok(ConsistencyResult::skip());
  };

  // Resolve the exit IP through the proxy, then geolocate it with the SAME
  // bundled MaxMind database the fingerprint generator (and the on-demand
  // match) use. Using one geo source everywhere means the check can never
  // disagree with what generation produced — a second source (e.g. ip-api)
  // routinely reports a different IANA zone for the same IP in multi-zone
  // countries, which would flag correctly-generated fingerprints and would
  // leave the "match to proxy" fix unable to satisfy the check.
  //
  // Bounded independently of fetch_public_ip's own per-request timeout: that
  // one races six endpoints and can add up to far longer than a user will wait
  // in front of a launch.
  let fetched = tokio::time::timeout(PROBE_TIMEOUT, crate::ip_utils::fetch_public_ip(Some(&url)))
    .await
    .map_err(|_| crate::backend_error("EXIT_PROBE_FAILED"))?;
  let exit_ip = fetched.map_err(|e| crate::backend_error_with_detail("EXIT_PROBE_FAILED", e))?;

  match crate::geolocation::get_geolocation(&exit_ip) {
    Ok(geo) => {
      let tz = Some(geo.timezone);
      let cc = geo.locale.region.clone();
      exit_cache().insert(
        key.scope.clone(),
        CachedExit {
          fetched_at: crate::proxy_manager::now_secs(),
          identity: key.identity.clone(),
          timezone: tz.clone(),
          country_code: cc.clone(),
          ip: Some(exit_ip.clone()),
        },
      );
      Ok(compare_exit_to_fingerprint(profile, tz, cc, Some(exit_ip)))
    }
    // Reached the exit but couldn't place it (database missing, or a private
    // exit IP). Skip rather than warn on an unknown location — the same
    // database gates fingerprint geo, so there's nothing to disagree with.
    Err(e) => {
      log::debug!("Consistency check: could not geolocate exit IP: {e}");
      Ok(ConsistencyResult::skip())
    }
  }
}

/// Measure the exit this machine reaches without any proxy, and compare it to
/// the fingerprint.
///
/// Used when a profile declares a route that did not materialize: the browser
/// is about to connect directly, so the direct exit is the one that matters.
/// Deliberately NOT cached — a direct exit belongs to this machine's network,
/// not to any stored proxy, and it changes without a config edit.
pub async fn probe_direct_and_check(profile: &BrowserProfile) -> Result<ConsistencyResult, String> {
  let fetched = tokio::time::timeout(PROBE_TIMEOUT, crate::ip_utils::fetch_public_ip(None))
    .await
    .map_err(|_| crate::backend_error("EXIT_PROBE_FAILED"))?;
  let exit_ip = fetched.map_err(|e| crate::backend_error_with_detail("EXIT_PROBE_FAILED", e))?;

  match crate::geolocation::get_geolocation(&exit_ip) {
    Ok(geo) => Ok(compare_exit_to_fingerprint(
      profile,
      Some(geo.timezone),
      geo.locale.region.clone(),
      Some(exit_ip),
    )),
    Err(e) => {
      log::debug!("Consistency check: could not geolocate direct exit IP: {e}");
      Ok(ConsistencyResult::skip())
    }
  }
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
    .filter(|c| c.fingerprint.is_some() || c.identity_id.is_some())
    .ok_or_else(|| serde_json::json!({ "code": "FINGERPRINT_MATCH_FAILED" }).to_string())?;

  let geoip_override = serde_json::Value::String(exit_ip);
  if let Some(fingerprint) = config.fingerprint.clone() {
    // Legacy payload: the location lives inside the stored device.
    let refreshed = crate::wayfern_manager::WayfernManager::refresh_fingerprint_geolocation(
      &fingerprint,
      None,
      Some(&geoip_override),
    )
    .await
    .ok_or_else(|| serde_json::json!({ "code": "FINGERPRINT_MATCH_FAILED" }).to_string())?;
    config.fingerprint = Some(refreshed);
  } else {
    // Identity-backed: only the location object moves; the device stays
    // whatever the identity derives.
    let location = config.location.clone().unwrap_or_else(|| "{}".to_string());
    let refreshed = crate::wayfern_manager::WayfernManager::refresh_fingerprint_geolocation(
      &location,
      None,
      Some(&geoip_override),
    )
    .await
    .ok_or_else(|| serde_json::json!({ "code": "FINGERPRINT_MATCH_FAILED" }).to_string())?;
    config.location = crate::wayfern_manager::WayfernManager::fingerprint_object(&refreshed)
      .and_then(|object| crate::wayfern_manager::WayfernManager::location_of(&object));
  }
  profile.wayfern_config = Some(config);
  manager.save_profile(&profile).map_err(|e| {
    serde_json::json!({ "code": "INTERNAL_ERROR", "params": { "detail": e.to_string() } })
      .to_string()
  })?;

  // The stored verdict was computed against the fingerprint we just rewrote.
  // Leaving it would re-block the very launch this fix exists to unblock.
  invalidate_exit_cache(&profile);

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

  fn settings(
    proxy_type: &str,
    user: Option<&str>,
    pass: Option<&str>,
  ) -> crate::browser::ProxySettings {
    crate::browser::ProxySettings {
      proxy_type: proxy_type.into(),
      host: "gw.provider.io".into(),
      port: 8080,
      username: user.map(str::to_string),
      password: pass.map(str::to_string),
      vless_uri: None,
    }
  }

  #[test]
  fn probe_url_percent_encodes_credentials_and_skips_shadowsocks() {
    assert_eq!(
      probe_url(&settings("http", Some("u"), Some("p"))).as_deref(),
      Some("http://u:p@gw.provider.io:8080")
    );

    // A password with URL-reserved characters must not break the authority —
    // unencoded, the `/` truncates the host and reqwest targets `u` instead.
    assert_eq!(
      probe_url(&settings("http", Some("user"), Some("ab/cd@ef"))).as_deref(),
      Some("http://user:ab%2Fcd%40ef@gw.provider.io:8080")
    );

    // Username-only proxies keep their auth.
    assert_eq!(
      probe_url(&settings("socks4", Some("justuser"), None)).as_deref(),
      Some("socks4://justuser@gw.provider.io:8080")
    );

    // Shadowsocks cannot carry a reqwest probe, so it is skipped rather than
    // guessed at.
    assert_eq!(probe_url(&settings("ss", None, None)), None);
  }

  #[test]
  fn probe_url_uses_socks5h_so_dns_resolves_at_the_exit() {
    let url = probe_url(&settings("socks5", Some("u"), Some("p"))).unwrap();
    assert!(
      url.starts_with("socks5h://"),
      "probe must not resolve the echo host locally, got {url}"
    );
    // The browser-facing builder is deliberately left alone.
    assert!(
      crate::proxy_manager::ProxyManager::build_proxy_url(&settings(
        "socks5",
        Some("u"),
        Some("p")
      ))
      .starts_with("socks5://")
    );
  }

  fn profile_with_fingerprint(timezone: &str, language: &str) -> BrowserProfile {
    let mut profile = BrowserProfile {
      id: uuid::Uuid::new_v4(),
      name: "p".into(),
      browser: "wayfern".into(),
      ..Default::default()
    };
    profile.wayfern_config = Some(crate::wayfern_manager::WayfernConfig {
      fingerprint: Some(
        serde_json::json!({ "timezone": timezone, "language": language }).to_string(),
      ),
      ..Default::default()
    });
    profile
  }

  #[test]
  fn compare_flags_a_timezone_mismatch() {
    let profile = profile_with_fingerprint("America/New_York", "en-US");
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      Some("1.2.3.4".into()),
    );
    assert!(result.checked);
    assert!(!result.consistent);
    assert!(result.mismatches.contains(&"timezone".to_string()));
  }

  #[test]
  fn compare_accepts_a_matching_exit() {
    let profile = profile_with_fingerprint("Europe/Berlin", "de-DE");
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      Some("1.2.3.4".into()),
    );
    assert!(result.consistent, "{:?}", result.mismatches);
  }

  #[test]
  fn compare_is_case_insensitive_on_timezone() {
    let profile = profile_with_fingerprint("Europe/Berlin", "de-DE");
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("europe/berlin".into()),
      Some("DE".into()),
      None,
    );
    assert!(result.consistent);
  }

  #[test]
  fn compare_skips_dimensions_the_fingerprint_does_not_declare() {
    // A profile with no fingerprint has nothing to contradict; it must not be
    // reported as a mismatch and so must never block a launch.
    let profile = BrowserProfile {
      id: uuid::Uuid::new_v4(),
      browser: "wayfern".into(),
      ..Default::default()
    };
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      None,
    );
    assert!(!result.is_mismatch());
    assert!(result.mismatches.is_empty());
    // ...but skipping every dimension is not a pass, and must not be dressed
    // as one.
    assert!(!result.is_verified());
    assert!(!result.checked);
    assert_eq!(result.unverified, vec!["timezone", "language"]);
  }

  fn profile_with_raw_fingerprint(fingerprint: serde_json::Value) -> BrowserProfile {
    let mut profile = BrowserProfile {
      id: uuid::Uuid::new_v4(),
      name: "p".into(),
      browser: "wayfern".into(),
      ..Default::default()
    };
    profile.wayfern_config = Some(crate::wayfern_manager::WayfernConfig {
      fingerprint: Some(fingerprint.to_string()),
      ..Default::default()
    });
    profile
  }

  #[test]
  fn a_fingerprint_with_no_timezone_is_unverified_never_a_match() {
    // The regression. Generation no longer invents `America/New_York` when the
    // geolocation probe fails, so a fingerprint can legitimately carry no
    // timezone. The comparison then has nothing to compare, and reporting that
    // as agreement is a green light the check never earned, on the one
    // dimension that carries the real signal.
    let profile = profile_with_raw_fingerprint(serde_json::json!({ "language": "de-DE" }));
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      Some("1.2.3.4".into()),
    );

    assert!(
      result.unverified.contains(&"timezone".to_string()),
      "an undeclared timezone must be reported as unverified, got {result:?}"
    );
    assert!(
      !result.is_verified(),
      "nothing compared the timezone, so this must not read as consistent"
    );
    // The language WAS compared and agreed, so the exit counts as checked...
    assert!(result.checked);
    assert!(result.mismatches.is_empty());
    // ...but a dimension nobody compared is never a reason to stop a launch.
    assert!(!result.is_mismatch());
  }

  #[test]
  fn a_fingerprint_with_no_locale_at_all_is_not_checked() {
    // Both dimensions undeclared: the exit was reached, and still nothing was
    // compared. `checked` has to say so, because every consumer reads it as
    // "there is a measurement here worth acting on".
    let profile = profile_with_raw_fingerprint(serde_json::json!({ "platform": "Win32" }));
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      Some("1.2.3.4".into()),
    );
    assert!(!result.checked);
    assert!(!result.is_verified());
    assert!(!result.is_mismatch());
    assert_eq!(result.unverified, vec!["timezone", "language"]);
  }

  #[test]
  fn a_legacy_wrapped_fingerprint_is_read_the_way_the_launcher_reads_it() {
    // The launcher accepts `{"fingerprint": {...}}` as well as the bare object,
    // so reading only the top level here answered "this profile declares no
    // timezone" for a profile whose launch presents one. The check then skipped
    // the dimension carrying the real signal, on exactly the profiles old
    // enough to still have the wrapper.
    let profile = profile_with_raw_fingerprint(serde_json::json!({
      "fingerprint": { "timezone": "America/New_York", "language": "en-US" }
    }));

    assert_eq!(
      unverifiable_dimensions(&profile),
      Vec::<String>::new(),
      "a wrapped fingerprint declares both dimensions"
    );
    assert!(can_verify_anything(&profile));

    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      Some("1.2.3.4".into()),
    );
    assert_eq!(
      result.fingerprint_timezone.as_deref(),
      Some("America/New_York"),
      "the nested timezone must be the one compared, got {result:?}"
    );
    assert_eq!(result.fingerprint_language.as_deref(), Some("en-US"));
    assert!(result.checked);
    assert!(
      result.mismatches.contains(&"timezone".to_string()),
      "a US timezone behind a German exit must flag, got {result:?}"
    );
    assert!(result.is_mismatch());
    assert!(
      result.unverified.is_empty(),
      "both dimensions were declared, so nothing is unverified: {result:?}"
    );
  }

  #[test]
  fn a_wrapped_fingerprint_with_no_timezone_is_still_unverified() {
    // The other half: unwrapping must not turn "declares nothing" into a pass.
    let profile = profile_with_raw_fingerprint(serde_json::json!({
      "fingerprint": { "language": "de-DE" }
    }));
    assert_eq!(unverifiable_dimensions(&profile), vec!["timezone"]);

    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      None,
    );
    assert_eq!(result.unverified, vec!["timezone"]);
    assert!(!result.is_verified());
    assert!(!result.is_mismatch());
  }

  #[test]
  fn a_language_the_country_has_no_cldr_data_for_is_unverified() {
    // The other way a comparison can silently not happen. `ZZ` has no CLDR
    // entry, so the language was never judged; the timezone still was.
    let profile = profile_with_fingerprint("Europe/Berlin", "de-DE");
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("ZZ".into()),
      None,
    );
    assert!(result.checked, "the timezone was compared");
    assert_eq!(result.unverified, vec!["language"]);
    assert!(!result.is_verified());
    assert!(!result.is_mismatch());
  }

  #[test]
  fn only_a_fully_compared_agreement_reads_as_verified() {
    let profile = profile_with_fingerprint("Europe/Berlin", "de-DE");
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      Some("1.2.3.4".into()),
    );
    assert!(result.is_verified());
    assert!(result.unverified.is_empty());
    assert!(!result.is_mismatch());
  }

  #[test]
  fn a_measured_mismatch_is_still_the_only_blocking_state() {
    let profile = profile_with_fingerprint("America/New_York", "en-US");
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      Some("1.2.3.4".into()),
    );
    assert!(result.is_mismatch());
    assert!(!result.is_verified());
    // A verdict that blocks must not be diluted into "unverified".
    assert!(result.unverified.is_empty());
  }

  #[test]
  fn skip_reads_as_neither_verified_nor_mismatched() {
    let result = ConsistencyResult::skip();
    assert!(!result.is_verified());
    assert!(!result.is_mismatch());
    assert!(result.unverified.is_empty());
  }

  #[test]
  fn unverifiable_dimensions_are_answered_without_measuring_anything() {
    // Pure and local, so the pre-launch report can say "this cannot be checked"
    // before a single worker starts.
    assert_eq!(
      unverifiable_dimensions(&profile_with_fingerprint("Europe/Berlin", "de-DE")),
      Vec::<String>::new()
    );
    assert_eq!(
      unverifiable_dimensions(&profile_with_raw_fingerprint(
        serde_json::json!({ "language": "de-DE" })
      )),
      vec!["timezone"]
    );
    assert_eq!(
      unverifiable_dimensions(&profile_with_raw_fingerprint(
        serde_json::json!({ "platform": "Win32" })
      )),
      CHECKED_DIMENSIONS.to_vec()
    );
  }

  #[test]
  fn a_probe_that_could_compare_nothing_is_not_pending_work() {
    assert!(can_verify_anything(&profile_with_fingerprint(
      "Europe/Berlin",
      "de-DE"
    )));
    // One dimension left is still worth probing for.
    assert!(can_verify_anything(&profile_with_raw_fingerprint(
      serde_json::json!({ "language": "de-DE" })
    )));
    // Nothing left: promising the user the launch will check it would be a
    // promise the gate cannot keep.
    assert!(!can_verify_anything(&profile_with_raw_fingerprint(
      serde_json::json!({ "platform": "Win32" })
    )));
  }

  #[test]
  fn an_unverified_dimension_survives_serialization_to_the_ui() {
    let profile = profile_with_raw_fingerprint(serde_json::json!({ "language": "de-DE" }));
    let result = compare_exit_to_fingerprint(
      &profile,
      Some("Europe/Berlin".into()),
      Some("DE".into()),
      None,
    );
    let encoded = serde_json::to_value(&result).expect("serializable");
    assert_eq!(encoded["unverified"], serde_json::json!(["timezone"]));
    // Older payloads without the field must still decode, defaulting to "we
    // were told nothing", not to a silent pass.
    let legacy: ConsistencyResult = serde_json::from_str(
      r#"{"consistent":true,"checked":false,"exit_ip":null,"exit_country_code":null,
          "exit_timezone":null,"fingerprint_timezone":null,"fingerprint_language":null,
          "mismatches":[]}"#,
    )
    .expect("legacy payloads stay decodable");
    assert!(legacy.unverified.is_empty());
    assert!(!legacy.is_verified());
  }

  #[test]
  fn cached_check_reports_unchecked_without_a_proxy_or_vpn() {
    let profile = profile_with_fingerprint("Europe/Berlin", "de-DE");
    let result = check_profile_consistency_cached(&profile);
    assert!(!result.checked);
    assert!(result.consistent, "an unchecked profile must never block");
  }

  #[test]
  fn exit_cache_key_is_absent_without_a_proxy_or_vpn() {
    let profile = profile_with_fingerprint("Europe/Berlin", "de-DE");
    assert_eq!(exit_cache_key(&profile), None);
  }

  #[test]
  fn exit_cache_key_scopes_a_vpn_profile_by_vpn_id() {
    let mut profile = profile_with_fingerprint("Europe/Berlin", "de-DE");
    profile.vpn_id = Some("vpn-abc".into());
    let key = exit_cache_key(&profile).expect("vpn profiles must be cacheable");
    assert_eq!(key.scope, "vpn:vpn-abc");
    assert_eq!(key.identity, "vpn-abc");
  }

  #[test]
  fn cached_entry_is_ignored_once_the_endpoint_identity_changes() {
    let key = ExitCacheKey {
      scope: "proxy:test-identity-change".into(),
      identity: "http://old@host:1".into(),
    };
    exit_cache().insert(
      key.scope.clone(),
      CachedExit {
        fetched_at: crate::proxy_manager::now_secs(),
        identity: key.identity.clone(),
        timezone: Some("Europe/Berlin".into()),
        country_code: Some("DE".into()),
        ip: Some("1.2.3.4".into()),
      },
    );
    assert!(cached_exit(&key).is_some());

    // Editing a stored proxy keeps its id but changes the endpoint; the old
    // measurement must not be reused for the new one.
    let rotated = ExitCacheKey {
      identity: "http://new@host:2".into(),
      ..key.clone()
    };
    assert!(cached_exit(&rotated).is_none());
    exit_cache().remove(&key.scope);
  }

  #[test]
  fn cached_entry_expires_after_the_ttl() {
    let key: ExitCacheKey = ExitCacheKey {
      scope: "proxy:test-ttl".into(),
      identity: "http://host:1".into(),
    };
    exit_cache().insert(
      key.scope.clone(),
      CachedExit {
        fetched_at: crate::proxy_manager::now_secs() - EXIT_CACHE_TTL_SECS - 1,
        identity: key.identity.clone(),
        timezone: Some("Europe/Berlin".into()),
        country_code: Some("DE".into()),
        ip: None,
      },
    );
    assert!(cached_exit(&key).is_none());
    exit_cache().remove(&key.scope);
  }

  #[test]
  fn exit_cache_survives_a_poisoned_lock() {
    // A panic under this lock must degrade the check, not brick every
    // subsequent launch that consults it.
    let _ = std::thread::spawn(|| {
      let _guard = EXIT_CACHE.lock().unwrap();
      panic!("poison the cache");
    })
    .join();
    assert!(EXIT_CACHE.is_poisoned());
    exit_cache().remove("nonexistent-scope");
  }
}

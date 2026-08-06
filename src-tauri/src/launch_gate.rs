//! The pre-spawn launch gate.
//!
//! Two findings can stop a launch being what the user expects:
//!
//! * a **VPN/proxy extension** in the profile, which can override the proxy
//!   Donut configured and silently move the browser's exit away from the one
//!   the fingerprint was generated for — a warning, since Donut cannot tell
//!   from outside whether it is actually routing anything;
//! * a measured **exit/fingerprint mismatch**, which is a hard block: the
//!   browser does not start until the user explicitly proceeds.
//!
//! The enforcing half runs inside `browser_runner::launch_browser_internal`,
//! after the upstream has been normalized (so VLESS and VPN profiles are
//! reachable at all) and before the local proxy starts or the browser spawns.
//! `get_profile_pre_launch_checks` is the cheap, local-only half the UI calls
//! first, so a profile whose exit is already known blocks without starting a
//! single worker.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::fingerprint_consistency::{self, ConsistencyResult};
use crate::profile::types::BrowserProfile;
use crate::vpn_extension_detect::{self, DetectedVpnExtension};

/// How long a "launch anyway" decision stays redeemable. Long enough to read
/// the dialog, short enough that a token cannot sit around across a session.
const CONSENT_TTL_SECS: u64 = 10 * 60;

/// What the gate is allowed to do on this launch.
#[derive(Debug, Clone, Default)]
pub enum FingerprintGate {
  /// Block on a measured mismatch. The default, and what the GUI uses.
  #[default]
  Enforce,
  /// Measure only from cache and report; never block, never probe the network.
  /// Automation runs here: a headless client has no dialog to answer and
  /// cannot regenerate its fingerprint mid-run, so a hard failure would turn a
  /// warning into an outage for a whole fleet.
  Advisory,
  /// The user already said "launch anyway" and handed back a token.
  Consented(String),
}

struct PendingConsent {
  profile_id: String,
  fingerprint_hash: String,
  exit_identity: String,
  issued_at: u64,
}

lazy_static::lazy_static! {
  static ref CONSENTS: Mutex<HashMap<String, PendingConsent>> = Mutex::new(HashMap::new());
}

fn consents() -> std::sync::MutexGuard<'static, HashMap<String, PendingConsent>> {
  CONSENTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn random_token() -> String {
  use rand::Rng;
  let mut rng = rand::rng();
  let mut bytes = [0u8; 16];
  rng.fill_bytes(&mut bytes);
  bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Issue a single-use token authorizing one launch of this exact
/// (profile, fingerprint, exit) combination.
///
/// A plain `bypass: bool` cannot express this: a "proceed" the user granted
/// while looking at proxy A would silently authorize a launch through proxy B
/// if they changed it before the retry landed.
pub fn mint_consent(profile: &BrowserProfile, exit_identity: &str) -> String {
  let token = random_token();
  let now = crate::proxy_manager::now_secs();
  let mut store = consents();
  store.retain(|_, c| now.saturating_sub(c.issued_at) < CONSENT_TTL_SECS);
  store.insert(
    token.clone(),
    PendingConsent {
      profile_id: profile.id.to_string(),
      fingerprint_hash: crate::launch_gate_prefs::fingerprint_hash(profile),
      exit_identity: exit_identity.to_string(),
      issued_at: now,
    },
  );
  token
}

/// Redeem a consent token. Single use — a redeemed token is removed whether or
/// not it validated, so a leaked token cannot be replayed.
pub fn redeem_consent(
  token: &str,
  profile: &BrowserProfile,
  exit_identity: &str,
) -> Result<(), String> {
  let now = crate::proxy_manager::now_secs();
  let pending = {
    let mut store = consents();
    store.retain(|_, c| now.saturating_sub(c.issued_at) < CONSENT_TTL_SECS);
    store.remove(token)
  };

  let Some(pending) = pending else {
    return Err(crate::backend_error("LAUNCH_CONSENT_EXPIRED"));
  };
  if pending.profile_id != profile.id.to_string()
    || pending.fingerprint_hash != crate::launch_gate_prefs::fingerprint_hash(profile)
    || pending.exit_identity != exit_identity
  {
    return Err(crate::backend_error("LAUNCH_CONSENT_EXPIRED"));
  }
  Ok(())
}

fn mismatch_error(result: &ConsistencyResult, token: &str) -> String {
  serde_json::json!({
    "code": "FINGERPRINT_EXIT_MISMATCH",
    "params": {
      "token": token,
      "exitIp": result.exit_ip.clone().unwrap_or_default(),
      "exitCountry": result.exit_country_code.clone().unwrap_or_default(),
      "exitTimezone": result.exit_timezone.clone().unwrap_or_default(),
      "fingerprintTimezone": result.fingerprint_timezone.clone().unwrap_or_default(),
      "fingerprintLanguage": result.fingerprint_language.clone().unwrap_or_default(),
      "mismatches": result.mismatches.join(","),
    }
  })
  .to_string()
}

fn gate_disabled() -> bool {
  crate::settings_manager::SettingsManager::instance()
    .load_settings()
    .map(|s| s.fingerprint_gate_disabled)
    .unwrap_or(false)
}

fn extension_warning_disabled() -> bool {
  crate::settings_manager::SettingsManager::instance()
    .load_settings()
    .map(|s| s.vpn_extension_warning_disabled)
    .unwrap_or(false)
}

/// Identity used for consent and acknowledgement when the browser will connect
/// directly. Distinct from any proxy identity, so accepting a direct-exit
/// mismatch never disarms the gate for a proxied one.
const DIRECT_EXIT_IDENTITY: &str = "direct";

/// Gate a launch that will connect directly despite the profile declaring a
/// route. Measures the exit the browser will really use.
async fn enforce_direct_exit(
  profile: &BrowserProfile,
  gate: &FingerprintGate,
) -> Result<(), String> {
  if crate::launch_gate_prefs::fingerprint_ack_matches(profile, DIRECT_EXIT_IDENTITY) {
    return Ok(());
  }
  if let FingerprintGate::Consented(token) = gate {
    return redeem_consent(token, profile, DIRECT_EXIT_IDENTITY);
  }
  // Automation never probes; without a cache to consult there is nothing to say.
  if matches!(gate, FingerprintGate::Advisory) {
    return Ok(());
  }

  let result = match fingerprint_consistency::probe_direct_and_check(profile).await {
    Ok(result) => result,
    Err(e) => {
      log::warn!(
        "Fingerprint gate: direct exit probe failed for profile {}, allowing launch: {e}",
        profile.name
      );
      return Ok(());
    }
  };
  if !result.checked || result.consistent {
    return Ok(());
  }

  let token = mint_consent(profile, DIRECT_EXIT_IDENTITY);
  Err(mismatch_error(&result, &token))
}

/// The enforcing gate. Called from the launch pipeline once the upstream is
/// normalized and before anything expensive or user-visible happens.
///
/// Fails **open** on every degradation — probe failure, timeout, missing geo
/// database, private exit IP. The gate blocks only on a positively measured
/// mismatch; a flaky IP-echo endpoint must never make profiles unlaunchable.
pub async fn enforce_fingerprint_gate(
  profile: &BrowserProfile,
  upstream: Option<&crate::browser::ProxySettings>,
  gate: &FingerprintGate,
) -> Result<(), String> {
  // A profile that declares no route is genuinely direct: the browser's exit is
  // this machine, which is what an un-proxied fingerprint should describe.
  //
  // But a profile that DOES declare one and still arrives here with no upstream
  // is about to go direct anyway — a deleted or unresolvable proxy resolves to
  // `None` and the launch continues. That is the exact leak this gate exists to
  // stop, so it must be measured, not waved through.
  let declares_route = profile.proxy_id.is_some() || profile.vpn_id.is_some();
  if upstream.is_none() && !declares_route {
    return Ok(());
  }
  let Some(key) = fingerprint_consistency::exit_cache_key(profile) else {
    // Declares a route we can no longer resolve at all (e.g. the stored proxy
    // was deleted). Nothing identifies the exit, so measure the direct one.
    if declares_route {
      log::warn!(
        "Fingerprint gate: {} declares a proxy/VPN that did not resolve; \
         measuring the direct exit it will actually use",
        profile.name
      );
    }
    return enforce_direct_exit(profile, gate).await;
  };
  if gate_disabled() {
    return Ok(());
  }

  // Ack first: a persisted acknowledgement already permits this launch, so a
  // stale token must not turn it into a hard failure.
  if crate::launch_gate_prefs::fingerprint_ack_matches(profile, &key.identity) {
    return Ok(());
  }

  if let FingerprintGate::Consented(token) = gate {
    redeem_consent(token, profile, &key.identity)?;
    return Ok(());
  }

  // The route is known but produced no usable upstream (e.g. a VPN worker that
  // came up without a local port). The browser still launches, direct.
  if upstream.is_none() {
    log::warn!(
      "Fingerprint gate: {} has a route that yielded no upstream; measuring the direct exit",
      profile.name
    );
    return enforce_direct_exit(profile, gate).await;
  }

  let result = if matches!(gate, FingerprintGate::Advisory) {
    // Automation: answer from a warm cache or say nothing. Probing here would
    // add seconds to every profile in a batch run.
    fingerprint_consistency::check_profile_consistency_cached(profile)
  } else {
    match fingerprint_consistency::probe_and_check_consistency(profile, upstream, &key).await {
      Ok(result) => result,
      Err(e) => {
        log::warn!(
          "Fingerprint gate: exit probe failed for profile {}, allowing launch: {e}",
          profile.name
        );
        return Ok(());
      }
    }
  };

  if !result.checked || result.consistent {
    return Ok(());
  }

  // Only now is the extension scan worth its disk walk. A confirmed
  // proxy-permission extension can redirect the browser's traffic away from the
  // upstream we just measured, so the measurement describes an exit the browser
  // may not take. Report it, but do not hard-block on a number known to be
  // unreliable.
  let measurement_unreliable =
    vpn_extension_detect::has_confirmed(&vpn_extension_detect::scan_profile(profile));

  if matches!(gate, FingerprintGate::Advisory) || measurement_unreliable {
    log::warn!(
      "Fingerprint gate: {} launching with a {} exit mismatch ({})",
      profile.name,
      if measurement_unreliable {
        "unverifiable"
      } else {
        "known"
      },
      result.mismatches.join(", ")
    );
    if let Err(e) = crate::events::emit("fingerprint-consistency-warning", &result) {
      log::warn!("Failed to emit fingerprint consistency warning: {e}");
    }
    return Ok(());
  }

  let token = mint_consent(profile, &key.identity);
  Err(mismatch_error(&result, &token))
}

/// Everything the UI needs to decide whether to stop a launch, answered
/// without touching the network or starting any worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreLaunchChecks {
  pub vpn_extensions: Vec<DetectedVpnExtension>,
  pub scan_state: String,
  /// Cache-only; `checked` is false when the exit has not been measured yet.
  pub consistency: ConsistencyResult,
  /// True when the enforcing gate will still probe during the launch, so the
  /// UI can say the check is not finished rather than implying it passed.
  pub exit_probe_pending: bool,
  /// A confirmed proxy-permission extension is present, so any exit
  /// measurement describes a route the browser may not take.
  pub exit_measurement_unreliable: bool,
  /// Present only when a cached mismatch is already blocking, so "launch
  /// anyway" can proceed without a second round trip.
  pub consent_token: Option<String>,
}

fn load_profile(profile_id: &str) -> Result<BrowserProfile, String> {
  crate::profile::ProfileManager::instance()
    .list_profiles()
    .map_err(|e| e.to_string())?
    .into_iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or_else(|| crate::backend_error("PROFILE_NOT_FOUND"))
}

#[tauri::command]
pub async fn get_profile_pre_launch_checks(profile_id: String) -> Result<PreLaunchChecks, String> {
  let profile = load_profile(&profile_id)?;

  let scan = if extension_warning_disabled() {
    vpn_extension_detect::ExtensionScan {
      extensions: Vec::new(),
      scan_state: "scanned".to_string(),
    }
  } else {
    vpn_extension_detect::scan_profile(&profile)
  };

  // Drop anything the user has already acknowledged for this profile, so the
  // dialog only ever opens for something new.
  let vpn_extensions: Vec<DetectedVpnExtension> = scan
    .extensions
    .iter()
    .filter(|e| {
      !crate::launch_gate_prefs::extensions_acked(&profile_id, std::slice::from_ref(&e.key))
    })
    .cloned()
    .collect();
  let exit_measurement_unreliable = vpn_extension_detect::has_confirmed(&scan);

  let disabled = gate_disabled();
  let key = fingerprint_consistency::exit_cache_key(&profile);

  let consistency = if disabled {
    ConsistencyResult::skip()
  } else {
    fingerprint_consistency::check_profile_consistency_cached(&profile)
  };

  let already_acked = key
    .as_ref()
    .is_some_and(|k| crate::launch_gate_prefs::fingerprint_ack_matches(&profile, &k.identity));

  let blocking = consistency.checked && !consistency.consistent && !already_acked;
  let consent_token = match (&key, blocking) {
    (Some(k), true) => Some(mint_consent(&profile, &k.identity)),
    _ => None,
  };

  Ok(PreLaunchChecks {
    vpn_extensions,
    scan_state: scan.scan_state,
    consistency: if blocking {
      consistency
    } else {
      ConsistencyResult::skip()
    },
    exit_probe_pending: !disabled && !already_acked && key.is_some() && !blocking,
    exit_measurement_unreliable,
    consent_token,
  })
}

/// Persist "don't ask me again" choices from the gate dialog.
#[tauri::command]
pub async fn ack_launch_gate(
  profile_id: String,
  ack_fingerprint: bool,
  ack_extension_keys: Vec<String>,
) -> Result<(), String> {
  let profile = load_profile(&profile_id)?;

  if ack_fingerprint {
    if let Some(key) = fingerprint_consistency::exit_cache_key(&profile) {
      crate::launch_gate_prefs::ack_fingerprint(&profile, &key.identity);
    }
  }
  crate::launch_gate_prefs::ack_extensions(&profile_id, &ack_extension_keys);
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn profile_with(fingerprint: &str) -> BrowserProfile {
    let mut profile = BrowserProfile {
      id: uuid::Uuid::new_v4(),
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
  fn consent_token_authorizes_exactly_one_launch() {
    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let token = mint_consent(&profile, "http://gw:1");
    assert!(redeem_consent(&token, &profile, "http://gw:1").is_ok());
    // Replaying it must fail, so a leaked token cannot re-authorize.
    assert!(redeem_consent(&token, &profile, "http://gw:1").is_err());
  }

  #[test]
  fn consent_token_is_rejected_for_a_different_profile() {
    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let other = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let token = mint_consent(&profile, "http://gw:1");
    assert!(redeem_consent(&token, &other, "http://gw:1").is_err());
  }

  #[test]
  fn consent_token_is_rejected_after_the_fingerprint_changes() {
    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let token = mint_consent(&profile, "http://gw:1");

    let mut regenerated = profile_with(r#"{"timezone":"America/New_York"}"#);
    regenerated.id = profile.id;
    assert!(redeem_consent(&token, &regenerated, "http://gw:1").is_err());
  }

  #[test]
  fn consent_token_is_rejected_after_the_exit_changes() {
    // The reason a bare `bypass: bool` is not enough: consent granted for one
    // proxy must not authorize a launch through another.
    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let token = mint_consent(&profile, "http://gw:1");
    assert!(redeem_consent(&token, &profile, "http://other:2").is_err());
  }

  #[test]
  fn an_unknown_token_is_rejected() {
    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let err = redeem_consent("deadbeef", &profile, "http://gw:1").unwrap_err();
    assert!(err.contains("LAUNCH_CONSENT_EXPIRED"), "{err}");
  }

  #[test]
  fn expired_tokens_are_swept_and_rejected() {
    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let token = mint_consent(&profile, "http://gw:1");
    // Back-date it past the TTL.
    {
      let mut store = consents();
      if let Some(pending) = store.get_mut(&token) {
        pending.issued_at = crate::proxy_manager::now_secs() - CONSENT_TTL_SECS - 1;
      }
    }
    assert!(redeem_consent(&token, &profile, "http://gw:1").is_err());
  }

  #[test]
  fn mismatch_error_carries_the_details_the_dialog_renders() {
    let result = ConsistencyResult {
      consistent: false,
      checked: true,
      exit_ip: Some("1.2.3.4".into()),
      exit_country_code: Some("DE".into()),
      exit_timezone: Some("Europe/Berlin".into()),
      fingerprint_timezone: Some("America/New_York".into()),
      fingerprint_language: Some("en-US".into()),
      mismatches: vec!["timezone".into(), "language".into()],
    };
    let encoded = mismatch_error(&result, "tok");
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(parsed["code"], "FINGERPRINT_EXIT_MISMATCH");
    assert_eq!(parsed["params"]["token"], "tok");
    assert_eq!(parsed["params"]["exitTimezone"], "Europe/Berlin");
    assert_eq!(parsed["params"]["fingerprintTimezone"], "America/New_York");
    // params values must be strings for the frontend's interpolation.
    assert_eq!(parsed["params"]["mismatches"], "timezone,language");
  }

  #[tokio::test]
  async fn gate_allows_a_direct_connection_without_measuring() {
    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    assert!(
      enforce_fingerprint_gate(&profile, None, &FingerprintGate::Enforce)
        .await
        .is_ok()
    );
  }

  #[tokio::test]
  async fn gate_allows_a_profile_with_no_proxy_or_vpn() {
    // No exit identity means nothing to compare against, so the launch must
    // proceed rather than block on an unmeasurable profile.
    let profile = profile_with(r#"{"timezone":"Europe/Berlin"}"#);
    let upstream = crate::browser::ProxySettings {
      proxy_type: "socks5".into(),
      host: "127.0.0.1".into(),
      port: 1080,
      username: None,
      password: None,
      vless_uri: None,
    };
    assert!(
      enforce_fingerprint_gate(&profile, Some(&upstream), &FingerprintGate::Enforce)
        .await
        .is_ok()
    );
  }
}

use crate::cloud_auth::CLOUD_AUTH;
use crate::profile::types::get_host_os;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::wayfern_manager::{
  cdp_error_is_invalid_params, is_temporary_wayfern_failure, supports_wayfern_152, wayfern_failure,
  HeadlessWayfern, WayfernConfig, WayfernManager,
};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredShape {
  Payload,
  IdentityWithView,
  Identity,
  Empty,
}

#[derive(Debug)]
enum ConversionError {
  Deferred(String),
  Changed,
  Yielded,
  Failed(String),
}

impl ConversionError {
  fn coded(self) -> String {
    match self {
      Self::Deferred(coded) | Self::Failed(coded) => coded,
      Self::Changed | Self::Yielded => crate::backend_error("WAYFERN_BROWSER_BUSY"),
    }
  }
}

const COMMAND: &str = "Wayfern.matchLegacyProfile";
const PASS_DELAY: Duration = Duration::from_secs(3);
const RETRY_DELAY: Duration = Duration::from_secs(600);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_RETRIES: u32 = 6;

static PASS_REQUESTED: AtomicBool = AtomicBool::new(false);
static PASS_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));
static RETRIES: AtomicU32 = AtomicU32::new(0);
static LAUNCHES: AtomicU32 = AtomicU32::new(0);
static LAUNCH_STARTED: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);
static SESSION: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

pub struct LaunchInProgress(());

impl Drop for LaunchInProgress {
  fn drop(&mut self) {
    LAUNCHES.fetch_sub(1, Ordering::SeqCst);
  }
}

pub async fn yield_to_launch() -> LaunchInProgress {
  LAUNCHES.fetch_add(1, Ordering::SeqCst);
  let in_progress = LaunchInProgress(());
  LAUNCH_STARTED.notify_waiters();
  drop(SESSION.lock().await);
  in_progress
}

fn launch_pending() -> bool {
  LAUNCHES.load(Ordering::SeqCst) > 0
}

fn stored_identity_id(config: &WayfernConfig) -> Option<&str> {
  config
    .identity_id
    .as_deref()
    .map(str::trim)
    .filter(|id| !id.is_empty())
}

fn stored_payload(config: &WayfernConfig) -> Option<Map<String, Value>> {
  let json = config.fingerprint.as_deref()?;
  WayfernManager::launch_fingerprint_payload(json)
    .ok()?
    .as_object()
    .cloned()
    .filter(|object| !object.is_empty())
}

pub fn stored_shape(config: &WayfernConfig) -> StoredShape {
  let payload = stored_payload(config).is_some();
  let baseline = !WayfernManager::stored_object(config.identity_baseline.as_deref()).is_empty();
  match (stored_identity_id(config).is_some(), payload, baseline) {
    (true, true, _) | (true, false, true) => StoredShape::IdentityWithView,
    (true, false, false) => StoredShape::Identity,
    (false, true, _) => StoredShape::Payload,
    (false, false, _) => StoredShape::Empty,
  }
}

pub fn needs_conversion(profile: &BrowserProfile) -> bool {
  profile.browser == "wayfern"
    && supports_wayfern_152(&profile.version)
    && !profile.is_cross_os()
    && profile.wayfern_config.as_ref().is_some_and(|config| {
      config.randomize_fingerprint_on_launch != Some(true)
        && matches!(
          stored_shape(config),
          StoredShape::Payload | StoredShape::IdentityWithView
        )
    })
}

pub fn conversion_params(profile: &BrowserProfile, config: &WayfernConfig) -> Value {
  let mut params = Map::new();
  if let Some(payload) = stored_payload(config) {
    params.insert("fingerprint".to_string(), Value::Object(payload));
  }
  if let Some(identity_id) = stored_identity_id(config) {
    params.insert("identityId".to_string(), json!(identity_id));
  }
  let baseline = WayfernManager::stored_object(config.identity_baseline.as_deref());
  if !baseline.is_empty() {
    params.insert("identityBaseline".to_string(), Value::Object(baseline));
  }
  let overrides = WayfernManager::stored_object(config.identity_overrides.as_deref());
  if !overrides.is_empty() {
    params.insert("identityOverrides".to_string(), Value::Object(overrides));
  }
  params.insert("profileSalt".to_string(), json!(profile.id.to_string()));
  Value::Object(params)
}

pub fn converted_profile(
  profile: &BrowserProfile,
  response: &Value,
) -> Result<BrowserProfile, String> {
  if response.get("usable").and_then(Value::as_bool) != Some(true)
    || response.get("kept").and_then(Value::as_bool).is_none()
  {
    return Err("the browser did not return a usable identity".to_string());
  }
  let identity_id = response
    .get("identityId")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|id| id.len() == 36)
    .ok_or_else(|| "the browser returned no identity".to_string())?;
  let os = response
    .get("operatingSystem")
    .and_then(Value::as_str)
    .and_then(WayfernManager::normalize_os_name)
    .ok_or_else(|| "the browser returned no operating system".to_string())?;
  let overrides = response
    .get("overrides")
    .and_then(Value::as_object)
    .cloned()
    .unwrap_or_default();
  let location = response
    .get("location")
    .and_then(Value::as_object)
    .and_then(WayfernManager::location_of);

  let mut updated = profile.clone();
  let config = updated
    .wayfern_config
    .get_or_insert_with(WayfernConfig::default);
  config.identity_id = Some(identity_id.to_string());
  config.os = Some(os.to_string());
  config.identity_overrides = if overrides.is_empty() {
    None
  } else {
    Some(Value::Object(overrides).to_string())
  };
  if location.is_some() {
    config.location = location;
  }
  config.fingerprint = None;
  config.identity_baseline = None;

  let host_os = get_host_os();
  if updated.host_os.is_none() && os != host_os {
    updated.host_os = Some(host_os);
  }
  Ok(updated)
}

fn read_profile(id: &uuid::Uuid) -> Option<BrowserProfile> {
  let path = ProfileManager::instance()
    .get_profiles_dir()
    .join(id.to_string())
    .join("metadata.json");
  let content = std::fs::read_to_string(path).ok()?;
  serde_json::from_str(&content).ok()
}

async fn is_running(profile: &BrowserProfile) -> bool {
  if profile.process_id.is_some() {
    return true;
  }
  let profiles_dir = ProfileManager::instance().get_profiles_dir();
  let data_path = crate::ephemeral_dirs::get_effective_profile_path(profile, &profiles_dir);
  WayfernManager::instance()
    .find_wayfern_by_profile(&data_path.to_string_lossy())
    .await
    .is_some()
}

fn write_backup(profile: &BrowserProfile) -> Result<(), String> {
  let dir = crate::app_dirs::data_subdir().join("wayfern-config-backups");
  std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
  let path = dir.join(format!("{}.json", profile.id));
  if path.exists() {
    return Ok(());
  }
  let body = serde_json::to_vec_pretty(&json!({
    "profileId": profile.id.to_string(),
    "version": profile.version,
    "wayfernConfig": profile.wayfern_config,
  }))
  .map_err(|e| e.to_string())?;
  crate::app_dirs::write_owner_only(&path, &body).map_err(|e| e.to_string())
}

fn config_value(profile: &BrowserProfile) -> Value {
  serde_json::to_value(&profile.wayfern_config).unwrap_or(Value::Null)
}

fn internal(detail: impl std::fmt::Display) -> String {
  crate::backend_error_with_detail("INTERNAL_ERROR", detail)
}

fn classify(detail: &str) -> ConversionError {
  let coded = wayfern_failure(detail, "WAYFERN_FINGERPRINT_APPLY_FAILED", None);
  let permanent = !is_temporary_wayfern_failure(&coded)
    && (cdp_error_is_invalid_params(detail) || detail.contains("wasn't found"));
  if permanent {
    ConversionError::Failed(coded)
  } else {
    ConversionError::Deferred(coded)
  }
}

async fn request(
  session: &HeadlessWayfern,
  params: Value,
  interruptible: bool,
) -> Result<Value, ConversionError> {
  let launch_started = LAUNCH_STARTED.notified();
  if interruptible && launch_pending() {
    return Err(ConversionError::Yielded);
  }
  let call = tokio::time::timeout(
    CALL_TIMEOUT,
    session.call(WayfernManager::instance(), COMMAND, params),
  );
  let outcome = if interruptible {
    tokio::select! {
      outcome = call => outcome,
      () = launch_started => return Err(ConversionError::Yielded),
    }
  } else {
    call.await
  };
  match outcome {
    Ok(Ok(response)) => Ok(response),
    Ok(Err(e)) => Err(classify(&e.to_string())),
    Err(_) => Err(ConversionError::Deferred(internal(format!(
      "{COMMAND} did not answer"
    )))),
  }
}

async fn convert_and_save(
  session: &HeadlessWayfern,
  profile: &BrowserProfile,
  interruptible: bool,
  wait_for_withheld: bool,
) -> Result<BrowserProfile, ConversionError> {
  let config = profile.wayfern_config.clone().unwrap_or_default();
  let response = request(session, conversion_params(profile, &config), interruptible).await?;
  if wait_for_withheld
    && response.get("withheld").and_then(Value::as_bool) == Some(true)
    && !CLOUD_AUTH.is_logged_in().await
  {
    return Err(ConversionError::Deferred(crate::backend_error(
      "WAYFERN_PLAN_CHECK_UNAVAILABLE",
    )));
  }
  let updated = converted_profile(profile, &response).map_err(|reason| {
    ConversionError::Failed(crate::backend_error_with_detail(
      "WAYFERN_FINGERPRINT_APPLY_FAILED",
      reason,
    ))
  })?;

  let Some(current) = read_profile(&profile.id) else {
    return Err(ConversionError::Failed(crate::backend_error(
      "PROFILE_NOT_FOUND",
    )));
  };
  if config_value(&current) != config_value(profile) || current.version != profile.version {
    return Err(ConversionError::Changed);
  }
  if is_running(&current).await {
    return Err(ConversionError::Deferred(crate::backend_error(
      "PROFILE_RUNNING",
    )));
  }
  write_backup(&current).map_err(|e| ConversionError::Deferred(internal(e)))?;
  let mut stored = current.clone();
  stored.wayfern_config = updated.wayfern_config;
  if stored.host_os.is_none() {
    stored.host_os = updated.host_os;
  }
  if !ProfileManager::instance()
    .save_profile_unchanged(&current, &stored)
    .map_err(|e| ConversionError::Deferred(internal(e)))?
  {
    return Err(ConversionError::Changed);
  }
  log::info!("Stored Wayfern profile {} as an identity", stored.id);
  Ok(stored)
}

async fn token_for_conversion() -> Result<Option<String>, String> {
  let mut token = CLOUD_AUTH.get_wayfern_token().await;
  if token.is_some() || !CLOUD_AUTH.is_entitled_to_wayfern_token().await {
    return Ok(token);
  }
  for _ in 0..3 {
    tokio::time::sleep(Duration::from_secs(1)).await;
    token = CLOUD_AUTH.get_wayfern_token().await;
    if token.is_some() {
      return Ok(token);
    }
  }
  Err(crate::backend_error("WAYFERN_PLAN_CHECK_UNAVAILABLE"))
}

pub async fn convert_for_launch(profile: &BrowserProfile) -> Result<BrowserProfile, String> {
  let Some(current) = read_profile(&profile.id) else {
    return Err(crate::backend_error("PROFILE_NOT_FOUND"));
  };
  if !needs_conversion(&current) {
    return Ok(current);
  }
  let token = token_for_conversion().await?;
  let session = HeadlessWayfern::start(
    WayfernManager::instance(),
    &current,
    token.as_deref(),
    &format!("profile {}", current.id),
  )
  .await
  .map_err(|e| wayfern_failure(&e.to_string(), "WAYFERN_FINGERPRINT_APPLY_FAILED", None))?;
  let mut result = convert_and_save(&session, &current, false, true).await;
  if matches!(result, Err(ConversionError::Changed)) {
    result = match read_profile(&profile.id) {
      Some(latest) if needs_conversion(&latest) && latest.version == current.version => {
        convert_and_save(&session, &latest, false, true).await
      }
      Some(latest) if !needs_conversion(&latest) => Ok(latest),
      Some(_) => Err(ConversionError::Changed),
      None => Err(ConversionError::Failed(crate::backend_error(
        "PROFILE_NOT_FOUND",
      ))),
    };
  }
  session.stop().await;
  match result {
    Ok(converted) => Ok(converted),
    Err(ConversionError::Failed(reason)) => {
      log::warn!(
        "Launching Wayfern profile {} on its stored device: {reason}",
        current.id
      );
      Ok(current)
    }
    Err(other) => Err(other.coded()),
  }
}

fn any_wayfern_running(profiles: &[BrowserProfile]) -> bool {
  profiles
    .iter()
    .any(|profile| profile.browser == "wayfern" && profile.process_id.is_some())
}

async fn run_pass() -> bool {
  if !crate::wayfern_terms::WayfernTermsManager::instance().is_terms_accepted() {
    return false;
  }
  let Ok(profiles) = ProfileManager::instance().list_profiles() else {
    return true;
  };
  let candidates: Vec<uuid::Uuid> = profiles
    .iter()
    .filter(|profile| needs_conversion(profile))
    .map(|profile| profile.id)
    .collect();
  if candidates.is_empty() {
    return false;
  }
  if any_wayfern_running(&profiles) {
    return true;
  }
  let token = match token_for_conversion().await {
    Ok(token) => token,
    Err(_) => return true,
  };

  let mut deferred = false;
  let mut stored = 0usize;
  let mut session: Option<(
    String,
    HeadlessWayfern,
    tokio::sync::MutexGuard<'static, ()>,
  )> = None;
  for id in candidates {
    if launch_pending() {
      deferred = true;
      break;
    }
    let Some(_launch_guard) = crate::browser_runner::try_lock_profile_launch(&id.to_string()).await
    else {
      deferred = true;
      continue;
    };
    let Some(profile) = read_profile(&id) else {
      continue;
    };
    if !needs_conversion(&profile) {
      continue;
    }
    let fleet = ProfileManager::instance()
      .list_profiles()
      .unwrap_or_default();
    if any_wayfern_running(&fleet) || is_running(&profile).await {
      deferred = true;
      break;
    }
    if session
      .as_ref()
      .is_none_or(|(version, _, _)| version != &profile.version)
    {
      if let Some((_, previous, _alive)) = session.take() {
        previous.stop().await;
      }
      if launch_pending() {
        deferred = true;
        break;
      }
      let alive = SESSION.lock().await;
      match HeadlessWayfern::start(
        WayfernManager::instance(),
        &profile,
        token.as_deref(),
        "stored profiles",
      )
      .await
      {
        Ok(started) => session = Some((profile.version.clone(), started, alive)),
        Err(e) => {
          log::warn!(
            "Could not start Wayfern {} for stored profiles: {e}",
            profile.version
          );
          deferred = true;
          continue;
        }
      }
    }
    let Some((_, browser, _)) = session.as_ref() else {
      continue;
    };
    match convert_and_save(browser, &profile, true, true).await {
      Ok(_) => stored += 1,
      Err(ConversionError::Yielded) => {
        deferred = true;
        break;
      }
      Err(ConversionError::Changed) => deferred = true,
      Err(ConversionError::Deferred(reason)) => {
        log::info!("Stored Wayfern profile {id} is unchanged for now: {reason}");
        deferred = true;
      }
      Err(ConversionError::Failed(reason)) => {
        log::warn!("Stored Wayfern profile {id} could not be converted: {reason}");
      }
    }
  }
  if let Some((_, browser, _alive)) = session.take() {
    browser.stop().await;
  }
  if stored > 0 {
    let _ = crate::events::emit_empty("profiles-changed");
  }
  deferred
}

pub fn request_conversion_pass() {
  if cfg!(test) || PASS_REQUESTED.swap(true, Ordering::SeqCst) {
    return;
  }
  tauri::async_runtime::spawn(async move {
    tokio::time::sleep(PASS_DELAY).await;
    let deferred = {
      let _pass = PASS_LOCK.lock().await;
      PASS_REQUESTED.store(false, Ordering::SeqCst);
      run_pass().await
    };
    if !deferred {
      RETRIES.store(0, Ordering::SeqCst);
      return;
    }
    if RETRIES.fetch_add(1, Ordering::SeqCst) >= MAX_RETRIES {
      return;
    }
    tokio::time::sleep(RETRY_DELAY).await;
    request_conversion_pass();
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn profile_with(version: &str, config: WayfernConfig) -> BrowserProfile {
    BrowserProfile {
      id: uuid::Uuid::parse_str("7d1f0f4e-9e36-4a0b-8c55-3f0c2c9f5a11").unwrap(),
      name: "stored".to_string(),
      browser: "wayfern".to_string(),
      version: version.to_string(),
      process_id: None,
      proxy_id: None,
      vpn_id: None,
      launch_hook: None,
      last_launch: None,
      release_type: "stable".to_string(),
      wayfern_config: Some(config),
      group_id: None,
      tags: Vec::new(),
      note: None,
      window_color: None,
      sync_mode: crate::profile::types::SyncMode::Disabled,
      encryption_salt: None,
      last_sync: None,
      host_os: Some(get_host_os()),
      ephemeral: false,
      temporary: false,
      extension_group_id: None,
      proxy_bypass_rules: Vec::new(),
      created_by_id: None,
      created_by_email: None,
      dns_blocklist: None,
      password_protected: false,
      clear_on_close: false,
      created_at: None,
      updated_at: None,
    }
  }

  fn payload() -> String {
    json!({
      "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36",
      "platform": "Win32",
      "webglRenderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 (0x00002503) Direct3D11 vs_5_0 ps_5_0, D3D11)",
      "hardwareConcurrency": 12,
      "timezone": "Europe/Berlin",
      "language": "de-DE",
      "languages": "de-DE,de",
      "latitude": 52.52,
      "longitude": 13.405
    })
    .to_string()
  }

  const LEGACY_ID: &str = "3fa85f64-5717-4562-b3fc-2c963f66a0e1";
  const CURRENT_ID: &str = "0b1c2d3e-4f50-4a61-8b72-93a4b5c6d7e8";

  #[test]
  fn shapes_follow_what_the_config_stores() {
    let empty = WayfernConfig::default();
    assert_eq!(stored_shape(&empty), StoredShape::Empty);

    let payload_only = WayfernConfig {
      fingerprint: Some(payload()),
      ..Default::default()
    };
    assert_eq!(stored_shape(&payload_only), StoredShape::Payload);

    let with_view = WayfernConfig {
      identity_id: Some(LEGACY_ID.to_string()),
      fingerprint: Some(payload()),
      identity_baseline: Some(payload()),
      ..Default::default()
    };
    assert_eq!(stored_shape(&with_view), StoredShape::IdentityWithView);

    let baseline_only = WayfernConfig {
      identity_id: Some(LEGACY_ID.to_string()),
      identity_baseline: Some(payload()),
      ..Default::default()
    };
    assert_eq!(stored_shape(&baseline_only), StoredShape::IdentityWithView);

    let identity = WayfernConfig {
      identity_id: Some(CURRENT_ID.to_string()),
      location: Some(r#"{"timezone":"Europe/Berlin"}"#.to_string()),
      ..Default::default()
    };
    assert_eq!(stored_shape(&identity), StoredShape::Identity);

    for unusable in ["{}", "null", "not json", "  "] {
      let config = WayfernConfig {
        fingerprint: Some(unusable.to_string()),
        ..Default::default()
      };
      assert_eq!(stored_shape(&config), StoredShape::Empty, "{unusable}");
    }

    let wrapped = WayfernConfig {
      fingerprint: Some(format!(r#"{{"fingerprint":{}}}"#, payload())),
      ..Default::default()
    };
    assert_eq!(stored_shape(&wrapped), StoredShape::Payload);
  }

  #[test]
  fn only_stored_devices_on_152_are_converted() {
    let payload_only = WayfernConfig {
      fingerprint: Some(payload()),
      ..Default::default()
    };
    assert!(needs_conversion(&profile_with(
      "152.0.7977.64",
      payload_only.clone()
    )));
    assert!(!needs_conversion(&profile_with(
      "151.0.7922.71",
      payload_only.clone()
    )));
    assert!(!needs_conversion(&profile_with(
      "150.0.7801.12",
      payload_only.clone()
    )));

    let mut randomized = payload_only.clone();
    randomized.randomize_fingerprint_on_launch = Some(true);
    assert!(!needs_conversion(&profile_with(
      "152.0.7977.64",
      randomized
    )));

    let identity = WayfernConfig {
      identity_id: Some(CURRENT_ID.to_string()),
      ..Default::default()
    };
    assert!(!needs_conversion(&profile_with("152.0.7977.64", identity)));

    let mut camoufox = profile_with("152.0.7977.64", payload_only.clone());
    camoufox.browser = "camoufox".to_string();
    assert!(!needs_conversion(&camoufox));

    let mut foreign = profile_with("152.0.7977.64", payload_only);
    foreign.host_os = Some(
      if get_host_os() == "windows" {
        "macos"
      } else {
        "windows"
      }
      .to_string(),
    );
    assert!(!needs_conversion(&foreign));
  }

  #[test]
  fn the_request_carries_every_stored_part_and_the_profile_salt() {
    let config = WayfernConfig {
      identity_id: Some(format!(" {LEGACY_ID} ")),
      fingerprint: Some(format!(r#"{{"fingerprint":{}}}"#, payload())),
      identity_baseline: Some(r#"{"hardwareConcurrency":8}"#.to_string()),
      identity_overrides: Some(r#"{"doNotTrack":"1"}"#.to_string()),
      ..Default::default()
    };
    let profile = profile_with("152.0.7977.64", config.clone());
    let params = conversion_params(&profile, &config);

    assert_eq!(params["identityId"], json!(LEGACY_ID));
    assert_eq!(params["profileSalt"], json!(profile.id.to_string()));
    assert_eq!(params["identityBaseline"]["hardwareConcurrency"], json!(8));
    assert_eq!(params["identityOverrides"]["doNotTrack"], json!("1"));
    assert_eq!(params["fingerprint"]["platform"], json!("Win32"));
    assert_eq!(params["fingerprint"]["languages"], json!(["de-DE", "de"]));
    assert!(params.get("operatingSystem").is_none());

    let bare = WayfernConfig {
      fingerprint: Some(payload()),
      ..Default::default()
    };
    let params = conversion_params(&profile, &bare);
    assert!(params.get("identityId").is_none());
    assert!(params.get("identityBaseline").is_none());
    assert!(params.get("identityOverrides").is_none());
  }

  fn response(overrides: Value) -> Value {
    json!({
      "identityId": CURRENT_ID,
      "operatingSystem": "windows",
      "distance": 3.5,
      "verdict": "close",
      "usable": true,
      "overrides": overrides,
      "location": {
        "timezone": "Europe/Berlin",
        "language": "de-DE",
        "latitude": 52.52,
        "longitude": 13.405,
        "platform": "Win32"
      },
      "drawn": 40,
      "kept": false
    })
  }

  #[test]
  fn a_converted_profile_stores_only_the_identity_location_and_overrides() {
    let config = WayfernConfig {
      fingerprint: Some(payload()),
      identity_baseline: Some(payload()),
      geo_proxy_signature: Some("direct".to_string()),
      webrtc_mode: Some("tcp_only".to_string()),
      ..Default::default()
    };
    let profile = profile_with("152.0.7977.64", config);

    let converted = converted_profile(&profile, &response(json!({}))).unwrap();
    let stored = converted.wayfern_config.as_ref().unwrap();
    assert_eq!(stored.identity_id.as_deref(), Some(CURRENT_ID));
    assert_eq!(stored.os.as_deref(), Some("windows"));
    assert!(stored.fingerprint.is_none());
    assert!(stored.identity_baseline.is_none());
    assert!(stored.identity_overrides.is_none());
    assert_eq!(stored.geo_proxy_signature.as_deref(), Some("direct"));
    assert_eq!(stored.webrtc_mode.as_deref(), Some("tcp_only"));

    let location = WayfernManager::stored_object(stored.location.as_deref());
    assert_eq!(location.get("timezone"), Some(&json!("Europe/Berlin")));
    assert_eq!(location.get("latitude"), Some(&json!(52.52)));
    assert!(location.get("platform").is_none());

    let written = serde_json::to_string(stored).unwrap();
    assert!(!written.contains("\"fingerprint\""));
    assert!(!written.contains("\"identity_baseline\""));
    assert_eq!(
      stored_shape(stored),
      StoredShape::Identity,
      "a converted profile is never converted again"
    );

    let with_overrides =
      converted_profile(&profile, &response(json!({"hardwareConcurrency": 12}))).unwrap();
    let overrides = WayfernManager::stored_object(
      with_overrides
        .wayfern_config
        .as_ref()
        .unwrap()
        .identity_overrides
        .as_deref(),
    );
    assert_eq!(overrides.get("hardwareConcurrency"), Some(&json!(12)));
  }

  #[test]
  fn an_incomplete_answer_is_never_stored() {
    let profile = profile_with(
      "152.0.7977.64",
      WayfernConfig {
        fingerprint: Some(payload()),
        ..Default::default()
      },
    );
    let mut unusable = response(json!({}));
    unusable["usable"] = json!(false);
    assert!(converted_profile(&profile, &unusable).is_err());

    let mut old_answer = response(json!({}));
    old_answer.as_object_mut().unwrap().remove("kept");
    assert!(converted_profile(&profile, &old_answer).is_err());

    let mut no_id = response(json!({}));
    no_id["identityId"] = json!("");
    assert!(converted_profile(&profile, &no_id).is_err());

    let mut no_os = response(json!({}));
    no_os["operatingSystem"] = json!("beos");
    assert!(converted_profile(&profile, &no_os).is_err());
  }

  #[test]
  fn a_claim_off_the_host_keeps_the_profile_launchable_here() {
    let other = if get_host_os() == "macos" {
      "windows"
    } else {
      "macos"
    };
    let mut profile = profile_with(
      "152.0.7977.64",
      WayfernConfig {
        fingerprint: Some(payload()),
        ..Default::default()
      },
    );
    profile.host_os = None;
    assert!(!profile.is_cross_os());

    let mut answer = response(json!({}));
    answer["operatingSystem"] = json!(other);
    let converted = converted_profile(&profile, &answer).unwrap();
    assert_eq!(converted.host_os.as_deref(), Some(get_host_os().as_str()));
    assert!(!converted.is_cross_os());

    let mut mobile = response(json!({}));
    mobile["operatingSystem"] = json!("android");
    let converted = converted_profile(&profile, &mobile).unwrap();
    assert!(!converted.is_cross_os());

    let mut same = response(json!({}));
    same["operatingSystem"] = json!(get_host_os());
    let converted = converted_profile(&profile, &same).unwrap();
    assert!(converted.host_os.is_none());
    assert!(!converted.is_cross_os());
  }

  #[tokio::test]
  async fn a_launch_stops_a_conversion_and_waits_for_its_browser() {
    let alive = SESSION.lock().await;
    let launch_started = LAUNCH_STARTED.notified();
    assert!(!launch_pending());

    let launch = tokio::spawn(yield_to_launch());
    tokio::time::timeout(Duration::from_secs(5), launch_started)
      .await
      .expect("a starting launch interrupts the running conversion");
    assert!(launch_pending());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
      !launch.is_finished(),
      "the launch waits while the conversion browser is still alive"
    );

    drop(alive);
    let in_progress = tokio::time::timeout(Duration::from_secs(5), launch)
      .await
      .expect("the launch continues once the conversion browser is gone")
      .unwrap();
    assert!(launch_pending());
    drop(in_progress);
    assert!(!launch_pending());
  }

  #[test]
  fn protocol_refusals_are_permanent_and_everything_else_waits() {
    let invalid =
      r#"CDP error: {"code":-32602,"message":"fingerprint must be the stored device payload"}"#;
    assert!(matches!(classify(invalid), ConversionError::Failed(_)));

    let missing =
      r#"CDP error: {"code":-32601,"message":"'Wayfern.matchLegacyProfile' wasn't found"}"#;
    assert!(matches!(classify(missing), ConversionError::Failed(_)));

    let transient = r#"CDP error: {"code":-32000,"message":"Fingerprint authorization service is temporarily unavailable. Retry the command."}"#;
    match classify(transient) {
      ConversionError::Deferred(coded) => {
        assert!(coded.contains("WAYFERN_PLAN_CHECK_UNAVAILABLE"))
      }
      other => panic!("unexpected {other:?}"),
    }

    assert!(matches!(
      classify("No response received from CDP"),
      ConversionError::Deferred(_)
    ));
  }
}

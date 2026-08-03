//! Cookie-bot transport.
//!
//! The bot warms a profile's cookies overnight by driving it on a leased
//! remote host. NONE of that lives here: the schedule, the calendar maths, the
//! preset expansion, the site ordering, the dwell and scroll model, the pooled
//! budget and the nightly dispatcher are all held by donutbrowser-infra and
//! the Wayfern manager.
//!
//! This module is the wire only. It sends the user's own scalars — when to
//! run, for how long, which of their sites, which server-issued preset id —
//! and renders back what the server says happened. It deliberately keeps NO
//! local copy of a schedule: the server holds the only one, so two desktops
//! signed into one account cannot disagree about when the bot runs.

use crate::cloud_errors::{self, BackendFailure, FailureCodes};
use crate::profile::types::BrowserProfile;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

/// Operating systems the fleet can lease. Linux is refused by the manager, so
/// refusing it here turns a nightly failure at 02:00 into a refusal at the
/// moment the user picks the profile.
pub const BOT_PLATFORMS: [&str; 2] = ["windows", "macos"];

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Failure codes for the schedule routes.
const SCHEDULE_CODES: FailureCodes = FailureCodes {
  bad_request: "COOKIE_BOT_INVALID_SCHEDULE",
  forbidden: "COOKIE_BOT_NOT_ENTITLED",
  not_found: "COOKIE_BOT_NOT_ENROLLED",
  conflict: "COOKIE_BOT_SCHEDULE_CONFLICT",
};

/// Failure codes for the run routes.
const RUN_CODES: FailureCodes = FailureCodes {
  bad_request: "COOKIE_BOT_INVALID_SCHEDULE",
  forbidden: "COOKIE_BOT_NOT_ENTITLED",
  not_found: "COOKIE_BOT_RUN_NOT_FOUND",
  conflict: "COOKIE_BOT_RUN_IN_PROGRESS",
};

/// Failure codes for the read-only reporting routes.
const REPORT_CODES: FailureCodes = FailureCodes {
  bad_request: "COOKIE_BOT_INVALID_PERIOD",
  forbidden: "NOT_TEAM_MEMBER",
  not_found: cloud_errors::UNAVAILABLE,
  conflict: cloud_errors::UNAVAILABLE,
};

/// Every cookie-bot call fails as a code the frontend can translate.
///
/// There is no `Other(String)` carrying backend English: a raw message reaches
/// the user untranslated, which is the bug pattern the `{"code":…}` convention
/// exists to block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieBotError(pub BackendFailure);

impl CookieBotError {
  pub fn code(&self) -> &str {
    &self.0.code
  }

  pub fn status(&self) -> u16 {
    self.0.status
  }

  /// The `{"code":…,"params":{…}}` string a Tauri command returns.
  pub fn to_error_json(&self) -> String {
    self.0.to_error_json()
  }
}

impl std::fmt::Display for CookieBotError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.to_error_json())
  }
}

impl From<BackendFailure> for CookieBotError {
  fn from(failure: BackendFailure) -> Self {
    Self(failure)
  }
}

// --- Wire types -------------------------------------------------------------
//
// One place for every request and response shape, so a backend contract change
// is a single edit here rather than a hunt through call sites.

/// A profile enrolled in the nightly bot.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotSchedule {
  pub profile_id: String,
  pub profile_name: String,
  pub platform: String,
  pub enabled: bool,
  /// Minutes past local midnight the run is anchored to.
  pub run_at_minute: u16,
  /// Bitmask of local weekdays, bit 0 = Monday.
  pub days_mask: u8,
  pub timezone: String,
  /// Server-issued preset id. Opaque here — what it expands to is infra's.
  pub preset: String,
  pub max_minutes: u32,
  #[serde(default)]
  pub sites: Vec<String>,
  #[serde(default)]
  pub jitter_seconds: u32,

  // The profile facts the desktop declared, echoed back on every read. Kept so
  // the UI can tell that what the server believes about a profile no longer
  // matches what this machine can see, and re-declare it.
  #[serde(default)]
  pub sync_enabled: bool,
  #[serde(default)]
  pub encrypted_sync: bool,
  #[serde(default)]
  pub has_proxy: bool,
  #[serde(default)]
  pub touch_fingerprint: bool,
  #[serde(default)]
  pub sticky_exit: bool,
  /// When those facts were last refreshed.
  #[serde(default)]
  pub profile_state_at: Option<String>,

  /// Why tonight would be refused, or `None`.
  ///
  /// The server computes this on every read precisely so a broken enrolment is
  /// visible the moment it breaks. Dropping it meant a profile whose proxy was
  /// detached in the afternoon still showed a healthy row and a next-run time,
  /// and first announced itself with a skipped run at 02:00.
  #[serde(default)]
  pub blocked_by: Option<String>,

  #[serde(default)]
  pub next_run_at: Option<String>,
  #[serde(default)]
  pub last_run_at: Option<String>,
  #[serde(default)]
  pub last_run_id: Option<String>,
  #[serde(default)]
  pub owner_user_id: Option<String>,
  #[serde(default)]
  pub owner_email: Option<String>,
  #[serde(default)]
  pub updated_at: Option<String>,
}

/// What the desktop sends when enrolling or editing.
///
/// `next_run_at` is absent by design: the server recomputes it and ignores any
/// client value, so there is nothing here for two devices to disagree about.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotScheduleInput {
  pub profile_name: String,
  pub platform: String,
  pub enabled: bool,
  pub run_at_minute: u16,
  pub days_mask: u8,
  pub timezone: String,
  pub preset: String,
  pub max_minutes: u32,
  pub sites: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub jitter_seconds: Option<u32>,

  // The profile facts the server refuses a run on. It cannot read them itself —
  // the profile lives in the user's sync namespace, not in its database — so the
  // desktop reports them and the server decides.
  //
  // Every caller (Tauri, REST, MCP) overwrites all five through
  // `with_profile_state`, derived from the profile itself, so a caller can never
  // assert them. They are therefore `default` on the way IN — which is what the
  // GUI sends, and what the Tauri command's own argument deserialization
  // requires, since demanding them made every enrolment fail with
  // `invalid args schedule: missing field sync_enabled` before the stamping
  // could run — and unconditionally present on the way OUT, because the server
  // rejects a write that omits them.
  //
  // Defaulting is safe in exactly one direction: `bool::default()` is false, so
  // an unstamped input reads as "no sync, no proxy" and is REFUSED. The failure
  // this must never have is the opposite one, a defaulted `has_proxy: true`
  // warming a profile out of the fleet's own datacenter address.
  #[serde(default)]
  pub sync_enabled: bool,
  #[serde(default)]
  pub has_proxy: bool,
  #[serde(default)]
  pub encrypted_sync: bool,
  #[serde(default)]
  pub touch_fingerprint: bool,
  #[serde(default)]
  pub sticky_exit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotScheduleList {
  #[serde(default)]
  pub schedules: Vec<CookieBotSchedule>,
  #[serde(default)]
  pub team_id: Option<String>,
  #[serde(default)]
  pub scope: Option<String>,
}

/// A teammate's enrolment of the same profile.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotConflict {
  pub user_id: String,
  pub email: String,
  pub run_at_minute: u16,
  pub timezone: String,
  pub days_mask: u8,
  pub enabled: bool,
  /// Set on the dry-run check: the two enrolments share a weekday and fire
  /// within an hour of each other.
  #[serde(default)]
  pub overlaps: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotScheduleSaved {
  pub schedule: CookieBotSchedule,
  /// Repeated on a successful acknowledged write so the UI can keep showing
  /// the warning rather than pretending the collision went away.
  #[serde(default)]
  pub conflicts: Vec<CookieBotConflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotConflictCheck {
  pub profile_id: String,
  #[serde(default)]
  pub conflicts: Vec<CookieBotConflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotScheduleDeleted {
  pub profile_id: String,
  pub deleted: bool,
}

/// One night's work on one profile.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotRun {
  pub id: String,
  pub profile_id: String,
  #[serde(default)]
  pub profile_name: Option<String>,
  #[serde(default)]
  pub user_id: Option<String>,
  #[serde(default)]
  pub email: Option<String>,
  /// `schedule` or `manual`.
  #[serde(default)]
  pub team_id: Option<String>,
  pub trigger: String,
  /// `pending` | `running` | `succeeded` | `partial` | `failed` | `skipped` |
  /// `cancelled`.
  pub status: String,
  pub scheduled_for: String,
  /// The jittered instant the run was allowed to start.
  #[serde(default)]
  pub dispatch_after: Option<String>,
  #[serde(default)]
  pub started_at: Option<String>,
  #[serde(default)]
  pub ended_at: Option<String>,
  /// The night's whole budget, which may be split across several chunks.
  #[serde(default)]
  pub max_minutes: u32,
  /// How many browser sessions this night is split into, and which one is
  /// running. A night longer than one session's cap is checkpointed at each
  /// boundary, and "chunk 2 of 3" is the only honest way to report that.
  #[serde(default)]
  pub chunks_total: u32,
  #[serde(default)]
  pub chunk_index: u32,
  #[serde(default)]
  pub sites_total: u32,
  #[serde(default)]
  pub sites_visited: u32,
  #[serde(default)]
  pub sites_failed: u32,
  #[serde(default)]
  pub consent_dismissed: u32,
  #[serde(default)]
  pub billed_seconds: u64,
  /// Why it ended the way it did, e.g. `profile_locked`, `no_capacity`.
  #[serde(default)]
  pub outcome_code: Option<String>,
  #[serde(default)]
  pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotRunPage {
  #[serde(default)]
  pub runs: Vec<CookieBotRun>,
  /// Keyset cursor; `None` on the last page.
  #[serde(default)]
  pub next_before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotRunStarted {
  pub run: CookieBotRun,
  #[serde(default)]
  pub session_id: Option<String>,
}

/// A named intensity the user can pick. The client never learns what it
/// expands to; only enough to label the choice and show its rough cost.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotPreset {
  pub id: String,
  #[serde(default)]
  pub typical_minutes: Option<u32>,
  #[serde(default)]
  pub recommended: bool,
  /// Server-supplied English label, present only so a preset added after this
  /// build still renders. The UI must prefer its own `t()` key for a known id.
  #[serde(default)]
  pub name: Option<String>,
  #[serde(default)]
  pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotPresetList {
  #[serde(default)]
  pub presets: Vec<CookieBotPreset>,
  /// Which preset the server suggests when the user has expressed no
  /// preference.
  #[serde(default)]
  pub default_preset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RemoteHoursBreakdown {
  #[serde(default)]
  pub interactive_hours: f64,
  #[serde(default)]
  pub bot_hours: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RemoteHoursMember {
  pub user_id: String,
  pub email: String,
  #[serde(default)]
  pub role: Option<String>,
  #[serde(default)]
  pub used_hours: f64,
  #[serde(default)]
  pub interactive_hours: f64,
  #[serde(default)]
  pub bot_hours: f64,
}

/// The single pooled remote-hour budget. Bot and interactive hours share it;
/// the breakdown is reporting, never a sub-cap.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RemoteHoursQuota {
  pub granted_hours: f64,
  pub remaining_hours: f64,
  #[serde(default)]
  pub used_hours: f64,
  #[serde(default)]
  pub period_start: Option<String>,
  #[serde(default)]
  pub period_end: Option<String>,
  /// `user` or `team`.
  #[serde(default)]
  pub scope: Option<String>,
  #[serde(default)]
  pub team_id: Option<String>,
  #[serde(default)]
  pub seats: u32,
  #[serde(default)]
  pub per_seat_hours: f64,
  #[serde(default)]
  pub breakdown: Option<RemoteHoursBreakdown>,
  /// The full roster for an owner or admin; just the caller otherwise.
  #[serde(default)]
  pub members: Vec<RemoteHoursMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotUsageMember {
  pub user_id: String,
  pub email: String,
  #[serde(default)]
  pub role: Option<String>,
  #[serde(default)]
  pub interactive_hours: f64,
  #[serde(default)]
  pub bot_hours: f64,
  #[serde(default)]
  pub used_hours: f64,
  #[serde(default)]
  pub sessions: u32,
  #[serde(default)]
  pub bot_runs: u32,
  #[serde(default)]
  pub bot_runs_failed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotUsageProfile {
  pub profile_id: String,
  #[serde(default)]
  pub profile_name: Option<String>,
  #[serde(default)]
  pub owner_email: Option<String>,
  #[serde(default)]
  pub bot_hours: f64,
  #[serde(default)]
  pub runs: u32,
  /// How many of those runs did not do what they were asked. Its sibling on the
  /// member view is `bot_runs_failed`; both are on the wire and both belong in
  /// the report.
  #[serde(default)]
  pub runs_failed: u32,
  #[serde(default)]
  pub last_run_at: Option<String>,
  #[serde(default)]
  pub last_status: Option<String>,
}

/// The team owner's after-the-fact view of who spent what.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotUsage {
  pub period: String,
  #[serde(default)]
  pub period_start: Option<String>,
  #[serde(default)]
  pub period_end: Option<String>,
  #[serde(default)]
  pub team_id: Option<String>,
  #[serde(default)]
  pub seats: u32,
  #[serde(default)]
  pub granted_hours: f64,
  #[serde(default)]
  pub used_hours: f64,
  #[serde(default)]
  pub remaining_hours: f64,
  #[serde(default)]
  pub members: Vec<CookieBotUsageMember>,
  #[serde(default)]
  pub profiles: Vec<CookieBotUsageProfile>,
}

// --- Client-side preconditions ---------------------------------------------

/// Whether this profile could ever be warmed by the bot.
///
/// The server is authoritative — it re-checks all of this and owns the parts
/// the client cannot see — but a profile that can never qualify should never
/// reach a confirm dialog, an hour of quota or a leased host. Returns the
/// `{"code":…}` string a Tauri command surfaces directly.
pub fn bot_precondition(profile: &BrowserProfile) -> Result<(), String> {
  if !profile.is_sync_enabled() {
    // The host materialises the profile by pulling it from donut-sync. A
    // local-only profile has nothing there, so there is no path to a run.
    return Err(error("COOKIE_BOT_REQUIRES_CLOUD_SYNC", &[]));
  }
  if profile.is_encrypted_sync() {
    // The key never leaves this machine, so the host would launch Chromium on
    // ciphertext and push the corruption back over the real profile.
    return Err(error("COOKIE_BOT_ENCRYPTED_SYNC_UNSUPPORTED", &[]));
  }
  let Some(platform) = profile.resolved_os() else {
    return Err(error("COOKIE_BOT_UNKNOWN_PLATFORM", &[]));
  };
  if !BOT_PLATFORMS.contains(&platform) {
    return Err(error(
      "COOKIE_BOT_UNSUPPORTED_PLATFORM",
      &[("platform", platform)],
    ));
  }
  if profile.proxy_id.is_none() && profile.vpn_id.is_none() {
    // Without one the run egresses from the fleet's own datacenter address.
    // Hours of traffic from a hosting ASN is worse for the profile's identity
    // than not warming it at all.
    return Err(error("COOKIE_BOT_REQUIRES_EXIT_NODE", &[]));
  }
  Ok(())
}

/// The profile facts the server needs but cannot see.
///
/// The server holds the schedule; the PROFILE lives in the user's sync
/// namespace, so `sync_enabled`, `has_proxy` and the rest are only knowable
/// here. It requires them on every write rather than defaulting them, because
/// a defaulted `has_proxy` is a profile warmed out of the fleet's own
/// datacenter address.
///
/// Derived in one place so the Tauri, REST and MCP call sites cannot drift into
/// three different answers about the same profile.
pub fn profile_state(profile: &BrowserProfile) -> ProfileState {
  ProfileState {
    sync_enabled: profile.is_sync_enabled(),
    encrypted_sync: profile.is_encrypted_sync(),
    // A VPN is an exit node just as much as a proxy is; the server only asks
    // whether the traffic leaves through something the user brought.
    has_proxy: profile.proxy_id.is_some() || profile.vpn_id.is_some(),
    // Always false: this data model has no mobile/touch profile. `resolved_os`
    // yields only windows, macos or linux, and `bot_precondition` already
    // refuses everything but the first two. Reported rather than omitted so the
    // server keeps one required shape, and it stays authoritative — it sees the
    // real fingerprint on the host and can still refuse a run this cannot know
    // to reject.
    touch_fingerprint: false,
    // A VPN is one persistent tunnel, so the night's chunks share an exit. A
    // stored proxy may rotate per connection, and claiming stickiness we cannot
    // guarantee is worse than declining it: the server's fallback is to run the
    // night as a single chunk, which is the safe answer either way.
    sticky_exit: profile.vpn_id.is_some(),
  }
}

/// What {@link profile_state} derives. Applied onto a schedule input before it
/// is sent.
#[derive(Debug, Clone, Copy)]
pub struct ProfileState {
  pub sync_enabled: bool,
  pub encrypted_sync: bool,
  pub has_proxy: bool,
  pub touch_fingerprint: bool,
  pub sticky_exit: bool,
}

impl CookieBotScheduleInput {
  /// Stamp the profile facts onto an input built from user-chosen values.
  pub fn with_profile_state(mut self, state: ProfileState) -> Self {
    self.sync_enabled = state.sync_enabled;
    self.encrypted_sync = state.encrypted_sync;
    self.has_proxy = state.has_proxy;
    self.touch_fingerprint = state.touch_fingerprint;
    self.sticky_exit = state.sticky_exit;
    self
  }
}

fn error(code: &str, params: &[(&str, &str)]) -> String {
  let mut object = serde_json::Map::new();
  object.insert(
    "code".to_string(),
    serde_json::Value::String(code.to_string()),
  );
  if !params.is_empty() {
    let map = params
      .iter()
      .map(|(k, v)| {
        (
          (*k).to_string(),
          serde_json::Value::String((*v).to_string()),
        )
      })
      .collect::<serde_json::Map<_, _>>();
    object.insert("params".to_string(), serde_json::Value::Object(map));
  }
  serde_json::Value::Object(object).to_string()
}

// --- Routes -----------------------------------------------------------------

fn base() -> String {
  format!("{}/api/cookie-bot", crate::cloud_auth::CLOUD_API_URL)
}

/// Every enrolment the caller can see.
pub async fn list_schedules(scope: Option<&str>) -> Result<CookieBotScheduleList, CookieBotError> {
  let query = scope
    .map(|s| vec![("scope".to_string(), s.to_string())])
    .unwrap_or_default();
  request(
    reqwest::Method::GET,
    format!("{}/schedules", base()),
    query,
    None,
    SCHEDULE_CODES,
  )
  .await
}

/// This profile's enrolment, or `None` when there is none.
///
/// "Not enrolled" is a state the UI renders, not a failure it reports, so the
/// 404 is folded into `Ok(None)` here rather than at every call site.
pub async fn get_schedule(profile_id: &str) -> Result<Option<CookieBotSchedule>, CookieBotError> {
  let result: Result<ScheduleEnvelope, CookieBotError> = request(
    reqwest::Method::GET,
    format!("{}/schedules/{}", base(), urlencoding::encode(profile_id)),
    Vec::new(),
    None,
    SCHEDULE_CODES,
  )
  .await;

  match result {
    Ok(envelope) => Ok(Some(envelope.schedule)),
    Err(e) if e.code() == "COOKIE_BOT_NOT_ENROLLED" => Ok(None),
    Err(e) => Err(e),
  }
}

#[derive(Debug, Deserialize)]
struct ScheduleEnvelope {
  schedule: CookieBotSchedule,
}

/// Create or replace this profile's enrolment.
///
/// `acknowledge_conflict` is the second half of a two-step write: the first
/// PUT is refused with the teammate's name and time, and the same PUT with the
/// flag set goes through. Two operators colliding into a silent nightly 409 is
/// exactly what that costs to avoid.
pub async fn save_schedule(
  profile_id: &str,
  input: &CookieBotScheduleInput,
  acknowledge_conflict: bool,
) -> Result<CookieBotScheduleSaved, CookieBotError> {
  let mut body = serde_json::to_value(input).map_err(|e| {
    CookieBotError(cloud_errors::transport_failure(&format!(
      "encode schedule: {e}"
    )))
  })?;
  if let Some(object) = body.as_object_mut() {
    object.insert(
      "acknowledge_conflict".to_string(),
      serde_json::Value::Bool(acknowledge_conflict),
    );
  }

  request(
    reqwest::Method::PUT,
    format!("{}/schedules/{}", base(), urlencoding::encode(profile_id)),
    Vec::new(),
    Some(body),
    SCHEDULE_CODES,
  )
  .await
}

/// Re-declare the profile facts the server cannot observe for itself.
///
/// Narrow on purpose: a detached proxy should not have to resend a whole
/// schedule and risk clobbering an edit made from another device in between.
pub async fn update_profile_state(
  profile_id: &str,
  state: ProfileState,
  timezone: Option<&str>,
) -> Result<CookieBotSchedule, CookieBotError> {
  let mut body = serde_json::Map::new();
  body.insert("sync_enabled".to_string(), state.sync_enabled.into());
  body.insert("encrypted_sync".to_string(), state.encrypted_sync.into());
  body.insert("has_proxy".to_string(), state.has_proxy.into());
  body.insert(
    "touch_fingerprint".to_string(),
    state.touch_fingerprint.into(),
  );
  body.insert("sticky_exit".to_string(), state.sticky_exit.into());
  if let Some(zone) = timezone {
    body.insert("timezone".to_string(), zone.into());
  }

  let envelope: ScheduleEnvelope = request(
    reqwest::Method::POST,
    format!(
      "{}/schedules/{}/profile-state",
      base(),
      urlencoding::encode(profile_id)
    ),
    Vec::new(),
    Some(serde_json::Value::Object(body)),
    SCHEDULE_CODES,
  )
  .await?;
  Ok(envelope.schedule)
}

/// Push this profile's current facts to the server, without blocking the edit
/// that changed them.
///
/// The server refuses a run on the copy the desktop last declared —
/// `has_proxy: false` is `proxy_required`, and that check exists because a run
/// without an exit node egresses from the leased host's own datacenter address.
/// Nothing but a full schedule write refreshed that copy, so detaching a proxy
/// from an enrolled profile left `has_proxy: true` on the row and the night ran
/// anyway. This closes that gap at the moment the profile changes.
///
/// Silent on failure by design: a profile edit must not fail because the cloud
/// is unreachable, an unenrolled profile answers `COOKIE_BOT_NOT_ENROLLED`
/// which is the normal case, and the next edit re-declares the same facts.
pub fn report_profile_state(profile: &BrowserProfile) {
  let profile_id = profile.id.to_string();
  let state = profile_state(profile);
  tauri::async_runtime::spawn(async move {
    if !crate::cloud_auth::CLOUD_AUTH.is_logged_in().await {
      return;
    }
    match update_profile_state(&profile_id, state, None).await {
      Ok(_) => {
        log::debug!("Re-declared cookie-bot profile state for {profile_id}");
      }
      // Not enrolled is the common answer and not worth a log line at warn.
      Err(e) if e.code() == "COOKIE_BOT_NOT_ENROLLED" => {}
      Err(e) => {
        log::warn!("Could not re-declare cookie-bot profile state for {profile_id}: {e}");
      }
    }
  });
}

/// Turn the bot off for this profile. Safe to repeat: deleting an enrolment
/// that is already gone succeeds with `deleted: false`.
pub async fn delete_schedule(profile_id: &str) -> Result<CookieBotScheduleDeleted, CookieBotError> {
  request(
    reqwest::Method::DELETE,
    format!("{}/schedules/{}", base(), urlencoding::encode(profile_id)),
    Vec::new(),
    None,
    SCHEDULE_CODES,
  )
  .await
}

/// Ask, without writing anything, who else already warms this profile.
pub async fn check_conflicts(
  profile_id: &str,
  run_at_minute: Option<u16>,
  timezone: Option<&str>,
  days_mask: Option<u8>,
) -> Result<CookieBotConflictCheck, CookieBotError> {
  let mut query = vec![("profile_id".to_string(), profile_id.to_string())];
  if let Some(minute) = run_at_minute {
    query.push(("run_at_minute".to_string(), minute.to_string()));
  }
  if let Some(zone) = timezone {
    query.push(("timezone".to_string(), zone.to_string()));
  }
  if let Some(mask) = days_mask {
    query.push(("days_mask".to_string(), mask.to_string()));
  }

  request(
    reqwest::Method::GET,
    format!("{}/conflicts", base()),
    query,
    None,
    SCHEDULE_CODES,
  )
  .await
}

/// One page of run history, newest first.
pub async fn list_runs(
  profile_id: Option<&str>,
  scope: Option<&str>,
  limit: Option<u32>,
  before: Option<&str>,
) -> Result<CookieBotRunPage, CookieBotError> {
  let mut query = Vec::new();
  if let Some(id) = profile_id {
    query.push(("profile_id".to_string(), id.to_string()));
  }
  if let Some(s) = scope {
    query.push(("scope".to_string(), s.to_string()));
  }
  if let Some(n) = limit {
    query.push(("limit".to_string(), n.to_string()));
  }
  if let Some(cursor) = before {
    query.push(("before".to_string(), cursor.to_string()));
  }

  request(
    reqwest::Method::GET,
    format!("{}/runs", base()),
    query,
    None,
    RUN_CODES,
  )
  .await
}

/// Start a run now instead of waiting for tonight.
///
/// The preset and the site list come from the stored schedule, so this carries
/// no behaviour of its own — an unenrolled profile is a 404, not an implicit
/// enrolment with client-chosen defaults.
pub async fn run_now(
  profile_id: &str,
  max_minutes: Option<u32>,
) -> Result<CookieBotRunStarted, CookieBotError> {
  let mut body = serde_json::Map::new();
  body.insert(
    "profile_id".to_string(),
    serde_json::Value::String(profile_id.to_string()),
  );
  if let Some(minutes) = max_minutes {
    body.insert("max_minutes".to_string(), serde_json::Value::from(minutes));
  }

  request(
    reqwest::Method::POST,
    format!("{}/runs", base()),
    Vec::new(),
    Some(serde_json::Value::Object(body)),
    RUN_CODES,
  )
  .await
}

/// Stop a run that is still going.
///
/// A 503 here means the fleet could not be reached and the browser is still
/// up, so the run stays `running` rather than being marked cancelled under a
/// live browser — retiring a row while something is still writing the cookie
/// jar is the two-writer case the profile lock exists to prevent.
pub async fn cancel_run(run_id: &str) -> Result<CookieBotRun, CookieBotError> {
  let envelope: RunEnvelope = request(
    reqwest::Method::DELETE,
    format!("{}/runs/{}", base(), urlencoding::encode(run_id)),
    Vec::new(),
    None,
    RUN_CODES,
  )
  .await?;
  Ok(envelope.run)
}

#[derive(Debug, Deserialize)]
struct RunEnvelope {
  run: CookieBotRun,
}

/// The intensities the server offers today.
pub async fn list_presets() -> Result<CookieBotPresetList, CookieBotError> {
  request(
    reqwest::Method::GET,
    format!("{}/presets", base()),
    Vec::new(),
    None,
    REPORT_CODES,
  )
  .await
}

/// Per-member and per-profile spend for a calendar month (`YYYY-MM`).
pub async fn team_usage(period: Option<&str>) -> Result<CookieBotUsage, CookieBotError> {
  let query = period
    .map(|p| vec![("period".to_string(), p.to_string())])
    .unwrap_or_default();
  request(
    reqwest::Method::GET,
    format!("{}/usage", base()),
    query,
    None,
    REPORT_CODES,
  )
  .await
}

/// The pooled remote-hour budget.
///
/// Being refused a launch must not be the only way to learn a limit exists,
/// which is what this route has been for as long as nothing called it.
pub async fn remote_hours_quota() -> Result<RemoteHoursQuota, CookieBotError> {
  request(
    reqwest::Method::GET,
    format!(
      "{}/api/remote-sessions/quota",
      crate::cloud_auth::CLOUD_API_URL
    ),
    Vec::new(),
    None,
    REPORT_CODES,
  )
  .await
}

// --- Transport --------------------------------------------------------------

fn http() -> &'static reqwest::Client {
  static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
  CLIENT.get_or_init(|| {
    reqwest::Client::builder()
      .timeout(REQUEST_TIMEOUT)
      .connect_timeout(CONNECT_TIMEOUT)
      .build()
      .unwrap_or_else(|_| reqwest::Client::new())
  })
}

/// Append a percent-encoded query string.
///
/// Built here rather than left to the HTTP client so a profile id or a keyset
/// cursor containing a `&` cannot smuggle a second parameter into the request.
fn with_query(url: &str, query: &[(String, String)]) -> String {
  if query.is_empty() {
    return url.to_string();
  }
  let encoded = query
    .iter()
    .map(|(key, value)| {
      format!(
        "{}={}",
        urlencoding::encode(key),
        urlencoding::encode(value)
      )
    })
    .collect::<Vec<_>>()
    .join("&");
  let separator = if url.contains('?') { '&' } else { '?' };
  format!("{url}{separator}{encoded}")
}

/// One request, one place.
///
/// Goes through `api_call_with_retry` so an expired access token is refreshed
/// and the call retried once — otherwise a user whose token aged out overnight
/// sees "not signed in" on a machine that is signed in.
async fn request<T: DeserializeOwned>(
  method: reqwest::Method,
  url: String,
  query: Vec<(String, String)>,
  body: Option<serde_json::Value>,
  codes: FailureCodes,
) -> Result<T, CookieBotError> {
  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let method = method.clone();
      let url = url.clone();
      let query = query.clone();
      let body = body.clone();
      async move {
        let url = with_query(&url, &query);
        let mut builder = http().request(method, &url).bearer_auth(token);
        if let Some(payload) = body {
          builder = builder.json(&payload);
        }

        let response = builder
          .send()
          .await
          .map_err(|e| format!("reach backend: {e}"))?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
          let text = response.text().await.unwrap_or_default();
          // Encode the status so api_call_with_retry can spot a 401 and
          // classify_message can recover the code afterwards.
          return Err(format!("({status}) {text}"));
        }

        response
          .json::<T>()
          .await
          .map_err(|e| format!("decode response: {e}"))
      }
    })
    .await
    .map_err(|e| CookieBotError(cloud_errors::classify_message(&e, codes)))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::profile::types::SyncMode;

  fn eligible_profile() -> BrowserProfile {
    BrowserProfile {
      id: uuid::Uuid::nil(),
      name: "warm me".to_string(),
      browser: "wayfern".to_string(),
      version: "latest".to_string(),
      sync_mode: SyncMode::Regular,
      host_os: Some("macos".to_string()),
      proxy_id: Some("proxy-1".to_string()),
      ..Default::default()
    }
  }

  #[test]
  fn the_profile_facts_the_server_requires_are_derived_from_the_profile() {
    let state = profile_state(&eligible_profile());

    assert!(state.sync_enabled);
    assert!(!state.encrypted_sync);
    assert!(state.has_proxy);
    assert!(!state.touch_fingerprint);
    // A stored proxy is not claimed to be sticky — only a VPN tunnel is.
    assert!(!state.sticky_exit);
  }

  #[test]
  fn a_vpn_counts_as_the_exit_node_and_as_a_sticky_one() {
    // `has_proxy` asks whether the user brought an exit, not whether that exit
    // is specifically a stored proxy. A VPN-only profile that reported false
    // would be refused a run it is perfectly entitled to.
    let profile = BrowserProfile {
      proxy_id: None,
      vpn_id: Some("vpn-1".to_string()),
      ..eligible_profile()
    };

    let state = profile_state(&profile);

    assert!(state.has_proxy);
    assert!(state.sticky_exit);
  }

  #[test]
  fn a_profile_with_no_exit_reports_none() {
    let profile = BrowserProfile {
      proxy_id: None,
      vpn_id: None,
      ..eligible_profile()
    };

    assert!(!profile_state(&profile).has_proxy);
  }

  fn code_of(err: &str) -> String {
    serde_json::from_str::<serde_json::Value>(err)
      .expect("a precondition failure must be a JSON error envelope")["code"]
      .as_str()
      .expect("the envelope must name a code")
      .to_string()
  }

  #[test]
  fn a_local_only_profile_has_no_path_to_a_run() {
    // The host obtains the profile from donut-sync. Without sync there is
    // nothing to pull, so the run would warm an empty browser and then push
    // that emptiness over the user's real profile.
    let mut profile = eligible_profile();
    profile.sync_mode = SyncMode::Disabled;
    let err = bot_precondition(&profile).expect_err("a local-only profile must be refused");
    assert_eq!(code_of(&err), "COOKIE_BOT_REQUIRES_CLOUD_SYNC");
  }

  #[test]
  fn end_to_end_encrypted_sync_is_refused_with_its_own_code() {
    // Distinct from "turn sync on": the fix is to switch to Regular sync, and
    // one code cannot carry two different instructions.
    let mut profile = eligible_profile();
    profile.sync_mode = SyncMode::Encrypted;
    let err = bot_precondition(&profile).expect_err("encrypted sync must be refused");
    assert_eq!(code_of(&err), "COOKIE_BOT_ENCRYPTED_SYNC_UNSUPPORTED");
  }

  #[test]
  fn linux_is_refused_at_enrolment_rather_than_at_two_in_the_morning() {
    let mut profile = eligible_profile();
    profile.host_os = Some("linux".to_string());
    let err = bot_precondition(&profile).expect_err("linux has no host to lease");
    let parsed: serde_json::Value = serde_json::from_str(&err).expect("valid envelope");
    assert_eq!(parsed["code"], "COOKIE_BOT_UNSUPPORTED_PLATFORM");
    assert_eq!(
      parsed["params"]["platform"], "linux",
      "the message must name the platform that cannot run"
    );
  }

  #[test]
  fn a_profile_with_no_recorded_os_cannot_be_scheduled_onto_a_host() {
    let mut profile = eligible_profile();
    profile.host_os = None;
    let err = bot_precondition(&profile).expect_err("no OS means no matching host");
    assert_eq!(code_of(&err), "COOKIE_BOT_UNKNOWN_PLATFORM");
  }

  #[test]
  fn a_run_without_a_proxy_or_vpn_is_refused() {
    // Hours of overnight traffic from a hosting ASN damages the profile's
    // identity more than not warming it would.
    let mut profile = eligible_profile();
    profile.proxy_id = None;
    profile.vpn_id = None;
    let err = bot_precondition(&profile).expect_err("datacenter egress must be refused");
    assert_eq!(code_of(&err), "COOKIE_BOT_REQUIRES_EXIT_NODE");
  }

  #[test]
  fn a_vpn_satisfies_the_exit_node_requirement_just_as_a_proxy_does() {
    let mut profile = eligible_profile();
    profile.proxy_id = None;
    profile.vpn_id = Some("vpn-1".to_string());
    assert!(bot_precondition(&profile).is_ok());
  }

  #[test]
  fn a_windows_profile_with_sync_and_a_proxy_qualifies() {
    let mut profile = eligible_profile();
    profile.host_os = Some("windows".to_string());
    assert!(bot_precondition(&profile).is_ok());
  }

  /// A verbatim `CookieBotScheduleView`, field for field, as `toScheduleView`
  /// in donutbrowser-infra's `cookie-bot.service.ts` builds it.
  const SERVER_SCHEDULE_VIEW: &str = r#"{
    "profile_id":"p1","profile_name":"Yu","platform":"macos","enabled":true,
    "run_at_minute":120,"days_mask":127,"timezone":"Europe/Berlin",
    "preset":"balanced","max_minutes":45,"sites":["https://example.com"],
    "jitter_seconds":900,"sync_enabled":true,"encrypted_sync":false,
    "has_proxy":true,"touch_fingerprint":false,"sticky_exit":false,
    "profile_state_at":"2026-08-03T09:00:00.000Z","next_run_at":"2026-08-04T00:00:00.000Z",
    "last_run_at":null,"last_run_id":null,"blocked_by":null,"owner_user_id":"u1",
    "owner_email":"a@example.com","updated_at":"2026-08-03T10:00:00.000Z"
  }"#;

  #[test]
  fn the_schedule_payload_matches_what_the_backend_sends() {
    // Pinned against the Schedule shape in donutbrowser-infra's
    // cookie-bot controller. A field name that drifts makes every read fail
    // at the decode step, which surfaces as "something went wrong" with no
    // hint that the contract moved.
    let schedule: CookieBotSchedule = serde_json::from_str(SERVER_SCHEDULE_VIEW)
      .expect("the backend's schedule payload must deserialize");

    assert_eq!(schedule.run_at_minute, 120);
    assert_eq!(schedule.days_mask, 127);
    assert_eq!(schedule.max_minutes, 45);
    assert_eq!(schedule.sites, vec!["https://example.com".to_string()]);
    assert!(schedule.last_run_at.is_none());
    // The declared facts, echoed back. Dropped, they left the UI unable to see
    // that the server's copy of a profile no longer matched this machine's.
    assert!(schedule.sync_enabled);
    assert!(schedule.has_proxy);
    assert!(!schedule.encrypted_sync);
    assert!(schedule.profile_state_at.is_some());
    assert!(schedule.blocked_by.is_none());
  }

  #[test]
  fn a_broken_enrolment_carries_the_reason_it_cannot_run() {
    // The whole point of `blocked_by`: a profile whose proxy was detached in
    // the afternoon should not first announce itself with a skipped run at
    // 02:00. Dropping the field made the enrolment look healthy until it
    // failed.
    let schedule: CookieBotSchedule = serde_json::from_str(
      &SERVER_SCHEDULE_VIEW
        .replace("\"has_proxy\":true", "\"has_proxy\":false")
        .replace("\"blocked_by\":null", "\"blocked_by\":\"proxy_required\""),
    )
    .expect("a blocked schedule must deserialize");

    assert!(!schedule.has_proxy);
    assert_eq!(schedule.blocked_by.as_deref(), Some("proxy_required"));
  }

  #[test]
  fn a_schedule_missing_every_optional_field_still_decodes() {
    // A freshly created enrolment has never run, so the backend omits or
    // nulls half the object. Failing to decode that would make the enrolment
    // the user just made look broken.
    let schedule: CookieBotSchedule = serde_json::from_str(
      r#"{"profile_id":"p1","profile_name":"Yu","platform":"windows","enabled":false,
          "run_at_minute":0,"days_mask":1,"timezone":"UTC","preset":"light",
          "max_minutes":5}"#,
    )
    .expect("a never-run schedule must deserialize");
    assert!(schedule.sites.is_empty());
    assert_eq!(schedule.jitter_seconds, 0);
    assert!(schedule.next_run_at.is_none());
  }

  #[test]
  fn the_run_payload_matches_what_the_backend_sends() {
    // Verbatim `CookieBotRunView`, as `toRunViews` builds it. `max_minutes`,
    // `chunks_total`, `chunk_index`, `dispatch_after` and `team_id` were all
    // already on the wire and all silently discarded, so a multi-chunk night
    // could not be reported as one.
    let page: CookieBotRunPage = serde_json::from_str(
      r#"{"runs":[{"id":"r1","profile_id":"p1","profile_name":"Yu","user_id":"u1",
          "email":"a@example.com","team_id":"t1","trigger":"schedule","status":"running",
          "scheduled_for":"2026-08-03T00:00:00.000Z",
          "dispatch_after":"2026-08-03T00:07:30.000Z","started_at":"2026-08-03T00:08:00.000Z",
          "ended_at":null,"max_minutes":180,"chunks_total":3,"chunk_index":2,
          "sites_total":12,"sites_visited":11,
          "sites_failed":1,"consent_dismissed":4,"billed_seconds":2220,
          "outcome_code":null,"session_id":"s1"}],"next_before":null}"#,
    )
    .expect("the backend's run page must deserialize");

    let run = &page.runs[0];
    assert_eq!(run.status, "running");
    assert_eq!(run.billed_seconds, 2220);
    assert_eq!(run.sites_visited, 11);
    assert_eq!(run.team_id.as_deref(), Some("t1"));
    assert_eq!(run.max_minutes, 180);
    assert_eq!(run.chunks_total, 3);
    assert_eq!(run.chunk_index, 2);
    assert!(run.dispatch_after.is_some());
    assert!(page.next_before.is_none());
  }

  #[test]
  fn the_usage_report_keeps_the_per_profile_failure_count() {
    // `runs_failed` sits beside `runs` on the wire and is the one number that
    // answers "is this enrolment actually working?" in the team dashboard.
    let usage: CookieBotUsage = serde_json::from_str(
      r#"{"period":"2026-08","period_start":"2026-08-01T00:00:00.000Z",
          "period_end":"2026-09-01T00:00:00.000Z","team_id":"t1","seats":2,
          "granted_hours":400,"used_hours":12.5,"remaining_hours":387.5,
          "members":[],
          "profiles":[{"profile_id":"p1","profile_name":"Yu","owner_email":"a@example.com",
            "bot_hours":12.5,"runs":9,"runs_failed":4,
            "last_run_at":"2026-08-03T00:41:00.000Z","last_status":"failed"}]}"#,
    )
    .expect("the usage report must deserialize");

    assert_eq!(usage.profiles[0].runs, 9);
    assert_eq!(usage.profiles[0].runs_failed, 4);
  }

  #[test]
  fn the_profile_state_body_declares_every_fact_the_server_gates_on() {
    // The narrow re-declaration route. `has_proxy` false is `proxy_required`
    // server-side, so an omitted field silently keeps the stale value and the
    // night runs unproxied.
    let state = profile_state(&eligible_profile());
    let body = serde_json::json!({
      "sync_enabled": state.sync_enabled,
      "encrypted_sync": state.encrypted_sync,
      "has_proxy": state.has_proxy,
      "touch_fingerprint": state.touch_fingerprint,
      "sticky_exit": state.sticky_exit,
    });
    for key in [
      "sync_enabled",
      "encrypted_sync",
      "has_proxy",
      "touch_fingerprint",
      "sticky_exit",
    ] {
      assert!(
        body.get(key).is_some_and(serde_json::Value::is_boolean),
        "{key} must be declared, not left to the server's stale copy"
      );
    }
  }

  #[test]
  fn the_gui_payload_deserialises_without_the_facts_the_command_stamps() {
    // This is exactly what `saveCookieBotSchedule` in src/lib/cookie-bot.ts
    // sends: the user's own scalars and nothing else. Requiring the profile
    // facts here made Tauri reject the argument with
    // `invalid args schedule: missing field sync_enabled` before
    // `with_profile_state` ever ran, so every enrolment from the GUI failed
    // while REST and MCP — which build the struct in Rust — worked.
    let input: CookieBotScheduleInput = serde_json::from_str(
      r#"{"profile_name":"Yu","platform":"macos","enabled":true,"run_at_minute":120,
          "days_mask":127,"timezone":"Europe/Berlin","preset":"balanced",
          "max_minutes":45,"sites":["https://example.com"]}"#,
    )
    .expect("the frontend's schedule payload must deserialize");

    // Fail-closed: an input nobody stamped claims no sync and no exit node, and
    // the server refuses both. The dangerous default would be the other way.
    assert!(!input.sync_enabled);
    assert!(!input.has_proxy);

    let stamped = input.with_profile_state(profile_state(&eligible_profile()));
    assert!(stamped.sync_enabled);
    assert!(stamped.has_proxy);
  }

  #[test]
  fn a_schedule_input_serialises_without_a_next_run_at() {
    // The server always recomputes the next fire instant. Sending one would
    // invite a client and a server that disagree about when the bot runs.
    let input = CookieBotScheduleInput {
      profile_name: "Yu".to_string(),
      platform: "macos".to_string(),
      enabled: true,
      run_at_minute: 120,
      days_mask: 127,
      timezone: "Europe/Berlin".to_string(),
      preset: "balanced".to_string(),
      max_minutes: 45,
      sites: vec!["https://example.com".to_string()],
      jitter_seconds: None,
      ..Default::default()
    };
    // The server rejects a write missing either of these with
    // COOKIE_BOT_INVALID_SCHEDULE, so they must be on the wire unconditionally
    // — `skip_serializing_if` on them would make every enrolment a 400.
    let encoded = serde_json::to_value(&input).expect("input must serialize");
    assert!(encoded.get("sync_enabled").is_some());
    assert!(encoded.get("has_proxy").is_some());
    assert!(encoded.get("next_run_at").is_none());
    assert!(
      encoded.get("jitter_seconds").is_none(),
      "an unset jitter must be omitted so the server's default applies"
    );
  }

  #[test]
  fn the_quota_payload_survives_a_backend_that_only_sends_the_original_two_keys() {
    // The route predates this feature and returned only these two fields.
    // A deployment that has not rolled forward must still render a budget.
    let quota: RemoteHoursQuota =
      serde_json::from_str(r#"{"granted_hours":200,"remaining_hours":187.25}"#)
        .expect("the legacy quota payload must deserialize");
    assert_eq!(quota.granted_hours, 200.0);
    assert_eq!(quota.seats, 0);
    assert!(quota.members.is_empty());
  }

  #[test]
  fn a_pooled_team_quota_decodes_its_roster() {
    let quota: RemoteHoursQuota = serde_json::from_str(
      r#"{"granted_hours":600,"remaining_hours":0,"used_hours":612.75,"scope":"team",
          "team_id":"t1","seats":3,"per_seat_hours":200,
          "breakdown":{"interactive_hours":100.5,"bot_hours":512.25},
          "members":[{"user_id":"u1","email":"a@example.com","role":"owner",
                      "used_hours":400,"interactive_hours":90,"bot_hours":310}]}"#,
    )
    .expect("the pooled quota payload must deserialize");

    // used_hours is deliberately unclamped while remaining_hours is: an
    // over-spent team must be able to see how far over it went.
    assert_eq!(quota.used_hours, 612.75);
    assert_eq!(quota.remaining_hours, 0.0);
    assert_eq!(quota.seats, 3);
    assert_eq!(quota.members.len(), 1);
    assert_eq!(
      quota.breakdown.map(|b| b.bot_hours),
      Some(512.25),
      "the bot/interactive split is reporting only, but it must survive the wire"
    );
  }

  #[test]
  fn a_conflict_response_carries_the_teammate_the_ui_has_to_name() {
    let saved: CookieBotScheduleSaved = serde_json::from_str(
      r#"{"schedule":{"profile_id":"p1","profile_name":"Yu","platform":"macos",
            "enabled":true,"run_at_minute":120,"days_mask":127,"timezone":"UTC",
            "preset":"deep","max_minutes":90},
          "conflicts":[{"user_id":"u2","email":"alex@example.com","run_at_minute":120,
            "timezone":"UTC","days_mask":127,"enabled":true}]}"#,
    )
    .expect("an acknowledged write must still report the conflict");
    assert_eq!(saved.conflicts[0].email, "alex@example.com");
    assert!(!saved.conflicts[0].overlaps);
  }

  #[test]
  fn errors_render_as_the_envelope_the_frontend_translates() {
    let err = CookieBotError(cloud_errors::classify_message(
      r#"(403) {"code":"COOKIE_BOT_NOT_ENTITLED"}"#,
      SCHEDULE_CODES,
    ));
    assert_eq!(err.code(), "COOKIE_BOT_NOT_ENTITLED");
    assert_eq!(err.status(), 403);
    assert_eq!(err.to_error_json(), r#"{"code":"COOKIE_BOT_NOT_ENTITLED"}"#);
  }

  #[test]
  fn a_bare_404_means_something_different_on_a_schedule_and_on_a_run() {
    // Both routes 404. Sharing one code would tell a user with no enrolment
    // that their run id is wrong, and vice versa.
    assert_eq!(
      cloud_errors::classify_message("(404) Not Found", SCHEDULE_CODES).code,
      "COOKIE_BOT_NOT_ENROLLED"
    );
    assert_eq!(
      cloud_errors::classify_message("(404) Not Found", RUN_CODES).code,
      "COOKIE_BOT_RUN_NOT_FOUND"
    );
  }

  #[test]
  fn query_values_are_encoded_so_they_cannot_smuggle_a_parameter() {
    // A keyset cursor is a server-issued opaque string. One containing `&`
    // would otherwise inject a second parameter into the request.
    let url = with_query(
      "https://api.example.com/runs",
      &[
        ("scope".to_string(), "team".to_string()),
        (
          "before".to_string(),
          "2026-08-03T00:00:00Z&limit=100".to_string(),
        ),
      ],
    );
    assert_eq!(
      url,
      "https://api.example.com/runs?scope=team&before=2026-08-03T00%3A00%3A00Z%26limit%3D100"
    );
    assert_eq!(
      with_query("https://api.example.com/runs", &[]),
      "https://api.example.com/runs",
      "an empty query must not leave a dangling separator"
    );
  }

  #[test]
  fn the_preset_list_carries_ids_not_behaviour() {
    // If this type ever gained a site list, a dwell range or a step
    // programme, the browsing model would have leaked into the open-source
    // client. Ids and a rough duration are all that may cross.
    let presets: CookieBotPresetList = serde_json::from_str(
      r#"{"presets":[{"id":"balanced","typical_minutes":35,"recommended":true}],
          "default_preset":"balanced"}"#,
    )
    .expect("the preset list must deserialize");
    assert_eq!(presets.presets[0].id, "balanced");
    assert_eq!(presets.presets[0].typical_minutes, Some(35));
    assert_eq!(presets.default_preset.as_deref(), Some("balanced"));
  }
}

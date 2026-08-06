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

/// Failure codes for the user-template routes.
///
/// Distinct from `SCHEDULE_CODES` on every axis that matters: a 404 here is a
/// template that was deleted (possibly from another device), not an unenrolled
/// profile, and a 409 is a name the user already used, not a teammate's
/// enrolment. Sharing the schedule set would have told someone renaming a site
/// list that a colleague already warms this profile.
const TEMPLATE_CODES: FailureCodes = FailureCodes {
  bad_request: "COOKIE_BOT_INVALID_TEMPLATE_NAME",
  forbidden: "COOKIE_BOT_NOT_ENTITLED",
  not_found: "COOKIE_BOT_TEMPLATE_NOT_FOUND",
  conflict: "COOKIE_BOT_TEMPLATE_NAME_TAKEN",
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

/// One time-of-day an enrolment fires, on a set of local weekdays.
///
/// Copy, and deliberately tiny: a calendar is a list of these, and the desktop
/// rebuilds that list on every keystroke in the enrolment form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotSlot {
  /// Bitmask of local weekdays, bit 0 = Monday. At least one bit set.
  pub days_mask: u8,
  /// Minutes past local midnight, in the schedule's timezone.
  pub run_at_minute: u16,
}

/// A profile enrolled in the nightly bot.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotSchedule {
  pub profile_id: String,
  pub profile_name: String,
  pub platform: String,
  pub enabled: bool,
  /// Minutes past local midnight the FIRST slot is anchored to. The server
  /// mirrors `slots[0]` onto this pair on every write.
  pub run_at_minute: u16,
  /// The first slot's weekdays, bit 0 = Monday. See `run_at_minute`.
  pub days_mask: u8,
  /// Every time-of-day this enrolment fires.
  ///
  /// `default` rather than required because a server older than multi-slot
  /// scheduling sends only the mirrored pair above, and a decode failure there
  /// would blank the whole Cookie Bot surface rather than show one time instead
  /// of several. Callers must therefore fall back to the pair when this is
  /// empty — never treat an empty list as "fires at no time".
  #[serde(default)]
  pub slots: Vec<CookieBotSlot>,
  pub timezone: String,
  /// Server-issued preset id. Opaque here — what it expands to is infra's.
  pub preset: String,
  /// The template the sites came from, or `None` for the user's own list.
  ///
  /// A built-in id (`low-intent-purchaser`) means `sites` is EMPTY on purpose:
  /// its URLs are server-owned and never sent to a client. A `user:<uuid>` id
  /// is provenance only — those sites were copied onto the enrolment and are
  /// present below.
  #[serde(default)]
  pub template_id: Option<String>,
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
  /// Whether that exit is one a leased fleet host could dial. Defaults to false
  /// on an older server that does not send it, which reads as "not reachable"
  /// and is the safe direction.
  #[serde(default)]
  pub proxy_remote_reachable: bool,
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
  /// The whole calendar, when the caller has one.
  ///
  /// `skip_serializing_if` is load-bearing rather than tidiness: the server
  /// reads an ABSENT `slots` as "one slot, from the pair above" and refuses a
  /// present-but-empty one, and `null` takes the refusing branch. Serialising
  /// `None` as null would 400 every write from a single-slot form.
  ///
  /// The pair above is still sent, mirrored from `slots[0]`, so a server that
  /// predates multi-slot stores the first time rather than nothing.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub slots: Option<Vec<CookieBotSlot>>,
  pub timezone: String,
  pub preset: String,
  /// A browsing template instead of a typed site list.
  ///
  /// Mutually exclusive with a non-empty `sites`: the server refuses a write
  /// carrying both, because merging a curated persona with the user's own list
  /// produces neither. A caller naming a template sends `sites: []`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub template_id: Option<String>,
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
  pub proxy_remote_reachable: bool,
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

/// A server-owned browsing template: a named answer to "what is this profile
/// for", which the user picks INSTEAD of typing a site list.
///
/// Carries no URLs, and must not gain any. The pool a template draws from is
/// server-side for the same reason a preset's browsing model is: a published
/// list is one a retailer can filter, and each profile is given its own sample
/// so the template never becomes a fleet-wide fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotTemplate {
  pub id: String,
  /// How many sites this template browses. Not which.
  #[serde(default)]
  pub site_count: u32,
  /// Server-supplied English label and blurb, present only so a template added
  /// after this build still renders. The UI prefers its own `t()` key for an id
  /// it recognises.
  #[serde(default)]
  pub name: Option<String>,
  #[serde(default)]
  pub description: Option<String>,
}

/// The bounds the schedule routes actually enforce, as this build reads them.
///
/// Every field is optional because a server that predates `limits` sends none
/// of them, and a client that read a missing bound as `0` would refuse every
/// value the form can produce. Only the bounds the desktop acts on are decoded
/// — serde drops the rest, and this struct is what the GUI ultimately receives,
/// so adding a field here is what makes one reachable from TypeScript.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotLimits {
  #[serde(default)]
  pub min_minutes: Option<u32>,
  #[serde(default)]
  pub max_minutes: Option<u32>,
  #[serde(default)]
  pub min_sites: Option<u32>,
  #[serde(default)]
  pub max_sites: Option<u32>,
  /// Most entries a calendar may carry.
  #[serde(default)]
  pub max_slots: Option<u32>,
  /// Longest name a saved site list may be given.
  #[serde(default)]
  pub max_template_name_length: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotPresetList {
  #[serde(default)]
  pub presets: Vec<CookieBotPreset>,
  /// Which preset the server suggests when the user has expressed no
  /// preference.
  #[serde(default)]
  pub default_preset: Option<String>,
  /// The curated templates on offer. Served beside the presets so a template
  /// added server-side appears without a desktop release.
  #[serde(default)]
  pub templates: Vec<CookieBotTemplate>,
  /// The server's own bounds, when it publishes them. The desktop mirrors a
  /// copy for offline form validation; these win where they disagree.
  #[serde(default)]
  pub limits: Option<CookieBotLimits>,
}

/// One of the caller's OWN saved site lists.
///
/// Carries its URLs, unlike {@link CookieBotTemplate} — they are the user's own
/// and there is nothing to withhold. Applying one copies the sites onto the
/// enrolment, so a list edited later does not silently change what an existing
/// enrolment browses until it is saved again.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CookieBotUserTemplate {
  /// Already carries the `user:` prefix: this id's job is to be pasted into a
  /// schedule's `template_id`, and assembling that convention on the client is
  /// how the two kinds of template get confused.
  pub id: String,
  pub name: String,
  #[serde(default)]
  pub sites: Vec<String>,
  #[serde(default)]
  pub updated_at: Option<String>,
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
pub fn bot_precondition(
  profile: &BrowserProfile,
  exit: &crate::remote_exit::ExitReachability,
) -> Result<(), String> {
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
  // ...and the exit has to be one the leased host can reach. The profile and its
  // proxy record are pulled onto the fleet with no address rewriting, so
  // 127.0.0.1 arrives meaning THAT host's loopback — an ordinary mistake (an SSH
  // tunnel, a local MITM proxy, a locally-run SOCKS client), and by the time the
  // run fails an hour has been leased and billed.
  //
  // Taken as an ARGUMENT rather than resolved here, for the same reason
  // `ProfileState` is required rather than defaulted: resolving it needs the
  // proxy and VPN stores, and a function that reaches into those globals is one
  // no test can set up and every caller silently depends on. `exit_reachability`
  // is the one place that resolution happens; this stays a pure predicate over
  // facts it is handed.
  if !exit.is_remote() {
    return Err(error("COOKIE_BOT_REQUIRES_REMOTE_EXIT_NODE", &[]));
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
    // ...and, separately, whether anyone OTHER than this machine could use it.
    // `has_proxy` answers "did the user bring an exit"; this answers "is that
    // exit an address a leased host can dial". They disagree for every local
    // proxy, which is the case that used to be accepted and then fail on the
    // fleet. See `remote_exit`.
    proxy_remote_reachable: exit_reachability(profile).is_remote(),
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
  /// Whether that exit is an address a leased fleet host can dial.
  pub proxy_remote_reachable: bool,
  pub touch_fingerprint: bool,
  pub sticky_exit: bool,
}

/// Whether this profile's exit could be used from a host that is not this one.
///
/// Resolves the profile's proxy or VPN out of local storage — the server cannot
/// do this, because it never sees a proxy record until sync has uploaded one and
/// even then would have to re-derive what the browser will actually dial.
///
/// A profile carrying BOTH a proxy and a VPN is judged on the proxy: that is
/// what the browser is pointed at, and it is the address the fleet has to reach.
pub fn exit_reachability(profile: &BrowserProfile) -> crate::remote_exit::ExitReachability {
  use crate::remote_exit::{classify_proxy, classify_wireguard_endpoint, ExitReachability};

  if let Some(proxy_id) = profile.proxy_id.as_deref() {
    let stored = crate::proxy_manager::PROXY_MANAGER
      .get_stored_proxies()
      .into_iter()
      .find(|candidate| candidate.id == proxy_id);
    return match stored {
      Some(proxy) => classify_proxy(&proxy.proxy_settings),
      // Referenced but missing. Fail closed: a dangling id is not evidence of a
      // reachable exit, and the launch would fail anyway.
      None => ExitReachability::Unknown {
        reason: "the profile references a proxy that no longer exists".to_string(),
        source: "proxy",
      },
    };
  }

  if let Some(vpn_id) = profile.vpn_id.as_deref() {
    let config = crate::vpn::VPN_STORAGE
      .lock()
      .ok()
      .and_then(|storage| storage.load_config(vpn_id).ok());
    return match config {
      Some(config) => match crate::vpn::parse_wireguard_config(&config.config_data) {
        Ok(parsed) => classify_wireguard_endpoint(&parsed.peer_endpoint),
        Err(error) => ExitReachability::Unknown {
          reason: format!("VPN config could not be parsed ({error})"),
          source: "VPN",
        },
      },
      None => ExitReachability::Unknown {
        reason: "the profile references a VPN config that no longer exists".to_string(),
        source: "VPN",
      },
    };
  }

  ExitReachability::None
}

impl CookieBotScheduleInput {
  /// Stamp the profile facts onto an input built from user-chosen values.
  pub fn with_profile_state(mut self, state: ProfileState) -> Self {
    self.sync_enabled = state.sync_enabled;
    self.encrypted_sync = state.encrypted_sync;
    self.has_proxy = state.has_proxy;
    self.proxy_remote_reachable = state.proxy_remote_reachable;
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
    "proxy_remote_reachable".to_string(),
    state.proxy_remote_reachable.into(),
  );
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

// --- User-defined templates -------------------------------------------------
//
// The caller's own saved site lists. Unlike every other route in this file
// these are addressed by an id the SERVER minted and the client echoes back,
// so each one percent-encodes it: the id is spelled `user:<uuid>`, and a bare
// colon in a path segment is a spelling the router is free to read differently.

#[derive(Debug, Deserialize)]
struct UserTemplateListEnvelope {
  #[serde(default)]
  templates: Vec<CookieBotUserTemplate>,
}

#[derive(Debug, Deserialize)]
struct UserTemplateEnvelope {
  template: CookieBotUserTemplate,
}

#[derive(Debug, Deserialize)]
struct UserTemplateDeleted {
  #[serde(default)]
  deleted: bool,
}

/// Every site list this user has saved, most recently edited first.
pub async fn list_user_templates() -> Result<Vec<CookieBotUserTemplate>, CookieBotError> {
  let envelope: UserTemplateListEnvelope = request(
    reqwest::Method::GET,
    format!("{}/user-templates", base()),
    Vec::new(),
    None,
    TEMPLATE_CODES,
  )
  .await?;
  Ok(envelope.templates)
}

/// Save a new one.
pub async fn create_user_template(
  name: &str,
  sites: &[String],
) -> Result<CookieBotUserTemplate, CookieBotError> {
  let body = serde_json::json!({ "name": name, "sites": sites });
  let envelope: UserTemplateEnvelope = request(
    reqwest::Method::POST,
    format!("{}/user-templates", base()),
    Vec::new(),
    Some(body),
    TEMPLATE_CODES,
  )
  .await?;
  Ok(envelope.template)
}

/// Rename one, replace its sites, or both.
///
/// A PATCH with only the fields that changed, because the two are independent:
/// a rename that had to carry the whole site list is a rename that silently
/// reverts an edit made to it from another device in the meantime. Sending an
/// omitted field as `null` would defeat that, so each is skipped when absent.
pub async fn update_user_template(
  id: &str,
  name: Option<&str>,
  sites: Option<&[String]>,
) -> Result<CookieBotUserTemplate, CookieBotError> {
  let mut body = serde_json::Map::new();
  if let Some(name) = name {
    body.insert(
      "name".to_string(),
      serde_json::Value::String(name.to_string()),
    );
  }
  if let Some(sites) = sites {
    body.insert("sites".to_string(), serde_json::json!(sites));
  }

  let envelope: UserTemplateEnvelope = request(
    reqwest::Method::PATCH,
    format!("{}/user-templates/{}", base(), urlencoding::encode(id)),
    Vec::new(),
    Some(serde_json::Value::Object(body)),
    TEMPLATE_CODES,
  )
  .await?;
  Ok(envelope.template)
}

/// Delete one. Enrolments that used it keep the sites they copied, so this is
/// never a way to stop a profile being warmed tonight.
///
/// Safe to repeat: deleting a list that is already gone answers `false` rather
/// than 404, which is what makes a retry after a dropped response harmless.
pub async fn delete_user_template(id: &str) -> Result<bool, CookieBotError> {
  let deleted: UserTemplateDeleted = request(
    reqwest::Method::DELETE,
    format!("{}/user-templates/{}", base(), urlencoding::encode(id)),
    Vec::new(),
    None,
    TEMPLATE_CODES,
  )
  .await?;
  Ok(deleted.deleted)
}

// --- Tauri commands ---------------------------------------------------------
//
// The user-template commands live here rather than in `lib.rs` beside the
// schedule ones because they carry no local precondition: nothing about a saved
// site list depends on a profile this machine holds, so there is no profile to
// look up and no `bot_precondition` to apply. They must still be registered in
// `lib.rs`'s `invoke_handler` to be reachable.

/// Log a refusal and hand the frontend the envelope it translates.
///
/// The raw HTTP text never reaches the user: an untranslated backend sentence
/// in a Japanese UI is the failure the `{"code":…}` convention exists to stop.
fn command_error(context: &str, err: CookieBotError) -> String {
  log::warn!(
    "Cookie bot {context} failed: {} (HTTP {})",
    err.code(),
    err.status()
  );
  err.to_error_json()
}

/// Every site list this user has saved.
#[tauri::command]
pub async fn get_cookie_bot_user_templates() -> Result<Vec<CookieBotUserTemplate>, String> {
  list_user_templates()
    .await
    .map_err(|e| command_error("template list", e))
}

/// Save the current site list under a name.
#[tauri::command]
pub async fn create_cookie_bot_user_template(
  name: String,
  sites: Vec<String>,
) -> Result<CookieBotUserTemplate, String> {
  create_user_template(&name, &sites)
    .await
    .map_err(|e| command_error("template create", e))
}

/// Rename a saved list, replace its sites, or both. Omitted fields are left
/// exactly as they are.
#[tauri::command]
pub async fn update_cookie_bot_user_template(
  id: String,
  name: Option<String>,
  sites: Option<Vec<String>>,
) -> Result<CookieBotUserTemplate, String> {
  update_user_template(&id, name.as_deref(), sites.as_deref())
    .await
    .map_err(|e| command_error("template update", e))
}

/// Delete a saved list. `false` means there was nothing left to delete.
#[tauri::command]
pub async fn delete_cookie_bot_user_template(id: String) -> Result<bool, String> {
  delete_user_template(&id)
    .await
    .map_err(|e| command_error("template delete", e))
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
  use crate::remote_exit::ExitReachability;

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
    let err = bot_precondition(&profile, &ExitReachability::Remote)
      .expect_err("a local-only profile must be refused");
    assert_eq!(code_of(&err), "COOKIE_BOT_REQUIRES_CLOUD_SYNC");
  }

  #[test]
  fn end_to_end_encrypted_sync_is_refused_with_its_own_code() {
    // Distinct from "turn sync on": the fix is to switch to Regular sync, and
    // one code cannot carry two different instructions.
    let mut profile = eligible_profile();
    profile.sync_mode = SyncMode::Encrypted;
    let err = bot_precondition(&profile, &ExitReachability::Remote)
      .expect_err("encrypted sync must be refused");
    assert_eq!(code_of(&err), "COOKIE_BOT_ENCRYPTED_SYNC_UNSUPPORTED");
  }

  #[test]
  fn linux_is_refused_at_enrolment_rather_than_at_two_in_the_morning() {
    let mut profile = eligible_profile();
    profile.host_os = Some("linux".to_string());
    let err = bot_precondition(&profile, &ExitReachability::Remote)
      .expect_err("linux has no host to lease");
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
    let err = bot_precondition(&profile, &ExitReachability::Remote)
      .expect_err("no OS means no matching host");
    assert_eq!(code_of(&err), "COOKIE_BOT_UNKNOWN_PLATFORM");
  }

  #[test]
  fn a_run_without_a_proxy_or_vpn_is_refused() {
    // Hours of overnight traffic from a hosting ASN damages the profile's
    // identity more than not warming it would.
    let mut profile = eligible_profile();
    profile.proxy_id = None;
    profile.vpn_id = None;
    let err = bot_precondition(&profile, &ExitReachability::None)
      .expect_err("datacenter egress must be refused");
    assert_eq!(code_of(&err), "COOKIE_BOT_REQUIRES_EXIT_NODE");
  }

  #[test]
  fn a_vpn_satisfies_the_exit_node_requirement_just_as_a_proxy_does() {
    let mut profile = eligible_profile();
    profile.proxy_id = None;
    profile.vpn_id = Some("vpn-1".to_string());
    assert!(bot_precondition(&profile, &ExitReachability::Remote).is_ok());
  }

  #[test]
  fn a_windows_profile_with_sync_and_a_proxy_qualifies() {
    let mut profile = eligible_profile();
    profile.host_os = Some("windows".to_string());
    assert!(bot_precondition(&profile, &ExitReachability::Remote).is_ok());
  }

  #[test]
  fn an_exit_only_this_machine_can_reach_is_refused() {
    // The gap `has_proxy` alone could never see, and — before the verdict became
    // an argument — a case no unit test could construct, because resolving it
    // reached into the global proxy store. The profile is otherwise perfect.
    let profile = eligible_profile();

    let err = bot_precondition(
      &profile,
      &ExitReachability::LocalOnly {
        host: "127.0.0.1".to_string(),
        source: "proxy",
      },
    )
    .expect_err("a loopback exit cannot be dialled from a leased host");

    // Its own code: "attach a proxy" is unactionable advice for someone whose
    // proxy is plainly attached.
    assert_eq!(code_of(&err), "COOKIE_BOT_REQUIRES_REMOTE_EXIT_NODE");
  }

  #[test]
  fn an_exit_we_could_not_read_is_refused_too() {
    // Fails closed. Refusing a working setup costs one support question;
    // accepting a broken one burns a leased hour and damages an identity.
    let err = bot_precondition(
      &eligible_profile(),
      &ExitReachability::Unknown {
        reason: "VPN config could not be parsed".to_string(),
        source: "VPN",
      },
    )
    .expect_err("an unreadable exit must not be assumed reachable");

    assert_eq!(code_of(&err), "COOKIE_BOT_REQUIRES_REMOTE_EXIT_NODE");
  }

  /// A verbatim `CookieBotScheduleView`, field for field, as `toScheduleView`
  /// in donutbrowser-infra's `cookie-bot.service.ts` builds it.
  const SERVER_SCHEDULE_VIEW: &str = r#"{
    "profile_id":"p1","profile_name":"Yu","platform":"macos","enabled":true,
    "run_at_minute":120,"days_mask":127,
    "slots":[{"days_mask":127,"run_at_minute":120},{"days_mask":31,"run_at_minute":690}],
    "timezone":"Europe/Berlin","template_id":null,
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
  fn a_schedule_carries_its_whole_calendar_not_just_the_first_time() {
    // The mirrored pair is `slots[0]`, so a client that read only the pair
    // would show "every night at 02:00" for an enrolment that also runs at
    // 11:30 on weeknights — fewer runs than the user booked, silently.
    let schedule: CookieBotSchedule =
      serde_json::from_str(SERVER_SCHEDULE_VIEW).expect("a multi-slot schedule must deserialize");

    assert_eq!(schedule.slots.len(), 2);
    assert_eq!(schedule.slots[0].run_at_minute, schedule.run_at_minute);
    assert_eq!(schedule.slots[0].days_mask, schedule.days_mask);
    assert_eq!(schedule.slots[1].run_at_minute, 690);
    assert_eq!(schedule.slots[1].days_mask, 31);
  }

  #[test]
  fn a_server_that_predates_multi_slot_still_decodes_with_no_slots() {
    // `slots` absent is a deployment that has not rolled forward, not a broken
    // enrolment. Requiring it would blank the whole Cookie Bot surface against
    // an older backend rather than show the one time it does know about.
    let schedule: CookieBotSchedule = serde_json::from_str(
      r#"{"profile_id":"p1","profile_name":"Yu","platform":"windows","enabled":true,
          "run_at_minute":120,"days_mask":31,"timezone":"UTC","preset":"light",
          "max_minutes":10}"#,
    )
    .expect("a pre-multi-slot schedule must deserialize");

    assert!(schedule.slots.is_empty());
    assert!(schedule.template_id.is_none());
  }

  #[test]
  fn a_templated_enrolment_reports_its_template_and_no_sites() {
    // A built-in template's URLs are server-owned. An empty `sites` here is the
    // contract working, not a schedule with nothing to browse — anything that
    // reads it as "no sites" would show a healthy enrolment as broken.
    let schedule: CookieBotSchedule = serde_json::from_str(
      &SERVER_SCHEDULE_VIEW
        .replace(
          "\"template_id\":null",
          "\"template_id\":\"low-intent-purchaser\"",
        )
        .replace("\"sites\":[\"https://example.com\"]", "\"sites\":[]"),
    )
    .expect("a templated schedule must deserialize");

    assert_eq!(
      schedule.template_id.as_deref(),
      Some("low-intent-purchaser")
    );
    assert!(schedule.sites.is_empty());
    assert!(schedule.blocked_by.is_none());
  }

  #[test]
  fn a_calendar_is_sent_as_slots_and_omitted_entirely_when_there_is_none() {
    // The server reads an ABSENT `slots` as "one slot, from the legacy pair"
    // and REFUSES a null or empty one. Serialising `None` as null would 400
    // every write from a form with a single time on it.
    let one_slot = CookieBotScheduleInput {
      profile_name: "Yu".to_string(),
      platform: "macos".to_string(),
      enabled: true,
      run_at_minute: 120,
      days_mask: 127,
      timezone: "Europe/Berlin".to_string(),
      preset: "balanced".to_string(),
      max_minutes: 45,
      sites: vec!["https://example.com".to_string()],
      ..Default::default()
    };
    let encoded = serde_json::to_value(&one_slot).expect("input must serialize");
    assert!(
      encoded.get("slots").is_none(),
      "an absent calendar must be absent on the wire, not null"
    );
    assert!(encoded.get("template_id").is_none());

    let many = CookieBotScheduleInput {
      slots: Some(vec![
        CookieBotSlot {
          days_mask: 127,
          run_at_minute: 120,
        },
        CookieBotSlot {
          days_mask: 31,
          run_at_minute: 690,
        },
      ]),
      ..one_slot
    };
    let encoded = serde_json::to_value(&many).expect("input must serialize");
    let slots = encoded["slots"].as_array().expect("slots must be a list");
    assert_eq!(slots.len(), 2);
    // Mirrored, because a server that predates multi-slot ignores `slots` and
    // stores this pair. Dropping it would leave that server with no time at all.
    assert_eq!(encoded["run_at_minute"], 120);
    assert_eq!(encoded["days_mask"], 127);
  }

  #[test]
  fn a_templated_write_names_the_template_and_sends_no_sites() {
    // The server refuses a body carrying both: a curated persona merged with
    // the user's own list is neither.
    let input = CookieBotScheduleInput {
      profile_name: "Yu".to_string(),
      platform: "macos".to_string(),
      enabled: true,
      run_at_minute: 120,
      days_mask: 127,
      timezone: "UTC".to_string(),
      preset: "balanced".to_string(),
      max_minutes: 45,
      sites: Vec::new(),
      template_id: Some("low-intent-purchaser".to_string()),
      ..Default::default()
    };
    let encoded = serde_json::to_value(&input).expect("input must serialize");
    assert_eq!(encoded["template_id"], "low-intent-purchaser");
    assert_eq!(
      encoded["sites"].as_array().map(Vec::len),
      Some(0),
      "sites must still be sent, and must be empty, beside a template"
    );
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
    // An older deployment sends neither of these, and the dialog has to render
    // against it: no templates simply means the picker offers the user's own
    // list, and no limits means the mirrored bounds apply.
    assert!(presets.templates.is_empty());
    assert!(presets.limits.is_none());
  }

  #[test]
  fn a_template_crosses_the_wire_as_a_count_and_never_as_urls() {
    // The pool is server-owned for the same reason a preset's browsing model
    // is. If this type ever gained a `sites` field the curation would be
    // published, and a published list is one a retailer can filter.
    let presets: CookieBotPresetList = serde_json::from_str(
      r#"{"presets":[],"default_preset":"balanced",
          "templates":[{"id":"low-intent-purchaser","site_count":32,
            "name":"Low-Intent Purchaser","description":"Price-sensitive browsing."}],
          "limits":{"min_minutes":5,"max_minutes":120,"min_sites":1,"max_sites":40,
            "max_site_length":2048,"max_jitter_seconds":3600,"max_slots":14,
            "max_template_name_length":80}}"#,
    )
    .expect("the preset list must carry templates and limits");

    assert_eq!(presets.templates[0].id, "low-intent-purchaser");
    assert_eq!(presets.templates[0].site_count, 32);
    let limits = presets.limits.expect("limits must decode");
    assert_eq!(limits.max_slots, Some(14));
    assert_eq!(limits.max_template_name_length, Some(80));
    assert_eq!(limits.max_sites, Some(40));
  }

  #[test]
  fn a_saved_list_arrives_with_the_prefix_a_schedule_write_needs() {
    // The id is what `template_id` takes verbatim. Handing the client a bare
    // uuid and expecting it to prepend `user:` is how a saved list gets looked
    // up against the built-in catalogue instead — which answers "no sites" and
    // silently unschedules the profile.
    let envelope: UserTemplateListEnvelope = serde_json::from_str(
      r#"{"templates":[{"id":"user:1c9a…","name":"My shops",
          "sites":["https://example.com"],"updated_at":"2026-08-05T10:00:00.000Z"}]}"#,
    )
    .expect("the user template list must deserialize");

    let template = &envelope.templates[0];
    assert!(template.id.starts_with("user:"));
    assert_eq!(template.name, "My shops");
    assert_eq!(template.sites.len(), 1);
  }

  #[test]
  fn deleting_a_saved_list_that_is_already_gone_is_not_a_failure() {
    // The route never 404s, so a delete retried after a dropped response has to
    // read as "nothing left to do" rather than as an error the user must act on.
    let deleted: UserTemplateDeleted =
      serde_json::from_str(r#"{"deleted":false,"id":"user:gone"}"#)
        .expect("a no-op delete must deserialize");
    assert!(!deleted.deleted);
  }

  #[test]
  fn a_template_404_is_a_missing_list_and_not_an_unenrolled_profile() {
    // Sharing SCHEDULE_CODES here would tell someone renaming a site list that
    // their profile is not enrolled, and a name collision that a teammate
    // already warms the profile.
    assert_eq!(
      cloud_errors::classify_message("(404) Not Found", TEMPLATE_CODES).code,
      "COOKIE_BOT_TEMPLATE_NOT_FOUND"
    );
    assert_eq!(
      cloud_errors::classify_message("(409) Conflict", TEMPLATE_CODES).code,
      "COOKIE_BOT_TEMPLATE_NAME_TAKEN"
    );
  }
}

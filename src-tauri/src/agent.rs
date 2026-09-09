//! Agent-run transport.
//!
//! An agent run is a goal the cloud pursues on one profile, either on this
//! desktop (`desktop`) or on a leased host (`fleet`). NONE of the reasoning
//! lives here: everything a run actually does is the cloud API's, and this side
//! never sees it.
//!
//! This module is the wire plus the two things a client is uniquely able to
//! check: that the profile the run names is actually on this machine, and that
//! the goal is not empty. Everything else is asked for and rendered back.
//!
//! It also owns the bridge that turns the run's `text/event-stream` into Tauri
//! events, because a run's steps are only observable through that stream — the
//! create call answers `queued` and nothing more.

use crate::cloud_errors::{self, BackendFailure, FailureCodes};
use crate::remote_session::{jittered, reconnect_delay, SseDecoder};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::AppHandle;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a live run may say nothing before the socket is treated as gone.
///
/// Longer than the cookie-bot equivalent on purpose: a single agent step can be
/// a slow page load followed by a model call, and cutting a healthy stream
/// mid-thought would replay the whole transcript for nothing.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Failure codes for the run routes.
const RUN_CODES: FailureCodes = FailureCodes {
  bad_request: "AGENT_GOAL_INVALID",
  forbidden: "AGENT_NOT_ENTITLED",
  not_found: "AGENT_RUN_NOT_FOUND",
  conflict: "AGENT_RUN_NOT_CANCELLABLE",
};

/// Failure codes for the recipe routes.
///
/// Deliberately not the run set: a 404 here is a recipe someone deleted from
/// another device, not a run id that was never the caller's, and telling a user
/// editing a saved list that "that run does not exist" is worse than saying
/// nothing. The backend sends its own `{"code":…}` body on every refusal it has
/// a name for, so these only decide a bodyless status.
const RECIPE_CODES: FailureCodes = FailureCodes {
  bad_request: "AGENT_RECIPE_INVALID",
  forbidden: "AGENT_NOT_ENTITLED",
  not_found: "AGENT_RECIPE_NOT_FOUND",
  conflict: "AGENT_RECIPE_INVALID",
};

/// Every agent call fails as a code the frontend can translate.
///
/// There is no `Other(String)` carrying backend English: a raw message reaches
/// the user untranslated, which is the bug pattern the `{"code":…}` convention
/// exists to block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentError(pub BackendFailure);

impl AgentError {
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

impl std::fmt::Display for AgentError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.to_error_json())
  }
}

impl From<BackendFailure> for AgentError {
  fn from(failure: BackendFailure) -> Self {
    Self(failure)
  }
}

// --- Wire types -------------------------------------------------------------
//
// The backend speaks camelCase on this surface, so every type here does too and
// the same spelling reaches the frontend unchanged. One shape end to end means
// a contract change is a single edit rather than two translations of it.

/// Where the run drives a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentTarget {
  /// This machine, through the desktop's own automation surface.
  Desktop,
  /// A leased host of the profile's operating system.
  Fleet,
}

impl AgentTarget {
  fn as_str(self) -> &'static str {
    match self {
      Self::Desktop => "desktop",
      Self::Fleet => "fleet",
    }
  }
}

/// Ceilings a run stops at rather than running until the money does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBudgets {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_steps: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_wall_ms: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_tokens: Option<u64>,
}

impl AgentBudgets {
  fn is_empty(&self) -> bool {
    self.max_steps.is_none() && self.max_wall_ms.is_none() && self.max_tokens.is_none()
  }
}

/// One run, as the server describes it.
///
/// Every field but the id defaults. A run view that fails to decode is a blank
/// page where a working one should be, and the desktop is a renderer here: a
/// field the server adds, drops or renames must cost one missing line, never
/// the whole surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunView {
  pub id: String,
  #[serde(default)]
  pub profile_id: String,
  #[serde(default)]
  pub target: String,
  #[serde(default)]
  pub platform: Option<String>,
  #[serde(default)]
  pub goal: String,
  #[serde(default)]
  pub status: String,
  #[serde(default)]
  pub model: Option<String>,
  #[serde(default)]
  pub effort: Option<String>,
  #[serde(default)]
  pub budgets: Option<AgentBudgets>,
  #[serde(default)]
  pub allowed_hosts: Option<Vec<String>>,
  #[serde(default)]
  pub remote_session_id: Option<String>,
  #[serde(default)]
  pub close_reason: Option<String>,
  #[serde(default)]
  pub error_code: Option<String>,
  #[serde(default)]
  pub result: Option<serde_json::Value>,
  #[serde(default)]
  pub tokens_in: Option<u64>,
  #[serde(default)]
  pub tokens_out: Option<u64>,
  #[serde(default)]
  pub cost_usd: Option<f64>,
  #[serde(default)]
  pub steps: Option<u32>,
  #[serde(default)]
  pub created_at: Option<String>,
  #[serde(default)]
  pub started_at: Option<String>,
  #[serde(default)]
  pub ended_at: Option<String>,
  #[serde(default)]
  pub updated_at: Option<String>,
}

/// One entry in a run's transcript.
///
/// `rest` keeps whatever the step kind carries — a thought's text, a tool's
/// name and arguments, an error's message. Flattened rather than enumerated
/// because the tool set is the server's and grows without a desktop release;
/// dropping the unknown half would render a tool step with nothing in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStep {
  #[serde(default)]
  pub index: u32,
  #[serde(default)]
  pub at: Option<String>,
  #[serde(default)]
  pub kind: String,
  #[serde(flatten)]
  pub rest: serde_json::Map<String, serde_json::Value>,
}

/// A run plus everything it has done so far.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunDetail {
  #[serde(flatten)]
  pub run: AgentRunView,
  #[serde(default)]
  pub transcript: Vec<AgentStep>,
}

/// One page of runs, newest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunPage {
  #[serde(default)]
  pub runs: Vec<AgentRunView>,
  #[serde(default)]
  pub next_cursor: Option<String>,
}

/// One step of a recipe, in the shape the API validates.
///
/// Kept as raw JSON rather than a mirrored enum: the server owns the schema
/// and refuses anything it does not recognise, so a second copy of the rules
/// here would only add a way for the two to disagree. What this side does
/// check is the shape a client can get wrong silently — a step must be an
/// object naming a `type` the server knows, and a targeted step must carry
/// exactly one of `selector` or `locator`.
pub type RecipeStep = serde_json::Value;

/// The step kinds the API accepts, in its own spelling.
const RECIPE_STEP_TYPES: [&str; 9] = [
  "navigate",
  "click",
  "type",
  "waitFor",
  "extract",
  "pressKey",
  "scroll",
  "screenshot",
  "sleep",
];

/// The kinds that name an element, and so must carry exactly one target.
const RECIPE_TARGETED_TYPES: [&str; 4] = ["click", "type", "waitFor", "extract"];

/// A recipe is a task, not a program: the API's own ceiling.
const MAX_RECIPE_STEPS: usize = 200;

/// A saved goal: a name and the steps it expands to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecipe {
  pub id: String,
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub steps: Vec<RecipeStep>,
  #[serde(default)]
  pub created_at: Option<String>,
  #[serde(default)]
  pub updated_at: Option<String>,
}

/// Either spelling of the recipe list.
///
/// A bare array and a `{"recipes":[…]}` envelope are both plausible readings of
/// the same route, and guessing wrong blanks the library with a decode error
/// rather than showing the user their own saved goals.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RecipeList {
  Enveloped { recipes: Vec<AgentRecipe> },
  Bare(Vec<AgentRecipe>),
}

impl RecipeList {
  fn into_vec(self) -> Vec<AgentRecipe> {
    match self {
      Self::Enveloped { recipes } => recipes,
      Self::Bare(recipes) => recipes,
    }
  }
}

/// What the desktop asks for when it starts a run.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAgentRunInput {
  pub profile_id: String,
  pub target: AgentTarget,
  pub goal: String,
  /// Which operating system the leased host must run. Ignored for a desktop
  /// run, and resolved from the profile when a fleet run omits it.
  #[serde(default)]
  pub platform: Option<String>,
  #[serde(default)]
  pub effort: Option<String>,
  #[serde(default)]
  pub budgets: Option<AgentBudgets>,
  #[serde(default)]
  pub allowed_hosts: Option<Vec<String>>,
}

// --- Local preconditions ----------------------------------------------------

/// The longest goal the desktop will send.
///
/// The server has its own ceiling; this one exists so a pasted document is
/// refused here instead of spending a round trip to be told so.
pub const MAX_GOAL_CHARS: usize = 4000;

/// Refuse a run the server would certainly refuse, before it costs anything.
///
/// Only the two things a client can actually know: the profile is on this
/// machine, and the goal says something. Everything else — entitlement, budget
/// ceilings, concurrency — is the server's to judge, and second-guessing it
/// here is how a client ends up refusing work the account is entitled to.
fn validate_goal(goal: &str) -> Result<String, String> {
  let trimmed = goal.trim();
  if trimmed.is_empty() || trimmed.chars().count() > MAX_GOAL_CHARS {
    return Err(serde_json::json!({ "code": "AGENT_GOAL_INVALID" }).to_string());
  }
  Ok(trimmed.to_string())
}

/// The local profile a run refers to.
fn local_profile(profile_id: &str) -> Result<crate::profile::types::BrowserProfile, String> {
  let profiles = crate::profile::manager::ProfileManager::instance()
    .list_profiles()
    .map_err(|e| {
      log::warn!("Agent run refused: profiles could not be read: {e}");
      serde_json::json!({ "code": "INTERNAL_ERROR" }).to_string()
    })?;
  profiles
    .into_iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or_else(|| serde_json::json!({ "code": "PROFILE_NOT_FOUND" }).to_string())
}

/// Which operating system a fleet run needs a host for.
///
/// The caller's choice when it made one, else the profile's own OS: a leased
/// host has to be the machine the profile was built for, so falling back to it
/// is the answer rather than a refusal the user cannot act on.
fn fleet_platform(
  requested: Option<&str>,
  profile: &crate::profile::types::BrowserProfile,
) -> Result<String, String> {
  let resolved = requested
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_string)
    .or_else(|| profile.host_os.clone())
    .unwrap_or_default();
  if crate::cookie_bot::BOT_PLATFORMS.contains(&resolved.as_str()) {
    return Ok(resolved);
  }
  Err(
    serde_json::json!({
      "code": "REMOTE_PLATFORM_UNSUPPORTED",
      "params": { "platform": resolved },
    })
    .to_string(),
  )
}

// --- Routes -----------------------------------------------------------------

fn base() -> String {
  format!("{}/api/agent", crate::cloud_auth::CLOUD_API_URL)
}

/// Start a run. Answers the created run, normally `queued`.
pub async fn start_run(input: StartAgentRunInput) -> Result<AgentRunView, String> {
  let goal = validate_goal(&input.goal)?;
  let profile = local_profile(&input.profile_id)?;

  let mut body = serde_json::Map::new();
  body.insert(
    "profileId".to_string(),
    serde_json::Value::String(input.profile_id.clone()),
  );
  body.insert(
    "target".to_string(),
    serde_json::Value::String(input.target.as_str().to_string()),
  );
  body.insert("goal".to_string(), serde_json::Value::String(goal));
  if input.target == AgentTarget::Fleet {
    body.insert(
      "platform".to_string(),
      serde_json::Value::String(fleet_platform(input.platform.as_deref(), &profile)?),
    );
  }
  if let Some(effort) = input
    .effort
    .as_deref()
    .map(str::trim)
    .filter(|s| !s.is_empty())
  {
    body.insert(
      "effort".to_string(),
      serde_json::Value::String(effort.to_string()),
    );
  }
  if let Some(budgets) = input.budgets.filter(|b| !b.is_empty()) {
    body.insert(
      "budgets".to_string(),
      serde_json::to_value(budgets).unwrap_or(serde_json::Value::Null),
    );
  }
  // An empty list is not the same as no list: `[]` would mean "navigate
  // nowhere", so only a non-empty allowlist is sent at all.
  let hosts = normalise_hosts(input.allowed_hosts.as_deref());
  if !hosts.is_empty() {
    body.insert(
      "allowedHosts".to_string(),
      serde_json::Value::Array(hosts.into_iter().map(serde_json::Value::String).collect()),
    );
  }

  request(
    reqwest::Method::POST,
    format!("{}/runs", base()),
    Vec::new(),
    Some(serde_json::Value::Object(body)),
    RUN_CODES,
  )
  .await
  .map_err(|e| agent_error("run start", e))
}

/// Trim, lowercase and de-duplicate an allowlist, dropping blanks.
///
/// Hosts are compared case-insensitively by every browser, so `Example.com` and
/// `example.com` reaching the server as two entries only makes the refusal
/// message longer.
pub fn normalise_hosts(hosts: Option<&[String]>) -> Vec<String> {
  let mut seen = Vec::new();
  for host in hosts.unwrap_or_default() {
    let host = host.trim().to_lowercase();
    if host.is_empty() || seen.contains(&host) {
      continue;
    }
    seen.push(host);
  }
  seen
}

/// One page of runs, newest first.
pub async fn list_runs(limit: Option<u32>, cursor: Option<&str>) -> Result<AgentRunPage, String> {
  let mut query = Vec::new();
  if let Some(n) = limit {
    query.push(("limit".to_string(), n.to_string()));
  }
  if let Some(c) = cursor.map(str::trim).filter(|s| !s.is_empty()) {
    query.push(("cursor".to_string(), c.to_string()));
  }
  request(
    reqwest::Method::GET,
    format!("{}/runs", base()),
    query,
    None,
    RUN_CODES,
  )
  .await
  .map_err(|e| agent_error("run list", e))
}

/// One run and everything it has done.
pub async fn get_run(run_id: &str) -> Result<AgentRunDetail, String> {
  request(
    reqwest::Method::GET,
    format!("{}/runs/{}", base(), urlencoding::encode(run_id)),
    Vec::new(),
    None,
    RUN_CODES,
  )
  .await
  .map_err(|e| agent_error("run read", e))
}

/// Stop a run that has not finished.
pub async fn cancel_run(run_id: &str) -> Result<AgentRunView, String> {
  request(
    reqwest::Method::POST,
    format!("{}/runs/{}/cancel", base(), urlencoding::encode(run_id)),
    Vec::new(),
    Some(serde_json::Value::Object(serde_json::Map::new())),
    RUN_CODES,
  )
  .await
  .map_err(|e| agent_error("run cancel", e))
}

/// Every saved goal this account has.
pub async fn list_recipes() -> Result<Vec<AgentRecipe>, String> {
  let list: RecipeList = request(
    reqwest::Method::GET,
    format!("{}/recipes", base()),
    Vec::new(),
    None,
    RECIPE_CODES,
  )
  .await
  .map_err(|e| agent_error("recipe list", e))?;
  Ok(list.into_vec())
}

/// Save a new one.
pub async fn create_recipe(name: &str, steps: &[RecipeStep]) -> Result<AgentRecipe, String> {
  let (name, steps) = validate_recipe(name, steps)?;
  request(
    reqwest::Method::POST,
    format!("{}/recipes", base()),
    Vec::new(),
    Some(serde_json::json!({ "name": name, "steps": steps })),
    RECIPE_CODES,
  )
  .await
  .map_err(|e| agent_error("recipe create", e))
}

/// Rename one, replace its steps, or both.
pub async fn update_recipe(
  id: &str,
  name: &str,
  steps: &[RecipeStep],
) -> Result<AgentRecipe, String> {
  let (name, steps) = validate_recipe(name, steps)?;
  request(
    reqwest::Method::PATCH,
    format!("{}/recipes/{}", base(), urlencoding::encode(id)),
    Vec::new(),
    Some(serde_json::json!({ "name": name, "steps": steps })),
    RECIPE_CODES,
  )
  .await
  .map_err(|e| agent_error("recipe update", e))
}

/// Delete one.
pub async fn delete_recipe(id: &str) -> Result<bool, String> {
  let outcome: RecipeDeleted = request(
    reqwest::Method::DELETE,
    format!("{}/recipes/{}", base(), urlencoding::encode(id)),
    Vec::new(),
    None,
    RECIPE_CODES,
  )
  .await
  .map_err(|e| agent_error("recipe delete", e))?;
  Ok(outcome.deleted.unwrap_or(true))
}

#[derive(Debug, Default, Deserialize)]
struct RecipeDeleted {
  #[serde(default)]
  deleted: Option<bool>,
}

/// A recipe needs a name and at least one step.
///
/// `NAME_CANNOT_BE_EMPTY` rather than an agent-specific code: it is the same
/// refusal the user already meets when naming a group or a proxy, and it is
/// already translated everywhere.
fn validate_recipe(name: &str, steps: &[RecipeStep]) -> Result<(String, Vec<RecipeStep>), String> {
  let name = name.trim().to_string();
  if name.is_empty() {
    return Err(serde_json::json!({ "code": "NAME_CANNOT_BE_EMPTY" }).to_string());
  }
  if steps.is_empty() || steps.len() > MAX_RECIPE_STEPS {
    return Err(serde_json::json!({ "code": "AGENT_RECIPE_INVALID" }).to_string());
  }
  for step in steps {
    let Some(object) = step.as_object() else {
      return Err(serde_json::json!({ "code": "AGENT_RECIPE_INVALID" }).to_string());
    };
    let Some(kind) = object.get("type").and_then(serde_json::Value::as_str) else {
      return Err(serde_json::json!({ "code": "AGENT_RECIPE_INVALID" }).to_string());
    };
    if !RECIPE_STEP_TYPES.contains(&kind) {
      return Err(serde_json::json!({ "code": "AGENT_RECIPE_INVALID" }).to_string());
    }
    if RECIPE_TARGETED_TYPES.contains(&kind) {
      let has_selector = object
        .get("selector")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|selector| !selector.trim().is_empty());
      let has_locator = object
        .get("locator")
        .is_some_and(|locator| locator.as_object().is_some_and(|fields| !fields.is_empty()));
      // Exactly one, which is the rule the API enforces: both is ambiguous and
      // neither names nothing.
      if has_selector == has_locator {
        return Err(serde_json::json!({ "code": "AGENT_RECIPE_INVALID" }).to_string());
      }
    }
  }
  Ok((name, steps.to_vec()))
}

/// Turn an agent failure into the code the frontend translates.
fn agent_error(context: &str, err: AgentError) -> String {
  log::warn!(
    "Agent {context} failed: {} (HTTP {})",
    err.code(),
    err.status()
  );
  err.to_error_json()
}

// --- Step stream ------------------------------------------------------------

/// One step was appended to a run. Payload: the SSE frame's own JSON.
pub const EVENT_AGENT_STEP: &str = "agent-run-step";
/// A run changed status. Payload: the SSE frame's own JSON.
pub const EVENT_AGENT_STATUS: &str = "agent-run-status";
/// Whether steps are currently arriving. Payload: `{connected, runId, reason}`.
pub const EVENT_AGENT_STREAM: &str = "agent-run-stream";

static STREAM_RUNNING: AtomicBool = AtomicBool::new(false);
static WATCHED_RUN: Mutex<Option<String>> = Mutex::new(None);
static STREAM_TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);

/// Granularity of the cancellable sleep, so a stop is not held up by a backoff.
const SHUTDOWN_POLL: Duration = Duration::from_millis(250);

/// A run that cannot move again on its own.
///
/// The stream ends after one of these, and a client that reconnected anyway
/// would replay a finished transcript on a loop for as long as the page stayed
/// open.
pub fn is_terminal_status(status: &str) -> bool {
  matches!(status, "succeeded" | "failed" | "cancelled")
}

/// Which run the desktop is currently streaming, if any.
pub fn watched_run() -> Option<String> {
  if !STREAM_RUNNING.load(Ordering::SeqCst) {
    return None;
  }
  WATCHED_RUN.lock().ok().and_then(|slot| slot.clone())
}

/// Start streaming one run's steps.
///
/// Idempotent for the run already being watched; asking for a different one
/// replaces the stream rather than opening a second socket, because the page
/// only ever shows one run at a time and two sockets would double every step.
pub fn start_run_events(app: AppHandle, run_id: String) {
  if watched_run().as_deref() == Some(run_id.as_str()) {
    return;
  }
  stop_run_events();

  if let Ok(mut slot) = WATCHED_RUN.lock() {
    *slot = Some(run_id.clone());
  }
  STREAM_RUNNING.store(true, Ordering::SeqCst);
  let handle = tauri::async_runtime::spawn(async move {
    run_step_events(app, run_id).await;
  });
  if let Ok(mut slot) = STREAM_TASK.lock() {
    *slot = Some(handle);
  }
}

/// Stop streaming. Safe to call when nothing is running.
pub fn stop_run_events() {
  if let Ok(mut slot) = WATCHED_RUN.lock() {
    *slot = None;
  }
  if !STREAM_RUNNING.swap(false, Ordering::SeqCst) {
    return;
  }
  if let Ok(mut slot) = STREAM_TASK.lock() {
    if let Some(handle) = slot.take() {
      handle.abort();
    }
  }
}

async fn run_step_events(app: AppHandle, run_id: String) {
  let mut attempt = 0u32;

  while STREAM_RUNNING.load(Ordering::SeqCst) && watched_run().as_deref() == Some(run_id.as_str()) {
    match connect_run_events(&run_id).await {
      Ok(response) => {
        attempt = 0;
        emit_stream_status(&app, &run_id, true, None);
        match consume_run_events(&app, response).await {
          Ok(saw_terminal) => {
            emit_stream_status(&app, &run_id, false, None);
            if saw_terminal {
              // The run is over and the transcript will not grow again.
              // Reconnecting would replay it on a loop.
              log::info!("Agent run {run_id} reached a terminal status; stream closed");
              break;
            }
            log::info!("Agent run {run_id} stream closed by the backend");
          }
          Err(reason) => {
            log::warn!("Agent run {run_id} stream ended: {reason}");
            emit_stream_status(&app, &run_id, false, Some(&reason));
          }
        }
      }
      Err(reason) => {
        log::warn!("Agent run {run_id} stream could not connect: {reason}");
        emit_stream_status(&app, &run_id, false, Some(&reason));
      }
    }

    if !STREAM_RUNNING.load(Ordering::SeqCst) {
      break;
    }
    let delay = jittered(reconnect_delay(attempt));
    attempt = attempt.saturating_add(1);
    sleep_unless_stopped(delay).await;
  }

  // Only clear the slot when this task still owns it: a newer `start_run_events`
  // may already have installed its own run id, and blanking that would make the
  // status command lie about a stream that is very much alive.
  if let Ok(mut slot) = WATCHED_RUN.lock() {
    if slot.as_deref() == Some(run_id.as_str()) {
      *slot = None;
      STREAM_RUNNING.store(false, Ordering::SeqCst);
    }
  }
}

async fn sleep_unless_stopped(total: Duration) {
  let mut slept = Duration::ZERO;
  while slept < total && STREAM_RUNNING.load(Ordering::SeqCst) {
    let step = SHUTDOWN_POLL.min(total - slept);
    tokio::time::sleep(step).await;
    slept += step;
  }
}

async fn connect_run_events(run_id: &str) -> Result<reqwest::Response, String> {
  let endpoint = format!("{}/runs/{}/events", base(), urlencoding::encode(run_id));

  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let endpoint = endpoint.clone();
      async move {
        // Through api_call_with_retry so a token that expired during a long run
        // is refreshed on the reconnect instead of leaving the page silent.
        let response = crate::remote_session::stream_client()
          .get(&endpoint)
          .bearer_auth(token)
          .header(reqwest::header::ACCEPT, "text/event-stream")
          .header(reqwest::header::CACHE_CONTROL, "no-cache")
          .send()
          .await
          .map_err(|e| format!("reach backend: {e}"))?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
          let text = response.text().await.unwrap_or_default();
          return Err(format!("({status}) {text}"));
        }
        Ok(response)
      }
    })
    .await
    .map_err(|e| cloud_errors::classify_message(&e, RUN_CODES).code)
}

/// Read frames until the socket closes. `true` when the run finished first.
async fn consume_run_events(app: &AppHandle, response: reqwest::Response) -> Result<bool, String> {
  use futures_util::StreamExt;

  let mut stream = response.bytes_stream();
  let mut decoder = SseDecoder::new();
  let mut saw_terminal = false;

  loop {
    if !STREAM_RUNNING.load(Ordering::SeqCst) {
      return Ok(saw_terminal);
    }

    let next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await;
    let chunk = match next {
      // No ping. The socket is gone even though nothing errored, which is what
      // a machine returning from sleep sees.
      Err(_) => return Err("no heartbeat within the idle timeout".to_string()),
      Ok(None) => return Ok(saw_terminal),
      Ok(Some(Err(e))) => return Err(format!("stream error: {e}")),
      Ok(Some(Ok(bytes))) => bytes,
    };

    for frame in decoder.push(&chunk) {
      let Some((target, payload)) = route_frame(frame.event.as_deref(), &frame.data) else {
        continue;
      };
      if target == EVENT_AGENT_STATUS && payload_is_terminal(&payload) {
        saw_terminal = true;
      }
      emit(app, target, payload);
    }
  }
}

fn payload_is_terminal(payload: &serde_json::Value) -> bool {
  payload
    .get("status")
    .and_then(serde_json::Value::as_str)
    .is_some_and(is_terminal_status)
}

/// Turn one decoded frame into the Tauri event and payload it becomes.
///
/// The backend names its frames (`step`, `status`, `ping`) AND repeats the name
/// inside the JSON as `type`. Either is honoured, because a proxy that strips
/// the `event:` line would otherwise turn every step into a dropped frame and
/// the page would sit empty under a run that is visibly progressing.
pub fn route_frame(event: Option<&str>, data: &str) -> Option<(&'static str, serde_json::Value)> {
  if matches!(event, Some("ping" | "heartbeat" | "keepalive")) {
    return None;
  }

  let payload = match serde_json::from_str::<serde_json::Value>(data) {
    Ok(value) => value,
    Err(e) => {
      log::warn!("Ignoring malformed agent event: {e}");
      return None;
    }
  };
  let object = payload.as_object()?;

  let kind = object
    .get("type")
    .and_then(serde_json::Value::as_str)
    .or(event)
    .unwrap_or_default();

  match kind {
    "step" => Some((EVENT_AGENT_STEP, payload)),
    "status" => Some((EVENT_AGENT_STATUS, payload)),
    "ping" | "heartbeat" | "keepalive" => None,
    other => {
      log::warn!("Ignoring agent frame of unknown kind: {other}");
      None
    }
  }
}

fn emit(app: &AppHandle, target: &str, payload: serde_json::Value) {
  use tauri::Emitter;
  if let Err(e) = app.emit(target, payload) {
    log::warn!("Failed to emit {target}: {e}");
  }
}

fn emit_stream_status(app: &AppHandle, run_id: &str, connected: bool, reason: Option<&str>) {
  emit(
    app,
    EVENT_AGENT_STREAM,
    serde_json::json!({ "connected": connected, "runId": run_id, "reason": reason }),
  );
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

/// One request, one place.
///
/// Goes through `api_call_with_retry` so an expired access token is refreshed
/// and the call retried once — otherwise a user whose token aged out mid-run
/// sees "not signed in" on a machine that is signed in.
async fn request<T: DeserializeOwned>(
  method: reqwest::Method,
  url: String,
  query: Vec<(String, String)>,
  body: Option<serde_json::Value>,
  codes: FailureCodes,
) -> Result<T, AgentError> {
  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let method = method.clone();
      let url = url.clone();
      let query = query.clone();
      let body = body.clone();
      async move {
        // Percent-encoded here rather than left to the HTTP client: a cursor
        // carrying a `&` must not be able to smuggle a second parameter into
        // the request. One implementation, shared with the cookie-bot wire.
        let url = crate::cookie_bot::with_query(&url, &query);
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
    .map_err(|e| AgentError(cloud_errors::classify_message(&e, codes)))
}

// --- Tauri commands ---------------------------------------------------------
//
// Defined here rather than in `lib.rs` because every local precondition they
// have lives in this file. They are registered in `generate_handler!` as
// `agent::…`; unregistered they are unreachable and the page fails at runtime
// with "command not found" rather than at build time.

/// Start a run against a profile this machine holds.
#[tauri::command]
pub async fn start_agent_run(input: StartAgentRunInput) -> Result<AgentRunView, String> {
  start_run(input).await
}

/// One page of runs, newest first.
#[tauri::command]
pub async fn get_agent_runs(
  limit: Option<u32>,
  cursor: Option<String>,
) -> Result<AgentRunPage, String> {
  list_runs(limit, cursor.as_deref()).await
}

/// One run and its transcript.
#[tauri::command]
pub async fn get_agent_run(run_id: String) -> Result<AgentRunDetail, String> {
  get_run(&run_id).await
}

/// Stop a run that has not finished.
#[tauri::command]
pub async fn cancel_agent_run(run_id: String) -> Result<AgentRunView, String> {
  cancel_run(&run_id).await
}

/// Every saved goal.
#[tauri::command]
pub async fn get_agent_recipes() -> Result<Vec<AgentRecipe>, String> {
  list_recipes().await
}

/// Save a new one.
#[tauri::command]
pub async fn create_agent_recipe(
  name: String,
  steps: Vec<RecipeStep>,
) -> Result<AgentRecipe, String> {
  create_recipe(&name, &steps).await
}

/// Rename one or replace its steps.
#[tauri::command]
pub async fn update_agent_recipe(
  id: String,
  name: String,
  steps: Vec<RecipeStep>,
) -> Result<AgentRecipe, String> {
  update_recipe(&id, &name, &steps).await
}

/// Delete one.
#[tauri::command]
pub async fn delete_agent_recipe(id: String) -> Result<bool, String> {
  delete_recipe(&id).await
}

/// Stream one run's steps. Idempotent for the run already being watched.
#[tauri::command]
pub fn start_agent_run_events(app_handle: AppHandle, run_id: String) {
  start_run_events(app_handle, run_id);
}

/// Stop streaming. Safe when nothing is running.
#[tauri::command]
pub fn stop_agent_run_events() {
  stop_run_events();
}

/// Which run is being streamed, if any.
///
/// A page that mounts after the stream started has no `agent-run-stream` event
/// to read, so this is how it decides whether to trust the live steps or fall
/// back to re-reading the transcript.
#[tauri::command]
pub fn get_agent_run_events_status() -> Option<String> {
  watched_run()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_blank_goal_is_refused_before_anything_is_spent() {
    for goal in ["", "   ", "\n\t "] {
      let err = validate_goal(goal).unwrap_err();
      assert!(err.contains("AGENT_GOAL_INVALID"), "{goal:?} -> {err}");
    }
    assert_eq!(validate_goal("  buy milk  ").unwrap(), "buy milk");
  }

  #[test]
  fn a_goal_longer_than_the_ceiling_is_refused_here_rather_than_by_the_server() {
    let long = "a".repeat(MAX_GOAL_CHARS + 1);
    assert!(validate_goal(&long)
      .unwrap_err()
      .contains("AGENT_GOAL_INVALID"));
    // Counted in characters, not bytes: a goal written in Japanese is not three
    // times shorter than the same goal written in English.
    let wide = "あ".repeat(MAX_GOAL_CHARS);
    assert_eq!(
      validate_goal(&wide).unwrap().chars().count(),
      MAX_GOAL_CHARS
    );
  }

  #[test]
  fn an_allowlist_is_lowercased_deduplicated_and_stripped_of_blanks() {
    let hosts = vec![
      "Example.com".to_string(),
      "  example.com ".to_string(),
      "".to_string(),
      "   ".to_string(),
      "docs.example.com".to_string(),
    ];
    assert_eq!(
      normalise_hosts(Some(&hosts)),
      vec!["example.com".to_string(), "docs.example.com".to_string()]
    );
    assert!(normalise_hosts(None).is_empty());
  }

  #[test]
  fn a_recipe_needs_a_name_and_steps_the_api_will_accept() {
    let navigate = serde_json::json!({"type": "navigate", "url": "https://example.com"});
    assert!(validate_recipe("  ", std::slice::from_ref(&navigate))
      .unwrap_err()
      .contains("NAME_CANNOT_BE_EMPTY"));
    assert!(validate_recipe("nightly", &[])
      .unwrap_err()
      .contains("AGENT_RECIPE_INVALID"));

    // A step is an object naming a kind the API knows. The old shape — a bare
    // line of prose — is exactly what the API refuses, so it is refused here
    // rather than sent and rejected.
    for rejected in [
      serde_json::json!("open the shop"),
      serde_json::json!({"url": "https://example.com"}),
      serde_json::json!({"type": "teleport", "url": "https://example.com"}),
    ] {
      assert!(
        validate_recipe("nightly", std::slice::from_ref(&rejected))
          .unwrap_err()
          .contains("AGENT_RECIPE_INVALID"),
        "{rejected} must be refused"
      );
    }

    // A step that names an element carries exactly one target.
    let neither = serde_json::json!({"type": "click"});
    let both = serde_json::json!({
      "type": "click",
      "selector": "#buy",
      "locator": {"role": "button"}
    });
    for rejected in [neither, both] {
      assert!(validate_recipe("nightly", std::slice::from_ref(&rejected))
        .unwrap_err()
        .contains("AGENT_RECIPE_INVALID"));
    }

    let click = serde_json::json!({"type": "click", "locator": {"role": "button", "name": "Buy"}});
    let (name, steps) = validate_recipe("  nightly  ", &[navigate.clone(), click.clone()]).unwrap();
    assert_eq!(name, "nightly");
    assert_eq!(steps, vec![navigate, click]);
  }

  #[test]
  fn a_fleet_run_falls_back_to_the_profile_operating_system() {
    let profile = crate::profile::types::BrowserProfile {
      host_os: Some("windows".to_string()),
      ..Default::default()
    };
    assert_eq!(fleet_platform(None, &profile).unwrap(), "windows");
    assert_eq!(fleet_platform(Some("  "), &profile).unwrap(), "windows");
    assert_eq!(fleet_platform(Some("linux"), &profile).unwrap(), "linux");

    let android = crate::profile::types::BrowserProfile {
      host_os: Some("android".to_string()),
      ..Default::default()
    };
    let err = fleet_platform(None, &android).unwrap_err();
    assert!(err.contains("REMOTE_PLATFORM_UNSUPPORTED"), "{err}");
    assert!(err.contains("android"), "{err}");
  }

  #[test]
  fn only_a_finished_run_ends_the_stream() {
    for status in ["succeeded", "failed", "cancelled"] {
      assert!(is_terminal_status(status), "{status}");
    }
    for status in ["queued", "running", ""] {
      assert!(!is_terminal_status(status), "{status}");
    }
  }

  #[test]
  fn steps_and_statuses_route_by_the_json_type_or_the_sse_name() {
    let step = r#"{"type":"step","runId":"r1","step":{"index":0,"kind":"thought"}}"#;
    assert_eq!(route_frame(Some("step"), step).unwrap().0, EVENT_AGENT_STEP);
    // A proxy that strips the event line must not cost the frontend its steps.
    assert_eq!(route_frame(None, step).unwrap().0, EVENT_AGENT_STEP);
    // ...and neither must a backend that stops repeating the type inside.
    assert_eq!(
      route_frame(Some("step"), r#"{"runId":"r1"}"#).unwrap().0,
      EVENT_AGENT_STEP
    );

    let status = r#"{"type":"status","runId":"r1","status":"running"}"#;
    assert_eq!(
      route_frame(Some("status"), status).unwrap().0,
      EVENT_AGENT_STATUS
    );
  }

  #[test]
  fn pings_and_nonsense_are_dropped_rather_than_emitted() {
    assert!(route_frame(Some("ping"), "").is_none());
    assert!(route_frame(None, r#"{"type":"ping"}"#).is_none());
    assert!(route_frame(Some("step"), "not json").is_none());
    assert!(route_frame(None, r#"{"type":"something-new"}"#).is_none());
  }

  #[test]
  fn a_terminal_status_frame_is_recognised_from_its_payload() {
    let (_, payload) =
      route_frame(Some("status"), r#"{"type":"status","status":"succeeded"}"#).unwrap();
    assert!(payload_is_terminal(&payload));
    let (_, running) =
      route_frame(Some("status"), r#"{"type":"status","status":"running"}"#).unwrap();
    assert!(!payload_is_terminal(&running));
  }

  #[test]
  fn a_run_view_survives_a_server_that_sends_almost_nothing() {
    // The desktop renders this; a decode failure is a blank page where a
    // working run should be.
    let view: AgentRunView = serde_json::from_str(r#"{"id":"run-1"}"#).unwrap();
    assert_eq!(view.id, "run-1");
    assert_eq!(view.status, "");
    assert!(view.budgets.is_none());
  }

  #[test]
  fn a_step_keeps_the_fields_this_release_has_never_heard_of() {
    let step: AgentStep = serde_json::from_str(
      r##"{"index":3,"at":"2026-01-01T00:00:00Z","kind":"tool","tool":"click","selector":"#buy"}"##,
    )
    .unwrap();
    assert_eq!(step.index, 3);
    assert_eq!(step.kind, "tool");
    assert_eq!(step.rest.get("tool").unwrap(), "click");
    assert_eq!(step.rest.get("selector").unwrap(), "#buy");
  }

  #[test]
  fn the_recipe_list_decodes_bare_or_enveloped() {
    let bare: RecipeList =
      serde_json::from_str(r#"[{"id":"a","name":"A","steps":["x"]}]"#).unwrap();
    assert_eq!(bare.into_vec().len(), 1);
    let enveloped: RecipeList =
      serde_json::from_str(r#"{"recipes":[{"id":"a","name":"A","steps":["x"]}]}"#).unwrap();
    assert_eq!(enveloped.into_vec()[0].id, "a");
  }

  #[test]
  fn budgets_only_travel_when_the_user_set_one() {
    assert!(AgentBudgets::default().is_empty());
    assert!(!AgentBudgets {
      max_steps: Some(10),
      ..Default::default()
    }
    .is_empty());
    // `skip_serializing_if` keeps an unset ceiling out of the body entirely,
    // so the server applies its own default rather than reading a null as zero.
    let json = serde_json::to_string(&AgentBudgets {
      max_steps: Some(10),
      ..Default::default()
    })
    .unwrap();
    assert_eq!(json, r#"{"maxSteps":10}"#);
  }
}

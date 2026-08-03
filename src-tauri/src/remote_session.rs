//! Launching a profile on a remote VM.
//!
//! The desktop app never talks to the Wayfern manager directly. It asks
//! donutbrowser-infra, which holds the service-account credentials and is the
//! only party that can mint a donut-sync token scoped to this user's namespace.
//! That indirection is the point: a desktop client that could call the manager
//! itself would need credentials capable of launching sessions for anyone.

use crate::cloud_errors::{self, FailureCodes};
use crate::profile::types::BrowserProfile;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::AppHandle;

/// Which code a remote-session failure resolves to when the backend sends no
/// envelope of its own.
const SESSION_CODES: FailureCodes = FailureCodes {
  bad_request: "REMOTE_SESSION_REFUSED",
  forbidden: "REMOTE_NOT_ENTITLED",
  not_found: "REMOTE_SESSION_NOT_FOUND",
  conflict: "REMOTE_SESSION_CONFLICT",
};

/// Why a remote launch failed, mapped to the status the local API should return.
#[derive(Debug)]
pub enum RemoteSessionError {
  /// No host of the profile's OS has a free slot right now.
  NoCapacity(String),
  /// The profile is already open somewhere — locally or in another session.
  Conflict(String),
  /// The user's plan does not cover remote automation.
  NotAuthorised(String),
  Other(String),
}

impl std::fmt::Display for RemoteSessionError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::NoCapacity(m) | Self::Conflict(m) | Self::NotAuthorised(m) | Self::Other(m) => {
        write!(f, "{m}")
      }
    }
  }
}

impl RemoteSessionError {
  /// The `{"code":…,"params":{…}}` string a Tauri command returns.
  ///
  /// The variants carry the backend's own English, which reaches the user
  /// untranslated if it is surfaced as-is. This recovers the machine code the
  /// frontend has a locale string for.
  pub fn to_error_json(&self) -> String {
    // The three typed variants know the status they came from; `Other` kept
    // only the body, so it is re-read for an embedded envelope.
    let (status, message) = match self {
      Self::NoCapacity(m) => (503, m),
      Self::Conflict(m) => (409, m),
      Self::NotAuthorised(m) => (403, m),
      Self::Other(m) => return cloud_errors::classify_message(m, SESSION_CODES).to_error_json(),
    };
    cloud_errors::classify(status, message, SESSION_CODES).to_error_json()
  }
}

/// What the backend returns when a session starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSessionOutcome {
  pub session_id: String,
  pub platform: String,
  pub status: String,
}

#[derive(Debug, Serialize)]
struct StartRemoteRequest {
  profile_id: String,
  /// The profile's own OS. The backend refuses to schedule it anywhere else.
  platform: String,
  /// Set when the caller wants a page opened once the browser is up.
  #[serde(skip_serializing_if = "Option::is_none")]
  url: Option<String>,
  /// De-duplicates retries so a flaky network cannot open two browsers against
  /// one profile.
  idempotency_key: String,
}

/// Map a backend status onto a typed error.
///
/// Kept separate from the request so the mapping is testable: getting 503
/// wrong would turn "come back in a minute" into "something is broken",
/// and getting 409 wrong would hide the fact that the profile is already open.
pub fn classify_backend_status(status: u16, body: &str) -> RemoteSessionError {
  let message = if body.is_empty() {
    format!("remote session request failed with HTTP {status}")
  } else {
    body.to_string()
  };
  match status {
    503 => RemoteSessionError::NoCapacity(message),
    409 => RemoteSessionError::Conflict(message),
    401..=403 => RemoteSessionError::NotAuthorised(message),
    _ => RemoteSessionError::Other(message),
  }
}

/// Build the idempotency key for one launch attempt.
///
/// Derived from the profile and a caller-supplied attempt id rather than
/// random, so a retry of the SAME user action de-duplicates while a genuinely
/// new launch does not. The attempt id is a plain uniqueness token, not a
/// cryptographic value.
pub fn idempotency_key(profile_id: &str, attempt: &str) -> String {
  format!("run-remote:{profile_id}:{attempt}")
}

/// Ask donutbrowser-infra to start a remote session for this profile.
///
/// Goes through `api_call_with_retry` so an expired access token is refreshed
/// and the request retried once, rather than surfacing to the user as a
/// spurious "not signed in".
pub async fn start_remote_session(
  _app: AppHandle,
  profile: &BrowserProfile,
  url: Option<String>,
) -> Result<RemoteSessionOutcome, RemoteSessionError> {
  let platform = profile
    .resolved_os()
    .ok_or_else(|| {
      RemoteSessionError::Other("profile has no recorded operating system".to_string())
    })?
    .to_string();
  let profile_id = profile.id.to_string();

  // One key for this user action: a retry inside api_call_with_retry must
  // de-duplicate rather than open a second browser on the same profile.
  let key = idempotency_key(&profile_id, &uuid::Uuid::new_v4().to_string());
  let endpoint = format!("{}/api/remote-sessions", crate::cloud_auth::CLOUD_API_URL);

  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let endpoint = endpoint.clone();
      let body = StartRemoteRequest {
        profile_id: profile_id.clone(),
        platform: platform.clone(),
        url: url.clone(),
        idempotency_key: key.clone(),
      };
      async move {
        let response = reqwest::Client::new()
          .post(&endpoint)
          .bearer_auth(token)
          .json(&body)
          .send()
          .await
          .map_err(|e| format!("reach backend: {e}"))?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
          let text = response.text().await.unwrap_or_default();
          // Encode the status so api_call_with_retry can spot a 401, and so
          // classify_backend_status can recover the kind afterwards.
          return Err(format!("({status}) {text}"));
        }
        response
          .json::<RemoteSessionOutcome>()
          .await
          .map_err(|e| format!("decode response: {e}"))
      }
    })
    .await
    .map_err(|e| classify_error_string(&e))
}

/// What the backend returns when a session is stopped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndRemoteSessionOutcome {
  pub session_id: String,
  pub status: String,
  /// What the session actually cost, in seconds.
  pub billed_seconds: u64,
}

/// Ask donutbrowser-infra to stop a remote session.
///
/// Without this the only thing that ends a session is the fleet's own two-hour
/// cap, so every launch bills the full 7200s however briefly it was used — a
/// handful of runs exhausts an allowance meant for a hundred. The backend
/// refuses to retire a row it could not stop on the fleet, so a successful
/// return here means the browser is really down and the profile lock released.
pub async fn end_remote_session(
  session_id: &str,
) -> Result<EndRemoteSessionOutcome, RemoteSessionError> {
  let endpoint = format!(
    "{}/api/remote-sessions/{}",
    crate::cloud_auth::CLOUD_API_URL,
    urlencoding::encode(session_id)
  );

  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let endpoint = endpoint.clone();
      async move {
        let response = reqwest::Client::new()
          .delete(&endpoint)
          .bearer_auth(token)
          .send()
          .await
          .map_err(|e| format!("reach backend: {e}"))?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
          let text = response.text().await.unwrap_or_default();
          return Err(format!("({status}) {text}"));
        }
        response
          .json::<EndRemoteSessionOutcome>()
          .await
          .map_err(|e| format!("decode response: {e}"))
      }
    })
    .await
    .map_err(|e| classify_error_string(&e))
}

/// Recover a typed error from `api_call_with_retry`'s string.
///
/// That helper flattens everything to `String` to do its 401 sniffing, so the
/// status is re-parsed here rather than lost — a 503 surfacing as a generic
/// failure would tell the user their fleet is broken when it is merely busy.
pub fn classify_error_string(message: &str) -> RemoteSessionError {
  match cloud_errors::split_status(message) {
    Some((status, body)) => classify_backend_status(status, body),
    None => RemoteSessionError::Other(message.to_string()),
  }
}

/// A session as the backend currently sees it.
///
/// `POST /api/remote-sessions` hands back the literal string `provisioning`
/// and nothing else, so until this type existed the only way anyone observed a
/// session becoming usable was by reading the production database.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RemoteSessionState {
  pub session_id: String,
  #[serde(default)]
  pub profile_id: Option<String>,
  #[serde(default)]
  pub platform: Option<String>,
  /// `provisioning` | `ready` | `live` | `closed` | `error`.
  ///
  /// Named `state` because that is what `RemoteSessionView` in
  /// donutbrowser-infra actually sends. It carried the name `status` until a
  /// real payload was compared against it, and because the field had no
  /// default, every list and single read failed at `missing field \`status\``
  /// and surfaced as CLOUD_UNREACHABLE. The alias keeps the launch reply —
  /// which predates the reconciled vocabulary and still says `status` —
  /// decoding through the same type.
  #[serde(rename = "state", alias = "status")]
  pub state: String,
  /// The relay is up, so the session can actually be driven.
  #[serde(default)]
  pub cdp_ready: bool,
  /// `interactive` or `cookie_bot`.
  #[serde(default)]
  pub kind: Option<String>,
  /// Set when this session belongs to a cookie-bot run.
  #[serde(default)]
  pub run_id: Option<String>,
  /// The team the hours are attributed to, when the caller belongs to one.
  #[serde(default)]
  pub team_id: Option<String>,
  #[serde(default)]
  pub started_at: Option<String>,
  /// When it finished. The backend sends one timestamp, not a
  /// `ready_at`/`closed_at` pair.
  #[serde(default)]
  pub ended_at: Option<String>,
  /// Why it ended: `stopped_by_user`, `max_duration`, `lost the profile lock`…
  #[serde(default)]
  pub close_reason: Option<String>,
  /// What it has cost so far. A live session is already being charged, so this
  /// is the running wall clock rather than 0 until it closes.
  #[serde(default)]
  pub billed_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteSessionListResponse {
  #[serde(default)]
  sessions: Vec<RemoteSessionState>,
}

/// Every session the caller currently owns.
pub async fn list_remote_sessions() -> Result<Vec<RemoteSessionState>, RemoteSessionError> {
  let endpoint = format!("{}/api/remote-sessions", crate::cloud_auth::CLOUD_API_URL);
  let response: RemoteSessionListResponse = get_json(endpoint).await?;
  Ok(response.sessions)
}

/// One session's real state, for a one-shot read.
///
/// The event stream is how the desktop normally learns a transition; this is
/// for the cases a stream cannot serve — a window opened after the fact, or a
/// reconnect that needs to confirm what it missed.
pub async fn get_remote_session(
  session_id: &str,
) -> Result<RemoteSessionState, RemoteSessionError> {
  let endpoint = format!(
    "{}/api/remote-sessions/{}",
    crate::cloud_auth::CLOUD_API_URL,
    urlencoding::encode(session_id)
  );
  get_json(endpoint).await
}

async fn get_json<T: serde::de::DeserializeOwned>(
  endpoint: String,
) -> Result<T, RemoteSessionError> {
  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let endpoint = endpoint.clone();
      async move {
        let response = reqwest::Client::new()
          .get(&endpoint)
          .bearer_auth(token)
          .send()
          .await
          .map_err(|e| format!("reach backend: {e}"))?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
          let text = response.text().await.unwrap_or_default();
          return Err(format!("({status}) {text}"));
        }
        response
          .json::<T>()
          .await
          .map_err(|e| format!("decode response: {e}"))
      }
    })
    .await
    .map_err(|e| classify_error_string(&e))
}

// --- Live state, without polling -------------------------------------------

/// A session transition. Payload is the session as the backend sees it.
pub const EVENT_SESSION_STATE: &str = "remote-session-state";
/// Everything the caller owns, sent once when the stream connects.
pub const EVENT_SESSION_SNAPSHOT: &str = "remote-session-snapshot";
/// Whether the desktop is currently receiving transitions.
pub const EVENT_STREAM_STATUS: &str = "remote-session-stream";

/// How long a silent stream is trusted before it is treated as dead.
///
/// The backend heartbeats, so silence past this means the socket died without
/// an error — which is exactly what a laptop returning from sleep sees. Held
/// well above the heartbeat interval so a slow network cannot cause a churn of
/// reconnects.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// First reconnect delay. Doubles per failure.
const RECONNECT_BASE: Duration = Duration::from_secs(1);
/// Ceiling on the reconnect delay.
const RECONNECT_MAX: Duration = Duration::from_secs(60);
/// Where the backoff restarts after an auth failure. Being signed out or
/// unentitled is not something a fast retry fixes, and hammering an endpoint
/// that will keep saying no is how a background task becomes a battery bug.
const AUTH_BACKOFF_ATTEMPT: u32 = 6;
/// Granularity of the cancellable sleep, so a shutdown is not held up by a
/// minute-long backoff.
const SHUTDOWN_POLL: Duration = Duration::from_millis(250);

static STREAM_RUNNING: AtomicBool = AtomicBool::new(false);
static STREAM_TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);

/// One frame off the wire.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseFrame {
  pub id: Option<String>,
  pub event: Option<String>,
  pub data: String,
}

/// Incremental `text/event-stream` decoder.
///
/// Kept as a value with no IO so the framing rules — multi-line data, the
/// blank-line terminator, comments, CRLF, a chunk boundary landing mid-field —
/// are testable without a server.
#[derive(Debug, Default)]
pub struct SseDecoder {
  buffer: Vec<u8>,
  event: Option<String>,
  data: String,
  id: Option<String>,
}

impl SseDecoder {
  pub fn new() -> Self {
    Self::default()
  }

  /// Feed bytes, get back whatever frames completed.
  pub fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
    self.buffer.extend_from_slice(chunk);
    let mut frames = Vec::new();

    // A newline is never part of a multi-byte UTF-8 sequence, so splitting the
    // raw bytes on it cannot cut a character in half.
    while let Some(index) = self.buffer.iter().position(|b| *b == b'\n') {
      let line: Vec<u8> = self.buffer.drain(..=index).collect();
      let line = String::from_utf8_lossy(&line[..line.len() - 1]);
      let line = line.strip_suffix('\r').unwrap_or(&line);

      if line.is_empty() {
        if let Some(frame) = self.take_frame() {
          frames.push(frame);
        }
        continue;
      }
      // A leading colon is a comment; some proxies keep a stream alive with
      // nothing else, so it must not be mistaken for a field.
      if line.starts_with(':') {
        continue;
      }

      let (field, value) = match line.split_once(':') {
        Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
        None => (line, ""),
      };
      match field {
        "event" => self.event = Some(value.to_string()),
        "id" => self.id = Some(value.to_string()),
        "data" => {
          if !self.data.is_empty() {
            self.data.push('\n');
          }
          self.data.push_str(value);
        }
        // `retry` is the server's reconnect hint; this client's own backoff
        // already bounds that, so honouring it would only make the interval
        // less predictable.
        _ => {}
      }
    }

    frames
  }

  fn take_frame(&mut self) -> Option<SseFrame> {
    let event = self.event.take();
    let id = self.id.take();
    let data = std::mem::take(&mut self.data);
    if data.is_empty() && event.is_none() {
      return None;
    }
    Some(SseFrame { id, event, data })
  }
}

/// A frame that carries nothing the frontend needs.
fn is_heartbeat(kind: &str) -> bool {
  matches!(kind, "heartbeat" | "ping" | "keepalive")
}

/// Turn one decoded frame into the Tauri event and payload it becomes.
///
/// The discriminator lives INSIDE the JSON, not in the SSE `event:` line: Nest
/// only sets `MessageEvent.type` for the heartbeat, so every real frame arrives
/// as the default `message` event carrying
/// `{"type":"snapshot"|"state"|"progress"|"closed","at":…,"sessions"|"session":…}`.
/// Routing on the event name alone emitted that whole envelope as a session, so
/// `profile_id` was always undefined and the frontend dropped every transition
/// — the desktop stayed exactly as blind between launch and stop as it was
/// before the stream existed.
///
/// The SSE name is still honoured when there is one, so a backend that starts
/// naming its frames keeps working without a desktop release.
pub fn route_frame(event: Option<&str>, data: &str) -> Option<(&'static str, serde_json::Value)> {
  if let Some(name) = event {
    if is_heartbeat(name) {
      return None;
    }
  }

  let payload = match serde_json::from_str::<serde_json::Value>(data) {
    Ok(value) => value,
    Err(e) => {
      log::warn!("Ignoring malformed remote-session event: {e}");
      return None;
    }
  };
  let object = payload.as_object()?;

  let kind = object
    .get("type")
    .and_then(serde_json::Value::as_str)
    .or(event)
    .unwrap_or("state");
  if is_heartbeat(kind) {
    return None;
  }

  if kind == "snapshot" {
    let sessions = object
      .get("sessions")
      .cloned()
      .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
    return Some((
      EVENT_SESSION_SNAPSHOT,
      serde_json::json!({ "sessions": sessions }),
    ));
  }

  // `state`, `progress` and `closed` all wrap one session. A frame that
  // carries neither an inner `session` nor a session of its own is not
  // something a consumer can apply, and forwarding it is how the envelope bug
  // happened in the first place.
  if let Some(session) = object.get("session").filter(|v| v.is_object()) {
    return Some((EVENT_SESSION_STATE, session.clone()));
  }
  if object.contains_key("session_id") {
    return Some((EVENT_SESSION_STATE, payload));
  }
  log::warn!("Ignoring remote-session frame with no session: {kind}");
  None
}

/// Delay before reconnect attempt `attempt`, doubling to a ceiling.
pub fn reconnect_delay(attempt: u32) -> Duration {
  let factor = 1u64.checked_shl(attempt.min(16)).unwrap_or(u64::MAX);
  RECONNECT_BASE
    .saturating_mul(factor.min(u32::MAX as u64) as u32)
    .min(RECONNECT_MAX)
}

/// Start receiving session transitions. Idempotent: a second call while the
/// stream is up is a no-op rather than a second socket.
pub fn start_session_events(app: AppHandle) {
  if STREAM_RUNNING.swap(true, Ordering::SeqCst) {
    return;
  }
  let handle = tauri::async_runtime::spawn(async move {
    run_session_events(app).await;
  });
  if let Ok(mut slot) = STREAM_TASK.lock() {
    *slot = Some(handle);
  }
}

/// Stop receiving. Safe to call when nothing is running.
pub fn stop_session_events() {
  if !STREAM_RUNNING.swap(false, Ordering::SeqCst) {
    return;
  }
  if let Ok(mut slot) = STREAM_TASK.lock() {
    if let Some(handle) = slot.take() {
      handle.abort();
    }
  }
}

/// Whether the subscriber task is alive.
pub fn session_events_running() -> bool {
  STREAM_RUNNING.load(Ordering::SeqCst)
}

async fn run_session_events(app: AppHandle) {
  let mut attempt = 0u32;
  // Echoed back on reconnect as `Last-Event-ID`, per the SSE spec, IF the
  // backend ever labels its frames. It does not today — `stream()` emits no
  // `id:` line and keeps no replay buffer — so this stays `None` and nothing is
  // resumed. What bounds the loss instead is the stream opening with a full
  // snapshot, which re-states every session the caller still owns.
  let mut last_event_id: Option<String> = None;

  while STREAM_RUNNING.load(Ordering::SeqCst) {
    match connect_session_events(last_event_id.as_deref()).await {
      Ok(response) => {
        attempt = 0;
        emit_stream_status(&app, true, None);
        match consume_session_events(&app, response, &mut last_event_id).await {
          Ok(()) => {
            log::info!("Remote session stream closed by the backend");
            emit_stream_status(&app, false, None);
          }
          Err(reason) => {
            log::warn!("Remote session stream ended: {reason}");
            emit_stream_status(&app, false, Some(&reason));
          }
        }
      }
      Err(err) => {
        let reason = err.to_string();
        if matches!(err, RemoteSessionError::NotAuthorised(_)) {
          attempt = attempt.max(AUTH_BACKOFF_ATTEMPT);
        }
        log::warn!("Remote session stream could not connect: {reason}");
        emit_stream_status(&app, false, Some(&reason));
      }
    }

    if !STREAM_RUNNING.load(Ordering::SeqCst) {
      break;
    }
    let delay = jittered(reconnect_delay(attempt));
    attempt = attempt.saturating_add(1);
    sleep_unless_stopped(delay).await;
  }

  log::info!("Remote session stream stopped");
}

/// Spread reconnects so every desktop that lost the same backend does not come
/// back in the same millisecond.
fn jittered(delay: Duration) -> Duration {
  use rand::RngExt;
  let factor = rand::rng().random_range(0.8f64..1.2f64);
  delay.mul_f64(factor)
}

async fn sleep_unless_stopped(total: Duration) {
  let mut slept = Duration::ZERO;
  while slept < total && STREAM_RUNNING.load(Ordering::SeqCst) {
    let step = SHUTDOWN_POLL.min(total - slept);
    tokio::time::sleep(step).await;
    slept += step;
  }
}

/// The stream's own HTTP client.
///
/// Deliberately not the shared one: a total request timeout would kill a
/// healthy stream on schedule, so only the connect phase is bounded and
/// liveness is enforced by the idle timeout instead.
fn stream_client() -> &'static reqwest::Client {
  static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
  CLIENT.get_or_init(|| {
    reqwest::Client::builder()
      .connect_timeout(Duration::from_secs(10))
      .build()
      .unwrap_or_else(|_| reqwest::Client::new())
  })
}

async fn connect_session_events(
  last_event_id: Option<&str>,
) -> Result<reqwest::Response, RemoteSessionError> {
  let endpoint = format!(
    "{}/api/remote-sessions/events",
    crate::cloud_auth::CLOUD_API_URL
  );

  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let endpoint = endpoint.clone();
      let resume_from = last_event_id.map(str::to_string);
      async move {
        // Going through api_call_with_retry means a token that expired during
        // a long stream is refreshed on the reconnect instead of turning a
        // signed-in desktop into a permanently silent one.
        let mut request = stream_client()
          .get(&endpoint)
          .bearer_auth(token)
          .header(reqwest::header::ACCEPT, "text/event-stream")
          .header(reqwest::header::CACHE_CONTROL, "no-cache");
        if let Some(id) = resume_from {
          request = request.header("Last-Event-ID", id);
        }

        let response = request
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
    .map_err(|e| classify_error_string(&e))
}

async fn consume_session_events(
  app: &AppHandle,
  response: reqwest::Response,
  last_event_id: &mut Option<String>,
) -> Result<(), String> {
  use futures_util::StreamExt;

  let mut stream = response.bytes_stream();
  let mut decoder = SseDecoder::new();

  loop {
    if !STREAM_RUNNING.load(Ordering::SeqCst) {
      return Ok(());
    }

    let next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await;
    let chunk = match next {
      // No heartbeat. The socket is gone even though nothing errored, which
      // is what a machine returning from sleep sees.
      Err(_) => return Err("no heartbeat within the idle timeout".to_string()),
      Ok(None) => return Ok(()),
      Ok(Some(Err(e))) => return Err(format!("stream error: {e}")),
      Ok(Some(Ok(bytes))) => bytes,
    };

    for frame in decoder.push(&chunk) {
      if let Some(id) = &frame.id {
        *last_event_id = Some(id.clone());
      }
      dispatch_frame(app, &frame);
    }
  }
}

fn dispatch_frame(app: &AppHandle, frame: &SseFrame) {
  let Some((target, payload)) = route_frame(frame.event.as_deref(), &frame.data) else {
    return;
  };

  use tauri::Emitter;
  if let Err(e) = app.emit(target, payload) {
    log::warn!("Failed to emit {target}: {e}");
  }
}

fn emit_stream_status(app: &AppHandle, connected: bool, reason: Option<&str>) {
  use tauri::Emitter;
  let payload = serde_json::json!({ "connected": connected, "reason": reason });
  if let Err(e) = app.emit(EVENT_STREAM_STATUS, payload) {
    log::warn!("Failed to emit {EVENT_STREAM_STATUS}: {e}");
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn no_capacity_is_distinguished_from_a_real_failure() {
    // 503 means "come back in a minute", not "something is broken" — conflating
    // them would make a busy fleet look like an outage to the user.
    assert!(matches!(
      classify_backend_status(503, "no macos host free"),
      RemoteSessionError::NoCapacity(_)
    ));
    assert!(matches!(
      classify_backend_status(500, "boom"),
      RemoteSessionError::Other(_)
    ));
  }

  #[test]
  fn conflict_is_surfaced_so_the_user_learns_the_profile_is_open() {
    assert!(matches!(
      classify_backend_status(409, "profile already has a live session"),
      RemoteSessionError::Conflict(_)
    ));
  }

  #[test]
  fn payment_and_auth_failures_map_to_not_authorised() {
    for status in [401u16, 402, 403] {
      assert!(
        matches!(
          classify_backend_status(status, ""),
          RemoteSessionError::NotAuthorised(_)
        ),
        "status {status} should be NotAuthorised"
      );
    }
  }

  #[test]
  fn an_empty_body_still_produces_a_useful_message() {
    let err = classify_backend_status(500, "");
    assert!(err.to_string().contains("500"));
  }

  #[test]
  fn a_status_encoded_error_string_round_trips_to_its_kind() {
    // api_call_with_retry flattens everything to String to sniff for 401s; the
    // status must survive that or a busy fleet looks like a broken one.
    assert!(matches!(
      classify_error_string("(503) no macos host free"),
      RemoteSessionError::NoCapacity(_)
    ));
    assert!(matches!(
      classify_error_string("(409) already running"),
      RemoteSessionError::Conflict(_)
    ));
  }

  #[test]
  fn an_unencoded_error_string_is_not_misread_as_a_status() {
    assert!(matches!(
      classify_error_string("reach backend: connection refused"),
      RemoteSessionError::Other(_)
    ));
  }

  #[test]
  fn the_stop_response_parses_what_the_backend_actually_sends() {
    // Pinned against EndRemoteSessionOutcome in donutbrowser-infra
    // (apps/backend/src/remote-sessions/remote-sessions.service.ts). A field
    // name that does not match makes every stop fail at the decode step, and
    // the session then runs to the 2h cap and bills 7200s — the exact defect
    // this endpoint exists to fix, reintroduced silently.
    let outcome: EndRemoteSessionOutcome =
      serde_json::from_str(r#"{"session_id":"sess-1","status":"closed","billed_seconds":42}"#)
        .expect("the backend's stop payload must deserialize");
    assert_eq!(outcome.session_id, "sess-1");
    assert_eq!(outcome.status, "closed");
    assert_eq!(outcome.billed_seconds, 42);
  }

  #[test]
  fn a_stop_of_an_unknown_session_is_not_reported_as_capacity() {
    // The backend 404s a session that is not the caller's. Mapping that to
    // NoCapacity would tell the user the fleet is busy and invite a retry.
    assert!(matches!(
      classify_error_string("(404) No such remote session"),
      RemoteSessionError::Other(_)
    ));
  }

  #[test]
  fn idempotency_key_is_stable_for_one_attempt_and_distinct_across_attempts() {
    let a = idempotency_key("p1", "attempt-1");
    assert_eq!(a, idempotency_key("p1", "attempt-1"));
    assert_ne!(a, idempotency_key("p1", "attempt-2"));
    assert_ne!(a, idempotency_key("p2", "attempt-1"));
  }

  #[test]
  fn a_typed_failure_becomes_a_code_the_frontend_can_translate() {
    // The variants carry the backend's English. Surfacing that verbatim is
    // how an untranslated string reaches a Russian user.
    let busy = RemoteSessionError::NoCapacity("no macos host free".to_string());
    assert_eq!(busy.to_error_json(), r#"{"code":"REMOTE_NO_CAPACITY"}"#);

    let taken = RemoteSessionError::Conflict("profile already has a live session".to_string());
    assert_eq!(
      taken.to_error_json(),
      r#"{"code":"REMOTE_SESSION_CONFLICT"}"#
    );
  }

  #[test]
  fn a_backend_supplied_code_survives_the_trip_through_the_typed_error() {
    // Once infra sends an envelope, its code must win over the status default
    // — "you are out of hours" and "the fleet is full" are both refusals but
    // only one of them is worth retrying.
    let err =
      classify_error_string(r#"(403) {"code":"REMOTE_HOURS_EXHAUSTED","granted":200,"used":200}"#);
    let json: serde_json::Value =
      serde_json::from_str(&err.to_error_json()).expect("valid envelope");
    assert_eq!(json["code"], "REMOTE_HOURS_EXHAUSTED");
    assert_eq!(json["params"]["granted"], "200");
  }

  /// A verbatim `RemoteSessionView`, field for field, as `toView` in
  /// donutbrowser-infra's `remote-sessions.service.ts` builds it.
  ///
  /// Hand-written JSON is what let this type declare `status`, `ready_at` and
  /// `closed_at` while the backend sent `state` and `ended_at`: the test agreed
  /// with the type and neither agreed with the server, so every list and single
  /// read failed to decode in production and passed in CI.
  const SERVER_SESSION_VIEW: &str = r#"{
    "session_id":"sess-1","profile_id":"p1","platform":"macos","kind":"cookie_bot",
    "run_id":"r1","team_id":"t1","state":"live","cdp_ready":true,
    "started_at":"2026-08-03T00:00:00.000Z","ended_at":null,
    "billed_seconds":1830,"close_reason":null
  }"#;

  #[test]
  fn the_session_state_payload_matches_what_the_backend_sends() {
    // The desktop has been blind between launch and stop; every field here is
    // one it could previously only learn by reading the production database.
    let state: RemoteSessionState = serde_json::from_str(SERVER_SESSION_VIEW)
      .expect("the backend's session payload must deserialize");

    assert_eq!(state.state, "live");
    assert!(state.cdp_ready);
    assert_eq!(state.run_id.as_deref(), Some("r1"));
    assert_eq!(state.team_id.as_deref(), Some("t1"));
    assert_eq!(state.kind.as_deref(), Some("cookie_bot"));
    assert_eq!(state.billed_seconds, Some(1830));
    assert!(state.ended_at.is_none());
  }

  #[test]
  fn a_list_response_of_real_server_views_decodes() {
    // `list_remote_sessions` is the fallback for everything the stream cannot
    // serve. It returned Err("decode response: missing field `status`") on
    // every call for as long as this type disagreed with `toView`.
    let body = format!(r#"{{"sessions":[{SERVER_SESSION_VIEW}]}}"#);
    let response: RemoteSessionListResponse =
      serde_json::from_str(&body).expect("the backend's list payload must deserialize");
    assert_eq!(response.sessions.len(), 1);
    assert_eq!(response.sessions[0].state, "live");
  }

  #[test]
  fn the_older_status_key_from_the_launch_reply_still_decodes() {
    // `POST /api/remote-sessions` predates the reconciled vocabulary and
    // answers `status`. One type reads both rather than two types drifting.
    let state: RemoteSessionState =
      serde_json::from_str(r#"{"session_id":"s1","status":"provisioning"}"#)
        .expect("a fresh session must deserialize");
    assert_eq!(state.state, "provisioning");
    assert!(!state.cdp_ready);
    assert!(state.platform.is_none());
  }

  #[test]
  fn a_close_transition_carries_what_the_session_cost() {
    let state: RemoteSessionState = serde_json::from_str(
      r#"{"session_id":"s1","state":"closed","close_reason":"stopped_by_user",
          "ended_at":"2026-08-03T01:30:00.000Z","billed_seconds":1830}"#,
    )
    .expect("a close payload must deserialize");
    assert_eq!(state.billed_seconds, Some(1830));
    assert_eq!(state.close_reason.as_deref(), Some("stopped_by_user"));
    assert!(state.ended_at.is_some());
  }

  #[test]
  fn an_error_state_decodes_rather_than_being_treated_as_unknown() {
    // `error` is one of the five states the backend reconciles to. A session
    // that failed on the fleet must reach the UI as itself.
    let state: RemoteSessionState =
      serde_json::from_str(r#"{"session_id":"s1","state":"error","close_reason":"agent_lost"}"#)
        .expect("an error payload must deserialize");
    assert_eq!(state.state, "error");
  }

  #[test]
  fn the_decoder_reads_a_whole_frame() {
    let mut decoder = SseDecoder::new();
    let frames = decoder.push(b"event: session\nid: 7\ndata: {\"status\":\"ready\"}\n\n");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].event.as_deref(), Some("session"));
    assert_eq!(frames[0].id.as_deref(), Some("7"));
    assert_eq!(frames[0].data, r#"{"status":"ready"}"#);
  }

  #[test]
  fn a_frame_split_across_chunks_is_not_lost() {
    // TCP does not respect frame boundaries. Dropping a half-arrived frame
    // would silently lose the transition that says the browser is ready.
    let mut decoder = SseDecoder::new();
    assert!(decoder.push(b"event: session\ndata: {\"sta").is_empty());
    assert!(decoder.push(b"tus\":\"live\"}").is_empty());
    let frames = decoder.push(b"\n\n");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].data, r#"{"status":"live"}"#);
  }

  #[test]
  fn several_frames_in_one_chunk_all_arrive() {
    let mut decoder = SseDecoder::new();
    let frames = decoder.push(b"data: 1\n\ndata: 2\n\ndata: 3\n\n");
    let payloads: Vec<&str> = frames.iter().map(|f| f.data.as_str()).collect();
    assert_eq!(payloads, vec!["1", "2", "3"]);
  }

  #[test]
  fn comments_and_crlf_framing_do_not_produce_phantom_events() {
    // A proxy that keeps the connection warm with `:` lines must not look
    // like a stream of empty transitions.
    let mut decoder = SseDecoder::new();
    let frames = decoder.push(b": keep-alive\r\n\r\ndata: {}\r\n\r\n");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].data, "{}");
  }

  #[test]
  fn multi_line_data_is_rejoined_with_newlines() {
    let mut decoder = SseDecoder::new();
    let frames = decoder.push(b"data: {\ndata: \"a\": 1\ndata: }\n\n");
    assert_eq!(frames[0].data, "{\n\"a\": 1\n}");
  }

  /// Route whatever the decoder makes of a literal wire capture, so the test
  /// exercises the same two steps production does.
  fn route_wire(bytes: &[u8]) -> Vec<(&'static str, serde_json::Value)> {
    let mut decoder = SseDecoder::new();
    decoder
      .push(bytes)
      .iter()
      .filter_map(|frame| route_frame(frame.event.as_deref(), &frame.data))
      .collect()
  }

  #[test]
  fn a_heartbeat_is_not_forwarded_to_the_frontend() {
    // Emitting one would make every consumer re-render twice a minute for
    // nothing. Nest names this one, so it arrives with an `event:` line.
    assert!(route_wire(b"event: ping\ndata: {}\n\n").is_empty());
    assert!(route_wire(b"event: heartbeat\ndata: {}\n\n").is_empty());
    // And the same frame with the discriminator inside the JSON instead.
    assert!(
      route_wire(b"data: {\"type\":\"ping\",\"at\":\"2026-08-03T00:00:00.000Z\"}\n\n").is_empty()
    );
  }

  #[test]
  fn the_opening_snapshot_reaches_the_snapshot_event() {
    // Byte-for-byte what Nest writes for `{type:'snapshot',at,sessions}`: no
    // `event:` line, because the controller only sets MessageEvent.type for the
    // ping. Routing on the event NAME sent this to `remote-session-state` as a
    // raw envelope, so `remote-session-snapshot` was never emitted at all and
    // the live view started empty and stayed empty.
    let routed = route_wire(
      b"data: {\"type\":\"snapshot\",\"at\":\"2026-08-03T00:00:00.000Z\",\"sessions\":[{\"session_id\":\"s1\",\"profile_id\":\"p1\",\"state\":\"live\"}]}\n\n",
    );
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].0, EVENT_SESSION_SNAPSHOT);
    assert_eq!(routed[0].1["sessions"][0]["profile_id"], "p1");
  }

  #[test]
  fn a_transition_is_unwrapped_to_the_session_the_frontend_indexes_by() {
    // The consumer keys `liveSessions` by `profile_id`. Emitting the envelope
    // meant every frame hit `if (!session.profile_id) return;` and a live run
    // showed as idle for its whole duration.
    for kind in ["state", "progress", "closed"] {
      let wire = format!(
        "data: {{\"type\":\"{kind}\",\"at\":\"2026-08-03T00:00:00.000Z\",\"session\":{{\"session_id\":\"s1\",\"profile_id\":\"p1\",\"state\":\"live\",\"cdp_ready\":true}}}}\n\n"
      );
      let routed = route_wire(wire.as_bytes());
      assert_eq!(routed.len(), 1, "{kind} must produce one event");
      assert_eq!(routed[0].0, EVENT_SESSION_STATE);
      assert_eq!(routed[0].1["profile_id"], "p1", "{kind} must be unwrapped");
      assert_eq!(routed[0].1["state"], "live");
      // And the payload must deserialize as the type the one-shot reads use.
      let session: RemoteSessionState = serde_json::from_value(routed[0].1.clone())
        .expect("a streamed session must decode as RemoteSessionState");
      assert_eq!(session.session_id, "s1");
    }
  }

  #[test]
  fn a_named_event_carrying_a_bare_session_is_still_routed() {
    // If the backend starts labelling its frames and drops the envelope, the
    // desktop must not need a release to keep working.
    let routed = route_wire(
      b"event: state\ndata: {\"session_id\":\"s1\",\"profile_id\":\"p1\",\"state\":\"ready\"}\n\n",
    );
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].0, EVENT_SESSION_STATE);
    assert_eq!(routed[0].1["profile_id"], "p1");

    let snapshot =
      route_wire(b"event: snapshot\ndata: {\"type\":\"snapshot\",\"sessions\":[]}\n\n");
    assert_eq!(snapshot[0].0, EVENT_SESSION_SNAPSHOT);
  }

  #[test]
  fn a_frame_carrying_no_session_is_dropped_rather_than_emitted_raw() {
    // Forwarding an envelope the consumer cannot apply is exactly the bug this
    // routing exists to close; a malformed frame must be silent, not wrong.
    assert!(
      route_wire(b"data: {\"type\":\"state\",\"at\":\"2026-08-03T00:00:00.000Z\"}\n\n").is_empty()
    );
    assert!(route_wire(b"data: not json\n\n").is_empty());
    assert!(route_wire(b"data: []\n\n").is_empty());
  }

  #[test]
  fn reconnect_backs_off_and_stops_growing() {
    assert_eq!(reconnect_delay(0), Duration::from_secs(1));
    assert_eq!(reconnect_delay(3), Duration::from_secs(8));
    // A backend that is down for an hour must not be probed thousands of
    // times, nor overflow the shift.
    assert_eq!(reconnect_delay(20), RECONNECT_MAX);
    assert_eq!(reconnect_delay(u32::MAX), RECONNECT_MAX);
  }

  #[test]
  fn the_auth_backoff_floor_is_far_longer_than_the_first_retry() {
    // Being signed out or unentitled is not fixed by retrying in a second.
    assert!(reconnect_delay(AUTH_BACKOFF_ATTEMPT) >= Duration::from_secs(30));
  }

  #[test]
  fn jitter_stays_within_a_fifth_of_the_delay() {
    let base = Duration::from_secs(10);
    for _ in 0..64 {
      let delay = jittered(base);
      assert!(
        delay >= Duration::from_secs(8) && delay <= Duration::from_secs(12),
        "jitter escaped its bounds: {delay:?}"
      );
    }
  }

  #[tokio::test]
  async fn a_stopped_stream_does_not_wait_out_its_backoff() {
    // A minute-long backoff must not hold up app shutdown.
    STREAM_RUNNING.store(true, Ordering::SeqCst);
    let started = std::time::Instant::now();
    let sleeper = tokio::spawn(sleep_unless_stopped(Duration::from_secs(60)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    STREAM_RUNNING.store(false, Ordering::SeqCst);
    sleeper.await.expect("the sleep task must finish");
    assert!(
      started.elapsed() < Duration::from_secs(5),
      "shutdown waited on the backoff"
    );
  }

  #[test]
  fn stopping_a_stream_that_never_started_is_harmless() {
    STREAM_RUNNING.store(false, Ordering::SeqCst);
    stop_session_events();
    assert!(!session_events_running());
  }
}

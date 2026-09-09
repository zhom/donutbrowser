//! The remote-control bridge: one outbound socket that lets Donut cloud drive
//! this installation's MCP tools.
//!
//! The local MCP server in [`crate::mcp_server`] answers on loopback, which is
//! only reachable by an agent running on this machine. Remote control inverts
//! the reach without inverting the trust: nothing dials in to the desktop. The
//! app dials OUT to `wss://api.donutbrowser.com/api/mcp-bridge`, proves who it
//! is with the same cloud access token every other cloud call uses, and then
//! answers JSON-RPC that arrives down that socket.
//!
//! That direction is the whole security argument. There is no listening port to
//! find, no inbound firewall hole, no credential parked on a server that could
//! drive a customer's browser if it leaked, the desktop can hang up at any
//! time and the capability disappears with it.
//!
//! ## The wire
//!
//! Text frames of JSON, versioned by [`BRIDGE_PROTOCOL`]. Server to app:
//!
//! ```jsonc
//! {"t":"hello","protocol":"donut-mcp-bridge/1","instanceId":"…"}
//! {"t":"rpc","cid":"…","sessionId":"…|null","payload":{ /* JSON-RPC */ }}
//! {"t":"endSession","sessionId":"…"}
//! ```
//!
//! App to server, one `result` per `rpc`, correlated by `cid`:
//!
//! ```jsonc
//! {"t":"result","cid":"…","status":"ok","sessionId":"…|null","payload":{…}}
//! {"t":"result","cid":"…","status":"accepted"}          // a notification
//! {"t":"result","cid":"…","status":"unknownSession"}
//! {"t":"result","cid":"…","status":"badRequest"}
//! {"t":"result","cid":"…","status":"rateLimited","retryAfter":42}
//! {"t":"result","cid":"…","status":"busy"}
//! {"t":"result","cid":"…","status":"tooLarge"}
//! ```
//!
//! The statuses are [`McpOutcome`]'s variants PLUS the two this transport
//! produces on its own, `busy` when the in-flight cap is spent, and
//! `tooLarge` when an answer exceeds the frame budget. Each status corresponds
//! to the HTTP status a local MCP client would have seen, so a caller needs no
//! special case for the remote transport.
//!
//! Liveness is protocol-level: the server PINGs, we PONG, and silence past
//! [`IDLE_TIMEOUT`] is treated as a dead socket. No JSON heartbeat is injected
//! into the stream, because a frame an MCP client did not ask for is a frame it
//! has to be taught to ignore.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::AppHandle;
use tokio::sync::{mpsc, Semaphore};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request as WsRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::cloud_auth::{CLOUD_API_URL, CLOUD_AUTH};
use crate::mcp_server::{McpOutcome, McpServer};

/// The wire contract's version. Bumped only for a change a current desktop
/// could not understand, so the relay can refuse a build it cannot talk to
/// instead of failing one frame at a time.
pub const BRIDGE_PROTOCOL: &str = "donut-mcp-bridge/1";

/// Path on the cloud API. Absolute rather than derived, and paired with the
/// protocol string above so the two can never be changed apart.
const BRIDGE_PATH: &str = "/api/mcp-bridge";

/// Emitted whenever the bridge's connection state changes.
pub const EVENT_MCP_REMOTE_STATUS: &str = "mcp-remote-status";

/// Silence that means the socket is gone.
///
/// Matches the idle budget of the endpoint: several missed protocol pings mean
/// a dead peer rather than a slow one, and an idle WebSocket is closed upstream
/// well before this, which is exactly what the pings prevent.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// How many tool calls may be in flight at once.
///
/// A cap, not a queue: the relay is ours and paces itself, but a bug on either
/// side must not be able to spawn unbounded work inside a customer's app. Past
/// this the app answers `busy`, which the caller can retry, rather than
/// accepting work it will not get to.
const MAX_IN_FLIGHT: usize = 8;

/// The in-flight budget, owned for the life of the bridge rather than per
/// socket.
///
/// `pump` used to create its own. But a socket teardown DELIBERATELY abandons
/// in-flight calls rather than aborting them, half of them are launching
/// browsers, so those tasks keep holding permits from the semaphore their old
/// socket owned. A reconnect then handed the new socket a fresh full budget,
/// and across a flapping connection the cap that exists to protect the
/// customer's machine stopped bounding anything. Held here, an abandoned call
/// still occupies its slot until it finishes, which is the whole point.
static BRIDGE_PERMITS: std::sync::LazyLock<Arc<Semaphore>> =
  std::sync::LazyLock::new(|| Arc::new(Semaphore::new(MAX_IN_FLIGHT)));

/// The maximum a single `result` frame may be.
///
/// A screenshot is base64 and genuinely large, so this is generous. It exists so
/// a runaway `get_page_content` cannot try to push an unbounded frame through a
/// socket whose far end will drop it anyway.
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

/// How long a dead socket's writer may take to drain before it is abandoned.
///
/// Long enough for a queued result to reach a socket that is merely slow, short
/// enough that reconnecting is never held up by a tool call the answer of which
/// nobody can receive any more.
const WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

static BRIDGE_RUNNING: AtomicBool = AtomicBool::new(false);
static BRIDGE_TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);
static BRIDGE_CONNECTED: AtomicBool = AtomicBool::new(false);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// What the Integrations page shows about remote control.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRemoteStatus {
  /// Whether the bridge task is meant to be up. Distinct from `connected`: a
  /// bridge that is enabled but reconnecting is a different thing to say than
  /// one that is switched off.
  pub enabled: bool,
  pub connected: bool,
  pub instance_id: String,
  /// Why the last attempt failed, as a `{"code": …}` envelope the UI resolves
  /// through `translateBackendError`. Cleared on a good connect.
  ///
  /// A code rather than a message: the underlying reasons are English, and some
  /// are written by the server, so neither can be shown to a customer reading
  /// the app in one of the other nine languages.
  pub last_error: Option<String>,
}

/// Why a connection attempt ended, which decides how hard to back off.
#[derive(Debug)]
enum BridgeError {
  /// The credential was refused. Retrying fast fixes nothing.
  Unauthorized(String),
  /// Another instance of this account holds the single bridge slot.
  SlotTaken(String),
  /// The plan does not include remote control.
  NotEntitled(String),
  /// Anything transient: DNS, TLS, a restarting backend.
  Unreachable(String),
}

impl std::fmt::Display for BridgeError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Unauthorized(reason)
      | Self::SlotTaken(reason)
      | Self::NotEntitled(reason)
      | Self::Unreachable(reason) => write!(f, "{reason}"),
    }
  }
}

impl BridgeError {
  /// The stable code the UI translates this into.
  ///
  /// The variants carry English prose, some of it the relay's own close
  /// reason, and that prose is for the log file, where a support engineer
  /// reads it. It must never reach the screen: the Integrations page is
  /// localised into ten languages, and rendering a server-authored English
  /// sentence under a Japanese UI is exactly what the translation rule exists
  /// to stop. So the status carries a code and the words are chosen locally.
  fn code(&self) -> &'static str {
    match self {
      Self::Unauthorized(_) => "MCP_REMOTE_UNAUTHORIZED",
      Self::SlotTaken(_) => "MCP_REMOTE_SLOT_TAKEN",
      Self::NotEntitled(_) => "MCP_REMOTE_NOT_ENTITLED",
      Self::Unreachable(_) => "MCP_REMOTE_UNREACHABLE",
    }
  }

  /// Whether a fast retry could plausibly succeed. It cannot for any refusal
  /// the server will keep repeating, and a client that retries one of those on
  /// a one-second timer is a battery bug wearing a reconnect loop.
  fn is_terminal_refusal(&self) -> bool {
    matches!(
      self,
      Self::Unauthorized(_) | Self::SlotTaken(_) | Self::NotEntitled(_)
    )
  }
}

/// The instance-id shape the bridge endpoint accepts.
///
/// Checked HERE rather than relying on the server to complain: a persisted id
/// that fails the shape is refused at the upgrade with "an instance id is
/// required", and the desktop can only guess what that means. Regenerating a
/// malformed one removes the condition instead of improving the error message
/// for it.
fn is_valid_instance_id(value: &str) -> bool {
  (8..=64).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// This installation's stable identity on the bridge.
///
/// Stable across restarts on purpose. The bridge slot is granted per instance
/// id, and a reconnect after a network blip must be able to take its OWN slot
/// back rather than be refused until the stale one is cleaned up. A fresh id
/// per process would make every dropped socket a minutes-long outage.
///
/// Not a secret and not a credential: it names a machine, it does not
/// authenticate one. The access token does that.
pub fn instance_id() -> String {
  static CACHED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
  CACHED
    .get_or_init(|| read_or_create_instance_id(&instance_id_path()))
    .clone()
}

/// Where the id lives, a temp path under `cfg(test)`, the real settings
/// directory otherwise.
///
/// Structural, not per-test discipline. Announcing the connection now happens
/// inside `dispatch`'s hello branch, which reaches `publish_state` -> `status`
/// -> `instance_id`, and that CREATES the file when it is absent. Four tests
/// feed a real `hello` through the real `pump`, so `cargo test` began writing
/// into the developer's own DonutBrowserDev settings directory. Making each of
/// those tests seed a cache would work only until the fifth test forgot; making
/// the PATH itself test-aware cannot be forgotten.
fn instance_id_path() -> std::path::PathBuf {
  #[cfg(test)]
  {
    std::env::temp_dir().join("donut-mcp-instance-id-test")
  }
  #[cfg(not(test))]
  {
    crate::app_dirs::settings_dir().join("mcp_instance_id")
  }
}

/// The body of [`instance_id`], taking its path so it can be tested.
///
/// `instance_id` caches in a `OnceLock`, so a test can only ever observe the
/// first call in the process; keeping the logic here is what lets the
/// regenerate-a-malformed-id behaviour actually be exercised rather than only
/// its helper.
fn read_or_create_instance_id(path: &std::path::Path) -> String {
  if let Ok(existing) = std::fs::read_to_string(path) {
    let trimmed = existing.trim();
    if is_valid_instance_id(trimmed) {
      return trimmed.to_string();
    }
    if !trimmed.is_empty() {
      log::warn!(
        "[mcp-remote] Persisted instance id is not a shape the bridge accepts; regenerating"
      );
    }
  }

  let fresh = uuid::Uuid::new_v4().to_string();
  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  if let Err(e) = std::fs::write(path, &fresh) {
    // A machine that cannot persist this still works; it just loses the
    // reclaim-my-own-slot property until the write succeeds.
    log::warn!("[mcp-remote] Could not persist the instance id: {e}");
  }
  fresh
}

pub fn is_running() -> bool {
  BRIDGE_RUNNING.load(Ordering::SeqCst)
}

pub fn is_connected() -> bool {
  BRIDGE_CONNECTED.load(Ordering::SeqCst)
}

pub fn status() -> McpRemoteStatus {
  McpRemoteStatus {
    enabled: is_running(),
    connected: is_connected(),
    instance_id: instance_id(),
    last_error: LAST_ERROR
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
      .clone(),
  }
}

/// Record the state and tell the screen, WITHOUT needing an `AppHandle`.
///
/// `stop(None)` is how both credential teardowns hang up, logout and the
/// automatic `invalidate_session`, and neither has a handle to pass. Emitting
/// only when one was supplied left the Integrations page reading
/// "Connected as <instance-id>" over a bridge that had already been torn down,
/// until the dialog happened to be reopened.
fn publish_state(connected: bool, error: Option<String>) {
  // A bridge that is not running cannot be connected, so a late "connected"
  // is downgraded rather than believed. `stop` flips BRIDGE_RUNNING first,
  // and cancellation only takes effect at an await point, so the read loop can
  // still be part-way through a frame when the stop lands. Without this the
  // greeting published microseconds later would overwrite the teardown and
  // leave the Integrations page reading "Connected as <instance-id>" over a
  // socket that was hung up and a credential logout had already deleted -
  // exactly the stale-state bug this function was introduced to fix.
  let connected = connected && BRIDGE_RUNNING.load(Ordering::SeqCst);
  BRIDGE_CONNECTED.store(connected, Ordering::SeqCst);
  {
    let mut slot = LAST_ERROR
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = error;
  }
  let _ = crate::events::emit(EVENT_MCP_REMOTE_STATUS, status());
}

/// Start the bridge. Idempotent: a second call while it is up is a no-op rather
/// than a second socket racing the first for the same slot.
pub fn start(app: AppHandle) {
  // The handle is still taken so callers keep a single obvious entry point,
  // and so a future need for it does not churn every call site.
  let _ = app;
  // The task lock is held across BOTH the flag flip and the handle store.
  // With the flag flipped first and the handle stored afterwards, a `stop`
  // landing in between saw the flag, found no handle to abort, and returned;
  // the spawn then parked its handle in the slot with the flag already false,
  // leaving a reconnect loop nobody could stop until the next start replaced
  // the handle. Under the lock a stop either runs before this (and the start
  // proceeds) or after it (and finds the handle).
  let mut slot = BRIDGE_TASK
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  if BRIDGE_RUNNING.swap(true, Ordering::SeqCst) {
    return;
  }
  *slot = Some(tauri::async_runtime::spawn(async move {
    run().await;
  }));
}

/// Stop the bridge. Safe to call when nothing is running.
pub fn stop(app: Option<&AppHandle>) {
  // Same lock, same reason as `start`: the flag and the handle change together.
  let mut slot = BRIDGE_TASK
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  if !BRIDGE_RUNNING.swap(false, Ordering::SeqCst) {
    return;
  }
  if let Some(handle) = slot.take() {
    handle.abort();
  }
  drop(slot);
  // Unconditionally, not only when there is a handle to notify: a stopped
  // bridge has no last error by definition, and the screen has to learn it was
  // stopped even when the caller is a teardown path with no handle.
  publish_state(false, None);
  // The `app` parameter is kept because callers that HAVE a handle read as
  // clearer at the call site, but nothing needs it any more: the global
  // emitter is a `TauriEmitter` installed during setup, so it reaches the
  // frontend identically. Emitting through both sent every status change
  // twice.
  let _ = app;
}

/// The reconnect loop.
///
/// Takes no `AppHandle`: status reaches the screen through the global emitter
/// installed at startup, which is the same `TauriEmitter`. Threading a handle
/// through here as well meant every state change was emitted twice, and left
/// the two teardown paths, which have no handle, unable to emit at all.
async fn run() {
  let mut attempt = 0u32;
  // Whether the current credential has already had its one refresh.
  //
  // Reset only when we have been GREETED, never merely on a handshake: the
  // upgrade completes before the credential is judged, so a completed handshake
  // says nothing about it. Resetting there made the bound inert for the only
  // path that can produce a credential refusal, a close frame, so a desktop
  // that keeps being refused rotated its refresh token on every cycle instead
  // of exactly once.
  let mut refreshed = false;
  let greeted = Arc::new(AtomicBool::new(false));

  while BRIDGE_RUNNING.load(Ordering::SeqCst) {
    // `dialled_with` is the access token the socket was dialled with, so a
    // refusal can be compared against what is on disk NOW. None when the dial
    // itself failed before a credential was read.
    let (failure, dialled_with) = match connect().await {
      Ok((stream, token)) => {
        greeted.store(false, Ordering::SeqCst);
        log::info!("[mcp-remote] Bridge dialled as instance {}", instance_id());

        // The connection is announced by `dispatch`, synchronously, the moment
        // the `hello` is read, NOT here, and no longer from a task of its own.
        // The upgrade completes before the credential is judged, so a completed
        // handshake is not a connection, it is a question that has not been
        // answered yet; announcing on it made a refused customer (an
        // unentitled plan, a taken slot) watch the tab flash connected and then
        // fail once a minute, for ever.
        //
        // Announcing from a spawned task was the previous shape and it was
        // wrong twice. The task waited on a `Notify` and `stop` could not reach
        // it, `BRIDGE_TASK` holds only this loop's handle, and dropping a
        // JoinHandle detaches rather than aborts, so a stop between dialling
        // and greeting parked it on a signal nobody could ever fire again, one
        // leaked task per stop. And `abort` cannot cancel a task that has
        // already passed its last await, so on a greet-then-close socket the
        // announcement could land AFTER the teardown below and leave the screen
        // reading connected with no error. Publishing on this task removes both:
        // there is one publisher, its order is the order of the loop, and a
        // cancellation takes the announcement with it.
        let opened_at = std::time::Instant::now();
        let outcome = pump(stream, Arc::clone(&greeted)).await;
        let lasted = opened_at.elapsed();
        // A `hello` is the first frame that arrives AFTER the credential has
        // been accepted, so it is the only evidence here of that, and therefore
        // the only thing that should clear either counter. Clearing them on the
        // handshake meant a bridge that upgrades and is then refused every time
        // never backed off at all: the upgrade completes before the credential
        // is judged, so `connect()` succeeds on every cycle no matter how
        // hopeless the credential is.
        if greeted.load(Ordering::SeqCst) {
          // The credential was accepted, so it may spend another refresh.
          refreshed = false;

          // The BACKOFF is a separate question, and needs more than a
          // greeting. The `hello` arrives as soon as the socket is registered,
          // before anything has been proved, so a connection that lived five
          // milliseconds reset the counter exactly like one that lived a day,
          // and a close that classifies as non-terminal (an idle 1011, say)
          // then retried at 1 Hz for ever. Only a connection that actually held
          // resets it.
          if lasted >= MIN_UPTIME_FOR_BACKOFF_RESET {
            attempt = 0;
          }
        }
        match &outcome {
          Ok(()) => log::info!("[mcp-remote] Bridge closed by the backend"),
          Err(e) => log::warn!("[mcp-remote] Bridge ended: {e}"),
        }
        // The code for the screen, the prose for the log above.
        publish_state(
          false,
          outcome
            .as_ref()
            .err()
            .map(|e| crate::backend_error(e.code())),
        );
        (outcome.err(), Some(token))
      }
      Err(e) => {
        log::warn!("[mcp-remote] Bridge could not connect: {e}");
        publish_state(false, Some(crate::backend_error(e.code())));
        (Some(e), None)
      }
    };

    if let Some(error) = &failure {
      // An expired access token is the one "unauthorized" that fixes itself,
      // and it MUST be handled here rather than at the dial.
      //
      // A refused credential never arrives as an HTTP 401: the upgrade
      // completes first, and the refusal arrives as a close frame on an
      // already-open socket. A refresh-and-retry hung off the dial therefore
      // never runs, and the desktop sits in the terminal backoff band
      // redialling the same dead token until an unrelated ten-minute loop
      // happens to renew it: minutes of "sign out and sign in again" for a
      // condition that needed neither.
      //
      // First, though: is the refused token still the one on disk? The
      // ten-minute loop and every `api_call_with_retry` also rotate it, and a
      // socket that lived for hours was dialled with a token that has almost
      // certainly been replaced since. Refreshing in that case spends the
      // one-shot on a token that is already gone, and the rotation itself is
      // what was refused. So a stale token is simply redialled with the current
      // one, and the refresh is reserved for a token that was refused while it
      // was still current.
      let rotated = matches!(error, BridgeError::Unauthorized(_))
        && credential_rotated_since(
          dialled_with.as_deref(),
          crate::remote_session::access_token_for_cdp()
            .ok()
            .as_deref(),
        );
      if rotated {
        log::info!(
          "[mcp-remote] The refused access token has already been replaced; redialling with the current one"
        );
        attempt = 0;
      } else if should_refresh_credential(error, refreshed) {
        match CLOUD_AUTH.refresh_access_token().await {
          Ok(()) => {
            log::info!("[mcp-remote] Refreshed the access token; retrying the bridge at once");
            // Spent only NOW. Marking it before the attempt meant a refresh
            // that FAILED, a transient network blip at exactly the wrong
            // moment, permanently disarmed the retry: the reset needs a
            // `hello`, which never arrives for a credential that keeps being
            // refused. The bridge then sat on a dead token until the app
            // restarted.
            refreshed = true;
            attempt = 0;
          }
          Err(e) => {
            log::warn!("[mcp-remote] Could not refresh the access token: {e}");
            attempt = attempt.max(TERMINAL_BACKOFF_ATTEMPT);
          }
        }
      } else if error.is_terminal_refusal() {
        // Everything the server will keep saying no to, a slot conflict, an
        // unentitled plan, a credential a refresh did not fix. Without this a
        // second copy of Donut reconnects on a one-second timer for as long as
        // both are open.
        attempt = attempt.max(TERMINAL_BACKOFF_ATTEMPT);
      }
    }

    if !BRIDGE_RUNNING.load(Ordering::SeqCst) {
      break;
    }
    let delay = crate::remote_session::jittered(crate::remote_session::reconnect_delay(attempt));
    attempt = attempt.saturating_add(1);
    sleep_unless_stopped(delay).await;
  }

  BRIDGE_CONNECTED.store(false, Ordering::SeqCst);
  log::info!("[mcp-remote] Bridge stopped");
}

/// Whether this failure is worth one token refresh and an immediate retry.
///
/// Only `Unauthorized`, and only once per credential. A slot conflict or an
/// unentitled plan is not something a new token fixes, and retrying either on a
/// fast timer is how a background task becomes a battery bug.
fn should_refresh_credential(error: &BridgeError, already_refreshed: bool) -> bool {
  !already_refreshed && matches!(error, BridgeError::Unauthorized(_))
}

/// Whether the token on disk is no longer the one that was refused.
///
/// Only a KNOWN refused token that differs from a KNOWN current one counts.
/// With nothing to compare (the dial failed before reading a credential, or
/// there is no credential now) the answer is no, and the refusal takes the
/// refresh path or the terminal backoff as it always did.
fn credential_rotated_since(refused: Option<&str>, current: Option<&str>) -> bool {
  match (refused, current) {
    (Some(refused), Some(current)) => refused != current,
    _ => false,
  }
}

/// Where the backoff restarts after a refusal the server will keep repeating.
/// Shares the SSE stream's constant so one deployment has one answer to "how
/// long before we ask again".
const TERMINAL_BACKOFF_ATTEMPT: u32 = 6;

/// How long a connection must hold before it counts as "working" for backoff.
///
/// Shorter than the idle timeout upstream, so a healthy but quiet bridge still
/// resets, and far longer than the greet-then-close cycle that made the retry
/// loop spin.
const MIN_UPTIME_FOR_BACKOFF_RESET: Duration = Duration::from_secs(60);

/// Granularity of the cancellable sleep, so switching remote control off is not
/// held up by a minute-long backoff.
const SHUTDOWN_POLL: Duration = Duration::from_millis(250);

async fn sleep_unless_stopped(total: Duration) {
  let mut slept = Duration::ZERO;
  while slept < total && BRIDGE_RUNNING.load(Ordering::SeqCst) {
    let step = SHUTDOWN_POLL.min(total - slept);
    tokio::time::sleep(step).await;
    slept += step;
  }
}

type BridgeStream =
  tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn bridge_url() -> String {
  format!(
    "{}{BRIDGE_PATH}",
    CLOUD_API_URL.replacen("https://", "wss://", 1)
  )
}

/// Build the upgrade request.
///
/// The credential is a header, never a query parameter: a URL that grants
/// control of a browser must not reach a proxy access log. The instance id and
/// build details ride alongside so one machine can be told from another and the
/// account page can name the device that holds the slot.
fn bridge_request(bearer: &str) -> Result<WsRequest, BridgeError> {
  let mut request = bridge_url()
    .into_client_request()
    .map_err(|e| BridgeError::Unreachable(format!("invalid bridge endpoint: {e}")))?;

  let header = |value: &str| {
    value.parse().map_err(|_| {
      BridgeError::Unauthorized("a bridge header value is not valid ASCII".to_string())
    })
  };

  let headers = request.headers_mut();
  headers.insert(
    tokio_tungstenite::tungstenite::http::header::AUTHORIZATION,
    header(&format!("Bearer {bearer}"))?,
  );
  headers.insert("x-donut-instance", header(&instance_id())?);
  headers.insert("x-donut-protocol", header(BRIDGE_PROTOCOL)?);
  headers.insert("x-donut-client-version", header(env!("CARGO_PKG_VERSION"))?);
  headers.insert("x-donut-platform", header(std::env::consts::OS)?);
  Ok(request)
}

/// Dial the bridge endpoint with the stored access token.
///
/// Deliberately does NOT refresh on refusal. The credential is judged AFTER the
/// handshake, so a bad one never reaches this function as an error. It arrives
/// later as a close frame, and `run` is the only place that can see it. A
/// refresh here would be dead code that reads like a safety net.
///
/// Returns the token alongside the socket so `run` can later tell whether a
/// refusal was for THIS token or for one that has since been replaced.
async fn connect() -> Result<(BridgeStream, String), BridgeError> {
  let token = crate::remote_session::access_token_for_cdp()
    .map_err(|e| BridgeError::Unauthorized(e.to_string()))?;
  let stream = dial(&token).await?;
  Ok((stream, token))
}

/// How long the upgrade may take before it counts as unreachable.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

async fn dial(bearer: &str) -> Result<BridgeStream, BridgeError> {
  let request = bridge_request(bearer)?;
  let connect = tokio_tungstenite::connect_async(request);
  match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
    Err(_) => Err(BridgeError::Unreachable(format!(
      "the bridge did not answer within {}s",
      CONNECT_TIMEOUT.as_secs()
    ))),
    Ok(Ok((stream, _response))) => Ok(stream),
    Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => {
      Err(classify_status(response.status().as_u16()))
    }
    Ok(Err(e)) => Err(BridgeError::Unreachable(e.to_string())),
  }
}

/// Turn a refused upgrade into the reason it means.
///
/// The three refusals a user can act on are told apart on purpose: sign in
/// again, close the other copy of Donut, or upgrade the plan. Collapsing them
/// into "connection failed" is what makes a feature look broken when it is
/// merely saying no.
fn classify_status(status: u16) -> BridgeError {
  match status {
    401 => BridgeError::Unauthorized(format!("the bridge refused the credential (HTTP {status})")),
    402 | 403 => BridgeError::NotEntitled(
      "this plan does not include remote control of the desktop app".to_string(),
    ),
    409 => BridgeError::SlotTaken(
      "another Donut instance on this account already holds the remote-control slot".to_string(),
    ),
    other => BridgeError::Unreachable(format!("the bridge answered HTTP {other}")),
  }
}

/// Map a close frame onto the same vocabulary as a refused handshake.
///
/// A 409 cannot answer a socket that has already been upgraded, so a slot taken
/// *after* connect arrives as a 1008 with a reason. Reading only the code would
/// report a plan problem and a slot conflict identically.
fn classify_close(code: u16, reason: &str) -> BridgeError {
  let lowered = reason.to_ascii_lowercase();
  match code {
    // Matched on "slot", not on the bare word "instance": the malformed-header
    // refusal is "an instance id is required", which shares that noun while
    // meaning something completely different. Reading it as a conflict told the
    // customer to close a second Donut that was not running, and never showed
    // the real cause.
    1008 if lowered.contains("slot") => BridgeError::SlotTaken(reason.to_string()),
    1008 if lowered.contains("entitle") || lowered.contains("plan") => {
      BridgeError::NotEntitled(reason.to_string())
    }
    1008 => BridgeError::Unauthorized(if reason.is_empty() {
      "the bridge revoked this connection".to_string()
    } else {
      reason.to_string()
    }),
    // Evicted because another socket took the slot. This arrives as a normal
    // close with a replacement reason, because being replaced by yourself after
    // a reconnect is not an error, but two installs that share a copied data
    // directory carry the SAME instance id, so each looks like the other
    // reconnecting and they evict each other. Read as `Unreachable` this
    // retried at once and never settled: a permanent one-per-second flip in
    // which whichever machine won the last flip served the account's tool
    // calls. It is a slot conflict whatever code carries it, and naming it one
    // puts it in the terminal backoff band the comment on
    // `is_terminal_refusal` already promises.
    1000 if lowered.contains("replaced") || lowered.contains("newer connection") => {
      BridgeError::SlotTaken(reason.to_string())
    }
    _ if reason.is_empty() => BridgeError::Unreachable(format!("the bridge closed ({code})")),
    _ => BridgeError::Unreachable(format!("the bridge closed ({code}: {reason})")),
  }
}

/// Carry frames until the socket dies.
///
/// Reads and writes are split so a long tool call never blocks the pong that
/// keeps the connection alive: work is spawned, and its answer is pushed onto a
/// channel that a dedicated writer drains.
async fn pump(stream: BridgeStream, greeted: Arc<AtomicBool>) -> Result<(), BridgeError> {
  let (mut sink, mut source) = stream.split();
  let (tx, mut rx) = mpsc::channel::<Message>(MAX_IN_FLIGHT * 2 + 8);
  let permits = Arc::clone(&BRIDGE_PERMITS);

  let writer = tokio::spawn(async move {
    while let Some(message) = rx.recv().await {
      if sink.send(message).await.is_err() {
        break;
      }
    }
    let _ = sink.close().await;
  });

  let result = loop {
    let next = match tokio::time::timeout(IDLE_TIMEOUT, source.next()).await {
      Err(_) => {
        break Err(BridgeError::Unreachable(format!(
          "no frame from the bridge in {}s",
          IDLE_TIMEOUT.as_secs()
        )))
      }
      Ok(None) => break Ok(()),
      Ok(Some(Err(e))) => break Err(BridgeError::Unreachable(e.to_string())),
      Ok(Some(Ok(message))) => message,
    };

    match next {
      Message::Text(text) => {
        if let Some(frame) = parse_frame(&text) {
          dispatch(frame, &tx, &permits, &greeted);
        }
      }
      Message::Ping(payload) => {
        // Answered through OUR writer, rather than relying on tungstenite's
        // automatic pong.
        //
        // The library does currently put its own pong on the wire, so this is
        // belt and braces and a test cannot tell the two apart, deleting this
        // line leaves `the_desktop_answers_the_relays_keepalive_ping` green,
        // because that test asserts the property that actually matters (a pong
        // reaches the peer) rather than which code path produced it. It stays
        // because the alternative is depending on when a third-party library
        // chooses to flush, and the cost of being wrong about that is every
        // desktop being dropped upstream at the idle timeout.
        if tx.send(Message::Pong(payload)).await.is_err() {
          break Ok(());
        }
      }
      Message::Close(frame) => {
        break match frame {
          Some(frame) => Err(classify_close(u16::from(frame.code), &frame.reason)),
          None => Ok(()),
        }
      }
      // Binary and pong frames carry nothing this protocol defines.
      _ => {}
    }
  };

  // Dropping this sender does NOT close the channel: `dispatch` clones it into
  // every spawned tool call, and the writer only stops when the LAST sender is
  // gone. So the join below is bounded and then abandoned.
  //
  // Without the bound, a single stuck tool call outlives the socket and pins
  // this function forever: the idle timeout fires, `result` is already decided,
  // and `run` still never reaches its reconnect sleep, so `BRIDGE_CONNECTED`
  // stays true, the Integrations page reports a healthy bridge, and remote
  // control is dead until the app restarts. That defeats the exact failure the
  // idle timeout exists to catch.
  //
  // The in-flight calls themselves are deliberately left running rather than
  // aborted. Their answers have nowhere to go, but half of them are launching
  // browsers, and cancelling one mid-launch leaves a profile in a worse state
  // than letting it finish into a closed channel.
  drop(tx);
  drain_writer(writer, WRITER_DRAIN_TIMEOUT).await;
  result
}

/// Wait for the writer to finish, then give up on it.
///
/// Split out so the bound can be tested at a cadence a test can wait for. The
/// bound is the whole point: without it a caller can wait forever, and the
/// only thing that would notice is the customer.
async fn drain_writer(writer: tokio::task::JoinHandle<()>, budget: Duration) {
  let mut writer = writer;
  if tokio::time::timeout(budget, &mut writer).await.is_err() {
    log::warn!(
      "[mcp-remote] A relayed call outlived its socket; abandoning the writer after {}s",
      budget.as_secs()
    );
    writer.abort();
  }
}

/// One decoded server frame.
enum BridgeFrame {
  Hello {
    protocol: String,
  },
  Rpc {
    cid: String,
    session_id: Option<String>,
    payload: Vec<u8>,
  },
  EndSession {
    session_id: String,
  },
}

fn parse_frame(text: &str) -> Option<BridgeFrame> {
  let value: serde_json::Value = serde_json::from_str(text).ok()?;
  match value.get("t").and_then(serde_json::Value::as_str)? {
    "hello" => Some(BridgeFrame::Hello {
      protocol: value
        .get("protocol")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string(),
    }),
    "rpc" => {
      // Non-empty, not merely present. A relayed call is correlated ONLY by
      // `cid`, so an empty one is as unanswerable as a missing one: the reply
      // would match nothing at the relay and the caller would wait out its full
      // timeout rather than learn anything.
      let cid = value
        .get("cid")
        .and_then(serde_json::Value::as_str)
        .filter(|cid| !cid.is_empty())?;
      let payload = value.get("payload")?;
      Some(BridgeFrame::Rpc {
        cid: cid.to_string(),
        session_id: value
          .get("sessionId")
          .and_then(serde_json::Value::as_str)
          .map(str::to_string),
        payload: serde_json::to_vec(payload).ok()?,
      })
    }
    "endSession" => Some(BridgeFrame::EndSession {
      // Likewise non-empty: forgetting the session named "" is a no-op the
      // relay would read as a successful teardown.
      session_id: value
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())?
        .to_string(),
    }),
    other => {
      // Forward compatibility: a newer relay may add frames this build has
      // never heard of, and dropping one must not take the socket down.
      log::debug!("[mcp-remote] Ignoring unknown bridge frame '{other}'");
      None
    }
  }
}

fn dispatch(
  frame: BridgeFrame,
  tx: &mpsc::Sender<Message>,
  permits: &Arc<Semaphore>,
  greeted: &AtomicBool,
) {
  match frame {
    BridgeFrame::Hello { protocol } => {
      // The relay sends this only once it has accepted the credential, which
      // is what makes it the signal `run` uses to allow another refresh and to
      // tell the screen the bridge is actually up.
      greeted.store(true, Ordering::SeqCst);
      log::info!("[mcp-remote] Bridge accepted; the relay greeted us");
      publish_state(true, None);
      if protocol != BRIDGE_PROTOCOL {
        log::warn!(
          "[mcp-remote] The bridge speaks '{protocol}' and this build speaks '{BRIDGE_PROTOCOL}'"
        );
      }
    }
    BridgeFrame::EndSession { session_id } => {
      tokio::spawn(async move {
        McpServer::instance().end_session(&session_id).await;
      });
    }
    BridgeFrame::Rpc {
      cid,
      session_id,
      payload,
    } => {
      let Ok(permit) = Arc::clone(permits).try_acquire_owned() else {
        log::warn!("[mcp-remote] Refused a relayed call: {MAX_IN_FLIGHT} already in flight");
        let _ = tx.try_send(encode(&serde_json::json!({
          "t": "result",
          "cid": cid,
          "status": "busy",
        })));
        return;
      };

      let tx = tx.clone();
      tokio::spawn(async move {
        let outcome = McpServer::instance()
          .handle_message(
            crate::mcp_server::McpOrigin::Bridge,
            session_id.as_deref(),
            &payload,
          )
          .await;
        drop(permit);
        let _ = tx.send(encode(&result_frame(&cid, outcome))).await;
      });
    }
  }
}

fn encode(value: &serde_json::Value) -> Message {
  Message::Text(value.to_string().into())
}

fn result_frame(cid: &str, outcome: McpOutcome) -> serde_json::Value {
  match outcome {
    McpOutcome::Body {
      body,
      new_session_id,
    } => {
      let encoded = body.to_string();
      if encoded.len() > MAX_RESULT_BYTES {
        log::warn!(
          "[mcp-remote] Result of {} bytes exceeds the {MAX_RESULT_BYTES}-byte frame budget",
          encoded.len()
        );
        return serde_json::json!({
          "t": "result",
          "cid": cid,
          "status": "tooLarge",
        });
      }
      serde_json::json!({
        "t": "result",
        "cid": cid,
        "status": "ok",
        "sessionId": new_session_id,
        "payload": body,
      })
    }
    McpOutcome::Accepted => serde_json::json!({
      "t": "result", "cid": cid, "status": "accepted",
    }),
    McpOutcome::UnknownSession => serde_json::json!({
      "t": "result", "cid": cid, "status": "unknownSession",
    }),
    McpOutcome::BadRequest => serde_json::json!({
      "t": "result", "cid": cid, "status": "badRequest",
    }),
    McpOutcome::RateLimited { retry_after_secs } => serde_json::json!({
      "t": "result", "cid": cid, "status": "rateLimited", "retryAfter": retry_after_secs,
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn bridge_url_is_the_websocket_scheme_of_the_cloud_api() {
    assert_eq!(bridge_url(), "wss://api.donutbrowser.com/api/mcp-bridge");
  }

  #[test]
  fn credentials_never_reach_the_url() {
    let request = bridge_request("secret-token").expect("request");
    assert!(!request.uri().to_string().contains("secret-token"));
    assert_eq!(
      request
        .headers()
        .get(tokio_tungstenite::tungstenite::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok()),
      Some("Bearer secret-token")
    );
  }

  #[test]
  fn handshake_statuses_map_onto_actionable_reasons() {
    assert!(matches!(classify_status(401), BridgeError::Unauthorized(_)));
    assert!(matches!(classify_status(402), BridgeError::NotEntitled(_)));
    assert!(matches!(classify_status(403), BridgeError::NotEntitled(_)));
    assert!(matches!(classify_status(409), BridgeError::SlotTaken(_)));
    assert!(matches!(classify_status(500), BridgeError::Unreachable(_)));
  }

  #[test]
  fn a_slot_conflict_after_connect_is_not_read_as_a_credential_problem() {
    assert!(matches!(
      classify_close(1008, "another instance already holds the slot"),
      BridgeError::SlotTaken(_)
    ));
    assert!(matches!(
      classify_close(1008, "remote control is not included in this plan"),
      BridgeError::NotEntitled(_)
    ));
    assert!(matches!(
      classify_close(1008, "credential revoked"),
      BridgeError::Unauthorized(_)
    ));
    assert!(matches!(
      classify_close(1011, ""),
      BridgeError::Unreachable(_)
    ));
  }

  #[tokio::test]
  async fn the_screen_says_connected_only_after_the_relay_greets_us() {
    // The upgrade completes BEFORE the credential is judged, so a completed
    // handshake is a question, not a connection. Announcing "Connected as" on
    // it made a refused customer watch the tab flash connected and fail once a
    // minute for ever, which reads as a flapping network rather than as the
    // refusal it is.
    // Observed through `greeted`, which is per-dial state owned by this test,
    // NOT through BRIDGE_CONNECTED. That global is shared with every sibling
    // test on cargo's parallel threads, and asserting on it would couple
    // this test to whatever they happened to publish. `greeted` is the same
    // flag `run` itself reads to decide whether the credential was accepted,
    // and the source assertion at the end pins the announcement to it.
    let greeted = Arc::new(AtomicBool::new(false));

    // Refused right after the upgrade: nothing to announce.
    let (url, server) = fake_relay(vec![], 0).await;
    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::clone(&greeted)).await;
    let _ = server.await;
    assert!(
      !greeted.load(Ordering::SeqCst),
      "a bare handshake must not announce a connection"
    );

    // A frame that is NOT a hello must not announce either: the relay sends
    // rpc and endSession over a socket it has already accepted, but reading
    // any of them as acceptance would put the "only hello" rule back to
    // "anything at all", which is what it replaced.
    let (url, server) = fake_relay(
      vec![
        serde_json::json!({
          "t": "rpc",
          "cid": "cid-x",
          "sessionId": serde_json::Value::Null,
          "payload": { "jsonrpc": "2.0", "id": 1, "method": "ping" },
        }),
        serde_json::json!({ "t": "endSession", "sessionId": "some-session" }),
      ],
      1,
    )
    .await;
    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::clone(&greeted)).await;
    let _ = server.await;
    assert!(
      !greeted.load(Ordering::SeqCst),
      "only the relay's hello may announce a connection"
    );

    // Greeted: now the screen may say connected.
    let (url, server) = fake_relay(
      vec![serde_json::json!({ "t": "hello", "protocol": BRIDGE_PROTOCOL })],
      0,
    )
    .await;
    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::clone(&greeted)).await;
    let _ = server.await;
    assert!(
      greeted.load(Ordering::SeqCst),
      "the relay's hello must announce the connection"
    );

    // And the announcement is wired to exactly that branch. Without this, the
    // behavioural half above would still pass if `publish_state(true, ..)` were
    // moved somewhere reached before the greeting, which is the bug the whole
    // test exists to prevent.
    let source = include_str!("mcp_remote.rs");
    let production = source
      .split_once("\n#[cfg(test)]")
      .map_or(source, |(code, _)| code);
    let announcements: Vec<&str> = production
      .match_indices("publish_state(true")
      .map(|(at, _)| {
        let start = production[..at].rfind("\nfn ").unwrap_or(0);
        production[start..].lines().nth(1).unwrap_or("").trim()
      })
      .collect();
    assert_eq!(
      announcements.len(),
      1,
      "exactly one place may announce a connection; found {announcements:?}"
    );
    let hello = production
      .split("BridgeFrame::Hello { protocol } => {")
      .nth(1)
      .expect("the hello branch must exist");
    assert!(
      hello[..hello.find("\n    }").unwrap_or(hello.len())].contains("publish_state(true, None)"),
      "the one announcement must sit in the hello branch"
    );
  }

  #[test]
  fn a_teardown_with_no_app_handle_still_corrects_the_screen() {
    // Both credential teardowns, `logout` and the automatic
    // `invalidate_session`, call `stop(None)`, because neither holds an
    // AppHandle. Emitting the new state only when one was supplied left the
    // Integrations page reading "Connected as <instance-id>" over a bridge
    // that had already been hung up.
    let source = include_str!("mcp_remote.rs");
    let publish = source
      .split("fn publish_state(")
      .nth(1)
      .expect("publish_state must exist");
    // Bounded by the function's own end, not by a character count: the window
    // used to be 700 characters and a later comment pushed the emit past it,
    // so the assertion stopped reading the code it was written to guard.
    let body = &publish[..publish.find("\n}").unwrap_or(publish.len())];
    assert!(
      body.contains("crate::events::emit(EVENT_MCP_REMOTE_STATUS"),
      "the state must be published through the handle-free global emitter"
    );

    let stop = source
      .split("pub fn stop(app: Option<&AppHandle>) {")
      .nth(1)
      .expect("stop must exist");
    let stop = &stop[..stop.len().min(700)];
    assert!(
      stop.contains("publish_state(false, None);"),
      "stop must publish, not only notify a handle it may not have been given"
    );
  }

  #[test]
  fn a_failed_refresh_does_not_spend_the_one_shot() {
    // `refreshed = true` before the attempt meant a refresh that FAILED, a
    // blip at the wrong moment, permanently disarmed the retry, because the
    // reset needs the relay's `hello` and it will never greet a credential it
    // keeps refusing. The bridge then sat on a dead token until restart.
    let source = include_str!("mcp_remote.rs");
    let block = source
      .split("if should_refresh_credential(error, refreshed) {")
      .nth(1)
      .expect("the refresh block must exist");
    let block = &block[..block.len().min(900)];

    let ok_arm = block.find("Ok(()) =>").expect("an Ok arm");
    let spend = block
      .find("refreshed = true;")
      .expect("the one-shot must be spent somewhere");
    assert!(
      spend > ok_arm,
      "the one-shot may only be spent AFTER a refresh succeeds, not before the attempt"
    );
    let err_arm = block.find("Err(e) =>").expect("an Err arm");
    assert!(spend < err_arm, "the spend belongs inside the success arm");
  }

  #[test]
  fn backoff_resets_only_after_a_connection_that_actually_held() {
    // The `hello` arrives as soon as the socket is registered, so a connection
    // that lived milliseconds greeted exactly like one that lived a day.
    // Resetting `attempt` on the greeting alone turned any non-terminal close,
    // an idle 1011 for instance, into a 1 Hz retry loop that never backed off.
    assert!(
      MIN_UPTIME_FOR_BACKOFF_RESET >= Duration::from_secs(30),
      "a threshold this short would not outlast a greet-then-close cycle"
    );
    assert!(
      MIN_UPTIME_FOR_BACKOFF_RESET < IDLE_TIMEOUT,
      "must be under the idle reap, or a healthy but quiet bridge never resets"
    );

    let source = include_str!("mcp_remote.rs");
    let block = source
      .split("if greeted.load(Ordering::SeqCst) {")
      .nth(1)
      .expect("the greeting block must exist");
    let block = &block[..block.len().min(900)];
    assert!(
      block.contains("if lasted >= MIN_UPTIME_FOR_BACKOFF_RESET {"),
      "the backoff reset must be gated on real uptime, not on the greeting alone"
    );
    let reset = block.find("attempt = 0;").expect("the reset must exist");
    let gate = block
      .find("if lasted >= MIN_UPTIME_FOR_BACKOFF_RESET {")
      .expect("the gate must exist");
    assert!(reset > gate, "the reset must sit inside the uptime gate");
  }

  #[tokio::test]
  async fn only_a_greeting_re_arms_the_credential_refresh() {
    // The upgrade completes BEFORE the credential is judged, so a completed
    // handshake is no evidence it was accepted, `hello` is the first frame that
    // follows that decision. Resetting the one-refresh bound on the handshake
    // made the bound inert for the only path that produces a credential refusal
    // (a close frame), so a desktop that keeps being refused rotated its
    // refresh token on every cycle instead of exactly once.
    let greeted = Arc::new(AtomicBool::new(false));

    // Refused before any greeting: the flag stays clear, so `run` keeps
    // `refreshed` set and does not refresh a second time.
    let (url, server) = fake_relay(vec![], 0).await;
    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::clone(&greeted)).await;
    let _ = server.await;
    assert!(
      !greeted.load(Ordering::SeqCst),
      "a bare handshake must not count as the credential being accepted"
    );

    // Greeted: the credential was accepted, so a later refusal may refresh.
    let (url, server) = fake_relay(
      vec![serde_json::json!({ "t": "hello", "protocol": BRIDGE_PROTOCOL })],
      0,
    )
    .await;
    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::clone(&greeted)).await;
    let _ = server.await;
    assert!(
      greeted.load(Ordering::SeqCst),
      "the relay's hello is what proves the credential was accepted"
    );
  }

  #[test]
  fn nothing_announces_a_connection_from_a_task_of_its_own() {
    // The connection used to be announced by a task parked on a `Notify`, and
    // that shape was wrong in two independent ways.
    //
    // `stop` could not reach it. BRIDGE_TASK holds only the reconnect loop's
    // handle, and dropping a JoinHandle DETACHES rather than aborts, so a stop
    // between dialling and greeting left the announcer waiting on a signal
    // whose only other holder had just been dropped, it could never be woken
    // and never finished. One leaked task per stop-during-connect, for the life
    // of the process.
    //
    // And `abort` could not cancel it once it mattered. The task's only await
    // was the wait for the greeting, so once that fired it ran straight through
    // to publishing; on a greet-then-close socket the announcement could land
    // after the teardown, leaving the screen reading connected with no error
    // and nothing to correct it until the next dial, up to a minute later in
    // the terminal backoff band.
    //
    // Publishing on the read loop's own task removes both, so the property to
    // hold is that no announcer is ever spawned again.
    let source = include_str!("mcp_remote.rs");
    let production = source
      .split_once("\n#[cfg(test)]")
      .map_or(source, |(code, _)| code);

    assert!(
      !production.contains("notified().await"),
      "the connection must not be announced from a task waiting on a signal: \
       stop cannot reach such a task and abort cannot cancel it once the \
       signal has fired"
    );
    assert!(
      !production.contains("Notify::new()"),
      "the greeting signal is gone; a new one would reintroduce the detached \
       announcer it existed to feed"
    );

    // The residual window is closed by an invariant rather than by ordering:
    // cancellation only takes effect at an await point, so the read loop can be
    // part-way through a frame when a stop lands. Asserted from the source
    // because BRIDGE_RUNNING and BRIDGE_CONNECTED are process-wide and cargo
    // runs these tests on parallel threads, a test that drove them would race
    // every sibling that publishes state.
    let publish = production
      .split("fn publish_state(")
      .nth(1)
      .expect("publish_state must exist");
    let body = &publish[..publish.find("\n}").unwrap_or(publish.len())];
    assert!(
      body.contains("connected && BRIDGE_RUNNING.load(Ordering::SeqCst)"),
      "a bridge that is not running must never be published as connected"
    );
  }

  #[test]
  fn the_in_flight_budget_outlives_the_socket_that_spent_it() {
    // A socket teardown deliberately abandons in-flight calls rather than
    // aborting them, half of them are launching browsers. Those tasks keep
    // their permits, so a per-socket semaphore handed each reconnect a fresh
    // full budget and the cap stopped bounding anything across a flapping
    // connection. The budget therefore belongs to the bridge, not the socket.
    let source = include_str!("mcp_remote.rs");
    let pump = source
      .split("async fn pump(")
      .nth(1)
      .expect("pump must exist");
    let body = &pump[..pump.len().min(1200)];
    assert!(
      !body.contains("Semaphore::new("),
      "pump must not mint its own budget; abandoned calls would keep permits \
       from a semaphore nobody is counting any more"
    );
    assert!(
      body.contains("Arc::clone(&BRIDGE_PERMITS)"),
      "pump must share the bridge-lifetime budget"
    );

    // And nowhere else mints one either. Scanning the whole of the production
    // half is the part that generalises: the check above only reads pump's
    // first 1200 characters, so a budget re-minted in any other helper on the
    // relayed-call path would pass it while re-creating the exact bug.
    //
    // This replaced a runtime check that counted permits on BRIDGE_PERMITS,
    // which was wrong twice over. It was VACUOUS, `first` and `second` were
    // two clones of one static, so comparing their counts compared an object
    // with itself and held whatever the code did. And it was RACY: a dozen
    // tests drive pump, whose relayed calls take global permits on spawned
    // tasks that outlive the test that started them, so a sibling holding one
    // made `assert_eq!(available, MAX_IN_FLIGHT)` fail intermittently. It went
    // green six runs in a row before failing, which is precisely why a count
    // over shared mutable state is not evidence of anything here.
    let production = source
      .split_once("\n#[cfg(test)]")
      .map_or(source, |(before, _)| before);
    // Each mint is reported with the declaration it belongs to, so a failure
    // names the offending site instead of just a count.
    let mints: Vec<&str> = production
      .match_indices("Semaphore::new(")
      .map(|(at, _)| {
        let stmt = production[..at].rfind(';').map_or(0, |n| n + 1);
        production[stmt..at].trim()
      })
      .collect();
    assert_eq!(
      mints.len(),
      1,
      "the bridge budget must be minted exactly once for the process; \
       minted by: {mints:?}"
    );
    assert!(
      mints[0].contains("BRIDGE_PERMITS"),
      "the one Semaphore must be the BRIDGE_PERMITS static, not {:?}",
      mints[0]
    );
  }

  #[test]
  fn every_reason_the_relay_can_close_with_is_classified_correctly() {
    // These are the literal close reasons the bridge endpoint sends.
    // Classifying them is not cosmetic: each one produces a different sentence
    // on the Integrations page telling the customer what to do about it.
    for (reason, expect_slot, expect_plan) in [
      (
        "another Donut instance on this account holds the remote-control slot",
        true,
        false,
      ),
      ("remote control is not included in this plan", false, true),
      ("this plan no longer includes remote control", false, true),
    ] {
      let error = classify_close(1008, reason);
      assert_eq!(
        matches!(error, BridgeError::SlotTaken(_)),
        expect_slot,
        "slot classification wrong for {reason:?}"
      );
      assert_eq!(
        matches!(error, BridgeError::NotEntitled(_)),
        expect_plan,
        "plan classification wrong for {reason:?}"
      );
    }

    // The malformed-header refusal shares the word "instance" with the slot
    // conflict and means something entirely different. Read as a conflict it
    // told the customer to close a second Donut that was not running.
    assert!(
      !matches!(
        classify_close(1008, "an instance id is required"),
        BridgeError::SlotTaken(_)
      ),
      "a malformed instance id is not a second copy of Donut"
    );

    for reason in [
      "unauthorized",
      "credential no longer valid",
      "credential revoked",
    ] {
      assert!(
        matches!(classify_close(1008, reason), BridgeError::Unauthorized(_)),
        "{reason:?} should read as a credential problem"
      );
    }
  }

  #[test]
  fn a_malformed_persisted_instance_id_is_not_sent_to_be_rejected() {
    // An instance id outside the accepted shape is refused at the upgrade.
    // Sending a value that cannot pass wastes a dial and produces a refusal the
    // desktop can only guess at.
    assert!(is_valid_instance_id(&uuid::Uuid::new_v4().to_string()));
    assert!(is_valid_instance_id("abcd1234"));
    assert!(is_valid_instance_id(&"a".repeat(64)));

    assert!(!is_valid_instance_id(""), "empty");
    assert!(!is_valid_instance_id("short7"), "under 8 characters");
    assert!(!is_valid_instance_id(&"a".repeat(65)), "over 64 characters");
    assert!(!is_valid_instance_id("has spaces here"), "space");
    assert!(!is_valid_instance_id("has_underscore1"), "underscore");
    assert!(!is_valid_instance_id("café-instance"), "non-ascii");
    assert!(!is_valid_instance_id("newline\n1234"), "control character");
  }

  #[test]
  fn a_malformed_id_on_disk_is_replaced_rather_than_reused() {
    // Exercises the real read-and-decide path, not just the shape helper: an
    // earlier version of this test asserted only `is_valid_instance_id`, so
    // deleting its call site from the reader changed nothing and the test
    // still passed.
    let dir = std::env::temp_dir().join(format!("donut-iid-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("mcp_instance_id");

    // A good id is kept exactly as written.
    let good = uuid::Uuid::new_v4().to_string();
    std::fs::write(&path, &good).expect("write");
    assert_eq!(read_or_create_instance_id(&path), good);

    // A malformed one is replaced, persisted, and stable from then on.
    std::fs::write(&path, "not a valid id!").expect("write");
    let replaced = read_or_create_instance_id(&path);
    assert!(is_valid_instance_id(&replaced), "{replaced:?}");
    assert_eq!(
      std::fs::read_to_string(&path).expect("read").trim(),
      replaced,
      "the replacement must be written back, or every launch regenerates"
    );
    assert_eq!(read_or_create_instance_id(&path), replaced);

    // A missing file mints one.
    std::fs::remove_file(&path).expect("remove");
    assert!(is_valid_instance_id(&read_or_create_instance_id(&path)));

    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn being_evicted_from_the_slot_backs_off_instead_of_racing() {
    // A superseded socket is retired with 1000 and this exact reason. Two
    // installs sharing a copied data directory carry the same instance id,
    // so each reads as the other reconnecting and they evict each other; read
    // as a plain close this reconnected immediately and flapped forever.
    let evicted = classify_close(1000, "replaced by a newer connection");
    assert!(matches!(evicted, BridgeError::SlotTaken(_)));
    assert!(
      evicted.is_terminal_refusal(),
      "an evicted desktop must land in the terminal backoff band, not retry at once"
    );

    // A normal close still is one; only the replacement reason is special.
    assert!(matches!(
      classify_close(1000, ""),
      BridgeError::Unreachable(_)
    ));
    assert!(matches!(
      classify_close(1000, "going away"),
      BridgeError::Unreachable(_)
    ));
  }

  #[test]
  fn the_status_carries_a_translatable_code_never_server_authored_prose() {
    // The Integrations page ships in ten languages, and these reasons are
    // English, some of them written by the relay and echoed in a close frame.
    // Rendering one verbatim puts an English sentence under a Japanese UI, so
    // the status carries a code and the words are chosen in the locale files.
    // Every code here has a `backendErrors.*` key in all ten of them and a
    // `case` in src/lib/backend-errors.ts.
    let cases = [
      (
        BridgeError::Unauthorized("the bridge refused the credential".into()),
        "MCP_REMOTE_UNAUTHORIZED",
      ),
      (
        BridgeError::SlotTaken("another Donut instance holds the slot".into()),
        "MCP_REMOTE_SLOT_TAKEN",
      ),
      (
        BridgeError::NotEntitled("this plan does not include it".into()),
        "MCP_REMOTE_NOT_ENTITLED",
      ),
      (
        BridgeError::Unreachable("dns failure".into()),
        "MCP_REMOTE_UNREACHABLE",
      ),
    ];

    let switch = std::fs::read_to_string("../src/lib/backend-errors.ts")
      .expect("the frontend error translator must exist");
    for (error, code) in cases {
      assert_eq!(error.code(), code);
      let envelope = crate::backend_error(error.code());
      // Exactly the shape `translateBackendError` parses.
      let parsed: serde_json::Value =
        serde_json::from_str(&envelope).expect("the status must be a JSON envelope");
      assert_eq!(parsed["code"], code);
      // And the prose is nowhere near it.
      assert!(!envelope.contains("bridge refused"));
      assert!(
        switch.contains(&format!("case \"{code}\":")),
        "{code} has no case in translateBackendError, so it would render raw"
      );
    }
  }

  #[test]
  fn an_expired_token_is_refreshed_and_retried_at_once() {
    // The one "unauthorized" that fixes itself. An access token eventually
    // expires, so every long-running desktop hits this once per token: the
    // socket is closed with 1008, and without this the desktop drops into the
    // terminal backoff band redialling the SAME dead token, showing "sign out
    // and sign in again" for a condition that needed neither.
    //
    // This lives here rather than at the dial because the credential is judged
    // AFTER the handshake, so a refused one is never an HTTP 401 the dial could
    // see.
    assert!(should_refresh_credential(
      &BridgeError::Unauthorized("credential no longer valid".into()),
      false
    ));

    // Once per credential. A token the server genuinely rejects must not send
    // the desktop round a refresh loop.
    assert!(!should_refresh_credential(
      &BridgeError::Unauthorized("credential revoked".into()),
      true
    ));

    // Nothing else is fixed by a new token, and retrying these fast is how a
    // background task becomes a battery bug.
    for error in [
      BridgeError::SlotTaken("another instance".into()),
      BridgeError::NotEntitled("not on this plan".into()),
      BridgeError::Unreachable("dns".into()),
    ] {
      assert!(
        !should_refresh_credential(&error, false),
        "{error:?} is not something a token refresh fixes"
      );
    }
  }

  #[test]
  fn a_refusal_of_a_token_that_was_already_replaced_redials_instead_of_refreshing() {
    // The decision itself.
    assert!(credential_rotated_since(Some("old"), Some("new")));
    assert!(!credential_rotated_since(Some("same"), Some("same")));
    // Nothing to compare: the dial failed before reading a credential, or the
    // account signed out meanwhile. Neither is a rotation.
    assert!(!credential_rotated_since(None, Some("new")));
    assert!(!credential_rotated_since(Some("old"), None));
    assert!(!credential_rotated_since(None, None));

    // And it sits AHEAD of the refresh in `run`, guarded on Unauthorized: a
    // slot conflict on a rotated token is still a slot conflict.
    let source = include_str!("mcp_remote.rs");
    let production = source
      .split_once("\n#[cfg(test)]")
      .map_or(source, |(code, _)| code);
    let run = production
      .split("async fn run() {")
      .nth(1)
      .expect("run must exist");
    let rotated = run
      .find("credential_rotated_since(")
      .expect("run must compare the refused token with the one on disk");
    let refresh = run
      .find("should_refresh_credential(error, refreshed)")
      .expect("run must still refresh");
    assert!(
      rotated < refresh,
      "the rotation check must come first, or the refresh is spent on a token that is already gone"
    );
    let guard = &run[..rotated];
    assert!(
      guard
        .rfind("matches!(error, BridgeError::Unauthorized(_))")
        .is_some_and(|at| at > guard.len().saturating_sub(200)),
      "the redial must be limited to an Unauthorized refusal"
    );
    // The token is remembered from the dial that produced the socket, and
    // forgotten when the dial fails before reading one.
    assert!(run.contains("(outcome.err(), Some(token))"));
    assert!(run.contains("(Some(e), None)"));
  }

  #[test]
  fn start_and_stop_change_the_flag_and_the_handle_under_one_lock() {
    // Flag first and handle second, with no lock across them, let a `stop`
    // between the two see the flag, find no handle, and return, after which
    // the start stored a handle for a loop nobody could stop any more.
    let source = include_str!("mcp_remote.rs");
    let production = source
      .split_once("\n#[cfg(test)]")
      .map_or(source, |(code, _)| code);
    for (name, header) in [
      ("start", "pub fn start(app: AppHandle) {"),
      ("stop", "pub fn stop(app: Option<&AppHandle>) {"),
    ] {
      let body = production
        .split(header)
        .nth(1)
        .unwrap_or_else(|| panic!("{name} must exist"));
      let body = &body[..body.find("\n}").unwrap_or(body.len())];
      let lock = body
        .find("BRIDGE_TASK\n    .lock()")
        .unwrap_or_else(|| panic!("{name} must take the task lock"));
      let flip = body
        .find("BRIDGE_RUNNING.swap(")
        .unwrap_or_else(|| panic!("{name} must flip the running flag"));
      assert!(
        lock < flip,
        "{name} must hold the task lock before it flips the running flag"
      );
      assert_eq!(
        body.matches("BRIDGE_TASK").count(),
        1,
        "{name} must touch the slot once, under the lock it already holds"
      );
    }
  }

  #[test]
  fn the_dial_no_longer_pretends_to_recover_a_credential_it_never_sees() {
    // Guards the reason the refresh moved. `connect` must not grow a
    // refresh-and-retry again: the upgrade completes before the credential is
    // judged, so `dial` cannot observe a rejected one, and a retry there would
    // be dead code that reads like a safety net.
    let source = std::fs::read_to_string("src/mcp_remote.rs").expect("mcp_remote.rs");
    let start = source
      .find("async fn connect()")
      .expect("connect must exist");
    let body = &source[start..start + 400];
    assert!(
      !body.contains("refresh_access_token"),
      "connect() must not refresh; the relay's refusals arrive as close frames, not dial errors"
    );
  }

  #[test]
  fn every_refusal_the_server_will_repeat_backs_off_slowly() {
    assert!(BridgeError::Unauthorized(String::new()).is_terminal_refusal());
    assert!(BridgeError::SlotTaken(String::new()).is_terminal_refusal());
    assert!(BridgeError::NotEntitled(String::new()).is_terminal_refusal());
    assert!(!BridgeError::Unreachable(String::new()).is_terminal_refusal());
  }

  #[test]
  fn rpc_frames_decode_with_and_without_a_session() {
    let with = parse_frame(
      r#"{"t":"rpc","cid":"c1","sessionId":"s1","payload":{"jsonrpc":"2.0","id":1,"method":"ping"}}"#,
    )
    .expect("frame");
    match with {
      BridgeFrame::Rpc {
        cid,
        session_id,
        payload,
      } => {
        assert_eq!(cid, "c1");
        assert_eq!(session_id.as_deref(), Some("s1"));
        let parsed: serde_json::Value = serde_json::from_slice(&payload).expect("payload");
        assert_eq!(parsed["method"], "ping");
      }
      _ => panic!("expected an rpc frame"),
    }

    let without =
      parse_frame(r#"{"t":"rpc","cid":"c2","payload":{"jsonrpc":"2.0","id":2,"method":"ping"}}"#)
        .expect("frame");
    match without {
      BridgeFrame::Rpc { session_id, .. } => assert!(session_id.is_none()),
      _ => panic!("expected an rpc frame"),
    }
  }

  #[test]
  fn a_frame_missing_the_field_that_makes_it_answerable_is_refused() {
    // A relayed call is correlated ONLY by `cid`. A frame without one cannot be
    // answered, the reply would carry an empty correlation id, the relay would
    // match it to nothing, and the caller would sit until its 90s timeout with
    // no idea why. Rejecting it here at least leaves the socket healthy.
    assert!(parse_frame(r#"{"t":"rpc","payload":{"jsonrpc":"2.0","id":1}}"#).is_none());
    assert!(parse_frame(r#"{"t":"rpc","cid":"","payload":{"jsonrpc":"2.0"}}"#).is_none());

    // Same for the one field `endSession` exists to carry. Accepting it empty
    // would make the desktop forget the session named "", which is every
    // session it does not have, a silent no-op the relay reads as success.
    assert!(parse_frame(r#"{"t":"endSession"}"#).is_none());
    // The empty case specifically: `?` already rejects a MISSING field, so
    // without this line the filter that rejects an empty one is unguarded.
    assert!(parse_frame(r#"{"t":"endSession","sessionId":""}"#).is_none());
    assert!(parse_frame(r#"{"t":"endSession","sessionId":"s1"}"#).is_some());
  }

  #[tokio::test]
  async fn a_relay_that_outruns_the_in_flight_cap_is_told_to_retry() {
    // The cap is what stops a bug on either side spawning unbounded work inside
    // a customer's app. Past it the desktop must ANSWER `busy`, a caller that
    // gets no frame at all waits out the call timeout upstream and then sees
    // the same refusal it could have had immediately.
    let (tx, mut rx) = mpsc::channel::<Message>(4);
    let permits = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let held: Vec<_> = (0..MAX_IN_FLIGHT)
      .map(|_| {
        Arc::clone(&permits)
          .try_acquire_owned()
          .expect("a free permit")
      })
      .collect();

    dispatch(
      BridgeFrame::Rpc {
        cid: "cid-over-cap".to_string(),
        session_id: None,
        payload: br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec(),
      },
      &tx,
      &permits,
      &AtomicBool::new(false),
    );

    let Message::Text(text) = rx.try_recv().expect("a refusal, not silence") else {
      panic!("expected a text frame");
    };
    let frame: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(frame["status"], "busy");
    assert_eq!(frame["cid"], "cid-over-cap");
    drop(held);
  }

  #[test]
  fn an_unknown_frame_is_ignored_rather_than_fatal() {
    assert!(parse_frame(r#"{"t":"somethingNewer","x":1}"#).is_none());
    assert!(parse_frame("not json").is_none());
    assert!(parse_frame(r#"{"t":"rpc","cid":"c1"}"#).is_none());
  }

  #[test]
  fn result_frames_carry_the_outcome_the_relay_has_to_render() {
    let ok = result_frame(
      "c1",
      McpOutcome::Body {
        body: serde_json::json!({"jsonrpc":"2.0","id":1,"result":{}}),
        new_session_id: Some("s9".to_string()),
      },
    );
    assert_eq!(ok["status"], "ok");
    assert_eq!(ok["sessionId"], "s9");
    assert_eq!(ok["payload"]["id"], 1);

    assert_eq!(
      result_frame("c2", McpOutcome::Accepted)["status"],
      "accepted"
    );
    assert_eq!(
      result_frame("c3", McpOutcome::UnknownSession)["status"],
      "unknownSession"
    );
    assert_eq!(
      result_frame("c4", McpOutcome::BadRequest)["status"],
      "badRequest"
    );

    let limited = result_frame(
      "c5",
      McpOutcome::RateLimited {
        retry_after_secs: 42,
      },
    );
    assert_eq!(limited["status"], "rateLimited");
    assert_eq!(limited["retryAfter"], 42);
  }

  #[test]
  fn an_oversized_result_is_refused_rather_than_sent() {
    let huge = "x".repeat(MAX_RESULT_BYTES + 1);
    let frame = result_frame(
      "c1",
      McpOutcome::Body {
        body: serde_json::json!({ "text": huge }),
        new_session_id: None,
      },
    );
    assert_eq!(frame["status"], "tooLarge");
  }

  #[test]
  fn the_instance_id_is_stable_within_a_process() {
    // Against a temp path, not `instance_id()`. That reads
    // `app_dirs::settings_dir()`, which `cargo test` does not redirect, so the
    // old version of this test MINTED AND WROTE an id into the developer's
    // real DonutBrowserDev data directory every time the suite ran, a test
    // reaching outside its sandbox to touch state the app itself owns.
    let dir = std::env::temp_dir().join(format!("donut-iid-stable-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("mcp_instance_id");

    let first = read_or_create_instance_id(&path);
    assert!(!first.is_empty());
    assert!(is_valid_instance_id(&first));
    // Stability is the property that matters: the slot is granted per instance
    // id, so one that changed between reads could never reclaim it.
    assert_eq!(read_or_create_instance_id(&path), first);
    assert_eq!(read_or_create_instance_id(&path), first);

    let _ = std::fs::remove_dir_all(&dir);
  }

  // ---------------------------------------------------------------------
  // The transport, end to end.
  //
  // Everything above tests one function against a literal. These drive the
  // REAL `pump` over a REAL WebSocket against a server that speaks the frames
  // the bridge endpoint actually sends, and assert the frames it actually
  // parses come back. That join is the part neither side's own tests can
  // cover: each was written against its own idea of the contract.
  //
  // The canonical frames below match the wire form observed from the endpoint:
  // a null `sessionId` stays on the wire rather than being omitted, so that is
  // what is sent here, a parser that only handled the absent case would pass a
  // friendlier fixture and fail in production.
  // ---------------------------------------------------------------------

  /// A server that speaks the relay's half of `donut-mcp-bridge/1`.
  ///
  /// Returns the URL to dial and a handle yielding every `result` frame the
  /// desktop sent back, in order.
  async fn fake_relay(
    script: Vec<serde_json::Value>,
    expected_results: usize,
  ) -> (String, tokio::task::JoinHandle<Vec<serde_json::Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
      .await
      .expect("the fake relay must bind");
    let port = listener.local_addr().expect("a bound port").port();

    let handle = tokio::spawn(async move {
      let mut results = Vec::new();
      let Ok((socket, _)) = listener.accept().await else {
        return results;
      };
      let Ok(mut stream) = tokio_tungstenite::accept_async(socket).await else {
        return results;
      };

      for frame in script {
        if stream
          .send(Message::Text(frame.to_string().into()))
          .await
          .is_err()
        {
          return results;
        }
      }

      while results.len() < expected_results {
        match stream.next().await {
          Some(Ok(Message::Text(text))) => {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
              results.push(value);
            }
          }
          Some(Ok(_)) => continue,
          _ => break,
        }
      }

      let _ = stream.close(None).await;
      results
    });

    (format!("ws://127.0.0.1:{port}"), handle)
  }

  async fn dial_fake(url: &str) -> BridgeStream {
    let (stream, _) = tokio_tungstenite::connect_async(url)
      .await
      .expect("the desktop must reach the fake relay");
    stream
  }

  #[tokio::test]
  async fn a_relayed_tools_list_reaches_the_engine_and_the_answer_comes_back() {
    McpServer::instance().mark_engine_ready_for_tests();

    let (url, server) = fake_relay(
      vec![
        serde_json::json!({
          "t": "hello",
          "protocol": BRIDGE_PROTOCOL,
          "instanceId": "instance-under-test",
        }),
        // Exactly what the relay puts on the wire, null sessionId and all.
        serde_json::json!({
          "t": "rpc",
          "cid": "cid-1",
          "sessionId": serde_json::Value::Null,
          "payload": { "jsonrpc": "2.0", "id": 7, "method": "tools/list" },
        }),
      ],
      1,
    )
    .await;

    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::new(AtomicBool::new(false))).await;
    let results = server.await.expect("the fake relay must finish");

    assert_eq!(results.len(), 1, "one result per rpc");
    let frame = &results[0];
    assert_eq!(frame["t"], "result");
    assert_eq!(frame["cid"], "cid-1");
    assert_eq!(frame["status"], "ok");
    assert!(frame["sessionId"].is_null());

    // The real tool list, carried whole. This is the assertion that proves the
    // bridge is a transport and not a second implementation: the relay adds no
    // schema of its own, so whatever this build exposes is what an agent sees.
    let payload = &frame["payload"];
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["id"], 7);
    let tools = payload["result"]["tools"]
      .as_array()
      .expect("tools/list must return an array");
    assert_eq!(tools.len(), McpServer::instance().get_tools().len());
    assert!(tools.iter().any(|tool| tool["name"] == "navigate"));
    assert!(tools
      .iter()
      .all(|tool| tool["inputSchema"].is_object() && tool["name"].is_string()));
  }

  #[tokio::test]
  async fn initialize_mints_a_session_the_relay_can_echo_back() {
    McpServer::instance().mark_engine_ready_for_tests();

    let (url, server) = fake_relay(
      vec![
        serde_json::json!({ "t": "hello", "protocol": BRIDGE_PROTOCOL, "instanceId": "i" }),
        serde_json::json!({
          "t": "rpc",
          "cid": "cid-init",
          "sessionId": serde_json::Value::Null,
          "payload": {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-11-25" },
          },
        }),
      ],
      1,
    )
    .await;

    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::new(AtomicBool::new(false))).await;
    let results = server.await.expect("the fake relay must finish");

    let frame = &results[0];
    assert_eq!(frame["status"], "ok");
    // The relay turns this into the `mcp-session-id` response header, which is
    // the whole reason it travels beside the payload rather than inside it.
    let session = frame["sessionId"]
      .as_str()
      .expect("initialize must mint a session id");
    assert!(!session.is_empty());
    assert_eq!(frame["payload"]["result"]["protocolVersion"], "2025-11-25");
  }

  #[tokio::test]
  async fn a_notification_is_accepted_with_no_reply_body() {
    McpServer::instance().mark_engine_ready_for_tests();

    let (url, server) = fake_relay(
      vec![serde_json::json!({
        "t": "rpc",
        "cid": "cid-note",
        "sessionId": "session-that-does-not-exist",
        "payload": { "jsonrpc": "2.0", "method": "notifications/initialized" },
      })],
      1,
    )
    .await;

    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::new(AtomicBool::new(false))).await;
    let results = server.await.expect("the fake relay must finish");

    // 202 over HTTP, `accepted` here. JSON-RPC forbids replying to a
    // notification, and the session check must not turn one into a 404.
    assert_eq!(results[0]["status"], "accepted");
    assert!(results[0].get("payload").is_none());
  }

  #[tokio::test]
  async fn an_unknown_session_is_reported_as_such_rather_than_answered() {
    McpServer::instance().mark_engine_ready_for_tests();

    let (url, server) = fake_relay(
      vec![serde_json::json!({
        "t": "rpc",
        "cid": "cid-ghost",
        "sessionId": "00000000-0000-4000-8000-000000000000",
        "payload": { "jsonrpc": "2.0", "id": 3, "method": "tools/list" },
      })],
      1,
    )
    .await;

    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::new(AtomicBool::new(false))).await;
    let results = server.await.expect("the fake relay must finish");

    // The relay renders this as 404, exactly as the loopback server does, so an
    // agent's session handling needs no special case for the remote transport.
    assert_eq!(results[0]["status"], "unknownSession");
  }

  #[tokio::test]
  async fn a_malformed_payload_is_refused_without_dropping_the_socket() {
    McpServer::instance().mark_engine_ready_for_tests();

    let (url, server) = fake_relay(
      vec![
        serde_json::json!({
          "t": "rpc",
          "cid": "cid-bad",
          "sessionId": serde_json::Value::Null,
          "payload": { "not": "json-rpc" },
        }),
        // Sent after the bad one: the socket has to survive it, or one
        // malformed call from a buggy client takes the whole bridge down.
        serde_json::json!({
          "t": "rpc",
          "cid": "cid-good",
          "sessionId": serde_json::Value::Null,
          "payload": { "jsonrpc": "2.0", "id": 9, "method": "ping" },
        }),
      ],
      2,
    )
    .await;

    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::new(AtomicBool::new(false))).await;
    let results = server.await.expect("the fake relay must finish");

    let by_cid = |cid: &str| {
      results
        .iter()
        .find(|frame| frame["cid"] == cid)
        .unwrap_or_else(|| panic!("no result for {cid}"))
    };
    assert_eq!(by_cid("cid-bad")["status"], "badRequest");
    assert_eq!(by_cid("cid-good")["status"], "ok");
  }

  #[tokio::test]
  async fn a_frame_this_build_does_not_know_is_ignored_rather_than_fatal() {
    McpServer::instance().mark_engine_ready_for_tests();

    let (url, server) = fake_relay(
      vec![
        // A newer server's frame. Dropping it must not cost the connection, or
        // a server-side rollout would take every desktop offline.
        serde_json::json!({ "t": "somethingNewer", "data": 1 }),
        serde_json::json!({
          "t": "rpc",
          "cid": "cid-after",
          "sessionId": serde_json::Value::Null,
          "payload": { "jsonrpc": "2.0", "id": 1, "method": "ping" },
        }),
      ],
      1,
    )
    .await;

    let stream = dial_fake(&url).await;
    let _ = pump(stream, Arc::new(AtomicBool::new(false))).await;
    let results = server.await.expect("the fake relay must finish");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["cid"], "cid-after");
    assert_eq!(results[0]["status"], "ok");
  }

  #[tokio::test]
  async fn the_desktop_answers_the_relays_keepalive_ping() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
      .await
      .expect("bind");
    let port = listener.local_addr().expect("port").port();

    let server = tokio::spawn(async move {
      let (socket, _) = listener.accept().await.expect("accept");
      let mut stream = tokio_tungstenite::accept_async(socket)
        .await
        .expect("handshake");
      stream
        .send(Message::Ping(b"keepalive".to_vec().into()))
        .await
        .expect("ping");
      // An idle WebSocket is closed upstream, so the pong is what keeps the
      // bridge alive. A desktop that never answers is dropped and reconnects
      // forever.
      let mut pong = None;
      while let Some(Ok(message)) = stream.next().await {
        if let Message::Pong(payload) = message {
          pong = Some(payload.to_vec());
          break;
        }
      }
      let _ = stream.close(None).await;
      pong
    });

    let stream = dial_fake(&format!("ws://127.0.0.1:{port}")).await;
    let _ = pump(stream, Arc::new(AtomicBool::new(false))).await;
    assert_eq!(
      server.await.expect("the fake relay must finish"),
      Some(b"keepalive".to_vec())
    );
  }

  #[tokio::test]
  async fn a_writer_held_open_by_a_stuck_call_is_abandoned_rather_than_waited_on() {
    // The scenario, reduced to its mechanism: `dispatch` clones the outbound
    // sender into every spawned tool call, so `drop(tx)` in `pump` closes
    // nothing while one is still running. A plain `writer.await` then waits for
    // the slowest tool call, and the socket it would write to is already gone.
    //
    // Unbounded, that pins `pump` forever: the idle timeout fires, `run` never
    // reaches its reconnect sleep, BRIDGE_CONNECTED stays true, and the
    // Integrations page reports a healthy bridge over a dead socket until the
    // app restarts. Exactly the failure the idle timeout exists to catch.
    let (tx, mut rx) = mpsc::channel::<Message>(4);
    let held = tx.clone(); // stands in for an in-flight tool call
    let writer = tokio::spawn(async move { while rx.recv().await.is_some() {} });

    drop(tx);
    let budget = Duration::from_millis(50);
    let started = tokio::time::Instant::now();
    // Bounded from the outside too: an unbounded drain does not return a wrong
    // answer, it never returns at all, and a bare `.await` here would hang the
    // whole test binary instead of reporting the regression.
    tokio::time::timeout(budget * 8, drain_writer(writer, budget))
      .await
      .expect("a stuck call must not be able to hold the reconnect loop open");
    let waited = started.elapsed();
    assert!(waited >= budget, "the budget must actually be honoured");
    // The stand-in outlives the drain, which is the point: the call is left to
    // finish into a closed channel rather than aborted mid-browser-launch.
    drop(held);
  }

  #[test]
  fn stopping_a_bridge_that_is_not_running_returns_before_it_publishes() {
    // Called from logout and from app exit, both of which run whether or not
    // remote control was ever switched on.
    //
    // This used to assert `!is_running()`, call `stop(None)` twice, and assert
    // it again. Both globals default to false, so every assertion already held
    // before a single line of `stop` ran, the test could only have failed if a
    // sibling on another of cargo's threads had flipped them, which is the one
    // thing it was not testing. The property that actually matters is that the
    // guard sits AHEAD of the publish, so a stop on a bridge nobody started
    // cannot emit a state change to a screen that never showed one.
    let source = include_str!("mcp_remote.rs");
    let production = source
      .split_once("\n#[cfg(test)]")
      .map_or(source, |(code, _)| code);
    let stop_fn = production
      .split("pub fn stop(app: Option<&AppHandle>) {")
      .nth(1)
      .expect("stop must exist");
    let body = &stop_fn[..stop_fn.find("\n}").unwrap_or(stop_fn.len())];

    let guard = body
      .find("if !BRIDGE_RUNNING.swap(false, Ordering::SeqCst) {")
      .expect("stop must be guarded by the running flag");
    let publish = body
      .find("publish_state(")
      .expect("stop must publish the new state");
    assert!(
      guard < publish,
      "the running guard must return before anything is published, or every \
       logout and every app exit emits a bridge state change for a bridge that \
       was never switched on"
    );

    // And it really is safe to call when nothing is running.
    stop(None);
    stop(None);
  }

  #[test]
  fn every_path_that_closes_the_bridge_has_one_that_can_reopen_it() {
    // A source assertion, because this is a WHOLE-LIFECYCLE property and no
    // single function can hold it. It exists because the obvious version of the
    // startup gate got it wrong: adding a signed-in check at boot, with nothing
    // on the sign-in path, left "sign out, sign back in" showing remote control
    // switched ON in Settings while the bridge was dead until a restart. The UI
    // said yes and the account page said no desktop was connected.
    //
    // The rule this pins: the bridge is stopped on logout and on exit, so it
    // must be (re)opened on sign-in and on a periodic tick, not only at boot.
    let cloud_auth = std::fs::read_to_string("src/cloud_auth.rs").expect("cloud_auth.rs");
    let lib = std::fs::read_to_string("src/lib.rs").expect("lib.rs");

    // BOTH teardown paths in cloud_auth, not just one. The original assertion
    // used `contains`, which logout alone satisfied, so `invalidate_session`,
    // the AUTOMATIC twin reached when the background refresh loop gives up,
    // silently left the bridge dialling a relay with a credential that no
    // longer existed, for ever, backing off into an "unauthorized" shown to
    // somebody whose session had merely expired.
    assert_eq!(
      cloud_auth.matches("crate::mcp_remote::stop(None);").count(),
      2,
      "both logout AND invalidate_session must hang up the bridge before \
       deleting the credential it was using"
    );
    for owner in ["pub async fn invalidate_session", "pub async fn logout"] {
      let body = cloud_auth
        .split(owner)
        .nth(1)
        .unwrap_or_else(|| panic!("{owner} not found in cloud_auth.rs"));
      let body = &body[..body.len().min(1200)];
      assert!(
        body.contains("crate::mcp_remote::stop(None);"),
        "{owner} clears the credential, so it must close the bridge too"
      );
    }
    assert!(
      lib.contains("mcp_remote::stop(None);"),
      "exiting must release the account's single bridge slot rather than let the server reap it"
    );

    // Three reopen paths, and each is load-bearing: boot restores it, sign-in
    // covers the logout/login round trip, and the tick catches every other way
    // the setting and the socket can drift apart.
    assert_eq!(
      cloud_auth
        .matches("ensure_remote_bridge(&app_handle).await;")
        .count(),
      2,
      "the sign-in path and the periodic reconnect tick must both reopen the bridge"
    );
    assert!(
      lib.contains("cloud_auth::ensure_remote_bridge(&bridge_handle).await;"),
      "startup must reopen a bridge the user had switched on"
    );
  }

  #[test]
  fn the_bridge_and_the_loopback_listener_accept_the_same_body_size() {
    // The frame budget must match the limit the remote endpoint accepts.
    // Changing this number without changing that one makes the remote
    // transport silently stricter than the local one.
    assert_eq!(McpServer::MAX_MESSAGE_BYTES, 1024 * 1024);
  }

  #[tokio::test]
  async fn a_writer_with_nothing_left_to_send_is_joined_immediately() {
    let (tx, mut rx) = mpsc::channel::<Message>(4);
    let writer = tokio::spawn(async move { while rx.recv().await.is_some() {} });

    drop(tx);
    let started = tokio::time::Instant::now();
    // The common case: no call in flight, so the channel really does close and
    // the join returns at once instead of waiting out the budget.
    drain_writer(writer, Duration::from_secs(30)).await;
    assert!(started.elapsed() < Duration::from_secs(5));
  }

  #[tokio::test]
  async fn a_close_naming_the_slot_conflict_is_reported_as_one() {
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
      .await
      .expect("bind");
    let port = listener.local_addr().expect("port").port();

    tokio::spawn(async move {
      let (socket, _) = listener.accept().await.expect("accept");
      let mut stream = tokio_tungstenite::accept_async(socket)
        .await
        .expect("handshake");
      // The exact wording the bridge endpoint sends when the slot is held.
      let _ = stream
        .close(Some(CloseFrame {
          code: CloseCode::Policy,
          reason: "another Donut instance on this account holds the remote-control slot".into(),
        }))
        .await;
      while stream.next().await.is_some() {}
    });

    let stream = dial_fake(&format!("ws://127.0.0.1:{port}")).await;
    let outcome = pump(stream, Arc::new(AtomicBool::new(false))).await;

    let error = outcome.expect_err("a policy close is not a clean shutdown");
    assert!(
      matches!(error, BridgeError::SlotTaken(_)),
      "got {error:?}, which would retry on a one-second timer against a server that keeps saying no"
    );
    assert!(error.is_terminal_refusal());
  }
}

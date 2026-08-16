//! Where a profile's browser actually is, and how to talk to it.
//!
//! Until this module existed, every automation tool answered "where is this
//! browser?" by reading a LOCAL debugging port out of the LOCAL profile
//! directory. A profile launched on a leased host has no local port and no
//! local process, so a customer who paid for remote execution could start a
//! session and then do nothing with it — the one thing the feature exists for.
//!
//! There is exactly one resolver here, [`resolve`], and one connection type,
//! [`CdpConnection`]. Tools ask for a target and get either a page socket on
//! this machine or a relayed socket to the fleet; nothing above this module
//! branches on which. That is deliberate: a parallel set of remote-only tools
//! would drift from the local ones within a release.
//!
//! The remote arm reaches donutbrowser-infra with the USER's own access token.
//! The desktop holds no fleet credential and knows no fleet hostname — infra
//! verifies the session belongs to the caller and relays onward with its own
//! service credential. That boundary is why this is a relay and not a direct
//! connection.

use crate::profile::types::BrowserProfile;
use serde_json::Value;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request as WsRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// How long the WebSocket handshake may take.
///
/// A remote attach crosses desktop → infra → wayfern → agent → the VM, so this
/// is far longer than a loopback connect needs. It matches the relay's own
/// upstream handshake budget: waiting longer than the server does can only
/// report a timeout the server already reported.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long one CDP command may wait for its reply.
///
/// Without a cap, a browser that never answers holds the caller until the
/// socket dies — 90 seconds on the relay, indefinitely on loopback. An
/// automation client that hangs is worse than one that fails.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// Attempts at establishing a connection before giving up.
const CONNECT_ATTEMPTS: u32 = 3;

/// Delay before the second connect attempt; doubles for the third.
const CONNECT_RETRY_BASE: Duration = Duration::from_millis(400);

/// Ceiling on a relayed CDP message.
///
/// Matches the relay's client-facing cap, which matches the fleet's upstream
/// frame cap. Lower, and a screenshot the server was willing to carry is
/// dropped on arrival; higher buys nothing, because the frame never crosses the
/// relay in the first place.
const REMOTE_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Command ids for the two messages the remote arm sends before any tool does.
///
/// Held far above anything a caller uses, so a late reply to the attach
/// handshake can never be mistaken for a tool's answer: both travel on one
/// socket, the handshake on the BROWSER session and every tool on the page
/// session.
const HANDSHAKE_GET_TARGETS_ID: u64 = 9_000_001;
const HANDSHAKE_ATTACH_ID: u64 = 9_000_002;

/// Where a profile's browser is, and what is needed to reach it.
#[derive(Debug, Clone)]
pub enum CdpTarget {
  /// A browser on this machine. The URL is a PAGE-level socket, so commands
  /// carry no CDP session id.
  Local { ws_url: String },
  /// A browser on the fleet, reached through the infra relay. The relay bridges
  /// a BROWSER-level socket, so the connection attaches to a page and stamps
  /// every subsequent message with the resulting session id.
  Remote {
    ws_url: String,
    bearer: String,
    session_id: String,
  },
}

impl CdpTarget {
  /// True when this browser is on the leased fleet rather than this machine.
  pub fn is_remote(&self) -> bool {
    matches!(self, Self::Remote { .. })
  }

  /// A short label for logs and errors. Never carries the credential.
  pub fn describe(&self) -> String {
    match self {
      Self::Local { .. } => "local browser".to_string(),
      Self::Remote { session_id, .. } => format!("remote session {session_id}"),
    }
  }
}

/// Why a target could not be reached, or a command could not be run.
///
/// The variants exist so a caller can tell "come back when it is up" from "that
/// credential is no good" from "the socket broke". Collapsing them into one
/// string is how a session that is merely still provisioning gets reported as a
/// broken one.
#[derive(Debug)]
pub enum CdpError {
  /// Nothing is listening, or the relay could not reach the browser.
  Unreachable(String),
  /// The relay refused the credential.
  Unauthorized(String),
  /// The session exists but is not in a state that can be driven.
  NotDrivable(String),
  /// The socket broke, or a reply never arrived.
  Transport(String),
  /// The browser answered with a CDP `error` object.
  Protocol(String),
}

impl std::fmt::Display for CdpError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Unreachable(m) => write!(f, "browser unreachable: {m}"),
      Self::Unauthorized(m) => write!(f, "not authorised to drive this browser: {m}"),
      Self::NotDrivable(m) => write!(f, "browser is not drivable yet: {m}"),
      Self::Transport(m) => write!(f, "CDP transport failed: {m}"),
      Self::Protocol(m) => write!(f, "CDP error: {m}"),
    }
  }
}

impl CdpError {
  /// Whether a fresh connection attempt could plausibly succeed.
  ///
  /// A refused credential and a session that is still provisioning are answers,
  /// not failures. Retrying either spends the caller's time and, on the relay,
  /// burns one of the four attachments a session is allowed — so the retry can
  /// make the next honest attempt fail too.
  fn is_retryable(&self) -> bool {
    matches!(self, Self::Unreachable(_) | Self::Transport(_))
  }
}

/// Why a profile could not be resolved to a browser at all.
#[derive(Debug)]
pub enum ResolveError {
  /// The profile is not one this app can drive.
  Unsupported(String),
  /// Neither a local process nor a live remote session.
  NotRunning(String),
  /// A remote session exists but its endpoint could not be read.
  Endpoint(String),
}

impl std::fmt::Display for ResolveError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Unsupported(m) | Self::NotRunning(m) | Self::Endpoint(m) => write!(f, "{m}"),
    }
  }
}

/// Whether the caller is willing to wait for a browser that is still coming up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Patience {
  /// One attempt, used to decide local-vs-remote without stalling.
  Immediate,
  /// The full retry budget, used once the answer is known to be local.
  WaitForLaunch,
}

impl Patience {
  fn attempts(self, waiting: u32) -> u32 {
    match self {
      Self::Immediate => 1,
      Self::WaitForLaunch => waiting,
    }
  }
}

/// Find the browser for `profile`, wherever it is running.
///
/// Local wins when both look possible: the profile lock makes a genuine overlap
/// impossible, and a browser on this machine is free to drive while a relayed
/// one crosses two networks.
///
/// The local check is deliberately split in two. One cheap probe decides the
/// arm, so a profile running on the fleet is not held behind twenty-five
/// seconds of local retries; only once remote has been ruled out does the local
/// probe spend its full budget waiting for a browser that is still starting.
/// The same split covers a stale `process_id` left by a crash — nothing answers
/// on the recorded port, so the fleet session is found instead of a dead one.
pub async fn resolve(profile: &BrowserProfile) -> Result<CdpTarget, ResolveError> {
  if profile.browser != "wayfern" {
    return Err(ResolveError::Unsupported(format!(
      "Profile '{}' runs {}, which cannot be driven over CDP",
      profile.name, profile.browser
    )));
  }

  let has_local_process = profile.process_id.is_some();

  if has_local_process {
    if let Some(ws_url) = local_page_ws_url(profile, Patience::Immediate).await {
      return Ok(CdpTarget::Local { ws_url });
    }
  }

  if let Some(session) =
    crate::remote_session::live_session_for_profile(&profile.id.to_string()).await
  {
    let endpoint = crate::remote_session::cdp_endpoint(&session.session_id)
      .await
      .map_err(|e| ResolveError::Endpoint(e.to_string()))?;
    let bearer = crate::remote_session::access_token_for_cdp().map_err(ResolveError::Endpoint)?;
    log::info!(
      "Driving profile '{}' through remote session {}",
      profile.name,
      session.session_id
    );
    return Ok(CdpTarget::Remote {
      ws_url: endpoint.ws_url,
      bearer,
      session_id: session.session_id,
    });
  }

  if has_local_process {
    return match local_page_ws_url(profile, Patience::WaitForLaunch).await {
      Some(ws_url) => Ok(CdpTarget::Local { ws_url }),
      None => Err(ResolveError::NotRunning(format!(
        "No CDP connection available for profile '{}'. Make sure the browser is running.",
        profile.name
      ))),
    };
  }

  Err(ResolveError::NotRunning(format!(
    "Profile '{}' is not running",
    profile.name
  )))
}

/// The debugging port a locally launched browser registered for this profile.
async fn local_cdp_port(profile: &BrowserProfile, patience: Patience) -> Option<u16> {
  let profiles_dir = crate::profile::manager::ProfileManager::instance().get_profiles_dir();
  let profile_path = profile.get_profile_data_path(&profiles_dir);
  let profile_path_str = profile_path.to_string_lossy().to_string();

  // Port info is written once the process is up, so a tool called straight
  // after a launch has to wait for it.
  for attempt in 0..patience.attempts(10) {
    if attempt > 0 {
      tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if let Some(port) = crate::wayfern_manager::WayfernManager::instance()
      .get_cdp_port(&profile_path_str)
      .await
    {
      return Some(port);
    }
  }
  None
}

/// A page-level socket on a locally running browser.
///
/// Returns `None` rather than an error: a miss is how the resolver decides the
/// browser is not local, and a port whose process has since died answers
/// nothing, which is exactly the signal that decision needs.
async fn local_page_ws_url(profile: &BrowserProfile, patience: Patience) -> Option<String> {
  let port = local_cdp_port(profile, patience).await?;
  let listing = format!("http://127.0.0.1:{port}/json");
  let client = reqwest::Client::new();

  let mut last_err = String::new();
  for attempt in 0..patience.attempts(15) {
    if attempt > 0 {
      tokio::time::sleep(Duration::from_secs(1)).await;
    }
    match client
      .get(&listing)
      .timeout(Duration::from_secs(3))
      .send()
      .await
    {
      Ok(response) => match response.json::<Vec<Value>>().await {
        Ok(targets) => {
          if let Some(ws_url) = pick_local_page_socket(&targets) {
            return Some(ws_url);
          }
          last_err = "no page target found in browser".to_string();
        }
        Err(e) => last_err = format!("failed to parse CDP targets: {e}"),
      },
      Err(e) => last_err = format!("failed to reach the browser's CDP endpoint: {e}"),
    }
  }

  if patience == Patience::WaitForLaunch {
    log::warn!("Local CDP discovery on port {port} gave up: {last_err}");
  }
  None
}

/// Pick a drivable page from what `/json` lists on a local browser.
pub fn pick_local_page_socket(targets: &[Value]) -> Option<String> {
  targets
    .iter()
    .find(|t| t.get("type").and_then(Value::as_str) == Some("page"))
    .and_then(|t| t.get("webSocketDebuggerUrl"))
    .and_then(Value::as_str)
    .map(str::to_string)
}

/// Pick a drivable page from a `Target.getTargets` reply.
///
/// DevTools' own frontend is a page target too, and attaching to it drives the
/// inspector instead of the site — a failure that reports success and moves
/// nothing.
pub fn pick_remote_page_target(result: &Value) -> Option<String> {
  result
    .get("targetInfos")
    .and_then(Value::as_array)?
    .iter()
    .find(|info| {
      let is_page = info.get("type").and_then(Value::as_str) == Some("page");
      let url = info.get("url").and_then(Value::as_str).unwrap_or_default();
      is_page && !url.starts_with("devtools://")
    })
    .and_then(|info| info.get("targetId"))
    .and_then(Value::as_str)
    .map(str::to_string)
}

/// One outgoing CDP message, addressed to a page when a session id is in play.
///
/// Flattened sessions keep `method` at the top level on the way back, so event
/// matching is identical on both arms and no caller has to know which it is on.
pub fn cdp_frame(session: Option<&str>, id: u64, method: &str, params: Value) -> Value {
  let mut message = serde_json::json!({ "id": id, "method": method, "params": params });
  if let Some(session) = session {
    message["sessionId"] = Value::String(session.to_string());
  }
  message
}

/// Why the peer hung up.
#[derive(Debug, Clone)]
struct CloseInfo {
  code: u16,
  reason: String,
}

/// An open CDP conversation with one browser, local or relayed.
pub struct CdpConnection {
  stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
  /// Set only for a relayed connection: stamped onto every outgoing message so
  /// page-level commands reach the page rather than the browser.
  cdp_session: Option<String>,
  closed: Option<CloseInfo>,
}

impl CdpConnection {
  fn new(stream: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
    Self {
      stream,
      cdp_session: None,
      closed: None,
    }
  }

  /// Send `method` as command `id`.
  pub async fn send_command(
    &mut self,
    id: u64,
    method: &str,
    params: Value,
  ) -> Result<(), CdpError> {
    use futures_util::sink::SinkExt;
    let frame = cdp_frame(self.cdp_session.as_deref(), id, method, params);
    self
      .stream
      .send(Message::Text(frame.to_string().into()))
      .await
      .map_err(|e| CdpError::Transport(format!("failed to send CDP command: {e}")))
  }

  /// The next text message, or `None` once the peer has gone.
  ///
  /// Binary frames, pings and pongs are consumed silently; a close is recorded
  /// so its reason survives into whatever error the caller builds.
  pub async fn next_text(&mut self) -> Option<Result<String, CdpError>> {
    use futures_util::stream::StreamExt;
    loop {
      match self.stream.next().await? {
        Ok(Message::Text(text)) => return Some(Ok(text.to_string())),
        Ok(Message::Close(frame)) => {
          self.closed = frame.map(|f| CloseInfo {
            code: u16::from(f.code),
            reason: f.reason.to_string(),
          });
          return None;
        }
        Ok(_) => continue,
        Err(e) => {
          return Some(Err(CdpError::Transport(format!(
            "CDP WebSocket error: {e}"
          ))))
        }
      }
    }
  }

  /// Turn a hang-up into the error it means.
  ///
  /// The relay's close codes are its whole vocabulary: 1008 is "that credential
  /// is no good", 1013 is "come back when the session is up". Reporting either
  /// as a generic transport failure throws away the only actionable thing the
  /// server said.
  pub fn closed_error(&self, context: &str) -> CdpError {
    match &self.closed {
      Some(info) if info.reason.is_empty() => {
        classify_close(info.code, format!("{context} (close {})", info.code))
      }
      Some(info) => classify_close(
        info.code,
        format!("{context} ({}: {})", info.code, info.reason),
      ),
      None => CdpError::Transport(format!("{context} (connection closed)")),
    }
  }

  /// Send a command and read its reply, bounded by [`COMMAND_TIMEOUT`].
  pub async fn call(&mut self, id: u64, method: &str, params: Value) -> Result<Value, CdpError> {
    self.send_command(id, method, params).await?;
    self.await_reply(id, COMMAND_TIMEOUT).await
  }

  /// Read until the reply to `id` arrives, discarding events on the way.
  pub async fn await_reply(&mut self, id: u64, timeout: Duration) -> Result<Value, CdpError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
      let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
      if remaining.is_zero() {
        return Err(CdpError::Transport(
          "timed out waiting for a CDP response".to_string(),
        ));
      }
      let text = match tokio::time::timeout(remaining, self.next_text()).await {
        Err(_) => {
          return Err(CdpError::Transport(
            "timed out waiting for a CDP response".to_string(),
          ))
        }
        Ok(None) => return Err(self.closed_error("no response received from CDP")),
        Ok(Some(result)) => result?,
      };

      let response: Value = serde_json::from_str(&text)
        .map_err(|e| CdpError::Protocol(format!("failed to parse CDP response: {e}")))?;
      if response.get("id") != Some(&Value::from(id)) {
        continue;
      }
      if let Some(error) = response.get("error") {
        return Err(CdpError::Protocol(error.to_string()));
      }
      return Ok(
        response
          .get("result")
          .cloned()
          .unwrap_or_else(|| serde_json::json!({})),
      );
    }
  }

  /// Hang up politely so the peer releases its side immediately.
  ///
  /// On the relay every open socket costs a real stream on the leased host and
  /// counts against the session's attachment cap, so dropping the TCP
  /// connection and letting it time out is not good enough.
  pub async fn close(mut self) {
    let _ = self.stream.close(None).await;
  }

  /// Move a browser-level socket onto a page.
  ///
  /// The relay bridges `/devtools/browser/<id>`. Every tool here speaks
  /// `Page.*`, `Runtime.*` and `Input.*`, which a browser socket answers with
  /// `'Page.navigate' wasn't found`. Attaching flat, and stamping the resulting
  /// session id onto everything after it, is what makes the tools this app
  /// already has work remotely without a single per-tool change.
  async fn attach_to_page(&mut self) -> Result<(), CdpError> {
    let targets = self
      .call(
        HANDSHAKE_GET_TARGETS_ID,
        "Target.getTargets",
        serde_json::json!({}),
      )
      .await?;
    let target_id = pick_remote_page_target(&targets)
      .ok_or_else(|| CdpError::NotDrivable("the remote browser has no page open".to_string()))?;

    let attached = self
      .call(
        HANDSHAKE_ATTACH_ID,
        "Target.attachToTarget",
        serde_json::json!({ "targetId": target_id, "flatten": true }),
      )
      .await?;

    let session = attached
      .get("sessionId")
      .and_then(Value::as_str)
      .ok_or_else(|| {
        CdpError::Protocol("Target.attachToTarget returned no sessionId".to_string())
      })?;
    self.cdp_session = Some(session.to_string());
    Ok(())
  }
}

/// Ids used by the one-shot command runners below.
///
/// A caller never picks these, so they are stated once here rather than being
/// re-derived at each call site.
const RUN_PAGE_ENABLE_ID: u64 = 1;
const RUN_COMMAND_ID: u64 = 2;
const RUN_PAGE_DISABLE_ID: u64 = 3;

/// The attach handshake and the commands after it share one socket, so a
/// handshake reply carrying a command's id would be handed back as that
/// command's result. Checked at compile time because the failure it prevents is
/// silent: the wrong reply is still a well-formed reply.
const _: () = {
  assert!(HANDSHAKE_GET_TARGETS_ID != HANDSHAKE_ATTACH_ID);
  assert!(HANDSHAKE_GET_TARGETS_ID != RUN_PAGE_ENABLE_ID);
  assert!(HANDSHAKE_GET_TARGETS_ID != RUN_COMMAND_ID);
  assert!(HANDSHAKE_GET_TARGETS_ID != RUN_PAGE_DISABLE_ID);
  assert!(HANDSHAKE_ATTACH_ID != RUN_PAGE_ENABLE_ID);
  assert!(HANDSHAKE_ATTACH_ID != RUN_COMMAND_ID);
  assert!(HANDSHAKE_ATTACH_ID != RUN_PAGE_DISABLE_ID);
};

/// Run one command on a fresh connection and hand back its result.
pub async fn run_command(
  target: &CdpTarget,
  method: &str,
  params: Value,
) -> Result<Value, CdpError> {
  let mut connection = target.connect().await?;
  let result = connection.call(RUN_COMMAND_ID, method, params).await;
  connection.close().await;
  result
}

/// Run one command, then wait for the page to finish loading.
///
/// Used for anything that might navigate: `Page.navigate` obviously, but also a
/// click or a script that turns out to follow a link. When nothing navigates,
/// the wait simply expires and the command's own result is returned — that is
/// the intended path, not a failure.
pub async fn run_command_awaiting_load(
  target: &CdpTarget,
  method: &str,
  params: Value,
  timeout_secs: u64,
) -> Result<Value, CdpError> {
  let mut connection = target.connect().await?;

  // Page events have to be on before the command runs, or `loadEventFired`
  // for a fast navigation is missed and the wait runs to its full timeout.
  connection
    .call(RUN_PAGE_ENABLE_ID, "Page.enable", serde_json::json!({}))
    .await?;
  connection
    .send_command(RUN_COMMAND_ID, method, params)
    .await?;

  let mut command_result = None;
  let mut failure = None;
  let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

  loop {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
      break;
    }
    let text = match tokio::time::timeout(remaining, connection.next_text()).await {
      Ok(Some(Ok(text))) => text,
      Ok(Some(Err(e))) => {
        // A dropped connection cannot unsay a command the browser already
        // answered: the navigation was accepted, and the command's result is
        // the best answer available even if the load event never arrived.
        // Only when nothing was answered yet is the lost socket a failure.
        if command_result.is_none() {
          failure = Some(e);
        }
        break;
      }
      // The peer hung up, or the wait expired. Either way whatever the command
      // already answered is the best result available.
      Ok(None) | Err(_) => break,
    };

    let response: Value = serde_json::from_str(&text).unwrap_or_default();

    if response.get("id") == Some(&Value::from(RUN_COMMAND_ID)) {
      if let Some(error) = response.get("error") {
        failure = Some(CdpError::Protocol(error.to_string()));
        break;
      }
      command_result = Some(
        response
          .get("result")
          .cloned()
          .unwrap_or_else(|| serde_json::json!({})),
      );
    }

    // Flattened remote sessions keep `method` at the top level, so this match
    // is identical on both arms.
    if response.get("method") == Some(&Value::from("Page.loadEventFired")) {
      break;
    }
  }

  let _ = connection
    .send_command(RUN_PAGE_DISABLE_ID, "Page.disable", serde_json::json!({}))
    .await;
  let closed = connection.closed_error("no response received from CDP");
  connection.close().await;

  if let Some(error) = failure {
    return Err(error);
  }
  command_result.ok_or(closed)
}

/// Point a browser at a URL and wait for it to settle.
///
/// This is what "open a URL in that profile" means once the browser is already
/// up, wherever it is. A remote session navigates its existing page rather than
/// opening a tab: a tab opened on a leased host that nobody can see or close is
/// not a feature, it is litter on hardware the user is paying for by the hour.
pub async fn navigate(target: &CdpTarget, url: &str, timeout_secs: u64) -> Result<(), CdpError> {
  run_command_awaiting_load(
    target,
    "Page.navigate",
    serde_json::json!({ "url": url }),
    timeout_secs,
  )
  .await
  .map(|_| ())
}

/// Map a WebSocket close code onto what the caller should do about it.
pub fn classify_close(code: u16, detail: String) -> CdpError {
  match code {
    1008 => CdpError::Unauthorized(detail),
    1013 => CdpError::NotDrivable(detail),
    1009 => CdpError::Transport(format!("{detail} — message too large")),
    1011 => CdpError::Unreachable(detail),
    _ => CdpError::Transport(detail),
  }
}

impl CdpTarget {
  /// Open a conversation with this browser.
  ///
  /// Retries a connection that failed for a reason a retry could fix, and never
  /// one that failed because the answer was no.
  pub async fn connect(&self) -> Result<CdpConnection, CdpError> {
    let mut attempt = 0u32;
    loop {
      let error = match self.connect_once().await {
        Ok(connection) => return Ok(connection),
        Err(e) => e,
      };
      attempt += 1;
      if attempt >= CONNECT_ATTEMPTS || !error.is_retryable() {
        return Err(error);
      }
      let delay = CONNECT_RETRY_BASE * 2u32.pow(attempt - 1);
      log::warn!(
        "CDP connect to {} failed ({error}); retrying in {}ms",
        self.describe(),
        delay.as_millis()
      );
      tokio::time::sleep(delay).await;
    }
  }

  async fn connect_once(&self) -> Result<CdpConnection, CdpError> {
    match self {
      Self::Local { ws_url } => {
        let request = ws_url
          .as_str()
          .into_client_request()
          .map_err(|e| CdpError::Unreachable(format!("invalid CDP endpoint: {e}")))?;
        Ok(CdpConnection::new(dial(request, None).await?))
      }
      Self::Remote {
        ws_url,
        bearer,
        session_id,
      } => {
        let mut connection = dial_relay(ws_url, bearer).await?;
        if let Err(e) = connection.attach_to_page().await {
          log::warn!("Could not attach to a page in remote session {session_id}: {e}");
          return Err(e);
        }
        Ok(connection)
      }
    }
  }
}

/// A relay socket with nothing done to it yet.
pub type RelaySocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Open a session's BROWSER-level relay socket and hand it back untouched.
///
/// Deliberately skips the page attach that [`CdpTarget::connect`] performs. The
/// tools in this app all speak `Page.*` and need a page session stamped onto
/// every message; an external automation client does not, and must not have
/// one. Playwright's `connectOverCDP` expects the browser endpoint: it drives
/// `Target.setAutoAttach` and `Target.getTargets` itself and builds its own
/// session map, so a socket already attached to one page would hide every other
/// target from it and stamp a session id onto messages it did not address.
///
/// This is what makes a remote session usable from outside the app at all. The
/// relay only accepts the user's cloud credential, which no API consumer holds
/// and none should — so the socket is opened here, with the credential this
/// process already has, and proxied to the caller.
pub async fn open_relay_socket(session_id: &str) -> Result<RelaySocket, CdpError> {
  let endpoint = crate::remote_session::cdp_endpoint(session_id)
    .await
    .map_err(endpoint_lookup_error)?;
  let bearer = crate::remote_session::access_token_for_cdp().map_err(CdpError::Unauthorized)?;

  let config = relay_socket_config();
  let refused = match dial(relay_request(&endpoint.ws_url, &bearer)?, Some(config)).await {
    Ok(stream) => return Ok(stream),
    Err(CdpError::Unauthorized(reason)) => reason,
    Err(e) => return Err(e),
  };

  log::info!("The CDP relay refused the stored access token; refreshing and retrying once");
  crate::cloud_auth::CLOUD_AUTH
    .refresh_access_token()
    .await
    .map_err(|e| CdpError::Unauthorized(format!("{refused}; token refresh failed: {e}")))?;
  let token = crate::remote_session::access_token_for_cdp()
    .map_err(|e| CdpError::Unauthorized(format!("{refused}; {e}")))?;
  dial(relay_request(&endpoint.ws_url, &token)?, Some(config)).await
}

/// Why the backend would not say where to attach.
///
/// Collapsing this into "unreachable" is what made a session the user had
/// already stopped answer 502, so an automation client read a finished session
/// as a broken gateway and retried it. A session that is over, or that is not
/// the caller's, is a 404: there is no browser at this address.
fn endpoint_lookup_error(err: crate::remote_session::RemoteSessionError) -> CdpError {
  use crate::remote_session::RemoteSessionError;
  match err {
    RemoteSessionError::NotAuthorised(m) => CdpError::Unauthorized(m),
    RemoteSessionError::Conflict(m) => CdpError::NotDrivable(m),
    RemoteSessionError::NoCapacity(m) => CdpError::Unreachable(m),
    RemoteSessionError::Other(m) => {
      // `Other` carries the backend's own envelope. A 404 for a closed or
      // foreign session arrives here, and it is the common case rather than an
      // exotic one, so it is read back out rather than lumped in with a
      // genuine transport failure.
      if m.contains("REMOTE_SESSION_NOT_FOUND") || m.contains("404") {
        CdpError::NotDrivable(m)
      } else {
        CdpError::Unreachable(m)
      }
    }
  }
}

/// Frame limits for a relay socket. Matches the relay's own client-facing cap.
pub fn relay_socket_config() -> WebSocketConfig {
  WebSocketConfig::default()
    .max_message_size(Some(REMOTE_MAX_MESSAGE_BYTES))
    .max_frame_size(Some(REMOTE_MAX_MESSAGE_BYTES))
}

/// The ceiling a proxied CDP message may reach, so both ends agree.
pub const MAX_RELAY_MESSAGE_BYTES: usize = REMOTE_MAX_MESSAGE_BYTES;

/// Open the relay socket, refreshing the access token once if it is refused.
///
/// The access token lives long enough that an app left open overnight still
/// holds a valid-looking one after it has been rotated. Failing a whole tool
/// call for that — when the very next HTTP request would have refreshed it
/// silently — is a bug the user reads as "remote driving is flaky".
async fn dial_relay(ws_url: &str, bearer: &str) -> Result<CdpConnection, CdpError> {
  let config = relay_socket_config();

  let refused = match dial(relay_request(ws_url, bearer)?, Some(config)).await {
    Ok(stream) => return Ok(CdpConnection::new(stream)),
    Err(CdpError::Unauthorized(reason)) => reason,
    Err(e) => return Err(e),
  };

  log::info!("The CDP relay refused the stored access token; refreshing and retrying once");
  crate::cloud_auth::CLOUD_AUTH
    .refresh_access_token()
    .await
    .map_err(|e| CdpError::Unauthorized(format!("{refused}; token refresh failed: {e}")))?;
  let token = crate::remote_session::access_token_for_cdp()
    .map_err(|e| CdpError::Unauthorized(format!("{refused}; {e}")))?;
  let stream = dial(relay_request(ws_url, &token)?, Some(config)).await?;
  Ok(CdpConnection::new(stream))
}

/// Build the upgrade request for the relay.
///
/// The credential goes in a header, never the query string: a URL that grants
/// full control of a live browser must not reach a proxy access log.
fn relay_request(ws_url: &str, bearer: &str) -> Result<WsRequest, CdpError> {
  let mut request = ws_url
    .into_client_request()
    .map_err(|e| CdpError::Unreachable(format!("invalid relay endpoint: {e}")))?;
  let value = format!("Bearer {bearer}").parse().map_err(|_| {
    CdpError::Unauthorized("the stored access token is not a valid header value".to_string())
  })?;
  request.headers_mut().insert(
    tokio_tungstenite::tungstenite::http::header::AUTHORIZATION,
    value,
  );
  Ok(request)
}

/// Perform the handshake, bounded by [`CONNECT_TIMEOUT`].
async fn dial(
  request: WsRequest,
  config: Option<WebSocketConfig>,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, CdpError> {
  let connect = tokio_tungstenite::connect_async_with_config(request, config, false);
  match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
    Err(_) => Err(CdpError::Unreachable(format!(
      "the CDP endpoint did not answer within {}s",
      CONNECT_TIMEOUT.as_secs()
    ))),
    Ok(Ok((stream, _response))) => Ok(stream),
    Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => {
      Err(classify_handshake_status(response.status().as_u16()))
    }
    Ok(Err(e)) => Err(CdpError::Unreachable(e.to_string())),
  }
}

/// Map a refused upgrade onto the error it means.
///
/// A 401 is a credential problem the caller can fix by signing in again; a 404
/// means the session is not theirs or no longer exists. Surfacing either as
/// "connection failed" is what makes an automation client retry forever.
pub fn classify_handshake_status(status: u16) -> CdpError {
  match status {
    401 | 403 => {
      CdpError::Unauthorized(format!("the relay refused the credential (HTTP {status})"))
    }
    404 => CdpError::NotDrivable(
      "no such remote session, or it does not belong to this account".to_string(),
    ),
    409 => CdpError::NotDrivable("the remote session is not drivable yet".to_string()),
    429 => CdpError::NotDrivable("too many attachments to this remote session".to_string()),
    other => CdpError::Unreachable(format!("the relay answered HTTP {other}")),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_page_target_is_preferred_over_the_devtools_frontend() {
    // Attaching to devtools:// drives the inspector, not the site, and the
    // failure is silent: navigate returns success and nothing moves.
    let targets = serde_json::json!({
      "targetInfos": [
        { "targetId": "t-devtools", "type": "page", "url": "devtools://devtools/bundled/x.html" },
        { "targetId": "t-page", "type": "page", "url": "https://example.com/" },
      ]
    });
    assert_eq!(pick_remote_page_target(&targets).as_deref(), Some("t-page"));
  }

  #[test]
  fn service_workers_and_browser_targets_are_not_pages() {
    let targets = serde_json::json!({
      "targetInfos": [
        { "targetId": "t-sw", "type": "service_worker", "url": "https://example.com/sw.js" },
        { "targetId": "t-browser", "type": "browser", "url": "" },
      ]
    });
    assert!(pick_remote_page_target(&targets).is_none());
  }

  #[test]
  fn a_fresh_browser_showing_only_about_blank_is_still_drivable() {
    // The first thing a remote launch has open is about:blank. Refusing it
    // would leave every session unusable until the user navigated by hand —
    // which they cannot do, because navigating is what needs the attach.
    let targets = serde_json::json!({
      "targetInfos": [{ "targetId": "t-blank", "type": "page", "url": "about:blank" }]
    });
    assert_eq!(
      pick_remote_page_target(&targets).as_deref(),
      Some("t-blank")
    );
  }

  #[test]
  fn an_empty_reply_resolves_to_no_target_rather_than_panicking() {
    assert!(pick_remote_page_target(&serde_json::json!({})).is_none());
    assert!(pick_remote_page_target(&serde_json::json!({ "targetInfos": [] })).is_none());
  }

  #[test]
  fn the_local_page_socket_is_read_from_the_json_listing() {
    let targets = vec![
      serde_json::json!({ "type": "background_page", "webSocketDebuggerUrl": "ws://x/bg" }),
      serde_json::json!({ "type": "page", "webSocketDebuggerUrl": "ws://127.0.0.1:1/devtools/page/A" }),
    ];
    assert_eq!(
      pick_local_page_socket(&targets).as_deref(),
      Some("ws://127.0.0.1:1/devtools/page/A")
    );
    assert!(pick_local_page_socket(&[]).is_none());
  }

  #[test]
  fn a_remote_frame_addresses_the_page_and_a_local_one_does_not() {
    // A page-level command sent on the relay's BROWSER socket comes back as
    // "'Page.navigate' wasn't found". One missing sessionId on one message is
    // enough to make a single tool fail while every other tool works — a
    // partial failure that reads as a flaky VM.
    let remote = cdp_frame(
      Some("SESSION-42"),
      7,
      "Page.navigate",
      serde_json::json!({ "url": "https://example.com" }),
    );
    assert_eq!(remote["sessionId"], "SESSION-42");
    assert_eq!(remote["id"], 7);
    assert_eq!(remote["method"], "Page.navigate");
    assert_eq!(remote["params"]["url"], "https://example.com");

    let local = cdp_frame(None, 7, "Page.navigate", serde_json::json!({}));
    assert!(local.get("sessionId").is_none());
    assert_eq!(local["id"], 7);
  }

  #[test]
  fn a_relay_close_says_what_the_caller_should_do_about_it() {
    // These codes are the relay's entire vocabulary. Collapsing them into one
    // transport failure is how "your session is still provisioning" and "you
    // are signed out" both become "something went wrong".
    assert!(matches!(
      classify_close(1008, "x".into()),
      CdpError::Unauthorized(_)
    ));
    assert!(matches!(
      classify_close(1013, "x".into()),
      CdpError::NotDrivable(_)
    ));
    assert!(matches!(
      classify_close(1011, "x".into()),
      CdpError::Unreachable(_)
    ));
    assert!(matches!(
      classify_close(1009, "x".into()),
      CdpError::Transport(_)
    ));
    assert!(matches!(
      classify_close(1000, "x".into()),
      CdpError::Transport(_)
    ));
  }

  #[test]
  fn only_the_failures_a_retry_could_fix_are_retried() {
    assert!(CdpError::Unreachable("x".into()).is_retryable());
    assert!(CdpError::Transport("x".into()).is_retryable());
    assert!(!CdpError::Unauthorized("x".into()).is_retryable());
    assert!(!CdpError::NotDrivable("x".into()).is_retryable());
    assert!(!CdpError::Protocol("x".into()).is_retryable());
  }

  #[test]
  fn a_refused_upgrade_is_not_reported_as_an_unreachable_browser() {
    assert!(matches!(
      classify_handshake_status(401),
      CdpError::Unauthorized(_)
    ));
    assert!(matches!(
      classify_handshake_status(404),
      CdpError::NotDrivable(_)
    ));
    assert!(matches!(
      classify_handshake_status(409),
      CdpError::NotDrivable(_)
    ));
    assert!(matches!(
      classify_handshake_status(429),
      CdpError::NotDrivable(_)
    ));
    assert!(matches!(
      classify_handshake_status(502),
      CdpError::Unreachable(_)
    ));
  }

  #[test]
  fn a_relay_endpoint_carries_the_credential_in_a_header() {
    // In the query string it would reach every proxy log between here and the
    // origin, and this credential grants full control of a live browser.
    let request = relay_request(
      "wss://api.donutbrowser.com/api/remote-sessions/cdp?session_id=s1",
      "secret-token",
    )
    .expect("a wss endpoint must build a request");
    assert_eq!(
      request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok()),
      Some("Bearer secret-token")
    );
    assert!(!request.uri().to_string().contains("secret-token"));
  }

  #[test]
  fn a_session_that_is_over_is_not_reported_as_a_broken_gateway() {
    // Observed against the real backend: attaching to a session the user had
    // just stopped answered 502, so a CDP client read "this is finished" as
    // "the gateway is down" and retried it.
    use crate::remote_session::RemoteSessionError;
    assert!(matches!(
      endpoint_lookup_error(RemoteSessionError::Other(
        r#"(404) {"code":"REMOTE_SESSION_NOT_FOUND"}"#.to_string()
      )),
      CdpError::NotDrivable(_)
    ));
    assert!(matches!(
      endpoint_lookup_error(RemoteSessionError::Conflict("already open".into())),
      CdpError::NotDrivable(_)
    ));
    assert!(matches!(
      endpoint_lookup_error(RemoteSessionError::NotAuthorised("signed out".into())),
      CdpError::Unauthorized(_)
    ));
    // A genuine transport failure must still read as one.
    assert!(matches!(
      endpoint_lookup_error(RemoteSessionError::Other(
        "reach backend: connection refused".to_string()
      )),
      CdpError::Unreachable(_)
    ));
  }

  #[test]
  fn a_malformed_endpoint_is_refused_rather_than_dialled() {
    assert!(relay_request("not a url", "t").is_err());
  }

  #[test]
  fn a_target_describes_itself_without_leaking_the_credential() {
    let remote = CdpTarget::Remote {
      ws_url: "wss://api.donutbrowser.com/api/remote-sessions/cdp?session_id=s1".to_string(),
      bearer: "secret-token".to_string(),
      session_id: "s1".to_string(),
    };
    assert!(remote.is_remote());
    let described = remote.describe();
    assert!(described.contains("s1"));
    assert!(!described.contains("secret-token"));

    let local = CdpTarget::Local {
      ws_url: "ws://127.0.0.1:1/devtools/page/A".to_string(),
    };
    assert!(!local.is_remote());
  }

  #[test]
  fn a_hasty_probe_tries_once_and_a_patient_one_waits() {
    // The split is what stops a profile running on the fleet from being held
    // behind twenty-five seconds of local retries before anyone looks remote.
    assert_eq!(Patience::Immediate.attempts(10), 1);
    assert_eq!(Patience::WaitForLaunch.attempts(10), 10);
  }

  // --- Against a real socket -----------------------------------------------
  //
  // Everything above is pure. These drive the client against a WebSocket
  // server that answers the way the relay does, because the failure this whole
  // module exists to fix — page commands sent on a browser-level socket coming
  // back as "'Page.navigate' wasn't found" — cannot be caught by inspecting a
  // JSON value. It only shows up when something actually answers.

  /// What the fake relay observed.
  #[derive(Debug, Default)]
  struct RelayLog {
    /// The credential the client presented on the upgrade.
    authorization: Option<String>,
    /// Every message the client sent, in order.
    received: Vec<Value>,
  }

  /// How the fake relay should behave once a client connects.
  #[derive(Clone, Copy, PartialEq, Eq)]
  enum RelayBehaviour {
    /// Answer the attach handshake, then echo every command back.
    Cooperative,
    /// Hang up the way a session that is not yet up does.
    RefuseAsNotDrivable,
    /// Answer the navigation, then drop the socket before the load event,
    /// the way a relay does when the browser it bridges dies mid-navigation.
    DropAfterNavigateReply,
    /// Drop the socket without answering the navigation.
    DropBeforeNavigateReply,
    /// Answer the navigation with a CDP error object.
    AnswerNavigateWithError,
    /// Answer commands but never emit the load event, so the wait expires.
    NeverLoadEvent,
  }

  /// The CDP session id the fake relay hands out for a flat attach.
  const FAKE_CDP_SESSION: &str = "CDP-SESSION-1";

  /// A stand-in for the infra relay bridged onto a browser-level socket.
  ///
  /// Answers `Target.getTargets` and `Target.attachToTarget` exactly as a real
  /// browser endpoint does, then echoes each command back so the test can read
  /// what was actually on the wire.
  //
  // The large-Err allow is forced by tungstenite's server-callback signature:
  // its `ErrorResponse` is a full `http::Response`, and the callback is the
  // only place the upgrade request's headers are visible.
  #[allow(clippy::result_large_err)]
  async fn fake_relay(behaviour: RelayBehaviour) -> (String, tokio::task::JoinHandle<RelayLog>) {
    use futures_util::sink::SinkExt;
    use futures_util::stream::StreamExt;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
      .await
      .expect("the fake relay must bind");
    let port = listener.local_addr().expect("a bound port").port();

    let handle = tokio::spawn(async move {
      let mut log = RelayLog::default();
      let Ok((socket, _)) = listener.accept().await else {
        return log;
      };

      let seen = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
      let captured = seen.clone();
      let Ok(mut stream) = tokio_tungstenite::accept_hdr_async(
        socket,
        |request: &WsRequest,
         response: tokio_tungstenite::tungstenite::handshake::server::Response| {
          *captured.lock().unwrap() = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
          Ok(response)
        },
      )
      .await
      else {
        return log;
      };
      log.authorization = seen.lock().unwrap().clone();

      if behaviour == RelayBehaviour::RefuseAsNotDrivable {
        let _ = stream
          .close(Some(CloseFrame {
            code: CloseCode::Library(1013),
            reason: "session is provisioning, not drivable".into(),
          }))
          .await;
        return log;
      }

      while let Some(Ok(message)) = stream.next().await {
        let Message::Text(text) = message else {
          continue;
        };
        let Ok(request) = serde_json::from_str::<Value>(&text) else {
          continue;
        };
        log.received.push(request.clone());

        let method = request.get("method").and_then(Value::as_str).unwrap_or("");

        // A reply dropped mid-flight must not look like the command answered.
        if behaviour == RelayBehaviour::DropBeforeNavigateReply && method == "Page.navigate" {
          break;
        }

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let reply = match method {
          "Target.getTargets" => serde_json::json!({
            "id": id,
            "result": { "targetInfos": [
              { "targetId": "page-1", "type": "page", "url": "https://example.com/" }
            ]}
          }),
          "Target.attachToTarget" => serde_json::json!({
            "id": id,
            "result": { "sessionId": FAKE_CDP_SESSION }
          }),
          "Page.navigate" if behaviour == RelayBehaviour::AnswerNavigateWithError => {
            serde_json::json!({
              "id": id,
              "sessionId": request.get("sessionId").cloned().unwrap_or(Value::Null),
              "error": { "code": -32000, "message": "fake navigation error" }
            })
          }
          // Everything else is handed straight back, so the test can assert on
          // the exact frame the client put on the wire.
          _ => serde_json::json!({
            "id": id,
            "sessionId": request.get("sessionId").cloned().unwrap_or(Value::Null),
            "result": { "echo": request }
          }),
        };
        if stream
          .send(Message::Text(reply.to_string().into()))
          .await
          .is_err()
        {
          break;
        }

        // The browser died mid-navigation: drop the socket without a close
        // frame, the way a relay does when the VM it bridges goes away. The
        // command's reply is already in the client's hands.
        if behaviour == RelayBehaviour::DropAfterNavigateReply && method == "Page.navigate" {
          break;
        }

        // A real browser follows a navigation with the load event, flattened
        // onto the same socket. Emitting it here is what proves the wait
        // actually terminates on the event rather than on its timeout.
        if behaviour != RelayBehaviour::NeverLoadEvent && method == "Page.navigate" {
          let loaded = serde_json::json!({
            "method": "Page.loadEventFired",
            "sessionId": request.get("sessionId").cloned().unwrap_or(Value::Null),
            "params": { "timestamp": 1.0 }
          });
          if stream
            .send(Message::Text(loaded.to_string().into()))
            .await
            .is_err()
          {
            break;
          }
        }
      }
      log
    });

    (format!("ws://127.0.0.1:{port}"), handle)
  }

  #[tokio::test]
  async fn a_relayed_page_command_is_attached_and_stamped_with_its_session() {
    // This is the whole feature. The relay bridges /devtools/browser/<id>, so
    // without the flat attach and the sessionId stamp every existing tool
    // answers "'Page.navigate' wasn't found" and a paid remote session cannot
    // be used for anything.
    let (ws_url, server) = fake_relay(RelayBehaviour::Cooperative).await;
    let target = CdpTarget::Remote {
      ws_url,
      bearer: "user-access-token".to_string(),
      session_id: "sess-1".to_string(),
    };

    let result = run_command(
      &target,
      "Page.navigate",
      serde_json::json!({ "url": "https://example.com" }),
    )
    .await
    .expect("a relayed navigate must succeed");

    assert_eq!(result["echo"]["method"], "Page.navigate");
    assert_eq!(result["echo"]["sessionId"], FAKE_CDP_SESSION);
    assert_eq!(result["echo"]["params"]["url"], "https://example.com");

    let log = server.await.expect("the fake relay must finish");
    assert_eq!(
      log.authorization.as_deref(),
      Some("Bearer user-access-token")
    );
    let methods: Vec<&str> = log
      .received
      .iter()
      .filter_map(|m| m.get("method").and_then(Value::as_str))
      .collect();
    assert_eq!(
      methods,
      vec![
        "Target.getTargets",
        "Target.attachToTarget",
        "Page.navigate"
      ]
    );
    // The handshake runs on the BROWSER session and must not be addressed to a
    // page, or the browser answers it with "no such session".
    assert!(log.received[0].get("sessionId").is_none());
    assert!(log.received[1].get("sessionId").is_none());
  }

  #[tokio::test]
  async fn a_relayed_navigation_waits_for_the_page_to_load() {
    // The load wait shares one implementation with the local arm, so a
    // flattened event that failed to match here would strand every navigate
    // for its full timeout.
    let (ws_url, server) = fake_relay(RelayBehaviour::Cooperative).await;
    let target = CdpTarget::Remote {
      ws_url,
      bearer: "t".to_string(),
      session_id: "sess-1".to_string(),
    };

    let started = std::time::Instant::now();
    navigate(&target, "https://example.com", 30)
      .await
      .expect("a relayed navigate must resolve");
    // It must return on the load event, not by outliving the timeout: a client
    // that always waits the full budget turns every navigation into a stall.
    assert!(
      started.elapsed() < Duration::from_secs(10),
      "navigate waited out its timeout instead of matching the load event"
    );

    let log = server.await.expect("the fake relay must finish");
    let methods: Vec<&str> = log
      .received
      .iter()
      .filter_map(|m| m.get("method").and_then(Value::as_str))
      .collect();
    assert!(methods.contains(&"Page.enable"));
    assert!(methods.contains(&"Page.navigate"));
    // Every page-domain message must carry the session, not just the command.
    for message in &log.received {
      let method = message.get("method").and_then(Value::as_str).unwrap_or("");
      if method.starts_with("Page.") {
        assert_eq!(
          message.get("sessionId").and_then(Value::as_str),
          Some(FAKE_CDP_SESSION),
          "{method} was not addressed to the attached page"
        );
      }
    }
  }

  #[tokio::test]
  async fn a_session_that_is_not_up_yet_is_reported_as_such_not_as_a_broken_one() {
    // 1013 is the relay saying "come back when it is live". Surfacing it as a
    // transport failure would send an automation client into a retry loop
    // against a session that is doing exactly what it should.
    let (ws_url, _server) = fake_relay(RelayBehaviour::RefuseAsNotDrivable).await;
    let target = CdpTarget::Remote {
      ws_url,
      bearer: "t".to_string(),
      session_id: "sess-1".to_string(),
    };

    let error = run_command(&target, "Page.navigate", serde_json::json!({}))
      .await
      .expect_err("a refused session must not look like a success");
    assert!(
      matches!(error, CdpError::NotDrivable(_)),
      "expected NotDrivable, got {error:?}"
    );
  }

  #[tokio::test]
  async fn a_local_command_skips_the_attach_and_carries_no_session() {
    // The local arm talks to a PAGE socket. Sending it a sessionId, or making
    // it pay for an attach handshake it does not need, would be a regression
    // in the path that already worked.
    let (ws_url, server) = fake_relay(RelayBehaviour::Cooperative).await;
    let target = CdpTarget::Local { ws_url };

    let result = run_command(
      &target,
      "Runtime.evaluate",
      serde_json::json!({ "expression": "1" }),
    )
    .await
    .expect("a local command must succeed");
    assert_eq!(result["echo"]["method"], "Runtime.evaluate");

    let log = server.await.expect("the fake relay must finish");
    let methods: Vec<&str> = log
      .received
      .iter()
      .filter_map(|m| m.get("method").and_then(Value::as_str))
      .collect();
    assert_eq!(methods, vec!["Runtime.evaluate"]);
    assert!(log.received[0].get("sessionId").is_none());
  }

  #[tokio::test]
  async fn a_navigation_result_survives_a_connection_dropped_before_the_load_event() {
    // The race this fix targets: a fast navigation can kill the connection
    // between the command reply and the load event. The browser already
    // accepted the navigation, so the reply is the answer — losing the socket
    // afterwards must not turn the answered command into a failure.
    let (ws_url, server) = fake_relay(RelayBehaviour::DropAfterNavigateReply).await;
    let target = CdpTarget::Local { ws_url };

    navigate(&target, "https://example.com", 30)
      .await
      .expect("an answered navigation must survive a dropped connection");

    let _log = server.await.expect("the fake relay must finish");
  }

  #[tokio::test]
  async fn a_connection_dropped_before_the_reply_is_still_reported_as_a_failure() {
    // The flip side of the race: if the socket dies before the command
    // answered, there is nothing to prefer — the navigation may never have
    // happened, so the call must still fail.
    let (ws_url, server) = fake_relay(RelayBehaviour::DropBeforeNavigateReply).await;
    let target = CdpTarget::Local { ws_url };

    let error = navigate(&target, "https://example.com", 30)
      .await
      .expect_err("a navigation that never answered must not look like a success");
    assert!(
      matches!(error, CdpError::Transport(_)),
      "expected Transport, got {error:?}"
    );

    let _log = server.await.expect("the fake relay must finish");
  }

  #[tokio::test]
  async fn a_navigation_error_from_the_browser_is_still_reported() {
    // A CDP error object is the browser saying no — that answer must not be
    // swallowed by the load wait.
    let (ws_url, server) = fake_relay(RelayBehaviour::AnswerNavigateWithError).await;
    let target = CdpTarget::Local { ws_url };

    let error = navigate(&target, "https://example.com", 30)
      .await
      .expect_err("a CDP error reply must surface as a failure");
    assert!(
      matches!(error, CdpError::Protocol(_)),
      "expected Protocol, got {error:?}"
    );

    let _log = server.await.expect("the fake relay must finish");
  }

  #[tokio::test]
  async fn a_navigation_without_a_load_event_returns_the_command_result() {
    // When nothing navigates there is no load event; the wait expires and the
    // command's own result is the answer, not a failure.
    let (ws_url, server) = fake_relay(RelayBehaviour::NeverLoadEvent).await;
    let target = CdpTarget::Local { ws_url };

    let started = std::time::Instant::now();
    navigate(&target, "https://example.com", 1)
      .await
      .expect("an answered command with no load event must still resolve");
    assert!(
      started.elapsed() >= Duration::from_secs(1),
      "the wait must run to its deadline when no load event arrives"
    );

    let _log = server.await.expect("the fake relay must finish");
  }
}

use axum::{
  body::Body,
  extract::State,
  http::{header, Request, StatusCode},
  middleware::Next,
  response::{IntoResponse, Response},
  routing::get,
  Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tokio::net::TcpListener;
use tokio::sync::Mutex as AsyncMutex;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::browser::ProxySettings;
use crate::cdp_target::{CdpError, CdpTarget};
use crate::cloud_auth::CLOUD_AUTH;
use crate::group_manager::GROUP_MANAGER;
use crate::log_redaction::ShortId;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::PROXY_MANAGER;
use crate::settings_manager::SettingsManager;
use crate::wayfern_cdp::{
  self, vellum, Engine, Extraction, ExtractionRequest, LocatorCandidate, LocatorDescription,
  LocatorResolution, PerceptionFrame, PerceptionNode, PerceptionPage, PerceptionRequest,
  PerceptionStats, PickedElement, ResolveOptions, ViewportTarget, WayfernError, WayfernSession,
};
use crate::wayfern_terms::WayfernTermsManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
  pub name: String,
  pub description: String,
  pub input_schema: serde_json::Value,
}

/// JavaScript executed in the target page to enumerate visible interactive
/// elements. Returns a JSON string `{elements, count, truncated}` where
/// `elements` is the newline-joined labeled list. Live references are stashed
/// on a per-caller `window[...]` slot so subsequent `click_by_index` /
/// `type_by_index` calls can resolve `index → Element` without round-tripping
/// a selector. `__MAX_CHARS__`, `__CACHE__`, `__REGISTRY__` and `__MAX_SLOTS__`
/// are substituted at call time.
const INTERACTIVE_ELEMENTS_JS: &str = r#"(() => {
  const SELECTORS = 'a, button, input, select, textarea, [role="button"], [role="link"], [role="checkbox"], [role="radio"], [role="tab"], [role="menuitem"], [role="combobox"], [role="option"], [contenteditable=""], [contenteditable="true"], [tabindex]:not([tabindex="-1"])';
  const ATTRS = ['type','name','id','role','aria-label','aria-checked','aria-expanded','placeholder','title','value','href','alt'];
  const MAX_CHARS = __MAX_CHARS__;
  const interactive = [];
  const lines = [];
  let truncated = false;
  let total = 0;
  const nodes = document.querySelectorAll(SELECTORS);
  for (const el of nodes) {
    if (el.disabled) continue;
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) continue;
    const style = window.getComputedStyle(el);
    if (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0') continue;
    const tag = el.tagName.toLowerCase();
    const parts = [];
    for (const a of ATTRS) {
      const v = el.getAttribute(a);
      if (v) parts.push(a + '="' + String(v).slice(0,100).replace(/"/g,'\\"') + '"');
    }
    let text = '';
    if (!['INPUT','TEXTAREA','SELECT'].includes(el.tagName)) {
      text = (el.innerText || el.textContent || '').trim().replace(/\s+/g,' ').slice(0,100);
    }
    const idx = interactive.length;
    const line = '[' + idx + ']<' + tag + (parts.length ? ' ' + parts.join(' ') : '') + '>' + text + '</' + tag + '>';
    if (total + line.length + 1 > MAX_CHARS) { truncated = true; break; }
    total += line.length + 1;
    interactive.push(el);
    lines.push(line);
  }
  window[__CACHE__] = interactive;
  // Bound how many snapshots one page carries. Each slot holds live element
  // references, so it pins every node in it, including nodes the page has
  // since detached, for as long as the tab lives, and NOTHING on the page
  // ever hears that a session ended. `end_session` deletes a session's own
  // slot, but no first-party client sends one, and an evicted or crashed
  // session never will; without this cap a long-lived tab accumulates one
  // array per session that ever listed it. Evicting the least recently
  // written slot is safe in the way that matters: the evicted caller's next
  // click_by_index finds no array and is told to re-list, which is an error,
  // not a click on the wrong element.
  try {
    const registry = Array.isArray(window[__REGISTRY__]) ? window[__REGISTRY__] : [];
    const kept = registry.filter((slot) => typeof slot === 'string' && slot !== __CACHE__ && slot in window);
    kept.push(__CACHE__);
    while (kept.length > __MAX_SLOTS__) {
      const evicted = kept.shift();
      try { delete window[evicted]; } catch (e) { window[evicted] = undefined; }
    }
    window[__REGISTRY__] = kept;
  } catch (e) {
    // A page that has made the registry unwritable only keeps its own slots
    // alive; it must not cost the caller the listing it asked for.
  }
  return JSON.stringify({ elements: lines.join('\n'), count: interactive.length, truncated: truncated });
})()"#;

/// Page global naming, in write order, every interactive-element slot the page
/// currently holds. Read and rewritten by the enumeration script so the oldest
/// slot can be evicted once [`MAX_CACHE_SLOTS_PER_PAGE`] is exceeded.
///
/// A quoted JS string literal, because it is substituted into `window[...]`.
const INTERACTIVE_SLOT_REGISTRY: &str = "'__donut_interactive_slots'";

/// How many interactive-element snapshots one page keeps at once.
///
/// Far above what any real page needs, a caller has one live snapshot, and
/// the handful of callers driving one page at the same moment are a browser
/// automation edge case already, and low enough that a tab open for a day
/// cannot collect hundreds of arrays of detached DOM nodes.
const MAX_CACHE_SLOTS_PER_PAGE: usize = 8;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct McpRequest {
  jsonrpc: String,
  id: Option<serde_json::Value>,
  method: String,
  params: Option<serde_json::Value>,
}

const PROTOCOL_VERSION: &str = "2025-11-25";

/// Every MCP protocol revision this engine can speak, newest first.
///
/// The spec says a server MUST echo the client's requested version when it
/// supports it. Answering with our own newest regardless meant a client on an
/// older SDK compared the reply against ITS supported list, found nothing, and
/// threw "Server's protocol version is not supported", so the whole feature
/// was unreachable from any agent that had not upgraded in lockstep.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
  &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The version to answer `initialize` with: the client's, when we speak it.
fn negotiate_protocol_version(requested: Option<&str>) -> &'static str {
  match requested {
    Some(asked) => SUPPORTED_PROTOCOL_VERSIONS
      .iter()
      .find(|known| **known == asked)
      .copied()
      // Not a version we know. The spec's fallback is to answer with one we
      // do support and let the client decide whether it can proceed.
      .unwrap_or(PROTOCOL_VERSION),
    None => PROTOCOL_VERSION,
  }
}
const SERVER_NAME: &str = "donut-browser";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize)]
pub struct McpResponse {
  jsonrpc: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  id: Option<serde_json::Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  result: Option<serde_json::Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  error: Option<McpError>,
}

#[derive(Debug, Serialize)]
pub struct McpError {
  code: i32,
  message: String,
  /// Structured detail for errors an agent has to act on, such as the
  /// candidate list behind an ambiguous locator. Absent for the rest, so the
  /// wire shape of every existing error is unchanged.
  #[serde(skip_serializing_if = "Option::is_none")]
  data: Option<serde_json::Value>,
}

/// Surface a CDP failure to the agent with the reason intact.
///
/// The distinction matters to whoever is on the other end: "the session is
/// still provisioning" invites a retry in a few seconds, "you are signed out"
/// does not, and flattening both into `-32000: something went wrong` is how an
/// automation client ends up retrying a refusal forever.
fn cdp_error(error: CdpError) -> McpError {
  McpError {
    code: -32000,
    message: error.to_string(),
    data: None,
  }
}

const DEFAULT_MCP_PORT: u16 = 51080;

/// The event the desktop turns into the "local MCP is being removed" dialog.
///
/// Emitted by the loopback tombstone (below) when anything still tries to reach
/// the removed local server, and by the enable/install commands when the user
/// asks for local MCP in the app.
pub const LOCAL_MCP_DEPRECATED_EVENT: &str = "mcp-local-deprecated";

/// Unix-seconds of the last deprecation event, so a client retry loop hitting
/// the dead port cannot pop the dialog on every attempt.
static LAST_LOCAL_DEPRECATION_EMIT: AtomicU64 = AtomicU64::new(0);

/// Seconds between deprecation dialogs, however many requests arrive.
const LOCAL_DEPRECATION_THROTTLE_SECS: u64 = 30;

/// How long a keystroke waits for its acknowledgement before moving on.
///
/// Generous enough to absorb a relayed round trip, short enough that a browser
/// which stops answering does not leave the caller typing into a socket that
/// will never reply.
const KEYSTROKE_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

struct McpSession {
  initialized: bool,
  /// When this session was last USED, which is what the cap evicts by.
  ///
  /// Not creation time: a long-lived agent's session is by definition the
  /// oldest, so evicting by age threw out the one client actually working and
  /// kept 512 abandoned ones from browser tabs that were closed hours ago.
  last_used: std::time::Instant,
  /// Every `(profile_id, slot)` this session has written an interactive-element
  /// snapshot to, so `end_session` can delete the page globals it left behind.
  ///
  /// Server-side state alone was never the whole session: the snapshot lives in
  /// somebody's still-open tab, holds live element references, and outlives the
  /// session that made it. Forgetting the session without deleting the array
  /// leaks one array per session for the life of the page.
  cached_pages: HashSet<(String, String)>,
}

/// How many MCP sessions one desktop will hold at once.
///
/// The map had no bound at all: nothing evicts a session except an explicit
/// `end_session`, and no first-party client sends one, the website mints a
/// session per page load in which the customer actually clicks something, and
/// the one-shot 404 retry abandons an id and mints a replacement. That is tens
/// of kilobytes over months rather than a leak with teeth, but "grows for the
/// life of the process with no ceiling" is not a property worth keeping when a
/// bound costs this little. Far above any real client's usage, so hitting it
/// means something is wrong rather than someone is busy.
const MAX_SESSIONS: usize = 512;

struct McpServerInner {
  app_handle: Option<AppHandle>,
  token: Option<String>,
  shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
  sessions: HashMap<String, McpSession>,
}

#[derive(Clone)]
struct McpHttpState {
  server: &'static McpServer,
  token: String,
}

/// What answering one JSON-RPC message produced, in transport-neutral terms.
///
/// The engine has two front doors, the loopback HTTP listener and the cloud
/// bridge in [`crate::mcp_remote`], and neither may own a rule the other
/// needs. Session validation, the notification path and the automation limiter
/// used to live inside the axum handler, which meant a second transport either
/// duplicated them or silently skipped them. They live in
/// [`McpServer::handle_message`] now, and each transport only translates this
/// enum into whatever "no such session" means on its own wire.
pub(crate) enum McpOutcome {
  /// A JSON-RPC response to send back. `new_session_id` is set only by
  /// `initialize`, and is the id the caller must echo on later messages.
  Body {
    body: serde_json::Value,
    new_session_id: Option<String>,
  },
  /// A notification was accepted. JSON-RPC defines no reply to one.
  Accepted,
  /// The caller named a session this process does not have.
  UnknownSession,
  /// The payload was not a JSON-RPC message.
  BadRequest,
  /// The shared automation limiter refused the call.
  RateLimited { retry_after_secs: u64 },
}

pub struct McpServer {
  inner: Arc<AsyncMutex<McpServerInner>>,
  is_running: AtomicBool,
  /// Whether the tool engine can answer at all, which is a different question
  /// from whether the loopback listener is bound.
  ///
  /// Remote control is usable without opening a local port, and turning the
  /// local server off must not sever a live bridge, so the engine's readiness
  /// tracks the app handle it needs, not the HTTP transport it no longer
  /// exclusively serves.
  engine_ready: AtomicBool,
  port: AtomicU16,
}

/// Which transport a message arrived on.
///
/// The two are the same engine and almost the same trust, but not quite: a
/// LOOPBACK caller is already on the machine, while a BRIDGE caller reached it
/// from the internet with an account credential. Tools that take a local
/// filesystem PATH are meaningful only to someone standing on the machine -
/// and `add_extension` reads that path and stores the bytes, which the sync
/// engine then uploads. Left open, that is an arbitrary local-file read with
/// the same shape as the `file://` hole the URL allowlist closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum McpOrigin {
  /// The 127.0.0.1 listener: the caller is already on this machine.
  Loopback,
  /// Relayed from the cloud bridge.
  Bridge,
}

/// Who is asking: the transport that carried the message, and the MCP session
/// it belongs to.
///
/// The two travel together because either one alone identifies a caller wrongly.
/// Origin decides what a caller is ALLOWED to do (a local-path tool is
/// meaningless to a remote caller); the session decides whose page state a
/// caller is looking at, and two callers routinely share one transport.
#[derive(Clone, Copy)]
pub(crate) struct McpCaller<'a> {
  origin: McpOrigin,
  /// The session id echoed back by the caller, when it opened one at all.
  session: Option<&'a str>,
}

/// The longest a single `type_text` call may plan to spend typing, on the
/// loopback transport.
///
/// Five minutes is far past any real form field (roughly 2,000 words at the
/// default rate) and far short of the hours an unbounded string can reach.
pub(crate) const MAX_TYPING_SECONDS: f64 = 300.0;

/// The same bound over the bridge.
///
/// A call over the bridge is answered with a timeout if it runs too long, so a
/// plan that would type for longer than that is not slow, it is a guaranteed
/// failure that still holds one of the eight process-wide permits for the full
/// five minutes. Sitting under that budget leaves room for the focus step, the
/// round trips and the answer itself.
const MAX_BRIDGE_TYPING_SECONDS: f64 = 80.0;

/// The typing budget for a caller on the given transport.
fn max_typing_seconds(origin: McpOrigin) -> f64 {
  match origin {
    McpOrigin::Loopback => MAX_TYPING_SECONDS,
    McpOrigin::Bridge => MAX_BRIDGE_TYPING_SECONDS,
  }
}

/// The longest text `type_text` will even PLAN, checked before planning starts.
///
/// `MarkovTyper::run` is superlinear, so the duration bound alone was not a
/// bound at all: the plan for a megabyte of text costs minutes of CPU to build,
/// and the refusal only arrives afterwards.
const MAX_TYPING_CHARS: usize = 4096;

/// Build the keystroke plan for `type_text`, or refuse it as too long.
///
/// Refused up front rather than typed for hours. Human typing sleeps between
/// keystrokes, `session_wpm` is floored at 10 (human_typing.rs), and the text is
/// bounded only by the 1 MiB frame cap, so at the floor a 10,000 character
/// string types for over three hours and a megabyte for a fortnight. Over the
/// bridge such a call holds one of eight PROCESS-WIDE permits the whole time, so
/// eight of them wedge remote control for the account, and the caller cannot
/// tell that apart from an outage.
///
/// Measured on the generated plan rather than the character count, so the bound
/// means the same thing at any wpm. Refusing beats truncating: a half-typed form
/// field is worse than an error that says what to do.
///
/// Separated from the sending loop so the decision can be tested for what it
/// DOES, not merely asserted to be present in the source.
fn plan_typing(
  text: &str,
  wpm: Option<f64>,
  max_seconds: f64,
) -> Result<Vec<crate::human_typing::TypingEvent>, McpError> {
  // Length FIRST, before a plan is built. `MarkovTyper::run` is superlinear -
  // planning 20,000 characters took a single unit test 24 seconds of solid CPU -
  // so a duration bound measured on the finished plan is defeated by the work of
  // producing it: the caller still burns minutes of CPU, and over the bridge
  // still holds one of eight process-wide permits, before being told no.
  //
  // 4,096 characters is far past any real form field and cheap to plan. The
  // duration bound below still applies, because a slow wpm can exceed the time
  // limit well under this many characters.
  let chars = text.chars().count();
  if chars > MAX_TYPING_CHARS {
    return Err(McpError {
      code: -32602,
      message: serde_json::json!({
        "code": "TYPING_TOO_LONG",
        "params": {
          "seconds": format!("{chars}"),
          "limit": format!("{MAX_TYPING_CHARS}"),
        }
      })
      .to_string(),
      data: None,
    });
  }

  let events = crate::human_typing::MarkovTyper::new(text, wpm).run();
  let planned = events.last().map_or(0.0, |event| event.time);
  if planned > max_seconds {
    return Err(McpError {
      code: -32602,
      message: serde_json::json!({
        "code": "TYPING_TOO_LONG",
        "params": {
          "seconds": format!("{planned:.0}"),
          "limit": format!("{max_seconds:.0}"),
        }
      })
      .to_string(),
      data: None,
    });
  }
  Ok(events)
}

/// The page-global slot one SESSION caches its interactive-element snapshot in.
///
/// Keyed by transport AND by MCP session. One slot per origin was not enough:
/// the website console and an agent both arrive over the BRIDGE, as do two runs
/// of the same agent, so one caller's `get_interactive_elements` overwrote the
/// array another had just built and that caller's next `click_by_index(3)`
/// resolved against the wrong array, a wrong click on somebody's real browser,
/// reported as a successful one. Sessions are what tell two callers on one
/// transport apart, so the slot is keyed by both.
///
/// The session is not an `Option` here on purpose. A caller that never ran
/// `initialize` used to fall back to the literal `"anon"`, which is a SHARED
/// slot by another name: two such callers on one transport still overwrote each
/// other's array and still clicked each other's elements. Taking a `&str` means
/// no caller can reach a slot without a session at all -
/// [`require_indexed_session`] is the only way to get one, and it refuses.
///
/// A quoted JS string literal, because it is substituted into `window[...]`.
fn interactive_cache_slot(origin: McpOrigin, session: &str) -> String {
  let transport = match origin {
    McpOrigin::Loopback => "local",
    McpOrigin::Bridge => "bridge",
  };
  format!(
    "'__donut_interactive_{transport}_{}'",
    cache_key_for_session(session)
  )
}

/// Reduce a session id to a distinct, JS-safe fragment of a global's name.
///
/// Hashed rather than truncated. Replacing every character outside
/// `[A-Za-z0-9_]` and cutting at 64 was safe but NOT distinct, whatever the doc
/// claimed: `"abc-def"` and `"abc_def"` mapped to one slot, as did any two ids
/// agreeing on their first 64 characters, and a slot collision is the wrong
/// click this key exists to prevent. A hash of the WHOLE id collides only at
/// the 128-bit birthday bound, and hex is inherently safe to paste inside a JS
/// string literal, so nothing caller-supplied survives to close the quote,
/// inject an expression, or grow the evaluated script.
///
/// Deterministic: the same id yields the same slot for the life of the session,
/// which is what lets a second `click_by_index` find the array the first
/// `get_interactive_elements` built.
fn cache_key_for_session(session: &str) -> String {
  let mut digest = blake3::hash(session.as_bytes()).to_hex().to_string();
  // 128 bits of it. The full 256 would only make the global's name longer.
  digest.truncate(32);
  digest
}

/// The session an index-based tool needs, or a refusal explaining why.
///
/// Refusing is the only answer that cannot click the wrong element. The
/// alternative considered, minting a per-connection identity for a caller that
/// skipped `initialize`, keeps such a caller working, but every scheme for it
/// either shares a slot between two connections (the bug again) or invents an
/// identity the caller cannot name on its NEXT request, so its `click_by_index`
/// resolves against a slot it does not own. An index is meaningless without a
/// session anyway: it is a pointer into an array a PREVIOUS call left on the
/// page, and only the session says whose array that is. An error the agent can
/// act on ("call initialize") beats a click on somebody's real browser reported
/// as a success.
fn require_indexed_session(caller: McpCaller<'_>) -> Result<&str, McpError> {
  caller.session.ok_or_else(|| McpError {
    code: -32600,
    message: "This tool needs an MCP session: call initialize and send the session id it \
              returns on every later request. An element index only means something inside \
              the session whose get_interactive_elements produced it."
      .to_string(),
    data: None,
  })
}

/// Tools that take a caller-supplied filesystem path.
///
/// Refused over the bridge rather than sanitised: a remote caller cannot know
/// this machine's filesystem, so there is no legitimate remote use to preserve
///, which makes refusing strictly better than guessing at safe roots.
const LOCAL_PATH_TOOLS: &[&str] = &[
  "add_extension",
  "update_extension",
  "detect_browser_profiles",
  // Its path arrives nested inside `items[].source_path` rather than as a
  // top-level argument, which is exactly how it stayed off this list: the
  // regression test used to grep each handler for the literal "path" and a
  // deserialized struct field has no such literal. It reads a caller-named
  // directory off this disk and copies it into a profile, including the
  // source browser's cookies and passwords, so over the bridge it is an
  // arbitrary local-file read with an account credential.
  "import_browser_profiles",
];

/// Tools refused over the bridge because their whole output is stored secrets.
///
/// The bridge gate is by tool, not by field. `export_proxies` exists to write
/// every proxy's password and VLESS URI out in full, which is exactly what the
/// redaction on `list_proxies` and `get_proxy` withholds from a remote caller,
/// so there is no redacted form of it worth serving. Refused with the same
/// code as the local-path tools: it is a local-only action.
const SECRET_EXPORT_TOOLS: &[&str] = &["export_proxies"];

/// Strip the fields of a stored proxy a remote caller must not see.
///
/// A bridge caller holds an account credential, not this machine. The proxy
/// password, the VLESS URI (which carries the credential inline) and the
/// dynamic list URL (which usually carries an API key) are secrets of the
/// machine, not of the account, and an agent needs none of them to pick a
/// proxy by id. Present values are replaced rather than removed so the shape
/// an agent has learned stays the same, and absent ones stay absent.
fn redact_proxy_secrets(proxy: &mut serde_json::Value) {
  const REDACTED: &str = "[redacted]";
  if let Some(settings) = proxy
    .get_mut("proxy_settings")
    .and_then(serde_json::Value::as_object_mut)
  {
    for field in ["password", "vless_uri"] {
      if settings.get(field).is_some_and(|v| !v.is_null()) {
        settings.insert(field.to_string(), serde_json::Value::from(REDACTED));
      }
    }
  }
  if let Some(map) = proxy.as_object_mut() {
    if map.get("dynamic_proxy_url").is_some_and(|v| !v.is_null()) {
      map.insert(
        "dynamic_proxy_url".to_string(),
        serde_json::Value::from(REDACTED),
      );
    }
  }
}

/// Reject a URL the browser should never be told to open on a customer's behalf.
///
/// `Page.navigate` will happily load `file:///Users/…/.ssh/id_rsa`, and
/// `get_page_content` will then hand the bytes straight back to the caller. On
/// the loopback transport that is merely a local tool reading local files; over
/// the CLOUD BRIDGE it is an arbitrary local-file read reachable from the
/// internet by anyone holding an account credential, which is not "control
/// your browser", and is exactly the machine this module exists to protect.
///
/// Allowed: the schemes a browser is actually asked to browse. `about:blank`
/// is permitted because it is the standard way to park a tab. Everything else,
/// including `file:`, `data:`, `blob:`, `chrome:`, `devtools:` and
/// `javascript:`, is refused.
pub(crate) fn is_navigable_url(url: &str) -> bool {
  let trimmed = url.trim();
  if trimmed.eq_ignore_ascii_case("about:blank") {
    return true;
  }
  let lowered = trimmed.to_ascii_lowercase();
  lowered.starts_with("http://") || lowered.starts_with("https://")
}

fn validate_navigable_url(url: &str) -> Result<(), McpError> {
  if is_navigable_url(url) {
    return Ok(());
  }
  Err(McpError {
    code: -32602,
    message: crate::backend_error("URL_SCHEME_NOT_ALLOWED"),
    data: None,
  })
}

// --- Agent surface: perception, locators, extraction, picker, humanized input --

/// The longest `pick_element` waits for a click on the loopback transport.
pub(crate) const MAX_PICK_TIMEOUT_MS: u64 = 300_000;

/// How long `pick_element` waits when the caller does not say.
pub(crate) const DEFAULT_PICK_TIMEOUT_MS: u64 = 60_000;

/// The same bound over the bridge.
///
/// A call over the bridge is answered with a timeout if it runs too long, so a
/// wait that would outlive it is not patience, it is a guaranteed failure that
/// still holds one of the eight process-wide permits. Same margin as typing.
const MAX_BRIDGE_PICK_TIMEOUT_MS: u64 = 80_000;

/// The picker budget for a caller on the given transport.
fn max_pick_timeout_ms(origin: McpOrigin) -> u64 {
  match origin {
    McpOrigin::Loopback => MAX_PICK_TIMEOUT_MS,
    McpOrigin::Bridge => MAX_BRIDGE_PICK_TIMEOUT_MS,
  }
}

/// The longest `extract_structured` may let the browser page through rows.
///
/// The browser's own ceiling is two minutes; over the bridge that is longer
/// than a call is allowed to run, so the same margin as typing applies.
const MAX_EXTRACTION_BUDGET_MS: u64 = 120_000;

/// The extraction budget for a caller on the given transport.
fn max_extraction_budget_ms(origin: McpOrigin) -> u64 {
  match origin {
    McpOrigin::Loopback => MAX_EXTRACTION_BUDGET_MS,
    McpOrigin::Bridge => MAX_BRIDGE_PICK_TIMEOUT_MS,
  }
}

/// How long the browser's own typing rhythm is budgeted per character.
///
/// `Vellum.inscribe` paces the keys itself, so this is a per-character ceiling
/// rather than an estimate: a text the budget refuses is one the browser would
/// have turned into a hung call.
const VELLUM_SECONDS_PER_CHAR: f64 = 0.25;

/// How long a click waits for the page it may have navigated to.
const CLICK_LOAD_TIMEOUT: Duration = Duration::from_secs(10);

/// The `TYPING_TOO_LONG` envelope, shared by both typing engines.
fn typing_too_long(seconds: f64, limit: f64) -> McpError {
  McpError {
    code: -32602,
    message: serde_json::json!({
      "code": "TYPING_TOO_LONG",
      "params": {
        "seconds": format!("{seconds:.0}"),
        "limit": format!("{limit:.0}"),
      }
    })
    .to_string(),
    data: None,
  }
}

/// The time a `Vellum.inscribe` of `text` may take, or the refusal.
///
/// Decided BEFORE anything touches the page, for the same reason `plan_typing`
/// is: a refusal that arrives after the field was emptied is a lie.
fn vellum_typing_budget(text: &str, max_seconds: f64) -> Result<Duration, McpError> {
  let chars = text.chars().count();
  if chars > MAX_TYPING_CHARS {
    return Err(typing_too_long(chars as f64, MAX_TYPING_CHARS as f64));
  }
  let estimate = chars as f64 * VELLUM_SECONDS_PER_CHAR + 1.0;
  if estimate > max_seconds {
    return Err(typing_too_long(estimate, max_seconds));
  }
  Ok(Duration::from_secs_f64(max_seconds))
}

/// Why an agent operation could not be completed.
///
/// One vocabulary for both front doors. The MCP transport renders it as a
/// JSON-RPC error whose `data` carries the structured part, and the REST
/// transport maps the same variants onto statuses, so a locator that matched
/// three buttons is reported the same way whichever door it came through.
#[derive(Debug)]
pub(crate) enum AgentError {
  /// The caller's arguments cannot mean anything on any page.
  InvalidArgument(String),
  /// The profile runs a Wayfern older than the feature.
  RequiresWayfern152 {
    version: String,
  },
  /// The browser refused: no paid plan behind this browser.
  PaymentRequired(String),
  /// The browser refused: too many calls too fast.
  RateLimited(String),
  /// The browser could not confirm this account's plan.
  AuthorizationUnavailable(String),
  AmbiguousLocator {
    match_count: u64,
    candidates: Vec<serde_json::Value>,
    message: String,
  },
  NoMatch {
    message: String,
  },
  PickerTimedOut {
    timeout_ms: u64,
  },
  PickerCancelled {
    reason: String,
  },
  TypingTooLong {
    seconds: f64,
    limit: f64,
  },
  /// The browser refused the request as malformed (`-32602`), or the page
  /// threw at it: what was asked for does not exist or cannot be done.
  BadRequest(String),
  /// The browser failed to do what it was asked (`-32000`).
  Browser(String),
  /// The transport, or the browser's absence.
  Cdp(CdpError),
  /// A reply without the documented shape.
  Malformed(String),
}

impl AgentError {
  /// A stable code an agent can branch on.
  pub(crate) fn code(&self) -> &'static str {
    match self {
      Self::InvalidArgument(_) => "INVALID_ARGUMENT",
      Self::RequiresWayfern152 { .. } => "WAYFERN_152_REQUIRED",
      Self::PaymentRequired(_) => "PAYMENT_REQUIRED",
      Self::RateLimited(_) => "RATE_LIMITED",
      Self::AuthorizationUnavailable(_) => "AUTHORIZATION_UNAVAILABLE",
      Self::AmbiguousLocator { .. } => "LOCATOR_AMBIGUOUS",
      Self::NoMatch { .. } => "LOCATOR_NO_MATCH",
      Self::PickerTimedOut { .. } => "PICKER_TIMEOUT",
      Self::PickerCancelled { .. } => "PICKER_CANCELLED",
      Self::TypingTooLong { .. } => "TYPING_TOO_LONG",
      Self::BadRequest(_) => "BAD_REQUEST",
      Self::Browser(_) => "BROWSER_ERROR",
      Self::Cdp(_) => "CDP_ERROR",
      Self::Malformed(_) => "MALFORMED_REPLY",
    }
  }

  /// The human-readable part.
  pub(crate) fn message(&self) -> String {
    match self {
      Self::InvalidArgument(m)
      | Self::PaymentRequired(m)
      | Self::RateLimited(m)
      | Self::AuthorizationUnavailable(m)
      | Self::BadRequest(m)
      | Self::Browser(m)
      | Self::Malformed(m) => m.clone(),
      Self::RequiresWayfern152 { version } => format!(
        "This tool requires Wayfern 152 or newer; the profile runs {version}. Update the profile's browser, or use the selector and index tools."
      ),
      Self::AmbiguousLocator { message, .. } | Self::NoMatch { message } => message.clone(),
      Self::PickerTimedOut { timeout_ms } => {
        format!("No element was picked within {timeout_ms} ms; the picker has been disarmed")
      }
      Self::PickerCancelled { reason } => format!("The element picker was cancelled ({reason})"),
      Self::TypingTooLong { seconds, limit } => {
        format!("Typing this text would take about {seconds:.0}s, over the {limit:.0}s limit")
      }
      Self::Cdp(e) => e.to_string(),
    }
  }

  /// The structured part, when there is one.
  pub(crate) fn detail(&self) -> serde_json::Value {
    let mut detail = serde_json::json!({ "code": self.code() });
    match self {
      Self::AmbiguousLocator {
        match_count,
        candidates,
        ..
      } => {
        detail["matchCount"] = serde_json::Value::from(*match_count);
        detail["candidates"] = serde_json::Value::Array(candidates.clone());
      }
      Self::NoMatch { .. } => {
        detail["matchCount"] = serde_json::Value::from(0);
        detail["candidates"] = serde_json::json!([]);
      }
      Self::RequiresWayfern152 { version } => {
        detail["version"] = serde_json::Value::from(version.as_str());
      }
      Self::PickerTimedOut { timeout_ms } => {
        detail["timeoutMs"] = serde_json::Value::from(*timeout_ms);
      }
      Self::PickerCancelled { reason } => {
        detail["reason"] = serde_json::Value::from(reason.as_str());
      }
      Self::TypingTooLong { seconds, limit } => {
        detail["seconds"] = serde_json::Value::from(*seconds);
        detail["limit"] = serde_json::Value::from(*limit);
      }
      _ => {}
    }
    detail
  }

  /// As a JSON-RPC error. Argument problems are `-32602`; the rest is the
  /// same `-32000` every other browser tool answers with, with `data` telling
  /// the two apart.
  fn into_mcp(self) -> McpError {
    let code = match self {
      Self::InvalidArgument(_) | Self::TypingTooLong { .. } | Self::BadRequest(_) => -32602,
      _ => -32000,
    };
    let message = match &self {
      // The same envelope `plan_typing` answers with, so a client that already
      // understands one typing refusal understands both.
      Self::TypingTooLong { seconds, limit } => typing_too_long(*seconds, *limit).message,
      _ => self.message(),
    };
    McpError {
      code,
      message,
      data: Some(self.detail()),
    }
  }
}

impl From<CdpError> for AgentError {
  fn from(error: CdpError) -> Self {
    if let Some(refusal) = wayfern_cdp::classify_refusal(&error) {
      let message = wayfern_cdp::protocol_message(&error).unwrap_or_else(|| error.to_string());
      return match refusal {
        wayfern_cdp::BrowserRefusal::PaymentRequired => Self::PaymentRequired(message),
        wayfern_cdp::BrowserRefusal::RateLimited => Self::RateLimited(message),
        wayfern_cdp::BrowserRefusal::AuthorizationUnavailable => {
          Self::AuthorizationUnavailable(message)
        }
      };
    }
    match wayfern_cdp::protocol_code(&error) {
      Some(-32602) => {
        Self::BadRequest(wayfern_cdp::protocol_message(&error).unwrap_or_else(|| error.to_string()))
      }
      Some(_) => {
        Self::Browser(wayfern_cdp::protocol_message(&error).unwrap_or_else(|| error.to_string()))
      }
      None => Self::Cdp(error),
    }
  }
}

impl From<WayfernError> for AgentError {
  fn from(error: WayfernError) -> Self {
    match error {
      WayfernError::Cdp(e) => Self::from(e),
      WayfernError::AmbiguousLocator {
        match_count,
        candidates,
        message,
      } => Self::AmbiguousLocator {
        match_count,
        candidates,
        message,
      },
      WayfernError::NoMatch { message } => Self::NoMatch { message },
      WayfernError::PickerTimedOut { timeout_ms } => Self::PickerTimedOut { timeout_ms },
      WayfernError::PickerCancelled { reason } => Self::PickerCancelled { reason },
      WayfernError::Malformed(m) => Self::Malformed(m),
    }
  }
}

impl From<McpError> for AgentError {
  /// The typing planner answers in the `TYPING_TOO_LONG` envelope; anything
  /// else from that layer is a browser-side failure.
  fn from(error: McpError) -> Self {
    if let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&error.message) {
      if envelope.get("code").and_then(|c| c.as_str()) == Some("TYPING_TOO_LONG") {
        let number = |key: &str| {
          envelope["params"][key]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0)
        };
        return Self::TypingTooLong {
          seconds: number("seconds"),
          limit: number("limit"),
        };
      }
    }
    Self::Browser(error.message)
  }
}

/// A running browser, and the engine its version entitles it to.
pub(crate) struct AgentContext {
  pub profile: BrowserProfile,
  pub target: CdpTarget,
  pub engine: Engine,
}

impl AgentContext {
  /// The engine is read from the profile HERE, at the point of use, never
  /// cached: a profile updated to 152 gets the native path on its next call.
  pub(crate) fn new(profile: BrowserProfile, target: CdpTarget) -> Self {
    let engine = Engine::for_version(&profile.version);
    Self {
      profile,
      target,
      engine,
    }
  }

  fn requires_wayfern_152(&self) -> Result<(), AgentError> {
    if self.engine.is_wayfern() {
      Ok(())
    } else {
      Err(AgentError::RequiresWayfern152 {
        version: self.profile.version.clone(),
      })
    }
  }
}

/// The body of `resolve_locator`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(crate) struct AgentResolveRequest {
  pub locator: LocatorDescription,
  /// How many candidates an ambiguity error lists. Default 10, ceiling 100.
  #[serde(default, alias = "candidateLimit")]
  pub candidate_limit: Option<u64>,
}

/// The body of `click_locator`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(crate) struct AgentClickRequest {
  pub locator: LocatorDescription,
  /// "left" (default), "middle", "right", "back" or "forward".
  #[serde(default)]
  pub button: Option<String>,
  /// 1 (default), 2 for a double click, 3 for a triple.
  #[serde(default, alias = "clickCount")]
  pub click_count: Option<u32>,
}

/// What `click_locator` did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct AgentClick {
  pub clicked: bool,
  #[serde(rename = "match")]
  pub matched: LocatorCandidate,
  pub engine: Engine,
  /// Whether a page load followed the click.
  pub navigated: bool,
}

/// The body of `type_locator`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(crate) struct AgentTypeRequest {
  pub locator: LocatorDescription,
  pub text: String,
  /// Empty the field first. Default true.
  #[serde(default, alias = "clearFirst")]
  pub clear_first: Option<bool>,
  /// Mistype and correct a few characters, as a hand does. Default true.
  #[serde(default)]
  pub typos: Option<bool>,
  /// Target words per minute. Honoured by the fallback engine only: Wayfern
  /// 152 types at the profile's own rhythm.
  #[serde(default)]
  pub wpm: Option<f64>,
}

/// What `type_locator` did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct AgentTyping {
  pub typed: bool,
  /// Characters delivered.
  pub characters: u64,
  /// Mistyped characters that were corrected. Absent on the fallback engine,
  /// which does not count its own.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub corrections: Option<u64>,
  pub duration_ms: f64,
  pub engine: Engine,
  #[serde(rename = "match")]
  pub matched: LocatorCandidate,
}

/// The body of `pick_element`.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub(crate) struct AgentPickRequest {
  /// How long to wait for the click. Default 60000, ceiling 300000.
  #[serde(default, alias = "timeoutMs")]
  pub timeout_ms: Option<u64>,
}

/// Where a locator refers to, spelled the way the browser spells it in its
/// own error messages.
fn describe_locator(locator: &LocatorDescription) -> String {
  let mut parts = Vec::new();
  if let Some(role) = &locator.role {
    parts.push(format!("role={role}"));
  }
  if let Some(name) = &locator.name {
    parts.push(format!("name={name}"));
  }
  if let Some(name) = &locator.name_contains {
    parts.push(format!("nameContains={name}"));
  }
  if let Some(text) = &locator.text {
    parts.push(format!("text={text}"));
  }
  if let Some(text) = &locator.text_contains {
    parts.push(format!("textContains={text}"));
  }
  for attribute in locator.attributes.iter().flatten() {
    parts.push(format!("{}={}", attribute.name, attribute.value));
  }
  parts.join(", ")
}

/// ARIA role names for the Blink AX tokens the browser indexes.
///
/// The perception snapshot and the locator resolver speak Blink's own role
/// vocabulary (`textField`, `staticText`, `radioButton`), and so does the
/// fallback engine, so the two agree. An agent that has read ARIA writes
/// `textbox`. The browser matches roles case- and separator-insensitively, so
/// only the tokens that differ in substance are translated here, and only
/// where the ARIA role has exactly one Blink counterpart.
const ROLE_SYNONYMS: [(&str, &str); 7] = [
  ("textbox", "textField"),
  ("radio", "radioButton"),
  ("progressbar", "progressIndicator"),
  ("separator", "splitter"),
  ("generic", "genericContainer"),
  ("text", "staticText"),
  ("img", "image"),
];

/// The locator with its role spelled the way the browser spells it.
fn canonical_locator(locator: &LocatorDescription) -> LocatorDescription {
  let mut canonical = locator.clone();
  if let Some(role) = &locator.role {
    let key: String = role
      .chars()
      .filter(char::is_ascii_alphanumeric)
      .collect::<String>()
      .to_ascii_lowercase();
    if let Some((_, blink)) = ROLE_SYNONYMS.iter().find(|(aria, _)| *aria == key) {
      canonical.role = Some((*blink).to_string());
    }
  }
  canonical
}

fn validate_locator(locator: &LocatorDescription) -> Result<(), AgentError> {
  if locator.is_empty() {
    return Err(AgentError::InvalidArgument(
      "locator needs at least one of role, name, nameContains, text, textContains or attributes"
        .to_string(),
    ));
  }
  for attribute in locator.attributes.iter().flatten() {
    if attribute.name.trim().is_empty() {
      return Err(AgentError::InvalidArgument(
        "every locator attribute needs a name".to_string(),
      ));
    }
  }
  Ok(())
}

/// The mouse button names both engines accept.
const MOUSE_BUTTONS: [&str; 5] = ["left", "middle", "right", "back", "forward"];

fn validate_click(request: &AgentClickRequest) -> Result<(), AgentError> {
  validate_locator(&request.locator)?;
  if let Some(button) = request.button.as_deref() {
    if !MOUSE_BUTTONS.contains(&button) {
      return Err(AgentError::InvalidArgument(format!(
        "button must be one of {}",
        MOUSE_BUTTONS.join(", ")
      )));
    }
  }
  if let Some(count) = request.click_count {
    if !(1..=3).contains(&count) {
      return Err(AgentError::InvalidArgument(
        "click_count must be 1, 2 or 3".to_string(),
      ));
    }
  }
  Ok(())
}

fn validate_extraction(request: &ExtractionRequest) -> Result<(), AgentError> {
  validate_locator(&request.container)?;
  if request.field_map.is_empty() {
    return Err(AgentError::InvalidArgument(
      "field_map needs at least one field".to_string(),
    ));
  }
  for field in &request.field_map {
    if field.key.trim().is_empty() {
      return Err(AgentError::InvalidArgument(
        "every field needs a key".to_string(),
      ));
    }
    validate_locator(&field.locator)?;
    match field.source.as_str() {
      "text" | "link" => {}
      "attribute" => {
        if field
          .attribute
          .as_deref()
          .is_none_or(|a| a.trim().is_empty())
        {
          return Err(AgentError::InvalidArgument(format!(
            "field '{}' reads an attribute but names none",
            field.key
          )));
        }
      }
      other => {
        return Err(AgentError::InvalidArgument(format!(
          "field '{}' has source '{other}'; it must be text, attribute or link",
          field.key
        )))
      }
    }
  }
  if let Some(next) = &request.next_page {
    validate_locator(next)?;
  }
  Ok(())
}

/// Evaluate `expression` on a fresh connection and hand back the JSON string
/// it returned, or the exception it threw as a [`AgentError::BadRequest`].
///
/// Every fallback script returns `JSON.stringify(...)`, so one reader serves
/// them all.
async fn evaluate_json_script(
  target: &CdpTarget,
  expression: String,
) -> Result<serde_json::Value, AgentError> {
  let result = crate::cdp_target::run_command(
    target,
    "Runtime.evaluate",
    serde_json::json!({ "expression": expression, "returnByValue": true }),
  )
  .await?;
  parse_json_script_result(&result)
}

/// The same reader for a script evaluated on an existing session.
fn parse_json_script_result(result: &serde_json::Value) -> Result<serde_json::Value, AgentError> {
  if let Some(exception) = result.get("exceptionDetails") {
    let message = exception
      .get("exception")
      .and_then(|e| e.get("description"))
      .or_else(|| exception.get("text"))
      .and_then(|v| v.as_str())
      .unwrap_or("the page script failed");
    return Err(AgentError::BadRequest(message.to_string()));
  }
  let text = result
    .get("result")
    .and_then(|r| r.get("value"))
    .and_then(|v| v.as_str())
    .ok_or_else(|| AgentError::Malformed("the page script returned no value".to_string()))?;
  serde_json::from_str(text)
    .map_err(|e| AgentError::Malformed(format!("the page script returned invalid JSON: {e}")))
}

/// The pre-152 stand-in for the browser's perception and locator domains.
///
/// One script, two modes. `perceive` walks the visible DOM and describes it
/// in the SAME node shape `Wayfern.capturePagePerception` answers with, so an
/// agent written against 152 reads both. `resolve` applies a locator with the
/// browser's own semantics (role token, exact or substring name and text, every
/// attribute pair) and refuses ambiguity the same way; with `act` it also
/// scrolls to, or focuses and prepares, the one match, because the element
/// reference cannot leave the page and a second script could resolve a
/// different node. `__OPTS__` is substituted with a JSON object at call time.
const AGENT_FALLBACK_JS: &str = r#"(() => {
  const opts = __OPTS__;
  const now = () => (window.performance && performance.now) ? performance.now() : Date.now();
  const started = now();
  const norm = (s) => String(s == null ? '' : s).replace(/\s+/g, ' ').trim();
  const roleKey = (r) => norm(r).toLowerCase().replace(/[^a-z0-9]/g, '');
  // Blink's own AX role tokens, which is what Wayfern 152 answers with, so an
  // agent reads one vocabulary whichever engine answered.
  const INPUT_ROLES = { button: 'button', submit: 'button', reset: 'button', image: 'button', file: 'button', color: 'button', checkbox: 'checkBox', radio: 'radioButton', range: 'slider', number: 'spinButton', search: 'searchBox', hidden: 'none' };
  const TAG_ROLES = { a: 'link', area: 'link', button: 'button', select: 'comboBoxSelect', textarea: 'textField', img: 'image', h1: 'heading', h2: 'heading', h3: 'heading', h4: 'heading', h5: 'heading', h6: 'heading', nav: 'navigation', main: 'main', header: 'banner', footer: 'contentInfo', form: 'form', table: 'table', tr: 'row', td: 'cell', th: 'columnHeader', ul: 'list', ol: 'list', li: 'listItem', p: 'paragraph', option: 'listBoxOption', label: 'labelText', dialog: 'dialog', section: 'region', article: 'article', aside: 'complementary', summary: 'disclosureTriangle', details: 'details', fieldset: 'group', legend: 'legend', progress: 'progressIndicator', meter: 'meter', hr: 'splitter', blockquote: 'blockquote', code: 'code', em: 'emphasis', strong: 'strong', figure: 'figure', menu: 'menu', output: 'status', time: 'time', dd: 'definition', dt: 'term', dl: 'descriptionList', caption: 'caption', video: 'video', audio: 'audio', iframe: 'iframe' };
  const NAMED_FROM_CONTENT = new Set(['button', 'link', 'heading', 'tab', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'listboxoption', 'option', 'cell', 'columnheader', 'rowheader', 'gridcell', 'checkbox', 'radiobutton', 'radio', 'switch', 'treeitem', 'tooltip', 'legend', 'caption', 'labeltext', 'label', 'disclosuretriangle']);
  const ATTRS = ['id', 'name', 'type', 'role', 'href', 'src', 'placeholder', 'aria-label', 'title', 'alt', 'data-testid', 'data-test', 'data-id', 'for', 'value'];
  const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD', 'META', 'LINK', 'TITLE', 'BR', 'WBR']);
  const vw = window.innerWidth, vh = window.innerHeight;
  const sx = window.scrollX || 0, sy = window.scrollY || 0;
  const fnv = (s) => { let h = 0x811c9dc5; for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; } return ('00000000' + h.toString(16)).slice(-8); };
  const tagOf = (el) => String(el.tagName || '').toLowerCase();
  const pathOf = (el) => { const parts = []; let n = el; while (n && n.nodeType === 1 && n !== document.documentElement) { let i = 1, s = n; while ((s = s.previousElementSibling)) { if (s.tagName === n.tagName) i++; } parts.push(tagOf(n) + ':' + i); n = n.parentElement; } return parts.reverse().join('>'); };
  const roleOf = (el) => {
    const explicit = el.getAttribute('role');
    if (explicit && norm(explicit)) return norm(explicit).split(' ')[0];
    const tag = tagOf(el);
    if (tag === 'input') { const t = (el.getAttribute('type') || 'text').toLowerCase(); return INPUT_ROLES[t] || 'textField'; }
    if (tag === 'a' && !el.hasAttribute('href')) return 'genericContainer';
    if (el.isContentEditable && !TAG_ROLES[tag]) return 'textField';
    return TAG_ROLES[tag] || 'genericContainer';
  };
  const isProtected = (el) => tagOf(el) === 'input' && ((el.getAttribute('type') || '').toLowerCase() === 'password' || /(^|\s)(cc-|password|one-time-code)/i.test(el.getAttribute('autocomplete') || ''));
  const contentText = (el) => norm(el.innerText != null ? el.innerText : el.textContent);
  const nameOf = (el, role) => {
    const ids = el.getAttribute('aria-labelledby');
    if (ids) { const t = norm(ids.split(/\s+/).map((id) => { const n = document.getElementById(id); return n ? (n.innerText != null ? n.innerText : n.textContent) : ''; }).join(' ')); if (t) return t.slice(0, 300); }
    const aria = el.getAttribute('aria-label'); if (aria && norm(aria)) return norm(aria).slice(0, 300);
    if (el.labels && el.labels.length) { const t = norm(Array.from(el.labels).map((l) => l.innerText != null ? l.innerText : l.textContent).join(' ')); if (t) return t.slice(0, 300); }
    const tag = tagOf(el);
    if (tag === 'img' || tag === 'area') { const alt = el.getAttribute('alt'); if (alt != null && norm(alt)) return norm(alt).slice(0, 300); }
    if (tag === 'input') { const t = (el.getAttribute('type') || 'text').toLowerCase(); if (t === 'button' || t === 'submit' || t === 'reset') { const v = el.getAttribute('value'); if (v && norm(v)) return norm(v).slice(0, 300); if (t === 'submit') return 'Submit'; if (t === 'reset') return 'Reset'; } }
    const ph = el.getAttribute('placeholder'); if (ph && norm(ph)) return norm(ph).slice(0, 300);
    const title = el.getAttribute('title'); if (title && norm(title)) return norm(title).slice(0, 300);
    if (NAMED_FROM_CONTENT.has(roleKey(role)) || tag === 'summary') return contentText(el).slice(0, 300);
    return '';
  };
  const liveValue = (el) => { const tag = tagOf(el); if (tag === 'input' || tag === 'textarea') return String(el.value); if (tag === 'select') { const o = el.options[el.selectedIndex]; return o ? norm(o.text) : ''; } return null; };
  const attrsOf = (el) => { const out = []; for (const a of ATTRS) { if (a === 'value') { if (isProtected(el)) continue; const v = liveValue(el); if (v !== null) { if (norm(v)) out.push({ name: a, value: norm(v).slice(0, 200) }); continue; } } const v = el.getAttribute(a); if (v != null && norm(v)) out.push({ name: a, value: norm(v).slice(0, 200) }); } return out; };
  const attrValue = (el, name) => { if (name === 'value') { if (isProtected(el)) return null; const v = liveValue(el); if (v !== null) return v; } const v = el.getAttribute(name); return v == null ? null : v; };
  const urlOf = (el) => { const tag = tagOf(el); if (tag === 'a' || tag === 'area') return el.href || undefined; if (tag === 'img') return el.currentSrc || el.src || undefined; return undefined; };
  const visibleRect = (el) => { if (SKIP.has(String(el.tagName).toUpperCase())) return null; const style = window.getComputedStyle(el); if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return null; const r = el.getBoundingClientRect(); if (!(r.width > 0 && r.height > 0)) return null; return r; };
  const describe = (el, r) => {
    const role = roleOf(el); const name = nameOf(el, role);
    const out = { role, name, text: contentText(el).slice(0, 2000), signature: 'js-' + fnv(tagOf(el) + '|' + role + '|' + name + '|' + pathOf(el)), attributes: attrsOf(el), bounds: { x: r.left + sx, y: r.top + sy, width: r.width, height: r.height } };
    if (!isProtected(el)) { const v = liveValue(el); if (v !== null) out.value = v; }
    const u = urlOf(el); if (u) out.url = u;
    return out;
  };
  const clampCenter = (r) => { const left = Math.max(r.left, 0), top = Math.max(r.top, 0), right = Math.min(r.right, vw), bottom = Math.min(r.bottom, vh); if (right <= left || bottom <= top) return { x: r.left + r.width / 2, y: r.top + r.height / 2, width: r.width, height: r.height, visible: false }; return { x: left + (right - left) / 2, y: top + (bottom - top) / 2, width: right - left, height: bottom - top, visible: true }; };
  const all = document.querySelectorAll('*');

  if (opts.mode === 'resolve') {
    const L = opts.locator || {};
    const roleWanted = L.role != null ? roleKey(L.role) : null;
    const attrs = Array.isArray(L.attributes) ? L.attributes : [];
    const matches = [];
    for (const el of all) {
      const r = visibleRect(el); if (!r) continue;
      const role = roleOf(el);
      if (roleWanted !== null && roleKey(role) !== roleWanted) continue;
      if (L.name != null || L.nameContains != null) {
        const name = nameOf(el, role);
        if (L.name != null && name !== norm(L.name)) continue;
        if (L.nameContains != null && !name.includes(norm(L.nameContains))) continue;
      }
      if (L.text != null || L.textContains != null) {
        const text = contentText(el);
        if (L.text != null && text !== norm(L.text)) continue;
        if (L.textContains != null && !text.includes(norm(L.textContains))) continue;
      }
      let ok = true;
      for (const a of attrs) { if (!a || typeof a.name !== 'string' || attrValue(el, a.name) !== String(a.value)) { ok = false; break; } }
      if (!ok) continue;
      matches.push({ el, r });
    }
    // A wrapper matches a text locator only because its child does; the
    // innermost of nested matches is the element the caller meant.
    const kept = matches.filter((m) => !matches.some((o) => o !== m && m.el !== o.el && m.el.contains(o.el)));
    const limit = Math.max(1, Math.min(100, Number(opts.candidateLimit) || 10));
    const result = { matchCount: kept.length, candidates: kept.slice(0, limit).map((m) => describe(m.el, m.r)) };
    if (kept.length === 1) {
      const el = kept[0].el;
      if (opts.act === 'scroll' || opts.act === 'focus') {
        try { el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); } catch (e) { el.scrollIntoView(); }
      }
      if (opts.act === 'focus') {
        el.focus();
        const editable = !!el.isContentEditable; const tag = tagOf(el);
        if (opts.clearFirst) {
          if (editable) { el.textContent = ''; } else if (tag === 'input' || tag === 'textarea') { el.value = ''; }
          el.dispatchEvent(new Event('input', { bubbles: true }));
        } else if (editable) {
          const sel = window.getSelection(); if (sel) { sel.selectAllChildren(el); sel.collapseToEnd(); }
        } else if (typeof el.setSelectionRange === 'function') {
          try { const n = String(el.value || '').length; el.setSelectionRange(n, n); } catch (e) {}
        }
      }
      result.match = describe(el, el.getBoundingClientRect());
      result.center = clampCenter(el.getBoundingClientRect());
    }
    return JSON.stringify(result);
  }

  const maxNodes = Number(opts.maxNodes) > 0 ? Number(opts.maxNodes) : 100000;
  const maxBytes = Number(opts.maxBytes) > 0 ? Number(opts.maxBytes) : 1048576;
  const includeText = opts.includeText !== false;
  const ids = new Map();
  const scrollables = new Set();
  const nodes = [];
  let total = 0;
  let truncated = false;
  for (const el of all) {
    const r = visibleRect(el); if (!r) continue;
    total++;
    const role = roleOf(el);
    if (role === 'none' || role === 'presentation') continue;
    const own = norm(Array.from(el.childNodes).filter((n) => n.nodeType === 3).map((n) => n.nodeValue).join(' '));
    if (role === 'genericContainer' && !own) continue;
    const inViewport = r.right > 0 && r.bottom > 0 && r.left < vw && r.top < vh;
    if (opts.viewportOnly && !inViewport) continue;
    if (nodes.length >= maxNodes) { truncated = true; break; }
    const id = 'n' + nodes.length;
    ids.set(el, id);
    const node = { id, frameId: 'f0', role, x: r.left + sx, y: r.top + sy, width: r.width, height: r.height, inViewport, visible: true, focused: document.activeElement === el, disabled: !!el.disabled || el.getAttribute('aria-disabled') === 'true' };
    for (let p = el.parentElement; p; p = p.parentElement) { const pid = ids.get(p); if (pid) { node.parentId = pid; if (scrollables.has(pid)) node.scrollContainerId = pid; break; } }
    if (!node.scrollContainerId) { for (let p = el.parentElement; p; p = p.parentElement) { const pid = ids.get(p); if (pid && scrollables.has(pid)) { node.scrollContainerId = pid; break; } } }
    const name = nameOf(el, role); if (name) node.name = name;
    if (own && includeText) node.text = own.slice(0, 2000);
    if (!isProtected(el)) { const v = liveValue(el); if (v !== null && v !== '') node.value = v.slice(0, 500); }
    const tag = tagOf(el);
    const rk = roleKey(role);
    if (rk === 'checkbox' || rk === 'radiobutton' || rk === 'radio' || rk === 'switch' || rk === 'menuitemcheckbox' || rk === 'menuitemradio') { const ac = el.getAttribute('aria-checked'); node.checked = ac != null ? ac : (el.indeterminate ? 'mixed' : (el.checked ? 'true' : 'false')); }
    const ae = el.getAttribute('aria-expanded');
    if (ae != null) node.expanded = ae === 'true'; else if (tag === 'summary' && el.parentElement && tagOf(el.parentElement) === 'details') node.expanded = !!el.parentElement.open;
    const style = window.getComputedStyle(el); const oy = style.overflowY, ox = style.overflowX;
    if (((oy === 'auto' || oy === 'scroll') && el.scrollHeight > el.clientHeight + 1) || ((ox === 'auto' || ox === 'scroll') && el.scrollWidth > el.clientWidth + 1)) { node.scrollable = true; scrollables.add(id); }
    nodes.push(node);
  }
  let text = includeText ? norm(document.body ? document.body.innerText : '') : '';
  let bytes = JSON.stringify(nodes).length + text.length;
  if (bytes > maxBytes) {
    truncated = true;
    if (text.length > maxBytes / 2) text = text.slice(0, Math.floor(maxBytes / 2));
    while (nodes.length && JSON.stringify(nodes).length + text.length > maxBytes) nodes.splice(Math.floor(nodes.length * 0.9));
    bytes = JSON.stringify(nodes).length + text.length;
  }
  const frames = [{ frameId: 'f0', url: location.href, crossOrigin: false }];
  let framesFailed = 0;
  document.querySelectorAll('iframe, frame').forEach((f, i) => { let cross = true; try { cross = !f.contentDocument; } catch (e) { cross = true; } frames.push({ frameId: 'f' + (i + 1), url: f.src || '', crossOrigin: cross, parentFrameId: 'f0' }); framesFailed++; });
  return JSON.stringify({ nodes, frames, text, truncated, stats: { totalNodes: total, returnedNodes: nodes.length, bytes, elapsedMs: Math.round(now() - started), framesVisited: 1, framesFailed } });
})()"#;

/// The fallback script with its options substituted.
fn fallback_script(opts: &serde_json::Value) -> String {
  AGENT_FALLBACK_JS.replace("__OPTS__", &opts.to_string())
}

/// What the fallback perception script answers with, before it is wrapped.
#[derive(Debug, Deserialize)]
struct FallbackPerception {
  nodes: Vec<PerceptionNode>,
  frames: Vec<PerceptionFrame>,
  text: String,
  truncated: bool,
  stats: PerceptionStats,
}

/// Where the fallback resolver left the one match, in viewport pixels.
#[derive(Debug, Clone, Copy, Deserialize)]
struct FallbackCenter {
  x: f64,
  y: f64,
  width: f64,
  height: f64,
  visible: bool,
}

/// What the fallback resolver answers with.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FallbackResolution {
  match_count: u64,
  #[serde(default)]
  candidates: Vec<serde_json::Value>,
  #[serde(rename = "match")]
  matched: Option<LocatorCandidate>,
  center: Option<FallbackCenter>,
}

/// What to do to the one match, once the fallback resolver has found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FallbackAct {
  /// Describe it and nothing more.
  Describe,
  /// Scroll it into view, for a click.
  Scroll,
  /// Scroll, focus and prepare the field, for typing.
  Focus { clear_first: bool },
}

/// Resolve `locator` in the page with the fallback script.
///
/// Refuses ambiguity and absence in the same structured form the native
/// resolver does, so a caller sees one contract whichever engine answered.
async fn fallback_resolve(
  target: &CdpTarget,
  locator: &LocatorDescription,
  candidate_limit: Option<u64>,
  act: FallbackAct,
) -> Result<(LocatorCandidate, Option<FallbackCenter>), AgentError> {
  let (act_name, clear_first) = match act {
    FallbackAct::Describe => ("none", false),
    FallbackAct::Scroll => ("scroll", false),
    FallbackAct::Focus { clear_first } => ("focus", clear_first),
  };
  let opts = serde_json::json!({
    "mode": "resolve",
    "locator": locator,
    "candidateLimit": candidate_limit.unwrap_or(10).clamp(1, 100),
    "act": act_name,
    "clearFirst": clear_first,
  });
  let answer = evaluate_json_script(target, fallback_script(&opts)).await?;
  let resolution: FallbackResolution = serde_json::from_value(answer)
    .map_err(|e| AgentError::Malformed(format!("fallback resolver: {e}")))?;
  match resolution.match_count {
    0 => Err(AgentError::NoMatch {
      message: format!("No node matches locator ({}).", describe_locator(locator)),
    }),
    1 => {
      let matched = resolution
        .matched
        .ok_or_else(|| AgentError::Malformed("fallback resolver answered one match without it".into()))?;
      Ok((matched, resolution.center))
    }
    count => Err(AgentError::AmbiguousLocator {
      match_count: count,
      message: format!(
        "Ambiguous locator: {count} nodes match. Refine it with a role, a stable attribute, or more exact text. Candidates: {}",
        serde_json::Value::Array(resolution.candidates.clone())
      ),
      candidates: resolution.candidates,
    }),
  }
}

/// A point the fallback resolver left an element at, as somewhere to strike.
fn fallback_target(center: Option<FallbackCenter>) -> Result<ViewportTarget, AgentError> {
  let center = center
    .ok_or_else(|| AgentError::Malformed("the fallback resolver answered no position".into()))?;
  if !center.visible {
    return Err(AgentError::BadRequest(
      "the element could not be scrolled into view; it may be hidden or clipped".to_string(),
    ));
  }
  Ok(ViewportTarget {
    x: center.x,
    y: center.y,
    width: center.width,
    height: center.height,
  })
}

/// Keep a strike point inside the layout viewport.
fn clamp_to_viewport(point: ViewportTarget, viewport: (f64, f64)) -> ViewportTarget {
  let (width, height) = viewport;
  if width <= 0.0 || height <= 0.0 {
    return point;
  }
  ViewportTarget {
    x: point.x.clamp(1.0, (width - 1.0).max(1.0)),
    y: point.y.clamp(1.0, (height - 1.0).max(1.0)),
    ..point
  }
}

/// Glide to `point` and strike it, on an open session with `Page.enable` on,
/// waiting for a load afterwards.
async fn vellum_click_in(
  session: &mut WayfernSession,
  point: ViewportTarget,
  button: Option<&str>,
  click_count: Option<u32>,
) -> Result<(vellum::Strike, bool), AgentError> {
  let viewport = wayfern_cdp::layout_viewport(session).await?;
  let point = clamp_to_viewport(point, viewport);
  let origin = wayfern_cdp::glide_origin(&point, viewport);
  let width = Some(point.width.min(point.height));
  // Owned by the gesture: the boxed future may only borrow what the gesture
  // itself holds, never the caller's frame.
  let button = button.map(str::to_owned);
  let outcome = vellum::with_pointer(session, origin.0, origin.1, |s, p| {
    Box::pin(async move {
      vellum::glide(s, p, point.x, point.y, width).await?;
      vellum::strike_awaiting_load(s, p, button.as_deref(), click_count, CLICK_LOAD_TIMEOUT).await
    })
  })
  .await?;
  Ok(outcome)
}

/// Strike `point` with a humanized pointer on a fresh session.
async fn vellum_click(
  target: &CdpTarget,
  point: ViewportTarget,
  button: Option<&str>,
  click_count: Option<u32>,
) -> Result<(vellum::Strike, bool), AgentError> {
  let mut session = WayfernSession::open(target).await?;
  let outcome = async {
    session.call("Page.enable", serde_json::json!({})).await?;
    vellum_click_in(&mut session, point, button, click_count).await
  }
  .await;
  let _ = session.call("Page.disable", serde_json::json!({})).await;
  session.close().await;
  outcome
}

/// What to do to a field between the strike that focuses it and the typing.
enum FieldPreparation {
  /// Nothing: the field was already prepared by the caller's script.
  None,
  /// Run this script, which must return `true`.
  Script(String),
  /// Clear, or move the caret to the end of, the node behind this id.
  Node { backend_node_id: i64, clear: bool },
}

/// Script run on a resolved node to empty it, or park the caret at its end.
///
/// A strike puts the caret where the click landed; without this a text typed
/// into a field that already holds one would land in the middle of it.
const PREPARE_FIELD_FN: &str = r#"function(clear) {
  const el = this;
  const editable = !!el.isContentEditable;
  const tag = String(el.tagName || '').toLowerCase();
  if (clear) {
    if (editable) { el.textContent = ''; } else if (tag === 'input' || tag === 'textarea') { el.value = ''; }
    el.dispatchEvent(new Event('input', { bubbles: true }));
  } else if (editable) {
    const sel = window.getSelection(); if (sel) { sel.selectAllChildren(el); sel.collapseToEnd(); }
  } else if (typeof el.setSelectionRange === 'function') {
    try { const n = String(el.value || '').length; el.setSelectionRange(n, n); } catch (e) {}
  }
  return true;
}"#;

/// Script run on the focused element to park the caret at its end.
const CARET_TO_END_JS: &str = r#"(() => {
  const el = document.activeElement;
  if (!el) return true;
  if (el.isContentEditable) { const sel = window.getSelection(); if (sel) { sel.selectAllChildren(el); sel.collapseToEnd(); } }
  else if (typeof el.setSelectionRange === 'function') { try { const n = String(el.value || '').length; el.setSelectionRange(n, n); } catch (e) {} }
  return true;
})()"#;

async fn prepare_field(
  session: &mut WayfernSession,
  preparation: &FieldPreparation,
) -> Result<(), AgentError> {
  match preparation {
    FieldPreparation::None => Ok(()),
    FieldPreparation::Script(script) => {
      let result = session
        .call(
          "Runtime.evaluate",
          serde_json::json!({ "expression": script, "returnByValue": true }),
        )
        .await?;
      if let Some(exception) = result.get("exceptionDetails") {
        let message = exception
          .get("exception")
          .and_then(|e| e.get("description"))
          .or_else(|| exception.get("text"))
          .and_then(|v| v.as_str())
          .unwrap_or("preparing the field failed");
        return Err(AgentError::BadRequest(message.to_string()));
      }
      Ok(())
    }
    FieldPreparation::Node {
      backend_node_id,
      clear,
    } => {
      let resolved = session
        .call(
          "DOM.resolveNode",
          serde_json::json!({ "backendNodeId": backend_node_id }),
        )
        .await?;
      let object_id = resolved
        .get("object")
        .and_then(|o| o.get("objectId"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgentError::Malformed("DOM.resolveNode answered no object".into()))?;
      session
        .call(
          "Runtime.callFunctionOn",
          serde_json::json!({
            "objectId": object_id,
            "functionDeclaration": PREPARE_FIELD_FN,
            "arguments": [{ "value": clear }],
            "returnByValue": true,
          }),
        )
        .await?;
      Ok(())
    }
  }
}

/// Focus `point` with a strike, prepare the field, and type `text`.
async fn vellum_type_in(
  session: &mut WayfernSession,
  point: ViewportTarget,
  text: &str,
  typos: bool,
  preparation: FieldPreparation,
  timeout: Duration,
) -> Result<vellum::Inscription, AgentError> {
  let viewport = wayfern_cdp::layout_viewport(session).await?;
  let point = clamp_to_viewport(point, viewport);
  let origin = wayfern_cdp::glide_origin(&point, viewport);
  let width = Some(point.width.min(point.height));
  // Owned by the gesture, for the reason `vellum_click_in` gives.
  let text = text.to_owned();
  let inscription = vellum::with_pointer(session, origin.0, origin.1, |s, p| {
    Box::pin(async move {
      vellum::glide(s, p, point.x, point.y, width).await?;
      vellum::strike(s, p, None, None).await?;
      prepare_field(s, &preparation).await?;
      Ok::<_, AgentError>(vellum::inscribe(s, p, &text, typos, timeout).await?)
    })
  })
  .await?;
  Ok(inscription)
}

/// Type `text` at `point` on a fresh session.
async fn vellum_type(
  target: &CdpTarget,
  point: ViewportTarget,
  text: &str,
  typos: bool,
  preparation: FieldPreparation,
  timeout: Duration,
) -> Result<vellum::Inscription, AgentError> {
  let mut session = WayfernSession::open(target).await?;
  let outcome = vellum_type_in(&mut session, point, text, typos, preparation, timeout).await;
  session.close().await;
  outcome
}

/// Deliver an accepted keystroke plan through `Input.dispatchKeyEvent`.
///
/// The transport-level half of the pre-152 typing path, shared by the MCP
/// handlers and the REST agent endpoints.
pub(crate) async fn dispatch_keystrokes(
  target: &CdpTarget,
  events: &[crate::human_typing::TypingEvent],
) -> Result<(), CdpError> {
  use crate::human_typing::TypingAction;

  let mut connection = target.connect().await?;

  let mut cmd_id = 1u64;
  let mut last_time = 0.0;

  for event in events {
    let delay = event.time - last_time;
    if delay > 0.0 {
      tokio::time::sleep(Duration::from_secs_f64(delay)).await;
    }
    last_time = event.time;

    let (down, up) = match &event.action {
      TypingAction::Char(ch) => {
        let ch = ch.to_string();
        (
          serde_json::json!({
            "type": "keyDown",
            "text": ch,
            "key": ch,
            "unmodifiedText": ch,
          }),
          serde_json::json!({ "type": "keyUp", "key": ch }),
        )
      }
      TypingAction::Backspace => (
        serde_json::json!({
          "type": "keyDown",
          "key": "Backspace",
          "code": "Backspace",
          "windowsVirtualKeyCode": 8,
          "nativeVirtualKeyCode": 8,
        }),
        serde_json::json!({
          "type": "keyUp",
          "key": "Backspace",
          "code": "Backspace",
          "windowsVirtualKeyCode": 8,
          "nativeVirtualKeyCode": 8,
        }),
      ),
    };

    for params in [down, up] {
      connection
        .send_command(cmd_id, "Input.dispatchKeyEvent", params)
        .await?;
      // Drained rather than matched: the point is to keep reading so the
      // browser is never writing into a full socket while the next keystroke
      // is being timed. Bounded, because a reply that never comes must not
      // freeze typing forever — the keystroke itself was already delivered.
      let _ = tokio::time::timeout(KEYSTROKE_ACK_TIMEOUT, connection.next_text()).await;
      cmd_id += 1;
    }
  }

  connection.close().await;
  Ok(())
}

/// Read the page as the agent sees it.
pub(crate) async fn agent_perceive(
  ctx: &AgentContext,
  request: &PerceptionRequest,
) -> Result<PerceptionPage, AgentError> {
  if let Some(order) = request.text_order.as_deref() {
    if order != "reading" && order != "visual" {
      return Err(AgentError::InvalidArgument(
        "text_order must be \"reading\" or \"visual\"".to_string(),
      ));
    }
  }
  if ctx.engine.is_wayfern() {
    let mut session = WayfernSession::open(&ctx.target).await?;
    let page = wayfern_cdp::capture_page_perception(&mut session, request).await;
    session.close().await;
    return Ok(page?);
  }

  if request.cursor.as_deref().is_some_and(|c| !c.is_empty()) {
    return Err(AgentError::InvalidArgument(
      "the fallback engine answers in one page and has no cursors to continue".to_string(),
    ));
  }
  let opts = serde_json::json!({
    "mode": "perceive",
    "maxNodes": request.max_nodes.unwrap_or(100_000),
    "maxBytes": request.byte_cap(),
    "includeText": request.include_text.unwrap_or(true),
    "viewportOnly": request.viewport_only.unwrap_or(false),
  });
  let answer = evaluate_json_script(&ctx.target, fallback_script(&opts)).await?;
  let perception: FallbackPerception = serde_json::from_value(answer)
    .map_err(|e| AgentError::Malformed(format!("fallback perception: {e}")))?;
  let snapshot_id = format!(
    "fallback-{}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|d| d.as_millis())
      .unwrap_or(0)
  );
  Ok(PerceptionPage {
    snapshot_id,
    nodes: perception.nodes,
    frames: perception.frames,
    text: perception.text,
    truncated: perception.truncated,
    stats: perception.stats,
    cursor: None,
    engine: Engine::Fallback,
  })
}

/// Resolve a locator to exactly one node.
pub(crate) async fn agent_resolve_locator(
  ctx: &AgentContext,
  request: &AgentResolveRequest,
) -> Result<LocatorResolution, AgentError> {
  validate_locator(&request.locator)?;
  let locator = canonical_locator(&request.locator);
  if ctx.engine.is_wayfern() {
    let mut session = WayfernSession::open(&ctx.target).await?;
    let resolved = wayfern_cdp::resolve_locator(
      &mut session,
      &locator,
      ResolveOptions {
        candidate_limit: request.candidate_limit,
        ..Default::default()
      },
    )
    .await;
    session.close().await;
    return Ok(resolved?);
  }

  let (matched, _) = fallback_resolve(
    &ctx.target,
    &locator,
    request.candidate_limit,
    FallbackAct::Describe,
  )
  .await?;
  Ok(LocatorResolution {
    backend_node_id: None,
    match_count: 1,
    matched,
    locator,
    engine: Engine::Fallback,
  })
}

/// Resolve a locator and click it.
pub(crate) async fn agent_click_locator(
  ctx: &AgentContext,
  request: &AgentClickRequest,
) -> Result<AgentClick, AgentError> {
  validate_click(request)?;
  let locator = canonical_locator(&request.locator);
  let button = request.button.as_deref();

  if ctx.engine.is_wayfern() {
    let mut session = WayfernSession::open(&ctx.target).await?;
    let outcome = async {
      session.call("Page.enable", serde_json::json!({})).await?;
      let resolved =
        wayfern_cdp::resolve_locator(&mut session, &locator, ResolveOptions::default()).await?;
      let backend_node_id = resolved
        .backend_node_id
        .ok_or_else(|| AgentError::Malformed("resolveLocator answered no backendNodeId".into()))?;
      let point = wayfern_cdp::viewport_target(&mut session, backend_node_id).await?;
      let (_, navigated) =
        vellum_click_in(&mut session, point, button, request.click_count).await?;
      Ok::<_, AgentError>((resolved, navigated))
    }
    .await;
    let _ = session.call("Page.disable", serde_json::json!({})).await;
    session.close().await;
    let (resolved, navigated) = outcome?;
    return Ok(AgentClick {
      clicked: true,
      matched: resolved.matched,
      engine: Engine::Wayfern,
      navigated,
    });
  }

  let (matched, center) =
    fallback_resolve(&ctx.target, &locator, None, FallbackAct::Scroll).await?;
  let point = fallback_target(center)?;
  let mut session = WayfernSession::open(&ctx.target).await?;
  let outcome = async {
    session.call("Page.enable", serde_json::json!({})).await?;
    session
      .call(
        "Input.dispatchMouseEvent",
        serde_json::json!({ "type": "mouseMoved", "x": point.x, "y": point.y }),
      )
      .await?;
    let press = serde_json::json!({
      "type": "mousePressed",
      "x": point.x,
      "y": point.y,
      "button": button.unwrap_or("left"),
      "clickCount": request.click_count.unwrap_or(1),
    });
    session.call("Input.dispatchMouseEvent", press).await?;
    let release = serde_json::json!({
      "type": "mouseReleased",
      "x": point.x,
      "y": point.y,
      "button": button.unwrap_or("left"),
      "clickCount": request.click_count.unwrap_or(1),
    });
    let (_, navigated) = session
      .call_then_await_event(
        "Input.dispatchMouseEvent",
        release,
        "Page.loadEventFired",
        CLICK_LOAD_TIMEOUT,
      )
      .await?;
    Ok::<_, AgentError>(navigated)
  }
  .await;
  let _ = session.call("Page.disable", serde_json::json!({})).await;
  session.close().await;
  Ok(AgentClick {
    clicked: true,
    matched,
    engine: Engine::Fallback,
    navigated: outcome?,
  })
}

/// Resolve a locator and type into it.
///
/// `max_seconds` is the caller's typing budget; the refusal for a text that
/// would outlive it comes before anything touches the page.
pub(crate) async fn agent_type_locator(
  ctx: &AgentContext,
  request: &AgentTypeRequest,
  max_seconds: f64,
) -> Result<AgentTyping, AgentError> {
  validate_locator(&request.locator)?;
  if request.text.is_empty() {
    return Err(AgentError::InvalidArgument(
      "text must not be empty".to_string(),
    ));
  }
  let clear_first = request.clear_first.unwrap_or(true);
  let typos = request.typos.unwrap_or(true);
  let locator = canonical_locator(&request.locator);

  if ctx.engine.is_wayfern() {
    let timeout = vellum_typing_budget(&request.text, max_seconds)?;
    let mut session = WayfernSession::open(&ctx.target).await?;
    let outcome = async {
      let resolved =
        wayfern_cdp::resolve_locator(&mut session, &locator, ResolveOptions::default()).await?;
      let backend_node_id = resolved
        .backend_node_id
        .ok_or_else(|| AgentError::Malformed("resolveLocator answered no backendNodeId".into()))?;
      let point = wayfern_cdp::viewport_target(&mut session, backend_node_id).await?;
      let inscription = vellum_type_in(
        &mut session,
        point,
        &request.text,
        typos,
        FieldPreparation::Node {
          backend_node_id,
          clear: clear_first,
        },
        timeout,
      )
      .await?;
      Ok::<_, AgentError>((resolved, inscription))
    }
    .await;
    session.close().await;
    let (resolved, inscription) = outcome?;
    return Ok(AgentTyping {
      typed: true,
      characters: inscription.characters,
      corrections: Some(inscription.corrections),
      duration_ms: inscription.duration_ms,
      engine: Engine::Wayfern,
      matched: resolved.matched,
    });
  }

  // Planned first, before the field is touched, for the reason the selector
  // tools plan first: a refusal after the clear is a lie.
  let plan = plan_typing(&request.text, request.wpm, max_seconds)?;
  let (matched, _) = fallback_resolve(
    &ctx.target,
    &locator,
    None,
    FallbackAct::Focus { clear_first },
  )
  .await?;
  dispatch_keystrokes(&ctx.target, &plan).await?;
  Ok(AgentTyping {
    typed: true,
    characters: request.text.chars().count() as u64,
    corrections: None,
    duration_ms: plan.last().map_or(0.0, |event| event.time * 1000.0),
    engine: Engine::Fallback,
    matched,
  })
}

/// Read rows off the page. Wayfern 152 only.
pub(crate) async fn agent_extract(
  ctx: &AgentContext,
  request: &ExtractionRequest,
) -> Result<Extraction, AgentError> {
  validate_extraction(request)?;
  ctx.requires_wayfern_152()?;
  let mut request = request.clone();
  request.container = canonical_locator(&request.container);
  for field in &mut request.field_map {
    field.locator = canonical_locator(&field.locator);
  }
  request.next_page = request.next_page.as_ref().map(canonical_locator);
  let mut session = WayfernSession::open(&ctx.target).await?;
  let extraction = wayfern_cdp::extract_structured(&mut session, &request).await;
  session.close().await;
  Ok(extraction?)
}

/// Arm the picker and wait for the user's click. Wayfern 152 only.
pub(crate) async fn agent_pick_element(
  ctx: &AgentContext,
  timeout_ms: u64,
) -> Result<PickedElement, AgentError> {
  ctx.requires_wayfern_152()?;
  let mut session = WayfernSession::open(&ctx.target).await?;
  let picked =
    wayfern_cdp::pick_element(&mut session, Duration::from_millis(timeout_ms), true).await;
  session.close().await;
  Ok(picked?)
}

/// The JSON schema of a locator argument, shared by every tool that takes one.
fn locator_schema(description: &str) -> serde_json::Value {
  serde_json::json!({
    "type": "object",
    "description": description,
    "properties": {
      "role": { "type": "string", "description": "AX role token as the browser reports it (button, link, textField, heading, listItem, checkBox, comboBoxSelect, staticText); matched case- and separator-insensitively, and the ARIA names textbox, radio, img, progressbar, separator, generic and text are accepted as synonyms" },
      "name": { "type": "string", "description": "Exact accessible name, after whitespace collapse" },
      "nameContains": { "type": "string", "description": "Substring of the accessible name" },
      "text": { "type": "string", "description": "Exact visible text, from the live layout" },
      "textContains": { "type": "string", "description": "Substring of the visible text" },
      "attributes": {
        "type": "array",
        "description": "Attribute pairs that must all match",
        "items": {
          "type": "object",
          "properties": {
            "name": { "type": "string" },
            "value": { "type": "string" }
          },
          "required": ["name", "value"]
        }
      }
    }
  })
}

/// Where a profile's browser is, as the browser tools report it.
async fn resolve_target(profile: &BrowserProfile) -> Result<CdpTarget, McpError> {
  crate::cdp_target::resolve(profile)
    .await
    .map_err(|e| McpError {
      code: -32000,
      message: e.to_string(),
      data: None,
    })
}

/// The tail of a page script that answers where `el` is, in viewport pixels.
const RETURN_RECT_JS: &str = "const r = el.getBoundingClientRect(); return JSON.stringify({x: r.left + r.width / 2, y: r.top + r.height / 2, width: r.width, height: r.height});";

/// A script that scrolls the element behind `selector_escaped` into view and
/// answers where it is. It clicks nothing.
fn element_rect_script(selector_escaped: &str) -> String {
  format!(
    r#"(() => {{
      const el = document.querySelector('{selector_escaped}');
      if (!el) throw new Error('Element not found: {selector_escaped}');
      el.scrollIntoView({{block: 'center', inline: 'center', behavior: 'instant'}});
      {RETURN_RECT_JS}
    }})()"#
  )
}

/// The same for the element at `index` of a caller's cached snapshot.
fn indexed_rect_script(cache: &str, index: u64) -> String {
  format!(
    r#"(() => {{
      const arr = window[{cache}];
      if (!arr || !arr[{index}]) throw new Error('No element at index {index}. Call get_interactive_elements first or after navigation.');
      const el = arr[{index}];
      el.scrollIntoView({{block: 'center', inline: 'center', behavior: 'instant'}});
      {RETURN_RECT_JS}
    }})()"#
  )
}

/// The strike point a rect script answered with.
fn rect_from_script_result(result: &serde_json::Value) -> Result<ViewportTarget, McpError> {
  let rect = parse_json_script_result(result).map_err(AgentError::into_mcp)?;
  let number = |key: &str| rect.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
  let point = ViewportTarget {
    x: number("x"),
    y: number("y"),
    width: number("width"),
    height: number("height"),
  };
  if !(point.width > 0.0 && point.height > 0.0) {
    return Err(McpError {
      code: -32000,
      message: "The element has no visible box to click; it may be hidden or collapsed".to_string(),
      data: None,
    });
  }
  Ok(point)
}

/// Run a rect script and answer the strike point it found.
async fn locate_by_script(target: &CdpTarget, script: String) -> Result<ViewportTarget, McpError> {
  let result = crate::cdp_target::run_command(
    target,
    "Runtime.evaluate",
    serde_json::json!({ "expression": script, "returnByValue": true }),
  )
  .await
  .map_err(cdp_error)?;
  rect_from_script_result(&result)
}

/// What the selector and index typing tools do to the caret after the strike
/// that focuses the field.
///
/// A cleared field has nowhere else to put it. One that keeps its text needs
/// the caret at the end, or the new text lands wherever the click did.
fn caret_preparation(clear_first: bool) -> FieldPreparation {
  if clear_first {
    FieldPreparation::None
  } else {
    FieldPreparation::Script(CARET_TO_END_JS.to_string())
  }
}

fn click_report(prefix: &str, navigated: bool) -> String {
  if navigated {
    format!("{prefix} (a page load followed)")
  } else {
    prefix.to_string()
  }
}

fn typing_report(prefix: &str, inscription: &vellum::Inscription) -> String {
  format!(
    "{prefix} ({} characters, {} corrected, {:.0} ms)",
    inscription.characters, inscription.corrections, inscription.duration_ms
  )
}

impl McpServer {
  fn new() -> Self {
    Self {
      inner: Arc::new(AsyncMutex::new(McpServerInner {
        app_handle: None,
        token: None,
        shutdown_tx: None,
        sessions: HashMap::new(),
      })),
      is_running: AtomicBool::new(false),
      engine_ready: AtomicBool::new(false),
      port: AtomicU16::new(0),
    }
  }

  pub fn instance() -> &'static McpServer {
    &MCP_SERVER
  }

  pub fn is_running(&self) -> bool {
    self.is_running.load(Ordering::SeqCst)
  }

  /// Hand the engine the app handle every tool needs, once, at startup.
  ///
  /// Called unconditionally rather than from `start`, because the bridge can be
  /// the only transport in play and it must not have to boot a loopback
  /// listener it does not use to get one.
  pub async fn attach_app_handle(&self, app_handle: AppHandle) {
    let mut inner = self.inner.lock().await;
    if inner.app_handle.is_none() {
      inner.app_handle = Some(app_handle);
    }
    self.engine_ready.store(true, Ordering::SeqCst);
  }

  /// Whether the tool engine can answer a JSON-RPC message.
  pub fn is_engine_ready(&self) -> bool {
    self.engine_ready.load(Ordering::SeqCst)
  }

  /// Let a test drive the engine without a Tauri `AppHandle`.
  ///
  /// Exists so the bridge's transport can be exercised end to end, a real
  /// socket carrying a real `tools/list`, instead of only against a mock of
  /// the thing under test. `attach_app_handle` needs an `AppHandle` no unit
  /// test has, and the tools this unlocks (`ping`, `tools/list`) read no app
  /// state; every tool that DOES need the handle still refuses without one, so
  /// this cannot make a test pass that production would fail.
  #[cfg(test)]
  pub(crate) fn mark_engine_ready_for_tests(&self) {
    self.engine_ready.store(true, Ordering::SeqCst);
  }

  /// Let a test reach `stop()`, which early-returns unless the listener is up.
  /// Only the flag is set: no socket is bound, so nothing here can make a test
  /// pass that production would fail.
  #[cfg(test)]
  pub(crate) fn mark_running_for_tests(&self) {
    self.is_running.store(true, Ordering::SeqCst);
  }

  /// Gate an MCP tool on a capability the caller already resolved (e.g.
  /// `CLOUD_AUTH.can_use_browser_automation().await`). Logs the rejected gate
  /// with enough state for support to diagnose, without leaking secrets.
  async fn require_capability(feature: &str, allowed: bool) -> Result<(), McpError> {
    if !allowed {
      let summary = match CLOUD_AUTH.get_user().await {
        Some(state) => format!(
          "logged_in=true plan={} status={} period={:?}",
          state.user.plan, state.user.subscription_status, state.user.plan_period,
        ),
        None => "logged_in=false".to_string(),
      };
      log::warn!("[mcp] Rejected '{feature}' — plan does not include it ({summary})");
      return Err(McpError {
        code: -32000,
        message: format!("{feature} requires a plan that includes this feature"),
        data: None,
      });
    }
    Ok(())
  }

  pub fn get_port(&self) -> Option<u16> {
    let port = self.port.load(Ordering::SeqCst);
    if port > 0 {
      Some(port)
    } else {
      None
    }
  }

  pub async fn start(&self, app_handle: AppHandle) -> Result<u16, String> {
    if !WayfernTermsManager::instance().is_terms_accepted() {
      return Err(crate::backend_error("WAYFERN_TERMS_REQUIRED"));
    }

    if self.is_running() {
      return Err(crate::backend_error("MCP_SERVER_ALREADY_RUNNING"));
    }

    let settings_manager = SettingsManager::instance();
    let settings = settings_manager
      .load_settings()
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;

    // Get or generate token
    let existing_token = settings_manager
      .get_mcp_token(&app_handle)
      .await
      .ok()
      .flatten();

    let (token, _token_is_new) = match existing_token {
      Some(t) => (t, false),
      None => (
        settings_manager
          .generate_mcp_token(&app_handle)
          .await
          .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?,
        true,
      ),
    };

    // Determine port (use saved port, or try default, or random)
    let preferred_port = settings.mcp_port.unwrap_or(DEFAULT_MCP_PORT);
    let listener = self.bind_to_available_port(preferred_port).await?;
    let actual_port = listener
      .local_addr()
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?
      .port();

    // Save port if it changed
    let port_changed = settings.mcp_port != Some(actual_port);
    if port_changed {
      let mut new_settings = settings;
      new_settings.mcp_port = Some(actual_port);
      settings_manager
        .save_settings(&new_settings)
        .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;
    }

    let installer_handle = app_handle.clone();

    // Store state
    let mut inner = self.inner.lock().await;
    inner.app_handle = Some(app_handle);
    inner.token = Some(token.clone());

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    inner.shutdown_tx = Some(shutdown_tx);

    self.port.store(actual_port, Ordering::SeqCst);
    self.is_running.store(true, Ordering::SeqCst);
    self.engine_ready.store(true, Ordering::SeqCst);

    // Start HTTP server in background
    let http_state = McpHttpState {
      server: McpServer::instance(),
      token,
    };
    tokio::spawn(Self::run_http_server(listener, http_state, shutdown_rx));
    drop(inner);

    log::info!("[mcp] Server started on port {}", actual_port);

    // Local MCP is removed: the listener above is a tombstone, so there is
    // nothing to (re)install into a client here. Migrating clients that still
    // point at the old local endpoint onto remote MCP is done once at startup
    // (see `crate::migrate_local_mcp_clients`), not on every bind.
    let _ = installer_handle;
    Ok(actual_port)
  }

  async fn bind_to_available_port(&self, preferred: u16) -> Result<TcpListener, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], preferred));
    if let Ok(listener) = TcpListener::bind(addr).await {
      return Ok(listener);
    }

    for _ in 0..10 {
      let port = 51000 + (rand::random::<u16>() % 1000);
      let addr = SocketAddr::from(([127, 0, 0, 1], port));
      if let Ok(listener) = TcpListener::bind(addr).await {
        return Ok(listener);
      }
    }

    Err(crate::backend_error("MCP_PORT_UNAVAILABLE"))
  }

  /// Serve the loopback port as a TOMBSTONE.
  ///
  /// Local MCP has been removed in favour of remote MCP, which a user can reach
  /// from anywhere. The old port is still bound for legacy installs so a client
  /// that still points at it gets a clear, actionable answer — a 410 with a
  /// message, and a desktop dialog — instead of a silent connection refusal.
  /// Nothing here touches the tool engine; the engine now serves the remote
  /// bridge only (see [`crate::mcp_remote`]).
  ///
  /// TODO(local-mcp-removal): once enough releases have passed that no client
  /// still points at the local port, delete this tombstone, the enable/install
  /// paths that reach it, and the loopback engine handlers below entirely.
  async fn run_http_server(
    listener: TcpListener,
    _state: McpHttpState,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
  ) {
    let app: Router = Router::new()
      .route("/health", get(Self::handle_health))
      .fallback(Self::handle_local_deprecated);

    let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
    let server = async move {
      log::info!(
        "[mcp] Local MCP is removed; the loopback tombstone is listening on http://127.0.0.1:{}/mcp",
        port
      );
      if let Err(e) = axum::serve(listener, app).await {
        log::error!("[mcp] Tombstone server error: {}", e);
      }
    };

    tokio::select! {
      _ = server => {},
      _ = shutdown_rx => {
        log::info!("[mcp] Tombstone server shutting down");
      },
    }
  }

  /// Answer any request to the removed local server, and raise the dialog once.
  async fn handle_local_deprecated() -> Response {
    Self::note_local_mcp_attempt();
    let body = serde_json::json!({
      "jsonrpc": "2.0",
      "id": serde_json::Value::Null,
      "error": {
        // -32001: the reserved server-error range. The message is written for a
        // human reading their MCP client's error, not just a machine.
        "code": -32001,
        "message": "Donut's local MCP server has been removed. Connect Donut over remote MCP                     from Settings > Integrations, then reach it from anywhere.",
      }
    });
    (StatusCode::GONE, Json(body)).into_response()
  }

  /// Record that something tried to use the removed local server, and emit the
  /// dialog event at most once per throttle window.
  ///
  /// Public so the enable/install commands can raise the same dialog when the
  /// user asks for local MCP from inside the app.
  pub fn note_local_mcp_attempt() {
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|d| d.as_secs())
      .unwrap_or(0);
    let last = LAST_LOCAL_DEPRECATION_EMIT.load(Ordering::Relaxed);
    if now.saturating_sub(last) < LOCAL_DEPRECATION_THROTTLE_SECS {
      return;
    }
    // Compare-and-set so concurrent hits emit once, not once each.
    if LAST_LOCAL_DEPRECATION_EMIT
      .compare_exchange(last, now, Ordering::SeqCst, Ordering::Relaxed)
      .is_err()
    {
      return;
    }
    let _ = crate::events::emit_empty(LOCAL_MCP_DEPRECATED_EVENT);
  }

  // TODO(local-mcp-removal): dead once the loopback served a tombstone; kept
  // beside it so the whole local transport is deleted in one change.
  #[allow(dead_code)]
  async fn auth_middleware(
    State(state): State<McpHttpState>,
    req: Request<Body>,
    next: Next,
  ) -> Result<Response, StatusCode> {
    let path = req.uri().path();

    if path == "/health" {
      return Ok(next.run(req).await);
    }

    // Check token from URL path: /mcp/{token}
    let path_token = path
      .strip_prefix("/mcp/")
      .filter(|t| !t.is_empty() && !t.contains('/'));

    // Check token from Authorization header
    let header_token = req
      .headers()
      .get(header::AUTHORIZATION)
      .and_then(|h| h.to_str().ok())
      .and_then(|h| h.strip_prefix("Bearer "));

    // Constant-time comparison to avoid leaking the token prefix via timing.
    use subtle::ConstantTimeEq;
    let expected = state.token.as_bytes();
    let ct_eq = |t: Option<&str>| {
      t.is_some_and(|t| {
        let b = t.as_bytes();
        b.len() == expected.len() && b.ct_eq(expected).into()
      })
    };
    let valid = ct_eq(path_token) || ct_eq(header_token);

    if !valid {
      return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
  }

  async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({
      "status": "ok",
      "server": SERVER_NAME,
      "version": SERVER_VERSION,
      "protocolVersion": PROTOCOL_VERSION,
    }))
  }

  // TODO(local-mcp-removal): dead once the loopback served a tombstone; kept
  // beside it so the whole local transport is deleted in one change.
  #[allow(dead_code)]
  async fn handle_mcp_get() -> impl IntoResponse {
    // We don't support server-initiated SSE streams
    StatusCode::METHOD_NOT_ALLOWED
  }

  /// Largest JSON-RPC payload any transport will parse.
  ///
  /// Shared rather than per-transport on purpose: a body the loopback listener
  /// refuses must not become one the bridge accepts.
  pub(crate) const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

  /// The longest a session teardown will spend deleting page globals.
  ///
  /// Best effort and time-boxed on purpose. `end_session` is awaited by the
  /// HTTP DELETE handler, and a browser that has stopped answering can hold a
  /// single CDP call open for a minute, so an unbounded cleanup turns "forget
  /// my session" into a hang. Losing a delete costs nothing worth waiting for:
  /// a browser that cannot answer has no page left to leak, and the page-side
  /// slot cap reclaims anything a missed delete leaves behind.
  const CACHE_RELEASE_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

  /// Forget a session, on the server AND in the pages it left snapshots in.
  /// Idempotent; an unknown id is not an error, because a caller tearing down a
  /// session it already lost has nothing to fix.
  pub(crate) async fn end_session(&self, session_id: &str) {
    let cached_pages = {
      let mut inner = self.inner.lock().await;
      match inner.sessions.remove(session_id) {
        Some(session) => {
          log::info!("[mcp] Session terminated: {}", ShortId(session_id));
          session.cached_pages
        }
        None => return,
      }
    };

    if cached_pages.is_empty() {
      return;
    }

    // Dropping the server-side session is only half of ending it: the other
    // half is an array of live element references sitting in somebody's still
    // open tab, which nothing else ever removes.
    if tokio::time::timeout(
      Self::CACHE_RELEASE_BUDGET,
      self.release_cached_pages(cached_pages),
    )
    .await
    .is_err()
    {
      log::debug!(
        "[mcp] Session {} ended before its element caches could be cleared; the \
         page-side slot cap will reclaim them",
        ShortId(session_id)
      );
    }
  }

  /// Delete the interactive-element slots a session wrote, page by page.
  ///
  /// Every failure here is logged and skipped rather than surfaced: the session
  /// is already gone, the caller asked to forget it, and a closed browser is
  /// the commonest reason a delete cannot land, which is also the case where
  /// there is nothing left to clean.
  async fn release_cached_pages(&self, cached_pages: HashSet<(String, String)>) {
    let registry = INTERACTIVE_SLOT_REGISTRY;
    for (profile_id, slot) in cached_pages {
      let target = match self.resolve_cdp_target(&profile_id).await {
        Ok(target) => target,
        Err(e) => {
          log::debug!(
            "[mcp] Skipped clearing an element cache on profile {profile_id}: {}",
            e.message
          );
          continue;
        }
      };

      let js = format!(
        r#"(() => {{
          try {{ delete window[{slot}]; }} catch (e) {{ window[{slot}] = undefined; }}
          const registry = window[{registry}];
          if (Array.isArray(registry)) window[{registry}] = registry.filter((s) => s !== {slot});
          return true;
        }})()"#
      );

      if let Err(e) = self
        .send_cdp(
          &target,
          "Runtime.evaluate",
          serde_json::json!({
            "expression": js,
            "returnByValue": true,
          }),
        )
        .await
      {
        log::debug!(
          "[mcp] Could not clear an element cache on profile {profile_id}: {}",
          e.message
        );
      }
    }
  }

  /// Remember that this session left a snapshot on this profile's page, so
  /// `end_session` knows which global to delete and where.
  ///
  /// Silently does nothing when the session has already been forgotten: the
  /// teardown that removed it has already run, and re-adding the entry would
  /// resurrect a session-shaped record nothing will ever clean up again.
  async fn remember_cached_page(&self, session_id: &str, profile_id: &str, slot: &str) {
    let mut inner = self.inner.lock().await;
    if let Some(session) = inner.sessions.get_mut(session_id) {
      session
        .cached_pages
        .insert((profile_id.to_string(), slot.to_string()));
    }
  }

  /// Answer one JSON-RPC message, whatever carried it here.
  ///
  /// This is the whole protocol: parse, route `initialize`, absorb
  /// notifications, validate the session, meter the automation tools, dispatch.
  /// Every transport calls exactly this, so none of them can drift away from
  /// the others on a rule that matters (the session check and the rate limiter
  /// were both HTTP-only before the bridge existed).
  pub(crate) async fn handle_message(
    &self,
    origin: McpOrigin,
    session_id: Option<&str>,
    body: &[u8],
  ) -> McpOutcome {
    if body.len() > Self::MAX_MESSAGE_BYTES {
      return McpOutcome::BadRequest;
    }

    let request: McpRequest = match serde_json::from_slice(body) {
      Ok(request) => request,
      Err(_) => return McpOutcome::BadRequest,
    };

    if request.method == "initialize" {
      return match self.handle_initialize(request).await {
        Ok((new_session_id, (id, result))) => McpOutcome::Body {
          body: serde_json::to_value(McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            result: Some(result),
            error: None,
          })
          .unwrap_or_else(|_| serde_json::json!({})),
          new_session_id: Some(new_session_id),
        },
        Err((id, error)) => McpOutcome::Body {
          body: serde_json::to_value(McpResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            result: None,
            error: Some(error),
          })
          .unwrap_or_else(|_| serde_json::json!({})),
          new_session_id: None,
        },
      };
    }

    // A message with no id is a notification, and JSON-RPC forbids replying to
    // one. `notifications/initialized` is the only one that carries meaning.
    if request.id.is_none() {
      if request.method == "notifications/initialized" {
        if let Some(sid) = session_id {
          let mut inner = self.inner.lock().await;
          if let Some(session) = inner.sessions.get_mut(sid) {
            session.initialized = true;
          }
        }
      }
      return McpOutcome::Accepted;
    }

    // Validated only when the caller supplied one: a client that never called
    // `initialize` is still served, exactly as it was over HTTP. That leniency
    // stops at the tools whose ANSWER depends on which caller is asking, the
    // index-based ones, because there is no such thing as a correct reply to
    // "click element 3" from a caller whose snapshot the server cannot name.
    // They refuse in the handler (`require_indexed_session`); everything else
    // is a self-contained request that a sessionless caller can safely make.
    if let Some(sid) = session_id {
      let mut inner = self.inner.lock().await;
      // Touched HERE, on the path every id-carrying request takes, so the cap
      // evicts what is genuinely idle. Doing it in the
      // `notifications/initialized` branch instead, as the first attempt did -
      // touches a session exactly once in its life, which is no better than
      // evicting by creation time.
      match inner.sessions.get_mut(sid) {
        Some(session) => session.last_used = std::time::Instant::now(),
        None => return McpOutcome::UnknownSession,
      }
    }

    if Self::is_automation_tool_call(&request) {
      if let crate::automation_rate_limiter::RateLimitOutcome::Limited { retry_after_secs } =
        crate::automation_rate_limiter::check_automation_rate_limit().await
      {
        log::warn!(
          "[mcp] Rejected tools/call: automation rate limit exceeded; retry in {retry_after_secs}s"
        );
        return McpOutcome::RateLimited { retry_after_secs };
      }
    }

    let response = self
      .handle_request(
        McpCaller {
          origin,
          session: session_id,
        },
        request,
      )
      .await;
    McpOutcome::Body {
      body: serde_json::to_value(response).unwrap_or_else(|_| serde_json::json!({})),
      new_session_id: None,
    }
  }

  // TODO(local-mcp-removal): dead once the loopback served a tombstone; kept
  // beside it so the whole local transport is deleted in one change.
  #[allow(dead_code)]
  async fn handle_mcp_delete(
    State(state): State<McpHttpState>,
    req: Request<Body>,
  ) -> impl IntoResponse {
    let session_id = req
      .headers()
      .get("mcp-session-id")
      .and_then(|h| h.to_str().ok())
      .map(|s| s.to_string());

    if let Some(sid) = session_id {
      state.server.end_session(&sid).await;
    }

    StatusCode::OK
  }

  // TODO(local-mcp-removal): dead once the loopback served a tombstone; kept
  // beside it so the whole local transport is deleted in one change.
  #[allow(dead_code)]
  async fn handle_mcp_post(State(state): State<McpHttpState>, req: Request<Body>) -> Response {
    let session_id = req
      .headers()
      .get("mcp-session-id")
      .and_then(|h| h.to_str().ok())
      .map(|s| s.to_string());

    let body_bytes = match axum::body::to_bytes(req.into_body(), Self::MAX_MESSAGE_BYTES).await {
      Ok(b) => b,
      Err(_) => {
        return (StatusCode::BAD_REQUEST, "Invalid request body").into_response();
      }
    };

    match state
      .server
      .handle_message(McpOrigin::Loopback, session_id.as_deref(), &body_bytes)
      .await
    {
      McpOutcome::Body {
        body,
        new_session_id,
      } => {
        let encoded = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
        let mut builder = Response::builder()
          .status(StatusCode::OK)
          .header(header::CONTENT_TYPE, "application/json");
        if let Some(sid) = new_session_id {
          builder = builder.header("mcp-session-id", sid);
        }
        builder
          .body(Body::from(encoded))
          .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
      }
      McpOutcome::Accepted => StatusCode::ACCEPTED.into_response(),
      McpOutcome::UnknownSession => StatusCode::NOT_FOUND.into_response(),
      McpOutcome::BadRequest => (StatusCode::BAD_REQUEST, "Invalid JSON").into_response(),
      McpOutcome::RateLimited { retry_after_secs } => (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, retry_after_secs.to_string())],
        "automation request rate limit exceeded",
      )
        .into_response(),
    }
  }

  fn is_automation_tool_call(request: &McpRequest) -> bool {
    if request.method != "tools/call" {
      return false;
    }

    let Some(tool_name) = request
      .params
      .as_ref()
      .and_then(|params| params.get("name"))
      .and_then(|name| name.as_str())
    else {
      return false;
    };

    matches!(
      tool_name,
      "run_profile"
        | "kill_profile"
        | "batch_run_profiles"
        | "batch_stop_profiles"
        | "start_sync_session"
        | "navigate"
        | "screenshot"
        | "evaluate_javascript"
        | "click_element"
        | "type_text"
        | "get_page_content"
        | "get_page_info"
        | "get_interactive_elements"
        | "click_by_index"
        | "type_by_index"
        // The agent surface drives the same browser through the same paid
        // gate; a native page read is automation exactly as a script one is.
        | "perceive_page"
        | "resolve_locator"
        | "click_locator"
        | "type_locator"
        | "extract_structured"
        | "pick_element"
        // Starting a bot run leases a remote host for up to two hours and
        // spends the account's pooled remote-hour budget, which makes it the
        // most expensive tool here. Cancelling one reaches the same fleet, and
        // is metered alongside the remote-session stop it mirrors.
        //
        // Deliberately absent: set_cookie_bot_schedule and
        // delete_cookie_bot_schedule. They are configuration and lease
        // nothing; metering them would throttle an agent enrolling many
        // profiles, and they are not what spends the account's hours.
        | "run_cookie_bot_now"
        | "cancel_cookie_bot_run"
        // Leasing a remote host is the single most expensive action here, and
        // ending one reaches the same fleet. Both are metered exactly as their
        // REST equivalents already are.
        | "run_profile_remote"
        | "stop_remote_session"
    )
  }

  pub async fn stop(&self) -> Result<(), String> {
    if !self.is_running() {
      return Err(crate::backend_error("MCP_SERVER_NOT_RUNNING"));
    }

    let mut inner = self.inner.lock().await;
    // The bearer token is loopback-only, so it goes with the listener.
    inner.token = None;
    // The session map is NOT cleared, for the same reason the app handle below
    // survives: it is shared with the cloud bridge, and clearing it here made
    // turning the local switch off answer 404 MCP_SESSION_NOT_FOUND to a remote
    // caller that had nothing to do with the loopback transport. The website
    // re-initializes on a 404 and self-heals, but the official MCP TypeScript
    // SDK does not, it throws on any non-ok POST, so a third-party agent took
    // a hard mid-run error from an unrelated toggle. A session holds only
    // `initialized: bool`; `end_session` is the eviction path.

    // Send shutdown signal
    if let Some(tx) = inner.shutdown_tx.take() {
      let _ = tx.send(());
    }

    self.port.store(0, Ordering::SeqCst);
    self.is_running.store(false, Ordering::SeqCst);

    // The app handle and `engine_ready` deliberately survive. Closing the
    // loopback listener is a statement about one transport; the cloud bridge
    // may still be carrying tool calls, and dropping the handle here would
    // break it with "MCP server not properly initialized" on the next call.

    log::info!("[mcp] Server stopped");
    Ok(())
  }

  pub fn get_tools(&self) -> Vec<McpTool> {
    vec![
      McpTool {
        name: "list_profiles".to_string(),
        description: "List all Wayfern browser profiles".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_profile".to_string(),
        description: "Get details of a specific browser profile".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to retrieve"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "run_profile".to_string(),
        description: "Launch a browser profile with an optional URL. Requires an active Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to launch"
            },
            "url": {
              "type": "string",
              "description": "Optional URL to open in the browser"
            },
            "headless": {
              "type": "boolean",
              "description": "Run the browser in headless mode"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "kill_profile".to_string(),
        description: "Stop a running browser profile. Requires an active Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to stop"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "batch_run_profiles".to_string(),
        description: "Launch multiple browser profiles at once with an optional URL. Requires an active Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_ids": {
              "type": "array",
              "items": { "type": "string" },
              "description": "UUIDs of the profiles to launch"
            },
            "url": {
              "type": "string",
              "description": "Optional URL to open in every launched profile"
            },
            "headless": {
              "type": "boolean",
              "description": "Run the browsers in headless mode"
            }
          },
          "required": ["profile_ids"]
        }),
      },
      McpTool {
        name: "batch_stop_profiles".to_string(),
        description: "Stop multiple running browser profiles at once. Requires an active Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_ids": {
              "type": "array",
              "items": { "type": "string" },
              "description": "UUIDs of the profiles to stop"
            }
          },
          "required": ["profile_ids"]
        }),
      },
      McpTool {
        name: "create_profile".to_string(),
        description: "Create a new browser profile".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "name": {
              "type": "string",
              "description": "Name for the new profile"
            },
            "browser": {
              "type": "string",
              "enum": ["wayfern"],
              "description": "Browser engine to use"
            },
            "proxy_id": {
              "type": "string",
              "description": "Optional proxy UUID to assign"
            },
            "launch_hook": {
              "type": "string",
              "description": "Optional HTTP(S) URL to call before launch for transient proxy overrides"
            },
            "group_id": {
              "type": "string",
              "description": "Optional group UUID to assign"
            },
            "tags": {
              "type": "array",
              "items": { "type": "string" },
              "description": "Optional tags for the profile"
            },
            "ephemeral": {
              "type": "boolean",
              "description": "Keep this profile's browsing data in memory only, so nothing it browses reaches real disk (default: false)"
            },
            "temporary": {
              "type": "boolean",
              "description": "Create a profile for one run: implies ephemeral, is deleted when its browser stops, and is swept at startup if it outlived a crash (default: false)"
            }
          },
          "required": ["name", "browser"]
        }),
      },
      McpTool {
        name: "detect_browser_profiles".to_string(),
        description: "Detect importable Chromium-family browser profiles (Chrome, Chromium, Brave) on this machine, or scan a custom folder for profile directories".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "folder": {
              "type": "string",
              "description": "Optional folder to scan instead of the default browser locations. Accepts a single profile dir, a Chromium user-data dir, or a folder holding one profile dir per child."
            }
          }
        }),
      },
      McpTool {
        name: "import_browser_profiles".to_string(),
        description: "Bulk-import browser profiles from on-disk profile folders (e.g. paths returned by detect_browser_profiles). Each imported profile becomes a Wayfern profile; items are isolated so one failure doesn't stop the rest".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "items": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "source_path": {
                    "type": "string",
                    "description": "Path to the source profile directory"
                  },
                  "new_profile_name": {
                    "type": "string",
                    "description": "Name for the imported profile"
                  },
                  "proxy_id": {
                    "type": "string",
                    "description": "Optional proxy UUID to assign to this profile"
                  },
                  "vpn_id": {
                    "type": "string",
                    "description": "Optional VPN UUID to assign to this profile"
                  },
                  "browser_type": {
                    "type": "string",
                    "description": "Source browser family (chromium, brave, edge, vivaldi, opera, arc, yandex, ...). Selects which OS keychain entry holds the key that unlocks the source's cookies and passwords, so an accurate value is what makes secrets survive the import"
                  },
                  "allow_running": {
                    "type": "boolean",
                    "description": "Import even though the source browser is running. Databases are still snapshotted consistently, but site data stored in LevelDB may be captured mid-write"
                  }
                },
                "required": ["source_path", "new_profile_name"]
              },
              "description": "Profiles to import"
            },
            "group_id": {
              "type": "string",
              "description": "Optional group UUID assigned to every imported profile"
            },
            "duplicate_strategy": {
              "type": "string",
              "enum": ["skip", "rename"],
              "description": "How to handle an already-taken profile name (default: rename with a numeric suffix)"
            }
          },
          "required": ["items"]
        }),
      },
      McpTool {
        name: "update_profile".to_string(),
        description: "Update an existing browser profile's settings".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to update"
            },
            "name": {
              "type": "string",
              "description": "New name for the profile"
            },
            "proxy_id": {
              "type": "string",
              "description": "Proxy UUID to assign (empty string to remove)"
            },
            "launch_hook": {
              "type": "string",
              "description": "Launch hook URL to assign (empty string to remove)"
            },
            "group_id": {
              "type": "string",
              "description": "Group UUID to assign (empty string to remove)"
            },
            "tags": {
              "type": "array",
              "items": { "type": "string" },
              "description": "Tags for the profile (replaces existing tags)"
            },
            "extension_group_id": {
              "type": "string",
              "description": "Extension group UUID to assign (empty string to remove)"
            },
            "proxy_bypass_rules": {
              "type": "array",
              "items": { "type": "string" },
              "description": "Proxy bypass rules (replaces existing rules)"
            },
            "clear_on_close": {
              "type": "boolean",
              "description": "Wipe browsing data (keeping extensions and bookmarks) when the browser exits. Not available for ephemeral or password-protected profiles."
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "delete_profile".to_string(),
        description: "Delete a browser profile and all its data".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to delete"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "list_tags".to_string(),
        description: "List all tags used across profiles".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "list_proxies".to_string(),
        description: "List all configured proxies".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_profile_status".to_string(),
        description: "Check whether a browser profile is running and can be driven. Returns is_running (true when the browser can be driven, wherever it is), location ('local', 'remote' or 'stopped'), is_running_locally, and remote_session_id when it is running on the remote fleet.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to check"
            }
          },
          "required": ["profile_id"]
        }),
      },
      // Group management tools
      McpTool {
        name: "list_groups".to_string(),
        description: "List all profile groups".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_group".to_string(),
        description: "Get details of a specific group".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": {
              "type": "string",
              "description": "The UUID of the group to retrieve"
            }
          },
          "required": ["group_id"]
        }),
      },
      McpTool {
        name: "create_group".to_string(),
        description: "Create a new profile group".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "name": {
              "type": "string",
              "description": "The name for the new group"
            }
          },
          "required": ["name"]
        }),
      },
      McpTool {
        name: "update_group".to_string(),
        description: "Update an existing group's name".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": {
              "type": "string",
              "description": "The UUID of the group to update"
            },
            "name": {
              "type": "string",
              "description": "The new name for the group"
            }
          },
          "required": ["group_id", "name"]
        }),
      },
      McpTool {
        name: "delete_group".to_string(),
        description: "Delete a profile group".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": {
              "type": "string",
              "description": "The UUID of the group to delete"
            }
          },
          "required": ["group_id"]
        }),
      },
      McpTool {
        name: "assign_profiles_to_group".to_string(),
        description: "Assign one or more profiles to a group".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_ids": {
              "type": "array",
              "items": { "type": "string" },
              "description": "Array of profile UUIDs to assign"
            },
            "group_id": {
              "type": "string",
              "description": "The UUID of the group to assign to (null to remove from group)"
            }
          },
          "required": ["profile_ids"]
        }),
      },
      McpTool {
        name: "distribute_proxies".to_string(),
        description: "Give each profile its own proxy. Pairs are applied one \
                      profile at a time and every outcome is reported, so a \
                      profile whose browser is running is refused by name \
                      instead of failing the whole request."
          .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "pairs": {
              "type": "array",
              "description": "Profile/proxy pairs to apply, one proxy per profile",
              "items": {
                "type": "object",
                "properties": {
                  "profile_id": {
                    "type": "string",
                    "description": "The UUID of the profile to move"
                  },
                  "proxy_id": {
                    "type": "string",
                    "description": "The UUID of the stored proxy to assign"
                  }
                },
                "required": ["profile_id", "proxy_id"]
              }
            }
          },
          "required": ["pairs"]
        }),
      },
      // Full proxy management tools
      McpTool {
        name: "get_proxy".to_string(),
        description: "Get details of a specific proxy".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "proxy_id": {
              "type": "string",
              "description": "The UUID of the proxy to retrieve"
            }
          },
          "required": ["proxy_id"]
        }),
      },
      McpTool {
        name: "create_proxy".to_string(),
        description: "Create a new proxy configuration.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "name": {
              "type": "string",
              "description": "The name for the new proxy"
            },
            "proxy_type": {
              "type": "string",
              "enum": ["http", "https", "httpstls", "socks4", "socks5", "vless"],
              "description": "The proxy protocol"
            },
            "host": {
              "type": "string",
              "description": "The proxy host address (for regular proxies)"
            },
            "port": {
              "type": "integer",
              "description": "The proxy port number (for regular proxies)"
            },
            "username": {
              "type": "string",
              "description": "Optional username for authentication (for regular proxies)"
            },
            "password": {
              "type": "string",
              "description": "Optional password for authentication (for regular proxies)"
            },
            "vless_uri": {
              "type": "string",
              "description": "VLESS + XTLS Vision + REALITY share URI"
            }
          },
          "required": ["name", "proxy_type"]
        }),
      },
      McpTool {
        name: "update_proxy".to_string(),
        description: "Update an existing proxy configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "proxy_id": {
              "type": "string",
              "description": "The UUID of the proxy to update"
            },
            "name": {
              "type": "string",
              "description": "New name for the proxy"
            },
            "proxy_type": {
              "type": "string",
              "enum": ["http", "https", "httpstls", "socks4", "socks5", "vless"],
              "description": "The proxy protocol"
            },
            "host": {
              "type": "string",
              "description": "The proxy host address (for regular proxies)"
            },
            "port": {
              "type": "integer",
              "description": "The proxy port number (for regular proxies)"
            },
            "username": {
              "type": "string",
              "description": "Optional username for authentication (for regular proxies)"
            },
            "password": {
              "type": "string",
              "description": "Optional password for authentication (for regular proxies)"
            },
            "vless_uri": {
              "type": "string",
              "description": "VLESS + XTLS Vision + REALITY share URI"
            }
          },
          "required": ["proxy_id"]
        }),
      },
      McpTool {
        name: "delete_proxy".to_string(),
        description: "Delete a proxy configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "proxy_id": {
              "type": "string",
              "description": "The UUID of the proxy to delete"
            }
          },
          "required": ["proxy_id"]
        }),
      },
      McpTool {
        name: "export_proxies".to_string(),
        description: "Export all proxy configurations".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "format": {
              "type": "string",
              "enum": ["json", "txt"],
              "description": "Export format (json for structured data, txt for URL format)"
            }
          },
          "required": ["format"]
        }),
      },
      McpTool {
        name: "import_proxies".to_string(),
        description: "Import proxy configurations from JSON or TXT content".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "content": {
              "type": "string",
              "description": "The proxy configuration content to import"
            },
            "format": {
              "type": "string",
              "enum": ["json", "txt"],
              "description": "Import format (json or txt)"
            },
            "name_prefix": {
              "type": "string",
              "description": "Optional prefix for imported proxy names (default: 'Imported')"
            }
          },
          "required": ["content", "format"]
        }),
      },
      // VPN management tools
      McpTool {
        name: "import_vpn".to_string(),
        description: "Import a WireGuard (.conf) configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "content": {
              "type": "string",
              "description": "Raw WireGuard config file content"
            },
            "filename": {
              "type": "string",
              "description": "Original filename (.conf)"
            },
            "name": {
              "type": "string",
              "description": "Optional display name for the VPN config"
            }
          },
          "required": ["content", "filename"]
        }),
      },
      McpTool {
        name: "list_vpn_configs".to_string(),
        description: "List all stored VPN configurations".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "delete_vpn".to_string(),
        description: "Delete a VPN configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "vpn_id": {
              "type": "string",
              "description": "The UUID of the VPN config to delete"
            }
          },
          "required": ["vpn_id"]
        }),
      },
      McpTool {
        name: "connect_vpn".to_string(),
        description: "Connect to a VPN configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "vpn_id": {
              "type": "string",
              "description": "The UUID of the VPN config to connect"
            }
          },
          "required": ["vpn_id"]
        }),
      },
      McpTool {
        name: "disconnect_vpn".to_string(),
        description: "Disconnect from a VPN".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "vpn_id": {
              "type": "string",
              "description": "The UUID of the VPN to disconnect"
            }
          },
          "required": ["vpn_id"]
        }),
      },
      McpTool {
        name: "get_vpn_status".to_string(),
        description: "Get the connection status of a VPN".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "vpn_id": {
              "type": "string",
              "description": "The UUID of the VPN to check"
            }
          },
          "required": ["vpn_id"]
        }),
      },
      // Fingerprint management tools
      McpTool {
        name: "get_profile_fingerprint".to_string(),
        description: "Get the fingerprint configuration for a Wayfern profile"
          .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "update_profile_fingerprint".to_string(),
        description:
          "Update the fingerprint configuration for a Wayfern profile. Requires an active Pro subscription."
            .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to update"
            },
            "fingerprint": {
              "type": "string",
              "description": "JSON string of the fingerprint configuration, or null to clear"
            },
            "os": {
              "type": "string",
              "enum": ["windows", "macos", "linux"],
              "description": "Operating system for fingerprint generation"
            },
            "restore_session": {
              "type": "boolean",
              "description": "Reopen the windows and tabs of the last session on an interactive launch (default: true). Automation runs never restore."
            },
            "webrtc_mode": {
              "type": "string",
              "enum": ["auto", "tcp_only", "block"],
              "description": "How WebRTC may reach the network: auto (default), tcp_only (publish only the proxy's exit address), or block (no ICE candidates at all)"
            },
            "randomize_fingerprint_on_launch": {
              "type": "boolean",
              "description": "Whether to generate a new fingerprint on every launch"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "update_profile_proxy_bypass_rules".to_string(),
        description:
          "Update proxy bypass rules for a profile. Requests matching these rules will connect directly, bypassing the proxy."
            .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to update"
            },
            "rules": {
              "type": "array",
              "items": { "type": "string" },
              "description": "Array of bypass rules. Supports hostnames (e.g. 'example.com'), IP addresses, and regex patterns."
            }
          },
          "required": ["profile_id", "rules"]
        }),
      },
      McpTool {
        name: "update_profile_dns_blocklist".to_string(),
        description:
          "Update the DNS blocklist level for a profile. Blocks ads, trackers, and malware domains at the proxy level."
            .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to update"
            },
            "level": {
              "type": "string",
              "enum": ["none", "light", "normal", "pro", "pro_plus", "ultimate"],
              "description": "DNS blocklist level. 'none' disables blocking."
            }
          },
          "required": ["profile_id", "level"]
        }),
      },
      McpTool {
        name: "get_dns_blocklist_status".to_string(),
        description: "Get the cache status of all DNS blocklist tiers including entry counts and freshness.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "list_extensions".to_string(),
        description: "List all managed browser extensions. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "list_extension_groups".to_string(),
        description: "List all extension groups. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "add_extension".to_string(),
        description: "Add a managed browser extension from a path on the machine running Donut: a .crx or .zip archive file, or an unpacked extension folder holding a top-level manifest.json. With link set to true, which only applies to a folder, the folder is loaded in place instead of being copied into Donut, so edits to it apply on the next browser start and the extension is machine-local and never synced. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "path": { "type": "string", "description": "Path on the machine running Donut to a .crx/.zip file or to an unpacked extension folder" },
            "name": { "type": "string", "description": "Display name, used only when the manifest carries no name of its own" },
            "link": { "type": "boolean", "description": "Folders only: load the folder in place instead of copying it into Donut. Linked extensions never sync. Defaults to false." }
          },
          "required": ["path"]
        }),
      },
      McpTool {
        name: "update_extension".to_string(),
        description: "Rename a managed extension and/or replace its payload from a path on the machine running Donut: a .crx or .zip archive file, or an unpacked extension folder holding a top-level manifest.json. With link set to true, which only applies to a folder, the folder is loaded in place instead of being copied into Donut, so the extension becomes machine-local and never syncs. At least one of name or path must be given. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "extension_id": { "type": "string", "description": "The extension ID to update" },
            "name": { "type": "string", "description": "New display name" },
            "path": { "type": "string", "description": "Path on the machine running Donut to the .crx/.zip file or unpacked extension folder to replace the payload with" },
            "link": { "type": "boolean", "description": "Folders only: load the folder in place instead of copying it into Donut. Linked extensions never sync. Defaults to false." }
          },
          "required": ["extension_id"]
        }),
      },
      McpTool {
        name: "create_extension_group".to_string(),
        description: "Create a new extension group. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "name": { "type": "string", "description": "Name for the extension group" }
          },
          "required": ["name"]
        }),
      },
      McpTool {
        name: "update_extension_group".to_string(),
        description: "Rename an extension group and/or replace its membership with an exact list of extension IDs. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": { "type": "string", "description": "The extension group ID to update" },
            "name": { "type": "string", "description": "New name for the extension group" },
            "extension_ids": {
              "type": "array",
              "items": { "type": "string" },
              "description": "The complete set of extension IDs the group should contain, replacing the current membership"
            }
          },
          "required": ["group_id"]
        }),
      },
      McpTool {
        name: "add_extension_to_group".to_string(),
        description: "Add an extension to an extension group. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": { "type": "string", "description": "The extension group ID" },
            "extension_id": { "type": "string", "description": "The extension ID to add to the group" }
          },
          "required": ["group_id", "extension_id"]
        }),
      },
      McpTool {
        name: "remove_extension_from_group".to_string(),
        description: "Remove an extension from an extension group. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": { "type": "string", "description": "The extension group ID" },
            "extension_id": { "type": "string", "description": "The extension ID to remove from the group" }
          },
          "required": ["group_id", "extension_id"]
        }),
      },
      McpTool {
        name: "delete_extension".to_string(),
        description: "Delete a managed extension. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "extension_id": { "type": "string", "description": "The extension ID to delete" }
          },
          "required": ["extension_id"]
        }),
      },
      McpTool {
        name: "delete_extension_group".to_string(),
        description: "Delete an extension group. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": { "type": "string", "description": "The extension group ID to delete" }
          },
          "required": ["group_id"]
        }),
      },
      McpTool {
        name: "assign_extension_group_to_profile".to_string(),
        description: "Assign an extension group to a profile, or remove the assignment. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": { "type": "string", "description": "The profile ID" },
            "extension_group_id": { "type": "string", "description": "The extension group ID, or empty string to remove" }
          },
          "required": ["profile_id"]
        }),
      },
      // Cookie management tools
      McpTool {
        name: "import_profile_cookies".to_string(),
        description: "Import cookies into a Wayfern profile from a JSON array (Puppeteer / EditThisCookie format) or a Netscape cookies.txt. Format is auto-detected. The browser must not be running.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the target profile"
            },
            "content": {
              "type": "string",
              "description": "Raw cookie file content (JSON array or Netscape cookies.txt)"
            }
          },
          "required": ["profile_id", "content"]
        }),
      },
      // Team lock tools
      McpTool {
        name: "get_team_locks".to_string(),
        description: "List all active team profile locks. Requires team plan.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_team_lock_status".to_string(),
        description: "Check if a profile is locked by a team member. Requires team plan.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to check"
            }
          },
          "required": ["profile_id"]
        }),
      },
      // Synchronizer tools
      McpTool {
        name: "start_sync_session".to_string(),
        description: "Start a synchronizer session. Launches a leader profile and follower profiles, then mirrors all actions from the leader to the followers in real time. Only Wayfern profiles are supported. Requires paid subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "leader_profile_id": {
              "type": "string",
              "description": "The UUID of the leader profile"
            },
            "follower_profile_ids": {
              "type": "array",
              "items": { "type": "string" },
              "description": "UUIDs of follower profiles"
            }
          },
          "required": ["leader_profile_id", "follower_profile_ids"]
        }),
      },
      McpTool {
        name: "stop_sync_session".to_string(),
        description: "Stop an active synchronizer session. Kills all follower profiles and the leader.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "session_id": {
              "type": "string",
              "description": "The sync session ID"
            }
          },
          "required": ["session_id"]
        }),
      },
      McpTool {
        name: "get_sync_sessions".to_string(),
        description: "List all active synchronizer sessions.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {}
        }),
      },
      McpTool {
        name: "remove_sync_follower".to_string(),
        description: "Remove a follower from an active synchronizer session.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "session_id": {
              "type": "string",
              "description": "The sync session ID"
            },
            "follower_profile_id": {
              "type": "string",
              "description": "The UUID of the follower to remove"
            }
          },
          "required": ["session_id", "follower_profile_id"]
        }),
      },
      // Browser interaction tools
      McpTool {
        name: "navigate".to_string(),
        description: "Navigate a running browser profile to a URL. Waits for the page to fully load before returning.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "url": {
              "type": "string",
              "description": "The URL to navigate to"
            }
          },
          "required": ["profile_id", "url"]
        }),
      },
      McpTool {
        name: "screenshot".to_string(),
        description: "Take a screenshot of the current page in a running browser profile. Returns base64-encoded image."
          .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "format": {
              "type": "string",
              "enum": ["png", "jpeg", "webp"],
              "description": "Image format (default: png)"
            },
            "quality": {
              "type": "integer",
              "description": "Image quality 0-100 for jpeg/webp (default: 80)"
            },
            "full_page": {
              "type": "boolean",
              "description": "Capture the full scrollable page (default: false)"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "evaluate_javascript".to_string(),
        description:
          "Execute JavaScript in the context of the current page and return the result. Works with both static and dynamically-generated content. Set wait_for_load=true if the script triggers navigation (e.g., form.submit())."
            .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "expression": {
              "type": "string",
              "description": "JavaScript expression to evaluate"
            },
            "await_promise": {
              "type": "boolean",
              "description": "Whether to await the result if it's a Promise (default: false)"
            },
            "wait_for_load": {
              "type": "boolean",
              "description": "Wait for page load after execution, use when the script triggers navigation like form.submit() (default: false)"
            }
          },
          "required": ["profile_id", "expression"]
        }),
      },
      McpTool {
        name: "click_element".to_string(),
        description: "Click on an element identified by a CSS selector. If the click triggers a page navigation, waits for the new page to load before returning.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "selector": {
              "type": "string",
              "description": "CSS selector for the element to click"
            }
          },
          "required": ["profile_id", "selector"]
        }),
      },
      McpTool {
        name: "type_text".to_string(),
        description: "Focus an element by CSS selector and type text into it. By default uses realistic human-like typing with variable speed, natural errors, and self-corrections. Only set instant=true when you are certain the target does not have bot detection (e.g. browser address bars, developer tools, internal apps) — using instant on public websites risks the profile being flagged as a bot.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "selector": {
              "type": "string",
              "description": "CSS selector for the input element"
            },
            "text": {
              "type": "string",
              "description": "Text to type into the element"
            },
            "clear_first": {
              "type": "boolean",
              "description": "Clear the input before typing (default: true)"
            },
            "instant": {
              "type": "boolean",
              "description": "Paste all text at once instead of human typing. WARNING: only use on targets without bot detection — using this on public websites risks the profile being flagged."
            },
            "wpm": {
              "type": "number",
              "description": "Target words per minute for human typing (default: 80)"
            }
          },
          "required": ["profile_id", "selector", "text"]
        }),
      },
      McpTool {
        name: "get_page_content".to_string(),
        description:
          "Get the content of the current page. Works with both static HTML and JavaScript-rendered content."
            .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "format": {
              "type": "string",
              "enum": ["html", "text"],
              "description": "Content format: 'html' for full HTML, 'text' for visible text only (default: text)"
            },
            "selector": {
              "type": "string",
              "description": "Optional CSS selector to get content of a specific element instead of the whole page"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "get_page_info".to_string(),
        description: "Get metadata about the current page including URL, title, and readiness state"
          .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "get_interactive_elements".to_string(),
        description: "Enumerate visible interactive elements on the page (buttons, links, inputs, etc.) as a compact indexed list. The returned indices are stable for the current page and can be used with click_by_index and type_by_index instead of guessing CSS selectors. Call this before click_by_index / type_by_index, and re-call after any navigation or major DOM change. Far cheaper in tokens than get_page_content for agentic browsing.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "max_chars": {
              "type": "integer",
              "description": "Cap on the serialized output length (default: 40000). The response carries a `truncated` flag if the list was cut off — narrow the viewport or scroll if you need elements past the cutoff."
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "click_by_index".to_string(),
        description: "Click the element at the given index from the last get_interactive_elements call. Indices are valid until the next navigation. If the click triggers navigation, waits for the new page to load before returning.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "index": {
              "type": "integer",
              "description": "Zero-based index from the last get_interactive_elements response"
            }
          },
          "required": ["profile_id", "index"]
        }),
      },
      McpTool {
        name: "type_by_index".to_string(),
        description: "Focus the element at the given index from the last get_interactive_elements call and type text into it. Same human-like-typing defaults as type_text; only set instant=true when you're sure the target lacks bot detection.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "index": {
              "type": "integer",
              "description": "Zero-based index from the last get_interactive_elements response"
            },
            "text": {
              "type": "string",
              "description": "Text to type into the element"
            },
            "clear_first": {
              "type": "boolean",
              "description": "Clear the input before typing (default: true)"
            },
            "instant": {
              "type": "boolean",
              "description": "Paste all text at once instead of human typing. WARNING: only use on targets without bot detection."
            },
            "wpm": {
              "type": "number",
              "description": "Target words per minute for human typing (default: 80)"
            }
          },
          "required": ["profile_id", "index", "text"]
        }),
      },
      // Remote fleet. An agent that could drive a remote profile but not start
      // one had to be handed a session by something else — the REST API or the
      // GUI — which is no use to an MCP client running on its own.
      McpTool {
        name: "run_profile_remote".to_string(),
        description: "Start this profile on a remote host of its own operating system. The profile must have Regular cloud sync enabled. Returns a session id; poll get_remote_session until state is 'live', then drive it with navigate, screenshot, click_element and the rest exactly as you would a local profile.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to run remotely"
            },
            "url": {
              "type": "string",
              "description": "Optional URL to open once the browser is up"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "stop_remote_session".to_string(),
        description: "Stop a remote session and settle what it cost. A session left running bills until the fleet's two-hour cap, so stop one as soon as you are done with it".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "session_id": {
              "type": "string",
              "description": "Session id returned by run_profile_remote"
            }
          },
          "required": ["session_id"]
        }),
      },
      // Observability. `run_profile_remote` hands back a session id and the
      // word "provisioning"; without these an agent can only learn that a
      // session became usable by trying to drive it and failing.
      McpTool {
        name: "list_remote_sessions".to_string(),
        description: "List the remote browser sessions this account currently owns, with their live status".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_remote_session".to_string(),
        description: "Read one remote session's real state: provisioning, ready, live or closed, plus whether it can be driven yet".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "session_id": {
              "type": "string",
              "description": "Session id returned when the remote session was started"
            }
          },
          "required": ["session_id"]
        }),
      },
      McpTool {
        name: "get_remote_hours_quota".to_string(),
        description: "Read the pooled remote-hour budget. Bot runs and interactive remote sessions spend the same pool".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      // Cookie bot. Every one of these is a proxy onto Donut cloud, which owns
      // the schedule and the browsing behaviour; the tools carry only the
      // user's own choices.
      McpTool {
        name: "list_cookie_bot_schedules".to_string(),
        description: "List profiles enrolled in the nightly cookie bot".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "scope": {
              "type": "string",
              "enum": ["mine", "team"],
              "description": "Whose enrolments to list (default: mine)"
            }
          },
          "required": []
        }),
      },
      McpTool {
        name: "get_cookie_bot_schedule".to_string(),
        description: "Get one profile's cookie-bot enrolment, or null when it is not enrolled".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "set_cookie_bot_schedule".to_string(),
        description: "Enrol a profile in the nightly cookie bot, or replace its enrolment. The profile must have cloud sync (not end-to-end encrypted), a recorded Windows, macOS or Linux operating system, and a proxy or VPN".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to enrol"
            },
            "profile_name": {
              "type": "string",
              "description": "Label shown in run history (default: the profile's own name)"
            },
            "platform": {
              "type": "string",
              "enum": ["windows", "macos", "linux"],
              "description": "Must match the profile's own operating system; taken from the profile when omitted"
            },
            "enabled": {
              "type": "boolean",
              "description": "Whether the nightly run is armed"
            },
            "run_at_minute": {
              "type": "integer",
              "description": "Minutes past local midnight, 0-1439"
            },
            "days_mask": {
              "type": "integer",
              "description": "Bitmask of local weekdays, bit 0 = Monday, 1-127"
            },
            "timezone": {
              "type": "string",
              "description": "IANA zone the run time is expressed in, e.g. Europe/Berlin"
            },
            "preset": {
              "type": "string",
              "description": "Preset id from list_cookie_bot_presets"
            },
            "max_minutes": {
              "type": "integer",
              "description": "Upper bound on one run, in minutes"
            },
            "sites": {
              "type": "array",
              "items": { "type": "string" },
              "description": "Absolute http(s) URLs to browse. The bot visits only these"
            },
            "jitter_seconds": {
              "type": "integer",
              "description": "Random spread around the run time, in seconds"
            },
            "acknowledge_conflict": {
              "type": "boolean",
              "description": "Write anyway when a teammate already enrols this profile"
            }
          },
          "required": ["profile_id", "enabled", "run_at_minute", "days_mask", "timezone", "preset", "max_minutes"]
        }),
      },
      McpTool {
        name: "delete_cookie_bot_schedule".to_string(),
        description: "Turn the cookie bot off for a profile. Safe to repeat; a run already in flight is not cancelled".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to unenrol"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "check_cookie_bot_conflicts".to_string(),
        description: "Ask, without writing anything, which teammates already enrol this profile and whether a proposed time would overlap theirs".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile"
            },
            "run_at_minute": {
              "type": "integer",
              "description": "Proposed minutes past local midnight, 0-1439"
            },
            "timezone": {
              "type": "string",
              "description": "Proposed IANA zone"
            },
            "days_mask": {
              "type": "integer",
              "description": "Proposed weekday bitmask, bit 0 = Monday"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "list_cookie_bot_runs".to_string(),
        description: "List cookie-bot runs, newest first, with how many sites each visited and what it cost".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "Restrict to one profile"
            },
            "scope": {
              "type": "string",
              "enum": ["mine", "team"],
              "description": "Whose runs to list (default: mine)"
            },
            "limit": {
              "type": "integer",
              "description": "Page size, 1-100 (default: 30)"
            },
            "before": {
              "type": "string",
              "description": "Keyset cursor from a previous page's next_before"
            }
          },
          "required": []
        }),
      },
      McpTool {
        name: "run_cookie_bot_now".to_string(),
        description: "Start a cookie-bot run immediately instead of waiting for the schedule. The profile must already be enrolled: the preset and site list live in its schedule. Requires an active Pro subscription and spends the pooled remote-hour budget".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the enrolled profile to warm"
            },
            "max_minutes": {
              "type": "integer",
              "description": "Cap this run only, overriding the schedule's own"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "cancel_cookie_bot_run".to_string(),
        description: "Stop a cookie-bot run that is still going. Idempotent: cancelling a finished run returns it unchanged".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "run_id": {
              "type": "string",
              "description": "Run id from list_cookie_bot_runs"
            }
          },
          "required": ["run_id"]
        }),
      },
      McpTool {
        name: "list_cookie_bot_presets".to_string(),
        description: "List the cookie-bot intensities that can be chosen, with roughly how long each takes".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_cookie_bot_usage".to_string(),
        description: "Per-member and per-profile cookie-bot spend for a calendar month. Reporting only".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "period": {
              "type": "string",
              "description": "Calendar month as YYYY-MM (default: the current UTC month)"
            }
          },
          "required": []
        }),
      },
      McpTool {
        name: "perceive_page".to_string(),
        description: "Read the page the way an agent needs it: every visible element with its role, accessible name, text, value, state and page-coordinate bounds, plus the readable text, in one call and with no script injected on Wayfern 152. Far more complete than get_interactive_elements. On a profile older than Wayfern 152 the same shape is synthesised from the DOM (engine: \"fallback\"). A truncated result carries a cursor; pass it back to continue.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "max_bytes": {
              "type": "integer",
              "description": "Total byte cap for nodes and text (default: 1048576, ceiling: 4194304)"
            },
            "budget_ms": {
              "type": "integer",
              "description": "Capture budget in milliseconds (default: 5000, clamped to 100-60000). Exceeding it truncates and paginates; it is never an error"
            },
            "max_nodes": {
              "type": "integer",
              "description": "Per-frame node ceiling (default: 100000; 0 for no limit)"
            },
            "include_text": {
              "type": "boolean",
              "description": "Include the readable text (default: true)"
            },
            "viewport_only": {
              "type": "boolean",
              "description": "Drop nodes outside the viewport (default: false)"
            },
            "text_order": {
              "type": "string",
              "enum": ["reading", "visual"],
              "description": "Emit text in accessibility reading order (default) or re-sorted by geometry (Wayfern 152; the fallback engine always answers in reading order)"
            },
            "cursor": {
              "type": "string",
              "description": "Continue a previous capture from the cursor it returned (Wayfern 152 only)"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "resolve_locator".to_string(),
        description: "Resolve a locator (role, accessible name, text, attributes) to EXACTLY ONE element and describe it. Ambiguity is an error whose data.candidates lists what matched, and a locator that matches nothing is an error too, so a click never lands on the wrong element. Use the result's signature or attributes to refine.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "locator": locator_schema("How to name the element. Every part given must match"),
            "candidate_limit": {
              "type": "integer",
              "description": "How many candidates an ambiguity error lists (default: 10, ceiling: 100)"
            }
          },
          "required": ["profile_id", "locator"]
        }),
      },
      McpTool {
        name: "click_locator".to_string(),
        description: "Resolve a locator to exactly one element and click it with a humanized pointer: on Wayfern 152 a real pointer glides to the element along a human path and presses with the profile's own timing (nothing is injected into the page); on older builds a trusted mouse event is dispatched at the element's centre. Waits for a page load when the click causes one.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "locator": locator_schema("How to name the element to click"),
            "button": {
              "type": "string",
              "enum": ["left", "middle", "right", "back", "forward"],
              "description": "Mouse button (default: left)"
            },
            "click_count": {
              "type": "integer",
              "description": "1 for a click (default), 2 for a double click, 3 for a triple"
            }
          },
          "required": ["profile_id", "locator"]
        }),
      },
      McpTool {
        name: "type_locator".to_string(),
        description: "Resolve a locator to exactly one field, focus it with a real click and type text into it one key at a time. On Wayfern 152 the keys are paced by the profile's own typing rhythm, with a few seed-determined typos corrected along the way when typos is on; on older builds the same human-typing model as type_text is used. The field is emptied first unless clear_first is false.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "locator": locator_schema("How to name the field to type into"),
            "text": {
              "type": "string",
              "description": "Text to type"
            },
            "clear_first": {
              "type": "boolean",
              "description": "Empty the field before typing (default: true)"
            },
            "typos": {
              "type": "boolean",
              "description": "Mistype and correct a few characters, as a hand does (default: true)"
            },
            "wpm": {
              "type": "number",
              "description": "Target words per minute for the fallback engine (default: 80). Wayfern 152 types at the profile's own rhythm and ignores this"
            }
          },
          "required": ["profile_id", "locator", "text"]
        }),
      },
      McpTool {
        name: "extract_structured".to_string(),
        description: "Read rows off the live page natively, with no script injected: a container locator matches every row, each field locator is evaluated inside a row, and an optional next-page locator is clicked to advance. A missing container is a result (stopReason \"no-container\"), not an error. Every bound is reported through stopReason and truncated. Requires Wayfern 152.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "container": locator_schema("Matches every row container; several matches are the expected case"),
            "field_map": {
              "type": "array",
              "description": "The columns to read from each row",
              "items": {
                "type": "object",
                "properties": {
                  "key": { "type": "string", "description": "The key this column appears under in each row's values" },
                  "locator": locator_schema("Evaluated inside each container; the first match wins. A field that matches nothing is an absent key"),
                  "source": { "type": "string", "enum": ["text", "attribute", "link"], "description": "Visible text, one named attribute, or the resolved href/src" },
                  "attribute": { "type": "string", "description": "The attribute to read; required when source is attribute" }
                },
                "required": ["key", "locator", "source"]
              }
            },
            "next_page": locator_schema("The control clicked to advance a page; absent means one page"),
            "max_pages": { "type": "integer", "description": "Default 1, ceiling 200" },
            "max_rows": { "type": "integer", "description": "Default 1000, ceiling 100000" },
            "max_bytes": { "type": "integer", "description": "Default 262144, ceiling 8388608" },
            "max_nodes": { "type": "integer", "description": "Node cap for each snapshot (default: 20000, ceiling: 200000)" },
            "time_budget_ms": { "type": "integer", "description": "Default 8000, ceiling 120000" }
          },
          "required": ["profile_id", "container", "field_map"]
        }),
      },
      McpTool {
        name: "pick_element".to_string(),
        description: "Arm the browser's element picker and wait for the user to click an element in the page. Returns the smallest locator that resolves to what they clicked, plus its description, so a person can point at something an agent then acts on with click_locator or type_locator. Errors when the user presses Escape, navigates away, or nothing is picked within timeout_ms. Requires Wayfern 152.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the running profile"
            },
            "timeout_ms": {
              "type": "integer",
              "description": "How long to wait for the click (default: 60000, ceiling: 300000)"
            }
          },
          "required": ["profile_id"]
        }),
      },
    ]
  }

  async fn handle_initialize(
    &self,
    request: McpRequest,
  ) -> Result<(String, (serde_json::Value, serde_json::Value)), (serde_json::Value, McpError)> {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);

    if !self.is_engine_ready() {
      return Err((
        id,
        McpError {
          code: -32001,
          message: "MCP server is not running".to_string(),
          data: None,
        },
      ));
    }

    let negotiated = negotiate_protocol_version(
      request
        .params
        .as_ref()
        .and_then(|params| params.get("protocolVersion"))
        .and_then(serde_json::Value::as_str),
    );

    // Create session
    let session_id = Uuid::new_v4().to_string();
    {
      let mut inner = self.inner.lock().await;
      inner.sessions.insert(
        session_id.clone(),
        McpSession {
          initialized: false,
          last_used: std::time::Instant::now(),
          cached_pages: HashSet::new(),
        },
      );

      // Evict the oldest rather than refusing the newest: a caller that just
      // asked for a session is the one actually present, and refusing it would
      // break a live customer to protect memory that is not under pressure.
      //
      // An evicted session's page snapshots are NOT deleted here. Doing it
      // would hold this lock, and the caller's `initialize`, on CDP round trips
      // to browsers that may be gone; the page-side slot cap
      // (MAX_CACHE_SLOTS_PER_PAGE) is what bounds them in exactly this case.
      while inner.sessions.len() > MAX_SESSIONS {
        let Some(oldest) = inner
          .sessions
          .iter()
          .min_by_key(|(_, session)| session.last_used)
          .map(|(id, _)| id.clone())
        else {
          break;
        };
        log::warn!("[mcp] Session cap reached; evicting the oldest session");
        inner.sessions.remove(&oldest);
      }
    }

    let result = serde_json::json!({
      "protocolVersion": negotiated,
      "capabilities": {
        "tools": {
          "listChanged": false
        }
      },
      "serverInfo": {
        "name": SERVER_NAME,
        "version": SERVER_VERSION,
      },
      "instructions": "Donut Browser MCP server. Use tools/list to discover available browser automation tools."
    });

    log::info!("[mcp] New session initialized: {}", ShortId(&session_id));
    Ok((session_id, (id, result)))
  }

  pub async fn handle_request(&self, caller: McpCaller<'_>, request: McpRequest) -> McpResponse {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);

    if !self.is_engine_ready() {
      return McpResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(id),
        result: None,
        error: Some(McpError {
          code: -32001,
          message: "MCP server is not running".to_string(),
          data: None,
        }),
      };
    }

    let result = match request.method.as_str() {
      "ping" => Ok(serde_json::json!({})),
      "tools/list" => self.handle_tools_list().await,
      "tools/call" => self.handle_tool_call(caller, request.params).await,
      _ => Err(McpError {
        code: -32601,
        message: format!("Method not found: {}", request.method),
        data: None,
      }),
    };

    match result {
      Ok(value) => McpResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(id),
        result: Some(value),
        error: None,
      },
      Err(error) => McpResponse {
        jsonrpc: "2.0".to_string(),
        id: Some(id),
        result: None,
        error: Some(error),
      },
    }
  }

  async fn handle_tools_list(&self) -> Result<serde_json::Value, McpError> {
    Ok(serde_json::json!({
      "tools": self.get_tools()
    }))
  }

  async fn handle_tool_call(
    &self,
    caller: McpCaller<'_>,
    params: Option<serde_json::Value>,
  ) -> Result<serde_json::Value, McpError> {
    let params = params.ok_or_else(|| McpError {
      code: -32602,
      message: "Missing parameters".to_string(),
      data: None,
    })?;

    let tool_name = params
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing tool name".to_string(),
        data: None,
      })?;

    let arguments = params
      .get("arguments")
      .cloned()
      .unwrap_or(serde_json::json!({}));

    // Surface the call in logs so customer reports show which tools the MCP
    // client is actually invoking (and therefore which gate any subsequent
    // error came from). Log only the tool name and the profile_id arg —
    // arbitrary URLs / JS / selectors can be sensitive.
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .unwrap_or("<none>");
    log::info!("[mcp] tools/call name={tool_name} profile_id={profile_id}");

    let started = std::time::Instant::now();
    // Refused here, at the one place every tool call passes, rather than in
    // each handler: a new path-taking tool added later inherits the rule
    // instead of having to remember it.
    if caller.origin == McpOrigin::Bridge
      && (LOCAL_PATH_TOOLS.contains(&tool_name) || SECRET_EXPORT_TOOLS.contains(&tool_name))
    {
      log::warn!(
        "[mcp] Refused '{tool_name}' over the bridge: it takes a local filesystem path or exports stored secrets"
      );
      return Err(McpError {
        code: -32000,
        message: crate::backend_error("TOOL_IS_LOCAL_ONLY"),
        data: None,
      });
    }

    let result = self.dispatch_tool_call(caller, tool_name, &arguments).await;
    let elapsed_ms = started.elapsed().as_millis();
    match &result {
      Ok(_) => {
        log::info!(
          "[mcp] tools/call name={tool_name} profile_id={profile_id} -> ok ({elapsed_ms} ms)"
        );
      }
      Err(e) => {
        log::warn!(
          "[mcp] tools/call name={tool_name} profile_id={profile_id} -> error code={} msg={:?} ({elapsed_ms} ms)",
          e.code,
          e.message
        );
      }
    }
    result
  }

  async fn dispatch_tool_call(
    &self,
    caller: McpCaller<'_>,
    tool_name: &str,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    match tool_name {
      "list_profiles" => self.handle_list_profiles().await,
      "get_profile" => self.handle_get_profile(arguments).await,
      "run_profile" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_run_profile(arguments).await
      }
      "kill_profile" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_kill_profile(arguments).await
      }
      "batch_run_profiles" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_batch_run_profiles(arguments).await
      }
      "batch_stop_profiles" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_batch_stop_profiles(arguments).await
      }
      "create_profile" => self.handle_create_profile(arguments).await,
      // Profile import (free, like create_profile — importing is not automation)
      "detect_browser_profiles" => self.handle_detect_browser_profiles(arguments).await,
      "import_browser_profiles" => self.handle_import_browser_profiles(arguments).await,
      "update_profile" => self.handle_update_profile(arguments).await,
      "delete_profile" => self.handle_delete_profile(arguments).await,
      "list_tags" => self.handle_list_tags().await,
      "list_proxies" => self.handle_list_proxies(caller).await,
      "get_profile_status" => self.handle_get_profile_status(arguments).await,
      // Group management
      "list_groups" => self.handle_list_groups().await,
      "get_group" => self.handle_get_group(arguments).await,
      "create_group" => self.handle_create_group(arguments).await,
      "update_group" => self.handle_update_group(arguments).await,
      "delete_group" => self.handle_delete_group(arguments).await,
      "assign_profiles_to_group" => self.handle_assign_profiles_to_group(arguments).await,
      "distribute_proxies" => self.handle_distribute_proxies(arguments).await,
      // Full proxy management
      "get_proxy" => self.handle_get_proxy(caller, arguments).await,
      "create_proxy" => self.handle_create_proxy(arguments).await,
      "update_proxy" => self.handle_update_proxy(arguments).await,
      "delete_proxy" => self.handle_delete_proxy(arguments).await,
      // Proxy import/export
      "export_proxies" => self.handle_export_proxies(arguments).await,
      "import_proxies" => self.handle_import_proxies(arguments).await,
      // VPN management
      "import_vpn" => self.handle_import_vpn(arguments).await,
      "list_vpn_configs" => self.handle_list_vpn_configs().await,
      "delete_vpn" => self.handle_delete_vpn(arguments).await,
      "connect_vpn" => self.handle_connect_vpn(arguments).await,
      "disconnect_vpn" => self.handle_disconnect_vpn(arguments).await,
      "get_vpn_status" => self.handle_get_vpn_status(arguments).await,
      // Fingerprint management — viewing is free everywhere (matches the REST
      // API and the get_profile tool, which already expose the config); only
      // editing requires a paid plan.
      "get_profile_fingerprint" => self.handle_get_profile_fingerprint(arguments).await,
      "update_profile_fingerprint" => {
        Self::require_capability(
          "Fingerprint editing",
          CLOUD_AUTH.can_use_cross_os_fingerprints().await,
        )
        .await?;
        self.handle_update_profile_fingerprint(arguments).await
      }
      "update_profile_proxy_bypass_rules" => {
        self
          .handle_update_profile_proxy_bypass_rules(arguments)
          .await
      }
      // DNS blocklist management
      "update_profile_dns_blocklist" => self.handle_update_profile_dns_blocklist(arguments).await,
      "get_dns_blocklist_status" => self.handle_get_dns_blocklist_status().await,
      // Extension management
      "list_extensions" => self.handle_list_extensions().await,
      "list_extension_groups" => self.handle_list_extension_groups().await,
      "add_extension" => self.handle_add_extension(arguments).await,
      "update_extension" => self.handle_update_extension(arguments).await,
      "create_extension_group" => self.handle_create_extension_group(arguments).await,
      "update_extension_group" => self.handle_update_extension_group(arguments).await,
      "add_extension_to_group" => self.handle_add_extension_to_group(arguments).await,
      "remove_extension_from_group" => self.handle_remove_extension_from_group(arguments).await,
      "delete_extension" => self.handle_delete_extension_mcp(arguments).await,
      "delete_extension_group" => self.handle_delete_extension_group_mcp(arguments).await,
      "assign_extension_group_to_profile" => {
        self
          .handle_assign_extension_group_to_profile(arguments)
          .await
      }
      // Cookie management
      "import_profile_cookies" => self.handle_import_profile_cookies(arguments).await,
      // Team lock tools
      "get_team_locks" => self.handle_get_team_locks().await,
      "get_team_lock_status" => self.handle_get_team_lock_status(arguments).await,
      // Synchronizer tools
      "start_sync_session" => {
        Self::require_capability(
          "Synchronizer",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_start_sync_session(arguments).await
      }
      "stop_sync_session" => self.handle_stop_sync_session(arguments).await,
      "get_sync_sessions" => self.handle_get_sync_sessions().await,
      "remove_sync_follower" => self.handle_remove_sync_follower(arguments).await,
      // Browser interaction tools (require paid subscription)
      "navigate" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_navigate(arguments).await
      }
      "screenshot" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_screenshot(arguments).await
      }
      "evaluate_javascript" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_evaluate_javascript(arguments).await
      }
      "click_element" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_click_element(arguments).await
      }
      "type_text" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_type_text(caller, arguments).await
      }
      "get_page_content" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_get_page_content(arguments).await
      }
      "get_page_info" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_get_page_info(arguments).await
      }
      "get_interactive_elements" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self
          .handle_get_interactive_elements(caller, arguments)
          .await
      }
      "click_by_index" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_click_by_index(caller, arguments).await
      }
      "type_by_index" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_type_by_index(caller, arguments).await
      }
      // The agent surface: perception, locators, extraction, the picker and
      // humanized input. Gated exactly like click_element, because every one
      // of them drives, or reads, the same browser.
      "perceive_page" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_perceive_page(arguments).await
      }
      "resolve_locator" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_resolve_locator(arguments).await
      }
      "click_locator" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_click_locator(arguments).await
      }
      "type_locator" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_type_locator(caller, arguments).await
      }
      "extract_structured" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_extract_structured(caller, arguments).await
      }
      "pick_element" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_pick_element(caller, arguments).await
      }
      // Leasing a host is the most expensive thing this server can do, so it
      // is gated exactly like the local launch it replaces.
      "run_profile_remote" => {
        Self::require_capability(
          "Browser automation",
          CLOUD_AUTH.can_use_browser_automation().await,
        )
        .await?;
        self.handle_run_profile_remote(arguments).await
      }
      // No capability gate on the stop. A lapsed plan must never be the reason
      // an agent cannot end something that is spending hours.
      "stop_remote_session" => Self::handle_stop_remote_session(arguments).await,
      // Remote fleet observability. Reads only, and free: being unable to see
      // that a session you are already paying for has become usable is not a
      // feature worth withholding.
      "list_remote_sessions" => Self::handle_list_remote_sessions().await,
      "get_remote_session" => Self::handle_get_remote_session(arguments).await,
      "get_remote_hours_quota" => Self::handle_get_remote_hours_quota().await,
      // Cookie bot. Reading and configuring are free; only starting a run,
      // which leases a host and spends the pooled hours, needs the plan.
      "list_cookie_bot_schedules" => Self::handle_list_cookie_bot_schedules(arguments).await,
      "get_cookie_bot_schedule" => Self::handle_get_cookie_bot_schedule(arguments).await,
      "set_cookie_bot_schedule" => Self::handle_set_cookie_bot_schedule(arguments).await,
      "delete_cookie_bot_schedule" => Self::handle_delete_cookie_bot_schedule(arguments).await,
      "check_cookie_bot_conflicts" => Self::handle_check_cookie_bot_conflicts(arguments).await,
      "list_cookie_bot_runs" => Self::handle_list_cookie_bot_runs(arguments).await,
      "run_cookie_bot_now" => {
        // The Cookie Bot, NOT browser automation. Solo pays for the bot and has
        // no automation; gating this on automation refused a Solo customer the
        // feature their plan is sold on while their scheduled runs kept firing.
        Self::require_capability("Cookie Bot", CLOUD_AUTH.can_use_cookie_bot().await).await?;
        Self::handle_run_cookie_bot_now(arguments).await
      }
      // No capability gate on the cancel. A lapsed plan must never be the
      // reason an agent cannot stop something that is spending hours.
      "cancel_cookie_bot_run" => Self::handle_cancel_cookie_bot_run(arguments).await,
      "list_cookie_bot_presets" => Self::handle_list_cookie_bot_presets().await,
      "get_cookie_bot_usage" => Self::handle_get_cookie_bot_usage(arguments).await,
      _ => Err(McpError {
        code: -32602,
        message: format!("Unknown tool: {tool_name}"),
        data: None,
      }),
    }
  }

  async fn handle_list_profiles(&self) -> Result<serde_json::Value, McpError> {
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    // Filter to only Wayfern profiles
    let filtered: Vec<&BrowserProfile> =
      profiles.iter().filter(|p| p.browser == "wayfern").collect();

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&filtered).unwrap_or_default()
      }]
    }))
  }

  async fn handle_get_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
        data: None,
      })?;

    // Check if it's a Wayfern profile
    if profile.browser != "wayfern" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern profiles".to_string(),
        data: None,
      });
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&profile).unwrap_or_default()
      }]
    }))
  }

  async fn handle_run_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    // Launching profiles programmatically requires the automation capability.
    Self::require_capability(
      "Launching a profile",
      CLOUD_AUTH.can_use_browser_automation().await,
    )
    .await?;

    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    let url = arguments.get("url").and_then(|v| v.as_str());
    if let Some(url) = url {
      validate_navigable_url(url)?;
    }
    let headless = arguments
      .get("headless")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    // Get the profile
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
        data: None,
      })?;

    // Check if it's a Wayfern profile
    if profile.browser != "wayfern" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern profiles".to_string(),
        data: None,
      });
    }

    // Team lock check
    crate::team_lock::acquire_team_lock_if_needed(profile)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e,
        data: None,
      })?;

    // Get app handle to launch
    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    // Launch a fresh instance, honoring the requested headless mode. The CDP
    // port is self-allocated and discovered later via get_cdp_port_for_profile.
    crate::browser_runner::launch_browser_profile_impl(
      app_handle.clone(),
      profile.clone(),
      url.map(|s| s.to_string()),
      crate::browser_runner::LaunchOptions::automation(None, headless),
    )
    .await
    .map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to launch browser: {e}"),
      data: None,
    })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Browser profile '{}' launched successfully", profile.name)
      }]
    }))
  }

  async fn handle_kill_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    // Stopping profiles programmatically requires the automation capability.
    Self::require_capability(
      "Killing a profile",
      CLOUD_AUTH.can_use_browser_automation().await,
    )
    .await?;

    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    // Get the profile
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
        data: None,
      })?;

    // Check if it's a Wayfern profile
    if profile.browser != "wayfern" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern profiles".to_string(),
        data: None,
      });
    }

    // Get app handle to kill
    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    // Kill the browser
    crate::browser_runner::BrowserRunner::instance()
      .kill_browser_process(app_handle.clone(), profile)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to kill browser: {e}"),
        data: None,
      })?;

    crate::team_lock::release_team_lock_if_needed(profile).await;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Browser profile '{}' stopped successfully", profile.name)
      }]
    }))
  }

  async fn handle_batch_run_profiles(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    Self::require_capability(
      "Batch launching profiles",
      CLOUD_AUTH.can_use_browser_automation().await,
    )
    .await?;

    let profile_ids: Vec<String> = arguments
      .get("profile_ids")
      .and_then(|v| v.as_array())
      .map(|a| {
        a.iter()
          .filter_map(|v| v.as_str().map(|s| s.to_string()))
          .collect()
      })
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_ids array".to_string(),
        data: None,
      })?;

    let url = arguments.get("url").and_then(|v| v.as_str());
    if let Some(url) = url {
      validate_navigable_url(url)?;
    }
    let headless = arguments
      .get("headless")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    // Clone the app handle and release the lock before the launch loop so we
    // never hold the inner mutex across the per-profile awaits.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let mut launched = 0usize;
    let mut lines: Vec<String> = Vec::with_capacity(profile_ids.len());
    for profile_id in &profile_ids {
      let Some(profile) = profiles.iter().find(|p| p.id.to_string() == *profile_id) else {
        lines.push(format!("{profile_id}: not found"));
        continue;
      };
      if profile.browser != "wayfern" {
        lines.push(format!(
          "{profile_id}: unsupported browser (MCP supports Wayfern)"
        ));
        continue;
      }
      if let Err(e) = crate::team_lock::acquire_team_lock_if_needed(profile).await {
        lines.push(format!("{profile_id}: {e}"));
        continue;
      }
      match crate::browser_runner::launch_browser_profile_impl(
        app_handle.clone(),
        profile.clone(),
        url.map(|s| s.to_string()),
        crate::browser_runner::LaunchOptions::automation(None, headless),
      )
      .await
      {
        Ok(_) => {
          launched += 1;
          lines.push(format!("{}: launched", profile.name));
        }
        Err(e) => lines.push(format!("{}: launch failed: {e}", profile.name)),
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Launched {}/{} profile(s):\n{}", launched, profile_ids.len(), lines.join("\n"))
      }]
    }))
  }

  async fn handle_batch_stop_profiles(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    Self::require_capability(
      "Batch stopping profiles",
      CLOUD_AUTH.can_use_browser_automation().await,
    )
    .await?;

    let profile_ids: Vec<String> = arguments
      .get("profile_ids")
      .and_then(|v| v.as_array())
      .map(|a| {
        a.iter()
          .filter_map(|v| v.as_str().map(|s| s.to_string()))
          .collect()
      })
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_ids array".to_string(),
        data: None,
      })?;

    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let mut stopped = 0usize;
    let mut lines: Vec<String> = Vec::with_capacity(profile_ids.len());
    for profile_id in &profile_ids {
      let Some(profile) = profiles.iter().find(|p| p.id.to_string() == *profile_id) else {
        lines.push(format!("{profile_id}: not found"));
        continue;
      };
      match crate::browser_runner::BrowserRunner::instance()
        .kill_browser_process(app_handle.clone(), profile)
        .await
      {
        Ok(_) => {
          crate::team_lock::release_team_lock_if_needed(profile).await;
          stopped += 1;
          lines.push(format!("{}: stopped", profile.name));
        }
        Err(e) => lines.push(format!("{}: stop failed: {e}", profile.name)),
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Stopped {}/{} profile(s):\n{}", stopped, profile_ids.len(), lines.join("\n"))
      }]
    }))
  }

  async fn handle_create_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing name".to_string(),
        data: None,
      })?;
    let browser = arguments
      .get("browser")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing browser".to_string(),
        data: None,
      })?;

    if browser != "wayfern" {
      return Err(McpError {
        code: -32602,
        message: "browser must be 'wayfern'".to_string(),
        data: None,
      });
    }

    let proxy_id = arguments
      .get("proxy_id")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());
    let launch_hook = arguments
      .get("launch_hook")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());
    let tags: Option<Vec<String>> = arguments.get("tags").and_then(|v| {
      v.as_array().map(|arr| {
        arr
          .iter()
          .filter_map(|item| item.as_str().map(|s| s.to_string()))
          .collect()
      })
    });

    // Pick the latest downloaded version for this browser
    let registry = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
    let versions = registry.get_downloaded_versions(browser);
    let version = versions.first().ok_or_else(|| McpError {
      code: -32000,
      message: format!("No downloaded version found for {browser}. Download it first."),
      data: None,
    })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let temporary = arguments
      .get("temporary")
      .and_then(serde_json::Value::as_bool)
      .unwrap_or(false);
    let ephemeral = temporary
      || arguments
        .get("ephemeral")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let mut profile = ProfileManager::instance()
      .create_profile_with_group(
        &app_handle,
        name,
        browser,
        version,
        "stable",
        proxy_id,
        None,
        None,
        group_id,
        ephemeral,
        None,
        launch_hook,
      )
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to create profile: {e}"),
        data: None,
      })?;

    if temporary {
      profile = ProfileManager::instance()
        .mark_profile_temporary(&profile.id.to_string())
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Profile created but could not be marked temporary: {e}"),
          data: None,
        })?;
    }

    if let Some(tags) = tags {
      let _ =
        ProfileManager::instance().update_profile_tags(&app_handle, &profile.name, tags.clone());
      profile.tags = tags;
      if let Ok(profiles) = ProfileManager::instance().list_profiles() {
        let _ = crate::tag_manager::TAG_MANAGER
          .lock()
          .map(|manager| manager.rebuild_from_profiles(&profiles));
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Profile '{}' created (id: {})", profile.name, profile.id)
      }]
    }))
  }

  async fn handle_update_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };
    let pm = ProfileManager::instance();

    if let Some(new_name) = arguments.get("name").and_then(|v| v.as_str()) {
      pm.rename_profile(&app_handle, profile_id, new_name)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to rename profile: {e}"),
          data: None,
        })?;
    }

    if let Some(proxy_id) = arguments.get("proxy_id").and_then(|v| v.as_str()) {
      // An empty id detaches the proxy; the manager normalizes it.
      pm.update_profile_proxy(app_handle.clone(), profile_id, Some(proxy_id.to_string()))
        .await
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to update proxy: {e}"),
          data: None,
        })?;
    }

    if let Some(launch_hook) = arguments.get("launch_hook").and_then(|v| v.as_str()) {
      let normalized = if launch_hook.is_empty() {
        None
      } else {
        Some(launch_hook.to_string())
      };
      pm.update_profile_launch_hook(&app_handle, profile_id, normalized)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to update launch hook: {e}"),
          data: None,
        })?;
    }

    if let Some(group_id) = arguments.get("group_id").and_then(|v| v.as_str()) {
      let gid = if group_id.is_empty() {
        None
      } else {
        Some(group_id.to_string())
      };
      pm.assign_profiles_to_group(&app_handle, vec![profile_id.to_string()], gid)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to update group: {e}"),
          data: None,
        })?;
    }

    if let Some(tags) = arguments.get("tags").and_then(|v| v.as_array()) {
      let tag_list: Vec<String> = tags
        .iter()
        .filter_map(|item| item.as_str().map(|s| s.to_string()))
        .collect();
      pm.update_profile_tags(&app_handle, profile_id, tag_list)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to update tags: {e}"),
          data: None,
        })?;
      if let Ok(profiles) = pm.list_profiles() {
        let _ = crate::tag_manager::TAG_MANAGER
          .lock()
          .map(|manager| manager.rebuild_from_profiles(&profiles));
      }
    }

    if let Some(ext_group_id) = arguments.get("extension_group_id").and_then(|v| v.as_str()) {
      let eid = if ext_group_id.is_empty() {
        None
      } else {
        Some(ext_group_id.to_string())
      };
      pm.update_profile_extension_group(profile_id, eid)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to update extension group: {e}"),
          data: None,
        })?;
    }

    if let Some(rules) = arguments
      .get("proxy_bypass_rules")
      .and_then(|v| v.as_array())
    {
      let rule_list: Vec<String> = rules
        .iter()
        .filter_map(|item| item.as_str().map(|s| s.to_string()))
        .collect();
      pm.update_profile_proxy_bypass_rules(&app_handle, profile_id, rule_list)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to update proxy bypass rules: {e}"),
          data: None,
        })?;
    }

    if let Some(clear_on_close) = arguments.get("clear_on_close").and_then(|v| v.as_bool()) {
      pm.update_profile_clear_on_close(&app_handle, profile_id, clear_on_close)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to update clear-on-close: {e}"),
          data: None,
        })?;
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Profile '{profile_id}' updated successfully")
      }]
    }))
  }

  async fn handle_delete_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    ProfileManager::instance()
      .delete_profile(&app_handle, profile_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to delete profile: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Profile '{profile_id}' deleted successfully")
      }]
    }))
  }

  async fn handle_list_tags(&self) -> Result<serde_json::Value, McpError> {
    let tags = crate::tag_manager::TAG_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to access tag manager: {e}"),
        data: None,
      })?
      .get_all_tags()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to get tags: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&tags).unwrap_or_default()
      }]
    }))
  }

  async fn handle_list_proxies(
    &self,
    caller: McpCaller<'_>,
  ) -> Result<serde_json::Value, McpError> {
    let mut proxies =
      serde_json::to_value(PROXY_MANAGER.get_stored_proxies()).map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to serialize proxies: {e}"),
        data: None,
      })?;
    if caller.origin == McpOrigin::Bridge {
      for proxy in proxies.as_array_mut().into_iter().flatten() {
        redact_proxy_secrets(proxy);
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&proxies).unwrap_or_default()
      }]
    }))
  }

  async fn handle_get_profile_status(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    // Get the profile
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
        data: None,
      })?;

    // Check if it's a Wayfern profile
    if profile.browser != "wayfern" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern profiles".to_string(),
        data: None,
      });
    }

    // "Running" has to mean "drivable", not "has a process on this machine".
    // A profile open on the leased fleet has no local process, and answering
    // `is_running: false` for it tells an agent not to bother calling the very
    // tools that would have worked.
    let is_running_locally = profile.process_id.is_some();
    let remote_session = if is_running_locally {
      None
    } else {
      crate::remote_session::live_session_for_profile(profile_id).await
    };

    let location = match (is_running_locally, &remote_session) {
      (true, _) => "local",
      (false, Some(_)) => "remote",
      (false, None) => "stopped",
    };

    // Whether a LOCAL launch would be refused, and why. An agent that reads
    // `location: "stopped"` and calls `run_profile` on a profile whose finished
    // remote session has not been pulled back would get a bare 409 with nothing
    // to act on; worse, before the gate existed it would have got a browser and
    // silently destroyed the session's work.
    let handoff = crate::remote_handoff::state_for(profile_id);
    let can_launch_locally = handoff.is_none() && !is_running_locally;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::json!({
          "profile_id": profile_id,
          "is_running": location != "stopped",
          "is_running_locally": is_running_locally,
          "location": location,
          "remote_session_id": remote_session.map(|session| session.session_id),
          "can_launch_locally": can_launch_locally,
          "local_launch_blocked_by": match handoff {
            Some(crate::remote_handoff::HandoffState::Running) => Some("remote_session_running"),
            Some(crate::remote_handoff::HandoffState::PendingSync) => {
              Some("remote_session_changes_downloading")
            }
            None => None,
          },
        }).to_string()
      }]
    }))
  }

  // Group management handlers
  async fn handle_list_groups(&self) -> Result<serde_json::Value, McpError> {
    let groups = GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
        data: None,
      })?
      .get_all_groups()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list groups: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&groups).unwrap_or_default()
      }]
    }))
  }

  async fn handle_get_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing group_id".to_string(),
        data: None,
      })?;

    let groups = GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
        data: None,
      })?
      .get_all_groups()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list groups: {e}"),
        data: None,
      })?;

    let group = groups
      .iter()
      .find(|g| g.id == group_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Group not found: {group_id}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&group).unwrap_or_default()
      }]
    }))
  }

  async fn handle_create_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing name".to_string(),
        data: None,
      })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let group = GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
        data: None,
      })?
      .create_group(&app_handle, name.to_string())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to create group: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Group '{}' created successfully with ID: {}", group.name, group.id)
      }]
    }))
  }

  async fn handle_update_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing group_id".to_string(),
        data: None,
      })?;

    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing name".to_string(),
        data: None,
      })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let group = GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
        data: None,
      })?
      .update_group(&app_handle, group_id.to_string(), name.to_string())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to update group: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Group '{}' updated successfully", group.name)
      }]
    }))
  }

  async fn handle_delete_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing group_id".to_string(),
        data: None,
      })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
        data: None,
      })?
      .delete_group(&app_handle, group_id.to_string())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to delete group: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Group '{}' deleted successfully", group_id)
      }]
    }))
  }

  async fn handle_assign_profiles_to_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_ids: Vec<String> = arguments
      .get("profile_ids")
      .and_then(|v| v.as_array())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_ids".to_string(),
        data: None,
      })?
      .iter()
      .filter_map(|v| v.as_str().map(|s| s.to_string()))
      .collect();

    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    ProfileManager::instance()
      .assign_profiles_to_group(&app_handle, profile_ids.clone(), group_id.clone())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to assign profiles to group: {e}"),
        data: None,
      })?;

    let group_name = group_id.as_deref().unwrap_or("default");
    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("{} profile(s) assigned to group '{}'", profile_ids.len(), group_name)
      }]
    }))
  }

  async fn handle_distribute_proxies(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let pairs: Vec<crate::proxy_distribution::ProxyPair> = arguments
      .get("pairs")
      .cloned()
      .map(serde_json::from_value)
      .transpose()
      .map_err(|e| McpError {
        code: -32602,
        message: format!("Invalid pairs: {e}"),
        data: None,
      })?
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing pairs".to_string(),
        data: None,
      })?;

    // Same reason the batch handlers clone and drop: the engine's single global
    // mutex serves every other MCP message, and fifty profile writes must not
    // hold it.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let results = crate::proxy_distribution::apply_pairs(app_handle, &pairs).await;
    let assigned = results.iter().filter(|result| result.ok).count();
    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "{assigned} of {} profile(s) assigned a proxy:\n{}",
          results.len(),
          serde_json::to_string_pretty(&results).unwrap_or_default()
        )
      }]
    }))
  }

  // Full proxy management handlers
  async fn handle_get_proxy(
    &self,
    caller: McpCaller<'_>,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let proxy_id = arguments
      .get("proxy_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing proxy_id".to_string(),
        data: None,
      })?;

    let proxies = PROXY_MANAGER.get_stored_proxies();
    let proxy = proxies
      .iter()
      .find(|p| p.id == proxy_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Proxy not found: {proxy_id}"),
        data: None,
      })?;

    let mut proxy = serde_json::to_value(proxy).map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to serialize proxy: {e}"),
      data: None,
    })?;
    if caller.origin == McpOrigin::Bridge {
      redact_proxy_secrets(&mut proxy);
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&proxy).unwrap_or_default()
      }]
    }))
  }

  async fn handle_create_proxy(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing name".to_string(),
        data: None,
      })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let proxy_type = arguments
      .get("proxy_type")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing proxy_type".to_string(),
        data: None,
      })?;

    // The tool schema declares an enum, but JSON-Schema enums are advisory only;
    // enforce it here so a bad value can't produce a non-functional proxy.
    if !matches!(
      proxy_type,
      "http" | "https" | "httpstls" | "socks4" | "socks5" | "vless"
    ) {
      return Err(McpError {
        code: -32602,
        message: "proxy_type must be one of: http, https, httpstls, socks4, socks5, vless"
          .to_string(),
        data: None,
      });
    }

    let vless_uri = arguments
      .get("vless_uri")
      .and_then(|value| value.as_str())
      .map(str::to_string);
    let (host, port) = if proxy_type == "vless" {
      if vless_uri.is_none() {
        return Err(McpError {
          code: -32602,
          message: "Missing vless_uri".to_string(),
          data: None,
        });
      }
      (String::new(), 1)
    } else {
      let host = arguments
        .get("host")
        .and_then(|value| value.as_str())
        .ok_or_else(|| McpError {
          code: -32602,
          message: "Missing host".to_string(),
          data: None,
        })?
        .to_string();
      let port = arguments
        .get("port")
        .and_then(|value| value.as_u64())
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| McpError {
          code: -32602,
          message: "Missing or invalid port".to_string(),
          data: None,
        })?;
      (host, port)
    };

    let username = arguments
      .get("username")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());
    let password = arguments
      .get("password")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    let proxy_settings = ProxySettings {
      proxy_type: proxy_type.to_string(),
      host,
      port,
      username,
      password,
      vless_uri,
    };

    let proxy = PROXY_MANAGER
      .create_stored_proxy(&app_handle, name.to_string(), proxy_settings)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to create proxy: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Proxy '{}' created successfully with ID: {}", proxy.name, proxy.id)
      }]
    }))
  }

  async fn handle_update_proxy(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let proxy_id = arguments
      .get("proxy_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing proxy_id".to_string(),
        data: None,
      })?;

    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    // Build proxy_settings if any settings fields are provided
    let has_settings = arguments.get("proxy_type").is_some()
      || arguments.get("host").is_some()
      || arguments.get("port").is_some()
      || arguments.get("username").is_some()
      || arguments.get("password").is_some()
      || arguments.get("vless_uri").is_some();

    let proxy_settings = if has_settings {
      // Get existing proxy to use as defaults
      let proxies = PROXY_MANAGER.get_stored_proxies();
      let existing = proxies
        .iter()
        .find(|p| p.id == proxy_id)
        .ok_or_else(|| McpError {
          code: -32000,
          message: format!("Proxy not found: {proxy_id}"),
          data: None,
        })?;

      let proxy_type = arguments
        .get("proxy_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| existing.proxy_settings.proxy_type.clone());
      if !matches!(
        proxy_type.as_str(),
        "http" | "https" | "httpstls" | "socks4" | "socks5" | "vless"
      ) {
        return Err(McpError {
          code: -32602,
          message: "proxy_type must be one of: http, https, httpstls, socks4, socks5, vless"
            .to_string(),
          data: None,
        });
      }

      let host = arguments
        .get("host")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| existing.proxy_settings.host.clone());

      let port = match arguments.get("port") {
        Some(value) => value
          .as_u64()
          .and_then(|port| u16::try_from(port).ok())
          .filter(|port| *port != 0)
          .ok_or_else(|| McpError {
            code: -32602,
            message: "Invalid port".to_string(),
            data: None,
          })?,
        None => existing.proxy_settings.port,
      };

      let username = arguments
        .get("username")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| existing.proxy_settings.username.clone());

      let password = arguments
        .get("password")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| existing.proxy_settings.password.clone());
      let vless_uri = arguments
        .get("vless_uri")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| existing.proxy_settings.vless_uri.clone());

      Some(ProxySettings {
        proxy_type,
        host,
        port,
        username,
        password,
        vless_uri,
      })
    } else {
      None
    };

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let proxy = PROXY_MANAGER
      .update_stored_proxy(&app_handle, proxy_id, name, proxy_settings)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to update proxy: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Proxy '{}' updated successfully", proxy.name)
      }]
    }))
  }

  async fn handle_delete_proxy(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let proxy_id = arguments
      .get("proxy_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing proxy_id".to_string(),
        data: None,
      })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    PROXY_MANAGER
      .delete_stored_proxy(&app_handle, proxy_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to delete proxy: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Proxy '{}' deleted successfully", proxy_id)
      }]
    }))
  }

  async fn handle_export_proxies(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let format = arguments
      .get("format")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing format".to_string(),
        data: None,
      })?;

    let content = match format {
      "json" => PROXY_MANAGER.export_proxies_json().map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to export proxies: {e}"),
        data: None,
      })?,
      "txt" => PROXY_MANAGER.export_proxies_txt(),
      _ => {
        return Err(McpError {
          code: -32602,
          message: format!("Invalid format '{}', must be 'json' or 'txt'", format),
          data: None,
        })
      }
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": content
      }]
    }))
  }

  async fn handle_import_proxies(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let content = arguments
      .get("content")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing content".to_string(),
        data: None,
      })?;

    let format = arguments
      .get("format")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing format".to_string(),
        data: None,
      })?;

    let name_prefix = arguments
      .get("name_prefix")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let result = match format {
      "json" => PROXY_MANAGER
        .import_proxies_json(&app_handle, content)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to import proxies: {e}"),
          data: None,
        })?,
      "txt" => {
        use crate::proxy_manager::{ProxyManager, ProxyParseResult};

        let parse_results = ProxyManager::parse_txt_proxies(content);
        let parsed: Vec<_> = parse_results
          .into_iter()
          .filter_map(|r| {
            if let ProxyParseResult::Parsed(p) = r {
              Some(p)
            } else {
              None
            }
          })
          .collect();

        if parsed.is_empty() {
          return Err(McpError {
            code: -32000,
            message: "No valid proxies found in content".to_string(),
            data: None,
          });
        }

        PROXY_MANAGER
          .import_proxies_from_parsed(&app_handle, parsed, name_prefix)
          .map_err(|e| McpError {
            code: -32000,
            message: format!("Failed to import proxies: {e}"),
            data: None,
          })?
      }
      _ => {
        return Err(McpError {
          code: -32602,
          message: format!("Invalid format '{}', must be 'json' or 'txt'", format),
          data: None,
        })
      }
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "Import complete: {} imported, {} skipped, {} errors",
          result.imported_count,
          result.skipped_count,
          result.errors.len()
        )
      }]
    }))
  }

  // Profile import handlers
  async fn handle_detect_browser_profiles(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let importer = crate::profile_importer::ProfileImporter::instance();
    let profiles = match arguments.get("folder").and_then(|v| v.as_str()) {
      Some(folder) => importer.scan_folder(std::path::Path::new(folder)),
      None => importer.detect_existing_profiles(),
    }
    .map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to detect profiles: {e}"),
      data: None,
    })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&profiles).unwrap_or_else(|_| "[]".to_string())
      }]
    }))
  }

  async fn handle_import_browser_profiles(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let items: Vec<crate::profile_importer::ImportProfileItem> = arguments
      .get("items")
      .cloned()
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing items".to_string(),
        data: None,
      })
      .and_then(|v| {
        serde_json::from_value(v).map_err(|e| McpError {
          code: -32602,
          message: format!("Invalid items: {e}"),
          data: None,
        })
      })?;

    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    let duplicate_strategy = arguments
      .get("duplicate_strategy")
      .cloned()
      .map(serde_json::from_value::<crate::profile_importer::DuplicateStrategy>)
      .transpose()
      .map_err(|e| McpError {
        code: -32602,
        message: format!("Invalid duplicate_strategy: {e}"),
        data: None,
      })?
      .unwrap_or_default();

    // Clone the handle instead of holding the inner lock across a potentially
    // multi-GB copy.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner.app_handle.clone().ok_or_else(|| McpError {
        code: -32000,
        message: "MCP server not properly initialized".to_string(),
        data: None,
      })?
    };

    let result = crate::profile_importer::ProfileImporter::instance()
      .import_profiles(&app_handle, items, group_id, duplicate_strategy, None)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to import profiles: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "Import complete: {} imported, {} skipped, {} failed\n{}",
          result.imported_count,
          result.skipped_count,
          result.failed_count,
          serde_json::to_string_pretty(&result.results).unwrap_or_default()
        )
      }]
    }))
  }

  // Cookie management handlers
  async fn handle_import_profile_cookies(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    let content = arguments
      .get("content")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing content".to_string(),
        data: None,
      })?;

    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let result =
      crate::cookie_manager::CookieManager::import_cookies(&app_handle, profile_id, content)
        .await
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to import cookies: {e}"),
          data: None,
        })?;

    if let Some(scheduler) = crate::sync::get_global_scheduler() {
      let profile_manager = crate::profile::manager::ProfileManager::instance();
      if let Ok(profiles) = profile_manager.list_profiles() {
        if let Some(profile) = profiles.iter().find(|p| p.id.to_string() == profile_id) {
          if profile.is_sync_enabled() {
            let pid = profile_id.to_string();
            tauri::async_runtime::spawn(async move {
              scheduler.queue_profile_sync(pid).await;
            });
          }
        }
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "Import complete: {} imported, {} replaced, {} parse error(s)",
          result.cookies_imported,
          result.cookies_replaced,
          result.errors.len()
        )
      }]
    }))
  }

  // VPN management handlers
  async fn handle_import_vpn(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let content = arguments
      .get("content")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing content".to_string(),
        data: None,
      })?;

    let filename = arguments
      .get("filename")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing filename".to_string(),
        data: None,
      })?;

    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    let storage = crate::vpn::VPN_STORAGE.lock().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to lock VPN storage: {e}"),
      data: None,
    })?;

    let config = storage
      .import_config(content, filename, name)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to import VPN config: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "VPN '{}' ({}) imported successfully with ID: {}",
          config.name,
          config.vpn_type,
          config.id
        )
      }]
    }))
  }

  async fn handle_list_vpn_configs(&self) -> Result<serde_json::Value, McpError> {
    let storage = crate::vpn::VPN_STORAGE.lock().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to lock VPN storage: {e}"),
      data: None,
    })?;

    let configs = storage.list_configs().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to list VPN configs: {e}"),
      data: None,
    })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&configs).unwrap_or_default()
      }]
    }))
  }

  async fn handle_delete_vpn(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let vpn_id = arguments
      .get("vpn_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing vpn_id".to_string(),
        data: None,
      })?;

    // First disconnect if connected (stop VPN worker)
    let _ = crate::vpn_worker_runner::stop_vpn_worker_by_vpn_id(vpn_id).await;

    let storage = crate::vpn::VPN_STORAGE.lock().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to lock VPN storage: {e}"),
      data: None,
    })?;

    storage.delete_config(vpn_id).map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to delete VPN config: {e}"),
      data: None,
    })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("VPN '{}' deleted successfully", vpn_id)
      }]
    }))
  }

  async fn handle_connect_vpn(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let vpn_id = arguments
      .get("vpn_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing vpn_id".to_string(),
        data: None,
      })?;

    // Start VPN worker process
    crate::vpn_worker_runner::start_vpn_worker(vpn_id)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to connect VPN: {e}"),
        data: None,
      })?;

    // Update last_used timestamp
    {
      let storage = crate::vpn::VPN_STORAGE.lock().map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock VPN storage: {e}"),
        data: None,
      })?;
      let _ = storage.update_last_used(vpn_id);
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("VPN '{}' connected successfully", vpn_id)
      }]
    }))
  }

  async fn handle_disconnect_vpn(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let vpn_id = arguments
      .get("vpn_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing vpn_id".to_string(),
        data: None,
      })?;

    crate::vpn_worker_runner::stop_vpn_worker_by_vpn_id(vpn_id)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to disconnect VPN: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("VPN '{}' disconnected successfully", vpn_id)
      }]
    }))
  }

  async fn handle_get_vpn_status(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let vpn_id = arguments
      .get("vpn_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing vpn_id".to_string(),
        data: None,
      })?;

    let connected =
      if let Some(worker) = crate::vpn_worker_storage::find_vpn_worker_by_vpn_id(vpn_id) {
        worker
          .pid
          .map(crate::proxy_storage::is_process_running)
          .unwrap_or(false)
      } else {
        false
      };

    let status = crate::vpn::VpnStatus {
      connected,
      vpn_id: vpn_id.to_string(),
      connected_at: None,
      bytes_sent: None,
      bytes_received: None,
      last_handshake: None,
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&status).unwrap_or_default()
      }]
    }))
  }

  // Fingerprint management handlers
  async fn handle_get_profile_fingerprint(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
        data: None,
      })?;

    let fingerprint_info = match profile.browser.as_str() {
      "wayfern" => {
        let config = profile.wayfern_config.as_ref().cloned().unwrap_or_default();
        serde_json::json!({
          "browser": "wayfern",
          "fingerprint": config.fingerprint,
          "identity_id": config.identity_id,
          "identity_overrides": config.identity_overrides,
          "location": config.location,
          "os": config.os,
          "randomize_fingerprint_on_launch": config.randomize_fingerprint_on_launch,
          "screen_max_width": config.screen_max_width,
          "screen_max_height": config.screen_max_height,
          "screen_min_width": config.screen_min_width,
          "screen_min_height": config.screen_min_height,
          // The launch behaviour an agent can also set through
          // `update_profile_fingerprint`, reported here so it never has to
          // guess what a profile will do when it starts.
          "restore_session": config.restore_session,
          "webrtc_mode": config.webrtc_mode,
          "camera_file": config.camera_file,
          "camera_crop": config.camera_crop,
        })
      }
      _ => {
        return Err(McpError {
          code: -32000,
          message: "MCP only supports Wayfern profiles".to_string(),
          data: None,
        })
      }
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&fingerprint_info).unwrap_or_default()
      }]
    }))
  }

  async fn handle_update_profile_fingerprint(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.can_use_cross_os_fingerprints().await {
      return Err(McpError {
        code: -32000,
        message: "Fingerprint editing requires a plan that includes it".to_string(),
        data: None,
      });
    }

    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    let fingerprint = arguments.get("fingerprint").and_then(|v| v.as_str());
    let os = arguments.get("os").and_then(|v| v.as_str());
    let restore_session = arguments
      .get("restore_session")
      .and_then(serde_json::Value::as_bool);
    let webrtc_mode = match arguments.get("webrtc_mode").and_then(|v| v.as_str()) {
      Some(mode) if crate::wayfern_manager::WebRtcMode::parse(mode).is_some() => {
        Some(mode.to_string())
      }
      Some(mode) => {
        return Err(McpError {
          code: -32602,
          message: format!("Unknown webrtc_mode {mode:?}; expected auto, tcp_only or block"),
          data: None,
        })
      }
      None => None,
    };
    let randomize = arguments
      .get("randomize_fingerprint_on_launch")
      .and_then(|v| v.as_bool());

    if let Some(os_val) = os {
      if !CLOUD_AUTH.is_fingerprint_os_allowed(Some(os_val)).await {
        return Err(McpError {
          code: -32000,
          message: format!(
            "OS spoofing to '{}' requires an active Pro subscription",
            os_val
          ),
          data: None,
        });
      }
    }

    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
        data: None,
      })?;

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    match profile.browser.as_str() {
      "wayfern" => {
        let mut config = profile.wayfern_config.as_ref().cloned().unwrap_or_default();
        if let Some(fp) = fingerprint {
          config.fingerprint = Some(fp.to_string());
        }
        if let Some(os_val) = os {
          config.os = Some(os_val.to_string());
        }
        if let Some(r) = randomize {
          config.randomize_fingerprint_on_launch = Some(r);
        }
        if let Some(restore) = restore_session {
          config.restore_session = Some(restore);
        }
        if let Some(mode) = webrtc_mode {
          config.webrtc_mode = Some(mode);
        }
        ProfileManager::instance()
          .update_wayfern_config(app_handle.clone(), profile_id, config)
          .await
          .map_err(|e| McpError {
            code: -32000,
            message: format!("Failed to update wayfern config: {e}"),
            data: None,
          })?;
      }
      _ => {
        return Err(McpError {
          code: -32000,
          message: "MCP only supports Wayfern profiles".to_string(),
          data: None,
        })
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Fingerprint configuration updated for profile '{}'", profile.name)
      }]
    }))
  }

  async fn handle_update_profile_proxy_bypass_rules(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    let rules: Vec<String> = arguments
      .get("rules")
      .and_then(|v| v.as_array())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing rules array".to_string(),
        data: None,
      })?
      .iter()
      .filter_map(|v| v.as_str().map(|s| s.to_string()))
      .collect();

    // The guard is dropped before the await below. Binding `app_handle` as a
    // REFERENCE out of `inner` keeps the engine's single global mutex locked
    // for the whole operation — and `handle_message` needs that same mutex to
    // validate the session on every request, and to serve `initialize`. So a
    // launch that takes twenty seconds froze every other MCP message on this
    // desktop for twenty seconds: the agent's own follow-up calls, a second
    // agent, the website console, and even a brand-new client trying to open a
    // session. The batch handlers already clone-and-drop for exactly this
    // reason; the single-profile ones did not.
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError {
          code: -32000,
          message: "MCP server not properly initialized".to_string(),
          data: None,
        })?
        .clone()
    };

    let profile = ProfileManager::instance()
      .update_profile_proxy_bypass_rules(&app_handle, profile_id, rules.clone())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to update proxy bypass rules: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "Proxy bypass rules updated for profile '{}': {} rule(s) configured",
          profile.name,
          rules.len()
        )
      }]
    }))
  }

  async fn handle_update_profile_dns_blocklist(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    let level = arguments
      .get("level")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing level".to_string(),
        data: None,
      })?;

    let dns_blocklist = if level == "none" {
      None
    } else {
      Some(level.to_string())
    };

    let profile = ProfileManager::instance()
      .update_profile_dns_blocklist(profile_id, dns_blocklist)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to update DNS blocklist: {e}"),
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "DNS blocklist updated for profile '{}': {}",
          profile.name,
          level
        )
      }]
    }))
  }

  async fn handle_get_dns_blocklist_status(&self) -> Result<serde_json::Value, McpError> {
    let statuses = crate::dns_blocklist::BlocklistManager::get_cache_status();
    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&statuses).unwrap_or_default()
      }]
    }))
  }

  async fn handle_list_extensions(&self) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let extensions = mgr.list_extensions().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to list extensions: {e}"),
      data: None,
    })?;
    Ok(serde_json::to_value(extensions).unwrap())
  }

  async fn handle_list_extension_groups(&self) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let groups = mgr.list_groups().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to list extension groups: {e}"),
      data: None,
    })?;
    Ok(serde_json::to_value(groups).unwrap())
  }

  async fn handle_add_extension(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let path = arguments
      .get("path")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: path".to_string(),
        data: None,
      })?;
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .unwrap_or_default()
      .to_string();
    let link = arguments
      .get("link")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let path = crate::extension_manager::client_named_path(path).map_err(|e| McpError {
      code: -32602,
      message: format!("Invalid path: {e}"),
      data: None,
    })?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let extension = mgr
      .add_extension_from_path(name, &path, link)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to add extension: {e}"),
        data: None,
      })?;
    Ok(serde_json::to_value(extension).unwrap())
  }

  async fn handle_update_extension(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let extension_id = arguments
      .get("extension_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: extension_id".to_string(),
        data: None,
      })?;
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .map(str::to_string);
    let path = arguments.get("path").and_then(|v| v.as_str());
    if name.is_none() && path.is_none() {
      return Err(McpError {
        code: -32602,
        message: "Provide at least one of: name, path".to_string(),
        data: None,
      });
    }
    let link = arguments
      .get("link")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let path = path
      .map(|path| {
        crate::extension_manager::client_named_path(path).map_err(|e| McpError {
          code: -32602,
          message: format!("Invalid path: {e}"),
          data: None,
        })
      })
      .transpose()?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let extension = match path {
      Some(path) => mgr.update_extension_from_path(extension_id, name, &path, link),
      None => mgr.update_extension(extension_id, name, None, None),
    }
    .map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to update extension: {e}"),
      data: None,
    })?;
    Ok(serde_json::to_value(extension).unwrap())
  }

  async fn handle_create_extension_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: name".to_string(),
        data: None,
      })?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let group = mgr.create_group(name.to_string()).map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to create extension group: {e}"),
      data: None,
    })?;
    Ok(serde_json::to_value(group).unwrap())
  }

  async fn handle_update_extension_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: group_id".to_string(),
        data: None,
      })?;
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .map(str::to_string);
    let extension_ids = arguments
      .get("extension_ids")
      .and_then(|v| v.as_array())
      .map(|ids| {
        ids
          .iter()
          .filter_map(|id| id.as_str().map(str::to_string))
          .collect::<Vec<String>>()
      });
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let group = mgr
      .update_group(group_id, name, extension_ids)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to update extension group: {e}"),
        data: None,
      })?;
    Ok(serde_json::to_value(group).unwrap())
  }

  async fn handle_add_extension_to_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let (group_id, extension_id) = Self::group_and_extension_ids(arguments)?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let group = mgr
      .add_extension_to_group(group_id, extension_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to add extension to group: {e}"),
        data: None,
      })?;
    Ok(serde_json::to_value(group).unwrap())
  }

  async fn handle_remove_extension_from_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let (group_id, extension_id) = Self::group_and_extension_ids(arguments)?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let group = mgr
      .remove_extension_from_group(group_id, extension_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to remove extension from group: {e}"),
        data: None,
      })?;
    Ok(serde_json::to_value(group).unwrap())
  }

  fn group_and_extension_ids(arguments: &serde_json::Value) -> Result<(&str, &str), McpError> {
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: group_id".to_string(),
        data: None,
      })?;
    let extension_id = arguments
      .get("extension_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: extension_id".to_string(),
        data: None,
      })?;
    Ok((group_id, extension_id))
  }

  async fn handle_delete_extension_mcp(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let extension_id = arguments
      .get("extension_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: extension_id".to_string(),
        data: None,
      })?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    mgr
      .delete_extension_internal(extension_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to delete extension: {e}"),
        data: None,
      })?;
    Ok(serde_json::json!({"success": true}))
  }

  async fn handle_delete_extension_group_mcp(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: group_id".to_string(),
        data: None,
      })?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    // For MCP, we don't have an app_handle, but we need one for sync deletion.
    // Use the delete_group_internal which skips sync remote deletion.
    mgr.delete_group_internal(group_id).map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to delete extension group: {e}"),
      data: None,
    })?;
    if let Err(e) = crate::events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }
    Ok(serde_json::json!({"success": true}))
  }

  async fn handle_assign_extension_group_to_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
        data: None,
      });
    }
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: profile_id".to_string(),
        data: None,
      })?;
    let extension_group_id = arguments
      .get("extension_group_id")
      .and_then(|v| v.as_str())
      .map(|s| {
        if s.is_empty() {
          None
        } else {
          Some(s.to_string())
        }
      })
      .unwrap_or(None);

    // Validate compatibility if assigning
    if let Some(ref gid) = extension_group_id {
      let profile_manager = ProfileManager::instance();
      let profiles = profile_manager.list_profiles().map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;
      let profile = profiles
        .iter()
        .find(|p| p.id.to_string() == profile_id)
        .ok_or_else(|| McpError {
          code: -32000,
          message: format!("Profile '{profile_id}' not found"),
          data: None,
        })?;
      let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      mgr
        .validate_group_compatibility(gid, &profile.browser)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("{e}"),
          data: None,
        })?;
    }

    let profile_manager = ProfileManager::instance();
    let profile = profile_manager
      .update_profile_extension_group(profile_id, extension_group_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to assign extension group: {e}"),
        data: None,
      })?;
    Ok(serde_json::to_value(profile).unwrap())
  }

  async fn handle_get_team_locks(&self) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.is_on_team_plan().await {
      return Err(McpError {
        code: -32000,
        message: "Team features require an active team plan".to_string(),
        data: None,
      });
    }
    let locks = crate::team_lock::TEAM_LOCK.get_locks().await;
    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&locks).unwrap_or_default()
      }]
    }))
  }

  async fn handle_get_team_lock_status(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.is_on_team_plan().await {
      return Err(McpError {
        code: -32000,
        message: "Team features require an active team plan".to_string(),
        data: None,
      });
    }
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let lock_status = crate::team_lock::TEAM_LOCK
      .get_lock_status(profile_id)
      .await;
    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&lock_status).unwrap_or_default()
      }]
    }))
  }

  // --- CDP utility methods for browser interaction ---

  /// Where this profile's browser is: on this machine, or on the fleet.
  ///
  /// Every interaction tool goes through here, so a profile launched with
  /// `run-remote` is driven by exactly the tools that drive a local one. The
  /// alternative — a parallel set of remote-only tools — drifts from the local
  /// set within a release and doubles every future change.
  async fn resolve_cdp_target(&self, profile_id: &str) -> Result<CdpTarget, McpError> {
    let profile = self.get_wayfern_profile(profile_id)?;
    crate::cdp_target::resolve(&profile)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e.to_string(),
        data: None,
      })
  }

  async fn send_cdp(
    &self,
    target: &CdpTarget,
    method: &str,
    params: serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    crate::cdp_target::run_command(target, method, params)
      .await
      .map_err(cdp_error)
  }

  async fn send_planned_keystrokes(
    &self,
    target: &CdpTarget,
    events: &[crate::human_typing::TypingEvent],
  ) -> Result<(), McpError> {
    dispatch_keystrokes(target, events).await.map_err(cdp_error)
  }

  async fn send_cdp_and_wait_for_load(
    &self,
    target: &CdpTarget,
    method: &str,
    params: serde_json::Value,
    timeout_secs: u64,
  ) -> Result<serde_json::Value, McpError> {
    crate::cdp_target::run_command_awaiting_load(target, method, params, timeout_secs)
      .await
      .map_err(cdp_error)
  }

  /// The profile a browser-interaction tool refers to.
  ///
  /// Deliberately does NOT require a local process. That check used to live
  /// here, and it is exactly the state a profile running on the fleet is in, so
  /// it refused every remote tool call before resolution had a chance to find
  /// the session. Whether a browser exists at all is [`resolve_cdp_target`]'s
  /// answer to give, because only it can see both places one could be.
  fn get_wayfern_profile(&self, profile_id: &str) -> Result<BrowserProfile, McpError> {
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let profile = profiles
      .into_iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
        data: None,
      })?;

    if profile.browser != "wayfern" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern profiles".to_string(),
        data: None,
      });
    }

    Ok(profile)
  }

  // --- Browser interaction handlers ---

  async fn handle_navigate(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    // Read and validated in one step, so the safe path is the short one.
    let url = Self::require_navigable_url(arguments, "url")?;

    let target = self.resolve_cdp_target(profile_id).await?;

    self
      .send_cdp_and_wait_for_load(
        &target,
        "Page.navigate",
        serde_json::json!({ "url": url }),
        30,
      )
      .await?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Navigated to {url}")
      }]
    }))
  }

  async fn handle_screenshot(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let format = arguments
      .get("format")
      .and_then(|v| v.as_str())
      .unwrap_or("png");
    let quality = arguments.get("quality").and_then(|v| v.as_i64());
    let full_page = arguments
      .get("full_page")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    let target = self.resolve_cdp_target(profile_id).await?;

    let mut params = serde_json::json!({ "format": format });

    if let Some(q) = quality {
      params["quality"] = serde_json::json!(q);
    }

    if full_page {
      let layout = self
        .send_cdp(&target, "Page.getLayoutMetrics", serde_json::json!({}))
        .await?;

      if let Some(content_size) = layout.get("contentSize") {
        params["clip"] = serde_json::json!({
          "x": 0,
          "y": 0,
          "width": content_size.get("width").and_then(|v| v.as_f64()).unwrap_or(1920.0),
          "height": content_size.get("height").and_then(|v| v.as_f64()).unwrap_or(1080.0),
          "scale": 1
        });
        params["captureBeyondViewport"] = serde_json::json!(true);
      }
    }

    let result = self
      .send_cdp(&target, "Page.captureScreenshot", params)
      .await?;

    let data = result
      .get("data")
      .and_then(|v| v.as_str())
      .unwrap_or_default();

    Ok(serde_json::json!({
      "content": [{
        "type": "image",
        "data": data,
        "mimeType": format!("image/{format}")
      }]
    }))
  }

  async fn handle_evaluate_javascript(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let expression = arguments
      .get("expression")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing expression".to_string(),
        data: None,
      })?;
    let await_promise = arguments
      .get("await_promise")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let wait_for_load = arguments
      .get("wait_for_load")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    let target = self.resolve_cdp_target(profile_id).await?;

    let cdp_params = serde_json::json!({
      "expression": expression,
      "returnByValue": true,
      "awaitPromise": await_promise,
    });

    let result = if wait_for_load {
      self
        .send_cdp_and_wait_for_load(&target, "Runtime.evaluate", cdp_params, 30)
        .await?
    } else {
      self
        .send_cdp(&target, "Runtime.evaluate", cdp_params)
        .await?
    };

    let value = if let Some(exception) = result.get("exceptionDetails") {
      let text = exception
        .get("text")
        .or_else(|| {
          exception
            .get("exception")
            .and_then(|e| e.get("description"))
        })
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown error");
      serde_json::json!({ "error": text })
    } else if let Some(r) = result.get("result") {
      let val = r.get("value").cloned().unwrap_or(serde_json::json!(null));
      serde_json::json!({ "value": val, "type": r.get("type") })
    } else {
      serde_json::json!({ "value": null })
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&value).unwrap_or_default()
      }]
    }))
  }

  async fn handle_click_element(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let selector = arguments
      .get("selector")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing selector".to_string(),
        data: None,
      })?;

    let ctx = self.agent_context(profile_id).await?;
    let selector_escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");

    if ctx.engine.is_wayfern() {
      // On 152 no script clicks anything. The page only says where the
      // element is; the click comes from a real pointer that glides there
      // and presses with the profile's own timing, through the same input
      // path a mouse uses.
      let point = locate_by_script(&ctx.target, element_rect_script(&selector_escaped)).await?;
      let (_, navigated) = vellum_click(&ctx.target, point, None, None)
        .await
        .map_err(AgentError::into_mcp)?;
      return Ok(serde_json::json!({
        "content": [{
          "type": "text",
          "text": click_report(&format!("Clicked element: {selector}"), navigated)
        }]
      }));
    }

    let js = format!(
      r#"(() => {{
        const el = document.querySelector('{}');
        if (!el) throw new Error('Element not found: {}');
        el.scrollIntoView({{block: 'center'}});
        el.click();
        return true;
      }})()"#,
      selector_escaped, selector_escaped
    );

    // Use send_cdp_and_wait_for_load: if the click triggers navigation,
    // we wait for the new page to load. If not, the 10s timeout expires
    // and we return immediately.
    let result = self
      .send_cdp_and_wait_for_load(
        &ctx.target,
        "Runtime.evaluate",
        serde_json::json!({
          "expression": js,
          "returnByValue": true,
        }),
        10,
      )
      .await?;

    if let Some(exception) = result.get("exceptionDetails") {
      let msg = exception
        .get("exception")
        .and_then(|e| e.get("description"))
        .or_else(|| exception.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("Click failed");
      return Err(McpError {
        code: -32000,
        message: msg.to_string(),
        data: None,
      });
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Clicked element: {selector}")
      }]
    }))
  }

  async fn handle_type_text(
    &self,
    caller: McpCaller<'_>,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let selector = arguments
      .get("selector")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing selector".to_string(),
        data: None,
      })?;
    let text = arguments
      .get("text")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing text".to_string(),
        data: None,
      })?;
    let clear_first = arguments
      .get("clear_first")
      .and_then(|v| v.as_bool())
      .unwrap_or(true);
    let instant = arguments
      .get("instant")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let wpm = arguments.get("wpm").and_then(|v| v.as_f64());
    let typos = arguments
      .get("typos")
      .and_then(|v| v.as_bool())
      .unwrap_or(true);

    // The engine is read off the profile before any budget is decided. A 152
    // browser types through Vellum, which paces the keys itself, so the plan
    // below is only built for the fallback engine.
    let profile = self.get_wayfern_profile(profile_id)?;
    let humanized = Engine::for_version(&profile.version).is_wayfern() && !instant;

    // PLANNED FIRST, before anything touches the page. The planner can refuse
    // (TYPING_TOO_LONG), and the focus step below empties the field when
    // `clear_first` is set — so planning after it meant the server answered
    // "refused, nothing happened" having already wiped the customer's form
    // field. The refusal now precedes every mutation on every branch,
    // including the Vellum budget, which is decided in the same place.
    let plan = if instant || humanized {
      None
    } else {
      Some(plan_typing(text, wpm, max_typing_seconds(caller.origin))?)
    };
    let inscribe_timeout = if humanized {
      Some(vellum_typing_budget(
        text,
        max_typing_seconds(caller.origin),
      )?)
    } else {
      None
    };

    let target = resolve_target(&profile).await?;

    let selector_escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");
    let focus_js = if clear_first {
      format!(
        r#"(() => {{
          const el = document.querySelector('{}');
          if (!el) throw new Error('Element not found: {}');
          el.scrollIntoView({{block: 'center'}});
          el.focus();
          el.value = '';
          el.dispatchEvent(new Event('input', {{bubbles: true}}));
          {}
        }})()"#,
        selector_escaped, selector_escaped, RETURN_RECT_JS
      )
    } else {
      format!(
        r#"(() => {{
          const el = document.querySelector('{}');
          if (!el) throw new Error('Element not found: {}');
          el.scrollIntoView({{block: 'center'}});
          el.focus();
          {}
        }})()"#,
        selector_escaped, selector_escaped, RETURN_RECT_JS
      )
    };

    let focus_result = self
      .send_cdp(
        &target,
        "Runtime.evaluate",
        serde_json::json!({
          "expression": focus_js,
          "returnByValue": true,
        }),
      )
      .await?;

    if let Some(exception) = focus_result.get("exceptionDetails") {
      let msg = exception
        .get("exception")
        .and_then(|e| e.get("description"))
        .or_else(|| exception.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("Focus failed");
      return Err(McpError {
        code: -32000,
        message: msg.to_string(),
        data: None,
      });
    }

    if let Some(timeout) = inscribe_timeout {
      let point = rect_from_script_result(&focus_result)?;
      let inscription = vellum_type(
        &target,
        point,
        text,
        typos,
        caret_preparation(clear_first),
        timeout,
      )
      .await
      .map_err(AgentError::into_mcp)?;
      return Ok(serde_json::json!({
        "content": [{
          "type": "text",
          "text": typing_report(&format!("Typed text into element: {selector}"), &inscription)
        }]
      }));
    }

    match &plan {
      Some(events) => self.send_planned_keystrokes(&target, events).await?,
      None => {
        self
          .send_cdp(
            &target,
            "Input.insertText",
            serde_json::json!({ "text": text }),
          )
          .await?;
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Typed text into element: {selector}")
      }]
    }))
  }

  async fn handle_get_page_content(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let format = arguments
      .get("format")
      .and_then(|v| v.as_str())
      .unwrap_or("text");
    let selector = arguments.get("selector").and_then(|v| v.as_str());
    let max_chars = arguments
      .get("max_chars")
      .and_then(|v| v.as_u64())
      .map(|n| n as usize)
      .unwrap_or(40_000);

    let target = self.resolve_cdp_target(profile_id).await?;

    let js = if let Some(sel) = selector {
      let sel_escaped = sel.replace('\\', "\\\\").replace('\'', "\\'");
      if format == "html" {
        format!(
          r#"(() => {{
            const el = document.querySelector('{}');
            return el ? el.outerHTML : null;
          }})()"#,
          sel_escaped
        )
      } else {
        format!(
          r#"(() => {{
            const el = document.querySelector('{}');
            return el ? el.innerText : null;
          }})()"#,
          sel_escaped
        )
      }
    } else if format == "html" {
      "document.documentElement.outerHTML".to_string()
    } else {
      "document.body.innerText".to_string()
    };

    let result = self
      .send_cdp(
        &target,
        "Runtime.evaluate",
        serde_json::json!({
          "expression": js,
          "returnByValue": true,
        }),
      )
      .await?;

    let content = result
      .get("result")
      .and_then(|r| r.get("value"))
      .and_then(|v| v.as_str())
      .unwrap_or("");

    // Cap output so a 500 KB DOM dump doesn't blow out the agent's context.
    // Slice on character boundaries (chars().take().collect()) rather than
    // byte indices, since the latter would panic on multi-byte boundaries.
    let total_chars = content.chars().count();
    let (text, truncated) = if total_chars > max_chars {
      (content.chars().take(max_chars).collect::<String>(), true)
    } else {
      (content.to_string(), false)
    };

    let payload = if truncated {
      format!(
        "{text}\n\n[truncated: showing {max_chars} of {total_chars} chars — call with a larger max_chars or use get_interactive_elements for an indexed view]"
      )
    } else {
      text
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": payload
      }]
    }))
  }

  async fn handle_get_page_info(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;

    let target = self.resolve_cdp_target(profile_id).await?;

    let result = self
      .send_cdp(
        &target,
        "Runtime.evaluate",
        serde_json::json!({
          "expression": "JSON.stringify({url: location.href, title: document.title, readyState: document.readyState})",
          "returnByValue": true,
        }),
      )
      .await?;

    let info_str = result
      .get("result")
      .and_then(|r| r.get("value"))
      .and_then(|v| v.as_str())
      .unwrap_or("{}");

    let info: serde_json::Value = serde_json::from_str(info_str).unwrap_or(serde_json::json!({}));

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&info).unwrap_or_default()
      }]
    }))
  }

  async fn handle_get_interactive_elements(
    &self,
    caller: McpCaller<'_>,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    // FIRST, before the page is touched. The indices this hands back are only
    // usable by a session, and a sessionless caller would otherwise write a
    // snapshot into a slot it shares with every other sessionless caller, the
    // exact array-mixing that makes click_by_index click the wrong element.
    let session = require_indexed_session(caller)?;
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let max_chars = arguments
      .get("max_chars")
      .and_then(|v| v.as_u64())
      .map(|n| n as usize)
      .unwrap_or(40_000);

    let target = self.resolve_cdp_target(profile_id).await?;

    // Walk the DOM for visible, non-disabled interactive elements, label them
    // with a zero-based index, and cache the live references on THIS CALLER'S
    // slot so click_by_index / type_by_index can resolve the index → Element
    // without round-tripping a selector.
    //
    // Per session, because the slot used to be one shared `__donut_interactive`
    // array on the page, and then one per transport. Either way a second client
    // listing elements overwrote the array the first had just built, so that
    // client's next `click_by_index(3)` clicked whatever happened to be third
    // in the OTHER caller's snapshot, a wrong click reported as a successful
    // one, which is the worst shape a failure can take on somebody's browser.
    let slot = interactive_cache_slot(caller.origin, session);
    let js = INTERACTIVE_ELEMENTS_JS
      .replace("__MAX_CHARS__", &max_chars.to_string())
      .replace("__CACHE__", &slot)
      .replace("__REGISTRY__", INTERACTIVE_SLOT_REGISTRY)
      .replace("__MAX_SLOTS__", &MAX_CACHE_SLOTS_PER_PAGE.to_string());

    let result = self
      .send_cdp(
        &target,
        "Runtime.evaluate",
        serde_json::json!({
          "expression": js,
          "returnByValue": true,
        }),
      )
      .await?;

    if let Some(exception) = result.get("exceptionDetails") {
      let msg = exception
        .get("exception")
        .and_then(|e| e.get("description"))
        .or_else(|| exception.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("Enumeration failed");
      return Err(McpError {
        code: -32000,
        message: msg.to_string(),
        data: None,
      });
    }

    // Recorded only once the write has actually landed, so `end_session` cleans
    // up pages this session really wrote to rather than every page it asked
    // about.
    self.remember_cached_page(session, profile_id, &slot).await;

    let payload_str = result
      .get("result")
      .and_then(|r| r.get("value"))
      .and_then(|v| v.as_str())
      .unwrap_or("{}");

    let payload: serde_json::Value =
      serde_json::from_str(payload_str).unwrap_or(serde_json::json!({}));
    let elements = payload
      .get("elements")
      .and_then(|v| v.as_str())
      .unwrap_or("");
    let count = payload.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
    let truncated = payload
      .get("truncated")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    let header = if truncated {
      format!("{count} interactive elements (truncated at {max_chars} chars — re-call with a larger max_chars or scroll the page):")
    } else {
      format!("{count} interactive elements:")
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("{header}\n{elements}")
      }]
    }))
  }

  async fn handle_click_by_index(
    &self,
    caller: McpCaller<'_>,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    // Refused before anything else runs: an index names a position in an array
    // a PREVIOUS call left on the page, and without a session there is no
    // answer to whose array that is, only a shared slot to guess against.
    let cache = interactive_cache_slot(caller.origin, require_indexed_session(caller)?);
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let index = arguments
      .get("index")
      .and_then(|v| v.as_u64())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing index".to_string(),
        data: None,
      })?;

    let ctx = self.agent_context(profile_id).await?;

    if ctx.engine.is_wayfern() {
      // As for click_element: the cached reference only says where the
      // element is, and a real pointer does the clicking.
      let point = locate_by_script(&ctx.target, indexed_rect_script(&cache, index)).await?;
      let (_, navigated) = vellum_click(&ctx.target, point, None, None)
        .await
        .map_err(AgentError::into_mcp)?;
      return Ok(serde_json::json!({
        "content": [{
          "type": "text",
          "text": click_report(&format!("Clicked element at index {index}"), navigated)
        }]
      }));
    }

    let js = format!(
      r#"(() => {{
        const arr = window[{cache}];
        if (!arr || !arr[{index}]) throw new Error('No element at index {index}. Call get_interactive_elements first or after navigation.');
        const el = arr[{index}];
        el.scrollIntoView({{block: 'center'}});
        el.click();
        return true;
      }})()"#
    );

    let result = self
      .send_cdp_and_wait_for_load(
        &ctx.target,
        "Runtime.evaluate",
        serde_json::json!({
          "expression": js,
          "returnByValue": true,
        }),
        10,
      )
      .await?;

    if let Some(exception) = result.get("exceptionDetails") {
      let msg = exception
        .get("exception")
        .and_then(|e| e.get("description"))
        .or_else(|| exception.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("Click failed");
      return Err(McpError {
        code: -32000,
        message: msg.to_string(),
        data: None,
      });
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Clicked element at index {index}")
      }]
    }))
  }

  async fn handle_type_by_index(
    &self,
    caller: McpCaller<'_>,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    // Same refusal as click_by_index, and for the same reason: typing into
    // whatever happens to sit at index 3 of somebody else's snapshot is a
    // wrong action reported as a successful one.
    let cache = interactive_cache_slot(caller.origin, require_indexed_session(caller)?);
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
        data: None,
      })?;
    let index = arguments
      .get("index")
      .and_then(|v| v.as_u64())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing index".to_string(),
        data: None,
      })?;
    let text = arguments
      .get("text")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing text".to_string(),
        data: None,
      })?;
    let clear_first = arguments
      .get("clear_first")
      .and_then(|v| v.as_bool())
      .unwrap_or(true);
    let instant = arguments
      .get("instant")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let wpm = arguments.get("wpm").and_then(|v| v.as_f64());
    let typos = arguments
      .get("typos")
      .and_then(|v| v.as_bool())
      .unwrap_or(true);

    // Engine first, then budgets, then the page: see handle_type_text.
    let profile = self.get_wayfern_profile(profile_id)?;
    let humanized = Engine::for_version(&profile.version).is_wayfern() && !instant;

    // PLANNED FIRST, before anything touches the page. The planner can refuse
    // (TYPING_TOO_LONG), and the focus step below empties the field when
    // `clear_first` is set — so planning after it meant the server answered
    // "refused, nothing happened" having already wiped the customer's form
    // field. The refusal now precedes every mutation on every branch.
    let plan = if instant || humanized {
      None
    } else {
      Some(plan_typing(text, wpm, max_typing_seconds(caller.origin))?)
    };
    let inscribe_timeout = if humanized {
      Some(vellum_typing_budget(
        text,
        max_typing_seconds(caller.origin),
      )?)
    } else {
      None
    };

    let target = resolve_target(&profile).await?;

    // Mirrors handle_type_text's focus step but resolves the element via the
    // cached index instead of a CSS selector.
    let focus_js = if clear_first {
      format!(
        r#"(() => {{
          const arr = window[{cache}];
          if (!arr || !arr[{index}]) throw new Error('No element at index {index}. Call get_interactive_elements first or after navigation.');
          const el = arr[{index}];
          el.scrollIntoView({{block: 'center'}});
          el.focus();
          el.value = '';
          el.dispatchEvent(new Event('input', {{bubbles: true}}));
          {RETURN_RECT_JS}
        }})()"#
      )
    } else {
      format!(
        r#"(() => {{
          const arr = window[{cache}];
          if (!arr || !arr[{index}]) throw new Error('No element at index {index}. Call get_interactive_elements first or after navigation.');
          const el = arr[{index}];
          el.scrollIntoView({{block: 'center'}});
          el.focus();
          {RETURN_RECT_JS}
        }})()"#
      )
    };

    let focus_result = self
      .send_cdp(
        &target,
        "Runtime.evaluate",
        serde_json::json!({
          "expression": focus_js,
          "returnByValue": true,
        }),
      )
      .await?;

    if let Some(exception) = focus_result.get("exceptionDetails") {
      let msg = exception
        .get("exception")
        .and_then(|e| e.get("description"))
        .or_else(|| exception.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("Focus failed");
      return Err(McpError {
        code: -32000,
        message: msg.to_string(),
        data: None,
      });
    }

    if let Some(timeout) = inscribe_timeout {
      let point = rect_from_script_result(&focus_result)?;
      let inscription = vellum_type(
        &target,
        point,
        text,
        typos,
        caret_preparation(clear_first),
        timeout,
      )
      .await
      .map_err(AgentError::into_mcp)?;
      return Ok(serde_json::json!({
        "content": [{
          "type": "text",
          "text": typing_report(&format!("Typed text into element at index {index}"), &inscription)
        }]
      }));
    }

    match &plan {
      Some(events) => self.send_planned_keystrokes(&target, events).await?,
      None => {
        self
          .send_cdp(
            &target,
            "Input.insertText",
            serde_json::json!({ "text": text }),
          )
          .await?;
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Typed text into element at index {index}")
      }]
    }))
  }

  // --- Agent handlers: perception, locators, extraction, picker ---

  /// The running browser behind `profile_id`, and the engine its version gets.
  async fn agent_context(&self, profile_id: &str) -> Result<AgentContext, McpError> {
    let profile = self.get_wayfern_profile(profile_id)?;
    let target = resolve_target(&profile).await?;
    Ok(AgentContext::new(profile, target))
  }

  /// Read a tool's arguments into the request type the shared operation takes.
  ///
  /// `profile_id` and any other argument the type does not name are ignored,
  /// which is what lets one struct serve both the MCP arguments and the REST
  /// body.
  fn agent_arguments<T: serde::de::DeserializeOwned>(
    arguments: &serde_json::Value,
  ) -> Result<T, McpError> {
    serde_json::from_value(arguments.clone()).map_err(|e| McpError {
      code: -32602,
      message: format!("Invalid arguments: {e}"),
      data: None,
    })
  }

  async fn handle_perceive_page(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let request: PerceptionRequest = Self::agent_arguments(arguments)?;
    let ctx = self.agent_context(profile_id).await?;
    let page = agent_perceive(&ctx, &request)
      .await
      .map_err(AgentError::into_mcp)?;
    Self::json_content(&page)
  }

  async fn handle_resolve_locator(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let request: AgentResolveRequest = Self::agent_arguments(arguments)?;
    let ctx = self.agent_context(profile_id).await?;
    let resolved = agent_resolve_locator(&ctx, &request)
      .await
      .map_err(AgentError::into_mcp)?;
    Self::json_content(&resolved)
  }

  async fn handle_click_locator(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let request: AgentClickRequest = Self::agent_arguments(arguments)?;
    let ctx = self.agent_context(profile_id).await?;
    let clicked = agent_click_locator(&ctx, &request)
      .await
      .map_err(AgentError::into_mcp)?;
    Self::json_content(&clicked)
  }

  async fn handle_type_locator(
    &self,
    caller: McpCaller<'_>,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let request: AgentTypeRequest = Self::agent_arguments(arguments)?;
    let ctx = self.agent_context(profile_id).await?;
    let typed = agent_type_locator(&ctx, &request, max_typing_seconds(caller.origin))
      .await
      .map_err(AgentError::into_mcp)?;
    Self::json_content(&typed)
  }

  async fn handle_extract_structured(
    &self,
    caller: McpCaller<'_>,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let mut request: ExtractionRequest = Self::agent_arguments(arguments)?;
    // Clamped to what the transport can carry, like the picker's wait.
    request.time_budget_ms = Some(
      request
        .time_budget_ms
        .unwrap_or(8_000)
        .min(max_extraction_budget_ms(caller.origin)),
    );
    let ctx = self.agent_context(profile_id).await?;
    let extraction = agent_extract(&ctx, &request)
      .await
      .map_err(AgentError::into_mcp)?;
    Self::json_content(&extraction)
  }

  async fn handle_pick_element(
    &self,
    caller: McpCaller<'_>,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let request: AgentPickRequest = Self::agent_arguments(arguments)?;
    // Clamped rather than refused: a caller asking for ten minutes over the
    // bridge gets the longest wait the relay will actually carry.
    let timeout_ms = request
      .timeout_ms
      .unwrap_or(DEFAULT_PICK_TIMEOUT_MS)
      .clamp(1_000, max_pick_timeout_ms(caller.origin));
    let ctx = self.agent_context(profile_id).await?;
    let picked = agent_pick_element(&ctx, timeout_ms)
      .await
      .map_err(AgentError::into_mcp)?;
    Self::json_content(&picked)
  }

  // --- Synchronizer handlers ---

  async fn handle_start_sync_session(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let leader_id = arguments
      .get("leader_profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing leader_profile_id".to_string(),
        data: None,
      })?;
    let follower_ids: Vec<String> = arguments
      .get("follower_profile_ids")
      .and_then(|v| v.as_array())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing follower_profile_ids".to_string(),
        data: None,
      })?
      .iter()
      .filter_map(|v| v.as_str().map(|s| s.to_string()))
      .collect();

    let app = {
      let inner = self.inner.lock().await;
      inner.app_handle.clone().ok_or_else(|| McpError {
        code: -32000,
        message: "MCP server not properly initialized".to_string(),
        data: None,
      })?
    };

    let info = crate::synchronizer::SynchronizerManager::instance()
      .start_session(app, leader_id.to_string(), follower_ids)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e,
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&info).unwrap_or_default()
      }]
    }))
  }

  async fn handle_stop_sync_session(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let session_id = arguments
      .get("session_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing session_id".to_string(),
        data: None,
      })?;

    let app = {
      let inner = self.inner.lock().await;
      inner.app_handle.clone().ok_or_else(|| McpError {
        code: -32000,
        message: "MCP server not properly initialized".to_string(),
        data: None,
      })?
    };

    crate::synchronizer::SynchronizerManager::instance()
      .stop_session(app, session_id)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e,
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": "Sync session stopped"
      }]
    }))
  }

  async fn handle_get_sync_sessions(&self) -> Result<serde_json::Value, McpError> {
    let sessions = crate::synchronizer::SynchronizerManager::instance()
      .get_sessions()
      .await;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&sessions).unwrap_or_default()
      }]
    }))
  }

  async fn handle_remove_sync_follower(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let session_id = arguments
      .get("session_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing session_id".to_string(),
        data: None,
      })?;
    let follower_id = arguments
      .get("follower_profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing follower_profile_id".to_string(),
        data: None,
      })?;

    let app = {
      let inner = self.inner.lock().await;
      inner.app_handle.clone().ok_or_else(|| McpError {
        code: -32000,
        message: "MCP server not properly initialized".to_string(),
        data: None,
      })?
    };

    crate::synchronizer::SynchronizerManager::instance()
      .remove_follower(app, session_id, follower_id)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e,
        data: None,
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": "Follower removed from sync session"
      }]
    }))
  }

  // --- Remote fleet and cookie bot -----------------------------------------
  //
  // Every tool below is a proxy onto Donut cloud, which owns the schedule, the
  // calendar arithmetic, the browsing behaviour and the pooled hour budget.
  // Nothing here decides when a run happens or what it does. What this file
  // DOES decide is which profiles may be offered to the bot at all.

  /// Render a value as the single text block an MCP tool answers with.
  fn json_content<T: Serialize>(value: &T) -> Result<serde_json::Value, McpError> {
    let text = serde_json::to_string_pretty(value).map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to encode response: {e}"),
      data: None,
    })?;
    Ok(serde_json::json!({ "content": [{ "type": "text", "text": text }] }))
  }

  /// Read a caller-supplied URL and validate its scheme in one step.
  ///
  /// The guard belongs HERE rather than at each call site: a regression test
  /// that scans for `get("url")` cannot see a handler that reads the same
  /// argument through `require_str`, and a handler that reads a URL without
  /// validating it is how `file:///…` plus a content tool became a remote file
  /// read. Reaching for this instead of `require_str` makes the safe path the
  /// short one.
  fn require_navigable_url<'a>(
    arguments: &'a serde_json::Value,
    key: &str,
  ) -> Result<&'a str, McpError> {
    let url = Self::require_str(arguments, key)?;
    validate_navigable_url(url)?;
    Ok(url)
  }

  fn require_str<'a>(arguments: &'a serde_json::Value, key: &str) -> Result<&'a str, McpError> {
    arguments
      .get(key)
      .and_then(|value| value.as_str())
      .filter(|value| !value.is_empty())
      .ok_or_else(|| McpError {
        code: -32602,
        message: format!("Missing {key}"),
        data: None,
      })
  }

  /// Read a whole-number argument, refusing anything that would silently wrap.
  ///
  /// `as_u64() as u16` would turn a run time of 1440 into 1440 but 65536 into
  /// 0, quietly scheduling a run at midnight nobody asked for.
  fn require_u16(arguments: &serde_json::Value, key: &str) -> Result<u16, McpError> {
    Self::optional_u16(arguments, key)?.ok_or_else(|| McpError {
      code: -32602,
      message: format!("Missing {key}"),
      data: None,
    })
  }

  fn optional_u16(arguments: &serde_json::Value, key: &str) -> Result<Option<u16>, McpError> {
    let Some(value) = arguments.get(key).filter(|value| !value.is_null()) else {
      return Ok(None);
    };
    value
      .as_u64()
      .and_then(|raw| u16::try_from(raw).ok())
      .map(Some)
      .ok_or_else(|| McpError {
        code: -32602,
        message: format!("{key} must be a whole number between 0 and 65535"),
        data: None,
      })
  }

  fn optional_u8(arguments: &serde_json::Value, key: &str) -> Result<Option<u8>, McpError> {
    let Some(value) = arguments.get(key).filter(|value| !value.is_null()) else {
      return Ok(None);
    };
    value
      .as_u64()
      .and_then(|raw| u8::try_from(raw).ok())
      .map(Some)
      .ok_or_else(|| McpError {
        code: -32602,
        message: format!("{key} must be a whole number between 0 and 255"),
        data: None,
      })
  }

  fn optional_u32(arguments: &serde_json::Value, key: &str) -> Result<Option<u32>, McpError> {
    let Some(value) = arguments.get(key).filter(|value| !value.is_null()) else {
      return Ok(None);
    };
    value
      .as_u64()
      .and_then(|raw| u32::try_from(raw).ok())
      .map(Some)
      .ok_or_else(|| McpError {
        code: -32602,
        message: format!("{key} must be a whole number between 0 and 4294967295"),
        data: None,
      })
  }

  /// A cloud failure, rendered as the `{"code":…,"params":{…}}` envelope.
  ///
  /// The backend's own English would be meaningless to an agent deciding what
  /// to do next; a stable code and its parameters are something it can branch
  /// on, and it is the same envelope the desktop and the REST API answer with.
  fn cloud_error(err: crate::cookie_bot::CookieBotError) -> McpError {
    McpError {
      code: -32000,
      message: err.to_error_json(),
      data: None,
    }
  }

  /// Resolve a profile the cookie bot is allowed to touch.
  ///
  /// The same gate the REST surface applies, for the same reason: the bot runs
  /// ONLY on the leased fleet, so a profile that cannot make the round trip to
  /// a remote host and back — never synced, encrypted with a key that never
  /// leaves this machine, no recorded OS, an OS the fleet cannot lease, or no
  /// proxy or VPN to egress through — must never reach an enrolment, a quota
  /// check or a leased host on ANY surface.
  fn cookie_bot_eligible_profile(profile_id: &str) -> Result<BrowserProfile, McpError> {
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
        data: None,
      })?;

    let profile = profiles
      .into_iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
        data: None,
      })?;

    crate::cookie_bot::bot_precondition(&profile, &crate::cookie_bot::exit_reachability(&profile))
      .map_err(|message| McpError {
        code: -32000,
        message,
        data: None,
      })?;
    Ok(profile)
  }

  /// Start this profile on a host of its own operating system.
  ///
  /// Deliberately no `is_cross_os` guard: local `run_profile` refuses a foreign
  /// profile because THIS machine is the wrong OS, and running it on a host of
  /// its own OS is precisely what this exists for.
  async fn handle_run_profile_remote(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let url = arguments
      .get("url")
      .and_then(|v| v.as_str())
      .map(str::to_string);
    if let Some(url) = url.as_deref() {
      validate_navigable_url(url)?;
    }
    let profile = self.get_wayfern_profile(profile_id)?;

    // The host pulls the profile from cloud storage, so one that has never
    // synced would launch an empty browser and push that emptiness back over
    // the real one. Same rule the REST route applies, from the same place.
    crate::api_server::remote_launch_precondition(&profile)
      .await
      .map_err(|message| McpError {
        code: -32000,
        message,
        data: None,
      })?;

    let app = {
      let inner = self.inner.lock().await;
      inner.app_handle.clone().ok_or_else(|| McpError {
        code: -32000,
        message: "MCP server not properly initialized".to_string(),
        data: None,
      })?
    };

    let outcome = crate::remote_session::start_remote_session(app, &profile, url)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e.to_error_json(),
        data: None,
      })?;
    Self::json_content(&outcome)
  }

  /// Stop a remote session and settle what it cost.
  async fn handle_stop_remote_session(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let session_id = Self::require_str(arguments, "session_id")?;
    let outcome = crate::remote_session::end_remote_session(session_id)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e.to_error_json(),
        data: None,
      })?;
    Self::json_content(&outcome)
  }

  async fn handle_list_remote_sessions() -> Result<serde_json::Value, McpError> {
    let sessions = crate::remote_session::list_remote_sessions()
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e.to_error_json(),
        data: None,
      })?;
    Self::json_content(&sessions)
  }

  async fn handle_get_remote_session(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let session_id = Self::require_str(arguments, "session_id")?;
    let state = crate::remote_session::get_remote_session(session_id)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: e.to_error_json(),
        data: None,
      })?;
    Self::json_content(&state)
  }

  async fn handle_get_remote_hours_quota() -> Result<serde_json::Value, McpError> {
    let quota = crate::cookie_bot::remote_hours_quota()
      .await
      .map_err(Self::cloud_error)?;
    Self::json_content(&quota)
  }

  async fn handle_list_cookie_bot_schedules(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let scope = arguments.get("scope").and_then(|value| value.as_str());
    let schedules = crate::cookie_bot::list_schedules(scope)
      .await
      .map_err(Self::cloud_error)?;
    Self::json_content(&schedules)
  }

  async fn handle_get_cookie_bot_schedule(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    // Not gated on eligibility: a profile whose sync was turned off after it
    // was enrolled must still be able to show what it is enrolled as.
    let schedule = crate::cookie_bot::get_schedule(profile_id)
      .await
      .map_err(Self::cloud_error)?;
    Self::json_content(&schedule)
  }

  async fn handle_set_cookie_bot_schedule(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let profile = Self::cookie_bot_eligible_profile(profile_id)?;

    // `bot_precondition` already proved the profile has an OS the fleet can
    // lease. Taking the platform from the profile rather than the arguments is
    // what stops an agent enrolling a macOS profile onto a Windows host.
    let platform = profile
      .resolved_os()
      .ok_or_else(|| McpError {
        code: -32000,
        message: "Profile has no recorded operating system".to_string(),
        data: None,
      })?
      .to_string();

    if let Some(requested) = arguments.get("platform").and_then(|v| v.as_str()) {
      if requested != platform {
        return Err(McpError {
          code: -32602,
          message: format!(
            "platform {requested:?} does not match the profile's own operating system {platform:?}"
          ),
          data: None,
        });
      }
    }

    let enabled = arguments
      .get("enabled")
      .and_then(|value| value.as_bool())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing enabled".to_string(),
        data: None,
      })?;

    let sites = arguments
      .get("sites")
      .and_then(|value| value.as_array())
      .map(|items| {
        items
          .iter()
          .filter_map(|item| item.as_str().map(str::to_string))
          .collect::<Vec<_>>()
      })
      .unwrap_or_default();

    let input = crate::cookie_bot::CookieBotScheduleInput {
      profile_name: arguments
        .get("profile_name")
        .and_then(|value| value.as_str())
        .map_or_else(|| profile.name.clone(), str::to_string),
      platform,
      enabled,
      run_at_minute: Self::require_u16(arguments, "run_at_minute")?,
      days_mask: Self::optional_u8(arguments, "days_mask")?.ok_or_else(|| McpError {
        code: -32602,
        message: "Missing days_mask".to_string(),
        data: None,
      })?,
      timezone: Self::require_str(arguments, "timezone")?.to_string(),
      preset: Self::require_str(arguments, "preset")?.to_string(),
      max_minutes: Self::optional_u32(arguments, "max_minutes")?.ok_or_else(|| McpError {
        code: -32602,
        message: "Missing max_minutes".to_string(),
        data: None,
      })?,
      sites,
      jitter_seconds: Self::optional_u32(arguments, "jitter_seconds")?,
      ..Default::default()
    }
    // Derived from the profile, never from the tool arguments: an agent must not
    // be able to claim a profile has a proxy when it does not.
    .with_profile_state(crate::cookie_bot::profile_state(&profile));

    let acknowledge_conflict = arguments
      .get("acknowledge_conflict")
      .and_then(|value| value.as_bool())
      .unwrap_or(false);

    let saved = crate::cookie_bot::save_schedule(profile_id, &input, acknowledge_conflict)
      .await
      .map_err(Self::cloud_error)?;
    Self::json_content(&saved)
  }

  async fn handle_delete_cookie_bot_schedule(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    // No eligibility gate: a profile that has since become ineligible is
    // exactly the one an agent most needs to be able to unenrol.
    let deleted = crate::cookie_bot::delete_schedule(profile_id)
      .await
      .map_err(Self::cloud_error)?;
    Self::json_content(&deleted)
  }

  async fn handle_check_cookie_bot_conflicts(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    let conflicts = crate::cookie_bot::check_conflicts(
      profile_id,
      Self::optional_u16(arguments, "run_at_minute")?,
      arguments.get("timezone").and_then(|value| value.as_str()),
      Self::optional_u8(arguments, "days_mask")?,
    )
    .await
    .map_err(Self::cloud_error)?;
    Self::json_content(&conflicts)
  }

  async fn handle_list_cookie_bot_runs(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let runs = crate::cookie_bot::list_runs(
      arguments.get("profile_id").and_then(|value| value.as_str()),
      arguments.get("scope").and_then(|value| value.as_str()),
      Self::optional_u32(arguments, "limit")?,
      arguments.get("before").and_then(|value| value.as_str()),
    )
    .await
    .map_err(Self::cloud_error)?;
    Self::json_content(&runs)
  }

  async fn handle_run_cookie_bot_now(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = Self::require_str(arguments, "profile_id")?;
    Self::cookie_bot_eligible_profile(profile_id)?;

    let started =
      crate::cookie_bot::run_now(profile_id, Self::optional_u32(arguments, "max_minutes")?)
        .await
        .map_err(Self::cloud_error)?;
    Self::json_content(&started)
  }

  async fn handle_cancel_cookie_bot_run(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let run_id = Self::require_str(arguments, "run_id")?;
    let run = crate::cookie_bot::cancel_run(run_id)
      .await
      .map_err(Self::cloud_error)?;
    Self::json_content(&run)
  }

  async fn handle_list_cookie_bot_presets() -> Result<serde_json::Value, McpError> {
    // Ids and a rough duration only. What a preset expands to is the server's,
    // and stays there.
    let presets = crate::cookie_bot::list_presets()
      .await
      .map_err(Self::cloud_error)?;
    Self::json_content(&presets)
  }

  async fn handle_get_cookie_bot_usage(
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let usage = crate::cookie_bot::team_usage(arguments.get("period").and_then(|v| v.as_str()))
      .await
      .map_err(Self::cloud_error)?;
    Self::json_content(&usage)
  }
}

lazy_static::lazy_static! {
  static ref MCP_SERVER: McpServer = McpServer::new();
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn the_local_tombstone_answers_gone_with_a_removal_message() {
    // Local MCP is removed; anything that still reaches the loopback port must
    // get a clear 410 with an actionable message, not the tool engine.
    let response = McpServer::handle_local_deprecated().await;
    assert_eq!(response.status(), StatusCode::GONE);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
      .await
      .expect("tombstone body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(body["error"]["code"], -32001);
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
      message.contains("removed") && message.to_lowercase().contains("remote"),
      "the removal message must name the removal and point at remote MCP: {message}"
    );
  }

  #[test]
  fn the_deprecation_dialog_is_throttled_to_one_per_window() {
    // A client retry loop hitting the dead port must not pop the dialog on
    // every attempt: the first attempt in a window advances the clock, the
    // next one inside it is a no-op.
    LAST_LOCAL_DEPRECATION_EMIT.store(0, Ordering::SeqCst);
    McpServer::note_local_mcp_attempt();
    let first = LAST_LOCAL_DEPRECATION_EMIT.load(Ordering::SeqCst);
    assert!(first > 0, "the first attempt records a timestamp");
    McpServer::note_local_mcp_attempt();
    let second = LAST_LOCAL_DEPRECATION_EMIT.load(Ordering::SeqCst);
    assert_eq!(
      first, second,
      "a second attempt inside the window does not re-emit"
    );
  }

  #[test]
  fn test_mcp_tools_count() {
    let server = McpServer::new();
    let tools = server.get_tools();

    // PINNED, not a floor. This read `>= 59` while the server actually served
    // 80, so twenty-one tools could be deleted without the assertion moving -
    // and every one of them is a published contract an MCP client is written
    // against. A floor that sits far below the real number is not a test, it
    // is a comment. Changing this number is the deliberate edit that says a
    // tool was added or removed on purpose.
    assert_eq!(
      tools.len(),
      87,
      "the tool list is a published contract; update this number deliberately"
    );

    // Names are the contract an MCP client is written against, so a duplicate
    // silently shadows one of the two in dispatch and the tool that loses is
    // simply never reachable.
    let mut seen = std::collections::HashSet::new();
    for tool in &tools {
      assert!(
        seen.insert(tool.name.as_str()),
        "duplicate MCP tool name: {}",
        tool.name
      );
    }

    // Check tool names
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    // Profile tools
    assert!(tool_names.contains(&"list_profiles"));
    assert!(tool_names.contains(&"get_profile"));
    assert!(tool_names.contains(&"run_profile"));
    assert!(tool_names.contains(&"kill_profile"));
    assert!(tool_names.contains(&"get_profile_status"));
    // Profile import tools
    assert!(tool_names.contains(&"detect_browser_profiles"));
    assert!(tool_names.contains(&"import_browser_profiles"));
    // Group tools
    assert!(tool_names.contains(&"list_groups"));
    assert!(tool_names.contains(&"get_group"));
    assert!(tool_names.contains(&"create_group"));
    assert!(tool_names.contains(&"update_group"));
    assert!(tool_names.contains(&"delete_group"));
    assert!(tool_names.contains(&"assign_profiles_to_group"));
    // Proxy tools
    assert!(tool_names.contains(&"distribute_proxies"));
    assert!(tool_names.contains(&"list_proxies"));
    assert!(tool_names.contains(&"get_proxy"));
    assert!(tool_names.contains(&"create_proxy"));
    assert!(tool_names.contains(&"update_proxy"));
    assert!(tool_names.contains(&"delete_proxy"));
    // Proxy import/export tools
    assert!(tool_names.contains(&"export_proxies"));
    assert!(tool_names.contains(&"import_proxies"));
    // VPN tools
    assert!(tool_names.contains(&"import_vpn"));
    assert!(tool_names.contains(&"list_vpn_configs"));
    assert!(tool_names.contains(&"delete_vpn"));
    assert!(tool_names.contains(&"connect_vpn"));
    assert!(tool_names.contains(&"disconnect_vpn"));
    assert!(tool_names.contains(&"get_vpn_status"));
    // Fingerprint tools
    assert!(tool_names.contains(&"get_profile_fingerprint"));
    assert!(tool_names.contains(&"update_profile_fingerprint"));
    assert!(tool_names.contains(&"update_profile_proxy_bypass_rules"));
    // Extension tools
    assert!(tool_names.contains(&"list_extensions"));
    assert!(tool_names.contains(&"list_extension_groups"));
    assert!(tool_names.contains(&"add_extension"));
    assert!(tool_names.contains(&"update_extension"));
    assert!(tool_names.contains(&"create_extension_group"));
    assert!(tool_names.contains(&"update_extension_group"));
    assert!(tool_names.contains(&"add_extension_to_group"));
    assert!(tool_names.contains(&"remove_extension_from_group"));
    assert!(tool_names.contains(&"delete_extension"));
    assert!(tool_names.contains(&"delete_extension_group"));
    assert!(tool_names.contains(&"assign_extension_group_to_profile"));
    // Cookie tools
    assert!(tool_names.contains(&"import_profile_cookies"));
    // Team lock tools
    assert!(tool_names.contains(&"get_team_locks"));
    assert!(tool_names.contains(&"get_team_lock_status"));
    // Synchronizer tools
    assert!(tool_names.contains(&"start_sync_session"));
    assert!(tool_names.contains(&"stop_sync_session"));
    assert!(tool_names.contains(&"get_sync_sessions"));
    assert!(tool_names.contains(&"remove_sync_follower"));
    // Browser interaction tools
    assert!(tool_names.contains(&"navigate"));
    assert!(tool_names.contains(&"screenshot"));
    assert!(tool_names.contains(&"evaluate_javascript"));
    assert!(tool_names.contains(&"click_element"));
    assert!(tool_names.contains(&"type_text"));
    assert!(tool_names.contains(&"get_page_content"));
    assert!(tool_names.contains(&"get_page_info"));
    // The agent surface: what an agent reads, how it names things, and how it
    // acts on them without a selector.
    assert!(tool_names.contains(&"perceive_page"));
    assert!(tool_names.contains(&"resolve_locator"));
    assert!(tool_names.contains(&"click_locator"));
    assert!(tool_names.contains(&"type_locator"));
    assert!(tool_names.contains(&"extract_structured"));
    assert!(tool_names.contains(&"pick_element"));
    // Remote fleet: an agent must be able to start a session, see it become
    // usable, drive it with the tools above, and stop it. Any one of those
    // missing makes remote driving unusable from MCP alone.
    assert!(tool_names.contains(&"run_profile_remote"));
    assert!(tool_names.contains(&"stop_remote_session"));
    assert!(tool_names.contains(&"list_remote_sessions"));
    assert!(tool_names.contains(&"get_remote_session"));
    assert!(tool_names.contains(&"get_remote_hours_quota"));
    // Cookie bot
    assert!(tool_names.contains(&"list_cookie_bot_schedules"));
    assert!(tool_names.contains(&"get_cookie_bot_schedule"));
    assert!(tool_names.contains(&"set_cookie_bot_schedule"));
    assert!(tool_names.contains(&"delete_cookie_bot_schedule"));
    assert!(tool_names.contains(&"check_cookie_bot_conflicts"));
    assert!(tool_names.contains(&"list_cookie_bot_runs"));
    assert!(tool_names.contains(&"run_cookie_bot_now"));
    assert!(tool_names.contains(&"cancel_cookie_bot_run"));
    assert!(tool_names.contains(&"list_cookie_bot_presets"));
    assert!(tool_names.contains(&"get_cookie_bot_usage"));
  }

  // A tool advertised in tools/list but missing from dispatch answers "Unknown
  // tool": the client can see it and cannot call it, and nothing else in the
  // build notices.
  //
  // Asserted against the source rather than by dispatching, because half these
  // tools take no arguments — calling them would reach Donut cloud, and a unit
  // test that needs the network is a test that gets deleted.
  #[test]
  fn every_cookie_bot_tool_is_both_advertised_and_dispatchable() {
    let server = McpServer::new();
    let advertised: Vec<String> = server
      .get_tools()
      .into_iter()
      .map(|tool| tool.name)
      .filter(|name| name.contains("cookie_bot") || name.contains("remote"))
      .collect();

    let dispatched = include_str!("mcp_server.rs");
    for name in &advertised {
      assert!(
        dispatched.contains(&format!("\"{name}\" =>")),
        "tool is advertised but has no dispatch arm: {name}"
      );
    }
    assert_eq!(
      advertised.len(),
      15,
      "expected the full remote-fleet and cookie-bot set: {advertised:?}"
    );
  }

  // The bot runs ONLY on the leased fleet. A profile that cannot be
  // materialised on a remote host has no path to a run, and every write tool
  // resolves its profile through this gate before the cloud is asked, so there
  // is no argument shape that points the bot at a local-only profile.
  #[test]
  fn a_profile_the_bot_could_never_run_is_refused_before_the_cloud_is_asked() {
    use crate::profile::types::SyncMode;

    let eligible = || BrowserProfile {
      id: uuid::Uuid::nil(),
      name: "warm me".to_string(),
      browser: "wayfern".to_string(),
      version: "latest".to_string(),
      sync_mode: SyncMode::Regular,
      host_os: Some("macos".to_string()),
      proxy_id: Some("proxy-1".to_string()),
      ..Default::default()
    };

    assert!(crate::cookie_bot::bot_precondition(
      &eligible(),
      &crate::remote_exit::ExitReachability::Remote
    )
    .is_ok());

    let mut local_only = eligible();
    local_only.sync_mode = SyncMode::Disabled;
    assert!(
      crate::cookie_bot::bot_precondition(
        &local_only,
        &crate::remote_exit::ExitReachability::Remote
      )
      .is_err(),
      "a profile with no cloud copy has nothing for a host to open"
    );

    let mut linux = eligible();
    linux.host_os = Some("linux".to_string());
    assert!(
      crate::cookie_bot::bot_precondition(&linux, &crate::remote_exit::ExitReachability::Remote)
        .is_ok(),
      "the fleet serves linux from a linux instance"
    );

    let mut android = eligible();
    android.host_os = Some("android".to_string());
    assert!(
      crate::cookie_bot::bot_precondition(&android, &crate::remote_exit::ExitReachability::Remote)
        .is_err(),
      "the fleet has no android host to lease"
    );

    let mut datacenter_egress = eligible();
    datacenter_egress.proxy_id = None;
    datacenter_egress.vpn_id = None;
    assert!(
      crate::cookie_bot::bot_precondition(
        &datacenter_egress,
        &crate::remote_exit::ExitReachability::None
      )
      .is_err(),
      "hours of traffic from a hosting ASN damages the identity being warmed"
    );
  }

  // Enrolment carries only the user's own scalars. Anything describing what a
  // run actually does appearing in the schema would mean the browsing model had
  // leaked out of the server and into this AGPL client.
  #[test]
  fn the_bot_tools_expose_choices_not_behaviour() {
    let server = McpServer::new();
    let tools = server.get_tools();

    let presets = tools
      .iter()
      .find(|tool| tool.name == "list_cookie_bot_presets")
      .expect("list_cookie_bot_presets tool");
    assert_eq!(
      presets.input_schema["properties"]
        .as_object()
        .map(serde_json::Map::len),
      Some(0),
      "a preset is chosen by id; it takes no behaviour parameters"
    );

    let set = tools
      .iter()
      .find(|tool| tool.name == "set_cookie_bot_schedule")
      .expect("set_cookie_bot_schedule tool");
    let properties = set.input_schema["properties"]
      .as_object()
      .expect("schedule properties");
    for leaked in [
      "dwell",
      "dwell_seconds",
      "scroll",
      "clicks",
      "steps",
      "actions",
      "corpus",
      "user_agent",
    ] {
      assert!(
        !properties.contains_key(leaked),
        "the browsing model leaked into the tool contract: {leaked}"
      );
    }

    // `platform` is accepted but not required: this machine already knows the
    // profile's operating system, and a supplied one that disagrees is
    // refused rather than honoured.
    let required = set.input_schema["required"]
      .as_array()
      .expect("required fields");
    assert!(!required.iter().any(|field| field == "platform"));
    assert!(required.iter().any(|field| field == "profile_id"));
    assert!(required.iter().any(|field| field == "preset"));
  }

  #[tokio::test]
  async fn a_body_past_the_shared_cap_is_refused_by_the_engine_itself() {
    // MAX_MESSAGE_BYTES is documented as one number for every transport, and it
    // must match what the cloud endpoint accepts. Nothing exercised the
    // check: deleting it left every test green, so the loopback listener and
    // the bridge could quietly drift apart again, which is exactly the defect
    // that made `import_profile_cookies` work locally and 413 remotely.
    let server = McpServer::instance();
    server.mark_engine_ready_for_tests();

    // Deliberately VALID JSON-RPC, just too big. A body of junk bytes would be
    // refused as unparsable whether or not the cap exists, so it would assert
    // nothing about the cap, the same two-paths-one-outcome trap that made an
    // earlier test in this feature pass against deleted code.
    let filler = "x".repeat(McpServer::MAX_MESSAGE_BYTES);
    let oversized = format!(
      r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"navigate","arguments":{{"url":"{filler}"}}}}}}"#
    );
    assert!(oversized.len() > McpServer::MAX_MESSAGE_BYTES);
    assert!(
      serde_json::from_str::<serde_json::Value>(&oversized).is_ok(),
      "the fixture must be parseable, or the cap is not what rejects it"
    );
    assert!(matches!(
      server
        .handle_message(McpOrigin::Loopback, None, oversized.as_bytes())
        .await,
      McpOutcome::BadRequest
    ));

    // And a body just under it is judged on its content, not its size, the cap
    // must not be doing the rejecting for ordinary calls.
    let ok = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    assert!(ok.len() < McpServer::MAX_MESSAGE_BYTES);
    assert!(matches!(
      server.handle_message(McpOrigin::Loopback, None, ok).await,
      McpOutcome::Body { .. }
    ));
  }

  /// Blank out comments and string literals so brace counting sees only code.
  ///
  /// Lengths and line breaks are preserved, so offsets and line numbers still
  /// line up with the original source.
  fn code_only(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;

    // Blank a span, keeping newlines so line numbers survive.
    let blank = |out: &mut String, from: usize, to: usize| {
      for &c in &chars[from..to] {
        out.push(if c == '\n' { '\n' } else { ' ' });
      }
    };

    while i < chars.len() {
      let c = chars[i];

      // A line comment: everything to the end of the line.
      if c == '/' && chars.get(i + 1) == Some(&'/') {
        let mut end = i;
        while end < chars.len() && chars[end] != '\n' {
          end += 1;
        }
        blank(&mut out, i, end);
        i = end;
        continue;
      }

      // A raw string: `r`, any number of `#`, then the quote. Only when the `r`
      // starts a token, or the tail of an identifier such as `for` opens one.
      let starts_token = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
      if c == 'r' && starts_token {
        let mut hashes = 0;
        while chars.get(i + 1 + hashes) == Some(&'#') {
          hashes += 1;
        }
        if chars.get(i + 1 + hashes) == Some(&'"') {
          let closing = format!("\"{}", "#".repeat(hashes));
          let body_start = i + 2 + hashes;
          let rest: String = chars[body_start..].iter().collect();
          let end = rest
            .find(&closing)
            .map_or(chars.len(), |at| body_start + rest[..at].chars().count());
          blank(&mut out, i, end);
          i = end + closing.chars().count();
          blank(&mut out, end, i.min(chars.len()));
          continue;
        }
      }

      // An ordinary string, with backslash escapes.
      if c == '"' {
        let mut end = i + 1;
        while end < chars.len() && chars[end] != '"' {
          end += if chars[end] == '\\' { 2 } else { 1 };
        }
        let end = end.min(chars.len());
        blank(&mut out, i, end);
        i = end;
        if i < chars.len() {
          out.push(' ');
          i += 1;
        }
        continue;
      }

      out.push(c);
      i += 1;
    }

    out
  }

  /// Every line (1-based) that binds a guard on the engine mutex and then
  /// reaches an `.await` while that guard is still alive.
  ///
  /// The discriminator is SCOPE, not spelling. A guard lives until the block
  /// that owns it closes, so the question is whether that closing brace comes
  /// before the next await:
  ///
  /// ```ignore
  /// let handle = {                             // safe: the block ends first
  ///   let inner = self.inner.lock().await;
  ///   inner.app_handle.as_ref().ok_or(..)?.clone()
  /// };
  /// something(&handle).await;
  ///
  /// let inner = self.inner.lock().await;       // held: no enclosing block
  /// let handle = inner.app_handle.as_ref().ok_or(..)?.clone();
  /// something(&handle).await;                  // the whole engine is frozen
  /// ```
  ///
  /// The previous version keyed on `.clone()` in the binding statement, which
  /// the second shape also has, so it skipped a real violation as though it
  /// were the safe one.
  fn engine_locks_held_across_an_await(source: &str) -> (Vec<usize>, usize) {
    let code = code_only(source);
    let chars: Vec<char> = code.chars().collect();

    // Char offset where each line begins, so a hit can be reported by line.
    let mut line_of = Vec::with_capacity(chars.len() + 1);
    let mut line = 1usize;
    for &c in &chars {
      line_of.push(line);
      if c == '\n' {
        line += 1;
      }
    }
    line_of.push(line);

    let needle: Vec<char> = "self.inner.lock().await;".chars().collect();
    let at = |i: usize, pat: &[char]| chars[i..].starts_with(pat);
    let await_pat: Vec<char> = ".await".chars().collect();
    let drop_pat: Vec<char> = "drop(inner)".chars().collect();

    let mut offenders = Vec::new();
    let mut sites = 0usize;

    let mut i = 0;
    while i < chars.len() {
      if !at(i, &needle) {
        i += 1;
        continue;
      }
      // Only a BOUND guard outlives its statement. `self.inner.lock().await.x`
      // is a temporary that dies at the semicolon.
      let line_start = chars[..i]
        .iter()
        .rposition(|&c| c == '\n')
        .map_or(0, |newline| newline + 1);
      let prefix: String = chars[line_start..i].iter().collect();
      if !prefix.trim_start().starts_with("let ") {
        i += 1;
        continue;
      }
      sites += 1;

      // Walk forward from the end of the lock statement. `depth` counts how
      // deep we are INSIDE the guard's own block; the `}` that takes it to -1
      // is the one that drops the guard.
      let mut depth = 0i32;
      let mut cursor = i + needle.len();
      while cursor < chars.len() {
        match chars[cursor] {
          '{' => depth += 1,
          '}' => {
            if depth == 0 {
              break; // the enclosing block closed: the guard is gone
            }
            depth -= 1;
          }
          _ => {
            if at(cursor, &drop_pat) {
              break; // released by hand before the await
            }
            if at(cursor, &await_pat) {
              offenders.push(line_of[i]);
              break;
            }
          }
        }
        cursor += 1;
      }

      i += needle.len();
    }

    (offenders, sites)
  }

  #[test]
  fn no_handler_holds_the_engine_lock_across_an_await() {
    // `handle_message` takes this same mutex to validate the session on EVERY
    // request, and to serve `initialize`. A handler that keeps the guard alive
    // across a browser launch therefore freezes the whole engine for the
    // duration: the launching agent's follow-up calls, a second agent, the
    // website console, and even a new client trying to open a session.
    //
    // The detector is exercised in BOTH directions first, because the previous
    // one could only be pointed at this file, and a rule that has never been
    // shown to fire is indistinguishable from one that cannot.
    let unscoped = r#"
  async fn bad(&self) -> Result<(), McpError> {
    let inner = self.inner.lock().await;
    let app_handle = inner
      .app_handle
      .as_ref()
      .ok_or_else(|| McpError { code: -32000, message: "no handle".to_string(), data: None })?
      .clone();
    something(&app_handle).await;
    Ok(())
  }
"#;
    let (flagged, sites) = engine_locks_held_across_an_await(unscoped);
    assert_eq!(sites, 1, "the fixture must be seen as a lock site at all");
    assert_eq!(
      flagged,
      vec![3],
      "a guard bound at statement level is alive at the await, whatever the \
       binding does with `.clone()`"
    );

    let scoped = r#"
  async fn good(&self) -> Result<(), McpError> {
    let app_handle = {
      let inner = self.inner.lock().await;
      inner
        .app_handle
        .as_ref()
        .ok_or_else(|| McpError { code: -32000, message: "no handle".to_string(), data: None })?
        .clone()
    };
    something(&app_handle).await;
    Ok(())
  }
"#;
    let (flagged, sites) = engine_locks_held_across_an_await(scoped);
    assert_eq!(sites, 1);
    assert!(
      flagged.is_empty(),
      "the block expression drops the guard before the await: {flagged:?}"
    );

    // And a guard let go by hand is not a violation either.
    let released = r#"
  async fn also_good(&self) -> Result<(), McpError> {
    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.clone();
    drop(inner);
    something(&app_handle).await;
    Ok(())
  }
"#;
    let (flagged, _) = engine_locks_held_across_an_await(released);
    assert!(
      flagged.is_empty(),
      "an explicit drop releases it: {flagged:?}"
    );

    // Now the file itself.
    let source = include_str!("mcp_server.rs");
    let (offenders, sites) = engine_locks_held_across_an_await(source);
    assert!(
      sites >= 25,
      "the scan found only {sites} lock sites, so it has gone blind to most of \
       the file, the same way the line-window version silently matched none"
    );
    assert!(
      offenders.is_empty(),
      "these lines hold the engine mutex across an await, freezing every \
       other MCP message for the duration: {offenders:?}. Clone the handle \
       inside a block expression so the guard drops first."
    );
  }

  #[test]
  fn initialize_echoes_a_protocol_version_the_client_can_accept() {
    // The client's check is unforgiving: the official SDK compares the
    // answer against ITS OWN supported list and throws
    // "Server's protocol version is not supported" on a miss. Answering with
    // our newest regardless made the whole remote feature unreachable from any
    // agent that had not upgraded in lockstep with us.
    for asked in SUPPORTED_PROTOCOL_VERSIONS {
      assert_eq!(
        negotiate_protocol_version(Some(asked)),
        *asked,
        "a version we speak must be echoed back verbatim"
      );
    }

    // Unknown or absent: answer with one we do support, so the client can
    // decide for itself rather than be handed something meaningless.
    assert_eq!(negotiate_protocol_version(None), PROTOCOL_VERSION);
    assert_eq!(
      negotiate_protocol_version(Some("1999-01-01")),
      PROTOCOL_VERSION
    );
    assert_eq!(negotiate_protocol_version(Some("")), PROTOCOL_VERSION);
    assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&PROTOCOL_VERSION));
  }

  #[tokio::test]
  async fn an_older_client_is_answered_in_its_own_dialect() {
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();

    let init = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}"#;
    let McpOutcome::Body { body, .. } =
      server.handle_message(McpOrigin::Loopback, None, init).await
    else {
      panic!("initialize must answer");
    };
    assert_eq!(
      body["result"]["protocolVersion"], "2024-11-05",
      "the client asked in 2024-11-05 and must be answered in it: {body}"
    );

    // And a client that names nothing still gets a usable answer.
    let bare = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let McpOutcome::Body { body, .. } =
      server.handle_message(McpOrigin::Loopback, None, bare).await
    else {
      panic!("initialize must answer");
    };
    assert_eq!(body["result"]["protocolVersion"], PROTOCOL_VERSION);
  }

  #[test]
  fn the_browser_is_never_told_to_open_a_local_file() {
    // Over the CLOUD BRIDGE this is the difference between "control your
    // browser" and an arbitrary local-file read reachable from the internet:
    // `Page.navigate` loads `file:///…/.ssh/id_rsa` happily, and
    // `get_page_content` hands the bytes back to whoever asked.
    for blocked in [
      "file:///etc/passwd",
      "FILE:///etc/passwd",
      "  file:///etc/passwd  ",
      "file://localhost/etc/passwd",
      "data:text/html,<script>fetch('/etc/passwd')</script>",
      "javascript:alert(1)",
      "chrome://settings",
      "devtools://devtools/bundled/inspector.html",
      "blob:https://example.com/abc",
      "view-source:file:///etc/passwd",
      "",
      "/etc/passwd",
      "\\\\server\\share",
    ] {
      let refused = validate_navigable_url(blocked);
      assert!(refused.is_err(), "{blocked:?} must be refused");
      assert!(
        refused
          .unwrap_err()
          .message
          .contains("URL_SCHEME_NOT_ALLOWED"),
        "{blocked:?} must refuse with a code the UI can translate"
      );
    }

    // And the ones a browser is actually asked to browse still work.
    for allowed in [
      "http://example.com",
      "https://example.com/path?q=1#frag",
      "HTTPS://EXAMPLE.COM",
      "about:blank",
      "  https://example.com  ",
    ] {
      assert!(
        validate_navigable_url(allowed).is_ok(),
        "{allowed:?} must be allowed"
      );
    }
  }

  #[test]
  fn every_url_entry_point_is_guarded() {
    // Derived from the source, and deliberately BROAD. Two earlier versions
    // could not fail for the drift they existed to catch:
    //   - `count >= 6` plus four hand-typed handler names saw nothing new;
    //   - keying on the literal `get("url")` skipped any handler reading the
    //     same argument through `require_str`, which is this file's newer
    //     idiom, and the scan counted ITSELF as a fifth guarded handler, so
    //     the floor tolerated a real one silently dropping out.
    // The test module is therefore cut off before scanning, and any method
    // mentioning a "url" argument at all must validate.
    let full = include_str!("mcp_server.rs");
    let source = full
      .split_once("\n#[cfg(test)]")
      .map(|(code, _)| code)
      .unwrap_or(full);

    let starts: Vec<usize> = source
      .match_indices("\n  ")
      .filter(|(i, _)| {
        let rest = &source[i + 3..];
        [
          "fn ",
          "async fn ",
          "pub fn ",
          "pub async fn ",
          "pub(crate) fn ",
          "pub(crate) async fn ",
        ]
        .iter()
        .any(|p| rest.starts_with(p))
      })
      .map(|(i, _)| i)
      .collect();
    assert!(starts.len() > 20, "method scan found {}", starts.len());

    let mut unguarded = Vec::new();
    let mut guarded = Vec::new();
    for (n, &begin) in starts.iter().enumerate() {
      let stop = starts.get(n + 1).copied().unwrap_or(source.len());
      let body = &source[begin..stop];
      // ANY method that reads a "url" argument from the caller, however it
      // spells the read.
      if !(body.contains("arguments") && body.contains(r#""url""#)) {
        continue;
      }
      let name = body
        .trim_start()
        .trim_start_matches("pub(crate) ")
        .trim_start_matches("pub ")
        .trim_start_matches("async ")
        .trim_start_matches("fn ")
        .split(['(', '<'])
        .next()
        .unwrap_or("?")
        .to_string();
      if body.contains("validate_navigable_url") || body.contains("require_navigable_url") {
        guarded.push(name);
      } else {
        unguarded.push(name);
      }
    }

    assert!(
      unguarded.is_empty(),
      "these handlers take a url from the caller and never validate its \
       scheme, which is how `file:///…` plus a content tool became a remote \
       file read: {unguarded:?}"
    );
    // Pinned exactly, so a handler that DISAPPEARS from the scan, renamed,
    // reformatted past the matcher, or deleted, fails loudly instead of
    // shrinking the set the test believes it is protecting.
    guarded.sort();
    assert_eq!(
      guarded,
      vec![
        "handle_batch_run_profiles",
        "handle_navigate",
        "handle_run_profile",
        "handle_run_profile_remote",
      ],
      "the set of url-taking handlers changed; add the new one here once it \
       validates, or find out why one stopped being seen"
    );
  }

  #[test]
  fn two_callers_never_share_an_element_index_cache() {
    // `get_interactive_elements` stashes live element references on the page and
    // hands back indices; `click_by_index` resolves an index against that stash.
    // It used to be ONE `window.__donut_interactive` array for the whole page,
    // and then one per TRANSPORT, which is still shared, because the website
    // console and an agent both arrive over the bridge, as do two runs of the
    // same agent. Session A lists elements, B lists them, A's
    // `click_by_index(3)` resolves against B's array and clicks the wrong
    // thing while reporting success, on somebody's real browser.
    fn slot(origin: McpOrigin, session: &str) -> String {
      interactive_cache_slot(origin, session)
    }

    let a = slot(McpOrigin::Bridge, "11111111-2222-3333-4444-555555555555");
    let b = slot(McpOrigin::Bridge, "66666666-7777-8888-9999-000000000000");
    assert_ne!(
      a, b,
      "two sessions on ONE transport must not share a slot, this is the \
       collision that origin-only keying could not see"
    );

    // Stable for one session, or an agent's own second call would lose the
    // array its first call built.
    assert_eq!(
      a,
      slot(McpOrigin::Bridge, "11111111-2222-3333-4444-555555555555"),
      "the same session must resolve to the same slot every time"
    );

    // The transport still separates callers that share a session id.
    assert_ne!(
      a,
      slot(McpOrigin::Loopback, "11111111-2222-3333-4444-555555555555")
    );

    // DISTINCTNESS, which the previous key only claimed. Truncating at 64
    // characters and mapping everything outside [A-Za-z0-9_] to `_` collapsed
    // both of these pairs onto ONE slot, and a collision here is precisely the
    // wrong click the key exists to prevent.
    let shared_prefix = "s".repeat(64);
    assert_ne!(
      slot(McpOrigin::Bridge, &format!("{shared_prefix}-one")),
      slot(McpOrigin::Bridge, &format!("{shared_prefix}-two")),
      "two ids agreeing on a 64-character prefix must not share a slot"
    );
    assert_ne!(
      slot(McpOrigin::Bridge, "abc-def"),
      slot(McpOrigin::Bridge, "abc_def"),
      "ids differing only in a character the old key erased must not share a slot"
    );

    // Quoted JS string literals, because they are substituted into
    // `window[...]`. An unquoted identifier would read a different global, and
    // the session half is the only caller-supplied part, so nothing in it may
    // close the quote or paste an expression.
    let overlong = "z".repeat(5_000);
    for candidate in [
      a,
      slot(McpOrigin::Bridge, "'; window.x = 1; //"),
      slot(McpOrigin::Loopback, "../../etc\\passwd"),
      slot(McpOrigin::Bridge, ""),
      slot(McpOrigin::Bridge, overlong.as_str()),
    ] {
      assert!(
        candidate.starts_with('\'') && candidate.ends_with('\''),
        "the slot must be a quoted JS string, got {candidate}"
      );
      let inside = &candidate[1..candidate.len() - 1];
      assert!(
        inside
          .chars()
          .all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "a caller-supplied id must not survive into the slot unsanitised: {candidate}"
      );
      assert!(
        candidate.len() < 128,
        "a caller must not be able to grow the evaluated script: {} chars",
        candidate.len()
      );
    }

    // And every script that touches the cache goes through the substitution
    // rather than naming a slot directly, or one of them would keep writing to
    // the shared global while the others moved.
    let source = include_str!("mcp_server.rs");
    let production = source
      .split_once("\n#[cfg(test)]")
      .map_or(source, |(code, _)| code);
    assert!(
      !production.contains("window.__donut_interactive"),
      "no script may name the shared global directly any more"
    );
    assert_eq!(
      production.matches("window[__CACHE__]").count()
        + production.matches("window[{cache}]").count()
        + production.matches("window[{slot}]").count(),
      7,
      "the seven cache sites (one write, four reads, two in the teardown) must \
       all be parameterised"
    );
  }

  #[tokio::test]
  async fn an_index_tool_without_a_session_is_refused_not_guessed_at() {
    // The hole the per-session key left open. `handle_message` serves a client
    // that never called `initialize`, and the bridge passes whatever `sessionId`
    // the relay frame carried, including none, so every sessionless caller
    // used to land on ONE literal `anon` slot per transport. Two of them then
    // shared an array, and session A's `click_by_index(3)` clicked whatever sat
    // third in B's snapshot and reported "Clicked element at index 3".
    //
    // There is no safe slot to give such a caller, so it is refused. The
    // refusal has to come FIRST, before argument parsing and before anything
    // touches the page.
    let server = McpServer::new();
    let sessionless = McpCaller {
      origin: McpOrigin::Bridge,
      session: None,
    };
    let with_session = McpCaller {
      origin: McpOrigin::Bridge,
      session: Some("11111111-2222-3333-4444-555555555555"),
    };
    // Deliberately complete arguments: the refusal must be about the missing
    // session, not about anything the caller forgot to send.
    let args = serde_json::json!({
      "profile_id": "00000000-0000-0000-0000-000000000000",
      "index": 3,
      "text": "hello",
    });

    let refusals = [
      server
        .handle_get_interactive_elements(sessionless, &args)
        .await
        .expect_err("listing must refuse a sessionless caller"),
      server
        .handle_click_by_index(sessionless, &args)
        .await
        .expect_err("clicking by index must refuse a sessionless caller"),
      server
        .handle_type_by_index(sessionless, &args)
        .await
        .expect_err("typing by index must refuse a sessionless caller"),
    ];
    for refusal in &refusals {
      assert_eq!(
        refusal.code, -32600,
        "a missing session is a bad request, not a tool failure: {}",
        refusal.message
      );
      assert!(
        refusal.message.contains("initialize"),
        "the refusal must tell the agent what to do about it: {}",
        refusal.message
      );
    }

    // And the refusal is about the SESSION, not a coincidence of this fixture:
    // the same call WITH a session gets past the gate and fails later, on the
    // profile that does not exist.
    let later = server
      .handle_click_by_index(with_session, &args)
      .await
      .expect_err("no such profile in a unit test");
    assert!(
      !later.message.contains("initialize"),
      "a caller that HAS a session must not be refused for lacking one: {}",
      later.message
    );
  }

  #[tokio::test]
  async fn ending_a_session_releases_the_page_globals_it_left_behind() {
    // Every session mints its own page global, and before this nothing ever
    // deleted one: `end_session` dropped the server-side record only, and no
    // CDP call removed the array. A long-lived tab therefore accumulated one
    // array of live element references per session that had ever listed it,
    // pinning detached nodes for as long as the page stayed open.
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();

    let init = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let McpOutcome::Body { new_session_id, .. } =
      server.handle_message(McpOrigin::Bridge, None, init).await
    else {
      panic!("initialize must mint a session");
    };
    let session = new_session_id.expect("session id");
    let slot = interactive_cache_slot(McpOrigin::Bridge, &session);

    server
      .remember_cached_page(&session, "profile-a", &slot)
      .await;
    server
      .remember_cached_page(&session, "profile-b", &slot)
      .await;
    // Recorded once per page, not once per call.
    server
      .remember_cached_page(&session, "profile-a", &slot)
      .await;
    assert_eq!(
      server.inner.lock().await.sessions[&session]
        .cached_pages
        .len(),
      2,
      "a session must remember every page it wrote a snapshot to, once each"
    );

    // Ending it takes the pages with it. Neither profile exists here, so the
    // CDP delete cannot land, which is exactly the path that must still leave
    // no server-side record behind, and must still return promptly.
    server.end_session(&session).await;
    let inner = server.inner.lock().await;
    assert!(
      !inner.sessions.contains_key(&session),
      "the session itself must be gone"
    );
    drop(inner);

    // A snapshot taken after the teardown must not resurrect the session.
    server
      .remember_cached_page(&session, "profile-c", &slot)
      .await;
    assert!(
      !server.inner.lock().await.sessions.contains_key(&session),
      "a late write must not recreate a session nothing will ever clean up"
    );
  }

  #[test]
  fn one_page_cannot_accumulate_unbounded_element_caches() {
    // The teardown above only fires when a session ENDS, and no first-party
    // client sends `end_session`, an evicted, crashed or simply abandoned
    // session never will. So the page bounds itself too: the enumeration
    // script keeps a registry of the slots it has written and deletes the
    // oldest once the cap is passed.
    let script = INTERACTIVE_ELEMENTS_JS
      .replace("__MAX_CHARS__", "40000")
      .replace("__CACHE__", "'__donut_interactive_bridge_abc'")
      .replace("__REGISTRY__", INTERACTIVE_SLOT_REGISTRY)
      .replace("__MAX_SLOTS__", &MAX_CACHE_SLOTS_PER_PAGE.to_string());

    assert!(
      !script.contains("__MAX_CHARS__")
        && !script.contains("__CACHE__")
        && !script.contains("__REGISTRY__")
        && !script.contains("__MAX_SLOTS__"),
      "every placeholder must be substituted, or the page throws instead of listing"
    );
    assert!(
      script.contains(&format!("kept.length > {MAX_CACHE_SLOTS_PER_PAGE}")),
      "the cap must be a real number in the emitted script"
    );
    assert!(
      script.contains("delete window[evicted]"),
      "passing the cap must actually delete the evicted slot, not merely stop \
       tracking it"
    );
    assert!(
      (1..=32).contains(&MAX_CACHE_SLOTS_PER_PAGE),
      "a cap of {MAX_CACHE_SLOTS_PER_PAGE} is either no cap at all or too tight \
       for the callers legitimately driving one page"
    );

    // The teardown deletes the same global the script writes, or `end_session`
    // clears a slot nobody uses while the real one leaks on.
    let production = include_str!("mcp_server.rs")
      .split_once("\n#[cfg(test)]")
      .map_or("", |(code, _)| code);
    assert!(
      production.contains("delete window[{slot}]"),
      "end_session must delete the page global itself"
    );
    assert!(
      production.contains("session.cached_pages"),
      "end_session must take the pages from the session it removes"
    );
  }

  #[tokio::test]
  async fn the_session_the_transport_validated_is_the_one_the_cache_keys_on() {
    // The slot is only per-session if the session actually REACHES the cache.
    // `handle_message` is the one place both transports hand a session id in,
    // so the wiring from there down to the three cache sites is asserted here:
    // without it `interactive_cache_slot` could be perfectly correct and every
    // caller would still be handed a slot chosen from nothing.
    let production = include_str!("mcp_server.rs")
      .split_once("\n#[cfg(test)]")
      .map_or("", |(code, _)| code);

    let flattened: String = production.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
      flattened.contains("McpCaller { origin, session: session_id"),
      "handle_message must pass the caller's session on to the dispatcher, or \
       every caller is refused the index tools it is entitled to"
    );
    for handler in [
      "handle_get_interactive_elements",
      "handle_click_by_index",
      "handle_type_by_index",
    ] {
      let body = production
        .split(&format!("async fn {handler}("))
        .nth(1)
        .unwrap_or_else(|| panic!("{handler} must exist"));
      let signature = &body[..body.find(") ->").unwrap_or(body.len())];
      assert!(
        signature.contains("caller: McpCaller"),
        "{handler} must be told WHO is asking, not just how they got here: \
         {signature}"
      );
    }

    // End to end: two sessions, one transport, and the engine must not answer
    // the second one's `click_by_index` from the first one's snapshot. Both
    // calls fail here (no browser in a unit test), so what is asserted is that
    // the sessions are distinct and both survive the session check, the
    // per-session slot is asserted directly above.
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();
    let init = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;

    let mut ids = Vec::new();
    for _ in 0..2 {
      let McpOutcome::Body { new_session_id, .. } =
        server.handle_message(McpOrigin::Bridge, None, init).await
      else {
        panic!("initialize must answer");
      };
      ids.push(new_session_id.expect("initialize must mint a session id"));
    }
    assert_ne!(
      ids[0], ids[1],
      "two initializes on one transport must be two different sessions"
    );
    assert_ne!(
      interactive_cache_slot(McpOrigin::Bridge, &ids[0]),
      interactive_cache_slot(McpOrigin::Bridge, &ids[1]),
      "two live sessions on one transport must not resolve to one slot"
    );

    // Both survive the session check, so neither is refused for a reason other
    // than the one under test.
    let ping = br#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#;
    for id in &ids {
      assert!(
        matches!(
          server
            .handle_message(McpOrigin::Bridge, Some(id.as_str()), ping)
            .await,
          McpOutcome::Body { .. }
        ),
        "a freshly minted session must be usable"
      );
    }
  }

  #[test]
  fn a_typing_request_that_would_run_for_hours_is_refused_before_it_starts() {
    // The DECISION, exercised. An earlier version of this test asserted only
    // that the plan arithmetic crossed the bound and that the constant appeared
    // before `target.connect()` in the source, and neither noticed the guard
    // being turned into `if false && planned > MAX_TYPING_SECONDS`, nor the
    // bound being raised past anything reachable. Both mutations were MISSED.
    // Calling the function that makes the decision catches both.
    let ordinary = plan_typing("hello there", None, MAX_TYPING_SECONDS).expect("an ordinary field");
    assert!(!ordinary.is_empty(), "a real plan must be produced");

    // `session_wpm` is floored at 10 in human_typing.rs, so this is the slowest
    // a plan can be, and the text length, which nothing bounds, is what makes
    // the hours reachable.
    // 1,000 characters, not 20,000. At the wpm floor a keystroke costs
    // 60 / (10 * 5) = 1.2s, so ~250 characters already crosses the 300s bound -
    // and generating a 20,000 character plan took this one test 24 seconds, on
    // a suite that otherwise runs in seven.
    // Still within MAX_TYPING_CHARS, so this exercises the DURATION bound
    // rather than the length one: 1,000 characters at the wpm floor plans in
    // well under a second but would take over an hour to type.
    let refusal = plan_typing(&"a".repeat(1_000), Some(10.0), MAX_TYPING_SECONDS)
      .expect_err("1,000 characters at the slowest rate must be refused");
    assert_eq!(refusal.code, -32602);
    assert!(
      refusal.message.contains("TYPING_TOO_LONG"),
      "the refusal must carry a translatable code: {}",
      refusal.message
    );
    // The numbers travel with it, so the caller is told what to change rather
    // than only that it was too much.
    let body: serde_json::Value =
      serde_json::from_str(&refusal.message).expect("the code envelope is JSON");
    assert_eq!(body["params"]["limit"], "300");
    assert!(
      body["params"]["seconds"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .is_some_and(|seconds| seconds > MAX_TYPING_SECONDS),
      "the reported duration must be the one that broke the bound: {body}"
    );

    // The LENGTH bound must bite before the plan is built, or the caller pays
    // the superlinear planning cost of an arbitrarily long string first. Timed,
    // because "it refuses" is not the property, "it refuses CHEAPLY" is.
    let huge = "a".repeat(400_000);
    let started = std::time::Instant::now();
    let long_refusal = plan_typing(&huge, None, MAX_TYPING_SECONDS).expect_err("400,000 chars");
    let took = started.elapsed();
    assert!(
      long_refusal.message.contains("TYPING_TOO_LONG"),
      "{}",
      long_refusal.message
    );
    assert!(
      took < std::time::Duration::from_millis(500),
      "the refusal must not require planning the text first; took {took:?}"
    );

    // And the sending path can no longer refuse at all: it is handed a plan
    // that was already accepted.
    let source = include_str!("mcp_server.rs");
    let production = source
      .split_once("\n#[cfg(test)]")
      .map_or(source, |(code, _)| code);
    let sender = production
      .split("async fn send_planned_keystrokes(")
      .nth(1)
      .expect("send_planned_keystrokes must exist");
    let sender = &sender[..sender.find("\n  }").unwrap_or(sender.len())];
    assert!(
      !sender.contains("plan_typing"),
      "the sender must take the accepted plan, not decide for itself, \
       deciding here is what put the refusal after the field was emptied"
    );
    assert!(
      !production.contains("send_human_keystrokes"),
      "the plan-it-yourself entry point must be gone, or a handler can call \
       it and reintroduce the late refusal"
    );
  }

  #[test]
  fn a_refused_typing_request_has_not_already_emptied_the_field() {
    // The ORDER, which the duration bound alone never pinned. Both typing
    // handlers used to focus the element and run `el.value = ''` first and call
    // `plan_typing` only afterwards, so a TYPING_TOO_LONG was a lie: the server
    // answered "refused" with the customer's form field already wiped. Nothing
    // may touch the page before the plan is accepted, on EITHER branch of
    // `clear_first`.
    let production = include_str!("mcp_server.rs")
      .split_once("\n#[cfg(test)]")
      .map_or("", |(code, _)| code);

    for handler in ["handle_type_text", "handle_type_by_index"] {
      let rest = production
        .split(&format!("async fn {handler}("))
        .nth(1)
        .unwrap_or_else(|| panic!("{handler} must exist"));
      let body = &rest[..rest.find("\n  async fn ").unwrap_or(rest.len())];

      let plan = body
        .find("plan_typing(text, wpm, max_typing_seconds(caller.origin))?")
        .unwrap_or_else(|| panic!("{handler} must plan through the bounded builder"));
      assert_eq!(
        body.matches("plan_typing(").count(),
        1,
        "{handler} must plan exactly once, before the page is touched"
      );

      // Both branches of `clear_first` are present, and BOTH are built after
      // the plan. The clearing branch is the one that destroys data; the other
      // still focuses and scrolls, which is a mutation the caller can see.
      assert_eq!(
        body.matches("el.value = ''").count(),
        1,
        "{handler} must still have exactly one clearing branch to order"
      );
      assert_eq!(
        body.matches("el.focus();").count(),
        2,
        "{handler} must have both a clearing and a non-clearing focus branch"
      );

      for mutation in [
        "el.value = ''",
        "el.focus();",
        "scrollIntoView",
        "let focus_js",
        ".send_cdp(",
        "send_planned_keystrokes",
        "Input.insertText",
      ] {
        let at = body
          .find(mutation)
          .unwrap_or_else(|| panic!("{handler} no longer contains {mutation}"));
        assert!(
          plan < at,
          "{handler} reaches {mutation:?} at {at} before planning at {plan}: a \
           refusal raised after that point has already changed the page it \
           claims it did not touch"
        );
      }

      // And the refusal really is a refusal: `?` on the plan, not a swallowed
      // error that lets the handler carry on and clear the field anyway.
      assert!(
        body.contains("Some(plan_typing(text, wpm, max_typing_seconds(caller.origin))?)"),
        "{handler} must propagate the refusal rather than absorb it"
      );
    }
  }

  #[test]
  fn a_bridge_caller_may_type_for_less_than_the_relay_will_wait() {
    // A call over the bridge is answered with a timeout if it runs too long, so
    // a plan that would type for longer is a guaranteed failure that still holds
    // a process-wide permit for its whole duration. Loopback keeps the five
    // minutes: nothing upstream times it out.
    assert_eq!(max_typing_seconds(McpOrigin::Loopback), 300.0);
    assert_eq!(max_typing_seconds(McpOrigin::Bridge), 80.0);
    assert!(
      max_typing_seconds(McpOrigin::Bridge) < 90.0,
      "the bridge budget must stay under the relay's 90 s call budget"
    );

    // Exercised, not merely asserted: the same text is refused over the
    // bridge and accepted over loopback. Each plan is built afresh with its
    // own randomness, so the fixture sits far from both bounds: 1,200
    // characters at 200 wpm plan to roughly 160 seconds, and the session rate
    // is sampled with a standard deviation of 10 wpm (five percent here; at
    // the 10 wpm floor it was a factor of three, which is why the fixture is
    // not typed at the floor). Fatigue makes the plan superlinear, so the
    // count is not to be scaled by eye.
    let text = "a".repeat(1_200);
    let over_loopback = plan_typing(&text, Some(200.0), max_typing_seconds(McpOrigin::Loopback))
      .expect("about 160 seconds is inside the loopback budget");
    let planned = over_loopback.last().map_or(0.0, |event| event.time);
    assert!(
      planned > 110.0 && planned < 250.0,
      "the fixture must sit well clear of both budgets, planned {planned}s"
    );

    let over_bridge = plan_typing(&text, Some(200.0), max_typing_seconds(McpOrigin::Bridge))
      .expect_err("the same text must be refused over the bridge");
    let body: serde_json::Value = serde_json::from_str(&over_bridge.message).unwrap();
    assert_eq!(body["code"], "TYPING_TOO_LONG");
    assert_eq!(
      body["params"]["limit"], "80",
      "the refusal must name the budget that applied, not the loopback one"
    );
  }

  #[test]
  fn a_stored_proxy_loses_its_secrets_and_nothing_else_under_redaction() {
    let mut proxy = serde_json::json!({
      "id": "p1",
      "name": "Berlin",
      "proxy_settings": {
        "proxy_type": "socks5",
        "host": "proxy.example",
        "port": 1080,
        "username": "user",
        "password": "hunter2",
        "vless_uri": "vless://uuid@host:443?security=reality#name"
      },
      "geo_country": "DE",
      "dynamic_proxy_url": "https://lists.example/rotate?key=SECRET"
    });
    redact_proxy_secrets(&mut proxy);

    assert_eq!(proxy["proxy_settings"]["password"], "[redacted]");
    assert_eq!(proxy["proxy_settings"]["vless_uri"], "[redacted]");
    assert_eq!(proxy["dynamic_proxy_url"], "[redacted]");
    // Everything an agent needs to choose a proxy survives.
    assert_eq!(proxy["id"], "p1");
    assert_eq!(proxy["name"], "Berlin");
    assert_eq!(proxy["proxy_settings"]["host"], "proxy.example");
    assert_eq!(proxy["proxy_settings"]["port"], 1080);
    assert_eq!(proxy["proxy_settings"]["username"], "user");
    assert_eq!(proxy["geo_country"], "DE");
    let text = proxy.to_string();
    assert!(!text.contains("hunter2") && !text.contains("SECRET") && !text.contains("vless://"));

    // A proxy with no secrets is left exactly as it was: no field is invented
    // just to say it was redacted.
    let mut bare = serde_json::json!({
      "id": "p2",
      "name": "Plain",
      "proxy_settings": {
        "proxy_type": "http",
        "host": "h",
        "port": 8080,
        "username": null,
        "password": null
      }
    });
    let before = bare.clone();
    redact_proxy_secrets(&mut bare);
    assert_eq!(bare, before);
  }

  #[test]
  fn the_proxy_readers_redact_for_the_bridge_and_only_for_the_bridge() {
    // `handle_list_proxies` and `handle_get_proxy` serialize a `StoredProxy`
    // whole. Both must route a BRIDGE caller through the redaction and leave
    // a loopback caller's answer untouched: the local agent is on the machine
    // that stores the password, and truncating its view would break the
    // existing export/import round trip over loopback.
    let production = include_str!("mcp_server.rs")
      .split_once("\n#[cfg(test)]")
      .map_or("", |(code, _)| code);
    for handler in ["handle_list_proxies", "handle_get_proxy"] {
      let rest = production
        .split(&format!("async fn {handler}("))
        .nth(1)
        .unwrap_or_else(|| panic!("{handler} must exist"));
      let body = &rest[..rest.find("\n  async fn ").unwrap_or(rest.len())];
      assert!(
        body.contains("caller: McpCaller<'_>"),
        "{handler} must know who is asking"
      );
      let gate = body
        .find("if caller.origin == McpOrigin::Bridge")
        .unwrap_or_else(|| panic!("{handler} must gate the redaction on the bridge origin"));
      let redact = body
        .find("redact_proxy_secrets(")
        .unwrap_or_else(|| panic!("{handler} must redact"));
      assert!(
        gate < redact,
        "{handler} must redact inside the bridge gate"
      );
      assert_eq!(
        body.matches("redact_proxy_secrets(").count(),
        1,
        "{handler} must redact in exactly one place, under the gate"
      );
    }
  }

  #[tokio::test]
  async fn exporting_proxies_is_refused_over_the_bridge_before_the_arguments_are_read() {
    // The export exists to write every password and VLESS URI out in full,
    // which is what the redaction above withholds from a remote caller. The
    // refusal sits at the gate, so a well-formed export and a malformed one
    // are indistinguishable from outside.
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();

    for arguments in [r#"{"format":"json"}"#, r#"{}"#] {
      let call = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"export_proxies","arguments":{arguments}}}}}"#
      );
      let McpOutcome::Body { body, .. } = server
        .handle_message(McpOrigin::Bridge, None, call.as_bytes())
        .await
      else {
        panic!("expected an answer");
      };
      assert!(
        body["error"]["message"]
          .as_str()
          .unwrap_or_default()
          .contains("TOOL_IS_LOCAL_ONLY"),
        "export_proxies must be refused over the bridge: {body}"
      );
    }

    // Over loopback it is not refused for that reason. With no format it
    // fails on its own terms, which proves the gate did not fire.
    let probe = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"export_proxies","arguments":{}}}"#;
    let McpOutcome::Body { body, .. } = server
      .handle_message(McpOrigin::Loopback, None, probe.as_bytes())
      .await
    else {
      panic!("expected an answer");
    };
    let local = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
      !local.contains("TOOL_IS_LOCAL_ONLY"),
      "a caller on this machine may still export: {body}"
    );
    assert!(local.contains("Missing format"), "{body}");

    // And the list is pinned: only the export is refused by name, and it is.
    assert_eq!(SECRET_EXPORT_TOOLS, &["export_proxies"]);
  }

  #[test]
  fn every_tool_that_reads_a_local_path_is_on_the_local_only_list() {
    // Derived from the source, and the list is PINNED. Iterating
    // LOCAL_PATH_TOOLS to check each entry is refused is a tautology -
    // deleting an entry just shortens the loop, so the membership itself is
    // asserted, and a handler that reads a path without being listed fails.
    let full = include_str!("mcp_server.rs");
    let source = full
      .split_once("\n#[cfg(test)]")
      .map(|(code, _)| code)
      .unwrap_or(full);

    let starts: Vec<usize> = source
      .match_indices("\n  ")
      .filter(|(i, _)| {
        let rest = &source[i + 3..];
        ["fn ", "async fn ", "pub fn ", "pub async fn "]
          .iter()
          .any(|p| rest.starts_with(p))
      })
      .map(|(i, _)| i)
      .collect();

    let mut readers = Vec::new();
    for (n, &begin) in starts.iter().enumerate() {
      let stop = starts.get(n + 1).copied().unwrap_or(source.len());
      let body = &source[begin..stop];
      if !body.contains("arguments") {
        continue;
      }
      if body.contains(r#""path""#) || body.contains(r#""folder""#) {
        let name = body
          .trim_start()
          .trim_start_matches("pub ")
          .trim_start_matches("async ")
          .trim_start_matches("fn ")
          .split(['(', '<'])
          .next()
          .unwrap_or("?")
          .trim_start_matches("handle_")
          .to_string();
        readers.push(name);
      }
    }

    // The scan above only sees a path named as a LITERAL argument key in the
    // handler. `import_browser_profiles` takes its path as `items[].source_path`
    // on a deserialized struct, so no such literal appears anywhere in its body
    // and it sat off the list, reachable from the internet, until a review
    // caught it by reading the struct instead. The declared schema is the
    // contract that does show nested arguments, so it is walked here too and
    // the two sources are unioned: a path reachable through EITHER the argument
    // keys or the published schema has to be refused over the bridge.
    fn path_like(name: &str) -> bool {
      let lowered = name.to_ascii_lowercase();
      lowered.contains("path")
        || lowered.contains("folder")
        || lowered.contains("directory")
        || lowered == "dir"
        || lowered.ends_with("_dir")
    }

    fn walk(schema: &serde_json::Value, found: &mut bool) {
      let serde_json::Value::Object(map) = schema else {
        return;
      };
      if let Some(serde_json::Value::Object(props)) = map.get("properties") {
        for (key, value) in props {
          if path_like(key) {
            *found = true;
          }
          walk(value, found);
        }
      }
      if let Some(items) = map.get("items") {
        walk(items, found);
      }
      for branch in ["anyOf", "oneOf", "allOf"] {
        if let Some(serde_json::Value::Array(options)) = map.get(branch) {
          for option in options {
            walk(option, found);
          }
        }
      }
    }

    for tool in McpServer::new().get_tools() {
      let mut found = false;
      walk(&tool.input_schema, &mut found);
      if found {
        readers.push(tool.name.clone());
      }
    }

    readers.sort();
    readers.dedup();

    let mut listed: Vec<String> = LOCAL_PATH_TOOLS.iter().map(|t| t.to_string()).collect();
    listed.sort();
    assert_eq!(
      readers, listed,
      "a handler reads a caller-supplied filesystem path but is not refused \
       over the bridge (or the list names one that no longer reads a path)"
    );
  }

  #[test]
  fn the_bridge_declares_itself_as_the_bridge() {
    // The whole property rests on the transport telling the truth about where
    // a message came from. Nothing else in the tree asserts this wiring, so a
    // one-word edit in mcp_remote.rs would silently reopen every local-path
    // tool to the internet.
    let remote = include_str!("mcp_remote.rs");
    assert!(
      remote.contains("McpOrigin::Bridge"),
      "the bridge must declare its own origin when handing a message to the engine"
    );
    assert!(
      !remote.contains("McpOrigin::Loopback"),
      "the bridge must never claim to be a caller standing on this machine"
    );
  }

  #[tokio::test]
  async fn a_tool_that_reads_this_machine_is_refused_over_the_bridge() {
    // `add_extension` takes a caller-supplied path, `fs::read`s it, stores the
    // bytes, and the sync engine uploads them, an arbitrary local-file read
    // with the same shape as the `file://` hole the URL allowlist closed, but
    // reachable from the internet with an account credential.
    //
    // A remote caller cannot know this machine's filesystem, so there is no
    // legitimate remote use to preserve: refusing is strictly better than
    // guessing at safe roots.
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();

    for tool in LOCAL_PATH_TOOLS {
      let call = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","arguments":{{"path":"/etc/passwd","folder":"/etc"}}}}}}"#
      );

      let McpOutcome::Body { body, .. } = server
        .handle_message(McpOrigin::Bridge, None, call.as_bytes())
        .await
      else {
        panic!("expected an answer for {tool}");
      };
      let message = body["error"]["message"].as_str().unwrap_or_default();
      assert!(
        message.contains("TOOL_IS_LOCAL_ONLY"),
        "{tool} must be refused over the bridge with a translatable code: {body}"
      );

      // And the SAME call over loopback is not refused for that reason: the
      // caller is already on the machine. It may fail for its own reasons -
      // the path does not exist, but never with this code.
      let McpOutcome::Body { body, .. } = server
        .handle_message(McpOrigin::Loopback, None, call.as_bytes())
        .await
      else {
        panic!("expected an answer for {tool}");
      };
      let local = body["error"]["message"].as_str().unwrap_or_default();
      assert!(
        !local.contains("TOOL_IS_LOCAL_ONLY"),
        "{tool} must still be available to a caller on this machine: {body}"
      );
    }
  }

  #[tokio::test]
  async fn a_nested_path_is_refused_over_the_bridge_in_its_real_shape() {
    // The loop above sends `{"path": ..., "folder": ...}` to every listed
    // tool, which is not the shape `import_browser_profiles` actually takes -
    // its path rides inside `items[].source_path`. Sending the real payload is
    // the difference between proving the NAME is on a list and proving the
    // CALL an attacker would make is refused.
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();

    let call = serde_json::json!({
      "jsonrpc": "2.0",
      "id": 1,
      "method": "tools/call",
      "params": {
        "name": "import_browser_profiles",
        "arguments": {
          "items": [{
            "source_path": "/Users/someone/Library/Application Support/Google/Chrome/Default",
            "new_profile_name": "stolen"
          }]
        }
      }
    })
    .to_string();

    let McpOutcome::Body { body, .. } = server
      .handle_message(McpOrigin::Bridge, None, call.as_bytes())
      .await
    else {
      panic!("expected an answer");
    };
    assert!(
      body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("TOOL_IS_LOCAL_ONLY"),
      "a remote caller must not be able to name a directory on this disk: {body}"
    );

    // The refusal must happen BEFORE the arguments are even parsed, so a
    // malformed remote payload cannot be told apart from a well-formed one.
    // Otherwise the error text itself answers "does this path exist".
    let probe = serde_json::json!({
      "jsonrpc": "2.0",
      "id": 2,
      "method": "tools/call",
      "params": { "name": "import_browser_profiles", "arguments": {} }
    })
    .to_string();
    let McpOutcome::Body { body, .. } = server
      .handle_message(McpOrigin::Bridge, None, probe.as_bytes())
      .await
    else {
      panic!("expected an answer");
    };
    assert!(
      body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("TOOL_IS_LOCAL_ONLY"),
      "the gate must sit ahead of argument parsing: {body}"
    );
  }

  #[test]
  fn test_mcp_server_initial_state() {
    let server = McpServer::new();
    assert!(!server.is_running());
  }

  #[tokio::test]
  async fn the_session_map_is_bounded() {
    // Nothing evicts a session except an explicit `end_session`, and no
    // first-party client sends one, so without a cap the map grows for the
    // life of the process. The oldest is evicted rather than the newest
    // refused: the caller asking for a session is the one actually present.
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();

    let init = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let mut first = None;
    for i in 0..(MAX_SESSIONS + 8) {
      let McpOutcome::Body { new_session_id, .. } =
        server.handle_message(McpOrigin::Loopback, None, init).await
      else {
        panic!("initialize must mint a session");
      };
      if i == 0 {
        first = new_session_id;
      }
    }

    assert!(
      server.inner.lock().await.sessions.len() <= MAX_SESSIONS,
      "the session map must stay bounded"
    );

    // The oldest went first, and the map still works for a fresh session.
    let ping = br#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#;
    assert!(matches!(
      server
        .handle_message(
          McpOrigin::Loopback,
          Some(&first.expect("first session")),
          ping
        )
        .await,
      McpOutcome::UnknownSession
    ));
  }

  #[tokio::test]
  async fn the_cap_evicts_what_is_idle_not_what_is_busy() {
    // Evicting by CREATION time threw out the wrong session every time: a
    // long-lived agent's is by definition the oldest, so a wall of abandoned
    // sessions from closed browser tabs would evict the one client actually
    // working, and the official MCP SDK does not re-initialize on the 404
    // that follows, so that agent stays wedged.
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();

    let init = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let ping = br#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#;

    let McpOutcome::Body { new_session_id, .. } =
      server.handle_message(McpOrigin::Loopback, None, init).await
    else {
      panic!("initialize must mint a session");
    };
    let agent = new_session_id.expect("session id");

    // Fill the map, keeping the agent's session in active use throughout.
    let mut newest = None;
    for i in 0..(MAX_SESSIONS + 16) {
      if let McpOutcome::Body { new_session_id, .. } =
        server.handle_message(McpOrigin::Loopback, None, init).await
      {
        newest = new_session_id;
      }
      if i % 4 == 0 {
        assert!(
          matches!(
            server
              .handle_message(McpOrigin::Loopback, Some(&agent), ping)
              .await,
            McpOutcome::Body { .. }
          ),
          "the busy session must survive: it is the one being used"
        );
      }
    }

    assert!(
      server.inner.lock().await.sessions.len() <= MAX_SESSIONS,
      "still bounded"
    );
    assert!(
      matches!(
        server
          .handle_message(McpOrigin::Loopback, Some(&agent), ping)
          .await,
        McpOutcome::Body { .. }
      ),
      "the session in continuous use must outlive the idle ones"
    );

    // And the newest survives too. Without this the test passes for an
    // eviction policy that throws out whatever just arrived, which keeps the
    // map bounded and the busy session alive while making every new client
    // unable to hold a session at all.
    assert!(
      matches!(
        server
          .handle_message(
            McpOrigin::Loopback,
            Some(&newest.expect("a newest session")),
            ping
          )
          .await,
        McpOutcome::Body { .. }
      ),
      "a freshly minted session must not be the one evicted"
    );
  }

  #[tokio::test]
  async fn closing_the_local_listener_keeps_sessions_the_bridge_is_using() {
    // One engine serves two transports. `stop()` is a statement about the
    // loopback listener only, the cloud bridge may be mid-conversation, but
    // it used to clear the shared session map, so turning the local switch off
    // answered 404 MCP_SESSION_NOT_FOUND to a remote caller that had nothing to
    // do with the local one. The website re-initializes on a 404 and self-heals;
    // the official MCP TypeScript SDK throws on any non-ok POST, so a
    // third-party agent took a hard mid-run error from an unrelated toggle.
    let server = McpServer::new();
    server.mark_engine_ready_for_tests();

    let init = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let McpOutcome::Body { new_session_id, .. } =
      server.handle_message(McpOrigin::Loopback, None, init).await
    else {
      panic!("initialize must mint a session");
    };
    let session = new_session_id.expect("initialize must return a session id");

    server.mark_running_for_tests();
    server
      .stop()
      .await
      .expect("stop should succeed once running");

    // The session must still be usable over the bridge. Destructured rather
    // than matched on the variant alone: `Body` is ALSO what an engine-not-
    // ready error comes back as, so a bare `matches!` would still pass if
    // stop() had torn down the engine, pinning only half of what it promises.
    let ping = br#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#;
    let McpOutcome::Body { body, .. } = server
      .handle_message(McpOrigin::Loopback, Some(&session), ping)
      .await
    else {
      panic!("stopping the loopback listener must not invalidate a bridge session");
    };
    assert!(
      body.get("error").is_none(),
      "stop() must leave the engine able to answer, not just the session id valid: {body}"
    );
    assert!(
      body.get("result").is_some(),
      "expected a real answer: {body}"
    );

    // And an id that was never minted is still rejected, so the check above is
    // not passing merely because session validation stopped happening.
    assert!(matches!(
      server
        .handle_message(
          McpOrigin::Loopback,
          Some("00000000-0000-4000-8000-000000000000"),
          ping
        )
        .await,
      McpOutcome::UnknownSession
    ));
  }

  #[tokio::test]
  async fn the_launch_settings_an_agent_can_set_are_the_ones_it_can_read() {
    let server = McpServer::new();
    let tools = server.handle_tools_list().await.expect("tools list");
    let update = tools["tools"]
      .as_array()
      .expect("tools array")
      .iter()
      .find(|tool| tool["name"] == "update_profile_fingerprint")
      .expect("update_profile_fingerprint is advertised");
    let properties = &update["inputSchema"]["properties"];
    for field in ["restore_session", "webrtc_mode"] {
      assert!(
        properties.get(field).is_some(),
        "{field} must be settable through the tool that owns the Wayfern config"
      );
    }
    assert_eq!(
      properties["webrtc_mode"]["enum"],
      serde_json::json!(["auto", "tcp_only", "block"]),
      "the modes offered must be the modes the launcher understands"
    );
    // A mode the launcher would silently read as `auto` is refused instead.
    assert!(crate::wayfern_manager::WebRtcMode::parse("sideways").is_none());
  }

  #[test]
  fn proxy_tool_schema_exposes_vless_reality_without_requiring_regular_endpoint_fields() {
    let server = McpServer::new();
    let tools = server.get_tools();
    let create = tools
      .iter()
      .find(|tool| tool.name == "create_proxy")
      .expect("create_proxy tool");
    let properties = &create.input_schema["properties"];
    assert!(properties["proxy_type"]["enum"]
      .as_array()
      .is_some_and(|values| values.iter().any(|value| value == "vless")));
    assert!(properties["vless_uri"].is_object());

    let required = create.input_schema["required"]
      .as_array()
      .expect("required fields");
    assert!(required.iter().any(|field| field == "name"));
    assert!(required.iter().any(|field| field == "proxy_type"));
    assert!(!required.iter().any(|field| field == "host"));
    assert!(!required.iter().any(|field| field == "port"));
  }

  #[test]
  fn rate_limit_only_classifies_browser_automation_tools() {
    let request = |method: &str, name: Option<&str>| McpRequest {
      jsonrpc: "2.0".to_string(),
      id: Some(serde_json::json!(1)),
      method: method.to_string(),
      params: name.map(|name| serde_json::json!({ "name": name, "arguments": {} })),
    };

    for name in [
      "run_profile",
      "kill_profile",
      "batch_run_profiles",
      "batch_stop_profiles",
      "start_sync_session",
      "navigate",
      "screenshot",
      "evaluate_javascript",
      "click_element",
      "type_text",
      "get_page_content",
      "get_page_info",
      "get_interactive_elements",
      "click_by_index",
      "type_by_index",
      // The agent surface reads and drives the same browser.
      "perceive_page",
      "resolve_locator",
      "click_locator",
      "type_locator",
      "extract_structured",
      "pick_element",
      // Leases a remote host for up to two hours and spends the pooled
      // remote-hour budget.
      "run_cookie_bot_now",
      "run_profile_remote",
      // Reaches the fleet, like the remote-session stop it mirrors.
      "cancel_cookie_bot_run",
      "stop_remote_session",
    ] {
      assert!(
        McpServer::is_automation_tool_call(&request("tools/call", Some(name))),
        "automation tool was not limited: {name}"
      );
    }

    for name in [
      "list_profiles",
      // Configuration, not automation: nothing is leased. Metering it would
      // throttle an agent enrolling many profiles, and it is not what spends
      // the account's hours.
      "set_cookie_bot_schedule",
      "delete_cookie_bot_schedule",
      "list_cookie_bot_schedules",
      "get_cookie_bot_schedule",
      "check_cookie_bot_conflicts",
      "list_cookie_bot_runs",
      "list_cookie_bot_presets",
      "get_cookie_bot_usage",
      "get_remote_hours_quota",
      "list_remote_sessions",
      "get_remote_session",
    ] {
      assert!(
        !McpServer::is_automation_tool_call(&request("tools/call", Some(name))),
        "free or non-leasing tool was limited: {name}"
      );
    }

    assert!(!McpServer::is_automation_tool_call(&request(
      "tools/list",
      None
    )));
  }

  // --- The agent surface --------------------------------------------------
  //
  // Driven against the fake Wayfern socket in `wayfern_cdp::test_support`,
  // because the property that matters, WHICH engine a profile gets and what
  // that engine puts on the wire, only shows up when something answers.

  fn agent_context_for(version: &str, target: CdpTarget) -> AgentContext {
    AgentContext::new(
      BrowserProfile {
        id: uuid::Uuid::nil(),
        name: "p".to_string(),
        browser: "wayfern".to_string(),
        version: version.to_string(),
        ..Default::default()
      },
      target,
    )
  }

  #[tokio::test]
  async fn a_152_profile_clicks_through_vellum_and_an_older_one_through_dispatch() {
    use crate::wayfern_cdp::test_support::{fake_browser, methods, Fake};

    let request: AgentClickRequest = serde_json::from_value(serde_json::json!({
      "locator": { "role": "button", "name": "Save" }
    }))
    .unwrap();

    // The 152 engine: the native resolver names the node, the DOM says where
    // it is on screen, and a real pointer strikes it. No script runs in the
    // page and nothing is dispatched synthetically.
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let ctx = agent_context_for("152.0.7977.64", target);
    assert_eq!(ctx.engine, Engine::Wayfern);
    let clicked = agent_click_locator(&ctx, &request)
      .await
      .expect("a cooperative browser completes the click");
    assert!(clicked.clicked);
    assert_eq!(clicked.engine, Engine::Wayfern);
    assert_eq!(clicked.matched.signature, "s7");
    let sent = methods(&frames);
    let vellum: Vec<&str> = sent
      .iter()
      .filter(|m| m.starts_with("Vellum."))
      .map(String::as_str)
      .collect();
    assert_eq!(
      vellum,
      vec![
        "Vellum.acquire",
        "Vellum.glide",
        "Vellum.strike",
        "Vellum.release"
      ]
    );
    assert!(sent.iter().any(|m| m == "Wayfern.resolveLocator"));
    assert!(sent.iter().any(|m| m == "DOM.getContentQuads"));
    assert!(
      !sent
        .iter()
        .any(|m| m == "Input.dispatchMouseEvent" || m == "Runtime.evaluate"),
      "the native path must inject nothing and dispatch nothing: {sent:?}"
    );
    // The page was told to expect a navigation, and told to stop afterwards.
    assert_eq!(sent.first().map(String::as_str), Some("Page.enable"));
    assert_eq!(sent.last().map(String::as_str), Some("Page.disable"));
    let wire = serde_json::to_value(&clicked).unwrap();
    assert_eq!(wire["match"]["backendNodeId"], 7);
    assert_eq!(wire["engine"], "wayfern");
    assert_eq!(wire["navigated"], false);

    // The fallback engine: the locator is applied by a script and the click
    // is a trusted mouse event at the element's centre, exactly what the
    // selector tools have always done.
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let ctx = agent_context_for("151.0.7922.76", target);
    assert_eq!(ctx.engine, Engine::Fallback);
    let clicked = agent_click_locator(&ctx, &request).await.unwrap();
    assert_eq!(clicked.engine, Engine::Fallback);
    assert_eq!(clicked.matched.signature, "s7");
    let sent = methods(&frames);
    assert!(sent.iter().any(|m| m == "Runtime.evaluate"));
    let mouse: Vec<serde_json::Value> = frames
      .lock()
      .unwrap()
      .iter()
      .filter(|f| f["method"] == "Input.dispatchMouseEvent")
      .cloned()
      .collect();
    assert_eq!(mouse.len(), 3, "move, press, release: {sent:?}");
    assert_eq!(mouse[0]["params"]["type"], "mouseMoved");
    assert_eq!(mouse[1]["params"]["type"], "mousePressed");
    assert_eq!(mouse[2]["params"]["type"], "mouseReleased");
    assert_eq!(mouse[1]["params"]["x"], 140.0);
    assert_eq!(mouse[1]["params"]["button"], "left");
    assert!(
      !sent
        .iter()
        .any(|m| m.starts_with("Vellum.") || m.starts_with("Wayfern.")),
      "an older browser must never be sent the domains it lacks: {sent:?}"
    );
  }

  #[tokio::test]
  async fn a_152_profile_types_through_inscribe_and_an_older_one_through_key_events() {
    use crate::wayfern_cdp::test_support::{fake_browser, methods, Fake};

    let request: AgentTypeRequest = serde_json::from_value(serde_json::json!({
      "locator": { "role": "textbox", "name": "Email" },
      "text": "hi",
      "clear_first": true
    }))
    .unwrap();

    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let ctx = agent_context_for("152.0.7977.64", target);
    let typed = agent_type_locator(&ctx, &request, 300.0).await.unwrap();
    assert!(typed.typed);
    assert_eq!(typed.engine, Engine::Wayfern);
    assert_eq!(typed.characters, 2);
    assert_eq!(typed.corrections, Some(1));
    let sent = methods(&frames);
    let native: Vec<&str> = sent
      .iter()
      .filter(|m| {
        m.starts_with("Vellum.") || m.starts_with("DOM.resolveNode") || m.starts_with("Runtime.")
      })
      .map(String::as_str)
      .collect();
    // Focus by a real strike, THEN the field is emptied on the resolved node,
    // then the keys: clearing before the click would leave the caret wherever
    // the click landed in a field that is no longer empty.
    assert_eq!(
      native,
      vec![
        "Vellum.acquire",
        "Vellum.glide",
        "Vellum.strike",
        "DOM.resolveNode",
        "Runtime.callFunctionOn",
        "Vellum.inscribe",
        "Vellum.release"
      ]
    );
    {
      let sent_frames = frames.lock().unwrap();
      let inscribe = sent_frames
        .iter()
        .find(|f| f["method"] == "Vellum.inscribe")
        .unwrap();
      assert_eq!(inscribe["params"]["text"], "hi");
      assert_eq!(inscribe["params"]["typos"], true);
      let clear = sent_frames
        .iter()
        .find(|f| f["method"] == "Runtime.callFunctionOn")
        .unwrap();
      assert_eq!(clear["params"]["arguments"][0]["value"], true);
      assert_eq!(clear["params"]["objectId"], "obj-7");
    }

    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let ctx = agent_context_for("151.0.7922.76", target);
    let typed = agent_type_locator(&ctx, &request, 300.0).await.unwrap();
    assert_eq!(typed.engine, Engine::Fallback);
    assert_eq!(typed.characters, 2);
    assert_eq!(typed.corrections, None);
    let sent = methods(&frames);
    assert_eq!(sent.first().map(String::as_str), Some("Runtime.evaluate"));
    assert!(
      sent
        .iter()
        .filter(|m| *m == "Input.dispatchKeyEvent")
        .count()
        >= 4,
      "two characters are at least two key downs and two key ups: {sent:?}"
    );
    assert!(!sent.iter().any(|m| m.starts_with("Vellum.")));
  }

  #[tokio::test]
  async fn the_152_only_tools_refuse_an_older_profile_before_touching_the_browser() {
    use crate::wayfern_cdp::test_support::{fake_browser, methods, Fake};

    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let ctx = agent_context_for("151.0.7922.76", target);

    let extraction: ExtractionRequest = serde_json::from_value(serde_json::json!({
      "container": { "role": "listitem" },
      "field_map": [{ "key": "title", "locator": { "role": "link" }, "source": "text" }]
    }))
    .unwrap();
    let refused = agent_extract(&ctx, &extraction)
      .await
      .expect_err("extraction has no fallback");
    assert!(
      matches!(&refused, AgentError::RequiresWayfern152 { version } if version == "151.0.7922.76")
    );
    let error = refused.into_mcp();
    assert_eq!(error.code, -32000);
    assert!(error.message.contains("Wayfern 152"));
    assert_eq!(error.data.as_ref().unwrap()["code"], "WAYFERN_152_REQUIRED");

    let refused = agent_pick_element(&ctx, 5_000)
      .await
      .expect_err("the picker has no fallback");
    assert!(matches!(refused, AgentError::RequiresWayfern152 { .. }));

    assert!(
      methods(&frames).is_empty(),
      "a refusal for the browser's version must not open a socket to it"
    );

    // And a request that cannot mean anything is refused on either engine,
    // before the version is even consulted.
    let malformed: ExtractionRequest = serde_json::from_value(serde_json::json!({
      "container": { "role": "listitem" },
      "field_map": [{ "key": "price", "locator": { "role": "cell" }, "source": "attribute" }]
    }))
    .unwrap();
    let ctx = agent_context_for(
      "152.0.7977.64",
      CdpTarget::Local {
        ws_url: "ws://127.0.0.1:1/never".to_string(),
      },
    );
    let refused = agent_extract(&ctx, &malformed)
      .await
      .expect_err("no attribute named");
    assert!(
      matches!(refused, AgentError::InvalidArgument(_)),
      "{refused:?}"
    );
    assert_eq!(refused.into_mcp().code, -32602);
  }

  #[tokio::test]
  async fn an_ambiguous_locator_carries_its_candidates_in_the_error_data() {
    use crate::wayfern_cdp::test_support::{fake_browser, Fake};

    let request: AgentResolveRequest = serde_json::from_value(serde_json::json!({
      "locator": { "role": "button", "name": "Save" },
      "candidate_limit": 5
    }))
    .unwrap();

    for version in ["152.0.7977.64", "151.0.7922.76"] {
      let (target, _) = fake_browser(Fake::AmbiguousLocator).await;
      let ctx = agent_context_for(version, target);
      let refused = agent_resolve_locator(&ctx, &request)
        .await
        .expect_err("two matches must be refused on both engines");
      let error = refused.into_mcp();
      assert_eq!(error.code, -32000, "{version}");
      assert!(
        error
          .message
          .starts_with("Ambiguous locator: 2 nodes match"),
        "{version}: {}",
        error.message
      );
      let data = error.data.expect("the candidates travel in data");
      assert_eq!(data["code"], "LOCATOR_AMBIGUOUS");
      assert_eq!(data["matchCount"], 2);
      assert_eq!(
        data["candidates"].as_array().map(Vec::len),
        Some(2),
        "{version}"
      );
      assert_eq!(data["candidates"][0]["backendNodeId"], 7);
    }

    // A clean resolution on both engines carries the engine that answered.
    for (version, engine) in [("152.0.7977.64", "wayfern"), ("151.0.7922.76", "fallback")] {
      let (target, _) = fake_browser(Fake::Cooperative).await;
      let ctx = agent_context_for(version, target);
      let resolved = agent_resolve_locator(&ctx, &request).await.unwrap();
      let wire = serde_json::to_value(&resolved).unwrap();
      assert_eq!(wire["engine"], engine);
      assert_eq!(wire["matchCount"], 1);
      assert_eq!(wire["match"]["signature"], "s7");
      assert_eq!(wire["locator"]["name"], "Save");
    }

    // An empty locator matches everything, which is never what was meant.
    let empty: AgentResolveRequest =
      serde_json::from_value(serde_json::json!({ "locator": {} })).unwrap();
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let ctx = agent_context_for("152.0.7977.64", target);
    let refused = agent_resolve_locator(&ctx, &empty)
      .await
      .expect_err("empty");
    assert!(matches!(refused, AgentError::InvalidArgument(_)));
    assert!(crate::wayfern_cdp::test_support::methods(&frames).is_empty());
  }

  #[tokio::test]
  async fn perception_keeps_the_browsers_shape_on_both_engines() {
    use crate::wayfern_cdp::test_support::{fake_browser, methods, Fake};

    let request = PerceptionRequest::default();
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let ctx = agent_context_for("152.0.7977.64", target);
    let page = agent_perceive(&ctx, &request).await.unwrap();
    assert_eq!(page.engine, Engine::Wayfern);
    assert_eq!(page.nodes[0].name.as_deref(), Some("Save"));
    assert_eq!(methods(&frames), vec!["Wayfern.capturePagePerception"]);

    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let ctx = agent_context_for("151.0.7922.76", target);
    let page = agent_perceive(&ctx, &request).await.unwrap();
    assert_eq!(page.engine, Engine::Fallback);
    assert!(page.snapshot_id.starts_with("fallback-"));
    assert_eq!(page.nodes[0].name.as_deref(), Some("Save"));
    assert_eq!(page.frames[0].frame_id, "f0");
    assert!(page.cursor.is_none());
    assert_eq!(methods(&frames), vec!["Runtime.evaluate"]);
    let wire = serde_json::to_value(&page).unwrap();
    assert_eq!(wire["engine"], "fallback");
    assert_eq!(wire["nodes"][0]["inViewport"], true);
    assert_eq!(wire["stats"]["returnedNodes"], 1);

    // A cursor is a 152 feature: the fallback cannot continue anything.
    let continuation: PerceptionRequest =
      serde_json::from_value(serde_json::json!({ "cursor": "snap-1.2" })).unwrap();
    let refused = agent_perceive(&ctx, &continuation)
      .await
      .expect_err("no cursors");
    assert!(matches!(refused, AgentError::InvalidArgument(_)));
  }

  #[test]
  fn the_picker_waits_less_over_the_bridge_than_the_relay_does() {
    assert_eq!(max_pick_timeout_ms(McpOrigin::Loopback), 300_000);
    assert_eq!(max_pick_timeout_ms(McpOrigin::Bridge), 80_000);
    assert!(
      max_pick_timeout_ms(McpOrigin::Bridge) < 90_000,
      "a wait that outlives the relay's 90 s call budget is a guaranteed failure"
    );
    assert!(DEFAULT_PICK_TIMEOUT_MS <= max_pick_timeout_ms(McpOrigin::Bridge));
    // Extraction pages through rows for up to two minutes on loopback and
    // is held to the same bridge margin as typing and the picker.
    assert_eq!(max_extraction_budget_ms(McpOrigin::Loopback), 120_000);
    assert!(max_extraction_budget_ms(McpOrigin::Bridge) < 90_000);
  }

  #[test]
  fn the_vellum_typing_budget_refuses_before_the_page_is_touched() {
    // The browser paces the keys itself, so the refusal has to be estimated
    // from the length; it must be conservative, and it must be the same
    // envelope the planner answers with.
    let budget = vellum_typing_budget("hello there", 300.0).expect("an ordinary field");
    assert_eq!(budget, Duration::from_secs(300));

    let refusal = vellum_typing_budget(&"a".repeat(2_000), 300.0).expect_err("too long");
    assert_eq!(refusal.code, -32602);
    let body: serde_json::Value = serde_json::from_str(&refusal.message).unwrap();
    assert_eq!(body["code"], "TYPING_TOO_LONG");
    assert_eq!(body["params"]["limit"], "300");

    // Over the bridge the same text is refused sooner.
    assert!(vellum_typing_budget(&"a".repeat(400), 300.0).is_ok());
    assert!(vellum_typing_budget(&"a".repeat(400), 80.0).is_err());

    // And the length bound bites first, whatever the budget.
    let huge = "a".repeat(MAX_TYPING_CHARS + 1);
    assert!(vellum_typing_budget(&huge, f64::MAX).is_err());

    // The refusal is the structured error both doors understand.
    let agent: AgentError = refusal.into();
    assert!(matches!(agent, AgentError::TypingTooLong { limit, .. } if limit == 300.0));
  }

  #[test]
  fn the_typing_tools_plan_only_for_the_engine_that_needs_a_plan() {
    // The selector and index typing tools keep their whole pre-152 body, and
    // the source-scanning test above pins its order. What is pinned here is
    // that the 152 branch is decided from the PROFILE'S version, read at the
    // point of use, and that the Vellum budget is decided in the same place as
    // the plan, before the focus script.
    let production = include_str!("mcp_server.rs")
      .split_once("\n#[cfg(test)]")
      .map_or("", |(code, _)| code);
    for handler in ["handle_type_text", "handle_type_by_index"] {
      let rest = production
        .split(&format!("async fn {handler}("))
        .nth(1)
        .unwrap_or_else(|| panic!("{handler} must exist"));
      let body = &rest[..rest.find("\n  async fn ").unwrap_or(rest.len())];
      let engine = body
        .find("Engine::for_version(&profile.version)")
        .unwrap_or_else(|| panic!("{handler} must read the engine off the profile"));
      let budget = body
        .find("vellum_typing_budget(")
        .unwrap_or_else(|| panic!("{handler} must budget Vellum typing"));
      assert!(
        body.matches("max_typing_seconds(caller.origin)").count() >= 2,
        "{handler} must budget both engines on the caller's transport"
      );
      let focus = body.find("let focus_js").unwrap();
      assert!(
        engine < budget && budget < focus,
        "{handler}: engine, budget, then the page"
      );
      assert!(
        body.contains("vellum_type("),
        "{handler} must type through Vellum on a 152 profile"
      );
      assert!(
        body.contains("Input.insertText"),
        "{handler} must keep the instant path for every engine"
      );
    }
    for handler in ["handle_click_element", "handle_click_by_index"] {
      let rest = production
        .split(&format!("async fn {handler}("))
        .nth(1)
        .unwrap_or_else(|| panic!("{handler} must exist"));
      let body = &rest[..rest.find("\n  async fn ").unwrap_or(rest.len())];
      assert!(body.contains("ctx.engine.is_wayfern()"), "{handler}");
      assert!(body.contains("vellum_click("), "{handler}");
      assert!(
        body.contains("el.click()"),
        "{handler} must keep the pre-152 click"
      );
    }
  }

  #[tokio::test]
  async fn every_agent_tool_is_advertised_gated_and_dispatchable() {
    let server = McpServer::new();
    let advertised: Vec<String> = server.get_tools().into_iter().map(|t| t.name).collect();
    let production = include_str!("mcp_server.rs")
      .split_once("\n#[cfg(test)]")
      .map_or("", |(code, _)| code);
    let dispatch = production
      .split("async fn dispatch_tool_call(")
      .nth(1)
      .expect("dispatch must exist");
    let dispatch = &dispatch[..dispatch.find("\n  async fn ").unwrap_or(dispatch.len())];

    for tool in [
      "perceive_page",
      "resolve_locator",
      "click_locator",
      "type_locator",
      "extract_structured",
      "pick_element",
    ] {
      assert!(
        advertised.iter().any(|t| t == tool),
        "{tool} is not advertised"
      );
      let arm = dispatch
        .split(&format!("\"{tool}\" => {{"))
        .nth(1)
        .unwrap_or_else(|| panic!("{tool} is not dispatched"));
      let arm = &arm[..arm.find("\n      }").unwrap_or(arm.len())];
      assert!(
        arm.contains("can_use_browser_automation"),
        "{tool} must sit behind the browser-automation gate"
      );
      // Every one of them takes a profile id and nothing from this disk, so
      // the bridge is allowed to call them.
      assert!(!LOCAL_PATH_TOOLS.contains(&tool));
      assert!(!SECRET_EXPORT_TOOLS.contains(&tool));
    }

    // And over the bridge they are answered, not refused as local-only. They
    // fail here for want of a browser, never with the local-only code.
    server.mark_engine_ready_for_tests();
    let call = serde_json::json!({
      "jsonrpc": "2.0",
      "id": 1,
      "method": "tools/call",
      "params": { "name": "perceive_page", "arguments": { "profile_id": "00000000-0000-0000-0000-000000000000" } }
    })
    .to_string();
    let McpOutcome::Body { body, .. } = server
      .handle_message(McpOrigin::Bridge, None, call.as_bytes())
      .await
    else {
      panic!("expected an answer");
    };
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
      !message.contains("TOOL_IS_LOCAL_ONLY"),
      "the agent surface must be reachable over the bridge: {body}"
    );
  }

  #[test]
  fn aria_role_names_are_translated_to_the_browsers_tokens() {
    // Wayfern indexes Blink's own role tokens, so `textbox` matches nothing
    // there while `textField` does. An agent that has read ARIA writes the
    // former; the translation is the client's job, and only for the tokens
    // that differ in substance, since the browser already ignores case and
    // separators.
    let role = |name: &str| {
      canonical_locator(&LocatorDescription {
        role: Some(name.to_string()),
        ..Default::default()
      })
      .role
      .unwrap()
    };
    assert_eq!(role("textbox"), "textField");
    assert_eq!(role("TextBox"), "textField");
    assert_eq!(role("text-box"), "textField");
    assert_eq!(role("radio"), "radioButton");
    assert_eq!(role("img"), "image");
    assert_eq!(role("text"), "staticText");
    // What the browser already understands passes through untouched.
    assert_eq!(role("textField"), "textField");
    assert_eq!(role("text_field"), "text_field");
    assert_eq!(role("button"), "button");
    assert_eq!(role("listitem"), "listitem");
    // And nothing else about the locator moves.
    let full = LocatorDescription {
      role: Some("textbox".into()),
      name: Some("Email".into()),
      attributes: Some(vec![crate::wayfern_cdp::LocatorAttribute {
        name: "id".into(),
        value: "email".into(),
      }]),
      ..Default::default()
    };
    let canonical = canonical_locator(&full);
    assert_eq!(canonical.name.as_deref(), Some("Email"));
    assert_eq!(canonical.attributes, full.attributes);
    assert!(canonical_locator(&LocatorDescription::default()).is_empty());
  }
}

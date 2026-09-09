//! Typed wrappers over the Wayfern 152 automation surface.
//!
//! Wayfern 152 answers four things an older build cannot: a native page
//! perception snapshot (`Wayfern.capturePagePerception`), a locator resolver
//! that refuses to guess (`Wayfern.resolveLocator`), structured extraction
//! (`Wayfern.extractStructured`) and an element picker driven by the user's own
//! click. It also carries the `Vellum` domain: a virtual pointer that glides,
//! strikes and types through the real input path, with timing that is stable
//! per profile.
//!
//! Everything here speaks to a PAGE session. A locally launched browser is
//! reached on its page socket, and a relayed one is attached flat to a page by
//! [`crate::cdp_target`], so on both arms `Vellum.acquire` resolves its surface
//! from the session's own frame and `surface` is never sent. The browser-target
//! form, which needs `surface`, is deliberately not used: it would mean a second
//! connection model for one feature.
//!
//! Nothing in this module decides WHICH engine a profile gets. That is
//! [`Engine::for_version`], read from the profile at the point of use, and the
//! pre-152 fallbacks live with the tools that own them.

use crate::cdp_target::{CdpConnection, CdpError, CdpTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use utoipa::ToSchema;

/// How long an ordinary command may wait for its reply.
///
/// The perception, extraction and typing commands carry their own budgets and
/// wait longer; this covers the rest, and matches the connection layer's own
/// ceiling so a hung browser is reported rather than waited on.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// Headroom added to a budget the browser itself enforces, so the reply for a
/// capture that ran to its full budget still arrives before this side gives up.
const BUDGET_HEADROOM: Duration = Duration::from_secs(10);

/// Ceiling on cursor pages followed in one perception call.
///
/// A browser that always answers "truncated, here is a cursor" must not hold a
/// tool call forever. Sixty-four pages at the smallest page size the browser
/// allows is still far more than any real document.
const MAX_PERCEPTION_PAGES: usize = 64;

/// Default total byte cap for one perception call, across cursor pages.
pub const DEFAULT_PERCEPTION_BYTE_CAP: u64 = 1024 * 1024;

/// The largest total a caller may ask a perception call for.
///
/// Sits under the largest result frame the bridge will carry, with room for the
/// pretty-printed JSON envelope the tool result travels in, so a result that
/// fits here is a result that can actually be delivered.
pub const MAX_PERCEPTION_BYTE_CAP: u64 = 4 * 1024 * 1024;

/// The smallest byte cap the browser accepts for one page.
const MIN_PERCEPTION_BYTE_CAP: u64 = 1024;

/// Which implementation answered a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
  /// The browser's native domains: no script in the page, real input path.
  Wayfern,
  /// The script-and-`Input.dispatch*` path an older Wayfern is driven with.
  Fallback,
}

impl Engine {
  /// The engine a profile on `version` gets, read at the point of use.
  pub fn for_version(version: &str) -> Self {
    if crate::wayfern_manager::supports_wayfern_152(version) {
      Self::Wayfern
    } else {
      Self::Fallback
    }
  }

  /// Serde default for results the browser produced: they are, by definition,
  /// the native engine's.
  fn wayfern() -> Self {
    Self::Wayfern
  }

  pub fn is_wayfern(self) -> bool {
    self == Self::Wayfern
  }
}

/// What went wrong with a Wayfern command, beyond the transport.
///
/// The locator failures are deliberately their own variants: the browser
/// encodes the candidates in its error message, and a caller that only sees
/// "CDP error" cannot show a user what was matched.
#[derive(Debug)]
pub enum WayfernError {
  /// The connection layer failed, or the browser answered with an error
  /// object this module does not interpret further.
  Cdp(CdpError),
  /// `resolveLocator` matched several nodes. `candidates` is the browser's own
  /// enumeration, capped at the requested limit; `match_count` is not.
  AmbiguousLocator {
    match_count: u64,
    candidates: Vec<Value>,
    message: String,
  },
  /// `resolveLocator` matched nothing.
  NoMatch { message: String },
  /// Nobody picked an element before the deadline.
  PickerTimedOut { timeout_ms: u64 },
  /// The picker ended without a pick: "escape", "stopped" or "navigated".
  PickerCancelled { reason: String },
  /// A reply did not have the shape the protocol documents.
  Malformed(String),
}

impl std::fmt::Display for WayfernError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Cdp(e) => write!(f, "{e}"),
      Self::AmbiguousLocator { message, .. } | Self::NoMatch { message } => f.write_str(message),
      Self::PickerTimedOut { timeout_ms } => {
        write!(f, "no element was picked within {timeout_ms} ms")
      }
      Self::PickerCancelled { reason } => write!(f, "the element picker was cancelled ({reason})"),
      Self::Malformed(m) => write!(f, "unexpected reply from the browser: {m}"),
    }
  }
}

impl From<CdpError> for WayfernError {
  fn from(error: CdpError) -> Self {
    Self::Cdp(error)
  }
}

/// A refusal the browser's own gate produced, told apart from a real failure.
///
/// Every command this module sends can be refused by the browser for the same
/// two reasons `Runtime.evaluate` can: no paid plan, or too many calls too
/// fast. The messages are the browser's contract, and it keeps the two
/// deliberately distinct so a client can tell "not entitled" from
/// "entitled but too fast".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRefusal {
  /// "Browser automation requires a paid Donut Browser plan."
  PaymentRequired,
  /// "Automation rate limit exceeded (N requests/minute). Retry shortly."
  RateLimited,
  /// The browser could not confirm this account's plan and asks for a retry
  /// rather than denying.
  AuthorizationUnavailable,
}

/// The `message` of the CDP error object a [`CdpError::Protocol`] carries.
pub fn protocol_message(error: &CdpError) -> Option<String> {
  let CdpError::Protocol(raw) = error else {
    return None;
  };
  let parsed: Value = serde_json::from_str(raw).ok()?;
  parsed
    .get("message")
    .and_then(Value::as_str)
    .map(str::to_string)
}

/// The `code` of the CDP error object a [`CdpError::Protocol`] carries.
///
/// `-32602` is the browser refusing the parameters, which is the caller's
/// problem; `-32000` is the browser failing to do what it was asked.
pub fn protocol_code(error: &CdpError) -> Option<i64> {
  let CdpError::Protocol(raw) = error else {
    return None;
  };
  let parsed: Value = serde_json::from_str(raw).ok()?;
  parsed.get("code").and_then(Value::as_i64)
}

/// Whether `error` is the browser's gate saying no, and which no it said.
pub fn classify_refusal(error: &CdpError) -> Option<BrowserRefusal> {
  let message = protocol_message(error)?;
  if message.contains("requires a paid Donut Browser plan") {
    Some(BrowserRefusal::PaymentRequired)
  } else if message.starts_with("Automation rate limit exceeded") {
    Some(BrowserRefusal::RateLimited)
  } else if message.contains("authorization service is temporarily unavailable") {
    Some(BrowserRefusal::AuthorizationUnavailable)
  } else {
    None
  }
}

// --- Locators ---------------------------------------------------------------

/// One `name=value` pair a locator requires, or a matched node carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LocatorAttribute {
  pub name: String,
  pub value: String,
}

/// How a caller names an element without a selector.
///
/// Mirrors the locator shape the browser takes. Keys are the browser's own
/// (`nameContains`, `textContains`); the snake_case spellings are accepted on
/// input so a REST body written in this API's usual style still parses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LocatorDescription {
  /// AX role token, matched case- and separator-insensitively.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub role: Option<String>,
  /// Computed accessible name, exact after whitespace collapse.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  /// Substring form of `name`.
  #[serde(
    default,
    skip_serializing_if = "Option::is_none",
    alias = "name_contains"
  )]
  pub name_contains: Option<String>,
  /// Visible text content, from the live layout.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub text: Option<String>,
  /// Substring form of `text`.
  #[serde(
    default,
    skip_serializing_if = "Option::is_none",
    alias = "text_contains"
  )]
  pub text_contains: Option<String>,
  /// Attribute pairs that must all match.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attributes: Option<Vec<LocatorAttribute>>,
}

impl LocatorDescription {
  /// A locator with nothing in it matches everything, which is never what a
  /// caller meant; refused here before the browser is asked.
  pub fn is_empty(&self) -> bool {
    self.role.is_none()
      && self.name.is_none()
      && self.name_contains.is_none()
      && self.text.is_none()
      && self.text_contains.is_none()
      && self.attributes.as_ref().is_none_or(Vec::is_empty)
  }
}

/// Where a matched node is, in root-document CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LocatorBounds {
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
}

/// Everything a caller needs about one matched node.
///
/// `value` is omitted, never blanked, for a control the page marked protected;
/// `backendNodeId` is absent only on the fallback engine, which has no DOM
/// agent behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LocatorCandidate {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub backend_node_id: Option<i64>,
  pub role: String,
  pub name: String,
  pub text: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub value: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub url: Option<String>,
  /// Per-profile deterministic identifier for the node's structural position.
  pub signature: String,
  #[serde(default)]
  pub attributes: Vec<LocatorAttribute>,
  pub bounds: LocatorBounds,
}

/// A locator resolved to exactly one node.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LocatorResolution {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub backend_node_id: Option<i64>,
  /// Always 1: present so a caller can assert it rather than infer it.
  pub match_count: u64,
  #[serde(rename = "match")]
  pub matched: LocatorCandidate,
  /// The canonical form of the locator that resolved.
  pub locator: LocatorDescription,
  #[serde(skip_deserializing, default = "Engine::wayfern")]
  pub engine: Engine,
}

/// Options for `Wayfern.resolveLocator` beyond the locator itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResolveOptions {
  /// How many candidates an ambiguity error enumerates. Default 10, ceiling 100.
  pub candidate_limit: Option<u64>,
  /// Node cap for the snapshot. Default 20000, ceiling 200000.
  pub max_nodes: Option<u64>,
  /// Snapshot budget in milliseconds. Default 3000, ceiling 30000.
  pub time_budget_ms: Option<u64>,
}

// --- Perception -------------------------------------------------------------

/// What a caller may ask `Wayfern.capturePagePerception` for.
///
/// Field names are this API's snake_case; the browser's camelCase spellings
/// are accepted too.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct PerceptionRequest {
  /// Total byte cap across cursor pages. Default 1 MiB, ceiling 4 MiB.
  #[serde(default, alias = "maxBytes")]
  pub max_bytes: Option<u64>,
  /// Capture budget in milliseconds. Default 5000, clamped to [100, 60000].
  #[serde(default, alias = "budgetMs")]
  pub budget_ms: Option<u64>,
  /// Per-frame node ceiling. Default 100000; 0 for no limit.
  #[serde(default, alias = "maxNodes")]
  pub max_nodes: Option<u64>,
  /// Include readable text. Default true.
  #[serde(default, alias = "includeText")]
  pub include_text: Option<bool>,
  /// Drop nodes outside the viewport. Default false.
  #[serde(default, alias = "viewportOnly")]
  pub viewport_only: Option<bool>,
  /// "reading" (default) or "visual".
  #[serde(default, alias = "textOrder")]
  pub text_order: Option<String>,
  /// Continue an earlier capture from the cursor it returned.
  #[serde(default)]
  pub cursor: Option<String>,
}

impl PerceptionRequest {
  /// The total byte cap this request asks for, within the allowed range.
  pub fn byte_cap(&self) -> u64 {
    self
      .max_bytes
      .unwrap_or(DEFAULT_PERCEPTION_BYTE_CAP)
      .clamp(MIN_PERCEPTION_BYTE_CAP, MAX_PERCEPTION_BYTE_CAP)
  }

  /// The parameters of the FIRST capture. A continuation carries the cursor
  /// alone, because the browser ignores everything else when one is present.
  fn browser_params(&self, byte_cap: u64) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("maxBytes".into(), Value::from(byte_cap));
    if let Some(budget) = self.budget_ms {
      params.insert("budgetMs".into(), Value::from(budget));
    }
    if let Some(nodes) = self.max_nodes {
      params.insert("maxNodes".into(), Value::from(nodes));
    }
    if let Some(text) = self.include_text {
      params.insert("includeText".into(), Value::from(text));
    }
    if let Some(viewport) = self.viewport_only {
      params.insert("viewportOnly".into(), Value::from(viewport));
    }
    if let Some(order) = &self.text_order {
      params.insert("textOrder".into(), Value::from(order.as_str()));
    }
    Value::Object(params)
  }
}

/// One node of a perception snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerceptionNode {
  /// Short, stable, frame-qualified handle.
  pub id: String,
  pub frame_id: String,
  pub role: String,
  /// Root-document page coordinates in CSS pixels, unclipped.
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
  pub in_viewport: bool,
  pub visible: bool,
  pub focused: bool,
  pub disabled: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub parent_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub text: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub value: Option<String>,
  /// "true", "false" or "mixed"; absent for anything not checkable.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub checked: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub expanded: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub scrollable: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub scroll_container_id: Option<String>,
}

/// One frame of a perception snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerceptionFrame {
  pub frame_id: String,
  pub url: String,
  pub cross_origin: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub parent_frame_id: Option<String>,
}

/// How much a capture covered, and how much it cost.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerceptionStats {
  /// Nodes in the whole snapshot, not on this page.
  pub total_nodes: u64,
  pub returned_nodes: u64,
  /// Serialized size of the nodes and text returned.
  pub bytes: u64,
  pub elapsed_ms: u64,
  pub frames_visited: u64,
  /// Frames whose renderer did not answer within the budget.
  pub frames_failed: u64,
}

/// A perception snapshot, or as much of one as the byte cap allowed.
///
/// When `truncated` is true a `cursor` follows: pass it back to continue from
/// where this call stopped.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerceptionPage {
  pub snapshot_id: String,
  pub nodes: Vec<PerceptionNode>,
  pub frames: Vec<PerceptionFrame>,
  /// Readable text for exactly the nodes returned.
  pub text: String,
  pub truncated: bool,
  pub stats: PerceptionStats,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cursor: Option<String>,
  #[serde(skip_deserializing, default = "Engine::wayfern")]
  pub engine: Engine,
}

// --- Extraction -------------------------------------------------------------

/// One output column of a structured extraction.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExtractionField {
  /// The key this column appears under in each row's values.
  pub key: String,
  /// Evaluated inside each container; the first match wins.
  pub locator: LocatorDescription,
  /// "text", "attribute" or "link".
  pub source: String,
  /// Required when `source` is "attribute".
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attribute: Option<String>,
}

/// What a caller asks `Wayfern.extractStructured` for.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ExtractionRequest {
  /// Matches every row container; several matches are the expected case.
  pub container: LocatorDescription,
  /// The columns. The browser calls this `fieldMap`; `fields` is accepted too.
  #[serde(alias = "fieldMap", alias = "fields")]
  pub field_map: Vec<ExtractionField>,
  /// The control clicked to advance a page. Absent means one page.
  #[serde(default, alias = "nextPage")]
  pub next_page: Option<LocatorDescription>,
  /// Default 1, ceiling 200.
  #[serde(default, alias = "maxPages")]
  pub max_pages: Option<u64>,
  /// Default 1000, ceiling 100000.
  #[serde(default, alias = "maxRows")]
  pub max_rows: Option<u64>,
  /// Default 262144, ceiling 8388608.
  #[serde(default, alias = "maxBytes")]
  pub max_bytes: Option<u64>,
  /// Node cap for each snapshot. Default 20000, ceiling 200000.
  #[serde(default, alias = "maxNodes")]
  pub max_nodes: Option<u64>,
  /// Default 8000, ceiling 120000.
  #[serde(default, alias = "timeBudgetMs")]
  pub time_budget_ms: Option<u64>,
}

impl ExtractionRequest {
  fn browser_params(&self) -> Result<Value, WayfernError> {
    let mut params = serde_json::Map::new();
    params.insert(
      "container".into(),
      serde_json::to_value(&self.container).map_err(|e| WayfernError::Malformed(e.to_string()))?,
    );
    params.insert(
      "fieldMap".into(),
      serde_json::to_value(&self.field_map).map_err(|e| WayfernError::Malformed(e.to_string()))?,
    );
    if let Some(next) = &self.next_page {
      params.insert(
        "nextPage".into(),
        serde_json::to_value(next).map_err(|e| WayfernError::Malformed(e.to_string()))?,
      );
    }
    for (key, value) in [
      ("maxPages", self.max_pages),
      ("maxRows", self.max_rows),
      ("maxBytes", self.max_bytes),
      ("maxNodes", self.max_nodes),
      ("timeBudgetMs", self.time_budget_ms),
    ] {
      if let Some(value) = value {
        params.insert(key.into(), Value::from(value));
      }
    }
    Ok(Value::Object(params))
  }

  /// How long to wait for the reply: the browser's own budget plus headroom.
  fn reply_timeout(&self) -> Duration {
    let budget = self.time_budget_ms.unwrap_or(8_000).clamp(1, 120_000);
    Duration::from_millis(budget) + BUDGET_HEADROOM
  }
}

/// One extracted row. A field that matched nothing is an absent key.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExtractionRow {
  /// Global across pages.
  pub index: u64,
  /// Zero-based page this row came from.
  pub page: u64,
  #[schema(value_type = Object)]
  pub values: serde_json::Map<String, Value>,
}

/// The rows an extraction produced and why it stopped.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Extraction {
  pub rows: Vec<ExtractionRow>,
  pub row_count: u64,
  pub page_count: u64,
  /// Byte length of the compact JSON serialization of `rows`.
  pub byte_size: u64,
  pub truncated: bool,
  /// "complete", "no-container", "no-next", "page-cap", "row-cap",
  /// "byte-cap" or "time-budget".
  pub stop_reason: String,
  #[serde(skip_deserializing, default = "Engine::wayfern")]
  pub engine: Engine,
}

// --- Picker -----------------------------------------------------------------

/// What the user clicked while the picker was armed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PickedElement {
  pub backend_node_id: i64,
  /// The smallest description that still resolves to this node.
  pub locator: LocatorDescription,
  /// How many nodes `locator` matches; more than 1 when the page genuinely
  /// holds indistinguishable elements.
  pub match_count: u64,
  pub node: LocatorCandidate,
  #[serde(skip_deserializing, default = "Engine::wayfern")]
  pub engine: Engine,
}

// --- Session ----------------------------------------------------------------

/// One open page session, with command ids handed out in order.
///
/// [`CdpConnection`] leaves id allocation to its caller because the one-shot
/// runners in `cdp_target` never send more than three commands. A gesture here
/// sends five or more on one socket, and a reply matched to the wrong id is a
/// silent wrong answer, so the ids are owned in one place.
pub struct WayfernSession {
  connection: CdpConnection,
  next_id: u64,
}

impl WayfernSession {
  /// Open a page session on `target`.
  pub async fn open(target: &CdpTarget) -> Result<Self, WayfernError> {
    Ok(Self {
      connection: target.connect().await?,
      next_id: 1,
    })
  }

  fn allocate_id(&mut self) -> u64 {
    let id = self.next_id;
    self.next_id += 1;
    id
  }

  /// Send `method` and wait for its reply within the ordinary budget.
  pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, WayfernError> {
    self
      .call_with_timeout(method, params, COMMAND_TIMEOUT)
      .await
  }

  /// Send `method` and wait up to `timeout` for its reply.
  pub async fn call_with_timeout(
    &mut self,
    method: &str,
    params: Value,
    timeout: Duration,
  ) -> Result<Value, WayfernError> {
    let id = self.allocate_id();
    self.connection.send_command(id, method, params).await?;
    Ok(self.connection.await_reply(id, timeout).await?)
  }

  /// Send `method`, then keep reading until BOTH its reply and `event` have
  /// arrived, or `timeout` passes.
  ///
  /// The reply alone is never enough for an action that may navigate: the
  /// browser answers `Vellum.strike` as soon as the release is dispatched, and
  /// the load that follows is what the caller is waiting for. The reply alone
  /// is, however, still the answer when no load ever comes; that is the normal
  /// case for a click that only changed state on the page.
  ///
  /// Returns the reply and whether `event` was seen. An error reply is an
  /// error; a socket that dies before the reply is one too.
  pub async fn call_then_await_event(
    &mut self,
    method: &str,
    params: Value,
    event: &str,
    timeout: Duration,
  ) -> Result<(Value, bool), WayfernError> {
    let id = self.allocate_id();
    self.connection.send_command(id, method, params).await?;

    let deadline = tokio::time::Instant::now() + timeout;
    let mut reply: Option<Value> = None;
    let mut event_seen = false;

    loop {
      if reply.is_some() && event_seen {
        break;
      }
      let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
      if remaining.is_zero() {
        break;
      }
      let text = match tokio::time::timeout(remaining, self.connection.next_text()).await {
        Ok(Some(Ok(text))) => text,
        Ok(Some(Err(e))) => {
          if reply.is_none() {
            return Err(e.into());
          }
          break;
        }
        Ok(None) => {
          if reply.is_none() {
            return Err(
              self
                .connection
                .closed_error("no response received from CDP")
                .into(),
            );
          }
          break;
        }
        Err(_) => break,
      };

      let message: Value = serde_json::from_str(&text).unwrap_or_default();
      if message.get("id") == Some(&Value::from(id)) {
        if let Some(error) = message.get("error") {
          return Err(CdpError::Protocol(error.to_string()).into());
        }
        reply = Some(
          message
            .get("result")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        );
        continue;
      }
      if message.get("method").and_then(Value::as_str) == Some(event) {
        event_seen = true;
      }
    }

    match reply {
      Some(reply) => Ok((reply, event_seen)),
      None => {
        Err(CdpError::Transport(format!("timed out waiting for the reply to {method}")).into())
      }
    }
  }

  /// Wait for the first of `events`, discarding everything else.
  ///
  /// `Ok(None)` is the deadline passing; a dead socket is an error.
  pub async fn await_any_event(
    &mut self,
    events: &[&str],
    timeout: Duration,
  ) -> Result<Option<(String, Value)>, WayfernError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
      let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
      if remaining.is_zero() {
        return Ok(None);
      }
      let text = match tokio::time::timeout(remaining, self.connection.next_text()).await {
        Ok(Some(Ok(text))) => text,
        Ok(Some(Err(e))) => return Err(e.into()),
        Ok(None) => {
          return Err(
            self
              .connection
              .closed_error("the browser closed the session while an event was awaited")
              .into(),
          )
        }
        Err(_) => return Ok(None),
      };
      let message: Value = serde_json::from_str(&text).unwrap_or_default();
      let Some(method) = message.get("method").and_then(Value::as_str) else {
        continue;
      };
      if events.contains(&method) {
        let params = message
          .get("params")
          .cloned()
          .unwrap_or_else(|| serde_json::json!({}));
        return Ok(Some((method.to_string(), params)));
      }
    }
  }

  /// Hang up politely.
  pub async fn close(self) {
    self.connection.close().await;
  }
}

// --- Perception, locators, extraction, picker --------------------------------

/// Capture the page, following cursors until the browser is done or the byte
/// cap is reached.
///
/// Pages are merged into one result. `truncated` and `cursor` describe what is
/// left AFTER this call: false and absent when the browser said it was done,
/// true with the next cursor when this side stopped at the cap.
pub async fn capture_page_perception(
  session: &mut WayfernSession,
  request: &PerceptionRequest,
) -> Result<PerceptionPage, WayfernError> {
  let byte_cap = request.byte_cap();
  let budget = request.budget_ms.unwrap_or(5_000).clamp(100, 60_000);
  let reply_timeout = Duration::from_millis(budget) + BUDGET_HEADROOM;

  let mut params = match &request.cursor {
    Some(cursor) if !cursor.is_empty() => serde_json::json!({ "cursor": cursor }),
    _ => request.browser_params(byte_cap),
  };

  let mut merged: Option<PerceptionPage> = None;
  let mut accumulated: u64 = 0;

  for _ in 0..MAX_PERCEPTION_PAGES {
    let raw = session
      .call_with_timeout("Wayfern.capturePagePerception", params, reply_timeout)
      .await?;
    let page: PerceptionPage = serde_json::from_value(raw)
      .map_err(|e| WayfernError::Malformed(format!("capturePagePerception: {e}")))?;
    accumulated = accumulated.saturating_add(page.stats.bytes);

    let next_cursor = page.cursor.clone().filter(|c| !c.is_empty());
    let more = page.truncated && next_cursor.is_some();

    merged = Some(match merged {
      None => page,
      Some(mut whole) => {
        whole.nodes.extend(page.nodes);
        whole.text.push_str(&page.text);
        whole.frames = page.frames;
        whole.stats.total_nodes = page.stats.total_nodes;
        whole.stats.returned_nodes += page.stats.returned_nodes;
        whole.stats.bytes += page.stats.bytes;
        whole.stats.elapsed_ms += page.stats.elapsed_ms;
        whole.stats.frames_visited = whole.stats.frames_visited.max(page.stats.frames_visited);
        whole.stats.frames_failed = whole.stats.frames_failed.max(page.stats.frames_failed);
        whole.truncated = page.truncated;
        whole.cursor = page.cursor;
        whole
      }
    });

    if !more || accumulated >= byte_cap {
      break;
    }
    params = serde_json::json!({ "cursor": next_cursor });
  }

  let mut result = merged
    .ok_or_else(|| WayfernError::Malformed("capturePagePerception answered nothing".into()))?;
  result.engine = Engine::Wayfern;
  if !result.truncated {
    result.cursor = None;
  }
  Ok(result)
}

/// Resolve `locator` to exactly one node, or say precisely why not.
pub async fn resolve_locator(
  session: &mut WayfernSession,
  locator: &LocatorDescription,
  options: ResolveOptions,
) -> Result<LocatorResolution, WayfernError> {
  let mut params = serde_json::Map::new();
  params.insert(
    "locator".into(),
    serde_json::to_value(locator).map_err(|e| WayfernError::Malformed(e.to_string()))?,
  );
  for (key, value) in [
    ("candidateLimit", options.candidate_limit),
    ("maxNodes", options.max_nodes),
    ("timeBudgetMs", options.time_budget_ms),
  ] {
    if let Some(value) = value {
      params.insert(key.into(), Value::from(value));
    }
  }
  let budget = options.time_budget_ms.unwrap_or(3_000).clamp(1, 30_000);
  let reply_timeout = Duration::from_millis(budget) + BUDGET_HEADROOM;

  let raw = match session
    .call_with_timeout(
      "Wayfern.resolveLocator",
      Value::Object(params),
      reply_timeout,
    )
    .await
  {
    Ok(raw) => raw,
    Err(WayfernError::Cdp(error)) => return Err(classify_locator_error(error)),
    Err(other) => return Err(other),
  };
  let mut resolution: LocatorResolution = serde_json::from_value(raw)
    .map_err(|e| WayfernError::Malformed(format!("resolveLocator: {e}")))?;
  resolution.engine = Engine::Wayfern;
  Ok(resolution)
}

/// Turn the browser's locator refusals into their structured form.
///
/// The ambiguity message is machine-readable by contract: the `Candidates: `
/// marker and the JSON after it are part of the protocol, not decoration.
pub fn classify_locator_error(error: CdpError) -> WayfernError {
  let Some(message) = protocol_message(&error) else {
    return WayfernError::Cdp(error);
  };
  if let Some(rest) = message.strip_prefix("Ambiguous locator: ") {
    let match_count = rest
      .split(|c: char| !c.is_ascii_digit())
      .next()
      .and_then(|digits| digits.parse::<u64>().ok())
      .unwrap_or(0);
    let candidates = message
      .split_once("Candidates: ")
      .and_then(|(_, json)| serde_json::from_str::<Vec<Value>>(json.trim()).ok())
      .unwrap_or_default();
    return WayfernError::AmbiguousLocator {
      match_count,
      candidates,
      message,
    };
  }
  if message.starts_with("No node matches locator") {
    return WayfernError::NoMatch { message };
  }
  WayfernError::Cdp(error)
}

/// Read rows off the live page.
pub async fn extract_structured(
  session: &mut WayfernSession,
  request: &ExtractionRequest,
) -> Result<Extraction, WayfernError> {
  let params = request.browser_params()?;
  let raw = session
    .call_with_timeout("Wayfern.extractStructured", params, request.reply_timeout())
    .await?;
  let mut extraction: Extraction = serde_json::from_value(raw)
    .map_err(|e| WayfernError::Malformed(format!("extractStructured: {e}")))?;
  extraction.engine = Engine::Wayfern;
  Ok(extraction)
}

/// Arm the picker and wait for the user to click something.
///
/// On the deadline the picker is disarmed before the error is returned, so a
/// highlight is never left on the page after the tool call that asked for it
/// has answered. The `elementPickerCancelled("stopped")` that disarming emits
/// is not read: the session is closed by the caller right after.
pub async fn pick_element(
  session: &mut WayfernSession,
  timeout: Duration,
  highlight: bool,
) -> Result<PickedElement, WayfernError> {
  session
    .call(
      "Wayfern.startElementPicker",
      serde_json::json!({ "highlight": highlight }),
    )
    .await?;

  let outcome = session
    .await_any_event(
      &["Wayfern.elementPicked", "Wayfern.elementPickerCancelled"],
      timeout,
    )
    .await;

  match outcome {
    Ok(Some((method, params))) if method == "Wayfern.elementPicked" => {
      let mut picked: PickedElement = serde_json::from_value(params)
        .map_err(|e| WayfernError::Malformed(format!("elementPicked: {e}")))?;
      picked.engine = Engine::Wayfern;
      Ok(picked)
    }
    Ok(Some((_, params))) => Err(WayfernError::PickerCancelled {
      reason: params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string(),
    }),
    Ok(None) => {
      let _ = session
        .call("Wayfern.stopElementPicker", serde_json::json!({}))
        .await;
      Err(WayfernError::PickerTimedOut {
        timeout_ms: timeout.as_millis() as u64,
      })
    }
    Err(e) => Err(e),
  }
}

// --- Geometry ---------------------------------------------------------------

/// Where to put the pointer for a node, in viewport CSS pixels.
///
/// `Vellum` takes the coordinates `Input.dispatchMouseEvent` takes, which are
/// relative to the viewport, while locator bounds are page coordinates. The
/// node is scrolled into view first, and the point is the centre of the part
/// of it that is actually visible, so an element taller than the window is
/// still struck somewhere on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportTarget {
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
}

/// The layout viewport's size in CSS pixels.
pub async fn layout_viewport(session: &mut WayfernSession) -> Result<(f64, f64), WayfernError> {
  let metrics = session
    .call("Page.getLayoutMetrics", serde_json::json!({}))
    .await?;
  let viewport = metrics
    .get("cssLayoutViewport")
    .or_else(|| metrics.get("layoutViewport"))
    .ok_or_else(|| WayfernError::Malformed("getLayoutMetrics has no layout viewport".into()))?;
  let width = viewport
    .get("clientWidth")
    .and_then(Value::as_f64)
    .unwrap_or(0.0);
  let height = viewport
    .get("clientHeight")
    .and_then(Value::as_f64)
    .unwrap_or(0.0);
  Ok((width, height))
}

/// Scroll a node into view and answer where to strike it.
pub async fn viewport_target(
  session: &mut WayfernSession,
  backend_node_id: i64,
) -> Result<ViewportTarget, WayfernError> {
  // A node that cannot be scrolled (already visible in a non-scrolling
  // container, say) still has quads; only the quads decide.
  let _ = session
    .call(
      "DOM.scrollIntoViewIfNeeded",
      serde_json::json!({ "backendNodeId": backend_node_id }),
    )
    .await;
  let quads = session
    .call(
      "DOM.getContentQuads",
      serde_json::json!({ "backendNodeId": backend_node_id }),
    )
    .await?;
  let (viewport_width, viewport_height) = layout_viewport(session).await?;
  let quads = quads
    .get("quads")
    .and_then(Value::as_array)
    .ok_or_else(|| WayfernError::Malformed("getContentQuads answered no quads".into()))?;
  quads
    .iter()
    .filter_map(|quad| quad_target(quad, viewport_width, viewport_height))
    .next()
    .ok_or_else(|| {
      WayfernError::Malformed(
        "the node has no visible box to strike; it may be hidden or clipped".into(),
      )
    })
}

/// The visible centre of one content quad, or `None` when nothing of it is on
/// screen.
fn quad_target(quad: &Value, viewport_width: f64, viewport_height: f64) -> Option<ViewportTarget> {
  let points: Vec<f64> = quad.as_array()?.iter().filter_map(Value::as_f64).collect();
  if points.len() < 8 {
    return None;
  }
  let xs = points.iter().step_by(2);
  let ys = points.iter().skip(1).step_by(2);
  let (mut left, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
  let (mut top, mut bottom) = (f64::INFINITY, f64::NEG_INFINITY);
  for x in xs {
    left = left.min(*x);
    right = right.max(*x);
  }
  for y in ys {
    top = top.min(*y);
    bottom = bottom.max(*y);
  }
  if viewport_width > 0.0 && viewport_height > 0.0 {
    left = left.max(0.0);
    top = top.max(0.0);
    right = right.min(viewport_width);
    bottom = bottom.min(viewport_height);
  }
  let width = right - left;
  let height = bottom - top;
  if !(width > 0.0 && height > 0.0) {
    return None;
  }
  Some(ViewportTarget {
    x: left + width / 2.0,
    y: top + height / 2.0,
    width,
    height,
  })
}

/// Where a glide to `target` starts.
///
/// A pointer has to come from somewhere, and a glide of zero length is not a
/// gesture. The origin sits between the target and the middle of the window,
/// nearer the middle, which is where a hand tends to rest; when the target IS
/// the middle, the origin is pulled towards the top-left instead.
pub fn glide_origin(target: &ViewportTarget, viewport: (f64, f64)) -> (f64, f64) {
  let (width, height) = viewport;
  let (centre_x, centre_y) = (width / 2.0, height / 2.0);
  let mut x = target.x + (centre_x - target.x) * 0.7;
  let mut y = target.y + (centre_y - target.y) * 0.7;
  if (x - target.x).abs() < 12.0 && (y - target.y).abs() < 12.0 {
    x = (target.x * 0.5).max(4.0);
    y = (target.y * 0.5).max(4.0);
  }
  (x.max(0.0), y.max(0.0))
}

// --- Vellum -----------------------------------------------------------------

/// The humanized-input domain.
///
/// A pointer is acquired for one tool call and released when that call is
/// done, on every path. A pointer left behind is not a leak in the browser,
/// which destroys it with the session, but a gesture still running against it
/// when the next one starts fails with "Pointer is already moving".
pub mod vellum {
  use super::{WayfernError, WayfernSession};
  use futures_util::future::BoxFuture;
  use serde::{Deserialize, Serialize};
  use serde_json::Value;
  use std::time::Duration;

  /// An acquired pointer. Release it with [`release`] (or [`with_pointer`],
  /// which does so on every path); dropping one that was never released is
  /// logged, because the browser side is only cleaned up when the session
  /// closes.
  #[derive(Debug)]
  pub struct VellumPointer {
    id: String,
    released: bool,
  }

  impl Drop for VellumPointer {
    fn drop(&mut self) {
      if !self.released {
        log::warn!(
          "[vellum] pointer {} dropped without release; the browser frees it with the session",
          self.id
        );
      }
    }
  }

  /// What `Vellum.glide` reported.
  #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
  #[serde(rename_all = "camelCase")]
  pub struct Glide {
    /// Move events dispatched.
    pub samples: u64,
    pub duration_ms: f64,
  }

  /// What `Vellum.strike` reported.
  #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
  #[serde(rename_all = "camelCase")]
  pub struct Strike {
    /// How long this profile held the press, in milliseconds.
    pub dwell_ms: f64,
  }

  /// What `Vellum.inscribe` reported.
  #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
  #[serde(rename_all = "camelCase")]
  pub struct Inscription {
    /// Characters of the text delivered; equals its length on success.
    pub characters: u64,
    /// Mistyped characters that were corrected.
    pub corrections: u64,
    pub duration_ms: f64,
  }

  /// Create a pointer at (`x`, `y`) in viewport CSS pixels.
  pub async fn acquire(
    session: &mut WayfernSession,
    x: f64,
    y: f64,
  ) -> Result<VellumPointer, WayfernError> {
    let reply = session
      .call("Vellum.acquire", serde_json::json!({ "x": x, "y": y }))
      .await?;
    let id = reply
      .get("pointer")
      .and_then(Value::as_str)
      .filter(|p| !p.is_empty())
      .ok_or_else(|| WayfernError::Malformed("Vellum.acquire answered no pointer".into()))?;
    Ok(VellumPointer {
      id: id.to_string(),
      released: false,
    })
  }

  /// Move the pointer to (`x`, `y`) along a human path. `width` is the Fitts
  /// target width in CSS pixels.
  pub async fn glide(
    session: &mut WayfernSession,
    pointer: &VellumPointer,
    x: f64,
    y: f64,
    width: Option<f64>,
  ) -> Result<Glide, WayfernError> {
    let mut params = serde_json::json!({ "pointer": pointer.id, "x": x, "y": y });
    if let Some(width) = width.filter(|w| w.is_finite() && *w > 0.0) {
      params["width"] = Value::from(width);
    }
    let reply = session.call("Vellum.glide", params).await?;
    serde_json::from_value(reply).map_err(|e| WayfernError::Malformed(format!("Vellum.glide: {e}")))
  }

  /// The parameters of `Vellum.strike`.
  pub fn strike_params(
    pointer: &VellumPointer,
    button: Option<&str>,
    click_count: Option<u32>,
  ) -> Value {
    let mut params = serde_json::json!({ "pointer": pointer.id });
    if let Some(button) = button {
      params["button"] = Value::from(button);
    }
    if let Some(count) = click_count {
      params["clickCount"] = Value::from(count);
    }
    params
  }

  /// Press and release where the pointer is.
  pub async fn strike(
    session: &mut WayfernSession,
    pointer: &VellumPointer,
    button: Option<&str>,
    click_count: Option<u32>,
  ) -> Result<Strike, WayfernError> {
    let reply = session
      .call("Vellum.strike", strike_params(pointer, button, click_count))
      .await?;
    serde_json::from_value(reply)
      .map_err(|e| WayfernError::Malformed(format!("Vellum.strike: {e}")))
  }

  /// Press and release, then wait up to `load_timeout` for a page load.
  ///
  /// `Page.enable` must already be on, or the load event is never delivered.
  /// Returns the strike and whether a load was seen.
  pub async fn strike_awaiting_load(
    session: &mut WayfernSession,
    pointer: &VellumPointer,
    button: Option<&str>,
    click_count: Option<u32>,
    load_timeout: Duration,
  ) -> Result<(Strike, bool), WayfernError> {
    let (reply, navigated) = session
      .call_then_await_event(
        "Vellum.strike",
        strike_params(pointer, button, click_count),
        "Page.loadEventFired",
        load_timeout,
      )
      .await?;
    let strike: Strike = serde_json::from_value(reply)
      .map_err(|e| WayfernError::Malformed(format!("Vellum.strike: {e}")))?;
    Ok((strike, navigated))
  }

  /// Type `text` into whatever is focused, one key at a time.
  ///
  /// The browser paces the keys, so the reply arrives only once the last one
  /// is delivered; `timeout` must cover the whole text.
  pub async fn inscribe(
    session: &mut WayfernSession,
    pointer: &VellumPointer,
    text: &str,
    typos: bool,
    timeout: Duration,
  ) -> Result<Inscription, WayfernError> {
    let reply = session
      .call_with_timeout(
        "Vellum.inscribe",
        serde_json::json!({ "pointer": pointer.id, "text": text, "typos": typos }),
        timeout,
      )
      .await?;
    serde_json::from_value(reply)
      .map_err(|e| WayfernError::Malformed(format!("Vellum.inscribe: {e}")))
  }

  /// Destroy the pointer.
  pub async fn release(
    session: &mut WayfernSession,
    mut pointer: VellumPointer,
  ) -> Result<(), WayfernError> {
    let result = session
      .call(
        "Vellum.release",
        serde_json::json!({ "pointer": pointer.id }),
      )
      .await;
    // Released as far as this side is concerned whether or not the browser
    // agreed: a second attempt could only fail with "No such pointer".
    pointer.released = true;
    result.map(|_| ())
  }

  /// Run `steps` against a freshly acquired pointer and release it afterwards,
  /// whether the steps succeeded or not.
  ///
  /// This is the shape every gesture takes. The release is the last thing on
  /// the socket in both outcomes, so a strike that failed still leaves the
  /// session clean for whatever the caller does next. A release that itself
  /// fails after a successful gesture is logged rather than reported: the
  /// click or the typing already happened, and the browser frees the pointer
  /// with the session regardless.
  pub async fn with_pointer<T, E, F>(
    session: &mut WayfernSession,
    x: f64,
    y: f64,
    steps: F,
  ) -> Result<T, E>
  where
    E: From<WayfernError>,
    F: for<'a> FnOnce(&'a mut WayfernSession, &'a VellumPointer) -> BoxFuture<'a, Result<T, E>>,
  {
    let pointer = acquire(session, x, y).await?;
    let outcome = steps(session, &pointer).await;
    let pointer_id = pointer.id.clone();
    let released = release(session, pointer).await;
    match outcome {
      Ok(value) => {
        if let Err(e) = released {
          log::warn!(
            "[vellum] pointer {pointer_id} could not be released after a completed gesture: {e}"
          );
        }
        Ok(value)
      }
      Err(e) => {
        if let Err(release_error) = released {
          log::warn!("[vellum] pointer {pointer_id} could not be released after a failed gesture: {release_error}");
        }
        Err(e)
      }
    }
  }
}

/// A stand-in for a Wayfern 152 page socket, for this module's tests and the
/// tool layer's.
///
/// Answers every command the wrappers send the way the browser does, records
/// each frame it receives, and serves any number of connections in turn, so a
/// tool that opens one socket per step is exercised end to end.
#[cfg(test)]
pub(crate) mod test_support {
  use crate::cdp_target::CdpTarget;
  use futures_util::sink::SinkExt;
  use futures_util::stream::StreamExt;
  use serde_json::Value;
  use std::sync::{Arc, Mutex};
  use std::time::Duration;
  use tokio_tungstenite::tungstenite::Message;

  /// How the fake browser behaves.
  #[derive(Clone, Copy, PartialEq, Eq, Debug)]
  pub(crate) enum Fake {
    /// Answer every command with a plausible result.
    Cooperative,
    /// Answer `Vellum.strike` with an error.
    StrikeFails,
    /// Every perception page is truncated and carries a cursor, forever.
    EndlessPerception,
    /// `resolveLocator` answers with the ambiguity error, and the fallback
    /// resolver script with two matches.
    AmbiguousLocator,
    /// Arm the picker, then emit `elementPicked` after a short pause.
    PickerPicks,
    /// Arm the picker, then emit `elementPickerCancelled`.
    PickerCancels,
    /// Arm the picker and never emit anything.
    PickerSilent,
    /// Answer `Vellum.strike`, then emit `Page.loadEventFired`.
    StrikeNavigates,
  }

  pub(crate) type Frames = Arc<Mutex<Vec<Value>>>;

  /// The candidate the fake resolves every locator to.
  fn canned_candidate() -> Value {
    serde_json::json!({
      "backendNodeId": 7, "role": "button", "name": "Save", "text": "Save",
      "signature": "s7", "attributes": [{ "name": "id", "value": "save" }],
      "bounds": { "x": 10.0, "y": 20.0, "width": 30.0, "height": 40.0 }
    })
  }

  /// What the fallback scripts answer with, keyed off the script's own text.
  fn script_answer(expression: &str, behaviour: Fake) -> Value {
    let value = if expression.contains("\"mode\":\"resolve\"") {
      if behaviour == Fake::AmbiguousLocator {
        serde_json::json!({
          "matchCount": 2,
          "candidates": [canned_candidate(), canned_candidate()]
        })
      } else {
        serde_json::json!({
          "matchCount": 1,
          "candidates": [canned_candidate()],
          "match": canned_candidate(),
          "center": { "x": 140.0, "y": 215.0, "width": 80.0, "height": 30.0, "visible": true }
        })
      }
    } else if expression.contains("\"mode\":\"perceive\"") {
      serde_json::json!({
        "nodes": [{
          "id": "n0", "frameId": "f0", "role": "button",
          "x": 1.0, "y": 2.0, "width": 3.0, "height": 4.0,
          "inViewport": true, "visible": true, "focused": false, "disabled": false,
          "name": "Save"
        }],
        "frames": [{ "frameId": "f0", "url": "https://example.com/", "crossOrigin": false }],
        "text": "Save",
        "truncated": false,
        "stats": { "totalNodes": 5, "returnedNodes": 1, "bytes": 120, "elapsedMs": 1, "framesVisited": 1, "framesFailed": 0 }
      })
    } else if expression.contains("getBoundingClientRect") {
      serde_json::json!({ "x": 140.0, "y": 215.0, "width": 80.0, "height": 30.0 })
    } else {
      Value::Bool(true)
    };
    let text = match value {
      Value::Bool(b) => return serde_json::json!({ "result": { "type": "boolean", "value": b } }),
      other => other.to_string(),
    };
    serde_json::json!({ "result": { "type": "string", "value": text } })
  }

  pub(crate) async fn fake_browser(behaviour: Fake) -> (CdpTarget, Frames) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
      .await
      .expect("the fake browser must bind");
    let port = listener.local_addr().expect("a bound port").port();
    let frames: Frames = Arc::new(Mutex::new(Vec::new()));
    let recorded = frames.clone();

    tokio::spawn(async move {
      while let Ok((socket, _)) = listener.accept().await {
        let recorded = recorded.clone();
        tokio::spawn(async move {
          let Ok(mut stream) = tokio_tungstenite::accept_async(socket).await else {
            return;
          };
          let mut perception_page = 0u64;

          while let Some(Ok(message)) = stream.next().await {
            let Message::Text(text) = message else {
              continue;
            };
            let Ok(request) = serde_json::from_str::<Value>(&text) else {
              continue;
            };
            recorded.lock().unwrap().push(request.clone());
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let method = request.get("method").and_then(Value::as_str).unwrap_or("");
            let params = request.get("params").cloned().unwrap_or_default();

            let error = |message: &str| serde_json::json!({ "id": id, "error": { "code": -32000, "message": message } });
            let ok = |result: Value| serde_json::json!({ "id": id, "result": result });

            let mut follow_up: Option<Value> = None;
            let reply = match method {
              "Vellum.acquire" => ok(serde_json::json!({ "pointer": "0000000000000001" })),
              "Vellum.glide" => ok(serde_json::json!({ "samples": 23, "durationMs": 412.5 })),
              "Vellum.strike" if behaviour == Fake::StrikeFails => error("Surface went away"),
              "Vellum.strike" => {
                if behaviour == Fake::StrikeNavigates {
                  follow_up = Some(serde_json::json!({
                    "method": "Page.loadEventFired", "params": { "timestamp": 1.0 }
                  }));
                }
                ok(serde_json::json!({ "dwellMs": 71.0 }))
              }
              "Vellum.inscribe" => {
                let text = params.get("text").and_then(Value::as_str).unwrap_or("");
                ok(serde_json::json!({
                  "characters": text.chars().count(), "corrections": 1, "durationMs": 900.0
                }))
              }
              "Wayfern.startElementPicker" => {
                follow_up = match behaviour {
                  Fake::PickerPicks => Some(serde_json::json!({
                    "method": "Wayfern.elementPicked",
                    "params": {
                      "backendNodeId": 42,
                      "locator": { "role": "button", "name": "Save" },
                      "matchCount": 1,
                      "node": {
                        "backendNodeId": 42, "role": "button", "name": "Save", "text": "Save",
                        "signature": "sig-42", "attributes": [],
                        "bounds": { "x": 1.0, "y": 2.0, "width": 30.0, "height": 10.0 }
                      }
                    }
                  })),
                  Fake::PickerCancels => Some(serde_json::json!({
                    "method": "Wayfern.elementPickerCancelled", "params": { "reason": "escape" }
                  })),
                  _ => None,
                };
                ok(serde_json::json!({}))
              }
              "Vellum.release"
              | "Page.enable"
              | "Page.disable"
              | "Wayfern.stopElementPicker"
              | "DOM.scrollIntoViewIfNeeded"
              | "Input.dispatchMouseEvent"
              | "Input.dispatchKeyEvent" => ok(serde_json::json!({})),
              "Wayfern.capturePagePerception" => {
                perception_page += 1;
                let endless = behaviour == Fake::EndlessPerception;
                let mut result = serde_json::json!({
                  "snapshotId": "snap-1",
                  "nodes": [{
                    "id": format!("n{perception_page}"), "frameId": "f0", "role": "button",
                    "x": 1.0, "y": 2.0, "width": 3.0, "height": 4.0,
                    "inViewport": true, "visible": true, "focused": false, "disabled": false,
                    "name": "Save"
                  }],
                  "frames": [{ "frameId": "f0", "url": "https://example.com/", "crossOrigin": false }],
                  "text": format!("page {perception_page} "),
                  "truncated": endless,
                  "stats": {
                    "totalNodes": 999, "returnedNodes": 1, "bytes": 600,
                    "elapsedMs": 5, "framesVisited": 1, "framesFailed": 0
                  }
                });
                if endless {
                  result["cursor"] = Value::from(format!("snap-1.{perception_page}"));
                }
                ok(result)
              }
              "Wayfern.resolveLocator" if behaviour == Fake::AmbiguousLocator => error(
                "Ambiguous locator: 2 nodes match. Refine it with a role, a stable attribute, or more exact text. Candidates: [{\"backendNodeId\":7,\"role\":\"button\",\"name\":\"Save\",\"text\":\"Save\",\"signature\":\"s7\",\"attributes\":[]},{\"backendNodeId\":8,\"role\":\"button\",\"name\":\"Save\",\"text\":\"Save\",\"signature\":\"s8\",\"attributes\":[]}]",
              ),
              "Wayfern.resolveLocator" => ok(serde_json::json!({
                "backendNodeId": 7, "matchCount": 1,
                "match": canned_candidate(),
                "locator": params.get("locator").cloned().unwrap_or_default()
              })),
              "Wayfern.extractStructured" => ok(serde_json::json!({
                "rows": [{ "index": 0, "page": 0, "values": { "title": "One" } }],
                "rowCount": 1, "pageCount": 1, "byteSize": 40, "truncated": false,
                "stopReason": "complete"
              })),
              "DOM.getContentQuads" => ok(serde_json::json!({
                "quads": [[100.0, 200.0, 180.0, 200.0, 180.0, 230.0, 100.0, 230.0]]
              })),
              "DOM.resolveNode" => ok(serde_json::json!({
                "object": { "type": "object", "objectId": "obj-7" }
              })),
              "Runtime.callFunctionOn" => {
                ok(serde_json::json!({ "result": { "type": "boolean", "value": true } }))
              }
              "Page.getLayoutMetrics" => ok(serde_json::json!({
                "cssLayoutViewport": { "clientWidth": 1280.0, "clientHeight": 720.0 }
              })),
              "Runtime.evaluate" => {
                let expression = params
                  .get("expression")
                  .and_then(Value::as_str)
                  .unwrap_or("");
                ok(script_answer(expression, behaviour))
              }
              _ => error(&format!("'{method}' wasn't found")),
            };

            if stream
              .send(Message::Text(reply.to_string().into()))
              .await
              .is_err()
            {
              break;
            }
            if let Some(event) = follow_up {
              tokio::time::sleep(Duration::from_millis(50)).await;
              if stream
                .send(Message::Text(event.to_string().into()))
                .await
                .is_err()
              {
                break;
              }
            }
          }
        });
      }
    });

    (
      CdpTarget::Local {
        ws_url: format!("ws://127.0.0.1:{port}"),
      },
      frames,
    )
  }

  /// The methods the fake saw, in order.
  pub(crate) fn methods(frames: &Frames) -> Vec<String> {
    frames
      .lock()
      .unwrap()
      .iter()
      .filter_map(|f| f.get("method").and_then(Value::as_str).map(str::to_string))
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::vellum;
  use super::*;

  #[test]
  fn the_engine_follows_the_profile_version() {
    // An older build takes the fallback path; a 152 or newer one takes the
    // native domains. An unparsable version takes the older path, which is
    // the one that cannot be refused by the browser.
    assert_eq!(Engine::for_version("151.0.7922.76"), Engine::Fallback);
    assert_eq!(Engine::for_version("152.0.7977.64"), Engine::Wayfern);
    assert_eq!(Engine::for_version("153.0.1.1"), Engine::Wayfern);
    assert_eq!(Engine::for_version("garbage"), Engine::Fallback);
    assert_eq!(
      serde_json::to_value(Engine::Wayfern).unwrap(),
      Value::from("wayfern")
    );
    assert_eq!(
      serde_json::to_value(Engine::Fallback).unwrap(),
      Value::from("fallback")
    );
  }

  #[test]
  fn a_locator_accepts_both_spellings_and_serializes_the_browsers() {
    let snake: LocatorDescription = serde_json::from_value(serde_json::json!({
      "role": "button", "name_contains": "Save", "text_contains": "Sa",
      "attributes": [{ "name": "id", "value": "save" }]
    }))
    .unwrap();
    let camel: LocatorDescription = serde_json::from_value(serde_json::json!({
      "role": "button", "nameContains": "Save", "textContains": "Sa",
      "attributes": [{ "name": "id", "value": "save" }]
    }))
    .unwrap();
    assert_eq!(snake, camel);
    let wire = serde_json::to_value(&camel).unwrap();
    assert_eq!(wire["nameContains"], "Save");
    assert_eq!(wire["textContains"], "Sa");
    assert!(wire.get("name_contains").is_none());
    assert!(
      wire.get("name").is_none(),
      "absent parts must stay absent on the wire"
    );

    assert!(LocatorDescription::default().is_empty());
    assert!(
      serde_json::from_value::<LocatorDescription>(serde_json::json!({ "attributes": [] }))
        .unwrap()
        .is_empty()
    );
    assert!(!camel.is_empty());
  }

  #[test]
  fn the_browsers_gate_is_told_apart_from_a_failure() {
    let refusal = |message: &str| {
      CdpError::Protocol(serde_json::json!({ "code": -32000, "message": message }).to_string())
    };
    assert_eq!(
      classify_refusal(&refusal(
        "Browser automation requires a paid Donut Browser plan."
      )),
      Some(BrowserRefusal::PaymentRequired)
    );
    assert_eq!(
      classify_refusal(&refusal(
        "Automation rate limit exceeded (60 requests/minute). Retry shortly."
      )),
      Some(BrowserRefusal::RateLimited)
    );
    assert_eq!(
      classify_refusal(&refusal(
        "Browser automation authorization service is temporarily unavailable. Retry the command."
      )),
      Some(BrowserRefusal::AuthorizationUnavailable)
    );
    assert_eq!(classify_refusal(&refusal("No such pointer")), None);
    assert_eq!(classify_refusal(&CdpError::Transport("x".into())), None);
    assert_eq!(
      protocol_code(&CdpError::Protocol(
        serde_json::json!({ "code": -32602, "message": "Position must be finite" }).to_string()
      )),
      Some(-32602)
    );
  }

  #[test]
  fn an_ambiguous_locator_becomes_a_candidate_list() {
    // The browser's message is machine-readable by contract: the count is the
    // FULL number of matches even when the list is capped.
    let message = "Ambiguous locator: 3 nodes match. Refine it with a role, a stable attribute, or more exact text. Candidates: [{\"backendNodeId\":11,\"role\":\"button\",\"name\":\"Save\",\"text\":\"Save\",\"signature\":\"a1\",\"attributes\":[{\"name\":\"id\",\"value\":\"s1\"}]},{\"backendNodeId\":12,\"role\":\"button\",\"name\":\"Save\",\"text\":\"Save\",\"signature\":\"a2\",\"attributes\":[]}]";
    let error = classify_locator_error(CdpError::Protocol(
      serde_json::json!({ "code": -32000, "message": message }).to_string(),
    ));
    match error {
      WayfernError::AmbiguousLocator {
        match_count,
        candidates,
        message: carried,
      } => {
        assert_eq!(match_count, 3);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0]["backendNodeId"], 11);
        assert_eq!(candidates[1]["signature"], "a2");
        assert!(carried.starts_with("Ambiguous locator"));
      }
      other => panic!("expected an ambiguity, got {other:?}"),
    }

    let missing = classify_locator_error(CdpError::Protocol(
      serde_json::json!({ "code": -32000, "message": "No node matches locator (role=button, name=Nope)." })
        .to_string(),
    ));
    assert!(
      matches!(missing, WayfernError::NoMatch { .. }),
      "{missing:?}"
    );

    // Anything else stays what it was.
    let other = classify_locator_error(CdpError::Protocol(
      serde_json::json!({ "code": -32000, "message": "No page is attached." }).to_string(),
    ));
    assert!(matches!(other, WayfernError::Cdp(_)));
  }

  #[test]
  fn a_quad_is_struck_at_the_centre_of_its_visible_part() {
    // An element taller than the window has its centre off screen; the point
    // must be inside the viewport or the strike lands on nothing.
    let quad = serde_json::json!([10.0, -500.0, 110.0, -500.0, 110.0, 900.0, 10.0, 900.0]);
    let target = quad_target(&quad, 800.0, 600.0).expect("a visible box");
    assert_eq!(target.x, 60.0);
    assert_eq!(target.y, 300.0);
    assert_eq!(target.width, 100.0);
    assert_eq!(target.height, 600.0);

    // Fully off screen is not a target.
    let hidden = serde_json::json!([900.0, 10.0, 950.0, 10.0, 950.0, 50.0, 900.0, 50.0]);
    assert!(quad_target(&hidden, 800.0, 600.0).is_none());
    assert!(quad_target(&serde_json::json!([1.0, 2.0]), 800.0, 600.0).is_none());

    // The origin of a glide is never the target itself.
    let origin = glide_origin(&target, (800.0, 600.0));
    assert!(origin != (target.x, target.y));
    let centred = ViewportTarget {
      x: 400.0,
      y: 300.0,
      width: 20.0,
      height: 20.0,
    };
    let origin = glide_origin(&centred, (800.0, 600.0));
    assert!((origin.0 - 400.0).abs() > 12.0 || (origin.1 - 300.0).abs() > 12.0);
  }

  #[test]
  fn a_perception_request_only_sends_what_the_caller_set() {
    let request: PerceptionRequest = serde_json::from_value(serde_json::json!({
      "max_bytes": 4096, "viewportOnly": true, "text_order": "visual"
    }))
    .unwrap();
    assert_eq!(request.byte_cap(), 4096);
    let params = request.browser_params(request.byte_cap());
    assert_eq!(params["maxBytes"], 4096);
    assert_eq!(params["viewportOnly"], true);
    assert_eq!(params["textOrder"], "visual");
    assert!(params.get("budgetMs").is_none());
    assert!(params.get("includeText").is_none());

    // The cap is bounded both ways.
    let tiny: PerceptionRequest =
      serde_json::from_value(serde_json::json!({ "max_bytes": 1 })).unwrap();
    assert_eq!(tiny.byte_cap(), MIN_PERCEPTION_BYTE_CAP);
    let huge: PerceptionRequest =
      serde_json::from_value(serde_json::json!({ "max_bytes": 1u64 << 40 })).unwrap();
    assert_eq!(huge.byte_cap(), MAX_PERCEPTION_BYTE_CAP);
    assert_eq!(
      PerceptionRequest::default().byte_cap(),
      DEFAULT_PERCEPTION_BYTE_CAP
    );
  }

  #[test]
  fn an_extraction_request_speaks_the_browsers_parameter_names() {
    let request: ExtractionRequest = serde_json::from_value(serde_json::json!({
      "container": { "role": "listitem" },
      "fields": [
        { "key": "title", "locator": { "role": "link" }, "source": "text" },
        { "key": "href", "locator": { "role": "link" }, "source": "link" }
      ],
      "next_page": { "role": "button", "name": "Next" },
      "max_pages": 3
    }))
    .unwrap();
    let params = request.browser_params().unwrap();
    assert_eq!(params["container"]["role"], "listitem");
    assert_eq!(params["fieldMap"].as_array().unwrap().len(), 2);
    assert_eq!(params["fieldMap"][1]["source"], "link");
    assert_eq!(params["nextPage"]["name"], "Next");
    assert_eq!(params["maxPages"], 3);
    assert!(
      params.get("fields").is_none(),
      "the browser's name is fieldMap"
    );
    assert!(params.get("maxRows").is_none());
  }

  // --- Against a fake browser ----------------------------------------------
  //
  // Everything above is pure. What follows drives the wrappers against the
  // fake Wayfern socket in `test_support`, because the properties that
  // matter, the frames actually put on the wire and the release that has to
  // follow a failed strike, only show up when something answers.
  use super::test_support::{fake_browser, methods, Fake};

  #[tokio::test]
  async fn a_gesture_releases_its_pointer_after_success() {
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let mut session = WayfernSession::open(&target).await.unwrap();

    let strike: vellum::Strike = vellum::with_pointer(&mut session, 5.0, 6.0, |s, p| {
      Box::pin(async move {
        vellum::glide(s, p, 140.0, 215.0, Some(30.0)).await?;
        vellum::strike(s, p, None, None).await
      })
    })
    .await
    .expect("a cooperative browser completes the gesture");
    assert_eq!(strike.dwell_ms, 71.0);
    session.close().await;

    assert_eq!(
      methods(&frames),
      vec![
        "Vellum.acquire",
        "Vellum.glide",
        "Vellum.strike",
        "Vellum.release"
      ]
    );
    let frames = frames.lock().unwrap();
    // The page session drives its own frame: no surface is ever named.
    assert!(frames[0]["params"].get("surface").is_none());
    assert_eq!(frames[0]["params"]["x"], 5.0);
    assert_eq!(frames[1]["params"]["pointer"], "0000000000000001");
    assert_eq!(frames[1]["params"]["width"], 30.0);
    assert_eq!(frames[3]["params"]["pointer"], "0000000000000001");
    // Local page sockets carry no session id.
    assert!(frames.iter().all(|f| f.get("sessionId").is_none()));
  }

  #[tokio::test]
  async fn a_gesture_releases_its_pointer_after_a_failure_mid_sequence() {
    // The property the guard exists for: a strike the browser refused must not
    // leave the pointer behind, or the next gesture on this session fails with
    // "Pointer is already moving".
    let (target, frames) = fake_browser(Fake::StrikeFails).await;
    let mut session = WayfernSession::open(&target).await.unwrap();

    let error: WayfernError = vellum::with_pointer(&mut session, 5.0, 6.0, |s, p| {
      Box::pin(async move {
        vellum::glide(s, p, 140.0, 215.0, None).await?;
        vellum::strike(s, p, Some("left"), Some(1)).await
      })
    })
    .await
    .expect_err("the refused strike must surface");
    assert!(
      matches!(&error, WayfernError::Cdp(CdpError::Protocol(m)) if m.contains("Surface went away")),
      "{error:?}"
    );
    session.close().await;

    assert_eq!(
      methods(&frames),
      vec![
        "Vellum.acquire",
        "Vellum.glide",
        "Vellum.strike",
        "Vellum.release"
      ]
    );
  }

  #[tokio::test]
  async fn typing_goes_through_inscribe_and_reports_what_the_browser_did() {
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let mut session = WayfernSession::open(&target).await.unwrap();

    let report: vellum::Inscription = vellum::with_pointer(&mut session, 1.0, 1.0, |s, p| {
      Box::pin(async move {
        vellum::glide(s, p, 50.0, 50.0, Some(20.0)).await?;
        vellum::strike(s, p, None, None).await?;
        vellum::inscribe(s, p, "héllo", true, Duration::from_secs(5)).await
      })
    })
    .await
    .unwrap();
    assert_eq!(report.characters, 5);
    assert_eq!(report.corrections, 1);
    session.close().await;

    let frames = frames.lock().unwrap();
    let inscribe = frames
      .iter()
      .find(|f| f["method"] == "Vellum.inscribe")
      .expect("inscribe was sent");
    assert_eq!(inscribe["params"]["text"], "héllo");
    assert_eq!(inscribe["params"]["typos"], true);
    assert_eq!(frames.last().unwrap()["method"], "Vellum.release");
  }

  #[tokio::test]
  async fn a_strike_that_navigates_reports_the_load() {
    let (target, frames) = fake_browser(Fake::StrikeNavigates).await;
    let mut session = WayfernSession::open(&target).await.unwrap();
    session
      .call("Page.enable", serde_json::json!({}))
      .await
      .unwrap();

    let (strike, navigated): (vellum::Strike, bool) =
      vellum::with_pointer(&mut session, 1.0, 1.0, |s, p| {
        Box::pin(async move {
          vellum::strike_awaiting_load(s, p, None, None, Duration::from_secs(5)).await
        })
      })
      .await
      .unwrap();
    assert_eq!(strike.dwell_ms, 71.0);
    assert!(
      navigated,
      "the load event that followed the strike must be seen"
    );

    // And a strike nothing follows still answers, on its own reply, without
    // outliving a long wait.
    let (target2, _) = fake_browser(Fake::Cooperative).await;
    let mut quiet = WayfernSession::open(&target2).await.unwrap();
    let started = std::time::Instant::now();
    let (_, navigated): (vellum::Strike, bool) =
      vellum::with_pointer(&mut quiet, 1.0, 1.0, |s, p| {
        Box::pin(async move {
          vellum::strike_awaiting_load(s, p, None, None, Duration::from_millis(300)).await
        })
      })
      .await
      .unwrap();
    assert!(!navigated);
    assert!(started.elapsed() < Duration::from_secs(3));
    session.close().await;
    quiet.close().await;
    assert!(methods(&frames).contains(&"Vellum.release".to_string()));
  }

  #[tokio::test]
  async fn the_perception_loop_stops_at_the_byte_cap_and_hands_back_the_cursor() {
    // Every page the fake serves is 600 bytes and "truncated, here is a
    // cursor". With a 2 KiB cap the loop must stop after the fourth page and
    // say so, rather than follow cursors until the sixty-four page ceiling.
    let (target, frames) = fake_browser(Fake::EndlessPerception).await;
    let mut session = WayfernSession::open(&target).await.unwrap();

    let request: PerceptionRequest =
      serde_json::from_value(serde_json::json!({ "max_bytes": 2048, "budget_ms": 500 })).unwrap();
    let page = capture_page_perception(&mut session, &request)
      .await
      .unwrap();
    session.close().await;

    assert_eq!(page.nodes.len(), 4, "four pages of one node each");
    assert_eq!(page.text, "page 1 page 2 page 3 page 4 ");
    assert_eq!(page.stats.bytes, 2400);
    assert_eq!(page.stats.returned_nodes, 4);
    assert_eq!(page.stats.total_nodes, 999);
    assert!(page.truncated);
    assert_eq!(page.cursor.as_deref(), Some("snap-1.4"));
    assert_eq!(page.engine, Engine::Wayfern);

    let sent = frames.lock().unwrap();
    assert_eq!(sent.len(), 4);
    assert_eq!(sent[0]["params"]["maxBytes"], 2048);
    assert_eq!(sent[0]["params"]["budgetMs"], 500);
    // A continuation carries the cursor and nothing else: the browser ignores
    // every other parameter when one is present.
    assert_eq!(sent[1]["params"]["cursor"], "snap-1.1");
    assert!(sent[1]["params"].get("maxBytes").is_none());
    assert_eq!(sent[3]["params"]["cursor"], "snap-1.3");
  }

  #[tokio::test]
  async fn a_complete_capture_returns_one_page_with_no_cursor() {
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let mut session = WayfernSession::open(&target).await.unwrap();
    let page = capture_page_perception(&mut session, &PerceptionRequest::default())
      .await
      .unwrap();
    session.close().await;
    assert_eq!(page.nodes.len(), 1);
    assert_eq!(page.nodes[0].name.as_deref(), Some("Save"));
    assert!(!page.truncated);
    assert!(page.cursor.is_none());
    assert_eq!(methods(&frames), vec!["Wayfern.capturePagePerception"]);

    // The output keeps the browser's own key spelling.
    let wire = serde_json::to_value(&page).unwrap();
    assert_eq!(wire["nodes"][0]["inViewport"], true);
    assert_eq!(wire["nodes"][0]["frameId"], "f0");
    assert_eq!(wire["stats"]["totalNodes"], 999);
    assert_eq!(wire["snapshotId"], "snap-1");
    assert_eq!(wire["engine"], "wayfern");
  }

  #[tokio::test]
  async fn resolving_a_locator_answers_the_match_or_the_candidates() {
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let mut session = WayfernSession::open(&target).await.unwrap();
    let locator = LocatorDescription {
      role: Some("button".into()),
      name: Some("Save".into()),
      ..Default::default()
    };
    let resolved = resolve_locator(
      &mut session,
      &locator,
      ResolveOptions {
        candidate_limit: Some(5),
        ..Default::default()
      },
    )
    .await
    .unwrap();
    assert_eq!(resolved.backend_node_id, Some(7));
    assert_eq!(resolved.match_count, 1);
    assert_eq!(resolved.matched.signature, "s7");
    assert_eq!(resolved.matched.bounds.width, 30.0);
    assert_eq!(resolved.locator, locator);
    let wire = serde_json::to_value(&resolved).unwrap();
    assert_eq!(wire["match"]["backendNodeId"], 7);
    assert_eq!(wire["matchCount"], 1);

    // The strike point is scrolled to and read from the DOM, in viewport
    // pixels, never from the page-coordinate bounds.
    let point = viewport_target(&mut session, 7).await.unwrap();
    assert_eq!((point.x, point.y), (140.0, 215.0));
    assert_eq!((point.width, point.height), (80.0, 30.0));
    session.close().await;
    {
      let sent = frames.lock().unwrap();
      assert_eq!(sent[0]["params"]["locator"]["name"], "Save");
      assert_eq!(sent[0]["params"]["candidateLimit"], 5);
      assert!(sent[0]["params"].get("maxNodes").is_none());
      assert_eq!(sent[1]["method"], "DOM.scrollIntoViewIfNeeded");
      assert_eq!(sent[1]["params"]["backendNodeId"], 7);
      assert_eq!(sent[2]["method"], "DOM.getContentQuads");
    }

    let (target, _) = fake_browser(Fake::AmbiguousLocator).await;
    let mut session = WayfernSession::open(&target).await.unwrap();
    let error = resolve_locator(&mut session, &locator, ResolveOptions::default())
      .await
      .expect_err("two matches must be refused");
    session.close().await;
    match error {
      WayfernError::AmbiguousLocator {
        match_count,
        candidates,
        ..
      } => {
        assert_eq!(match_count, 2);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[1]["backendNodeId"], 8);
      }
      other => panic!("expected an ambiguity, got {other:?}"),
    }
  }

  #[tokio::test]
  async fn extraction_passes_the_browsers_result_through() {
    let (target, frames) = fake_browser(Fake::Cooperative).await;
    let mut session = WayfernSession::open(&target).await.unwrap();
    let request: ExtractionRequest = serde_json::from_value(serde_json::json!({
      "container": { "role": "listitem" },
      "field_map": [{ "key": "title", "locator": { "role": "link" }, "source": "text" }],
      "time_budget_ms": 1000
    }))
    .unwrap();
    let extraction = extract_structured(&mut session, &request).await.unwrap();
    session.close().await;
    assert_eq!(extraction.row_count, 1);
    assert_eq!(extraction.rows[0].values["title"], "One");
    assert_eq!(extraction.stop_reason, "complete");
    let wire = serde_json::to_value(&extraction).unwrap();
    assert_eq!(wire["rowCount"], 1);
    assert_eq!(wire["stopReason"], "complete");
    assert_eq!(wire["engine"], "wayfern");
    let sent = frames.lock().unwrap();
    assert_eq!(sent[0]["params"]["fieldMap"][0]["key"], "title");
    assert_eq!(sent[0]["params"]["timeBudgetMs"], 1000);
  }

  #[tokio::test]
  async fn the_picker_answers_a_pick_a_cancel_and_a_timeout() {
    let (target, _) = fake_browser(Fake::PickerPicks).await;
    let mut session = WayfernSession::open(&target).await.unwrap();
    let picked = pick_element(&mut session, Duration::from_secs(5), true)
      .await
      .unwrap();
    session.close().await;
    assert_eq!(picked.backend_node_id, 42);
    assert_eq!(picked.locator.name.as_deref(), Some("Save"));
    assert_eq!(picked.node.signature, "sig-42");
    assert_eq!(picked.match_count, 1);

    let (target, _) = fake_browser(Fake::PickerCancels).await;
    let mut session = WayfernSession::open(&target).await.unwrap();
    let error = pick_element(&mut session, Duration::from_secs(5), true)
      .await
      .expect_err("escape must not look like a pick");
    session.close().await;
    assert!(
      matches!(&error, WayfernError::PickerCancelled { reason } if reason == "escape"),
      "{error:?}"
    );

    // A picker nobody answers is disarmed before the timeout is reported, so
    // no highlight outlives the call.
    let (target, frames) = fake_browser(Fake::PickerSilent).await;
    let mut session = WayfernSession::open(&target).await.unwrap();
    let started = std::time::Instant::now();
    let error = pick_element(&mut session, Duration::from_millis(200), false)
      .await
      .expect_err("silence must time out");
    session.close().await;
    assert!(matches!(
      error,
      WayfernError::PickerTimedOut { timeout_ms: 200 }
    ));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
      methods(&frames),
      vec!["Wayfern.startElementPicker", "Wayfern.stopElementPicker"]
    );
    let sent = frames.lock().unwrap();
    assert_eq!(sent[0]["params"]["highlight"], false);
  }
}

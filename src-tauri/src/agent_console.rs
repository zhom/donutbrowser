//! The person's side of connected MCP agents: who is connected, what they did,
//! questions and help requests, notes, profile take-overs and a global pause.
//! Memory only; nothing here is persisted.

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const EVENT_SESSION: &str = "agent-console-session";
pub const EVENT_ACTIVITY: &str = "agent-console-activity";
pub const EVENT_THREAD: &str = "agent-console-thread";
pub const EVENT_STATE: &str = "agent-console-state";
pub const EVENT_CLEARED: &str = "agent-console-cleared";

pub const MAX_TEXT_CHARS: usize = 2000;
const MAX_PROGRESS_CHARS: usize = 280;
const MAX_CHOICES: usize = 8;
const MAX_CHOICE_CHARS: usize = 120;
const MAX_ACTIVITY: usize = 1000;
const MAX_THREAD: usize = 2000;
// Twice the MCP server's live-session cap, so only ended sessions overflow it.
const MAX_SESSIONS: usize = 1024;
const SNAPSHOT_SESSIONS: usize = 50;
pub const DEFAULT_WAIT_SECS: u64 = 45;
// The cloud relay answers a call with 503 after 90 s.
pub const MAX_WAIT_SECS: u64 = 55;
// Each waiting call holds one of the bridge's eight in-flight slots.
const MAX_CONCURRENT_WAITS: usize = 4;
// A note sent to all agents also reaches agents that connect shortly after.
const BROADCAST_GRACE_MS: u64 = 10 * 60 * 1000;
const STATE_EMIT_INTERVAL_MS: u64 = 2000;

pub const HUMAN_TOOLS: [&str; 5] = [
  "ask_human",
  "request_human_help",
  "wait_for_human",
  "get_human_updates",
  "report_progress",
];

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
  pub message: String,
  pub done: Option<u64>,
  pub total: Option<u64>,
  pub profile_id: Option<String>,
  pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentSession {
  pub session_id: String,
  pub client_name: Option<String>,
  pub client_version: Option<String>,
  pub connected_at: u64,
  pub last_seen_at: u64,
  pub calls: u64,
  pub errors: u64,
  pub ended: bool,
  pub status: Option<AgentStatus>,
  #[serde(skip)]
  joined: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentActivity {
  pub id: u64,
  pub at: u64,
  pub session_id: Option<String>,
  pub tool: String,
  pub profile_id: Option<String>,
  pub profile_count: Option<u32>,
  pub ok: bool,
  pub error_code: Option<String>,
  pub duration_ms: u64,
  pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadKind {
  Question,
  Help,
  Note,
  Progress,
  Joined,
  Left,
  Feedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
  Open,
  Answered,
  Dismissed,
  Pending,
  Delivered,
  None,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentThreadItem {
  pub id: u64,
  pub at: u64,
  pub kind: ThreadKind,
  pub session_id: Option<String>,
  pub text: String,
  pub profile_id: Option<String>,
  pub choices: Vec<String>,
  pub state: ThreadState,
  pub answer: Option<String>,
  pub answered_at: Option<u64>,
  pub delivered_to: Vec<String>,
  pub done: Option<u64>,
  pub total: Option<u64>,
  /// Set on a `feedback` item: what the agent sent to the Donut team.
  pub feedback: Option<ThreadFeedback>,
  #[serde(skip)]
  outcome_seen: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadFeedback {
  pub kind: String,
  pub logs: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentHold {
  pub profile_id: String,
  pub since: u64,
  pub note: Option<String>,
  pub request_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentsPause {
  pub since: u64,
  pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentConsoleSnapshot {
  pub sessions: Vec<AgentSession>,
  pub activity: Vec<AgentActivity>,
  pub thread: Vec<AgentThreadItem>,
  pub holds: Vec<AgentHold>,
  pub paused: Option<AgentsPause>,
  pub quota: Option<crate::automation_rate_limiter::AutomationQuota>,
}

#[derive(Debug, Clone, Serialize)]
struct StatePayload {
  holds: Vec<AgentHold>,
  paused: Option<AgentsPause>,
  quota: Option<crate::automation_rate_limiter::AutomationQuota>,
}

/// What an agent's wait ends with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestOutcome {
  Pending,
  Answered(String),
  Done(Option<String>),
  Dismissed,
}

/// Why a tool call may not run right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
  Paused {
    note: Option<String>,
  },
  ProfileHeld {
    profile_id: String,
    note: Option<String>,
  },
}

#[derive(Default)]
struct Console {
  sessions: HashMap<String, AgentSession>,
  activity: VecDeque<AgentActivity>,
  thread: VecDeque<AgentThreadItem>,
  holds: HashMap<String, AgentHold>,
  paused: Option<AgentsPause>,
  next_id: u64,
  last_state_emit: u64,
}

impl Console {
  fn next_id(&mut self) -> u64 {
    self.next_id += 1;
    self.next_id
  }

  fn holds_sorted(&self) -> Vec<AgentHold> {
    let mut holds: Vec<AgentHold> = self.holds.values().cloned().collect();
    holds.sort_by_key(|hold| hold.since);
    holds
  }

  fn push_thread(&mut self, item: AgentThreadItem) -> AgentThreadItem {
    self.thread.push_back(item.clone());
    while self.thread.len() > MAX_THREAD {
      // Open requests are never dropped; the oldest settled item goes first.
      match self
        .thread
        .iter()
        .position(|entry| entry.state != ThreadState::Open)
      {
        Some(index) => {
          self.thread.remove(index);
        }
        None => break,
      }
    }
    item
  }

  fn find_thread_mut(&mut self, id: u64) -> Option<&mut AgentThreadItem> {
    self.thread.iter_mut().find(|item| item.id == id)
  }

  fn evict_sessions(&mut self) {
    while self.sessions.len() > MAX_SESSIONS {
      let Some(oldest) = self
        .sessions
        .values()
        .min_by_key(|session| (!session.ended, session.calls > 0, session.last_seen_at))
        .map(|session| session.session_id.clone())
      else {
        break;
      };
      self.sessions.remove(&oldest);
    }
  }
}

static CONSOLE: LazyLock<Mutex<Console>> = LazyLock::new(|| Mutex::new(Console::default()));
static WAITERS: AtomicUsize = AtomicUsize::new(0);
static CHANGES: LazyLock<tokio::sync::watch::Sender<u64>> =
  LazyLock::new(|| tokio::sync::watch::channel(0).0);

fn console() -> std::sync::MutexGuard<'static, Console> {
  CONSOLE
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_ms() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0)
}

fn notify_waiters() {
  CHANGES.send_modify(|version| *version = version.wrapping_add(1));
}

fn emit<S: Serialize>(event: &str, payload: &S) {
  let _ = crate::events::emit(event, payload);
}

fn clean_text(text: &str, max: usize) -> Result<String, String> {
  let text = text.trim();
  if text.chars().count() > max {
    return Err(
      serde_json::json!({ "code": "AGENT_TEXT_TOO_LONG", "params": { "max": max.to_string() } })
        .to_string(),
    );
  }
  Ok(text.to_string())
}

fn optional_text(text: Option<String>) -> Result<Option<String>, String> {
  match text {
    Some(text) => {
      let text = clean_text(&text, MAX_TEXT_CHARS)?;
      Ok((!text.is_empty()).then_some(text))
    }
    None => Ok(None),
  }
}

fn thread_item(kind: ThreadKind, at: u64) -> AgentThreadItem {
  AgentThreadItem {
    id: 0,
    at,
    kind,
    session_id: None,
    text: String::new(),
    profile_id: None,
    choices: Vec::new(),
    state: ThreadState::None,
    answer: None,
    answered_at: None,
    delivered_to: Vec::new(),
    done: None,
    total: None,
    feedback: None,
    outcome_seen: false,
  }
}

async fn emit_state(force: bool) {
  let now = now_ms();
  {
    let mut state = console();
    if !force && now.saturating_sub(state.last_state_emit) < STATE_EMIT_INTERVAL_MS {
      return;
    }
    state.last_state_emit = now;
  }
  let quota = crate::automation_rate_limiter::automation_quota().await;
  let payload = {
    let state = console();
    StatePayload {
      holds: state.holds_sorted(),
      paused: state.paused.clone(),
      quota,
    }
  };
  emit(EVENT_STATE, &payload);
}

// --- Called by the MCP server ------------------------------------------------

pub fn session_started(
  session_id: &str,
  client_name: Option<String>,
  client_version: Option<String>,
) {
  let now = now_ms();
  let mut state = console();
  state.sessions.insert(
    session_id.to_string(),
    AgentSession {
      session_id: session_id.to_string(),
      client_name: client_name.map(|name| name.chars().take(80).collect()),
      client_version: client_version.map(|version| version.chars().take(40).collect()),
      connected_at: now,
      last_seen_at: now,
      calls: 0,
      errors: 0,
      ended: false,
      status: None,
      joined: false,
    },
  );
  state.evict_sessions();
}

pub fn session_ended(session_id: &str) {
  let now = now_ms();
  let (session, left) = {
    let mut state = console();
    let Some(session) = state.sessions.get_mut(session_id) else {
      return;
    };
    session.ended = true;
    let joined = session.joined;
    let session = session.clone();
    let left = joined.then(|| {
      let mut item = thread_item(ThreadKind::Left, now);
      item.session_id = Some(session_id.to_string());
      item.id = state.next_id();
      state.push_thread(item)
    });
    (session, left)
  };
  emit(EVENT_SESSION, &session);
  if let Some(item) = left {
    emit(EVENT_THREAD, &item);
  }
}

fn targeted_profiles(arguments: &serde_json::Value) -> Vec<String> {
  let mut ids = Vec::new();
  for key in ["profile_id", "leader_profile_id"] {
    if let Some(id) = arguments.get(key).and_then(serde_json::Value::as_str) {
      ids.push(id.to_string());
    }
  }
  for key in ["profile_ids", "follower_profile_ids"] {
    if let Some(list) = arguments.get(key).and_then(serde_json::Value::as_array) {
      ids.extend(list.iter().filter_map(|v| v.as_str().map(str::to_string)));
    }
  }
  ids
}

/// `passive` tools (reads and the human tools) are never refused.
pub fn check_call(passive: bool, arguments: &serde_json::Value) -> Result<(), Refusal> {
  refusal_for(&console(), passive, arguments)
}

fn refusal_for(
  state: &Console,
  passive: bool,
  arguments: &serde_json::Value,
) -> Result<(), Refusal> {
  if passive {
    return Ok(());
  }
  if let Some(pause) = &state.paused {
    return Err(Refusal::Paused {
      note: pause.note.clone(),
    });
  }
  for profile_id in targeted_profiles(arguments) {
    if let Some(hold) = state.holds.get(&profile_id) {
      return Err(Refusal::ProfileHeld {
        profile_id,
        note: hold.note.clone(),
      });
    }
  }
  Ok(())
}

pub fn safe_detail(tool: &str, arguments: &serde_json::Value) -> Option<String> {
  match tool {
    "navigate" => arguments
      .get("url")
      .and_then(serde_json::Value::as_str)
      .and_then(|url| url::Url::parse(url).ok())
      .and_then(|url| url.host_str().map(str::to_string)),
    "type_text" | "type_by_index" | "type_locator" => arguments
      .get("text")
      .and_then(serde_json::Value::as_str)
      .map(|text| text.chars().count().to_string()),
    _ => None,
  }
}

/// A human-channel call: the agent is alive, but it is not browser activity.
pub fn touch_session(session_id: Option<&str>) {
  let Some(sid) = session_id else {
    return;
  };
  let now = now_ms();
  let (session, joined) = {
    let mut state = console();
    let joined = mark_joined(&mut state, sid, now);
    let session = state.sessions.get_mut(sid).map(|session| {
      session.last_seen_at = now;
      session.clone()
    });
    (session, joined)
  };
  if let Some(item) = joined {
    emit(EVENT_THREAD, &item);
  }
  if let Some(session) = session {
    emit(EVENT_SESSION, &session);
  }
}

fn mark_joined(state: &mut Console, sid: &str, now: u64) -> Option<AgentThreadItem> {
  let session = state.sessions.get_mut(sid)?;
  if session.joined {
    return None;
  }
  session.joined = true;
  let mut item = thread_item(ThreadKind::Joined, now);
  item.session_id = Some(sid.to_string());
  item.id = state.next_id();
  Some(state.push_thread(item))
}

pub struct CallRecord<'a> {
  pub session_id: Option<&'a str>,
  pub tool: &'a str,
  pub arguments: &'a serde_json::Value,
  pub error_code: Option<String>,
  pub duration_ms: u64,
}

pub async fn record_call(record: CallRecord<'_>) {
  let now = now_ms();
  let profiles = targeted_profiles(record.arguments);
  let (profile_id, profile_count) = match profiles.len() {
    0 => (None, None),
    1 => (profiles.into_iter().next(), None),
    n => (None, Some(n as u32)),
  };
  let ok = record.error_code.is_none();
  let (entry, session, joined) = {
    let mut state = console();
    let id = state.next_id();
    let entry = AgentActivity {
      id,
      at: now,
      session_id: record.session_id.map(str::to_string),
      tool: record.tool.to_string(),
      profile_id,
      profile_count,
      ok,
      error_code: record.error_code,
      duration_ms: record.duration_ms,
      detail: safe_detail(record.tool, record.arguments),
    };
    state.activity.push_back(entry.clone());
    while state.activity.len() > MAX_ACTIVITY {
      state.activity.pop_front();
    }
    let mut joined_item = None;
    let session = match record.session_id {
      Some(sid) => {
        joined_item = mark_joined(&mut state, sid, now);
        state.sessions.get_mut(sid).map(|session| {
          session.last_seen_at = now;
          session.calls += 1;
          if !ok {
            session.errors += 1;
          }
          session.clone()
        })
      }
      None => None,
    };
    (entry, session, joined_item)
  };
  if let Some(item) = joined {
    emit(EVENT_THREAD, &item);
  }
  if let Some(session) = session {
    emit(EVENT_SESSION, &session);
  }
  emit(EVENT_ACTIVITY, &entry);
  emit_state(false).await;
}

/// Notes this session has not seen yet, marked as delivered.
pub fn take_notes(session_id: Option<&str>) -> Vec<AgentThreadItem> {
  let Some(sid) = session_id else {
    return Vec::new();
  };
  let mut delivered = Vec::new();
  {
    let mut state = console();
    let Some(connected_at) = state.sessions.get(sid).map(|s| s.connected_at) else {
      return Vec::new();
    };
    for item in state.thread.iter_mut() {
      if item.kind != ThreadKind::Note || item.delivered_to.iter().any(|s| s == sid) {
        continue;
      }
      let eligible = match &item.session_id {
        Some(target) => target == sid,
        None => item.at + BROADCAST_GRACE_MS >= connected_at,
      };
      if !eligible {
        continue;
      }
      item.delivered_to.push(sid.to_string());
      item.state = ThreadState::Delivered;
      item.answered_at.get_or_insert_with(now_ms);
      delivered.push(item.clone());
    }
  }
  for item in &delivered {
    emit(EVENT_THREAD, item);
  }
  delivered
}

pub fn note_text(note: &AgentThreadItem) -> String {
  match &note.profile_id {
    Some(profile_id) => format!(
      "Message from the person at this computer (about profile {profile_id}): {}",
      note.text
    ),
    None => format!("Message from the person at this computer: {}", note.text),
  }
}

pub struct NewRequest {
  pub session_id: Option<String>,
  pub kind: ThreadKind,
  pub text: String,
  pub profile_id: Option<String>,
  pub choices: Vec<String>,
}

pub fn validate_question(
  question: &str,
  choices: &[String],
) -> Result<(String, Vec<String>), String> {
  let question = question.trim();
  if question.is_empty() {
    return Err("question must not be empty".to_string());
  }
  if question.chars().count() > MAX_TEXT_CHARS {
    return Err(format!(
      "question is longer than {MAX_TEXT_CHARS} characters"
    ));
  }
  if choices.len() > MAX_CHOICES {
    return Err(format!("at most {MAX_CHOICES} choices"));
  }
  let mut cleaned = Vec::new();
  for choice in choices {
    let choice = choice.trim();
    if choice.is_empty() || choice.chars().count() > MAX_CHOICE_CHARS {
      return Err(format!(
        "each choice must be 1 to {MAX_CHOICE_CHARS} characters"
      ));
    }
    cleaned.push(choice.to_string());
  }
  Ok((question.to_string(), cleaned))
}

pub async fn open_request(request: NewRequest) -> u64 {
  let now = now_ms();
  let item = {
    let mut state = console();
    let mut item = thread_item(request.kind, now);
    item.id = state.next_id();
    item.session_id = request.session_id;
    item.text = request.text;
    item.profile_id = request.profile_id.clone();
    item.choices = request.choices;
    item.state = ThreadState::Open;
    if request.kind == ThreadKind::Help {
      if let Some(profile_id) = request.profile_id {
        let hold = AgentHold {
          profile_id: profile_id.clone(),
          since: now,
          note: None,
          request_id: Some(item.id),
        };
        state.holds.insert(profile_id, hold);
      }
    }
    state.push_thread(item)
  };
  emit(EVENT_THREAD, &item);
  if request.kind == ThreadKind::Help {
    emit_state(true).await;
  }
  item.id
}

pub fn request_exists(request_id: u64) -> bool {
  console().thread.iter().any(|item| {
    item.id == request_id && matches!(item.kind, ThreadKind::Question | ThreadKind::Help)
  })
}

fn outcome_of(item: &AgentThreadItem) -> RequestOutcome {
  match (item.kind, item.state) {
    (_, ThreadState::Dismissed) => RequestOutcome::Dismissed,
    (ThreadKind::Help, ThreadState::Answered) => RequestOutcome::Done(item.answer.clone()),
    (_, ThreadState::Answered) => RequestOutcome::Answered(item.answer.clone().unwrap_or_default()),
    _ => RequestOutcome::Pending,
  }
}

fn current_outcome(request_id: u64) -> Option<RequestOutcome> {
  let mut state = console();
  let item = state.find_thread_mut(request_id)?;
  let outcome = outcome_of(item);
  if outcome != RequestOutcome::Pending {
    item.outcome_seen = true;
  }
  Some(outcome)
}

struct WaitSlot;

impl WaitSlot {
  fn acquire() -> Option<Self> {
    if WAITERS.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_WAITS {
      WAITERS.fetch_sub(1, Ordering::SeqCst);
      return None;
    }
    Some(WaitSlot)
  }
}

impl Drop for WaitSlot {
  fn drop(&mut self) {
    WAITERS.fetch_sub(1, Ordering::SeqCst);
  }
}

/// Wait up to `wait_secs` for the person to settle a request.
pub async fn wait_for(request_id: u64, wait_secs: u64) -> Option<RequestOutcome> {
  let outcome = current_outcome(request_id)?;
  if outcome != RequestOutcome::Pending || wait_secs == 0 {
    return Some(outcome);
  }
  let Some(_slot) = WaitSlot::acquire() else {
    return Some(RequestOutcome::Pending);
  };
  let mut changes = CHANGES.subscribe();
  let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_secs.min(MAX_WAIT_SECS));
  loop {
    match tokio::time::timeout_at(deadline, changes.changed()).await {
      Ok(Ok(())) => {
        let outcome = current_outcome(request_id)?;
        if outcome != RequestOutcome::Pending {
          return Some(outcome);
        }
      }
      _ => return current_outcome(request_id),
    }
  }
}

pub fn report_progress(
  session_id: Option<&str>,
  message: &str,
  done: Option<u64>,
  total: Option<u64>,
  profile_id: Option<String>,
) {
  let now = now_ms();
  let message: String = message.trim().chars().take(MAX_PROGRESS_CHARS).collect();
  let (session, item) = {
    let mut state = console();
    let session = session_id.and_then(|sid| {
      state.sessions.get_mut(sid).map(|session| {
        session.status = Some(AgentStatus {
          message: message.clone(),
          done,
          total,
          profile_id: profile_id.clone(),
          updated_at: now,
        });
        session.last_seen_at = now;
        session.clone()
      })
    });
    let owner = session_id.map(str::to_string);
    // Consecutive reports from one agent update a single line.
    let reuse = state
      .thread
      .iter()
      .rposition(|item| item.session_id == owner && item.kind != ThreadKind::Note)
      .filter(|&index| state.thread[index].kind == ThreadKind::Progress);
    let item = match reuse {
      Some(index) => {
        let item = &mut state.thread[index];
        item.at = now;
        item.text = message;
        item.done = done;
        item.total = total;
        item.profile_id = profile_id;
        item.clone()
      }
      None => {
        let mut item = thread_item(ThreadKind::Progress, now);
        item.id = state.next_id();
        item.session_id = owner;
        item.text = message;
        item.done = done;
        item.total = total;
        item.profile_id = profile_id;
        state.push_thread(item)
      }
    };
    (session, item)
  };
  if let Some(session) = session {
    emit(EVENT_SESSION, &session);
  }
  emit(EVENT_THREAD, &item);
}

/// How many of a session's latest calls go with its feedback.
const FEEDBACK_RECENT_CALLS: usize = 20;

/// What feedback from this session carries about the agent: its client name
/// and version, and its latest calls (tool, error code and time only, never
/// the arguments).
pub fn feedback_context(session_id: Option<&str>) -> (Option<String>, Vec<serde_json::Value>) {
  let state = console();
  let client = session_id
    .and_then(|sid| state.sessions.get(sid))
    .and_then(|session| {
      let name = session.client_name.as_deref()?;
      Some(match session.client_version.as_deref() {
        Some(version) => format!("{name} {version}"),
        None => name.to_string(),
      })
    });
  let owner = session_id.map(str::to_string);
  let mut calls: Vec<serde_json::Value> = state
    .activity
    .iter()
    .rev()
    .filter(|entry| entry.session_id == owner)
    .take(FEEDBACK_RECENT_CALLS)
    .map(|entry| {
      serde_json::json!({
        "tool": entry.tool,
        "errorCode": entry.error_code,
        "durationMs": entry.duration_ms,
        "at": entry.at,
      })
    })
    .collect();
  calls.reverse();
  (client, calls)
}

/// Show the person that an agent sent feedback to the Donut team.
pub fn feedback_sent(session_id: Option<&str>, kind: &str, message: &str, logs: bool) {
  let now = now_ms();
  let item = {
    let mut state = console();
    let mut item = thread_item(ThreadKind::Feedback, now);
    item.id = state.next_id();
    item.session_id = session_id.map(str::to_string);
    item.text = message.chars().take(MAX_TEXT_CHARS).collect();
    item.feedback = Some(ThreadFeedback {
      kind: kind.to_string(),
      logs,
    });
    if let Some(session) = session_id.and_then(|sid| state.sessions.get_mut(sid)) {
      session.last_seen_at = now;
    }
    state.push_thread(item)
  };
  emit(EVENT_THREAD, &item);
}

/// Everything an agent should know before its next step.
pub fn human_updates(session_id: Option<&str>) -> serde_json::Value {
  let notes = take_notes(session_id);
  let mut state = console();
  let owner = session_id.map(str::to_string);
  let mut answers = Vec::new();
  for item in state.thread.iter_mut() {
    if item.session_id != owner
      || !matches!(item.kind, ThreadKind::Question | ThreadKind::Help)
      || item.outcome_seen
    {
      continue;
    }
    let outcome = outcome_of(item);
    if outcome == RequestOutcome::Pending {
      continue;
    }
    item.outcome_seen = true;
    answers.push(outcome_json(item.id, &outcome));
  }
  let open: Vec<u64> = state
    .thread
    .iter()
    .filter(|item| item.session_id == owner && item.state == ThreadState::Open)
    .map(|item| item.id)
    .collect();
  serde_json::json!({
    "paused": state.paused.as_ref().map(|pause| serde_json::json!({ "note": pause.note })),
    "held_profiles": state.holds_sorted().into_iter().map(|hold| serde_json::json!({
      "profile_id": hold.profile_id,
      "note": hold.note,
    })).collect::<Vec<_>>(),
    "notes": notes.iter().map(|note| serde_json::json!({
      "id": note.id,
      "text": note.text,
      "profile_id": note.profile_id,
    })).collect::<Vec<_>>(),
    "answers": answers,
    "open_request_ids": open,
  })
}

pub fn outcome_json(request_id: u64, outcome: &RequestOutcome) -> serde_json::Value {
  match outcome {
    RequestOutcome::Pending => serde_json::json!({
      "status": "pending",
      "request_id": request_id,
      "next": "Not answered yet. Call wait_for_human with this request_id to keep waiting, or continue with other work and check get_human_updates later.",
    }),
    RequestOutcome::Answered(answer) => serde_json::json!({
      "status": "answered",
      "request_id": request_id,
      "answer": answer,
    }),
    RequestOutcome::Done(note) => serde_json::json!({
      "status": "done",
      "request_id": request_id,
      "note": note,
      "next": "The person handed the profile back. Re-read the page before you continue.",
    }),
    RequestOutcome::Dismissed => serde_json::json!({
      "status": "dismissed",
      "request_id": request_id,
      "next": "The person dismissed this request. Do not ask the same thing again; choose another approach or skip this item.",
    }),
  }
}

pub fn reset() {
  {
    let mut state = console();
    *state = Console::default();
  }
  notify_waiters();
  let _ = crate::events::emit_empty(EVENT_CLEARED);
}

pub async fn snapshot() -> AgentConsoleSnapshot {
  let quota = crate::automation_rate_limiter::automation_quota().await;
  let state = console();
  let mut sessions: Vec<AgentSession> = state.sessions.values().cloned().collect();
  sessions.sort_by_key(|session| std::cmp::Reverse(session.last_seen_at));
  sessions.truncate(SNAPSHOT_SESSIONS);
  AgentConsoleSnapshot {
    sessions,
    activity: state.activity.iter().cloned().collect(),
    thread: state.thread.iter().cloned().collect(),
    holds: state.holds_sorted(),
    paused: state.paused.clone(),
    quota,
  }
}

// --- Human side ------------------------------------------------------------

fn profile_exists(profile_id: &str) -> Result<(), String> {
  let profiles = crate::profile::manager::ProfileManager::instance()
    .list_profiles()
    .map_err(|e| {
      log::warn!("Agent console: profile list failed err=\"{e}\"");
      crate::backend_error("INTERNAL_ERROR")
    })?;
  if profiles.iter().any(|p| p.id.to_string() == profile_id) {
    Ok(())
  } else {
    Err(crate::backend_error("PROFILE_NOT_FOUND"))
  }
}

async fn settle(
  request_id: u64,
  state_to: ThreadState,
  answer: Option<String>,
) -> Result<AgentThreadItem, String> {
  let now = now_ms();
  let (item, released) = {
    let mut state = console();
    let item = state
      .find_thread_mut(request_id)
      .filter(|item| matches!(item.kind, ThreadKind::Question | ThreadKind::Help))
      .ok_or_else(|| crate::backend_error("AGENT_REQUEST_NOT_FOUND"))?;
    if item.state != ThreadState::Open {
      return Err(crate::backend_error("AGENT_REQUEST_CLOSED"));
    }
    item.state = state_to;
    item.answer = answer;
    item.answered_at = Some(now);
    let item = item.clone();
    let mut released = false;
    if item.kind == ThreadKind::Help {
      if let Some(profile_id) = &item.profile_id {
        if state
          .holds
          .get(profile_id)
          .is_some_and(|hold| hold.request_id == Some(request_id))
        {
          state.holds.remove(profile_id);
          released = true;
        }
      }
    }
    (item, released)
  };
  notify_waiters();
  emit(EVENT_THREAD, &item);
  if released {
    emit_state(true).await;
  }
  Ok(item)
}

#[tauri::command]
pub async fn get_agent_console() -> Result<AgentConsoleSnapshot, String> {
  Ok(snapshot().await)
}

#[tauri::command]
pub async fn answer_agent_request(
  request_id: u64,
  answer: String,
) -> Result<AgentThreadItem, String> {
  let answer = clean_text(&answer, MAX_TEXT_CHARS)?;
  let is_help = console()
    .thread
    .iter()
    .any(|item| item.id == request_id && item.kind == ThreadKind::Help);
  if answer.is_empty() && !is_help {
    return Err(crate::backend_error("AGENT_ANSWER_EMPTY"));
  }
  settle(
    request_id,
    ThreadState::Answered,
    (!answer.is_empty()).then_some(answer),
  )
  .await
}

#[tauri::command]
pub async fn dismiss_agent_request(request_id: u64) -> Result<AgentThreadItem, String> {
  settle(request_id, ThreadState::Dismissed, None).await
}

#[tauri::command]
pub async fn send_agent_note(
  text: String,
  session_id: Option<String>,
  profile_id: Option<String>,
) -> Result<AgentThreadItem, String> {
  let text = clean_text(&text, MAX_TEXT_CHARS)?;
  if text.is_empty() {
    return Err(crate::backend_error("AGENT_NOTE_EMPTY"));
  }
  if let Some(profile_id) = &profile_id {
    profile_exists(profile_id)?;
  }
  let item = {
    let mut state = console();
    if let Some(sid) = &session_id {
      if !state.sessions.contains_key(sid) {
        return Err(crate::backend_error("AGENT_SESSION_NOT_FOUND"));
      }
    }
    let mut item = thread_item(ThreadKind::Note, now_ms());
    item.id = state.next_id();
    item.session_id = session_id;
    item.text = text;
    item.profile_id = profile_id;
    item.state = ThreadState::Pending;
    state.push_thread(item)
  };
  emit(EVENT_THREAD, &item);
  Ok(item)
}

#[tauri::command]
pub async fn take_over_profile(
  profile_id: String,
  note: Option<String>,
) -> Result<AgentHold, String> {
  let note = optional_text(note)?;
  profile_exists(&profile_id)?;
  let hold = {
    let mut state = console();
    let hold = state
      .holds
      .entry(profile_id.clone())
      .and_modify(|hold| {
        if note.is_some() {
          hold.note = note.clone();
        }
      })
      .or_insert_with(|| AgentHold {
        profile_id: profile_id.clone(),
        since: now_ms(),
        note: note.clone(),
        request_id: None,
      });
    hold.clone()
  };
  emit_state(true).await;
  Ok(hold)
}

#[tauri::command]
pub async fn hand_back_profile(profile_id: String, note: Option<String>) -> Result<(), String> {
  let note = optional_text(note)?;
  let removed = console().holds.remove(&profile_id);
  let Some(hold) = removed else {
    return Ok(());
  };
  if let Some(request_id) = hold.request_id {
    let still_open = console()
      .thread
      .iter()
      .any(|item| item.id == request_id && item.state == ThreadState::Open);
    if still_open {
      settle(request_id, ThreadState::Answered, note.clone()).await?;
    }
  } else if let Some(text) = note {
    let item = {
      let mut state = console();
      let mut item = thread_item(ThreadKind::Note, now_ms());
      item.id = state.next_id();
      item.text = text;
      item.profile_id = Some(profile_id.clone());
      item.state = ThreadState::Pending;
      state.push_thread(item)
    };
    emit(EVENT_THREAD, &item);
  }
  notify_waiters();
  emit_state(true).await;
  Ok(())
}

#[tauri::command]
pub async fn set_agents_paused(
  paused: bool,
  note: Option<String>,
) -> Result<Option<AgentsPause>, String> {
  let note = optional_text(note)?;
  let pause = {
    let mut state = console();
    state.paused = paused.then(|| AgentsPause {
      since: now_ms(),
      note,
    });
    state.paused.clone()
  };
  notify_waiters();
  emit_state(true).await;
  Ok(pause)
}

#[tauri::command]
pub async fn clear_agent_activity() -> Result<(), String> {
  console().activity.clear();
  let _ = crate::events::emit_empty(EVENT_CLEARED);
  Ok(())
}

#[tauri::command]
pub async fn show_profile_window(profile_id: String) -> Result<(), String> {
  profile_exists(&profile_id)?;
  crate::mcp_server::McpServer::instance()
    .bring_profile_to_front(&profile_id)
    .await
}

#[cfg(test)]
mod tests {
  use super::*;

  fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
  }

  #[test]
  fn details_never_carry_text_or_full_urls() {
    assert_eq!(
      safe_detail(
        "navigate",
        &serde_json::json!({"url": "https://shop.example.com/login?token=secret"})
      )
      .as_deref(),
      Some("shop.example.com")
    );
    assert_eq!(
      safe_detail("type_text", &serde_json::json!({"text": "hunter2"})).as_deref(),
      Some("7")
    );
    assert_eq!(
      safe_detail("evaluate_javascript", &serde_json::json!({"script": "1"})),
      None
    );
    assert_eq!(
      safe_detail("navigate", &serde_json::json!({"url": "not a url"})),
      None
    );
  }

  #[test]
  fn every_profile_argument_counts_as_a_target() {
    let targets = targeted_profiles(&serde_json::json!({
      "profile_id": "a",
      "profile_ids": ["b", "c"],
      "leader_profile_id": "d",
      "follower_profile_ids": ["e"],
    }));
    assert_eq!(targets, vec!["a", "d", "b", "c", "e"]);
  }

  #[test]
  fn questions_and_choices_are_bounded() {
    assert!(validate_question("  ", &[]).is_err());
    assert!(validate_question(&"a".repeat(MAX_TEXT_CHARS + 1), &[]).is_err());
    assert!(validate_question("ok?", &vec!["x".to_string(); MAX_CHOICES + 1]).is_err());
    assert!(validate_question("ok?", &[" ".to_string()]).is_err());
    let (question, choices) = validate_question(" Which one? ", &[" A ".to_string()]).unwrap();
    assert_eq!(question, "Which one?");
    assert_eq!(choices, vec!["A"]);
  }

  #[test]
  fn a_pause_refuses_active_calls_but_not_passive_ones() {
    let mut state = Console {
      paused: Some(AgentsPause {
        since: 1,
        note: Some("lunch".to_string()),
      }),
      ..Default::default()
    };
    assert_eq!(
      refusal_for(&state, false, &serde_json::json!({})),
      Err(Refusal::Paused {
        note: Some("lunch".to_string())
      })
    );
    assert!(refusal_for(&state, true, &serde_json::json!({})).is_ok());
    state.paused = None;
    state.holds.insert(
      "p1".to_string(),
      AgentHold {
        profile_id: "p1".to_string(),
        since: 1,
        note: None,
        request_id: None,
      },
    );
    assert!(matches!(
      refusal_for(&state, false, &serde_json::json!({"profile_ids": ["p2", "p1"]})),
      Err(Refusal::ProfileHeld { profile_id, .. }) if profile_id == "p1"
    ));
    assert!(refusal_for(&state, false, &serde_json::json!({"profile_id": "p2"})).is_ok());
  }

  #[tokio::test]
  async fn a_help_request_holds_the_profile_until_the_person_answers() {
    let session = unique("session");
    let profile = unique("profile");
    session_started(
      &session,
      Some("Claude Code".to_string()),
      Some("2.1".to_string()),
    );
    let id = open_request(NewRequest {
      session_id: Some(session.clone()),
      kind: ThreadKind::Help,
      text: "Solve the captcha".to_string(),
      profile_id: Some(profile.clone()),
      choices: Vec::new(),
    })
    .await;
    assert!(matches!(
      check_call(false, &serde_json::json!({"profile_id": profile})),
      Err(Refusal::ProfileHeld { .. })
    ));
    assert_eq!(wait_for(id, 0).await, Some(RequestOutcome::Pending));

    let waiter = tokio::spawn(wait_for(id, 5));
    tokio::time::sleep(Duration::from_millis(50)).await;
    answer_agent_request(id, "done, you are in".to_string())
      .await
      .unwrap();
    assert_eq!(
      waiter.await.unwrap(),
      Some(RequestOutcome::Done(Some("done, you are in".to_string())))
    );
    assert!(!console().holds.contains_key(&profile));
    assert_eq!(
      answer_agent_request(id, "again".to_string())
        .await
        .unwrap_err(),
      crate::backend_error("AGENT_REQUEST_CLOSED")
    );
    let updates = human_updates(Some(&session));
    assert_eq!(updates["answers"], serde_json::json!([]));
  }

  #[tokio::test]
  async fn a_question_needs_an_answer_and_can_be_dismissed() {
    let session = unique("session");
    session_started(&session, None, None);
    let id = open_request(NewRequest {
      session_id: Some(session.clone()),
      kind: ThreadKind::Question,
      text: "Which account?".to_string(),
      profile_id: None,
      choices: vec!["A".to_string(), "B".to_string()],
    })
    .await;
    assert_eq!(
      answer_agent_request(id, "  ".to_string())
        .await
        .unwrap_err(),
      crate::backend_error("AGENT_ANSWER_EMPTY")
    );
    dismiss_agent_request(id).await.unwrap();
    let updates = human_updates(Some(&session));
    assert_eq!(updates["answers"][0]["status"], "dismissed");
    assert_eq!(wait_for(id, 1).await, Some(RequestOutcome::Dismissed));
    assert_eq!(
      dismiss_agent_request(u64::MAX).await.unwrap_err(),
      crate::backend_error("AGENT_REQUEST_NOT_FOUND")
    );
  }

  #[tokio::test]
  async fn notes_reach_their_agent_once() {
    let first = unique("session");
    let second = unique("session");
    session_started(&first, None, None);
    session_started(&second, None, None);
    let direct = send_agent_note("only first".to_string(), Some(first.clone()), None)
      .await
      .unwrap();
    let broadcast = send_agent_note("everyone".to_string(), None, None)
      .await
      .unwrap();
    let ours = |notes: Vec<AgentThreadItem>| -> Vec<u64> {
      notes
        .into_iter()
        .map(|n| n.id)
        .filter(|id| *id == direct.id || *id == broadcast.id)
        .collect()
    };
    assert_eq!(
      ours(take_notes(Some(&first))),
      vec![direct.id, broadcast.id]
    );
    assert!(ours(take_notes(Some(&first))).is_empty());
    assert_eq!(ours(take_notes(Some(&second))), vec![broadcast.id]);
    assert!(take_notes(None).is_empty());
    assert_eq!(
      send_agent_note("x".to_string(), Some(unique("missing")), None)
        .await
        .unwrap_err(),
      crate::backend_error("AGENT_SESSION_NOT_FOUND")
    );
    assert_eq!(
      send_agent_note(" ".to_string(), None, None)
        .await
        .unwrap_err(),
      crate::backend_error("AGENT_NOTE_EMPTY")
    );
  }

  #[tokio::test]
  async fn progress_reports_update_one_line_per_agent() {
    let session = unique("session");
    session_started(&session, None, None);
    report_progress(Some(&session), "Checking", Some(1), Some(10), None);
    report_progress(Some(&session), "Checking", Some(2), Some(10), None);
    let state = console();
    let progress: Vec<_> = state
      .thread
      .iter()
      .filter(|item| {
        item.kind == ThreadKind::Progress && item.session_id.as_deref() == Some(&session)
      })
      .collect();
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].done, Some(2));
    assert_eq!(
      state.sessions[&session]
        .status
        .as_ref()
        .and_then(|s| s.done),
      Some(2)
    );
  }

  #[tokio::test]
  async fn the_first_call_marks_a_session_as_joined_and_counts_errors() {
    let session = unique("session");
    session_started(&session, Some("Cursor".to_string()), None);
    let arguments = serde_json::json!({"profile_id": "p1", "url": "https://example.com/a"});
    record_call(CallRecord {
      session_id: Some(&session),
      tool: "navigate",
      arguments: &arguments,
      error_code: None,
      duration_ms: 12,
    })
    .await;
    record_call(CallRecord {
      session_id: Some(&session),
      tool: "click_locator",
      arguments: &arguments,
      error_code: Some("LOCATOR_NO_MATCH".to_string()),
      duration_ms: 30,
    })
    .await;
    session_ended(&session);
    let state = console();
    let activity: Vec<_> = state
      .activity
      .iter()
      .filter(|entry| entry.session_id.as_deref() == Some(&session))
      .collect();
    assert_eq!(activity.len(), 2);
    assert_eq!(activity[0].detail.as_deref(), Some("example.com"));
    let record = &state.sessions[&session];
    assert_eq!((record.calls, record.errors, record.ended), (2, 1, true));
    let kinds: Vec<ThreadKind> = state
      .thread
      .iter()
      .filter(|item| item.session_id.as_deref() == Some(&session))
      .map(|item| item.kind)
      .collect();
    assert_eq!(kinds, vec![ThreadKind::Joined, ThreadKind::Left]);
  }
}

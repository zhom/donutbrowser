//! Feedback to the Donut team, from the person (the feedback dialog) or from an
//! agent (the `send_feedback` MCP tool). The cloud stores it; anyone may send
//! it, signed in or not. A signed-in sender is named by their access token, so
//! a reply can reach them.

use crate::cloud_errors;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const MAX_MESSAGE_CHARS: usize = 5000;
/// The cloud refuses bodies over 1 MiB; this leaves room for JSON escaping.
pub const MAX_LOG_BYTES: usize = 512 * 1024;
const MAX_EMAIL_CHARS: usize = 254;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// An agent caught in a loop must not flood the team's inbox.
const AGENT_SENDS_PER_HOUR: usize = 10;
const TRANSPORT_PREFIX: &str = "reach backend: ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
  Bug,
  Idea,
  Praise,
  Other,
}

impl FeedbackKind {
  pub fn parse(value: &str) -> Option<Self> {
    match value {
      "bug" => Some(Self::Bug),
      "idea" => Some(Self::Idea),
      "praise" => Some(Self::Praise),
      "other" => Some(Self::Other),
      _ => None,
    }
  }

  pub fn as_str(self) -> &'static str {
    match self {
      Self::Bug => "bug",
      Self::Idea => "idea",
      Self::Praise => "praise",
      Self::Other => "other",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSource {
  Person,
  Agent,
}

impl FeedbackSource {
  fn as_str(self) -> &'static str {
    match self {
      Self::Person => "person",
      Self::Agent => "agent",
    }
  }
}

pub struct Feedback {
  pub kind: FeedbackKind,
  pub source: FeedbackSource,
  pub message: String,
  /// Where to reply, for a sender who is not signed in.
  pub email: Option<String>,
  pub locale: Option<String>,
  pub logs: Option<String>,
  pub agent_client: Option<String>,
  pub context: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
  kind: FeedbackKind,
  source: FeedbackSource,
  message: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  email: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  logs: Option<&'a str>,
  app_version: String,
  os: String,
  arch: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  locale: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  agent_client: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  context: Option<&'a serde_json::Value>,
}

impl<'a> Payload<'a> {
  fn of(feedback: &'a Feedback) -> Self {
    let info = crate::settings_manager::get_system_info();
    Self {
      kind: feedback.kind,
      source: feedback.source,
      message: &feedback.message,
      email: feedback.email.as_deref(),
      logs: feedback.logs.as_deref().filter(|logs| !logs.is_empty()),
      app_version: info.app_version,
      os: info.os,
      arch: info.arch,
      locale: feedback.locale.as_deref(),
      agent_client: feedback.agent_client.as_deref(),
      context: feedback.context.as_ref(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackReceipt {
  pub id: String,
}

/// The message as it is sent: trimmed, not empty, not too long.
pub fn validate_message(message: &str) -> Result<String, String> {
  let message = message.trim();
  if message.is_empty() {
    return Err(crate::backend_error("FEEDBACK_MESSAGE_EMPTY"));
  }
  if message.chars().count() > MAX_MESSAGE_CHARS {
    return Err(
      serde_json::json!({
        "code": "FEEDBACK_MESSAGE_TOO_LONG",
        "params": { "max": MAX_MESSAGE_CHARS.to_string() }
      })
      .to_string(),
    );
  }
  Ok(message.to_string())
}

/// A reply address: blank is none, anything else must look like an address.
pub fn validate_email(email: Option<&str>) -> Result<Option<String>, String> {
  let Some(email) = email.map(str::trim).filter(|email| !email.is_empty()) else {
    return Ok(None);
  };
  let plausible = email.len() <= MAX_EMAIL_CHARS
    && !email.chars().any(char::is_whitespace)
    && email.split_once('@').is_some_and(|(local, domain)| {
      !local.is_empty()
        && !domain.contains('@')
        && domain
          .split_once('.')
          .is_some_and(|(host, tld)| !host.is_empty() && !tld.is_empty() && !tld.ends_with('.'))
    });
  if plausible {
    Ok(Some(email.to_string()))
  } else {
    Err(crate::backend_error("FEEDBACK_EMAIL_INVALID"))
  }
}

/// The log excerpt feedback carries: the newest lines, redacted, at most
/// `MAX_LOG_BYTES`. Empty when the app has not written a log yet.
pub async fn log_excerpt(dir: PathBuf) -> Result<String, String> {
  tokio::task::spawn_blocking(move || {
    if !dir.exists() {
      return Ok(String::new());
    }
    crate::settings_manager::recent_logs(&dir, MAX_LOG_BYTES)
  })
  .await
  .map_err(|e| crate::backend_error_with_detail("FEEDBACK_LOGS_UNREADABLE", e))?
  .map_err(|e| crate::backend_error_with_detail("FEEDBACK_LOGS_UNREADABLE", e))
}

/// Whether an agent may send one more piece of feedback this hour. Counts the
/// attempt when it may.
pub fn agent_send_allowed() -> bool {
  static SENDS: Mutex<VecDeque<Instant>> = Mutex::new(VecDeque::new());
  let mut sends = SENDS
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  allow_send(&mut sends, Instant::now(), AGENT_SENDS_PER_HOUR)
}

fn allow_send(sends: &mut VecDeque<Instant>, now: Instant, per_hour: usize) -> bool {
  let hour = Duration::from_secs(3600);
  while sends
    .front()
    .is_some_and(|at| now.saturating_duration_since(*at) >= hour)
  {
    sends.pop_front();
  }
  if sends.len() >= per_hour {
    return false;
  }
  sends.push_back(now);
  true
}

/// Send feedback to the cloud. A signed-in sender's access token goes with it;
/// when the session cannot be used, the feedback is sent without one rather
/// than lost.
pub async fn send(feedback: Feedback) -> Result<FeedbackReceipt, String> {
  let body = serde_json::to_vec(&Payload::of(&feedback))
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;
  let logs_bytes = feedback.logs.as_ref().map_or(0, String::len);
  let url = endpoint();
  let started = Instant::now();

  let auth = &crate::cloud_auth::CLOUD_AUTH;
  let result = if auth.is_logged_in().await {
    match auth
      .api_call_with_retry(|token| post(url.clone(), body.clone(), Some(token)))
      .await
    {
      Err(error) if retry_without_account(&error) => post(url, body, None).await,
      other => other,
    }
  } else {
    post(url, body, None).await
  };

  let elapsed_ms = started.elapsed().as_millis();
  match result {
    Ok(receipt) => {
      log::info!(
        "Feedback sent id={} source={} kind={} logs_bytes={logs_bytes} elapsed_ms={elapsed_ms}",
        receipt.id,
        feedback.source.as_str(),
        feedback.kind.as_str(),
      );
      Ok(receipt)
    }
    Err(error) => {
      let coded = failure_code(&error);
      log::warn!(
        "Feedback send failed source={} kind={} err={coded} elapsed_ms={elapsed_ms}",
        feedback.source.as_str(),
        feedback.kind.as_str(),
      );
      Err(coded)
    }
  }
}

fn endpoint() -> String {
  // A test build never reaches the production API: it posts to the suite's
  // capture server, or to a port where nothing listens.
  #[cfg(feature = "e2e")]
  {
    std::env::var("DONUT_E2E_FEEDBACK_URL")
      .ok()
      .filter(|url| !url.is_empty())
      .unwrap_or_else(|| "http://127.0.0.1:9/api/feedback".to_string())
  }
  #[cfg(not(feature = "e2e"))]
  {
    format!("{}/api/feedback", crate::cloud_auth::CLOUD_API_URL)
  }
}

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

async fn post(
  url: String,
  body: Vec<u8>,
  token: Option<String>,
) -> Result<FeedbackReceipt, String> {
  let mut request = http()
    .post(&url)
    .header(reqwest::header::CONTENT_TYPE, "application/json")
    .body(body);
  if let Some(token) = token {
    request = request.bearer_auth(token);
  }
  let response = request.send().await.map_err(|e| {
    crate::system_proxy::explain(&e).unwrap_or_else(|| format!("{TRANSPORT_PREFIX}{e}"))
  })?;
  let status = response.status().as_u16();
  if !(200..300).contains(&status) {
    let text = response.text().await.unwrap_or_default();
    // api_call_with_retry and cloud_errors::split_status read the status from this prefix.
    return Err(format!("({status}) {text}"));
  }
  response
    .json::<FeedbackReceipt>()
    .await
    .map_err(|e| format!("({status}) decode response: {e}"))
}

/// The account could not be used: its token was refused, or it could not be
/// refreshed before anything was sent.
fn retry_without_account(error: &str) -> bool {
  match cloud_errors::split_status(error) {
    Some((status, _)) => status == 401,
    None => {
      !error.starts_with(TRANSPORT_PREFIX) && !crate::system_proxy::is_unreachable_error(error)
    }
  }
}

fn failure_code(error: &str) -> String {
  if crate::system_proxy::is_unreachable_error(error) {
    return error.to_string();
  }
  match cloud_errors::split_status(error) {
    Some((429, _)) => crate::backend_error("FEEDBACK_RATE_LIMITED"),
    Some((400 | 413 | 422, body)) if body.contains("FEEDBACK_MESSAGE_EMPTY") => {
      crate::backend_error("FEEDBACK_MESSAGE_EMPTY")
    }
    Some((400 | 413 | 422, _)) => crate::backend_error("FEEDBACK_INVALID"),
    Some(_) => crate::backend_error("FEEDBACK_SEND_FAILED"),
    None => crate::backend_error(cloud_errors::UNREACHABLE),
  }
}

/// Send the person's feedback from the feedback dialog.
#[tauri::command]
pub async fn send_feedback(
  app_handle: tauri::AppHandle,
  kind: String,
  message: String,
  include_logs: bool,
  email: Option<String>,
  locale: Option<String>,
) -> Result<FeedbackReceipt, String> {
  let kind = FeedbackKind::parse(&kind).ok_or_else(|| crate::backend_error("FEEDBACK_INVALID"))?;
  let message = validate_message(&message)?;
  let email = validate_email(email.as_deref())?;
  let logs = if include_logs {
    Some(log_excerpt(crate::app_dirs::log_dir(&app_handle)).await?)
  } else {
    None
  };
  let receipt = send(Feedback {
    kind,
    source: FeedbackSource::Person,
    message,
    email,
    locale: locale.filter(|locale| !locale.trim().is_empty()),
    logs,
    agent_client: None,
    context: None,
  })
  .await?;
  tauri::async_runtime::spawn_blocking(crate::settings_manager::note_feedback_sent);
  Ok(receipt)
}

/// Exactly the log excerpt `send_feedback` would attach, so the person can read
/// it before sending.
#[tauri::command]
pub async fn preview_feedback_logs(app_handle: tauri::AppHandle) -> Result<String, String> {
  log_excerpt(crate::app_dirs::log_dir(&app_handle)).await
}

#[cfg(test)]
mod tests {
  use super::*;
  use wiremock::matchers::{header, header_exists, method, path};
  use wiremock::{Mock, MockServer, ResponseTemplate};

  fn code_of(error: &str) -> String {
    serde_json::from_str::<serde_json::Value>(error).unwrap()["code"]
      .as_str()
      .unwrap()
      .to_string()
  }

  #[test]
  fn kinds_round_trip_and_unknown_ones_are_refused() {
    for kind in ["bug", "idea", "praise", "other"] {
      assert_eq!(FeedbackKind::parse(kind).unwrap().as_str(), kind);
    }
    assert_eq!(FeedbackKind::parse("Bug"), None);
    assert_eq!(FeedbackKind::parse(""), None);
  }

  #[test]
  fn a_message_is_trimmed_and_bounded() {
    assert_eq!(validate_message("  Hello \n").unwrap(), "Hello");
    assert_eq!(
      code_of(&validate_message(" \n\t ").unwrap_err()),
      "FEEDBACK_MESSAGE_EMPTY"
    );
    let at_limit = "é".repeat(MAX_MESSAGE_CHARS);
    assert_eq!(validate_message(&at_limit).unwrap(), at_limit);
    let over = validate_message(&"a".repeat(MAX_MESSAGE_CHARS + 1)).unwrap_err();
    assert_eq!(code_of(&over), "FEEDBACK_MESSAGE_TOO_LONG");
  }

  #[test]
  fn a_reply_address_is_optional_but_must_be_plausible() {
    assert_eq!(validate_email(None).unwrap(), None);
    assert_eq!(validate_email(Some("   ")).unwrap(), None);
    assert_eq!(
      validate_email(Some(" me@example.com ")).unwrap().as_deref(),
      Some("me@example.com")
    );
    for bad in [
      "me",
      "me@",
      "@example.com",
      "me@example",
      "a b@example.com",
      "a@b@c.com",
      "me@example.",
    ] {
      assert_eq!(
        code_of(&validate_email(Some(bad)).unwrap_err()),
        "FEEDBACK_EMAIL_INVALID",
        "{bad}"
      );
    }
  }

  #[test]
  fn an_agent_may_send_a_bounded_amount_per_hour() {
    let mut sends = VecDeque::new();
    let start = Instant::now();
    for _ in 0..3 {
      assert!(allow_send(&mut sends, start, 3));
    }
    assert!(!allow_send(&mut sends, start + Duration::from_secs(60), 3));
    assert!(allow_send(&mut sends, start + Duration::from_secs(3600), 3));
  }

  #[test]
  fn failures_become_codes_the_dialog_can_explain() {
    assert_eq!(
      code_of(&failure_code("(429) slow down")),
      "FEEDBACK_RATE_LIMITED"
    );
    assert_eq!(
      code_of(&failure_code(r#"(400) {"code":"FEEDBACK_MESSAGE_EMPTY"}"#)),
      "FEEDBACK_MESSAGE_EMPTY"
    );
    assert_eq!(code_of(&failure_code("(400) bad")), "FEEDBACK_INVALID");
    assert_eq!(
      code_of(&failure_code("(502) gateway")),
      "FEEDBACK_SEND_FAILED"
    );
    assert_eq!(
      code_of(&failure_code("reach backend: connection refused")),
      cloud_errors::UNREACHABLE
    );
    let proxied = crate::system_proxy::unreachable_error("127.0.0.1:8888");
    assert_eq!(failure_code(&proxied), proxied);
  }

  #[test]
  fn only_an_unusable_account_is_retried_without_it() {
    assert!(retry_without_account("(401) expired"));
    assert!(retry_without_account("Not logged in"));
    assert!(!retry_without_account("(429) slow down"));
    assert!(!retry_without_account("(500) broken"));
    assert!(!retry_without_account("reach backend: timed out"));
    assert!(!retry_without_account(
      &crate::system_proxy::unreachable_error("127.0.0.1:8888")
    ));
  }

  #[tokio::test]
  async fn the_payload_carries_the_app_and_the_bearer_when_there_is_one() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
      .and(path("/api/feedback"))
      .and(header("authorization", "Bearer t0ken"))
      .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
        "id": "f-1", "createdAt": "2026-10-09T00:00:00.000Z"
      })))
      .expect(1)
      .mount(&server)
      .await;
    let feedback = Feedback {
      kind: FeedbackKind::Bug,
      source: FeedbackSource::Agent,
      message: "navigate times out".to_string(),
      email: None,
      locale: Some("en".to_string()),
      logs: Some("line\n".to_string()),
      agent_client: Some("Claude Code 2.1.0".to_string()),
      context: Some(serde_json::json!({ "recentCalls": [] })),
    };
    let body = serde_json::to_vec(&Payload::of(&feedback)).unwrap();
    let receipt = post(
      format!("{}/api/feedback", server.uri()),
      body,
      Some("t0ken".to_string()),
    )
    .await
    .unwrap();
    assert_eq!(receipt.id, "f-1");

    let sent: serde_json::Value =
      serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(sent["kind"], "bug");
    assert_eq!(sent["source"], "agent");
    assert_eq!(sent["logs"], "line\n");
    assert_eq!(sent["agentClient"], "Claude Code 2.1.0");
    assert_eq!(sent["locale"], "en");
    assert!(sent["appVersion"].is_string() && sent["os"].is_string() && sent["arch"].is_string());
    assert!(sent.get("email").is_none(), "{sent}");
  }

  #[tokio::test]
  async fn an_anonymous_post_has_no_bearer_and_a_refusal_keeps_its_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
      .and(path("/api/feedback"))
      .and(header_exists("content-type"))
      .respond_with(ResponseTemplate::new(429))
      .mount(&server)
      .await;
    let feedback = Feedback {
      kind: FeedbackKind::Idea,
      source: FeedbackSource::Person,
      message: "More themes".to_string(),
      email: Some("me@example.com".to_string()),
      locale: None,
      logs: Some(String::new()),
      agent_client: None,
      context: None,
    };
    let body = serde_json::to_vec(&Payload::of(&feedback)).unwrap();
    let error = post(format!("{}/api/feedback", server.uri()), body, None)
      .await
      .unwrap_err();
    assert_eq!(code_of(&failure_code(&error)), "FEEDBACK_RATE_LIMITED");

    let request = &server.received_requests().await.unwrap()[0];
    assert!(request.headers.get("authorization").is_none());
    let sent: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(sent["email"], "me@example.com");
    assert!(
      sent.get("logs").is_none(),
      "empty logs are not sent: {sent}"
    );
  }
}

//! Scheduled automation: recipes Donut cloud runs on a calendar, on this
//! desktop or on the remote fleet. Solo, Pro, Team and Enterprise.
//!
//! The cloud stores the schedules, decides when they fire, and checks the
//! plan on every save and again at every fire, so a schedule can never run
//! on a plan that does not include it, whatever this side does. This client
//! only passes the requests on and turns failures into codes the app
//! translates.

use crate::cloud_errors::{BackendFailure, FailureCodes};
use serde_json::Value;

const SCHEDULE_CODES: FailureCodes = FailureCodes {
  bad_request: "AGENT_SCHEDULE_INVALID",
  forbidden: "SCHEDULED_AUTOMATION_NOT_ENTITLED",
  not_found: "AGENT_SCHEDULE_NOT_FOUND",
  conflict: "AGENT_SCHEDULE_INVALID",
};

fn base() -> String {
  format!("{}/api/agent/schedules", crate::cloud_auth::CLOUD_API_URL)
}

fn url_for(id: &str, tail: &str) -> Result<String, String> {
  let id = id.trim();
  if id.is_empty() || id.len() > 128 {
    return Err(crate::backend_error("AGENT_SCHEDULE_NOT_FOUND"));
  }
  Ok(format!("{}/{}{tail}", base(), urlencoding::encode(id)))
}

fn failure(action: &str, mut err: BackendFailure) -> String {
  log::warn!(
    "Schedule request failed action={action} code={} status={}",
    err.code,
    err.status
  );
  // The cloud names a schedule a job; the app has one word for it.
  if err.code == "AGENT_JOB_NOT_FOUND" {
    err.code = "AGENT_SCHEDULE_NOT_FOUND".to_string();
  }
  err.to_error_json()
}

/// A schedule as the app sends it: an object, never anything else.
fn checked_body(body: Value) -> Result<Value, String> {
  if body.is_object() {
    Ok(body)
  } else {
    Err(crate::backend_error("AGENT_SCHEDULE_INVALID"))
  }
}

async fn call(
  action: &str,
  method: reqwest::Method,
  url: String,
  body: Option<Value>,
) -> Result<Value, String> {
  crate::cloud_api::call::<Option<Value>>(method, url, body, SCHEDULE_CODES)
    .await
    .map(|value| value.unwrap_or(Value::Null))
    .map_err(|e| failure(action, e))
}

#[tauri::command]
pub async fn list_agent_schedules() -> Result<Value, String> {
  call("list", reqwest::Method::GET, base(), None).await
}

#[tauri::command]
pub async fn create_agent_schedule(schedule: Value) -> Result<Value, String> {
  let schedule = checked_body(schedule)?;
  let created = call("create", reqwest::Method::POST, base(), Some(schedule)).await?;
  log::info!(
    "Schedule created schedule={}",
    created.get("id").and_then(Value::as_str).unwrap_or("?")
  );
  Ok(created)
}

#[tauri::command]
pub async fn update_agent_schedule(id: String, patch: Value) -> Result<Value, String> {
  let patch = checked_body(patch)?;
  call(
    "update",
    reqwest::Method::PATCH,
    url_for(&id, "")?,
    Some(patch),
  )
  .await
}

#[tauri::command]
pub async fn delete_agent_schedule(id: String) -> Result<(), String> {
  call("delete", reqwest::Method::DELETE, url_for(&id, "")?, None).await?;
  log::info!("Schedule deleted schedule={id}");
  Ok(())
}

#[tauri::command]
pub async fn run_agent_schedule_now(id: String) -> Result<Value, String> {
  call(
    "run_now",
    reqwest::Method::POST,
    url_for(&id, "/run-now")?,
    Some(serde_json::json!({})),
  )
  .await
}

#[tauri::command]
pub async fn list_agent_schedule_runs(id: String) -> Result<Value, String> {
  call("runs", reqwest::Method::GET, url_for(&id, "/runs")?, None).await
}

/// Send a test email and webhook for the schedule `id`, as a failed run would.
#[tauri::command]
pub async fn test_agent_schedule_notice(id: String) -> Result<Value, String> {
  call(
    "test_notice",
    reqwest::Method::POST,
    url_for(&id, "/test-notice")?,
    Some(serde_json::json!({})),
  )
  .await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_id_is_one_path_segment() {
    let url = url_for("a/../b", "/runs").unwrap();
    assert!(url.ends_with("/api/agent/schedules/a%2F..%2Fb/runs"));
    assert!(url_for("  ", "")
      .unwrap_err()
      .contains("AGENT_SCHEDULE_NOT_FOUND"));
    assert!(url_for(&"x".repeat(129), "").is_err());
  }

  #[test]
  fn only_an_object_is_sent() {
    assert!(checked_body(serde_json::json!({"name": "nightly"})).is_ok());
    assert!(checked_body(serde_json::json!(["x"]))
      .unwrap_err()
      .contains("AGENT_SCHEDULE_INVALID"));
  }

  #[test]
  fn a_plan_without_schedules_is_refused_with_the_server_code() {
    let refusal = crate::cloud_errors::classify(
      402,
      r#"{"code":"SCHEDULED_AUTOMATION_NOT_ENTITLED"}"#,
      SCHEDULE_CODES,
    );
    assert_eq!(refusal.code, "SCHEDULED_AUTOMATION_NOT_ENTITLED");
    let forbidden = crate::cloud_errors::classify(403, "", SCHEDULE_CODES);
    assert_eq!(forbidden.code, "SCHEDULED_AUTOMATION_NOT_ENTITLED");
  }
}

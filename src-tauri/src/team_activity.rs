//! This computer's share of the team activity log: what a member changed
//! here (profiles, proxies, groups, VPNs, extensions, templates, statuses).
//! The cloud records sign-ins, membership changes and profile launches itself.
//!
//! Only a team seat whose plan includes team collaboration reports. Events
//! wait in memory, never on disk, and leave in batches. A batch that fails is
//! tried again a few times, then dropped: the log is a record, not a sync.
//! Names leave this computer only when the team's data is not end-to-end
//! encrypted, because the cloud then holds those names already; otherwise the
//! log carries ids, and a viewer shows its own local names for them.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::cloud_auth::CloudUser;
use crate::cloud_errors::FailureCodes;
use crate::log_streak::Streak;

/// The cloud's limits: events per report, and the longest id and name.
const BATCH: usize = 50;
const MAX_ID_CHARS: usize = 128;
const MAX_NAME_CHARS: usize = 200;
/// Events kept while the cloud cannot be reached. Older ones drop first.
const MAX_QUEUED: usize = 500;
const ATTEMPTS: u8 = 3;
const SETTLE: Duration = Duration::from_secs(5);
const RETRY_AFTER: Duration = Duration::from_secs(60);

const ACTIVITY_CODES: FailureCodes = FailureCodes {
  bad_request: "TEAM_ACTIVITY_INVALID",
  forbidden: "TEAM_ACTIVITY_FORBIDDEN",
  not_found: "TEAM_ACTIVITY_FORBIDDEN",
  conflict: "TEAM_ACTIVITY_INVALID",
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static FLUSHING: AtomicBool = AtomicBool::new(false);
static QUEUE: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
static REPORT: Streak = Streak::new(module_path!(), "Team activity report");

/// Record the signed-in account, or `None` once signed out.
pub fn observe(user: Option<&CloudUser>) {
  let enabled =
    user.is_some_and(|user| user.team_id.is_some() && user.entitlements().team_collaboration);
  if !enabled {
    queue().clear();
  }
  ENABLED.store(enabled, Ordering::Relaxed);
}

fn queue() -> std::sync::MutexGuard<'static, VecDeque<Value>> {
  QUEUE
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn clip(value: &str, max: usize) -> String {
  value.chars().take(max).collect()
}

/// One event, as the cloud takes it. `action` is `<target type>.<verb>`.
fn event(action: &str, id: &str, name: Option<&str>, detail: Option<Value>, names: bool) -> Value {
  let target_type = action.split('.').next().unwrap_or_default();
  let mut event = json!({
    "action": action,
    "targetType": target_type,
    "targetId": clip(id, MAX_ID_CHARS),
  });
  if let Some(name) = name.filter(|name| names && !name.trim().is_empty()) {
    event["targetName"] = json!(clip(name.trim(), MAX_NAME_CHARS));
  }
  if let Some(detail) = detail.filter(|detail| detail.as_object().is_some_and(|o| !o.is_empty())) {
    event["detail"] = detail;
  }
  event
}

/// Note that the person changed `id`. Free when this seat does not report.
pub fn record(action: &str, id: &str, name: Option<&str>, detail: Option<Value>) {
  if !ENABLED.load(Ordering::Relaxed) {
    return;
  }
  let names = !crate::sync::encryption::has_e2e_password();
  {
    let mut queue = queue();
    if queue.len() >= MAX_QUEUED {
      queue.pop_front();
    }
    queue.push_back(event(action, id, name, detail, names));
  }
  if !FLUSHING.swap(true, Ordering::AcqRel) {
    tauri::async_runtime::spawn(flush());
  }
}

/// The fields an edit changed, as the detail the log shows.
pub fn fields(fields: &[&str]) -> Option<Value> {
  (!fields.is_empty()).then(|| json!({ "fields": fields }))
}

async fn flush() {
  tokio::time::sleep(SETTLE).await;
  let mut attempts = 0u8;
  loop {
    let batch: Vec<Value> = {
      let mut queue = queue();
      let take = queue.len().min(BATCH);
      queue.drain(..take).collect()
    };
    if batch.is_empty() {
      FLUSHING.store(false, Ordering::Release);
      // An event recorded between the drain and the flag store would wait for
      // the next one; take it now instead.
      if queue().is_empty() || FLUSHING.swap(true, Ordering::AcqRel) {
        return;
      }
      continue;
    }
    let count = batch.len();
    let url = format!("{}/api/teams/activity", crate::cloud_auth::CLOUD_API_URL);
    let sent = crate::cloud_api::call::<Option<Value>>(
      reqwest::Method::POST,
      url,
      Some(json!({ "events": batch })),
      ACTIVITY_CODES,
    )
    .await;
    match sent {
      Ok(_) => {
        REPORT.succeeded();
        attempts = 0;
        log::debug!("Team activity reported events={count}");
      }
      Err(err) if err.status == 400 || err.status == 403 => {
        // Not entitled any more, or events this build cannot shape: sending
        // them again changes nothing.
        log::warn!(
          "Team activity dropped events={count} code={} status={}",
          err.code,
          err.status
        );
      }
      Err(err) => {
        REPORT.failed(format!("code={} status={}", err.code, err.status));
        attempts += 1;
        if attempts >= ATTEMPTS {
          log::warn!("Team activity dropped events={count} attempts={attempts}");
          attempts = 0;
        } else {
          let mut queue = queue();
          for event in batch.into_iter().rev() {
            queue.push_front(event);
          }
          queue.truncate(MAX_QUEUED);
        }
        tokio::time::sleep(RETRY_AFTER).await;
      }
    }
  }
}

/// A page of the team's log, newest first. Owners and admins only; the cloud
/// checks the role and the plan.
#[tauri::command]
pub async fn list_team_activity(
  cursor: Option<String>,
  limit: Option<u32>,
  action: Option<String>,
) -> Result<Value, String> {
  let Some(team_id) = crate::cloud_auth::CLOUD_AUTH
    .get_user()
    .await
    .and_then(|auth| auth.user.team_id)
  else {
    return Err(crate::backend_error("TEAM_ACTIVITY_FORBIDDEN"));
  };
  let url = activity_url(&team_id, cursor.as_deref(), limit, action.as_deref())?;
  crate::cloud_api::call::<Value>(reqwest::Method::GET, url, None, ACTIVITY_CODES)
    .await
    .map_err(|err| {
      log::warn!(
        "Team activity request failed code={} status={}",
        err.code,
        err.status
      );
      err.to_error_json()
    })
}

fn activity_url(
  team_id: &str,
  cursor: Option<&str>,
  limit: Option<u32>,
  action: Option<&str>,
) -> Result<String, String> {
  let mut query = Vec::new();
  if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
    if cursor.len() > 15 || !cursor.bytes().all(|b| b.is_ascii_digit()) {
      return Err(crate::backend_error("TEAM_ACTIVITY_INVALID"));
    }
    query.push(format!("cursor={cursor}"));
  }
  if let Some(limit) = limit {
    query.push(format!("limit={}", limit.clamp(1, 200)));
  }
  if let Some(action) = action.filter(|action| !action.is_empty()) {
    query.push(format!("action={}", urlencoding::encode(&clip(action, 64))));
  }
  let mut url = format!(
    "{}/api/teams/{}/activity",
    crate::cloud_auth::CLOUD_API_URL,
    urlencoding::encode(team_id)
  );
  if !query.is_empty() {
    url.push('?');
    url.push_str(&query.join("&"));
  }
  Ok(url)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_event_names_its_target_only_when_allowed() {
    let named = event(
      "profile.rename",
      "p1",
      Some("  Shop  "),
      fields(&["name"]),
      true,
    );
    assert_eq!(named["targetType"], "profile");
    assert_eq!(named["targetName"], "Shop");
    assert_eq!(named["detail"], json!({ "fields": ["name"] }));

    let sealed = event("profile.rename", "p1", Some("Shop"), None, false);
    assert!(sealed.get("targetName").is_none());
    assert!(sealed.get("detail").is_none());
  }

  #[test]
  fn an_event_fits_the_cloud_limits() {
    let long = "x".repeat(300);
    let clipped = event("proxy.update", &long, Some(&long), None, true);
    assert_eq!(clipped["targetId"].as_str().unwrap().len(), MAX_ID_CHARS);
    assert_eq!(
      clipped["targetName"].as_str().unwrap().len(),
      MAX_NAME_CHARS
    );
  }

  #[test]
  fn nothing_queues_for_a_seat_that_does_not_report() {
    observe(None);
    record("profile.delete", "p1", None, None);
    assert!(queue().is_empty());
  }

  #[test]
  fn the_activity_url_keeps_the_team_id_one_segment() {
    let url = activity_url("a/b", Some("42"), Some(500), Some("profile.delete")).unwrap();
    assert!(url.contains("/api/teams/a%2Fb/activity?"), "{url}");
    assert!(url.contains("cursor=42"));
    assert!(url.contains("limit=200"));
    assert!(url.contains("action=profile.delete"));
    assert!(activity_url("t1", None, None, None)
      .unwrap()
      .ends_with("/api/teams/t1/activity"));
    assert!(activity_url("t1", Some("1;drop"), None, None).is_err());
  }
}

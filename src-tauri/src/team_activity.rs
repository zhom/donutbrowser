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

/// What part of the team's log to read. Every field is optional.
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityFilter {
  /// An action (`profile.delete`) or every action of a kind (`profile.*`).
  pub action: Option<String>,
  pub actor_user_id: Option<String>,
  pub target_type: Option<String>,
  /// The oldest time to include, ISO 8601.
  pub since: Option<String>,
}

const TARGET_TYPES: &[&str] = &[
  "profile",
  "proxy",
  "group",
  "vpn",
  "extension",
  "template",
  "status",
  "member",
  "invitation",
];

impl ActivityFilter {
  /// The query pairs this filter adds, or `TEAM_ACTIVITY_INVALID`.
  fn query(&self) -> Result<Vec<String>, String> {
    let invalid = || crate::backend_error("TEAM_ACTIVITY_INVALID");
    let present = |value: &Option<String>| value.clone().filter(|v| !v.is_empty());
    let mut query = Vec::new();
    if let Some(action) = present(&self.action) {
      let (kind, verb) = action.split_once('.').ok_or_else(invalid)?;
      let word = |w: &str| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
      if action.len() > 64 || !word(kind) || !(verb == "*" || word(verb)) {
        return Err(invalid());
      }
      query.push(format!("action={}", urlencoding::encode(&action)));
    }
    if let Some(actor) = present(&self.actor_user_id) {
      if actor.len() > 128
        || !actor
          .bytes()
          .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
      {
        return Err(invalid());
      }
      query.push(format!("actorUserId={actor}"));
    }
    if let Some(target_type) = present(&self.target_type) {
      if !TARGET_TYPES.contains(&target_type.as_str()) {
        return Err(invalid());
      }
      query.push(format!("targetType={target_type}"));
    }
    if let Some(since) = present(&self.since) {
      let at = chrono::DateTime::parse_from_rfc3339(&since).map_err(|_| invalid())?;
      query.push(format!(
        "since={}",
        urlencoding::encode(
          &at
            .to_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        )
      ));
    }
    Ok(query)
  }
}

async fn signed_in_team() -> Result<String, String> {
  crate::cloud_auth::CLOUD_AUTH
    .get_user()
    .await
    .and_then(|auth| auth.user.team_id)
    .ok_or_else(|| crate::backend_error("TEAM_ACTIVITY_FORBIDDEN"))
}

fn activity_failure(err: crate::cloud_errors::BackendFailure) -> String {
  log::warn!(
    "Team activity request failed code={} status={}",
    err.code,
    err.status
  );
  err.to_error_json()
}

/// A page of the team's log, newest first. Owners and admins only; the cloud
/// checks the role and the plan.
#[tauri::command]
pub async fn list_team_activity(
  cursor: Option<String>,
  limit: Option<u32>,
  filter: Option<ActivityFilter>,
) -> Result<Value, String> {
  let team_id = signed_in_team().await?;
  let url = activity_url(
    &team_id,
    "activity",
    cursor.as_deref(),
    limit,
    &filter.unwrap_or_default(),
  )?;
  crate::cloud_api::call::<Value>(reqwest::Method::GET, url, None, ACTIVITY_CODES)
    .await
    .map_err(activity_failure)
}

/// The filtered log as a CSV file at `path`, which the person chose. Returns
/// how many rows it holds.
#[tauri::command]
pub async fn export_team_activity_csv(
  path: String,
  filter: Option<ActivityFilter>,
) -> Result<u64, String> {
  let team_id = signed_in_team().await?;
  let url = activity_url(
    &team_id,
    "activity.csv",
    None,
    None,
    &filter.unwrap_or_default(),
  )?;
  let csv = crate::cloud_api::call_text(reqwest::Method::GET, url, ACTIVITY_CODES)
    .await
    .map_err(activity_failure)?;
  std::fs::write(&path, csv.as_bytes())
    .map_err(|e| crate::backend_error_with_detail("TEAM_ACTIVITY_EXPORT_FAILED", e))?;
  let rows = csv.lines().count().saturating_sub(1) as u64;
  log::info!("Team activity exported rows={rows}");
  Ok(rows)
}

fn activity_url(
  team_id: &str,
  resource: &str,
  cursor: Option<&str>,
  limit: Option<u32>,
  filter: &ActivityFilter,
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
  query.extend(filter.query()?);
  let mut url = format!(
    "{}/api/teams/{}/{resource}",
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
    let filter = ActivityFilter {
      action: Some("profile.*".into()),
      actor_user_id: Some("u-1".into()),
      target_type: Some("proxy".into()),
      since: Some("2026-10-01T12:00:00+02:00".into()),
    };
    let url = activity_url("a/b", "activity", Some("42"), Some(500), &filter).unwrap();
    assert!(url.contains("/api/teams/a%2Fb/activity?"), "{url}");
    assert!(url.contains("cursor=42"));
    assert!(url.contains("limit=200"));
    assert!(url.contains("action=profile.%2A"), "{url}");
    assert!(url.contains("actorUserId=u-1"));
    assert!(url.contains("targetType=proxy"));
    assert!(url.contains("since=2026-10-01T10%3A00%3A00Z"), "{url}");
    let plain = ActivityFilter::default();
    assert!(activity_url("t1", "activity.csv", None, None, &plain)
      .unwrap()
      .ends_with("/api/teams/t1/activity.csv"));
    assert!(activity_url("t1", "activity", Some("1;drop"), None, &plain).is_err());
  }

  #[test]
  fn a_filter_the_cloud_would_refuse_is_refused_here() {
    for filter in [
      ActivityFilter {
        action: Some("Profile.delete".into()),
        ..Default::default()
      },
      ActivityFilter {
        action: Some("profile".into()),
        ..Default::default()
      },
      ActivityFilter {
        actor_user_id: Some("a&b=c".into()),
        ..Default::default()
      },
      ActivityFilter {
        target_type: Some("planet".into()),
        ..Default::default()
      },
      ActivityFilter {
        since: Some("yesterday".into()),
        ..Default::default()
      },
    ] {
      assert!(filter.query().is_err(), "{filter:?}");
    }
  }
}

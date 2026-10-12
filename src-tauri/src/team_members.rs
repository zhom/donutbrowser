//! The team's members and the profile groups each one may reach. Owners and
//! admins set the access here, because group names live only in the app; the
//! cloud stores group ids and the sync server holds each member to them.

use serde_json::{json, Value};

use crate::cloud_errors::FailureCodes;

/// The `group-id` the sync server uses for profiles in no group.
pub const UNGROUPED: &str = "__ungrouped__";
const MAX_GROUPS: usize = 500;

const MEMBER_CODES: FailureCodes = FailureCodes {
  bad_request: "TEAM_ACCESS_INVALID",
  forbidden: "TEAM_ACCESS_FORBIDDEN",
  not_found: "TEAM_MEMBER_NOT_FOUND",
  conflict: "TEAM_ACCESS_INVALID",
};

async fn team_id() -> Result<String, String> {
  crate::cloud_auth::CLOUD_AUTH
    .get_user()
    .await
    .and_then(|auth| auth.user.team_id)
    .ok_or_else(|| crate::backend_error("TEAM_ACCESS_FORBIDDEN"))
}

fn failure(action: &str, err: crate::cloud_errors::BackendFailure) -> String {
  log::warn!(
    "Team member request failed action={action} code={} status={}",
    err.code,
    err.status
  );
  err.to_error_json()
}

fn is_group_id(id: &str) -> bool {
  (1..=128).contains(&id.len())
    && id
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The access to send: `None` is every group; a list is deduplicated, in order.
fn checked_groups(groups: Option<Vec<String>>) -> Result<Option<Vec<String>>, String> {
  let Some(groups) = groups else {
    return Ok(None);
  };
  let mut seen = std::collections::HashSet::new();
  let groups: Vec<String> = groups
    .into_iter()
    .map(|id| id.trim().to_string())
    .filter(|id| seen.insert(id.clone()))
    .collect();
  if groups.len() > MAX_GROUPS || !groups.iter().all(|id| is_group_id(id)) {
    return Err(crate::backend_error("TEAM_ACCESS_INVALID"));
  }
  Ok(Some(groups))
}

fn members_url(team_id: &str, tail: &str) -> String {
  format!(
    "{}/api/teams/{}/members{tail}",
    crate::cloud_auth::CLOUD_API_URL,
    urlencoding::encode(team_id)
  )
}

#[tauri::command]
pub async fn list_team_members() -> Result<Value, String> {
  let team = team_id().await?;
  crate::cloud_api::call::<Value>(
    reqwest::Method::GET,
    members_url(&team, ""),
    None,
    MEMBER_CODES,
  )
  .await
  .map_err(|e| failure("list", e))
}

/// Limit `user_id` to `groups`, or give back every group with `None`.
#[tauri::command]
pub async fn set_team_member_access(
  user_id: String,
  groups: Option<Vec<String>>,
) -> Result<Value, String> {
  let team = team_id().await?;
  if !is_group_id(user_id.trim()) {
    return Err(crate::backend_error("TEAM_MEMBER_NOT_FOUND"));
  }
  let groups = checked_groups(groups)?;
  let count = groups.as_ref().map(Vec::len);
  let url = members_url(
    &team,
    &format!("/{}/access", urlencoding::encode(user_id.trim())),
  );
  let member = crate::cloud_api::call::<Value>(
    reqwest::Method::PUT,
    url,
    Some(json!({ "groups": groups })),
    MEMBER_CODES,
  )
  .await
  .map_err(|e| failure("set_access", e))?;
  match count {
    Some(count) => log::info!("Team member access set groups={count}"),
    None => log::info!("Team member access set groups=all"),
  }
  Ok(member)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn access_is_every_group_or_a_clean_list_of_ids() {
    assert_eq!(checked_groups(None).unwrap(), None);
    assert_eq!(
      checked_groups(Some(vec!["g1".into(), " g1 ".into(), UNGROUPED.into()])).unwrap(),
      Some(vec!["g1".to_string(), UNGROUPED.to_string()])
    );
    assert_eq!(checked_groups(Some(vec![])).unwrap(), Some(vec![]));
    assert!(checked_groups(Some(vec!["a/b".into()])).is_err());
    assert!(checked_groups(Some(vec!["x".repeat(129)])).is_err());
    assert!(checked_groups(Some((0..501).map(|i| format!("g{i}")).collect())).is_err());
  }

  #[test]
  fn a_member_id_is_one_path_segment() {
    assert!(members_url("t/1", "/u/access").contains("/api/teams/t%2F1/members/u/access"));
    assert!(!is_group_id("../x"));
  }
}

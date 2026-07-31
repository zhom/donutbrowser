//! Launching a profile on a remote VM.
//!
//! The desktop app never talks to the Wayfern manager directly. It asks
//! donutbrowser-infra, which holds the service-account credentials and is the
//! only party that can mint a donut-sync token scoped to this user's namespace.
//! That indirection is the point: a desktop client that could call the manager
//! itself would need credentials capable of launching sessions for anyone.

use crate::profile::types::BrowserProfile;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Why a remote launch failed, mapped to the status the local API should return.
#[derive(Debug)]
pub enum RemoteSessionError {
  /// No host of the profile's OS has a free slot right now.
  NoCapacity(String),
  /// The profile is already open somewhere — locally or in another session.
  Conflict(String),
  /// The user's plan does not cover remote automation.
  NotAuthorised(String),
  Other(String),
}

impl std::fmt::Display for RemoteSessionError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::NoCapacity(m) | Self::Conflict(m) | Self::NotAuthorised(m) | Self::Other(m) => {
        write!(f, "{m}")
      }
    }
  }
}

/// What the backend returns when a session starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSessionOutcome {
  pub session_id: String,
  pub platform: String,
  pub status: String,
}

#[derive(Debug, Serialize)]
struct StartRemoteRequest {
  profile_id: String,
  /// The profile's own OS. The backend refuses to schedule it anywhere else.
  platform: String,
  /// Set when the caller wants a page opened once the browser is up.
  #[serde(skip_serializing_if = "Option::is_none")]
  url: Option<String>,
  /// De-duplicates retries so a flaky network cannot open two browsers against
  /// one profile.
  idempotency_key: String,
}

/// Map a backend status onto a typed error.
///
/// Kept separate from the request so the mapping is testable: getting 503
/// wrong would turn "come back in a minute" into "something is broken",
/// and getting 409 wrong would hide the fact that the profile is already open.
pub fn classify_backend_status(status: u16, body: &str) -> RemoteSessionError {
  let message = if body.is_empty() {
    format!("remote session request failed with HTTP {status}")
  } else {
    body.to_string()
  };
  match status {
    503 => RemoteSessionError::NoCapacity(message),
    409 => RemoteSessionError::Conflict(message),
    401..=403 => RemoteSessionError::NotAuthorised(message),
    _ => RemoteSessionError::Other(message),
  }
}

/// Build the idempotency key for one launch attempt.
///
/// Derived from the profile and a caller-supplied attempt id rather than
/// random, so a retry of the SAME user action de-duplicates while a genuinely
/// new launch does not. The attempt id is a plain uniqueness token, not a
/// cryptographic value.
pub fn idempotency_key(profile_id: &str, attempt: &str) -> String {
  format!("run-remote:{profile_id}:{attempt}")
}

/// Ask donutbrowser-infra to start a remote session for this profile.
///
/// Goes through `api_call_with_retry` so an expired access token is refreshed
/// and the request retried once, rather than surfacing to the user as a
/// spurious "not signed in".
pub async fn start_remote_session(
  _app: AppHandle,
  profile: &BrowserProfile,
  url: Option<String>,
) -> Result<RemoteSessionOutcome, RemoteSessionError> {
  let platform = profile
    .resolved_os()
    .ok_or_else(|| {
      RemoteSessionError::Other("profile has no recorded operating system".to_string())
    })?
    .to_string();
  let profile_id = profile.id.to_string();

  // One key for this user action: a retry inside api_call_with_retry must
  // de-duplicate rather than open a second browser on the same profile.
  let key = idempotency_key(&profile_id, &uuid::Uuid::new_v4().to_string());
  let endpoint = format!("{}/api/remote-sessions", crate::cloud_auth::CLOUD_API_URL);

  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let endpoint = endpoint.clone();
      let body = StartRemoteRequest {
        profile_id: profile_id.clone(),
        platform: platform.clone(),
        url: url.clone(),
        idempotency_key: key.clone(),
      };
      async move {
        let response = reqwest::Client::new()
          .post(&endpoint)
          .bearer_auth(token)
          .json(&body)
          .send()
          .await
          .map_err(|e| format!("reach backend: {e}"))?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
          let text = response.text().await.unwrap_or_default();
          // Encode the status so api_call_with_retry can spot a 401, and so
          // classify_backend_status can recover the kind afterwards.
          return Err(format!("({status}) {text}"));
        }
        response
          .json::<RemoteSessionOutcome>()
          .await
          .map_err(|e| format!("decode response: {e}"))
      }
    })
    .await
    .map_err(|e| classify_error_string(&e))
}

/// Recover a typed error from `api_call_with_retry`'s string.
///
/// That helper flattens everything to `String` to do its 401 sniffing, so the
/// status is re-parsed here rather than lost — a 503 surfacing as a generic
/// failure would tell the user their fleet is broken when it is merely busy.
pub fn classify_error_string(message: &str) -> RemoteSessionError {
  if let Some(rest) = message.strip_prefix('(') {
    if let Some((code, tail)) = rest.split_once(')') {
      if let Ok(status) = code.trim().parse::<u16>() {
        return classify_backend_status(status, tail.trim());
      }
    }
  }
  RemoteSessionError::Other(message.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn no_capacity_is_distinguished_from_a_real_failure() {
    // 503 means "come back in a minute", not "something is broken" — conflating
    // them would make a busy fleet look like an outage to the user.
    assert!(matches!(
      classify_backend_status(503, "no macos host free"),
      RemoteSessionError::NoCapacity(_)
    ));
    assert!(matches!(
      classify_backend_status(500, "boom"),
      RemoteSessionError::Other(_)
    ));
  }

  #[test]
  fn conflict_is_surfaced_so_the_user_learns_the_profile_is_open() {
    assert!(matches!(
      classify_backend_status(409, "profile already has a live session"),
      RemoteSessionError::Conflict(_)
    ));
  }

  #[test]
  fn payment_and_auth_failures_map_to_not_authorised() {
    for status in [401u16, 402, 403] {
      assert!(
        matches!(
          classify_backend_status(status, ""),
          RemoteSessionError::NotAuthorised(_)
        ),
        "status {status} should be NotAuthorised"
      );
    }
  }

  #[test]
  fn an_empty_body_still_produces_a_useful_message() {
    let err = classify_backend_status(500, "");
    assert!(err.to_string().contains("500"));
  }

  #[test]
  fn a_status_encoded_error_string_round_trips_to_its_kind() {
    // api_call_with_retry flattens everything to String to sniff for 401s; the
    // status must survive that or a busy fleet looks like a broken one.
    assert!(matches!(
      classify_error_string("(503) no macos host free"),
      RemoteSessionError::NoCapacity(_)
    ));
    assert!(matches!(
      classify_error_string("(409) already running"),
      RemoteSessionError::Conflict(_)
    ));
  }

  #[test]
  fn an_unencoded_error_string_is_not_misread_as_a_status() {
    assert!(matches!(
      classify_error_string("reach backend: connection refused"),
      RemoteSessionError::Other(_)
    ));
  }

  #[test]
  fn idempotency_key_is_stable_for_one_attempt_and_distinct_across_attempts() {
    let a = idempotency_key("p1", "attempt-1");
    assert_eq!(a, idempotency_key("p1", "attempt-1"));
    assert_ne!(a, idempotency_key("p1", "attempt-2"));
    assert_ne!(a, idempotency_key("p2", "attempt-1"));
  }
}

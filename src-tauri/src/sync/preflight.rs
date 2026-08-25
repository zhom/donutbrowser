//! Pre-flight check for a sync server, run from the network stack that
//! actually performs transfers.
//!
//! A self-hosted server almost always reaches its storage over an address only
//! it can resolve: the documented compose file points `S3_ENDPOINT` at
//! `http://minio:9000`, a Docker service name that exists on the compose
//! network and nowhere else. Presigned URLs are signed against the host they
//! name, so every URL handed to this device names a host it cannot open. The
//! server is healthy, `/health` and `/readyz` are green, and every single file
//! transfer fails at connect.
//!
//! Checking the server alone is what let that configuration look correct. This
//! module also opens the storage host the server says it hands out, from here,
//! with the same client the uploader uses, so the break is named at the moment
//! the user configures sync instead of after the first sync fails.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Both probes are liveness questions, not transfers, so they must fail fast
/// rather than sit on a connect that is never going to answer.
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// What a pre-flight found. Every field is reported rather than collapsed into
/// one boolean: "the server answers but its storage is unreachable from here"
/// is a different problem with a different fix than "the server is down", and
/// the UI has to be able to say which one happened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SyncServerCheck {
  /// The sync server itself answered.
  pub server_reachable: bool,
  /// The server reports it can reach its own storage. `None` when the server
  /// is too old to serve `/readyz`, which is a working server, not a broken
  /// one.
  pub storage_ready: Option<bool>,
  /// The host the server signs into presigned URLs, when it discloses one.
  /// Withheld by cloud deployments on purpose.
  pub storage_endpoint: Option<String>,
  /// Whether that host answered *this device*. `None` when there was nothing
  /// to probe.
  pub storage_reachable: Option<bool>,
  /// Why the storage probe failed, for the log and the error surface.
  pub storage_error: Option<String>,
}

impl SyncServerCheck {
  /// Whether sync can actually move bytes. A green server with an unreachable
  /// storage host is the exact state this check exists to stop reporting as
  /// success.
  pub fn is_usable(&self) -> bool {
    self.server_reachable
      && self.storage_ready != Some(false)
      && self.storage_reachable != Some(false)
  }
}

/// The `/readyz` body. Every field is optional: older servers answer `/health`
/// only, and cloud deployments withhold `storageEndpoint`.
#[derive(Debug, Deserialize)]
struct ReadyzBody {
  #[serde(default)]
  s3: Option<bool>,
  #[serde(default, rename = "storageEndpoint")]
  storage_endpoint: Option<String>,
}

fn probe_client() -> reqwest::Client {
  // Matches how `SyncClient` builds its client, so a TLS trust or proxy
  // condition that would fail an upload fails the probe the same way. A probe
  // that is more permissive than the uploader would report a working setup for
  // a configuration that cannot transfer.
  reqwest::Client::builder()
    .timeout(PROBE_TIMEOUT)
    .build()
    .unwrap_or_default()
}

/// Ask the sync server about itself, then verify the storage host it names.
pub async fn check_sync_server(server_url: &str) -> SyncServerCheck {
  let base = server_url.trim().trim_end_matches('/');
  if base.is_empty() {
    return SyncServerCheck::default();
  }

  let client = probe_client();
  let mut check = SyncServerCheck::default();

  let readyz = match client.get(format!("{base}/readyz")).send().await {
    Ok(response) => response,
    Err(e) => {
      log::warn!("Sync pre-flight: {base}/readyz did not answer: {e}");
      return check;
    }
  };

  if readyz.status() == reqwest::StatusCode::NOT_FOUND {
    // Predates /readyz. It is still a working server, so fall back rather than
    // failing a healthy setup, and leave the storage fields unknown.
    check.server_reachable = matches!(
      client.get(format!("{base}/health")).send().await,
      Ok(health) if health.status().is_success()
    );
    return check;
  }

  // A 503 from /readyz is the server telling us its storage is down. That is a
  // reachable server with a real diagnosis in the body, so read it rather than
  // discarding it as a failed request.
  check.server_reachable = readyz.status().is_success() || readyz.status().as_u16() == 503;
  if !check.server_reachable {
    return check;
  }

  let body = readyz.json::<ReadyzBody>().await.ok();
  check.storage_ready = body.as_ref().and_then(|b| b.s3);
  check.storage_endpoint = body.and_then(|b| b.storage_endpoint);

  if let Some(endpoint) = check.storage_endpoint.clone() {
    match probe_storage_endpoint(&client, &endpoint).await {
      Ok(()) => check.storage_reachable = Some(true),
      Err(e) => {
        log::warn!("Sync pre-flight: storage endpoint {endpoint} is unreachable from here: {e}");
        check.storage_reachable = Some(false);
        check.storage_error = Some(e);
      }
    }
  }

  check
}

/// Open the storage host and report only whether it answered.
///
/// ANY HTTP status counts as reachable, including 403 and 404. An unsigned GET
/// of a bucket root is supposed to be refused; being refused proves DNS, TCP
/// and TLS all worked, which is the entire question. Only a transport error
/// means the presigned URLs cannot be opened from this device.
async fn probe_storage_endpoint(client: &reqwest::Client, endpoint: &str) -> Result<(), String> {
  match client.get(endpoint).send().await {
    Ok(_) => Ok(()),
    Err(e) => Err(transport_reason(&e)),
  }
}

/// A short reason for a failed request.
///
/// `reqwest::Error`'s own `Display` is one line about the request and hides the
/// cause chain, so a DNS failure reads as "error sending request" — the exact
/// uninformative text that made this class of failure undiagnosable in the
/// first place. Walk to the innermost source instead.
///
/// Shared with the transfer path so a failed upload and a failed probe describe
/// the same network condition in the same words.
pub(crate) fn transport_reason(error: &reqwest::Error) -> String {
  let kind = if error.is_timeout() {
    "timed out"
  } else if error.is_connect() {
    "connection failed"
  } else {
    "request failed"
  };

  let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(error);
  let mut innermost: Option<String> = None;
  while let Some(cause) = source {
    innermost = Some(cause.to_string());
    source = cause.source();
  }

  match innermost {
    Some(detail) => format!("{kind}: {detail}"),
    None => kind.to_string(),
  }
}

/// Pre-flight a sync server before saving it, and before trusting it to sync.
#[tauri::command]
pub async fn check_sync_server_connection(server_url: String) -> Result<SyncServerCheck, String> {
  Ok(check_sync_server(&server_url).await)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn unreachable_storage_is_not_usable() {
    // The shape that used to report as healthy: server up, server's own
    // storage fine, and the host it hands to clients resolving nowhere but the
    // compose network.
    let check = SyncServerCheck {
      server_reachable: true,
      storage_ready: Some(true),
      storage_endpoint: Some("http://minio:9000".to_string()),
      storage_reachable: Some(false),
      storage_error: Some("connection failed: dns error".to_string()),
    };
    assert!(!check.is_usable());
  }

  #[test]
  fn reachable_storage_is_usable() {
    let check = SyncServerCheck {
      server_reachable: true,
      storage_ready: Some(true),
      storage_endpoint: Some("http://localhost:9101".to_string()),
      storage_reachable: Some(true),
      storage_error: None,
    };
    assert!(check.is_usable());
  }

  #[test]
  fn server_without_readyz_is_usable() {
    // A server old enough to predate /readyz discloses nothing about storage.
    // Unknown must not read as broken, or every older self-hosted server would
    // start reporting a failure it does not have.
    let check = SyncServerCheck {
      server_reachable: true,
      storage_ready: None,
      storage_endpoint: None,
      storage_reachable: None,
      storage_error: None,
    };
    assert!(check.is_usable());
  }

  #[test]
  fn server_reporting_its_own_storage_down_is_not_usable() {
    let check = SyncServerCheck {
      server_reachable: true,
      storage_ready: Some(false),
      ..Default::default()
    };
    assert!(!check.is_usable());
  }

  #[test]
  fn unreachable_server_is_not_usable() {
    assert!(!SyncServerCheck::default().is_usable());
  }

  #[tokio::test]
  async fn empty_url_reports_unreachable_without_a_request() {
    assert_eq!(check_sync_server("   ").await, SyncServerCheck::default());
  }

  #[tokio::test]
  async fn unresolvable_storage_host_is_reported_with_a_cause() {
    // Exercises the real probe against a host that cannot resolve, which is
    // what a container-only endpoint looks like from the desktop.
    let client = probe_client();
    let error = probe_storage_endpoint(&client, "http://minio.invalid:9000")
      .await
      .expect_err("an unresolvable host must not report as reachable");
    assert!(
      error.contains("failed") || error.contains("timed out"),
      "unexpected reason: {error}"
    );
    // The bare reqwest Display is what this exists to avoid.
    assert_ne!(error, "error sending request");
  }
}

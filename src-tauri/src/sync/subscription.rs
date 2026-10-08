use crate::events;
use crate::log_streak::Streak;
use crate::settings_manager::SettingsManager;
use reqwest::Client;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeEvent {
  #[serde(rename = "type")]
  pub event_type: String,
  pub key: Option<String>,
  #[serde(rename = "lastModified")]
  pub last_modified: Option<String>,
  pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum SyncWorkItem {
  Profile(String),
  Proxy(String),
  Group(String),
  Vpn(String),
  Extension(String),
  ExtensionGroup(String),
  Tombstone(String, String),
}

/// Where a subscription's sync token comes from, so reconnects can re-fetch a
/// fresh one (tokens are short-lived, ~15 min).
#[derive(Clone, Copy)]
enum TokenSource {
  Cloud,
  SelfHosted,
}

const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(6);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(600);
const STOP_POLL_INTERVAL: Duration = Duration::from_secs(1);

static SSE_CONNECT: Streak = Streak::new(module_path!(), "Sync SSE connect");
// Cloud token failures are already logged by `cloud_auth`.
static SELF_HOSTED_TOKEN: Streak = Streak::new(module_path!(), "Self-hosted sync token read");

#[derive(Debug)]
enum StreamFailure {
  Refused(u16),
  RateLimited(Duration),
  Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
enum TokenStep {
  Use(String),
  Keep,
  Wait,
  Stop,
}

fn reconnect_delay(failures: u32) -> Duration {
  let doublings = failures.saturating_sub(1).min(16);
  RECONNECT_BASE_DELAY
    .saturating_mul(1u32 << doublings)
    .min(RECONNECT_MAX_DELAY)
}

fn is_refused_fetch(error: &str) -> bool {
  error.contains("(401")
    || error.contains("(403")
    || error.contains("No refresh token")
    || error.contains("Not logged in")
}

fn next_token(refused: Option<&str>, fetched: Result<Option<String>, String>) -> TokenStep {
  match fetched {
    Ok(Some(token)) if refused == Some(token.as_str()) => TokenStep::Stop,
    Ok(Some(token)) => TokenStep::Use(token),
    Ok(None) => TokenStep::Stop,
    Err(error) if is_refused_fetch(&error) => TokenStep::Stop,
    Err(_) if refused.is_some() => TokenStep::Wait,
    Err(_) => TokenStep::Keep,
  }
}

fn stream_failure(status: u16, headers: &reqwest::header::HeaderMap) -> StreamFailure {
  match status {
    401 | 403 => StreamFailure::Refused(status),
    429 => StreamFailure::RateLimited(crate::cloud_auth::retry_after(headers)),
    _ => StreamFailure::Failed(format!("status={status}")),
  }
}

async fn sleep_while_running(running: &AtomicBool, total: Duration) {
  let mut left = total;
  while !left.is_zero() && running.load(Ordering::SeqCst) {
    let step = left.min(STOP_POLL_INTERVAL);
    sleep(step).await;
    left = left.saturating_sub(step);
  }
}

pub struct SyncSubscription {
  client: Client,
  base_url: String,
  token: String,
  source: TokenSource,
  running: Arc<AtomicBool>,
  work_tx: mpsc::UnboundedSender<SyncWorkItem>,
}

impl SyncSubscription {
  fn new(
    base_url: String,
    token: String,
    source: TokenSource,
    work_tx: mpsc::UnboundedSender<SyncWorkItem>,
  ) -> Self {
    Self {
      client: Client::new(),
      base_url: base_url.trim_end_matches('/').to_string(),
      token,
      source,
      running: Arc::new(AtomicBool::new(false)),
      work_tx,
    }
  }

  pub async fn create_from_settings(
    app_handle: &tauri::AppHandle,
    work_tx: mpsc::UnboundedSender<SyncWorkItem>,
  ) -> Result<Option<Self>, String> {
    // Cloud auth takes priority
    if crate::cloud_auth::CLOUD_AUTH.is_logged_in().await {
      let url = crate::cloud_auth::CLOUD_SYNC_URL.to_string();
      let token = crate::cloud_auth::CLOUD_AUTH
        .get_or_refresh_sync_token()
        .await?;
      let Some(token) = token else {
        return Ok(None);
      };
      return Ok(Some(Self::new(url, token, TokenSource::Cloud, work_tx)));
    }

    // Fall back to self-hosted settings
    let manager = SettingsManager::instance();
    let settings = manager
      .load_settings()
      .map_err(|e| format!("Failed to load settings: {e}"))?;

    let Some(server_url) = settings.sync_server_url else {
      return Ok(None);
    };

    let token = manager
      .get_sync_token(app_handle)
      .await
      .map_err(|e| format!("Failed to get sync token: {e}"))?;

    let Some(token) = token else {
      return Ok(None);
    };

    Ok(Some(Self::new(
      server_url,
      token,
      TokenSource::SelfHosted,
      work_tx,
    )))
  }

  pub fn is_running(&self) -> bool {
    self.running.load(Ordering::SeqCst)
  }

  pub fn stop(&self) {
    self.running.store(false, Ordering::SeqCst);
  }

  pub async fn start(&self, app_handle: tauri::AppHandle) {
    if self.running.swap(true, Ordering::SeqCst) {
      return;
    }

    let running = self.running.clone();
    let base_url = self.base_url.clone();
    let source = self.source;
    let work_tx = self.work_tx.clone();
    let client = self.client.clone();
    let mut token = Some(self.token.clone());

    tokio::spawn(async move {
      let mut refused: Option<String> = None;
      let mut failures: u32 = 0;
      let mut stop_reason = "requested";

      while running.load(Ordering::SeqCst) {
        let mut wait_at_least = Duration::ZERO;

        if let Some(current) = token.clone() {
          let mut connected = false;
          let outcome = Self::connect_and_listen(
            &client,
            &base_url,
            &current,
            &work_tx,
            &running,
            &mut connected,
          )
          .await;
          if connected {
            failures = 0;
          }
          match outcome {
            Ok(()) => {
              log::debug!("Sync SSE stream closed by server");
            }
            Err(StreamFailure::Refused(status)) => {
              log::warn!("Sync subscription refused status={status}");
              if matches!(source, TokenSource::Cloud) {
                crate::cloud_auth::CloudAuthManager::discard_cloud_sync_token(&current);
              }
              refused = Some(current);
              token = None;
            }
            Err(StreamFailure::RateLimited(retry)) => {
              SSE_CONNECT.failed(format!("rate limited retry_after_s={}", retry.as_secs()));
              wait_at_least = retry;
            }
            Err(StreamFailure::Failed(e)) => {
              SSE_CONNECT.failed(e);
            }
          }
        }

        if !running.load(Ordering::SeqCst) {
          break;
        }

        failures = failures.saturating_add(1);
        let delay = reconnect_delay(failures).max(wait_at_least);
        log::debug!("Sync SSE reconnect scheduled delay_s={}", delay.as_secs());
        sleep_while_running(&running, delay).await;

        if running.load(Ordering::SeqCst) {
          // Refresh the sync token before reconnecting. The token may have
          // expired while the stream was open (tokens last ~15 min); reusing
          // the construction-time token otherwise produces an endless 401
          // reconnect loop until the app is restarted.
          let fetched = Self::fetch_sync_token(source, &app_handle).await;
          if matches!(source, TokenSource::SelfHosted) {
            match &fetched {
              Ok(_) => SELF_HOSTED_TOKEN.succeeded(),
              Err(e) => SELF_HOSTED_TOKEN.failed(e),
            }
          }
          match next_token(refused.as_deref(), fetched) {
            TokenStep::Use(fresh) => {
              token = Some(fresh);
              refused = None;
            }
            TokenStep::Keep | TokenStep::Wait => {}
            TokenStep::Stop => {
              stop_reason = "no_token";
              break;
            }
          }
        }
      }

      running.store(false, Ordering::SeqCst);
      log::info!("Sync subscription stopped reason={stop_reason}");
    });
  }

  /// Fetch a current sync token from the same source the subscription was
  /// created from, so reconnects never reuse a stale (expired) token.
  async fn fetch_sync_token(
    source: TokenSource,
    app_handle: &tauri::AppHandle,
  ) -> Result<Option<String>, String> {
    match source {
      TokenSource::Cloud => {
        crate::cloud_auth::CLOUD_AUTH
          .get_or_refresh_sync_token()
          .await
      }
      TokenSource::SelfHosted => SettingsManager::instance()
        .get_sync_token(app_handle)
        .await
        .map_err(|e| e.to_string()),
    }
  }

  async fn connect_and_listen(
    client: &Client,
    base_url: &str,
    token: &str,
    work_tx: &mpsc::UnboundedSender<SyncWorkItem>,
    running: &Arc<AtomicBool>,
    connected: &mut bool,
  ) -> Result<(), StreamFailure> {
    let url = format!("{base_url}/v1/objects/subscribe");

    let response = client
      .get(&url)
      .header("Authorization", format!("Bearer {token}"))
      .header("Accept", "text/event-stream")
      .send()
      .await
      .map_err(|e| StreamFailure::Failed(e.to_string()))?;

    if !response.status().is_success() {
      return Err(stream_failure(
        response.status().as_u16(),
        response.headers(),
      ));
    }

    *connected = true;
    log::debug!("Sync SSE connected");
    let _ = events::emit("sync-subscription-status", "connected");

    let mut buffer = String::new();
    let mut bytes_stream = response.bytes_stream();
    // A proxy can accept the connection and cut it before any data; only data
    // proves the link works, so that is what ends a failure streak.
    let mut proven = false;

    use futures_util::StreamExt;

    while running.load(Ordering::SeqCst) {
      match tokio::time::timeout(Duration::from_secs(60), bytes_stream.next()).await {
        Ok(Some(Ok(bytes))) => {
          if !proven {
            proven = true;
            SSE_CONNECT.succeeded();
          }
          let chunk = String::from_utf8_lossy(&bytes);
          buffer.push_str(&chunk);

          while let Some(event_end) = buffer.find("\n\n") {
            let event_str = buffer[..event_end].to_string();
            buffer = buffer[event_end + 2..].to_string();

            if let Some(event) = Self::parse_sse_event(&event_str) {
              Self::handle_event(&event, work_tx);
            }
          }
        }
        Ok(Some(Err(e))) => {
          return Err(StreamFailure::Failed(format!("stream: {e}")));
        }
        Ok(None) => {
          return Ok(());
        }
        Err(_) => {
          log::debug!("Sync SSE idle for 60s");
        }
      }
    }

    Ok(())
  }

  fn parse_sse_event(event_str: &str) -> Option<SubscribeEvent> {
    let mut data_line = None;

    for line in event_str.lines() {
      if let Some(data) = line.strip_prefix("data:") {
        data_line = Some(data.trim());
      }
    }

    data_line.and_then(|data| serde_json::from_str(data).ok())
  }

  fn strip_team_prefix(key: &str) -> &str {
    if key.starts_with("teams/") {
      if let Some(rest) = key.find('/').and_then(|first_slash| {
        key[first_slash + 1..]
          .find('/')
          .map(|second_slash| first_slash + 1 + second_slash + 1)
      }) {
        return &key[rest..];
      }
    }
    key
  }

  fn handle_event(event: &SubscribeEvent, work_tx: &mpsc::UnboundedSender<SyncWorkItem>) {
    let Some(raw_key) = &event.key else {
      return;
    };

    if event.event_type == "ping" {
      return;
    }

    let key = Self::strip_team_prefix(raw_key);

    let work_item = if key.starts_with("profiles/") {
      // Match both bundle uploads (profiles/{id}.tar.gz) and delta sync updates
      // (profiles/{id}/manifest.json, profiles/{id}/files/*, profiles/{id}/metadata.json)
      let profile_id = key.strip_prefix("profiles/").and_then(|rest| {
        // profiles/{id}.tar.gz → id
        rest
          .strip_suffix(".tar.gz")
          // profiles/{id}/manifest.json → id
          .or_else(|| rest.split('/').next().filter(|s| !s.is_empty()))
      });
      profile_id.map(|s| SyncWorkItem::Profile(s.to_string()))
    } else if key.starts_with("proxies/") {
      key
        .strip_prefix("proxies/")
        .and_then(|s| s.strip_suffix(".json"))
        .map(|s| SyncWorkItem::Proxy(s.to_string()))
    } else if key.starts_with("groups/") {
      key
        .strip_prefix("groups/")
        .and_then(|s| s.strip_suffix(".json"))
        .map(|s| SyncWorkItem::Group(s.to_string()))
    } else if key.starts_with("vpns/") {
      key
        .strip_prefix("vpns/")
        .and_then(|s| s.strip_suffix(".json"))
        .map(|s| SyncWorkItem::Vpn(s.to_string()))
    } else if key.starts_with("extensions/") {
      key
        .strip_prefix("extensions/")
        .and_then(|s| s.strip_suffix(".json"))
        .map(|s| SyncWorkItem::Extension(s.to_string()))
    } else if key.starts_with("extension_groups/") {
      key
        .strip_prefix("extension_groups/")
        .and_then(|s| s.strip_suffix(".json"))
        .map(|s| SyncWorkItem::ExtensionGroup(s.to_string()))
    } else if key.starts_with("tombstones/") {
      key.strip_prefix("tombstones/").and_then(|rest| {
        if rest.starts_with("profiles/") {
          rest
            .strip_prefix("profiles/")
            .and_then(|s| s.strip_suffix(".json"))
            .map(|id| SyncWorkItem::Tombstone("profile".to_string(), id.to_string()))
        } else if rest.starts_with("proxies/") {
          rest
            .strip_prefix("proxies/")
            .and_then(|s| s.strip_suffix(".json"))
            .map(|id| SyncWorkItem::Tombstone("proxy".to_string(), id.to_string()))
        } else if rest.starts_with("groups/") {
          rest
            .strip_prefix("groups/")
            .and_then(|s| s.strip_suffix(".json"))
            .map(|id| SyncWorkItem::Tombstone("group".to_string(), id.to_string()))
        } else if rest.starts_with("vpns/") {
          rest
            .strip_prefix("vpns/")
            .and_then(|s| s.strip_suffix(".json"))
            .map(|id| SyncWorkItem::Tombstone("vpn".to_string(), id.to_string()))
        } else if rest.starts_with("extensions/") {
          rest
            .strip_prefix("extensions/")
            .and_then(|s| s.strip_suffix(".json"))
            .map(|id| SyncWorkItem::Tombstone("extension".to_string(), id.to_string()))
        } else if rest.starts_with("extension_groups/") {
          rest
            .strip_prefix("extension_groups/")
            .and_then(|s| s.strip_suffix(".json"))
            .map(|id| SyncWorkItem::Tombstone("extension_group".to_string(), id.to_string()))
        } else {
          None
        }
      })
    } else {
      None
    };

    if let Some(item) = work_item {
      log::debug!("Sync work queued item={item:?}");
      let _ = work_tx.send(item);
    }
  }
}

pub struct SubscriptionManager {
  subscription: Option<SyncSubscription>,
  work_tx: mpsc::UnboundedSender<SyncWorkItem>,
  work_rx: Option<mpsc::UnboundedReceiver<SyncWorkItem>>,
}

impl Default for SubscriptionManager {
  fn default() -> Self {
    Self::new()
  }
}

impl SubscriptionManager {
  pub fn new() -> Self {
    let (work_tx, work_rx) = mpsc::unbounded_channel();
    Self {
      subscription: None,
      work_tx,
      work_rx: Some(work_rx),
    }
  }

  pub fn get_work_sender(&self) -> mpsc::UnboundedSender<SyncWorkItem> {
    self.work_tx.clone()
  }

  pub fn take_work_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<SyncWorkItem>> {
    self.work_rx.take()
  }

  pub async fn start(&mut self, app_handle: tauri::AppHandle) -> Result<(), String> {
    if self.is_running() {
      return Ok(());
    }
    self.subscription = None;

    let subscription =
      SyncSubscription::create_from_settings(&app_handle, self.work_tx.clone()).await?;

    if let Some(sub) = subscription {
      sub.start(app_handle).await;
      self.subscription = Some(sub);
      log::info!("Sync subscription started");
    } else {
      log::debug!("Sync subscription not started: sync not configured");
    }

    Ok(())
  }

  pub fn stop(&mut self) {
    if let Some(sub) = &self.subscription {
      sub.stop();
    }
    self.subscription = None;
  }

  pub fn is_running(&self) -> bool {
    self.subscription.as_ref().is_some_and(|s| s.is_running())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reconnects_back_off_from_six_seconds_to_ten_minutes() {
    let delays: Vec<u64> = (1..=10).map(|n| reconnect_delay(n).as_secs()).collect();
    assert_eq!(delays, vec![6, 12, 24, 48, 96, 192, 384, 600, 600, 600]);
    assert_eq!(reconnect_delay(0), Duration::from_secs(6));
    assert_eq!(reconnect_delay(u32::MAX), Duration::from_secs(600));
  }

  #[test]
  fn a_refused_token_is_never_sent_again() {
    assert_eq!(
      next_token(Some("old"), Ok(Some("old".to_string()))),
      TokenStep::Stop
    );
    assert_eq!(
      next_token(Some("old"), Ok(Some("new".to_string()))),
      TokenStep::Use("new".to_string())
    );
    assert_eq!(
      next_token(
        Some("old"),
        Err("Failed to get sync token: timed out".to_string())
      ),
      TokenStep::Wait
    );
  }

  #[test]
  fn a_refused_refresh_stops_the_subscription() {
    for refusal in [
      "Failed to refresh cloud sync token: Token refresh failed (401 Unauthorized)",
      "Failed to refresh cloud sync token: Sync token request failed (403 Forbidden): {}",
      "Failed to refresh cloud sync token: No refresh token stored",
      "Failed to refresh cloud sync token: Not logged in",
    ] {
      assert_eq!(next_token(None, Err(refusal.to_string())), TokenStep::Stop);
      assert_eq!(
        next_token(Some("old"), Err(refusal.to_string())),
        TokenStep::Stop
      );
    }
    assert_eq!(next_token(None, Ok(None)), TokenStep::Stop);
  }

  #[test]
  fn a_transient_refresh_failure_keeps_a_token_the_server_still_accepts() {
    assert_eq!(
      next_token(
        None,
        Err("Failed to get sync token: connection reset".to_string())
      ),
      TokenStep::Keep
    );
    assert_eq!(
      next_token(None, Ok(Some("same".to_string()))),
      TokenStep::Use("same".to_string())
    );
  }

  #[test]
  fn the_stream_status_decides_refusal_and_rate_limits() {
    let mut headers = reqwest::header::HeaderMap::new();
    assert!(matches!(
      stream_failure(401, &headers),
      StreamFailure::Refused(401)
    ));
    assert!(matches!(
      stream_failure(403, &headers),
      StreamFailure::Refused(403)
    ));
    assert!(matches!(
      stream_failure(502, &headers),
      StreamFailure::Failed(_)
    ));
    headers.insert(reqwest::header::RETRY_AFTER, "900".parse().unwrap());
    match stream_failure(429, &headers) {
      StreamFailure::RateLimited(wait) => assert_eq!(wait, Duration::from_secs(900)),
      other => panic!("expected a rate limit, got {other:?}"),
    }
  }

  #[tokio::test]
  async fn a_stopped_subscription_ends_its_wait_promptly() {
    let running = AtomicBool::new(false);
    let started = std::time::Instant::now();
    sleep_while_running(&running, Duration::from_secs(600)).await;
    assert!(started.elapsed() < Duration::from_millis(100));
  }
}

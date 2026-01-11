use crate::events;
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
  Tombstone(String, String),
}

pub struct SyncSubscription {
  client: Client,
  base_url: String,
  token: String,
  running: Arc<AtomicBool>,
  work_tx: mpsc::UnboundedSender<SyncWorkItem>,
}

impl SyncSubscription {
  pub fn new(
    base_url: String,
    token: String,
    work_tx: mpsc::UnboundedSender<SyncWorkItem>,
  ) -> Self {
    Self {
      client: Client::new(),
      base_url: base_url.trim_end_matches('/').to_string(),
      token,
      running: Arc::new(AtomicBool::new(false)),
      work_tx,
    }
  }

  pub async fn create_from_settings(
    app_handle: &tauri::AppHandle,
    work_tx: mpsc::UnboundedSender<SyncWorkItem>,
  ) -> Result<Option<Self>, String> {
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

    Ok(Some(Self::new(server_url, token, work_tx)))
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
    let token = self.token.clone();
    let work_tx = self.work_tx.clone();
    let client = self.client.clone();

    tokio::spawn(async move {
      while running.load(Ordering::SeqCst) {
        match Self::connect_and_listen(&client, &base_url, &token, &work_tx, &running, &app_handle)
          .await
        {
          Ok(()) => {
            log::info!("SSE connection closed gracefully");
          }
          Err(e) => {
            log::warn!("SSE connection error: {e}, reconnecting in 5s");
            sleep(Duration::from_secs(5)).await;
          }
        }

        if running.load(Ordering::SeqCst) {
          sleep(Duration::from_secs(1)).await;
        }
      }

      log::info!("Sync subscription stopped");
    });
  }

  async fn connect_and_listen(
    client: &Client,
    base_url: &str,
    token: &str,
    work_tx: &mpsc::UnboundedSender<SyncWorkItem>,
    running: &Arc<AtomicBool>,
    _app_handle: &tauri::AppHandle,
  ) -> Result<(), String> {
    let url = format!("{base_url}/v1/objects/subscribe");

    let response = client
      .get(&url)
      .header("Authorization", format!("Bearer {token}"))
      .header("Accept", "text/event-stream")
      .send()
      .await
      .map_err(|e| format!("Failed to connect to SSE: {e}"))?;

    if !response.status().is_success() {
      return Err(format!(
        "SSE connection failed with status: {}",
        response.status()
      ));
    }

    log::info!("Connected to sync subscription at {url}");
    let _ = events::emit("sync-subscription-status", "connected");

    let mut buffer = String::new();
    let mut bytes_stream = response.bytes_stream();

    use futures_util::StreamExt;

    while running.load(Ordering::SeqCst) {
      match tokio::time::timeout(Duration::from_secs(60), bytes_stream.next()).await {
        Ok(Some(Ok(bytes))) => {
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
          return Err(format!("SSE stream error: {e}"));
        }
        Ok(None) => {
          return Ok(());
        }
        Err(_) => {
          log::debug!("SSE timeout, continuing...");
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

  fn handle_event(event: &SubscribeEvent, work_tx: &mpsc::UnboundedSender<SyncWorkItem>) {
    let Some(key) = &event.key else {
      return;
    };

    if event.event_type == "ping" {
      return;
    }

    let work_item = if key.starts_with("profiles/") {
      key
        .strip_prefix("profiles/")
        .and_then(|s| s.strip_suffix(".tar.gz"))
        .map(|s| SyncWorkItem::Profile(s.to_string()))
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
        } else {
          None
        }
      })
    } else {
      None
    };

    if let Some(item) = work_item {
      log::debug!("Queueing sync work: {:?}", item);
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
    if self.subscription.is_some() {
      return Ok(());
    }

    let subscription =
      SyncSubscription::create_from_settings(&app_handle, self.work_tx.clone()).await?;

    if let Some(sub) = subscription {
      sub.start(app_handle).await;
      self.subscription = Some(sub);
      log::info!("Sync subscription manager started");
    } else {
      log::debug!("Sync not configured, subscription not started");
    }

    Ok(())
  }

  pub fn stop(&mut self) {
    if let Some(sub) = &self.subscription {
      sub.stop();
    }
    self.subscription = None;
    log::info!("Sync subscription manager stopped");
  }

  pub fn is_running(&self) -> bool {
    self.subscription.as_ref().is_some_and(|s| s.is_running())
  }
}

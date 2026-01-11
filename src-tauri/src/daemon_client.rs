use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
  #[serde(rename = "type")]
  pub msg_type: String,
  pub event: Option<String>,
  pub payload: Option<serde_json::Value>,
}

pub struct DaemonClient {
  app_handle: tauri::AppHandle,
  connected: Arc<AtomicBool>,
  shutdown: Arc<AtomicBool>,
  daemon_port: Arc<Mutex<Option<u16>>>,
}

impl DaemonClient {
  pub fn new(app_handle: tauri::AppHandle) -> Self {
    Self {
      app_handle,
      connected: Arc::new(AtomicBool::new(false)),
      shutdown: Arc::new(AtomicBool::new(false)),
      daemon_port: Arc::new(Mutex::new(None)),
    }
  }

  pub fn is_connected(&self) -> bool {
    self.connected.load(Ordering::SeqCst)
  }

  pub async fn connect(&self, port: u16) -> Result<(), String> {
    *self.daemon_port.lock().await = Some(port);

    let url = format!("ws://127.0.0.1:{}/ws/events", port);

    log::info!("[daemon-client] Connecting to daemon at {}", url);

    let (ws_stream, _) = connect_async(&url)
      .await
      .map_err(|e| format!("Failed to connect to daemon: {}", e))?;

    self.connected.store(true, Ordering::SeqCst);
    log::info!("[daemon-client] Connected to daemon");

    let (mut write, mut read) = ws_stream.split();

    let app_handle = self.app_handle.clone();
    let connected = self.connected.clone();
    let shutdown = self.shutdown.clone();

    // Spawn task to handle incoming messages
    tokio::spawn(async move {
      while !shutdown.load(Ordering::SeqCst) {
        match read.next().await {
          Some(Ok(Message::Text(text))) => {
            if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
              match ws_msg.msg_type.as_str() {
                "event" => {
                  if let (Some(event), Some(payload)) = (ws_msg.event, ws_msg.payload) {
                    // Forward event to Tauri frontend
                    if let Err(e) = app_handle.emit(&event, payload) {
                      log::error!("[daemon-client] Failed to emit event: {}", e);
                    }
                  }
                }
                "connected" => {
                  log::info!("[daemon-client] Received connection confirmation");
                }
                "pong" => {
                  log::debug!("[daemon-client] Received pong");
                }
                _ => {
                  log::debug!("[daemon-client] Unknown message type: {}", ws_msg.msg_type);
                }
              }
            }
          }
          Some(Ok(Message::Ping(data))) => {
            log::debug!("[daemon-client] Received ping");
            if let Err(e) = write.send(Message::Pong(data)).await {
              log::error!("[daemon-client] Failed to send pong: {}", e);
              break;
            }
          }
          Some(Ok(Message::Close(_))) => {
            log::info!("[daemon-client] Daemon closed connection");
            break;
          }
          Some(Err(e)) => {
            log::error!("[daemon-client] WebSocket error: {}", e);
            break;
          }
          None => {
            log::info!("[daemon-client] WebSocket stream ended");
            break;
          }
          _ => {}
        }
      }

      connected.store(false, Ordering::SeqCst);
      log::info!("[daemon-client] Disconnected from daemon");
    });

    Ok(())
  }

  pub fn disconnect(&self) {
    self.shutdown.store(true, Ordering::SeqCst);
    self.connected.store(false, Ordering::SeqCst);
  }
}

pub async fn start_daemon_connection(app_handle: tauri::AppHandle, port: u16) -> DaemonClient {
  let client = DaemonClient::new(app_handle);

  if let Err(e) = client.connect(port).await {
    log::error!("[daemon-client] Failed to connect: {}", e);
  }

  client
}

pub async fn find_and_connect_to_daemon(app_handle: tauri::AppHandle) -> Option<DaemonClient> {
  // Try default port first
  let default_port = 10108;

  log::info!(
    "[daemon-client] Looking for daemon on port {}",
    default_port
  );

  let client = DaemonClient::new(app_handle);

  match client.connect(default_port).await {
    Ok(()) => Some(client),
    Err(e) => {
      log::warn!(
        "[daemon-client] Could not connect to daemon on default port: {}",
        e
      );
      None
    }
  }
}

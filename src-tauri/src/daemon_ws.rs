use axum::{
  extract::{
    ws::{Message, WebSocket, WebSocketUpgrade},
    State,
  },
  response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::events::{DaemonEmitter, DaemonEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
  #[serde(rename = "type")]
  pub msg_type: String,
  pub event: Option<String>,
  pub payload: Option<serde_json::Value>,
}

#[derive(Clone)]
pub struct WsState {
  event_emitter: Option<Arc<DaemonEmitter>>,
}

impl WsState {
  pub fn new() -> Self {
    Self {
      event_emitter: None,
    }
  }

  pub fn with_emitter(emitter: Arc<DaemonEmitter>) -> Self {
    Self {
      event_emitter: Some(emitter),
    }
  }
}

impl Default for WsState {
  fn default() -> Self {
    Self::new()
  }
}

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WsState>) -> impl IntoResponse {
  ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: WsState) {
  let (mut sender, mut receiver) = socket.split();

  // Subscribe to daemon events if emitter is available
  let mut event_rx = state.event_emitter.as_ref().map(|e| e.subscribe());

  log::info!("[ws] Client connected");

  // Send initial ping to confirm connection
  let ping_msg = WsMessage {
    msg_type: "connected".to_string(),
    event: None,
    payload: None,
  };
  if let Ok(msg_str) = serde_json::to_string(&ping_msg) {
    let _ = sender.send(Message::Text(msg_str.into())).await;
  }

  loop {
    tokio::select! {
      // Handle incoming messages from client
      Some(msg) = receiver.next() => {
        match msg {
          Ok(Message::Text(text)) => {
            if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
              match ws_msg.msg_type.as_str() {
                "ping" => {
                  let pong = WsMessage {
                    msg_type: "pong".to_string(),
                    event: None,
                    payload: None,
                  };
                  if let Ok(msg_str) = serde_json::to_string(&pong) {
                    let _ = sender.send(Message::Text(msg_str.into())).await;
                  }
                }
                _ => {
                  log::debug!("[ws] Received unknown message type: {}", ws_msg.msg_type);
                }
              }
            }
          }
          Ok(Message::Ping(data)) => {
            let _ = sender.send(Message::Pong(data)).await;
          }
          Ok(Message::Close(_)) => {
            log::info!("[ws] Client disconnected");
            break;
          }
          Err(e) => {
            log::error!("[ws] Error receiving message: {}", e);
            break;
          }
          _ => {}
        }
      }

      // Forward daemon events to client
      Some(daemon_event) = async {
        if let Some(ref mut rx) = event_rx {
          rx.recv().await.ok()
        } else {
          std::future::pending::<Option<DaemonEvent>>().await
        }
      } => {
        let ws_msg = WsMessage {
          msg_type: "event".to_string(),
          event: Some(daemon_event.event_type),
          payload: Some(daemon_event.payload),
        };
        if let Ok(msg_str) = serde_json::to_string(&ws_msg) {
          if sender.send(Message::Text(msg_str.into())).await.is_err() {
            log::error!("[ws] Failed to send event to client");
            break;
          }
        }
      }

      else => break,
    }
  }

  log::info!("[ws] WebSocket connection closed");
}

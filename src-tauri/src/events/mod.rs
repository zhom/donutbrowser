use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Trait for emitting events to the frontend or connected clients.
/// This abstraction allows the same code to work in both GUI (Tauri) mode
/// and daemon mode (WebSocket broadcast).
///
/// Note: This trait uses `serde_json::Value` to be dyn-compatible.
/// Use the convenience functions `emit()` and `emit_empty()` which accept
/// any Serialize type.
pub trait EventEmitter: Send + Sync {
  /// Emit an event with a JSON value payload.
  fn emit_value(&self, event: &str, payload: serde_json::Value) -> Result<(), String>;
}

/// Tauri-based event emitter for GUI mode.
/// Wraps an AppHandle and emits events directly to the Tauri frontend.
#[derive(Clone)]
pub struct TauriEmitter {
  app_handle: tauri::AppHandle,
}

impl TauriEmitter {
  pub fn new(app_handle: tauri::AppHandle) -> Self {
    Self { app_handle }
  }
}

impl EventEmitter for TauriEmitter {
  fn emit_value(&self, event: &str, payload: serde_json::Value) -> Result<(), String> {
    use tauri::Emitter;
    self
      .app_handle
      .emit(event, payload)
      .map_err(|e| e.to_string())
  }
}

/// Event message sent through the daemon's broadcast channel.
#[derive(Clone, Debug)]
pub struct DaemonEvent {
  pub event_type: String,
  pub payload: serde_json::Value,
}

/// Daemon-based event emitter for background daemon mode.
/// Broadcasts events to all connected WebSocket clients.
#[derive(Clone)]
pub struct DaemonEmitter {
  tx: broadcast::Sender<DaemonEvent>,
}

impl DaemonEmitter {
  pub fn new(tx: broadcast::Sender<DaemonEvent>) -> Self {
    Self { tx }
  }

  /// Create a new DaemonEmitter with a default channel capacity.
  pub fn with_capacity(capacity: usize) -> (Self, broadcast::Receiver<DaemonEvent>) {
    let (tx, rx) = broadcast::channel(capacity);
    (Self { tx }, rx)
  }

  /// Subscribe to events from this emitter.
  pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
    self.tx.subscribe()
  }
}

impl EventEmitter for DaemonEmitter {
  fn emit_value(&self, event: &str, payload: serde_json::Value) -> Result<(), String> {
    let daemon_event = DaemonEvent {
      event_type: event.to_string(),
      payload,
    };
    // Ignore send errors (no receivers connected)
    let _ = self.tx.send(daemon_event);
    Ok(())
  }
}

/// No-op emitter for testing or when events are not needed.
#[derive(Clone, Default)]
pub struct NoopEmitter;

impl EventEmitter for NoopEmitter {
  fn emit_value(&self, _event: &str, _payload: serde_json::Value) -> Result<(), String> {
    Ok(())
  }
}

/// Global event emitter that can be set at runtime.
/// This allows managers to emit events without knowing whether they're
/// running in GUI or daemon mode.
static GLOBAL_EMITTER: std::sync::OnceLock<Arc<dyn EventEmitter>> = std::sync::OnceLock::new();

/// Set the global event emitter. This should be called once during app startup.
/// Returns an error if the emitter has already been set.
pub fn set_global_emitter(emitter: Arc<dyn EventEmitter>) -> Result<(), String> {
  GLOBAL_EMITTER
    .set(emitter)
    .map_err(|_| "Global emitter already set".to_string())
}

/// Get the global event emitter, or a no-op emitter if none has been set.
pub fn global_emitter() -> Arc<dyn EventEmitter> {
  GLOBAL_EMITTER
    .get()
    .cloned()
    .unwrap_or_else(|| Arc::new(NoopEmitter))
}

/// Emit an event using the global emitter.
/// This is a convenience function for use in managers.
/// Accepts any type that implements Serialize.
pub fn emit<S: Serialize>(event: &str, payload: S) -> Result<(), String> {
  let value = serde_json::to_value(payload).map_err(|e| e.to_string())?;
  global_emitter().emit_value(event, value)
}

/// Emit an event with no payload using the global emitter.
pub fn emit_empty(event: &str) -> Result<(), String> {
  global_emitter().emit_value(event, serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_noop_emitter() {
    let emitter = NoopEmitter;
    assert!(emitter
      .emit_value("test-event", serde_json::json!("payload"))
      .is_ok());
  }

  #[test]
  fn test_daemon_emitter() {
    let (emitter, mut rx) = DaemonEmitter::with_capacity(16);

    // Emit an event
    let _ = emitter.emit_value("test-event", serde_json::json!("hello"));

    // Check we received it
    let event = rx.try_recv().unwrap();
    assert_eq!(event.event_type, "test-event");
    assert_eq!(event.payload, serde_json::json!("hello"));
  }

  #[test]
  fn test_daemon_emitter_no_receivers() {
    let (tx, _) = broadcast::channel::<DaemonEvent>(16);
    let emitter = DaemonEmitter::new(tx);

    // Should not error even with no receivers
    assert!(emitter
      .emit_value("test-event", serde_json::json!("hello"))
      .is_ok());
  }

  #[test]
  fn test_emit_convenience_function() {
    // Test that emit() works with various types
    assert!(emit("test", "string").is_ok());
    assert!(emit("test", 42).is_ok());
    assert!(emit("test", serde_json::json!({"key": "value"})).is_ok());
    assert!(emit_empty("test").is_ok());
  }
}

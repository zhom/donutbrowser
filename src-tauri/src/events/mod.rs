use serde::Serialize;
use std::sync::Arc;

/// Trait for emitting events to the frontend.
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

/// No-op emitter for testing or when events are not needed.
#[derive(Clone, Default)]
pub struct NoopEmitter;

impl EventEmitter for NoopEmitter {
  fn emit_value(&self, _event: &str, _payload: serde_json::Value) -> Result<(), String> {
    Ok(())
  }
}

/// Global event emitter that can be set at runtime.
/// This allows managers to emit events without holding an AppHandle directly.
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
  fn test_emit_convenience_function() {
    // Test that emit() works with various types
    assert!(emit("test", "string").is_ok());
    assert!(emit("test", 42).is_ok());
    assert!(emit("test", serde_json::json!({"key": "value"})).is_ok());
    assert!(emit_empty("test").is_ok());
  }
}

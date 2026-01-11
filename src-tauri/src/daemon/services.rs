use crate::events::{self, DaemonEmitter, DaemonEvent};
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct DaemonServices {
  pub api_port: Option<u16>,
  pub mcp_running: bool,
  event_emitter: Arc<DaemonEmitter>,
}

impl DaemonServices {
  pub async fn start() -> Result<Self, String> {
    log::info!("Starting daemon services...");

    // Create the daemon event emitter
    let (emitter, _rx) = DaemonEmitter::with_capacity(256);
    let emitter_arc = Arc::new(emitter);

    // Set the global event emitter
    if let Err(e) = events::set_global_emitter(emitter_arc.clone()) {
      log::warn!("Failed to set global event emitter: {}", e);
    }

    // NOTE: The API server currently requires an AppHandle which is only available
    // in the Tauri GUI context. For now, the daemon starts with minimal services.
    // The GUI will start the API server when it connects to the daemon.
    //
    // TODO: Refactor API server to work without AppHandle for daemon mode
    let api_port = None;
    let mcp_running = false;

    log::info!("Daemon services started (minimal mode - waiting for GUI connection)");

    Ok(Self {
      api_port,
      mcp_running,
      event_emitter: emitter_arc,
    })
  }

  pub fn subscribe_events(&self) -> broadcast::Receiver<DaemonEvent> {
    self.event_emitter.subscribe()
  }

  pub async fn stop(&mut self) {
    log::info!("Stopping daemon services...");

    self.api_port = None;
    self.mcp_running = false;
  }
}

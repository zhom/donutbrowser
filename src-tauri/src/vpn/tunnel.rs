//! VPN tunnel trait and management.

use super::config::{VpnError, VpnStatus};
use async_trait::async_trait;
use std::collections::HashMap;

/// Trait for VPN tunnel implementations
#[async_trait]
pub trait VpnTunnel: Send + Sync {
  /// Connect the VPN tunnel
  async fn connect(&mut self) -> Result<(), VpnError>;

  /// Disconnect the VPN tunnel
  async fn disconnect(&mut self) -> Result<(), VpnError>;

  /// Check if the tunnel is connected
  fn is_connected(&self) -> bool;

  /// Get the VPN config ID
  fn vpn_id(&self) -> &str;

  /// Get the current status of the tunnel
  fn get_status(&self) -> VpnStatus;

  /// Get bytes sent through the tunnel
  fn bytes_sent(&self) -> u64;

  /// Get bytes received through the tunnel
  fn bytes_received(&self) -> u64;
}

/// Manager for active VPN tunnels
pub struct TunnelManager {
  active_tunnels: HashMap<String, Box<dyn VpnTunnel>>,
}

impl Default for TunnelManager {
  fn default() -> Self {
    Self::new()
  }
}

impl TunnelManager {
  /// Create a new tunnel manager
  pub fn new() -> Self {
    Self {
      active_tunnels: HashMap::new(),
    }
  }

  /// Register an active tunnel
  pub fn register_tunnel(&mut self, vpn_id: String, tunnel: Box<dyn VpnTunnel>) {
    self.active_tunnels.insert(vpn_id, tunnel);
  }

  /// Remove a tunnel from management
  pub fn remove_tunnel(&mut self, vpn_id: &str) -> Option<Box<dyn VpnTunnel>> {
    self.active_tunnels.remove(vpn_id)
  }

  /// Get a reference to an active tunnel
  pub fn get_tunnel(&self, vpn_id: &str) -> Option<&dyn VpnTunnel> {
    self.active_tunnels.get(vpn_id).map(|t| t.as_ref())
  }

  /// Get a mutable reference to an active tunnel
  pub fn get_tunnel_mut(&mut self, vpn_id: &str) -> Option<&mut Box<dyn VpnTunnel>> {
    self.active_tunnels.get_mut(vpn_id)
  }

  /// Check if a tunnel is active
  pub fn is_tunnel_active(&self, vpn_id: &str) -> bool {
    self
      .active_tunnels
      .get(vpn_id)
      .is_some_and(|t| t.is_connected())
  }

  /// Get status of all active tunnels
  pub fn get_all_statuses(&self) -> Vec<VpnStatus> {
    self
      .active_tunnels
      .values()
      .map(|t| t.get_status())
      .collect()
  }

  /// Disconnect all active tunnels
  pub async fn disconnect_all(&mut self) -> Vec<Result<(), VpnError>> {
    let mut results = Vec::new();

    for tunnel in self.active_tunnels.values_mut() {
      results.push(tunnel.disconnect().await);
    }

    self.active_tunnels.clear();
    results
  }

  /// Get the number of active tunnels
  pub fn active_count(&self) -> usize {
    self
      .active_tunnels
      .values()
      .filter(|t| t.is_connected())
      .count()
  }

  /// List IDs of all active VPN connections
  pub fn list_active_ids(&self) -> Vec<String> {
    self
      .active_tunnels
      .iter()
      .filter(|(_, t)| t.is_connected())
      .map(|(id, _)| id.clone())
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  struct MockTunnel {
    id: String,
    connected: bool,
    bytes_sent: u64,
    bytes_received: u64,
  }

  #[async_trait]
  impl VpnTunnel for MockTunnel {
    async fn connect(&mut self) -> Result<(), VpnError> {
      self.connected = true;
      Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), VpnError> {
      self.connected = false;
      Ok(())
    }

    fn is_connected(&self) -> bool {
      self.connected
    }

    fn vpn_id(&self) -> &str {
      &self.id
    }

    fn get_status(&self) -> VpnStatus {
      VpnStatus {
        connected: self.connected,
        vpn_id: self.id.clone(),
        connected_at: if self.connected { Some(1000) } else { None },
        bytes_sent: Some(self.bytes_sent),
        bytes_received: Some(self.bytes_received),
        last_handshake: None,
      }
    }

    fn bytes_sent(&self) -> u64 {
      self.bytes_sent
    }

    fn bytes_received(&self) -> u64 {
      self.bytes_received
    }
  }

  #[test]
  fn test_tunnel_manager_register() {
    let mut manager = TunnelManager::new();
    let tunnel = Box::new(MockTunnel {
      id: "test-1".to_string(),
      connected: true,
      bytes_sent: 100,
      bytes_received: 200,
    });

    manager.register_tunnel("test-1".to_string(), tunnel);
    assert!(manager.is_tunnel_active("test-1"));
    assert!(!manager.is_tunnel_active("test-2"));
  }

  #[test]
  fn test_tunnel_manager_remove() {
    let mut manager = TunnelManager::new();
    let tunnel = Box::new(MockTunnel {
      id: "test-1".to_string(),
      connected: true,
      bytes_sent: 0,
      bytes_received: 0,
    });

    manager.register_tunnel("test-1".to_string(), tunnel);
    assert!(manager.is_tunnel_active("test-1"));

    let removed = manager.remove_tunnel("test-1");
    assert!(removed.is_some());
    assert!(!manager.is_tunnel_active("test-1"));
  }

  #[test]
  fn test_tunnel_manager_active_count() {
    let mut manager = TunnelManager::new();

    let tunnel1 = Box::new(MockTunnel {
      id: "t1".to_string(),
      connected: true,
      bytes_sent: 0,
      bytes_received: 0,
    });

    let tunnel2 = Box::new(MockTunnel {
      id: "t2".to_string(),
      connected: false,
      bytes_sent: 0,
      bytes_received: 0,
    });

    manager.register_tunnel("t1".to_string(), tunnel1);
    manager.register_tunnel("t2".to_string(), tunnel2);

    assert_eq!(manager.active_count(), 1);
  }

  #[tokio::test]
  async fn test_tunnel_manager_disconnect_all() {
    let mut manager = TunnelManager::new();

    let tunnel1 = Box::new(MockTunnel {
      id: "t1".to_string(),
      connected: true,
      bytes_sent: 0,
      bytes_received: 0,
    });

    let tunnel2 = Box::new(MockTunnel {
      id: "t2".to_string(),
      connected: true,
      bytes_sent: 0,
      bytes_received: 0,
    });

    manager.register_tunnel("t1".to_string(), tunnel1);
    manager.register_tunnel("t2".to_string(), tunnel2);

    assert_eq!(manager.active_count(), 2);

    let results = manager.disconnect_all().await;
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.is_ok()));
    assert_eq!(manager.active_count(), 0);
  }
}

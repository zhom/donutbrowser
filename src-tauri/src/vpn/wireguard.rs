//! WireGuard tunnel implementation using boringtun.

use super::config::{VpnError, VpnStatus, WireGuardConfig};
use super::tunnel::VpnTunnel;
use async_trait::async_trait;
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use chrono::Utc;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// WireGuard tunnel implementation
pub struct WireGuardTunnel {
  vpn_id: String,
  config: WireGuardConfig,
  tunnel: Option<Arc<Mutex<Box<Tunn>>>>,
  socket: Option<Arc<UdpSocket>>,
  connected: AtomicBool,
  connected_at: Option<i64>,
  bytes_sent: AtomicU64,
  bytes_received: AtomicU64,
  last_handshake: Option<i64>,
  peer_addr: Option<SocketAddr>,
}

impl WireGuardTunnel {
  /// Create a new WireGuard tunnel
  pub fn new(vpn_id: String, config: WireGuardConfig) -> Self {
    Self {
      vpn_id,
      config,
      tunnel: None,
      socket: None,
      connected: AtomicBool::new(false),
      connected_at: None,
      bytes_sent: AtomicU64::new(0),
      bytes_received: AtomicU64::new(0),
      last_handshake: None,
      peer_addr: None,
    }
  }

  /// Parse base64 key to bytes
  fn parse_key(key: &str) -> Result<[u8; 32], VpnError> {
    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, key)
      .map_err(|e| VpnError::InvalidWireGuard(format!("Invalid key encoding: {e}")))?;

    if decoded.len() != 32 {
      return Err(VpnError::InvalidWireGuard(format!(
        "Invalid key length: {} (expected 32)",
        decoded.len()
      )));
    }

    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&decoded);
    Ok(key_bytes)
  }

  /// Initialize the WireGuard tunnel
  fn init_tunnel(&mut self) -> Result<(), VpnError> {
    // Parse private key
    let private_key_bytes = Self::parse_key(&self.config.private_key)?;
    let static_private = StaticSecret::from(private_key_bytes);

    // Parse peer public key
    let peer_public_bytes = Self::parse_key(&self.config.peer_public_key)?;
    let peer_public = PublicKey::from(peer_public_bytes);

    // Parse optional preshared key
    let preshared_key = if let Some(ref psk) = self.config.preshared_key {
      Some(Self::parse_key(psk)?)
    } else {
      None
    };

    // Create the boringtun tunnel
    let tunn = Tunn::new(
      static_private,
      peer_public,
      preshared_key,
      self.config.persistent_keepalive,
      0, // index
      None,
    );

    self.tunnel = Some(Arc::new(Mutex::new(Box::new(tunn))));
    Ok(())
  }

  /// Resolve peer endpoint to socket address
  fn resolve_endpoint(&mut self) -> Result<SocketAddr, VpnError> {
    let endpoint = &self.config.peer_endpoint;

    // Try to resolve the endpoint
    let addrs: Vec<SocketAddr> = endpoint
      .to_socket_addrs()
      .map_err(|e| VpnError::Connection(format!("Failed to resolve endpoint '{endpoint}': {e}")))?
      .collect();

    addrs
      .into_iter()
      .next()
      .ok_or_else(|| VpnError::Connection(format!("No addresses found for endpoint: {endpoint}")))
  }

  /// Perform WireGuard handshake
  async fn handshake(&mut self) -> Result<(), VpnError> {
    let tunnel = self
      .tunnel
      .as_ref()
      .ok_or_else(|| VpnError::Tunnel("Tunnel not initialized".to_string()))?;

    let socket = self
      .socket
      .as_ref()
      .ok_or_else(|| VpnError::Tunnel("Socket not initialized".to_string()))?;

    let peer_addr = self
      .peer_addr
      .ok_or_else(|| VpnError::Tunnel("Peer address not resolved".to_string()))?;

    let mut tunnel_guard = tunnel.lock().await;

    // Generate handshake initiation
    let mut dst = vec![0u8; 2048];
    let result = tunnel_guard.format_handshake_initiation(&mut dst, false);

    match result {
      TunnResult::WriteToNetwork(packet) => {
        socket
          .send_to(packet, peer_addr)
          .map_err(|e| VpnError::Connection(format!("Failed to send handshake: {e}")))?;

        self
          .bytes_sent
          .fetch_add(packet.len() as u64, Ordering::Relaxed);
      }
      TunnResult::Err(e) => {
        return Err(VpnError::Tunnel(format!(
          "Handshake initiation failed: {e:?}"
        )));
      }
      _ => {}
    }

    // Wait for handshake response (with timeout)
    socket
      .set_read_timeout(Some(std::time::Duration::from_secs(10)))
      .map_err(|e| VpnError::Connection(format!("Failed to set timeout: {e}")))?;

    let mut recv_buf = vec![0u8; 2048];

    match socket.recv_from(&mut recv_buf) {
      Ok((len, _from)) => {
        self.bytes_received.fetch_add(len as u64, Ordering::Relaxed);

        let result = tunnel_guard.decapsulate(None, &recv_buf[..len], &mut dst);

        match result {
          TunnResult::WriteToNetwork(response) => {
            socket
              .send_to(response, peer_addr)
              .map_err(|e| VpnError::Connection(format!("Failed to send response: {e}")))?;

            self
              .bytes_sent
              .fetch_add(response.len() as u64, Ordering::Relaxed);
            self.last_handshake = Some(Utc::now().timestamp());
          }
          TunnResult::Done => {
            self.last_handshake = Some(Utc::now().timestamp());
          }
          TunnResult::Err(e) => {
            return Err(VpnError::Tunnel(format!(
              "Handshake response failed: {e:?}"
            )));
          }
          _ => {}
        }
      }
      Err(e) => {
        return Err(VpnError::Connection(format!(
          "Handshake timeout or error: {e}"
        )));
      }
    }

    Ok(())
  }

  /// Encrypt and send data through the tunnel
  pub async fn send(&self, data: &[u8]) -> Result<(), VpnError> {
    let tunnel = self
      .tunnel
      .as_ref()
      .ok_or_else(|| VpnError::Tunnel("Tunnel not initialized".to_string()))?;

    let socket = self
      .socket
      .as_ref()
      .ok_or_else(|| VpnError::Tunnel("Socket not initialized".to_string()))?;

    let peer_addr = self
      .peer_addr
      .ok_or_else(|| VpnError::Tunnel("Peer address not resolved".to_string()))?;

    let mut tunnel_guard = tunnel.lock().await;
    let mut dst = vec![0u8; data.len() + 256]; // Extra space for WireGuard overhead

    let result = tunnel_guard.encapsulate(data, &mut dst);

    match result {
      TunnResult::WriteToNetwork(packet) => {
        socket
          .send_to(packet, peer_addr)
          .map_err(|e| VpnError::Connection(format!("Failed to send data: {e}")))?;

        self
          .bytes_sent
          .fetch_add(packet.len() as u64, Ordering::Relaxed);
      }
      TunnResult::Err(e) => {
        return Err(VpnError::Tunnel(format!("Encryption failed: {e:?}")));
      }
      _ => {}
    }

    Ok(())
  }

  /// Receive and decrypt data from the tunnel
  pub async fn receive(&self, buf: &mut [u8]) -> Result<usize, VpnError> {
    let tunnel = self
      .tunnel
      .as_ref()
      .ok_or_else(|| VpnError::Tunnel("Tunnel not initialized".to_string()))?;

    let socket = self
      .socket
      .as_ref()
      .ok_or_else(|| VpnError::Tunnel("Socket not initialized".to_string()))?;

    let mut recv_buf = vec![0u8; 2048];

    let (len, _from) = socket
      .recv_from(&mut recv_buf)
      .map_err(|e| VpnError::Connection(format!("Receive failed: {e}")))?;

    self.bytes_received.fetch_add(len as u64, Ordering::Relaxed);

    let mut tunnel_guard = tunnel.lock().await;
    // decapsulate writes decrypted data directly to buf and returns a slice pointing to it
    let result = tunnel_guard.decapsulate(None, &recv_buf[..len], buf);

    match result {
      // Data is already written to buf by decapsulate, just return the length
      TunnResult::WriteToTunnelV4(decrypted, _) => Ok(decrypted.len()),
      TunnResult::WriteToTunnelV6(decrypted, _) => Ok(decrypted.len()),
      TunnResult::Done => Ok(0),
      TunnResult::Err(e) => Err(VpnError::Tunnel(format!("Decryption failed: {e:?}"))),
      _ => Ok(0),
    }
  }
}

#[async_trait]
impl VpnTunnel for WireGuardTunnel {
  async fn connect(&mut self) -> Result<(), VpnError> {
    if self.connected.load(Ordering::Relaxed) {
      return Ok(());
    }

    // Initialize the tunnel
    self.init_tunnel()?;

    // Resolve endpoint
    self.peer_addr = Some(self.resolve_endpoint()?);

    // Create UDP socket
    let socket = UdpSocket::bind("0.0.0.0:0")
      .map_err(|e| VpnError::Connection(format!("Failed to create socket: {e}")))?;

    socket
      .set_nonblocking(false)
      .map_err(|e| VpnError::Connection(format!("Failed to set socket options: {e}")))?;

    self.socket = Some(Arc::new(socket));

    // Perform handshake
    self.handshake().await?;

    self.connected.store(true, Ordering::Release);
    self.connected_at = Some(Utc::now().timestamp());

    log::info!("[vpn] WireGuard tunnel {} connected", self.vpn_id);

    Ok(())
  }

  async fn disconnect(&mut self) -> Result<(), VpnError> {
    if !self.connected.load(Ordering::Relaxed) {
      return Ok(());
    }

    self.connected.store(false, Ordering::Release);
    self.tunnel = None;
    self.socket = None;
    self.connected_at = None;

    log::info!("[vpn] WireGuard tunnel {} disconnected", self.vpn_id);

    Ok(())
  }

  fn is_connected(&self) -> bool {
    self.connected.load(Ordering::Acquire)
  }

  fn vpn_id(&self) -> &str {
    &self.vpn_id
  }

  fn get_status(&self) -> VpnStatus {
    VpnStatus {
      connected: self.is_connected(),
      vpn_id: self.vpn_id.clone(),
      connected_at: self.connected_at,
      bytes_sent: Some(self.bytes_sent.load(Ordering::Relaxed)),
      bytes_received: Some(self.bytes_received.load(Ordering::Relaxed)),
      last_handshake: self.last_handshake,
    }
  }

  fn bytes_sent(&self) -> u64 {
    self.bytes_sent.load(Ordering::Relaxed)
  }

  fn bytes_received(&self) -> u64 {
    self.bytes_received.load(Ordering::Relaxed)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn create_test_config() -> WireGuardConfig {
    WireGuardConfig {
      // These are test keys, not real ones
      private_key: "YEocP0e2o1WT5GlvBvQzVF7EeR6z9aCk+ZdZ5NKEuXA=".to_string(),
      address: "10.0.0.2/24".to_string(),
      dns: Some("1.1.1.1".to_string()),
      mtu: Some(1420),
      peer_public_key: "aGnF7JlG+U5t0BqB1PVf1yOuELHrWLGGcUJb0eCK9Aw=".to_string(),
      peer_endpoint: "127.0.0.1:51820".to_string(),
      allowed_ips: vec!["0.0.0.0/0".to_string()],
      persistent_keepalive: Some(25),
      preshared_key: None,
    }
  }

  #[test]
  fn test_wireguard_tunnel_creation() {
    let config = create_test_config();
    let tunnel = WireGuardTunnel::new("test-wg-1".to_string(), config);

    assert_eq!(tunnel.vpn_id(), "test-wg-1");
    assert!(!tunnel.is_connected());
    assert_eq!(tunnel.bytes_sent(), 0);
    assert_eq!(tunnel.bytes_received(), 0);
  }

  #[test]
  fn test_parse_key_valid() {
    // Valid base64-encoded 32-byte key
    let key = "YEocP0e2o1WT5GlvBvQzVF7EeR6z9aCk+ZdZ5NKEuXA=";
    let result = WireGuardTunnel::parse_key(key);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 32);
  }

  #[test]
  fn test_parse_key_invalid_base64() {
    let key = "not-valid-base64!!!";
    let result = WireGuardTunnel::parse_key(key);
    assert!(result.is_err());
  }

  #[test]
  fn test_parse_key_wrong_length() {
    // Valid base64 but wrong length
    let key = "YWJjZA=="; // "abcd" in base64
    let result = WireGuardTunnel::parse_key(key);
    assert!(result.is_err());
  }

  #[test]
  fn test_wireguard_status() {
    let config = create_test_config();
    let tunnel = WireGuardTunnel::new("test-wg-2".to_string(), config);

    let status = tunnel.get_status();
    assert!(!status.connected);
    assert_eq!(status.vpn_id, "test-wg-2");
    assert!(status.connected_at.is_none());
    assert_eq!(status.bytes_sent, Some(0));
    assert_eq!(status.bytes_received, Some(0));
  }
}

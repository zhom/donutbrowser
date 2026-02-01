//! VPN integration tests
//!
//! These tests verify VPN config parsing, storage, and tunnel functionality.
//! Connection tests require Docker and are skipped if Docker is not available.

mod test_harness;

use donutbrowser_lib::vpn::{
  detect_vpn_type, parse_openvpn_config, parse_wireguard_config, OpenVpnConfig, VpnConfig,
  VpnStorage, VpnType, WireGuardConfig,
};
use serial_test::serial;

// ============================================================================
// Config Parsing Tests
// ============================================================================

#[test]
fn test_wireguard_config_import() {
  let config = include_str!("fixtures/test.conf");
  let result = parse_wireguard_config(config);

  assert!(
    result.is_ok(),
    "Failed to parse WireGuard config: {:?}",
    result.err()
  );

  let wg = result.unwrap();
  assert!(!wg.private_key.is_empty());
  assert_eq!(wg.address, "10.0.0.2/24");
  assert_eq!(wg.dns, Some("1.1.1.1".to_string()));
  assert!(!wg.peer_public_key.is_empty());
  assert_eq!(wg.peer_endpoint, "vpn.example.com:51820");
  assert!(wg.allowed_ips.contains(&"0.0.0.0/0".to_string()));
  assert_eq!(wg.persistent_keepalive, Some(25));
}

#[test]
fn test_openvpn_config_import() {
  let config = include_str!("fixtures/test.ovpn");
  let result = parse_openvpn_config(config);

  assert!(
    result.is_ok(),
    "Failed to parse OpenVPN config: {:?}",
    result.err()
  );

  let ovpn = result.unwrap();
  assert_eq!(ovpn.remote_host, "vpn.example.com");
  assert_eq!(ovpn.remote_port, 1194);
  assert_eq!(ovpn.protocol, "udp");
  assert_eq!(ovpn.dev_type, "tun");
  assert!(ovpn.has_inline_ca);
  assert!(ovpn.has_inline_cert);
  assert!(ovpn.has_inline_key);
}

#[test]
fn test_detect_vpn_type_wireguard_by_extension() {
  let content = "[Interface]\nPrivateKey = test\n[Peer]\nPublicKey = peer";
  let result = detect_vpn_type(content, "my-vpn.conf");

  assert!(result.is_ok());
  assert_eq!(result.unwrap(), VpnType::WireGuard);
}

#[test]
fn test_detect_vpn_type_openvpn_by_extension() {
  let content = "client\nremote vpn.example.com 1194";
  let result = detect_vpn_type(content, "my-vpn.ovpn");

  assert!(result.is_ok());
  assert_eq!(result.unwrap(), VpnType::OpenVPN);
}

#[test]
fn test_detect_vpn_type_wireguard_by_content() {
  let content = r#"
[Interface]
PrivateKey = somekey
Address = 10.0.0.2/24

[Peer]
PublicKey = peerkey
Endpoint = 1.2.3.4:51820
"#;
  let result = detect_vpn_type(content, "config.txt");

  assert!(result.is_ok());
  assert_eq!(result.unwrap(), VpnType::WireGuard);
}

#[test]
fn test_detect_vpn_type_openvpn_by_content() {
  let content = r#"
client
dev tun
proto udp
remote vpn.server.com 443
"#;
  let result = detect_vpn_type(content, "config.txt");

  assert!(result.is_ok());
  assert_eq!(result.unwrap(), VpnType::OpenVPN);
}

#[test]
fn test_detect_vpn_type_unknown() {
  let content = "this is just some random text that is not a vpn config";
  let result = detect_vpn_type(content, "random.txt");

  assert!(result.is_err());
}

#[test]
fn test_wireguard_config_missing_private_key() {
  let config = r#"
[Interface]
Address = 10.0.0.2/24

[Peer]
PublicKey = somekey
Endpoint = 1.2.3.4:51820
"#;
  let result = parse_wireguard_config(config);

  assert!(result.is_err());
  let err = result.unwrap_err().to_string();
  assert!(err.contains("PrivateKey"));
}

#[test]
fn test_wireguard_config_missing_peer() {
  let config = r#"
[Interface]
PrivateKey = somekey
Address = 10.0.0.2/24
"#;
  let result = parse_wireguard_config(config);

  assert!(result.is_err());
  let err = result.unwrap_err().to_string();
  assert!(err.contains("PublicKey") || err.contains("Peer"));
}

#[test]
fn test_openvpn_config_missing_remote() {
  let config = r#"
client
dev tun
proto udp
"#;
  let result = parse_openvpn_config(config);

  assert!(result.is_err());
  let err = result.unwrap_err().to_string();
  assert!(err.contains("remote"));
}

#[test]
fn test_openvpn_config_with_port_in_remote() {
  let config = "client\nremote server.example.com 443 tcp";
  let result = parse_openvpn_config(config);

  assert!(result.is_ok());
  let ovpn = result.unwrap();
  assert_eq!(ovpn.remote_host, "server.example.com");
  assert_eq!(ovpn.remote_port, 443);
  assert_eq!(ovpn.protocol, "tcp");
}

// ============================================================================
// Storage Tests
// ============================================================================

#[test]
#[serial]
fn test_vpn_storage_save_and_load() {
  let temp_dir = tempfile::TempDir::new().unwrap();
  let storage = create_test_storage(&temp_dir);

  let config = VpnConfig {
    id: "test-id-1".to_string(),
    name: "Test VPN".to_string(),
    vpn_type: VpnType::WireGuard,
    config_data: "[Interface]\nPrivateKey=key\n[Peer]\nPublicKey=peer".to_string(),
    created_at: 1234567890,
    last_used: None,
  };

  let save_result = storage.save_config(&config);
  assert!(
    save_result.is_ok(),
    "Failed to save config: {:?}",
    save_result.err()
  );

  let load_result = storage.load_config("test-id-1");
  assert!(
    load_result.is_ok(),
    "Failed to load config: {:?}",
    load_result.err()
  );

  let loaded = load_result.unwrap();
  assert_eq!(loaded.id, config.id);
  assert_eq!(loaded.name, config.name);
  assert_eq!(loaded.vpn_type, config.vpn_type);
  assert_eq!(loaded.config_data, config.config_data);
}

#[test]
#[serial]
fn test_vpn_storage_list() {
  let temp_dir = tempfile::TempDir::new().unwrap();
  let storage = create_test_storage(&temp_dir);

  // Save two configs
  for i in 1..=2 {
    let config = VpnConfig {
      id: format!("list-test-{i}"),
      name: format!("VPN {i}"),
      vpn_type: if i == 1 {
        VpnType::WireGuard
      } else {
        VpnType::OpenVPN
      },
      config_data: "secret data".to_string(),
      created_at: 1000 * i as i64,
      last_used: None,
    };
    storage.save_config(&config).unwrap();
  }

  let list = storage.list_configs().unwrap();
  assert_eq!(list.len(), 2);

  // Config data should be empty in listing
  for cfg in &list {
    assert!(cfg.config_data.is_empty());
  }
}

#[test]
#[serial]
fn test_vpn_storage_delete() {
  let temp_dir = tempfile::TempDir::new().unwrap();
  let storage = create_test_storage(&temp_dir);

  let config = VpnConfig {
    id: "delete-test".to_string(),
    name: "To Delete".to_string(),
    vpn_type: VpnType::WireGuard,
    config_data: "data".to_string(),
    created_at: 1000,
    last_used: None,
  };

  storage.save_config(&config).unwrap();
  assert!(storage.load_config("delete-test").is_ok());

  storage.delete_config("delete-test").unwrap();
  assert!(storage.load_config("delete-test").is_err());
}

#[test]
#[serial]
fn test_vpn_storage_import() {
  let temp_dir = tempfile::TempDir::new().unwrap();
  let storage = create_test_storage(&temp_dir);

  let wg_config = include_str!("fixtures/test.conf");
  let result = storage.import_config(wg_config, "my-vpn.conf", Some("My WireGuard".to_string()));

  assert!(result.is_ok(), "Import failed: {:?}", result.err());

  let imported = result.unwrap();
  assert_eq!(imported.name, "My WireGuard");
  assert_eq!(imported.vpn_type, VpnType::WireGuard);
  assert!(!imported.id.is_empty());
}

// ============================================================================
// Helper Functions
// ============================================================================

fn create_test_storage(temp_dir: &tempfile::TempDir) -> VpnStorage {
  VpnStorage::with_dir(temp_dir.path())
}

// ============================================================================
// Connection Tests (require Docker)
// ============================================================================

/// These tests require Docker to be available.
/// They are automatically skipped if Docker is not installed.

#[tokio::test]
#[serial]
async fn test_wireguard_tunnel_init() {
  // This test only verifies tunnel creation, not actual connection
  let config = WireGuardConfig {
    private_key: "YEocP0e2o1WT5GlvBvQzVF7EeR6z9aCk+ZdZ5NKEuXA=".to_string(),
    address: "10.0.0.2/24".to_string(),
    dns: Some("1.1.1.1".to_string()),
    mtu: None,
    peer_public_key: "aGnF7JlG+U5t0BqB1PVf1yOuELHrWLGGcUJb0eCK9Aw=".to_string(),
    peer_endpoint: "127.0.0.1:51820".to_string(),
    allowed_ips: vec!["0.0.0.0/0".to_string()],
    persistent_keepalive: Some(25),
    preshared_key: None,
  };

  use donutbrowser_lib::vpn::{VpnTunnel, WireGuardTunnel};

  let tunnel = WireGuardTunnel::new("test-wg".to_string(), config);
  assert_eq!(tunnel.vpn_id(), "test-wg");
  assert!(!tunnel.is_connected());
  assert_eq!(tunnel.bytes_sent(), 0);
  assert_eq!(tunnel.bytes_received(), 0);
}

#[tokio::test]
#[serial]
async fn test_openvpn_tunnel_init() {
  // This test only verifies tunnel creation, not actual connection
  let config = OpenVpnConfig {
    raw_config: "client\nremote localhost 1194".to_string(),
    remote_host: "localhost".to_string(),
    remote_port: 1194,
    protocol: "udp".to_string(),
    dev_type: "tun".to_string(),
    has_inline_ca: false,
    has_inline_cert: false,
    has_inline_key: false,
  };

  use donutbrowser_lib::vpn::{OpenVpnTunnel, VpnTunnel};

  let tunnel = OpenVpnTunnel::new("test-ovpn".to_string(), config);
  assert_eq!(tunnel.vpn_id(), "test-ovpn");
  assert!(!tunnel.is_connected());
  assert_eq!(tunnel.bytes_sent(), 0);
  assert_eq!(tunnel.bytes_received(), 0);
}

#[tokio::test]
#[serial]
async fn test_tunnel_manager() {
  use donutbrowser_lib::vpn::{TunnelManager, VpnStatus, VpnTunnel};

  // Create a mock tunnel for testing the manager
  struct MockTunnel {
    id: String,
    connected: bool,
  }

  #[async_trait::async_trait]
  impl VpnTunnel for MockTunnel {
    async fn connect(&mut self) -> Result<(), donutbrowser_lib::vpn::VpnError> {
      self.connected = true;
      Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), donutbrowser_lib::vpn::VpnError> {
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
        connected_at: None,
        bytes_sent: Some(0),
        bytes_received: Some(0),
        last_handshake: None,
      }
    }

    fn bytes_sent(&self) -> u64 {
      0
    }

    fn bytes_received(&self) -> u64 {
      0
    }
  }

  let mut manager = TunnelManager::new();

  let tunnel = Box::new(MockTunnel {
    id: "mock-1".to_string(),
    connected: true,
  });

  manager.register_tunnel("mock-1".to_string(), tunnel);
  assert!(manager.is_tunnel_active("mock-1"));
  assert!(!manager.is_tunnel_active("nonexistent"));
  assert_eq!(manager.active_count(), 1);

  manager.remove_tunnel("mock-1");
  assert!(!manager.is_tunnel_active("mock-1"));
  assert_eq!(manager.active_count(), 0);
}

// NOTE: Actual connection tests require Docker containers running.
// These are meant to be run with the CI workflow that sets up service containers.
// To run locally: docker run -d --cap-add=NET_ADMIN -p 51820:51820/udp -e PEERS=1 linuxserver/wireguard

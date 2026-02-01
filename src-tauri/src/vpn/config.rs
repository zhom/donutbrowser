//! VPN configuration types and parsing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// VPN-related errors
#[derive(Error, Debug)]
pub enum VpnError {
  #[error("Unknown VPN config format")]
  UnknownFormat,
  #[error("Invalid WireGuard config: {0}")]
  InvalidWireGuard(String),
  #[error("Invalid OpenVPN config: {0}")]
  InvalidOpenVpn(String),
  #[error("Storage error: {0}")]
  Storage(String),
  #[error("Connection error: {0}")]
  Connection(String),
  #[error("Encryption error: {0}")]
  Encryption(String),
  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),
  #[error("VPN not found: {0}")]
  NotFound(String),
  #[error("Tunnel error: {0}")]
  Tunnel(String),
}

/// The type of VPN configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VpnType {
  WireGuard,
  OpenVPN,
}

impl std::fmt::Display for VpnType {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      VpnType::WireGuard => write!(f, "WireGuard"),
      VpnType::OpenVPN => write!(f, "OpenVPN"),
    }
  }
}

/// A stored VPN configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnConfig {
  pub id: String,
  pub name: String,
  pub vpn_type: VpnType,
  pub config_data: String, // Raw config content (encrypted at rest)
  pub created_at: i64,
  pub last_used: Option<i64>,
}

/// Parsed WireGuard configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireGuardConfig {
  pub private_key: String,
  pub address: String,
  pub dns: Option<String>,
  pub mtu: Option<u16>,
  pub peer_public_key: String,
  pub peer_endpoint: String,
  pub allowed_ips: Vec<String>,
  pub persistent_keepalive: Option<u16>,
  pub preshared_key: Option<String>,
}

/// Parsed OpenVPN configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenVpnConfig {
  pub raw_config: String,
  pub remote_host: String,
  pub remote_port: u16,
  pub protocol: String, // "udp" or "tcp"
  pub dev_type: String, // "tun" or "tap"
  pub has_inline_ca: bool,
  pub has_inline_cert: bool,
  pub has_inline_key: bool,
}

/// Result of importing a VPN configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnImportResult {
  pub success: bool,
  pub vpn_id: Option<String>,
  pub vpn_type: Option<VpnType>,
  pub name: String,
  pub error: Option<String>,
}

/// VPN connection status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpnStatus {
  pub connected: bool,
  pub vpn_id: String,
  pub connected_at: Option<i64>,
  pub bytes_sent: Option<u64>,
  pub bytes_received: Option<u64>,
  pub last_handshake: Option<i64>,
}

/// Detect the VPN type from file content and filename
pub fn detect_vpn_type(content: &str, filename: &str) -> Result<VpnType, VpnError> {
  let filename_lower = filename.to_lowercase();

  // Check file extension first
  if filename_lower.ends_with(".conf") {
    // .conf could be WireGuard - check content
    if content.contains("[Interface]") && content.contains("[Peer]") {
      return Ok(VpnType::WireGuard);
    }
  }

  if filename_lower.ends_with(".ovpn") {
    return Ok(VpnType::OpenVPN);
  }

  // Check content patterns
  if content.contains("[Interface]") && content.contains("PrivateKey") && content.contains("[Peer]")
  {
    return Ok(VpnType::WireGuard);
  }

  if content.contains("remote ") && (content.contains("client") || content.contains("dev tun")) {
    return Ok(VpnType::OpenVPN);
  }

  Err(VpnError::UnknownFormat)
}

/// Parse a WireGuard configuration file
pub fn parse_wireguard_config(content: &str) -> Result<WireGuardConfig, VpnError> {
  let mut interface: HashMap<String, String> = HashMap::new();
  let mut peer: HashMap<String, String> = HashMap::new();
  let mut current_section: Option<&str> = None;

  for line in content.lines() {
    let line = line.trim();

    // Skip empty lines and comments
    if line.is_empty() || line.starts_with('#') {
      continue;
    }

    // Check for section headers
    if line == "[Interface]" {
      current_section = Some("interface");
      continue;
    }
    if line == "[Peer]" {
      current_section = Some("peer");
      continue;
    }

    // Parse key-value pairs
    if let Some((key, value)) = line.split_once('=') {
      let key = key.trim().to_string();
      let value = value.trim().to_string();

      match current_section {
        Some("interface") => {
          interface.insert(key, value);
        }
        Some("peer") => {
          peer.insert(key, value);
        }
        _ => {}
      }
    }
  }

  // Validate required fields
  let private_key = interface
    .get("PrivateKey")
    .ok_or_else(|| VpnError::InvalidWireGuard("Missing PrivateKey in [Interface]".to_string()))?
    .clone();

  let address = interface
    .get("Address")
    .ok_or_else(|| VpnError::InvalidWireGuard("Missing Address in [Interface]".to_string()))?
    .clone();

  let peer_public_key = peer
    .get("PublicKey")
    .ok_or_else(|| VpnError::InvalidWireGuard("Missing PublicKey in [Peer]".to_string()))?
    .clone();

  let peer_endpoint = peer
    .get("Endpoint")
    .ok_or_else(|| VpnError::InvalidWireGuard("Missing Endpoint in [Peer]".to_string()))?
    .clone();

  let allowed_ips = peer
    .get("AllowedIPs")
    .map(|s| s.split(',').map(|ip| ip.trim().to_string()).collect())
    .unwrap_or_else(|| vec!["0.0.0.0/0".to_string()]);

  let persistent_keepalive = peer.get("PersistentKeepalive").and_then(|s| s.parse().ok());

  let dns = interface.get("DNS").cloned();
  let mtu = interface.get("MTU").and_then(|s| s.parse().ok());
  let preshared_key = peer.get("PresharedKey").cloned();

  Ok(WireGuardConfig {
    private_key,
    address,
    dns,
    mtu,
    peer_public_key,
    peer_endpoint,
    allowed_ips,
    persistent_keepalive,
    preshared_key,
  })
}

/// Parse an OpenVPN configuration file
pub fn parse_openvpn_config(content: &str) -> Result<OpenVpnConfig, VpnError> {
  let mut remote_host = String::new();
  let mut remote_port: u16 = 1194; // Default OpenVPN port
  let mut protocol = "udp".to_string();
  let mut dev_type = "tun".to_string();

  let has_inline_ca = content.contains("<ca>") && content.contains("</ca>");
  let has_inline_cert = content.contains("<cert>") && content.contains("</cert>");
  let has_inline_key = content.contains("<key>") && content.contains("</key>");

  for line in content.lines() {
    let line = line.trim();

    // Skip empty lines and comments
    if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
      continue;
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
      continue;
    }

    match parts[0] {
      "remote" => {
        if parts.len() >= 2 {
          remote_host = parts[1].to_string();
        }
        if parts.len() >= 3 {
          if let Ok(port) = parts[2].parse() {
            remote_port = port;
          }
        }
        if parts.len() >= 4 {
          protocol = parts[3].to_string();
        }
      }
      "proto" => {
        if parts.len() >= 2 {
          protocol = parts[1].to_string();
        }
      }
      "port" => {
        if parts.len() >= 2 {
          if let Ok(port) = parts[1].parse() {
            remote_port = port;
          }
        }
      }
      "dev" => {
        if parts.len() >= 2 {
          dev_type = parts[1].to_string();
        }
      }
      _ => {}
    }
  }

  if remote_host.is_empty() {
    return Err(VpnError::InvalidOpenVpn(
      "Missing 'remote' directive".to_string(),
    ));
  }

  Ok(OpenVpnConfig {
    raw_config: content.to_string(),
    remote_host,
    remote_port,
    protocol,
    dev_type,
    has_inline_ca,
    has_inline_cert,
    has_inline_key,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_detect_wireguard_by_extension() {
    let content = "[Interface]\nPrivateKey = test\n[Peer]\nPublicKey = test";
    assert_eq!(
      detect_vpn_type(content, "test.conf").unwrap(),
      VpnType::WireGuard
    );
  }

  #[test]
  fn test_detect_openvpn_by_extension() {
    let content = "client\nremote vpn.example.com 1194";
    assert_eq!(
      detect_vpn_type(content, "test.ovpn").unwrap(),
      VpnType::OpenVPN
    );
  }

  #[test]
  fn test_detect_wireguard_by_content() {
    let content = "[Interface]\nPrivateKey = testkey123\nAddress = 10.0.0.2/24\n\n[Peer]\nPublicKey = peerkey456\nEndpoint = vpn.example.com:51820";
    assert_eq!(
      detect_vpn_type(content, "config").unwrap(),
      VpnType::WireGuard
    );
  }

  #[test]
  fn test_detect_openvpn_by_content() {
    let content = "client\ndev tun\nproto udp\nremote vpn.example.com 1194";
    assert_eq!(
      detect_vpn_type(content, "config").unwrap(),
      VpnType::OpenVPN
    );
  }

  #[test]
  fn test_detect_unknown_format() {
    let content = "random text that is not a vpn config";
    assert!(detect_vpn_type(content, "random.txt").is_err());
  }

  #[test]
  fn test_parse_wireguard_config() {
    let content = r#"
[Interface]
PrivateKey = WGTestPrivateKey123456789012345678901234567890
Address = 10.0.0.2/24
DNS = 1.1.1.1
MTU = 1420

[Peer]
PublicKey = WGTestPublicKey1234567890123456789012345678901
Endpoint = vpn.example.com:51820
AllowedIPs = 0.0.0.0/0, ::/0
PersistentKeepalive = 25
"#;

    let config = parse_wireguard_config(content).unwrap();
    assert_eq!(
      config.private_key,
      "WGTestPrivateKey123456789012345678901234567890"
    );
    assert_eq!(config.address, "10.0.0.2/24");
    assert_eq!(config.dns, Some("1.1.1.1".to_string()));
    assert_eq!(config.mtu, Some(1420));
    assert_eq!(
      config.peer_public_key,
      "WGTestPublicKey1234567890123456789012345678901"
    );
    assert_eq!(config.peer_endpoint, "vpn.example.com:51820");
    assert_eq!(config.allowed_ips, vec!["0.0.0.0/0", "::/0"]);
    assert_eq!(config.persistent_keepalive, Some(25));
  }

  #[test]
  fn test_parse_wireguard_config_minimal() {
    let content = r#"
[Interface]
PrivateKey = minimalkey
Address = 10.0.0.2/32

[Peer]
PublicKey = peerpubkey
Endpoint = 1.2.3.4:51820
"#;

    let config = parse_wireguard_config(content).unwrap();
    assert_eq!(config.private_key, "minimalkey");
    assert_eq!(config.address, "10.0.0.2/32");
    assert!(config.dns.is_none());
    assert!(config.mtu.is_none());
    assert_eq!(config.peer_public_key, "peerpubkey");
    assert_eq!(config.peer_endpoint, "1.2.3.4:51820");
  }

  #[test]
  fn test_parse_wireguard_missing_private_key() {
    let content = r#"
[Interface]
Address = 10.0.0.2/24

[Peer]
PublicKey = key
Endpoint = 1.2.3.4:51820
"#;

    let result = parse_wireguard_config(content);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("PrivateKey"));
  }

  #[test]
  fn test_parse_openvpn_config() {
    let content = r#"
client
dev tun
proto udp
remote vpn.example.com 1194
resolv-retry infinite
nobind
persist-key
persist-tun
<ca>
-----BEGIN CERTIFICATE-----
...certificate data...
-----END CERTIFICATE-----
</ca>
<cert>
-----BEGIN CERTIFICATE-----
...cert data...
-----END CERTIFICATE-----
</cert>
<key>
-----BEGIN PRIVATE KEY-----
...key data...
-----END PRIVATE KEY-----
</key>
"#;

    let config = parse_openvpn_config(content).unwrap();
    assert_eq!(config.remote_host, "vpn.example.com");
    assert_eq!(config.remote_port, 1194);
    assert_eq!(config.protocol, "udp");
    assert_eq!(config.dev_type, "tun");
    assert!(config.has_inline_ca);
    assert!(config.has_inline_cert);
    assert!(config.has_inline_key);
  }

  #[test]
  fn test_parse_openvpn_config_minimal() {
    let content = r#"
client
remote vpn.example.com
"#;

    let config = parse_openvpn_config(content).unwrap();
    assert_eq!(config.remote_host, "vpn.example.com");
    assert_eq!(config.remote_port, 1194); // Default
    assert_eq!(config.protocol, "udp"); // Default
  }

  #[test]
  fn test_parse_openvpn_config_with_port_and_proto() {
    let content = r#"
client
remote vpn.example.com 443 tcp
"#;

    let config = parse_openvpn_config(content).unwrap();
    assert_eq!(config.remote_host, "vpn.example.com");
    assert_eq!(config.remote_port, 443);
    assert_eq!(config.protocol, "tcp");
  }

  #[test]
  fn test_parse_openvpn_missing_remote() {
    let content = r#"
client
dev tun
proto udp
"#;

    let result = parse_openvpn_config(content);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("remote"));
  }
}

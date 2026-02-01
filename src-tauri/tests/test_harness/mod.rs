//! Test harness for VPN integration tests.
//!
//! This module provides Docker-based test infrastructure for WireGuard and OpenVPN tests.
//! In CI environments, it uses pre-configured service containers.
//! In local development, it spawns Docker containers on demand.
//!
//! Note: These utilities are available for tests that need Docker containers,
//! but may not be used in all test configurations.
#![allow(dead_code)]

use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

const WIREGUARD_IMAGE: &str = "linuxserver/wireguard:latest";
const OPENVPN_IMAGE: &str = "kylemanna/openvpn:latest";
const WG_CONTAINER: &str = "donut-wg-test";
const OVPN_CONTAINER: &str = "donut-ovpn-test";

/// Check if running in CI environment
pub fn is_ci() -> bool {
  std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok()
}

/// Check if Docker is available
pub fn is_docker_available() -> bool {
  Command::new("docker")
    .arg("version")
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// Start a WireGuard test server and return client config
pub async fn start_wireguard_server() -> Result<WireGuardTestConfig, String> {
  if is_ci() {
    // In CI, use the service container configured in workflow
    let host = std::env::var("VPN_TEST_WG_HOST").unwrap_or_else(|_| "localhost".into());
    let port = std::env::var("VPN_TEST_WG_PORT").unwrap_or_else(|_| "51820".into());

    // Wait for service to be ready
    wait_for_service(&host, port.parse().unwrap_or(51820)).await?;

    return get_ci_wireguard_config(&host, &port);
  }

  if !is_docker_available() {
    return Err("Docker is not available for local testing".to_string());
  }

  // Stop any existing container
  let _ = Command::new("docker")
    .args(["rm", "-f", WG_CONTAINER])
    .output();

  // Start WireGuard container
  let output = Command::new("docker")
    .args([
      "run",
      "-d",
      "--name",
      WG_CONTAINER,
      "--cap-add=NET_ADMIN",
      "-p",
      "51820:51820/udp",
      "-e",
      "PEERS=1",
      "-e",
      "SERVERURL=127.0.0.1",
      "-e",
      "SERVERPORT=51820",
      "-e",
      "PEERDNS=auto",
      WIREGUARD_IMAGE,
    ])
    .output()
    .map_err(|e| format!("Failed to start WireGuard container: {e}"))?;

  if !output.status.success() {
    return Err(format!(
      "Docker run failed: {}",
      String::from_utf8_lossy(&output.stderr)
    ));
  }

  // Wait for container to be ready and generate configs
  sleep(Duration::from_secs(10)).await;

  // Extract client config from container
  let config_output = Command::new("docker")
    .args(["exec", WG_CONTAINER, "cat", "/config/peer1/peer1.conf"])
    .output()
    .map_err(|e| format!("Failed to get client config: {e}"))?;

  if !config_output.status.success() {
    return Err(format!(
      "Failed to read config: {}",
      String::from_utf8_lossy(&config_output.stderr)
    ));
  }

  let config_str = String::from_utf8_lossy(&config_output.stdout).to_string();
  parse_wireguard_test_config(&config_str)
}

/// Start an OpenVPN test server and return client config
pub async fn start_openvpn_server() -> Result<OpenVpnTestConfig, String> {
  if is_ci() {
    // In CI, use the service container configured in workflow
    let host = std::env::var("VPN_TEST_OVPN_HOST").unwrap_or_else(|_| "localhost".into());
    let port = std::env::var("VPN_TEST_OVPN_PORT").unwrap_or_else(|_| "1194".into());

    // Wait for service to be ready
    wait_for_service(&host, port.parse().unwrap_or(1194)).await?;

    return get_ci_openvpn_config(&host, &port);
  }

  if !is_docker_available() {
    return Err("Docker is not available for local testing".to_string());
  }

  // Stop any existing container
  let _ = Command::new("docker")
    .args(["rm", "-f", OVPN_CONTAINER])
    .output();

  // For OpenVPN, we need to initialize PKI first, which is complex
  // For simplicity in tests, we'll use a pre-configured test config
  Err("OpenVPN container setup requires pre-configured PKI. Use test fixtures instead.".to_string())
}

/// Stop all VPN test servers
pub async fn stop_vpn_servers() {
  let _ = Command::new("docker")
    .args(["rm", "-f", WG_CONTAINER, OVPN_CONTAINER])
    .output();
}

/// Wait for a network service to be ready
async fn wait_for_service(host: &str, port: u16) -> Result<(), String> {
  let timeout = Duration::from_secs(30);
  let start = std::time::Instant::now();

  while start.elapsed() < timeout {
    if std::net::TcpStream::connect(format!("{host}:{port}")).is_ok() {
      return Ok(());
    }
    sleep(Duration::from_millis(500)).await;
  }

  Err(format!("Timeout waiting for service at {host}:{port}"))
}

/// WireGuard test configuration
pub struct WireGuardTestConfig {
  pub private_key: String,
  pub address: String,
  pub dns: Option<String>,
  pub peer_public_key: String,
  pub peer_endpoint: String,
  pub allowed_ips: Vec<String>,
}

/// OpenVPN test configuration
pub struct OpenVpnTestConfig {
  pub raw_config: String,
  pub remote_host: String,
  pub remote_port: u16,
  pub protocol: String,
}

/// Parse WireGuard test config from INI content
fn parse_wireguard_test_config(content: &str) -> Result<WireGuardTestConfig, String> {
  let mut private_key = String::new();
  let mut address = String::new();
  let mut dns = None;
  let mut peer_public_key = String::new();
  let mut peer_endpoint = String::new();
  let mut allowed_ips = vec!["0.0.0.0/0".to_string()];
  let mut current_section = "";

  for line in content.lines() {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }

    if line == "[Interface]" {
      current_section = "interface";
      continue;
    }
    if line == "[Peer]" {
      current_section = "peer";
      continue;
    }

    if let Some((key, value)) = line.split_once('=') {
      let key = key.trim();
      let value = value.trim();

      match (current_section, key) {
        ("interface", "PrivateKey") => private_key = value.to_string(),
        ("interface", "Address") => address = value.to_string(),
        ("interface", "DNS") => dns = Some(value.to_string()),
        ("peer", "PublicKey") => peer_public_key = value.to_string(),
        ("peer", "Endpoint") => peer_endpoint = value.to_string(),
        ("peer", "AllowedIPs") => {
          allowed_ips = value.split(',').map(|s| s.trim().to_string()).collect();
        }
        _ => {}
      }
    }
  }

  if private_key.is_empty() || address.is_empty() || peer_public_key.is_empty() {
    return Err("Invalid WireGuard config: missing required fields".to_string());
  }

  // Replace Endpoint with localhost for local testing
  if peer_endpoint.contains("10.") || peer_endpoint.contains("172.") {
    let port = peer_endpoint.split(':').next_back().unwrap_or("51820");
    peer_endpoint = format!("127.0.0.1:{port}");
  }

  Ok(WireGuardTestConfig {
    private_key,
    address,
    dns,
    peer_public_key,
    peer_endpoint,
    allowed_ips,
  })
}

/// Get WireGuard config from CI environment
fn get_ci_wireguard_config(host: &str, port: &str) -> Result<WireGuardTestConfig, String> {
  // In CI, use environment variables or test fixtures
  let private_key =
    std::env::var("VPN_TEST_WG_PRIVATE_KEY").unwrap_or_else(|_| "test-private-key".to_string());
  let public_key =
    std::env::var("VPN_TEST_WG_PUBLIC_KEY").unwrap_or_else(|_| "test-public-key".to_string());

  Ok(WireGuardTestConfig {
    private_key,
    address: "10.0.0.2/24".to_string(),
    dns: Some("1.1.1.1".to_string()),
    peer_public_key: public_key,
    peer_endpoint: format!("{host}:{port}"),
    allowed_ips: vec!["0.0.0.0/0".to_string()],
  })
}

/// Get OpenVPN config from CI environment
fn get_ci_openvpn_config(host: &str, port: &str) -> Result<OpenVpnTestConfig, String> {
  let raw_config = format!(
    r#"
client
dev tun
proto udp
remote {host} {port}
resolv-retry infinite
nobind
persist-key
persist-tun
"#
  );

  Ok(OpenVpnTestConfig {
    raw_config,
    remote_host: host.to_string(),
    remote_port: port.parse().unwrap_or(1194),
    protocol: "udp".to_string(),
  })
}

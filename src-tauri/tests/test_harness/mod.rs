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
const OVPN_VOLUME: &str = "donut-ovpn-test-data";

/// Check if running in CI environment
pub fn is_ci() -> bool {
  std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok()
}

fn has_external_wireguard_service() -> bool {
  std::env::var("VPN_TEST_WG_HOST").is_ok()
}

fn has_external_openvpn_service() -> bool {
  std::env::var("VPN_TEST_OVPN_HOST").is_ok()
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
  if has_external_wireguard_service() {
    let host = std::env::var("VPN_TEST_WG_HOST").unwrap_or_else(|_| "localhost".into());
    let port = std::env::var("VPN_TEST_WG_PORT").unwrap_or_else(|_| "51820".into());

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
      "-e",
      "INTERNAL_SUBNET=10.64.0.0",
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

  // Wait for container to generate configs and bring up the WireGuard interface.
  // A fixed sleep is flaky — on busy machines the interface takes longer. Instead
  // we poll `wg show` inside the container until it reports an active interface,
  // with a generous upper bound.
  let wg_ready_deadline = tokio::time::Instant::now() + Duration::from_secs(45);
  loop {
    sleep(Duration::from_secs(2)).await;

    // Check if peer config file has been generated
    let config_check = Command::new("docker")
      .args(["exec", WG_CONTAINER, "cat", "/config/peer1/peer1.conf"])
      .output();
    let config_exists = config_check
      .as_ref()
      .map(|o| o.status.success())
      .unwrap_or(false);

    // Check if WireGuard interface is actually up and listening
    let wg_check = Command::new("docker")
      .args(["exec", WG_CONTAINER, "wg", "show"])
      .output();
    let wg_up = wg_check
      .as_ref()
      .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("listening port"))
      .unwrap_or(false);

    if config_exists && wg_up {
      break;
    }

    if tokio::time::Instant::now() >= wg_ready_deadline {
      return Err("WireGuard container did not become ready within 45s".to_string());
    }
  }

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
  let mut config = parse_wireguard_test_config(&config_str)?;

  // Start a lightweight HTTP server inside the container on the WireGuard
  // interface so tests can verify traffic flows through the tunnel without
  // depending on internet access (Docker Desktop for Mac can't reliably NAT
  // WireGuard tunnel traffic to the internet). The linuxserver/wireguard
  // image doesn't have python3 or busybox httpd, but it has nc (netcat).
  let _ = Command::new("docker")
    .args([
      "exec",
      "-d",
      WG_CONTAINER,
      "sh",
      "-c",
      r#"while true; do printf "HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nWG-TUNNEL-OK\n" | nc -l -p 8080 2>/dev/null; done"#,
    ])
    .output();
  // Give the nc loop a moment to start accepting
  sleep(Duration::from_millis(500)).await;

  // Extract the server's tunnel IP (first octet group from INTERNAL_SUBNET + .1)
  config.server_tunnel_ip = "10.64.0.1".to_string();

  Ok(config)
}

/// Start an OpenVPN test server and return client config
pub async fn start_openvpn_server() -> Result<OpenVpnTestConfig, String> {
  if has_external_openvpn_service() {
    let host = std::env::var("VPN_TEST_OVPN_HOST").unwrap_or_else(|_| "localhost".into());
    let port = std::env::var("VPN_TEST_OVPN_PORT").unwrap_or_else(|_| "1194".into());

    return get_ci_openvpn_config(&host, &port);
  }

  if !is_docker_available() {
    return Err("Docker is not available for local testing".to_string());
  }

  // Stop any existing container
  let _ = Command::new("docker")
    .args(["rm", "-f", OVPN_CONTAINER])
    .output();

  let _ = Command::new("docker")
    .args(["volume", "rm", "-f", OVPN_VOLUME])
    .output();

  let create_volume = Command::new("docker")
    .args(["volume", "create", OVPN_VOLUME])
    .output()
    .map_err(|e| format!("Failed to create OpenVPN test volume: {e}"))?;
  if !create_volume.status.success() {
    return Err(format!(
      "Failed to create OpenVPN test volume: {}",
      String::from_utf8_lossy(&create_volume.stderr)
    ));
  }

  let genconfig = Command::new("docker")
    .args([
      "run",
      "--rm",
      "-v",
      &format!("{OVPN_VOLUME}:/etc/openvpn"),
      "-e",
      "EASYRSA_BATCH=1",
      OPENVPN_IMAGE,
      "ovpn_genconfig",
      "-u",
      "udp://127.0.0.1",
      "-s",
      "10.9.0.0/24",
    ])
    .output()
    .map_err(|e| format!("Failed to generate OpenVPN config: {e}"))?;
  if !genconfig.status.success() {
    return Err(format!(
      "OpenVPN config generation failed: {}",
      String::from_utf8_lossy(&genconfig.stderr)
    ));
  }

  let init_pki = Command::new("docker")
    .args([
      "run",
      "--rm",
      "-v",
      &format!("{OVPN_VOLUME}:/etc/openvpn"),
      "-e",
      "EASYRSA_BATCH=1",
      OPENVPN_IMAGE,
      "ovpn_initpki",
      "nopass",
    ])
    .output()
    .map_err(|e| format!("Failed to initialize OpenVPN PKI: {e}"))?;
  if !init_pki.status.success() {
    return Err(format!(
      "OpenVPN PKI initialization failed: {}",
      String::from_utf8_lossy(&init_pki.stderr)
    ));
  }

  let build_client = Command::new("docker")
    .args([
      "run",
      "--rm",
      "-v",
      &format!("{OVPN_VOLUME}:/etc/openvpn"),
      "-e",
      "EASYRSA_BATCH=1",
      OPENVPN_IMAGE,
      "easyrsa",
      "build-client-full",
      "donut-test-client",
      "nopass",
    ])
    .output()
    .map_err(|e| format!("Failed to build OpenVPN client certificate: {e}"))?;
  if !build_client.status.success() {
    return Err(format!(
      "OpenVPN client certificate build failed: {}",
      String::from_utf8_lossy(&build_client.stderr)
    ));
  }

  let start_server = Command::new("docker")
    .args([
      "run",
      "-d",
      "--name",
      OVPN_CONTAINER,
      "--cap-add=NET_ADMIN",
      "-p",
      "1194:1194/udp",
      "-v",
      &format!("{OVPN_VOLUME}:/etc/openvpn"),
      OPENVPN_IMAGE,
    ])
    .output()
    .map_err(|e| format!("Failed to start OpenVPN container: {e}"))?;
  if !start_server.status.success() {
    return Err(format!(
      "OpenVPN container start failed: {}",
      String::from_utf8_lossy(&start_server.stderr)
    ));
  }

  sleep(Duration::from_secs(10)).await;

  let client_config = Command::new("docker")
    .args([
      "run",
      "--rm",
      "-v",
      &format!("{OVPN_VOLUME}:/etc/openvpn"),
      OPENVPN_IMAGE,
      "ovpn_getclient",
      "donut-test-client",
    ])
    .output()
    .map_err(|e| format!("Failed to fetch OpenVPN client config: {e}"))?;
  if !client_config.status.success() {
    return Err(format!(
      "Failed to read OpenVPN client config: {}",
      String::from_utf8_lossy(&client_config.stderr)
    ));
  }

  let raw_config = String::from_utf8_lossy(&client_config.stdout).to_string();
  Ok(OpenVpnTestConfig {
    raw_config,
    remote_host: "127.0.0.1".to_string(),
    remote_port: 1194,
    protocol: "udp".to_string(),
  })
}

/// Stop all VPN test servers
pub async fn stop_vpn_servers() {
  let _ = Command::new("docker")
    .args(["rm", "-f", WG_CONTAINER, OVPN_CONTAINER])
    .output();
  let _ = Command::new("docker")
    .args(["volume", "rm", "-f", OVPN_VOLUME])
    .output();
}

/// WireGuard test configuration
pub struct WireGuardTestConfig {
  pub private_key: String,
  pub address: String,
  pub dns: Option<String>,
  pub peer_public_key: String,
  pub peer_endpoint: String,
  pub allowed_ips: Vec<String>,
  pub preshared_key: Option<String>,
  /// IP of the WireGuard server on the tunnel interface (e.g. 10.64.0.1).
  /// Tests use this to reach an HTTP server inside the container without
  /// needing internet access from Docker.
  pub server_tunnel_ip: String,
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
  let mut preshared_key = None;
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
        ("peer", "PresharedKey") => preshared_key = Some(value.to_string()),
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
    preshared_key,
    server_tunnel_ip: String::new(), // filled in by caller
  })
}

/// Get WireGuard config from CI environment
fn get_ci_wireguard_config(host: &str, port: &str) -> Result<WireGuardTestConfig, String> {
  if std::env::var("VPN_TEST_WG_PRIVATE_KEY").is_err()
    || std::env::var("VPN_TEST_WG_PUBLIC_KEY").is_err()
  {
    return Err(
      "External WireGuard test service is configured, but VPN_TEST_WG_PRIVATE_KEY and VPN_TEST_WG_PUBLIC_KEY are missing"
        .to_string(),
    );
  }

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
    preshared_key: std::env::var("VPN_TEST_WG_PRESHARED_KEY").ok(),
    server_tunnel_ip: std::env::var("VPN_TEST_WG_SERVER_IP")
      .unwrap_or_else(|_| "10.0.0.1".to_string()),
  })
}

/// Get OpenVPN config from CI environment
fn get_ci_openvpn_config(host: &str, port: &str) -> Result<OpenVpnTestConfig, String> {
  if let Ok(raw_config) = std::env::var("VPN_TEST_OVPN_RAW_CONFIG") {
    return Ok(OpenVpnTestConfig {
      raw_config,
      remote_host: host.to_string(),
      remote_port: port.parse().unwrap_or(1194),
      protocol: "udp".to_string(),
    });
  }

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

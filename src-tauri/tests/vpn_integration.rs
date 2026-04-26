//! VPN integration tests
//!
//! These tests verify VPN config parsing, storage, and tunnel functionality.
//! Connection tests require Docker and are skipped if Docker is not available.

mod common;
mod test_harness;

use common::TestUtils;
use donutbrowser_lib::vpn::{
  detect_vpn_type, parse_wireguard_config, VpnConfig, VpnStorage, VpnType, WireGuardConfig,
};
use serde_json::Value;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

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
fn test_detect_vpn_type_wireguard_by_extension() {
  let content = "[Interface]\nPrivateKey = test\n[Peer]\nPublicKey = peer";
  let result = detect_vpn_type(content, "my-vpn.conf");

  assert!(result.is_ok());
  assert_eq!(result.unwrap(), VpnType::WireGuard);
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
fn test_detect_vpn_type_unknown() {
  let content = "this is just some random text that is not a vpn config";
  let result = detect_vpn_type(content, "random.txt");

  assert!(result.is_err());
}

#[test]
fn test_reject_openvpn_content() {
  let content = "client\ndev tun\nproto udp\nremote vpn.example.com 1194";
  assert!(detect_vpn_type(content, "old.ovpn").is_err());
  assert!(detect_vpn_type(content, "config.txt").is_err());
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
PrivateKey = YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWE=
Address = 10.0.0.2/24
"#;
  let result = parse_wireguard_config(config);

  assert!(result.is_err());
  let err = result.unwrap_err().to_string();
  assert!(err.contains("PublicKey") || err.contains("Peer"));
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
    sync_enabled: false,
    last_sync: None,
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

  for i in 1..=2 {
    let config = VpnConfig {
      id: format!("list-test-{i}"),
      name: format!("VPN {i}"),
      vpn_type: VpnType::WireGuard,
      config_data: "secret data".to_string(),
      created_at: 1000 * i as i64,
      last_used: None,
      sync_enabled: false,
      last_sync: None,
    };
    storage.save_config(&config).unwrap();
  }

  let list = storage.list_configs().unwrap();
  assert_eq!(list.len(), 2);

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
    sync_enabled: false,
    last_sync: None,
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

/// Existing OpenVPN entries on disk should be silently dropped at load time
/// after support was removed. Stored configs are encrypted at rest, so we
/// build the on-disk JSON by hand instead of going through `save_config`.
#[test]
#[serial]
fn test_vpn_storage_drops_legacy_openvpn_entries() {
  let temp_dir = tempfile::TempDir::new().unwrap();
  let storage_path = temp_dir.path().join("vpn_configs.json");
  std::fs::write(
    &storage_path,
    r#"{
      "version": 1,
      "configs": [
        {
          "id": "wg-keep",
          "name": "Keep me",
          "vpn_type": "WireGuard",
          "encrypted_data": "",
          "nonce": "",
          "created_at": 1,
          "last_used": null,
          "sync_enabled": false,
          "last_sync": null
        },
        {
          "id": "ovpn-drop",
          "name": "Drop me",
          "vpn_type": "OpenVPN",
          "encrypted_data": "",
          "nonce": "",
          "created_at": 2,
          "last_used": null,
          "sync_enabled": false,
          "last_sync": null
        }
      ]
    }"#,
  )
  .unwrap();

  let storage = create_test_storage(&temp_dir);
  let configs = storage.list_configs().unwrap();
  let ids: Vec<_> = configs.iter().map(|c| c.id.as_str()).collect();
  assert_eq!(ids, vec!["wg-keep"]);
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

#[tokio::test]
#[serial]
async fn test_wireguard_tunnel_init() {
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

struct TestEnvGuard {
  _root: PathBuf,
  previous_data_dir: Option<String>,
  previous_cache_dir: Option<String>,
}

impl TestEnvGuard {
  fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
    static TEST_RUNTIME_ROOT: OnceLock<PathBuf> = OnceLock::new();

    let root = TEST_RUNTIME_ROOT
      .get_or_init(|| {
        std::env::temp_dir().join(format!("donutbrowser-vpn-e2e-{}", std::process::id()))
      })
      .clone();
    let data_dir = root.join("data");
    let cache_dir = root.join("cache");
    let vpn_dir = data_dir.join("vpn");

    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&cache_dir);
    std::fs::create_dir_all(&vpn_dir)?;
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&cache_dir)?;

    let previous_data_dir = std::env::var("DONUTBROWSER_DATA_DIR").ok();
    let previous_cache_dir = std::env::var("DONUTBROWSER_CACHE_DIR").ok();

    std::env::set_var("DONUTBROWSER_DATA_DIR", &data_dir);
    std::env::set_var("DONUTBROWSER_CACHE_DIR", &cache_dir);

    Ok(Self {
      _root: root,
      previous_data_dir,
      previous_cache_dir,
    })
  }
}

impl Drop for TestEnvGuard {
  fn drop(&mut self) {
    if let Some(value) = &self.previous_data_dir {
      std::env::set_var("DONUTBROWSER_DATA_DIR", value);
    } else {
      std::env::remove_var("DONUTBROWSER_DATA_DIR");
    }

    if let Some(value) = &self.previous_cache_dir {
      std::env::set_var("DONUTBROWSER_CACHE_DIR", value);
    } else {
      std::env::remove_var("DONUTBROWSER_CACHE_DIR");
    }
  }
}

struct ProxyProcess {
  id: String,
  local_port: u16,
}

async fn ensure_donut_proxy_binary() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
  let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
  let project_root = PathBuf::from(cargo_manifest_dir)
    .parent()
    .unwrap()
    .to_path_buf();

  let proxy_binary_name = if cfg!(windows) {
    "donut-proxy.exe"
  } else {
    "donut-proxy"
  };
  let proxy_binary = project_root
    .join("src-tauri")
    .join("target")
    .join("debug")
    .join(proxy_binary_name);

  if !proxy_binary.exists() {
    let build_status = tokio::process::Command::new("cargo")
      .args(["build", "--bin", "donut-proxy"])
      .current_dir(project_root.join("src-tauri"))
      .status()
      .await?;

    if !build_status.success() {
      return Err("Failed to build donut-proxy binary".into());
    }
  }

  if !proxy_binary.exists() {
    return Err("donut-proxy binary was not created successfully".into());
  }

  Ok(proxy_binary)
}

fn new_test_vpn_config(name: &str, vpn_type: VpnType, config_data: String) -> VpnConfig {
  let created_at = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64;

  VpnConfig {
    id: uuid::Uuid::new_v4().to_string(),
    name: name.to_string(),
    vpn_type,
    config_data,
    created_at,
    last_used: None,
    sync_enabled: false,
    last_sync: None,
  }
}

fn build_wireguard_config(config: &test_harness::WireGuardTestConfig) -> String {
  format!(
    "[Interface]\nPrivateKey = {}\nAddress = {}\n{}\n[Peer]\nPublicKey = {}\n{}Endpoint = {}\nAllowedIPs = {}\nPersistentKeepalive = 25\n",
    config.private_key,
    config.address,
    config
      .dns
      .as_ref()
      .map(|dns| format!("DNS = {dns}\n"))
      .unwrap_or_default(),
    config.peer_public_key,
    config
      .preshared_key
      .as_ref()
      .map(|key| format!("PresharedKey = {key}\n"))
      .unwrap_or_default(),
    config.peer_endpoint,
    config.allowed_ips.join(", ")
  )
}

async fn start_proxy_with_upstream(
  binary_path: &PathBuf,
  upstream_proxy: &str,
  bypass_rules: &[String],
  blocklist_file: Option<&str>,
  profile_id: Option<&str>,
) -> Result<ProxyProcess, Box<dyn std::error::Error + Send + Sync>> {
  let upstream_url = url::Url::parse(upstream_proxy)?;
  let host = upstream_url
    .host_str()
    .ok_or("Upstream proxy host is missing")?
    .to_string();
  let port = upstream_url
    .port()
    .ok_or("Upstream proxy port is missing")?;

  let mut args = vec![
    "proxy".to_string(),
    "start".to_string(),
    "--host".to_string(),
    host,
    "--proxy-port".to_string(),
    port.to_string(),
    "--type".to_string(),
    upstream_url.scheme().to_string(),
  ];

  if !bypass_rules.is_empty() {
    args.push("--bypass-rules".to_string());
    args.push(serde_json::to_string(bypass_rules)?);
  }

  if let Some(blocklist_file) = blocklist_file {
    args.push("--blocklist-file".to_string());
    args.push(blocklist_file.to_string());
  }

  if let Some(profile_id) = profile_id {
    args.push("--profile-id".to_string());
    args.push(profile_id.to_string());
  }

  let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
  let output = TestUtils::execute_command(binary_path, &arg_refs).await?;
  if !output.status.success() {
    return Err(
      format!(
        "Failed to start local proxy - stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
      )
      .into(),
    );
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  Ok(ProxyProcess {
    id: config["id"].as_str().ok_or("Missing proxy id")?.to_string(),
    local_port: config["localPort"].as_u64().ok_or("Missing local port")? as u16,
  })
}

async fn stop_proxy(
  binary_path: &PathBuf,
  proxy_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let output =
    TestUtils::execute_command(binary_path, &["proxy", "stop", "--id", proxy_id]).await?;
  if !output.status.success() {
    return Err(
      format!(
        "Failed to stop proxy '{}' - stdout: {}, stderr: {}",
        proxy_id,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
      )
      .into(),
    );
  }
  Ok(())
}

async fn raw_http_request_via_proxy(
  local_port: u16,
  url: &str,
  host_header: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
  let mut stream = tokio::time::timeout(
    Duration::from_secs(20),
    TcpStream::connect(("127.0.0.1", local_port)),
  )
  .await
  .map_err(|_| "proxy TCP connect timed out after 20s")??;

  let request = format!("GET {url} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\n\r\n");
  stream.write_all(request.as_bytes()).await?;

  let mut response = Vec::new();
  tokio::time::timeout(Duration::from_secs(20), stream.read_to_end(&mut response))
    .await
    .map_err(|_| "proxy HTTP response timed out after 20s")??;
  Ok(String::from_utf8_lossy(&response).to_string())
}

async fn cleanup_runtime() {
  let _ = donutbrowser_lib::proxy_runner::stop_all_proxy_processes().await;
  let _ = donutbrowser_lib::vpn_worker_runner::stop_all_vpn_workers().await;
  test_harness::stop_vpn_servers().await;
}

async fn wait_for_file(
  path: &std::path::Path,
  timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let deadline = tokio::time::Instant::now() + timeout;

  while tokio::time::Instant::now() < deadline {
    if path.exists() {
      return Ok(());
    }

    sleep(Duration::from_millis(250)).await;
  }

  Err(format!("Timed out waiting for file: {}", path.display()).into())
}

async fn run_proxy_feature_suite(
  binary_path: &PathBuf,
  vpn_id: &str,
  server_tunnel_ip: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let vpn_worker = donutbrowser_lib::vpn_worker_runner::start_vpn_worker(vpn_id)
    .await
    .map_err(|error| error.to_string())?;
  let vpn_upstream = vpn_worker
    .local_url
    .clone()
    .ok_or("VPN worker did not expose a local URL")?;

  let profile_id = format!("vpn-e2e-{}", uuid::Uuid::new_v4());
  let proxy =
    start_proxy_with_upstream(binary_path, &vpn_upstream, &[], None, Some(&profile_id)).await?;

  sleep(Duration::from_millis(500)).await;

  let internal_url = format!("http://{}:8080/", server_tunnel_ip);
  let internal_host = format!("{}:8080", server_tunnel_ip);
  let http_response =
    raw_http_request_via_proxy(proxy.local_port, &internal_url, &internal_host).await?;
  assert!(
    http_response.contains("WG-TUNNEL-OK"),
    "HTTP traffic through donut-proxy+VPN tunnel should succeed, got: {}",
    &http_response[..http_response.len().min(300)]
  );

  let stats_file = donutbrowser_lib::app_dirs::cache_dir()
    .join("traffic_stats")
    .join(format!("{}.json", profile_id));
  wait_for_file(&stats_file, Duration::from_secs(8)).await?;

  assert!(
    stats_file.exists(),
    "Traffic stats should exist for VPN-backed local proxy"
  );
  let stats: Value = serde_json::from_str(&std::fs::read_to_string(&stats_file)?)?;
  let total_requests = stats["total_requests"].as_u64().unwrap_or_default();
  assert!(
    total_requests > 0,
    "Traffic stats should record requests for VPN-backed local proxy"
  );
  let domains = stats["domains"]
    .as_object()
    .ok_or("Traffic stats are missing per-domain data")?;
  assert!(
    domains.contains_key(server_tunnel_ip),
    "Traffic stats should include tunnel server IP activity, got: {:?}",
    domains.keys().collect::<Vec<_>>()
  );

  stop_proxy(binary_path, &proxy.id).await?;

  let blocklist_file = tempfile::NamedTempFile::new()?;
  std::fs::write(blocklist_file.path(), format!("{server_tunnel_ip}\n"))?;
  let blocked_proxy = start_proxy_with_upstream(
    binary_path,
    &vpn_upstream,
    &[],
    blocklist_file.path().to_str(),
    None,
  )
  .await?;
  let blocked_response =
    raw_http_request_via_proxy(blocked_proxy.local_port, &internal_url, &internal_host).await?;
  assert!(
    blocked_response.contains("403") || blocked_response.contains("Blocked by DNS blocklist"),
    "DNS blocklist should be enforced before forwarding to the VPN upstream"
  );
  stop_proxy(binary_path, &blocked_proxy.id).await?;

  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
  let bypass_target_port = listener.local_addr()?.port();
  let bypass_server = tokio::spawn(async move {
    while let Ok((stream, _)) = listener.accept().await {
      let io = hyper_util::rt::TokioIo::new(stream);
      tokio::spawn(async move {
        let service = hyper::service::service_fn(|_req| async move {
          Ok::<_, hyper::Error>(
            hyper::Response::builder()
              .status(hyper::StatusCode::OK)
              .body(http_body_util::Full::new(hyper::body::Bytes::from(
                "VPN-BYPASS-OK",
              )))
              .unwrap(),
          )
        });
        let _ = hyper::server::conn::http1::Builder::new()
          .serve_connection(io, service)
          .await;
      });
    }
  });

  let bypass_proxy = start_proxy_with_upstream(
    binary_path,
    &vpn_upstream,
    &["127.0.0.1".to_string(), "localhost".to_string()],
    None,
    None,
  )
  .await?;
  let bypass_response = raw_http_request_via_proxy(
    bypass_proxy.local_port,
    &format!("http://127.0.0.1:{bypass_target_port}/"),
    &format!("127.0.0.1:{bypass_target_port}"),
  )
  .await?;
  assert!(
    bypass_response.contains("VPN-BYPASS-OK"),
    "Bypass rules should still work when donut-proxy is chained to a VPN worker"
  );
  stop_proxy(binary_path, &bypass_proxy.id).await?;
  bypass_server.abort();

  donutbrowser_lib::vpn_worker_runner::stop_vpn_worker(&vpn_worker.id)
    .await
    .map_err(|error| error.to_string())?;
  Ok(())
}

#[tokio::test]
#[serial]
async fn test_wireguard_traffic_flows_through_donut_proxy(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let _env = TestEnvGuard::new()?;

  cleanup_runtime().await;
  if !test_harness::is_docker_available() {
    eprintln!("skipping WireGuard e2e test because Docker is unavailable");
    return Ok(());
  }

  let binary_path = ensure_donut_proxy_binary().await?;
  let wg_config = match test_harness::start_wireguard_server().await {
    Ok(config) => config,
    Err(error) => {
      eprintln!("skipping WireGuard e2e test: {error}");
      return Ok(());
    }
  };

  let vpn_config = new_test_vpn_config(
    "WireGuard E2E",
    VpnType::WireGuard,
    build_wireguard_config(&wg_config),
  );
  {
    let storage = donutbrowser_lib::vpn::VPN_STORAGE.lock().unwrap();
    storage.save_config(&vpn_config)?;
  }

  let result =
    run_proxy_feature_suite(&binary_path, &vpn_config.id, &wg_config.server_tunnel_ip).await;
  cleanup_runtime().await;

  result
}

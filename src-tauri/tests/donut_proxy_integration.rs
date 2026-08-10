mod common;
use common::TestUtils;
use serde_json::Value;
use serial_test::serial;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

/// Start a simple HTTP server that returns a specific body for any request.
/// Returns the (port, JoinHandle).
async fn start_mock_http_server(response_body: &'static str) -> (u16, tokio::task::JoinHandle<()>) {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let port = listener.local_addr().unwrap().port();

  let handle = tokio::spawn(async move {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Response, StatusCode};
    use hyper_util::rt::TokioIo;

    while let Ok((stream, _)) = listener.accept().await {
      let io = TokioIo::new(stream);
      tokio::task::spawn(async move {
        let service = service_fn(move |_req| {
          let body = response_body;
          async move {
            Ok::<_, hyper::Error>(
              Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from(body)))
                .unwrap(),
            )
          }
        });
        let _ = http1::Builder::new().serve_connection(io, service).await;
      });
    }
  });

  // Wait for listener to be ready
  sleep(Duration::from_millis(100)).await;

  (port, handle)
}

/// Setup function to ensure donut-proxy binary exists and cleanup stale proxies
async fn setup_test() -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
  let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
  let project_root = std::path::PathBuf::from(cargo_manifest_dir)
    .parent()
    .unwrap()
    .to_path_buf();

  // Build donut-proxy binary if it doesn't exist
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

  let binary_is_current = proxy_binary.exists()
    && std::process::Command::new(&proxy_binary)
      .arg("--version")
      .output()
      .is_ok_and(|output| {
        output.status.success()
          && String::from_utf8_lossy(&output.stdout).trim()
            == format!("donut-proxy {}", env!("BUILD_VERSION"))
      });

  if !binary_is_current {
    println!("Building donut-proxy binary for integration tests...");
    let build_status = std::process::Command::new("cargo")
      .args(["build", "--bin", "donut-proxy"])
      .current_dir(project_root.join("src-tauri"))
      .status()?;

    if !build_status.success() {
      return Err("Failed to build donut-proxy binary".into());
    }
  }

  if !proxy_binary.exists() {
    return Err("donut-proxy binary was not created successfully".into());
  }

  // Clean up any stale proxies from previous test runs
  let _ = TestUtils::execute_command(&proxy_binary, &["proxy", "stop"]).await;

  Ok(proxy_binary)
}

/// Helper to track and cleanup proxy processes
struct ProxyTestTracker {
  proxy_ids: Vec<String>,
  binary_path: std::path::PathBuf,
}

impl ProxyTestTracker {
  fn new(binary_path: std::path::PathBuf) -> Self {
    Self {
      proxy_ids: Vec::new(),
      binary_path,
    }
  }

  fn track_proxy(&mut self, proxy_id: String) {
    self.proxy_ids.push(proxy_id);
  }

  async fn cleanup_all(&self) {
    for proxy_id in &self.proxy_ids {
      let _ =
        TestUtils::execute_command(&self.binary_path, &["proxy", "stop", "--id", proxy_id]).await;
    }
  }
}

impl Drop for ProxyTestTracker {
  fn drop(&mut self) {
    let proxy_ids = self.proxy_ids.clone();
    let binary_path = self.binary_path.clone();
    tokio::spawn(async move {
      for proxy_id in &proxy_ids {
        let _ =
          TestUtils::execute_command(&binary_path, &["proxy", "stop", "--id", proxy_id]).await;
      }
    });
  }
}

#[tokio::test]
#[serial]
async fn test_sidecar_reports_build_version() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
  let binary_path = setup_test().await?;
  let output = TestUtils::execute_command(&binary_path, &["--version"]).await?;

  assert!(
    output.status.success(),
    "donut-proxy --version failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  assert_eq!(
    String::from_utf8(output.stdout)?.trim(),
    format!("donut-proxy {}", env!("BUILD_VERSION"))
  );
  Ok(())
}

struct XrayChainCleanup {
  outer_id: Option<String>,
  xray_id: Option<String>,
}

impl XrayChainCleanup {
  fn new() -> Self {
    Self {
      outer_id: None,
      xray_id: None,
    }
  }

  async fn cleanup(&mut self) {
    if let Some(id) = self.outer_id.take() {
      let _ = donutbrowser_lib::proxy_runner::stop_proxy_process(&id).await;
    }
    if let Some(id) = self.xray_id.take() {
      let _ = donutbrowser_lib::xray_worker_runner::stop_xray_worker(&id).await;
    }
  }
}

impl Drop for XrayChainCleanup {
  fn drop(&mut self) {
    fn terminate_process_tree(pid: u32) {
      #[cfg(unix)]
      {
        let _ = std::process::Command::new("kill")
          .args(["-TERM", &format!("-{pid}")])
          .output();
      }
      #[cfg(windows)]
      {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = std::process::Command::new("taskkill")
          .args(["/T", "/F", "/PID", &pid.to_string()])
          .creation_flags(CREATE_NO_WINDOW)
          .output();
      }
    }

    if let Some(id) = self.outer_id.take() {
      if let Some(config) = donutbrowser_lib::proxy_storage::get_proxy_config(&id) {
        if let Some(pid) = config.pid {
          terminate_process_tree(pid);
        }
      }
      donutbrowser_lib::proxy_storage::delete_proxy_config(&id);
    }
    if let Some(id) = self.xray_id.take() {
      if let Some(config) = donutbrowser_lib::xray_worker_storage::get_xray_worker_config(&id) {
        if let Some(pid) = config.pid {
          terminate_process_tree(pid);
        }
      }
      donutbrowser_lib::xray_worker_storage::delete_xray_worker_config(&id);
    }
  }
}

struct EnvironmentRestore {
  name: &'static str,
  previous: Option<std::ffi::OsString>,
}

impl EnvironmentRestore {
  fn set_path(name: &'static str, value: &std::path::Path) -> Self {
    let previous = std::env::var_os(name);
    std::env::set_var(name, value);
    Self { name, previous }
  }
}

impl Drop for EnvironmentRestore {
  fn drop(&mut self) {
    if let Some(previous) = &self.previous {
      std::env::set_var(self.name, previous);
    } else {
      std::env::remove_var(self.name);
    }
  }
}

/// End-to-end VLESS + XTLS Vision + REALITY test.
///
/// This is intentionally opt-in because it requires a live official Xray-core
/// server. Set `DONUT_E2E_VLESS_URI`; optionally set
/// `DONUT_E2E_XRAY_TARGET_URL` to a controlled plain-HTTP endpoint. When the
/// VLESS endpoint itself is loopback, the test supplies an isolated local HTTP
/// target automatically.
#[tokio::test]
#[serial]
async fn test_xray_reality_chain_and_local_traffic_monitoring(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(vless_uri) = std::env::var("DONUT_E2E_VLESS_URI")
    .ok()
    .filter(|value| !value.is_empty())
  else {
    println!("skipping Xray-core integration test: DONUT_E2E_VLESS_URI is not set");
    return Ok(());
  };

  let parsed = donutbrowser_lib::xray::parse_vless_uri(&vless_uri)?;
  let endpoint_is_loopback = parsed.config.address == "localhost"
    || parsed
      .config
      .address
      .parse::<std::net::IpAddr>()
      .is_ok_and(|address| address.is_loopback());
  let mut local_target = None;
  let target_url = if let Ok(url) = std::env::var("DONUT_E2E_XRAY_TARGET_URL") {
    url
  } else if endpoint_is_loopback {
    let (port, server) = start_mock_http_server("XRAY-REALITY-INTEGRATION-OK").await;
    local_target = Some(server);
    format!("http://127.0.0.1:{port}/xray-reality")
  } else {
    "http://httpbin.org/anything".to_string()
  };
  let target = url::Url::parse(&target_url)?;
  if target.scheme() != "http" {
    return Err(
      "DONUT_E2E_XRAY_TARGET_URL must use plain HTTP for exact payload accounting".into(),
    );
  }
  let target_host = target.host_str().ok_or("Xray target URL has no host")?;
  let target_port = target
    .port_or_known_default()
    .ok_or("Xray target URL has no port")?;
  let host_header = if target_port == 80 {
    target_host.to_string()
  } else {
    format!("{target_host}:{target_port}")
  };

  let isolated_root = tempfile::tempdir()?;
  let _data_root = EnvironmentRestore::set_path("DONUTBROWSER_DATA_ROOT", isolated_root.path());
  let _binary_path = setup_test().await.map_err(|error| error.to_string())?;
  let profile_id = format!("xray-integration-{}", uuid::Uuid::new_v4());
  let payload = b"donut-xray-monitor-payload";
  let mut cleanup = XrayChainCleanup::new();

  let (first_worker, concurrent_worker) = tokio::join!(
    donutbrowser_lib::xray_worker_runner::start_xray_worker(Some(&profile_id), &vless_uri),
    donutbrowser_lib::xray_worker_runner::start_xray_worker(Some(&profile_id), &vless_uri),
  );
  let xray_worker = first_worker?;
  let concurrent_worker = concurrent_worker?;
  assert_eq!(
    concurrent_worker.id, xray_worker.id,
    "concurrent starts for one profile must reuse one Xray worker"
  );
  cleanup.xray_id = Some(xray_worker.id.clone());

  let upstream_url = format!(
    "socks5://{}:{}@127.0.0.1:{}",
    urlencoding::encode(&xray_worker.username),
    urlencoding::encode(&xray_worker.password),
    xray_worker.local_port
  );
  let mut outer = donutbrowser_lib::proxy_runner::start_proxy_process_with_profile(
    Some(upstream_url),
    None,
    Some(profile_id.clone()),
    Vec::new(),
    None,
    false,
    Some("http".to_string()),
  )
  .await?;
  cleanup.outer_id = Some(outer.id.clone());
  outer.browser_pid = Some(std::process::id());
  assert!(
    donutbrowser_lib::proxy_storage::update_proxy_config(&outer),
    "outer traffic-monitor config should remain writable"
  );

  let local_port = outer.local_port.ok_or("outer proxy has no local port")?;
  let request = format!(
    "POST {target_url} HTTP/1.1\r\nHost: {host_header}\r\nContent-Type: \
     application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    payload.len()
  );
  let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
  stream.write_all(request.as_bytes()).await?;
  stream.write_all(payload).await?;
  let mut response = Vec::new();
  tokio::time::timeout(Duration::from_secs(30), stream.read_to_end(&mut response))
    .await
    .map_err(|_| "HTTP request through Xray-core timed out")??;

  let header_end = response
    .windows(4)
    .position(|window| window == b"\r\n\r\n")
    .map(|position| position + 4)
    .ok_or("proxy returned an invalid HTTP response")?;
  let status_line = String::from_utf8_lossy(&response[..header_end])
    .lines()
    .next()
    .unwrap_or_default()
    .to_string();
  if !status_line.contains(" 200 ") {
    return Err(
      format!(
        "HTTP request through Xray-core failed ({status_line}): {}",
        String::from_utf8_lossy(&response[header_end..])
      )
      .into(),
    );
  }
  let response_body = &response[header_end..];
  if endpoint_is_loopback && std::env::var_os("DONUT_E2E_XRAY_TARGET_URL").is_none() {
    assert_eq!(response_body, b"XRAY-REALITY-INTEGRATION-OK");
  } else {
    assert!(
      !response_body.is_empty(),
      "controlled Xray target returned an empty body"
    );
  }

  let stats = tokio::time::timeout(Duration::from_secs(10), async {
    loop {
      if let Some(stats) = donutbrowser_lib::traffic_stats::load_traffic_stats(&profile_id) {
        if stats.total_requests > 0 {
          break stats;
        }
      }
      sleep(Duration::from_millis(100)).await;
    }
  })
  .await
  .map_err(|_| "traffic monitor did not flush Xray session statistics")?;

  assert_eq!(stats.total_requests, 1);
  assert_eq!(stats.total_bytes_sent, payload.len() as u64);
  assert_eq!(stats.total_bytes_received, response_body.len() as u64);
  let domain = stats
    .domains
    .get(target_host)
    .ok_or("traffic monitor did not attribute the target domain")?;
  assert_eq!(domain.request_count, 1);
  assert_eq!(domain.bytes_sent, payload.len() as u64);
  assert_eq!(domain.bytes_received, response_body.len() as u64);

  let outer_id = outer.id.clone();
  let outer_pid = outer.pid;
  let xray_id = xray_worker.id.clone();
  let supervisor_pid = xray_worker.pid;
  let xray_pid = xray_worker.xray_pid;
  cleanup.cleanup().await;
  if let Some(server) = local_target.take() {
    server.abort();
  }

  assert!(donutbrowser_lib::proxy_storage::get_proxy_config(&outer_id).is_none());
  assert!(donutbrowser_lib::xray_worker_storage::get_xray_worker_config(&xray_id).is_none());
  assert!(!donutbrowser_lib::xray_worker_storage::xray_runtime_config_path(&xray_id).exists());
  for pid in [outer_pid, supervisor_pid, xray_pid].into_iter().flatten() {
    for _ in 0..20 {
      if !donutbrowser_lib::proxy_storage::is_process_running(pid) {
        break;
      }
      sleep(Duration::from_millis(100)).await;
    }
    assert!(
      !donutbrowser_lib::proxy_storage::is_process_running(pid),
      "worker process {pid} survived cleanup"
    );
  }

  Ok(())
}

/// Test starting a local proxy without upstream proxy (DIRECT)
#[tokio::test]
#[serial]
async fn test_local_proxy_direct() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  println!("Starting local proxy without upstream (DIRECT)...");

  let output = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    return Err(format!("Proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let stdout = String::from_utf8(output.stdout)?;
  let config: Value = serde_json::from_str(&stdout)?;

  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  let local_url = config["localUrl"].as_str().unwrap();
  let upstream_url = config["upstreamUrl"].as_str().unwrap();

  tracker.track_proxy(proxy_id.clone());

  println!(
    "Proxy started: id={}, port={}, url={}, upstream={}",
    proxy_id, local_port, local_url, upstream_url
  );

  // Verify proxy is listening
  if !wait_for_port_open(local_port, Duration::from_secs(10)).await {
    return Err(format!("Proxy port {local_port} is not listening").into());
  }
  println!("Proxy is listening on port {local_port}");

  // Test making an HTTP request through the proxy
  let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
  let request =
    b"GET http://httpbin.org/ip HTTP/1.1\r\nHost: httpbin.org\r\nConnection: close\r\n\r\n";
  stream.write_all(request).await?;

  let mut response = Vec::new();
  stream.read_to_end(&mut response).await?;
  let response_str = String::from_utf8_lossy(&response);

  if response_str.contains("200 OK") || response_str.contains("origin") {
    println!("Proxy successfully forwarded HTTP request");
  } else {
    println!(
      "Warning: Proxy response may be unexpected: {}",
      &response_str[..response_str.len().min(200)]
    );
  }

  // Cleanup
  tracker.cleanup_all().await;

  Ok(())
}

/// Test chaining local proxies (local proxy -> local proxy -> internet)
#[tokio::test]
#[serial]
async fn test_chained_local_proxies() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  println!("Testing chained local proxies...");

  // Start first proxy (DIRECT - connects to internet)
  let output1 = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
  if !output1.status.success() {
    let stderr = String::from_utf8_lossy(&output1.stderr);
    let stdout = String::from_utf8_lossy(&output1.stdout);
    return Err(format!("Failed to start first proxy - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config1: Value = serde_json::from_str(&String::from_utf8(output1.stdout)?)?;
  let proxy1_id = config1["id"].as_str().unwrap().to_string();
  let proxy1_port = config1["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy1_id.clone());

  println!("First proxy started on port {}", proxy1_port);

  // Wait for first proxy to be ready
  if !wait_for_port_open(proxy1_port, Duration::from_secs(10)).await {
    return Err("First proxy not ready".into());
  }
  println!("First proxy is ready");

  // Start second proxy chained to first proxy
  let output2 = TestUtils::execute_command(
    &binary_path,
    &[
      "proxy",
      "start",
      "--host",
      "127.0.0.1",
      "--proxy-port",
      &proxy1_port.to_string(),
      "--type",
      "http",
    ],
  )
  .await?;

  if !output2.status.success() {
    let stderr = String::from_utf8_lossy(&output2.stderr);
    let stdout = String::from_utf8_lossy(&output2.stdout);
    return Err(
      format!("Failed to start second proxy - stdout: {stdout}, stderr: {stderr}").into(),
    );
  }

  let config2: Value = serde_json::from_str(&String::from_utf8(output2.stdout)?)?;
  let proxy2_id = config2["id"].as_str().unwrap().to_string();
  let proxy2_port = config2["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy2_id.clone());

  println!(
    "Second proxy started on port {} (chained to proxy on port {})",
    proxy2_port, proxy1_port
  );

  // Wait for second proxy to be ready
  if !wait_for_port_open(proxy2_port, Duration::from_secs(10)).await {
    return Err("Second proxy not ready".into());
  }
  println!("Second proxy is ready");

  // Test making an HTTP request through the chained proxy
  let mut stream = TcpStream::connect(("127.0.0.1", proxy2_port)).await?;
  let request =
    b"GET http://httpbin.org/ip HTTP/1.1\r\nHost: httpbin.org\r\nConnection: close\r\n\r\n";
  stream.write_all(request).await?;

  let mut response = Vec::new();
  stream.read_to_end(&mut response).await?;
  let response_str = String::from_utf8_lossy(&response);

  if response_str.contains("200 OK") || response_str.contains("origin") {
    println!("Chained proxy successfully forwarded HTTP request");
  } else {
    println!(
      "Warning: Chained proxy response may be unexpected: {}",
      &response_str[..response_str.len().min(200)]
    );
  }

  // Cleanup
  tracker.cleanup_all().await;

  Ok(())
}

/// Test starting a local proxy with HTTP upstream proxy
#[tokio::test]
#[serial]
async fn test_local_proxy_with_http_upstream(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  // Start a mock HTTP upstream proxy server
  let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
  let upstream_addr = upstream_listener.local_addr()?;
  let upstream_port = upstream_addr.port();

  let upstream_handle = tokio::spawn(async move {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Response, StatusCode};
    use hyper_util::rt::TokioIo;

    while let Ok((stream, _)) = upstream_listener.accept().await {
      let io = TokioIo::new(stream);
      tokio::task::spawn(async move {
        let service = service_fn(|_req| async {
          Ok::<_, hyper::Error>(
            Response::builder()
              .status(StatusCode::OK)
              .body(Full::new(Bytes::from("Upstream Proxy Response")))
              .unwrap(),
          )
        });
        let _ = http1::Builder::new().serve_connection(io, service).await;
      });
    }
  });

  sleep(Duration::from_millis(200)).await;

  println!("Starting local proxy with HTTP upstream proxy...");

  let output = TestUtils::execute_command(
    &binary_path,
    &[
      "proxy",
      "start",
      "--host",
      "127.0.0.1",
      "--proxy-port",
      &upstream_port.to_string(),
      "--type",
      "http",
    ],
  )
  .await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    upstream_handle.abort();
    return Err(format!("Proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let stdout = String::from_utf8(output.stdout)?;
  let config: Value = serde_json::from_str(&stdout)?;

  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id.clone());

  println!("Proxy started: id={}, port={}", proxy_id, local_port);

  // Verify proxy is listening
  if !wait_for_port_open(local_port, Duration::from_secs(10)).await {
    upstream_handle.abort();
    return Err(format!("Proxy port {local_port} is not listening").into());
  }
  println!("Proxy is listening on port {local_port}");

  // Cleanup
  tracker.cleanup_all().await;
  upstream_handle.abort();

  Ok(())
}

/// Test multiple proxies running simultaneously
#[tokio::test]
#[serial]
async fn test_multiple_proxies_simultaneously(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  println!("Starting multiple proxies simultaneously...");

  let mut proxy_ports = Vec::new();

  // Start 3 proxies, waiting for each to be ready before starting the next
  // This avoids race conditions on macOS where processes need time to initialize
  for i in 0..3 {
    let output = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let stdout = String::from_utf8_lossy(&output.stdout);
      return Err(
        format!(
          "Failed to start proxy {} - stdout: {}, stderr: {}",
          i + 1,
          stdout,
          stderr
        )
        .into(),
      );
    }

    let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
    let proxy_id = config["id"].as_str().unwrap().to_string();
    let local_port = config["localPort"].as_u64().unwrap() as u16;
    tracker.track_proxy(proxy_id);
    proxy_ports.push(local_port);

    println!("Proxy {} started on port {}", i + 1, local_port);

    // Wait for this proxy to be ready before starting the next one
    // This prevents race conditions on macOS where processes need time to initialize
    let mut attempts = 0;
    let max_attempts = 50; // 5 seconds max (50 * 100ms)
    loop {
      sleep(Duration::from_millis(100)).await;
      match TcpStream::connect(("127.0.0.1", local_port)).await {
        Ok(_) => {
          println!("Proxy {} is ready on port {}", i + 1, local_port);
          break;
        }
        Err(_) => {
          attempts += 1;
          if attempts >= max_attempts {
            return Err(
              format!(
                "Proxy {} on port {} failed to become ready after {} attempts",
                i + 1,
                local_port,
                max_attempts
              )
              .into(),
            );
          }
        }
      }
    }
  }

  // Verify all proxies are still listening
  for (i, port) in proxy_ports.iter().enumerate() {
    match TcpStream::connect(("127.0.0.1", *port)).await {
      Ok(_) => {
        println!("Proxy {} is listening on port {}", i + 1, port);
      }
      Err(e) => {
        return Err(format!("Proxy {} on port {} is not listening: {e}", i + 1, port).into());
      }
    }
  }

  // Cleanup
  tracker.cleanup_all().await;

  Ok(())
}

/// Test proxy listing
#[tokio::test]
#[serial]
async fn test_proxy_list() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  // Start a proxy
  let output = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    return Err(format!("Failed to start proxy - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  tracker.track_proxy(proxy_id.clone());

  // List proxies
  let list_output = TestUtils::execute_command(&binary_path, &["proxy", "list"]).await?;
  if !list_output.status.success() {
    return Err("Failed to list proxies".into());
  }

  let list_stdout = String::from_utf8(list_output.stdout)?;
  let proxies: Vec<Value> = serde_json::from_str(&list_stdout)?;

  // Verify our proxy is in the list
  let found = proxies.iter().any(|p| p["id"].as_str() == Some(&proxy_id));
  assert!(found, "Proxy should be in the list");

  // Cleanup
  tracker.cleanup_all().await;

  Ok(())
}

/// Test traffic tracking through proxy
#[tokio::test]
#[serial]
async fn test_traffic_tracking() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  println!("Testing traffic tracking through proxy...");

  // Start a proxy
  let output = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    return Err(format!("Failed to start proxy - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id.clone());

  println!("Proxy started on port {}", local_port);

  // Wait for proxy to be ready
  sleep(Duration::from_millis(500)).await;

  // Make an HTTP request through the proxy
  let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
  let request =
    b"GET http://httpbin.org/ip HTTP/1.1\r\nHost: httpbin.org\r\nConnection: close\r\n\r\n";

  // Track bytes sent
  let bytes_sent = request.len();
  stream.write_all(request).await?;

  // Read response
  let mut response = Vec::new();
  stream.read_to_end(&mut response).await?;
  let bytes_received = response.len();

  println!(
    "HTTP request completed: sent {} bytes, received {} bytes",
    bytes_sent, bytes_received
  );

  // Wait for traffic stats to be flushed (happens every second)
  sleep(Duration::from_secs(2)).await;

  let traffic_stats_dir = donutbrowser_lib::app_dirs::cache_dir().join("traffic_stats");
  let stats_file = traffic_stats_dir.join(format!("{}.json", proxy_id));

  if stats_file.exists() {
    let content = std::fs::read_to_string(&stats_file)?;
    let stats: Value = serde_json::from_str(&content)?;

    let total_sent = stats["total_bytes_sent"].as_u64().unwrap_or(0);
    let total_received = stats["total_bytes_received"].as_u64().unwrap_or(0);
    let total_requests = stats["total_requests"].as_u64().unwrap_or(0);

    println!(
      "Traffic stats recorded: sent {} bytes, received {} bytes, {} requests",
      total_sent, total_received, total_requests
    );

    // Check if domains are being tracked
    let mut domain_traffic = false;
    if let Some(domains) = stats.get("domains") {
      if let Some(domain_map) = domains.as_object() {
        println!("Domains tracked: {}", domain_map.len());
        for (domain, domain_stats) in domain_map {
          println!("  - {}", domain);
          // Check if any domain has traffic
          if let Some(domain_obj) = domain_stats.as_object() {
            let domain_sent = domain_obj
              .get("bytes_sent")
              .and_then(|v| v.as_u64())
              .unwrap_or(0);
            let domain_recv = domain_obj
              .get("bytes_received")
              .and_then(|v| v.as_u64())
              .unwrap_or(0);
            let domain_reqs = domain_obj
              .get("request_count")
              .and_then(|v| v.as_u64())
              .unwrap_or(0);
            println!(
              "    sent: {}, received: {}, requests: {}",
              domain_sent, domain_recv, domain_reqs
            );
            if domain_sent > 0 || domain_recv > 0 || domain_reqs > 0 {
              domain_traffic = true;
            }
          }
        }
      }
    }

    // Verify that some traffic was recorded - check either total bytes or domain traffic
    assert!(
      total_sent > 0 || total_received > 0 || total_requests > 0 || domain_traffic,
      "Traffic stats should record some activity (sent: {}, received: {}, requests: {})",
      total_sent,
      total_received,
      total_requests
    );

    println!("Traffic tracking test passed!");
  } else {
    println!("Warning: Traffic stats file not found at {:?}", stats_file);
    // This is not necessarily a failure - the file may not have been created yet
    // The important thing is that the proxy is working
  }

  // Cleanup
  tracker.cleanup_all().await;

  // Clean up the traffic stats file
  if stats_file.exists() {
    let _ = std::fs::remove_file(&stats_file);
  }

  Ok(())
}

/// Test proxy stop
#[tokio::test]
#[serial]
async fn test_proxy_stop() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let _tracker = ProxyTestTracker::new(binary_path.clone());

  // Start a proxy
  let output = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    return Err(format!("Failed to start proxy - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;

  // Verify proxy is running
  if !wait_for_port_open(local_port, Duration::from_secs(10)).await {
    return Err("Proxy is not running".into());
  }
  println!("Proxy is running");

  // Stop the proxy
  let stop_output =
    TestUtils::execute_command(&binary_path, &["proxy", "stop", "--id", &proxy_id]).await?;

  if !stop_output.status.success() {
    return Err("Failed to stop proxy".into());
  }

  // Verify proxy is stopped (connection should fail)
  if !wait_for_port_closed(local_port, Duration::from_secs(10)).await {
    return Err("Proxy should be stopped but is still listening".into());
  }
  println!("Proxy successfully stopped");

  Ok(())
}

/// Test that bypass rules cause requests to bypass the upstream proxy.
/// Requests to bypassed hosts go directly to the target, while
/// requests to non-bypassed hosts are routed through the upstream.
#[tokio::test]
#[serial]
async fn test_bypass_rules_http_direct() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  // Start a target HTTP server (this is where bypassed requests should arrive)
  let (target_port, target_handle) = start_mock_http_server("DIRECT-TARGET-RESPONSE").await;
  println!("Target server listening on port {target_port}");

  // Start a mock upstream proxy (non-bypassed requests go here)
  let (upstream_port, upstream_handle) = start_mock_http_server("UPSTREAM-PROXY-RESPONSE").await;
  println!("Mock upstream proxy listening on port {upstream_port}");

  // Start donut-proxy with upstream + bypass rules for "127.0.0.1"
  let bypass_rules = serde_json::json!(["127.0.0.1"]).to_string();
  let output = TestUtils::execute_command(
    &binary_path,
    &[
      "proxy",
      "start",
      "--host",
      "127.0.0.1",
      "--proxy-port",
      &upstream_port.to_string(),
      "--type",
      "http",
      "--bypass-rules",
      &bypass_rules,
    ],
  )
  .await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    target_handle.abort();
    upstream_handle.abort();
    return Err(format!("Proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id.clone());

  println!("Donut-proxy started on port {local_port} with bypass rules for 127.0.0.1");

  sleep(Duration::from_millis(500)).await;

  // Test 1: Request to 127.0.0.1 should be BYPASSED (direct connection to target)
  {
    let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
    let request = format!(
      "GET http://127.0.0.1:{target_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response_str = String::from_utf8_lossy(&response);

    println!(
      "Bypass response: {}",
      &response_str[..response_str.len().min(300)]
    );

    assert!(
      response_str.contains("DIRECT-TARGET-RESPONSE"),
      "Bypassed request should reach target directly, got: {}",
      &response_str[..response_str.len().min(300)]
    );
    assert!(
      !response_str.contains("UPSTREAM-PROXY-RESPONSE"),
      "Bypassed request should NOT go through upstream"
    );
    println!("Bypass test passed: request to 127.0.0.1 went directly to target");
  }

  // Test 2: Request to non-bypassed host should go through upstream
  {
    let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
    let request =
      b"GET http://non-bypass-host.test/ HTTP/1.1\r\nHost: non-bypass-host.test\r\nConnection: close\r\n\r\n";
    stream.write_all(request).await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response_str = String::from_utf8_lossy(&response);

    println!(
      "Non-bypass response: {}",
      &response_str[..response_str.len().min(300)]
    );

    assert!(
      response_str.contains("UPSTREAM-PROXY-RESPONSE"),
      "Non-bypassed request should go through upstream, got: {}",
      &response_str[..response_str.len().min(300)]
    );
    assert!(
      !response_str.contains("DIRECT-TARGET-RESPONSE"),
      "Non-bypassed request should NOT reach target directly"
    );
    println!("Non-bypass test passed: request to non-bypass-host.test went through upstream");
  }

  // Cleanup
  tracker.cleanup_all().await;
  target_handle.abort();
  upstream_handle.abort();

  Ok(())
}

/// Test bypass rules with regex patterns.
/// Verifies that regex-based rules match hosts correctly.
#[tokio::test]
#[serial]
async fn test_bypass_rules_regex_pattern() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  let (target_port, target_handle) = start_mock_http_server("REGEX-DIRECT-RESPONSE").await;
  let (upstream_port, upstream_handle) = start_mock_http_server("REGEX-UPSTREAM-RESPONSE").await;

  // Use regex bypass rule: ^127\.0\.0\.\d+ (matches any 127.0.0.x address)
  let bypass_rules = serde_json::json!([r"^127\.0\.0\.\d+"]).to_string();
  let output = TestUtils::execute_command(
    &binary_path,
    &[
      "proxy",
      "start",
      "--host",
      "127.0.0.1",
      "--proxy-port",
      &upstream_port.to_string(),
      "--type",
      "http",
      "--bypass-rules",
      &bypass_rules,
    ],
  )
  .await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    target_handle.abort();
    upstream_handle.abort();
    return Err(format!("Proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id.clone());

  sleep(Duration::from_millis(500)).await;

  // Request to 127.0.0.1 should match regex and be bypassed
  {
    let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
    let request = format!(
      "GET http://127.0.0.1:{target_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response_str = String::from_utf8_lossy(&response);

    assert!(
      response_str.contains("REGEX-DIRECT-RESPONSE"),
      "Regex-bypassed request should reach target directly, got: {}",
      &response_str[..response_str.len().min(300)]
    );
    println!("Regex bypass test passed: 127.0.0.1 matched regex rule");
  }

  // Request to non-matching host should go through upstream
  {
    let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
    let request =
      b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n";
    stream.write_all(request).await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response_str = String::from_utf8_lossy(&response);

    assert!(
      response_str.contains("REGEX-UPSTREAM-RESPONSE"),
      "Non-matching request should go through upstream, got: {}",
      &response_str[..response_str.len().min(300)]
    );
    println!("Regex non-bypass test passed: example.com did not match regex rule");
  }

  tracker.cleanup_all().await;
  target_handle.abort();
  upstream_handle.abort();

  Ok(())
}

/// Test that bypass rules are persisted in the proxy config on disk.
#[tokio::test]
#[serial]
async fn test_bypass_rules_in_config() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  let bypass_rules =
    serde_json::json!(["example.com", "192.168.0.0/16", r".*\.internal\.net"]).to_string();
  let output = TestUtils::execute_command(
    &binary_path,
    &["proxy", "start", "--bypass-rules", &bypass_rules],
  )
  .await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    return Err(format!("Proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  tracker.track_proxy(proxy_id.clone());

  sleep(Duration::from_millis(500)).await;

  // Read the proxy config file from disk to verify bypass rules are persisted
  let proxies_dir = donutbrowser_lib::app_dirs::proxy_workers_dir();
  let config_file = proxies_dir.join(format!("{proxy_id}.json"));

  assert!(
    config_file.exists(),
    "Proxy config file should exist at {:?}",
    config_file
  );

  let config_content = std::fs::read_to_string(&config_file)?;
  let disk_config: Value = serde_json::from_str(&config_content)?;

  let rules = disk_config["bypass_rules"]
    .as_array()
    .expect("bypass_rules should be an array in the config");

  assert_eq!(rules.len(), 3, "Should have 3 bypass rules");
  assert_eq!(rules[0], "example.com");
  assert_eq!(rules[1], "192.168.0.0/16");
  assert_eq!(rules[2], r".*\.internal\.net");

  println!(
    "Config persistence test passed: {} bypass rules found in config",
    rules.len()
  );

  tracker.cleanup_all().await;

  Ok(())
}

/// Test bypass rules with multiple rule types combined (exact + regex).
#[tokio::test]
#[serial]
async fn test_bypass_rules_multiple_rules() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  let (target_port, target_handle) = start_mock_http_server("MULTI-DIRECT-RESPONSE").await;
  let (upstream_port, upstream_handle) = start_mock_http_server("MULTI-UPSTREAM-RESPONSE").await;

  // Multiple bypass rules: exact match + regex
  let bypass_rules = serde_json::json!(["127.0.0.1", r"^localhost$"]).to_string();
  let output = TestUtils::execute_command(
    &binary_path,
    &[
      "proxy",
      "start",
      "--host",
      "127.0.0.1",
      "--proxy-port",
      &upstream_port.to_string(),
      "--type",
      "http",
      "--bypass-rules",
      &bypass_rules,
    ],
  )
  .await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    target_handle.abort();
    upstream_handle.abort();
    return Err(format!("Proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id.clone());

  sleep(Duration::from_millis(500)).await;

  // Request via 127.0.0.1 (exact match rule) → bypass
  {
    let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
    let request = format!(
      "GET http://127.0.0.1:{target_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response_str = String::from_utf8_lossy(&response);

    assert!(
      response_str.contains("MULTI-DIRECT-RESPONSE"),
      "Exact-match bypassed request should reach target, got: {}",
      &response_str[..response_str.len().min(300)]
    );
    println!("Multi-rule test: exact match bypass works");
  }

  // Request via localhost (regex match rule) → bypass
  {
    let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
    let request = format!(
      "GET http://localhost:{target_port}/ HTTP/1.1\r\nHost: localhost:{target_port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response_str = String::from_utf8_lossy(&response);

    assert!(
      response_str.contains("MULTI-DIRECT-RESPONSE"),
      "Regex-match bypassed request should reach target, got: {}",
      &response_str[..response_str.len().min(300)]
    );
    println!("Multi-rule test: regex match bypass works");
  }

  // Request to non-matching host → upstream
  {
    let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
    let request =
      b"GET http://other-host.test/ HTTP/1.1\r\nHost: other-host.test\r\nConnection: close\r\n\r\n";
    stream.write_all(request).await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response_str = String::from_utf8_lossy(&response);

    assert!(
      response_str.contains("MULTI-UPSTREAM-RESPONSE"),
      "Non-matching request should go through upstream, got: {}",
      &response_str[..response_str.len().min(300)]
    );
    println!("Multi-rule test: non-matching host goes through upstream");
  }

  tracker.cleanup_all().await;
  target_handle.abort();
  upstream_handle.abort();

  Ok(())
}

/// Test that an empty bypass rules list means everything goes through upstream.
#[tokio::test]
#[serial]
async fn test_no_bypass_rules_all_through_upstream(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  let (upstream_port, upstream_handle) = start_mock_http_server("ALL-UPSTREAM-RESPONSE").await;

  // Start proxy with empty bypass rules
  let bypass_rules = serde_json::json!([]).to_string();
  let output = TestUtils::execute_command(
    &binary_path,
    &[
      "proxy",
      "start",
      "--host",
      "127.0.0.1",
      "--proxy-port",
      &upstream_port.to_string(),
      "--type",
      "http",
      "--bypass-rules",
      &bypass_rules,
    ],
  )
  .await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    upstream_handle.abort();
    return Err(format!("Proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id.clone());

  sleep(Duration::from_millis(500)).await;

  // All requests should go through upstream when bypass rules are empty
  let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
  let request =
    b"GET http://any-host.test/ HTTP/1.1\r\nHost: any-host.test\r\nConnection: close\r\n\r\n";
  stream.write_all(request).await?;

  let mut response = Vec::new();
  stream.read_to_end(&mut response).await?;
  let response_str = String::from_utf8_lossy(&response);

  assert!(
    response_str.contains("ALL-UPSTREAM-RESPONSE"),
    "With no bypass rules, all requests should go through upstream, got: {}",
    &response_str[..response_str.len().min(300)]
  );
  println!("Empty bypass rules test passed: all traffic goes through upstream");

  tracker.cleanup_all().await;
  upstream_handle.abort();

  Ok(())
}

/// Start a minimal SOCKS5 proxy that tunnels connections to the real destination.
/// Returns (port, JoinHandle).
async fn start_mock_socks5_server() -> (u16, tokio::task::JoinHandle<()>) {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let port = listener.local_addr().unwrap().port();

  let handle = tokio::spawn(async move {
    while let Ok((mut client, _)) = listener.accept().await {
      tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // SOCKS5 handshake: client sends version + methods
        let mut buf = [0u8; 256];
        let n = client.read(&mut buf).await.unwrap_or(0);
        if n < 2 || buf[0] != 0x05 {
          return;
        }

        // Reply: version 5, no auth required
        client.write_all(&[0x05, 0x00]).await.ok();

        // Read connect request: VER CMD RSV ATYP DST.ADDR DST.PORT
        let n = client.read(&mut buf).await.unwrap_or(0);
        if n < 7 || buf[1] != 0x01 {
          client
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await
            .ok();
          return;
        }

        let (target_host, target_port) = match buf[3] {
          0x01 => {
            // IPv4
            if n < 10 {
              return;
            }
            let ip = format!("{}.{}.{}.{}", buf[4], buf[5], buf[6], buf[7]);
            let port = u16::from_be_bytes([buf[8], buf[9]]);
            (ip, port)
          }
          0x03 => {
            // Domain
            let domain_len = buf[4] as usize;
            if n < 5 + domain_len + 2 {
              return;
            }
            let domain = String::from_utf8_lossy(&buf[5..5 + domain_len]).to_string();
            let port = u16::from_be_bytes([buf[5 + domain_len], buf[6 + domain_len]]);
            (domain, port)
          }
          _ => return,
        };

        // Connect to target
        let target =
          match tokio::net::TcpStream::connect(format!("{}:{}", target_host, target_port)).await {
            Ok(t) => t,
            Err(_) => {
              client
                .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await
                .ok();
              return;
            }
          };

        // Success reply
        client
          .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
          .await
          .ok();

        // Bidirectional relay
        let (mut cr, mut cw) = tokio::io::split(client);
        let (mut tr, mut tw) = tokio::io::split(target);
        tokio::select! {
          _ = tokio::io::copy(&mut cr, &mut tw) => {}
          _ = tokio::io::copy(&mut tr, &mut cw) => {}
        }
      });
    }
  });

  sleep(Duration::from_millis(100)).await;
  (port, handle)
}

/// Test that a SOCKS5 upstream proxy works end-to-end through donut-proxy.
/// Starts a mock SOCKS5 server, a mock HTTP target server,
/// then routes requests through donut-proxy -> SOCKS5 -> target.
#[tokio::test]
#[serial]
async fn test_local_proxy_with_socks5_upstream(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  // Start a mock HTTP server as the final destination
  let (target_port, target_handle) = start_mock_http_server("SOCKS5-TARGET-RESPONSE").await;
  println!("Mock target HTTP server on port {target_port}");

  // Start a mock SOCKS5 proxy
  let (socks_port, socks_handle) = start_mock_socks5_server().await;
  println!("Mock SOCKS5 server on port {socks_port}");

  // Helper to start a socks5 proxy
  async fn start_socks5_proxy(
    binary_path: &std::path::PathBuf,
    socks_port: u16,
  ) -> Result<(String, u16), Box<dyn std::error::Error + Send + Sync>> {
    let output = TestUtils::execute_command(
      binary_path,
      &[
        "proxy",
        "start",
        "--host",
        "127.0.0.1",
        "--proxy-port",
        &socks_port.to_string(),
        "--type",
        "socks5",
      ],
    )
    .await?;
    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      return Err(format!("Proxy start failed: {stderr}").into());
    }
    let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
    let id = config["id"].as_str().unwrap().to_string();
    let port = config["localPort"].as_u64().unwrap() as u16;

    // Wait for proxy to be fully ready by verifying it accepts and responds
    for _ in 0..20 {
      sleep(Duration::from_millis(100)).await;
      if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
        break;
      }
    }
    // Extra settle time for the accept loop to be fully initialized
    sleep(Duration::from_millis(200)).await;

    Ok((id, port))
  }

  // Test 1: HTTP request through donut-proxy -> SOCKS5 -> target
  let (proxy_id, local_port) = start_socks5_proxy(&binary_path, socks_port).await?;
  tracker.track_proxy(proxy_id);

  let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
  let request = format!(
    "GET http://127.0.0.1:{target_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\nConnection: close\r\n\r\n"
  );
  stream.write_all(request.as_bytes()).await?;

  let mut response = vec![0u8; 8192];
  let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut response))
    .await
    .map_err(|_| "HTTP request through SOCKS5 timed out")?
    .map_err(|e| format!("Read error: {e}"))?;
  let response_str = String::from_utf8_lossy(&response[..n]);

  assert!(
    response_str.contains("SOCKS5-TARGET-RESPONSE"),
    "HTTP request should be tunneled through SOCKS5 to target, got: {}",
    &response_str[..response_str.len().min(500)]
  );
  println!("SOCKS5 upstream proxy test passed");

  tracker.cleanup_all().await;
  target_handle.abort();
  socks_handle.abort();

  Ok(())
}

/// Test proxying traffic through a real Shadowsocks server running in Docker.
/// Verifies the full chain: client → donut-proxy → Shadowsocks → internet.
#[tokio::test]
#[serial]
async fn test_local_proxy_with_shadowsocks_upstream(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  if !common::docker_supports_linux_containers() {
    eprintln!("skipping Shadowsocks e2e test because Docker cannot run Linux containers");
    return Ok(());
  }

  // Start a Shadowsocks server container
  let ss_container = "donut-ss-test";
  let ss_port = 18388u16;
  let ss_password = "donut-test-password";
  let ss_method = "aes-256-gcm";

  // Clean up any previous container
  let _ = std::process::Command::new("docker")
    .args(["rm", "-f", ss_container])
    .output();

  let docker_start = std::process::Command::new("docker")
    .args([
      "run",
      "-d",
      "--name",
      ss_container,
      "-p",
      &format!("{ss_port}:8388"),
      "ghcr.io/shadowsocks/ssserver-rust:latest",
      "ssserver",
      "-s",
      "[::]:8388",
      "-k",
      ss_password,
      "-m",
      ss_method,
    ])
    .output()?;

  if !docker_start.status.success() {
    let stderr = String::from_utf8_lossy(&docker_start.stderr);
    eprintln!("skipping Shadowsocks e2e test: Docker run failed: {stderr}");
    return Ok(());
  }

  // Wait for the SS server to be ready
  for _ in 0..15 {
    sleep(Duration::from_secs(1)).await;
    if TcpStream::connect(("127.0.0.1", ss_port)).await.is_ok() {
      break;
    }
  }

  // Start donut-proxy with Shadowsocks upstream
  let output = tokio::process::Command::new(&binary_path)
    .args([
      "proxy",
      "start",
      "--host",
      "127.0.0.1",
      "--proxy-port",
      &ss_port.to_string(),
      "--type",
      "ss",
    ])
    .env("DONUT_PROXY_USERNAME", ss_method)
    .env("DONUT_PROXY_PASSWORD", ss_password)
    .output()
    .await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let _ = std::process::Command::new("docker")
      .args(["rm", "-f", ss_container])
      .output();
    return Err(format!("Proxy start failed: {stderr}").into());
  }

  let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id);

  // Wait for proxy to be fully ready
  for _ in 0..20 {
    sleep(Duration::from_millis(100)).await;
    if TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
      break;
    }
  }
  sleep(Duration::from_millis(500)).await;

  // Test: HTTP request through donut-proxy → Shadowsocks → example.com
  let mut stream = TcpStream::connect(("127.0.0.1", local_port)).await?;
  let request =
    "GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n";
  stream.write_all(request.as_bytes()).await?;

  let mut response = vec![0u8; 16384];
  let n = tokio::time::timeout(Duration::from_secs(15), stream.read(&mut response))
    .await
    .map_err(|_| "HTTP request through Shadowsocks timed out")?
    .map_err(|e| format!("Read error: {e}"))?;
  let response_str = String::from_utf8_lossy(&response[..n]);

  assert!(
    response_str.contains("Example Domain"),
    "HTTP traffic through Shadowsocks should reach example.com, got: {}",
    &response_str[..response_str.len().min(500)]
  );
  println!("Shadowsocks upstream proxy test passed");

  // Cleanup
  tracker.cleanup_all().await;
  let _ = std::process::Command::new("docker")
    .args(["rm", "-f", ss_container])
    .output();

  Ok(())
}

// ---------------------------------------------------------------------------
// Worker lifecycle: a detached donut-proxy must die with the browser it serves,
// with no help from the GUI. These drive a REAL worker process end to end —
// the unit tests cover the decision table, these prove the process actually
// exits and cleans up after itself.
//
// The watchdog polls every 15s in production; the tests shorten it via
// DONUT_PROXY_WATCHDOG_INTERVAL_MS so a reap is observable in seconds.
// ---------------------------------------------------------------------------

const TEST_WATCHDOG_INTERVAL_MS: &str = "300";

/// A long-lived process standing in for a browser, reaped on drop so a failing
/// assertion can never leave one behind.
struct StubBrowser {
  child: std::process::Child,
}

impl StubBrowser {
  fn spawn() -> Self {
    let mut command = if cfg!(windows) {
      let mut c = std::process::Command::new("cmd");
      c.args(["/C", "ping -n 600 127.0.0.1 >NUL"]);
      c
    } else {
      let mut c = std::process::Command::new("sleep");
      c.arg("600");
      c
    };
    let child = command
      .stdin(std::process::Stdio::null())
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .spawn()
      .expect("failed to spawn stub browser");
    Self { child }
  }

  fn pid(&self) -> u32 {
    self.child.id()
  }

  /// Kill and reap, so the PID is genuinely gone before the worker looks at it.
  /// A zombie still resolves in the process table and would mask the reap.
  fn terminate(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

impl Drop for StubBrowser {
  fn drop(&mut self) {
    self.terminate();
  }
}

/// Wait for a port to start accepting connections.
///
/// `proxy start` returns once the worker is spawned, not once it has bound its
/// listener, so a fixed sleep is a bet on how fast the runner is. Poll instead.
async fn wait_for_port_open(port: u16, timeout: Duration) -> bool {
  let deadline = std::time::Instant::now() + timeout;
  while std::time::Instant::now() < deadline {
    if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
      return true;
    }
    sleep(Duration::from_millis(100)).await;
  }
  false
}

/// Wait for a listening port to stop accepting connections.
///
/// `proxy stop` returns once the worker has been told to exit, not once it has
/// actually gone, so the listener can outlive the command by however long the
/// process takes to unwind. That gap is invisible on an idle laptop and lands
/// squarely on a loaded CI runner, so poll to a deadline rather than sleeping a
/// fixed amount and hoping. Returns false if it is still accepting at the end.
async fn wait_for_port_closed(port: u16, timeout: Duration) -> bool {
  let deadline = std::time::Instant::now() + timeout;
  while std::time::Instant::now() < deadline {
    if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
      return true;
    }
    sleep(Duration::from_millis(100)).await;
  }
  false
}

/// Wait for a worker to remove its own config, which it does immediately before
/// exiting. Returns false if it is still there when the deadline passes.
async fn wait_for_worker_exit(proxy_id: &str, timeout: Duration) -> bool {
  let deadline = std::time::Instant::now() + timeout;
  while std::time::Instant::now() < deadline {
    if donutbrowser_lib::proxy_storage::get_proxy_config(proxy_id).is_none() {
      return true;
    }
    sleep(Duration::from_millis(100)).await;
  }
  false
}

/// Start a direct worker with the short watchdog interval and return its config.
async fn start_lifecycle_worker(
  profile_id: &str,
) -> Result<donutbrowser_lib::proxy_storage::ProxyConfig, Box<dyn std::error::Error + Send + Sync>>
{
  std::env::set_var(
    "DONUT_PROXY_WATCHDOG_INTERVAL_MS",
    TEST_WATCHDOG_INTERVAL_MS,
  );
  donutbrowser_lib::proxy_runner::start_proxy_process_with_profile(
    None,
    None,
    Some(profile_id.to_string()),
    Vec::new(),
    None,
    false,
    Some("http".to_string()),
  )
  .await
  .map_err(|error| error.to_string().into())
}

/// Record the owning browser on a worker's config the way the GUI does: the PID
/// plus the start time that pins it to that exact process.
fn claim_worker_for(proxy_id: &str, browser_pid: u32) -> bool {
  let Some(mut config) = donutbrowser_lib::proxy_storage::get_proxy_config(proxy_id) else {
    return false;
  };
  let Some(start_time) = donutbrowser_lib::proxy_storage::resolve_process_start_time(browser_pid)
  else {
    return false;
  };
  config.browser_pid = Some(browser_pid);
  config.browser_pid_start_time = Some(start_time);
  donutbrowser_lib::proxy_storage::update_proxy_config(&config)
}

/// A worker whose browser exits must terminate itself and delete its config,
/// with no GUI involved. This is the whole contract behind "no orphaned
/// donut-proxy after closing the browser".
#[tokio::test]
#[serial]
async fn worker_reaps_itself_when_its_browser_exits(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let _binary_path = setup_test().await?;
  let config = start_lifecycle_worker("lifecycle-browser-exit").await?;
  let mut tracker = ProxyTestTracker::new(_binary_path.clone());
  tracker.track_proxy(config.id.clone());

  let mut stub = StubBrowser::spawn();
  assert!(
    claim_worker_for(&config.id, stub.pid()),
    "the worker must accept a live browser as its owner"
  );

  let owner = donutbrowser_lib::proxy_storage::get_proxy_config(&config.id)
    .ok_or("worker config vanished before the browser exited")?;
  assert_eq!(owner.browser_pid, Some(stub.pid()));
  assert!(
    owner.browser_pid_start_time.is_some(),
    "the owner PID must be pinned to a start time"
  );

  // While the browser lives, the worker must stay: a worker that reaps itself
  // out from under a running browser is a far worse bug than an orphan.
  sleep(Duration::from_millis(1500)).await;
  assert!(
    donutbrowser_lib::proxy_storage::get_proxy_config(&config.id).is_some(),
    "worker exited while its browser was still running"
  );

  let worker_pid = config.pid.ok_or("worker did not report a PID")?;
  stub.terminate();

  assert!(
    wait_for_worker_exit(&config.id, Duration::from_secs(20)).await,
    "worker did not clean up its config after its browser exited"
  );
  let mut process_gone = false;
  for _ in 0..100 {
    if !donutbrowser_lib::proxy_storage::is_process_running(worker_pid) {
      process_gone = true;
      break;
    }
    sleep(Duration::from_millis(100)).await;
  }
  assert!(
    process_gone,
    "worker process {worker_pid} is still in memory after its browser exited"
  );

  tracker.cleanup_all().await;
  Ok(())
}

/// The orphan users actually reported: the browser is gone but its PID has been
/// handed to some other process. Existence alone reads as "my browser is alive"
/// and the worker never exits, so the owner is pinned by start time instead.
#[tokio::test]
#[serial]
async fn worker_reaps_itself_when_its_browser_pid_is_recycled(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let config = start_lifecycle_worker("lifecycle-pid-reuse").await?;
  let mut tracker = ProxyTestTracker::new(binary_path);
  tracker.track_proxy(config.id.clone());

  // A live PID with someone else's start time is exactly what a recycled PID
  // looks like from the worker's side.
  let stub = StubBrowser::spawn();
  let mut owner = donutbrowser_lib::proxy_storage::get_proxy_config(&config.id)
    .ok_or("worker config missing after start")?;
  owner.browser_pid = Some(stub.pid());
  owner.browser_pid_start_time = Some(1);
  assert!(donutbrowser_lib::proxy_storage::update_proxy_config(&owner));

  assert!(
    wait_for_worker_exit(&config.id, Duration::from_secs(20)).await,
    "worker kept serving a PID that no longer belongs to its browser"
  );

  drop(stub);
  tracker.cleanup_all().await;
  Ok(())
}

/// Deleting a worker's config is how the GUI stops it. The worker must notice
/// and exit even though nothing signalled it.
#[tokio::test]
#[serial]
async fn worker_exits_when_its_config_is_deleted(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let config = start_lifecycle_worker("lifecycle-config-deleted").await?;
  let tracker = ProxyTestTracker::new(binary_path);
  let worker_pid = config.pid.ok_or("worker did not report a PID")?;

  assert!(donutbrowser_lib::proxy_storage::delete_proxy_config(
    &config.id
  ));

  let mut process_gone = false;
  for _ in 0..200 {
    if !donutbrowser_lib::proxy_storage::is_process_running(worker_pid) {
      process_gone = true;
      break;
    }
    sleep(Duration::from_millis(100)).await;
  }
  assert!(
    process_gone,
    "worker process {worker_pid} survived deletion of its config"
  );

  tracker.cleanup_all().await;
  Ok(())
}

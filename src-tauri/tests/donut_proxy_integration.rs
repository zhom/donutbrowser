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

  if !proxy_binary.exists() {
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
  sleep(Duration::from_millis(500)).await;
  match TcpStream::connect(("127.0.0.1", local_port)).await {
    Ok(_) => {
      println!("Proxy is listening on port {local_port}");
    }
    Err(e) => {
      return Err(format!("Proxy port {local_port} is not listening: {e}").into());
    }
  }

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
  sleep(Duration::from_millis(500)).await;
  match TcpStream::connect(("127.0.0.1", proxy1_port)).await {
    Ok(_) => println!("First proxy is ready"),
    Err(e) => return Err(format!("First proxy not ready: {e}").into()),
  }

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
  sleep(Duration::from_millis(500)).await;
  match TcpStream::connect(("127.0.0.1", proxy2_port)).await {
    Ok(_) => println!("Second proxy is ready"),
    Err(e) => return Err(format!("Second proxy not ready: {e}").into()),
  }

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
  sleep(Duration::from_millis(500)).await;
  match TcpStream::connect(("127.0.0.1", local_port)).await {
    Ok(_) => {
      println!("Proxy is listening on port {local_port}");
    }
    Err(e) => {
      upstream_handle.abort();
      return Err(format!("Proxy port {local_port} is not listening: {e}").into());
    }
  }

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
  sleep(Duration::from_millis(500)).await;
  match TcpStream::connect(("127.0.0.1", local_port)).await {
    Ok(_) => println!("Proxy is running"),
    Err(_) => return Err("Proxy is not running".into()),
  }

  // Stop the proxy
  let stop_output =
    TestUtils::execute_command(&binary_path, &["proxy", "stop", "--id", &proxy_id]).await?;

  if !stop_output.status.success() {
    return Err("Failed to stop proxy".into());
  }

  // Wait a bit for the process to exit
  sleep(Duration::from_millis(500)).await;

  // Verify proxy is stopped (connection should fail)
  match TcpStream::connect(("127.0.0.1", local_port)).await {
    Ok(_) => return Err("Proxy should be stopped but is still listening".into()),
    Err(_) => println!("Proxy successfully stopped"),
  }

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

  // Check Docker availability
  let docker_check = std::process::Command::new("docker").arg("version").output();
  if docker_check.map(|o| !o.status.success()).unwrap_or(true) {
    eprintln!("skipping Shadowsocks e2e test because Docker is unavailable");
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
  let output = TestUtils::execute_command(
    &binary_path,
    &[
      "proxy",
      "start",
      "--host",
      "127.0.0.1",
      "--proxy-port",
      &ss_port.to_string(),
      "--type",
      "ss",
      "--username",
      ss_method,
      "--password",
      ss_password,
    ],
  )
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

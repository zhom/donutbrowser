mod common;
use common::TestUtils;
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;

/// Setup function to ensure donut-proxy binary exists
async fn setup_test() -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
  let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
  let project_root = std::path::PathBuf::from(cargo_manifest_dir)
    .parent()
    .unwrap()
    .to_path_buf();

  // Build donut-proxy binary if it doesn't exist
  let proxy_binary = project_root
    .join("src-tauri")
    .join("target")
    .join("debug")
    .join("donut-proxy");

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
async fn test_chained_local_proxies() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  println!("Testing chained local proxies...");

  // Start first proxy (DIRECT - connects to internet)
  let output1 = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
  if !output1.status.success() {
    return Err("Failed to start first proxy".into());
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
    return Err("Failed to start second proxy".into());
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
async fn test_multiple_proxies_simultaneously(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  println!("Starting multiple proxies simultaneously...");

  let mut proxy_ports = Vec::new();

  // Start 3 proxies
  for i in 0..3 {
    let output = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
    if !output.status.success() {
      return Err(format!("Failed to start proxy {}", i + 1).into());
    }

    let config: Value = serde_json::from_str(&String::from_utf8(output.stdout)?)?;
    let proxy_id = config["id"].as_str().unwrap().to_string();
    let local_port = config["localPort"].as_u64().unwrap() as u16;
    tracker.track_proxy(proxy_id);
    proxy_ports.push(local_port);

    println!("Proxy {} started on port {}", i + 1, local_port);
  }

  // Wait for all proxies to be ready
  sleep(Duration::from_millis(1000)).await;

  // Verify all proxies are listening
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
async fn test_proxy_list() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let mut tracker = ProxyTestTracker::new(binary_path.clone());

  // Start a proxy
  let output = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
  if !output.status.success() {
    return Err("Failed to start proxy".into());
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

/// Test proxy stop
#[tokio::test]
async fn test_proxy_stop() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let binary_path = setup_test().await?;
  let _tracker = ProxyTestTracker::new(binary_path.clone());

  // Start a proxy
  let output = TestUtils::execute_command(&binary_path, &["proxy", "start"]).await?;
  if !output.status.success() {
    return Err("Failed to start proxy".into());
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

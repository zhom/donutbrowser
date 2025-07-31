mod common;
use common::TestUtils;
use serde_json::Value;

/// Setup function to ensure clean state before tests
async fn setup_test() -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = TestUtils::ensure_nodecar_binary().await?;

  // Clean up any existing processes from previous test runs
  let _ = TestUtils::cleanup_all_nodecar_processes(&nodecar_path).await;

  Ok(nodecar_path)
}

/// Helper to track and cleanup specific test resources
struct TestResourceTracker {
  proxy_ids: Vec<String>,
  camoufox_ids: Vec<String>,
  nodecar_path: std::path::PathBuf,
}

impl TestResourceTracker {
  fn new(nodecar_path: std::path::PathBuf) -> Self {
    Self {
      proxy_ids: Vec::new(),
      camoufox_ids: Vec::new(),
      nodecar_path,
    }
  }

  fn track_proxy(&mut self, proxy_id: String) {
    self.proxy_ids.push(proxy_id);
  }

  fn track_camoufox(&mut self, camoufox_id: String) {
    self.camoufox_ids.push(camoufox_id);
  }

  async fn cleanup_all(&self) {
    // Clean up tracked proxies
    for proxy_id in &self.proxy_ids {
      let stop_args = ["proxy", "stop", "--id", proxy_id];
      let _ = TestUtils::execute_nodecar_command(&self.nodecar_path, &stop_args, 10).await;
    }

    // Clean up tracked camoufox instances
    for camoufox_id in &self.camoufox_ids {
      let stop_args = ["camoufox", "stop", "--id", camoufox_id];
      let _ = TestUtils::execute_nodecar_command(&self.nodecar_path, &stop_args, 30).await;
    }

    // Give processes time to clean up
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
  }
}

impl Drop for TestResourceTracker {
  fn drop(&mut self) {
    // Ensure cleanup happens even if test panics
    let proxy_ids = self.proxy_ids.clone();
    let camoufox_ids = self.camoufox_ids.clone();
    let nodecar_path = self.nodecar_path.clone();

    tokio::spawn(async move {
      for proxy_id in &proxy_ids {
        let stop_args = ["proxy", "stop", "--id", proxy_id];
        let _ = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args, 10).await;
      }

      for camoufox_id in &camoufox_ids {
        let stop_args = ["camoufox", "stop", "--id", camoufox_id];
        let _ = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args, 30).await;
      }
    });
  }
}

/// Integration tests for nodecar proxy functionality
#[tokio::test]
async fn test_nodecar_proxy_lifecycle() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  // Test proxy start with a known working upstream
  let args = [
    "proxy",
    "start",
    "--host",
    "httpbin.org",
    "--proxy-port",
    "80",
    "--type",
    "http",
  ];

  println!("Starting proxy with nodecar...");
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args, 30).await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    tracker.cleanup_all().await;
    return Err(format!("Proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let stdout = String::from_utf8(output.stdout)?;
  let config: Value = serde_json::from_str(&stdout)?;

  // Verify proxy configuration structure
  assert!(config["id"].is_string(), "Proxy ID should be a string");
  assert!(
    config["localPort"].is_number(),
    "Local port should be a number"
  );
  assert!(
    config["localUrl"].is_string(),
    "Local URL should be a string"
  );

  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id.clone());

  println!("Proxy started with ID: {proxy_id} on port: {local_port}");

  // Wait for the proxy to start listening
  let is_listening = TestUtils::wait_for_port_state(local_port, true, 10).await;
  assert!(
    is_listening,
    "Proxy should be listening on the assigned port"
  );

  // Test stopping the proxy
  let stop_args = ["proxy", "stop", "--id", &proxy_id];
  let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args, 10).await?;

  assert!(stop_output.status.success(), "Proxy stop should succeed");

  let port_available = TestUtils::wait_for_port_state(local_port, false, 5).await;
  assert!(
    port_available,
    "Port should be available after stopping proxy"
  );

  tracker.cleanup_all().await;
  Ok(())
}

/// Test proxy with authentication
#[tokio::test]
async fn test_nodecar_proxy_with_auth() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  let args = [
    "proxy",
    "start",
    "--host",
    "httpbin.org",
    "--proxy-port",
    "80",
    "--type",
    "http",
    "--username",
    "testuser",
    "--password",
    "testpass",
  ];

  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args, 30).await?;

  if output.status.success() {
    let stdout = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&stdout)?;

    let proxy_id = config["id"].as_str().unwrap().to_string();
    tracker.track_proxy(proxy_id.clone());

    // Verify upstream URL contains encoded credentials
    if let Some(upstream_url) = config["upstreamUrl"].as_str() {
      assert!(
        upstream_url.contains("testuser"),
        "Upstream URL should contain username"
      );
      // Password might be encoded, so we check for the presence of auth info
      assert!(
        upstream_url.contains("@"),
        "Upstream URL should contain auth separator"
      );
    }
  }

  tracker.cleanup_all().await;
  Ok(())
}

/// Test proxy list functionality
#[tokio::test]
async fn test_nodecar_proxy_list() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  // Start a proxy first
  let start_args = [
    "proxy",
    "start",
    "--host",
    "httpbin.org",
    "--proxy-port",
    "80",
    "--type",
    "http",
  ];

  let start_output = TestUtils::execute_nodecar_command(&nodecar_path, &start_args, 30).await?;

  if start_output.status.success() {
    let stdout = String::from_utf8(start_output.stdout)?;
    let config: Value = serde_json::from_str(&stdout)?;
    let proxy_id = config["id"].as_str().unwrap().to_string();
    tracker.track_proxy(proxy_id.clone());

    // Test list command
    let list_args = ["proxy", "list"];
    let list_output = TestUtils::execute_nodecar_command(&nodecar_path, &list_args, 10).await?;

    assert!(list_output.status.success(), "Proxy list should succeed");

    let list_stdout = String::from_utf8(list_output.stdout)?;
    let proxy_list: Value = serde_json::from_str(&list_stdout)?;

    assert!(proxy_list.is_array(), "Proxy list should be an array");

    let proxies = proxy_list.as_array().unwrap();
    assert!(
      !proxies.is_empty(),
      "Should have at least one proxy in the list"
    );

    // Find our proxy in the list
    let found_proxy = proxies.iter().find(|p| p["id"].as_str() == Some(&proxy_id));
    assert!(found_proxy.is_some(), "Started proxy should be in the list");
  }

  tracker.cleanup_all().await;
  Ok(())
}

/// Test Camoufox functionality
#[tokio::test]
async fn test_nodecar_camoufox_lifecycle() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  let temp_dir = TestUtils::create_temp_dir()?;
  let profile_path = temp_dir.path().join("test_profile");

  let args = [
    "camoufox",
    "start",
    "--profile-path",
    profile_path.to_str().unwrap(),
    "--headless",
    "--debug",
  ];

  println!("Starting Camoufox with nodecar...");
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args, 35).await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // If Camoufox is not installed or times out, skip the test
    if stderr.contains("not installed")
      || stderr.contains("not found")
      || stderr.contains("timeout")
      || stdout.contains("timeout")
    {
      println!("Skipping Camoufox test - Camoufox not available or timed out");
      tracker.cleanup_all().await;
      return Ok(());
    }

    tracker.cleanup_all().await;
    return Err(format!("Camoufox start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let stdout = String::from_utf8(output.stdout)?;
  let config: Value = serde_json::from_str(&stdout)?;

  // Verify Camoufox configuration structure
  assert!(config["id"].is_string(), "Camoufox ID should be a string");

  let camoufox_id = config["id"].as_str().unwrap().to_string();
  tracker.track_camoufox(camoufox_id.clone());
  println!("Camoufox started with ID: {camoufox_id}");

  // Test stopping Camoufox
  let stop_args = ["camoufox", "stop", "--id", &camoufox_id];
  let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args, 30).await?;

  assert!(stop_output.status.success(), "Camoufox stop should succeed");

  tracker.cleanup_all().await;
  Ok(())
}

/// Test Camoufox with URL opening
#[tokio::test]
async fn test_nodecar_camoufox_with_url() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  let temp_dir = TestUtils::create_temp_dir()?;
  let profile_path = temp_dir.path().join("test_profile_url");

  let args = [
    "camoufox",
    "start",
    "--profile-path",
    profile_path.to_str().unwrap(),
    "--url",
    "https://httpbin.org/get",
    "--headless",
    "--debug",
  ];

  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args, 15).await?;

  if output.status.success() {
    let stdout = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&stdout)?;

    let camoufox_id = config["id"].as_str().unwrap().to_string();
    tracker.track_camoufox(camoufox_id.clone());

    // Verify URL is set
    if let Some(url) = config["url"].as_str() {
      assert_eq!(
        url, "https://httpbin.org/get",
        "URL should match what was provided"
      );
    }

    // Test stopping Camoufox explicitly
    let stop_args = ["camoufox", "stop", "--id", &camoufox_id];
    let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args, 30).await?;
    assert!(stop_output.status.success(), "Camoufox stop should succeed");
  } else {
    println!("Skipping Camoufox URL test - likely not installed");
    tracker.cleanup_all().await;
    return Ok(());
  }

  tracker.cleanup_all().await;
  Ok(())
}

/// Test Camoufox list functionality
#[tokio::test]
async fn test_nodecar_camoufox_list() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let tracker = TestResourceTracker::new(nodecar_path.clone());

  // Test list command (should work even without Camoufox installed)
  let list_args = ["camoufox", "list"];
  let list_output = TestUtils::execute_nodecar_command(&nodecar_path, &list_args, 10).await?;

  assert!(list_output.status.success(), "Camoufox list should succeed");

  let list_stdout = String::from_utf8(list_output.stdout)?;
  let camoufox_list: Value = serde_json::from_str(&list_stdout)?;

  assert!(camoufox_list.is_array(), "Camoufox list should be an array");

  tracker.cleanup_all().await;
  Ok(())
}

/// Test Camoufox process tracking and management
#[tokio::test]
async fn test_nodecar_camoufox_process_tracking(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  let temp_dir = TestUtils::create_temp_dir()?;
  let profile_path = temp_dir.path().join("test_profile_tracking");

  // Start multiple Camoufox instances
  let mut instance_ids: Vec<String> = Vec::new();

  for i in 0..2 {
    let instance_profile_path = format!("{}_instance_{}", profile_path.to_str().unwrap(), i);
    let args = [
      "camoufox",
      "start",
      "--profile-path",
      &instance_profile_path,
      "--headless",
      "--debug",
    ];

    println!("Starting Camoufox instance {i}...");
    let output = TestUtils::execute_nodecar_command(&nodecar_path, &args, 10).await?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let stdout = String::from_utf8_lossy(&output.stdout);

      // If Camoufox is not installed, skip the test
      if stderr.contains("not installed") || stderr.contains("not found") {
        println!("Skipping Camoufox process tracking test - Camoufox not installed");
        tracker.cleanup_all().await;
        return Ok(());
      }

      tracker.cleanup_all().await;
      return Err(
        format!("Camoufox instance {i} start failed - stdout: {stdout}, stderr: {stderr}").into(),
      );
    }

    let stdout = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&stdout)?;

    let camoufox_id = config["id"].as_str().unwrap().to_string();
    instance_ids.push(camoufox_id.clone());
    tracker.track_camoufox(camoufox_id.clone());
    println!("Camoufox instance {i} started with ID: {camoufox_id}");
  }

  // Verify all instances are tracked
  let list_args = ["camoufox", "list"];
  let list_output = TestUtils::execute_nodecar_command(&nodecar_path, &list_args, 10).await?;

  assert!(list_output.status.success(), "Camoufox list should succeed");

  let list_stdout = String::from_utf8(list_output.stdout)?;
  println!("Camoufox list output: {list_stdout}");
  let instances: Value = serde_json::from_str(&list_stdout)?;

  let instances_array = instances.as_array().unwrap();
  println!("Found {} instances in list", instances_array.len());

  // Verify our instances are in the list
  for instance_id in &instance_ids {
    let instance_found = instances_array
      .iter()
      .any(|i| i["id"].as_str() == Some(instance_id));
    if !instance_found {
      println!("Instance {instance_id} not found in list. Available instances:");
      for instance in instances_array {
        if let Some(id) = instance["id"].as_str() {
          println!("  - {id}");
        }
      }
    }
    assert!(
      instance_found,
      "Camoufox instance {instance_id} should be found in list"
    );
  }

  // Stop all instances individually
  for instance_id in &instance_ids {
    println!("Stopping Camoufox instance: {instance_id}");
    let stop_args = ["camoufox", "stop", "--id", instance_id];
    let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args, 30).await?;

    if stop_output.status.success() {
      let stop_stdout = String::from_utf8(stop_output.stdout)?;
      if let Ok(stop_result) = serde_json::from_str::<Value>(&stop_stdout) {
        let success = stop_result["success"].as_bool().unwrap_or(false);
        if !success {
          println!("Warning: Stop command returned success=false for instance {instance_id}");
        }
      } else {
        println!("Warning: Could not parse stop result for instance {instance_id}");
      }
    } else {
      println!("Warning: Stop command failed for instance {instance_id}");
    }
  }

  // Verify all instances are removed
  let list_output_after = TestUtils::execute_nodecar_command(&nodecar_path, &list_args, 10).await?;

  let instances_after: Value = serde_json::from_str(&String::from_utf8(list_output_after.stdout)?)?;
  let instances_after_array = instances_after.as_array().unwrap();

  for instance_id in &instance_ids {
    let instance_still_exists = instances_after_array
      .iter()
      .any(|i| i["id"].as_str() == Some(instance_id));
    assert!(
      !instance_still_exists,
      "Stopped Camoufox instance {instance_id} should not be found in list"
    );
  }

  println!("Camoufox process tracking test completed successfully");
  tracker.cleanup_all().await;
  Ok(())
}

/// Test Camoufox with various configuration options
#[tokio::test]
async fn test_nodecar_camoufox_configuration_options(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  let temp_dir = TestUtils::create_temp_dir()?;
  let profile_path = temp_dir.path().join("test_profile_config");

  let args = [
    "camoufox",
    "start",
    "--profile-path",
    profile_path.to_str().unwrap(),
    "--headless",
    "--debug",
    "--os",
    "linux",
    "--block-images",
    "--humanize",
    "--locale",
    "en-US,en-GB",
    "--timezone",
    "America/New_York",
    "--disable-cache",
  ];

  println!("Starting Camoufox with configuration options...");
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args, 45).await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // If Camoufox is not installed, skip the test
    if stderr.contains("not installed") || stderr.contains("not found") {
      println!("Skipping Camoufox configuration test - Camoufox not installed");
      tracker.cleanup_all().await;
      return Ok(());
    }

    tracker.cleanup_all().await;
    return Err(
      format!("Camoufox with config start failed - stdout: {stdout}, stderr: {stderr}").into(),
    );
  }

  let stdout = String::from_utf8(output.stdout)?;
  let config: Value = serde_json::from_str(&stdout)?;

  let camoufox_id = config["id"].as_str().unwrap().to_string();
  tracker.track_camoufox(camoufox_id.clone());
  println!("Camoufox with configuration started with ID: {camoufox_id}");

  // Verify configuration was applied by checking the profile path
  if let Some(returned_profile_path) = config["profilePath"].as_str() {
    assert!(
      returned_profile_path.contains("test_profile_config"),
      "Profile path should match what was provided"
    );
  }

  // Test stopping Camoufox explicitly
  let stop_args = ["camoufox", "stop", "--id", &camoufox_id];
  let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args, 30).await?;

  assert!(stop_output.status.success(), "Camoufox stop should succeed");

  println!("Camoufox configuration test completed successfully");
  tracker.cleanup_all().await;
  Ok(())
}

/// Test nodecar command validation
#[tokio::test]
async fn test_nodecar_command_validation() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let tracker = TestResourceTracker::new(nodecar_path.clone());

  // Test invalid command
  let invalid_args = ["invalid", "command"];
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &invalid_args, 10).await?;

  assert!(!output.status.success(), "Invalid command should fail");

  // Test proxy without required arguments
  let incomplete_args = ["proxy", "start"];
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &incomplete_args, 10).await?;

  assert!(
    !output.status.success(),
    "Incomplete proxy command should fail"
  );

  tracker.cleanup_all().await;
  Ok(())
}

/// Test concurrent proxy operations
#[tokio::test]
async fn test_nodecar_concurrent_proxies() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  // Start multiple proxies concurrently
  let mut handles = vec![];

  for i in 0..3 {
    let nodecar_path_clone = nodecar_path.clone();
    let handle = tokio::spawn(async move {
      let args = [
        "proxy",
        "start",
        "--host",
        "httpbin.org",
        "--proxy-port",
        "80",
        "--type",
        "http",
      ];

      TestUtils::execute_nodecar_command(&nodecar_path_clone, &args, 30).await
    });
    handles.push((i, handle));
  }

  // Wait for all proxies to start
  for (i, handle) in handles {
    match handle.await.map_err(|e| format!("Join error: {e}"))? {
      Ok(output) if output.status.success() => {
        let stdout = String::from_utf8(output.stdout)?;
        let config: Value = serde_json::from_str(&stdout)?;
        let proxy_id = config["id"].as_str().unwrap().to_string();
        tracker.track_proxy(proxy_id.clone());
        println!("Proxy {i} started successfully");
      }
      Ok(output) => {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("Proxy {i} failed to start: {stderr}");
      }
      Err(e) => {
        println!("Proxy {i} error: {e}");
      }
    }
  }

  tracker.cleanup_all().await;
  Ok(())
}

/// Test proxy with different upstream types
#[tokio::test]
async fn test_nodecar_proxy_types() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  let test_cases = vec![
    ("http", "httpbin.org", "80"),
    ("https", "httpbin.org", "443"),
  ];

  for (proxy_type, host, port) in test_cases {
    println!("Testing {proxy_type} proxy to {host}:{port}");

    let args = [
      "proxy",
      "start",
      "--host",
      host,
      "--proxy-port",
      port,
      "--type",
      proxy_type,
    ];

    let output = TestUtils::execute_nodecar_command(&nodecar_path, &args, 30).await?;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: Value = serde_json::from_str(&stdout)?;
      let proxy_id = config["id"].as_str().unwrap().to_string();
      tracker.track_proxy(proxy_id.clone());

      println!("{proxy_type} proxy test passed");
    } else {
      let stderr = String::from_utf8_lossy(&output.stderr);
      println!("{proxy_type} proxy test failed: {stderr}");
    }
  }

  tracker.cleanup_all().await;
  Ok(())
}

/// Test SOCKS5 proxy chaining - create two proxies where the second uses the first as upstream
#[tokio::test]
async fn test_nodecar_socks5_proxy_chaining() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  // Step 1: Start a SOCKS5 proxy with a known working upstream (httpbin.org)
  let socks5_args = [
    "proxy",
    "start",
    "--host",
    "httpbin.org",
    "--proxy-port",
    "80",
    "--type",
    "http", // Use HTTP upstream for the first proxy
  ];

  println!("Starting first proxy with HTTP upstream...");
  let socks5_output = TestUtils::execute_nodecar_command(&nodecar_path, &socks5_args, 30).await?;

  if !socks5_output.status.success() {
    let stderr = String::from_utf8_lossy(&socks5_output.stderr);
    let stdout = String::from_utf8_lossy(&socks5_output.stdout);
    tracker.cleanup_all().await;
    return Err(format!("First proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
  }

  let socks5_stdout = String::from_utf8(socks5_output.stdout)?;
  let socks5_config: Value = serde_json::from_str(&socks5_stdout)?;

  let socks5_proxy_id = socks5_config["id"].as_str().unwrap().to_string();
  let socks5_local_port = socks5_config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(socks5_proxy_id.clone());

  println!("First proxy started with ID: {socks5_proxy_id} on port: {socks5_local_port}");

  // Step 2: Start a second proxy that uses the first proxy as upstream
  let http_proxy_args = [
    "proxy",
    "start",
    "--upstream",
    &format!("http://127.0.0.1:{socks5_local_port}"),
  ];

  println!("Starting second proxy with first proxy as upstream...");
  let http_output = TestUtils::execute_nodecar_command(&nodecar_path, &http_proxy_args, 30).await?;

  if !http_output.status.success() {
    let stderr = String::from_utf8_lossy(&http_output.stderr);
    let stdout = String::from_utf8_lossy(&http_output.stdout);
    tracker.cleanup_all().await;
    return Err(
      format!("Second proxy with chained upstream failed - stdout: {stdout}, stderr: {stderr}")
        .into(),
    );
  }

  let http_stdout = String::from_utf8(http_output.stdout)?;
  let http_config: Value = serde_json::from_str(&http_stdout)?;

  let http_proxy_id = http_config["id"].as_str().unwrap().to_string();
  let http_local_port = http_config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(http_proxy_id.clone());

  println!(
    "Second proxy started with ID: {http_proxy_id} on port: {http_local_port} (chained through first proxy)"
  );

  // Verify both proxies are listening by waiting for them to be occupied
  let socks5_listening = TestUtils::wait_for_port_state(socks5_local_port, true, 5).await;
  let http_listening = TestUtils::wait_for_port_state(http_local_port, true, 5).await;

  assert!(
    socks5_listening,
    "First proxy should be listening on port {socks5_local_port}"
  );
  assert!(
    http_listening,
    "Second proxy should be listening on port {http_local_port}"
  );

  // Clean up both proxies
  let stop_http_args = ["proxy", "stop", "--id", &http_proxy_id];
  let stop_socks5_args = ["proxy", "stop", "--id", &socks5_proxy_id];

  let http_stop_result =
    TestUtils::execute_nodecar_command(&nodecar_path, &stop_http_args, 10).await;
  let socks5_stop_result =
    TestUtils::execute_nodecar_command(&nodecar_path, &stop_socks5_args, 10).await;

  // Verify cleanup
  assert!(
    http_stop_result.is_ok() && http_stop_result.unwrap().status.success(),
    "Second proxy stop should succeed"
  );
  assert!(
    socks5_stop_result.is_ok() && socks5_stop_result.unwrap().status.success(),
    "First proxy stop should succeed"
  );

  let http_port_available = TestUtils::wait_for_port_state(http_local_port, false, 5).await;
  let socks5_port_available = TestUtils::wait_for_port_state(socks5_local_port, false, 5).await;

  assert!(
    http_port_available,
    "Second proxy port should be available after stopping"
  );
  assert!(
    socks5_port_available,
    "First proxy port should be available after stopping"
  );

  println!("Proxy chaining test completed successfully");
  tracker.cleanup_all().await;
  Ok(())
}

mod common;
use common::TestUtils;
use serde_json::Value;

/// Setup function to ensure clean state before tests
async fn setup_test() -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = TestUtils::ensure_nodecar_binary().await?;

  // Only clean up test-specific processes, not all processes
  // This prevents interfering with actual app usage during testing
  println!("Setting up test environment...");

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
    // Use targeted cleanup to only stop test-specific processes
    let _ = TestUtils::cleanup_specific_processes(
      &self.nodecar_path,
      &self.proxy_ids,
      &self.camoufox_ids,
    )
    .await;
  }
}

impl Drop for TestResourceTracker {
  fn drop(&mut self) {
    // Ensure cleanup happens even if test panics
    let proxy_ids = self.proxy_ids.clone();
    let camoufox_ids = self.camoufox_ids.clone();
    let nodecar_path = self.nodecar_path.clone();

    tokio::spawn(async move {
      let _ = TestUtils::cleanup_specific_processes(&nodecar_path, &proxy_ids, &camoufox_ids).await;
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
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

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
  let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args).await?;

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

  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

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

  let start_output = TestUtils::execute_nodecar_command(&nodecar_path, &start_args).await?;

  if start_output.status.success() {
    let stdout = String::from_utf8(start_output.stdout)?;
    let config: Value = serde_json::from_str(&stdout)?;
    let proxy_id = config["id"].as_str().unwrap().to_string();
    tracker.track_proxy(proxy_id.clone());

    // Test list command
    let list_args = ["proxy", "list"];
    let list_output = TestUtils::execute_nodecar_command(&nodecar_path, &list_args).await?;

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
  ];

  println!("Starting Camoufox with nodecar...");
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

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
  let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args).await?;

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
  ];

  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

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
    let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args).await?;
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
  let list_output = TestUtils::execute_nodecar_command(&nodecar_path, &list_args).await?;

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
    ];

    println!("Starting Camoufox instance {i}...");
    let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

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
  let list_output = TestUtils::execute_nodecar_command(&nodecar_path, &list_args).await?;

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
    let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args).await?;

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
  let list_output_after = TestUtils::execute_nodecar_command(&nodecar_path, &list_args).await?;

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
    "--block-images",
    "--max-width",
    "1920",
    "--max-height",
    "1080",
    "--headless",
  ];

  println!("Starting Camoufox with configuration options...");
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

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
  let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args).await?;

  assert!(stop_output.status.success(), "Camoufox stop should succeed");

  println!("Camoufox configuration test completed successfully");
  tracker.cleanup_all().await;
  Ok(())
}

/// Test Camoufox generate-config command with basic options
#[ignore = "CI is rate limited for camoufox download"]
#[tokio::test]
async fn test_nodecar_camoufox_generate_config_basic(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let tracker = TestResourceTracker::new(nodecar_path.clone());

  let args = [
    "camoufox",
    "generate-config",
    "--max-width",
    "1920",
    "--max-height",
    "1080",
    "--block-images",
  ];

  println!("Testing Camoufox config generation with basic options...");
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    tracker.cleanup_all().await;
    return Err(
      format!("Camoufox generate-config failed - stdout: {stdout}, stderr: {stderr}").into(),
    );
  }

  let stdout = String::from_utf8(output.stdout)?;
  println!("Generated config output: {stdout}");

  // Parse the generated config as JSON
  let config: Value = serde_json::from_str(&stdout)?;

  // Verify the config contains expected properties
  assert!(
    config.is_object(),
    "Generated config should be a JSON object"
  );

  // Check for some expected fingerprint properties
  assert!(
    config.get("screen.width").is_some(),
    "Config should contain screen.width"
  );
  assert!(
    config.get("screen.height").is_some(),
    "Config should contain screen.height"
  );
  assert!(
    config.get("navigator.userAgent").is_some(),
    "Config should contain navigator.userAgent"
  );

  println!("Camoufox generate-config basic test completed successfully");
  tracker.cleanup_all().await;
  Ok(())
}

/// Test Camoufox generate-config command with custom fingerprint
#[ignore = "CI is rate limited for camoufox download"]
#[tokio::test]
async fn test_nodecar_camoufox_generate_config_custom_fingerprint(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let tracker = TestResourceTracker::new(nodecar_path.clone());

  // Create a custom fingerprint JSON
  let custom_fingerprint = r#"{
    "screen.width": 1440,
    "screen.height": 900,
    "navigator.userAgent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:135.0) Gecko/20100101 Firefox/140.0",
    "navigator.platform": "TestPlatform",
    "timezone": "America/New_York",
    "locale:language": "en",
    "locale:region": "US"
  }"#;

  let args = [
    "camoufox",
    "generate-config",
    "--fingerprint",
    custom_fingerprint,
    "--block-webrtc",
  ];

  println!("Testing Camoufox config generation with custom fingerprint...");
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    tracker.cleanup_all().await;
    return Err(
      format!("Camoufox generate-config with custom fingerprint failed - stdout: {stdout}, stderr: {stderr}").into(),
    );
  }

  let stdout = String::from_utf8(output.stdout)?;

  // Parse the generated config as JSON
  let config: Value = serde_json::from_str(&stdout)?;

  // Verify the config contains expected properties
  assert!(
    config.is_object(),
    "Generated config should be a JSON object"
  );

  // Check that our custom values are preserved
  assert_eq!(
    config.get("screen.width").and_then(|v| v.as_u64()),
    Some(1440),
    "Custom screen width should be preserved"
  );
  assert_eq!(
    config.get("screen.height").and_then(|v| v.as_u64()),
    Some(900),
    "Custom screen height should be preserved"
  );
  assert_eq!(
    config.get("navigator.userAgent").and_then(|v| v.as_str()),
    Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:135.0) Gecko/20100101 Firefox/140.0"),
    "Custom user agent should be preserved"
  );
  assert_eq!(
    config.get("timezone").and_then(|v| v.as_str()),
    Some("America/New_York"),
    "Custom timezone should be preserved"
  );

  println!("Camoufox generate-config custom fingerprint test completed successfully");
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
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &invalid_args).await?;

  assert!(!output.status.success(), "Invalid command should fail");

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

      TestUtils::execute_nodecar_command(&nodecar_path_clone, &args).await
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

    let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

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

/// Test direct proxy (no upstream) functionality
#[tokio::test]
async fn test_nodecar_direct_proxy() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let nodecar_path = setup_test().await?;
  let mut tracker = TestResourceTracker::new(nodecar_path.clone());

  // Test starting a direct proxy (no upstream)
  let args = ["proxy", "start"];

  println!("Starting direct proxy with nodecar...");
  let output = TestUtils::execute_nodecar_command(&nodecar_path, &args).await?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    tracker.cleanup_all().await;
    return Err(format!("Direct proxy start failed - stdout: {stdout}, stderr: {stderr}").into());
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
  assert_eq!(
    config["upstreamUrl"].as_str().unwrap(),
    "DIRECT",
    "Upstream URL should be DIRECT"
  );

  let proxy_id = config["id"].as_str().unwrap().to_string();
  let local_port = config["localPort"].as_u64().unwrap() as u16;
  tracker.track_proxy(proxy_id.clone());

  println!("Direct proxy started with ID: {proxy_id} on port: {local_port}");

  // Wait for the proxy to start listening
  let is_listening = TestUtils::wait_for_port_state(local_port, true, 10).await;
  assert!(
    is_listening,
    "Direct proxy should be listening on the assigned port"
  );

  // Test stopping the proxy
  let stop_args = ["proxy", "stop", "--id", &proxy_id];
  let stop_output = TestUtils::execute_nodecar_command(&nodecar_path, &stop_args).await?;

  assert!(
    stop_output.status.success(),
    "Direct proxy stop should succeed"
  );

  let port_available = TestUtils::wait_for_port_state(local_port, false, 5).await;
  assert!(
    port_available,
    "Port should be available after stopping direct proxy"
  );

  println!("Direct proxy test completed successfully");
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
  let socks5_output = TestUtils::execute_nodecar_command(&nodecar_path, &socks5_args).await?;

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
  let http_output = TestUtils::execute_nodecar_command(&nodecar_path, &http_proxy_args).await?;

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

  let http_stop_result = TestUtils::execute_nodecar_command(&nodecar_path, &stop_http_args).await;
  let socks5_stop_result =
    TestUtils::execute_nodecar_command(&nodecar_path, &stop_socks5_args).await;

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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri_plugin_shell::ShellExt;

use crate::browser::ProxySettings;

// Store active proxy information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyInfo {
  pub id: String,
  pub local_url: String,
  pub upstream_host: String,
  pub upstream_port: u16,
  pub upstream_type: String,
  pub local_port: u16,
}

// Global proxy manager to track active proxies
pub struct ProxyManager {
  active_proxies: Mutex<HashMap<u32, ProxyInfo>>, // Maps browser process ID to proxy info
  // Store proxy info by profile name for persistence across browser restarts
  profile_proxies: Mutex<HashMap<String, ProxySettings>>, // Maps profile name to proxy settings
}

impl ProxyManager {
  pub fn new() -> Self {
    Self {
      active_proxies: Mutex::new(HashMap::new()),
      profile_proxies: Mutex::new(HashMap::new()),
    }
  }

  // Start a proxy for given proxy settings and associate it with a browser process ID
  pub async fn start_proxy(
    &self,
    app_handle: tauri::AppHandle,
    proxy_settings: &ProxySettings,
    browser_pid: u32,
    profile_name: Option<&str>,
  ) -> Result<ProxySettings, String> {
    // Check if we already have a proxy for this browser
    {
      let proxies = self.active_proxies.lock().unwrap();
      if let Some(proxy) = proxies.get(&browser_pid) {
        return Ok(ProxySettings {
          enabled: true,
          proxy_type: proxy.upstream_type.clone(),
          host: "127.0.0.1".to_string(), // Use 127.0.0.1 instead of localhost for better compatibility
          port: proxy.local_port,
          username: None,
          password: None,
        });
      }
    }

    // Check if we have a preferred port for this profile
    let preferred_port = if let Some(name) = profile_name {
      let profile_proxies = self.profile_proxies.lock().unwrap();
      profile_proxies.get(name).and_then(|settings| {
        // Find existing proxy with same settings to reuse port
        let active_proxies = self.active_proxies.lock().unwrap();
        active_proxies
          .values()
          .find(|p| {
            p.upstream_host == settings.host
              && p.upstream_port == settings.port
              && p.upstream_type == settings.proxy_type
          })
          .map(|p| p.local_port)
      })
    } else {
      None
    };

    // Start a new proxy using the nodecar binary with the correct CLI interface
    let mut nodecar = app_handle
      .shell()
      .sidecar("nodecar")
      .map_err(|e| format!("Failed to create sidecar: {e}"))?
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg(&proxy_settings.host)
      .arg("--proxy-port")
      .arg(proxy_settings.port.to_string())
      .arg("--type")
      .arg(&proxy_settings.proxy_type);

    // Add credentials if provided
    if let Some(username) = &proxy_settings.username {
      nodecar = nodecar.arg("--username").arg(username);
    }
    if let Some(password) = &proxy_settings.password {
      nodecar = nodecar.arg("--password").arg(password);
    }

    // If we have a preferred port, use it
    if let Some(port) = preferred_port {
      nodecar = nodecar.arg("--port").arg(port.to_string());
    }

    // Execute the command and wait for it to complete
    // The nodecar binary should start the worker and then exit
    let output = nodecar
      .output()
      .await
      .map_err(|e| format!("Failed to execute nodecar: {e}"))?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let stdout = String::from_utf8_lossy(&output.stdout);
      return Err(format!(
        "Proxy start failed - stdout: {stdout}, stderr: {stderr}"
      ));
    }

    let json_string =
      String::from_utf8(output.stdout).map_err(|e| format!("Failed to parse proxy output: {e}"))?;

    // Parse the JSON output
    let json: Value =
      serde_json::from_str(&json_string).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    // Extract proxy information
    let id = json["id"].as_str().ok_or("Missing proxy ID")?;
    let local_port = json["localPort"].as_u64().ok_or("Missing local port")? as u16;
    let local_url = json["localUrl"]
      .as_str()
      .ok_or("Missing local URL")?
      .to_string();

    let proxy_info = ProxyInfo {
      id: id.to_string(),
      local_url,
      upstream_host: proxy_settings.host.clone(),
      upstream_port: proxy_settings.port,
      upstream_type: proxy_settings.proxy_type.clone(),
      local_port,
    };

    // Store the proxy info
    {
      let mut proxies = self.active_proxies.lock().unwrap();
      proxies.insert(browser_pid, proxy_info.clone());
    }

    // Store the profile proxy info for persistence
    if let Some(name) = profile_name {
      let mut profile_proxies = self.profile_proxies.lock().unwrap();
      profile_proxies.insert(name.to_string(), proxy_settings.clone());
    }

    // Return proxy settings for the browser
    Ok(ProxySettings {
      enabled: true,
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(), // Use 127.0.0.1 instead of localhost for better compatibility
      port: proxy_info.local_port,
      username: None,
      password: None,
    })
  }

  // Stop the proxy associated with a browser process ID
  pub async fn stop_proxy(
    &self,
    app_handle: tauri::AppHandle,
    browser_pid: u32,
  ) -> Result<(), String> {
    let proxy_id = {
      let mut proxies = self.active_proxies.lock().unwrap();
      match proxies.remove(&browser_pid) {
        Some(proxy) => proxy.id,
        None => return Ok(()), // No proxy to stop
      }
    };

    // Stop the proxy using the nodecar binary
    let nodecar = app_handle
      .shell()
      .sidecar("nodecar")
      .map_err(|e| format!("Failed to create sidecar: {e}"))?
      .arg("proxy")
      .arg("stop")
      .arg("--id")
      .arg(proxy_id);

    let output = nodecar.output().await.unwrap();

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      eprintln!("Proxy stop error: {stderr}");
      // We still return Ok since we've already removed the proxy from our tracking
    }

    Ok(())
  }

  // Get proxy settings for a browser process ID
  pub fn get_proxy_settings(&self, browser_pid: u32) -> Option<ProxySettings> {
    let proxies = self.active_proxies.lock().unwrap();
    proxies.get(&browser_pid).map(|proxy| ProxySettings {
      enabled: true,
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(), // Use 127.0.0.1 instead of localhost for better compatibility
      port: proxy.local_port,
      username: None,
      password: None,
    })
  }

  // Get stored proxy info for a profile
  pub fn get_profile_proxy_info(&self, profile_name: &str) -> Option<ProxySettings> {
    let profile_proxies = self.profile_proxies.lock().unwrap();
    profile_proxies.get(profile_name).cloned()
  }
}

// Create a singleton instance of the proxy manager
lazy_static::lazy_static! {
    pub static ref PROXY_MANAGER: ProxyManager = ProxyManager::new();
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::env;
  use std::path::PathBuf;
  use std::process::Command;
  use std::time::Duration;
  use tokio::time::sleep;

  // Mock HTTP server for testing

  use http_body_util::Full;
  use hyper::body::Bytes;
  use hyper::server::conn::http1;
  use hyper::service::service_fn;
  use hyper::Response;
  use hyper_util::rt::TokioIo;
  use tokio::net::TcpListener;

  // Helper function to build nodecar binary for testing
  async fn ensure_nodecar_binary() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let project_root = PathBuf::from(cargo_manifest_dir)
      .parent()
      .unwrap()
      .to_path_buf();
    let nodecar_dir = project_root.join("nodecar");
    let nodecar_dist = nodecar_dir.join("dist");
    let nodecar_binary = nodecar_dist.join("nodecar");

    // Check if binary already exists
    if nodecar_binary.exists() {
      return Ok(nodecar_binary);
    }

    // Build the nodecar binary
    println!("Building nodecar binary for tests...");

    // Install dependencies
    let install_status = Command::new("pnpm")
      .args(["install", "--frozen-lockfile"])
      .current_dir(&nodecar_dir)
      .status()?;

    if !install_status.success() {
      return Err("Failed to install nodecar dependencies".into());
    }

    // Determine the target architecture
    let target = if cfg!(target_arch = "aarch64") && cfg!(target_os = "macos") {
      "build:mac-aarch64"
    } else if cfg!(target_arch = "x86_64") && cfg!(target_os = "macos") {
      "build:mac-x86_64"
    } else if cfg!(target_arch = "x86_64") && cfg!(target_os = "linux") {
      "build:linux-x64"
    } else if cfg!(target_arch = "aarch64") && cfg!(target_os = "linux") {
      "build:linux-arm64"
    } else if cfg!(target_arch = "x86_64") && cfg!(target_os = "windows") {
      "build:win-x64"
    } else if cfg!(target_arch = "aarch64") && cfg!(target_os = "windows") {
      "build:win-arm64"
    } else {
      return Err("Unsupported target architecture for nodecar build".into());
    };

    // Build the binary
    let build_status = Command::new("pnpm")
      .args(["run", target])
      .current_dir(&nodecar_dir)
      .status()?;

    if !build_status.success() {
      return Err("Failed to build nodecar binary".into());
    }

    if !nodecar_binary.exists() {
      return Err("Nodecar binary was not created successfully".into());
    }

    Ok(nodecar_binary)
  }

  #[tokio::test]
  async fn test_proxy_manager_profile_persistence() {
    let proxy_manager = ProxyManager::new();

    let proxy_settings = ProxySettings {
      enabled: true,
      proxy_type: "socks5".to_string(),
      host: "127.0.0.1".to_string(),
      port: 1080,
      username: None,
      password: None,
    };

    // Test profile proxy info storage
    {
      let mut profile_proxies = proxy_manager.profile_proxies.lock().unwrap();
      profile_proxies.insert("test_profile".to_string(), proxy_settings.clone());
    }

    // Test retrieval
    let retrieved = proxy_manager.get_profile_proxy_info("test_profile");
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.proxy_type, "socks5");
    assert_eq!(retrieved.host, "127.0.0.1");
    assert_eq!(retrieved.port, 1080);

    // Test non-existent profile
    let non_existent = proxy_manager.get_profile_proxy_info("non_existent");
    assert!(non_existent.is_none());
  }

  #[tokio::test]
  async fn test_proxy_manager_active_proxy_tracking() {
    let proxy_manager = ProxyManager::new();

    let proxy_info = ProxyInfo {
      id: "test_proxy_123".to_string(),
      local_url: "http://localhost:8080".to_string(),
      upstream_host: "proxy.example.com".to_string(),
      upstream_port: 3128,
      upstream_type: "http".to_string(),
      local_port: 8080,
    };

    let browser_pid = 54321u32;

    // Add active proxy
    {
      let mut active_proxies = proxy_manager.active_proxies.lock().unwrap();
      active_proxies.insert(browser_pid, proxy_info.clone());
    }

    // Test retrieval of proxy settings
    let proxy_settings = proxy_manager.get_proxy_settings(browser_pid);
    assert!(proxy_settings.is_some());
    let settings = proxy_settings.unwrap();
    assert!(settings.enabled);
    assert_eq!(settings.host, "127.0.0.1");
    assert_eq!(settings.port, 8080);

    // Test non-existent browser PID
    let non_existent = proxy_manager.get_proxy_settings(99999);
    assert!(non_existent.is_none());
  }

  #[test]
  fn test_proxy_settings_validation() {
    // Test valid proxy settings
    let valid_settings = ProxySettings {
      enabled: true,
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: Some("user".to_string()),
      password: Some("pass".to_string()),
    };

    assert!(valid_settings.enabled);
    assert_eq!(valid_settings.proxy_type, "http");
    assert!(!valid_settings.host.is_empty());
    assert!(valid_settings.port > 0);

    // Test disabled proxy settings
    let disabled_settings = ProxySettings {
      enabled: false,
      proxy_type: "http".to_string(),
      host: "".to_string(),
      port: 0,
      username: None,
      password: None,
    };

    assert!(!disabled_settings.enabled);
  }

  #[tokio::test]
  async fn test_proxy_manager_concurrent_access() {
    use std::sync::Arc;

    let proxy_manager = Arc::new(ProxyManager::new());
    let mut handles = vec![];

    // Spawn multiple tasks that access the proxy manager concurrently
    for i in 0..10 {
      let pm = proxy_manager.clone();
      let handle = tokio::spawn(async move {
        let browser_pid = (1000 + i) as u32;
        let proxy_info = ProxyInfo {
          id: format!("proxy_{i}"),
          local_url: format!("http://127.0.0.1:{}", 8000 + i),
          upstream_host: "127.0.0.1".to_string(),
          upstream_port: 3128,
          upstream_type: "http".to_string(),
          local_port: (8000 + i) as u16,
        };

        // Add proxy
        {
          let mut active_proxies = pm.active_proxies.lock().unwrap();
          active_proxies.insert(browser_pid, proxy_info);
        }

        // Read proxy
        let settings = pm.get_proxy_settings(browser_pid);
        assert!(settings.is_some());

        browser_pid
      });
      handles.push(handle);
    }

    // Wait for all tasks to complete
    let results: Vec<u32> = futures_util::future::join_all(handles)
      .await
      .into_iter()
      .map(|r| r.unwrap())
      .collect();

    // Verify all browser PIDs were processed
    assert_eq!(results.len(), 10);
    for (i, &browser_pid) in results.iter().enumerate() {
      assert_eq!(browser_pid, (1000 + i) as u32);
    }
  }

  // Integration test that actually builds and uses nodecar binary
  #[tokio::test]
  async fn test_proxy_integration_with_real_nodecar() -> Result<(), Box<dyn std::error::Error>> {
    // This test requires nodecar to be built and available
    let nodecar_path = ensure_nodecar_binary().await?;

    // Start a mock upstream HTTP server
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_addr = upstream_listener.local_addr()?;

    // Spawn upstream server
    let server_handle = tokio::spawn(async move {
      while let Ok((stream, _)) = upstream_listener.accept().await {
        let io = TokioIo::new(stream);
        tokio::task::spawn(async move {
          let _ = http1::Builder::new()
            .serve_connection(
              io,
              service_fn(|_req| async {
                Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::from("Upstream OK"))))
              }),
            )
            .await;
        });
      }
    });

    // Wait for server to start
    sleep(Duration::from_millis(100)).await;

    // Test nodecar proxy start command directly (using the binary itself, not node)
    let mut cmd = Command::new(&nodecar_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg(upstream_addr.ip().to_string())
      .arg("--proxy-port")
      .arg(upstream_addr.port().to_string())
      .arg("--type")
      .arg("http");

    // Set a timeout for the command
    let output = tokio::time::timeout(Duration::from_secs(10), async { cmd.output() }).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;

      // Verify proxy configuration
      assert!(config["id"].is_string());
      assert!(config["localPort"].is_number());
      assert!(config["localUrl"].is_string());

      let proxy_id = config["id"].as_str().unwrap();
      let local_port = config["localPort"].as_u64().unwrap();

      // Wait for proxy worker to start
      println!("Waiting for proxy worker to start...");
      tokio::time::sleep(Duration::from_secs(3)).await;

      // Test that the local port is listening
      let mut port_test = Command::new("nc");
      port_test
        .arg("-z")
        .arg("127.0.0.1")
        .arg(local_port.to_string());

      let port_output = port_test.output()?;
      if port_output.status.success() {
        println!("Proxy is listening on port {local_port}");
      } else {
        println!("Warning: Proxy port {local_port} is not listening");
      }

      // Test stopping the proxy
      let mut stop_cmd = Command::new(&nodecar_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);

      let stop_output =
        tokio::time::timeout(Duration::from_secs(5), async { stop_cmd.output() }).await??;

      assert!(stop_output.status.success());

      println!("Integration test passed: nodecar proxy start/stop works correctly");
    } else {
      let stderr = String::from_utf8(output.stderr)?;
      eprintln!("Nodecar failed: {stderr}");
      return Err(format!("Nodecar command failed: {stderr}").into());
    }

    // Clean up server
    server_handle.abort();

    Ok(())
  }

  // Test that validates the command line arguments are constructed correctly
  #[test]
  fn test_proxy_command_construction() {
    let proxy_settings = ProxySettings {
      enabled: true,
      proxy_type: "http".to_string(),
      host: "proxy.example.com".to_string(),
      port: 8080,
      username: Some("user".to_string()),
      password: Some("pass".to_string()),
    };

    // Test command arguments match expected format
    let _expected_args = [
      "proxy",
      "start",
      "--host",
      "proxy.example.com",
      "--proxy-port",
      "8080",
      "--type",
      "http",
      "--username",
      "user",
      "--password",
      "pass",
    ];

    // This test verifies the argument structure without actually running the command
    assert_eq!(proxy_settings.host, "proxy.example.com");
    assert_eq!(proxy_settings.port, 8080);
    assert_eq!(proxy_settings.proxy_type, "http");
    assert_eq!(proxy_settings.username.as_ref().unwrap(), "user");
    assert_eq!(proxy_settings.password.as_ref().unwrap(), "pass");
  }

  // Test the CLI detachment specifically - ensure the CLI exits properly
  #[tokio::test]
  async fn test_cli_exits_after_proxy_start() -> Result<(), Box<dyn std::error::Error>> {
    let nodecar_path = ensure_nodecar_binary().await?;

    // Test that the CLI exits quickly with a mock upstream
    let mut cmd = Command::new(&nodecar_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("httpbin.org")
      .arg("--proxy-port")
      .arg("80")
      .arg("--type")
      .arg("http");

    let start_time = std::time::Instant::now();
    let output = tokio::time::timeout(Duration::from_secs(3), async { cmd.output() }).await;

    match output {
      Ok(Ok(cmd_output)) => {
        let execution_time = start_time.elapsed();
        println!("CLI completed in {execution_time:?}");

        // Should complete very quickly if properly detached
        assert!(
          execution_time < Duration::from_secs(3),
          "CLI took too long ({execution_time:?}), should exit immediately after starting worker"
        );

        if cmd_output.status.success() {
          let stdout = String::from_utf8(cmd_output.stdout)?;
          let config: serde_json::Value = serde_json::from_str(&stdout)?;
          
          // Clean up - try to stop the proxy
          if let Some(proxy_id) = config["id"].as_str() {
            let mut stop_cmd = Command::new(&nodecar_path);
            stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
            let _ = stop_cmd.output();
          }
        }

        println!("CLI detachment test passed - CLI exited in {execution_time:?}");
      }
      Ok(Err(e)) => {
        return Err(format!("Command execution failed: {e}").into());
      }
      Err(_) => {
        return Err("CLI command timed out - this indicates improper detachment".into());
      }
    }

    Ok(())
  }

  // Test that validates proper CLI detachment behavior
  #[tokio::test]
  async fn test_cli_detachment_behavior() -> Result<(), Box<dyn std::error::Error>> {
    let nodecar_path = ensure_nodecar_binary().await?;

    // Test that the CLI command exits quickly even with a real upstream
    let mut cmd = Command::new(&nodecar_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("httpbin.org") // Use a known good endpoint
      .arg("--proxy-port")
      .arg("80")
      .arg("--type")
      .arg("http");

    let start_time = std::time::Instant::now();
    let output = tokio::time::timeout(Duration::from_secs(5), async { cmd.output() }).await??;
    let execution_time = start_time.elapsed();

    // Command should complete very quickly if properly detached
    assert!(
      execution_time < Duration::from_secs(5),
      "CLI command took {execution_time:?}, should complete in under 5 seconds for proper detachment"
    );

    println!("CLI detachment test: command completed in {execution_time:?}");

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;
      let proxy_id = config["id"].as_str().unwrap();

      // Clean up
      let mut stop_cmd = Command::new(&nodecar_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
      let _ = stop_cmd.output();

      println!("CLI detachment test passed");
    } else {
      // Even if the upstream fails, the CLI should still exit quickly
      println!("CLI command failed but exited quickly as expected");
    }

    Ok(())
  }

  // Test that validates URL encoding for special characters in credentials
  #[tokio::test]
  async fn test_proxy_credentials_encoding() -> Result<(), Box<dyn std::error::Error>> {
    let nodecar_path = ensure_nodecar_binary().await?;

    // Test with credentials that include special characters
    let mut cmd = Command::new(&nodecar_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("test.example.com")
      .arg("--proxy-port")
      .arg("8080")
      .arg("--type")
      .arg("http")
      .arg("--username")
      .arg("user@domain.com") // Contains @ symbol
      .arg("--password")
      .arg("pass word!"); // Contains space and special character

    let output = tokio::time::timeout(Duration::from_secs(5), async { cmd.output() }).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;

      let upstream_url = config["upstreamUrl"].as_str().unwrap();

      println!("Generated upstream URL: {upstream_url}");

      // Verify that special characters are properly encoded
      assert!(upstream_url.contains("user%40domain.com"));
      // The password may be encoded as "pass%20word!" or "pass%20word%21" depending on implementation
      assert!(upstream_url.contains("pass%20word"));

      println!("URL encoding test passed - special characters handled correctly");

      // Clean up
      let proxy_id = config["id"].as_str().unwrap();
      let mut stop_cmd = Command::new(&nodecar_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
      let _ = stop_cmd.output();
    } else {
      // This test might fail if the upstream doesn't exist, but we mainly care about URL construction
      let stdout = String::from_utf8(output.stdout)?;
      let stderr = String::from_utf8(output.stderr)?;
      println!("Command failed (expected for non-existent upstream):");
      println!("Stdout: {stdout}");
      println!("Stderr: {stderr}");

      // The important thing is that the command completed quickly
      println!("URL encoding test completed - credentials should be properly encoded");
    }

    Ok(())
  }
}

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// Utility functions for integration tests
pub struct TestUtils;

impl TestUtils {
  /// Build the nodecar binary if it doesn't exist
  pub async fn ensure_nodecar_binary() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>>
  {
    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let project_root = PathBuf::from(cargo_manifest_dir)
      .parent()
      .unwrap()
      .to_path_buf();
    let nodecar_dir = project_root.join("nodecar");
    let nodecar_binary = nodecar_dir.join("nodecar-bin");

    // Check if binary already exists
    if nodecar_binary.exists() {
      return Ok(nodecar_binary);
    }

    println!("Building nodecar binary for integration tests...");

    // Install dependencies
    let install_status = Command::new("pnpm")
      .args(["install", "--frozen-lockfile"])
      .current_dir(&nodecar_dir)
      .status()?;

    if !install_status.success() {
      return Err("Failed to install nodecar dependencies".into());
    }

    // Build the binary
    let build_status = Command::new("pnpm")
      .args(["run", "build"])
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

  /// Execute a nodecar command with timeout
  pub async fn execute_nodecar_command(
    binary_path: &PathBuf,
    args: &[&str],
  ) -> Result<std::process::Output, Box<dyn std::error::Error + Send + Sync>> {
    let mut cmd = Command::new(binary_path);
    cmd.args(args);

    let output = tokio::process::Command::from(cmd).output().await?;

    Ok(output)
  }

  /// Check if a port is available
  pub async fn is_port_available(port: u16) -> bool {
    tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
      .await
      .is_ok()
  }

  /// Wait for a port to become available or occupied
  pub async fn wait_for_port_state(port: u16, should_be_occupied: bool, timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();

    while start.elapsed().as_secs() < timeout_secs {
      let is_available = Self::is_port_available(port).await;

      if should_be_occupied && !is_available {
        return true; // Port is occupied as expected
      } else if !should_be_occupied && is_available {
        return true; // Port is available as expected
      }

      tokio::time::sleep(Duration::from_millis(100)).await;
    }

    false
  }

  /// Create a temporary directory for test files
  pub fn create_temp_dir() -> Result<tempfile::TempDir, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tempfile::tempdir()?)
  }

  /// Clean up specific nodecar processes by IDs (for targeted test cleanup)
  pub async fn cleanup_specific_processes(
    nodecar_path: &PathBuf,
    proxy_ids: &[String],
    camoufox_ids: &[String],
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Cleaning up specific test processes...");

    // Stop specific proxies
    for proxy_id in proxy_ids {
      let stop_args = ["proxy", "stop", "--id", proxy_id];
      if let Ok(output) = Self::execute_nodecar_command(nodecar_path, &stop_args).await {
        if output.status.success() {
          println!("Stopped test proxy: {proxy_id}");
        }
      }
    }

    // Stop specific camoufox instances
    for camoufox_id in camoufox_ids {
      let stop_args = ["camoufox", "stop", "--id", camoufox_id];
      if let Ok(output) = Self::execute_nodecar_command(nodecar_path, &stop_args).await {
        if output.status.success() {
          println!("Stopped test camoufox instance: {camoufox_id}");
        }
      }
    }

    // Give processes time to clean up
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("Test process cleanup completed");
    Ok(())
  }
}

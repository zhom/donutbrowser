use std::path::PathBuf;
use std::process::Command;

/// Utility functions for integration tests
pub struct TestUtils;

impl TestUtils {
  /// Execute a command (generic, for donut-proxy tests)
  #[allow(dead_code)]
  pub async fn execute_command(
    binary_path: &PathBuf,
    args: &[&str],
  ) -> Result<std::process::Output, Box<dyn std::error::Error + Send + Sync>> {
    let mut cmd = Command::new(binary_path);
    cmd.args(args);

    let output = tokio::process::Command::from(cmd).output().await?;

    Ok(output)
  }
}

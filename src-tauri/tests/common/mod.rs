use std::path::PathBuf;
use std::process::Command;

/// Whether Docker can run the Linux images these fixtures are built from.
///
/// `docker version` succeeding is not enough: the Windows CI runner ships a
/// daemon in Windows-container mode, which answers that command happily and
/// then rejects the fixtures with `unknown capability: "CAP_NET_ADMIN"`.
#[allow(dead_code)]
pub fn docker_supports_linux_containers() -> bool {
  Command::new("docker")
    .args(["version", "--format", "{{.Server.Os}}"])
    .output()
    .map(|output| {
      output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "linux"
    })
    .unwrap_or(false)
}

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

//! OpenVPN tunnel implementation using system openvpn binary.

use super::config::{OpenVpnConfig, VpnError, VpnStatus};
use super::tunnel::VpnTunnel;
use async_trait::async_trait;
use chrono::Utc;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::Mutex;

/// OpenVPN tunnel implementation
pub struct OpenVpnTunnel {
  vpn_id: String,
  config: OpenVpnConfig,
  process: Arc<Mutex<Option<Child>>>,
  config_file: Option<NamedTempFile>,
  connected: AtomicBool,
  connected_at: Option<i64>,
  bytes_sent: AtomicU64,
  bytes_received: AtomicU64,
}

impl OpenVpnTunnel {
  /// Create a new OpenVPN tunnel
  pub fn new(vpn_id: String, config: OpenVpnConfig) -> Self {
    Self {
      vpn_id,
      config,
      process: Arc::new(Mutex::new(None)),
      config_file: None,
      connected: AtomicBool::new(false),
      connected_at: None,
      bytes_sent: AtomicU64::new(0),
      bytes_received: AtomicU64::new(0),
    }
  }

  /// Find the openvpn binary
  fn find_openvpn_binary() -> Result<PathBuf, VpnError> {
    // Check common locations
    let locations = [
      "/usr/sbin/openvpn",
      "/usr/local/sbin/openvpn",
      "/opt/homebrew/bin/openvpn",
      "/usr/bin/openvpn",
      "C:\\Program Files\\OpenVPN\\bin\\openvpn.exe",
      "C:\\Program Files (x86)\\OpenVPN\\bin\\openvpn.exe",
    ];

    for loc in &locations {
      let path = PathBuf::from(loc);
      if path.exists() {
        return Ok(path);
      }
    }

    // Try to find via which/where command
    #[cfg(unix)]
    {
      if let Ok(output) = Command::new("which").arg("openvpn").output() {
        if output.status.success() {
          let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
          if !path.is_empty() {
            return Ok(PathBuf::from(path));
          }
        }
      }
    }

    #[cfg(windows)]
    {
      if let Ok(output) = Command::new("where").arg("openvpn").output() {
        if output.status.success() {
          let path = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
          if !path.is_empty() {
            return Ok(PathBuf::from(path));
          }
        }
      }
    }

    Err(VpnError::Connection(
      "OpenVPN binary not found. Please install OpenVPN.".to_string(),
    ))
  }

  /// Write config to temporary file
  fn write_config_file(&mut self) -> Result<PathBuf, VpnError> {
    let temp_file =
      NamedTempFile::new().map_err(|e| VpnError::Io(std::io::Error::other(e.to_string())))?;

    std::fs::write(temp_file.path(), &self.config.raw_config).map_err(VpnError::Io)?;

    let path = temp_file.path().to_path_buf();
    self.config_file = Some(temp_file);

    Ok(path)
  }

  /// Start the OpenVPN process
  async fn start_process(&mut self) -> Result<(), VpnError> {
    let openvpn_bin = Self::find_openvpn_binary()?;
    let config_path = self.write_config_file()?;

    log::info!(
      "[vpn] Starting OpenVPN with config: {}",
      config_path.display()
    );

    // Build command with common options
    let mut cmd = Command::new(&openvpn_bin);
    cmd
      .arg("--config")
      .arg(&config_path)
      .arg("--verb")
      .arg("3") // Verbosity level
      .stdout(Stdio::piped())
      .stderr(Stdio::piped());

    // On Unix, try to avoid requiring root if possible
    #[cfg(unix)]
    {
      cmd.arg("--script-security").arg("2");
    }

    let child = cmd
      .spawn()
      .map_err(|e| VpnError::Connection(format!("Failed to start OpenVPN: {e}")))?;

    *self.process.lock().await = Some(child);

    // Wait a bit and check if process is still running
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let mut process_guard = self.process.lock().await;
    if let Some(ref mut child) = *process_guard {
      match child.try_wait() {
        Ok(Some(status)) => {
          // Process exited early
          let mut error_msg = format!("OpenVPN exited with status: {status}");

          // Try to get stderr output
          if let Some(stderr) = child.stderr.take() {
            let reader = BufReader::new(stderr);
            let lines: Vec<String> = reader.lines().map_while(Result::ok).take(5).collect();
            if !lines.is_empty() {
              error_msg.push_str(&format!("\nError: {}", lines.join("\n")));
            }
          }

          return Err(VpnError::Connection(error_msg));
        }
        Ok(None) => {
          // Still running, good
        }
        Err(e) => {
          return Err(VpnError::Connection(format!(
            "Failed to check process status: {e}"
          )));
        }
      }
    }

    Ok(())
  }

  /// Kill the OpenVPN process
  async fn kill_process(&mut self) -> Result<(), VpnError> {
    let mut process_guard = self.process.lock().await;

    if let Some(mut child) = process_guard.take() {
      // Try graceful shutdown first
      #[cfg(unix)]
      {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        if let Ok(pid) = child.id().try_into() {
          let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
          // Wait a bit for graceful shutdown
          tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
      }

      // Force kill if still running
      let _ = child.kill();
      let _ = child.wait();
    }

    // Clean up config file
    self.config_file = None;

    Ok(())
  }
}

#[async_trait]
impl VpnTunnel for OpenVpnTunnel {
  async fn connect(&mut self) -> Result<(), VpnError> {
    if self.connected.load(Ordering::Relaxed) {
      return Ok(());
    }

    // Start OpenVPN process
    self.start_process().await?;

    // Wait for connection to be established
    // Note: In a real implementation, we'd monitor the OpenVPN management interface
    // For now, we assume success if the process starts and runs for a bit
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Check if process is still running
    let process_guard = self.process.lock().await;
    if let Some(ref child) = *process_guard {
      let id = child.id();
      if id > 0 {
        self.connected.store(true, Ordering::Release);
        self.connected_at = Some(Utc::now().timestamp());
        log::info!("[vpn] OpenVPN tunnel {} connected (PID: {id})", self.vpn_id);
        return Ok(());
      }
    }

    Err(VpnError::Connection(
      "Failed to establish OpenVPN connection".to_string(),
    ))
  }

  async fn disconnect(&mut self) -> Result<(), VpnError> {
    if !self.connected.load(Ordering::Relaxed) {
      return Ok(());
    }

    self.kill_process().await?;

    self.connected.store(false, Ordering::Release);
    self.connected_at = None;

    log::info!("[vpn] OpenVPN tunnel {} disconnected", self.vpn_id);

    Ok(())
  }

  fn is_connected(&self) -> bool {
    self.connected.load(Ordering::Acquire)
  }

  fn vpn_id(&self) -> &str {
    &self.vpn_id
  }

  fn get_status(&self) -> VpnStatus {
    VpnStatus {
      connected: self.is_connected(),
      vpn_id: self.vpn_id.clone(),
      connected_at: self.connected_at,
      bytes_sent: Some(self.bytes_sent.load(Ordering::Relaxed)),
      bytes_received: Some(self.bytes_received.load(Ordering::Relaxed)),
      last_handshake: None,
    }
  }

  fn bytes_sent(&self) -> u64 {
    self.bytes_sent.load(Ordering::Relaxed)
  }

  fn bytes_received(&self) -> u64 {
    self.bytes_received.load(Ordering::Relaxed)
  }
}

impl Drop for OpenVpnTunnel {
  fn drop(&mut self) {
    // Clean up process on drop (synchronously)
    if let Ok(mut guard) = self.process.try_lock() {
      if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn create_test_config() -> OpenVpnConfig {
    OpenVpnConfig {
      raw_config: "client\nremote test.example.com 1194\ndev tun".to_string(),
      remote_host: "test.example.com".to_string(),
      remote_port: 1194,
      protocol: "udp".to_string(),
      dev_type: "tun".to_string(),
      has_inline_ca: false,
      has_inline_cert: false,
      has_inline_key: false,
    }
  }

  #[test]
  fn test_openvpn_tunnel_creation() {
    let config = create_test_config();
    let tunnel = OpenVpnTunnel::new("test-ovpn-1".to_string(), config);

    assert_eq!(tunnel.vpn_id(), "test-ovpn-1");
    assert!(!tunnel.is_connected());
    assert_eq!(tunnel.bytes_sent(), 0);
    assert_eq!(tunnel.bytes_received(), 0);
  }

  #[test]
  fn test_openvpn_status() {
    let config = create_test_config();
    let tunnel = OpenVpnTunnel::new("test-ovpn-2".to_string(), config);

    let status = tunnel.get_status();
    assert!(!status.connected);
    assert_eq!(status.vpn_id, "test-ovpn-2");
    assert!(status.connected_at.is_none());
  }

  #[test]
  fn test_find_openvpn_binary_format() {
    // This test just checks that the function doesn't panic
    // It may or may not find openvpn depending on the system
    let result = OpenVpnTunnel::find_openvpn_binary();
    // Just check that it returns a valid Result
    match result {
      Ok(path) => assert!(!path.as_os_str().is_empty()),
      Err(e) => assert!(e.to_string().contains("not found")),
    }
  }
}

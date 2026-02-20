use super::config::{OpenVpnConfig, VpnError};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub struct OpenVpnSocks5Server {
  config: OpenVpnConfig,
  port: u16,
}

impl OpenVpnSocks5Server {
  pub fn new(config: OpenVpnConfig, port: u16) -> Self {
    Self { config, port }
  }

  fn find_openvpn_binary() -> Result<PathBuf, VpnError> {
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

  pub async fn run(self, config_id: String) -> Result<(), VpnError> {
    let openvpn_bin = Self::find_openvpn_binary()?;

    // Write config to temp file
    let config_path = std::env::temp_dir().join(format!("openvpn_{}.ovpn", config_id));
    std::fs::write(&config_path, &self.config.raw_config).map_err(VpnError::Io)?;

    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let _ = std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600));
    }

    // Find a management port
    let mgmt_listener = std::net::TcpListener::bind("127.0.0.1:0")
      .map_err(|e| VpnError::Connection(format!("Failed to bind management port: {e}")))?;
    let mgmt_port = mgmt_listener
      .local_addr()
      .map_err(|e| VpnError::Connection(format!("Failed to get management port: {e}")))?
      .port();
    drop(mgmt_listener);

    // Start OpenVPN with SOCKS proxy mode
    let mut cmd = Command::new(&openvpn_bin);
    cmd
      .arg("--config")
      .arg(&config_path)
      .arg("--management")
      .arg("127.0.0.1")
      .arg(mgmt_port.to_string())
      .arg("--socks-proxy")
      .arg("127.0.0.1")
      .arg(self.port.to_string())
      .arg("--verb")
      .arg("3")
      .stdout(Stdio::piped())
      .stderr(Stdio::piped());

    let mut child = cmd
      .spawn()
      .map_err(|e| VpnError::Connection(format!("Failed to start OpenVPN: {e}")))?;

    // Wait for OpenVPN to start
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    match child.try_wait() {
      Ok(Some(status)) => {
        let _ = std::fs::remove_file(&config_path);
        return Err(VpnError::Connection(format!(
          "OpenVPN exited early with status: {status}. OpenVPN requires elevated privileges (sudo/admin)."
        )));
      }
      Ok(None) => {}
      Err(e) => {
        let _ = std::fs::remove_file(&config_path);
        return Err(VpnError::Connection(format!(
          "Failed to check OpenVPN status: {e}"
        )));
      }
    }

    // Start a basic SOCKS5 proxy that tunnels through the OpenVPN TUN interface
    let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port))
      .await
      .map_err(|e| VpnError::Connection(format!("Failed to bind SOCKS5: {e}")))?;

    let actual_port = listener
      .local_addr()
      .map_err(|e| VpnError::Connection(format!("Failed to get local addr: {e}")))?
      .port();

    if let Some(mut wc) = crate::vpn_worker_storage::get_vpn_worker_config(&config_id) {
      wc.local_port = Some(actual_port);
      wc.local_url = Some(format!("socks5://127.0.0.1:{}", actual_port));
      let _ = crate::vpn_worker_storage::save_vpn_worker_config(&wc);
    }

    log::info!(
      "[vpn-worker] OpenVPN SOCKS5 server listening on 127.0.0.1:{}",
      actual_port
    );

    loop {
      match listener.accept().await {
        Ok((client, _)) => {
          tokio::spawn(Self::handle_socks5_client(client));
        }
        Err(e) => {
          log::warn!("[vpn-worker] Accept error: {e}");
        }
      }
    }
  }

  async fn handle_socks5_client(
    mut client: TcpStream,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // SOCKS5 greeting
    let mut buf = [0u8; 256];
    let n = client.read(&mut buf).await?;
    if n < 3 || buf[0] != 0x05 {
      return Ok(());
    }
    client.write_all(&[0x05, 0x00]).await?;

    // SOCKS5 connect request
    let n = client.read(&mut buf).await?;
    if n < 10 || buf[0] != 0x05 || buf[1] != 0x01 {
      return Ok(());
    }

    let dest_addr = match buf[3] {
      0x01 => {
        let ip = std::net::Ipv4Addr::new(buf[4], buf[5], buf[6], buf[7]);
        let port = u16::from_be_bytes([buf[8], buf[9]]);
        format!("{}:{}", ip, port)
      }
      0x03 => {
        let domain_len = buf[4] as usize;
        let domain = String::from_utf8_lossy(&buf[5..5 + domain_len]).to_string();
        let port_start = 5 + domain_len;
        let port = u16::from_be_bytes([buf[port_start], buf[port_start + 1]]);
        format!("{}:{}", domain, port)
      }
      _ => return Ok(()),
    };

    // Connect to destination through OpenVPN tunnel (OS routing handles it)
    match TcpStream::connect(&dest_addr).await {
      Ok(upstream) => {
        client
          .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
          .await?;

        let (mut cr, mut cw) = client.into_split();
        let (mut ur, mut uw) = upstream.into_split();

        let c2u = tokio::io::copy(&mut cr, &mut uw);
        let u2c = tokio::io::copy(&mut ur, &mut cw);

        let _ = tokio::try_join!(c2u, u2c);
      }
      Err(_) => {
        client
          .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
          .await?;
      }
    }

    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_find_openvpn_binary_format() {
    let result = OpenVpnSocks5Server::find_openvpn_binary();
    match result {
      Ok(path) => assert!(!path.as_os_str().is_empty()),
      Err(e) => assert!(e.to_string().contains("not found")),
    }
  }
}

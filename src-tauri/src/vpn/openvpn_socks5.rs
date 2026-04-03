use super::config::{OpenVpnConfig, VpnError};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{lookup_host, TcpListener, TcpSocket, TcpStream};

const OPENVPN_CONNECT_TIMEOUT_SECS: u64 = 90;

enum SocksTarget {
  Address(SocketAddr),
  Domain(String, u16),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenVpnDependencyStatus {
  pub binary_found: bool,
  pub missing_windows_adapter: bool,
  pub dependency_check_failed: bool,
}

pub struct OpenVpnSocks5Server {
  config: OpenVpnConfig,
  port: u16,
}

impl OpenVpnSocks5Server {
  pub fn new(config: OpenVpnConfig, port: u16) -> Self {
    Self { config, port }
  }

  fn read_log_tail(path: &Path, lines: usize) -> String {
    std::fs::read_to_string(path)
      .unwrap_or_default()
      .lines()
      .rev()
      .take(lines)
      .collect::<Vec<_>>()
      .into_iter()
      .rev()
      .collect::<Vec<_>>()
      .join("\n")
  }

  fn extract_vpn_ip(line: &str) -> Option<Ipv4Addr> {
    for field in line.split(',') {
      let trimmed = field.trim();
      if let Ok(ip) = trimmed.parse::<Ipv4Addr>() {
        if ip.is_private() && !ip.is_loopback() {
          return Some(ip);
        }
      }
    }

    None
  }

  fn log_indicates_connected(log_content: &str) -> bool {
    log_content.contains("Initialization Sequence Completed")
  }

  fn log_indicates_failure(log_content: &str) -> bool {
    log_content.contains("AUTH_FAILED")
      || log_content.contains("Exiting due to fatal error")
      || log_content.contains("Fatal error")
      || log_content.contains("Options error")
      || log_content.contains("Exiting")
  }

  fn has_config_directive(config: &str, directive: &str) -> bool {
    config.lines().any(|line| {
      let trimmed = line.trim();
      !trimmed.is_empty()
        && !trimmed.starts_with('#')
        && !trimmed.starts_with(';')
        && trimmed.starts_with(directive)
    })
  }

  fn strip_config_directive(config: &str, directive: &str) -> String {
    config
      .lines()
      .filter(|line| {
        let trimmed = line.trim();
        trimmed.is_empty()
          || trimmed.starts_with('#')
          || trimmed.starts_with(';')
          || !trimmed.starts_with(directive)
      })
      .collect::<Vec<_>>()
      .join("\n")
  }

  fn build_runtime_config(&self) -> String {
    let mut runtime_config = self.config.raw_config.clone();

    runtime_config = Self::strip_config_directive(&runtime_config, "redirect-gateway");
    runtime_config = Self::strip_config_directive(&runtime_config, "block-outside-dns");
    runtime_config = Self::strip_config_directive(&runtime_config, "dhcp-option");

    if !runtime_config.contains("pull-filter ignore \"redirect-gateway\"") {
      runtime_config.push_str("\npull-filter ignore \"redirect-gateway\"\n");
    }
    if !runtime_config.contains("pull-filter ignore \"block-outside-dns\"") {
      runtime_config.push_str("pull-filter ignore \"block-outside-dns\"\n");
    }
    if !runtime_config.contains("pull-filter ignore \"dhcp-option\"") {
      runtime_config.push_str("pull-filter ignore \"dhcp-option\"\n");
    }

    if !Self::has_config_directive(&runtime_config, "route 0.0.0.0") {
      runtime_config.push_str("\nroute 0.0.0.0 0.0.0.0 vpn_gateway 9999\n");
    }

    #[cfg(windows)]
    {
      if Self::has_config_directive(&runtime_config, "dev-node") {
        runtime_config = runtime_config
          .lines()
          .filter(|line| {
            let trimmed = line.trim();
            trimmed.is_empty()
              || trimmed.starts_with('#')
              || trimmed.starts_with(';')
              || !trimmed.starts_with("dev-node")
          })
          .collect::<Vec<_>>()
          .join("\n");
      }

      if !Self::has_config_directive(&runtime_config, "disable-dco") {
        runtime_config.push_str("\ndisable-dco\n");
      }

      if self.config.dev_type.starts_with("tun")
        && !Self::has_config_directive(&runtime_config, "windows-driver")
      {
        runtime_config.push_str("\nwindows-driver wintun\n");
      }
    }

    runtime_config
  }

  pub(crate) fn dependency_status() -> OpenVpnDependencyStatus {
    let Ok(openvpn_bin) = Self::find_openvpn_binary() else {
      return OpenVpnDependencyStatus {
        binary_found: false,
        missing_windows_adapter: false,
        dependency_check_failed: false,
      };
    };

    #[cfg(windows)]
    {
      match Self::windows_openvpn_has_adapter(&openvpn_bin) {
        Ok(has_adapter) => OpenVpnDependencyStatus {
          binary_found: true,
          missing_windows_adapter: !has_adapter,
          dependency_check_failed: false,
        },
        Err(_) => OpenVpnDependencyStatus {
          binary_found: true,
          missing_windows_adapter: false,
          dependency_check_failed: true,
        },
      }
    }

    #[cfg(not(windows))]
    {
      OpenVpnDependencyStatus {
        binary_found: true,
        missing_windows_adapter: false,
        dependency_check_failed: false,
      }
    }
  }

  pub(crate) fn find_openvpn_binary() -> Result<PathBuf, VpnError> {
    if let Ok(path) = std::env::var("DONUTBROWSER_OPENVPN_BIN") {
      let path = PathBuf::from(path);
      if path.exists() {
        return Ok(path);
      }

      return Err(VpnError::Connection(format!(
        "Configured OpenVPN binary does not exist: {}",
        path.display()
      )));
    }

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
      use std::os::windows::process::CommandExt;
      const CREATE_NO_WINDOW: u32 = 0x08000000;
      if let Ok(output) = Command::new("where")
        .arg("openvpn")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
      {
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

  fn openvpn_supports_management(openvpn_bin: &Path) -> bool {
    let mut command = Command::new(openvpn_bin);
    command.arg("--version");

    #[cfg(windows)]
    {
      use std::os::windows::process::CommandExt;
      const CREATE_NO_WINDOW: u32 = 0x08000000;
      command.creation_flags(CREATE_NO_WINDOW);
    }

    let Ok(output) = command.output() else {
      return true;
    };

    let version_text = format!(
      "{}{}",
      String::from_utf8_lossy(&output.stdout),
      String::from_utf8_lossy(&output.stderr)
    );

    !version_text.contains("enable_management=no")
  }

  #[cfg(windows)]
  pub(crate) fn windows_openvpn_has_adapter(openvpn_bin: &Path) -> Result<bool, VpnError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let output = Command::new(openvpn_bin)
      .arg("--show-adapters")
      .creation_flags(CREATE_NO_WINDOW)
      .output()
      .map_err(|e| VpnError::Connection(format!("Failed to inspect OpenVPN adapters: {e}")))?;

    let text = format!(
      "{}{}",
      String::from_utf8_lossy(&output.stdout),
      String::from_utf8_lossy(&output.stderr)
    );

    Ok(
      text
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with("Available adapters")),
    )
  }

  fn extract_vpn_ip_from_log(log_content: &str) -> Option<Ipv4Addr> {
    for line in log_content.lines() {
      if let Some(ip) = Self::extract_vpn_ip(line) {
        return Some(ip);
      }

      if let Some(position) = line.find("ifconfig ") {
        let after = &line[position + "ifconfig ".len()..];
        if let Some(ip_str) = after
          .split_whitespace()
          .next()
          .or_else(|| after.split(',').next())
        {
          if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
            if ip.is_private() && !ip.is_loopback() {
              return Some(ip);
            }
          }
        }
      }
    }

    None
  }

  async fn wait_for_openvpn_ready_via_management(
    child: &mut std::process::Child,
    mgmt_port: u16,
    log_path: &Path,
  ) -> Result<Option<Ipv4Addr>, VpnError> {
    let deadline =
      tokio::time::Instant::now() + tokio::time::Duration::from_secs(OPENVPN_CONNECT_TIMEOUT_SECS);

    let mgmt_stream = loop {
      if tokio::time::Instant::now() >= deadline {
        return Err(VpnError::Connection(format!(
          "Timed out connecting to OpenVPN management interface. Last OpenVPN output:\n{}",
          Self::read_log_tail(log_path, 20)
        )));
      }

      if let Ok(Some(status)) = child.try_wait() {
        return Err(VpnError::Connection(format!(
          "OpenVPN exited (status: {}) before the tunnel was established. Last output:\n{}",
          status,
          Self::read_log_tail(log_path, 20)
        )));
      }

      match TcpStream::connect(("127.0.0.1", mgmt_port)).await {
        Ok(stream) => break stream,
        Err(_) => tokio::time::sleep(tokio::time::Duration::from_millis(500)).await,
      }
    };

    let (mgmt_reader, mut mgmt_writer) = mgmt_stream.into_split();
    let _ = mgmt_writer.write_all(b"state on\nstate\n").await;

    let mut lines = BufReader::new(mgmt_reader).lines();
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.tick().await;

    let mut vpn_ip = None;

    loop {
      if tokio::time::Instant::now() >= deadline {
        return Err(VpnError::Connection(format!(
          "Timed out waiting for OpenVPN to reach CONNECTED state. Last OpenVPN output:\n{}",
          Self::read_log_tail(log_path, 20)
        )));
      }

      if let Ok(Some(status)) = child.try_wait() {
        return Err(VpnError::Connection(format!(
          "OpenVPN exited (status: {}) before connecting. Last output:\n{}",
          status,
          Self::read_log_tail(log_path, 20)
        )));
      }

      tokio::select! {
        line_result = lines.next_line() => {
          match line_result {
            Ok(Some(line)) => {
              if let Some(ip) = Self::extract_vpn_ip(&line) {
                vpn_ip = Some(ip);
              }

              if line.contains(",CONNECTED,") {
                break;
              }

              if line.contains("AUTH_FAILED") {
                return Err(VpnError::Connection(format!(
                  "OpenVPN authentication failed. Last output:\n{}",
                  Self::read_log_tail(log_path, 20)
                )));
              }

              if line.contains(",EXITING,") || line.contains(">FATAL:") {
                return Err(VpnError::Connection(format!(
                  "OpenVPN is exiting. Last output:\n{}",
                  Self::read_log_tail(log_path, 20)
                )));
              }
            }
            Ok(None) => {
              return Err(VpnError::Connection(format!(
                "OpenVPN management connection closed before CONNECTED state. Last output:\n{}",
                Self::read_log_tail(log_path, 20)
              )));
            }
            Err(_) => {}
          }
        }
        _ = interval.tick() => {
          let _ = mgmt_writer.write_all(b"state\n").await;

          let log_path = log_path.to_path_buf();
          let log_content = tokio::task::spawn_blocking(move || std::fs::read_to_string(log_path))
            .await
            .ok()
            .and_then(Result::ok);

          if let Some(content) = log_content {
            if Self::log_indicates_connected(&content) {
              break;
            }
          }
        }
      }
    }

    if vpn_ip.is_none() {
      if let Ok(log_content) = std::fs::read_to_string(log_path) {
        vpn_ip = Self::extract_vpn_ip_from_log(&log_content);
      }
    }

    Ok(vpn_ip)
  }

  async fn wait_for_openvpn_ready_via_log(
    child: &mut std::process::Child,
    log_path: &Path,
  ) -> Result<Option<Ipv4Addr>, VpnError> {
    let deadline =
      tokio::time::Instant::now() + tokio::time::Duration::from_secs(OPENVPN_CONNECT_TIMEOUT_SECS);

    loop {
      if tokio::time::Instant::now() >= deadline {
        return Err(VpnError::Connection(format!(
          "Timed out waiting for OpenVPN to connect. Last OpenVPN output:\n{}",
          Self::read_log_tail(log_path, 40)
        )));
      }

      if let Ok(Some(status)) = child.try_wait() {
        return Err(VpnError::Connection(format!(
          "OpenVPN exited (status: {}) before connecting. Last output:\n{}",
          status,
          Self::read_log_tail(log_path, 40)
        )));
      }

      let log_path_buf = log_path.to_path_buf();
      let log_content = tokio::task::spawn_blocking(move || std::fs::read_to_string(log_path_buf))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();

      if Self::log_indicates_connected(&log_content) {
        return Ok(Self::extract_vpn_ip_from_log(&log_content));
      }

      if Self::log_indicates_failure(&log_content) {
        return Err(VpnError::Connection(format!(
          "OpenVPN reported a fatal error while connecting. Last output:\n{}",
          Self::read_log_tail(log_path, 40)
        )));
      }

      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
  }

  async fn connect_target(
    target: SocksTarget,
    vpn_bind_ip: Ipv4Addr,
  ) -> Result<(TcpStream, SocketAddr), Box<dyn std::error::Error + Send + Sync>> {
    let mut addresses = match target {
      SocksTarget::Address(addr) => vec![addr],
      SocksTarget::Domain(host, port) => {
        let mut resolved = lookup_host((host.as_str(), port))
          .await?
          .collect::<Vec<_>>();
        resolved.sort_by_key(|addr| if addr.is_ipv4() { 0 } else { 1 });
        resolved
      }
    };

    if addresses.is_empty() {
      return Err("No addresses resolved for SOCKS5 target".into());
    }

    let mut last_error = None;

    for address in addresses.drain(..) {
      let socket = if address.is_ipv4() {
        let socket = TcpSocket::new_v4()?;
        if !vpn_bind_ip.is_unspecified() {
          socket.bind(SocketAddr::new(IpAddr::V4(vpn_bind_ip), 0))?;
        }
        socket
      } else {
        TcpSocket::new_v6()?
      };

      match socket.connect(address).await {
        Ok(stream) => return Ok((stream, address)),
        Err(error) => last_error = Some(error),
      }
    }

    Err(
      last_error
        .map(|error| error.into())
        .unwrap_or_else(|| "Failed to connect to any resolved SOCKS5 target".into()),
    )
  }

  pub async fn run(self, config_id: String) -> Result<(), VpnError> {
    let openvpn_bin = Self::find_openvpn_binary()?;
    let supports_management = Self::openvpn_supports_management(&openvpn_bin);

    #[cfg(windows)]
    if !Self::windows_openvpn_has_adapter(&openvpn_bin)? {
      return Err(VpnError::Connection(
        "OpenVPN requires a TAP/Wintun/ovpn-dco adapter on Windows, but none were found. Install or provision an adapter before connecting.".to_string(),
      ));
    }

    let config_path = std::env::temp_dir().join(format!("openvpn_{}.ovpn", config_id));
    std::fs::write(&config_path, self.build_runtime_config()).map_err(VpnError::Io)?;

    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let _ = std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600));
    }

    let mgmt_port = if supports_management {
      let mgmt_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| VpnError::Connection(format!("Failed to bind management port: {e}")))?;
      let port = mgmt_listener
        .local_addr()
        .map_err(|e| VpnError::Connection(format!("Failed to get management port: {e}")))?
        .port();
      drop(mgmt_listener);
      Some(port)
    } else {
      log::info!(
        "[vpn-worker] OpenVPN build does not support management; using log-based readiness"
      );
      None
    };

    let openvpn_log_path = std::env::temp_dir().join(format!("openvpn-{}.log", config_id));
    let log_file = std::fs::OpenOptions::new()
      .create(true)
      .write(true)
      .truncate(true)
      .open(&openvpn_log_path)
      .map_err(VpnError::Io)?;

    let mut cmd = Command::new(&openvpn_bin);
    cmd.arg("--config").arg(&config_path);
    if let Some(mgmt_port) = mgmt_port {
      cmd
        .arg("--management")
        .arg("127.0.0.1")
        .arg(mgmt_port.to_string());
    }
    cmd
      .arg("--verb")
      .arg("3")
      .stdout(
        log_file
          .try_clone()
          .map(Stdio::from)
          .map_err(VpnError::Io)?,
      )
      .stderr(Stdio::from(log_file));

    #[cfg(windows)]
    {
      use std::os::windows::process::CommandExt;
      const CREATE_NO_WINDOW: u32 = 0x08000000;

      cmd.arg("--disable-dco");
      if self.config.dev_type.starts_with("tun") {
        cmd.arg("--windows-driver").arg("wintun");
      }
      cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
      .spawn()
      .map_err(|e| VpnError::Connection(format!("Failed to start OpenVPN: {e}")))?;

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    match child.try_wait() {
      Ok(Some(status)) => {
        let _ = std::fs::remove_file(&config_path);
        return Err(VpnError::Connection(format!(
          "OpenVPN exited immediately (status: {}). Last output:\n{}",
          status,
          Self::read_log_tail(&openvpn_log_path, 20)
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

    let vpn_bind_ip = if let Some(mgmt_port) = mgmt_port {
      Self::wait_for_openvpn_ready_via_management(&mut child, mgmt_port, &openvpn_log_path).await?
    } else {
      Self::wait_for_openvpn_ready_via_log(&mut child, &openvpn_log_path).await?
    }
    .unwrap_or(Ipv4Addr::UNSPECIFIED);
    let vpn_bind_ip = Arc::new(vpn_bind_ip);

    let listener = TcpListener::bind(("127.0.0.1", self.port))
      .await
      .map_err(|e| VpnError::Connection(format!("Failed to bind SOCKS5: {e}")))?;

    let actual_port = listener
      .local_addr()
      .map_err(|e| VpnError::Connection(format!("Failed to get local addr: {e}")))?
      .port();

    if let Some(mut worker_config) = crate::vpn_worker_storage::get_vpn_worker_config(&config_id) {
      worker_config.local_port = Some(actual_port);
      worker_config.local_url = Some(format!("socks5://127.0.0.1:{}", actual_port));
      let _ = crate::vpn_worker_storage::save_vpn_worker_config(&worker_config);
    }

    log::info!(
      "[vpn-worker] OpenVPN SOCKS5 server listening on 127.0.0.1:{}",
      actual_port
    );

    loop {
      match listener.accept().await {
        Ok((client, _)) => {
          let bind_ip = vpn_bind_ip.clone();
          tokio::spawn(async move {
            let _ = Self::handle_socks5_client(client, bind_ip).await;
          });
        }
        Err(error) => {
          log::warn!("[vpn-worker] Accept error: {error}");
        }
      }
    }
  }

  async fn handle_socks5_client(
    mut client: TcpStream,
    vpn_bind_ip: Arc<Ipv4Addr>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut greeting = [0u8; 2];
    if let Err(error) = client.read_exact(&mut greeting).await {
      if error.kind() != std::io::ErrorKind::UnexpectedEof {
        log::debug!("[socks5] Failed to read greeting header: {}", error);
      }
      return Ok(());
    }

    if greeting[0] != 0x05 {
      return Ok(());
    }

    let mut methods = vec![0u8; greeting[1] as usize];
    if let Err(error) = client.read_exact(&mut methods).await {
      if error.kind() != std::io::ErrorKind::UnexpectedEof {
        log::debug!("[socks5] Failed to read methods list: {}", error);
      }
      return Ok(());
    }

    client.write_all(&[0x05, 0x00]).await?;

    let mut request_header = [0u8; 4];
    if let Err(error) = client.read_exact(&mut request_header).await {
      if error.kind() != std::io::ErrorKind::UnexpectedEof {
        log::debug!("[socks5] Failed to read request header: {}", error);
      }
      return Ok(());
    }

    if request_header[0] != 0x05 {
      return Ok(());
    }

    if request_header[1] != 0x01 {
      let _ = client
        .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await;
      return Ok(());
    }

    let target = match request_header[3] {
      0x01 => {
        let mut addr_port = [0u8; 6];
        client.read_exact(&mut addr_port).await?;
        SocksTarget::Address(SocketAddr::new(
          IpAddr::V4(Ipv4Addr::new(
            addr_port[0],
            addr_port[1],
            addr_port[2],
            addr_port[3],
          )),
          u16::from_be_bytes([addr_port[4], addr_port[5]]),
        ))
      }
      0x03 => {
        let mut len = [0u8; 1];
        client.read_exact(&mut len).await?;
        if len[0] == 0 {
          return Ok(());
        }

        let mut domain = vec![0u8; len[0] as usize];
        client.read_exact(&mut domain).await?;

        let mut port = [0u8; 2];
        client.read_exact(&mut port).await?;

        SocksTarget::Domain(
          String::from_utf8_lossy(&domain).to_string(),
          u16::from_be_bytes(port),
        )
      }
      0x04 => {
        let mut addr_port = [0u8; 18];
        client.read_exact(&mut addr_port).await?;

        let mut octets = [0u8; 16];
        octets.copy_from_slice(&addr_port[..16]);

        SocksTarget::Address(SocketAddr::new(
          IpAddr::V6(std::net::Ipv6Addr::from(octets)),
          u16::from_be_bytes([addr_port[16], addr_port[17]]),
        ))
      }
      _ => {
        let _ = client
          .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
          .await;
        return Ok(());
      }
    };

    match Self::connect_target(target, *vpn_bind_ip).await {
      Ok((upstream, _address)) => {
        client
          .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
          .await?;

        let (mut client_read, mut client_write) = client.into_split();
        let (mut upstream_read, mut upstream_write) = upstream.into_split();

        let client_to_upstream = tokio::io::copy(&mut client_read, &mut upstream_write);
        let upstream_to_client = tokio::io::copy(&mut upstream_read, &mut client_write);
        let _ = tokio::try_join!(client_to_upstream, upstream_to_client)?;
      }
      Err(error) => {
        log::debug!(
          "[socks5] Failed to connect through OpenVPN tunnel: {}",
          error
        );
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

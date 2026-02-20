use super::config::{VpnError, WireGuardConfig};
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use smoltcp::iface::{Config as IfaceConfig, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer};
use smoltcp::time::Instant as SmolInstant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, Ipv4Address};
use std::collections::VecDeque;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};

const SMOLTCP_TCP_RX_BUF: usize = 65536;
const SMOLTCP_TCP_TX_BUF: usize = 65536;

struct WgDevice {
  tunn: Arc<Mutex<Box<Tunn>>>,
  udp_socket: Arc<UdpSocket>,
  peer_addr: SocketAddr,
  rx_queue: VecDeque<Vec<u8>>,
  tx_queue: VecDeque<Vec<u8>>,
}

impl WgDevice {
  fn pump_wg_to_rx(&mut self) {
    let mut recv_buf = vec![0u8; 2048];
    loop {
      match self.udp_socket.recv_from(&mut recv_buf) {
        Ok((len, _)) => {
          let mut dst = vec![0u8; 2048];
          let mut tunn = self.tunn.lock().unwrap();
          let result = tunn.decapsulate(None, &recv_buf[..len], &mut dst);
          match result {
            TunnResult::WriteToTunnelV4(data, _) | TunnResult::WriteToTunnelV6(data, _) => {
              self.rx_queue.push_back(data.to_vec());
            }
            TunnResult::WriteToNetwork(response) => {
              let _ = self.udp_socket.send_to(response, self.peer_addr);
            }
            _ => {}
          }
        }
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
        Err(_) => break,
      }
    }
  }

  fn flush_tx_queue(&mut self) {
    while let Some(ip_packet) = self.tx_queue.pop_front() {
      let mut dst = vec![0u8; ip_packet.len() + 256];
      let mut tunn = self.tunn.lock().unwrap();
      let result = tunn.encapsulate(&ip_packet, &mut dst);
      if let TunnResult::WriteToNetwork(packet) = result {
        let _ = self.udp_socket.send_to(packet, self.peer_addr);
      }
    }
  }

  fn tick_timers(&mut self) {
    let mut dst = vec![0u8; 2048];
    let mut tunn = self.tunn.lock().unwrap();
    let result = tunn.update_timers(&mut dst);
    if let TunnResult::WriteToNetwork(packet) = result {
      let _ = self.udp_socket.send_to(packet, self.peer_addr);
    }
  }
}

struct WgRxToken {
  data: Vec<u8>,
}

impl RxToken for WgRxToken {
  fn consume<R, F>(mut self, f: F) -> R
  where
    F: FnOnce(&mut [u8]) -> R,
  {
    f(&mut self.data)
  }
}

struct WgTxToken<'a> {
  tx_queue: &'a mut VecDeque<Vec<u8>>,
}

impl<'a> TxToken for WgTxToken<'a> {
  fn consume<R, F>(self, len: usize, f: F) -> R
  where
    F: FnOnce(&mut [u8]) -> R,
  {
    let mut buf = vec![0u8; len];
    let result = f(&mut buf);
    self.tx_queue.push_back(buf);
    result
  }
}

impl Device for WgDevice {
  type RxToken<'a> = WgRxToken;
  type TxToken<'a> = WgTxToken<'a>;

  fn receive(&mut self, _timestamp: SmolInstant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
    if let Some(data) = self.rx_queue.pop_front() {
      Some((
        WgRxToken { data },
        WgTxToken {
          tx_queue: &mut self.tx_queue,
        },
      ))
    } else {
      None
    }
  }

  fn transmit(&mut self, _timestamp: SmolInstant) -> Option<Self::TxToken<'_>> {
    Some(WgTxToken {
      tx_queue: &mut self.tx_queue,
    })
  }

  fn capabilities(&self) -> DeviceCapabilities {
    let mut caps = DeviceCapabilities::default();
    caps.medium = Medium::Ip;
    caps.max_transmission_unit = 1420;
    caps
  }
}

fn parse_key(key: &str) -> Result<[u8; 32], VpnError> {
  let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, key)
    .map_err(|e| VpnError::InvalidWireGuard(format!("Invalid key encoding: {e}")))?;
  if decoded.len() != 32 {
    return Err(VpnError::InvalidWireGuard(format!(
      "Invalid key length: {} (expected 32)",
      decoded.len()
    )));
  }
  let mut key_bytes = [0u8; 32];
  key_bytes.copy_from_slice(&decoded);
  Ok(key_bytes)
}

fn parse_cidr_address(addr: &str) -> Result<(IpCidr, IpAddress), VpnError> {
  let first_addr = addr.split(',').next().unwrap_or(addr).trim();

  let parts: Vec<&str> = first_addr.split('/').collect();
  let ip_str = parts[0];
  let prefix = if parts.len() > 1 {
    parts[1]
      .parse::<u8>()
      .map_err(|_| VpnError::InvalidWireGuard(format!("Invalid prefix length: {}", parts[1])))?
  } else {
    32
  };

  let ip: std::net::IpAddr = ip_str
    .parse()
    .map_err(|_| VpnError::InvalidWireGuard(format!("Invalid IP address: {ip_str}")))?;

  match ip {
    std::net::IpAddr::V4(v4) => {
      let smol_ip = Ipv4Address::new(
        v4.octets()[0],
        v4.octets()[1],
        v4.octets()[2],
        v4.octets()[3],
      );
      Ok((
        IpCidr::new(IpAddress::Ipv4(smol_ip), prefix),
        IpAddress::Ipv4(smol_ip),
      ))
    }
    std::net::IpAddr::V6(v6) => {
      let smol_ip = smoltcp::wire::Ipv6Address::from_bytes(&v6.octets());
      Ok((
        IpCidr::new(IpAddress::Ipv6(smol_ip), prefix),
        IpAddress::Ipv6(smol_ip),
      ))
    }
  }
}

pub struct WireGuardSocks5Server {
  config: WireGuardConfig,
  port: u16,
}

impl WireGuardSocks5Server {
  pub fn new(config: WireGuardConfig, port: u16) -> Self {
    Self { config, port }
  }

  fn create_tunnel(&self) -> Result<Box<Tunn>, VpnError> {
    let private_key_bytes = parse_key(&self.config.private_key)?;
    let static_private = StaticSecret::from(private_key_bytes);

    let peer_public_bytes = parse_key(&self.config.peer_public_key)?;
    let peer_public = PublicKey::from(peer_public_bytes);

    let preshared_key = if let Some(ref psk) = self.config.preshared_key {
      Some(parse_key(psk)?)
    } else {
      None
    };

    Ok(Box::new(Tunn::new(
      static_private,
      peer_public,
      preshared_key,
      self.config.persistent_keepalive,
      0,
      None,
    )))
  }

  fn resolve_endpoint(&self) -> Result<SocketAddr, VpnError> {
    self
      .config
      .peer_endpoint
      .to_socket_addrs()
      .map_err(|e| {
        VpnError::Connection(format!(
          "Failed to resolve endpoint '{}': {e}",
          self.config.peer_endpoint
        ))
      })?
      .next()
      .ok_or_else(|| {
        VpnError::Connection(format!(
          "No addresses found for endpoint: {}",
          self.config.peer_endpoint
        ))
      })
  }

  fn do_handshake(
    tunn: &mut Tunn,
    socket: &UdpSocket,
    peer_addr: SocketAddr,
  ) -> Result<(), VpnError> {
    let mut dst = vec![0u8; 2048];
    let result = tunn.format_handshake_initiation(&mut dst, false);

    match result {
      TunnResult::WriteToNetwork(packet) => {
        socket
          .send_to(packet, peer_addr)
          .map_err(|e| VpnError::Connection(format!("Failed to send handshake: {e}")))?;
      }
      TunnResult::Err(e) => {
        return Err(VpnError::Tunnel(format!(
          "Handshake initiation failed: {e:?}"
        )));
      }
      _ => {}
    }

    socket
      .set_read_timeout(Some(std::time::Duration::from_secs(10)))
      .map_err(|e| VpnError::Connection(format!("Failed to set timeout: {e}")))?;

    let mut recv_buf = vec![0u8; 2048];
    match socket.recv_from(&mut recv_buf) {
      Ok((len, _)) => {
        let result = tunn.decapsulate(None, &recv_buf[..len], &mut dst);
        match result {
          TunnResult::WriteToNetwork(response) => {
            socket
              .send_to(response, peer_addr)
              .map_err(|e| VpnError::Connection(format!("Failed to send response: {e}")))?;
          }
          TunnResult::Done => {}
          TunnResult::Err(e) => {
            return Err(VpnError::Tunnel(format!(
              "Handshake response failed: {e:?}"
            )));
          }
          _ => {}
        }
      }
      Err(e) => {
        return Err(VpnError::Connection(format!(
          "Handshake timeout or error: {e}"
        )));
      }
    }

    socket
      .set_read_timeout(None)
      .map_err(|e| VpnError::Connection(format!("Failed to clear timeout: {e}")))?;

    Ok(())
  }

  pub async fn run(self, config_id: String) -> Result<(), VpnError> {
    let peer_addr = self.resolve_endpoint()?;
    let mut tunn = self.create_tunnel()?;

    let udp_socket = UdpSocket::bind("0.0.0.0:0")
      .map_err(|e| VpnError::Connection(format!("Failed to create UDP socket: {e}")))?;

    Self::do_handshake(&mut tunn, &udp_socket, peer_addr)?;

    udp_socket
      .set_nonblocking(true)
      .map_err(|e| VpnError::Connection(format!("Failed to set non-blocking: {e}")))?;

    log::info!("[vpn-worker] WireGuard handshake completed");

    let (cidr, local_ip) = parse_cidr_address(&self.config.address)?;

    let tunn_arc = Arc::new(Mutex::new(tunn));
    let udp_arc = Arc::new(udp_socket);

    let mut device = WgDevice {
      tunn: tunn_arc.clone(),
      udp_socket: udp_arc.clone(),
      peer_addr,
      rx_queue: VecDeque::new(),
      tx_queue: VecDeque::new(),
    };

    let iface_config = IfaceConfig::new(HardwareAddress::Ip);
    let mut iface = Interface::new(iface_config, &mut device, SmolInstant::now());
    iface.update_ip_addrs(|addrs| {
      let _ = addrs.push(cidr);
    });

    // Set default gateway
    match local_ip {
      IpAddress::Ipv4(v4) => {
        let octets = v4.as_bytes();
        let gw = Ipv4Address::new(octets[0], octets[1], octets[2], 1);
        iface
          .routes_mut()
          .add_default_ipv4_route(gw)
          .map_err(|e| VpnError::Tunnel(format!("Failed to add default route: {e}")))?;
      }
      IpAddress::Ipv6(_) => {
        // IPv6 routing not yet implemented
      }
    }

    let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port))
      .await
      .map_err(|e| VpnError::Connection(format!("Failed to bind SOCKS5 listener: {e}")))?;

    let actual_port = listener
      .local_addr()
      .map_err(|e| VpnError::Connection(format!("Failed to get local addr: {e}")))?
      .port();

    // Update config with actual port and local_url
    if let Some(mut wc) = crate::vpn_worker_storage::get_vpn_worker_config(&config_id) {
      wc.local_port = Some(actual_port);
      wc.local_url = Some(format!("socks5://127.0.0.1:{}", actual_port));
      let _ = crate::vpn_worker_storage::save_vpn_worker_config(&wc);
    }

    log::info!(
      "[vpn-worker] SOCKS5 server listening on 127.0.0.1:{}",
      actual_port
    );

    let mut sockets = SocketSet::new(vec![]);

    struct Connection {
      smol_handle: SocketHandle,
      tcp_stream: TcpStream,
      socks_done: bool,
      read_buf: Vec<u8>,
      dest_addr: Option<SocketAddr>,
    }

    let mut connections: Vec<Connection> = Vec::new();
    let mut timer_counter: u64 = 0;

    loop {
      // Accept new SOCKS5 connections (non-blocking via short timeout)
      if let Ok(Ok((stream, _addr))) =
        tokio::time::timeout(tokio::time::Duration::from_millis(1), listener.accept()).await
      {
        let tcp_rx = SocketBuffer::new(vec![0u8; SMOLTCP_TCP_RX_BUF]);
        let tcp_tx = SocketBuffer::new(vec![0u8; SMOLTCP_TCP_TX_BUF]);
        let tcp_socket = TcpSocket::new(tcp_rx, tcp_tx);
        let handle = sockets.add(tcp_socket);

        connections.push(Connection {
          smol_handle: handle,
          tcp_stream: stream,
          socks_done: false,
          read_buf: Vec::new(),
          dest_addr: None,
        });
      }

      // Pump WireGuard packets into smoltcp rx queue
      device.pump_wg_to_rx();

      // Poll the smoltcp interface
      let timestamp = SmolInstant::now();
      let _changed = iface.poll(timestamp, &mut device, &mut sockets);

      // Flush encrypted packets out through WireGuard
      device.flush_tx_queue();

      // Process each connection
      let mut completed = Vec::new();
      for (idx, conn) in connections.iter_mut().enumerate() {
        if !conn.socks_done {
          // Handle SOCKS5 handshake
          let mut buf = [0u8; 512];
          match conn.tcp_stream.try_read(&mut buf) {
            Ok(0) => {
              completed.push(idx);
              continue;
            }
            Ok(n) => {
              conn.read_buf.extend_from_slice(&buf[..n]);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
              completed.push(idx);
              continue;
            }
          }

          if conn.dest_addr.is_none() && conn.read_buf.len() >= 3 {
            // SOCKS5 greeting: version, nmethods, methods
            if conn.read_buf[0] != 0x05 {
              completed.push(idx);
              continue;
            }
            // Reply: no auth required
            let _ = conn.tcp_stream.try_write(&[0x05, 0x00]);
            let nmethods = conn.read_buf[1] as usize;
            conn.read_buf.drain(..2 + nmethods);
          }

          if conn.dest_addr.is_none() && conn.read_buf.len() >= 10 {
            // SOCKS5 connect request
            if conn.read_buf[0] != 0x05 || conn.read_buf[1] != 0x01 {
              completed.push(idx);
              continue;
            }

            let (addr, addr_len) = match conn.read_buf[3] {
              0x01 => {
                // IPv4
                if conn.read_buf.len() < 10 {
                  continue;
                }
                let ip = std::net::Ipv4Addr::new(
                  conn.read_buf[4],
                  conn.read_buf[5],
                  conn.read_buf[6],
                  conn.read_buf[7],
                );
                let port = u16::from_be_bytes([conn.read_buf[8], conn.read_buf[9]]);
                (SocketAddr::new(std::net::IpAddr::V4(ip), port), 10)
              }
              0x03 => {
                // Domain name
                let domain_len = conn.read_buf[4] as usize;
                let needed = 4 + 1 + domain_len + 2;
                if conn.read_buf.len() < needed {
                  continue;
                }
                let domain = String::from_utf8_lossy(&conn.read_buf[5..5 + domain_len]).to_string();
                let port_start = 5 + domain_len;
                let port =
                  u16::from_be_bytes([conn.read_buf[port_start], conn.read_buf[port_start + 1]]);
                // Resolve domain
                match format!("{}:{}", domain, port).to_socket_addrs() {
                  Ok(mut addrs) => {
                    if let Some(addr) = addrs.next() {
                      (addr, needed)
                    } else {
                      // Send SOCKS5 error: host unreachable
                      let _ = conn
                        .tcp_stream
                        .try_write(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
                      completed.push(idx);
                      continue;
                    }
                  }
                  Err(_) => {
                    let _ = conn
                      .tcp_stream
                      .try_write(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
                    completed.push(idx);
                    continue;
                  }
                }
              }
              0x04 => {
                // IPv6
                if conn.read_buf.len() < 22 {
                  continue;
                }
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&conn.read_buf[4..20]);
                let ip = std::net::Ipv6Addr::from(octets);
                let port = u16::from_be_bytes([conn.read_buf[20], conn.read_buf[21]]);
                (SocketAddr::new(std::net::IpAddr::V6(ip), port), 22)
              }
              _ => {
                completed.push(idx);
                continue;
              }
            };

            conn.read_buf.drain(..addr_len);
            conn.dest_addr = Some(addr);

            // Open smoltcp TCP socket to the destination
            let socket = sockets.get_mut::<TcpSocket>(conn.smol_handle);
            let smol_addr = match addr.ip() {
              std::net::IpAddr::V4(v4) => {
                let o = v4.octets();
                IpAddress::Ipv4(Ipv4Address::new(o[0], o[1], o[2], o[3]))
              }
              std::net::IpAddr::V6(v6) => {
                IpAddress::Ipv6(smoltcp::wire::Ipv6Address::from_bytes(&v6.octets()))
              }
            };

            let local_port = 10000 + (rand::random::<u16>() % 50000);
            if socket
              .connect(iface.context(), (smol_addr, addr.port()), local_port)
              .is_err()
            {
              let _ = conn
                .tcp_stream
                .try_write(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
              completed.push(idx);
              continue;
            }

            // Send SOCKS5 success reply
            let _ = conn.tcp_stream.try_write(&[
              0x05,
              0x00,
              0x00,
              0x01,
              127,
              0,
              0,
              1,
              (actual_port >> 8) as u8,
              (actual_port & 0xff) as u8,
            ]);
            conn.socks_done = true;
          }
        } else {
          // Data relay between SOCKS5 client and smoltcp socket
          let socket = sockets.get_mut::<TcpSocket>(conn.smol_handle);

          // Client → smoltcp
          let mut buf = [0u8; 4096];
          match conn.tcp_stream.try_read(&mut buf) {
            Ok(0) => {
              socket.close();
              completed.push(idx);
              continue;
            }
            Ok(n) => {
              if socket.can_send() {
                let _ = socket.send_slice(&buf[..n]);
              }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
              socket.close();
              completed.push(idx);
              continue;
            }
          }

          // smoltcp → Client
          if socket.can_recv() {
            match socket.recv(|data| (data.len(), data.to_vec())) {
              Ok(data) if !data.is_empty() => {
                if conn.tcp_stream.try_write(&data).is_err() {
                  socket.close();
                  completed.push(idx);
                  continue;
                }
              }
              _ => {}
            }
          }

          // Check if smoltcp socket closed
          if !socket.is_open() && !socket.is_active() {
            completed.push(idx);
          }
        }
      }

      // Remove completed connections (in reverse order)
      completed.sort_unstable();
      completed.dedup();
      for idx in completed.into_iter().rev() {
        let conn = connections.remove(idx);
        sockets.remove(conn.smol_handle);
      }

      // Timer ticks for WireGuard keepalives
      timer_counter += 1;
      if timer_counter.is_multiple_of(500) {
        device.tick_timers();
      }

      // Small sleep to avoid busy-spinning
      tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_parse_cidr_ipv4() {
    let (cidr, ip) = parse_cidr_address("10.0.0.2/24").unwrap();
    assert_eq!(cidr.prefix_len(), 24);
    assert_eq!(ip, IpAddress::Ipv4(Ipv4Address::new(10, 0, 0, 2)));
  }

  #[test]
  fn test_parse_cidr_no_prefix() {
    let (cidr, _) = parse_cidr_address("10.0.0.2").unwrap();
    assert_eq!(cidr.prefix_len(), 32);
  }

  #[test]
  fn test_parse_cidr_multi_address() {
    let (_, ip) = parse_cidr_address("10.0.0.2/24, fd00::2/128").unwrap();
    assert_eq!(ip, IpAddress::Ipv4(Ipv4Address::new(10, 0, 0, 2)));
  }

  #[test]
  fn test_parse_key_valid() {
    let key = "YEocP0e2o1WT5GlvBvQzVF7EeR6z9aCk+ZdZ5NKEuXA=";
    assert!(parse_key(key).is_ok());
  }

  #[test]
  fn test_parse_key_invalid() {
    assert!(parse_key("not-valid").is_err());
  }
}

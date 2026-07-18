//! Local SOCKS5 server served to the browser (Wayfern/Chromium).
//!
//! The HTTP front-end (`proxy_server::handle_proxy_connection`) can only tunnel
//! TCP, so QUIC and WebRTC — which are UDP — would be forced direct and leak the
//! real IP. Serving SOCKS5 instead lets Chromium proxy UDP via SOCKS5 UDP
//! ASSOCIATE (RFC 1928). TCP CONNECT reuses the exact same upstream-dial and
//! tunnel code as the HTTP path, so every upstream type (direct, HTTP/HTTPS
//! CONNECT, SOCKS4/5, Shadowsocks) behaves identically.
//!
//! UDP ASSOCIATE is leak-safe by construction: UDP is only relayed where it
//! cannot expose the host IP — directly when there is no upstream proxy, or
//! tunneled through a UDP-capable SOCKS5 upstream. For upstreams that cannot
//! carry UDP (HTTP/HTTPS/SOCKS4/Shadowsocks, or a SOCKS5 upstream that refuses
//! the association) the request is refused, so Chromium falls back to proxied
//! TCP rather than sending UDP from the real IP.
//!
//! Both commands enforce the same DNS blocklist and feed the same traffic
//! tracker as the HTTP front-end: CONNECT via `handle_connect`, and every UDP
//! datagram via [`UdpRelayContext`]. Without that, QUIC and WebRTC would be an
//! unfiltered, unmetered side channel around the TCP filter.

use crate::proxy_server::{
  connect_to_target_via_upstream, tunnel_streams, BlocklistMatcher, BypassMatcher,
};
use crate::traffic_stats::{get_traffic_tracker, LiveTrafficTracker};
use async_socks5::{AddrKind, Auth, SocksDatagram};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use url::Url;

// SOCKS5 reply codes (RFC 1928 §6).
const REP_SUCCEEDED: u8 = 0x00;
const REP_GENERAL_FAILURE: u8 = 0x01;
const REP_NOT_ALLOWED: u8 = 0x02;
const REP_COMMAND_NOT_SUPPORTED: u8 = 0x07;

// SOCKS5 commands (RFC 1928 §4).
const CMD_CONNECT: u8 = 0x01;
const CMD_UDP_ASSOCIATE: u8 = 0x03;

// Max UDP datagram payload; sized for a full 64 KiB datagram plus header slack.
const UDP_BUF: usize = 65_536;

// How often per-domain UDP byte totals are pushed into the traffic tracker.
// Datagram relay is a per-packet path, so totals accumulate locally and flush
// on this interval rather than taking the tracker's domain write lock per
// packet. Global byte counters are plain atomics and update inline, so live
// bandwidth stays real-time exactly as it does for TCP tunnels.
const UDP_STATS_FLUSH_INTERVAL: Duration = Duration::from_secs(2);

// Cap on remembered destination->host flows per association. A long-lived
// association fanning out to many peers must not grow this map without bound.
const UDP_FLOW_MAP_MAX: usize = 1024;

/// Per-association filtering and traffic accounting for the UDP relay loops.
///
/// UDP has no per-datagram error channel (RFC 1928 §7 has no reply code), so a
/// blocked destination is dropped silently — which is what the browser sees for
/// any unreachable UDP peer, and makes it fall back to proxied TCP.
struct UdpRelayContext {
  blocklist_matcher: BlocklistMatcher,
  tracker: Option<Arc<LiveTrafficTracker>>,
  /// Pending per-domain (sent, received) deltas awaiting flush.
  pending: HashMap<String, (u64, u64)>,
  /// Destinations already counted as a request, so each is recorded once.
  seen: std::collections::HashSet<String>,
  /// Maps a peer key back to the destination host we sent to, so reply bytes
  /// are attributed to the domain rather than to a bare IP.
  flow: HashMap<String, String>,
  last_flush: Instant,
}

impl UdpRelayContext {
  fn new(blocklist_matcher: BlocklistMatcher) -> Self {
    Self {
      blocklist_matcher,
      tracker: get_traffic_tracker(),
      pending: HashMap::new(),
      seen: std::collections::HashSet::new(),
      flow: HashMap::new(),
      last_flush: Instant::now(),
    }
  }

  /// Apply the DNS blocklist to a datagram destination. Mirrors the TCP path
  /// exactly: the host string is matched whether it is a domain or an IP
  /// literal, so allowlist mode rejects unlisted IP destinations here too.
  fn is_blocked(&self, host: &str) -> bool {
    self.blocklist_matcher.is_blocked(host)
  }

  /// Account a datagram relayed browser -> destination.
  fn record_sent(&mut self, host: &str, peer_key: String, bytes: u64) {
    let Some(tracker) = &self.tracker else {
      return;
    };
    tracker.add_bytes_sent(bytes);
    if self.seen.insert(host.to_string()) {
      // First datagram to this destination: count it once so UDP peers show up
      // in the domain list the way CONNECT targets do.
      tracker.record_request(host, 0, 0);
    }
    if self.flow.len() < UDP_FLOW_MAP_MAX {
      self.flow.insert(peer_key, host.to_string());
    }
    self.pending.entry(host.to_string()).or_insert((0, 0)).0 += bytes;
    self.maybe_flush();
  }

  /// Account a datagram relayed destination -> browser. `peer_key` is looked up
  /// against the flows recorded by `record_sent`; an unmatched reply (an
  /// upstream that reports a different address than we addressed) is attributed
  /// to `fallback_host`.
  fn record_received(&mut self, peer_key: &str, fallback_host: &str, bytes: u64) {
    let Some(tracker) = &self.tracker else {
      return;
    };
    tracker.add_bytes_received(bytes);
    let host = self
      .flow
      .get(peer_key)
      .cloned()
      .unwrap_or_else(|| fallback_host.to_string());
    self.pending.entry(host).or_insert((0, 0)).1 += bytes;
    self.maybe_flush();
  }

  fn maybe_flush(&mut self) {
    if self.last_flush.elapsed() >= UDP_STATS_FLUSH_INTERVAL {
      self.flush();
    }
  }

  /// Push accumulated per-domain totals into the tracker.
  fn flush(&mut self) {
    let Some(tracker) = &self.tracker else {
      self.pending.clear();
      return;
    };
    for (domain, (sent, recv)) in self.pending.drain() {
      tracker.update_domain_bytes(&domain, sent, recv);
    }
    self.last_flush = Instant::now();
  }
}

/// Host portion of a UDP destination, for blocklist matching. An IP literal is
/// rendered without its port so it matches the same way a CONNECT host does.
fn addrkind_host(addr: &AddrKind) -> String {
  match addr {
    AddrKind::Ip(s) => s.ip().to_string(),
    AddrKind::Domain(domain, _) => domain.clone(),
  }
}

/// Stable key identifying a UDP peer, used to tie replies back to the
/// destination host they belong to.
fn addrkind_key(addr: &AddrKind) -> String {
  match addr {
    AddrKind::Ip(s) => s.to_string(),
    AddrKind::Domain(domain, port) => format!("{domain}:{port}"),
  }
}

/// How a UDP ASSOCIATE request must be served for a given upstream so the real
/// IP never leaks.
#[derive(Debug, PartialEq, Eq)]
enum UdpMode {
  /// No upstream proxy: relay UDP directly (the host IP is the profile's IP,
  /// so there is nothing to hide).
  Direct,
  /// SOCKS5 upstream: attempt SOCKS5 UDP ASSOCIATE against it. Tunnels UDP if
  /// the upstream grants it; refuses (no leak) if it does not.
  Socks5Upstream,
  /// Upstream that cannot carry UDP (HTTP/HTTPS/SOCKS4/Shadowsocks): refuse so
  /// Chromium falls back to proxied TCP instead of leaking UDP.
  Refuse,
}

/// Decide the leak-safe UDP policy for an upstream URL.
fn udp_mode(upstream_url: Option<&str>) -> UdpMode {
  match upstream_url {
    None => UdpMode::Direct,
    Some("DIRECT") => UdpMode::Direct,
    Some(url) => match Url::parse(url).ok().map(|u| u.scheme().to_lowercase()) {
      Some(scheme) if scheme == "socks5" => UdpMode::Socks5Upstream,
      // http / https / socks4 / ss / shadowsocks / anything else: TCP-only.
      _ => UdpMode::Refuse,
    },
  }
}

/// `0.0.0.0:0` — used for BND fields in replies where the bound address is
/// irrelevant to the client (e.g. CONNECT).
fn unspecified() -> SocketAddr {
  SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
}

/// Handle one SOCKS5 client connection from the browser. Mirrors the spawn
/// contract of `proxy_server::handle_proxy_connection`.
pub async fn handle_socks5_connection(
  mut stream: TcpStream,
  upstream_url: Option<String>,
  bypass_matcher: BypassMatcher,
  blocklist_matcher: BlocklistMatcher,
) {
  let _ = stream.set_nodelay(true);

  if let Err(e) = negotiate_method(&mut stream).await {
    log::debug!("SOCKS5 method negotiation failed: {e}");
    return;
  }

  let request = match read_request(&mut stream).await {
    Ok(r) => r,
    Err(e) => {
      log::debug!("SOCKS5 request parse failed: {e}");
      let _ = send_reply(&mut stream, REP_GENERAL_FAILURE, unspecified()).await;
      return;
    }
  };

  match request.cmd {
    CMD_CONNECT => {
      handle_connect(
        stream,
        request.host,
        request.port,
        upstream_url,
        bypass_matcher,
        blocklist_matcher,
      )
      .await;
    }
    CMD_UDP_ASSOCIATE => {
      handle_udp_associate(stream, upstream_url, blocklist_matcher).await;
    }
    other => {
      log::debug!("SOCKS5 unsupported command {other:#04x}");
      let _ = send_reply(&mut stream, REP_COMMAND_NOT_SUPPORTED, unspecified()).await;
    }
  }
}

/// Read the SOCKS5 greeting and select the no-auth method. The local proxy is
/// loopback-only, so no authentication is required (Chromium offers no-auth).
async fn negotiate_method(stream: &mut TcpStream) -> std::io::Result<()> {
  let mut head = [0u8; 2];
  stream.read_exact(&mut head).await?;
  if head[0] != 0x05 {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "not a SOCKS5 greeting",
    ));
  }
  let nmethods = head[1] as usize;
  let mut methods = vec![0u8; nmethods];
  stream.read_exact(&mut methods).await?;

  if methods.contains(&0x00) {
    stream.write_all(&[0x05, 0x00]).await?;
    Ok(())
  } else {
    // No acceptable methods.
    let _ = stream.write_all(&[0x05, 0xFF]).await;
    Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "no no-auth method offered",
    ))
  }
}

struct Socks5Request {
  cmd: u8,
  host: String,
  port: u16,
}

/// Read a SOCKS5 request line: VER, CMD, RSV, ATYP, DST.ADDR, DST.PORT.
async fn read_request(stream: &mut TcpStream) -> std::io::Result<Socks5Request> {
  let mut head = [0u8; 4];
  stream.read_exact(&mut head).await?;
  if head[0] != 0x05 {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "bad SOCKS5 request version",
    ));
  }
  let cmd = head[1];
  let atyp = head[3];
  let host = read_addr(stream, atyp).await?;
  let mut port = [0u8; 2];
  stream.read_exact(&mut port).await?;
  Ok(Socks5Request {
    cmd,
    host,
    port: u16::from_be_bytes(port),
  })
}

/// Read a SOCKS5 address of the given type into a host string (an IP literal or
/// a domain name; `connect_to_target_via_upstream` handles both).
async fn read_addr(stream: &mut TcpStream, atyp: u8) -> std::io::Result<String> {
  match atyp {
    0x01 => {
      let mut b = [0u8; 4];
      stream.read_exact(&mut b).await?;
      Ok(Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string())
    }
    0x04 => {
      let mut b = [0u8; 16];
      stream.read_exact(&mut b).await?;
      Ok(Ipv6Addr::from(b).to_string())
    }
    0x03 => {
      let mut len = [0u8; 1];
      stream.read_exact(&mut len).await?;
      let mut domain = vec![0u8; len[0] as usize];
      stream.read_exact(&mut domain).await?;
      Ok(String::from_utf8_lossy(&domain).to_string())
    }
    other => Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      format!("unsupported SOCKS5 address type {other:#04x}"),
    )),
  }
}

/// Write a SOCKS5 reply with the given code and bound address.
async fn send_reply(stream: &mut TcpStream, rep: u8, bnd: SocketAddr) -> std::io::Result<()> {
  let mut resp = vec![0x05, rep, 0x00];
  push_addr(&mut resp, bnd);
  stream.write_all(&resp).await
}

/// Append an ATYP + address + port to a SOCKS5 message buffer.
fn push_addr(buf: &mut Vec<u8>, addr: SocketAddr) {
  match addr.ip() {
    IpAddr::V4(v4) => {
      buf.push(0x01);
      buf.extend_from_slice(&v4.octets());
    }
    IpAddr::V6(v6) => {
      buf.push(0x04);
      buf.extend_from_slice(&v6.octets());
    }
  }
  buf.extend_from_slice(&addr.port().to_be_bytes());
}

/// SOCKS5 CONNECT: dial the target via the upstream and bidirectionally tunnel,
/// reusing the same code path as the HTTP CONNECT proxy.
async fn handle_connect(
  mut stream: TcpStream,
  host: String,
  port: u16,
  upstream_url: Option<String>,
  bypass_matcher: BypassMatcher,
  blocklist_matcher: BlocklistMatcher,
) {
  if blocklist_matcher.is_blocked(&host) {
    log::debug!("[blocklist] Blocked SOCKS5 CONNECT to {host}");
    let _ = send_reply(&mut stream, REP_NOT_ALLOWED, unspecified()).await;
    return;
  }

  if let Some(tracker) = get_traffic_tracker() {
    tracker.record_request(&host, 0, 0);
  }

  log::debug!(
    "SOCKS5 CONNECT {}:{} (upstream={})",
    host,
    port,
    upstream_url.as_deref().unwrap_or("DIRECT")
  );

  // Resolve to the target stream, logging and dropping the (non-Send) dial
  // error inside the match arm so it is never held across the await below.
  let target = match connect_to_target_via_upstream(
    &host,
    port,
    upstream_url.as_deref(),
    &bypass_matcher,
  )
  .await
  {
    Ok(t) => Some(t),
    Err(e) => {
      let key = format!("socks5-connect:{host}:{port}");
      if let Some(suppressed) = crate::proxy_server::log_throttle(&key) {
        if suppressed > 0 {
          log::warn!(
              "SOCKS5 CONNECT to {host}:{port} failed: {e} ({suppressed} more suppressed in last 30s)"
            );
        } else {
          log::warn!("SOCKS5 CONNECT to {host}:{port} failed: {e}");
        }
      }
      None
    }
  };

  let Some(target) = target else {
    let _ = send_reply(&mut stream, REP_GENERAL_FAILURE, unspecified()).await;
    return;
  };

  if send_reply(&mut stream, REP_SUCCEEDED, unspecified())
    .await
    .is_err()
  {
    return;
  }
  tunnel_streams(stream, target, host).await;
}

/// SOCKS5 UDP ASSOCIATE, leak-safe per upstream (see [`UdpMode`]).
///
/// `control` is the TCP control connection; the UDP association lives exactly
/// as long as it stays open (RFC 1928 §6), so the relay loop tears down when
/// the browser closes it. Every relayed datagram is filtered and metered via
/// [`UdpRelayContext`], matching the TCP CONNECT path.
async fn handle_udp_associate(
  mut control: TcpStream,
  upstream_url: Option<String>,
  blocklist_matcher: BlocklistMatcher,
) {
  let mode = udp_mode(upstream_url.as_deref());

  if mode == UdpMode::Refuse {
    log::info!(
      "SOCKS5 UDP ASSOCIATE refused: upstream ({}) cannot carry UDP without leaking; Chromium will use proxied TCP",
      upstream_url.as_deref().unwrap_or("DIRECT")
    );
    let _ = send_reply(&mut control, REP_COMMAND_NOT_SUPPORTED, unspecified()).await;
    return;
  }

  // The UDP relay socket the browser sends its datagrams to. Loopback-only.
  let relay = match UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await {
    Ok(s) => s,
    Err(e) => {
      log::warn!("Failed to bind UDP relay socket: {e}");
      let _ = send_reply(&mut control, REP_GENERAL_FAILURE, unspecified()).await;
      return;
    }
  };
  let relay_addr = match relay.local_addr() {
    Ok(a) => a,
    Err(e) => {
      log::warn!("Failed to read UDP relay addr: {e}");
      let _ = send_reply(&mut control, REP_GENERAL_FAILURE, unspecified()).await;
      return;
    }
  };

  match mode {
    UdpMode::Direct => {
      // Bind the egress socket before replying so a failure surfaces as a
      // refusal (no half-open association).
      let out = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await {
        Ok(s) => s,
        Err(e) => {
          log::warn!("Failed to bind UDP egress socket: {e}");
          let _ = send_reply(&mut control, REP_GENERAL_FAILURE, unspecified()).await;
          return;
        }
      };
      if send_reply(&mut control, REP_SUCCEEDED, relay_addr)
        .await
        .is_err()
      {
        return;
      }
      log::info!("SOCKS5 UDP ASSOCIATE (direct) relaying on {relay_addr}");
      run_udp_relay_direct(control, relay, out, UdpRelayContext::new(blocklist_matcher)).await;
    }
    UdpMode::Socks5Upstream => {
      // Establish the upstream association FIRST; if the upstream refuses UDP,
      // refuse to the browser too (no leak).
      let upstream = upstream_url.as_deref().unwrap_or("");
      let datagram = match associate_upstream(upstream).await {
        Ok(d) => d,
        Err(e) => {
          log::info!(
            "SOCKS5 upstream did not grant UDP ASSOCIATE ({e}); refusing so Chromium uses proxied TCP"
          );
          let _ = send_reply(&mut control, REP_COMMAND_NOT_SUPPORTED, unspecified()).await;
          return;
        }
      };
      if send_reply(&mut control, REP_SUCCEEDED, relay_addr)
        .await
        .is_err()
      {
        return;
      }
      log::info!("SOCKS5 UDP ASSOCIATE (via SOCKS5 upstream) relaying on {relay_addr}");
      run_udp_relay_socks5(
        control,
        relay,
        datagram,
        UdpRelayContext::new(blocklist_matcher),
      )
      .await;
    }
    UdpMode::Refuse => unreachable!("handled above"),
  }
}

/// Open a SOCKS5 UDP association against the upstream proxy.
async fn associate_upstream(
  upstream_url: &str,
) -> Result<SocksDatagram<TcpStream>, Box<dyn std::error::Error + Send + Sync>> {
  let upstream = Url::parse(upstream_url)?;
  let host = upstream.host_str().unwrap_or("127.0.0.1");
  let port = upstream.port().unwrap_or(1080);
  let auth = if !upstream.username().is_empty() {
    Some(Auth {
      username: upstream.username().to_string(),
      password: upstream.password().unwrap_or("").to_string(),
    })
  } else {
    None
  };

  let proxy_stream = tokio::time::timeout(
    crate::proxy_server::UPSTREAM_DIAL_TIMEOUT,
    TcpStream::connect((host, port)),
  )
  .await
  .map_err(|_| format!("upstream SOCKS5 connect to {host}:{port} timed out"))??;
  let bind_sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
  // association_addr None => 0.0.0.0:0 (we accept replies from any peer).
  let datagram = tokio::time::timeout(
    crate::proxy_server::UPSTREAM_DIAL_TIMEOUT,
    SocksDatagram::associate(proxy_stream, bind_sock, auth, None::<AddrKind>),
  )
  .await
  .map_err(|_| "upstream SOCKS5 UDP ASSOCIATE handshake timed out")??;
  Ok(datagram)
}

/// Parsed SOCKS5 UDP datagram header (RFC 1928 §7): the destination and the
/// offset at which the payload begins. Fragmented datagrams (FRAG != 0) are
/// rejected by the caller.
struct UdpHeader {
  frag: u8,
  dst: AddrKind,
  data_offset: usize,
}

fn parse_udp_header(buf: &[u8]) -> Option<UdpHeader> {
  if buf.len() < 4 {
    return None;
  }
  let frag = buf[2];
  let atyp = buf[3];
  match atyp {
    0x01 => {
      if buf.len() < 10 {
        return None;
      }
      let ip = Ipv4Addr::new(buf[4], buf[5], buf[6], buf[7]);
      let port = u16::from_be_bytes([buf[8], buf[9]]);
      Some(UdpHeader {
        frag,
        dst: AddrKind::Ip(SocketAddr::new(IpAddr::V4(ip), port)),
        data_offset: 10,
      })
    }
    0x04 => {
      if buf.len() < 22 {
        return None;
      }
      let mut octets = [0u8; 16];
      octets.copy_from_slice(&buf[4..20]);
      let ip = Ipv6Addr::from(octets);
      let port = u16::from_be_bytes([buf[20], buf[21]]);
      Some(UdpHeader {
        frag,
        dst: AddrKind::Ip(SocketAddr::new(IpAddr::V6(ip), port)),
        data_offset: 22,
      })
    }
    0x03 => {
      let dlen = *buf.get(4)? as usize;
      let needed = 5 + dlen + 2;
      if buf.len() < needed {
        return None;
      }
      let domain = String::from_utf8_lossy(&buf[5..5 + dlen]).to_string();
      let port = u16::from_be_bytes([buf[5 + dlen], buf[6 + dlen]]);
      Some(UdpHeader {
        frag,
        dst: AddrKind::Domain(domain, port),
        data_offset: needed,
      })
    }
    _ => None,
  }
}

/// Build a SOCKS5 UDP response datagram (header + payload) to send back to the
/// browser, naming `peer` as the source.
fn build_udp_response(peer: SocketAddr, data: &[u8]) -> Vec<u8> {
  let mut out = vec![0x00, 0x00, 0x00]; // RSV(2) + FRAG(0)
  push_addr(&mut out, peer);
  out.extend_from_slice(data);
  out
}

/// Direct UDP relay: browser <-> a plain egress UDP socket. Used only when
/// there is no upstream proxy, so the host IP is the profile's own IP.
async fn run_udp_relay_direct(
  mut control: TcpStream,
  relay: UdpSocket,
  out: UdpSocket,
  mut ctx: UdpRelayContext,
) {
  let mut client_addr: Option<SocketAddr> = None;
  let mut from_client = vec![0u8; UDP_BUF];
  let mut from_target = vec![0u8; UDP_BUF];
  let mut ctrl_buf = [0u8; 256];

  loop {
    tokio::select! {
      // Control connection closed => association ends.
      r = control.read(&mut ctrl_buf) => {
        match r {
          Ok(0) | Err(_) => break,
          Ok(_) => {} // ignore any data on the control channel
        }
      }
      // Browser -> target.
      r = relay.recv_from(&mut from_client) => {
        let Ok((n, src)) = r else { break };
        client_addr = Some(src);
        let Some(header) = parse_udp_header(&from_client[..n]) else { continue };
        if header.frag != 0 {
          continue; // fragmentation unsupported
        }
        let host = addrkind_host(&header.dst);
        if ctx.is_blocked(&host) {
          log::debug!("[blocklist] Blocked SOCKS5 UDP datagram to {host}");
          continue;
        }
        let payload = &from_client[header.data_offset..n];
        // Resolve after the blocklist check so a blocked host is never even
        // looked up, and count bytes against the resolved peer so replies from
        // it are attributed back to this host.
        let dst = match resolve_addr(&header.dst).await {
          Some(d) => d,
          None => continue,
        };
        if out.send_to(payload, dst).await.is_ok() {
          ctx.record_sent(&host, dst.to_string(), payload.len() as u64);
        }
      }
      // Target -> browser.
      r = out.recv_from(&mut from_target) => {
        let Ok((n, peer)) = r else { continue };
        if let Some(client) = client_addr {
          let resp = build_udp_response(peer, &from_target[..n]);
          if relay.send_to(&resp, client).await.is_ok() {
            ctx.record_received(&peer.to_string(), &peer.ip().to_string(), n as u64);
          }
        }
      }
    }
  }

  ctx.flush();
}

/// UDP relay tunneled through a SOCKS5 upstream that granted UDP ASSOCIATE.
async fn run_udp_relay_socks5(
  mut control: TcpStream,
  relay: UdpSocket,
  datagram: SocksDatagram<TcpStream>,
  mut ctx: UdpRelayContext,
) {
  let mut client_addr: Option<SocketAddr> = None;
  let mut from_client = vec![0u8; UDP_BUF];
  let mut from_upstream = vec![0u8; UDP_BUF];
  let mut ctrl_buf = [0u8; 256];

  loop {
    tokio::select! {
      r = control.read(&mut ctrl_buf) => {
        match r {
          Ok(0) | Err(_) => break,
          Ok(_) => {}
        }
      }
      // Browser -> upstream.
      r = relay.recv_from(&mut from_client) => {
        let Ok((n, src)) = r else { break };
        client_addr = Some(src);
        let Some(header) = parse_udp_header(&from_client[..n]) else { continue };
        if header.frag != 0 {
          continue;
        }
        let host = addrkind_host(&header.dst);
        if ctx.is_blocked(&host) {
          log::debug!("[blocklist] Blocked SOCKS5 UDP datagram to {host}");
          continue;
        }
        let peer_key = addrkind_key(&header.dst);
        let payload = from_client[header.data_offset..n].to_vec();
        if datagram.send_to(&payload, header.dst).await.is_ok() {
          ctx.record_sent(&host, peer_key, payload.len() as u64);
        }
      }
      // Upstream -> browser.
      r = datagram.recv_from(&mut from_upstream) => {
        let Ok((n, peer)) = r else { continue };
        if let Some(client) = client_addr {
          let resp = build_udp_response(addrkind_to_socketaddr(&peer), &from_upstream[..n]);
          if relay.send_to(&resp, client).await.is_ok() {
            // The upstream usually reports the peer by IP even when we
            // addressed a domain, so an unmatched flow falls back to that IP.
            ctx.record_received(&addrkind_key(&peer), &addrkind_host(&peer), n as u64);
          }
        }
      }
    }
  }

  ctx.flush();
}

/// Resolve a UDP destination to a concrete socket address for direct relay.
async fn resolve_addr(addr: &AddrKind) -> Option<SocketAddr> {
  match addr {
    AddrKind::Ip(s) => Some(*s),
    AddrKind::Domain(domain, port) => tokio::net::lookup_host(format!("{domain}:{port}"))
      .await
      .ok()
      .and_then(|mut it| it.next()),
  }
}

/// Best-effort conversion of an upstream-reported source address into a
/// `SocketAddr` for the response header. A domain (rare for UDP) collapses to
/// `0.0.0.0:port`, which clients treat as "from the proxy".
fn addrkind_to_socketaddr(addr: &AddrKind) -> SocketAddr {
  match addr {
    AddrKind::Ip(s) => *s,
    AddrKind::Domain(_, port) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), *port),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn udp_mode_direct_for_none_and_direct() {
    assert_eq!(udp_mode(None), UdpMode::Direct);
    assert_eq!(udp_mode(Some("DIRECT")), UdpMode::Direct);
  }

  #[test]
  fn udp_mode_socks5_upstream() {
    assert_eq!(
      udp_mode(Some("socks5://user:pass@1.2.3.4:1080")),
      UdpMode::Socks5Upstream
    );
    assert_eq!(
      udp_mode(Some("socks5://1.2.3.4:1080")),
      UdpMode::Socks5Upstream
    );
  }

  #[test]
  fn udp_mode_refuses_tcp_only_upstreams() {
    // HTTP/HTTPS CONNECT, SOCKS4, and Shadowsocks cannot carry UDP, so UDP
    // ASSOCIATE must be refused (Chromium then uses proxied TCP — no leak).
    assert_eq!(udp_mode(Some("http://1.2.3.4:8080")), UdpMode::Refuse);
    assert_eq!(udp_mode(Some("https://1.2.3.4:8080")), UdpMode::Refuse);
    assert_eq!(udp_mode(Some("socks4://1.2.3.4:1080")), UdpMode::Refuse);
    assert_eq!(
      udp_mode(Some("ss://aes-256-gcm:pw@1.2.3.4:8388")),
      UdpMode::Refuse
    );
  }

  #[test]
  fn parse_udp_header_ipv4() {
    // RSV RSV FRAG ATYP=1 1.2.3.4 :443 payload="hi"
    let buf = [0, 0, 0, 0x01, 1, 2, 3, 4, 0x01, 0xBB, b'h', b'i'];
    let h = parse_udp_header(&buf).expect("ipv4 header");
    assert_eq!(h.frag, 0);
    assert_eq!(h.data_offset, 10);
    assert_eq!(
      h.dst,
      AddrKind::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), 443))
    );
    assert_eq!(&buf[h.data_offset..], b"hi");
  }

  #[test]
  fn parse_udp_header_domain() {
    // ATYP=3, len=3, "abc", port 8080, payload "x"
    let mut buf = vec![0, 0, 0, 0x03, 3, b'a', b'b', b'c', 0x1F, 0x90];
    buf.push(b'x');
    let h = parse_udp_header(&buf).expect("domain header");
    assert_eq!(h.dst, AddrKind::Domain("abc".to_string(), 8080));
    assert_eq!(&buf[h.data_offset..], b"x");
  }

  #[test]
  fn parse_udp_header_rejects_truncated() {
    assert!(parse_udp_header(&[0, 0, 0]).is_none());
    assert!(parse_udp_header(&[0, 0, 0, 0x01, 1, 2]).is_none());
  }

  #[test]
  fn addrkind_host_strips_port_for_blocklist_matching() {
    // The blocklist matches CONNECT hosts without a port, so UDP destinations
    // must be rendered the same way or IP rules would never match.
    assert_eq!(
      addrkind_host(&AddrKind::Ip(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
        443
      ))),
      "1.2.3.4"
    );
    assert_eq!(
      addrkind_host(&AddrKind::Domain("ads.example.com".into(), 443)),
      "ads.example.com"
    );
  }

  #[test]
  fn addrkind_key_distinguishes_ports() {
    assert_eq!(
      addrkind_key(&AddrKind::Domain("example.com".into(), 443)),
      "example.com:443"
    );
    assert_eq!(
      addrkind_key(&AddrKind::Ip(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
        53
      ))),
      "1.2.3.4:53"
    );
  }

  #[test]
  fn udp_relay_context_blocks_like_the_tcp_path() {
    let dir = std::env::temp_dir().join(format!("donut-udp-blocklist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("block.txt");
    std::fs::write(&path, "ads.example.com\n").unwrap();

    let matcher = BlocklistMatcher::from_file(path.to_str().unwrap()).unwrap();
    let ctx = UdpRelayContext::new(matcher);
    assert!(ctx.is_blocked("ads.example.com"));
    // Subdomains are blocked by the parent rule, as on the CONNECT path.
    assert!(ctx.is_blocked("cdn.ads.example.com"));
    assert!(!ctx.is_blocked("example.com"));

    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn udp_relay_context_allowlist_mode_blocks_unlisted_ip_destinations() {
    // Parity check: allowlist mode blocks a bare IP destination over UDP the
    // same way is_blocked does for a CONNECT to an IP literal — otherwise QUIC
    // to a hardcoded IP would walk straight through a strict allowlist.
    let dir = std::env::temp_dir().join(format!("donut-udp-allowlist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("allow.txt");
    std::fs::write(&path, "trusted.example.com\n").unwrap();

    let matcher = BlocklistMatcher::from_file_with_mode(path.to_str().unwrap(), true).unwrap();
    let ctx = UdpRelayContext::new(matcher);
    assert!(!ctx.is_blocked("trusted.example.com"));
    assert!(ctx.is_blocked("evil.example.com"));
    assert!(ctx.is_blocked(&addrkind_host(&AddrKind::Ip(SocketAddr::new(
      IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
      443
    )))));

    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn udp_relay_context_attributes_replies_to_the_destination_host() {
    // No tracker is initialised in tests, so this exercises the flow map that
    // decides which domain reply bytes belong to.
    let mut ctx = UdpRelayContext::new(BlocklistMatcher::new());
    ctx.flow.insert("1.2.3.4:443".into(), "example.com".into());
    assert_eq!(
      ctx.flow.get("1.2.3.4:443").map(String::as_str),
      Some("example.com")
    );
    // An unknown peer has no flow and falls back to the peer's own host.
    assert!(!ctx.flow.contains_key("9.9.9.9:53"));
  }

  #[test]
  fn build_udp_response_prefixes_header() {
    let resp = build_udp_response(
      SocketAddr::new(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)), 53),
      b"data",
    );
    // RSV RSV FRAG ATYP=1 9.9.9.9 :53 "data"
    assert_eq!(
      resp,
      vec![0, 0, 0, 0x01, 9, 9, 9, 9, 0x00, 0x35, b'd', b'a', b't', b'a']
    );
  }
}

//! The DNS server a profile resolves names with.
//!
//! Blank keeps the old behavior: the proxy resolves every name. When a profile
//! names a server, its local proxy worker resolves each destination with it and
//! asks the upstream for the address instead of the name. The queries travel
//! the profile's own route (proxy, VLESS worker or VPN worker), so the resolver
//! sees the proxy's exit address, its answers match the proxy's location, and
//! nothing new reaches the local network.
//!
//! Encryption is the default wherever the server offers it: a plain address of
//! a well-known public resolver is upgraded to that resolver's DNS over HTTPS
//! endpoint on the same addresses, and `tcp://` keeps it plain on purpose. A
//! server that does not answer fails the connection. Falling back to the
//! proxy's resolver would quietly undo what the person chose.

use crate::proxy_server::BoxedAsyncStream;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::{Request, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::Serialize;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

const DOH_PATH: &str = "/dns-query";
const MAX_INPUT_LEN: usize = 512;

/// Public resolvers that serve DNS over HTTPS on the same addresses as their
/// plain DNS, at `/dns-query`, with a certificate for `name`. Every address
/// here was checked to answer over HTTP/2; Quad9 and Mullvad refuse HTTP/1.1.
struct KnownResolver {
  name: &'static str,
  addresses: &'static [&'static str],
}

const KNOWN_RESOLVERS: &[KnownResolver] = &[
  KnownResolver {
    name: "cloudflare-dns.com",
    addresses: &[
      "1.1.1.1",
      "1.0.0.1",
      "2606:4700:4700::1111",
      "2606:4700:4700::1001",
    ],
  },
  KnownResolver {
    name: "security.cloudflare-dns.com",
    addresses: &[
      "1.1.1.2",
      "1.0.0.2",
      "2606:4700:4700::1112",
      "2606:4700:4700::1002",
    ],
  },
  KnownResolver {
    name: "family.cloudflare-dns.com",
    addresses: &[
      "1.1.1.3",
      "1.0.0.3",
      "2606:4700:4700::1113",
      "2606:4700:4700::1003",
    ],
  },
  KnownResolver {
    name: "dns.google",
    addresses: &[
      "8.8.8.8",
      "8.8.4.4",
      "2001:4860:4860::8888",
      "2001:4860:4860::8844",
    ],
  },
  KnownResolver {
    name: "dns.quad9.net",
    addresses: &["9.9.9.9", "149.112.112.112", "2620:fe::fe", "2620:fe::9"],
  },
  KnownResolver {
    name: "dns10.quad9.net",
    addresses: &["9.9.9.10", "149.112.112.10"],
  },
  KnownResolver {
    name: "dns11.quad9.net",
    addresses: &["9.9.9.11", "149.112.112.11"],
  },
  KnownResolver {
    name: "dns.mullvad.net",
    addresses: &["194.242.2.2", "2a07:e340::2"],
  },
  KnownResolver {
    name: "adblock.dns.mullvad.net",
    addresses: &["194.242.2.3"],
  },
  KnownResolver {
    name: "base.dns.mullvad.net",
    addresses: &["194.242.2.4"],
  },
  KnownResolver {
    name: "extended.dns.mullvad.net",
    addresses: &["194.242.2.5"],
  },
  KnownResolver {
    name: "family.dns.mullvad.net",
    addresses: &["194.242.2.6"],
  },
  KnownResolver {
    name: "all.dns.mullvad.net",
    addresses: &["194.242.2.9"],
  },
  KnownResolver {
    name: "dns.adguard-dns.com",
    addresses: &["94.140.14.14", "94.140.15.15"],
  },
  KnownResolver {
    name: "family.adguard-dns.com",
    addresses: &["94.140.14.15", "94.140.15.16"],
  },
  KnownResolver {
    name: "unfiltered.adguard-dns.com",
    addresses: &["94.140.14.140", "94.140.14.141"],
  },
  KnownResolver {
    name: "doh.opendns.com",
    addresses: &[
      "208.67.222.222",
      "208.67.220.220",
      "2620:119:35::35",
      "2620:119:53::53",
    ],
  },
];

fn known_by_address(ip: IpAddr) -> Option<&'static KnownResolver> {
  KNOWN_RESOLVERS.iter().find(|known| {
    known
      .addresses
      .iter()
      .any(|address| address.parse::<IpAddr>().ok() == Some(ip))
  })
}

fn known_by_name(name: &str) -> Option<&'static KnownResolver> {
  KNOWN_RESOLVERS
    .iter()
    .find(|known| known.name.eq_ignore_ascii_case(name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsTransport {
  /// Plain DNS over TCP. TCP and not UDP because it has to cross the proxy.
  Tcp,
  /// DNS over TLS (RFC 7858).
  Tls,
  /// DNS over HTTPS (RFC 8484).
  Https,
}

impl DnsTransport {
  pub fn as_str(self) -> &'static str {
    match self {
      DnsTransport::Tcp => "tcp",
      DnsTransport::Tls => "tls",
      DnsTransport::Https => "https",
    }
  }
}

/// A parsed profile DNS server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsServer {
  pub transport: DnsTransport,
  /// What the route dials: an IP literal, or a name the route resolves.
  dial_host: String,
  port: u16,
  /// The name the certificate must carry; also the DoH authority.
  tls_name: String,
  /// The DoH request target, with any query string.
  path: String,
  /// A plain address that now uses the provider's encrypted endpoint.
  upgraded: bool,
}

/// What the person typed is not a DNS server Donut can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidDnsServer;

/// How the UI describes a server, so the person sees what the field means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DnsServerSummary {
  pub transport: DnsTransport,
  /// Host, plus the port when it is not the transport's default.
  pub server: String,
  pub encrypted: bool,
  pub upgraded: bool,
}

fn bracketed(host: &str) -> String {
  if host.parse::<std::net::Ipv6Addr>().is_ok() {
    format!("[{host}]")
  } else {
    host.to_string()
  }
}

fn valid_hostname(host: &str) -> bool {
  let host = host.strip_suffix('.').unwrap_or(host);
  !host.is_empty()
    && host.len() <= 253
    && host.split('.').all(|label| {
      !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
          .bytes()
          .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// An IP literal or a valid host name, lowercased and without the brackets
/// of an IPv6 literal.
fn checked_host(host: &str) -> Result<String, InvalidDnsServer> {
  let host = host
    .strip_prefix('[')
    .and_then(|h| h.strip_suffix(']'))
    .unwrap_or(host);
  if let Ok(ip) = host.parse::<IpAddr>() {
    return Ok(ip.to_string());
  }
  if valid_hostname(host) {
    Ok(host.trim_end_matches('.').to_ascii_lowercase())
  } else {
    Err(InvalidDnsServer)
  }
}

fn known_for(host: &str) -> Option<&'static KnownResolver> {
  match host.parse::<IpAddr>() {
    Ok(ip) => known_by_address(ip),
    Err(_) => known_by_name(host),
  }
}

impl DnsServer {
  /// Accepts `https://host/path` (DNS over HTTPS), `tls://host[:port]` (DNS
  /// over TLS), `tcp://host[:port]` (plain DNS), and a bare `host[:port]`.
  /// A bare address of a well-known resolver uses its DNS over HTTPS endpoint.
  pub fn parse(input: &str) -> Result<Self, InvalidDnsServer> {
    let input = input.trim();
    // RFC 8484 servers publish URI templates ending in `{?dns}`. Queries go
    // as POST, so the template variable is not needed.
    let input = input.strip_suffix("{?dns}").unwrap_or(input);
    if input.is_empty()
      || input.len() > MAX_INPUT_LEN
      || input.chars().any(|c| c.is_whitespace() || c.is_control())
    {
      return Err(InvalidDnsServer);
    }

    let Some((scheme, _)) = input.split_once("://") else {
      return Self::parse_bare(input);
    };
    let url = Url::parse(input).map_err(|_| InvalidDnsServer)?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
      return Err(InvalidDnsServer);
    }
    let host = checked_host(url.host_str().ok_or(InvalidDnsServer)?)?;
    let bare_path = matches!(url.path(), "" | "/") && url.query().is_none();
    let known = known_for(&host);
    // A known name is dialed at its published address, so reaching the
    // resolver does not depend on the proxy's own resolver working.
    let dial_host = match known {
      Some(known) if host.parse::<IpAddr>().is_err() => known.addresses[0].to_string(),
      _ => host.clone(),
    };
    // A known address is checked against its provider's name: that is what
    // its certificate carries for certain.
    let tls_name = known.map_or_else(|| host.clone(), |known| known.name.to_string());

    match scheme.to_ascii_lowercase().as_str() {
      "https" => {
        let mut path = match url.path() {
          "" | "/" => DOH_PATH.to_string(),
          path => path.to_string(),
        };
        if let Some(query) = url.query() {
          path.push('?');
          path.push_str(query);
        }
        Ok(Self {
          transport: DnsTransport::Https,
          dial_host,
          port: url.port().unwrap_or(443),
          tls_name,
          path,
          upgraded: false,
        })
      }
      "tls" if bare_path => Ok(Self {
        transport: DnsTransport::Tls,
        dial_host,
        port: url.port().unwrap_or(853),
        tls_name,
        path: String::new(),
        upgraded: false,
      }),
      "tcp" if bare_path => Ok(Self {
        transport: DnsTransport::Tcp,
        port: url.port().unwrap_or(53),
        tls_name: host.clone(),
        dial_host: host,
        path: String::new(),
        upgraded: false,
      }),
      _ => Err(InvalidDnsServer),
    }
  }

  fn parse_bare(input: &str) -> Result<Self, InvalidDnsServer> {
    let (host, port) = if let Ok(ip) = input.parse::<IpAddr>() {
      (ip.to_string(), None)
    } else if let Ok(address) = input.parse::<SocketAddr>() {
      (address.ip().to_string(), Some(address.port()))
    } else if input.starts_with('[') && input.ends_with(']') {
      (checked_host(input)?, None)
    } else {
      match input.rsplit_once(':') {
        Some((host, port)) => (
          checked_host(host)?,
          Some(port.parse::<u16>().map_err(|_| InvalidDnsServer)?),
        ),
        None => (checked_host(input)?, None),
      }
    };
    if port == Some(0) {
      return Err(InvalidDnsServer);
    }

    if matches!(port, None | Some(53)) {
      if let Some(known) = known_for(&host) {
        let dial_host = if host.parse::<IpAddr>().is_ok() {
          host
        } else {
          known.addresses[0].to_string()
        };
        return Ok(Self {
          transport: DnsTransport::Https,
          dial_host,
          port: 443,
          tls_name: known.name.to_string(),
          path: DOH_PATH.to_string(),
          upgraded: true,
        });
      }
    }
    Ok(Self {
      transport: DnsTransport::Tcp,
      tls_name: host.clone(),
      dial_host: host,
      port: port.unwrap_or(53),
      path: String::new(),
      upgraded: false,
    })
  }

  pub fn summary(&self) -> DnsServerSummary {
    let (host, default_port) = match self.transport {
      DnsTransport::Tcp => (&self.dial_host, 53),
      DnsTransport::Tls => (&self.tls_name, 853),
      DnsTransport::Https => (&self.tls_name, 443),
    };
    let server = if self.port == default_port {
      host.clone()
    } else {
      format!("{}:{}", bracketed(host), self.port)
    };
    DnsServerSummary {
      transport: self.transport,
      server,
      encrypted: self.transport != DnsTransport::Tcp,
      upgraded: self.upgraded,
    }
  }

  fn authority(&self) -> String {
    if self.port == 443 {
      bracketed(&self.tls_name)
    } else {
      format!("{}:{}", bracketed(&self.tls_name), self.port)
    }
  }
}

/// The stored form of a DNS server setting: trimmed, with blank as `None`.
/// Rejects anything `DnsServer::parse` cannot use, as `DNS_SERVER_INVALID`.
pub fn normalize_dns_server(value: Option<String>) -> Result<Option<String>, String> {
  let Some(value) = value
    .map(|v| v.trim().to_string())
    .filter(|v| !v.is_empty())
  else {
    return Ok(None);
  };
  DnsServer::parse(&value).map_err(|_| crate::backend_error("DNS_SERVER_INVALID"))?;
  Ok(Some(value))
}

/// What a DNS server setting will do, for the field that edits it. `None` for
/// blank: the proxy keeps resolving names.
#[tauri::command]
pub fn describe_dns_server(value: String) -> Result<Option<DnsServerSummary>, String> {
  if value.trim().is_empty() {
    return Ok(None);
  }
  DnsServer::parse(&value)
    .map(|server| Some(server.summary()))
    .map_err(|_| crate::backend_error("DNS_SERVER_INVALID"))
}

// --- DNS messages ---------------------------------------------------------

const TYPE_A: u16 = 1;
const TYPE_AAAA: u16 = 28;
const TYPE_OPT: u16 = 41;
const CLASS_IN: u16 = 1;
const RCODE_NOERROR: u8 = 0;
const RCODE_NXDOMAIN: u8 = 3;
const EDNS_UDP_SIZE: u16 = 1232;
const EDNS_PADDING: u16 = 12;
/// RFC 8467: pad encrypted queries to a multiple of 128 octets, so their
/// length does not tell an observer which name was asked for.
const PAD_BLOCK: usize = 128;
const MAX_MESSAGE: usize = 65535;

fn encode_name(out: &mut Vec<u8>, name: &str) -> Option<()> {
  if name.is_empty() {
    return None;
  }
  let start = out.len();
  for label in name.split('.') {
    let bytes = label.as_bytes();
    if bytes.is_empty()
      || bytes.len() > 63
      || !bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
    {
      return None;
    }
    out.push(bytes.len() as u8);
    out.extend_from_slice(bytes);
  }
  out.push(0);
  (out.len() - start <= 255).then_some(())
}

/// A recursive query for one name and type. No client subnet is ever sent;
/// `pad` adds an EDNS(0) padding option.
fn encode_query(id: u16, name: &str, qtype: u16, pad: bool) -> Option<Vec<u8>> {
  let mut out = Vec::with_capacity(PAD_BLOCK);
  out.extend_from_slice(&id.to_be_bytes());
  out.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
  out.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
  out.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
  out.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
  out.extend_from_slice(&u16::from(pad).to_be_bytes()); // ARCOUNT
  encode_name(&mut out, name)?;
  out.extend_from_slice(&qtype.to_be_bytes());
  out.extend_from_slice(&CLASS_IN.to_be_bytes());
  if pad {
    // OPT record (11 octets) with one padding option (4 octets + padding).
    let unpadded = out.len() + 11 + 4;
    let padding = (PAD_BLOCK - unpadded % PAD_BLOCK) % PAD_BLOCK;
    out.push(0);
    out.extend_from_slice(&TYPE_OPT.to_be_bytes());
    out.extend_from_slice(&EDNS_UDP_SIZE.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&((4 + padding) as u16).to_be_bytes());
    out.extend_from_slice(&EDNS_PADDING.to_be_bytes());
    out.extend_from_slice(&(padding as u16).to_be_bytes());
    out.resize(out.len() + padding, 0);
  }
  Some(out)
}

#[derive(Debug, PartialEq, Eq)]
struct Reply {
  rcode: u8,
  addresses: Vec<IpAddr>,
  /// Lowest TTL in the answer section, in seconds.
  ttl: u32,
}

fn be16(packet: &[u8], at: usize) -> Result<u16, String> {
  packet
    .get(at..at + 2)
    .map(|b| u16::from_be_bytes([b[0], b[1]]))
    .ok_or_else(|| "truncated reply".to_string())
}

fn skip_name(packet: &[u8], mut pos: usize) -> Result<usize, String> {
  for _ in 0..128 {
    let len = *packet.get(pos).ok_or("truncated name")? as usize;
    match len & 0xC0 {
      0x00 if len == 0 => return Ok(pos + 1),
      0x00 => pos += 1 + len,
      0xC0 => {
        packet.get(pos + 1).ok_or("truncated name")?;
        return Ok(pos + 2);
      }
      _ => return Err("malformed name".to_string()),
    }
  }
  Err("malformed name".to_string())
}

fn parse_reply(id: u16, qtype: u16, packet: &[u8]) -> Result<Reply, String> {
  if be16(packet, 0)? != id {
    return Err("reply to another query".to_string());
  }
  let flags = be16(packet, 2)?;
  if flags & 0x8000 == 0 {
    return Err("not a reply".to_string());
  }
  if flags & 0x0200 != 0 {
    return Err("truncated reply".to_string());
  }
  let rcode = (flags & 0x000F) as u8;
  let questions = be16(packet, 4)?;
  let answers = be16(packet, 6)?;
  let mut pos = 12;
  for _ in 0..questions {
    pos = skip_name(packet, pos)? + 4;
  }
  let mut addresses = Vec::new();
  let mut ttl = u32::MAX;
  for _ in 0..answers {
    pos = skip_name(packet, pos)?;
    let rtype = be16(packet, pos)?;
    let class = be16(packet, pos + 2)?;
    let record_ttl = (u32::from(be16(packet, pos + 4)?) << 16) | u32::from(be16(packet, pos + 6)?);
    let len = usize::from(be16(packet, pos + 8)?);
    pos += 10;
    let data = packet.get(pos..pos + len).ok_or("truncated record")?;
    pos += len;
    if class != CLASS_IN {
      continue;
    }
    // The CNAME records of the chain bound how long the address is good for.
    ttl = ttl.min(record_ttl);
    let address = match (rtype, data.len()) {
      (TYPE_A, 4) if rtype == qtype => IpAddr::from([data[0], data[1], data[2], data[3]]),
      (TYPE_AAAA, 16) if rtype == qtype => {
        let octets: [u8; 16] = data.try_into().map_err(|_| "malformed record")?;
        IpAddr::from(octets)
      }
      _ => continue,
    };
    if !addresses.contains(&address) {
      addresses.push(address);
    }
  }
  Ok(Reply {
    rcode,
    ttl: if addresses.is_empty() { 0 } else { ttl },
    addresses,
  })
}

// --- Resolver ---------------------------------------------------------------

/// One exchange once connected. Dialing has its own bound in the route.
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const TLS_TIMEOUT: Duration = Duration::from_secs(10);
/// Very short TTLs would cost a query through the proxy per connection.
const MIN_TTL: Duration = Duration::from_secs(30);
const MAX_TTL: Duration = Duration::from_secs(30 * 60);
const NEGATIVE_TTL: Duration = Duration::from_secs(30);
/// Lets a burst of connections to one name share a failure instead of each
/// waiting out its own timeout.
const FAILURE_TTL: Duration = Duration::from_secs(2);
const MAX_CACHED: usize = 4096;
const MAX_IDLE: usize = 4;
const IDLE_FOR: Duration = Duration::from_secs(120);
const MAX_QUERIES: usize = 8;

static SERVER_HEALTH: crate::log_streak::Streak =
  crate::log_streak::Streak::new(module_path!(), "Profile DNS server query");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
  /// The server answered: the name has no address.
  NotFound,
  /// No usable answer: the server is unreachable, refused, or failed.
  Failed(String),
}

impl std::fmt::Display for ResolveError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      ResolveError::NotFound => write!(f, "the profile DNS server has no address for the name"),
      ResolveError::Failed(e) => write!(f, "the profile DNS server did not answer: {e}"),
    }
  }
}

impl std::error::Error for ResolveError {}

/// True when a dial failed only because the name does not exist. That is an
/// answer, not a fault: a filtering resolver gives it for every blocked name.
pub fn is_missing_name(error: &(dyn std::error::Error + 'static)) -> bool {
  matches!(
    error.downcast_ref::<ResolveError>(),
    Some(ResolveError::NotFound)
  )
}

struct Cached {
  outcome: Result<Vec<IpAddr>, ResolveError>,
  until: Instant,
}

enum Conn {
  /// TCP or TLS, one length-prefixed exchange at a time.
  Stream(BoxedAsyncStream),
  Http1(hyper::client::conn::http1::SendRequest<Full<Bytes>>),
  /// Multiplexed: one connection carries every concurrent query.
  Http2(hyper::client::conn::http2::SendRequest<Full<Bytes>>),
}

impl Conn {
  fn is_closed(&self) -> bool {
    match self {
      Conn::Stream(_) => false,
      Conn::Http1(sender) => sender.is_closed(),
      Conn::Http2(sender) => sender.is_closed(),
    }
  }
}

struct Inner {
  server: DnsServer,
  upstream: Option<String>,
  cache: Mutex<HashMap<String, Cached>>,
  /// One lookup per name at a time; the others wait and read the cache.
  pending: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
  idle: Mutex<Vec<(Conn, Instant)>>,
  queries: tokio::sync::Semaphore,
}

/// Resolves a worker's destinations with its profile's DNS server, over the
/// worker's own upstream. Cheap to clone: clones share the cache and the
/// connections.
#[derive(Clone)]
pub struct ProfileResolver {
  inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
  mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn tls_handshake(
  stream: BoxedAsyncStream,
  name: &str,
  alpn: &[&str],
) -> Result<tokio_native_tls::TlsStream<BoxedAsyncStream>, String> {
  let mut builder = native_tls::TlsConnector::builder();
  if !alpn.is_empty() {
    builder.request_alpns(alpn);
  }
  let connector = tokio_native_tls::TlsConnector::from(builder.build().map_err(|e| e.to_string())?);
  tokio::time::timeout(TLS_TIMEOUT, connector.connect(name, stream))
    .await
    .map_err(|_| "TLS handshake timed out".to_string())?
    .map_err(|e| format!("TLS handshake failed: {e}"))
}

impl ProfileResolver {
  /// `upstream` is the worker's upstream URL; `None` dials the server directly.
  pub fn new(server: DnsServer, upstream: Option<String>) -> Self {
    Self {
      inner: Arc::new(Inner {
        server,
        upstream: upstream.filter(|u| u != "DIRECT"),
        cache: Mutex::new(HashMap::new()),
        pending: Mutex::new(HashMap::new()),
        idle: Mutex::new(Vec::new()),
        queries: tokio::sync::Semaphore::new(MAX_QUERIES),
      }),
    }
  }

  pub fn transport(&self) -> DnsTransport {
    self.inner.server.transport
  }

  /// Open the connection to the server while the browser is still starting,
  /// so the first page does not wait for the tunnel and the TLS handshake. A
  /// failure here is not logged: the first real query reports it.
  pub async fn warm_up(&self) {
    if let Ok(conn) = self.connect().await {
      self.put_idle(conn);
    }
  }

  /// Addresses for `host`, IPv4 first. An IP literal is returned as is.
  pub async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, ResolveError> {
    let host = host
      .strip_prefix('[')
      .and_then(|h| h.strip_suffix(']'))
      .unwrap_or(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
      return Ok(vec![ip]);
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    if let Some(outcome) = self.cached(&name) {
      return outcome;
    }

    let gate = lock(&self.inner.pending)
      .entry(name.clone())
      .or_default()
      .clone();
    let outcome = {
      let _turn = gate.lock().await;
      match self.cached(&name) {
        Some(outcome) => outcome,
        None => {
          let (outcome, ttl) = self.lookup(&name).await;
          self.store(&name, outcome.clone(), ttl);
          outcome
        }
      }
    };
    lock(&self.inner.pending).remove(&name);
    outcome
  }

  fn cached(&self, name: &str) -> Option<Result<Vec<IpAddr>, ResolveError>> {
    let cache = lock(&self.inner.cache);
    let entry = cache.get(name)?;
    (entry.until > Instant::now()).then(|| entry.outcome.clone())
  }

  fn store(&self, name: &str, outcome: Result<Vec<IpAddr>, ResolveError>, ttl: Duration) {
    let mut cache = lock(&self.inner.cache);
    if cache.len() >= MAX_CACHED {
      let now = Instant::now();
      cache.retain(|_, entry| entry.until > now);
      if cache.len() >= MAX_CACHED {
        cache.clear();
      }
    }
    cache.insert(
      name.to_string(),
      Cached {
        outcome,
        until: Instant::now() + ttl,
      },
    );
  }

  /// A first, then AAAA only when the name has no IPv4 address: most exits
  /// reach IPv4, and one query is the common case.
  async fn lookup(&self, name: &str) -> (Result<Vec<IpAddr>, ResolveError>, Duration) {
    for qtype in [TYPE_A, TYPE_AAAA] {
      match self.query(name, qtype).await {
        Ok(reply) if reply.rcode == RCODE_NOERROR && !reply.addresses.is_empty() => {
          let ttl = Duration::from_secs(u64::from(reply.ttl)).clamp(MIN_TTL, MAX_TTL);
          return (Ok(reply.addresses), ttl);
        }
        Ok(reply) if reply.rcode == RCODE_NOERROR => continue,
        Ok(reply) if reply.rcode == RCODE_NXDOMAIN => {
          return (Err(ResolveError::NotFound), NEGATIVE_TTL)
        }
        Ok(reply) => {
          return (
            Err(ResolveError::Failed(format!(
              "server answered rcode={}",
              reply.rcode
            ))),
            FAILURE_TTL,
          )
        }
        Err(e) => return (Err(e), FAILURE_TTL),
      }
    }
    (Err(ResolveError::NotFound), NEGATIVE_TTL)
  }

  async fn query(&self, name: &str, qtype: u16) -> Result<Reply, ResolveError> {
    let transport = self.inner.server.transport;
    // RFC 8484 asks for ID 0 over HTTPS; elsewhere the ID matches the reply.
    let id = if transport == DnsTransport::Https {
      0
    } else {
      rand::random::<u16>()
    };
    let Some(message) = encode_query(id, name, qtype, transport != DnsTransport::Tcp) else {
      return Err(ResolveError::NotFound);
    };
    let _slot = self
      .inner
      .queries
      .acquire()
      .await
      .map_err(|_| ResolveError::Failed("resolver closed".to_string()))?;

    let result = self.exchange_with_retry(&message).await;
    let result = result.and_then(|answer| parse_reply(id, qtype, &answer));
    // The transport is on the worker's start line.
    match &result {
      Ok(_) => SERVER_HEALTH.succeeded(),
      Err(e) => SERVER_HEALTH.failed(e),
    }
    result.map_err(ResolveError::Failed)
  }

  /// A pooled connection may have been closed by the server while it sat
  /// idle, so a failure on one is retried once on a fresh connection.
  async fn exchange_with_retry(&self, message: &[u8]) -> Result<Vec<u8>, String> {
    let mut pooled = self.take_idle();
    loop {
      let reused = pooled.is_some();
      let conn = match pooled.take() {
        Some(conn) => conn,
        None => self.connect().await?,
      };
      match tokio::time::timeout(
        QUERY_TIMEOUT,
        Self::exchange(&self.inner.server, conn, message),
      )
      .await
      {
        Ok(Ok((answer, conn))) => {
          self.put_idle(conn);
          return Ok(answer);
        }
        Ok(Err(_)) | Err(_) if reused => continue,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("query timed out".to_string()),
      }
    }
  }

  fn take_idle(&self) -> Option<Conn> {
    let mut idle = lock(&self.inner.idle);
    idle.retain(|(conn, since)| since.elapsed() < IDLE_FOR && !conn.is_closed());
    if let Some((Conn::Http2(sender), _)) = idle.iter().find(|(c, _)| matches!(c, Conn::Http2(_))) {
      return Some(Conn::Http2(sender.clone()));
    }
    idle.pop().map(|(conn, _)| conn)
  }

  fn put_idle(&self, conn: Conn) {
    let mut idle = lock(&self.inner.idle);
    if matches!(conn, Conn::Http2(_)) {
      // The pool keeps one handle to a multiplexed connection: the one that
      // just answered. The handle it replaces may be to a connection that
      // failed and made the caller dial this one.
      if let Some(entry) = idle.iter_mut().find(|(c, _)| matches!(c, Conn::Http2(_))) {
        *entry = (conn, Instant::now());
        return;
      }
    }
    if idle.len() < MAX_IDLE {
      idle.push((conn, Instant::now()));
    }
  }

  async fn connect(&self) -> Result<Conn, String> {
    let server = &self.inner.server;
    let stream = crate::proxy_server::dial_route(
      &server.dial_host,
      server.port,
      self.inner.upstream.as_deref(),
      false,
    )
    .await
    .map_err(|e| e.to_string())?;
    match server.transport {
      DnsTransport::Tcp => Ok(Conn::Stream(stream)),
      DnsTransport::Tls => Ok(Conn::Stream(Box::new(
        tls_handshake(stream, &server.tls_name, &[]).await?,
      ))),
      DnsTransport::Https => {
        let tls = tls_handshake(stream, &server.tls_name, &["h2", "http/1.1"]).await?;
        let h2 =
          matches!(tls.get_ref().negotiated_alpn(), Ok(Some(ref protocol)) if protocol == b"h2");
        let io = TokioIo::new(tls);
        if h2 {
          let (sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), io)
              .await
              .map_err(|e| format!("HTTP/2 handshake failed: {e}"))?;
          tokio::spawn(async move {
            let _ = connection.await;
          });
          Ok(Conn::Http2(sender))
        } else {
          let (sender, connection) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| format!("HTTP/1.1 handshake failed: {e}"))?;
          tokio::spawn(async move {
            let _ = connection.await;
          });
          Ok(Conn::Http1(sender))
        }
      }
    }
  }

  async fn exchange(
    server: &DnsServer,
    conn: Conn,
    message: &[u8],
  ) -> Result<(Vec<u8>, Conn), String> {
    match conn {
      Conn::Stream(mut stream) => {
        let mut framed = Vec::with_capacity(message.len() + 2);
        framed.extend_from_slice(&(message.len() as u16).to_be_bytes());
        framed.extend_from_slice(message);
        stream.write_all(&framed).await.map_err(|e| e.to_string())?;
        stream.flush().await.map_err(|e| e.to_string())?;
        let mut len = [0u8; 2];
        stream
          .read_exact(&mut len)
          .await
          .map_err(|e| e.to_string())?;
        let mut answer = vec![0u8; usize::from(u16::from_be_bytes(len))];
        stream
          .read_exact(&mut answer)
          .await
          .map_err(|e| e.to_string())?;
        Ok((answer, Conn::Stream(stream)))
      }
      Conn::Http1(mut sender) => {
        sender.ready().await.map_err(|e| e.to_string())?;
        let request = doh_request(server, message, false)?;
        let response = sender
          .send_request(request)
          .await
          .map_err(|e| e.to_string())?;
        Ok((read_doh_response(response).await?, Conn::Http1(sender)))
      }
      Conn::Http2(mut sender) => {
        sender.ready().await.map_err(|e| e.to_string())?;
        let request = doh_request(server, message, true)?;
        let response = sender
          .send_request(request)
          .await
          .map_err(|e| e.to_string())?;
        Ok((read_doh_response(response).await?, Conn::Http2(sender)))
      }
    }
  }
}

/// HTTP/2 takes the authority from the URI; HTTP/1.1 from `Host`, with the
/// origin-form target every server accepts.
fn doh_request(
  server: &DnsServer,
  message: &[u8],
  absolute: bool,
) -> Result<Request<Full<Bytes>>, String> {
  let authority = server.authority();
  let builder = if absolute {
    Request::post(format!("https://{authority}{}", server.path))
  } else {
    Request::post(server.path.as_str()).header(hyper::header::HOST, authority)
  };
  builder
    .header(hyper::header::CONTENT_TYPE, "application/dns-message")
    .header(hyper::header::ACCEPT, "application/dns-message")
    .body(Full::new(Bytes::copy_from_slice(message)))
    .map_err(|e| e.to_string())
}

async fn read_doh_response(
  response: hyper::Response<hyper::body::Incoming>,
) -> Result<Vec<u8>, String> {
  if response.status() != StatusCode::OK {
    return Err(format!(
      "server answered HTTP {}",
      response.status().as_u16()
    ));
  }
  let is_dns_message = response
    .headers()
    .get(hyper::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .is_some_and(|v| {
      v.trim()
        .to_ascii_lowercase()
        .starts_with("application/dns-message")
    });
  if !is_dns_message {
    return Err("server answered with something other than a DNS message".to_string());
  }
  let body = Limited::new(response.into_body(), MAX_MESSAGE)
    .collect()
    .await
    .map_err(|e| e.to_string())?;
  Ok(body.to_bytes().to_vec())
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokio::net::TcpListener;

  fn summary(input: &str) -> DnsServerSummary {
    DnsServer::parse(input).unwrap().summary()
  }

  #[test]
  fn a_known_plain_address_is_upgraded_to_its_encrypted_endpoint() {
    for input in [
      "1.1.1.1",
      " 1.1.1.1 ",
      "1.1.1.1:53",
      "[2606:4700:4700::1111]:53",
    ] {
      let server = DnsServer::parse(input).unwrap();
      assert_eq!(server.transport, DnsTransport::Https, "{input}");
      assert_eq!(server.tls_name, "cloudflare-dns.com");
      assert_eq!(server.port, 443);
      assert_eq!(server.path, "/dns-query");
      assert!(server.upgraded);
    }
    // The address the person chose is the one dialed.
    assert_eq!(DnsServer::parse("1.0.0.1").unwrap().dial_host, "1.0.0.1");
    assert_eq!(summary("9.9.9.9").server, "dns.quad9.net");
    assert_eq!(summary("dns.quad9.net").server, "dns.quad9.net");
    assert_eq!(
      DnsServer::parse("dns.quad9.net").unwrap().dial_host,
      "9.9.9.9"
    );
  }

  #[test]
  fn an_explicit_scheme_or_port_is_never_upgraded() {
    let plain = DnsServer::parse("tcp://1.1.1.1").unwrap();
    assert_eq!(plain.transport, DnsTransport::Tcp);
    assert_eq!(plain.port, 53);
    assert!(!plain.summary().encrypted);

    let other_port = DnsServer::parse("1.1.1.1:5353").unwrap();
    assert_eq!(other_port.transport, DnsTransport::Tcp);
    assert_eq!(other_port.summary().server, "1.1.1.1:5353");
  }

  #[test]
  fn an_unknown_plain_address_stays_plain_dns_over_tcp() {
    let server = DnsServer::parse("10.0.0.53").unwrap();
    assert_eq!(server.transport, DnsTransport::Tcp);
    assert_eq!(
      summary("10.0.0.53"),
      DnsServerSummary {
        transport: DnsTransport::Tcp,
        server: "10.0.0.53".to_string(),
        encrypted: false,
        upgraded: false,
      }
    );
    assert_eq!(summary("2001:db8::53").server, "2001:db8::53");
    assert_eq!(summary("[2001:db8::53]:5353").server, "[2001:db8::53]:5353");
    assert_eq!(
      summary("Resolver.Example:5353").server,
      "resolver.example:5353"
    );
  }

  #[test]
  fn https_urls_keep_their_path_and_query() {
    let server = DnsServer::parse("https://dns.example/abc123?x=1").unwrap();
    assert_eq!(server.transport, DnsTransport::Https);
    assert_eq!(server.path, "/abc123?x=1");
    assert_eq!(server.dial_host, "dns.example");
    assert_eq!(server.tls_name, "dns.example");

    let template = DnsServer::parse("https://dns.example/dns-query{?dns}").unwrap();
    assert_eq!(template.path, "/dns-query");
    assert_eq!(
      DnsServer::parse("https://dns.example").unwrap().path,
      "/dns-query"
    );
    assert_eq!(
      summary("https://dns.example:8443/q").server,
      "dns.example:8443"
    );
    // A known provider by name is dialed at its address and keeps its name.
    let known = DnsServer::parse("https://dns.mullvad.net/dns-query").unwrap();
    assert_eq!(known.dial_host, "194.242.2.2");
    assert_eq!(known.tls_name, "dns.mullvad.net");
    assert!(!known.upgraded);
  }

  #[test]
  fn tls_addresses_default_to_port_853() {
    let server = DnsServer::parse("tls://1.1.1.1").unwrap();
    assert_eq!(server.transport, DnsTransport::Tls);
    assert_eq!(server.port, 853);
    assert_eq!(server.dial_host, "1.1.1.1");
    assert_eq!(server.tls_name, "cloudflare-dns.com");
    assert_eq!(summary("tls://dns.example:8853").server, "dns.example:8853");
  }

  #[test]
  fn unusable_input_is_rejected() {
    for input in [
      "",
      "   ",
      "udp://1.1.1.1",
      "quic://dns.example",
      "sdns://AgcAAAAAAAAA",
      "https://user:secret@dns.example/dns-query",
      "https://dns.example/dns-query#x",
      "tls://dns.example/path",
      "tcp://1.1.1.1/?q",
      "1.1.1.1:0",
      "1.1.1.1:99999",
      "dns example",
      "-bad.example",
      "dns..example",
      "dnś.example",
      "https://",
    ] {
      assert_eq!(DnsServer::parse(input), Err(InvalidDnsServer), "{input:?}");
    }
    assert_eq!(
      DnsServer::parse(&format!("https://dns.example/{}", "a".repeat(600))),
      Err(InvalidDnsServer)
    );
  }

  #[test]
  fn normalize_trims_and_maps_blank_to_none() {
    assert_eq!(normalize_dns_server(None), Ok(None));
    assert_eq!(normalize_dns_server(Some("  ".to_string())), Ok(None));
    assert_eq!(
      normalize_dns_server(Some(" 9.9.9.9 ".to_string())),
      Ok(Some("9.9.9.9".to_string()))
    );
    let err = normalize_dns_server(Some("udp://9.9.9.9".to_string())).unwrap_err();
    assert!(err.contains("DNS_SERVER_INVALID"));
    assert_eq!(describe_dns_server(" ".to_string()), Ok(None));
    assert!(describe_dns_server("bogus value".to_string()).is_err());
  }

  #[test]
  fn every_known_resolver_address_parses() {
    for known in KNOWN_RESOLVERS {
      assert!(valid_hostname(known.name));
      for address in known.addresses {
        let ip: IpAddr = address.parse().unwrap();
        assert_eq!(known_by_address(ip).map(|k| k.name), Some(known.name));
      }
    }
  }

  #[test]
  fn encrypted_queries_are_padded_to_the_block_size() {
    for name in [
      "a.b",
      "example.com",
      &"x".repeat(63),
      "www.a-much-longer-name.example.org",
    ] {
      let padded = encode_query(0, name, TYPE_A, true).unwrap();
      assert_eq!(padded.len() % PAD_BLOCK, 0, "{name}");
      let plain = encode_query(7, name, TYPE_A, false).unwrap();
      assert_eq!(plain.len(), 12 + name.len() + 2 + 4);
      assert_eq!(&plain[10..12], &[0, 0], "no OPT record when unpadded");
    }
    assert!(encode_query(0, "", TYPE_A, false).is_none());
    assert!(encode_query(0, "bad name.example", TYPE_A, false).is_none());
    assert!(encode_query(0, &format!("{}.example", "x".repeat(64)), TYPE_A, false).is_none());
  }

  /// A reply to `query` carrying `records` as (type, ttl, rdata), the first
  /// owner name compressed to the question.
  fn reply_for(query: &[u8], rcode: u8, records: &[(u16, u32, Vec<u8>)]) -> Vec<u8> {
    let question_end = skip_name(query, 12).unwrap() + 4;
    let mut out = query[..question_end].to_vec();
    out[2] = 0x81;
    out[3] = 0x80 | rcode;
    out[6..8].copy_from_slice(&(records.len() as u16).to_be_bytes());
    out[10..12].copy_from_slice(&[0, 0]);
    for (rtype, ttl, data) in records {
      out.extend_from_slice(&[0xC0, 12]);
      out.extend_from_slice(&rtype.to_be_bytes());
      out.extend_from_slice(&CLASS_IN.to_be_bytes());
      out.extend_from_slice(&ttl.to_be_bytes());
      out.extend_from_slice(&(data.len() as u16).to_be_bytes());
      out.extend_from_slice(data);
    }
    out
  }

  #[test]
  fn replies_yield_the_addresses_of_the_asked_type_and_the_lowest_ttl() {
    let query = encode_query(0x1234, "example.com", TYPE_A, false).unwrap();
    let cname = vec![3, b'w', b'w', b'w', 0xC0, 12];
    let reply = reply_for(
      &query,
      0,
      &[
        (5, 60, cname),
        (TYPE_A, 300, vec![93, 184, 215, 14]),
        (TYPE_A, 300, vec![93, 184, 215, 14]),
        (TYPE_AAAA, 10, vec![0; 16]),
        (TYPE_A, 120, vec![10, 0, 0, 1]),
      ],
    );
    let parsed = parse_reply(0x1234, TYPE_A, &reply).unwrap();
    assert_eq!(
      parsed.addresses,
      vec![
        IpAddr::from([93, 184, 215, 14]),
        IpAddr::from([10, 0, 0, 1])
      ]
    );
    assert_eq!(parsed.rcode, RCODE_NOERROR);
    assert_eq!(parsed.ttl, 10);

    assert!(parse_reply(0x9999, TYPE_A, &reply).is_err(), "wrong id");
    assert!(parse_reply(0x1234, TYPE_A, &reply[..reply.len() - 2]).is_err());
    assert!(
      parse_reply(0x1234, TYPE_A, &query).is_err(),
      "a query is not a reply"
    );
    let mut truncated = reply.clone();
    truncated[2] |= 0x02;
    assert!(parse_reply(0x1234, TYPE_A, &truncated).is_err());

    let nxdomain = reply_for(&query, RCODE_NXDOMAIN, &[]);
    let parsed = parse_reply(0x1234, TYPE_A, &nxdomain).unwrap();
    assert_eq!(parsed.rcode, RCODE_NXDOMAIN);
    assert!(parsed.addresses.is_empty());
  }

  #[test]
  fn a_compression_loop_is_rejected_not_followed() {
    let mut packet = vec![0u8; 12];
    packet[4..6].copy_from_slice(&1u16.to_be_bytes());
    packet.extend_from_slice(&[0x01, b'a']);
    packet.extend(std::iter::repeat_n(0x01, 300));
    assert!(skip_name(&packet, 12).is_err());
    assert!(skip_name(&[0x40, 0, 0], 0).is_err());
  }

  type Answer = (u8, Vec<(u16, u32, Vec<u8>)>);

  fn query_name(query: &[u8]) -> (String, u16) {
    let mut name = String::new();
    let mut pos = 12;
    while query[pos] != 0 {
      let len = query[pos] as usize;
      if !name.is_empty() {
        name.push('.');
      }
      name.push_str(std::str::from_utf8(&query[pos + 1..pos + 1 + len]).unwrap());
      pos += 1 + len;
    }
    (name, u16::from_be_bytes([query[pos + 1], query[pos + 2]]))
  }

  /// A DNS-over-TCP server that answers with `answer(name, qtype)` and counts
  /// the queries it receives.
  async fn start_dns_server(
    answer: fn(&str, u16) -> Answer,
  ) -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let queries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = queries.clone();
    tokio::spawn(async move {
      while let Ok((mut stream, _)) = listener.accept().await {
        let counter = counter.clone();
        tokio::spawn(async move {
          loop {
            let mut len = [0u8; 2];
            if stream.read_exact(&mut len).await.is_err() {
              return;
            }
            let mut query = vec![0u8; usize::from(u16::from_be_bytes(len))];
            if stream.read_exact(&mut query).await.is_err() {
              return;
            }
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (name, qtype) = query_name(&query);
            let (rcode, records) = answer(&name, qtype);
            let reply = reply_for(&query, rcode, &records);
            let mut framed = (reply.len() as u16).to_be_bytes().to_vec();
            framed.extend_from_slice(&reply);
            if stream.write_all(&framed).await.is_err() {
              return;
            }
          }
        });
      }
    });
    (port, queries)
  }

  fn tcp_resolver(port: u16) -> ProfileResolver {
    let server = DnsServer::parse(&format!("tcp://127.0.0.1:{port}")).unwrap();
    ProfileResolver::new(server, None)
  }

  #[tokio::test]
  async fn resolves_over_tcp_and_serves_repeats_from_the_cache() {
    let (port, queries) = start_dns_server(|name, qtype| match (name, qtype) {
      ("site.test", TYPE_A) => (0, vec![(TYPE_A, 600, vec![192, 0, 2, 7])]),
      ("site.test", _) => (0, Vec::new()),
      _ => (RCODE_NXDOMAIN, Vec::new()),
    })
    .await;
    let resolver = tcp_resolver(port);

    let first = resolver.resolve("Site.Test.").await.unwrap();
    assert_eq!(first, vec![IpAddr::from([192, 0, 2, 7])]);
    let concurrent: Vec<_> =
      futures_util::future::join_all((0..8).map(|_| resolver.resolve("site.test"))).await;
    assert!(concurrent.iter().all(|r| r.as_deref() == Ok(&first[..])));
    assert_eq!(queries.load(std::sync::atomic::Ordering::SeqCst), 1);

    assert_eq!(
      resolver.resolve("missing.test").await,
      Err(ResolveError::NotFound)
    );
    assert_eq!(
      resolver.resolve("192.0.2.99").await,
      Ok(vec![IpAddr::from([192, 0, 2, 99])])
    );
    assert_eq!(
      resolver.resolve("[2001:db8::1]").await,
      Ok(vec!["2001:db8::1".parse().unwrap()])
    );
    assert_eq!(queries.load(std::sync::atomic::Ordering::SeqCst), 2);
  }

  #[tokio::test]
  async fn a_name_without_ipv4_is_asked_for_ipv6() {
    let (port, queries) = start_dns_server(|_, qtype| match qtype {
      TYPE_AAAA => (
        0,
        vec![(
          TYPE_AAAA,
          600,
          vec![0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        )],
      ),
      _ => (0, Vec::new()),
    })
    .await;
    let resolver = tcp_resolver(port);
    assert_eq!(
      resolver.resolve("v6only.test").await,
      Ok(vec!["2001:db8::1".parse().unwrap()])
    );
    assert_eq!(queries.load(std::sync::atomic::Ordering::SeqCst), 2);
  }

  #[tokio::test]
  async fn a_server_failure_is_not_read_as_a_missing_name() {
    let (port, _) = start_dns_server(|_, _| (2, Vec::new())).await;
    assert!(matches!(
      tcp_resolver(port).resolve("site.test").await,
      Err(ResolveError::Failed(_))
    ));
  }

  #[tokio::test]
  async fn an_unreachable_server_fails_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    // On [::1], not 127.0.0.1: the direct dial backs off per host for the
    // whole process, and the other tests here dial 127.0.0.1 in parallel.
    let server = DnsServer::parse(&format!("tcp://[::1]:{port}")).unwrap();
    assert!(matches!(
      ProfileResolver::new(server, None)
        .resolve("site.test")
        .await,
      Err(ResolveError::Failed(_))
    ));
  }

  #[tokio::test]
  async fn the_warm_connection_carries_the_first_query() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
      while let Ok((mut stream, _)) = listener.accept().await {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::spawn(async move {
          let mut len = [0u8; 2];
          while stream.read_exact(&mut len).await.is_ok() {
            let mut query = vec![0u8; usize::from(u16::from_be_bytes(len))];
            stream.read_exact(&mut query).await.unwrap();
            let reply = reply_for(&query, 0, &[(TYPE_A, 600, vec![192, 0, 2, 9])]);
            let mut framed = (reply.len() as u16).to_be_bytes().to_vec();
            framed.extend_from_slice(&reply);
            stream.write_all(&framed).await.unwrap();
          }
        });
      }
    });
    let resolver = tcp_resolver(port);
    resolver.warm_up().await;
    assert_eq!(
      resolver.resolve("warm.test").await,
      Ok(vec![IpAddr::from([192, 0, 2, 9])])
    );
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
  }

  #[tokio::test]
  async fn a_server_that_closed_an_idle_connection_is_redialed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // Answers exactly one query per connection, then closes it.
    tokio::spawn(async move {
      while let Ok((mut stream, _)) = listener.accept().await {
        tokio::spawn(async move {
          let mut len = [0u8; 2];
          stream.read_exact(&mut len).await.unwrap();
          let mut query = vec![0u8; usize::from(u16::from_be_bytes(len))];
          stream.read_exact(&mut query).await.unwrap();
          let reply = reply_for(&query, 0, &[(TYPE_A, 600, vec![192, 0, 2, 1])]);
          let mut framed = (reply.len() as u16).to_be_bytes().to_vec();
          framed.extend_from_slice(&reply);
          stream.write_all(&framed).await.unwrap();
        });
      }
    });
    let resolver = tcp_resolver(port);
    assert!(resolver.resolve("one.test").await.is_ok());
    assert!(resolver.resolve("two.test").await.is_ok());
  }

  /// A plain-text DoH endpoint: checks the request the way a real server
  /// would and answers every A query with 192.0.2.53.
  async fn doh_service(
    request: Request<hyper::body::Incoming>,
  ) -> Result<hyper::Response<Full<Bytes>>, std::convert::Infallible> {
    let well_formed = request.method() == hyper::Method::POST
      && request.uri().path() == "/dns-query"
      && request
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .map(|v| v.as_bytes())
        == Some(b"application/dns-message".as_slice())
      && (request.version() == hyper::Version::HTTP_2
        || request.headers().contains_key(hyper::header::HOST));
    let query = request.into_body().collect().await.unwrap().to_bytes();
    if !well_formed || query.len() % PAD_BLOCK != 0 {
      let mut response = hyper::Response::new(Full::new(Bytes::new()));
      *response.status_mut() = StatusCode::BAD_REQUEST;
      return Ok(response);
    }
    let reply = reply_for(&query, 0, &[(TYPE_A, 300, vec![192, 0, 2, 53])]);
    Ok(
      hyper::Response::builder()
        .header(hyper::header::CONTENT_TYPE, "application/dns-message")
        .body(Full::new(Bytes::from(reply)))
        .unwrap(),
    )
  }

  #[tokio::test]
  async fn doh_queries_are_well_formed_over_http1_and_http2() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
      while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(async move {
          let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
            .serve_connection(
              TokioIo::new(stream),
              hyper::service::service_fn(doh_service),
            )
            .await;
        });
      }
    });
    let server = DnsServer::parse("https://doh.test/dns-query").unwrap();
    let query = encode_query(0, "example.com", TYPE_A, true).unwrap();

    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
      .await
      .unwrap();
    let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
      .await
      .unwrap();
    tokio::spawn(connection);
    let (answer, conn) = ProfileResolver::exchange(&server, Conn::Http1(sender), &query)
      .await
      .unwrap();
    let reply = parse_reply(0, TYPE_A, &answer).unwrap();
    assert_eq!(reply.addresses, vec![IpAddr::from([192, 0, 2, 53])]);
    // The same HTTP/1.1 connection carries the next query.
    let (answer, _) = ProfileResolver::exchange(&server, conn, &query)
      .await
      .unwrap();
    assert!(parse_reply(0, TYPE_A, &answer).is_ok());

    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
      .await
      .unwrap();
    let (sender, connection) =
      hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(connection);
    let (answer, _) = ProfileResolver::exchange(&server, Conn::Http2(sender), &query)
      .await
      .unwrap();
    let reply = parse_reply(0, TYPE_A, &answer).unwrap();
    assert_eq!(reply.addresses, vec![IpAddr::from([192, 0, 2, 53])]);
  }

  /// Real public resolvers over every transport. Needs the internet, so it
  /// runs only when asked: `cargo test profile_dns -- --ignored`.
  #[tokio::test]
  #[ignore]
  async fn live_public_resolvers_answer_over_every_transport() {
    for input in [
      "1.1.1.1",
      "9.9.9.9",
      "194.242.2.2",
      "8.8.8.8",
      "94.140.14.14",
      "208.67.222.222",
      "https://dns.quad9.net/dns-query",
      "https://cloudflare-dns.com/dns-query",
      "tls://1.1.1.1",
      "tls://dns.quad9.net",
      "tcp://1.1.1.1",
    ] {
      let resolver = ProfileResolver::new(DnsServer::parse(input).unwrap(), None);
      let addresses = resolver.resolve("example.com").await;
      assert!(
        addresses.as_ref().is_ok_and(|a| !a.is_empty()),
        "{input}: {addresses:?}"
      );
      assert_eq!(
        resolver.resolve("does-not-exist.invalid").await,
        Err(ResolveError::NotFound),
        "{input}"
      );
    }
  }
}

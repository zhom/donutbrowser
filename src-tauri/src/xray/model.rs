use std::net::IpAddr;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use url::Host;
use uuid::Uuid;

use super::{XrayError, XrayResult};

const MAX_SPIDER_X_BYTES: usize = 2048;
const MAX_PATH_BYTES: usize = 2048;
const MAX_SECRET_BYTES: usize = 1024;
const MAX_ALPN_BYTES: usize = 255;

/// A protocol an Xray-core sidecar carries for Donut. Each one is at the same
/// time the stored `proxy_type` and the scheme of its share link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrayProtocol {
  Vless,
  Vmess,
  Trojan,
  Hysteria2,
}

impl XrayProtocol {
  pub const fn proxy_type(self) -> &'static str {
    match self {
      Self::Vless => "vless",
      Self::Vmess => "vmess",
      Self::Trojan => "trojan",
      Self::Hysteria2 => "hysteria2",
    }
  }

  /// The protocol as the user would name it in a sentence.
  pub const fn display_name(self) -> &'static str {
    match self {
      Self::Vless => "VLESS",
      Self::Vmess => "VMess",
      Self::Trojan => "Trojan",
      Self::Hysteria2 => "Hysteria2",
    }
  }

  /// Also reads a stored `proxy_type`, which the REST API keeps verbatim.
  /// `hy2` is the short scheme Hysteria2 clients export.
  pub fn from_scheme(scheme: &str) -> Option<Self> {
    match scheme.trim().to_ascii_lowercase().as_str() {
      "vless" => Some(Self::Vless),
      "vmess" => Some(Self::Vmess),
      "trojan" => Some(Self::Trojan),
      "hysteria2" | "hy2" => Some(Self::Hysteria2),
      _ => None,
    }
  }

  /// The protocol a share link names, read from its scheme alone.
  pub fn from_share_link(uri: &str) -> Option<Self> {
    let (scheme, _) = uri.trim().split_once("://")?;
    Self::from_scheme(scheme)
  }
}

/// Whether a stored proxy is carried by an Xray-core sidecar rather than by a
/// `donut-proxy` worker.
pub fn is_xray_proxy_type(proxy_type: &str) -> bool {
  XrayProtocol::from_scheme(proxy_type).is_some()
}

/// The uTLS presets Xray-core offers to GUI clients. `unsafe` is left out on
/// purpose: it turns uTLS off and gives a Go TLS hello that is easy to spot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Fingerprint {
  #[default]
  Chrome,
  Firefox,
  Safari,
  Ios,
  Android,
  Edge,
  Browser360,
  Qq,
  Random,
  Randomized,
  RandomizedNoAlpn,
}

impl Fingerprint {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Chrome => "chrome",
      Self::Firefox => "firefox",
      Self::Safari => "safari",
      Self::Ios => "ios",
      Self::Android => "android",
      Self::Edge => "edge",
      Self::Browser360 => "360",
      Self::Qq => "qq",
      Self::Random => "random",
      Self::Randomized => "randomized",
      Self::RandomizedNoAlpn => "randomizednoalpn",
    }
  }

  pub(crate) fn parse(value: &str) -> XrayResult<Self> {
    match value.to_ascii_lowercase().as_str() {
      "chrome" => Ok(Self::Chrome),
      "firefox" => Ok(Self::Firefox),
      "safari" => Ok(Self::Safari),
      "ios" => Ok(Self::Ios),
      "android" => Ok(Self::Android),
      "edge" => Ok(Self::Edge),
      "360" => Ok(Self::Browser360),
      "qq" => Ok(Self::Qq),
      "random" => Ok(Self::Random),
      "randomized" => Ok(Self::Randomized),
      "randomizednoalpn" => Ok(Self::RandomizedNoAlpn),
      _ => Err(XrayError::UnsupportedValue {
        field: "fp",
        expected: "chrome, firefox, safari, ios, android, edge, 360, qq, random, randomized or randomizednoalpn",
      }),
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VlessFlow {
  Vision,
  VisionUdp443,
}

impl VlessFlow {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Vision => "xtls-rprx-vision",
      Self::VisionUdp443 => "xtls-rprx-vision-udp443",
    }
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VmessSecurity {
  #[default]
  Auto,
  Aes128Gcm,
  Chacha20Poly1305,
  None,
  Zero,
}

impl VmessSecurity {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Auto => "auto",
      Self::Aes128Gcm => "aes-128-gcm",
      Self::Chacha20Poly1305 => "chacha20-poly1305",
      Self::None => "none",
      Self::Zero => "zero",
    }
  }

  pub(crate) fn parse(value: &str) -> XrayResult<Self> {
    match value.to_ascii_lowercase().as_str() {
      "auto" => Ok(Self::Auto),
      "aes-128-gcm" => Ok(Self::Aes128Gcm),
      "chacha20-poly1305" => Ok(Self::Chacha20Poly1305),
      "none" => Ok(Self::None),
      "zero" => Ok(Self::Zero),
      _ => Err(XrayError::UnsupportedValue {
        field: "encryption",
        expected: "auto, aes-128-gcm, chacha20-poly1305, none or zero",
      }),
    }
  }
}

/// TLS towards the server. Certificate checks always stay on: Xray-core
/// refuses `allowInsecure` since 2026-06-01, and a self-signed server is
/// reached with a pinned certificate hash instead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsSettings {
  pub server_name: Option<String>,
  pub alpn: Vec<String>,
  pub fingerprint: Option<Fingerprint>,
  /// Lowercase hex SHA-256 of a certificate in the server's chain.
  pub pinned_peer_cert_sha256: Vec<String>,
  pub verify_peer_cert_by_name: Vec<String>,
}

impl TlsSettings {
  pub fn validate(&self) -> XrayResult<()> {
    if let Some(server_name) = &self.server_name {
      validate_tls_server_name(server_name)?;
    }
    for protocol in &self.alpn {
      validate_alpn(protocol)?;
    }
    for pin in &self.pinned_peer_cert_sha256 {
      validate_certificate_pin(pin)?;
    }
    for name in &self.verify_peer_cert_by_name {
      validate_dns_name(name, "vcn")?;
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealitySettings {
  pub server_name: String,
  pub public_key: String,
  pub short_id: String,
  pub fingerprint: Fingerprint,
  pub spider_x: String,
}

impl RealitySettings {
  pub fn validate(&self) -> XrayResult<()> {
    validate_dns_name(&self.server_name, "sni")?;
    validate_public_key(&self.public_key)?;
    validate_short_id(&self.short_id)?;
    validate_spider_x(&self.spider_x)
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Security {
  None,
  Tls(TlsSettings),
  Reality(RealitySettings),
}

impl Security {
  pub const fn as_str(&self) -> &'static str {
    match self {
      Self::None => "none",
      Self::Tls(_) => "tls",
      Self::Reality(_) => "reality",
    }
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GrpcMode {
  #[default]
  Gun,
  Multi,
}

impl GrpcMode {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Gun => "gun",
      Self::Multi => "multi",
    }
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum XhttpMode {
  #[default]
  Auto,
  PacketUp,
  StreamUp,
  StreamOne,
}

impl XhttpMode {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Auto => "auto",
      Self::PacketUp => "packet-up",
      Self::StreamUp => "stream-up",
      Self::StreamOne => "stream-one",
    }
  }
}

/// The HTTP/1.1 request disguise RAW can put in front of a connection
/// (`headerType=http`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpHeader {
  pub hosts: Vec<String>,
  pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
  Raw {
    http_header: Option<HttpHeader>,
  },
  WebSocket {
    host: Option<String>,
    path: String,
  },
  HttpUpgrade {
    host: Option<String>,
    path: String,
  },
  Grpc {
    service_name: String,
    authority: Option<String>,
    mode: GrpcMode,
  },
  Xhttp {
    host: Option<String>,
    path: String,
    mode: XhttpMode,
  },
}

impl Transport {
  /// The share-link `type` value.
  pub const fn as_str(&self) -> &'static str {
    match self {
      Self::Raw { .. } => "tcp",
      Self::WebSocket { .. } => "ws",
      Self::HttpUpgrade { .. } => "httpupgrade",
      Self::Grpc { .. } => "grpc",
      Self::Xhttp { .. } => "xhttp",
    }
  }

  /// The Host header the transport sends, which CDN fronted links also expect
  /// as the TLS server name when they do not give one.
  pub fn host(&self) -> Option<&str> {
    match self {
      Self::WebSocket { host, .. } | Self::HttpUpgrade { host, .. } | Self::Xhttp { host, .. } => {
        host.as_deref()
      }
      Self::Raw { .. } | Self::Grpc { .. } => None,
    }
  }

  fn validate(&self) -> XrayResult<()> {
    match self {
      Self::Raw { http_header: None } => Ok(()),
      Self::Raw {
        http_header: Some(header),
      } => {
        for host in &header.hosts {
          validate_host_header(host)?;
        }
        validate_path(&header.path)
      }
      Self::WebSocket { host, path }
      | Self::HttpUpgrade { host, path }
      | Self::Xhttp { host, path, .. } => {
        if let Some(host) = host {
          validate_host_header(host)?;
        }
        validate_path(path)
      }
      Self::Grpc {
        service_name,
        authority,
        ..
      } => {
        if service_name.len() > MAX_PATH_BYTES
          || service_name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        {
          return Err(XrayError::InvalidField {
            field: "serviceName",
            reason: "must not contain whitespace or exceed 2048 bytes",
          });
        }
        if let Some(authority) = authority {
          validate_host_header(authority)?;
        }
        Ok(())
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSettings {
  pub transport: Transport,
  pub security: Security,
}

impl StreamSettings {
  pub fn validate(&self) -> XrayResult<()> {
    self.transport.validate()?;
    match &self.security {
      Security::None => {}
      Security::Tls(tls) => tls.validate()?,
      Security::Reality(reality) => {
        // REALITY borrows a real site's handshake, so it only rides the
        // transports Xray-core lets it wrap.
        if !matches!(
          self.transport,
          Transport::Raw { http_header: None } | Transport::Grpc { .. } | Transport::Xhttp { .. }
        ) {
          return Err(XrayError::UnsupportedValue {
            field: "security",
            expected: "REALITY only over RAW, gRPC or XHTTP",
          });
        }
        reality.validate()?;
      }
    }
    Ok(())
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XrayOutbound {
  Vless {
    id: String,
    flow: Option<VlessFlow>,
    stream: StreamSettings,
  },
  Vmess {
    id: String,
    security: VmessSecurity,
    stream: StreamSettings,
  },
  Trojan {
    password: String,
    stream: StreamSettings,
  },
  Hysteria2 {
    auth: String,
    tls: TlsSettings,
    /// Salamander obfuscation password.
    obfs_password: Option<String>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XrayConfig {
  pub address: String,
  pub port: u16,
  pub outbound: XrayOutbound,
}

impl XrayConfig {
  pub const fn protocol(&self) -> XrayProtocol {
    match self.outbound {
      XrayOutbound::Vless { .. } => XrayProtocol::Vless,
      XrayOutbound::Vmess { .. } => XrayProtocol::Vmess,
      XrayOutbound::Trojan { .. } => XrayProtocol::Trojan,
      XrayOutbound::Hysteria2 { .. } => XrayProtocol::Hysteria2,
    }
  }

  pub fn validate(&self) -> XrayResult<()> {
    validate_endpoint_address(&self.address)?;
    if self.port == 0 {
      return Err(XrayError::InvalidField {
        field: "port",
        reason: "must be between 1 and 65535",
      });
    }
    match &self.outbound {
      XrayOutbound::Vless { id, flow, stream } => {
        validate_uuid(id)?;
        stream.validate()?;
        // Vision splices the inner TLS stream, which only works when the
        // connection is a bare TLS or REALITY stream over RAW.
        if flow.is_some()
          && !(matches!(stream.transport, Transport::Raw { http_header: None })
            && matches!(stream.security, Security::Tls(_) | Security::Reality(_)))
        {
          return Err(XrayError::UnsupportedValue {
            field: "flow",
            expected: "XTLS Vision only over RAW with TLS or REALITY",
          });
        }
        Ok(())
      }
      XrayOutbound::Vmess { id, stream, .. } => {
        validate_uuid(id)?;
        stream.validate()
      }
      XrayOutbound::Trojan { password, stream } => {
        validate_secret(password, "password")?;
        stream.validate()
      }
      XrayOutbound::Hysteria2 {
        auth,
        tls,
        obfs_password,
      } => {
        validate_secret(auth, "auth")?;
        if let Some(password) = obfs_password {
          validate_secret(password, "obfs-password")?;
          // Xray-core rejects a shorter Salamander key only when it dials,
          // which left a link that saved fine and never connected.
          if password.len() < 4 {
            return Err(XrayError::InvalidField {
              field: "obfs-password",
              reason: "must contain at least 4 bytes",
            });
          }
        }
        tls.validate()
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedShareLink {
  pub name: Option<String>,
  pub config: XrayConfig,
}

impl ParsedShareLink {
  pub fn validate(&self) -> XrayResult<()> {
    if let Some(name) = &self.name {
      validate_display_name(name)?;
    }
    self.config.validate()
  }
}

pub(crate) fn validate_display_name(name: &str) -> XrayResult<()> {
  if name.is_empty() {
    return Err(XrayError::InvalidField {
      field: "name",
      reason: "must not be empty",
    });
  }
  if name.chars().count() > 200 {
    return Err(XrayError::InvalidField {
      field: "name",
      reason: "must not exceed 200 characters",
    });
  }
  if name.chars().any(char::is_control) {
    return Err(XrayError::InvalidField {
      field: "name",
      reason: "must not contain control characters",
    });
  }
  Ok(())
}

fn validate_uuid(id: &str) -> XrayResult<()> {
  Uuid::parse_str(id)
    .map(|_| ())
    .map_err(|_| XrayError::InvalidField {
      field: "id",
      reason: "must be a UUID",
    })
}

fn validate_secret(value: &str, field: &'static str) -> XrayResult<()> {
  if value.is_empty() || value.len() > MAX_SECRET_BYTES || value.chars().any(char::is_control) {
    return Err(XrayError::InvalidField {
      field,
      reason: "must contain 1 to 1024 bytes and no control characters",
    });
  }
  Ok(())
}

fn validate_endpoint_address(address: &str) -> XrayResult<()> {
  if address.is_empty()
    || address.trim() != address
    || address.starts_with('[')
    || address.ends_with(']')
  {
    return Err(XrayError::InvalidField {
      field: "address",
      reason: "must be a valid hostname or IP address",
    });
  }
  if address.parse::<IpAddr>().is_ok() {
    return Ok(());
  }
  // Canonicality, not just parseability: `Host::parse` percent-decodes and
  // punycodes before it validates, so `caf%C3%A9.example.com` parses fine while
  // Xray dials the stored string verbatim and never resolves it. Case is the
  // one difference that is safe, since parsing only lowercases.
  match Host::parse(address) {
    Ok(host) if host.to_string().eq_ignore_ascii_case(address) => Ok(()),
    _ => Err(XrayError::InvalidField {
      field: "address",
      reason: "must be a valid hostname or IP address",
    }),
  }
}

fn validate_dns_name(name: &str, field: &'static str) -> XrayResult<()> {
  if name.is_empty() || name.trim() != name {
    return Err(XrayError::InvalidField {
      field,
      reason: "must be a valid DNS name",
    });
  }
  match Host::parse(name) {
    Ok(Host::Domain(domain)) if domain.eq_ignore_ascii_case(name) => Ok(()),
    _ => Err(XrayError::InvalidField {
      field,
      reason: "must be a valid DNS name",
    }),
  }
}

/// A TLS server name may also be an IP address: Go then checks the
/// certificate's IP entries and sends no SNI.
fn validate_tls_server_name(server_name: &str) -> XrayResult<()> {
  if server_name.parse::<IpAddr>().is_ok() {
    return Ok(());
  }
  validate_dns_name(server_name, "sni")
}

fn validate_host_header(host: &str) -> XrayResult<()> {
  if host.parse::<IpAddr>().is_ok() {
    return Ok(());
  }
  validate_dns_name(host, "host")
}

fn validate_path(path: &str) -> XrayResult<()> {
  if !path.starts_with('/')
    || path.len() > MAX_PATH_BYTES
    || path.chars().any(|c| c.is_control() || c.is_whitespace())
  {
    return Err(XrayError::InvalidField {
      field: "path",
      reason: "must start with / and contain no whitespace, at most 2048 bytes",
    });
  }
  Ok(())
}

fn validate_alpn(protocol: &str) -> XrayResult<()> {
  if protocol.is_empty()
    || protocol.len() > MAX_ALPN_BYTES
    || !protocol
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/' | b'-' | b'_'))
  {
    return Err(XrayError::InvalidField {
      field: "alpn",
      reason: "must be a comma-separated list of protocol names such as h2,http/1.1",
    });
  }
  Ok(())
}

fn validate_certificate_pin(pin: &str) -> XrayResult<()> {
  if pin.len() != 64
    || !pin
      .bytes()
      .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
  {
    return Err(XrayError::InvalidField {
      field: "pcs",
      reason: "must be SHA-256 hashes in hexadecimal",
    });
  }
  Ok(())
}

fn validate_public_key(public_key: &str) -> XrayResult<()> {
  let decoded = URL_SAFE_NO_PAD
    .decode(public_key)
    .map_err(|_| XrayError::InvalidField {
      field: "pbk",
      reason: "must be an unpadded base64url-encoded 32-byte key",
    })?;
  if decoded.len() != 32 || URL_SAFE_NO_PAD.encode(decoded) != public_key {
    return Err(XrayError::InvalidField {
      field: "pbk",
      reason: "must be an unpadded base64url-encoded 32-byte key",
    });
  }
  Ok(())
}

fn validate_short_id(short_id: &str) -> XrayResult<()> {
  if short_id.len() > 16 || !short_id.len().is_multiple_of(2) {
    return Err(XrayError::InvalidField {
      field: "sid",
      reason: "must be empty or contain up to 16 even-length hexadecimal characters",
    });
  }
  if !short_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
    return Err(XrayError::InvalidField {
      field: "sid",
      reason: "must be empty or contain up to 16 even-length hexadecimal characters",
    });
  }
  Ok(())
}

fn validate_spider_x(spider_x: &str) -> XrayResult<()> {
  if !spider_x.starts_with('/') {
    return Err(XrayError::InvalidField {
      field: "spx",
      reason: "must start with /",
    });
  }
  if spider_x.len() > MAX_SPIDER_X_BYTES || spider_x.chars().any(char::is_control) {
    return Err(XrayError::InvalidField {
      field: "spx",
      reason: "must be a valid relative path no longer than 2048 bytes",
    });
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn public_key() -> String {
    URL_SAFE_NO_PAD.encode([7_u8; 32])
  }

  fn reality() -> RealitySettings {
    RealitySettings {
      server_name: "www.example.com".to_string(),
      public_key: public_key(),
      short_id: "0123456789abcdef".to_string(),
      fingerprint: Fingerprint::Chrome,
      spider_x: "/".to_string(),
    }
  }

  fn vless(flow: Option<VlessFlow>, transport: Transport, security: Security) -> XrayConfig {
    XrayConfig {
      address: "vpn.example.com".to_string(),
      port: 443,
      outbound: XrayOutbound::Vless {
        id: "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4".to_string(),
        flow,
        stream: StreamSettings {
          transport,
          security,
        },
      },
    }
  }

  fn valid_config() -> XrayConfig {
    vless(
      Some(VlessFlow::Vision),
      Transport::Raw { http_header: None },
      Security::Reality(reality()),
    )
  }

  fn ws(path: &str) -> Transport {
    Transport::WebSocket {
      host: Some("cdn.example.com".to_string()),
      path: path.to_string(),
    }
  }

  fn with_reality(mutate: impl FnOnce(&mut RealitySettings)) -> XrayConfig {
    let mut settings = reality();
    mutate(&mut settings);
    vless(
      Some(VlessFlow::Vision),
      Transport::Raw { http_header: None },
      Security::Reality(settings),
    )
  }

  #[test]
  fn valid_model_passes_validation() {
    assert_eq!(valid_config().validate(), Ok(()));
  }

  #[test]
  fn protocol_names_round_trip_through_schemes() {
    for protocol in [
      XrayProtocol::Vless,
      XrayProtocol::Vmess,
      XrayProtocol::Trojan,
      XrayProtocol::Hysteria2,
    ] {
      assert_eq!(
        XrayProtocol::from_scheme(protocol.proxy_type()),
        Some(protocol)
      );
    }
    assert_eq!(
      XrayProtocol::from_scheme(" HY2 "),
      Some(XrayProtocol::Hysteria2)
    );
    for other in ["ss", "socks5", "http", "", "vless2"] {
      assert_eq!(XrayProtocol::from_scheme(other), None, "{other}");
    }
    assert_eq!(
      XrayProtocol::from_share_link("trojan://secret@a.com:443"),
      Some(XrayProtocol::Trojan)
    );
    assert_eq!(XrayProtocol::from_share_link("not a link"), None);
  }

  #[test]
  fn endpoint_accepts_ipv4_ipv6_and_dns() {
    for address in ["198.51.100.4", "2001:db8::1", "vpn.example.com"] {
      let mut config = valid_config();
      config.address = address.to_string();
      assert_eq!(config.validate(), Ok(()), "{address}");
    }
  }

  #[test]
  fn endpoint_rejects_empty_whitespace_and_invalid_hosts() {
    for address in [
      "",
      " vpn.example.com",
      "vpn example.com",
      "vpn.example.com:443",
      "[2001:db8::1]",
    ] {
      let mut config = valid_config();
      config.address = address.to_string();
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField {
          field: "address",
          ..
        })
      ));
    }
  }

  #[test]
  fn endpoint_rejects_hosts_xray_would_dial_verbatim() {
    // Both reach the sidecar unchanged, so accepting them buys a dead tunnel
    // with no import-time error.
    for address in ["caf%C3%A9.example.com", "café.example.com"] {
      let mut config = valid_config();
      config.address = address.to_string();
      assert!(
        matches!(
          config.validate(),
          Err(XrayError::InvalidField {
            field: "address",
            ..
          })
        ),
        "{address}"
      );
    }
  }

  #[test]
  fn endpoint_accepts_mixed_case_and_punycode_hosts() {
    for address in ["VPN.Example.com", "xn--caf-dma.example.com"] {
      let mut config = valid_config();
      config.address = address.to_string();
      assert_eq!(config.validate(), Ok(()), "{address}");
    }
  }

  #[test]
  fn id_must_be_a_uuid() {
    let mut config = valid_config();
    if let XrayOutbound::Vless { id, .. } = &mut config.outbound {
      *id = "not-a-uuid".to_string();
    }
    assert_eq!(
      config.validate(),
      Err(XrayError::InvalidField {
        field: "id",
        reason: "must be a UUID",
      })
    );
  }

  #[test]
  fn reality_server_name_must_be_dns_name() {
    for server_name in ["", "203.0.113.5", "bad server"] {
      let config = with_reality(|reality| reality.server_name = server_name.to_string());
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "sni", .. })
      ));
    }
  }

  #[test]
  fn tls_server_name_may_be_an_ip_address() {
    for server_name in ["203.0.113.5", "2001:db8::1", "cdn.example.com"] {
      let config = vless(
        None,
        ws("/ray"),
        Security::Tls(TlsSettings {
          server_name: Some(server_name.to_string()),
          ..TlsSettings::default()
        }),
      );
      assert_eq!(config.validate(), Ok(()), "{server_name}");
    }
  }

  #[test]
  fn public_key_must_be_canonical_base64url_and_32_bytes() {
    let short_key = URL_SAFE_NO_PAD.encode([1_u8; 31]);
    for public_key in [
      "not-base64!",
      short_key.as_str(),
      "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=",
    ] {
      let config = with_reality(|reality| reality.public_key = public_key.to_string());
      let error = config.validate().unwrap_err();
      assert!(matches!(
        error,
        XrayError::InvalidField { field: "pbk", .. }
      ));
      assert!(!error.to_string().contains(public_key));
    }
  }

  #[test]
  fn short_id_accepts_empty_or_even_hex_up_to_sixteen_chars() {
    for short_id in ["", "ab", "0123456789abcdef", "ABCDEF"] {
      let config = with_reality(|reality| reality.short_id = short_id.to_string());
      assert_eq!(config.validate(), Ok(()), "{short_id}");
    }
  }

  #[test]
  fn short_id_rejects_odd_non_hex_and_overlong_values() {
    for short_id in ["a", "xz", "0123456789abcdef00"] {
      let config = with_reality(|reality| reality.short_id = short_id.to_string());
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "sid", .. })
      ));
    }
  }

  #[test]
  fn spider_x_must_be_safe_relative_path() {
    for spider_x in ["relative", "/line\nbreak"] {
      let config = with_reality(|reality| reality.spider_x = spider_x.to_string());
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "spx", .. })
      ));
    }
  }

  #[test]
  fn vision_needs_bare_raw_with_tls_or_reality() {
    let tls = || Security::Tls(TlsSettings::default());
    assert_eq!(
      vless(
        Some(VlessFlow::Vision),
        Transport::Raw { http_header: None },
        tls()
      )
      .validate(),
      Ok(())
    );
    for config in [
      vless(Some(VlessFlow::Vision), ws("/"), tls()),
      vless(
        Some(VlessFlow::VisionUdp443),
        Transport::Raw { http_header: None },
        Security::None,
      ),
      vless(
        Some(VlessFlow::Vision),
        Transport::Raw {
          http_header: Some(HttpHeader {
            hosts: vec![],
            path: "/".to_string(),
          }),
        },
        tls(),
      ),
    ] {
      assert!(
        matches!(
          config.validate(),
          Err(XrayError::UnsupportedValue { field: "flow", .. })
        ),
        "{config:?}"
      );
    }
  }

  #[test]
  fn reality_rides_only_raw_grpc_or_xhttp() {
    assert!(matches!(
      vless(None, ws("/"), Security::Reality(reality())).validate(),
      Err(XrayError::UnsupportedValue {
        field: "security",
        ..
      })
    ));
    for transport in [
      Transport::Grpc {
        service_name: "gun".to_string(),
        authority: None,
        mode: GrpcMode::Gun,
      },
      Transport::Xhttp {
        host: None,
        path: "/".to_string(),
        mode: XhttpMode::Auto,
      },
    ] {
      assert_eq!(
        vless(None, transport, Security::Reality(reality())).validate(),
        Ok(())
      );
    }
  }

  #[test]
  fn transport_paths_hosts_and_service_names_are_checked() {
    for (transport, field) in [
      (ws("relative"), "path"),
      (ws("/with space"), "path"),
      (
        Transport::HttpUpgrade {
          host: Some("bad host".to_string()),
          path: "/".to_string(),
        },
        "host",
      ),
      (
        Transport::Grpc {
          service_name: "gun\nname".to_string(),
          authority: None,
          mode: GrpcMode::Multi,
        },
        "serviceName",
      ),
    ] {
      let error = vless(None, transport, Security::None)
        .validate()
        .unwrap_err();
      assert!(
        matches!(error, XrayError::InvalidField { field: f, .. } if f == field),
        "{error:?}"
      );
    }
  }

  #[test]
  fn tls_options_are_checked() {
    for (tls, field) in [
      (
        TlsSettings {
          alpn: vec!["h2 ".to_string()],
          ..TlsSettings::default()
        },
        "alpn",
      ),
      (
        TlsSettings {
          pinned_peer_cert_sha256: vec!["abcd".to_string()],
          ..TlsSettings::default()
        },
        "pcs",
      ),
      (
        TlsSettings {
          verify_peer_cert_by_name: vec!["203.0.113.5".to_string()],
          ..TlsSettings::default()
        },
        "vcn",
      ),
    ] {
      let error = vless(None, ws("/"), Security::Tls(tls))
        .validate()
        .unwrap_err();
      assert!(
        matches!(error, XrayError::InvalidField { field: f, .. } if f == field),
        "{error:?}"
      );
    }
  }

  #[test]
  fn secrets_must_be_present_and_printable() {
    let stream = || StreamSettings {
      transport: Transport::Raw { http_header: None },
      security: Security::Tls(TlsSettings::default()),
    };
    for password in ["", "line\nbreak"] {
      let config = XrayConfig {
        address: "a.example.com".to_string(),
        port: 443,
        outbound: XrayOutbound::Trojan {
          password: password.to_string(),
          stream: stream(),
        },
      };
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField {
          field: "password",
          ..
        })
      ));
    }
    let config = XrayConfig {
      address: "a.example.com".to_string(),
      port: 443,
      outbound: XrayOutbound::Hysteria2 {
        auth: "secret".to_string(),
        tls: TlsSettings::default(),
        obfs_password: Some(String::new()),
      },
    };
    assert!(matches!(
      config.validate(),
      Err(XrayError::InvalidField {
        field: "obfs-password",
        ..
      })
    ));
  }
}

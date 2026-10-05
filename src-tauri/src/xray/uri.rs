use std::collections::HashMap;
use std::net::IpAddr;

use base64::{
  engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
  Engine as _,
};
use serde_json::{Map, Value};
use url::{Host, Url};
use uuid::Uuid;

use super::{
  model::validate_display_name, Fingerprint, GrpcMode, HttpHeader, ParsedShareLink,
  RealitySettings, Security, StreamSettings, TlsSettings, Transport, VlessFlow, VmessSecurity,
  XhttpMode, XrayConfig, XrayError, XrayOutbound, XrayProtocol, XrayResult,
};

/// Options that change nothing on the wire and are dropped on import.
///
/// `allowInsecure`/`insecure` asks to skip the certificate check. Xray-core
/// refuses that since 2026-06-01, and many panels add it to every link by
/// habit, so the check simply stays on: a server with a valid certificate
/// still works, and a self-signed one needs a pinned hash (`pcs`).
/// `upmbps`/`downmbps` only pick Hysteria's congestion control; without them
/// the client uses BBR, which every server accepts.
const IGNORED_PARAMETERS: &[&str] = &["allowInsecure", "insecure", "upmbps", "downmbps"];

const STREAM_PARAMETERS: &[&str] = &[
  "security",
  "sni",
  "alpn",
  "fp",
  "pcs",
  "vcn",
  "pbk",
  "sid",
  "spx",
  "type",
  "headerType",
  "host",
  "path",
  "serviceName",
  "authority",
  "mode",
];

const HYSTERIA2_PARAMETERS: &[&str] = &["sni", "pinSHA256", "obfs", "obfs-password"];

/// Parse a `vless://`, `vmess://`, `trojan://`, `hysteria2://` or `hy2://`
/// share link into the configuration an Xray-core sidecar dials.
pub fn parse_share_link(input: &str) -> XrayResult<ParsedShareLink> {
  if input.trim() != input {
    return Err(XrayError::InvalidUri);
  }
  let (scheme, _) = input.split_once("://").ok_or(XrayError::InvalidUri)?;
  let protocol = XrayProtocol::from_scheme(scheme).ok_or(XrayError::UnsupportedScheme)?;
  let parsed = match protocol {
    XrayProtocol::Vmess if !is_url_form(input) => parse_vmess_json(input)?,
    XrayProtocol::Hysteria2 => parse_hysteria2(input)?,
    XrayProtocol::Vless | XrayProtocol::Vmess | XrayProtocol::Trojan => {
      parse_url_link(input, protocol)?
    }
  };
  parsed.validate()?;
  Ok(parsed)
}

/// `vmess://` is either v2rayN's base64 JSON or the URL form the other
/// schemes use; only the URL form has a user part.
fn is_url_form(input: &str) -> bool {
  input
    .split_once("://")
    .and_then(|(_, rest)| rest.split('#').next())
    .is_some_and(|body| body.contains('@'))
}

/// The host a share link dials, read without validating the rest of it.
///
/// For callers that must name the server even when Donut cannot use the link,
/// such as the check that refuses an Xray exit for a remote host.
pub fn share_link_host(uri: &str) -> Option<String> {
  if let Ok(parsed) = parse_share_link(uri.trim()) {
    return Some(parsed.config.address);
  }
  let (scheme, rest) = uri.trim().split_once("://")?;
  if XrayProtocol::from_scheme(scheme) == Some(XrayProtocol::Vmess) && !is_url_form(uri.trim()) {
    let json = decode_vmess_body(rest.split('#').next()?).ok()?;
    let address = json_string(&json, "add").ok()??;
    return Some(address.trim_matches(['[', ']']).to_string()).filter(|host| !host.is_empty());
  }
  // Cut the fragment (`#label`) and query (`?type=...`) before looking for the
  // authority — either may contain '@' or ':'.
  let rest = rest.split('#').next()?;
  let rest = rest.split('?').next()?;
  let authority = rest.split('/').next()?;
  let host_port = authority
    .rsplit_once('@')
    .map(|(_, host)| host)
    .unwrap_or(authority);
  let host = if let Some(bracketed) = host_port.strip_prefix('[') {
    bracketed.split(']').next().unwrap_or_default()
  } else {
    host_port
      .rsplit_once(':')
      .map_or(host_port, |(host, _)| host)
  };
  let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
  Some(host).filter(|host| !host.is_empty())
}

/// Write the canonical share link for a configuration. Parsing the result
/// gives the same configuration back.
pub fn export_share_link(config: &XrayConfig, name: Option<&str>) -> XrayResult<String> {
  config.validate()?;
  if let Some(name) = name {
    validate_display_name(name)?;
  }
  match &config.outbound {
    XrayOutbound::Vmess {
      id,
      security,
      stream,
    } => export_vmess_json(config, id, *security, stream, name),
    XrayOutbound::Vless { id, flow, stream } => {
      let mut url = base_url(config, "vless", id)?;
      {
        let mut query = url.query_pairs_mut();
        query.append_pair("encryption", "none");
        if let Some(flow) = flow {
          query.append_pair("flow", flow.as_str());
        }
        append_stream(&mut query, stream);
      }
      Ok(finish_url(url, name))
    }
    XrayOutbound::Trojan { password, stream } => {
      let mut url = base_url(config, "trojan", password)?;
      append_stream(&mut url.query_pairs_mut(), stream);
      Ok(finish_url(url, name))
    }
    XrayOutbound::Hysteria2 {
      auth,
      tls,
      obfs_password,
    } => {
      let mut url = base_url(config, "hysteria2", auth)?;
      url.set_path("/");
      {
        let mut query = url.query_pairs_mut();
        if let Some(server_name) = &tls.server_name {
          query.append_pair("sni", server_name);
        }
        if let Some(password) = obfs_password {
          query.append_pair("obfs", "salamander");
          query.append_pair("obfs-password", password);
        }
        if !tls.pinned_peer_cert_sha256.is_empty() {
          query.append_pair("pinSHA256", &tls.pinned_peer_cert_sha256.join(","));
        }
      }
      Ok(finish_url(url, name))
    }
  }
}

/// The recognized query of a share link.
///
/// Unrecognized names are kept rather than rejected on the spot so the caller
/// can report the *shape* problem first. An mKCP link always carries `seed`,
/// and naming that key instead of the transport sends the user deleting
/// parameters when the real answer is that Donut does not speak mKCP.
struct Query {
  values: HashMap<String, String>,
  unsupported: Vec<String>,
}

impl Query {
  fn new() -> Self {
    Self {
      values: HashMap::new(),
      unsupported: Vec::new(),
    }
  }

  fn from_url(url: &Url, supported: &[&[&str]]) -> XrayResult<Self> {
    let mut query = Self::new();
    for (name, value) in url.query_pairs() {
      query.insert(&name, value.into_owned(), supported)?;
    }
    Ok(query)
  }

  fn insert(&mut self, name: &str, value: String, supported: &[&[&str]]) -> XrayResult<()> {
    if self.values.contains_key(name) {
      return Err(XrayError::DuplicateParameter(name.to_string()));
    }
    if IGNORED_PARAMETERS.contains(&name) {
      self.values.insert(name.to_string(), String::new());
    } else if supported.iter().any(|names| names.contains(&name)) {
      self.values.insert(name.to_string(), value);
    } else if !value.is_empty() {
      // An empty option asks for nothing, so it is not worth refusing a link
      // over.
      self.unsupported.push(name.to_string());
    }
    Ok(())
  }

  /// An empty value means the option is absent.
  fn get(&self, name: &str) -> Option<&str> {
    self
      .values
      .get(name)
      .map(String::as_str)
      .filter(|value| !value.is_empty())
  }

  fn required(&self, name: &'static str) -> XrayResult<&str> {
    self.get(name).ok_or(XrayError::MissingField(name))
  }

  fn reject_unsupported(&self) -> XrayResult<()> {
    match self.unsupported.first() {
      Some(name) => Err(XrayError::UnsupportedParameter(name.clone())),
      None => Ok(()),
    }
  }
}

fn parse_url_link(input: &str, protocol: XrayProtocol) -> XrayResult<ParsedShareLink> {
  let url = Url::parse(input).map_err(|_| XrayError::InvalidUri)?;
  reject_path(&url)?;
  let (user, has_password) = userinfo(&url)?;
  let address = canonical_host(&url)?;
  let port = url.port().ok_or(XrayError::MissingField("port"))?;
  let protocol_parameters: &[&str] = match protocol {
    XrayProtocol::Vless => &["encryption", "flow"],
    XrayProtocol::Vmess => &["encryption"],
    XrayProtocol::Trojan => &["flow", "peer"],
    XrayProtocol::Hysteria2 => &[],
  };
  let query = Query::from_url(&url, &[STREAM_PARAMETERS, protocol_parameters])?;
  let default_security = if protocol == XrayProtocol::Trojan {
    "tls"
  } else {
    "none"
  };
  let shape = parse_stream_shape(&query, default_security)?;

  let outbound = match protocol {
    XrayProtocol::Vless => {
      let id = parse_uuid(&user, has_password)?;
      let flow = match query.get("flow") {
        None | Some("none") => None,
        Some("xtls-rprx-vision") => Some(VlessFlow::Vision),
        Some("xtls-rprx-vision-udp443") => Some(VlessFlow::VisionUdp443),
        Some(_) => {
          return Err(XrayError::UnsupportedValue {
            field: "flow",
            expected: "xtls-rprx-vision",
          })
        }
      };
      if query.get("encryption").is_some_and(|value| value != "none") {
        return Err(XrayError::UnsupportedValue {
          field: "encryption",
          expected: "none",
        });
      }
      query.reject_unsupported()?;
      XrayOutbound::Vless {
        id,
        flow,
        stream: shape.finish(&query)?,
      }
    }
    XrayProtocol::Vmess => {
      let id = parse_uuid(&user, has_password)?;
      let security = query
        .get("encryption")
        .map(VmessSecurity::parse)
        .transpose()?
        .unwrap_or_default();
      query.reject_unsupported()?;
      XrayOutbound::Vmess {
        id,
        security,
        stream: shape.finish(&query)?,
      }
    }
    XrayProtocol::Trojan => {
      if query.get("flow").is_some() {
        return Err(XrayError::UnsupportedValue {
          field: "flow",
          expected: "no flow for Trojan",
        });
      }
      query.reject_unsupported()?;
      XrayOutbound::Trojan {
        password: user,
        stream: shape.finish(&query)?,
      }
    }
    XrayProtocol::Hysteria2 => unreachable!("Hysteria2 links have their own parser"),
  };

  Ok(ParsedShareLink {
    name: fragment_name(&url)?,
    config: XrayConfig {
      address,
      port,
      outbound,
    },
  })
}

fn parse_hysteria2(input: &str) -> XrayResult<ParsedShareLink> {
  let url = Url::parse(input).map_err(|_| XrayError::InvalidUri)?;
  reject_path(&url)?;
  let (auth, _) = userinfo(&url)?;
  let address = canonical_host(&url)?;
  let port = url.port().unwrap_or(443);
  let query = Query::from_url(&url, &[HYSTERIA2_PARAMETERS])?;

  let obfs_password = match query.get("obfs") {
    None => None,
    Some(obfs) if obfs.eq_ignore_ascii_case("salamander") => {
      Some(query.required("obfs-password")?.to_string())
    }
    Some(_) => {
      return Err(XrayError::UnsupportedValue {
        field: "obfs",
        expected: "salamander",
      })
    }
  };
  query.reject_unsupported()?;

  if auth.is_empty() {
    return Err(XrayError::MissingField("auth"));
  }
  // The QUIC handshake has no destination to fall back on, so the server name
  // is always explicit, exactly as the Hysteria client itself defaults it.
  let server_name = canonical_name(query.get("sni").unwrap_or(&address));
  let tls = TlsSettings {
    server_name: Some(server_name),
    pinned_peer_cert_sha256: query
      .get("pinSHA256")
      .map(parse_certificate_pins)
      .unwrap_or_default(),
    ..TlsSettings::default()
  };

  Ok(ParsedShareLink {
    name: fragment_name(&url)?,
    config: XrayConfig {
      address,
      port,
      outbound: XrayOutbound::Hysteria2 {
        auth,
        tls,
        obfs_password,
      },
    },
  })
}

/// The v2rayN `vmess://<base64 JSON>` form, which is what VMess providers
/// actually hand out. Its keys are mapped onto the query names the other links
/// use so one stream parser reads both.
fn parse_vmess_json(input: &str) -> XrayResult<ParsedShareLink> {
  let (_, body) = input.split_once("://").ok_or(XrayError::InvalidUri)?;
  let (encoded, fragment) = match body.split_once('#') {
    Some((encoded, fragment)) => (encoded, Some(fragment)),
    None => (body, None),
  };
  let json = decode_vmess_body(encoded)?;

  let net = json_string(&json, "net")?
    .unwrap_or_default()
    .to_ascii_lowercase();
  let mut query = Query::new();
  let mut address = None;
  let mut port = None;
  let mut id = None;
  let mut name = None;
  for (key, value) in &json {
    let value = json_value_string(value).ok_or(XrayError::InvalidField {
      field: "vmess",
      reason: "fields must be strings or numbers",
    })?;
    // `type` and `host`/`path` mean different things per transport, exactly
    // as v2rayN writes them.
    let name_for_query = match (key.as_str(), net.as_str()) {
      ("add", _) => {
        address = Some(value);
        continue;
      }
      ("port", _) => {
        port = Some(value);
        continue;
      }
      ("id", _) => {
        id = Some(value);
        continue;
      }
      ("ps", _) => {
        name = Some(value);
        continue;
      }
      // Xray-core speaks only AEAD VMess, which every server that still lists
      // an alterId also accepts.
      ("v" | "aid", _) => continue,
      ("scy", _) => "encryption",
      ("tls", _) => "security",
      ("net", _) => "type",
      ("type", "grpc" | "xhttp" | "splithttp") => "mode",
      ("type", _) => "headerType",
      ("host", "grpc") => "authority",
      ("path", "grpc") => "serviceName",
      (other, _) => other,
    };
    query.insert(name_for_query, value, &[STREAM_PARAMETERS, &["encryption"]])?;
  }
  // v2rayN writes `"type": "none"` for every transport and `"tls": ""` for no
  // security.
  if query.get("mode") == Some("none") {
    query.values.remove("mode");
  }

  let shape = parse_stream_shape(&query, "none")?;
  let security = query
    .get("encryption")
    .map(VmessSecurity::parse)
    .transpose()?
    .unwrap_or_default();
  query.reject_unsupported()?;
  let stream = shape.finish(&query)?;

  let address = canonical_address(&address.ok_or(XrayError::MissingField("address"))?)?;
  let port = port
    .ok_or(XrayError::MissingField("port"))?
    .parse::<u16>()
    .map_err(|_| XrayError::InvalidField {
      field: "port",
      reason: "must be between 1 and 65535",
    })?;
  let id = parse_uuid(&id.ok_or(XrayError::MissingField("id"))?, false)?;
  let name = match name.filter(|name| !name.is_empty()) {
    Some(name) => Some(name),
    None => fragment
      .filter(|fragment| !fragment.is_empty())
      .map(decode_fragment)
      .transpose()?,
  };

  Ok(ParsedShareLink {
    name,
    config: XrayConfig {
      address,
      port,
      outbound: XrayOutbound::Vmess {
        id,
        security,
        stream,
      },
    },
  })
}

fn decode_vmess_body(encoded: &str) -> XrayResult<Map<String, Value>> {
  let compact = urlencoding::decode(encoded)
    .map_err(|_| XrayError::InvalidUri)?
    .chars()
    .filter(|c| !c.is_whitespace())
    .collect::<String>();
  let bytes = [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
    .iter()
    .find_map(|engine| engine.decode(&compact).ok())
    .ok_or(XrayError::InvalidUri)?;
  match serde_json::from_slice::<Value>(&bytes) {
    Ok(Value::Object(map)) => Ok(map),
    _ => Err(XrayError::InvalidUri),
  }
}

fn json_value_string(value: &Value) -> Option<String> {
  match value {
    Value::String(value) => Some(value.trim().to_string()),
    Value::Number(value) => Some(value.to_string()),
    Value::Bool(value) => Some(value.to_string()),
    Value::Null => Some(String::new()),
    Value::Array(_) | Value::Object(_) => None,
  }
}

fn json_string(json: &Map<String, Value>, key: &str) -> XrayResult<Option<String>> {
  json
    .get(key)
    .map(|value| {
      json_value_string(value).ok_or(XrayError::InvalidField {
        field: "vmess",
        reason: "fields must be strings or numbers",
      })
    })
    .transpose()
}

/// The transport and the kind of security, read before anything else.
///
/// Transport first: it is the most common reason a real-world server is
/// unusable here, and it explains the stray parameters that come with it.
struct StreamShape {
  transport: Transport,
  security: String,
}

fn parse_stream_shape(query: &Query, default_security: &str) -> XrayResult<StreamShape> {
  let header_type = query.get("headerType");
  let path = || query.get("path").unwrap_or("/").to_string();
  let host = || query.get("host").map(canonical_name);
  let network = query.get("type").map(str::to_ascii_lowercase);
  let transport = match network.as_deref() {
    None | Some("tcp" | "raw") => match header_type {
      None | Some("none") => Transport::Raw { http_header: None },
      Some("http") => Transport::Raw {
        http_header: Some(HttpHeader {
          hosts: split_list(query.get("host")),
          path: path(),
        }),
      },
      Some(_) => {
        return Err(XrayError::UnsupportedValue {
          field: "headerType",
          expected: "none or http",
        })
      }
    },
    Some("ws" | "websocket") => Transport::WebSocket {
      host: host(),
      path: path(),
    },
    Some("httpupgrade") => Transport::HttpUpgrade {
      host: host(),
      path: path(),
    },
    Some("grpc") => Transport::Grpc {
      service_name: query.get("serviceName").unwrap_or_default().to_string(),
      authority: query.get("authority").map(canonical_name),
      mode: match query.get("mode") {
        None | Some("gun") => GrpcMode::Gun,
        Some("multi") => GrpcMode::Multi,
        Some(_) => {
          return Err(XrayError::UnsupportedValue {
            field: "mode",
            expected: "gun or multi",
          })
        }
      },
    },
    Some("xhttp" | "splithttp") => Transport::Xhttp {
      host: host(),
      path: path(),
      mode: match query.get("mode") {
        None | Some("auto") => XhttpMode::Auto,
        Some("packet-up") => XhttpMode::PacketUp,
        Some("stream-up") => XhttpMode::StreamUp,
        Some("stream-one") => XhttpMode::StreamOne,
        Some(_) => {
          return Err(XrayError::UnsupportedValue {
            field: "mode",
            expected: "auto, packet-up, stream-up or stream-one",
          })
        }
      },
    },
    Some(_) => {
      return Err(XrayError::UnsupportedValue {
        field: "type",
        expected: "tcp, ws, grpc, httpupgrade or xhttp",
      })
    }
  };
  if !matches!(transport, Transport::Raw { .. }) && header_type.is_some_and(|value| value != "none")
  {
    return Err(XrayError::UnsupportedValue {
      field: "headerType",
      expected: "none",
    });
  }

  let security = query
    .get("security")
    .unwrap_or(default_security)
    .to_ascii_lowercase();
  if !matches!(security.as_str(), "none" | "tls" | "reality") {
    return Err(XrayError::UnsupportedValue {
      field: "security",
      expected: "none, tls or reality",
    });
  }
  Ok(StreamShape {
    transport,
    security,
  })
}

impl StreamShape {
  /// The security details, read only once the shape and the options are known
  /// to be good, so a missing key is not reported for a link that was never
  /// usable.
  fn finish(self, query: &Query) -> XrayResult<StreamSettings> {
    let fingerprint = query.get("fp").map(Fingerprint::parse).transpose()?;
    let security = match self.security.as_str() {
      "tls" => Security::Tls(TlsSettings {
        // A CDN-fronted link often names its real domain only as the Host
        // header; v2rayN sends that as the server name too.
        server_name: query
          .get("sni")
          .or_else(|| query.get("peer"))
          .map(canonical_name)
          .or_else(|| self.transport.host().map(str::to_string)),
        alpn: split_list(query.get("alpn")),
        fingerprint,
        pinned_peer_cert_sha256: query
          .get("pcs")
          .map(parse_certificate_pins)
          .unwrap_or_default(),
        verify_peer_cert_by_name: split_list(query.get("vcn")),
      }),
      "reality" => Security::Reality(RealitySettings {
        server_name: canonical_name(query.required("sni")?),
        public_key: query.required("pbk")?.to_string(),
        short_id: query.get("sid").unwrap_or_default().to_string(),
        fingerprint: fingerprint.unwrap_or_default(),
        spider_x: query.get("spx").unwrap_or("/").to_string(),
      }),
      _ => Security::None,
    };
    Ok(StreamSettings {
      transport: self.transport,
      security,
    })
  }
}

/// A server or Host name in the form the sidecar can use. A name such as
/// `café.example.com` becomes its punycode form; any other value stays as it
/// is, so validation still sees it. Links stored before more protocols arrived
/// kept such names verbatim, and must still load.
fn canonical_name(value: &str) -> String {
  match Host::parse(value) {
    Ok(Host::Domain(domain)) if !domain.eq_ignore_ascii_case(value) => domain,
    _ => value.to_string(),
  }
}

fn split_list(value: Option<&str>) -> Vec<String> {
  value
    .unwrap_or_default()
    .split(',')
    .map(str::trim)
    .filter(|item| !item.is_empty())
    .map(str::to_string)
    .collect()
}

/// OpenSSL prints certificate hashes with colons and in upper case; Xray-core
/// and Hysteria both take either form.
fn parse_certificate_pins(value: &str) -> Vec<String> {
  split_list(Some(value))
    .into_iter()
    .map(|pin| pin.replace(':', "").to_ascii_lowercase())
    .collect()
}

fn reject_path(url: &Url) -> XrayResult<()> {
  if !matches!(url.path(), "" | "/") {
    return Err(XrayError::InvalidField {
      field: "path",
      reason: "share links must not contain a path",
    });
  }
  Ok(())
}

/// The user part, percent-decoded. A secret that itself holds a `:` arrives
/// split in two by the URL parser and is joined back.
fn userinfo(url: &Url) -> XrayResult<(String, bool)> {
  let decode = |value: &str| {
    urlencoding::decode(value)
      .map(|value| value.into_owned())
      .map_err(|_| XrayError::InvalidField {
        field: "id",
        reason: "must use valid percent encoding",
      })
  };
  let mut user = decode(url.username())?;
  let has_password = url.password().is_some();
  if let Some(password) = url.password() {
    user.push(':');
    user.push_str(&decode(password)?);
  }
  Ok((user, has_password))
}

fn parse_uuid(raw: &str, has_password: bool) -> XrayResult<String> {
  if has_password {
    return Err(XrayError::InvalidField {
      field: "id",
      reason: "password-style user information is not supported",
    });
  }
  if raw.is_empty() {
    return Err(XrayError::MissingField("id"));
  }
  Uuid::parse_str(raw)
    .map(|id| id.to_string())
    .map_err(|_| XrayError::InvalidField {
      field: "id",
      reason: "must be a UUID",
    })
}

fn canonical_host(url: &Url) -> XrayResult<String> {
  match url.host().ok_or(XrayError::MissingField("address"))? {
    // These schemes are not special, so `Url` keeps the host exactly as
    // written, percent-escapes included, and that string is what the sidecar
    // dials. Re-parsing canonicalizes an internationalized host into the
    // punycode form that actually resolves.
    Host::Domain(value) => canonical_address(value),
    Host::Ipv4(value) => Ok(value.to_string()),
    Host::Ipv6(value) => Ok(value.to_string()),
  }
}

fn canonical_address(value: &str) -> XrayResult<String> {
  let value = value.trim_start_matches('[').trim_end_matches(']');
  if let Ok(address) = value.parse::<IpAddr>() {
    return Ok(address.to_string());
  }
  Host::parse(value)
    .map(|host| host.to_string())
    .map_err(|_| XrayError::InvalidField {
      field: "address",
      reason: "must be a valid hostname or IP address",
    })
}

fn decode_fragment(fragment: &str) -> XrayResult<String> {
  urlencoding::decode(fragment)
    .map(|value| value.into_owned())
    .map_err(|_| XrayError::InvalidField {
      field: "name",
      reason: "must use valid percent encoding",
    })
}

fn fragment_name(url: &Url) -> XrayResult<Option<String>> {
  url
    .fragment()
    .filter(|fragment| !fragment.is_empty())
    .map(decode_fragment)
    .transpose()
}

fn base_url(config: &XrayConfig, scheme: &str, user: &str) -> XrayResult<Url> {
  let mut url =
    Url::parse(&format!("{scheme}://placeholder@127.0.0.1")).expect("static share URL is valid");
  // The user part is percent-decoded on import, but `set_username` leaves `%`
  // as it is, so a Trojan password `pa%41ss` came back as `paAss`.
  url
    .set_username(&user.replace('%', "%25"))
    .map_err(|_| XrayError::InvalidUri)?;
  let uri_host = match config.address.parse::<IpAddr>() {
    Ok(IpAddr::V6(address)) => format!("[{address}]"),
    _ => config.address.clone(),
  };
  url
    .set_host(Some(&uri_host))
    .map_err(|_| XrayError::InvalidField {
      field: "address",
      reason: "must be a valid hostname or IP address",
    })?;
  url
    .set_port(Some(config.port))
    .map_err(|_| XrayError::InvalidField {
      field: "port",
      reason: "must be between 1 and 65535",
    })?;
  Ok(url)
}

fn finish_url(mut url: Url, name: Option<&str>) -> String {
  // The parser percent-DECODES the fragment, so the exporter must encode it or
  // a name containing `%` (or `#`) comes back different every time the link is
  // canonicalized — the name mutates a little more on each save.
  let encoded_name = name.map(|value| urlencoding::encode(value).into_owned());
  url.set_fragment(encoded_name.as_deref());
  url.into()
}

fn append_stream(
  query: &mut url::form_urlencoded::Serializer<'_, url::UrlQuery<'_>>,
  stream: &StreamSettings,
) {
  query.append_pair("security", stream.security.as_str());
  match &stream.security {
    Security::None => {}
    Security::Tls(tls) => {
      if let Some(server_name) = &tls.server_name {
        query.append_pair("sni", server_name);
      }
      if let Some(fingerprint) = tls.fingerprint {
        query.append_pair("fp", fingerprint.as_str());
      }
      if !tls.alpn.is_empty() {
        query.append_pair("alpn", &tls.alpn.join(","));
      }
      if !tls.pinned_peer_cert_sha256.is_empty() {
        query.append_pair("pcs", &tls.pinned_peer_cert_sha256.join(","));
      }
      if !tls.verify_peer_cert_by_name.is_empty() {
        query.append_pair("vcn", &tls.verify_peer_cert_by_name.join(","));
      }
    }
    Security::Reality(reality) => {
      query.append_pair("sni", &reality.server_name);
      query.append_pair("fp", reality.fingerprint.as_str());
      query.append_pair("pbk", &reality.public_key);
      query.append_pair("sid", &reality.short_id);
      query.append_pair("spx", &reality.spider_x);
    }
  }
  query.append_pair("type", stream.transport.as_str());
  match &stream.transport {
    Transport::Raw { http_header: None } => {
      query.append_pair("headerType", "none");
    }
    Transport::Raw {
      http_header: Some(header),
    } => {
      query.append_pair("headerType", "http");
      if !header.hosts.is_empty() {
        query.append_pair("host", &header.hosts.join(","));
      }
      query.append_pair("path", &header.path);
    }
    Transport::WebSocket { host, path }
    | Transport::HttpUpgrade { host, path }
    | Transport::Xhttp { host, path, .. } => {
      if let Some(host) = host {
        query.append_pair("host", host);
      }
      query.append_pair("path", path);
      if let Transport::Xhttp { mode, .. } = &stream.transport {
        query.append_pair("mode", mode.as_str());
      }
    }
    Transport::Grpc {
      service_name,
      authority,
      mode,
    } => {
      query.append_pair("serviceName", service_name);
      if let Some(authority) = authority {
        query.append_pair("authority", authority);
      }
      query.append_pair("mode", mode.as_str());
    }
  }
}

fn export_vmess_json(
  config: &XrayConfig,
  id: &str,
  security: VmessSecurity,
  stream: &StreamSettings,
  name: Option<&str>,
) -> XrayResult<String> {
  let (header_type, host, path) = match &stream.transport {
    Transport::Raw { http_header: None } => ("none".to_string(), String::new(), String::new()),
    Transport::Raw {
      http_header: Some(header),
    } => (
      "http".to_string(),
      header.hosts.join(","),
      header.path.clone(),
    ),
    Transport::WebSocket { host, path } | Transport::HttpUpgrade { host, path } => (
      "none".to_string(),
      host.clone().unwrap_or_default(),
      path.clone(),
    ),
    Transport::Xhttp { host, path, mode } => (
      mode.as_str().to_string(),
      host.clone().unwrap_or_default(),
      path.clone(),
    ),
    Transport::Grpc {
      service_name,
      authority,
      mode,
    } => (
      mode.as_str().to_string(),
      authority.clone().unwrap_or_default(),
      service_name.clone(),
    ),
  };

  let mut json = Map::new();
  let mut put = |key: &str, value: String| {
    json.insert(key.to_string(), Value::String(value));
  };
  put("v", "2".to_string());
  put("ps", name.unwrap_or_default().to_string());
  put("add", config.address.clone());
  put("port", config.port.to_string());
  put("id", id.to_string());
  put("aid", "0".to_string());
  put("scy", security.as_str().to_string());
  put("net", stream.transport.as_str().to_string());
  put("type", header_type);
  put("host", host);
  put("path", path);
  match &stream.security {
    Security::None => put("tls", String::new()),
    Security::Tls(tls) => {
      put("tls", "tls".to_string());
      put("sni", tls.server_name.clone().unwrap_or_default());
      put("alpn", tls.alpn.join(","));
      put(
        "fp",
        tls
          .fingerprint
          .map(|fingerprint| fingerprint.as_str().to_string())
          .unwrap_or_default(),
      );
      if !tls.pinned_peer_cert_sha256.is_empty() {
        put("pcs", tls.pinned_peer_cert_sha256.join(","));
      }
      if !tls.verify_peer_cert_by_name.is_empty() {
        put("vcn", tls.verify_peer_cert_by_name.join(","));
      }
    }
    Security::Reality(reality) => {
      put("tls", "reality".to_string());
      put("sni", reality.server_name.clone());
      put("fp", reality.fingerprint.as_str().to_string());
      put("pbk", reality.public_key.clone());
      put("sid", reality.short_id.clone());
      put("spx", reality.spider_x.clone());
    }
  }
  let body = serde_json::to_vec(&json).map_err(|_| XrayError::Serialization)?;
  Ok(format!("vmess://{}", STANDARD.encode(body)))
}

#[cfg(test)]
#[path = "uri_tests.rs"]
mod tests;

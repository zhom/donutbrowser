use std::collections::HashMap;

use std::net::IpAddr;

use url::{Host, Url};
use uuid::Uuid;

use super::{
  model::validate_display_name, ParsedVlessUri, RealityFingerprint, RealitySettings, VlessFlow,
  VlessRealityConfig, XrayError, XrayResult,
};

const SUPPORTED_PARAMETERS: &[&str] = &[
  "encryption",
  "flow",
  "security",
  "sni",
  "fp",
  "pbk",
  "sid",
  "spx",
  "type",
  "headerType",
];

pub fn parse_vless_uri(input: &str) -> XrayResult<ParsedVlessUri> {
  if input.trim() != input {
    return Err(XrayError::InvalidUri);
  }
  let url = Url::parse(input).map_err(|_| XrayError::InvalidUri)?;
  if url.scheme() != "vless" {
    return Err(XrayError::UnsupportedScheme);
  }
  if url.password().is_some() {
    return Err(XrayError::InvalidField {
      field: "id",
      reason: "password-style user information is not supported",
    });
  }
  if !matches!(url.path(), "" | "/") {
    return Err(XrayError::InvalidField {
      field: "path",
      reason: "VLESS TCP URIs must not contain a path",
    });
  }

  let raw_id = url.username();
  if raw_id.is_empty() {
    return Err(XrayError::MissingField("id"));
  }
  let id = Uuid::parse_str(raw_id)
    .map_err(|_| XrayError::InvalidField {
      field: "id",
      reason: "must be a UUID",
    })?
    .to_string();
  let address = match url.host().ok_or(XrayError::MissingField("address"))? {
    Host::Domain(value) => value.to_string(),
    Host::Ipv4(value) => value.to_string(),
    Host::Ipv6(value) => value.to_string(),
  };
  let port = url.port().ok_or(XrayError::MissingField("port"))?;

  let (parameters, unsupported) = parse_parameters(&url)?;

  // Transport first: it is the most common reason a real-world VLESS server is
  // unusable here, and it explains the stray parameters that come with it.
  match parameters.get("type").map(String::as_str) {
    None | Some("tcp" | "raw") => {}
    Some(_) => {
      return Err(XrayError::UnsupportedValue {
        field: "type",
        expected: "tcp",
      });
    }
  }
  require_value(&parameters, "security", "reality")?;
  require_value(&parameters, "flow", VlessFlow::Vision.as_str())?;
  optional_value(&parameters, "encryption", "none")?;
  optional_value(&parameters, "headerType", "none")?;

  // Only once the shape is known-good does an unrecognized parameter become
  // the most useful thing to report.
  if let Some(name) = unsupported.into_iter().next() {
    return Err(XrayError::UnsupportedParameter(name));
  }

  let server_name = required_parameter(&parameters, "sni")?.to_string();
  let public_key = required_parameter(&parameters, "pbk")?.to_string();
  let short_id = parameters.get("sid").cloned().unwrap_or_default();
  let spider_x = parameters
    .get("spx")
    .cloned()
    .unwrap_or_else(|| "/".to_string());
  let fingerprint = parameters
    .get("fp")
    .map(|value| RealityFingerprint::parse(value))
    .transpose()?
    .unwrap_or_default();

  let name = url
    .fragment()
    .filter(|fragment| !fragment.is_empty())
    .map(|fragment| {
      urlencoding::decode(fragment)
        .map(|value| value.into_owned())
        .map_err(|_| XrayError::InvalidField {
          field: "name",
          reason: "must use valid percent encoding",
        })
    })
    .transpose()?;

  let parsed = ParsedVlessUri {
    name,
    config: VlessRealityConfig {
      address,
      port,
      id,
      flow: VlessFlow::Vision,
      reality: RealitySettings {
        server_name,
        public_key,
        short_id,
        fingerprint,
        spider_x,
      },
    },
  };
  parsed.validate()?;
  Ok(parsed)
}

pub fn export_vless_uri(config: &VlessRealityConfig, name: Option<&str>) -> XrayResult<String> {
  config.validate()?;
  if let Some(name) = name {
    validate_display_name(name)?;
  }

  let mut url = Url::parse("vless://placeholder@127.0.0.1").expect("static VLESS URL is valid");
  url
    .set_username(&config.id)
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

  {
    let mut query = url.query_pairs_mut();
    query.append_pair("encryption", "none");
    query.append_pair("flow", config.flow.as_str());
    query.append_pair("security", "reality");
    query.append_pair("sni", &config.reality.server_name);
    query.append_pair("fp", config.reality.fingerprint.as_str());
    query.append_pair("pbk", &config.reality.public_key);
    query.append_pair("sid", &config.reality.short_id);
    query.append_pair("spx", &config.reality.spider_x);
    query.append_pair("type", "tcp");
    query.append_pair("headerType", "none");
  }
  // The parser percent-DECODES the fragment, so the exporter must encode it or
  // a name containing `%` (or `#`) comes back different every time the URI is
  // canonicalized — the name mutates a little more on each save.
  let encoded_name = name.map(|value| urlencoding::encode(value).into_owned());
  url.set_fragment(encoded_name.as_deref());
  Ok(url.into())
}

/// Split the query into recognized parameters and the names of the rest.
///
/// Unrecognized names are returned rather than rejected on the spot so the
/// caller can report the *shape* problem first. A WebSocket URI always carries
/// `path` (and usually `host`), gRPC carries `serviceName` — naming those keys
/// instead of the transport sends the user deleting parameters when the real
/// answer is that Donut only speaks plain TCP.
fn parse_parameters(url: &Url) -> XrayResult<(HashMap<String, String>, Vec<String>)> {
  let mut parameters = HashMap::new();
  let mut unsupported = Vec::new();
  for (name, value) in url.query_pairs() {
    if !SUPPORTED_PARAMETERS.contains(&name.as_ref()) {
      unsupported.push(name.into_owned());
      continue;
    }
    if parameters
      .insert(name.to_string(), value.into_owned())
      .is_some()
    {
      return Err(XrayError::DuplicateParameter(name.into_owned()));
    }
  }
  Ok((parameters, unsupported))
}

fn required_parameter<'a>(
  parameters: &'a HashMap<String, String>,
  name: &'static str,
) -> XrayResult<&'a str> {
  parameters
    .get(name)
    .filter(|value| !value.is_empty())
    .map(String::as_str)
    .ok_or(XrayError::MissingField(name))
}

fn require_value(
  parameters: &HashMap<String, String>,
  name: &'static str,
  expected: &'static str,
) -> XrayResult<()> {
  let value = required_parameter(parameters, name)?;
  if value != expected {
    return Err(XrayError::UnsupportedValue {
      field: name,
      expected,
    });
  }
  Ok(())
}

fn optional_value(
  parameters: &HashMap<String, String>,
  name: &'static str,
  expected: &'static str,
) -> XrayResult<()> {
  if parameters.get(name).is_some_and(|value| value != expected) {
    return Err(XrayError::UnsupportedValue {
      field: name,
      expected,
    });
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

  use super::*;

  const ID: &str = "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4";

  /// Donut accepts exactly one VLESS shape, so most rejections mean "your
  /// server is a kind we do not support" rather than "you mistyped". These pin
  /// the reason each rejection reports, because the UI turns it into the one
  /// sentence that tells a user with a working WebSocket or plain-TLS server
  /// why Donut will not take it.
  #[test]
  fn unsupported_setups_report_which_part_is_unsupported() {
    let good = format!(
      "vless://{ID}@example.com:443?security=reality&flow=xtls-rprx-vision\
&encryption=none&type=tcp&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4&sid=00&fp=chrome"
    );
    assert!(parse_vless_uri(&good).is_ok(), "baseline URI must parse");

    let reason = |uri: &str| parse_vless_uri(uri).unwrap_err().reason_code();

    // Plain TLS instead of REALITY — the most common real-world setup.
    assert_eq!(
      reason(&good.replace("security=reality", "security=tls")),
      "security"
    );
    assert_eq!(
      reason(&good.replace("flow=xtls-rprx-vision", "flow=none")),
      "flow"
    );
    // WebSocket / gRPC transports.
    assert_eq!(reason(&good.replace("type=tcp", "type=ws")), "transport");
    assert_eq!(reason(&good.replace("type=tcp", "type=grpc")), "transport");
    assert_eq!(reason(&good.replace("&sni=a.com", "")), "sni");
    assert_eq!(
      reason(&good.replace("&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4", "")),
      "publicKey"
    );
    assert_eq!(reason(&good.replace("vless://", "vmess://")), "scheme");
    assert_eq!(reason("not a uri"), "malformed");
  }

  /// The URIs users actually paste, not canonical-REALITY-with-one-field-changed.
  ///
  /// A real WebSocket link carries `path` (and usually `host`); a gRPC link
  /// carries `serviceName`. Those keys are not in SUPPORTED_PARAMETERS, so
  /// before the shape was checked first they produced "unsupported option"
  /// and sent the user deleting query parameters instead of telling them
  /// Donut only speaks plain TCP.
  #[test]
  fn a_display_name_survives_an_export_parse_round_trip() {
    // Percent signs are legal in a fragment, so they used to pass through
    // unencoded and then get decoded on the way back in — "50% off" became
    // "50 off"-ish and drifted further on every canonicalizing save.
    for name in ["50% off", "a#b", "spaced name", "100%25", "üñî"] {
      let parsed = parse_vless_uri(&format!(
        "vless://{ID}@example.com:443?security=reality&flow=xtls-rprx-vision\
&encryption=none&type=tcp&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4"
      ))
      .expect("baseline parses");

      let exported = export_vless_uri(&parsed.config, Some(name)).expect("exports");
      let reparsed = parse_vless_uri(&exported).expect("re-parses");
      assert_eq!(
        reparsed.name.as_deref(),
        Some(name),
        "display name mutated across a round trip: {exported}"
      );

      // And a second round trip must be a fixed point, not drift again.
      let exported_again =
        export_vless_uri(&reparsed.config, reparsed.name.as_deref()).expect("re-exports");
      assert_eq!(exported, exported_again);
    }
  }

  #[test]
  fn real_world_websocket_and_grpc_links_name_the_transport() {
    let ws = format!(
      "vless://{ID}@cdn.example.com:443?encryption=none&security=tls&type=ws\
&path=%2Fray&host=cdn.example.com&sni=cdn.example.com#WS%20node"
    );
    assert_eq!(
      parse_vless_uri(&ws).unwrap_err().reason_code(),
      "transport",
      "a WebSocket link must be told its transport is unsupported"
    );

    let grpc = format!(
      "vless://{ID}@grpc.example.com:443?encryption=none&security=reality&type=grpc\
&serviceName=gun&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4"
    );
    assert_eq!(
      parse_vless_uri(&grpc).unwrap_err().reason_code(),
      "transport"
    );

    // A genuinely unknown option on an otherwise-supported URI still reports
    // as a parameter problem, which is the accurate answer there.
    let odd = format!(
      "vless://{ID}@example.com:443?security=reality&flow=xtls-rprx-vision\
&encryption=none&type=tcp&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4&madeUpKey=1"
    );
    assert_eq!(
      parse_vless_uri(&odd).unwrap_err().reason_code(),
      "parameter"
    );
  }

  fn public_key() -> String {
    URL_SAFE_NO_PAD.encode([7_u8; 32])
  }

  fn uri(overrides: &[(&str, &str)]) -> String {
    let key = public_key();
    let mut parameters = vec![
      ("encryption", "none"),
      ("flow", "xtls-rprx-vision"),
      ("security", "reality"),
      ("sni", "www.example.com"),
      ("fp", "chrome"),
      ("pbk", key.as_str()),
      ("sid", "0123456789abcdef"),
      ("spx", "/"),
      ("type", "tcp"),
      ("headerType", "none"),
    ];
    for (name, value) in overrides {
      if let Some(parameter) = parameters.iter_mut().find(|(key, _)| key == name) {
        parameter.1 = value;
      } else {
        parameters.push((name, value));
      }
    }
    let query = parameters
      .into_iter()
      .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
      .collect::<Vec<_>>()
      .join("&");
    format!("vless://{ID}@vpn.example.com:443?{query}#Primary")
  }

  #[test]
  fn parses_supported_reality_vision_uri() {
    let parsed = parse_vless_uri(&uri(&[])).unwrap();
    assert_eq!(parsed.name.as_deref(), Some("Primary"));
    assert_eq!(parsed.config.address, "vpn.example.com");
    assert_eq!(parsed.config.port, 443);
    assert_eq!(parsed.config.id, ID);
    assert_eq!(parsed.config.flow, VlessFlow::Vision);
    assert_eq!(parsed.config.reality.server_name, "www.example.com");
    assert_eq!(parsed.config.reality.public_key, public_key());
    assert_eq!(parsed.config.reality.short_id, "0123456789abcdef");
    assert_eq!(
      parsed.config.reality.fingerprint,
      RealityFingerprint::Chrome
    );
    assert_eq!(parsed.config.reality.spider_x, "/");
  }

  #[test]
  fn parses_ipv6_and_percent_encoded_metadata() {
    let input = uri(&[("spx", "/search?q=hello world")])
      .replace("vpn.example.com", "[2001:db8::1]")
      .replace("#Primary", "#Home%20server");
    let parsed = parse_vless_uri(&input).unwrap();
    assert_eq!(parsed.config.address, "2001:db8::1");
    assert_eq!(parsed.config.reality.spider_x, "/search?q=hello world");
    assert_eq!(parsed.name.as_deref(), Some("Home server"));
  }

  #[test]
  fn applies_only_safe_optional_defaults() {
    let key = public_key();
    let input = format!(
      "vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&sni=www.example.com&pbk={key}"
    );
    let parsed = parse_vless_uri(&input).unwrap();
    assert_eq!(
      parsed.config.reality.fingerprint,
      RealityFingerprint::Chrome
    );
    assert_eq!(parsed.config.reality.short_id, "");
    assert_eq!(parsed.config.reality.spider_x, "/");
  }

  #[test]
  fn accepts_raw_as_tcp_alias() {
    assert!(parse_vless_uri(&uri(&[("type", "raw")])).is_ok());
  }

  #[test]
  fn rejects_wrong_scheme_credentials_path_and_missing_port() {
    let valid = uri(&[]);
    let cases = [
      valid.replacen("vless://", "https://", 1),
      valid.replacen(ID, &format!("{ID}:password"), 1),
      valid.replacen(":443?", ":443/path?", 1),
      valid.replacen(":443?", "?", 1),
    ];
    for input in cases {
      assert!(parse_vless_uri(&input).is_err(), "{input}");
    }
  }

  #[test]
  fn rejects_missing_required_reality_values() {
    let key = public_key();
    let cases = [
      format!("vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&sni=www.example.com&pbk={key}"),
      format!("vless://{ID}@vpn.example.com:443?security=reality&sni=www.example.com&pbk={key}"),
      format!("vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&pbk={key}"),
      format!("vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&sni=www.example.com"),
    ];
    for input in cases {
      assert!(parse_vless_uri(&input).is_err(), "{input}");
    }
  }

  #[test]
  fn rejects_unsupported_security_transport_flow_and_encryption() {
    for (name, value) in [
      ("security", "tls"),
      ("type", "ws"),
      ("flow", ""),
      ("encryption", "auto"),
      ("headerType", "http"),
      ("fp", "unsafe"),
    ] {
      assert!(
        matches!(
          parse_vless_uri(&uri(&[(name, value)])),
          Err(XrayError::UnsupportedValue { .. } | XrayError::MissingField(_))
        ),
        "{name}={value}"
      );
    }
  }

  #[test]
  fn rejects_unknown_and_duplicate_parameters() {
    assert_eq!(
      parse_vless_uri(&uri(&[("serviceName", "unsupported")])),
      Err(XrayError::UnsupportedParameter("serviceName".to_string()))
    );
    let input = uri(&[]).replace("#Primary", "&sni=duplicate.example.com#Primary");
    assert_eq!(
      parse_vless_uri(&input),
      Err(XrayError::DuplicateParameter("sni".to_string()))
    );
  }

  #[test]
  fn rejects_invalid_uuid_key_short_id_and_spider_x_without_echoing_secrets() {
    let invalid_key = "private-value-that-must-not-be-echoed";
    let cases = [
      uri(&[]).replacen(ID, "not-a-uuid", 1),
      uri(&[("pbk", invalid_key)]),
      uri(&[("sid", "xyz")]),
      uri(&[("spx", "relative")]),
    ];
    for input in cases {
      let error = parse_vless_uri(&input).unwrap_err();
      assert!(!error.to_string().contains(invalid_key));
      assert!(!error.to_string().contains(ID));
    }
  }

  #[test]
  fn export_is_canonical_and_round_trips() {
    let parsed = parse_vless_uri(&uri(&[("fp", "firefox")])).unwrap();
    let exported = export_vless_uri(&parsed.config, Some("Home server")).unwrap();
    assert!(exported.starts_with(&format!("vless://{ID}@vpn.example.com:443?")));
    assert!(exported.contains("type=tcp"));
    assert!(exported.contains("flow=xtls-rprx-vision"));
    assert!(exported.ends_with("#Home%20server"));

    let reparsed = parse_vless_uri(&exported).unwrap();
    assert_eq!(
      reparsed,
      ParsedVlessUri {
        name: Some("Home server".to_string()),
        config: parsed.config,
      }
    );
  }

  #[test]
  fn export_handles_ipv6_host() {
    let mut parsed = parse_vless_uri(&uri(&[])).unwrap();
    parsed.config.address = "2001:db8::1".to_string();
    let exported = export_vless_uri(&parsed.config, None).unwrap();
    assert!(exported.starts_with(&format!("vless://{ID}@[2001:db8::1]:443?")));
    assert_eq!(
      parse_vless_uri(&exported).unwrap().config.address,
      "2001:db8::1"
    );
  }
}

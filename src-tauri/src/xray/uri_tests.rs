use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::json;

use super::*;

const ID: &str = "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4";
const KEY: &str = "mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4";

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

fn vmess(json: serde_json::Value) -> String {
  format!("vmess://{}", STANDARD.encode(json.to_string()))
}

fn reason(link: &str) -> &'static str {
  parse_share_link(link).unwrap_err().reason_code()
}

/// Parse, export, parse again: the configuration must survive and the
/// exported link must be a fixed point, or every save rewrites the link.
fn assert_round_trip(link: &str) -> ParsedShareLink {
  let parsed = parse_share_link(link).unwrap_or_else(|error| panic!("{link}: {error}"));
  let exported = export_share_link(&parsed.config, parsed.name.as_deref()).unwrap();
  let reparsed = parse_share_link(&exported).unwrap_or_else(|error| panic!("{exported}: {error}"));
  assert_eq!(reparsed, parsed, "{link} -> {exported}");
  let exported_again = export_share_link(&reparsed.config, reparsed.name.as_deref()).unwrap();
  assert_eq!(exported, exported_again);
  parsed
}

#[test]
fn parses_supported_reality_vision_uri() {
  let parsed = parse_share_link(&uri(&[])).unwrap();
  assert_eq!(parsed.name.as_deref(), Some("Primary"));
  assert_eq!(parsed.config.address, "vpn.example.com");
  assert_eq!(parsed.config.port, 443);
  assert_eq!(parsed.config.protocol(), XrayProtocol::Vless);
  let XrayOutbound::Vless { id, flow, stream } = parsed.config.outbound else {
    panic!("not VLESS");
  };
  assert_eq!(id, ID);
  assert_eq!(flow, Some(VlessFlow::Vision));
  assert_eq!(stream.transport, Transport::Raw { http_header: None });
  assert_eq!(
    stream.security,
    Security::Reality(RealitySettings {
      server_name: "www.example.com".to_string(),
      public_key: public_key(),
      short_id: "0123456789abcdef".to_string(),
      fingerprint: Fingerprint::Chrome,
      spider_x: "/".to_string(),
    })
  );
}

/// Stored links and the Xray worker reuse check compare the canonical string,
/// so the one shape Donut took before more protocols arrived must still come
/// out byte for byte the same.
#[test]
fn reality_vision_export_keeps_its_original_canonical_form() {
  let parsed = parse_share_link(&uri(&[("fp", "firefox")])).unwrap();
  let exported = export_share_link(&parsed.config, Some("Home server")).unwrap();
  assert_eq!(
    exported,
    format!(
      "vless://{ID}@vpn.example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality\
&sni=www.example.com&fp=firefox&pbk={}&sid=0123456789abcdef&spx=%2F&type=tcp&headerType=none\
#Home%20server",
      public_key()
    )
  );
}

/// The links providers actually hand out, as v2rayN, V2Board/Xboard and the
/// Hysteria client export them.
#[test]
fn real_world_links_parse_and_round_trip() {
  let pin = "AB:".repeat(31) + "AB";
  let links = [
    // Plain VLESS over WebSocket + TLS behind a CDN, no flow.
    format!(
      "vless://{ID}@104.16.1.1:443?encryption=none&security=tls&sni=cdn.example.com\
&alpn=http%2F1.1&fp=chrome&type=ws&host=cdn.example.com&path=%2Fray%3Fed%3D2048#WS%20node"
    ),
    // VLESS with no security at all, the "just isolation" case.
    format!("vless://{ID}@node.example.com:8080?encryption=none&security=none&type=tcp#Plain"),
    // VLESS over gRPC and over XHTTP with REALITY.
    format!(
      "vless://{ID}@grpc.example.com:443?encryption=none&security=reality&type=grpc\
&serviceName=gun&mode=multi&sni=a.com&pbk={KEY}&sid=00&fp=safari"
    ),
    format!(
      "vless://{ID}@x.example.com:443?encryption=none&security=reality&type=xhttp&path=%2Fx\
&mode=packet-up&sni=a.com&pbk={KEY}"
    ),
    // VLESS + Vision + TLS over RAW.
    format!(
      "vless://{ID}@tls.example.com:443?encryption=none&flow=xtls-rprx-vision&security=tls\
&sni=tls.example.com&fp=chrome&type=tcp&headerType=none"
    ),
    // Xboard's Trojan export: legacy `peer`, `allowInsecure` and no security.
    "trojan://S3cr3t%2Bpass@tj.example.com:443?allowInsecure=1&peer=tj.example.com\
&sni=tj.example.com#Trojan%20HK"
      .to_string(),
    format!("trojan://pw@tj.example.com:443?security=tls&type=ws&path=%2Ftj&pcs={pin}"),
    "trojan://pw@tj.example.com:443?security=tls&type=grpc&serviceName=tj&sni=tj.example.com"
      .to_string(),
    // VMess in the URL form of the Xray share-link standard.
    format!(
      "vmess://{ID}@vm.example.com:443?encryption=auto&security=tls&type=httpupgrade&path=%2Fu"
    ),
    // Hysteria2 in both schemes, with and without obfuscation.
    "hysteria2://letmein@hy.example.com:443/?sni=hy.example.com&insecure=1#HY2".to_string(),
    format!(
      "hy2://user:pass@hy.example.com:8443?obfs=salamander&obfs-password=salt&pinSHA256={pin}"
    ),
    "hysteria2://letmein@[2001:db8::1]:443/".to_string(),
  ];
  for link in links {
    assert_round_trip(&link);
  }
}

#[test]
fn v2rayn_vmess_json_parses_and_round_trips() {
  let ws = vmess(json!({
    "v": "2", "ps": "VMess WS", "add": "vm.example.com", "port": "443", "id": ID, "aid": "0",
    "scy": "auto", "net": "ws", "type": "none", "host": "cdn.example.com", "path": "/vm",
    "tls": "tls", "sni": "", "alpn": "", "fp": "chrome"
  }));
  let parsed = assert_round_trip(&ws);
  assert_eq!(parsed.name.as_deref(), Some("VMess WS"));
  assert_eq!(parsed.config.protocol(), XrayProtocol::Vmess);
  let XrayOutbound::Vmess {
    id,
    security,
    stream,
  } = parsed.config.outbound
  else {
    panic!("not VMess");
  };
  assert_eq!(id, ID);
  assert_eq!(security, VmessSecurity::Auto);
  assert_eq!(
    stream.transport,
    Transport::WebSocket {
      host: Some("cdn.example.com".to_string()),
      path: "/vm".to_string(),
    }
  );
  let Security::Tls(tls) = stream.security else {
    panic!("not TLS");
  };
  assert_eq!(tls.server_name.as_deref(), Some("cdn.example.com"));
  assert_eq!(tls.fingerprint, Some(Fingerprint::Chrome));

  // Numbers instead of strings, an old alterId, gRPC with its service in
  // `path` and its mode in `type`, and a TCP HTTP disguise.
  for link in [
    vmess(json!({
      "v": 2, "ps": "", "add": "203.0.113.9", "port": 8443, "id": ID, "aid": 64,
      "scy": "aes-128-gcm", "net": "grpc", "type": "multi", "host": "", "path": "svc", "tls": "tls"
    })),
    vmess(json!({
      "v": "2", "ps": "tcp", "add": "vm.example.com", "port": "80", "id": ID, "aid": "0",
      "scy": "none", "net": "tcp", "type": "http", "host": "a.com", "path": "/", "tls": ""
    })),
  ] {
    assert_round_trip(&link);
  }
}

#[test]
fn vmess_json_reads_padding_free_url_safe_base64_and_a_fragment_name() {
  let body = json!({ "add": "vm.example.com", "port": "443", "id": ID, "net": "tcp" }).to_string();
  let link = format!("vmess://{}#From%20fragment", URL_SAFE_NO_PAD.encode(body));
  let parsed = parse_share_link(&link).unwrap();
  assert_eq!(parsed.name.as_deref(), Some("From fragment"));
  assert_eq!(parsed.config.port, 443);
}

#[test]
fn hysteria2_defaults_port_and_server_name_and_joins_userpass_auth() {
  let parsed = parse_share_link("hy2://user:p%40ss@hy.example.com").unwrap();
  assert_eq!(parsed.config.port, 443);
  let XrayOutbound::Hysteria2 { auth, tls, .. } = parsed.config.outbound else {
    panic!("not Hysteria2");
  };
  assert_eq!(auth, "user:p@ss");
  assert_eq!(tls.server_name.as_deref(), Some("hy.example.com"));
}

#[test]
fn trojan_defaults_to_tls_and_takes_peer_as_server_name() {
  let parsed = parse_share_link("trojan://pw@tj.example.com:443?peer=front.example.com").unwrap();
  let XrayOutbound::Trojan { password, stream } = parsed.config.outbound else {
    panic!("not Trojan");
  };
  assert_eq!(password, "pw");
  let Security::Tls(tls) = stream.security else {
    panic!("Trojan must default to TLS");
  };
  assert_eq!(tls.server_name.as_deref(), Some("front.example.com"));
}

#[test]
fn a_percent_escape_in_a_secret_survives_saving() {
  // `pa%2541ss` is the password `pa%41ss`; written back unescaped it read as
  // `paAss` on the next load and authentication failed.
  for link in [
    "trojan://pa%2541ss@tj.example.com:443?sni=tj.example.com",
    "hysteria2://u%25s:p%2541@hy.example.com:443/",
  ] {
    let parsed = assert_round_trip(link);
    match parsed.config.outbound {
      XrayOutbound::Trojan { password, .. } => assert_eq!(password, "pa%41ss"),
      XrayOutbound::Hysteria2 { auth, .. } => assert_eq!(auth, "u%s:p%41"),
      other => panic!("unexpected {other:?}"),
    }
  }
}

#[test]
fn a_salamander_key_xray_would_refuse_is_refused_on_import() {
  let short = "hy2://pw@hy.example.com?obfs=salamander&obfs-password=abc";
  assert_eq!(reason(short), "malformed");
  assert!(parse_share_link(&short.replace("=abc", "=abcd")).is_ok());
}

#[test]
fn an_internationalized_server_name_is_read_as_punycode() {
  // Links stored before more protocols arrived kept such names verbatim; they
  // must still load.
  let parsed = parse_share_link(&uri(&[("sni", "café.example.com")])).unwrap();
  let XrayOutbound::Vless {
    stream: StreamSettings {
      security: Security::Reality(reality),
      ..
    },
    ..
  } = parsed.config.outbound
  else {
    panic!("not VLESS REALITY");
  };
  assert_eq!(reality.server_name, "xn--caf-dma.example.com");

  let tls = assert_round_trip(
    "trojan://pw@tj.example.com:443?sni=caf%C3%A9.example.com&type=ws&host=caf%C3%A9.example.com",
  );
  let XrayOutbound::Trojan { stream, .. } = tls.config.outbound else {
    panic!("not Trojan");
  };
  assert_eq!(stream.transport.host(), Some("xn--caf-dma.example.com"));
  let Security::Tls(settings) = stream.security else {
    panic!("not TLS");
  };
  assert_eq!(
    settings.server_name.as_deref(),
    Some("xn--caf-dma.example.com")
  );
}

#[test]
fn insecure_flags_are_dropped_and_the_certificate_is_still_checked() {
  for link in [
    "trojan://pw@tj.example.com:443?allowInsecure=1&sni=tj.example.com",
    "hysteria2://pw@hy.example.com:443/?insecure=1",
  ] {
    let parsed = parse_share_link(link).unwrap();
    let exported = export_share_link(&parsed.config, None).unwrap();
    assert!(!exported.contains("nsecure"), "{exported}");
  }
}

/// Most rejections mean "your server is a kind we do not support" rather than
/// "you mistyped". These pin the reason each rejection reports, because the
/// UI turns it into the one sentence that tells the user which part of their
/// setup Donut cannot use.
#[test]
fn unsupported_setups_report_which_part_is_unsupported() {
  let good = format!(
    "vless://{ID}@example.com:443?security=reality&flow=xtls-rprx-vision\
&encryption=none&type=tcp&sni=a.com&pbk={KEY}&sid=00&fp=chrome"
  );
  assert!(parse_share_link(&good).is_ok(), "baseline URI must parse");

  let cases = [
    // Transports Xray-core dropped or Donut does not speak. mKCP brings its own
    // `seed` option, which must not hide the real problem.
    (good.replace("type=tcp", "type=kcp&seed=x"), "transport"),
    (good.replace("type=tcp", "type=h2"), "transport"),
    (good.replace("type=tcp", "type=quic"), "transport"),
    (good.replace("type=tcp", "type=grpc&mode=tun"), "transport"),
    (
      good.replace("security=reality", "security=xtls"),
      "security",
    ),
    (
      good
        .replace("type=tcp", "type=ws")
        .replace("&flow=xtls-rprx-vision", ""),
      "security",
    ),
    (
      good
        .replace("type=tcp", "type=ws")
        .replace("security=reality", "security=tls"),
      "flow",
    ),
    (
      good.replace("flow=xtls-rprx-vision", "flow=xtls-rprx-direct"),
      "flow",
    ),
    (
      good.replace(
        "encryption=none",
        "encryption=mlkem768x25519plus.native.0rtt.abc",
      ),
      "encryption",
    ),
    (
      good.replace("type=tcp", "type=tcp&headerType=srtp"),
      "headerType",
    ),
    (good.replace("fp=chrome", "fp=unsafe"), "fingerprint"),
    (good.replace("&sni=a.com", ""), "sni"),
    (good.replace(&format!("&pbk={KEY}"), ""), "publicKey"),
    (good.replace("vless://", "ss://"), "scheme"),
    (good.replace("vless://", "https://"), "scheme"),
    (format!("{good}&madeUpKey=1"), "parameter"),
    ("not a uri".to_string(), "malformed"),
    (
      "trojan://pw@tj.example.com:443?flow=xtls-rprx-vision".to_string(),
      "flow",
    ),
    (
      format!("vmess://{ID}@vm.example.com:443?encryption=rc4"),
      "encryption",
    ),
    (
      "hysteria2://pw@hy.example.com:443/?obfs=gost".to_string(),
      "obfs",
    ),
    (
      "hysteria2://pw@hy.example.com:443/?mport=20000-30000".to_string(),
      "parameter",
    ),
    (
      vmess(
        json!({ "add": "vm.example.com", "port": "443", "id": ID, "net": "kcp", "type": "wechat-video" }),
      ),
      "transport",
    ),
    (
      vmess(
        json!({ "add": "vm.example.com", "port": "443", "id": ID, "net": "tcp", "tls": "xtls" }),
      ),
      "security",
    ),
    ("vmess://bm90IGpzb24=".to_string(), "malformed"),
  ];
  for (link, expected) in cases {
    assert_eq!(reason(&link), expected, "{link}");
  }
}

#[test]
fn an_empty_unknown_option_is_not_worth_refusing_a_link_over() {
  assert!(parse_share_link(&format!("{}&ech=&pqv=", uri(&[]).replace("#Primary", ""))).is_ok());
}

#[test]
fn a_display_name_survives_an_export_parse_round_trip() {
  // Percent signs are legal in a fragment, so they used to pass through
  // unencoded and then get decoded on the way back in — "50% off" became
  // "50 off"-ish and drifted further on every canonicalizing save.
  for name in ["50% off", "a#b", "spaced name", "100%25", "üñî"] {
    let parsed = parse_share_link(&uri(&[])).expect("baseline parses");
    let exported = export_share_link(&parsed.config, Some(name)).expect("exports");
    let reparsed = parse_share_link(&exported).expect("re-parses");
    assert_eq!(
      reparsed.name.as_deref(),
      Some(name),
      "display name mutated across a round trip: {exported}"
    );
    let exported_again =
      export_share_link(&reparsed.config, reparsed.name.as_deref()).expect("re-exports");
    assert_eq!(exported, exported_again);
  }
}

#[test]
fn parses_ipv6_and_percent_encoded_metadata() {
  let input = uri(&[("spx", "/search?q=hello world")])
    .replace("vpn.example.com", "[2001:db8::1]")
    .replace("#Primary", "#Home%20server");
  let parsed = parse_share_link(&input).unwrap();
  assert_eq!(parsed.config.address, "2001:db8::1");
  assert_eq!(parsed.name.as_deref(), Some("Home server"));
  let XrayOutbound::Vless {
    stream: StreamSettings {
      security: Security::Reality(reality),
      ..
    },
    ..
  } = parsed.config.outbound
  else {
    panic!("not VLESS REALITY");
  };
  assert_eq!(reality.spider_x, "/search?q=hello world");
}

#[test]
fn an_internationalized_host_is_stored_as_punycode() {
  let input = uri(&[]).replace("vpn.example.com", "café.example.com");
  let parsed = parse_share_link(&input).unwrap();
  assert_eq!(parsed.config.address, "xn--caf-dma.example.com");

  // And the canonical form survives an export/import round trip.
  let exported = export_share_link(&parsed.config, None).unwrap();
  assert_eq!(
    parse_share_link(&exported).unwrap().config.address,
    "xn--caf-dma.example.com"
  );
}

#[test]
fn rejects_a_host_that_percent_decodes_into_something_undialable() {
  let input = uri(&[]).replace("vpn.example.com", "vpn%2Fexample.com");
  assert!(matches!(
    parse_share_link(&input),
    Err(XrayError::InvalidField {
      field: "address",
      ..
    })
  ));
}

#[test]
fn applies_only_safe_optional_defaults() {
  let key = public_key();
  let input = format!(
    "vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&sni=www.example.com&pbk={key}"
  );
  let parsed = parse_share_link(&input).unwrap();
  let XrayOutbound::Vless {
    stream: StreamSettings {
      security: Security::Reality(reality),
      ..
    },
    ..
  } = parsed.config.outbound
  else {
    panic!("not VLESS REALITY");
  };
  assert_eq!(reality.fingerprint, Fingerprint::Chrome);
  assert_eq!(reality.short_id, "");
  assert_eq!(reality.spider_x, "/");
}

#[test]
fn accepts_raw_as_tcp_alias() {
  assert!(parse_share_link(&uri(&[("type", "raw")])).is_ok());
}

#[test]
fn rejects_wrong_scheme_credentials_path_and_missing_port() {
  let valid = uri(&[]);
  let cases = [
    valid.replacen("vless://", "https://", 1),
    valid.replacen(ID, &format!("{ID}:password"), 1),
    valid.replacen(":443?", ":443/path?", 1),
    valid.replacen(":443?", "?", 1),
    format!(" {valid}"),
  ];
  for input in cases {
    assert!(parse_share_link(&input).is_err(), "{input}");
  }
}

#[test]
fn rejects_missing_required_reality_values() {
  let key = public_key();
  let cases = [
    // No security means none, and Vision needs TLS or REALITY.
    format!("vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&sni=www.example.com&pbk={key}"),
    format!("vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&pbk={key}"),
    format!(
      "vless://{ID}@vpn.example.com:443?flow=xtls-rprx-vision&security=reality&sni=www.example.com"
    ),
  ];
  for input in cases {
    assert!(parse_share_link(&input).is_err(), "{input}");
  }
}

#[test]
fn rejects_unknown_and_duplicate_parameters() {
  assert_eq!(
    parse_share_link(&uri(&[("madeUp", "unsupported")])),
    Err(XrayError::UnsupportedParameter("madeUp".to_string()))
  );
  let input = uri(&[]).replace("#Primary", "&sni=duplicate.example.com#Primary");
  assert_eq!(
    parse_share_link(&input),
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
    let error = parse_share_link(&input).unwrap_err();
    assert!(!error.to_string().contains(invalid_key));
    assert!(!error.to_string().contains(ID));
  }
  let error = parse_share_link("trojan://hunter2@tj.example.com:443?type=kcp").unwrap_err();
  assert!(!error.to_string().contains("hunter2"));
}

#[test]
fn export_handles_ipv6_host() {
  let mut parsed = parse_share_link(&uri(&[])).unwrap();
  parsed.config.address = "2001:db8::1".to_string();
  let exported = export_share_link(&parsed.config, None).unwrap();
  assert!(exported.starts_with(&format!("vless://{ID}@[2001:db8::1]:443?")));
  assert_eq!(
    parse_share_link(&exported).unwrap().config.address,
    "2001:db8::1"
  );
}

#[test]
fn share_link_host_names_the_server_even_for_links_donut_refuses() {
  assert_eq!(
    share_link_host("vless://uuid@example.com:443?sni=a@b.com&x=1#my@label").as_deref(),
    Some("example.com")
  );
  assert_eq!(
    share_link_host("vless://uuid@[2606:4700::1111]:443?type=ws").as_deref(),
    Some("2606:4700::1111")
  );
  assert_eq!(
    share_link_host(&vmess(json!({ "add": "VM.example.com", "net": "kcp" }))).as_deref(),
    Some("VM.example.com")
  );
  assert_eq!(
    share_link_host("hysteria2://pw@hy.example.com:20000-30000/").as_deref(),
    Some("hy.example.com")
  );
  assert_eq!(share_link_host("not-a-share-link"), None);
}

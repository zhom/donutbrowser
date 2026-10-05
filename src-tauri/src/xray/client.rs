use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
  GrpcMode, Security, StreamSettings, TlsSettings, Transport, XrayConfig, XrayError, XrayOutbound,
  XrayResult,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XrayClientRuntime {
  pub listen_port: u16,
  pub username: String,
  pub password: String,
}

impl XrayClientRuntime {
  pub fn validate(&self) -> XrayResult<()> {
    if self.listen_port == 0 {
      return Err(XrayError::InvalidField {
        field: "listen_port",
        reason: "must be between 1 and 65535",
      });
    }
    validate_socks_credential("username", &self.username)?;
    validate_socks_credential("password", &self.password)
  }
}

pub fn build_client_config(config: &XrayConfig, runtime: &XrayClientRuntime) -> XrayResult<Value> {
  config.validate()?;
  runtime.validate()?;

  Ok(json!({
    "log": {
      "loglevel": "warning"
    },
    "inbounds": [{
      "tag": "local-socks",
      "listen": "127.0.0.1",
      "port": runtime.listen_port,
      "protocol": "socks",
      "settings": {
        "auth": "password",
        "accounts": [{
          "user": runtime.username,
          "pass": runtime.password
        }],
        "udp": true,
        "ip": "127.0.0.1"
      }
    }],
    "outbounds": [outbound(config)]
  }))
}

fn outbound(config: &XrayConfig) -> Value {
  match &config.outbound {
    XrayOutbound::Vless { id, flow, stream } => {
      let mut user = json!({ "id": id, "encryption": "none" });
      if let Some(flow) = flow {
        user["flow"] = json!(flow.as_str());
      }
      json!({
        "tag": "proxy",
        "protocol": "vless",
        "settings": {
          "vnext": [{ "address": config.address, "port": config.port, "users": [user] }]
        },
        "streamSettings": stream_settings(stream)
      })
    }
    XrayOutbound::Vmess {
      id,
      security,
      stream,
    } => json!({
      "tag": "proxy",
      "protocol": "vmess",
      "settings": {
        "vnext": [{
          "address": config.address,
          "port": config.port,
          "users": [{ "id": id, "security": security.as_str() }]
        }]
      },
      "streamSettings": stream_settings(stream)
    }),
    XrayOutbound::Trojan { password, stream } => json!({
      "tag": "proxy",
      "protocol": "trojan",
      "settings": {
        "servers": [{ "address": config.address, "port": config.port, "password": password }]
      },
      "streamSettings": stream_settings(stream)
    }),
    XrayOutbound::Hysteria2 {
      auth,
      tls,
      obfs_password,
    } => {
      // HTTP/3 is the only protocol a Hysteria2 server answers, and Xray-core
      // offers h2 and http/1.1 unless told otherwise.
      let mut tls_settings = tls_settings(tls);
      tls_settings["alpn"] = json!(["h3"]);
      let mut stream = json!({
        "network": "hysteria",
        "hysteriaSettings": { "version": 2, "auth": auth },
        "security": "tls",
        "tlsSettings": tls_settings
      });
      if let Some(password) = obfs_password {
        stream["finalmask"] = json!({
          "udp": [{ "type": "salamander", "settings": { "password": password } }]
        });
      }
      json!({
        "tag": "proxy",
        "protocol": "hysteria",
        "settings": { "version": 2, "address": config.address, "port": config.port },
        "streamSettings": stream
      })
    }
  }
}

fn stream_settings(stream: &StreamSettings) -> Value {
  let mut settings = json!({
    "network": stream.transport.as_str(),
    "security": stream.security.as_str(),
    "sockopt": {
      "tcpKeepAliveIdle": 30,
      "tcpKeepAliveInterval": 15
    }
  });
  match &stream.security {
    Security::None => {}
    Security::Tls(tls) => settings["tlsSettings"] = tls_settings(tls),
    Security::Reality(reality) => {
      settings["realitySettings"] = json!({
        "show": false,
        "fingerprint": reality.fingerprint.as_str(),
        "serverName": reality.server_name,
        "publicKey": reality.public_key,
        "shortId": reality.short_id,
        "spiderX": reality.spider_x
      });
    }
  }
  match &stream.transport {
    Transport::Raw { http_header: None } => {}
    Transport::Raw {
      http_header: Some(header),
    } => {
      let mut request = json!({ "path": [header.path] });
      if !header.hosts.is_empty() {
        request["headers"] = json!({ "Host": header.hosts });
      }
      settings["tcpSettings"] = json!({ "header": { "type": "http", "request": request } });
    }
    Transport::WebSocket { host, path } => {
      settings["wsSettings"] = path_and_host(path, host.as_deref());
    }
    Transport::HttpUpgrade { host, path } => {
      settings["httpupgradeSettings"] = path_and_host(path, host.as_deref());
    }
    Transport::Xhttp { host, path, mode } => {
      let mut xhttp = path_and_host(path, host.as_deref());
      xhttp["mode"] = json!(mode.as_str());
      settings["xhttpSettings"] = xhttp;
    }
    Transport::Grpc {
      service_name,
      authority,
      mode,
    } => {
      let mut grpc = json!({
        "serviceName": service_name,
        "multiMode": *mode == GrpcMode::Multi
      });
      if let Some(authority) = authority {
        grpc["authority"] = json!(authority);
      }
      settings["grpcSettings"] = grpc;
    }
  }
  settings
}

fn path_and_host(path: &str, host: Option<&str>) -> Value {
  let mut value = json!({ "path": path });
  if let Some(host) = host {
    value["host"] = json!(host);
  }
  value
}

fn tls_settings(tls: &TlsSettings) -> Value {
  let mut value = json!({});
  if let Some(server_name) = &tls.server_name {
    value["serverName"] = json!(server_name);
  }
  if !tls.alpn.is_empty() {
    value["alpn"] = json!(tls.alpn);
  }
  if let Some(fingerprint) = tls.fingerprint {
    value["fingerprint"] = json!(fingerprint.as_str());
  }
  if !tls.pinned_peer_cert_sha256.is_empty() {
    value["pinnedPeerCertSha256"] = json!(tls.pinned_peer_cert_sha256.join(","));
  }
  if !tls.verify_peer_cert_by_name.is_empty() {
    value["verifyPeerCertByName"] = json!(tls.verify_peer_cert_by_name.join(","));
  }
  value
}

pub fn build_client_config_json(
  config: &XrayConfig,
  runtime: &XrayClientRuntime,
) -> XrayResult<String> {
  serde_json::to_string_pretty(&build_client_config(config, runtime)?)
    .map_err(|_| XrayError::Serialization)
}

fn validate_socks_credential(field: &'static str, value: &str) -> XrayResult<()> {
  if value.is_empty() || value.len() > 255 {
    return Err(XrayError::InvalidField {
      field,
      reason: "must contain between 1 and 255 URL-safe ASCII characters",
    });
  }
  if !value
    .bytes()
    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
  {
    return Err(XrayError::InvalidField {
      field,
      reason: "must contain only URL-safe ASCII characters",
    });
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

  use super::super::parse_share_link;
  use super::*;

  const ID: &str = "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4";

  fn valid_config() -> XrayConfig {
    let key = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    parse_share_link(&format!(
      "vless://{ID}@vpn.example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality\
&sni=www.example.com&fp=chrome&pbk={key}&sid=0123456789abcdef&spx=%2F&type=tcp&headerType=none"
    ))
    .unwrap()
    .config
  }

  fn outbound_for(link: &str) -> Value {
    let config = parse_share_link(link).unwrap().config;
    build_client_config(&config, &runtime()).unwrap()["outbounds"][0].clone()
  }

  fn runtime() -> XrayClientRuntime {
    XrayClientRuntime {
      listen_port: 41_321,
      username: "local_user-1".to_string(),
      password: "local_password-1".to_string(),
    }
  }

  #[test]
  fn generates_minimal_authenticated_tcp_reality_config() {
    let value = build_client_config(&valid_config(), &runtime()).unwrap();

    assert_eq!(value["log"]["loglevel"], "warning");
    assert_eq!(value["inbounds"].as_array().unwrap().len(), 1);
    assert_eq!(value["inbounds"][0]["listen"], "127.0.0.1");
    assert_eq!(value["inbounds"][0]["port"], 41_321);
    assert_eq!(value["inbounds"][0]["protocol"], "socks");
    assert_eq!(value["inbounds"][0]["settings"]["auth"], "password");
    assert_eq!(
      value["inbounds"][0]["settings"]["accounts"][0]["user"],
      "local_user-1"
    );
    assert_eq!(
      value["inbounds"][0]["settings"]["accounts"][0]["pass"],
      "local_password-1"
    );
    assert_eq!(value["inbounds"][0]["settings"]["udp"], true);
    assert_eq!(value["inbounds"][0]["settings"]["ip"], "127.0.0.1");

    assert_eq!(value["outbounds"].as_array().unwrap().len(), 1);
    assert_eq!(value["outbounds"][0]["protocol"], "vless");
    assert_eq!(
      value["outbounds"][0]["settings"]["vnext"][0]["address"],
      "vpn.example.com"
    );
    assert_eq!(
      value["outbounds"][0]["settings"]["vnext"][0]["users"][0]["encryption"],
      "none"
    );
    assert_eq!(
      value["outbounds"][0]["settings"]["vnext"][0]["users"][0]["flow"],
      "xtls-rprx-vision"
    );
    assert_eq!(value["outbounds"][0]["streamSettings"]["network"], "tcp");
    assert_eq!(
      value["outbounds"][0]["streamSettings"]["security"],
      "reality"
    );
    assert_eq!(
      value["outbounds"][0]["streamSettings"]["realitySettings"]["fingerprint"],
      "chrome"
    );
    assert_eq!(
      value["outbounds"][0]["streamSettings"]["realitySettings"]["serverName"],
      "www.example.com"
    );
    assert_eq!(
      value["outbounds"][0]["streamSettings"]["realitySettings"]["shortId"],
      "0123456789abcdef"
    );
  }

  #[test]
  fn plain_vless_over_websocket_and_tls_has_no_flow() {
    let outbound = outbound_for(&format!(
      "vless://{ID}@203.0.113.7:443?encryption=none&security=tls&type=ws\
&host=cdn.example.com&path=%2Fray%3Fed%3D2048&alpn=http%2F1.1&fp=firefox"
    ));
    let user = &outbound["settings"]["vnext"][0]["users"][0];
    assert_eq!(user["encryption"], "none");
    assert!(user.get("flow").is_none());
    let stream = &outbound["streamSettings"];
    assert_eq!(stream["network"], "ws");
    assert_eq!(stream["wsSettings"]["path"], "/ray?ed=2048");
    assert_eq!(stream["wsSettings"]["host"], "cdn.example.com");
    assert_eq!(stream["security"], "tls");
    // The Host header doubles as the server name when the link names none.
    assert_eq!(stream["tlsSettings"]["serverName"], "cdn.example.com");
    assert_eq!(stream["tlsSettings"]["alpn"], json!(["http/1.1"]));
    assert_eq!(stream["tlsSettings"]["fingerprint"], "firefox");
    assert!(stream["tlsSettings"].get("allowInsecure").is_none());
  }

  #[test]
  fn plain_vless_without_security_sends_no_tls_settings() {
    let outbound = outbound_for(&format!("vless://{ID}@a.example.com:8080?type=tcp"));
    assert_eq!(outbound["streamSettings"]["security"], "none");
    assert!(outbound["streamSettings"].get("tlsSettings").is_none());
    assert!(outbound["streamSettings"].get("realitySettings").is_none());
  }

  #[test]
  fn vmess_carries_its_cipher_and_grpc_service() {
    let outbound = outbound_for(&format!(
      "vmess://{ID}@a.example.com:443?encryption=chacha20-poly1305&security=tls\
&type=grpc&serviceName=gun-svc&mode=multi&authority=grpc.example.com&sni=a.example.com"
    ));
    assert_eq!(outbound["protocol"], "vmess");
    assert_eq!(
      outbound["settings"]["vnext"][0]["users"][0]["security"],
      "chacha20-poly1305"
    );
    let grpc = &outbound["streamSettings"]["grpcSettings"];
    assert_eq!(grpc["serviceName"], "gun-svc");
    assert_eq!(grpc["multiMode"], true);
    assert_eq!(grpc["authority"], "grpc.example.com");
  }

  #[test]
  fn trojan_pins_a_certificate_instead_of_skipping_the_check() {
    let pin = "AB:".repeat(31) + "AB";
    let outbound = outbound_for(&format!(
      "trojan://p%40ss@a.example.com:443?sni=a.example.com&pcs={pin}&allowInsecure=1&type=tcp"
    ));
    assert_eq!(outbound["protocol"], "trojan");
    assert_eq!(outbound["settings"]["servers"][0]["password"], "p@ss");
    let tls = &outbound["streamSettings"]["tlsSettings"];
    assert_eq!(tls["pinnedPeerCertSha256"], "ab".repeat(32));
    assert!(tls.get("allowInsecure").is_none());
  }

  #[test]
  fn raw_http_header_disguise_is_written_as_a_request() {
    let outbound = outbound_for(&format!(
      "vmess://{ID}@a.example.com:80?type=tcp&headerType=http&host=a.com,b.com&path=%2Findex"
    ));
    let header = &outbound["streamSettings"]["tcpSettings"]["header"];
    assert_eq!(header["type"], "http");
    assert_eq!(header["request"]["path"], json!(["/index"]));
    assert_eq!(
      header["request"]["headers"]["Host"],
      json!(["a.com", "b.com"])
    );
  }

  #[test]
  fn xhttp_and_httpupgrade_keep_path_host_and_mode() {
    let xhttp = outbound_for(&format!(
      "vless://{ID}@a.example.com:443?security=tls&type=xhttp&path=%2Fx&host=h.example.com&mode=stream-one"
    ));
    assert_eq!(xhttp["streamSettings"]["network"], "xhttp");
    assert_eq!(
      xhttp["streamSettings"]["xhttpSettings"]["mode"],
      "stream-one"
    );
    assert_eq!(xhttp["streamSettings"]["xhttpSettings"]["path"], "/x");
    let upgrade = outbound_for(&format!(
      "vless://{ID}@a.example.com:80?type=httpupgrade&path=%2Fu&host=h.example.com"
    ));
    assert_eq!(
      upgrade["streamSettings"]["httpupgradeSettings"],
      json!({ "path": "/u", "host": "h.example.com" })
    );
  }

  #[test]
  fn hysteria2_speaks_h3_over_quic_with_salamander() {
    let pin = "cd".repeat(32);
    let outbound = outbound_for(&format!(
      "hysteria2://letmein@hy.example.com:8443/?obfs=salamander&obfs-password=salt&pinSHA256={pin}"
    ));
    assert_eq!(outbound["protocol"], "hysteria");
    assert_eq!(outbound["settings"]["version"], 2);
    assert_eq!(outbound["settings"]["address"], "hy.example.com");
    assert_eq!(outbound["settings"]["port"], 8443);
    let stream = &outbound["streamSettings"];
    assert_eq!(stream["network"], "hysteria");
    assert_eq!(stream["hysteriaSettings"]["version"], 2);
    assert_eq!(stream["hysteriaSettings"]["auth"], "letmein");
    assert_eq!(stream["security"], "tls");
    assert_eq!(stream["tlsSettings"]["alpn"], json!(["h3"]));
    assert_eq!(stream["tlsSettings"]["serverName"], "hy.example.com");
    assert_eq!(
      stream["finalmask"]["udp"][0],
      json!({ "type": "salamander", "settings": { "password": "salt" } })
    );
    assert_eq!(stream["tlsSettings"]["pinnedPeerCertSha256"], pin);
    assert!(stream.get("sockopt").is_none());
  }

  #[test]
  fn does_not_add_bypass_or_observability_surfaces() {
    let value = build_client_config(&valid_config(), &runtime()).unwrap();
    for absent in ["api", "dns", "policy", "routing", "stats"] {
      assert!(value.get(absent).is_none(), "{absent}");
    }
    assert!(value["outbounds"][0].get("mux").is_none());
    assert!(!value.to_string().contains("freedom"));
    assert!(!value.to_string().contains("blackhole"));
  }

  #[test]
  fn json_output_round_trips_without_shape_changes() {
    let value = build_client_config(&valid_config(), &runtime()).unwrap();
    let json = build_client_config_json(&valid_config(), &runtime()).unwrap();
    let reparsed: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(reparsed, value);
  }

  #[test]
  fn runtime_requires_nonzero_port_and_url_safe_credentials() {
    let cases = [
      XrayClientRuntime {
        listen_port: 0,
        ..runtime()
      },
      XrayClientRuntime {
        username: String::new(),
        ..runtime()
      },
      XrayClientRuntime {
        password: "contains:@".to_string(),
        ..runtime()
      },
      XrayClientRuntime {
        username: "a".repeat(256),
        ..runtime()
      },
    ];
    for runtime in cases {
      assert!(runtime.validate().is_err());
    }
  }

  #[test]
  fn invalid_model_is_rejected_before_generation() {
    let mut config = valid_config();
    let secret = "secret-invalid-key";
    if let XrayOutbound::Vless {
      stream: StreamSettings {
        security: Security::Reality(reality),
        ..
      },
      ..
    } = &mut config.outbound
    {
      reality.public_key = secret.to_string();
    }
    let error = build_client_config(&config, &runtime()).unwrap_err();
    assert!(matches!(
      error,
      XrayError::InvalidField { field: "pbk", .. }
    ));
    assert!(!error.to_string().contains(secret));
  }
}

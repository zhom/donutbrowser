use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{VlessRealityConfig, XrayError, XrayResult};

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

pub fn build_client_config(
  config: &VlessRealityConfig,
  runtime: &XrayClientRuntime,
) -> XrayResult<Value> {
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
    "outbounds": [{
      "tag": "proxy",
      "protocol": "vless",
      "settings": {
        "vnext": [{
          "address": config.address,
          "port": config.port,
          "users": [{
            "id": config.id,
            "encryption": "none",
            "flow": config.flow.as_str()
          }]
        }]
      },
      "streamSettings": {
        "network": "tcp",
        "security": "reality",
        "realitySettings": {
          "show": false,
          "fingerprint": config.reality.fingerprint.as_str(),
          "serverName": config.reality.server_name,
          "publicKey": config.reality.public_key,
          "shortId": config.reality.short_id,
          "spiderX": config.reality.spider_x
        },
        "sockopt": {
          "tcpKeepAliveIdle": 30,
          "tcpKeepAliveInterval": 15
        }
      }
    }]
  }))
}

pub fn build_client_config_json(
  config: &VlessRealityConfig,
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

  use super::super::{RealityFingerprint, RealitySettings, VlessFlow};
  use super::*;

  fn valid_config() -> VlessRealityConfig {
    VlessRealityConfig {
      address: "vpn.example.com".to_string(),
      port: 443,
      id: "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4".to_string(),
      flow: VlessFlow::Vision,
      reality: RealitySettings {
        server_name: "www.example.com".to_string(),
        public_key: URL_SAFE_NO_PAD.encode([7_u8; 32]),
        short_id: "0123456789abcdef".to_string(),
        fingerprint: RealityFingerprint::Chrome,
        spider_x: "/".to_string(),
      },
    }
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
    config.reality.public_key = "secret-invalid-key".to_string();
    let error = build_client_config(&config, &runtime()).unwrap_err();
    assert!(matches!(
      error,
      XrayError::InvalidField { field: "pbk", .. }
    ));
    assert!(!error.to_string().contains(&config.reality.public_key));
  }
}

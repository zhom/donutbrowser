use std::net::IpAddr;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use url::Host;
use uuid::Uuid;

use super::{XrayError, XrayResult};

const MAX_SPIDER_X_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VlessFlow {
  #[default]
  #[serde(rename = "xtls-rprx-vision")]
  Vision,
}

impl VlessFlow {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Vision => "xtls-rprx-vision",
    }
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RealityFingerprint {
  #[default]
  Chrome,
  Firefox,
  Safari,
  Edge,
  Ios,
  Android,
}

impl RealityFingerprint {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Chrome => "chrome",
      Self::Firefox => "firefox",
      Self::Safari => "safari",
      Self::Edge => "edge",
      Self::Ios => "ios",
      Self::Android => "android",
    }
  }

  pub(crate) fn parse(value: &str) -> XrayResult<Self> {
    match value {
      "chrome" => Ok(Self::Chrome),
      "firefox" => Ok(Self::Firefox),
      "safari" => Ok(Self::Safari),
      "edge" => Ok(Self::Edge),
      "ios" => Ok(Self::Ios),
      "android" => Ok(Self::Android),
      _ => Err(XrayError::UnsupportedValue {
        field: "fp",
        expected: "chrome, firefox, safari, edge, ios, or android",
      }),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealitySettings {
  pub server_name: String,
  pub public_key: String,
  #[serde(default)]
  pub short_id: String,
  #[serde(default)]
  pub fingerprint: RealityFingerprint,
  #[serde(default = "default_spider_x")]
  pub spider_x: String,
}

impl RealitySettings {
  pub fn validate(&self) -> XrayResult<()> {
    validate_server_name(&self.server_name)?;
    validate_public_key(&self.public_key)?;
    validate_short_id(&self.short_id)?;
    validate_spider_x(&self.spider_x)
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VlessRealityConfig {
  pub address: String,
  pub port: u16,
  pub id: String,
  #[serde(default)]
  pub flow: VlessFlow,
  pub reality: RealitySettings,
}

impl VlessRealityConfig {
  pub fn validate(&self) -> XrayResult<()> {
    validate_endpoint_address(&self.address)?;
    if self.port == 0 {
      return Err(XrayError::InvalidField {
        field: "port",
        reason: "must be between 1 and 65535",
      });
    }
    Uuid::parse_str(&self.id).map_err(|_| XrayError::InvalidField {
      field: "id",
      reason: "must be a UUID",
    })?;
    self.reality.validate()
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParsedVlessUri {
  pub name: Option<String>,
  pub config: VlessRealityConfig,
}

impl ParsedVlessUri {
  pub fn validate(&self) -> XrayResult<()> {
    if let Some(name) = &self.name {
      validate_display_name(name)?;
    }
    self.config.validate()
  }
}

pub(crate) fn default_spider_x() -> String {
  "/".to_string()
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
  Host::parse(address).map_err(|_| XrayError::InvalidField {
    field: "address",
    reason: "must be a valid hostname or IP address",
  })?;
  Ok(())
}

fn validate_server_name(server_name: &str) -> XrayResult<()> {
  if server_name.is_empty() || server_name.trim() != server_name {
    return Err(XrayError::InvalidField {
      field: "sni",
      reason: "must be a valid DNS name",
    });
  }
  match Host::parse(server_name) {
    Ok(Host::Domain(_)) => Ok(()),
    _ => Err(XrayError::InvalidField {
      field: "sni",
      reason: "must be a valid DNS name",
    }),
  }
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

  fn valid_config() -> VlessRealityConfig {
    VlessRealityConfig {
      address: "vpn.example.com".to_string(),
      port: 443,
      id: "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4".to_string(),
      flow: VlessFlow::Vision,
      reality: RealitySettings {
        server_name: "www.example.com".to_string(),
        public_key: public_key(),
        short_id: "0123456789abcdef".to_string(),
        fingerprint: RealityFingerprint::Chrome,
        spider_x: "/".to_string(),
      },
    }
  }

  #[test]
  fn valid_model_passes_validation() {
    assert_eq!(valid_config().validate(), Ok(()));
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
  fn id_must_be_a_uuid() {
    let mut config = valid_config();
    config.id = "not-a-uuid".to_string();
    assert_eq!(
      config.validate(),
      Err(XrayError::InvalidField {
        field: "id",
        reason: "must be a UUID",
      })
    );
  }

  #[test]
  fn server_name_must_be_dns_name() {
    for server_name in ["", "203.0.113.5", "bad server"] {
      let mut config = valid_config();
      config.reality.server_name = server_name.to_string();
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "sni", .. })
      ));
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
      let mut config = valid_config();
      config.reality.public_key = public_key.to_string();
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
      let mut config = valid_config();
      config.reality.short_id = short_id.to_string();
      assert_eq!(config.validate(), Ok(()), "{short_id}");
    }
  }

  #[test]
  fn short_id_rejects_odd_non_hex_and_overlong_values() {
    for short_id in ["a", "xz", "0123456789abcdef00"] {
      let mut config = valid_config();
      config.reality.short_id = short_id.to_string();
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "sid", .. })
      ));
    }
  }

  #[test]
  fn spider_x_must_be_safe_relative_path() {
    for spider_x in ["relative", "/line\nbreak"] {
      let mut config = valid_config();
      config.reality.spider_x = spider_x.to_string();
      assert!(matches!(
        config.validate(),
        Err(XrayError::InvalidField { field: "spx", .. })
      ));
    }
  }

  #[test]
  fn serde_defaults_preserve_the_supported_profile() {
    let value = serde_json::json!({
      "address": "vpn.example.com",
      "port": 443,
      "id": "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4",
      "reality": {
        "server_name": "www.example.com",
        "public_key": public_key()
      }
    });
    let config: VlessRealityConfig = serde_json::from_value(value).unwrap();
    assert_eq!(config.flow, VlessFlow::Vision);
    assert_eq!(config.reality.fingerprint, RealityFingerprint::Chrome);
    assert_eq!(config.reality.short_id, "");
    assert_eq!(config.reality.spider_x, "/");
  }

  #[test]
  fn serde_rejects_unknown_configuration_fields() {
    let value = serde_json::json!({
      "address": "vpn.example.com",
      "port": 443,
      "id": "6d6e21a1-4829-4d2b-bc7f-1b25707b61e4",
      "transport": "websocket",
      "reality": {
        "server_name": "www.example.com",
        "public_key": public_key()
      }
    });
    assert!(serde_json::from_value::<VlessRealityConfig>(value).is_err());
  }
}

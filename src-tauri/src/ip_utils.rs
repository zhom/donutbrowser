//! IP address utilities shared across the application.
//!
//! Provides IP validation and public IP fetching functionality.

use std::net::IpAddr;
use std::str::FromStr;

/// IP utility error type.
#[derive(Debug, thiserror::Error)]
pub enum IpError {
  #[error("Network error: {0}")]
  Network(String),

  #[error("Invalid IP address: {0}")]
  InvalidIP(String),
}

/// Validate an IP address (IPv4 or IPv6).
pub fn validate_ip(ip: &str) -> bool {
  IpAddr::from_str(ip).is_ok()
}

/// Check if an IP is IPv4.
pub fn is_ipv4(ip: &str) -> bool {
  if let Ok(addr) = IpAddr::from_str(ip) {
    addr.is_ipv4()
  } else {
    false
  }
}

/// Check if an IP is IPv6.
pub fn is_ipv6(ip: &str) -> bool {
  if let Ok(addr) = IpAddr::from_str(ip) {
    addr.is_ipv6()
  } else {
    false
  }
}

/// Fetch public IP address, optionally through a proxy.
pub async fn fetch_public_ip(proxy: Option<&str>) -> Result<String, IpError> {
  let urls = [
    "https://api.ipify.org",
    "https://checkip.amazonaws.com",
    "https://ipinfo.io/ip",
    "https://icanhazip.com",
    "https://ifconfig.co/ip",
    "https://ipecho.net/plain",
  ];

  let client_builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5));

  let client = if let Some(proxy_url) = proxy {
    let proxy = reqwest::Proxy::all(proxy_url)
      .map_err(|e| IpError::Network(format!("Invalid proxy: {}", e)))?;
    client_builder
      .proxy(proxy)
      .build()
      .map_err(|e| IpError::Network(e.to_string()))?
  } else {
    client_builder
      .build()
      .map_err(|e| IpError::Network(e.to_string()))?
  };

  let mut last_error = None;

  for url in &urls {
    match client.get(*url).send().await {
      Ok(response) if response.status().is_success() => match response.text().await {
        Ok(text) => {
          let ip = text.trim().to_string();
          if validate_ip(&ip) {
            return Ok(ip);
          }
        }
        Err(e) => {
          last_error = Some(format!("Failed to read response from {}: {}", url, e));
        }
      },
      Ok(response) => {
        last_error = Some(format!("HTTP {} from {}", response.status(), url));
      }
      Err(e) => {
        last_error = Some(format!("Request to {} failed: {}", url, e));
      }
    }
  }

  Err(IpError::Network(last_error.unwrap_or_else(|| {
    "Failed to fetch public IP from any endpoint".to_string()
  })))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_validate_ip() {
    assert!(validate_ip("8.8.8.8"));
    assert!(validate_ip("192.168.1.1"));
    assert!(validate_ip("2001:4860:4860::8888"));
    assert!(!validate_ip("invalid"));
    assert!(!validate_ip("256.256.256.256"));
  }

  #[test]
  fn test_is_ipv4() {
    assert!(is_ipv4("8.8.8.8"));
    assert!(!is_ipv4("2001:4860:4860::8888"));
    assert!(!is_ipv4("invalid"));
  }

  #[test]
  fn test_is_ipv6() {
    assert!(is_ipv6("2001:4860:4860::8888"));
    assert!(!is_ipv6("8.8.8.8"));
    assert!(!is_ipv6("invalid"));
  }
}

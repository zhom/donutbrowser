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
}

/// Validate an IP address (IPv4 or IPv6).
pub fn validate_ip(ip: &str) -> bool {
  IpAddr::from_str(ip).is_ok()
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

  // 10s rather than 5s: residential proxies that allocate an exit on first
  // connect routinely need more than 5s for the initial request.
  let client_builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10));

  let client = if let Some(proxy_url) = proxy {
    let proxy = reqwest::Proxy::all(proxy_url)
      .map_err(|e| IpError::Network(format!("Invalid proxy: {}", e)))?;
    client_builder
      .no_proxy()
      .proxy(proxy)
      .build()
      .map_err(|e| IpError::Network(e.to_string()))?
  } else {
    client_builder
      .build()
      .map_err(|e| IpError::Network(e.to_string()))?
  };

  let mut errors = Vec::new();

  // Overall deadline across all endpoints. Without it, a proxy that accepts
  // connections but stalls holds callers for the full 6 x 10s; slow-but-live
  // proxies still get the whole 10s on the endpoints that fit the budget.
  let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

  for url in &urls {
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    if remaining.is_zero() {
      errors.push(format!("{}: skipped (30s overall deadline reached)", url));
      continue;
    }

    let attempt = async {
      match client.get(*url).send().await {
        Ok(response) if response.status().is_success() => match response.text().await {
          Ok(text) => {
            let ip = text.trim().to_string();
            if validate_ip(&ip) {
              Ok(ip)
            } else {
              Err(format!("{}: response is not an IP address", url))
            }
          }
          Err(e) => Err(format!("{}: {}", url, e)),
        },
        Ok(response) => Err(format!("{}: HTTP {}", url, response.status())),
        Err(e) => Err(format!("{}: {}", url, e)),
      }
    };

    match tokio::time::timeout(remaining, attempt).await {
      Ok(Ok(ip)) => return Ok(ip),
      Ok(Err(e)) => errors.push(e),
      Err(_) => errors.push(format!("{}: timed out (30s overall deadline reached)", url)),
    }
  }

  if errors.is_empty() {
    Err(IpError::Network(
      "Failed to fetch public IP from any endpoint".to_string(),
    ))
  } else {
    Err(IpError::Network(format!(
      "All {} endpoints failed: {}",
      errors.len(),
      errors.join("; ")
    )))
  }
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
}

//! The proxy the operating system hands Donut's own HTTP clients.
//!
//! reqwest is built with `system-proxy`, so every client that does not call
//! `no_proxy()` sends its requests through whatever proxy the environment or,
//! on Windows, the WinINet settings name. A proxy left behind by an uninstalled
//! VPN or debugging tool therefore breaks Donut's own downloads, and the user
//! sees a transfer that never starts (issue #625). This module reads the same
//! settings so a failed request can say which proxy it went through.

/// The proxy used for `https` requests, as `host:port` or a URL. `None` when
/// requests go out directly.
pub fn https_proxy() -> Option<String> {
  env_https_proxy(|name| std::env::var(name).ok()).or_else(platform_https_proxy)
}

/// The error a caller returns when a request that went through `proxy` could
/// not connect. The frontend turns it into a sentence that names the proxy.
pub fn unreachable_error(proxy: &str) -> String {
  serde_json::json!({
    "code": "SYSTEM_PROXY_UNREACHABLE",
    "params": { "proxy": proxy }
  })
  .to_string()
}

/// The coded error for a failed request, when the failure is the connection
/// itself and a system proxy carried it. `None` for any other failure, which
/// the caller reports as before.
pub fn explain(error: &reqwest::Error) -> Option<String> {
  if !(error.is_connect() || error.is_timeout()) {
    return None;
  }
  https_proxy().map(|proxy| unreachable_error(&proxy))
}

/// Whether `message` is the error `unreachable_error` builds. Retrying that one
/// only repeats the wait, because the setting stays wrong until the user
/// changes it.
pub fn is_unreachable_error(message: &str) -> bool {
  message.contains("\"SYSTEM_PROXY_UNREACHABLE\"")
}

/// The variables reqwest reads, in the order it reads them.
fn env_https_proxy(read: impl Fn(&str) -> Option<String>) -> Option<String> {
  ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    .into_iter()
    .filter_map(read)
    .map(|value| value.trim().to_string())
    .find(|value| !value.is_empty())
}

#[cfg(target_os = "windows")]
fn platform_https_proxy() -> Option<String> {
  use winreg::enums::HKEY_CURRENT_USER;
  use winreg::RegKey;

  let settings = RegKey::predef(HKEY_CURRENT_USER)
    .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
    .ok()?;
  let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
  if enabled == 0 {
    return None;
  }
  let server: String = settings.get_value("ProxyServer").ok()?;
  parse_proxy_server(&server)
}

#[cfg(not(target_os = "windows"))]
fn platform_https_proxy() -> Option<String> {
  None
}

/// Read the WinINet `ProxyServer` value. It is either one `host:port` for
/// every scheme, or a list such as `http=a:80;https=b:443;socks=c:1080`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_proxy_server(value: &str) -> Option<String> {
  let value = value.trim();
  if value.is_empty() {
    return None;
  }
  if !value.contains('=') {
    return Some(value.to_string());
  }

  let entry = |scheme: &str| {
    value.split(';').find_map(|part| {
      let (name, address) = part.split_once('=')?;
      let address = address.trim();
      (name.trim().eq_ignore_ascii_case(scheme) && !address.is_empty()).then(|| address.to_string())
    })
  };
  entry("https").or_else(|| entry("http"))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn https_proxy_variables_win_over_all_proxy() {
    let proxy = env_https_proxy(|name| match name {
      "https_proxy" => Some("http://127.0.0.1:8080".to_string()),
      "ALL_PROXY" => Some("socks5://127.0.0.1:1080".to_string()),
      _ => None,
    });
    assert_eq!(proxy.as_deref(), Some("http://127.0.0.1:8080"));
  }

  #[test]
  fn blank_variables_mean_no_proxy() {
    assert_eq!(env_https_proxy(|_| Some("  ".to_string())), None);
    assert_eq!(env_https_proxy(|_| None), None);
  }

  #[test]
  fn a_single_proxy_server_serves_every_scheme() {
    assert_eq!(
      parse_proxy_server("127.0.0.1:8888").as_deref(),
      Some("127.0.0.1:8888")
    );
  }

  #[test]
  fn a_per_scheme_list_uses_the_https_entry() {
    assert_eq!(
      parse_proxy_server("http=10.0.0.1:80;https=10.0.0.2:443;socks=10.0.0.3:1080").as_deref(),
      Some("10.0.0.2:443")
    );
    assert_eq!(
      parse_proxy_server("HTTP=10.0.0.1:80").as_deref(),
      Some("10.0.0.1:80")
    );
    assert_eq!(parse_proxy_server("socks=10.0.0.3:1080"), None);
    assert_eq!(parse_proxy_server(""), None);
  }

  #[test]
  fn the_unreachable_error_is_recognised() {
    let error = unreachable_error("127.0.0.1:8888");
    assert!(is_unreachable_error(&error));
    assert!(!is_unreachable_error("Download failed after 6 attempts"));
    let parsed: serde_json::Value = serde_json::from_str(&error).unwrap();
    assert_eq!(parsed["params"]["proxy"], "127.0.0.1:8888");
  }
}

//! A proxy provider's "change IP" link: opening it gives the proxy a new exit
//! address. Mobile and some residential providers hand one out per port.
//!
//! The link is called from this machine, not through the proxy: providers
//! accept it from anywhere, and a rotating proxy may be unreachable through
//! itself while it rotates. Every profile behind the proxy moves with it.

use crate::proxy_manager::PROXY_MANAGER;
use serde::Serialize;
use std::time::{Duration, Instant};

const ROTATION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_URL_CHARS: usize = 2048;
const BULK_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct RotationResult {
  pub proxy_id: String,
  /// The provider's HTTP status, when it answered.
  pub status: Option<u16>,
  pub elapsed_ms: u64,
  /// A coded backend error when the IP change was not confirmed.
  pub error: Option<String>,
}

/// Blank clears; anything else must be an http(s) URL.
pub fn checked_rotation_url(value: Option<String>) -> Result<Option<String>, String> {
  let Some(value) = value
    .map(|v| v.trim().to_string())
    .filter(|v| !v.is_empty())
  else {
    return Ok(None);
  };
  let valid = value.chars().count() <= MAX_URL_CHARS
    && !value.chars().any(char::is_whitespace)
    && url::Url::parse(&value)
      .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
  if valid {
    Ok(Some(value))
  } else {
    Err(crate::backend_error("PROXY_ROTATION_URL_INVALID"))
  }
}

fn client() -> Result<reqwest::Client, String> {
  reqwest::Client::builder()
    .timeout(ROTATION_TIMEOUT)
    .build()
    .map_err(|e| crate::backend_error_with_detail("PROXY_ROTATION_FAILED", e))
}

async fn call(client: &reqwest::Client, proxy_id: &str) -> RotationResult {
  let refused = |code: &str| RotationResult {
    proxy_id: proxy_id.to_string(),
    status: None,
    elapsed_ms: 0,
    error: Some(crate::backend_error(code)),
  };
  match PROXY_MANAGER
    .get_stored_proxies()
    .into_iter()
    .find(|proxy| proxy.id == proxy_id)
  {
    None => refused("PROXY_NOT_FOUND"),
    Some(proxy) => match proxy.rotation_url {
      Some(url) => request(client, proxy_id, &url).await,
      None => refused("PROXY_ROTATION_URL_MISSING"),
    },
  }
}

async fn request(client: &reqwest::Client, proxy_id: &str, url: &str) -> RotationResult {
  let started = Instant::now();
  let mut result = RotationResult {
    proxy_id: proxy_id.to_string(),
    status: None,
    elapsed_ms: 0,
    error: None,
  };
  // The URL is never logged: providers put the account key in it.
  match client.get(url).send().await {
    Ok(response) => {
      let status = response.status();
      result.status = Some(status.as_u16());
      if !status.is_success() {
        result.error = Some(
          serde_json::json!({
            "code": "PROXY_ROTATION_FAILED",
            "params": { "status": status.as_u16().to_string() }
          })
          .to_string(),
        );
      }
    }
    Err(e) => {
      result.error = Some(crate::system_proxy::explain(&e).unwrap_or_else(|| {
        if e.is_timeout() {
          crate::backend_error("PROXY_ROTATION_TIMEOUT")
        } else {
          crate::backend_error("PROXY_ROTATION_UNREACHABLE")
        }
      }));
    }
  }
  result.elapsed_ms = started.elapsed().as_millis() as u64;
  match (&result.error, result.status) {
    (None, Some(status)) => log::info!(
      "Proxy IP change requested proxy={proxy_id} status={status} elapsed_ms={}",
      result.elapsed_ms
    ),
    (Some(_), status) => log::warn!(
      "Proxy IP change failed proxy={proxy_id} status={} elapsed_ms={}",
      status.map_or_else(|| "none".to_string(), |s| s.to_string()),
      result.elapsed_ms
    ),
    (None, None) => {}
  }
  result
}

pub async fn rotate(proxy_id: &str) -> Result<RotationResult, String> {
  let result = call(&client()?, proxy_id).await;
  match result.error {
    Some(error) => Err(error),
    None => Ok(result),
  }
}

/// Several proxies at once, a few in flight at a time. Each gets its own
/// result, so one failing provider does not hide the others.
pub async fn rotate_many(proxy_ids: Vec<String>) -> Result<Vec<RotationResult>, String> {
  use futures_util::StreamExt;
  let client = client()?;
  let mut seen = std::collections::HashSet::new();
  let unique: Vec<String> = proxy_ids
    .into_iter()
    .filter(|id| seen.insert(id.clone()))
    .collect();
  Ok(
    futures_util::stream::iter(unique)
      .map(|id| {
        let client = client.clone();
        async move { call(&client, &id).await }
      })
      .buffered(BULK_CONCURRENCY)
      .collect()
      .await,
  )
}

#[tauri::command]
pub fn set_proxy_rotation_url(
  proxy_id: String,
  rotation_url: Option<String>,
) -> Result<crate::proxy_manager::StoredProxy, String> {
  let rotation_url = checked_rotation_url(rotation_url)?;
  PROXY_MANAGER.set_rotation_url(&proxy_id, rotation_url)
}

#[tauri::command]
pub async fn rotate_proxy_ip(proxy_id: String) -> Result<RotationResult, String> {
  rotate(&proxy_id).await
}

#[tauri::command]
pub async fn rotate_proxy_ips(proxy_ids: Vec<String>) -> Result<Vec<RotationResult>, String> {
  rotate_many(proxy_ids).await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rotation_urls_must_be_http_links() {
    assert_eq!(checked_rotation_url(None), Ok(None));
    assert_eq!(checked_rotation_url(Some("  ".to_string())), Ok(None));
    assert_eq!(
      checked_rotation_url(Some(
        " https://api.provider.example/rotate?key=abc ".to_string()
      )),
      Ok(Some(
        "https://api.provider.example/rotate?key=abc".to_string()
      ))
    );
    for bad in [
      "ftp://provider.example/rotate",
      "provider.example/rotate",
      "https://",
      "https://provider.example/rotate key",
      "javascript:alert(1)",
    ] {
      assert!(
        checked_rotation_url(Some(bad.to_string()))
          .unwrap_err()
          .contains("PROXY_ROTATION_URL_INVALID"),
        "{bad}"
      );
    }
    assert!(checked_rotation_url(Some(format!("https://p.example/{}", "a".repeat(2100)))).is_err());
  }

  /// Answers every request with `status` and counts the requests.
  async fn provider(status: u16) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
      "http://{}/rotate?key=secret",
      listener.local_addr().unwrap()
    );
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
      while let Ok((mut stream, _)) = listener.accept().await {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).await;
        let _ = stream
          .write_all(
            format!("HTTP/1.1 {status} X\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
              .as_bytes(),
          )
          .await;
      }
    });
    (url, hits)
  }

  #[tokio::test]
  async fn a_provider_answer_decides_the_result() {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    let (url, hits) = provider(200).await;
    let ok = request(&client, "p1", &url).await;
    assert_eq!((ok.status, ok.error.as_deref()), (Some(200), None));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);

    let (url, _) = provider(429).await;
    let refused = request(&client, "p1", &url).await;
    assert_eq!(refused.status, Some(429));
    let error = refused.error.unwrap();
    assert!(error.contains("PROXY_ROTATION_FAILED") && error.contains("429"));

    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let unreachable = request(&client, "p1", &format!("http://127.0.0.1:{port}/rotate")).await;
    assert_eq!(unreachable.status, None);
    assert!(unreachable
      .error
      .unwrap()
      .contains("PROXY_ROTATION_UNREACHABLE"));
  }
}

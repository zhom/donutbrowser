//! JSON calls to Donut cloud as the signed-in account: the token is refreshed
//! and the call retried once when it went stale, and a failure becomes the
//! `{"code":…}` error the frontend translates, with the caller's codes for
//! the statuses that mean something to it.

use crate::cloud_errors::{self, BackendFailure, FailureCodes};
use serde::de::DeserializeOwned;
use std::sync::OnceLock;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

fn http() -> &'static reqwest::Client {
  static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
  CLIENT.get_or_init(|| {
    reqwest::Client::builder()
      .timeout(REQUEST_TIMEOUT)
      .connect_timeout(CONNECT_TIMEOUT)
      .build()
      .unwrap_or_else(|_| reqwest::Client::new())
  })
}

pub(crate) async fn call<T: DeserializeOwned>(
  method: reqwest::Method,
  url: String,
  body: Option<serde_json::Value>,
  codes: FailureCodes,
) -> Result<T, BackendFailure> {
  let bytes = send(method, url, body, codes).await?;
  decode_body::<T>(&bytes)
    .map_err(|e| cloud_errors::classify_message(&format!("decode response: {e}"), codes))
}

/// `call` for an answer that is text (a CSV file), not JSON.
pub(crate) async fn call_text(
  method: reqwest::Method,
  url: String,
  codes: FailureCodes,
) -> Result<String, BackendFailure> {
  let bytes = send(method, url, None, codes).await?;
  String::from_utf8(bytes)
    .map_err(|e| cloud_errors::classify_message(&format!("decode response: {e}"), codes))
}

async fn send(
  method: reqwest::Method,
  url: String,
  body: Option<serde_json::Value>,
  codes: FailureCodes,
) -> Result<Vec<u8>, BackendFailure> {
  crate::cloud_auth::CLOUD_AUTH
    .api_call_with_retry(|token| {
      let method = method.clone();
      let url = url.clone();
      let body = body.clone();
      async move {
        let mut builder = http().request(method, &url).bearer_auth(token);
        if let Some(payload) = body {
          builder = builder.json(&payload);
        }
        let response = builder
          .send()
          .await
          .map_err(|e| format!("reach backend: {e}"))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
          let text = response.text().await.unwrap_or_default();
          // api_call_with_retry and classify_message read the status from this prefix.
          return Err(format!("({status}) {text}"));
        }
        response
          .bytes()
          .await
          .map(|bytes| bytes.to_vec())
          .map_err(|e| format!("decode response: {e}"))
      }
    })
    .await
    .map_err(|e| cloud_errors::classify_message(&e, codes))
}

pub(crate) fn decode_body<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
  if bytes.iter().all(u8::is_ascii_whitespace) {
    serde_json::from_slice(b"null")
  } else {
    serde_json::from_slice(bytes)
  }
}

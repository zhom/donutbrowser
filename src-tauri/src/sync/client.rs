use super::types::*;
use reqwest::Client;

#[derive(Clone)]
pub struct SyncClient {
  client: Client,
  base_url: String,
  token: String,
}

impl SyncClient {
  pub fn new(base_url: String, token: String) -> Self {
    Self {
      client: Client::new(),
      base_url: base_url.trim_end_matches('/').to_string(),
      token,
    }
  }

  fn url(&self, path: &str) -> String {
    format!("{}/v1/objects/{}", self.base_url, path)
  }

  pub async fn stat(&self, key: &str) -> SyncResult<StatResponse> {
    let response = self
      .client
      .post(self.url("stat"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&StatRequest {
        key: key.to_string(),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn presign_upload(
    &self,
    key: &str,
    content_type: Option<&str>,
  ) -> SyncResult<PresignUploadResponse> {
    let response = self
      .client
      .post(self.url("presign-upload"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&PresignUploadRequest {
        key: key.to_string(),
        content_type: content_type.map(|s| s.to_string()),
        expires_in: Some(3600),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn presign_download(&self, key: &str) -> SyncResult<PresignDownloadResponse> {
    let response = self
      .client
      .post(self.url("presign-download"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&PresignDownloadRequest {
        key: key.to_string(),
        expires_in: Some(3600),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn delete(&self, key: &str, tombstone_key: Option<&str>) -> SyncResult<DeleteResponse> {
    let response = self
      .client
      .post(self.url("delete"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&DeleteRequest {
        key: key.to_string(),
        tombstone_key: tombstone_key.map(|s| s.to_string()),
        deleted_at: Some(chrono::Utc::now().to_rfc3339()),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn list(&self, prefix: &str) -> SyncResult<ListResponse> {
    let response = self
      .client
      .post(self.url("list"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&ListRequest {
        prefix: prefix.to_string(),
        max_keys: Some(1000),
        continuation_token: None,
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn upload_bytes(
    &self,
    presigned_url: &str,
    data: &[u8],
    content_type: Option<&str>,
  ) -> SyncResult<()> {
    let mut req = self
      .client
      .put(presigned_url)
      .header("Content-Length", data.len().to_string())
      .body(data.to_vec());

    if let Some(ct) = content_type {
      req = req.header("Content-Type", ct);
    }

    let response = req
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::NetworkError(format!(
        "Upload failed with status {status}: {body}"
      )));
    }

    Ok(())
  }

  pub async fn download_bytes(&self, presigned_url: &str) -> SyncResult<Vec<u8>> {
    let response = self
      .client
      .get(presigned_url)
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if !response.status().is_success() {
      return Err(SyncError::NetworkError(format!(
        "Download failed with status: {}",
        response.status()
      )));
    }

    response
      .bytes()
      .await
      .map(|b| b.to_vec())
      .map_err(|e| SyncError::NetworkError(e.to_string()))
  }

  pub async fn presign_upload_batch(
    &self,
    items: Vec<(String, Option<String>)>,
  ) -> SyncResult<PresignUploadBatchResponse> {
    let request = PresignUploadBatchRequest {
      items: items
        .into_iter()
        .map(|(key, content_type)| PresignUploadBatchItem { key, content_type })
        .collect(),
      expires_in: Some(3600),
    };

    let response = self
      .client
      .post(self.url("presign-upload-batch"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&request)
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn presign_download_batch(
    &self,
    keys: Vec<String>,
  ) -> SyncResult<PresignDownloadBatchResponse> {
    let request = PresignDownloadBatchRequest {
      keys,
      expires_in: Some(3600),
    };

    let response = self
      .client
      .post(self.url("presign-download-batch"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&request)
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn delete_prefix(
    &self,
    prefix: &str,
    tombstone_key: Option<&str>,
  ) -> SyncResult<DeletePrefixResponse> {
    let response = self
      .client
      .post(self.url("delete-prefix"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&DeletePrefixRequest {
        prefix: prefix.to_string(),
        tombstone_key: tombstone_key.map(|s| s.to_string()),
        deleted_at: Some(chrono::Utc::now().to_rfc3339()),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }
}

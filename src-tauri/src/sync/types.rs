use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatRequest {
  pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatResponse {
  pub exists: bool,
  #[serde(rename = "lastModified")]
  pub last_modified: Option<String>,
  pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadRequest {
  pub key: String,
  #[serde(rename = "contentType")]
  pub content_type: Option<String>,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadResponse {
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadRequest {
  pub key: String,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadResponse {
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRequest {
  pub key: String,
  #[serde(rename = "tombstoneKey")]
  pub tombstone_key: Option<String>,
  #[serde(rename = "deletedAt")]
  pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResponse {
  pub deleted: bool,
  #[serde(rename = "tombstoneCreated")]
  pub tombstone_created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {
  pub prefix: String,
  #[serde(rename = "maxKeys")]
  pub max_keys: Option<u32>,
  #[serde(rename = "continuationToken")]
  pub continuation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListObject {
  pub key: String,
  #[serde(rename = "lastModified")]
  pub last_modified: String,
  pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
  pub objects: Vec<ListObject>,
  #[serde(rename = "isTruncated")]
  pub is_truncated: bool,
  #[serde(rename = "nextContinuationToken")]
  pub next_continuation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tombstone {
  pub id: String,
  pub deleted_at: String,
}

// Batch presign types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadBatchItem {
  pub key: String,
  #[serde(rename = "contentType")]
  pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadBatchRequest {
  pub items: Vec<PresignUploadBatchItem>,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadBatchItemResponse {
  pub key: String,
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadBatchResponse {
  pub items: Vec<PresignUploadBatchItemResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadBatchRequest {
  pub keys: Vec<String>,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadBatchItemResponse {
  pub key: String,
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadBatchResponse {
  pub items: Vec<PresignDownloadBatchItemResponse>,
}

// Delete prefix types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePrefixRequest {
  pub prefix: String,
  #[serde(rename = "tombstoneKey")]
  pub tombstone_key: Option<String>,
  #[serde(rename = "deletedAt")]
  pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePrefixResponse {
  #[serde(rename = "deletedCount")]
  pub deleted_count: u32,
  #[serde(rename = "tombstoneCreated")]
  pub tombstone_created: bool,
}

#[derive(Debug)]
pub enum SyncError {
  NotConfigured,
  NetworkError(String),
  AuthError(String),
  IoError(String),
  SerializationError(String),
  ConflictError(String),
  InvalidData(String),
}

impl std::fmt::Display for SyncError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      SyncError::NotConfigured => write!(f, "Sync not configured"),
      SyncError::NetworkError(msg) => write!(f, "Network error: {msg}"),
      SyncError::AuthError(msg) => write!(f, "Authentication error: {msg}"),
      SyncError::IoError(msg) => write!(f, "IO error: {msg}"),
      SyncError::SerializationError(msg) => write!(f, "Serialization error: {msg}"),
      SyncError::ConflictError(msg) => write!(f, "Conflict error: {msg}"),
      SyncError::InvalidData(msg) => write!(f, "Invalid data: {msg}"),
    }
  }
}

impl std::error::Error for SyncError {}

pub type SyncResult<T> = Result<T, SyncError>;

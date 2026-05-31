use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
  /// User-defined S3 object metadata (`x-amz-meta-*`), lowercased keys without
  /// the prefix. `None` from older servers that don't return it. Used to read
  /// `updated-at` for sync conflict resolution without downloading the body.
  #[serde(default)]
  pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadRequest {
  pub key: String,
  #[serde(rename = "contentType")]
  pub content_type: Option<String>,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
  /// Object metadata to sign into the presigned PUT (stored as `x-amz-meta-*`).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadResponse {
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
  /// The metadata the server actually signed into the URL. The client must send
  /// exactly these as `x-amz-meta-*` headers on the PUT or S3 rejects it. `None`
  /// from older servers → client sends no metadata headers (body-GET fallback).
  #[serde(default)]
  pub metadata: Option<HashMap<String, String>>,
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
  Cancelled,
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
      SyncError::Cancelled => write!(f, "Sync cancelled by user"),
    }
  }
}

impl std::error::Error for SyncError {}

pub type SyncResult<T> = Result<T, SyncError>;

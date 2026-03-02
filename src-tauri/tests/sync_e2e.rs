use donutbrowser_lib::sync::types::*;
use reqwest::Client;
use serde_json::json;
use std::env;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const TEST_TOKEN: &str = "test-sync-token";

fn get_sync_server_url() -> String {
  env::var("SYNC_SERVER_URL").unwrap_or_else(|_| "http://localhost:12342".to_string())
}

/// Check if sync server is available and fail with a clear error message if not.
/// This ensures tests fail with helpful information rather than being silently ignored.
async fn ensure_sync_server_available() {
  let client = Client::new();
  let health_url = format!("{}/health", get_sync_server_url());

  match client
    .get(&health_url)
    .timeout(std::time::Duration::from_secs(5))
    .send()
    .await
  {
    Ok(response) => {
      if !response.status().is_success() {
        panic!(
          "Sync server is not healthy. Health check returned status: {}\n\
          Server URL: {}\n\
          Please ensure:\n\
          1. MinIO is running (docker compose up -d in donut-sync/)\n\
          2. donut-sync server is running (cd donut-sync && pnpm start:dev)\n\
          3. SYNC_SERVER_URL environment variable is set correctly",
          response.status(),
          get_sync_server_url()
        );
      }
    }
    Err(e) => {
      panic!(
        "Cannot connect to sync server: {}\n\
        Server URL: {}\n\
        Please ensure:\n\
        1. MinIO is running (docker compose up -d in donut-sync/)\n\
        2. donut-sync server is running (cd donut-sync && pnpm start:dev)\n\
        3. SYNC_SERVER_URL environment variable is set correctly\n\
        4. Network connectivity is available",
        e,
        get_sync_server_url()
      );
    }
  }
}

struct TestClient {
  client: Client,
  base_url: String,
  token: String,
}

impl TestClient {
  fn new() -> Self {
    Self {
      client: Client::new(),
      base_url: get_sync_server_url(),
      token: TEST_TOKEN.to_string(),
    }
  }

  fn url(&self, path: &str) -> String {
    format!("{}/v1/objects/{}", self.base_url, path)
  }

  async fn stat(&self, key: &str) -> Result<StatResponse, Box<dyn std::error::Error>> {
    let response = self
      .client
      .post(self.url("stat"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&json!({ "key": key }))
      .send()
      .await?;

    let status = response.status();
    if !status.is_success() {
      let body = response.text().await.unwrap_or_default();
      return Err(format!("stat failed with status {status}: {body}").into());
    }

    Ok(response.json().await?)
  }

  async fn presign_upload(
    &self,
    key: &str,
    content_type: &str,
  ) -> Result<PresignUploadResponse, Box<dyn std::error::Error>> {
    let response = self
      .client
      .post(self.url("presign-upload"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&json!({
        "key": key,
        "contentType": content_type
      }))
      .send()
      .await?;

    let status = response.status();
    if !status.is_success() {
      let body = response.text().await.unwrap_or_default();
      return Err(format!("presign-upload failed with status {status}: {body}").into());
    }

    Ok(response.json().await?)
  }

  async fn presign_download(
    &self,
    key: &str,
  ) -> Result<PresignDownloadResponse, Box<dyn std::error::Error>> {
    let response = self
      .client
      .post(self.url("presign-download"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&json!({ "key": key }))
      .send()
      .await?;

    let status = response.status();
    if !status.is_success() {
      let body = response.text().await.unwrap_or_default();
      return Err(format!("presign-download failed with status {status}: {body}").into());
    }

    Ok(response.json().await?)
  }

  async fn delete(
    &self,
    key: &str,
    tombstone_key: Option<&str>,
  ) -> Result<DeleteResponse, Box<dyn std::error::Error>> {
    let mut body = json!({ "key": key });
    if let Some(tk) = tombstone_key {
      body["tombstoneKey"] = json!(tk);
      body["deletedAt"] = json!(chrono::Utc::now().to_rfc3339());
    }

    let response = self
      .client
      .post(self.url("delete"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&body)
      .send()
      .await?;

    let status = response.status();
    if !status.is_success() {
      let body_text = response.text().await.unwrap_or_default();
      return Err(format!("delete failed with status {status}: {body_text}").into());
    }

    Ok(response.json().await?)
  }

  async fn upload_bytes(
    &self,
    url: &str,
    data: &[u8],
    content_type: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let response = self
      .client
      .put(url)
      .header("Content-Type", content_type)
      .body(data.to_vec())
      .send()
      .await?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(format!("Upload failed with status {status}: {body}").into());
    }
    Ok(())
  }

  async fn download_bytes(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let response = self.client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
      let body = response.text().await.unwrap_or_default();
      return Err(format!("Download failed with status {status}: {body}").into());
    }
    Ok(response.bytes().await?.to_vec())
  }
}

fn create_test_profile_bundle(temp_dir: &Path) -> Vec<u8> {
  use flate2::write::GzEncoder;
  use flate2::Compression;
  use tar::Builder;

  let metadata = json!({
    "id": "test-profile-id",
    "name": "Test Profile",
    "browser": "chromium",
    "version": "120.0.0",
    "release_type": "stable",
    "sync_enabled": true,
    "tags": ["test", "e2e"],
    "note": "Test profile for e2e"
  });

  let profile_dir = temp_dir.join("profile");
  fs::create_dir_all(&profile_dir).unwrap();
  fs::write(profile_dir.join("test_file.txt"), "test content").unwrap();

  let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
  {
    let mut tar = Builder::new(&mut encoder);

    let metadata_json = serde_json::to_string_pretty(&metadata).unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(metadata_json.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar
      .append_data(&mut header, "metadata.json", metadata_json.as_bytes())
      .unwrap();

    tar.append_dir_all("profile", &profile_dir).unwrap();
    tar.finish().unwrap();
  }

  encoder.finish().unwrap()
}

fn create_test_profile_bundle_with_bypass_rules(temp_dir: &Path, bypass_rules: &[&str]) -> Vec<u8> {
  use flate2::write::GzEncoder;
  use flate2::Compression;
  use tar::Builder;

  let metadata = json!({
    "id": "test-bypass-profile-id",
    "name": "Bypass Rules Profile",
    "browser": "camoufox",
    "version": "120.0.0",
    "release_type": "stable",
    "sync_enabled": true,
    "tags": [],
    "proxy_bypass_rules": bypass_rules
  });

  let profile_dir = temp_dir.join("bypass_profile");
  fs::create_dir_all(&profile_dir).unwrap();
  fs::write(profile_dir.join("test_file.txt"), "bypass test content").unwrap();

  let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
  {
    let mut tar = Builder::new(&mut encoder);

    let metadata_json = serde_json::to_string_pretty(&metadata).unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(metadata_json.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar
      .append_data(&mut header, "metadata.json", metadata_json.as_bytes())
      .unwrap();

    tar.append_dir_all("profile", &profile_dir).unwrap();
    tar.finish().unwrap();
  }

  encoder.finish().unwrap()
}

fn extract_bundle(data: &[u8], target_dir: &Path) -> serde_json::Value {
  use flate2::read::GzDecoder;
  use tar::Archive;

  let decoder = GzDecoder::new(data);
  let mut archive = Archive::new(decoder);
  archive.unpack(target_dir).unwrap();

  let metadata_path = target_dir.join("metadata.json");
  let metadata_content = fs::read_to_string(metadata_path).unwrap();
  serde_json::from_str(&metadata_content).unwrap()
}

#[tokio::test]
async fn test_sync_server_health() {
  ensure_sync_server_available().await;
  let client = Client::new();
  let url = format!("{}/health", get_sync_server_url());
  let response = client.get(&url).send().await.unwrap();
  assert!(response.status().is_success());
}

#[tokio::test]
async fn test_stat_nonexistent_key() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let result = client.stat("nonexistent-key").await.unwrap();
  assert!(!result.exists);
}

#[tokio::test]
async fn test_upload_download_cycle() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let test_key = format!("test/e2e-rust-{}.txt", uuid::Uuid::new_v4());
  let test_content = b"Hello from Rust e2e test!";

  let presign = client
    .presign_upload(&test_key, "text/plain")
    .await
    .unwrap();
  client
    .upload_bytes(&presign.url, test_content, "text/plain")
    .await
    .unwrap();

  let stat = client.stat(&test_key).await.unwrap();
  assert!(stat.exists);
  assert_eq!(stat.size, Some(test_content.len() as u64));

  let download_presign = client.presign_download(&test_key).await.unwrap();
  let downloaded = client.download_bytes(&download_presign.url).await.unwrap();
  assert_eq!(downloaded, test_content);

  let delete_result = client.delete(&test_key, None).await.unwrap();
  assert!(delete_result.deleted);

  let final_stat = client.stat(&test_key).await.unwrap();
  assert!(!final_stat.exists);
}

#[tokio::test]
async fn test_profile_bundle_upload_download() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let temp_dir = TempDir::new().unwrap();
  let profile_id = uuid::Uuid::new_v4().to_string();
  let test_key = format!("profiles/{}.tar.gz", profile_id);

  let bundle = create_test_profile_bundle(temp_dir.path());

  let presign = client
    .presign_upload(&test_key, "application/gzip")
    .await
    .unwrap();
  client
    .upload_bytes(&presign.url, &bundle, "application/gzip")
    .await
    .unwrap();

  let stat = client.stat(&test_key).await.unwrap();
  assert!(stat.exists);

  let download_presign = client.presign_download(&test_key).await.unwrap();
  let downloaded = client.download_bytes(&download_presign.url).await.unwrap();
  assert_eq!(downloaded.len(), bundle.len());

  let extract_dir = temp_dir.path().join("extracted");
  fs::create_dir_all(&extract_dir).unwrap();
  let metadata = extract_bundle(&downloaded, &extract_dir);

  assert_eq!(metadata["name"], "Test Profile");
  assert_eq!(metadata["browser"], "chromium");
  assert!(metadata["sync_enabled"].as_bool().unwrap());

  let test_file = extract_dir.join("profile").join("test_file.txt");
  assert!(test_file.exists());
  let content = fs::read_to_string(test_file).unwrap();
  assert_eq!(content, "test content");

  client.delete(&test_key, None).await.unwrap();
}

#[tokio::test]
async fn test_tombstone_creation() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let test_key = format!("test/tombstone-test-{}.txt", uuid::Uuid::new_v4());
  let tombstone_key = format!("tombstones/{}", test_key.replace("test/", ""));

  let presign = client
    .presign_upload(&test_key, "text/plain")
    .await
    .unwrap();
  client
    .upload_bytes(&presign.url, b"to be deleted", "text/plain")
    .await
    .unwrap();

  let delete_result = client
    .delete(&test_key, Some(&tombstone_key))
    .await
    .unwrap();
  assert!(delete_result.deleted);
  assert!(delete_result.tombstone_created);

  let tombstone_stat = client.stat(&tombstone_key).await.unwrap();
  assert!(tombstone_stat.exists);

  client.delete(&tombstone_key, None).await.unwrap();
}

#[tokio::test]
async fn test_device_a_to_device_b_sync() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let temp_dir_a = TempDir::new().unwrap();
  let temp_dir_b = TempDir::new().unwrap();
  let profile_id = uuid::Uuid::new_v4().to_string();
  let test_key = format!("profiles/{}.tar.gz", profile_id);

  let bundle_a = create_test_profile_bundle(temp_dir_a.path());
  let presign = client
    .presign_upload(&test_key, "application/gzip")
    .await
    .unwrap();
  client
    .upload_bytes(&presign.url, &bundle_a, "application/gzip")
    .await
    .unwrap();

  let download_presign = client.presign_download(&test_key).await.unwrap();
  let downloaded_b = client.download_bytes(&download_presign.url).await.unwrap();

  let extract_dir_b = temp_dir_b.path().join("extracted");
  fs::create_dir_all(&extract_dir_b).unwrap();
  let metadata_b = extract_bundle(&downloaded_b, &extract_dir_b);

  assert_eq!(metadata_b["name"], "Test Profile");
  assert_eq!(metadata_b["browser"], "chromium");

  let test_file_b = extract_dir_b.join("profile").join("test_file.txt");
  assert!(test_file_b.exists());
  let content_b = fs::read_to_string(test_file_b).unwrap();
  assert_eq!(content_b, "test content");

  client.delete(&test_key, None).await.unwrap();
}

#[tokio::test]
async fn test_proxy_sync() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let proxy_id = uuid::Uuid::new_v4().to_string();
  let test_key = format!("proxies/{}.json", proxy_id);

  let proxy_data = json!({
    "id": proxy_id,
    "name": "Test Proxy",
    "proxy_settings": {
      "proxy_type": "http",
      "host": "proxy.example.com",
      "port": 8080,
      "username": "user",
      "password": "pass"
    }
  });

  let proxy_json = serde_json::to_string(&proxy_data).unwrap();
  let presign = client
    .presign_upload(&test_key, "application/json")
    .await
    .unwrap();
  client
    .upload_bytes(&presign.url, proxy_json.as_bytes(), "application/json")
    .await
    .unwrap();

  let stat = client.stat(&test_key).await.unwrap();
  assert!(stat.exists);

  let download_presign = client.presign_download(&test_key).await.unwrap();
  let downloaded = client.download_bytes(&download_presign.url).await.unwrap();
  let downloaded_proxy: serde_json::Value = serde_json::from_slice(&downloaded).unwrap();

  assert_eq!(downloaded_proxy["name"], "Test Proxy");
  assert_eq!(
    downloaded_proxy["proxy_settings"]["host"],
    "proxy.example.com"
  );

  client.delete(&test_key, None).await.unwrap();
}

#[tokio::test]
async fn test_group_sync() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let group_id = uuid::Uuid::new_v4().to_string();
  let test_key = format!("groups/{}.json", group_id);

  let group_data = json!({
    "id": group_id,
    "name": "Test Group"
  });

  let group_json = serde_json::to_string(&group_data).unwrap();
  let presign = client
    .presign_upload(&test_key, "application/json")
    .await
    .unwrap();
  client
    .upload_bytes(&presign.url, group_json.as_bytes(), "application/json")
    .await
    .unwrap();

  let stat = client.stat(&test_key).await.unwrap();
  assert!(stat.exists);

  let download_presign = client.presign_download(&test_key).await.unwrap();
  let downloaded = client.download_bytes(&download_presign.url).await.unwrap();
  let downloaded_group: serde_json::Value = serde_json::from_slice(&downloaded).unwrap();

  assert_eq!(downloaded_group["name"], "Test Group");

  client.delete(&test_key, None).await.unwrap();
}

#[tokio::test]
async fn test_batch_presign_upload() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let profile_id = uuid::Uuid::new_v4().to_string();

  let items = vec![
    json!({
      "key": format!("profiles/{}/files/file1.txt", profile_id),
      "contentType": "text/plain"
    }),
    json!({
      "key": format!("profiles/{}/files/file2.txt", profile_id),
      "contentType": "text/plain"
    }),
    json!({
      "key": format!("profiles/{}/files/subdir/file3.txt", profile_id),
      "contentType": "text/plain"
    }),
  ];

  let response = client
    .client
    .post(client.url("presign-upload-batch"))
    .header("Authorization", format!("Bearer {}", client.token))
    .json(&json!({ "items": items }))
    .send()
    .await
    .unwrap();

  assert!(response.status().is_success());

  let result: serde_json::Value = response.json().await.unwrap();
  let items_result = result["items"].as_array().unwrap();

  assert_eq!(items_result.len(), 3);
  for item in items_result {
    assert!(item["url"].as_str().is_some());
    assert!(item["key"].as_str().is_some());
  }
}

#[tokio::test]
async fn test_batch_presign_download() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let profile_id = uuid::Uuid::new_v4().to_string();

  // First upload some files
  let file_keys = vec![
    format!("profiles/{}/files/file1.txt", profile_id),
    format!("profiles/{}/files/file2.txt", profile_id),
  ];

  for key in &file_keys {
    let presign = client.presign_upload(key, "text/plain").await.unwrap();
    client
      .upload_bytes(&presign.url, b"test content", "text/plain")
      .await
      .unwrap();
  }

  // Now test batch download presign
  let response = client
    .client
    .post(client.url("presign-download-batch"))
    .header("Authorization", format!("Bearer {}", client.token))
    .json(&json!({ "keys": file_keys }))
    .send()
    .await
    .unwrap();

  assert!(response.status().is_success());

  let result: serde_json::Value = response.json().await.unwrap();
  let items_result = result["items"].as_array().unwrap();

  assert_eq!(items_result.len(), 2);
  for item in items_result {
    assert!(item["url"].as_str().is_some());
    assert!(item["key"].as_str().is_some());
  }

  // Cleanup
  for key in &file_keys {
    client.delete(key, None).await.unwrap();
  }
}

#[tokio::test]
async fn test_delete_prefix() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let profile_id = uuid::Uuid::new_v4().to_string();
  let prefix = format!("profiles/{}/", profile_id);

  // Upload multiple files under the profile prefix
  let file_keys = vec![
    format!("profiles/{}/manifest.json", profile_id),
    format!("profiles/{}/metadata.json", profile_id),
    format!("profiles/{}/files/file1.txt", profile_id),
    format!("profiles/{}/files/subdir/file2.txt", profile_id),
  ];

  for key in &file_keys {
    let content_type = if key.ends_with(".json") {
      "application/json"
    } else {
      "text/plain"
    };
    let presign = client.presign_upload(key, content_type).await.unwrap();
    client
      .upload_bytes(&presign.url, b"test content", content_type)
      .await
      .unwrap();
  }

  // Verify all files exist
  for key in &file_keys {
    let stat = client.stat(key).await.unwrap();
    assert!(stat.exists, "File should exist before delete: {}", key);
  }

  // Delete with prefix
  let tombstone_key = format!("tombstones/profiles/{}.json", profile_id);
  let response = client
    .client
    .post(client.url("delete-prefix"))
    .header("Authorization", format!("Bearer {}", client.token))
    .json(&json!({
      "prefix": prefix,
      "tombstoneKey": tombstone_key
    }))
    .send()
    .await
    .unwrap();

  assert!(response.status().is_success());

  let result: serde_json::Value = response.json().await.unwrap();
  assert_eq!(result["deletedCount"].as_u64().unwrap(), 4);
  assert!(result["tombstoneCreated"].as_bool().unwrap());

  // Verify all files are deleted
  for key in &file_keys {
    let stat = client.stat(key).await.unwrap();
    assert!(
      !stat.exists,
      "File should be deleted after delete-prefix: {}",
      key
    );
  }

  // Verify tombstone exists
  let tombstone_stat = client.stat(&tombstone_key).await.unwrap();
  assert!(tombstone_stat.exists, "Tombstone should exist");

  // Cleanup tombstone
  client.delete(&tombstone_key, None).await.unwrap();
}

#[tokio::test]
async fn test_delta_sync_only_changed_files() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let profile_id = uuid::Uuid::new_v4().to_string();

  // Simulate initial upload of 3 files
  let file1_key = format!("profiles/{}/files/file1.txt", profile_id);
  let file2_key = format!("profiles/{}/files/file2.txt", profile_id);
  let file3_key = format!("profiles/{}/files/file3.txt", profile_id);

  let presign1 = client
    .presign_upload(&file1_key, "text/plain")
    .await
    .unwrap();
  client
    .upload_bytes(&presign1.url, b"content1", "text/plain")
    .await
    .unwrap();

  let presign2 = client
    .presign_upload(&file2_key, "text/plain")
    .await
    .unwrap();
  client
    .upload_bytes(&presign2.url, b"content2", "text/plain")
    .await
    .unwrap();

  let presign3 = client
    .presign_upload(&file3_key, "text/plain")
    .await
    .unwrap();
  client
    .upload_bytes(&presign3.url, b"content3", "text/plain")
    .await
    .unwrap();

  // Get initial stats
  let stat1_before = client.stat(&file1_key).await.unwrap();
  let stat2_before = client.stat(&file2_key).await.unwrap();
  let stat3_before = client.stat(&file3_key).await.unwrap();

  // Wait a moment for timestamp differentiation
  tokio::time::sleep(std::time::Duration::from_secs(1)).await;

  // Simulate delta sync: only update file2
  let presign2_update = client
    .presign_upload(&file2_key, "text/plain")
    .await
    .unwrap();
  client
    .upload_bytes(&presign2_update.url, b"content2-updated", "text/plain")
    .await
    .unwrap();

  // Check that file2's metadata changed
  let stat2_after = client.stat(&file2_key).await.unwrap();
  assert_ne!(
    stat2_before.size, stat2_after.size,
    "File2 size should have changed"
  );

  // Verify file1 and file3 are unchanged (same size)
  let stat1_after = client.stat(&file1_key).await.unwrap();
  let stat3_after = client.stat(&file3_key).await.unwrap();
  assert_eq!(
    stat1_before.size, stat1_after.size,
    "File1 should be unchanged"
  );
  assert_eq!(
    stat3_before.size, stat3_after.size,
    "File3 should be unchanged"
  );

  // Cleanup
  client.delete(&file1_key, None).await.unwrap();
  client.delete(&file2_key, None).await.unwrap();
  client.delete(&file3_key, None).await.unwrap();
}

#[tokio::test]
async fn test_profile_bypass_rules_sync() {
  ensure_sync_server_available().await;
  let client = TestClient::new();
  let temp_dir = TempDir::new().unwrap();
  let profile_id = uuid::Uuid::new_v4().to_string();
  let test_key = format!("profiles/{}.tar.gz", profile_id);

  let bypass_rules = vec!["example.com", "192.168.1.0/24", ".*\\.internal\\.net"];

  let bundle = create_test_profile_bundle_with_bypass_rules(temp_dir.path(), &bypass_rules);

  let presign = client
    .presign_upload(&test_key, "application/gzip")
    .await
    .unwrap();
  client
    .upload_bytes(&presign.url, &bundle, "application/gzip")
    .await
    .unwrap();

  let stat = client.stat(&test_key).await.unwrap();
  assert!(stat.exists);

  // Download and verify bypass rules survive the round-trip
  let download_presign = client.presign_download(&test_key).await.unwrap();
  let downloaded = client.download_bytes(&download_presign.url).await.unwrap();
  assert_eq!(downloaded.len(), bundle.len());

  let extract_dir = temp_dir.path().join("extracted");
  fs::create_dir_all(&extract_dir).unwrap();
  let metadata = extract_bundle(&downloaded, &extract_dir);

  assert_eq!(metadata["name"], "Bypass Rules Profile");
  assert_eq!(metadata["browser"], "camoufox");

  let synced_rules = metadata["proxy_bypass_rules"]
    .as_array()
    .expect("proxy_bypass_rules should be an array");
  assert_eq!(synced_rules.len(), 3);
  assert_eq!(synced_rules[0], "example.com");
  assert_eq!(synced_rules[1], "192.168.1.0/24");
  assert_eq!(synced_rules[2], ".*\\.internal\\.net");

  // Also verify empty bypass rules are handled correctly
  let empty_bundle = create_test_profile_bundle_with_bypass_rules(temp_dir.path(), &[]);
  let empty_key = format!("profiles/{}.tar.gz", uuid::Uuid::new_v4());

  let presign2 = client
    .presign_upload(&empty_key, "application/gzip")
    .await
    .unwrap();
  client
    .upload_bytes(&presign2.url, &empty_bundle, "application/gzip")
    .await
    .unwrap();

  let download_presign2 = client.presign_download(&empty_key).await.unwrap();
  let downloaded2 = client.download_bytes(&download_presign2.url).await.unwrap();

  let extract_dir2 = temp_dir.path().join("extracted2");
  fs::create_dir_all(&extract_dir2).unwrap();
  let metadata2 = extract_bundle(&downloaded2, &extract_dir2);

  let empty_rules = metadata2["proxy_bypass_rules"]
    .as_array()
    .expect("proxy_bypass_rules should be an array");
  assert!(empty_rules.is_empty());

  // Cleanup
  client.delete(&test_key, None).await.unwrap();
  client.delete(&empty_key, None).await.unwrap();
}

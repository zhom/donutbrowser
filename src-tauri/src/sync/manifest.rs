use chrono::{DateTime, Utc};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::SystemTime;

use super::types::{SyncError, SyncResult};

/// Default exclude patterns for volatile Chromium profile files
pub const DEFAULT_EXCLUDE_PATTERNS: &[&str] = &[
  "Cache/**",
  "Code Cache/**",
  "GPUCache/**",
  "GrShaderCache/**",
  "ShaderCache/**",
  "Service Worker/CacheStorage/**",
  "Crashpad/**",
  "Crash Reports/**",
  "BrowserMetrics/**",
  "blob_storage/**",
  "*.log",
  "*.tmp",
  "LOG",
  "LOG.old",
  "LOCK",
  ".donut-sync/**",
];

/// A single file entry in the manifest
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestFileEntry {
  pub path: String,
  pub size: u64,
  pub mtime: i64,
  pub hash: String,
}

/// The sync manifest for a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifest {
  pub version: u32,
  #[serde(rename = "profileId")]
  pub profile_id: String,
  #[serde(rename = "generatedAt")]
  pub generated_at: String,
  #[serde(rename = "updatedAt")]
  pub updated_at: String,
  #[serde(rename = "excludeGlobs")]
  pub exclude_globs: Vec<String>,
  pub files: Vec<ManifestFileEntry>,
}

impl SyncManifest {
  pub fn new(profile_id: String, exclude_globs: Vec<String>) -> Self {
    let now = Utc::now().to_rfc3339();
    Self {
      version: 1,
      profile_id,
      generated_at: now.clone(),
      updated_at: now,
      exclude_globs,
      files: Vec::new(),
    }
  }

  pub fn updated_at_datetime(&self) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&self.updated_at)
      .ok()
      .map(|dt| dt.with_timezone(&Utc))
  }
}

/// Local hash cache to avoid re-hashing unchanged files
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HashCache {
  pub entries: HashMap<String, HashCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashCacheEntry {
  pub size: u64,
  pub mtime: i64,
  pub hash: String,
}

impl HashCache {
  pub fn load(cache_path: &Path) -> Self {
    if !cache_path.exists() {
      return Self::default();
    }

    match fs::read_to_string(cache_path) {
      Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
      Err(_) => Self::default(),
    }
  }

  pub fn save(&self, cache_path: &Path) -> SyncResult<()> {
    if let Some(parent) = cache_path.parent() {
      fs::create_dir_all(parent).map_err(|e| {
        SyncError::IoError(format!(
          "Failed to create cache directory {}: {e}",
          parent.display()
        ))
      })?;
    }

    let json = serde_json::to_string_pretty(self)
      .map_err(|e| SyncError::SerializationError(format!("Failed to serialize hash cache: {e}")))?;

    fs::write(cache_path, json).map_err(|e| {
      SyncError::IoError(format!(
        "Failed to write hash cache {}: {e}",
        cache_path.display()
      ))
    })?;

    Ok(())
  }

  pub fn get(&self, path: &str, size: u64, mtime: i64) -> Option<&str> {
    self.entries.get(path).and_then(|entry| {
      if entry.size == size && entry.mtime == mtime {
        Some(entry.hash.as_str())
      } else {
        None
      }
    })
  }

  pub fn insert(&mut self, path: String, size: u64, mtime: i64, hash: String) {
    self
      .entries
      .insert(path, HashCacheEntry { size, mtime, hash });
  }
}

/// Build a GlobSet from exclude patterns
fn build_exclude_globset(patterns: &[String]) -> SyncResult<GlobSet> {
  let mut builder = GlobSetBuilder::new();
  for pattern in patterns {
    let glob = Glob::new(pattern)
      .map_err(|e| SyncError::InvalidData(format!("Invalid exclude pattern '{}': {e}", pattern)))?;
    builder.add(glob);
  }
  builder
    .build()
    .map_err(|e| SyncError::InvalidData(format!("Failed to build exclude globset: {e}")))
}

/// Compute blake3 hash of a file
/// Returns None if the file doesn't exist (was deleted)
fn hash_file(path: &Path) -> Result<Option<String>, SyncError> {
  let file = match File::open(path) {
    Ok(f) => f,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
    Err(e) => {
      return Err(SyncError::IoError(format!(
        "Failed to open {}: {e}",
        path.display()
      )));
    }
  };

  let mut reader = BufReader::new(file);
  let mut hasher = blake3::Hasher::new();
  let mut buffer = [0u8; 65536]; // 64KB buffer

  loop {
    let bytes_read = reader
      .read(&mut buffer)
      .map_err(|e| SyncError::IoError(format!("Failed to read {}: {e}", path.display())))?;
    if bytes_read == 0 {
      break;
    }
    hasher.update(&buffer[..bytes_read]);
  }

  Ok(Some(hasher.finalize().to_hex().to_string()))
}

/// Get mtime as unix timestamp
/// Returns None if the file doesn't exist (was deleted)
fn get_mtime(path: &Path) -> Result<Option<i64>, SyncError> {
  let metadata = match path.metadata() {
    Ok(m) => m,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
    Err(e) => {
      return Err(SyncError::IoError(format!(
        "Failed to get metadata for {}: {e}",
        path.display()
      )));
    }
  };

  let mtime = metadata
    .modified()
    .map_err(|e| SyncError::IoError(format!("Failed to get mtime for {}: {e}", path.display())))?;

  Ok(Some(
    mtime
      .duration_since(SystemTime::UNIX_EPOCH)
      .map(|d| d.as_secs() as i64)
      .unwrap_or(0),
  ))
}

/// Generate a manifest for a profile directory
pub fn generate_manifest(
  profile_id: &str,
  profile_dir: &Path,
  cache: &mut HashCache,
) -> SyncResult<SyncManifest> {
  let exclude_patterns: Vec<String> = DEFAULT_EXCLUDE_PATTERNS
    .iter()
    .map(|s| s.to_string())
    .collect();
  let globset = build_exclude_globset(&exclude_patterns)?;

  let mut manifest = SyncManifest::new(profile_id.to_string(), exclude_patterns);
  let mut max_mtime: i64 = 0;

  if !profile_dir.exists() {
    log::debug!(
      "Profile directory doesn't exist: {}, creating empty manifest",
      profile_dir.display()
    );
    return Ok(manifest);
  }

  fn walk_dir(
    dir: &Path,
    base_dir: &Path,
    globset: &GlobSet,
    cache: &mut HashCache,
    files: &mut Vec<ManifestFileEntry>,
    max_mtime: &mut i64,
  ) -> SyncResult<()> {
    let entries = fs::read_dir(dir).map_err(|e| {
      SyncError::IoError(format!("Failed to read directory {}: {e}", dir.display()))
    })?;

    for entry in entries {
      let entry = entry.map_err(|e| {
        SyncError::IoError(format!("Failed to read entry in {}: {e}", dir.display()))
      })?;

      let path = entry.path();
      let relative_path = path
        .strip_prefix(base_dir)
        .map_err(|_| SyncError::IoError("Failed to compute relative path".to_string()))?
        .to_string_lossy()
        .replace('\\', "/");

      // Check if excluded
      if globset.is_match(&relative_path) {
        continue;
      }

      // Get metadata - skip if file was deleted between directory read and metadata access
      let metadata = match path.metadata() {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
          log::debug!(
            "File disappeared during manifest generation, skipping: {}",
            path.display()
          );
          continue;
        }
        Err(e) => {
          return Err(SyncError::IoError(format!(
            "Failed to get metadata for {}: {e}",
            path.display()
          )));
        }
      };

      if metadata.is_dir() {
        walk_dir(&path, base_dir, globset, cache, files, max_mtime)?;
      } else if metadata.is_file() {
        let size = metadata.len();
        let mtime = match get_mtime(&path)? {
          Some(m) => m,
          None => {
            // File was deleted, skip it
            log::debug!(
              "File disappeared during manifest generation, skipping: {}",
              path.display()
            );
            continue;
          }
        };

        *max_mtime = (*max_mtime).max(mtime);

        // Check cache for existing hash
        let hash = if let Some(cached_hash) = cache.get(&relative_path, size, mtime) {
          cached_hash.to_string()
        } else {
          match hash_file(&path)? {
            Some(computed_hash) => {
              cache.insert(relative_path.clone(), size, mtime, computed_hash.clone());
              computed_hash
            }
            None => {
              // File was deleted, skip it
              log::debug!(
                "File disappeared during manifest generation, skipping: {}",
                path.display()
              );
              continue;
            }
          }
        };

        files.push(ManifestFileEntry {
          path: relative_path,
          size,
          mtime,
          hash,
        });
      }
    }

    Ok(())
  }

  walk_dir(
    profile_dir,
    profile_dir,
    &globset,
    cache,
    &mut manifest.files,
    &mut max_mtime,
  )?;

  // Sort files for deterministic manifest
  manifest.files.sort_by(|a, b| a.path.cmp(&b.path));

  // Update the updatedAt timestamp to max mtime
  if max_mtime > 0 {
    if let Some(dt) = DateTime::from_timestamp(max_mtime, 0) {
      manifest.updated_at = dt.to_rfc3339();
    }
  }

  Ok(manifest)
}

/// Compute the diff between local and remote manifests
#[derive(Debug, Default)]
pub struct ManifestDiff {
  pub files_to_upload: Vec<ManifestFileEntry>,
  pub files_to_download: Vec<ManifestFileEntry>,
  pub files_to_delete_local: Vec<String>,
  pub files_to_delete_remote: Vec<String>,
}

impl ManifestDiff {
  pub fn is_empty(&self) -> bool {
    self.files_to_upload.is_empty()
      && self.files_to_download.is_empty()
      && self.files_to_delete_local.is_empty()
      && self.files_to_delete_remote.is_empty()
  }
}

/// Compute what needs to be synced between local and remote
pub fn compute_diff(local: &SyncManifest, remote: Option<&SyncManifest>) -> ManifestDiff {
  let mut diff = ManifestDiff::default();

  let Some(remote) = remote else {
    // No remote manifest - upload everything
    diff.files_to_upload = local.files.clone();
    return diff;
  };

  // Build hash maps for quick lookup
  let local_files: HashMap<&str, &ManifestFileEntry> =
    local.files.iter().map(|f| (f.path.as_str(), f)).collect();
  let remote_files: HashMap<&str, &ManifestFileEntry> =
    remote.files.iter().map(|f| (f.path.as_str(), f)).collect();

  // Compare timestamps to determine direction
  let local_updated = local.updated_at_datetime();
  let remote_updated = remote.updated_at_datetime();

  let local_is_newer = match (local_updated, remote_updated) {
    (Some(l), Some(r)) => l > r,
    (Some(_), None) => true,
    (None, Some(_)) => false,
    (None, None) => true, // Default to uploading
  };

  if local_is_newer {
    // Upload changed/new files, delete remote files that don't exist locally
    for (path, local_entry) in &local_files {
      match remote_files.get(path) {
        Some(remote_entry) if remote_entry.hash != local_entry.hash => {
          diff.files_to_upload.push((*local_entry).clone());
        }
        None => {
          diff.files_to_upload.push((*local_entry).clone());
        }
        _ => {}
      }
    }

    for path in remote_files.keys() {
      if !local_files.contains_key(path) {
        diff.files_to_delete_remote.push(path.to_string());
      }
    }
  } else {
    // Download changed/new files, delete local files that don't exist remotely
    for (path, remote_entry) in &remote_files {
      match local_files.get(path) {
        Some(local_entry) if local_entry.hash != remote_entry.hash => {
          diff.files_to_download.push((*remote_entry).clone());
        }
        None => {
          diff.files_to_download.push((*remote_entry).clone());
        }
        _ => {}
      }
    }

    for path in local_files.keys() {
      if !remote_files.contains_key(path) {
        diff.files_to_delete_local.push(path.to_string());
      }
    }
  }

  diff
}

/// Get the path to the hash cache file for a profile
pub fn get_cache_path(profile_dir: &Path) -> std::path::PathBuf {
  profile_dir.join(".donut-sync").join("cache.json")
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  #[test]
  fn test_hash_cache_operations() {
    let cache_dir = TempDir::new().unwrap();
    let cache_path = cache_dir.path().join("cache.json");

    let mut cache = HashCache::default();
    cache.insert(
      "test.txt".to_string(),
      100,
      1234567890,
      "abc123".to_string(),
    );

    assert_eq!(cache.get("test.txt", 100, 1234567890), Some("abc123"));
    assert_eq!(cache.get("test.txt", 100, 999), None); // Different mtime
    assert_eq!(cache.get("test.txt", 50, 1234567890), None); // Different size

    cache.save(&cache_path).unwrap();

    let loaded = HashCache::load(&cache_path);
    assert_eq!(loaded.get("test.txt", 100, 1234567890), Some("abc123"));
  }

  #[test]
  fn test_generate_manifest_empty_dir() {
    let temp_dir = TempDir::new().unwrap();
    let profile_dir = temp_dir.path().join("profile");
    fs::create_dir_all(&profile_dir).unwrap();

    let mut cache = HashCache::default();
    let manifest = generate_manifest("test-profile", &profile_dir, &mut cache).unwrap();

    assert_eq!(manifest.profile_id, "test-profile");
    assert_eq!(manifest.version, 1);
    assert!(manifest.files.is_empty());
  }

  #[test]
  fn test_generate_manifest_with_files() {
    let temp_dir = TempDir::new().unwrap();
    let profile_dir = temp_dir.path().join("profile");
    fs::create_dir_all(&profile_dir).unwrap();

    fs::write(profile_dir.join("file1.txt"), "hello").unwrap();
    fs::write(profile_dir.join("file2.txt"), "world").unwrap();
    fs::create_dir_all(profile_dir.join("subdir")).unwrap();
    fs::write(profile_dir.join("subdir/file3.txt"), "nested").unwrap();

    let mut cache = HashCache::default();
    let manifest = generate_manifest("test-profile", &profile_dir, &mut cache).unwrap();

    assert_eq!(manifest.files.len(), 3);
    assert!(manifest.files.iter().any(|f| f.path == "file1.txt"));
    assert!(manifest.files.iter().any(|f| f.path == "file2.txt"));
    assert!(manifest.files.iter().any(|f| f.path == "subdir/file3.txt"));
  }

  #[test]
  fn test_generate_manifest_excludes_cache() {
    let temp_dir = TempDir::new().unwrap();
    let profile_dir = temp_dir.path().join("profile");
    fs::create_dir_all(&profile_dir).unwrap();

    fs::write(profile_dir.join("file1.txt"), "keep").unwrap();
    fs::create_dir_all(profile_dir.join("Cache")).unwrap();
    fs::write(profile_dir.join("Cache/data"), "exclude").unwrap();
    fs::create_dir_all(profile_dir.join("Code Cache")).unwrap();
    fs::write(profile_dir.join("Code Cache/wasm"), "exclude").unwrap();

    let mut cache = HashCache::default();
    let manifest = generate_manifest("test-profile", &profile_dir, &mut cache).unwrap();

    assert_eq!(manifest.files.len(), 1);
    assert_eq!(manifest.files[0].path, "file1.txt");
  }

  #[test]
  fn test_compute_diff_upload_all_when_no_remote() {
    let local = SyncManifest {
      version: 1,
      profile_id: "test".to_string(),
      generated_at: Utc::now().to_rfc3339(),
      updated_at: Utc::now().to_rfc3339(),
      exclude_globs: vec![],
      files: vec![
        ManifestFileEntry {
          path: "file1.txt".to_string(),
          size: 10,
          mtime: 1000,
          hash: "abc".to_string(),
        },
        ManifestFileEntry {
          path: "file2.txt".to_string(),
          size: 20,
          mtime: 2000,
          hash: "def".to_string(),
        },
      ],
    };

    let diff = compute_diff(&local, None);

    assert_eq!(diff.files_to_upload.len(), 2);
    assert!(diff.files_to_download.is_empty());
    assert!(diff.files_to_delete_local.is_empty());
    assert!(diff.files_to_delete_remote.is_empty());
  }

  #[test]
  fn test_compute_diff_detect_changes() {
    let old_time = "2024-01-01T00:00:00Z";
    let new_time = "2024-01-02T00:00:00Z";

    let local = SyncManifest {
      version: 1,
      profile_id: "test".to_string(),
      generated_at: new_time.to_string(),
      updated_at: new_time.to_string(),
      exclude_globs: vec![],
      files: vec![
        ManifestFileEntry {
          path: "unchanged.txt".to_string(),
          size: 10,
          mtime: 1000,
          hash: "same".to_string(),
        },
        ManifestFileEntry {
          path: "changed.txt".to_string(),
          size: 10,
          mtime: 2000,
          hash: "new_hash".to_string(),
        },
        ManifestFileEntry {
          path: "new_file.txt".to_string(),
          size: 5,
          mtime: 3000,
          hash: "new".to_string(),
        },
      ],
    };

    let remote = SyncManifest {
      version: 1,
      profile_id: "test".to_string(),
      generated_at: old_time.to_string(),
      updated_at: old_time.to_string(),
      exclude_globs: vec![],
      files: vec![
        ManifestFileEntry {
          path: "unchanged.txt".to_string(),
          size: 10,
          mtime: 1000,
          hash: "same".to_string(),
        },
        ManifestFileEntry {
          path: "changed.txt".to_string(),
          size: 10,
          mtime: 1000,
          hash: "old_hash".to_string(),
        },
        ManifestFileEntry {
          path: "deleted.txt".to_string(),
          size: 8,
          mtime: 500,
          hash: "gone".to_string(),
        },
      ],
    };

    let diff = compute_diff(&local, Some(&remote));

    // Local is newer, so we upload changed/new and delete remote-only
    assert_eq!(diff.files_to_upload.len(), 2); // changed + new
    assert!(diff.files_to_upload.iter().any(|f| f.path == "changed.txt"));
    assert!(diff
      .files_to_upload
      .iter()
      .any(|f| f.path == "new_file.txt"));
    assert!(diff.files_to_download.is_empty());
    assert!(diff.files_to_delete_local.is_empty());
    assert_eq!(diff.files_to_delete_remote.len(), 1);
    assert!(diff
      .files_to_delete_remote
      .contains(&"deleted.txt".to_string()));
  }
}

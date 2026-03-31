use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::app_dirs;

const REFRESH_INTERVAL: Duration = Duration::from_secs(43200); // 12 hours

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlocklistLevel {
  #[default]
  None,
  Light,
  Normal,
  Pro,
  ProPlus,
  Ultimate,
}

impl BlocklistLevel {
  pub fn parse_level(s: &str) -> Option<Self> {
    match s {
      "light" => Some(Self::Light),
      "normal" => Some(Self::Normal),
      "pro" => Some(Self::Pro),
      "pro_plus" => Some(Self::ProPlus),
      "ultimate" => Some(Self::Ultimate),
      "none" => Some(Self::None),
      _ => None,
    }
  }

  pub fn as_str(&self) -> &'static str {
    match self {
      Self::None => "none",
      Self::Light => "light",
      Self::Normal => "normal",
      Self::Pro => "pro",
      Self::ProPlus => "pro_plus",
      Self::Ultimate => "ultimate",
    }
  }

  pub fn display_name(&self) -> &'static str {
    match self {
      Self::None => "None",
      Self::Light => "Light",
      Self::Normal => "Normal",
      Self::Pro => "Pro",
      Self::ProPlus => "Pro++",
      Self::Ultimate => "Ultimate",
    }
  }

  pub fn url(&self) -> Option<&'static str> {
    match self {
      Self::None => None,
      Self::Light => {
        Some("https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/domains/light.txt")
      }
      Self::Normal => {
        Some("https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/domains/multi.txt")
      }
      Self::Pro => Some("https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/domains/pro.txt"),
      Self::ProPlus => {
        Some("https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/domains/pro.plus.txt")
      }
      Self::Ultimate => {
        Some("https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/domains/ultimate.txt")
      }
    }
  }

  pub fn filename(&self) -> Option<&'static str> {
    match self {
      Self::None => None,
      Self::Light => Some("light.txt"),
      Self::Normal => Some("multi.txt"),
      Self::Pro => Some("pro.txt"),
      Self::ProPlus => Some("pro.plus.txt"),
      Self::Ultimate => Some("ultimate.txt"),
    }
  }

  pub fn all_downloadable() -> &'static [BlocklistLevel] {
    &[
      Self::Light,
      Self::Normal,
      Self::Pro,
      Self::ProPlus,
      Self::Ultimate,
    ]
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlocklistCacheStatus {
  pub level: String,
  pub display_name: String,
  pub entry_count: usize,
  pub file_size_bytes: u64,
  pub last_updated: Option<u64>,
  pub is_fresh: bool,
  pub is_cached: bool,
}

pub struct BlocklistManager;

lazy_static::lazy_static! {
  static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::builder()
    .timeout(Duration::from_secs(60))
    .build()
    .expect("Failed to create HTTP client");
}

impl BlocklistManager {
  pub fn instance() -> &'static BlocklistManager {
    &BLOCKLIST_MANAGER
  }

  fn cache_dir() -> PathBuf {
    app_dirs::dns_blocklist_dir()
  }

  pub fn cached_file_path(level: BlocklistLevel) -> Option<PathBuf> {
    level.filename().map(|f| Self::cache_dir().join(f))
  }

  pub fn is_cache_fresh(level: BlocklistLevel) -> bool {
    let Some(path) = Self::cached_file_path(level) else {
      return false;
    };
    if !path.exists() {
      return false;
    }
    match std::fs::metadata(&path).and_then(|m| m.modified()) {
      Ok(modified) => SystemTime::now()
        .duration_since(modified)
        .map(|age| age < REFRESH_INTERVAL)
        .unwrap_or(false),
      Err(_) => false,
    }
  }

  pub async fn fetch_blocklist(level: BlocklistLevel) -> Result<PathBuf, String> {
    let url = level
      .url()
      .ok_or_else(|| format!("No URL for level {:?}", level))?;
    let path =
      Self::cached_file_path(level).ok_or_else(|| format!("No filename for level {:?}", level))?;

    let cache_dir = Self::cache_dir();
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("Failed to create cache dir: {e}"))?;

    log::info!(
      "[dns-blocklist] Fetching {} from {}",
      level.display_name(),
      url
    );

    let response = HTTP_CLIENT
      .get(url)
      .send()
      .await
      .map_err(|e| format!("Failed to fetch blocklist: {e}"))?;

    if !response.status().is_success() {
      return Err(format!("HTTP {} when fetching {}", response.status(), url));
    }

    let body = response
      .text()
      .await
      .map_err(|e| format!("Failed to read response body: {e}"))?;

    // Write atomically: write to temp file, then rename
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, &body).map_err(|e| format!("Failed to write blocklist: {e}"))?;
    std::fs::rename(&tmp_path, &path).map_err(|e| format!("Failed to rename blocklist: {e}"))?;

    let entry_count = body
      .lines()
      .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
      .count();
    log::info!(
      "[dns-blocklist] Cached {} ({} domains)",
      level.display_name(),
      entry_count
    );

    Ok(path)
  }

  pub async fn ensure_cached(level: BlocklistLevel) -> Result<PathBuf, String> {
    if let Some(path) = Self::cached_file_path(level) {
      if path.exists() {
        return Ok(path);
      }
    }
    Self::fetch_blocklist(level).await
  }

  pub async fn refresh_all_stale(&self) {
    for &level in BlocklistLevel::all_downloadable() {
      if !Self::is_cache_fresh(level) {
        if let Err(e) = Self::fetch_blocklist(level).await {
          log::error!(
            "[dns-blocklist] Failed to refresh {}: {e}",
            level.display_name()
          );
          let _ = crate::events::emit(
            "dns-blocklist-refresh-failed",
            serde_json::json!({
              "level": level.as_str(),
              "error": e,
            }),
          );
        }
      }
    }
  }

  pub fn get_blocklist_file_path(level: BlocklistLevel) -> Option<PathBuf> {
    Self::cached_file_path(level).filter(|p| p.exists())
  }

  pub fn get_cache_status() -> Vec<BlocklistCacheStatus> {
    BlocklistLevel::all_downloadable()
      .iter()
      .map(|&level| {
        let path = Self::cached_file_path(level);
        let metadata = path.as_ref().and_then(|p| std::fs::metadata(p).ok());
        let is_cached = metadata.is_some();

        let entry_count = if is_cached {
          path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|content| {
              content
                .lines()
                .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                .count()
            })
            .unwrap_or(0)
        } else {
          0
        };

        let file_size_bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

        let last_updated = metadata
          .as_ref()
          .and_then(|m| m.modified().ok())
          .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
          .map(|d| d.as_secs());

        BlocklistCacheStatus {
          level: level.as_str().to_string(),
          display_name: level.display_name().to_string(),
          entry_count,
          file_size_bytes,
          last_updated,
          is_fresh: Self::is_cache_fresh(level),
          is_cached,
        }
      })
      .collect()
  }
}

lazy_static::lazy_static! {
  static ref BLOCKLIST_MANAGER: BlocklistManager = BlocklistManager;
}

// Tauri commands

#[tauri::command]
pub async fn get_dns_blocklist_cache_status() -> Result<Vec<BlocklistCacheStatus>, String> {
  Ok(BlocklistManager::get_cache_status())
}

#[tauri::command]
pub async fn refresh_dns_blocklists() -> Result<(), String> {
  for &level in BlocklistLevel::all_downloadable() {
    BlocklistManager::fetch_blocklist(level).await?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_level_roundtrip() {
    for &level in BlocklistLevel::all_downloadable() {
      let s = level.as_str();
      let parsed = BlocklistLevel::parse_level(s);
      assert_eq!(parsed, Some(level), "Roundtrip failed for {s}");
    }
    assert_eq!(
      BlocklistLevel::parse_level("none"),
      Some(BlocklistLevel::None)
    );
  }

  #[test]
  fn test_level_urls_all_present() {
    for &level in BlocklistLevel::all_downloadable() {
      assert!(
        level.url().is_some(),
        "{} should have a URL",
        level.as_str()
      );
      assert!(
        level.filename().is_some(),
        "{} should have a filename",
        level.as_str()
      );
    }
    assert!(BlocklistLevel::None.url().is_none());
    assert!(BlocklistLevel::None.filename().is_none());
  }

  #[test]
  fn test_cache_status_returns_all_levels() {
    let statuses = BlocklistManager::get_cache_status();
    assert_eq!(statuses.len(), 5);
    assert_eq!(statuses[0].level, "light");
    assert_eq!(statuses[1].level, "normal");
    assert_eq!(statuses[2].level, "pro");
    assert_eq!(statuses[3].level, "pro_plus");
    assert_eq!(statuses[4].level, "ultimate");
  }

  #[test]
  fn test_cache_fresh_returns_false_when_missing() {
    assert!(!BlocklistManager::is_cache_fresh(BlocklistLevel::Light));
    assert!(!BlocklistManager::is_cache_fresh(BlocklistLevel::None));
  }
}

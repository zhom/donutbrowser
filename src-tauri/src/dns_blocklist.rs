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
  /// User-defined list: compiled from custom source URLs + custom block
  /// domains, with custom allow domains removed (allowlist overrides).
  Custom,
}

impl BlocklistLevel {
  pub fn parse_level(s: &str) -> Option<Self> {
    match s {
      "light" => Some(Self::Light),
      "normal" => Some(Self::Normal),
      "pro" => Some(Self::Pro),
      "pro_plus" => Some(Self::ProPlus),
      "ultimate" => Some(Self::Ultimate),
      "custom" => Some(Self::Custom),
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
      Self::Custom => "custom",
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
      Self::Custom => "Custom",
    }
  }

  pub fn url(&self) -> Option<&'static str> {
    match self {
      Self::None | Self::Custom => None,
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
      Self::Custom => Some("custom.txt"),
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

/// User-defined DNS filtering: extra blocklist source URLs plus manual block /
/// allow domain rules. Allow rules override blocks (the standard exceptions
/// model). Stored as one JSON blob; the compiled result lands in `custom.txt`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomDnsConfig {
  #[serde(default)]
  pub sources: Vec<String>,
  #[serde(default)]
  pub block_domains: Vec<String>,
  #[serde(default)]
  pub allow_domains: Vec<String>,
  /// When true the custom list is a strict allowlist: the browser may only
  /// reach `allow_domains` (and their subdomains); everything else is blocked.
  /// Sources/block_domains are ignored in this mode.
  #[serde(default)]
  pub allowlist_mode: bool,
  #[serde(default)]
  pub updated_at: Option<u64>,
}

fn normalize_domain(raw: &str) -> Option<String> {
  let mut line = raw.trim();
  if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
    return None;
  }
  // Hosts-file format ("0.0.0.0 ads.example.com", "127.0.0.1 tracker.net") is
  // common in public blocklists — take the domain after the sink IP rather
  // than rejecting the whole line on the embedded space.
  if let Some((first, rest)) = line.split_once(char::is_whitespace) {
    if matches!(first, "0.0.0.0" | "127.0.0.1" | "::" | "::1") {
      line = rest.trim();
    }
  }
  // Strip a trailing comment ("0.0.0.0 ads.example.com # AdGuard"). Public
  // hosts lists annotate entries this way; without this the whitespace guard
  // below rejects every annotated line, silently dropping most of the source.
  for marker in ['#', '!'] {
    if let Some((before, _)) = line.split_once(marker) {
      line = before.trim();
    }
  }
  let d = line
    .trim_start_matches("*.")
    .trim_start_matches("||")
    .trim_end_matches('^')
    .trim_end_matches('.')
    .to_lowercase();
  if d.is_empty() {
    return None;
  }
  // Reject anything still containing whitespace or a scheme — not a bare domain.
  if d.contains(char::is_whitespace) || d.contains("://") {
    return None;
  }
  Some(d)
}

impl CustomDnsConfig {
  fn path() -> PathBuf {
    app_dirs::data_subdir().join("custom_dns.json")
  }

  pub fn load() -> Self {
    match std::fs::read_to_string(Self::path()) {
      Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
      Err(_) => Self::default(),
    }
  }

  pub fn save(&self) -> Result<(), String> {
    let path = Self::path();
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
  }

  /// Serialize to a plain-text rule list (uBlock-ish): `! source:` comments for
  /// source URLs, `@@domain` for allow rules, bare domains for blocks.
  pub fn to_txt(&self) -> String {
    let mut out = String::new();
    for s in &self.sources {
      out.push_str("! source: ");
      out.push_str(s);
      out.push('\n');
    }
    for d in &self.allow_domains {
      out.push_str("@@");
      out.push_str(d);
      out.push('\n');
    }
    for d in &self.block_domains {
      out.push_str(d);
      out.push('\n');
    }
    out
  }

  /// Parse a plain-text rule list back into a config (sources from
  /// `! source:` lines, `@@`-prefixed as allow, bare domains as block).
  pub fn from_txt(content: &str) -> Self {
    let mut cfg = CustomDnsConfig::default();
    for line in content.lines() {
      let line = line.trim();
      if line.is_empty() {
        continue;
      }
      if let Some(src) = line.strip_prefix("! source:") {
        let src = src.trim();
        if !src.is_empty() {
          cfg.sources.push(src.to_string());
        }
        continue;
      }
      if line.starts_with('#') || line.starts_with('!') {
        continue;
      }
      if let Some(allow) = line.strip_prefix("@@") {
        if let Some(d) = normalize_domain(allow) {
          cfg.allow_domains.push(d);
        }
      } else if let Some(d) = normalize_domain(line) {
        cfg.block_domains.push(d);
      }
    }
    cfg.dedup();
    cfg
  }

  /// Drop duplicates, preserving first-seen order so an exported rule list
  /// still reads the way the user wrote it.
  fn dedup(&mut self) {
    for v in [
      &mut self.sources,
      &mut self.block_domains,
      &mut self.allow_domains,
    ] {
      let mut seen = std::collections::HashSet::new();
      v.retain(|item| seen.insert(item.clone()));
    }
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
    if level == BlocklistLevel::Custom {
      // Recompile only when the compiled file is missing or stale. Edits call
      // compile_custom_blocklist directly and refresh_all_stale keeps sources
      // current, so recompiling unconditionally would re-download every source
      // on every profile launch — and this is awaited on the blocking launch
      // path, so the browser cannot start until it finishes.
      if let Some(path) = Self::cached_file_path(level) {
        if Self::is_cache_fresh(level) {
          return Ok(path);
        }
      }
      return Self::compile_custom_blocklist().await;
    }
    if let Some(path) = Self::cached_file_path(level) {
      if path.exists() {
        return Ok(path);
      }
    }
    Self::fetch_blocklist(level).await
  }

  /// Compile the user's custom DNS file. In blocklist mode: fetch every source,
  /// union with the manual block domains, then remove the allow domains
  /// (allowlist overrides). In allowlist mode: the file is just the allow
  /// domains — the worker then blocks everything NOT listed. Always rewrites
  /// `custom.txt`.
  pub async fn compile_custom_blocklist() -> Result<PathBuf, String> {
    use std::collections::HashSet;

    let config = CustomDnsConfig::load();
    let mut domains: HashSet<String> = HashSet::new();
    let path =
      Self::cached_file_path(BlocklistLevel::Custom).ok_or("No filename for custom level")?;

    if config.allowlist_mode {
      // Strict allowlist: the compiled file is the set of permitted domains.
      for d in &config.allow_domains {
        if let Some(n) = normalize_domain(d) {
          domains.insert(n);
        }
      }
    } else {
      // Fetch every source concurrently. Sequential awaits make this the sum of
      // all source latencies, and it runs on the blocking profile-launch path.
      let fetches = config.sources.iter().map(|source| async move {
        match HTTP_CLIENT.get(source).send().await {
          Ok(resp) if resp.status().is_success() => resp
            .text()
            .await
            .map_err(|e| format!("custom source {source} body read failed: {e}")),
          Ok(resp) => Err(format!(
            "custom source {source} returned HTTP {}",
            resp.status()
          )),
          Err(e) => Err(format!("custom source {source} failed: {e}")),
        }
      });

      let mut source_failures = 0usize;
      for result in futures_util::future::join_all(fetches).await {
        match result {
          Ok(body) => {
            for line in body.lines() {
              if let Some(d) = normalize_domain(line) {
                domains.insert(d);
              }
            }
          }
          Err(e) => {
            log::warn!("[dns-blocklist] {e}");
            source_failures += 1;
          }
        }
      }

      // A failed source must never shrink the compiled list. Overwriting with a
      // short (or empty) result would silently stop blocking whatever that
      // source contributed — and `is_blocked` fails open on an empty set, so a
      // single offline launch would disable custom filtering entirely and
      // destroy the good cached list. Re-seed from the cached file so a network
      // failure degrades to "stale" instead of "off"; manual rule edits below
      // still apply, and the next successful compile rewrites cleanly.
      if source_failures > 0 {
        match std::fs::read_to_string(&path) {
          Ok(cached) => {
            let before = domains.len();
            for line in cached.lines() {
              if let Some(d) = normalize_domain(line) {
                domains.insert(d);
              }
            }
            log::warn!(
              "[dns-blocklist] {source_failures} custom source(s) failed; retained {} domain(s) from the cached list",
              domains.len().saturating_sub(before)
            );
          }
          Err(_) => log::warn!(
            "[dns-blocklist] {source_failures} custom source(s) failed and no cached list exists; custom filtering is incomplete"
          ),
        }
      }

      for d in &config.block_domains {
        if let Some(n) = normalize_domain(d) {
          domains.insert(n);
        }
      }
      // Allow rules override blocks (exceptions).
      for d in &config.allow_domains {
        if let Some(n) = normalize_domain(d) {
          domains.remove(&n);
        }
      }
    }

    let cache_dir = Self::cache_dir();
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("Failed to create cache dir: {e}"))?;

    let mut sorted: Vec<&str> = domains.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let body = sorted.join("\n");

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, &body)
      .map_err(|e| format!("Failed to write custom blocklist: {e}"))?;
    std::fs::rename(&tmp_path, &path)
      .map_err(|e| format!("Failed to rename custom blocklist: {e}"))?;

    log::info!(
      "[dns-blocklist] Compiled custom blocklist ({} domains)",
      domains.len()
    );
    Ok(path)
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
    // Recompile the custom list too so its sources track upstream changes.
    let config = CustomDnsConfig::load();
    if !config.sources.is_empty() || !config.block_domains.is_empty() {
      if let Err(e) = Self::compile_custom_blocklist().await {
        log::error!("[dns-blocklist] Failed to recompile custom list: {e}");
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

#[tauri::command]
pub async fn get_custom_dns_config() -> Result<CustomDnsConfig, String> {
  Ok(CustomDnsConfig::load())
}

/// Normalize, persist, recompile and announce a custom DNS config. Shared by
/// the set and import commands so both apply the same normalization and the
/// same side effects — a step added here reaches both.
async fn persist_custom_config(mut config: CustomDnsConfig) -> Result<CustomDnsConfig, String> {
  config.sources = std::mem::take(&mut config.sources)
    .into_iter()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect();
  config.block_domains = config
    .block_domains
    .iter()
    .filter_map(|d| normalize_domain(d))
    .collect();
  config.allow_domains = config
    .allow_domains
    .iter()
    .filter_map(|d| normalize_domain(d))
    .collect();
  config.updated_at = Some(crate::proxy_manager::now_secs());
  config.dedup();
  config
    .save()
    .map_err(|_| serde_json::json!({ "code": "DNS_RULES_SAVE_FAILED" }).to_string())?;
  // Recompile so a profile relaunch picks up the new rules immediately.
  let _ = BlocklistManager::compile_custom_blocklist().await;
  let _ = crate::events::emit_empty("custom-dns-changed");
  Ok(config)
}

/// Persist the custom DNS config and recompile the `custom.txt` list. Domains
/// are normalized; blank/comment entries are dropped.
#[tauri::command]
pub async fn set_custom_dns_config(
  sources: Vec<String>,
  block_domains: Vec<String>,
  allow_domains: Vec<String>,
  allowlist_mode: bool,
) -> Result<CustomDnsConfig, String> {
  persist_custom_config(CustomDnsConfig {
    sources,
    block_domains,
    allow_domains,
    allowlist_mode,
    updated_at: None,
  })
  .await
}

/// Import custom DNS rules. `format` is "json" (a full CustomDnsConfig) or
/// "txt" (uBlock-ish: `! source:`, `@@allow`, bare block domains).
#[tauri::command]
pub async fn import_custom_dns_rules(
  content: String,
  format: String,
) -> Result<CustomDnsConfig, String> {
  let config = match format.as_str() {
    "json" => serde_json::from_str::<CustomDnsConfig>(&content)
      .map_err(|_| serde_json::json!({ "code": "INVALID_DNS_RULES_JSON" }).to_string())?,
    "txt" => CustomDnsConfig::from_txt(&content),
    other => {
      return Err(
        serde_json::json!({
          "code": "UNSUPPORTED_DNS_RULES_FORMAT",
          "params": { "format": other },
        })
        .to_string(),
      )
    }
  };
  persist_custom_config(config).await
}

/// Export the custom DNS rules as "json" or "txt".
#[tauri::command]
pub async fn export_custom_dns_rules(format: String) -> Result<String, String> {
  let config = CustomDnsConfig::load();
  match format.as_str() {
    "json" => serde_json::to_string_pretty(&config)
      .map_err(|_| serde_json::json!({ "code": "DNS_RULES_EXPORT_FAILED" }).to_string()),
    "txt" => Ok(config.to_txt()),
    other => Err(
      serde_json::json!({
        "code": "UNSUPPORTED_DNS_RULES_FORMAT",
        "params": { "format": other },
      })
      .to_string(),
    ),
  }
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
  fn test_custom_config_txt_roundtrip() {
    let txt = "! source: https://example.com/list.txt\n@@allowed.com\nblocked.com\n*.tracker.net\n";
    let cfg = CustomDnsConfig::from_txt(txt);
    assert_eq!(cfg.sources, vec!["https://example.com/list.txt"]);
    assert_eq!(cfg.allow_domains, vec!["allowed.com"]);
    assert_eq!(cfg.block_domains, vec!["blocked.com", "tracker.net"]);
    // Re-exporting produces parseable text.
    let reparsed = CustomDnsConfig::from_txt(&cfg.to_txt());
    assert_eq!(reparsed.block_domains, cfg.block_domains);
    assert_eq!(reparsed.allow_domains, cfg.allow_domains);
    assert_eq!(reparsed.sources, cfg.sources);
  }

  #[test]
  fn test_normalize_domain() {
    assert_eq!(
      normalize_domain("*.Example.COM"),
      Some("example.com".into())
    );
    assert_eq!(normalize_domain("||ads.net^"), Some("ads.net".into()));
    assert_eq!(normalize_domain("  "), None);
    assert_eq!(normalize_domain("# comment"), None);
    assert_eq!(normalize_domain("http://x.com"), None);
    // Hosts-file format lines yield the domain, not None.
    assert_eq!(
      normalize_domain("0.0.0.0 ads.example.com"),
      Some("ads.example.com".into())
    );
    assert_eq!(
      normalize_domain("127.0.0.1\ttracker.net"),
      Some("tracker.net".into())
    );
    // A bare two-token line that isn't hosts-format is still rejected.
    assert_eq!(normalize_domain("foo bar"), None);
  }

  #[test]
  fn test_normalize_domain_strips_trailing_comments() {
    // Public hosts lists (StevenBlack, AdAway) annotate entries inline. Without
    // comment stripping the whitespace guard drops every annotated line, so a
    // source compiles down to a fraction of its real domain count.
    assert_eq!(
      normalize_domain("0.0.0.0 ads.example.com # AdGuard"),
      Some("ads.example.com".into())
    );
    assert_eq!(
      normalize_domain("127.0.0.1 tracker.net #comment"),
      Some("tracker.net".into())
    );
    assert_eq!(
      normalize_domain("plain.example.com  # trailing note"),
      Some("plain.example.com".into())
    );
    assert_eq!(
      normalize_domain("ads.example.com ! adblock note"),
      Some("ads.example.com".into())
    );
    // A sink IP followed only by a comment has no domain left.
    assert_eq!(normalize_domain("0.0.0.0 # just a comment"), None);
    // Full-line comments stay rejected.
    assert_eq!(normalize_domain("! adblock header"), None);
  }

  #[test]
  fn test_custom_level_roundtrip() {
    assert_eq!(
      BlocklistLevel::parse_level("custom"),
      Some(BlocklistLevel::Custom)
    );
    assert_eq!(BlocklistLevel::Custom.as_str(), "custom");
    assert_eq!(BlocklistLevel::Custom.filename(), Some("custom.txt"));
    assert!(BlocklistLevel::Custom.url().is_none());
    // Custom must NOT be in the auto-refresh set (no upstream URL).
    assert!(!BlocklistLevel::all_downloadable().contains(&BlocklistLevel::Custom));
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

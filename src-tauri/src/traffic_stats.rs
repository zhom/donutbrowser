use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Individual bandwidth data point for time-series tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandwidthDataPoint {
  /// Unix timestamp in seconds
  pub timestamp: u64,
  /// Bytes sent in this interval
  pub bytes_sent: u64,
  /// Bytes received in this interval
  pub bytes_received: u64,
}

/// Domain access information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainAccess {
  /// Domain name
  pub domain: String,
  /// Number of requests to this domain
  pub request_count: u64,
  /// Total bytes sent to this domain
  pub bytes_sent: u64,
  /// Total bytes received from this domain
  pub bytes_received: u64,
  /// First access timestamp
  pub first_access: u64,
  /// Last access timestamp
  pub last_access: u64,
}

/// Lightweight snapshot for real-time updates (sent via events)
/// Contains only the data needed for the mini chart and summary display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficSnapshot {
  /// Profile ID (for matching)
  pub profile_id: Option<String>,
  /// Session start timestamp
  pub session_start: u64,
  /// Last update timestamp
  pub last_update: u64,
  /// Total bytes sent across all time
  pub total_bytes_sent: u64,
  /// Total bytes received across all time
  pub total_bytes_received: u64,
  /// Total requests made
  pub total_requests: u64,
  /// Current bandwidth (bytes per second) sent
  pub current_bytes_sent: u64,
  /// Current bandwidth (bytes per second) received
  pub current_bytes_received: u64,
  /// Recent bandwidth history (last 60 seconds only, for mini chart)
  pub recent_bandwidth: Vec<BandwidthDataPoint>,
}

/// Traffic statistics for a profile/proxy session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficStats {
  /// Proxy ID this stats belong to (for backwards compatibility)
  pub proxy_id: String,
  /// Profile ID (if associated) - this is now the primary key for storage
  pub profile_id: Option<String>,
  /// Session start timestamp
  pub session_start: u64,
  /// Last update timestamp
  pub last_update: u64,
  /// Total bytes sent across all time
  pub total_bytes_sent: u64,
  /// Total bytes received across all time
  pub total_bytes_received: u64,
  /// Total requests made
  pub total_requests: u64,
  /// Bandwidth data points (time-series, 1 point per second, stored indefinitely)
  #[serde(default)]
  pub bandwidth_history: Vec<BandwidthDataPoint>,
  /// Domain access statistics
  #[serde(default)]
  pub domains: HashMap<String, DomainAccess>,
  /// Unique IPs accessed
  #[serde(default)]
  pub unique_ips: Vec<String>,
}

impl TrafficStats {
  pub fn new(proxy_id: String, profile_id: Option<String>) -> Self {
    let now = current_timestamp();
    Self {
      proxy_id,
      profile_id,
      session_start: now,
      last_update: now,
      total_bytes_sent: 0,
      total_bytes_received: 0,
      total_requests: 0,
      bandwidth_history: Vec::new(),
      domains: HashMap::new(),
      unique_ips: Vec::new(),
    }
  }

  /// Create a lightweight snapshot for real-time UI updates
  pub fn to_snapshot(&self) -> TrafficSnapshot {
    let now = current_timestamp();
    let cutoff = now.saturating_sub(60); // Last 60 seconds for mini chart

    // Get current bandwidth from last data point
    let (current_sent, current_recv) = self
      .bandwidth_history
      .last()
      .filter(|dp| dp.timestamp >= now.saturating_sub(2)) // Within last 2 seconds
      .map(|dp| (dp.bytes_sent, dp.bytes_received))
      .unwrap_or((0, 0));

    TrafficSnapshot {
      profile_id: self.profile_id.clone(),
      session_start: self.session_start,
      last_update: self.last_update,
      total_bytes_sent: self.total_bytes_sent,
      total_bytes_received: self.total_bytes_received,
      total_requests: self.total_requests,
      current_bytes_sent: current_sent,
      current_bytes_received: current_recv,
      recent_bandwidth: self
        .bandwidth_history
        .iter()
        .filter(|dp| dp.timestamp >= cutoff)
        .cloned()
        .collect(),
    }
  }

  /// Record bandwidth for current second (data is stored indefinitely)
  pub fn record_bandwidth(&mut self, bytes_sent: u64, bytes_received: u64) {
    let now = current_timestamp();
    self.last_update = now;
    self.total_bytes_sent += bytes_sent;
    self.total_bytes_received += bytes_received;

    // Find or create data point for this second
    if let Some(last) = self.bandwidth_history.last_mut() {
      if last.timestamp == now {
        last.bytes_sent += bytes_sent;
        last.bytes_received += bytes_received;
        return;
      }
    }

    // Add new data point (even if bytes are zero, to ensure chart has continuous data)
    self.bandwidth_history.push(BandwidthDataPoint {
      timestamp: now,
      bytes_sent,
      bytes_received,
    });
  }

  /// Record a request to a domain
  pub fn record_request(&mut self, domain: &str, bytes_sent: u64, bytes_received: u64) {
    let now = current_timestamp();
    self.total_requests += 1;

    let entry = self
      .domains
      .entry(domain.to_string())
      .or_insert(DomainAccess {
        domain: domain.to_string(),
        request_count: 0,
        bytes_sent: 0,
        bytes_received: 0,
        first_access: now,
        last_access: now,
      });

    entry.request_count += 1;
    entry.bytes_sent += bytes_sent;
    entry.bytes_received += bytes_received;
    entry.last_access = now;
  }

  /// Record an IP address access
  pub fn record_ip(&mut self, ip: &str) {
    if !self.unique_ips.contains(&ip.to_string()) {
      self.unique_ips.push(ip.to_string());
    }
  }

  /// Get bandwidth data for the last N seconds
  pub fn get_recent_bandwidth(&self, seconds: u64) -> Vec<BandwidthDataPoint> {
    let now = current_timestamp();
    let cutoff = now.saturating_sub(seconds);
    self
      .bandwidth_history
      .iter()
      .filter(|dp| dp.timestamp >= cutoff)
      .cloned()
      .collect()
  }
}

/// Get current Unix timestamp in seconds
fn current_timestamp() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

/// Get the traffic stats storage directory
pub fn get_traffic_stats_dir() -> PathBuf {
  let base_dirs = BaseDirs::new().expect("Failed to get base directories");
  let mut path = base_dirs.cache_dir().to_path_buf();
  path.push(if cfg!(debug_assertions) {
    "DonutBrowserDev"
  } else {
    "DonutBrowser"
  });
  path.push("traffic_stats");
  path
}

/// Get the storage key for traffic stats (profile_id if available, otherwise proxy_id)
fn get_stats_storage_key(stats: &TrafficStats) -> String {
  stats
    .profile_id
    .clone()
    .unwrap_or_else(|| stats.proxy_id.clone())
}

/// Save traffic stats to disk using profile_id as the key
pub fn save_traffic_stats(stats: &TrafficStats) -> Result<(), Box<dyn std::error::Error>> {
  let storage_dir = get_traffic_stats_dir();
  fs::create_dir_all(&storage_dir)?;

  let key = get_stats_storage_key(stats);
  let file_path = storage_dir.join(format!("{key}.json"));
  let content = serde_json::to_string(stats)?;
  fs::write(&file_path, content)?;

  Ok(())
}

/// Load traffic stats from disk by profile_id or proxy_id
pub fn load_traffic_stats(id: &str) -> Option<TrafficStats> {
  let storage_dir = get_traffic_stats_dir();
  let file_path = storage_dir.join(format!("{id}.json"));

  if !file_path.exists() {
    return None;
  }

  let content = fs::read_to_string(&file_path).ok()?;
  serde_json::from_str(&content).ok()
}

/// Load traffic stats by profile_id
pub fn load_traffic_stats_by_profile(profile_id: &str) -> Option<TrafficStats> {
  load_traffic_stats(profile_id)
}

/// List all traffic stats files and migrate old proxy-id based files to profile-id based
pub fn list_traffic_stats() -> Vec<TrafficStats> {
  let storage_dir = get_traffic_stats_dir();

  if !storage_dir.exists() {
    return Vec::new();
  }

  let mut stats_map: HashMap<String, TrafficStats> = HashMap::new();
  let mut files_to_delete: Vec<std::path::PathBuf> = Vec::new();

  if let Ok(entries) = fs::read_dir(&storage_dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().is_some_and(|ext| ext == "json") {
        if let Ok(content) = fs::read_to_string(&path) {
          if let Ok(s) = serde_json::from_str::<TrafficStats>(&content) {
            // Determine the key for this stats entry
            let key = s.profile_id.clone().unwrap_or_else(|| s.proxy_id.clone());

            // Check if this is an old proxy-id based file that should be migrated
            let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let is_old_proxy_file = file_stem.starts_with("proxy_")
              && s.profile_id.is_some()
              && file_stem != s.profile_id.as_ref().unwrap();

            if let Some(existing) = stats_map.get_mut(&key) {
              // Merge stats from this file into existing
              merge_traffic_stats(existing, &s);
              if is_old_proxy_file {
                files_to_delete.push(path.clone());
              }
            } else {
              stats_map.insert(key.clone(), s);
              if is_old_proxy_file {
                files_to_delete.push(path.clone());
              }
            }
          }
        }
      }
    }
  }

  // Save merged stats and delete old files
  for stats in stats_map.values() {
    if let Err(e) = save_traffic_stats(stats) {
      log::warn!("Failed to save merged traffic stats: {}", e);
    }
  }

  for path in files_to_delete {
    if let Err(e) = fs::remove_file(&path) {
      log::warn!("Failed to delete old traffic stats file {:?}: {}", path, e);
    }
  }

  stats_map.into_values().collect()
}

/// Merge traffic stats from source into destination
fn merge_traffic_stats(dest: &mut TrafficStats, src: &TrafficStats) {
  // Update totals
  dest.total_bytes_sent += src.total_bytes_sent;
  dest.total_bytes_received += src.total_bytes_received;
  dest.total_requests += src.total_requests;

  // Update timestamps
  dest.session_start = dest.session_start.min(src.session_start);
  dest.last_update = dest.last_update.max(src.last_update);

  // Merge bandwidth history (keep all data, sorted by timestamp)
  let mut combined_history: Vec<BandwidthDataPoint> = dest.bandwidth_history.clone();
  for point in &src.bandwidth_history {
    if !combined_history
      .iter()
      .any(|p| p.timestamp == point.timestamp)
    {
      combined_history.push(point.clone());
    }
  }
  combined_history.sort_by_key(|p| p.timestamp);
  dest.bandwidth_history = combined_history;

  // Merge domains
  for (domain, access) in &src.domains {
    let entry = dest.domains.entry(domain.clone()).or_insert(DomainAccess {
      domain: domain.clone(),
      request_count: 0,
      bytes_sent: 0,
      bytes_received: 0,
      first_access: access.first_access,
      last_access: access.last_access,
    });
    entry.request_count += access.request_count;
    entry.bytes_sent += access.bytes_sent;
    entry.bytes_received += access.bytes_received;
    entry.first_access = entry.first_access.min(access.first_access);
    entry.last_access = entry.last_access.max(access.last_access);
  }

  // Merge unique IPs
  for ip in &src.unique_ips {
    if !dest.unique_ips.contains(ip) {
      dest.unique_ips.push(ip.clone());
    }
  }
}

/// Delete traffic stats by id (profile_id or proxy_id)
pub fn delete_traffic_stats(id: &str) -> bool {
  let storage_dir = get_traffic_stats_dir();
  let file_path = storage_dir.join(format!("{id}.json"));

  if file_path.exists() {
    fs::remove_file(&file_path).is_ok()
  } else {
    false
  }
}

/// Clear all traffic stats (used when clearing cache)
pub fn clear_all_traffic_stats() -> Result<(), Box<dyn std::error::Error>> {
  let storage_dir = get_traffic_stats_dir();

  if storage_dir.exists() {
    for entry in fs::read_dir(&storage_dir)?.flatten() {
      let path = entry.path();
      if path.extension().is_some_and(|ext| ext == "json") {
        let _ = fs::remove_file(&path);
      }
    }
  }

  Ok(())
}

/// Live bandwidth tracker for real-time stats collection in the proxy
/// This is designed to be used from within the proxy server
pub struct LiveTrafficTracker {
  pub proxy_id: String,
  pub profile_id: Option<String>,
  bytes_sent: AtomicU64,
  bytes_received: AtomicU64,
  requests: AtomicU64,
  domain_stats: RwLock<HashMap<String, (u64, u64, u64)>>, // domain -> (count, sent, recv)
  ips: RwLock<Vec<String>>,
  #[allow(dead_code)]
  session_start: u64,
}

impl LiveTrafficTracker {
  pub fn new(proxy_id: String, profile_id: Option<String>) -> Self {
    Self {
      proxy_id,
      profile_id,
      bytes_sent: AtomicU64::new(0),
      bytes_received: AtomicU64::new(0),
      requests: AtomicU64::new(0),
      domain_stats: RwLock::new(HashMap::new()),
      ips: RwLock::new(Vec::new()),
      session_start: current_timestamp(),
    }
  }

  pub fn add_bytes_sent(&self, bytes: u64) {
    self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
  }

  pub fn add_bytes_received(&self, bytes: u64) {
    self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
  }

  pub fn record_request(&self, domain: &str, bytes_sent: u64, bytes_received: u64) {
    self.requests.fetch_add(1, Ordering::Relaxed);
    // Also update total byte counters for HTTP requests (not tunneled)
    self.bytes_sent.fetch_add(bytes_sent, Ordering::Relaxed);
    self
      .bytes_received
      .fetch_add(bytes_received, Ordering::Relaxed);
    if let Ok(mut stats) = self.domain_stats.write() {
      let entry = stats.entry(domain.to_string()).or_insert((0, 0, 0));
      entry.0 += 1;
      entry.1 += bytes_sent;
      entry.2 += bytes_received;
    }
  }

  pub fn record_ip(&self, ip: &str) {
    if let Ok(mut ips) = self.ips.write() {
      if !ips.contains(&ip.to_string()) {
        ips.push(ip.to_string());
      }
    }
  }

  /// Update domain-specific byte counts (called when CONNECT tunnel closes)
  pub fn update_domain_bytes(&self, domain: &str, bytes_sent: u64, bytes_received: u64) {
    if let Ok(mut stats) = self.domain_stats.write() {
      let entry = stats.entry(domain.to_string()).or_insert((0, 0, 0));
      entry.1 += bytes_sent;
      entry.2 += bytes_received;
    }
  }

  /// Get current stats snapshot
  pub fn get_snapshot(&self) -> (u64, u64, u64) {
    (
      self.bytes_sent.load(Ordering::Relaxed),
      self.bytes_received.load(Ordering::Relaxed),
      self.requests.load(Ordering::Relaxed),
    )
  }

  /// Flush current stats to disk and return the delta
  pub fn flush_to_disk(&self) -> Result<(u64, u64), Box<dyn std::error::Error>> {
    let bytes_sent = self.bytes_sent.swap(0, Ordering::Relaxed);
    let bytes_received = self.bytes_received.swap(0, Ordering::Relaxed);

    // Use profile_id as storage key if available, otherwise fall back to proxy_id
    let storage_key = self
      .profile_id
      .clone()
      .unwrap_or_else(|| self.proxy_id.clone());

    // Load or create stats using the storage key
    let mut stats = load_traffic_stats(&storage_key)
      .unwrap_or_else(|| TrafficStats::new(self.proxy_id.clone(), self.profile_id.clone()));

    // Ensure profile_id is set (in case stats were loaded from disk without it)
    if stats.profile_id.is_none() && self.profile_id.is_some() {
      stats.profile_id = self.profile_id.clone();
    }

    // Update the proxy_id to current session (for debugging/tracking)
    stats.proxy_id = self.proxy_id.clone();

    // Update bandwidth history
    stats.record_bandwidth(bytes_sent, bytes_received);

    // Update domain stats
    if let Ok(mut domain_map) = self.domain_stats.write() {
      for (domain, (count, sent, recv)) in domain_map.drain() {
        stats.record_request(&domain, sent, recv);
        // Adjust request count (record_request increments total_requests)
        stats.total_requests = stats.total_requests.saturating_sub(1) + count;
      }
    }

    // Update IPs
    if let Ok(ips) = self.ips.read() {
      for ip in ips.iter() {
        stats.record_ip(ip);
      }
    }

    // Save to disk
    save_traffic_stats(&stats)?;

    Ok((bytes_sent, bytes_received))
  }
}

/// Global traffic tracker that can be accessed from connection handlers
/// Using RwLock to allow reinitialization when proxy config changes
static TRAFFIC_TRACKER: std::sync::RwLock<Option<Arc<LiveTrafficTracker>>> =
  std::sync::RwLock::new(None);

/// Initialize the global traffic tracker
/// This can be called multiple times to update the tracker when proxy config changes
pub fn init_traffic_tracker(proxy_id: String, profile_id: Option<String>) {
  let tracker = Arc::new(LiveTrafficTracker::new(proxy_id, profile_id));
  if let Ok(mut guard) = TRAFFIC_TRACKER.write() {
    *guard = Some(tracker);
  }
}

/// Get the global traffic tracker
pub fn get_traffic_tracker() -> Option<Arc<LiveTrafficTracker>> {
  TRAFFIC_TRACKER.read().ok().and_then(|guard| guard.clone())
}

/// Filtered traffic stats for client display (only contains data for requested period)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilteredTrafficStats {
  pub profile_id: Option<String>,
  pub session_start: u64,
  pub last_update: u64,
  pub total_bytes_sent: u64,
  pub total_bytes_received: u64,
  pub total_requests: u64,
  /// Bandwidth history filtered to requested time period
  pub bandwidth_history: Vec<BandwidthDataPoint>,
  /// Period stats: bytes sent/received within the requested period
  pub period_bytes_sent: u64,
  pub period_bytes_received: u64,
  /// Domain access statistics (always full, as it's already aggregated)
  pub domains: HashMap<String, DomainAccess>,
  /// Unique IPs accessed
  pub unique_ips: Vec<String>,
}

/// Get traffic stats for a profile, filtered to a specific time period
/// seconds: number of seconds to include (0 = all time)
pub fn get_traffic_stats_for_period(
  profile_id: &str,
  seconds: u64,
) -> Option<FilteredTrafficStats> {
  let stats = load_traffic_stats(profile_id)?;

  let now = current_timestamp();
  let cutoff = if seconds == 0 {
    0 // All time
  } else {
    now.saturating_sub(seconds)
  };

  // Filter bandwidth history to requested period
  let filtered_history: Vec<BandwidthDataPoint> = stats
    .bandwidth_history
    .iter()
    .filter(|dp| dp.timestamp >= cutoff)
    .cloned()
    .collect();

  // Calculate period totals
  let period_bytes_sent: u64 = filtered_history.iter().map(|dp| dp.bytes_sent).sum();
  let period_bytes_received: u64 = filtered_history.iter().map(|dp| dp.bytes_received).sum();

  Some(FilteredTrafficStats {
    profile_id: stats.profile_id,
    session_start: stats.session_start,
    last_update: stats.last_update,
    total_bytes_sent: stats.total_bytes_sent,
    total_bytes_received: stats.total_bytes_received,
    total_requests: stats.total_requests,
    bandwidth_history: filtered_history,
    period_bytes_sent,
    period_bytes_received,
    domains: stats.domains,
    unique_ips: stats.unique_ips,
  })
}

/// Get lightweight traffic snapshot for a profile (for mini charts, only recent 60 seconds)
pub fn get_traffic_snapshot_for_profile(profile_id: &str) -> Option<TrafficSnapshot> {
  let stats = load_traffic_stats(profile_id)?;
  Some(stats.to_snapshot())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_traffic_stats_creation() {
    let stats = TrafficStats::new(
      "test_proxy".to_string(),
      Some("test-profile-id".to_string()),
    );
    assert_eq!(stats.proxy_id, "test_proxy");
    assert_eq!(stats.profile_id, Some("test-profile-id".to_string()));
    assert_eq!(stats.total_bytes_sent, 0);
    assert_eq!(stats.total_bytes_received, 0);
  }

  #[test]
  fn test_bandwidth_recording() {
    let mut stats = TrafficStats::new("test_proxy".to_string(), None);

    stats.record_bandwidth(1000, 2000);
    assert_eq!(stats.total_bytes_sent, 1000);
    assert_eq!(stats.total_bytes_received, 2000);
    assert_eq!(stats.bandwidth_history.len(), 1);

    stats.record_bandwidth(500, 1000);
    assert_eq!(stats.total_bytes_sent, 1500);
    assert_eq!(stats.total_bytes_received, 3000);
  }

  #[test]
  fn test_domain_recording() {
    let mut stats = TrafficStats::new("test_proxy".to_string(), None);

    stats.record_request("example.com", 100, 500);
    stats.record_request("example.com", 200, 1000);
    stats.record_request("google.com", 50, 200);

    assert_eq!(stats.domains.len(), 2);
    assert_eq!(stats.domains["example.com"].request_count, 2);
    assert_eq!(stats.domains["example.com"].bytes_sent, 300);
    assert_eq!(stats.domains["google.com"].request_count, 1);
  }

  #[test]
  fn test_ip_recording() {
    let mut stats = TrafficStats::new("test_proxy".to_string(), None);

    stats.record_ip("192.168.1.1");
    stats.record_ip("192.168.1.1"); // Duplicate
    stats.record_ip("10.0.0.1");

    assert_eq!(stats.unique_ips.len(), 2);
  }
}

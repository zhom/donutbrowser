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
  /// Proxy ID this stats belong to
  pub proxy_id: String,
  /// Profile ID (if associated)
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
  /// Bandwidth data points (time-series, 1 point per second, max 300 = 5 min)
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

  /// Record bandwidth for current second
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

    // Keep only last 5 minutes (300 seconds) of data
    const MAX_HISTORY_SECONDS: usize = 300;
    if self.bandwidth_history.len() > MAX_HISTORY_SECONDS {
      self.bandwidth_history.remove(0);
    }
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

/// Save traffic stats to disk
pub fn save_traffic_stats(stats: &TrafficStats) -> Result<(), Box<dyn std::error::Error>> {
  let storage_dir = get_traffic_stats_dir();
  fs::create_dir_all(&storage_dir)?;

  let file_path = storage_dir.join(format!("{}.json", stats.proxy_id));
  let content = serde_json::to_string(stats)?;
  fs::write(&file_path, content)?;

  Ok(())
}

/// Load traffic stats from disk
pub fn load_traffic_stats(proxy_id: &str) -> Option<TrafficStats> {
  let storage_dir = get_traffic_stats_dir();
  let file_path = storage_dir.join(format!("{proxy_id}.json"));

  if !file_path.exists() {
    return None;
  }

  let content = fs::read_to_string(&file_path).ok()?;
  serde_json::from_str(&content).ok()
}

/// List all traffic stats files
pub fn list_traffic_stats() -> Vec<TrafficStats> {
  let storage_dir = get_traffic_stats_dir();

  if !storage_dir.exists() {
    return Vec::new();
  }

  let mut stats = Vec::new();
  if let Ok(entries) = fs::read_dir(&storage_dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().is_some_and(|ext| ext == "json") {
        if let Ok(content) = fs::read_to_string(&path) {
          if let Ok(s) = serde_json::from_str::<TrafficStats>(&content) {
            stats.push(s);
          }
        }
      }
    }
  }

  stats
}

/// Delete traffic stats for a proxy
pub fn delete_traffic_stats(proxy_id: &str) -> bool {
  let storage_dir = get_traffic_stats_dir();
  let file_path = storage_dir.join(format!("{proxy_id}.json"));

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

    // Load or create stats
    let mut stats = load_traffic_stats(&self.proxy_id)
      .unwrap_or_else(|| TrafficStats::new(self.proxy_id.clone(), self.profile_id.clone()));

    // Ensure profile_id is set (in case stats were loaded from disk without it)
    if stats.profile_id.is_none() && self.profile_id.is_some() {
      stats.profile_id = self.profile_id.clone();
    }

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

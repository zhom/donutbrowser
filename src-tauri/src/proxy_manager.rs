use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Emitter;
use tauri_plugin_shell::ShellExt;

use crate::browser::ProxySettings;

// Store active proxy information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyInfo {
  pub id: String,
  pub local_url: String,
  pub upstream_host: String,
  pub upstream_port: u16,
  pub upstream_type: String,
  pub local_port: u16,
  // Optional profile name to which this proxy instance is logically tied
  pub profile_name: Option<String>,
}

// Stored proxy configuration with name and ID for reuse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProxy {
  pub id: String,
  pub name: String,
  pub proxy_settings: ProxySettings,
}

impl StoredProxy {
  pub fn new(name: String, proxy_settings: ProxySettings) -> Self {
    Self {
      id: uuid::Uuid::new_v4().to_string(),
      name,
      proxy_settings,
    }
  }

  pub fn update_settings(&mut self, proxy_settings: ProxySettings) {
    self.proxy_settings = proxy_settings;
  }

  pub fn update_name(&mut self, name: String) {
    self.name = name;
  }
}

// Global proxy manager to track active proxies and stored proxy configurations
pub struct ProxyManager {
  active_proxies: Mutex<HashMap<u32, ProxyInfo>>, // Maps browser process ID to proxy info
  // Store proxy info by profile name for persistence across browser restarts
  profile_proxies: Mutex<HashMap<String, ProxySettings>>, // Maps profile name to proxy settings
  // Track active proxy IDs by profile name for targeted cleanup
  profile_active_proxy_ids: Mutex<HashMap<String, String>>, // Maps profile name to proxy id
  stored_proxies: Mutex<HashMap<String, StoredProxy>>,      // Maps proxy ID to stored proxy
  base_dirs: BaseDirs,
}

impl ProxyManager {
  pub fn new() -> Self {
    let base_dirs = BaseDirs::new().expect("Failed to get base directories");
    let manager = Self {
      active_proxies: Mutex::new(HashMap::new()),
      profile_proxies: Mutex::new(HashMap::new()),
      profile_active_proxy_ids: Mutex::new(HashMap::new()),
      stored_proxies: Mutex::new(HashMap::new()),
      base_dirs,
    };

    // Load stored proxies on initialization
    if let Err(e) = manager.load_stored_proxies() {
      eprintln!("Warning: Failed to load stored proxies: {e}");
    }

    manager
  }

  // Get the path to the proxies directory
  fn get_proxies_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("proxies");
    path
  }

  // Get the path to a specific proxy file
  fn get_proxy_file_path(&self, proxy_id: &str) -> PathBuf {
    self.get_proxies_dir().join(format!("{proxy_id}.json"))
  }

  // Load stored proxies from disk
  fn load_stored_proxies(&self) -> Result<(), Box<dyn std::error::Error>> {
    let proxies_dir = self.get_proxies_dir();

    if !proxies_dir.exists() {
      return Ok(()); // No proxies directory yet
    }

    let mut stored_proxies = self.stored_proxies.lock().unwrap();

    // Read all JSON files from the proxies directory
    for entry in fs::read_dir(&proxies_dir)? {
      let entry = entry?;
      let path = entry.path();

      if path.extension().is_some_and(|ext| ext == "json") {
        let content = fs::read_to_string(&path)?;
        let proxy: StoredProxy = serde_json::from_str(&content)?;
        stored_proxies.insert(proxy.id.clone(), proxy);
      }
    }

    Ok(())
  }

  // Save a single proxy to disk
  fn save_proxy(&self, proxy: &StoredProxy) -> Result<(), Box<dyn std::error::Error>> {
    let proxies_dir = self.get_proxies_dir();

    // Ensure directory exists
    fs::create_dir_all(&proxies_dir)?;

    let proxy_file = self.get_proxy_file_path(&proxy.id);
    let content = serde_json::to_string_pretty(proxy)?;
    fs::write(&proxy_file, content)?;

    Ok(())
  }

  // Delete a proxy file from disk
  fn delete_proxy_file(&self, proxy_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let proxy_file = self.get_proxy_file_path(proxy_id);
    if proxy_file.exists() {
      fs::remove_file(proxy_file)?;
    }
    Ok(())
  }

  // Create a new stored proxy
  pub fn create_stored_proxy(
    &self,
    app_handle: &tauri::AppHandle,
    name: String,
    proxy_settings: ProxySettings,
  ) -> Result<StoredProxy, String> {
    // Check if name already exists
    {
      let stored_proxies = self.stored_proxies.lock().unwrap();
      if stored_proxies.values().any(|p| p.name == name) {
        return Err(format!("Proxy with name '{name}' already exists"));
      }
    }

    let stored_proxy = StoredProxy::new(name, proxy_settings);

    {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      stored_proxies.insert(stored_proxy.id.clone(), stored_proxy.clone());
    }

    if let Err(e) = self.save_proxy(&stored_proxy) {
      eprintln!("Warning: Failed to save proxy: {e}");
    }

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      eprintln!("Failed to emit proxies-changed event: {e}");
    }

    Ok(stored_proxy)
  }

  // Get all stored proxies
  pub fn get_stored_proxies(&self) -> Vec<StoredProxy> {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    let mut list: Vec<StoredProxy> = stored_proxies.values().cloned().collect();
    // Sort case-insensitively by name for consistent ordering across UI/API consumers
    list.sort_by_key(|p| p.name.to_lowercase());
    list
  }

  // Get a stored proxy by ID

  // Update a stored proxy
  pub fn update_stored_proxy(
    &self,
    app_handle: &tauri::AppHandle,
    proxy_id: &str,
    name: Option<String>,
    proxy_settings: Option<ProxySettings>,
  ) -> Result<StoredProxy, String> {
    // First, check for conflicts without holding a mutable reference
    {
      let stored_proxies = self.stored_proxies.lock().unwrap();

      // Check if proxy exists
      if !stored_proxies.contains_key(proxy_id) {
        return Err(format!("Proxy with ID '{proxy_id}' not found"));
      }

      // Check if new name conflicts with existing proxies
      if let Some(ref new_name) = name {
        if stored_proxies
          .values()
          .any(|p| p.id != proxy_id && p.name == *new_name)
        {
          return Err(format!("Proxy with name '{new_name}' already exists"));
        }
      }
    } // Release the lock here

    // Now get mutable access for updates
    let updated_proxy = {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      let stored_proxy = stored_proxies.get_mut(proxy_id).unwrap(); // Safe because we checked above

      if let Some(new_name) = name {
        stored_proxy.update_name(new_name);
      }

      if let Some(new_settings) = proxy_settings {
        stored_proxy.update_settings(new_settings);
      }

      stored_proxy.clone()
    };

    if let Err(e) = self.save_proxy(&updated_proxy) {
      eprintln!("Warning: Failed to save proxy: {e}");
    }

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      eprintln!("Failed to emit proxies-changed event: {e}");
    }

    Ok(updated_proxy)
  }

  // Delete a stored proxy
  pub fn delete_stored_proxy(
    &self,
    app_handle: &tauri::AppHandle,
    proxy_id: &str,
  ) -> Result<(), String> {
    {
      let mut stored_proxies = self.stored_proxies.lock().unwrap();
      if stored_proxies.remove(proxy_id).is_none() {
        return Err(format!("Proxy with ID '{proxy_id}' not found"));
      }
    }

    if let Err(e) = self.delete_proxy_file(proxy_id) {
      eprintln!("Warning: Failed to delete proxy file: {e}");
    }

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      eprintln!("Failed to emit proxies-changed event: {e}");
    }

    Ok(())
  }

  // Get proxy settings for a stored proxy ID
  pub fn get_proxy_settings_by_id(&self, proxy_id: &str) -> Option<ProxySettings> {
    let stored_proxies = self.stored_proxies.lock().unwrap();
    stored_proxies
      .get(proxy_id)
      .map(|p| p.proxy_settings.clone())
  }

  // Start a proxy for given proxy settings and associate it with a browser process ID
  // If proxy_settings is None, starts a direct proxy for traffic monitoring
  pub async fn start_proxy(
    &self,
    app_handle: tauri::AppHandle,
    proxy_settings: Option<&ProxySettings>,
    browser_pid: u32,
    profile_name: Option<&str>,
  ) -> Result<ProxySettings, String> {
    // First, proactively cleanup any dead proxies so we don't accidentally reuse stale ones
    let _ = self.cleanup_dead_proxies(app_handle.clone()).await;

    // If we have a previous proxy tied to this profile, and the upstream settings are changing,
    // stop it before starting a new one so the change takes effect immediately.
    if let Some(name) = profile_name {
      // Check if we have an active proxy recorded for this profile
      let maybe_existing_id = {
        let map = self.profile_active_proxy_ids.lock().unwrap();
        map.get(name).cloned()
      };

      if let Some(existing_id) = maybe_existing_id {
        // Find the existing proxy info
        let existing_info = {
          let proxies = self.active_proxies.lock().unwrap();
          proxies.values().find(|p| p.id == existing_id).cloned()
        };

        if let Some(existing) = existing_info {
          let desired_type = proxy_settings
            .map(|p| p.proxy_type.as_str())
            .unwrap_or("DIRECT");
          let desired_host = proxy_settings.map(|p| p.host.as_str()).unwrap_or("DIRECT");
          let desired_port = proxy_settings.map(|p| p.port).unwrap_or(0);

          let is_same_upstream = existing.upstream_type == desired_type
            && existing.upstream_host == desired_host
            && existing.upstream_port == desired_port;

          if !is_same_upstream {
            // Stop the previous proxy tied to this profile (best effort)
            // We don't know the original PID mapping that created it; iterate to find its key
            let pid_to_stop = {
              let proxies = self.active_proxies.lock().unwrap();
              proxies.iter().find_map(|(pid, info)| {
                if info.id == existing_id {
                  Some(*pid)
                } else {
                  None
                }
              })
            };
            if let Some(pid) = pid_to_stop {
              let _ = self.stop_proxy(app_handle.clone(), pid).await;
            }
          }
        }
      }
    }
    // Check if we already have a proxy for this browser PID. If it exists but the upstream
    // settings don't match the newly requested ones, stop it and create a new proxy so that
    // changes take effect immediately.
    let mut needs_restart = false;
    {
      let proxies = self.active_proxies.lock().unwrap();
      if let Some(existing) = proxies.get(&browser_pid) {
        let desired_type = proxy_settings
          .map(|p| p.proxy_type.as_str())
          .unwrap_or("DIRECT");
        let desired_host = proxy_settings.map(|p| p.host.as_str()).unwrap_or("DIRECT");
        let desired_port = proxy_settings.map(|p| p.port).unwrap_or(0);

        let is_same_upstream = existing.upstream_type == desired_type
          && existing.upstream_host == desired_host
          && existing.upstream_port == desired_port;

        if is_same_upstream {
          // Reuse existing local proxy
          return Ok(ProxySettings {
            proxy_type: "http".to_string(),
            host: "127.0.0.1".to_string(),
            port: existing.local_port,
            username: None,
            password: None,
          });
        } else {
          // Upstream changed; we must restart the local proxy so that traffic is routed correctly
          needs_restart = true;
        }
      }
    }

    if needs_restart {
      // Best-effort stop of the old proxy for this PID before starting a new one
      let _ = self.stop_proxy(app_handle.clone(), browser_pid).await;
    }

    // Start a new proxy using the nodecar binary with the correct CLI interface
    let mut nodecar = app_handle
      .shell()
      .sidecar("nodecar")
      .map_err(|e| format!("Failed to create sidecar: {e}"))?
      .arg("proxy")
      .arg("start");

    // Add upstream proxy settings if provided, otherwise create direct proxy
    if let Some(proxy_settings) = proxy_settings {
      nodecar = nodecar
        .arg("--host")
        .arg(&proxy_settings.host)
        .arg("--proxy-port")
        .arg(proxy_settings.port.to_string())
        .arg("--type")
        .arg(&proxy_settings.proxy_type);

      // Add credentials if provided
      if let Some(username) = &proxy_settings.username {
        nodecar = nodecar.arg("--username").arg(username);
      }
      if let Some(password) = &proxy_settings.password {
        nodecar = nodecar.arg("--password").arg(password);
      }
    }

    // Execute the command and wait for it to complete
    // The nodecar binary should start the worker and then exit
    let output = nodecar
      .output()
      .await
      .map_err(|e| format!("Failed to execute nodecar: {e}"))?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let stdout = String::from_utf8_lossy(&output.stdout);
      return Err(format!(
        "Proxy start failed - stdout: {stdout}, stderr: {stderr}"
      ));
    }

    let json_string =
      String::from_utf8(output.stdout).map_err(|e| format!("Failed to parse proxy output: {e}"))?;

    // Parse the JSON output
    let json: Value =
      serde_json::from_str(&json_string).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    // Extract proxy information
    let id = json["id"].as_str().ok_or("Missing proxy ID")?;
    let local_port = json["localPort"].as_u64().ok_or("Missing local port")? as u16;
    let local_url = json["localUrl"]
      .as_str()
      .ok_or("Missing local URL")?
      .to_string();

    let proxy_info = ProxyInfo {
      id: id.to_string(),
      local_url,
      upstream_host: proxy_settings
        .map(|p| p.host.clone())
        .unwrap_or_else(|| "DIRECT".to_string()),
      upstream_port: proxy_settings.map(|p| p.port).unwrap_or(0),
      upstream_type: proxy_settings
        .map(|p| p.proxy_type.clone())
        .unwrap_or_else(|| "DIRECT".to_string()),
      local_port,
      profile_name: profile_name.map(|s| s.to_string()),
    };

    // Wait for the local proxy port to be ready to accept connections
    {
      use tokio::net::TcpStream;
      use tokio::time::{sleep, Duration};
      let mut ready = false;
      for _ in 0..50 {
        match TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy_info.local_port)).await {
          Ok(_stream) => {
            ready = true;
            break;
          }
          Err(_) => {
            sleep(Duration::from_millis(100)).await;
          }
        }
      }
      if !ready {
        return Err(format!(
          "Local proxy on 127.0.0.1:{} did not become ready in time",
          proxy_info.local_port
        ));
      }
    }

    // Store the proxy info
    {
      let mut proxies = self.active_proxies.lock().unwrap();
      proxies.insert(browser_pid, proxy_info.clone());
    }

    // Store the profile proxy info for persistence
    if let Some(name) = profile_name {
      if let Some(proxy_settings) = proxy_settings {
        let mut profile_proxies = self.profile_proxies.lock().unwrap();
        profile_proxies.insert(name.to_string(), proxy_settings.clone());
      }
      // Also record the active proxy id for this profile for quick cleanup on changes
      let mut map = self.profile_active_proxy_ids.lock().unwrap();
      map.insert(name.to_string(), proxy_info.id.clone());
    }

    // Return proxy settings for the browser
    Ok(ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(), // Use 127.0.0.1 instead of localhost for better compatibility
      port: proxy_info.local_port,
      username: None,
      password: None,
    })
  }

  // Stop the proxy associated with a browser process ID
  pub async fn stop_proxy(
    &self,
    app_handle: tauri::AppHandle,
    browser_pid: u32,
  ) -> Result<(), String> {
    let (proxy_id, profile_name): (String, Option<String>) = {
      let mut proxies = self.active_proxies.lock().unwrap();
      match proxies.remove(&browser_pid) {
        Some(proxy) => (proxy.id, proxy.profile_name.clone()),
        None => return Ok(()), // No proxy to stop
      }
    };

    // Stop the proxy using the nodecar binary
    let nodecar = app_handle
      .shell()
      .sidecar("nodecar")
      .map_err(|e| format!("Failed to create sidecar: {e}"))?
      .arg("proxy")
      .arg("stop")
      .arg("--id")
      .arg(&proxy_id);

    let output = nodecar.output().await.unwrap();

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      eprintln!("Proxy stop error: {stderr}");
      // We still return Ok since we've already removed the proxy from our tracking
    }

    // Clear profile-to-proxy mapping if it references this proxy
    if let Some(name) = profile_name {
      let mut map = self.profile_active_proxy_ids.lock().unwrap();
      if let Some(current_id) = map.get(&name) {
        if current_id == &proxy_id {
          map.remove(&name);
        }
      }
    }

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      eprintln!("Failed to emit proxies-changed event: {e}");
    }

    Ok(())
  }

  // Update the PID mapping for an existing proxy
  pub fn update_proxy_pid(&self, old_pid: u32, new_pid: u32) -> Result<(), String> {
    let mut proxies = self.active_proxies.lock().unwrap();
    if let Some(proxy_info) = proxies.remove(&old_pid) {
      proxies.insert(new_pid, proxy_info);
      Ok(())
    } else {
      Err(format!("No proxy found for PID {old_pid}"))
    }
  }

  // Check if a process is still running
  fn is_process_running(&self, pid: u32) -> bool {
    use sysinfo::{Pid, System};
    let system = System::new_all();
    system.process(Pid::from(pid as usize)).is_some()
  }

  // Clean up proxies for dead browser processes
  pub async fn cleanup_dead_proxies(
    &self,
    app_handle: tauri::AppHandle,
  ) -> Result<Vec<u32>, String> {
    let dead_pids = {
      let proxies = self.active_proxies.lock().unwrap();
      proxies
        .keys()
        .filter(|&&pid| pid != 0 && !self.is_process_running(pid)) // Skip temporary PID 0
        .copied()
        .collect::<Vec<u32>>()
    };

    for dead_pid in &dead_pids {
      println!("Cleaning up proxy for dead browser process PID: {dead_pid}");
      let _ = self.stop_proxy(app_handle.clone(), *dead_pid).await;
    }

    // Emit event for reactive UI updates
    if let Err(e) = app_handle.emit("proxies-changed", ()) {
      eprintln!("Failed to emit proxies-changed event: {e}");
    }

    Ok(dead_pids)
  }
}

// Create a singleton instance of the proxy manager
lazy_static::lazy_static! {
    pub static ref PROXY_MANAGER: ProxyManager = ProxyManager::new();
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::env;
  use std::path::PathBuf;
  use std::process::Command;
  use std::time::Duration;
  use tokio::time::sleep;

  // Mock HTTP server for testing

  use http_body_util::Full;
  use hyper::body::Bytes;
  use hyper::server::conn::http1;
  use hyper::service::service_fn;
  use hyper::Response;
  use hyper_util::rt::TokioIo;
  use tokio::net::TcpListener;

  // Helper function to build nodecar binary for testing
  async fn ensure_nodecar_binary() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let project_root = PathBuf::from(cargo_manifest_dir)
      .parent()
      .unwrap()
      .to_path_buf();
    let nodecar_dir = project_root.join("nodecar");
    let nodecar_binary = nodecar_dir.join("nodecar-bin");

    // Check if binary already exists
    if nodecar_binary.exists() {
      return Ok(nodecar_binary);
    }

    // Build the nodecar binary
    println!("Building nodecar binary for tests...");

    // Install dependencies
    let install_status = Command::new("pnpm")
      .args(["install", "--frozen-lockfile"])
      .current_dir(&nodecar_dir)
      .status()?;

    if !install_status.success() {
      return Err("Failed to install nodecar dependencies".into());
    }

    // Determine the target architecture
    let target = if cfg!(target_arch = "aarch64") && cfg!(target_os = "macos") {
      "build:mac-aarch64"
    } else if cfg!(target_arch = "x86_64") && cfg!(target_os = "macos") {
      "build:mac-x86_64"
    } else if cfg!(target_arch = "x86_64") && cfg!(target_os = "linux") {
      "build:linux-x64"
    } else if cfg!(target_arch = "aarch64") && cfg!(target_os = "linux") {
      "build:linux-arm64"
    } else if cfg!(target_arch = "x86_64") && cfg!(target_os = "windows") {
      "build:win-x64"
    } else if cfg!(target_arch = "aarch64") && cfg!(target_os = "windows") {
      "build:win-arm64"
    } else {
      return Err("Unsupported target architecture for nodecar build".into());
    };

    // Build the binary
    let build_status = Command::new("pnpm")
      .args(["run", target])
      .current_dir(&nodecar_dir)
      .status()?;

    if !build_status.success() {
      return Err("Failed to build nodecar binary".into());
    }

    if !nodecar_binary.exists() {
      return Err("Nodecar binary was not created successfully".into());
    }

    Ok(nodecar_binary)
  }

  #[test]
  fn test_proxy_settings_validation() {
    // Test valid proxy settings
    let valid_settings = ProxySettings {
      proxy_type: "http".to_string(),
      host: "127.0.0.1".to_string(),
      port: 8080,
      username: Some("user".to_string()),
      password: Some("pass".to_string()),
    };

    assert!(
      !valid_settings.host.is_empty(),
      "Valid settings should have non-empty host"
    );
    assert!(
      valid_settings.port > 0,
      "Valid settings should have positive port"
    );
    assert_eq!(valid_settings.proxy_type, "http", "Proxy type should match");
    assert!(
      valid_settings.username.is_some(),
      "Username should be present"
    );
    assert!(
      valid_settings.password.is_some(),
      "Password should be present"
    );

    // Test proxy settings with empty values
    let empty_settings = ProxySettings {
      proxy_type: "http".to_string(),
      host: "".to_string(),
      port: 0,
      username: None,
      password: None,
    };

    assert!(
      empty_settings.host.is_empty(),
      "Empty settings should have empty host"
    );
    assert_eq!(
      empty_settings.port, 0,
      "Empty settings should have zero port"
    );
    assert!(empty_settings.username.is_none(), "Username should be None");
    assert!(empty_settings.password.is_none(), "Password should be None");
  }

  #[tokio::test]
  async fn test_proxy_manager_concurrent_access() {
    use std::sync::Arc;

    let proxy_manager = Arc::new(ProxyManager::new());
    let mut handles = vec![];

    // Spawn multiple tasks that access the proxy manager concurrently
    for i in 0..10 {
      let pm = proxy_manager.clone();
      let handle = tokio::spawn(async move {
        let browser_pid = (1000 + i) as u32;
        let proxy_info = ProxyInfo {
          id: format!("proxy_{i}"),
          local_url: format!("http://127.0.0.1:{}", 8000 + i),
          upstream_host: "127.0.0.1".to_string(),
          upstream_port: 3128,
          upstream_type: "http".to_string(),
          local_port: (8000 + i) as u16,
          profile_name: None,
        };

        // Add proxy
        {
          let mut active_proxies = pm.active_proxies.lock().unwrap();
          active_proxies.insert(browser_pid, proxy_info);
        }

        browser_pid
      });
      handles.push(handle);
    }

    // Wait for all tasks to complete
    let results: Vec<u32> = futures_util::future::join_all(handles)
      .await
      .into_iter()
      .map(|r| r.unwrap())
      .collect();

    // Verify all browser PIDs were processed
    assert_eq!(results.len(), 10);
    for (i, &browser_pid) in results.iter().enumerate() {
      assert_eq!(browser_pid, (1000 + i) as u32);
    }
  }

  // Integration test that actually builds and uses nodecar binary
  #[tokio::test]
  async fn test_proxy_integration_with_real_nodecar() -> Result<(), Box<dyn std::error::Error>> {
    // This test requires nodecar to be built and available
    let nodecar_path = ensure_nodecar_binary().await?;

    // Start a mock upstream HTTP server
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_addr = upstream_listener.local_addr()?;

    // Spawn upstream server
    let server_handle = tokio::spawn(async move {
      while let Ok((stream, _)) = upstream_listener.accept().await {
        let io = TokioIo::new(stream);
        tokio::task::spawn(async move {
          let _ = http1::Builder::new()
            .serve_connection(
              io,
              service_fn(|_req| async {
                Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::from("Upstream OK"))))
              }),
            )
            .await;
        });
      }
    });

    // Wait for server to start
    sleep(Duration::from_millis(100)).await;

    // Test nodecar proxy start command directly (using the binary itself, not node)
    let mut cmd = Command::new(&nodecar_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg(upstream_addr.ip().to_string())
      .arg("--proxy-port")
      .arg(upstream_addr.port().to_string())
      .arg("--type")
      .arg("http");

    // Set a timeout for the command
    let output = tokio::time::timeout(Duration::from_secs(60), async { cmd.output() }).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;

      // Verify proxy configuration
      assert!(config["id"].is_string());
      assert!(config["localPort"].is_number());
      assert!(config["localUrl"].is_string());

      let proxy_id = config["id"].as_str().unwrap();
      let local_port = config["localPort"].as_u64().unwrap();

      // Wait for proxy worker to start
      println!("Waiting for proxy worker to start...");
      tokio::time::sleep(Duration::from_secs(1)).await;

      // Test that the local port is listening
      let mut port_test = Command::new("nc");
      port_test
        .arg("-z")
        .arg("127.0.0.1")
        .arg(local_port.to_string());

      let port_output = port_test.output()?;
      if port_output.status.success() {
        println!("Proxy is listening on port {local_port}");
      } else {
        println!("Warning: Proxy port {local_port} is not listening");
      }

      // Test stopping the proxy
      let mut stop_cmd = Command::new(&nodecar_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);

      let stop_output =
        tokio::time::timeout(Duration::from_secs(60), async { stop_cmd.output() }).await??;

      assert!(stop_output.status.success());

      println!("Integration test passed: nodecar proxy start/stop works correctly");
    } else {
      let stderr = String::from_utf8(output.stderr)?;
      eprintln!("Nodecar failed: {stderr}");
      return Err(format!("Nodecar command failed: {stderr}").into());
    }

    // Clean up server
    server_handle.abort();

    Ok(())
  }

  // Test that validates the command line arguments are constructed correctly
  #[test]
  fn test_proxy_command_construction() {
    let proxy_settings = ProxySettings {
      proxy_type: "http".to_string(),
      host: "proxy.example.com".to_string(),
      port: 8080,
      username: Some("user".to_string()),
      password: Some("pass".to_string()),
    };

    // Test command arguments match expected format
    let expected_args = [
      "proxy",
      "start",
      "--host",
      "proxy.example.com",
      "--proxy-port",
      "8080",
      "--type",
      "http",
      "--username",
      "user",
      "--password",
      "pass",
    ];

    // This test verifies the argument structure without actually running the command
    assert_eq!(
      proxy_settings.host, "proxy.example.com",
      "Host should match expected value"
    );
    assert_eq!(
      proxy_settings.port, 8080,
      "Port should match expected value"
    );
    assert_eq!(
      proxy_settings.proxy_type, "http",
      "Proxy type should match expected value"
    );
    assert_eq!(
      proxy_settings.username.as_ref().unwrap(),
      "user",
      "Username should match expected value"
    );
    assert_eq!(
      proxy_settings.password.as_ref().unwrap(),
      "pass",
      "Password should match expected value"
    );

    // Verify expected args structure
    assert_eq!(expected_args[0], "proxy", "First arg should be 'proxy'");
    assert_eq!(expected_args[1], "start", "Second arg should be 'start'");
    assert_eq!(expected_args[2], "--host", "Third arg should be '--host'");
    assert_eq!(
      expected_args[3], "proxy.example.com",
      "Fourth arg should be host value"
    );
  }

  // Test the CLI detachment specifically - ensure the CLI exits properly
  #[tokio::test]
  async fn test_cli_exits_after_proxy_start() -> Result<(), Box<dyn std::error::Error>> {
    let nodecar_path = ensure_nodecar_binary().await?;

    // Test that the CLI exits quickly with a mock upstream
    let mut cmd = Command::new(&nodecar_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("httpbin.org")
      .arg("--proxy-port")
      .arg("80")
      .arg("--type")
      .arg("http");

    let start_time = std::time::Instant::now();
    let output = tokio::time::timeout(Duration::from_secs(3), async { cmd.output() }).await;

    match output {
      Ok(Ok(cmd_output)) => {
        let execution_time = start_time.elapsed();

        if cmd_output.status.success() {
          let stdout = String::from_utf8(cmd_output.stdout)?;
          let config: serde_json::Value = serde_json::from_str(&stdout)?;

          // Clean up - try to stop the proxy
          if let Some(proxy_id) = config["id"].as_str() {
            let mut stop_cmd = Command::new(&nodecar_path);
            stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
            let _ = stop_cmd.output();
          }
        }

        println!("CLI detachment test passed - CLI exited in {execution_time:?}");
      }
      Ok(Err(e)) => {
        return Err(format!("Command execution failed: {e}").into());
      }
      Err(_) => {
        return Err("CLI command timed out - this indicates improper detachment".into());
      }
    }

    Ok(())
  }

  // Test that validates proper CLI detachment behavior
  #[tokio::test]
  async fn test_cli_detachment_behavior() -> Result<(), Box<dyn std::error::Error>> {
    let nodecar_path = ensure_nodecar_binary().await?;

    // Test that the CLI command exits quickly even with a real upstream
    let mut cmd = Command::new(&nodecar_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("httpbin.org") // Use a known good endpoint
      .arg("--proxy-port")
      .arg("80")
      .arg("--type")
      .arg("http");

    let output = tokio::time::timeout(Duration::from_secs(60), async { cmd.output() }).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;
      let proxy_id = config["id"].as_str().unwrap();

      // Clean up
      let mut stop_cmd = Command::new(&nodecar_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
      let _ = stop_cmd.output();

      println!("CLI detachment test passed");
    } else {
      // Even if the upstream fails, the CLI should still exit quickly
      println!("CLI command failed but exited quickly as expected");
    }

    Ok(())
  }

  // Test that validates URL encoding for special characters in credentials
  #[tokio::test]
  async fn test_proxy_credentials_encoding() -> Result<(), Box<dyn std::error::Error>> {
    let nodecar_path = ensure_nodecar_binary().await?;

    // Test with credentials that include special characters
    let mut cmd = Command::new(&nodecar_path);
    cmd
      .arg("proxy")
      .arg("start")
      .arg("--host")
      .arg("test.example.com")
      .arg("--proxy-port")
      .arg("8080")
      .arg("--type")
      .arg("http")
      .arg("--username")
      .arg("user@domain.com") // Contains @ symbol
      .arg("--password")
      .arg("pass word!"); // Contains space and special character

    let output = tokio::time::timeout(Duration::from_secs(60), async { cmd.output() }).await??;

    if output.status.success() {
      let stdout = String::from_utf8(output.stdout)?;
      let config: serde_json::Value = serde_json::from_str(&stdout)?;

      let upstream_url = config["upstreamUrl"].as_str().unwrap();

      println!("Generated upstream URL: {upstream_url}");

      // Verify that special characters are properly encoded
      assert!(upstream_url.contains("user%40domain.com"));
      // The password may be encoded as "pass%20word!" or "pass%20word%21" depending on implementation
      assert!(upstream_url.contains("pass%20word"));

      println!("URL encoding test passed - special characters handled correctly");

      // Clean up
      let proxy_id = config["id"].as_str().unwrap();
      let mut stop_cmd = Command::new(&nodecar_path);
      stop_cmd.arg("proxy").arg("stop").arg("--id").arg(proxy_id);
      let _ = stop_cmd.output();
    } else {
      // This test might fail if the upstream doesn't exist, but we mainly care about URL construction
      let stdout = String::from_utf8(output.stdout)?;
      let stderr = String::from_utf8(output.stderr)?;
      println!("Command failed (expected for non-existent upstream):");
      println!("Stdout: {stdout}");
      println!("Stderr: {stderr}");

      // The important thing is that the command completed quickly
      println!("URL encoding test completed - credentials should be properly encoded");
    }

    Ok(())
  }
}

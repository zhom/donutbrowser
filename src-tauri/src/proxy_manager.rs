use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri_plugin_shell::ShellExt;

use crate::browser::ProxySettings;

// Store active proxy information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyInfo {
  pub id: String,
  pub local_url: String,
  pub upstream_url: String,
  pub local_port: u16,
}

// Global proxy manager to track active proxies
pub struct ProxyManager {
  active_proxies: Mutex<HashMap<u32, ProxyInfo>>, // Maps browser process ID to proxy info
  // Store proxy info by profile name for persistence across browser restarts
  profile_proxies: Mutex<HashMap<String, (String, u16)>>, // Maps profile name to (upstream_url, port)
}

impl ProxyManager {
  pub fn new() -> Self {
    Self {
      active_proxies: Mutex::new(HashMap::new()),
      profile_proxies: Mutex::new(HashMap::new()),
    }
  }

  // Start a proxy for a given upstream URL and associate it with a browser process ID
  pub async fn start_proxy(
    &self,
    app_handle: tauri::AppHandle,
    upstream_url: &str,
    browser_pid: u32,
    profile_name: Option<&str>,
  ) -> Result<ProxySettings, String> {
    // Check if we already have a proxy for this browser
    {
      let proxies = self.active_proxies.lock().unwrap();
      if let Some(proxy) = proxies.get(&browser_pid) {
        return Ok(ProxySettings {
          enabled: true,
          proxy_type: "http".to_string(),
          host: "localhost".to_string(),
          port: proxy.local_port,
        });
      }
    }

    // Check if we have a preferred port for this profile
    let preferred_port = if let Some(name) = profile_name {
      let profile_proxies = self.profile_proxies.lock().unwrap();
      profile_proxies.get(name).map(|(_, port)| *port)
    } else {
      None
    };

    // Start a new proxy using the nodecar binary
    let mut nodecar = app_handle
      .shell()
      .sidecar("nodecar")
      .unwrap()
      .arg("proxy")
      .arg("start")
      .arg("-u")
      .arg(upstream_url);

    // If we have a preferred port, use it
    if let Some(port) = preferred_port {
      nodecar = nodecar.arg("-p").arg(port.to_string());
    }

    let output = nodecar.output().await.unwrap();

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      return Err(format!("Proxy start failed: {}", stderr));
    }

    let json_string = String::from_utf8(output.stdout)
      .map_err(|e| format!("Failed to parse proxy output: {}", e))?;

    // Parse the JSON output
    let json: Value =
      serde_json::from_str(&json_string).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Extract proxy information
    let id = json["id"].as_str().ok_or("Missing proxy ID")?;
    let local_port = json["localPort"].as_u64().ok_or("Missing local port")? as u16;
    let local_url = json["localUrl"]
      .as_str()
      .ok_or("Missing local URL")?
      .to_string();
    let upstream_url_str = json["upstreamUrl"]
      .as_str()
      .ok_or("Missing upstream URL")?
      .to_string();

    let proxy_info = ProxyInfo {
      id: id.to_string(),
      local_url,
      upstream_url: upstream_url_str.clone(),
      local_port,
    };

    // Store the proxy info
    {
      let mut proxies = self.active_proxies.lock().unwrap();
      proxies.insert(browser_pid, proxy_info.clone());
    }

    // Store the profile proxy info for persistence
    if let Some(name) = profile_name {
      let mut profile_proxies = self.profile_proxies.lock().unwrap();
      profile_proxies.insert(name.to_string(), (upstream_url_str, local_port));
    }

    // Return proxy settings for the browser
    Ok(ProxySettings {
      enabled: true,
      proxy_type: "http".to_string(),
      host: "localhost".to_string(),
      port: proxy_info.local_port,
    })
  }

  // Stop the proxy associated with a browser process ID
  pub async fn stop_proxy(
    &self,
    app_handle: tauri::AppHandle,
    browser_pid: u32,
  ) -> Result<(), String> {
    let proxy_id = {
      let mut proxies = self.active_proxies.lock().unwrap();
      match proxies.remove(&browser_pid) {
        Some(proxy) => proxy.id,
        None => return Ok(()), // No proxy to stop
      }
    };

    // Stop the proxy using the nodecar binary
    let nodecar = app_handle
      .shell()
      .sidecar("nodecar")
      .map_err(|e| format!("Failed to create sidecar: {}", e))?
      .arg("proxy")
      .arg("stop")
      .arg("--id")
      .arg(proxy_id);

    let output = nodecar.output().await.unwrap();

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      eprintln!("Proxy stop error: {}", stderr);
      // We still return Ok since we've already removed the proxy from our tracking
    }

    Ok(())
  }

  // Get proxy settings for a browser process ID
  pub fn get_proxy_settings(&self, browser_pid: u32) -> Option<ProxySettings> {
    let proxies = self.active_proxies.lock().unwrap();
    proxies.get(&browser_pid).map(|proxy| ProxySettings {
      enabled: true,
      proxy_type: "http".to_string(),
      host: "localhost".to_string(),
      port: proxy.local_port,
    })
  }

  // Get stored proxy info for a profile
  pub fn get_profile_proxy_info(&self, profile_name: &str) -> Option<(String, u16)> {
    let profile_proxies = self.profile_proxies.lock().unwrap();
    profile_proxies.get(profile_name).cloned()
  }
}

// Create a singleton instance of the proxy manager
lazy_static::lazy_static! {
    pub static ref PROXY_MANAGER: ProxyManager = ProxyManager::new();
}

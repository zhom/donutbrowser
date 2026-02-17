use crate::browser_runner::BrowserRunner;
use crate::camoufox::{CamoufoxConfigBuilder, GeoIPOption, ScreenConstraints};
use crate::profile::BrowserProfile;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex as AsyncMutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CamoufoxConfig {
  pub proxy: Option<String>,
  pub screen_max_width: Option<u32>,
  pub screen_max_height: Option<u32>,
  pub screen_min_width: Option<u32>,
  pub screen_min_height: Option<u32>,
  pub geoip: Option<serde_json::Value>, // Can be String or bool
  pub block_images: Option<bool>,
  pub block_webrtc: Option<bool>,
  pub block_webgl: Option<bool>,
  pub executable_path: Option<String>,
  pub fingerprint: Option<String>, // JSON string of the complete fingerprint config
  pub randomize_fingerprint_on_launch: Option<bool>, // Generate new fingerprint on every launch
  pub os: Option<String>, // Operating system for fingerprint generation: "windows", "macos", or "linux"
}

impl Default for CamoufoxConfig {
  fn default() -> Self {
    Self {
      proxy: None,
      screen_max_width: None,
      screen_max_height: None,
      screen_min_width: None,
      screen_min_height: None,
      geoip: Some(serde_json::Value::Bool(true)),
      block_images: None,
      block_webrtc: None,
      block_webgl: None,
      executable_path: None,
      fingerprint: None,
      randomize_fingerprint_on_launch: None,
      os: None,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct CamoufoxLaunchResult {
  pub id: String,
  #[serde(alias = "process_id")]
  pub processId: Option<u32>,
  #[serde(alias = "profile_path")]
  pub profilePath: Option<String>,
  pub url: Option<String>,
}

#[derive(Debug)]
struct CamoufoxInstance {
  #[allow(dead_code)]
  id: String,
  process_id: Option<u32>,
  profile_path: Option<String>,
  url: Option<String>,
}

struct CamoufoxManagerInner {
  instances: HashMap<String, CamoufoxInstance>,
}

pub struct CamoufoxManager {
  inner: Arc<AsyncMutex<CamoufoxManagerInner>>,
  base_dirs: BaseDirs,
}

impl CamoufoxManager {
  fn new() -> Self {
    Self {
      inner: Arc::new(AsyncMutex::new(CamoufoxManagerInner {
        instances: HashMap::new(),
      })),
      base_dirs: BaseDirs::new().expect("Failed to get base directories"),
    }
  }

  pub fn instance() -> &'static CamoufoxManager {
    &CAMOUFOX_LAUNCHER
  }

  pub fn get_profiles_dir(&self) -> PathBuf {
    let mut path = self.base_dirs.data_local_dir().to_path_buf();
    path.push(if cfg!(debug_assertions) {
      "DonutBrowserDev"
    } else {
      "DonutBrowser"
    });
    path.push("profiles");
    path
  }

  /// Generate Camoufox fingerprint configuration during profile creation
  pub async fn generate_fingerprint_config(
    &self,
    _app_handle: &AppHandle,
    profile: &crate::profile::BrowserProfile,
    config: &CamoufoxConfig,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Get executable path
    let executable_path = if let Some(path) = &config.executable_path {
      let p = PathBuf::from(path);
      if p.exists() {
        p
      } else {
        log::warn!("Stored Camoufox executable path does not exist: {path}, falling back to dynamic resolution");
        BrowserRunner::instance()
          .get_browser_executable_path(profile)
          .map_err(|e| format!("Failed to get Camoufox executable path: {e}"))?
      }
    } else {
      BrowserRunner::instance()
        .get_browser_executable_path(profile)
        .map_err(|e| format!("Failed to get Camoufox executable path: {e}"))?
    };

    // Build the config using CamoufoxConfigBuilder
    let mut builder = CamoufoxConfigBuilder::new()
      .block_images(config.block_images.unwrap_or(false))
      .block_webrtc(config.block_webrtc.unwrap_or(false))
      .block_webgl(config.block_webgl.unwrap_or(false));

    // Set operating system
    if let Some(os) = &config.os {
      builder = builder.operating_system(os);
    }

    // Build screen constraints if provided
    if config.screen_min_width.is_some()
      || config.screen_max_width.is_some()
      || config.screen_min_height.is_some()
      || config.screen_max_height.is_some()
    {
      let screen_constraints = ScreenConstraints {
        min_width: config.screen_min_width,
        max_width: config.screen_max_width,
        min_height: config.screen_min_height,
        max_height: config.screen_max_height,
      };
      builder = builder.screen_constraints(screen_constraints);
    }

    // Parse proxy if provided
    if let Some(proxy_str) = &config.proxy {
      let proxy_config = crate::camoufox::ProxyConfig::from_url(proxy_str)
        .map_err(|e| format!("Failed to parse proxy URL: {e}"))?;
      builder = builder.proxy(proxy_config);
    }

    // Set Firefox version from executable
    if let Some(version) = crate::camoufox::config::get_firefox_version(&executable_path) {
      builder = builder.ff_version(version);
    }

    // Handle geoip option
    if let Some(geoip_value) = &config.geoip {
      match geoip_value {
        serde_json::Value::Bool(true) => {
          // Auto-detect IP (through proxy if set)
          builder = builder.geoip(GeoIPOption::Auto);
        }
        serde_json::Value::String(ip) => {
          // Use specific IP
          builder = builder.geoip(GeoIPOption::IP(ip.clone()));
        }
        _ => {
          // geoip: false or other values - don't apply geolocation
        }
      }
    }

    // Build the config (async to handle geoip)
    let launch_config = builder
      .build_async()
      .await
      .map_err(|e| format!("Failed to build Camoufox config: {e}"))?;

    // Return the fingerprint config as JSON
    let config_json = serde_json::to_string(&launch_config.fingerprint_config)
      .map_err(|e| format!("Failed to serialize config: {e}"))?;

    Ok(config_json)
  }

  /// Launch Camoufox browser by directly spawning the process
  pub async fn launch_camoufox(
    &self,
    _app_handle: &AppHandle,
    profile: &crate::profile::BrowserProfile,
    profile_path: &str,
    config: &CamoufoxConfig,
    url: Option<&str>,
  ) -> Result<CamoufoxLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
    let custom_config = if let Some(existing_fingerprint) = &config.fingerprint {
      log::info!("Using existing fingerprint from profile metadata");
      existing_fingerprint.clone()
    } else {
      return Err("No fingerprint provided".into());
    };

    // Get executable path
    let executable_path = if let Some(path) = &config.executable_path {
      let p = PathBuf::from(path);
      if p.exists() {
        p
      } else {
        log::warn!("Stored Camoufox executable path does not exist: {path}, falling back to dynamic resolution");
        BrowserRunner::instance()
          .get_browser_executable_path(profile)
          .map_err(|e| format!("Failed to get Camoufox executable path: {e}"))?
      }
    } else {
      BrowserRunner::instance()
        .get_browser_executable_path(profile)
        .map_err(|e| format!("Failed to get Camoufox executable path: {e}"))?
    };

    // Parse the fingerprint config JSON
    let fingerprint_config: HashMap<String, serde_json::Value> =
      serde_json::from_str(&custom_config)
        .map_err(|e| format!("Failed to parse fingerprint config: {e}"))?;

    // Convert to environment variables using CAMOU_CONFIG chunking
    let env_vars = crate::camoufox::env_vars::config_to_env_vars(&fingerprint_config)
      .map_err(|e| format!("Failed to convert config to env vars: {e}"))?;

    // Build command arguments
    // Note: We intentionally do NOT use -no-remote to allow opening URLs in existing instances
    // via Firefox's remote messaging mechanism. This enables "open in new tab" functionality
    // when Donut is set as the default browser.
    let mut args = vec![
      "-profile".to_string(),
      std::path::Path::new(profile_path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf())
        .to_string_lossy()
        .to_string(),
    ];

    // Add URL if provided
    if let Some(url) = url {
      args.push("-new-tab".to_string());
      args.push(url.to_string());
    }

    // Add headless flag for tests
    if std::env::var("CAMOUFOX_HEADLESS").is_ok() {
      args.push("--headless".to_string());
    }

    log::info!(
      "Launching Camoufox: {:?} with args: {:?}",
      executable_path,
      args
    );

    // Spawn the browser process
    let mut command = TokioCommand::new(&executable_path);
    command
      .args(&args)
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null());

    // Add environment variables
    for (key, value) in &env_vars {
      command.env(key, value);
    }

    // Handle fontconfig on Linux
    if cfg!(target_os = "linux") {
      let target_os = config.os.as_deref().unwrap_or("linux");
      if let Some(fontconfig_path) =
        crate::camoufox::env_vars::get_fontconfig_env(target_os, &executable_path)
      {
        command.env("FONTCONFIG_PATH", fontconfig_path);
      }
    }

    let child = command
      .spawn()
      .map_err(|e| format!("Failed to spawn Camoufox process: {e}"))?;

    let process_id = child.id();
    let instance_id = format!("camoufox_{}", process_id.unwrap_or(0));

    log::info!("Camoufox launched with PID: {:?}", process_id);

    // Store the instance
    let instance = CamoufoxInstance {
      id: instance_id.clone(),
      process_id,
      profile_path: Some(profile_path.to_string()),
      url: url.map(String::from),
    };

    let launch_result = CamoufoxLaunchResult {
      id: instance_id.clone(),
      processId: process_id,
      profilePath: Some(profile_path.to_string()),
      url: url.map(String::from),
    };

    {
      let mut inner = self.inner.lock().await;
      inner.instances.insert(instance_id, instance);
    }

    Ok(launch_result)
  }

  /// Stop a Camoufox process by ID
  pub async fn stop_camoufox(
    &self,
    _app_handle: &AppHandle,
    id: &str,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Get the process ID from our tracking
    let process_id = {
      let inner = self.inner.lock().await;
      inner
        .instances
        .get(id)
        .and_then(|instance| instance.process_id)
    };

    if let Some(pid) = process_id {
      // Kill the process
      let success = self.kill_process(pid);

      if success {
        // Remove from our tracking
        let mut inner = self.inner.lock().await;
        inner.instances.remove(id);
        log::info!("Stopped Camoufox instance {} (PID: {})", id, pid);
      }

      Ok(success)
    } else {
      // No process ID found, just remove from tracking
      let mut inner = self.inner.lock().await;
      inner.instances.remove(id);
      Ok(true)
    }
  }

  /// Kill a process by PID
  fn kill_process(&self, pid: u32) -> bool {
    #[cfg(unix)]
    {
      use std::os::unix::process::ExitStatusExt;
      let result = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();

      match result {
        Ok(status) => status.success() || status.signal() == Some(0),
        Err(e) => {
          log::warn!("Failed to kill process {}: {}", pid, e);
          false
        }
      }
    }

    #[cfg(windows)]
    {
      let result = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T"])
        .status();

      match result {
        Ok(status) => status.success(),
        Err(e) => {
          log::warn!("Failed to kill process {}: {}", pid, e);
          false
        }
      }
    }
  }

  /// Find Camoufox server by profile path (for integration with browser_runner)
  /// This method first checks in-memory instances, then scans system processes
  /// to detect Camoufox instances that may have been started before the app restarted.
  pub async fn find_camoufox_by_profile(
    &self,
    profile_path: &str,
  ) -> Result<Option<CamoufoxLaunchResult>, Box<dyn std::error::Error + Send + Sync>> {
    // First clean up any dead instances
    self.cleanup_dead_instances().await?;

    // Convert paths to canonical form for comparison
    let target_path = std::path::Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf());

    // Check in-memory instances first
    {
      let inner = self.inner.lock().await;

      for (id, instance) in inner.instances.iter() {
        if let Some(instance_profile_path) = &instance.profile_path {
          let instance_path = std::path::Path::new(instance_profile_path)
            .canonicalize()
            .unwrap_or_else(|_| std::path::Path::new(instance_profile_path).to_path_buf());

          if instance_path == target_path {
            // Verify the server is actually running by checking the process
            if let Some(process_id) = instance.process_id {
              if self.is_server_running(process_id).await {
                // Found running Camoufox instance
                return Ok(Some(CamoufoxLaunchResult {
                  id: id.clone(),
                  processId: instance.process_id,
                  profilePath: instance.profile_path.clone(),
                  url: instance.url.clone(),
                }));
              }
            }
          }
        }
      }
    }

    // If not found in in-memory instances, scan system processes
    // This handles the case where the app was restarted but Camoufox is still running
    if let Some((pid, found_profile_path)) = self.find_camoufox_process_by_profile(&target_path) {
      log::info!(
        "Found running Camoufox process (PID: {}) for profile path via system scan",
        pid
      );

      // Register this instance in our tracking
      let instance_id = format!("recovered_{}", pid);
      let mut inner = self.inner.lock().await;
      inner.instances.insert(
        instance_id.clone(),
        CamoufoxInstance {
          id: instance_id.clone(),
          process_id: Some(pid),
          profile_path: Some(found_profile_path.clone()),
          url: None,
        },
      );

      return Ok(Some(CamoufoxLaunchResult {
        id: instance_id,
        processId: Some(pid),
        profilePath: Some(found_profile_path),
        url: None,
      }));
    }

    Ok(None)
  }

  /// Scan system processes to find a Camoufox process using a specific profile path
  fn find_camoufox_process_by_profile(
    &self,
    target_path: &std::path::Path,
  ) -> Option<(u32, String)> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};

    let system = System::new_with_specifics(
      RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );

    let target_path_str = target_path.to_string_lossy();

    for (pid, process) in system.processes() {
      let cmd = process.cmd();
      if cmd.is_empty() {
        continue;
      }

      // Check if this is a Camoufox/Firefox process
      let exe_name = process.name().to_string_lossy().to_lowercase();
      let is_firefox_like = exe_name.contains("firefox")
        || exe_name.contains("camoufox")
        || exe_name.contains("firefox-bin");

      if !is_firefox_like {
        continue;
      }

      // Check if the command line contains our profile path
      for (i, arg) in cmd.iter().enumerate() {
        if let Some(arg_str) = arg.to_str() {
          // Check for -profile argument followed by our path
          if arg_str == "-profile" && i + 1 < cmd.len() {
            if let Some(next_arg) = cmd.get(i + 1).and_then(|a| a.to_str()) {
              let cmd_path = std::path::Path::new(next_arg)
                .canonicalize()
                .unwrap_or_else(|_| std::path::Path::new(next_arg).to_path_buf());

              if cmd_path == target_path {
                return Some((pid.as_u32(), next_arg.to_string()));
              }
            }
          }

          // Also check if the argument contains the profile path directly
          if arg_str.contains(&*target_path_str) {
            return Some((pid.as_u32(), target_path_str.to_string()));
          }
        }
      }
    }

    None
  }

  /// Check if servers are still alive and clean up dead instances
  pub async fn cleanup_dead_instances(
    &self,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut dead_instances = Vec::new();
    let mut instances_to_remove = Vec::new();

    {
      let inner = self.inner.lock().await;

      for (id, instance) in inner.instances.iter() {
        if let Some(process_id) = instance.process_id {
          // Check if the process is still alive
          if !self.is_server_running(process_id).await {
            // Process is dead
            // Camoufox instance is no longer running
            dead_instances.push(id.clone());
            instances_to_remove.push(id.clone());
          }
        } else {
          // No process_id means it's likely a dead instance
          // Camoufox instance has no PID, marking as dead
          dead_instances.push(id.clone());
          instances_to_remove.push(id.clone());
        }
      }
    }

    // Remove dead instances
    if !instances_to_remove.is_empty() {
      let mut inner = self.inner.lock().await;
      for id in &instances_to_remove {
        inner.instances.remove(id);
        // Removed dead Camoufox instance
      }
    }

    Ok(dead_instances)
  }

  /// Check if a Camoufox server is running with the given process ID
  async fn is_server_running(&self, process_id: u32) -> bool {
    // Check if the process is still running
    use sysinfo::{Pid, System};

    let system = System::new_all();
    if let Some(process) = system.process(Pid::from(process_id as usize)) {
      // Check if this is actually a Camoufox process by looking at the command line
      let cmd = process.cmd();
      let is_camoufox = cmd.iter().any(|arg| {
        let arg_str = arg.to_str().unwrap_or("");
        arg_str.contains("camoufox-worker") || arg_str.contains("camoufox")
      });

      if is_camoufox {
        // Found running Camoufox process
        return true;
      }
    }

    false
  }
}

impl CamoufoxManager {
  pub async fn launch_camoufox_profile(
    &self,
    app_handle: AppHandle,
    profile: BrowserProfile,
    config: CamoufoxConfig,
    url: Option<String>,
  ) -> Result<CamoufoxLaunchResult, String> {
    // Get profile path
    let profiles_dir = self.get_profiles_dir();
    let profile_path = profile.get_profile_data_path(&profiles_dir);
    let profile_path_str = profile_path.to_string_lossy();

    // Check if there's already a running instance for this profile
    if let Ok(Some(existing)) = self.find_camoufox_by_profile(&profile_path_str).await {
      // If there's an existing instance, stop it first to avoid conflicts
      let _ = self.stop_camoufox(&app_handle, &existing.id).await;
    }

    // Clean up any dead instances before launching
    let _ = self.cleanup_dead_instances().await;

    self
      .launch_camoufox(
        &app_handle,
        &profile,
        &profile_path_str,
        &config,
        url.as_deref(),
      )
      .await
      .map_err(|e| format!("Failed to launch Camoufox: {e}"))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_default_config() {
    let default_config = CamoufoxConfig::default();

    // Verify defaults
    assert_eq!(default_config.geoip, Some(serde_json::Value::Bool(true)));
    assert_eq!(default_config.proxy, None);
    assert_eq!(default_config.fingerprint, None);
    assert_eq!(default_config.randomize_fingerprint_on_launch, None);
    assert_eq!(default_config.os, None);
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref CAMOUFOX_LAUNCHER: CamoufoxManager = CamoufoxManager::new();
}

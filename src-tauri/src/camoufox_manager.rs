use crate::browser_runner::BrowserRunner;
use crate::profile::BrowserProfile;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
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
    &CAMOUFOX_NODECAR_LAUNCHER
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
    app_handle: &AppHandle,
    profile: &crate::profile::BrowserProfile,
    config: &CamoufoxConfig,
  ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut config_args = vec!["camoufox".to_string(), "generate-config".to_string()];

    // Always ensure executable_path is set to the user's binary location
    let executable_path = if let Some(path) = &config.executable_path {
      path.clone()
    } else {
      // Use the browser runner helper with the real profile
      // Use self.browser_runner instead of instance()
      BrowserRunner::instance()
        .get_browser_executable_path(profile)
        .map_err(|e| format!("Failed to get Camoufox executable path: {e}"))?
        .to_string_lossy()
        .to_string()
    };
    config_args.extend(["--executable-path".to_string(), executable_path]);

    // Pass existing fingerprint if provided (for advanced form partial fingerprints)
    if let Some(fingerprint) = &config.fingerprint {
      config_args.extend(["--fingerprint".to_string(), fingerprint.clone()]);
    }

    if let Some(serde_json::Value::Bool(true)) = &config.geoip {
      config_args.push("--geoip".to_string());
    }

    // Add proxy if provided (can be passed directly during fingerprint generation)
    if let Some(proxy) = &config.proxy {
      config_args.extend(["--proxy".to_string(), proxy.clone()]);
    }

    // Add screen dimensions if provided
    if let Some(max_width) = config.screen_max_width {
      config_args.extend(["--max-width".to_string(), max_width.to_string()]);
    }

    if let Some(max_height) = config.screen_max_height {
      config_args.extend(["--max-height".to_string(), max_height.to_string()]);
    }

    if let Some(min_width) = config.screen_min_width {
      config_args.extend(["--min-width".to_string(), min_width.to_string()]);
    }

    if let Some(min_height) = config.screen_min_height {
      config_args.extend(["--min-height".to_string(), min_height.to_string()]);
    }

    // Add block_* options
    if let Some(block_images) = config.block_images {
      if block_images {
        config_args.push("--block-images".to_string());
      }
    }

    if let Some(block_webrtc) = config.block_webrtc {
      if block_webrtc {
        config_args.push("--block-webrtc".to_string());
      }
    }

    if let Some(block_webgl) = config.block_webgl {
      if block_webgl {
        config_args.push("--block-webgl".to_string());
      }
    }

    // Add OS option for fingerprint generation
    if let Some(os) = &config.os {
      config_args.extend(["--os".to_string(), os.clone()]);
    }

    // Execute config generation command
    let mut config_sidecar = self.get_nodecar_sidecar(app_handle)?;
    for arg in &config_args {
      config_sidecar = config_sidecar.arg(arg);
    }

    let config_output = config_sidecar.output().await?;
    if !config_output.status.success() {
      let stderr = String::from_utf8_lossy(&config_output.stderr);
      return Err(format!("Failed to generate camoufox fingerprint config: {stderr}").into());
    }

    Ok(String::from_utf8_lossy(&config_output.stdout).to_string())
  }

  /// Get the nodecar sidecar command
  fn get_nodecar_sidecar(
    &self,
    app_handle: &AppHandle,
  ) -> Result<tauri_plugin_shell::process::Command, Box<dyn std::error::Error + Send + Sync>> {
    let shell = app_handle.shell();
    let sidecar_command = shell
      .sidecar("nodecar")
      .map_err(|e| format!("Failed to create nodecar sidecar: {e}"))?;
    Ok(sidecar_command)
  }

  /// Launch Camoufox browser using nodecar sidecar
  pub async fn launch_camoufox(
    &self,
    app_handle: &AppHandle,
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

    // Always ensure executable_path is set to the user's binary location
    let executable_path = if let Some(path) = &config.executable_path {
      path.clone()
    } else {
      // Use the browser runner helper with the real profile
      // Use self.browser_runner instead of instance()
      BrowserRunner::instance()
        .get_browser_executable_path(profile)
        .map_err(|e| format!("Failed to get Camoufox executable path: {e}"))?
        .to_string_lossy()
        .to_string()
    };

    // Build nodecar command arguments
    let mut args = vec!["camoufox".to_string(), "start".to_string()];

    // Add profile path - ensure it's an absolute path
    let absolute_profile_path = std::path::Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf())
      .to_string_lossy()
      .to_string();
    args.extend(["--profile-path".to_string(), absolute_profile_path]);

    // Add URL if provided
    if let Some(url) = url {
      args.extend(["--url".to_string(), url.to_string()]);
    }

    // Always add the executable path
    args.extend(["--executable-path".to_string(), executable_path]);

    // Always add the generated custom config
    args.extend(["--custom-config".to_string(), custom_config]);

    // Add proxy if provided
    if let Some(proxy) = &config.proxy {
      args.extend(["--proxy".to_string(), proxy.clone()]);
    }

    // Add headless flag for tests
    if std::env::var("CAMOUFOX_HEADLESS").is_ok() {
      args.push("--headless".to_string());
    }

    // Get the nodecar sidecar command
    let mut sidecar_command = self.get_nodecar_sidecar(app_handle)?;

    // Add all arguments to the sidecar command
    for arg in &args {
      sidecar_command = sidecar_command.arg(arg);
    }

    // Execute nodecar sidecar command
    log::info!("Executing nodecar command with args: {args:?}");
    let output = sidecar_command.output().await?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let stdout = String::from_utf8_lossy(&output.stdout);
      log::info!("nodecar camoufox failed - stdout: {stdout}, stderr: {stderr}");
      return Err(format!("nodecar camoufox failed: {stderr}").into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    log::info!("nodecar camoufox output: {stdout}");

    // Parse the JSON output
    let launch_result: CamoufoxLaunchResult = serde_json::from_str(&stdout)
      .map_err(|e| format!("Failed to parse nodecar output as JSON: {e}\nOutput was: {stdout}"))?;

    // Store the instance
    let instance = CamoufoxInstance {
      id: launch_result.id.clone(),
      process_id: launch_result.processId,
      profile_path: launch_result.profilePath.clone(),
      url: launch_result.url.clone(),
    };

    {
      let mut inner = self.inner.lock().await;
      inner.instances.insert(launch_result.id.clone(), instance);
    }

    Ok(launch_result)
  }

  /// Stop a Camoufox process by ID
  pub async fn stop_camoufox(
    &self,
    app_handle: &AppHandle,
    id: &str,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Get the nodecar sidecar command
    let sidecar_command = self
      .get_nodecar_sidecar(app_handle)?
      .arg("camoufox")
      .arg("stop")
      .arg("--id")
      .arg(id);

    // Execute nodecar stop command
    let output = sidecar_command.output().await?;

    if !output.status.success() {
      let _stderr = String::from_utf8_lossy(&output.stderr);
      return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(&stdout)
      .map_err(|e| format!("Failed to parse nodecar stop output: {e}"))?;

    let success = result
      .get("success")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    if success {
      // Remove from our tracking
      let mut inner = self.inner.lock().await;
      inner.instances.remove(id);
    }

    Ok(success)
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
  pub async fn launch_camoufox_profile_nodecar(
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
      .map_err(|e| format!("Failed to launch Camoufox via nodecar: {e}"))
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
  static ref CAMOUFOX_NODECAR_LAUNCHER: CamoufoxManager = CamoufoxManager::new();
}

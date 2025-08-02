use crate::profile::BrowserProfile;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex as AsyncMutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CamoufoxConfig {
  pub os: Option<Vec<String>>,
  pub block_images: Option<bool>,
  pub block_webrtc: Option<bool>,
  pub block_webgl: Option<bool>,
  pub disable_coop: Option<bool>,
  pub geoip: Option<serde_json::Value>, // Can be String or bool
  pub country: Option<String>,
  pub timezone: Option<String>,
  pub latitude: Option<f64>,
  pub longitude: Option<f64>,
  pub humanize: Option<bool>,
  pub humanize_duration: Option<f64>,
  pub headless: Option<bool>,
  pub locale: Option<Vec<String>>,
  pub addons: Option<Vec<String>>,
  pub fonts: Option<Vec<String>>,
  pub custom_fonts_only: Option<bool>,
  pub exclude_addons: Option<Vec<String>>,
  pub screen_min_width: Option<u32>,
  pub screen_max_width: Option<u32>,
  pub screen_min_height: Option<u32>,
  pub screen_max_height: Option<u32>,
  pub window_width: Option<u32>,
  pub window_height: Option<u32>,
  pub ff_version: Option<u32>,
  pub main_world_eval: Option<bool>,
  pub webgl_vendor: Option<String>,
  pub webgl_renderer: Option<String>,
  pub proxy: Option<String>,
  pub enable_cache: Option<bool>,
  pub virtual_display: Option<String>,
  pub debug: Option<bool>,
  pub additional_args: Option<Vec<String>>,
  pub env_vars: Option<HashMap<String, String>>,
  pub firefox_prefs: Option<HashMap<String, serde_json::Value>>,
  pub disable_theming: Option<bool>,
  pub showcursor: Option<bool>,
}

impl Default for CamoufoxConfig {
  fn default() -> Self {
    Self {
      os: None,
      block_images: None,
      block_webrtc: None,
      block_webgl: None,
      disable_coop: None,
      geoip: None,
      country: None,
      timezone: None,
      latitude: None,
      longitude: None,
      humanize: None,
      humanize_duration: None,
      headless: None,
      locale: None,
      addons: None,
      fonts: None,
      custom_fonts_only: None,
      exclude_addons: None,
      screen_min_width: None,
      screen_max_width: None,
      screen_min_height: None,
      screen_max_height: None,
      window_width: None,
      window_height: None,
      ff_version: None,
      main_world_eval: None,
      webgl_vendor: None,
      webgl_renderer: None,
      proxy: None,
      enable_cache: Some(true), // Cache enabled by default
      virtual_display: None,
      debug: None,
      additional_args: None,
      env_vars: None,
      firefox_prefs: None,
      disable_theming: Some(true),
      showcursor: Some(false),
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

struct CamoufoxNodecarLauncherInner {
  instances: HashMap<String, CamoufoxInstance>,
}

pub struct CamoufoxNodecarLauncher {
  inner: Arc<AsyncMutex<CamoufoxNodecarLauncherInner>>,
}

impl CamoufoxNodecarLauncher {
  fn new() -> Self {
    Self {
      inner: Arc::new(AsyncMutex::new(CamoufoxNodecarLauncherInner {
        instances: HashMap::new(),
      })),
    }
  }

  pub fn instance() -> &'static CamoufoxNodecarLauncher {
    &CAMOUFOX_NODECAR_LAUNCHER
  }

  /// Create a test configuration to verify anti-fingerprinting is working
  pub fn create_test_config() -> CamoufoxConfig {
    CamoufoxConfig {
      // Core anti-fingerprinting settings
      timezone: Some("Europe/London".to_string()),
      screen_min_width: Some(1440),
      screen_min_height: Some(900),
      window_width: Some(1200),
      window_height: Some(800),

      // Locale settings
      locale: Some(vec!["en-GB".to_string(), "en-US".to_string()]),

      // WebGL spoofing
      webgl_vendor: Some("Intel Inc.".to_string()),
      webgl_renderer: Some("Intel Iris Pro OpenGL Engine".to_string()),

      // Geolocation spoofing (London coordinates)
      latitude: Some(51.5074),
      longitude: Some(-0.1278),

      // Font settings
      fonts: Some(vec![
        "Arial".to_string(),
        "Times New Roman".to_string(),
        "Helvetica".to_string(),
        "Georgia".to_string(),
      ]),
      custom_fonts_only: Some(true),

      // Humanization
      humanize: Some(true),
      humanize_duration: Some(2.0),

      // Blocking features
      block_images: Some(false), // Don't block images for testing
      block_webrtc: Some(true),
      block_webgl: Some(false), // Don't block WebGL so we can test spoofing

      // Other settings
      debug: Some(true),
      enable_cache: Some(true),
      headless: Some(false), // Not headless for testing
      disable_theming: Some(true),
      showcursor: Some(false),

      ..Default::default()
    }
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
    profile_path: &str,
    config: &CamoufoxConfig,
    url: Option<&str>,
  ) -> Result<CamoufoxLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
    // Build nodecar command arguments
    let mut args = vec!["camoufox".to_string(), "start".to_string()];

    // Add profile path
    args.extend(["--profile-path".to_string(), profile_path.to_string()]);

    // Add URL if provided
    if let Some(url) = url {
      args.extend(["--url".to_string(), url.to_string()]);
    }

    // Add configuration options
    if let Some(os_list) = &config.os {
      let os_str = os_list.join(",");
      args.extend(["--os".to_string(), os_str]);
    }

    if let Some(block_images) = config.block_images {
      if block_images {
        args.push("--block-images".to_string());
      }
    }

    if let Some(block_webrtc) = config.block_webrtc {
      if block_webrtc {
        args.push("--block-webrtc".to_string());
      }
    }

    if let Some(block_webgl) = config.block_webgl {
      if block_webgl {
        args.push("--block-webgl".to_string());
      }
    }

    if let Some(disable_coop) = config.disable_coop {
      if disable_coop {
        args.push("--disable-coop".to_string());
      }
    }

    if let Some(geoip) = &config.geoip {
      match geoip {
        serde_json::Value::Bool(true) => {
          args.extend(["--geoip".to_string(), "auto".to_string()]);
        }
        serde_json::Value::String(ip) => {
          args.extend(["--geoip".to_string(), ip.clone()]);
        }
        _ => {}
      }
    }

    if let Some(country) = &config.country {
      args.extend(["--country".to_string(), country.clone()]);
    }

    if let Some(timezone) = &config.timezone {
      args.extend(["--timezone".to_string(), timezone.clone()]);
    }

    if let Some(latitude) = config.latitude {
      args.extend(["--latitude".to_string(), latitude.to_string()]);
    }

    if let Some(longitude) = config.longitude {
      args.extend(["--longitude".to_string(), longitude.to_string()]);
    }

    if let Some(humanize) = config.humanize {
      if humanize {
        if let Some(duration) = config.humanize_duration {
          args.extend(["--humanize".to_string(), duration.to_string()]);
        } else {
          args.push("--humanize".to_string());
        }
      }
    }

    if let Some(headless) = config.headless {
      if headless {
        args.push("--headless".to_string());
      }
    }

    if let Some(locale_list) = &config.locale {
      let locale_str = locale_list.join(",");
      args.extend(["--locale".to_string(), locale_str]);
    }

    if let Some(addons) = &config.addons {
      let addons_str = addons.join(",");
      args.extend(["--addons".to_string(), addons_str]);
    }

    if let Some(fonts) = &config.fonts {
      let fonts_str = fonts.join(",");
      args.extend(["--fonts".to_string(), fonts_str]);
    }

    if let Some(custom_fonts_only) = config.custom_fonts_only {
      if custom_fonts_only {
        args.push("--custom-fonts-only".to_string());
      }
    }

    if let Some(exclude_addons) = &config.exclude_addons {
      let exclude_str = exclude_addons.join(",");
      args.extend(["--exclude-addons".to_string(), exclude_str]);
    }

    if let Some(screen_min_width) = config.screen_min_width {
      args.extend([
        "--screen-min-width".to_string(),
        screen_min_width.to_string(),
      ]);
    }

    if let Some(screen_max_width) = config.screen_max_width {
      args.extend([
        "--screen-max-width".to_string(),
        screen_max_width.to_string(),
      ]);
    }

    if let Some(screen_min_height) = config.screen_min_height {
      args.extend([
        "--screen-min-height".to_string(),
        screen_min_height.to_string(),
      ]);
    }

    if let Some(screen_max_height) = config.screen_max_height {
      args.extend([
        "--screen-max-height".to_string(),
        screen_max_height.to_string(),
      ]);
    }

    if let Some(window_width) = config.window_width {
      args.extend(["--window-width".to_string(), window_width.to_string()]);
    }

    if let Some(window_height) = config.window_height {
      args.extend(["--window-height".to_string(), window_height.to_string()]);
    }

    if let Some(ff_version) = config.ff_version {
      args.extend(["--ff-version".to_string(), ff_version.to_string()]);
    }

    if let Some(main_world_eval) = config.main_world_eval {
      if main_world_eval {
        args.push("--main-world-eval".to_string());
      }
    }

    if let Some(webgl_vendor) = &config.webgl_vendor {
      args.extend(["--webgl-vendor".to_string(), webgl_vendor.clone()]);
    }

    if let Some(webgl_renderer) = &config.webgl_renderer {
      args.extend(["--webgl-renderer".to_string(), webgl_renderer.clone()]);
    }

    if let Some(proxy) = &config.proxy {
      args.extend(["--proxy".to_string(), proxy.clone()]);
    }

    if let Some(enable_cache) = config.enable_cache {
      if !enable_cache {
        args.push("--disable-cache".to_string());
      }
    }

    if let Some(virtual_display) = &config.virtual_display {
      args.extend(["--virtual-display".to_string(), virtual_display.clone()]);
    }

    if let Some(debug) = config.debug {
      if debug {
        args.push("--debug".to_string());
      }
    }

    if let Some(additional_args) = &config.additional_args {
      let args_str = additional_args.join(",");
      args.extend(["--args".to_string(), args_str]);
    }

    if let Some(env_vars) = &config.env_vars {
      let env_json = serde_json::to_string(env_vars)?;
      args.extend(["--env".to_string(), env_json]);
    }

    if let Some(firefox_prefs) = &config.firefox_prefs {
      let prefs_json = serde_json::to_string(firefox_prefs)?;
      args.extend(["--firefox-prefs".to_string(), prefs_json]);
    }

    if let Some(disable_theming) = config.disable_theming {
      if disable_theming {
        args.push("--disable-theming".to_string());
      }
    }

    if let Some(showcursor) = config.showcursor {
      if showcursor {
        args.push("--showcursor".to_string());
      } else {
        args.push("--no-showcursor".to_string());
      }
    }

    // Get the nodecar sidecar command
    let mut sidecar_command = self.get_nodecar_sidecar(app_handle)?;

    // Add all arguments to the sidecar command
    for arg in &args {
      sidecar_command = sidecar_command.arg(arg);
    }

    // Execute nodecar sidecar command
    println!("Executing nodecar command with args: {args:?}");
    let output = sidecar_command.output().await?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let stdout = String::from_utf8_lossy(&output.stdout);
      println!("nodecar camoufox failed - stdout: {stdout}, stderr: {stderr}");
      return Err(format!("nodecar camoufox failed: {stderr}").into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("nodecar camoufox output: {stdout}");

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
  pub async fn find_camoufox_by_profile(
    &self,
    profile_path: &str,
  ) -> Result<Option<CamoufoxLaunchResult>, Box<dyn std::error::Error + Send + Sync>> {
    // First clean up any dead instances
    self.cleanup_dead_instances().await?;

    let inner = self.inner.lock().await;

    // Convert paths to canonical form for comparison
    let target_path = std::path::Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf());

    for (id, instance) in inner.instances.iter() {
      if let Some(instance_profile_path) = &instance.profile_path {
        let instance_path = std::path::Path::new(instance_profile_path)
          .canonicalize()
          .unwrap_or_else(|_| std::path::Path::new(instance_profile_path).to_path_buf());

        if instance_path == target_path {
          // Verify the server is actually running by checking the process
          if let Some(process_id) = instance.process_id {
            if self.is_server_running(process_id).await {
              println!("Found running Camoufox instance for profile: {profile_path}");
              return Ok(Some(CamoufoxLaunchResult {
                id: id.clone(),
                processId: instance.process_id,
                profilePath: instance.profile_path.clone(),
                url: instance.url.clone(),
              }));
            } else {
              println!("Camoufox instance found but process is not running: {id}");
            }
          }
        }
      }
    }

    println!("No running Camoufox instance found for profile: {profile_path}");
    Ok(None)
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
            println!("Camoufox instance {id} (PID: {process_id}) is no longer running");
            dead_instances.push(id.clone());
            instances_to_remove.push(id.clone());
          }
        } else {
          // No process_id means it's likely a dead instance
          println!("Camoufox instance {id} has no PID, marking as dead");
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
        println!("Removed dead Camoufox instance: {id}");
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
        println!("Found running Camoufox process with PID: {process_id}");
        return true;
      }
    }

    false
  }
}

impl CamoufoxNodecarLauncher {
  pub async fn launch_camoufox_profile_nodecar(
    &self,
    app_handle: AppHandle,
    profile: BrowserProfile,
    config: CamoufoxConfig,
    url: Option<String>,
  ) -> Result<CamoufoxLaunchResult, String> {
    // Get profile path
    let browser_runner = crate::browser_runner::BrowserRunner::instance();
    let profiles_dir = browser_runner.get_profiles_dir();
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
      .launch_camoufox(&app_handle, &profile_path_str, &config, url.as_deref())
      .await
      .map_err(|e| format!("Failed to launch Camoufox via nodecar: {e}"))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_camoufox_config_creation() {
    let test_config = CamoufoxNodecarLauncher::create_test_config();

    // Verify test config has expected values
    assert_eq!(test_config.timezone, Some("Europe/London".to_string()));
    assert_eq!(test_config.screen_min_width, Some(1440));
    assert_eq!(test_config.screen_min_height, Some(900));
    assert_eq!(test_config.window_width, Some(1200));
    assert_eq!(test_config.window_height, Some(800));
    assert_eq!(test_config.webgl_vendor, Some("Intel Inc.".to_string()));
    assert_eq!(
      test_config.webgl_renderer,
      Some("Intel Iris Pro OpenGL Engine".to_string())
    );
    assert_eq!(test_config.latitude, Some(51.5074));
    assert_eq!(test_config.longitude, Some(-0.1278));
    assert_eq!(test_config.humanize, Some(true));
    assert_eq!(test_config.debug, Some(true));
    assert_eq!(test_config.enable_cache, Some(true));
    assert_eq!(test_config.headless, Some(false));
  }

  #[test]
  fn test_default_config() {
    let default_config = CamoufoxConfig::default();

    // Verify defaults
    assert_eq!(default_config.enable_cache, Some(true));
    assert_eq!(default_config.timezone, None);
    assert_eq!(default_config.debug, None);
    assert_eq!(default_config.headless, None);
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref CAMOUFOX_NODECAR_LAUNCHER: CamoufoxNodecarLauncher = CamoufoxNodecarLauncher::new();
}

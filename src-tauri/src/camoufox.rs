use crate::browser_runner::BrowserProfile;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

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
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct CamoufoxLaunchResult {
  pub id: String,
  pub pid: Option<u32>,
  #[serde(alias = "executable_path")]
  pub executablePath: String,
  #[serde(alias = "profile_path")]
  pub profilePath: String,
  pub url: Option<String>,
}

pub struct CamoufoxLauncher {
  app_handle: AppHandle,
}

impl CamoufoxLauncher {
  pub fn new(app_handle: AppHandle) -> Self {
    Self { app_handle }
  }

  /// Launch Camoufox browser with the specified configuration
  pub async fn launch_camoufox(
    &self,
    executable_path: &str,
    profile_path: &str,
    config: &CamoufoxConfig,
    url: Option<&str>,
  ) -> Result<CamoufoxLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
    println!("Launching Camoufox with executable: {executable_path}");
    println!("Profile path: {profile_path}");
    println!("URL: {url:?}");

    // Use Tauri's sidecar to call nodecar
    let mut sidecar = self
      .app_handle
      .shell()
      .sidecar("nodecar")
      .map_err(|e| format!("Failed to create nodecar sidecar: {e}"))?
      .arg("camoufox")
      .arg("launch")
      .arg("--executable-path")
      .arg(executable_path)
      .arg("--profile-path")
      .arg(profile_path);

    // Add URL if provided
    if let Some(url) = url {
      sidecar = sidecar.arg("--url").arg(url);
    }

    // Add configuration options
    if let Some(os_list) = &config.os {
      sidecar = sidecar.arg("--os").arg(os_list.join(","));
    }

    if config.block_images.unwrap_or(false) {
      sidecar = sidecar.arg("--block-images");
    }

    if config.block_webrtc.unwrap_or(false) {
      sidecar = sidecar.arg("--block-webrtc");
    }

    if config.block_webgl.unwrap_or(false) {
      sidecar = sidecar.arg("--block-webgl");
    }

    if config.disable_coop.unwrap_or(false) {
      sidecar = sidecar.arg("--disable-coop");
    }

    if let Some(geoip) = &config.geoip {
      match geoip {
        serde_json::Value::String(s) => {
          sidecar = sidecar.arg("--geoip").arg(s);
        }
        serde_json::Value::Bool(b) => {
          sidecar = sidecar
            .arg("--geoip")
            .arg(if *b { "auto" } else { "false" });
        }
        _ => {
          sidecar = sidecar.arg("--geoip").arg(geoip.to_string());
        }
      }
    }

    if let Some(country) = &config.country {
      sidecar = sidecar.arg("--country").arg(country);
    }

    if let Some(timezone) = &config.timezone {
      sidecar = sidecar.arg("--timezone").arg(timezone);
    }

    if let Some(latitude) = config.latitude {
      if let Some(longitude) = config.longitude {
        sidecar = sidecar.arg("--latitude").arg(latitude.to_string());
        sidecar = sidecar.arg("--longitude").arg(longitude.to_string());
      }
    }

    if let Some(humanize) = config.humanize {
      if humanize {
        if let Some(duration) = config.humanize_duration {
          sidecar = sidecar.arg("--humanize").arg(duration.to_string());
        } else {
          sidecar = sidecar.arg("--humanize");
        }
      }
    }

    if config.headless.unwrap_or(false) {
      sidecar = sidecar.arg("--headless");
    }

    if let Some(locale_list) = &config.locale {
      sidecar = sidecar.arg("--locale").arg(locale_list.join(","));
    }

    if let Some(addons_list) = &config.addons {
      sidecar = sidecar.arg("--addons").arg(addons_list.join(","));
    }

    if let Some(fonts_list) = &config.fonts {
      sidecar = sidecar.arg("--fonts").arg(fonts_list.join(","));
    }

    if config.custom_fonts_only.unwrap_or(false) {
      sidecar = sidecar.arg("--custom-fonts-only");
    }

    if let Some(exclude_addons_list) = &config.exclude_addons {
      sidecar = sidecar
        .arg("--exclude-addons")
        .arg(exclude_addons_list.join(","));
    }

    // Screen size configuration
    if let Some(width) = config.screen_min_width {
      sidecar = sidecar.arg("--screen-min-width").arg(width.to_string());
    }

    if let Some(width) = config.screen_max_width {
      sidecar = sidecar.arg("--screen-max-width").arg(width.to_string());
    }

    if let Some(height) = config.screen_min_height {
      sidecar = sidecar.arg("--screen-min-height").arg(height.to_string());
    }

    if let Some(height) = config.screen_max_height {
      sidecar = sidecar.arg("--screen-max-height").arg(height.to_string());
    }

    if let Some(width) = config.window_width {
      sidecar = sidecar.arg("--window-width").arg(width.to_string());
    }

    if let Some(height) = config.window_height {
      sidecar = sidecar.arg("--window-height").arg(height.to_string());
    }

    // Advanced options
    if let Some(ff_version) = config.ff_version {
      sidecar = sidecar.arg("--ff-version").arg(ff_version.to_string());
    }

    if config.main_world_eval.unwrap_or(false) {
      sidecar = sidecar.arg("--main-world-eval");
    }

    if let Some(vendor) = &config.webgl_vendor {
      if let Some(renderer) = &config.webgl_renderer {
        sidecar = sidecar.arg("--webgl-vendor").arg(vendor);
        sidecar = sidecar.arg("--webgl-renderer").arg(renderer);
      }
    }

    if let Some(proxy) = &config.proxy {
      sidecar = sidecar.arg("--proxy").arg(proxy);
    }

    // Cache is enabled by default, only add flag if disabled
    if !config.enable_cache.unwrap_or(true) {
      sidecar = sidecar.arg("--disable-cache");
    }

    if let Some(virtual_display) = &config.virtual_display {
      sidecar = sidecar.arg("--virtual-display").arg(virtual_display);
    }

    if config.debug.unwrap_or(false) {
      sidecar = sidecar.arg("--debug");
    }

    if let Some(args) = &config.additional_args {
      sidecar = sidecar.arg("--args").arg(args.join(","));
    }

    if let Some(env_vars) = &config.env_vars {
      let env_json = serde_json::to_string(env_vars)
        .map_err(|e| format!("Failed to serialize environment variables: {e}"))?;
      sidecar = sidecar.arg("--env").arg(env_json);
    }

    if let Some(firefox_prefs) = &config.firefox_prefs {
      let prefs_json = serde_json::to_string(firefox_prefs)
        .map_err(|e| format!("Failed to serialize Firefox preferences: {e}"))?;
      sidecar = sidecar.arg("--firefox-prefs").arg(prefs_json);
    }

    // Execute the command
    println!("Executing nodecar command...");
    let output = sidecar
      .output()
      .await
      .map_err(|e| format!("Failed to execute nodecar command: {e}"))?;

    // Check the command status first
    if !output.status.success() {
      let error_msg = String::from_utf8_lossy(&output.stderr);
      let stdout_msg = String::from_utf8_lossy(&output.stdout);
      return Err(
        format!(
          "Failed to launch Camoufox: Command failed with status {:?}\nstderr: {}\nstdout: {}",
          output.status, error_msg, stdout_msg
        )
        .into(),
      );
    }

    // Parse the JSON response
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("Nodecar stdout: {stdout}");

    // Try to parse the JSON response
    let result: CamoufoxLaunchResult = serde_json::from_str(&stdout)
      .map_err(|e| format!("Failed to parse nodecar response as JSON: {e}\nResponse: {stdout}"))?;

    println!("Successfully launched Camoufox with ID: {}", result.id);

    Ok(result)
  }

  /// Stop a Camoufox process by ID
  pub async fn stop_camoufox(
    &self,
    id: &str,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    println!("Stopping Camoufox process with ID: {id}");

    // First, we need to find the process to get its executable and profile paths
    let processes = self.list_camoufox_processes().await?;
    let target_process = processes.iter().find(|p| p.id == id);

    if let Some(process) = target_process {
      println!(
        "Found process to stop: executable={}, profile={}",
        process.executablePath, process.profilePath
      );

      let sidecar = self
        .app_handle
        .shell()
        .sidecar("nodecar")
        .map_err(|e| format!("Failed to create nodecar sidecar: {e}"))?
        .arg("camoufox")
        .arg("stop")
        .arg("--executable-path")
        .arg(&process.executablePath)
        .arg("--profile-path")
        .arg(&process.profilePath)
        .arg("--id")
        .arg(id);

      let output = sidecar
        .output()
        .await
        .map_err(|e| format!("Failed to execute nodecar stop command: {e}"))?;

      if !output.status.success() {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        let stdout_msg = String::from_utf8_lossy(&output.stdout);
        println!("Failed to stop Camoufox process - stderr: {error_msg}, stdout: {stdout_msg}");
        return Err(format!("Failed to stop Camoufox process: {error_msg}").into());
      }

      let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
      println!("Stop command result: {stdout}");

      // Parse the JSON response which contains a "success" field
      let response: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("Failed to parse stop response as JSON: {e}\nResponse: {stdout}"))?;

      let success = response
        .get("success")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| {
          format!("Invalid response format - missing or invalid 'success' field: {stdout}")
        })?;

      if success {
        println!("Successfully stopped Camoufox process: {id}");
      } else {
        println!("Failed to stop Camoufox process: {id} (process may not exist)");
      }

      Ok(success)
    } else {
      println!("Camoufox process with ID {id} not found in running processes");
      // If we can't find the process, it might already be stopped
      Ok(false)
    }
  }

  /// List all Camoufox processes
  pub async fn list_camoufox_processes(
    &self,
  ) -> Result<Vec<CamoufoxLaunchResult>, Box<dyn std::error::Error + Send + Sync>> {
    println!("Listing Camoufox processes...");

    // For the list command, we need to provide dummy executable-path and profile-path
    // even though they're not used by the list action
    let sidecar = self
      .app_handle
      .shell()
      .sidecar("nodecar")
      .map_err(|e| format!("Failed to create nodecar sidecar: {e}"))?
      .arg("camoufox")
      .arg("list")
      .arg("--executable-path")
      .arg("/dummy/path") // Dummy path since list doesn't use it
      .arg("--profile-path")
      .arg("/dummy/profile"); // Dummy path since list doesn't use it

    let output = sidecar
      .output()
      .await
      .map_err(|e| format!("Failed to execute nodecar list command: {e}"))?;

    if !output.status.success() {
      let error_msg = String::from_utf8_lossy(&output.stderr);
      return Err(format!("Failed to list Camoufox processes: {error_msg}").into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("List command result: {stdout}");

    // Parse the response as an array of process info
    let processes: Vec<serde_json::Value> =
      serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse list response: {e}"))?;

    // Convert to CamoufoxLaunchResult format
    let mut results = Vec::new();
    for process in processes {
      // Handle both camelCase and snake_case formats from nodecar
      let id = process.get("id").and_then(|v| v.as_str());

      // Try both formats for executable path
      let executable_path = process
        .get("executable_path")
        .and_then(|v| v.as_str())
        .or_else(|| process.get("executablePath").and_then(|v| v.as_str()));

      // Try both formats for profile path
      let profile_path = process
        .get("profile_path")
        .and_then(|v| v.as_str())
        .or_else(|| process.get("profilePath").and_then(|v| v.as_str()));

      if let Some(id) = id {
        let pid = process
          .get("pid")
          .and_then(|v| v.as_u64())
          .map(|v| v as u32);

        let url = process
          .get("url")
          .and_then(|v| v.as_str())
          .map(|s| s.to_string());

        // Use empty strings if executable_path or profile_path are missing
        let executable_path = executable_path.unwrap_or("");
        let profile_path = profile_path.unwrap_or("");

        results.push(CamoufoxLaunchResult {
          id: id.to_string(),
          pid,
          executablePath: executable_path.to_string(),
          profilePath: profile_path.to_string(),
          url,
        });
      } else {
        println!("Skipping malformed process entry: {process:?}");
      }
    }

    println!("Parsed {} valid Camoufox processes", results.len());
    Ok(results)
  }

  /// Find Camoufox process by profile path (for integration with browser_runner)
  pub async fn find_camoufox_by_profile(
    &self,
    profile_path: &str,
  ) -> Result<Option<CamoufoxLaunchResult>, Box<dyn std::error::Error + Send + Sync>> {
    println!("Looking for Camoufox process with profile path: {profile_path}");

    let processes = self.list_camoufox_processes().await?;
    println!("Found {} running Camoufox processes", processes.len());

    for process in &processes {
      println!(
        "Checking process with profile path: {}",
        process.profilePath
      );
    }

    // Convert both paths to canonical form for comparison
    let target_path = std::path::Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| std::path::Path::new(profile_path).to_path_buf());

    for process in &processes {
      println!(
        "Comparing target path: {} with process path: {}",
        target_path.display(),
        process.profilePath
      );

      // Try multiple comparison methods
      let process_path = std::path::Path::new(&process.profilePath)
        .canonicalize()
        .unwrap_or_else(|_| std::path::Path::new(&process.profilePath).to_path_buf());

      // Method 1: Canonical path comparison
      if process_path == target_path {
        println!("Found match using canonical path comparison");
        return Ok(Some(process.clone()));
      }

      // Method 2: Direct string comparison
      if process.profilePath == profile_path {
        println!("Found match using direct string comparison");
        return Ok(Some(process.clone()));
      }

      // Method 3: Compare as strings after canonicalization
      if process_path.to_string_lossy() == target_path.to_string_lossy() {
        println!("Found match using canonical string comparison");
        return Ok(Some(process.clone()));
      }

      // Method 4: Compare file names if full paths don't match
      if let (Some(process_file), Some(target_file)) =
        (process_path.file_name(), target_path.file_name())
      {
        if process_file == target_file {
          // If the parent directories also match, it's likely the same profile
          if let (Some(process_parent), Some(target_parent)) =
            (process_path.parent(), target_path.parent())
          {
            if process_parent == target_parent {
              println!("Found match using parent directory and file name comparison");
              return Ok(Some(process.clone()));
            }
          }
        }
      }

      // Method 5: Check if either path contains the other (for symlinks or different representations)
      let process_path_str = process_path.to_string_lossy();
      let target_path_str = target_path.to_string_lossy();

      if process_path_str.contains(target_path_str.as_ref())
        || target_path_str.contains(process_path_str.as_ref())
      {
        println!("Found match using path containment check");
        return Ok(Some(process.clone()));
      }
    }

    println!("No matching Camoufox process found for profile path: {profile_path}");
    Ok(None)
  }
}

pub async fn launch_camoufox_profile(
  app_handle: AppHandle,
  profile: BrowserProfile,
  config: CamoufoxConfig,
  url: Option<String>,
) -> Result<CamoufoxLaunchResult, String> {
  let launcher = CamoufoxLauncher::new(app_handle);

  // Get the executable path for Camoufox
  let browser_runner = crate::browser_runner::BrowserRunner::new();
  let binaries_dir = browser_runner.get_binaries_dir();
  let browser_dir = binaries_dir.join("camoufox").join(&profile.version);

  // Get executable path
  let browser = crate::browser::create_browser(crate::browser::BrowserType::Camoufox);
  let executable_path = browser
    .get_executable_path(&browser_dir)
    .map_err(|e| format!("Failed to get Camoufox executable path: {e}"))?;

  // Get profile path
  let profiles_dir = browser_runner.get_profiles_dir();
  let profile_path = profile.get_profile_data_path(&profiles_dir);

  launcher
    .launch_camoufox(
      &executable_path.to_string_lossy(),
      &profile_path.to_string_lossy(),
      &config,
      url.as_deref(),
    )
    .await
    .map_err(|e| format!("Failed to launch Camoufox: {e}"))
}

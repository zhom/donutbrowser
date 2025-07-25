use crate::browser_runner::BrowserProfile;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use sysinfo::{Pid, System};
use tauri::AppHandle;
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

#[derive(Debug)]
struct CamoufoxInstance {
  pid: u32,
  executable_path: String,
  profile_path: String,
  url: Option<String>,
  _child: Option<Child>, // Keep handle to prevent zombie processes
}

struct CamoufoxDirectLauncherInner {
  instances: HashMap<String, CamoufoxInstance>,
}

pub struct CamoufoxDirectLauncher {
  inner: Arc<AsyncMutex<CamoufoxDirectLauncherInner>>,
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref GLOBAL_DIRECT_LAUNCHER: CamoufoxDirectLauncher = CamoufoxDirectLauncher::new_singleton();
}

impl CamoufoxDirectLauncher {
  pub fn new(_app_handle: AppHandle) -> Self {
    // Return a reference to the global singleton
    GLOBAL_DIRECT_LAUNCHER.clone()
  }

  pub fn new_singleton() -> Self {
    Self {
      inner: Arc::new(AsyncMutex::new(CamoufoxDirectLauncherInner {
        instances: HashMap::new(),
      })),
    }
  }

  fn clone(&self) -> Self {
    Self {
      inner: Arc::clone(&self.inner),
    }
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

      ..Default::default()
    }
  }

  /// Generate Camoufox configuration using nodecar with camoufox-js-lsd
  async fn generate_camoufox_config_with_nodecar(
    &self,
    config: &CamoufoxConfig,
  ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    println!("Generating Camoufox configuration using nodecar with camoufox-js-lsd...");

    // Build nodecar command arguments
    let mut args = vec!["camoufox-config".to_string(), "generate".to_string()];

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

    // Get the nodecar binary path
    let nodecar_path = self.get_nodecar_binary_path()?;

    println!(
      "Executing nodecar command: {:?} with args: {:?}",
      nodecar_path, args
    );

    // Execute nodecar command
    let output = tokio::process::Command::new(nodecar_path)
      .args(&args)
      .output()
      .await?;

    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      return Err(format!("nodecar camoufox-config failed: {stderr}").into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("nodecar output: {}", stdout);

    // Parse the JSON output
    let config_json: serde_json::Value = serde_json::from_str(&stdout)
      .map_err(|e| format!("Failed to parse nodecar output as JSON: {e}"))?;

    Ok(config_json)
  }

  /// Get the path to the nodecar binary
  fn get_nodecar_binary_path(
    &self,
  ) -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    // Try to find nodecar binary in the same directory as the current executable
    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe
      .parent()
      .ok_or("Failed to get executable directory")?;

    // Check for nodecar in the same directory
    let nodecar_path = exe_dir.join("nodecar");
    if nodecar_path.exists() {
      return Ok(nodecar_path);
    }

    // Check for nodecar with .exe extension on Windows
    #[cfg(target_os = "windows")]
    {
      let nodecar_exe_path = exe_dir.join("nodecar.exe");
      if nodecar_exe_path.exists() {
        return Ok(nodecar_exe_path);
      }
    }

    // Fallback to system PATH
    Ok(std::path::PathBuf::from("nodecar"))
  }

  /// Build the CAMOU_CONFIG JSON from CamoufoxConfig (fallback method)
  fn build_camou_config(&self, config: &CamoufoxConfig) -> serde_json::Value {
    let mut camou_config = serde_json::Map::new();

    // Always set some basic anti-fingerprinting defaults to ensure the system works
    camou_config.insert("debug".to_string(), serde_json::Value::Bool(true)); // Enable debug for troubleshooting

    // Set some default values that should always work to test the system
    if config.timezone.is_none() {
      camou_config.insert(
        "timezone".to_string(),
        serde_json::Value::String("America/New_York".to_string()),
      );
    }

    // Set default screen size if not specified
    if config.screen_min_width.is_none() {
      camou_config.insert(
        "screen.width".to_string(),
        serde_json::Value::Number(1920.into()),
      );
      camou_config.insert(
        "screen.availWidth".to_string(),
        serde_json::Value::Number(1920.into()),
      );
    }
    if config.screen_min_height.is_none() {
      camou_config.insert(
        "screen.height".to_string(),
        serde_json::Value::Number(1080.into()),
      );
      camou_config.insert(
        "screen.availHeight".to_string(),
        serde_json::Value::Number(1080.into()),
      );
    }

    // Set default window size if not specified
    if config.window_width.is_none() {
      camou_config.insert(
        "window.outerWidth".to_string(),
        serde_json::Value::Number(1366.into()),
      );
      camou_config.insert(
        "window.innerWidth".to_string(),
        serde_json::Value::Number(1350.into()),
      );
    }
    if config.window_height.is_none() {
      camou_config.insert(
        "window.outerHeight".to_string(),
        serde_json::Value::Number(768.into()),
      );
      camou_config.insert(
        "window.innerHeight".to_string(),
        serde_json::Value::Number(668.into()),
      );
    }

    // Screen dimensions - use proper camoufox format
    if let Some(width) = config.screen_min_width {
      camou_config.insert(
        "screen.width".to_string(),
        serde_json::Value::Number(width.into()),
      );
      camou_config.insert(
        "screen.availWidth".to_string(),
        serde_json::Value::Number(width.into()),
      );
    }
    if let Some(height) = config.screen_min_height {
      camou_config.insert(
        "screen.height".to_string(),
        serde_json::Value::Number(height.into()),
      );
      camou_config.insert(
        "screen.availHeight".to_string(),
        serde_json::Value::Number(height.into()),
      );
    }

    // Window dimensions - use proper camoufox format
    if let Some(width) = config.window_width {
      camou_config.insert(
        "window.outerWidth".to_string(),
        serde_json::Value::Number(width.into()),
      );
      camou_config.insert(
        "window.innerWidth".to_string(),
        serde_json::Value::Number((width.saturating_sub(16)).into()), // Account for scrollbar
      );
    }
    if let Some(height) = config.window_height {
      camou_config.insert(
        "window.outerHeight".to_string(),
        serde_json::Value::Number(height.into()),
      );
      camou_config.insert(
        "window.innerHeight".to_string(),
        serde_json::Value::Number((height.saturating_sub(100)).into()), // Account for browser chrome
      );
    }

    // Geolocation - use proper camoufox format (colon notation)
    if let Some(latitude) = config.latitude {
      camou_config.insert(
        "geolocation:latitude".to_string(),
        serde_json::Value::Number(
          serde_json::Number::from_f64(latitude).unwrap_or(serde_json::Number::from(0)),
        ),
      );
    }
    if let Some(longitude) = config.longitude {
      camou_config.insert(
        "geolocation:longitude".to_string(),
        serde_json::Value::Number(
          serde_json::Number::from_f64(longitude).unwrap_or(serde_json::Number::from(0)),
        ),
      );
    }

    // Timezone - use proper camoufox format
    if let Some(timezone) = &config.timezone {
      camou_config.insert(
        "timezone".to_string(),
        serde_json::Value::String(timezone.clone()),
      );
    }

    // Locale - use proper camoufox format (colon notation)
    if let Some(locale_list) = &config.locale {
      if let Some(first_locale) = locale_list.first() {
        // Parse locale (e.g., "en-US" -> language: "en", region: "US")
        let parts: Vec<&str> = first_locale.split('-').collect();
        if parts.len() >= 2 {
          camou_config.insert(
            "locale:language".to_string(),
            serde_json::Value::String(parts[0].to_string()),
          );
          camou_config.insert(
            "locale:region".to_string(),
            serde_json::Value::String(parts[1].to_string()),
          );
        }

        // Set the full locale
        camou_config.insert(
          "locale:all".to_string(),
          serde_json::Value::String(first_locale.clone()),
        );

        // Set navigator language properties
        camou_config.insert(
          "navigator.language".to_string(),
          serde_json::Value::String(first_locale.clone()),
        );

        // Set Accept-Language header
        camou_config.insert(
          "headers.Accept-Language".to_string(),
          serde_json::Value::String(first_locale.clone()),
        );

        // Convert to languages array for navigator.languages
        let languages: Vec<serde_json::Value> = locale_list
          .iter()
          .map(|l| serde_json::Value::String(l.clone()))
          .collect();
        camou_config.insert(
          "navigator.languages".to_string(),
          serde_json::Value::Array(languages),
        );
      }
    }

    // WebGL - use proper camoufox format (colon notation)
    if let Some(vendor) = &config.webgl_vendor {
      camou_config.insert(
        "webGl:vendor".to_string(),
        serde_json::Value::String(vendor.clone()),
      );
    }
    if let Some(renderer) = &config.webgl_renderer {
      camou_config.insert(
        "webGl:renderer".to_string(),
        serde_json::Value::String(renderer.clone()),
      );
    }

    // Fonts - use proper camoufox format
    if let Some(fonts) = &config.fonts {
      let font_values: Vec<serde_json::Value> = fonts
        .iter()
        .map(|f| serde_json::Value::String(f.clone()))
        .collect();
      camou_config.insert("fonts".to_string(), serde_json::Value::Array(font_values));
    }

    // Custom fonts only
    if let Some(custom_fonts_only) = config.custom_fonts_only {
      camou_config.insert(
        "customFontsOnly".to_string(),
        serde_json::Value::Bool(custom_fonts_only),
      );
    }

    // Humanization - use proper camoufox format (colon notation)
    if let Some(humanize) = config.humanize {
      camou_config.insert("humanize".to_string(), serde_json::Value::Bool(humanize));
      if let Some(duration) = config.humanize_duration {
        camou_config.insert(
          "humanize:maxTime".to_string(),
          serde_json::Value::Number(
            serde_json::Number::from_f64(duration * 1000.0).unwrap_or(serde_json::Number::from(0)), // Convert to milliseconds
          ),
        );
      }
    }

    // Debug mode
    if let Some(debug) = config.debug {
      camou_config.insert("debug".to_string(), serde_json::Value::Bool(debug));
    }

    // Main world evaluation
    if let Some(main_world_eval) = config.main_world_eval {
      camou_config.insert(
        "allowMainWorld".to_string(),
        serde_json::Value::Bool(main_world_eval),
      );
    }

    // Addons
    if let Some(addons) = &config.addons {
      let addon_values: Vec<serde_json::Value> = addons
        .iter()
        .map(|a| serde_json::Value::String(a.clone()))
        .collect();
      camou_config.insert("addons".to_string(), serde_json::Value::Array(addon_values));
    }

    // Exclude addons
    if let Some(exclude_addons) = &config.exclude_addons {
      let exclude_addon_values: Vec<serde_json::Value> = exclude_addons
        .iter()
        .map(|a| serde_json::Value::String(a.clone()))
        .collect();
      camou_config.insert(
        "excludeAddons".to_string(),
        serde_json::Value::Array(exclude_addon_values),
      );
    }

    // Block features
    if let Some(block_images) = config.block_images {
      camou_config.insert(
        "blockImages".to_string(),
        serde_json::Value::Bool(block_images),
      );
    }
    if let Some(block_webrtc) = config.block_webrtc {
      camou_config.insert(
        "blockWebRTC".to_string(),
        serde_json::Value::Bool(block_webrtc),
      );
    }
    if let Some(block_webgl) = config.block_webgl {
      camou_config.insert(
        "blockWebGL".to_string(),
        serde_json::Value::Bool(block_webgl),
      );
    }

    // COOP disable
    if let Some(disable_coop) = config.disable_coop {
      camou_config.insert(
        "disableCOOP".to_string(),
        serde_json::Value::Bool(disable_coop),
      );
    }

    // GeoIP
    if let Some(geoip) = &config.geoip {
      camou_config.insert("geoip".to_string(), geoip.clone());
    }

    // Country
    if let Some(country) = &config.country {
      camou_config.insert(
        "country".to_string(),
        serde_json::Value::String(country.clone()),
      );
    }

    // Firefox version
    if let Some(ff_version) = config.ff_version {
      camou_config.insert(
        "ffVersion".to_string(),
        serde_json::Value::Number(ff_version.into()),
      );
    }

    // Enable cache
    if let Some(enable_cache) = config.enable_cache {
      camou_config.insert(
        "enableCache".to_string(),
        serde_json::Value::Bool(enable_cache),
      );
    }

    // Proxy configuration
    if let Some(proxy) = &config.proxy {
      camou_config.insert(
        "proxy".to_string(),
        serde_json::Value::String(proxy.clone()),
      );
    }

    // Firefox preferences
    if let Some(firefox_prefs) = &config.firefox_prefs {
      let mut prefs_obj = serde_json::Map::new();
      for (key, value) in firefox_prefs {
        prefs_obj.insert(key.clone(), value.clone());
      }
      camou_config.insert(
        "firefoxPrefs".to_string(),
        serde_json::Value::Object(prefs_obj),
      );
    }

    camou_config.insert("showcursor".to_string(), serde_json::Value::Bool(false));

    camou_config.insert("disableTheming".to_string(), serde_json::Value::Bool(true));

    let final_config = serde_json::Value::Object(camou_config);
    println!(
      "Built CAMOU_CONFIG: {}",
      serde_json::to_string_pretty(&final_config).unwrap_or_default()
    );

    // Validate that we have some basic anti-fingerprinting settings
    let config_obj = final_config.as_object().unwrap();
    let has_timezone = config_obj.contains_key("timezone");
    let has_screen =
      config_obj.contains_key("screen.width") || config_obj.contains_key("screen.height");
    let has_window =
      config_obj.contains_key("window.outerWidth") || config_obj.contains_key("window.outerHeight");

    println!("Anti-fingerprinting validation:");
    println!("  - Has timezone: {has_timezone}");
    println!("  - Has screen dimensions: {has_screen}");
    println!("  - Has window dimensions: {has_window}");

    if !has_timezone && !has_screen && !has_window {
      println!(
        "WARNING: No anti-fingerprinting settings detected! Camoufox may not work as expected."
      );
    }

    final_config
  }

  /// Launch Camoufox browser with the specified configuration using direct process management
  pub async fn launch_camoufox(
    &self,
    executable_path: &str,
    profile_path: &str,
    config: &CamoufoxConfig,
    url: Option<&str>,
  ) -> Result<CamoufoxLaunchResult, Box<dyn std::error::Error + Send + Sync>> {
    println!("Launching Camoufox directly with executable: {executable_path}");
    println!("Profile path: {profile_path}");
    println!("URL: {url:?}");

    // Generate unique ID for this instance
    let instance_id = uuid::Uuid::new_v4().to_string();

    // Try to generate configuration using nodecar first, fallback to manual build
    let camou_config_json = match self.generate_camoufox_config_with_nodecar(config).await {
      Ok(config) => {
        println!("✅ Successfully generated Camoufox config using nodecar with camoufox-js-lsd");
        config
      }
      Err(e) => {
        println!("⚠️ Failed to generate config with nodecar, falling back to manual build: {e}");
        self.build_camou_config(config)
      }
    };

    // Build command arguments
    let mut args = vec!["-profile".to_string(), profile_path.to_string()];

    // Add URL if provided
    if let Some(url) = url {
      args.push(url.to_string());
    }

    // Add headless mode if specified
    // if config.headless.unwrap_or(false) {
    //   args.push("-headless".to_string());
    // }

    // Add additional arguments
    if let Some(additional_args) = &config.additional_args {
      args.extend(additional_args.clone());
    }

    // Extract the env object from the generated config if it exists
    let mut final_env_vars = std::collections::HashMap::new();
    if let Some(env_obj) = camou_config_json.get("env") {
      if let Some(env_map) = env_obj.as_object() {
        for (key, value) in env_map {
          let value_str = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => value.to_string(),
          };
          final_env_vars.insert(key.clone(), value_str);
        }
      }
    }

    // Add user-specified environment variables (they override generated ones)
    if let Some(user_env_vars) = &config.env_vars {
      for (key, value) in user_env_vars {
        final_env_vars.insert(key.clone(), value.clone());
      }
    }

    // Remove the env key from the config JSON since we'll set it as actual env vars
    let mut config_for_env = camou_config_json.clone();
    if let Some(config_obj) = config_for_env.as_object_mut() {
      config_obj.remove("env");
    }
    let camou_config_str = config_for_env.to_string();

    // Set CAMOU_CONFIG environment variable - this is crucial for anti-fingerprinting
    println!(
      "Setting CAMOU_CONFIG environment variable: {}",
      camou_config_str
    );

    // Build environment variables
    let mut cmd = Command::new(executable_path);
    cmd.args(&args);

    // Don't suppress stderr in debug mode so we can see Camoufox error messages
    if config.debug.unwrap_or(false) {
      println!("Debug mode enabled - keeping stderr output for troubleshooting");
    } else {
      cmd.stdout(Stdio::null());
      cmd.stderr(Stdio::null());
    }

    // CRITICAL: Add cache-busting environment variables to force Camoufox config refresh
    // This works around the std::call_once limitation in MaskConfig.hpp
    let timestamp = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_nanos();

    let cache_buster = format!("{}_{}", std::process::id(), timestamp);

    // Multiple cache-busting strategies to ensure config refresh
    cmd.env("CAMOU_CACHE_INVALIDATE", &cache_buster);
    cmd.env("CAMOU_CONFIG_REFRESH", &timestamp.to_string());
    cmd.env("CAMOU_PROCESS_ISOLATION", &cache_buster);

    // Force Camoufox to treat this as a completely new process context
    cmd.env("CAMOU_FORCE_CONFIG_RELOAD", "1");
    cmd.env("CAMOU_DISABLE_CONFIG_CACHE", "1");

    println!(
      "Setting cache-busting environment variables with timestamp: {}",
      timestamp
    );

    // Check if the config string is too large for a single environment variable
    const MAX_ENV_SIZE: usize = 2000;

    if camou_config_str.len() > MAX_ENV_SIZE {
      // Split into multiple environment variables
      let chunks: Vec<&str> = camou_config_str
        .as_bytes()
        .chunks(MAX_ENV_SIZE)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or(""))
        .collect();

      for (i, chunk) in chunks.iter().enumerate() {
        let env_name = format!("CAMOU_CONFIG_{}", i + 1);
        println!(
          "Setting {} (chunk {} of {}): {} bytes",
          env_name,
          i + 1,
          chunks.len(),
          chunk.len()
        );
        cmd.env(&env_name, chunk);
      }
    } else {
      // Use single environment variable
      cmd.env("CAMOU_CONFIG", &camou_config_str);
    }

    // Set working directory to the executable's directory for better compatibility
    if let Some(parent_dir) = std::path::Path::new(executable_path).parent() {
      cmd.current_dir(parent_dir);
      println!("Set working directory to: {:?}", parent_dir);
    }

    // Set all environment variables from the generated config
    for (key, value) in &final_env_vars {
      println!("Setting generated environment variable: {}={}", key, value);
      cmd.env(key, value);
    }

    // Add user-specified environment variables (they override generated ones)
    if let Some(user_env_vars) = &config.env_vars {
      for (key, value) in user_env_vars {
        println!("Setting user environment variable: {}={}", key, value);
        cmd.env(key, value);
      }
    }

    // Set virtual display if specified
    if let Some(virtual_display) = &config.virtual_display {
      println!("Setting DISPLAY environment variable: {}", virtual_display);
      cmd.env("DISPLAY", virtual_display);
    }

    // Debug: Print launch information
    println!("=== Camoufox Launch Debug Info ===");
    println!("Executable: {}", executable_path);
    println!("Arguments: {:?}", args);
    println!("CAMOU_CONFIG length: {} bytes", camou_config_str.len());

    // Verify the JSON is valid
    match serde_json::from_str::<serde_json::Value>(&camou_config_str) {
      Ok(parsed) => {
        println!("✅ CAMOU_CONFIG JSON is valid");
        if let Some(obj) = parsed.as_object() {
          println!("📊 Config contains {} keys:", obj.len());
          for key in obj.keys() {
            println!("   - {}", key);
          }
        }
      }
      Err(e) => {
        println!("❌ CAMOU_CONFIG JSON is invalid: {}", e);
      }
    }

    // Launch the process
    let child = cmd
      .spawn()
      .map_err(|e| format!("Failed to launch Camoufox process: {e}"))?;

    let pid = child.id();
    println!("Launched Camoufox with PID: {pid}");

    // Store the instance
    let instance = CamoufoxInstance {
      pid,
      executable_path: executable_path.to_string(),
      profile_path: profile_path.to_string(),
      url: url.map(|u| u.to_string()),
      _child: Some(child),
    };

    {
      let mut inner = self.inner.lock().await;
      inner.instances.insert(instance_id.clone(), instance);
    }

    // Return launch result
    Ok(CamoufoxLaunchResult {
      id: instance_id,
      pid: Some(pid),
      executablePath: executable_path.to_string(),
      profilePath: profile_path.to_string(),
      url: url.map(|u| u.to_string()),
    })
  }

  /// Stop a Camoufox process by ID
  pub async fn stop_camoufox(
    &self,
    id: &str,
  ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    println!("Stopping Camoufox process with ID: {id}");

    let instance = {
      let mut inner = self.inner.lock().await;
      inner.instances.remove(id)
    };

    if let Some(mut instance) = instance {
      // Try to kill the process gracefully first
      let system = System::new_all();
      if let Some(process) = system.process(Pid::from(instance.pid as usize)) {
        if process.kill() {
          println!(
            "Successfully killed Camoufox process: {id} (PID: {})",
            instance.pid
          );
        } else {
          println!(
            "Failed to kill Camoufox process: {id} (PID: {})",
            instance.pid
          );
        }
      }

      // Also try to kill the child process if we still have a handle
      if let Some(ref mut child) = instance._child {
        let _ = child.kill();
      }

      Ok(true)
    } else {
      println!("Camoufox process with ID {id} not found");
      Ok(false)
    }
  }

  /// Find Camoufox process by profile path (for integration with browser_runner)
  pub async fn find_camoufox_by_profile(
    &self,
    profile_path: &str,
  ) -> Result<Option<CamoufoxLaunchResult>, Box<dyn std::error::Error + Send + Sync>> {
    println!("Looking for Camoufox process with profile path: {profile_path}");

    let inner = self.inner.lock().await;

    // Convert paths to canonical form for comparison
    let target_path = Path::new(profile_path)
      .canonicalize()
      .unwrap_or_else(|_| Path::new(profile_path).to_path_buf());

    for (id, instance) in inner.instances.iter() {
      let instance_path = Path::new(&instance.profile_path)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&instance.profile_path).to_path_buf());

      if instance_path == target_path {
        println!("Found match using canonical path comparison");
        return Ok(Some(CamoufoxLaunchResult {
          id: id.clone(),
          pid: Some(instance.pid),
          executablePath: instance.executable_path.clone(),
          profilePath: instance.profile_path.clone(),
          url: instance.url.clone(),
        }));
      }
    }

    println!("No matching Camoufox process found for profile path: {profile_path}");
    Ok(None)
  }

  /// Check if processes are still alive and clean up dead instances
  pub async fn cleanup_dead_instances(
    &self,
  ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut dead_instances = Vec::new();
    let mut instances_to_remove = Vec::new();

    {
      let inner = self.inner.lock().await;
      let system = System::new_all();

      for (id, instance) in inner.instances.iter() {
        // Check if the process is still alive
        if let Some(_process) = system.process(Pid::from(instance.pid as usize)) {
          // Process is still alive
          continue;
        } else {
          // Process is dead
          println!(
            "Detected dead Camoufox instance: {} (PID: {})",
            id, instance.pid
          );
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
      }
      println!(
        "Cleaned up {} dead Camoufox instances",
        instances_to_remove.len()
      );
    }

    Ok(dead_instances)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn test_nodecar_config_generation() {
    let launcher = CamoufoxDirectLauncher::new_singleton();

    // Test with empty config (should generate random config)
    let empty_config = CamoufoxConfig::default();
    let empty_result = launcher
      .generate_camoufox_config_with_nodecar(&empty_config)
      .await;

    match empty_result {
      Ok(config) => {
        println!("✅ Empty config test passed");

        // Check if it has essential properties
        if let Some(obj) = config.as_object() {
          let has_navigator_ua = obj.contains_key("navigator.userAgent");
          let has_screen_width = obj.contains_key("screen.width");
          let has_timezone = obj.contains_key("timezone");

          // At least one of these should be present in a valid config
          assert!(
            has_navigator_ua || has_screen_width || has_timezone,
            "Generated config should have at least one fingerprinting property"
          );
        }
      }
      Err(e) => {
        // This is expected if nodecar is not available in test environment
        println!("⚠️ Nodecar not available in test environment: {}", e);
      }
    }

    // Test with configured values
    let test_config = CamoufoxDirectLauncher::create_test_config();
    let test_result = launcher
      .generate_camoufox_config_with_nodecar(&test_config)
      .await;

    match test_result {
      Ok(config) => {
        println!("✅ Test config generation passed");

        // Verify the config is valid JSON
        assert!(
          config.is_object(),
          "Generated config should be a JSON object"
        );

        // Check if user settings might be respected (this depends on nodecar being available)
        if let Some(obj) = config.as_object() {
          // At least verify we got a valid config structure
          assert!(!obj.is_empty(), "Generated config should not be empty");
        }
      }
      Err(e) => {
        println!("⚠️ Nodecar not available for test config: {}", e);
      }
    }
  }

  #[test]
  fn test_camoufox_config_creation() {
    let test_config = CamoufoxDirectLauncher::create_test_config();

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
  fn test_fallback_config_generation() {
    let launcher = CamoufoxDirectLauncher::new_singleton();

    let test_config = CamoufoxDirectLauncher::create_test_config();
    let fallback_config = launcher.build_camou_config(&test_config);

    // Verify fallback config structure
    assert!(
      fallback_config.is_object(),
      "Fallback config should be a JSON object"
    );

    let config_obj = fallback_config.as_object().unwrap();

    // Check essential anti-fingerprinting properties
    assert!(config_obj.contains_key("timezone"), "Should have timezone");
    assert!(
      config_obj.contains_key("screen.width"),
      "Should have screen width"
    );
    assert!(
      config_obj.contains_key("window.outerWidth"),
      "Should have window width"
    );
    assert!(config_obj.contains_key("debug"), "Should have debug flag");

    // Verify specific values
    assert_eq!(
      config_obj.get("timezone").unwrap().as_str().unwrap(),
      "Europe/London"
    );
    assert_eq!(
      config_obj.get("screen.width").unwrap().as_u64().unwrap(),
      1440
    );
    assert_eq!(
      config_obj
        .get("window.outerWidth")
        .unwrap()
        .as_u64()
        .unwrap(),
      1200
    );
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

pub async fn launch_camoufox_profile_direct(
  app_handle: AppHandle,
  profile: BrowserProfile,
  config: CamoufoxConfig,
  url: Option<String>,
) -> Result<CamoufoxLaunchResult, String> {
  let launcher = CamoufoxDirectLauncher::new(app_handle);

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

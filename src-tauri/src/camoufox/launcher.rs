//! Camoufox browser launcher using playwright-rust.
//!
//! Provides functionality to launch Camoufox browser instances with fingerprint injection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use playwright::api::{Browser, BrowserContext, Playwright, ProxySettings};
use playwright::Error as PlaywrightError;

use crate::camoufox::config::{CamoufoxConfigBuilder, CamoufoxLaunchConfig, ProxyConfig};
use crate::camoufox::fingerprint::types::{Fingerprint, ScreenConstraints};

/// Camoufox launcher for creating browser instances.
pub struct CamoufoxLauncher {
  playwright: Arc<Playwright>,
  executable_path: PathBuf,
}

/// Error type for launcher operations.
#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
  #[error("Playwright error: {0}")]
  Playwright(PlaywrightError),

  #[error("Playwright Arc error: {0}")]
  PlaywrightArc(#[from] Arc<PlaywrightError>),

  #[error("Configuration error: {0}")]
  Config(#[from] crate::camoufox::config::ConfigError),

  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  #[error("Camoufox executable not found at: {0}")]
  ExecutableNotFound(PathBuf),

  #[error("Failed to generate environment variables: {0}")]
  EnvVars(#[from] serde_json::Error),
}

/// Options for launching Camoufox.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
  /// Operating system to spoof: "windows", "macos", "linux"
  pub os: Option<String>,
  /// Block all images
  pub block_images: bool,
  /// Block WebRTC entirely
  pub block_webrtc: bool,
  /// Block WebGL (not recommended unless necessary)
  pub block_webgl: bool,
  /// Screen dimension constraints
  pub screen: Option<ScreenConstraints>,
  /// Fixed window size [width, height]
  pub window: Option<(u32, u32)>,
  /// Custom fingerprint (if not provided, one will be generated)
  pub fingerprint: Option<Fingerprint>,
  /// Run in headless mode
  pub headless: bool,
  /// Custom fonts to load
  pub fonts: Option<Vec<String>>,
  /// Only use custom fonts (disable OS fonts)
  pub custom_fonts_only: bool,
  /// Firefox user preferences
  pub firefox_user_prefs: Option<HashMap<String, serde_json::Value>>,
  /// Proxy configuration
  pub proxy: Option<ProxyConfig>,
  /// Additional browser arguments
  pub args: Option<Vec<String>>,
  /// Additional environment variables
  pub env: Option<HashMap<String, String>>,
  /// Profile/user data directory
  pub user_data_dir: Option<PathBuf>,
  /// Enable debug output
  pub debug: bool,
}

impl CamoufoxLauncher {
  /// Create a new Camoufox launcher.
  pub async fn new(executable_path: impl AsRef<Path>) -> Result<Self, LauncherError> {
    let executable_path = executable_path.as_ref().to_path_buf();

    if !executable_path.exists() {
      return Err(LauncherError::ExecutableNotFound(executable_path));
    }

    let playwright = Playwright::initialize()
      .await
      .map_err(LauncherError::Playwright)?;

    Ok(Self {
      playwright: Arc::new(playwright),
      executable_path,
    })
  }

  /// Launch a new Camoufox browser instance.
  pub async fn launch(&self, options: LaunchOptions) -> Result<Browser, LauncherError> {
    let config = self.build_config(&options)?;

    if options.debug {
      log::debug!("Camoufox config: {:?}", config.fingerprint_config);
    }

    // Get environment variables
    let env_vars = config.get_env_vars()?;

    // Build launch arguments
    let mut args = options.args.clone().unwrap_or_default();

    // Add headless flag if needed
    if options.headless {
      args.push("--headless".to_string());
    }

    // Merge environment variables
    let mut env = options.env.clone().unwrap_or_default();
    for (key, value) in env_vars {
      env.insert(key, value);
    }

    // Handle fontconfig on Linux
    if cfg!(target_os = "linux") {
      if let Some(fontconfig_path) =
        crate::camoufox::env_vars::get_fontconfig_env(&config.target_os, &self.executable_path)
      {
        env.insert("FONTCONFIG_PATH".to_string(), fontconfig_path);
      }
    }

    // Build Firefox user prefs
    let mut firefox_prefs = config.firefox_prefs.clone();
    if let Some(user_prefs) = options.firefox_user_prefs {
      for (key, value) in user_prefs {
        firefox_prefs.insert(key, value);
      }
    }

    // Get the Firefox browser type
    let firefox = self.playwright.firefox();

    // Build launch options
    let mut launch_options = firefox.launcher();
    launch_options = launch_options.executable(&self.executable_path);
    launch_options = launch_options.headless(options.headless);

    // Add args
    if !args.is_empty() {
      launch_options = launch_options.args(&args);
    }

    // Add environment as serde_json::Map
    if !env.is_empty() {
      let env_map: serde_json::Map<String, serde_json::Value> = env
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
      launch_options = launch_options.env(env_map);
    }

    // Add proxy if configured
    if let Some(proxy) = &config.proxy {
      let proxy_settings = ProxySettings {
        server: proxy.server.clone(),
        username: proxy.username.clone(),
        password: proxy.password.clone(),
        bypass: proxy.bypass.clone(),
      };
      launch_options = launch_options.proxy(proxy_settings);
    }

    // Add Firefox preferences
    if !firefox_prefs.is_empty() {
      let prefs_map: serde_json::Map<String, serde_json::Value> =
        firefox_prefs.into_iter().collect();
      launch_options = launch_options.firefox_user_prefs(prefs_map);
    }

    // Launch the browser
    let browser = launch_options.launch().await?;

    Ok(browser)
  }

  /// Launch a persistent browser context.
  pub async fn launch_persistent_context(
    &self,
    user_data_dir: impl AsRef<Path>,
    options: LaunchOptions,
  ) -> Result<BrowserContext, LauncherError> {
    let config = self.build_config(&options)?;

    if options.debug {
      log::debug!("Camoufox config: {:?}", config.fingerprint_config);
    }

    // Get environment variables
    let env_vars = config.get_env_vars()?;

    // Build launch arguments
    let mut args = options.args.clone().unwrap_or_default();

    if options.headless {
      args.push("--headless".to_string());
    }

    // Merge environment variables
    let mut env = options.env.clone().unwrap_or_default();
    for (key, value) in env_vars {
      env.insert(key, value);
    }

    // Handle fontconfig on Linux
    if cfg!(target_os = "linux") {
      if let Some(fontconfig_path) =
        crate::camoufox::env_vars::get_fontconfig_env(&config.target_os, &self.executable_path)
      {
        env.insert("FONTCONFIG_PATH".to_string(), fontconfig_path);
      }
    }

    // Build Firefox user prefs
    let mut firefox_prefs = config.firefox_prefs.clone();
    if let Some(user_prefs) = options.firefox_user_prefs {
      for (key, value) in user_prefs {
        firefox_prefs.insert(key, value);
      }
    }

    // Get the Firefox browser type
    let firefox = self.playwright.firefox();

    // Build persistent context options
    let mut context_options = firefox.persistent_context_launcher(user_data_dir.as_ref());
    context_options = context_options.executable(&self.executable_path);
    context_options = context_options.headless(options.headless);

    // Add args
    if !args.is_empty() {
      context_options = context_options.args(&args);
    }

    // Add environment as serde_json::Map
    if !env.is_empty() {
      let env_map: serde_json::Map<String, serde_json::Value> = env
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
      context_options = context_options.env(env_map);
    }

    // Add proxy if configured
    if let Some(proxy) = &config.proxy {
      let proxy_settings = ProxySettings {
        server: proxy.server.clone(),
        username: proxy.username.clone(),
        password: proxy.password.clone(),
        bypass: proxy.bypass.clone(),
      };
      context_options = context_options.proxy(proxy_settings);
    }

    // Note: PersistentContextLauncher doesn't support firefox_user_prefs
    // Firefox preferences should be set via about:config or prefs.js in the profile

    // Launch the persistent context
    let context = context_options.launch().await?;

    Ok(context)
  }

  /// Build Camoufox configuration from launch options.
  fn build_config(&self, options: &LaunchOptions) -> Result<CamoufoxLaunchConfig, LauncherError> {
    let mut builder = CamoufoxConfigBuilder::new();

    if let Some(os) = &options.os {
      builder = builder.operating_system(os);
    }

    if let Some(screen) = &options.screen {
      builder = builder.screen_constraints(screen.clone());
    }

    if let Some(fingerprint) = &options.fingerprint {
      builder = builder.fingerprint(fingerprint.clone());
    }

    builder = builder.block_images(options.block_images);
    builder = builder.block_webrtc(options.block_webrtc);
    builder = builder.block_webgl(options.block_webgl);
    builder = builder.headless(options.headless);

    if let Some(fonts) = &options.fonts {
      builder = builder.custom_fonts(fonts.clone());
    }

    builder = builder.custom_fonts_only(options.custom_fonts_only);

    if let Some(proxy) = &options.proxy {
      builder = builder.proxy(proxy.clone());
    }

    // Get Firefox version from executable
    if let Some(version) = crate::camoufox::config::get_firefox_version(&self.executable_path) {
      builder = builder.ff_version(version);
    }

    Ok(builder.build()?)
  }

  /// Get the executable path.
  pub fn executable_path(&self) -> &Path {
    &self.executable_path
  }
}

/// Convenience function to launch Camoufox with default settings.
pub async fn launch_camoufox(
  executable_path: impl AsRef<Path>,
  options: LaunchOptions,
) -> Result<Browser, LauncherError> {
  let launcher = CamoufoxLauncher::new(executable_path).await?;
  launcher.launch(options).await
}

/// Convenience function to launch a persistent Camoufox context.
pub async fn launch_persistent_camoufox(
  executable_path: impl AsRef<Path>,
  user_data_dir: impl AsRef<Path>,
  options: LaunchOptions,
) -> Result<BrowserContext, LauncherError> {
  let launcher = CamoufoxLauncher::new(executable_path).await?;
  launcher
    .launch_persistent_context(user_data_dir, options)
    .await
}

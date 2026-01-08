//! Camoufox browser integration module.
//!
//! Provides native Rust support for launching Camoufox browsers with realistic
//! fingerprint injection using playwright-rust.
//!
//! # Overview
//!
//! This module replaces the previous Node.js-based nodecar implementation with
//! a pure Rust solution. Key components:
//!
//! - **Fingerprint Generation**: Bayesian network-based fingerprint generation
//! - **WebGL Sampling**: Realistic WebGL configurations from a SQLite database
//! - **Configuration Builder**: Converts fingerprints to Camoufox config format
//! - **Launcher**: playwright-rust integration for browser launching
//!
//! # Example
//!
//! ```rust,ignore
//! use donutbrowser_lib::camoufox::{CamoufoxLauncher, LaunchOptions};
//!
//! async fn launch_browser() -> Result<(), Box<dyn std::error::Error>> {
//!     let launcher = CamoufoxLauncher::new("/path/to/camoufox").await?;
//!
//!     let options = LaunchOptions {
//!         os: Some("windows".to_string()),
//!         headless: false,
//!         ..Default::default()
//!     };
//!
//!     let browser = launcher.launch(options).await?;
//!
//!     // Use the browser...
//!
//!     browser.close().await?;
//!     Ok(())
//! }
//! ```

pub mod config;
pub mod data;
pub mod env_vars;
pub mod fingerprint;
pub mod fonts;
pub mod geolocation;
pub mod launcher;
pub mod webgl;

// Re-export main types for convenience
pub use config::{
  CamoufoxConfigBuilder, CamoufoxLaunchConfig, ConfigError, GeoIPOption, ProxyConfig,
};
pub use fingerprint::types::{
  Fingerprint, FingerprintOptions, FingerprintWithHeaders, NavigatorFingerprint, ScreenConstraints,
  ScreenFingerprint, VideoCard,
};
pub use fingerprint::{FingerprintError, FingerprintGenerator};
pub use geolocation::{
  fetch_public_ip, get_geolocation, is_geoip_available, is_ipv4, is_ipv6, validate_ip, Geolocation,
  GeolocationError, Locale, LocaleSelector,
};
pub use launcher::{
  launch_camoufox, launch_persistent_camoufox, CamoufoxLauncher, LaunchOptions, LauncherError,
};
pub use webgl::{sample_webgl, WebGLData, WebGLError};

/// Unified error type for all Camoufox operations.
#[derive(Debug, thiserror::Error)]
pub enum CamoufoxError {
  #[error("Launcher error: {0}")]
  Launcher(#[from] LauncherError),

  #[error("Configuration error: {0}")]
  Config(#[from] ConfigError),

  #[error("Fingerprint error: {0}")]
  Fingerprint(#[from] FingerprintError),

  #[error("WebGL error: {0}")]
  WebGL(#[from] WebGLError),

  #[error("Geolocation error: {0}")]
  Geolocation(#[from] GeolocationError),

  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fingerprint_generation() {
    let generator = FingerprintGenerator::new().unwrap();
    let options = FingerprintOptions {
      browsers: Some(vec!["firefox".to_string()]),
      operating_system: Some("windows".to_string()),
      ..Default::default()
    };

    let result = generator.get_fingerprint(&options);
    assert!(result.is_ok());

    let fp = result.unwrap();
    assert!(!fp.fingerprint.navigator.user_agent.is_empty());
    assert!(fp.fingerprint.screen.width > 0);
  }

  #[test]
  fn test_config_builder() {
    let config = CamoufoxConfigBuilder::new()
      .operating_system("windows")
      .block_images(false)
      .build();

    assert!(config.is_ok());

    let config = config.unwrap();
    assert!(!config.fingerprint_config.is_empty());
    assert!(config
      .fingerprint_config
      .contains_key("navigator.userAgent"));
  }

  #[test]
  fn test_webgl_sampling() {
    let result = webgl::sample_webgl("win", None, None);
    assert!(result.is_ok());

    let webgl_data = result.unwrap();
    assert!(!webgl_data.vendor.is_empty());
    assert!(!webgl_data.renderer.is_empty());
  }

  #[test]
  fn test_fonts() {
    let fonts = fonts::get_fonts_for_os("win");
    assert!(!fonts.is_empty());
    assert!(fonts.contains(&"Arial".to_string()));
  }

  #[test]
  fn test_env_vars() {
    let mut config = std::collections::HashMap::new();
    config.insert(
      "navigator.userAgent".to_string(),
      serde_json::json!("Mozilla/5.0"),
    );

    let env_vars = env_vars::config_to_env_vars(&config).unwrap();
    assert!(!env_vars.is_empty());
    assert!(env_vars.contains_key("CAMOU_CONFIG_1"));
  }
}

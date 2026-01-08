//! Fingerprint type definitions.
//!
//! These types represent browser fingerprints that can be injected into Camoufox.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A complete browser fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
  pub screen: ScreenFingerprint,
  pub navigator: NavigatorFingerprint,
  #[serde(default)]
  pub video_codecs: HashMap<String, String>,
  #[serde(default)]
  pub audio_codecs: HashMap<String, String>,
  #[serde(default)]
  pub plugins_data: HashMap<String, String>,
  #[serde(default)]
  pub battery: Option<BatteryFingerprint>,
  pub video_card: VideoCard,
  #[serde(default)]
  pub multimedia_devices: Vec<String>,
  #[serde(default)]
  pub fonts: Vec<String>,
  #[serde(default)]
  pub mock_web_rtc: bool,
  #[serde(default)]
  pub slim: bool,
}

/// Screen-related fingerprint properties.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScreenFingerprint {
  pub width: u32,
  pub height: u32,
  pub avail_width: u32,
  pub avail_height: u32,
  #[serde(default)]
  pub avail_top: u32,
  #[serde(default)]
  pub avail_left: u32,
  pub color_depth: u32,
  pub pixel_depth: u32,
  #[serde(default = "default_device_pixel_ratio")]
  pub device_pixel_ratio: f64,
  #[serde(default)]
  pub page_x_offset: f64,
  #[serde(default)]
  pub page_y_offset: f64,
  pub inner_width: u32,
  pub inner_height: u32,
  pub outer_width: u32,
  pub outer_height: u32,
  #[serde(default)]
  pub screen_x: i32,
  #[serde(default)]
  pub screen_y: i32,
  #[serde(default)]
  pub client_width: Option<u32>,
  #[serde(default)]
  pub client_height: Option<u32>,
  #[serde(default)]
  pub has_hdr: bool,
}

fn default_device_pixel_ratio() -> f64 {
  1.0
}

/// Brand information for User-Agent Client Hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brand {
  pub brand: String,
  pub version: String,
}

/// User-Agent Client Hints data.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserAgentData {
  #[serde(default)]
  pub brands: Vec<Brand>,
  #[serde(default)]
  pub mobile: bool,
  #[serde(default)]
  pub platform: String,
  #[serde(default)]
  pub architecture: String,
  #[serde(default)]
  pub bitness: String,
  #[serde(default)]
  pub full_version_list: Vec<Brand>,
  #[serde(default)]
  pub model: String,
  #[serde(default)]
  pub platform_version: String,
  #[serde(default)]
  pub ua_full_version: String,
}

/// Extra navigator properties.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtraProperties {
  #[serde(default)]
  pub vendor_flavors: Vec<String>,
  #[serde(default)]
  pub is_bluetooth_supported: bool,
  #[serde(default)]
  pub global_privacy_control: Option<bool>,
  #[serde(default = "default_pdf_viewer_enabled")]
  pub pdf_viewer_enabled: bool,
  #[serde(default)]
  pub installed_apps: Vec<serde_json::Value>,
}

fn default_pdf_viewer_enabled() -> bool {
  true
}

/// Navigator-related fingerprint properties.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NavigatorFingerprint {
  pub user_agent: String,
  #[serde(default)]
  pub user_agent_data: Option<UserAgentData>,
  #[serde(default)]
  pub do_not_track: Option<String>,
  #[serde(default = "default_app_code_name")]
  pub app_code_name: String,
  #[serde(default = "default_app_name")]
  pub app_name: String,
  #[serde(default)]
  pub app_version: String,
  #[serde(default)]
  pub oscpu: Option<String>,
  #[serde(default)]
  pub webdriver: Option<String>,
  pub language: String,
  pub languages: Vec<String>,
  pub platform: String,
  #[serde(default)]
  pub device_memory: Option<u32>,
  pub hardware_concurrency: u32,
  #[serde(default = "default_product")]
  pub product: String,
  #[serde(default)]
  pub product_sub: String,
  #[serde(default)]
  pub vendor: String,
  #[serde(default)]
  pub vendor_sub: String,
  #[serde(default)]
  pub max_touch_points: u32,
  #[serde(default)]
  pub extra_properties: Option<ExtraProperties>,
}

fn default_app_code_name() -> String {
  "Mozilla".to_string()
}

fn default_app_name() -> String {
  "Netscape".to_string()
}

fn default_product() -> String {
  "Gecko".to_string()
}

/// WebGL video card information.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VideoCard {
  pub vendor: String,
  pub renderer: String,
}

/// Battery status fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryFingerprint {
  pub charging: bool,
  pub charging_time: f64,
  pub discharging_time: f64,
  pub level: f64,
}

/// HTTP headers for a fingerprint.
pub type Headers = HashMap<String, String>;

/// A fingerprint combined with matching HTTP headers.
#[derive(Debug, Clone)]
pub struct FingerprintWithHeaders {
  pub fingerprint: Fingerprint,
  pub headers: Headers,
}

/// Options for generating fingerprints.
#[derive(Debug, Clone, Default)]
pub struct FingerprintOptions {
  /// Target operating system: "windows", "macos", "linux"
  pub operating_system: Option<String>,
  /// Target browser: "firefox", "chrome", "safari", "edge"
  pub browsers: Option<Vec<String>>,
  /// Target device type: "desktop", "mobile"
  pub devices: Option<Vec<String>>,
  /// Locales for Accept-Language header
  pub locales: Option<Vec<String>>,
  /// HTTP version: "1" or "2"
  pub http_version: Option<String>,
  /// Screen dimension constraints
  pub screen: Option<ScreenConstraints>,
  /// Whether to mock WebRTC
  pub mock_web_rtc: bool,
  /// Slim mode (fewer evasions)
  pub slim: bool,
}

/// Constraints for screen dimensions.
#[derive(Debug, Clone, Default)]
pub struct ScreenConstraints {
  pub min_width: Option<u32>,
  pub max_width: Option<u32>,
  pub min_height: Option<u32>,
  pub max_height: Option<u32>,
}

impl ScreenConstraints {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn with_min_width(mut self, width: u32) -> Self {
    self.min_width = Some(width);
    self
  }

  pub fn with_max_width(mut self, width: u32) -> Self {
    self.max_width = Some(width);
    self
  }

  pub fn with_min_height(mut self, height: u32) -> Self {
    self.min_height = Some(height);
    self
  }

  pub fn with_max_height(mut self, height: u32) -> Self {
    self.max_height = Some(height);
    self
  }

  /// Check if a screen size matches these constraints.
  pub fn matches(&self, width: u32, height: u32) -> bool {
    if let Some(min_w) = self.min_width {
      if width < min_w {
        return false;
      }
    }
    if let Some(max_w) = self.max_width {
      if width > max_w {
        return false;
      }
    }
    if let Some(min_h) = self.min_height {
      if height < min_h {
        return false;
      }
    }
    if let Some(max_h) = self.max_height {
      if height > max_h {
        return false;
      }
    }
    true
  }
}

/// Constants used in fingerprint generation.
pub const MISSING_VALUE_DATASET_TOKEN: &str = "*MISSING_VALUE*";
pub const STRINGIFIED_PREFIX: &str = "*STRINGIFIED*";

/// Special node names in the Bayesian networks.
pub const BROWSER_HTTP_NODE_NAME: &str = "*BROWSER_HTTP";
pub const OPERATING_SYSTEM_NODE_NAME: &str = "*OPERATING_SYSTEM";
pub const DEVICE_NODE_NAME: &str = "*DEVICE";

/// Supported browsers.
pub const SUPPORTED_BROWSERS: &[&str] = &["chrome", "firefox", "safari", "edge"];

/// Supported operating systems.
pub const SUPPORTED_OPERATING_SYSTEMS: &[&str] = &["windows", "macos", "linux", "android", "ios"];

/// Supported devices.
pub const SUPPORTED_DEVICES: &[&str] = &["desktop", "mobile"];

/// Supported HTTP versions.
pub const SUPPORTED_HTTP_VERSIONS: &[&str] = &["1", "2"];

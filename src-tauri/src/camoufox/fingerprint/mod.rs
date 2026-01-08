//! Fingerprint generation module.
//!
//! Generates realistic browser fingerprints using Bayesian networks trained on real browser data.

pub mod bayesian_network;
pub mod bayesian_node;
pub mod types;

use bayesian_network::{BayesianNetwork, BayesianNetworkError};
use std::collections::HashMap;
use types::*;

use crate::camoufox::data;

/// Fingerprint generator using Bayesian networks.
pub struct FingerprintGenerator {
  fingerprint_network: BayesianNetwork,
  input_network: BayesianNetwork,
  header_network: BayesianNetwork,
  browser_helper: Vec<BrowserHttpInfo>,
  headers_order: HashMap<String, Vec<String>>,
}

/// Parsed browser/HTTP version info.
#[derive(Debug, Clone)]
pub struct BrowserHttpInfo {
  pub name: String,
  pub version: Vec<u32>,
  pub http_version: String,
  pub complete_string: String,
}

impl BrowserHttpInfo {
  fn parse(s: &str) -> Option<Self> {
    if s == MISSING_VALUE_DATASET_TOKEN {
      return None;
    }

    let parts: Vec<&str> = s.split('|').collect();
    if parts.len() != 2 {
      return None;
    }

    let browser_string = parts[0];
    let http_version = parts[1].to_string();

    let browser_parts: Vec<&str> = browser_string.split('/').collect();
    if browser_parts.len() != 2 {
      return None;
    }

    let name = browser_parts[0].to_string();
    let version: Vec<u32> = browser_parts[1]
      .split('.')
      .filter_map(|v| v.parse().ok())
      .collect();

    Some(Self {
      name,
      version,
      http_version,
      complete_string: s.to_string(),
    })
  }

  pub fn major_version(&self) -> u32 {
    self.version.first().copied().unwrap_or(0)
  }
}

/// Error type for fingerprint generation.
#[derive(Debug, thiserror::Error)]
pub enum FingerprintError {
  #[error("Bayesian network error: {0}")]
  Network(#[from] BayesianNetworkError),

  #[error("JSON parsing error: {0}")]
  Json(#[from] serde_json::Error),

  #[error("Failed to generate consistent fingerprint after {0} attempts")]
  GenerationFailed(u32),

  #[error("No valid fingerprint generated")]
  NoValidFingerprint,
}

impl FingerprintGenerator {
  /// Create a new fingerprint generator.
  pub fn new() -> Result<Self, FingerprintError> {
    let fingerprint_network = BayesianNetwork::from_zip_bytes(data::FINGERPRINT_NETWORK_ZIP)?;
    let input_network = BayesianNetwork::from_zip_bytes(data::INPUT_NETWORK_ZIP)?;
    let header_network = BayesianNetwork::from_zip_bytes(data::HEADER_NETWORK_ZIP)?;

    let browser_strings: Vec<String> = serde_json::from_str(data::BROWSER_HELPER_JSON)?;
    let browser_helper: Vec<BrowserHttpInfo> = browser_strings
      .iter()
      .filter_map(|s| BrowserHttpInfo::parse(s))
      .collect();

    let headers_order: HashMap<String, Vec<String>> =
      serde_json::from_str(data::HEADERS_ORDER_JSON)?;

    Ok(Self {
      fingerprint_network,
      input_network,
      header_network,
      browser_helper,
      headers_order,
    })
  }

  /// Generate a fingerprint with matching headers.
  pub fn get_fingerprint(
    &self,
    options: &FingerprintOptions,
  ) -> Result<FingerprintWithHeaders, FingerprintError> {
    const MAX_RETRIES: u32 = 10;

    // Build constraints from options
    let mut value_possibilities = self.build_constraints(options);

    // Handle screen constraints
    let screen_values = if let Some(screen_constraints) = &options.screen {
      self.filter_screen_values(screen_constraints)
    } else {
      None
    };

    if let Some(sv) = screen_values {
      value_possibilities.insert("screen".to_string(), sv);
    }

    for attempt in 0..MAX_RETRIES {
      // Generate input sample consistent with constraints
      let input_sample = self
        .input_network
        .generate_consistent_sample_when_possible(&value_possibilities);

      let Some(input_sample) = input_sample else {
        continue;
      };

      // Generate header sample from input
      let header_sample = self.header_network.generate_sample(&input_sample);

      // Extract user agent
      let user_agent = header_sample
        .get("user-agent")
        .or_else(|| header_sample.get("User-Agent"))
        .cloned()
        .unwrap_or_default();

      // Build fingerprint constraints with the generated user agent
      let mut fp_constraints = value_possibilities.clone();
      fp_constraints.insert("userAgent".to_string(), vec![user_agent.clone()]);

      // Generate fingerprint sample
      let fingerprint_sample = self
        .fingerprint_network
        .generate_consistent_sample_when_possible(&fp_constraints);

      let Some(fp_sample) = fingerprint_sample else {
        log::debug!(
          "Failed to generate fingerprint on attempt {}, retrying",
          attempt + 1
        );
        continue;
      };

      // Transform the sample to a Fingerprint struct
      match self.transform_sample(&fp_sample, &header_sample, options) {
        Ok(result) => return Ok(result),
        Err(e) => {
          log::debug!(
            "Failed to transform fingerprint on attempt {}: {}",
            attempt + 1,
            e
          );
          continue;
        }
      }
    }

    Err(FingerprintError::GenerationFailed(MAX_RETRIES))
  }

  /// Build constraint map from options.
  fn build_constraints(&self, options: &FingerprintOptions) -> HashMap<String, Vec<String>> {
    let mut constraints = HashMap::new();

    // Operating system constraint
    if let Some(os) = &options.operating_system {
      constraints.insert(OPERATING_SYSTEM_NODE_NAME.to_string(), vec![os.clone()]);
    }

    // Device constraint (default to desktop)
    let devices = options
      .devices
      .clone()
      .unwrap_or_else(|| vec!["desktop".to_string()]);
    constraints.insert(DEVICE_NODE_NAME.to_string(), devices);

    // Browser constraint
    let browsers = options
      .browsers
      .clone()
      .unwrap_or_else(|| SUPPORTED_BROWSERS.iter().map(|s| s.to_string()).collect());

    let http_version = options
      .http_version
      .clone()
      .unwrap_or_else(|| "2".to_string());

    // Filter browser helper entries by browser names and HTTP version
    let browser_http_values: Vec<String> = self
      .browser_helper
      .iter()
      .filter(|bh| browsers.contains(&bh.name) && bh.http_version == http_version)
      .map(|bh| bh.complete_string.clone())
      .collect();

    if !browser_http_values.is_empty() {
      constraints.insert(BROWSER_HTTP_NODE_NAME.to_string(), browser_http_values);
    }

    constraints
  }

  /// Filter screen values based on constraints.
  fn filter_screen_values(&self, constraints: &ScreenConstraints) -> Option<Vec<String>> {
    let possible_values = self.fingerprint_network.get_possible_values("screen")?;

    let filtered: Vec<String> = possible_values
      .into_iter()
      .filter(|screen_str| {
        // Screen values are stored as "*STRINGIFIED*{...json...}"
        if let Some(json_str) = screen_str.strip_prefix(STRINGIFIED_PREFIX) {
          if let Ok(screen) = serde_json::from_str::<serde_json::Value>(json_str) {
            let width = screen["width"].as_u64().unwrap_or(0) as u32;
            let height = screen["height"].as_u64().unwrap_or(0) as u32;
            return constraints.matches(width, height);
          }
        }
        true
      })
      .collect();

    if filtered.is_empty() {
      None
    } else {
      Some(filtered)
    }
  }

  /// Transform raw sample data into a Fingerprint struct.
  fn transform_sample(
    &self,
    fp_sample: &HashMap<String, String>,
    header_sample: &HashMap<String, String>,
    options: &FingerprintOptions,
  ) -> Result<FingerprintWithHeaders, FingerprintError> {
    // Parse values, handling STRINGIFIED prefix and MISSING_VALUE token
    let mut parsed: HashMap<String, serde_json::Value> = HashMap::new();

    for (key, value) in fp_sample {
      if value == MISSING_VALUE_DATASET_TOKEN {
        continue;
      }

      let parsed_value = if let Some(json_str) = value.strip_prefix(STRINGIFIED_PREFIX) {
        serde_json::from_str(json_str)?
      } else {
        serde_json::Value::String(value.clone())
      };

      parsed.insert(key.clone(), parsed_value);
    }

    // Check if screen was generated
    let screen_value = parsed.get("screen");
    if screen_value.is_none() {
      return Err(FingerprintError::NoValidFingerprint);
    }

    // Extract screen fingerprint
    let screen = if let Some(screen_val) = screen_value {
      serde_json::from_value(screen_val.clone()).unwrap_or_default()
    } else {
      ScreenFingerprint::default()
    };

    // Build languages from Accept-Language header
    let accept_language = header_sample
      .get("accept-language")
      .or_else(|| header_sample.get("Accept-Language"))
      .cloned()
      .unwrap_or_else(|| "en-US".to_string());

    let languages: Vec<String> = accept_language
      .split(',')
      .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
      .collect();

    let language = languages
      .first()
      .cloned()
      .unwrap_or_else(|| "en-US".to_string());

    // Build navigator fingerprint
    let navigator = NavigatorFingerprint {
      user_agent: get_string(&parsed, "userAgent"),
      user_agent_data: parsed
        .get("userAgentData")
        .and_then(|v| serde_json::from_value(v.clone()).ok()),
      do_not_track: parsed
        .get("doNotTrack")
        .and_then(|v| v.as_str().map(String::from)),
      app_code_name: get_string_or(&parsed, "appCodeName", "Mozilla"),
      app_name: get_string_or(&parsed, "appName", "Netscape"),
      app_version: get_string(&parsed, "appVersion"),
      oscpu: parsed
        .get("oscpu")
        .and_then(|v| v.as_str().map(String::from)),
      webdriver: parsed
        .get("webdriver")
        .and_then(|v| v.as_str().map(String::from)),
      language,
      languages,
      platform: get_string(&parsed, "platform"),
      device_memory: parsed
        .get("deviceMemory")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok()),
      hardware_concurrency: parsed
        .get("hardwareConcurrency")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(4),
      product: get_string_or(&parsed, "product", "Gecko"),
      product_sub: get_string(&parsed, "productSub"),
      vendor: get_string(&parsed, "vendor"),
      vendor_sub: get_string(&parsed, "vendorSub"),
      max_touch_points: parsed
        .get("maxTouchPoints")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0),
      extra_properties: parsed
        .get("extraProperties")
        .and_then(|v| serde_json::from_value(v.clone()).ok()),
    };

    // Build video card (will be filled later by WebGL sampler)
    let video_card = parsed
      .get("videoCard")
      .and_then(|v| serde_json::from_value(v.clone()).ok())
      .unwrap_or_default();

    // Build other components
    let audio_codecs = parsed
      .get("audioCodecs")
      .and_then(|v| serde_json::from_value(v.clone()).ok())
      .unwrap_or_default();

    let video_codecs = parsed
      .get("videoCodecs")
      .and_then(|v| serde_json::from_value(v.clone()).ok())
      .unwrap_or_default();

    let plugins_data = parsed
      .get("pluginsData")
      .and_then(|v| serde_json::from_value(v.clone()).ok())
      .unwrap_or_default();

    let battery = parsed
      .get("battery")
      .and_then(|v| serde_json::from_value(v.clone()).ok());

    let multimedia_devices = parsed
      .get("multimediaDevices")
      .and_then(|v| serde_json::from_value(v.clone()).ok())
      .unwrap_or_default();

    let fonts = parsed
      .get("fonts")
      .and_then(|v| serde_json::from_value(v.clone()).ok())
      .unwrap_or_default();

    let fingerprint = Fingerprint {
      screen,
      navigator,
      video_codecs,
      audio_codecs,
      plugins_data,
      battery,
      video_card,
      multimedia_devices,
      fonts,
      mock_web_rtc: options.mock_web_rtc,
      slim: options.slim,
    };

    // Build headers (filter out internal nodes and missing values)
    let headers: Headers = header_sample
      .iter()
      .filter(|(k, v)| !k.starts_with('*') && *v != MISSING_VALUE_DATASET_TOKEN)
      .map(|(k, v)| (k.clone(), v.clone()))
      .collect();

    // Order headers
    let ordered_headers = self.order_headers(&headers, &fingerprint.navigator.user_agent);

    Ok(FingerprintWithHeaders {
      fingerprint,
      headers: ordered_headers,
    })
  }

  /// Order headers according to browser-specific ordering.
  fn order_headers(&self, headers: &Headers, user_agent: &str) -> Headers {
    let browser = detect_browser_from_ua(user_agent);
    let order = self.headers_order.get(browser).cloned().unwrap_or_default();

    let mut ordered = HashMap::new();

    // Add headers in order
    for header_name in &order {
      if let Some(value) = headers.get(header_name) {
        ordered.insert(header_name.clone(), value.clone());
      }
    }

    // Add remaining headers not in order
    for (key, value) in headers {
      if !order.contains(key) {
        ordered.insert(key.clone(), value.clone());
      }
    }

    ordered
  }
}

fn get_string(map: &HashMap<String, serde_json::Value>, key: &str) -> String {
  map
    .get(key)
    .and_then(|v| v.as_str())
    .map(String::from)
    .unwrap_or_default()
}

fn get_string_or(map: &HashMap<String, serde_json::Value>, key: &str, default: &str) -> String {
  map
    .get(key)
    .and_then(|v| v.as_str())
    .map(String::from)
    .unwrap_or_else(|| default.to_string())
}

fn detect_browser_from_ua(user_agent: &str) -> &str {
  let ua_lower = user_agent.to_lowercase();
  if ua_lower.contains("firefox") {
    "firefox"
  } else if ua_lower.contains("edg/") || ua_lower.contains("edge") {
    "edge"
  } else if ua_lower.contains("chrome") {
    "chrome"
  } else if ua_lower.contains("safari") {
    "safari"
  } else {
    "chrome"
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_create_generator() {
    let generator = FingerprintGenerator::new();
    assert!(
      generator.is_ok(),
      "Failed to create generator: {:?}",
      generator.err()
    );
  }

  #[test]
  fn test_generate_fingerprint() {
    let generator = FingerprintGenerator::new().unwrap();
    let options = FingerprintOptions::default();

    let result = generator.get_fingerprint(&options);
    assert!(
      result.is_ok(),
      "Failed to generate fingerprint: {:?}",
      result.err()
    );

    if let Ok(fp) = result {
      assert!(!fp.fingerprint.navigator.user_agent.is_empty());
      assert!(fp.fingerprint.screen.width > 0);
      assert!(fp.fingerprint.screen.height > 0);
    }
  }

  #[test]
  fn test_generate_firefox_fingerprint() {
    let generator = FingerprintGenerator::new().unwrap();
    let options = FingerprintOptions {
      browsers: Some(vec!["firefox".to_string()]),
      ..Default::default()
    };

    let result = generator.get_fingerprint(&options);
    assert!(result.is_ok(), "Failed to generate Firefox fingerprint");

    if let Ok(fp) = result {
      assert!(
        fp.fingerprint
          .navigator
          .user_agent
          .to_lowercase()
          .contains("firefox"),
        "User agent should contain Firefox: {}",
        fp.fingerprint.navigator.user_agent
      );
    }
  }

  #[test]
  fn test_generate_with_screen_constraints() {
    let generator = FingerprintGenerator::new().unwrap();
    let options = FingerprintOptions {
      screen: Some(ScreenConstraints {
        min_width: Some(1900),
        max_width: Some(1920),
        min_height: Some(1000),
        max_height: Some(1100),
      }),
      ..Default::default()
    };

    let result = generator.get_fingerprint(&options);
    assert!(
      result.is_ok(),
      "Failed to generate fingerprint with screen constraints"
    );

    if let Ok(fp) = result {
      assert!(
        fp.fingerprint.screen.width >= 1900 && fp.fingerprint.screen.width <= 1920,
        "Screen width {} should be between 1900 and 1920",
        fp.fingerprint.screen.width
      );
    }
  }

  #[test]
  fn test_browser_http_info_parse() {
    let info = BrowserHttpInfo::parse("chrome/143.0.0.0|2");
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.name, "chrome");
    assert_eq!(info.major_version(), 143);
    assert_eq!(info.http_version, "2");
  }
}

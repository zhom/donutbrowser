//! Environment variable handling for Camoufox configuration.
//!
//! Camoufox reads its configuration from environment variables named CAMOU_CONFIG_1, CAMOU_CONFIG_2, etc.
//! The configuration JSON is chunked to fit within environment variable size limits.

use std::collections::HashMap;

/// Maximum chunk size for environment variables on Windows.
const CHUNK_SIZE_WINDOWS: usize = 2047;

/// Maximum chunk size for environment variables on Unix systems.
const CHUNK_SIZE_UNIX: usize = 32767;

/// Get the chunk size for the current platform.
fn get_chunk_size() -> usize {
  if cfg!(windows) {
    CHUNK_SIZE_WINDOWS
  } else {
    CHUNK_SIZE_UNIX
  }
}

/// Convert a Camoufox config map to environment variables.
///
/// The config is serialized to JSON and split into chunks that fit within
/// environment variable size limits. Each chunk is stored in a variable
/// named CAMOU_CONFIG_1, CAMOU_CONFIG_2, etc.
pub fn config_to_env_vars(
  config: &HashMap<String, serde_json::Value>,
) -> Result<HashMap<String, String>, serde_json::Error> {
  let config_json = serde_json::to_string(config)?;
  Ok(chunk_config_string(&config_json))
}

/// Split a config string into chunks and create environment variable map.
pub fn chunk_config_string(config_str: &str) -> HashMap<String, String> {
  let chunk_size = get_chunk_size();
  let mut env_vars = HashMap::new();

  for (i, chunk) in config_str.as_bytes().chunks(chunk_size).enumerate() {
    let chunk_str = String::from_utf8_lossy(chunk).to_string();
    let env_name = format!("CAMOU_CONFIG_{}", i + 1);
    env_vars.insert(env_name, chunk_str);
  }

  env_vars
}

/// Determine the target OS from a user agent string.
pub fn determine_ua_os(user_agent: &str) -> &'static str {
  let ua_lower = user_agent.to_lowercase();

  if ua_lower.contains("mac os") || ua_lower.contains("macos") || ua_lower.contains("macintosh") {
    "mac"
  } else if ua_lower.contains("windows") {
    "win"
  } else {
    "lin"
  }
}

/// Get the fontconfig path environment variable for Linux.
pub fn get_fontconfig_env(target_os: &str, camoufox_path: &std::path::Path) -> Option<String> {
  if cfg!(target_os = "linux") {
    let fontconfig_dir = camoufox_path.join("fontconfig").join(target_os);
    if fontconfig_dir.exists() {
      return Some(fontconfig_dir.to_string_lossy().to_string());
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_chunk_small_config() {
    let config = r#"{"navigator.userAgent": "Mozilla/5.0"}"#;
    let env_vars = chunk_config_string(config);

    assert_eq!(env_vars.len(), 1);
    assert!(env_vars.contains_key("CAMOU_CONFIG_1"));
    assert_eq!(env_vars.get("CAMOU_CONFIG_1").unwrap(), config);
  }

  #[test]
  fn test_chunk_large_config() {
    // Create a config string larger than the chunk size
    let chunk_size = get_chunk_size();
    let large_value = "x".repeat(chunk_size * 2 + 100);
    let config = format!(r#"{{"key": "{}"}}"#, large_value);

    let env_vars = chunk_config_string(&config);

    // Should have at least 2 chunks
    assert!(env_vars.len() >= 2);
    assert!(env_vars.contains_key("CAMOU_CONFIG_1"));
    assert!(env_vars.contains_key("CAMOU_CONFIG_2"));

    // Reconstruct and verify
    let mut reconstructed = String::new();
    let mut i = 1;
    while let Some(chunk) = env_vars.get(&format!("CAMOU_CONFIG_{}", i)) {
      reconstructed.push_str(chunk);
      i += 1;
    }
    assert_eq!(reconstructed, config);
  }

  #[test]
  fn test_determine_ua_os_windows() {
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:135.0) Gecko/20100101 Firefox/135.0";
    assert_eq!(determine_ua_os(ua), "win");
  }

  #[test]
  fn test_determine_ua_os_macos() {
    let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:135.0) Gecko/20100101 Firefox/135.0";
    assert_eq!(determine_ua_os(ua), "mac");
  }

  #[test]
  fn test_determine_ua_os_linux() {
    let ua = "Mozilla/5.0 (X11; Linux x86_64; rv:135.0) Gecko/20100101 Firefox/135.0";
    assert_eq!(determine_ua_os(ua), "lin");
  }

  #[test]
  fn test_config_to_env_vars() {
    let mut config = HashMap::new();
    config.insert(
      "navigator.userAgent".to_string(),
      serde_json::json!("Mozilla/5.0 Firefox/135.0"),
    );
    config.insert("screen.width".to_string(), serde_json::json!(1920));

    let env_vars = config_to_env_vars(&config).unwrap();
    assert!(!env_vars.is_empty());
    assert!(env_vars.contains_key("CAMOU_CONFIG_1"));
  }
}

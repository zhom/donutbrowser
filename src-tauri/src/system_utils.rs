use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemLocale {
  pub locale: String,
  pub language: String,
  pub country: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemTimezone {
  pub timezone: String,
  pub offset: String,
}

pub struct SystemUtils;

impl SystemUtils {
  pub fn new() -> Self {
    Self
  }

  /// Detect the system's locale settings
  pub fn detect_system_locale(&self) -> SystemLocale {
    #[cfg(target_os = "macos")]
    return macos::detect_system_locale();

    #[cfg(target_os = "linux")]
    return linux::detect_system_locale();

    #[cfg(target_os = "windows")]
    return windows::detect_system_locale();

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return SystemLocale {
      locale: "en-US".to_string(),
      language: "en".to_string(),
      country: "US".to_string(),
    };
  }

  /// Detect the system's timezone settings
  pub fn detect_system_timezone(&self) -> SystemTimezone {
    #[cfg(target_os = "macos")]
    return macos::detect_system_timezone();

    #[cfg(target_os = "linux")]
    return linux::detect_system_timezone();

    #[cfg(target_os = "windows")]
    return windows::detect_system_timezone();

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return SystemTimezone {
      timezone: "UTC".to_string(),
      offset: "+00:00".to_string(),
    };
  }
}

#[cfg(target_os = "macos")]
mod macos {
  use super::*;

  pub fn detect_system_locale() -> SystemLocale {
    // Try to get the system locale from macOS
    if let Ok(output) = Command::new("defaults")
      .args(["read", "-g", "AppleLocale"])
      .output()
    {
      if output.status.success() {
        let locale_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return parse_locale(&locale_str);
      }
    }

    // Fallback to environment variables
    detect_locale_from_env()
  }

  pub fn detect_system_timezone() -> SystemTimezone {
    // Try to get timezone from macOS system
    if let Ok(output) = Command::new("date").arg("+%Z").output() {
      if output.status.success() {
        let tz_abbr = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Get the full timezone name
        if let Ok(tz_output) = Command::new("systemsetup").args(["-gettimezone"]).output() {
          if tz_output.status.success() {
            let tz_full = String::from_utf8_lossy(&tz_output.stdout);
            if let Some(tz_name) = tz_full.strip_prefix("Time Zone: ") {
              let tz_clean = tz_name.trim().to_string();
              if !tz_clean.is_empty() {
                return SystemTimezone {
                  timezone: tz_clean,
                  offset: tz_abbr,
                };
              }
            }
          }
        }
      }
    }

    // Fallback to reading /etc/localtime link
    detect_timezone_from_files()
  }
}

#[cfg(target_os = "linux")]
mod linux {
  use super::*;

  pub fn detect_system_locale() -> SystemLocale {
    // Try to get locale from locale command
    if let Ok(output) = Command::new("locale").output() {
      if output.status.success() {
        let output_str = String::from_utf8_lossy(&output.stdout);
        for line in output_str.lines() {
          if line.starts_with("LANG=") {
            let locale_value = line.strip_prefix("LANG=").unwrap_or("");
            let locale_clean = locale_value.trim_matches('"');
            return parse_locale(locale_clean);
          }
        }
      }
    }

    // Fallback to environment variables
    detect_locale_from_env()
  }

  pub fn detect_system_timezone() -> SystemTimezone {
    // Try to read /etc/timezone first (Debian/Ubuntu)
    if let Ok(tz_content) = std::fs::read_to_string("/etc/timezone") {
      let tz_name = tz_content.trim().to_string();
      if !tz_name.is_empty() {
        return SystemTimezone {
          timezone: tz_name,
          offset: get_timezone_offset(),
        };
      }
    }

    // Try timedatectl (systemd systems)
    if let Ok(output) = Command::new("timedatectl")
      .args(["show", "--property=Timezone", "--value"])
      .output()
    {
      if output.status.success() {
        let tz_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !tz_name.is_empty() {
          return SystemTimezone {
            timezone: tz_name,
            offset: get_timezone_offset(),
          };
        }
      }
    }

    // Fallback to reading /etc/localtime symlink
    detect_timezone_from_files()
  }

  fn get_timezone_offset() -> String {
    if let Ok(output) = Command::new("date").arg("+%z").output() {
      if output.status.success() {
        return String::from_utf8_lossy(&output.stdout).trim().to_string();
      }
    }
    "+00:00".to_string()
  }
}

#[cfg(target_os = "windows")]
mod windows {
  use super::*;

  pub fn detect_system_locale() -> SystemLocale {
    // Try to get locale from Windows registry/powershell
    if let Ok(output) = Command::new("powershell")
      .args([
        "-Command",
        "Get-Culture | Select-Object -ExpandProperty Name",
      ])
      .output()
    {
      if output.status.success() {
        let locale_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return parse_locale(&locale_str);
      }
    }

    // Fallback to environment variables
    detect_locale_from_env()
  }

  pub fn detect_system_timezone() -> SystemTimezone {
    // Try to get timezone from Windows
    if let Ok(output) = Command::new("powershell")
      .args([
        "-Command",
        "Get-TimeZone | Select-Object -ExpandProperty Id",
      ])
      .output()
    {
      if output.status.success() {
        let tz_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !tz_id.is_empty() {
          return SystemTimezone {
            timezone: tz_id,
            offset: get_windows_timezone_offset(),
          };
        }
      }
    }

    // Fallback
    SystemTimezone {
      timezone: "UTC".to_string(),
      offset: "+00:00".to_string(),
    }
  }

  fn get_windows_timezone_offset() -> String {
    if let Ok(output) = Command::new("powershell")
      .args([
        "-Command",
        "Get-TimeZone | Select-Object -ExpandProperty BaseUtcOffset",
      ])
      .output()
    {
      if output.status.success() {
        let offset_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Convert Windows offset format to standard format
        if let Some(colon_pos) = offset_str.find(':') {
          let hours = &offset_str[..colon_pos];
          let minutes = &offset_str[colon_pos + 1..];
          if let (Ok(h), Ok(m)) = (hours.parse::<i32>(), minutes.parse::<i32>()) {
            return format!("{:+03}:{:02}", h, m);
          }
        }
      }
    }
    "+00:00".to_string()
  }
}

// Helper functions used across platforms
fn parse_locale(locale_str: &str) -> SystemLocale {
  // Remove encoding suffix if present (e.g., "en_US.UTF-8" -> "en_US")
  let locale_base = locale_str.split('.').next().unwrap_or(locale_str);

  // Split language and country (e.g., "en_US" -> ["en", "US"])
  let parts: Vec<&str> = locale_base.split(&['_', '-']).collect();

  let language = parts.first().unwrap_or(&"en").to_string();
  let country = parts.get(1).unwrap_or(&"US").to_string();

  // Convert to standard format (e.g., "en-US")
  let standard_locale = if parts.len() >= 2 {
    format!("{}-{}", language, country.to_uppercase())
  } else {
    format!("{language}-US")
  };

  SystemLocale {
    locale: standard_locale,
    language,
    country: country.to_uppercase(),
  }
}

fn detect_locale_from_env() -> SystemLocale {
  // Check environment variables in order of preference
  let env_vars = ["LANG", "LC_ALL", "LC_CTYPE", "LANGUAGE"];

  for var in &env_vars {
    if let Ok(value) = std::env::var(var) {
      if !value.is_empty() {
        return parse_locale(&value);
      }
    }
  }

  // Default fallback
  SystemLocale {
    locale: "en-US".to_string(),
    language: "en".to_string(),
    country: "US".to_string(),
  }
}

fn detect_timezone_from_files() -> SystemTimezone {
  // Try to read timezone from /etc/localtime symlink
  if let Ok(link_target) = std::fs::read_link("/etc/localtime") {
    if let Some(tz_path) = link_target.to_str() {
      // Extract timezone name from path like /usr/share/zoneinfo/America/New_York
      if let Some(zoneinfo_pos) = tz_path.find("zoneinfo/") {
        let tz_name = &tz_path[zoneinfo_pos + 9..];
        if !tz_name.is_empty() {
          return SystemTimezone {
            timezone: tz_name.to_string(),
            offset: "+00:00".to_string(), // Could be improved with actual offset calculation
          };
        }
      }
    }
  }

  // Default fallback
  SystemTimezone {
    timezone: "UTC".to_string(),
    offset: "+00:00".to_string(),
  }
}

/// Tauri command to get system locale
#[tauri::command]
pub async fn get_system_locale() -> Result<SystemLocale, String> {
  let utils = SystemUtils::new();
  Ok(utils.detect_system_locale())
}

/// Tauri command to get system timezone
#[tauri::command]
pub async fn get_system_timezone() -> Result<SystemTimezone, String> {
  let utils = SystemUtils::new();
  Ok(utils.detect_system_timezone())
}

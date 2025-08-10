use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemTheme {
  pub theme: String, // "light", "dark", or "unknown"
}

pub struct ThemeDetector;

impl ThemeDetector {
  fn new() -> Self {
    Self
  }

  pub fn instance() -> &'static ThemeDetector {
    &THEME_DETECTOR
  }

  /// Detect the system theme preference
  pub fn detect_system_theme(&self) -> SystemTheme {
    #[cfg(target_os = "linux")]
    return linux::detect_system_theme();

    #[cfg(target_os = "macos")]
    return macos::detect_system_theme();

    #[cfg(target_os = "windows")]
    return windows::detect_system_theme();

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    return SystemTheme {
      theme: "unknown".to_string(),
    };
  }
}

#[cfg(target_os = "linux")]
mod linux {
  use super::*;

  pub fn detect_system_theme() -> SystemTheme {
    // Try multiple methods in order of preference

    // 1. Try GNOME/GTK settings via gsettings
    if let Ok(theme) = detect_gnome_theme() {
      return SystemTheme { theme };
    }

    // 2. Try KDE Plasma settings via kreadconfig5/kreadconfig6
    if let Ok(theme) = detect_kde_theme() {
      return SystemTheme { theme };
    }

    // 3. Try XFCE settings via xfconf-query
    if let Ok(theme) = detect_xfce_theme() {
      return SystemTheme { theme };
    }

    // 4. Try looking at current GTK theme name
    if let Ok(theme) = detect_gtk_theme() {
      return SystemTheme { theme };
    }

    // 5. Try dconf directly (fallback for GNOME-based systems)
    if let Ok(theme) = detect_dconf_theme() {
      return SystemTheme { theme };
    }

    // 6. Try environment variables
    if let Ok(theme) = detect_env_theme() {
      return SystemTheme { theme };
    }

    // 7. Try freedesktop portal
    if let Ok(theme) = detect_portal_theme() {
      return SystemTheme { theme };
    }

    // 8. Try looking at system color scheme files
    if let Ok(theme) = detect_system_files_theme() {
      return SystemTheme { theme };
    }

    // Fallback to unknown
    SystemTheme {
      theme: "unknown".to_string(),
    }
  }

  fn detect_gnome_theme() -> Result<String, Box<dyn std::error::Error>> {
    // Check if gsettings is available
    if !is_command_available("gsettings") {
      return Err("gsettings not available".into());
    }

    // Try GNOME color scheme first (modern way)
    if let Ok(output) = Command::new("gsettings")
      .args(["get", "org.gnome.desktop.interface", "color-scheme"])
      .output()
    {
      if output.status.success() {
        let scheme = String::from_utf8_lossy(&output.stdout).trim().to_string();
        match scheme.as_str() {
          "'prefer-dark'" => return Ok("dark".to_string()),
          "'prefer-light'" => return Ok("light".to_string()),
          _ => {}
        }
      }
    }

    // Fallback to GTK theme name detection
    if let Ok(output) = Command::new("gsettings")
      .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
      .output()
    {
      if output.status.success() {
        let theme_name = String::from_utf8_lossy(&output.stdout)
          .trim()
          .trim_matches('\'')
          .to_lowercase();

        if theme_name.contains("dark") || theme_name.contains("night") {
          return Ok("dark".to_string());
        } else if theme_name.contains("light") || theme_name.contains("adwaita") {
          return Ok("light".to_string());
        }
      }
    }

    Err("Could not detect GNOME theme".into())
  }

  fn detect_kde_theme() -> Result<String, Box<dyn std::error::Error>> {
    // Try KDE Plasma 6 first
    if is_command_available("kreadconfig6") {
      if let Ok(output) = Command::new("kreadconfig6")
        .args([
          "--file",
          "kdeglobals",
          "--group",
          "KDE",
          "--key",
          "LookAndFeelPackage",
        ])
        .output()
      {
        if output.status.success() {
          let theme = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_lowercase();
          if theme.contains("dark") || theme.contains("breezedark") {
            return Ok("dark".to_string());
          } else if theme.contains("light") || theme.contains("breeze") {
            return Ok("light".to_string());
          }
        }
      }

      // Try color scheme as well
      if let Ok(output) = Command::new("kreadconfig6")
        .args([
          "--file",
          "kdeglobals",
          "--group",
          "General",
          "--key",
          "ColorScheme",
        ])
        .output()
      {
        if output.status.success() {
          let scheme = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_lowercase();
          if scheme.contains("dark") || scheme.contains("breezedark") {
            return Ok("dark".to_string());
          } else if scheme.contains("light") || scheme.contains("breeze") {
            return Ok("light".to_string());
          }
        }
      }
    }

    // Try KDE Plasma 5 as fallback
    if is_command_available("kreadconfig5") {
      if let Ok(output) = Command::new("kreadconfig5")
        .args([
          "--file",
          "kdeglobals",
          "--group",
          "KDE",
          "--key",
          "LookAndFeelPackage",
        ])
        .output()
      {
        if output.status.success() {
          let theme = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_lowercase();
          if theme.contains("dark") || theme.contains("breezedark") {
            return Ok("dark".to_string());
          } else if theme.contains("light") || theme.contains("breeze") {
            return Ok("light".to_string());
          }
        }
      }
    }

    Err("Could not detect KDE theme".into())
  }

  fn detect_xfce_theme() -> Result<String, Box<dyn std::error::Error>> {
    if !is_command_available("xfconf-query") {
      return Err("xfconf-query not available".into());
    }

    // Check XFCE theme
    if let Ok(output) = Command::new("xfconf-query")
      .args(["-c", "xsettings", "-p", "/Net/ThemeName"])
      .output()
    {
      if output.status.success() {
        let theme = String::from_utf8_lossy(&output.stdout)
          .trim()
          .to_lowercase();
        if theme.contains("dark") || theme.contains("night") {
          return Ok("dark".to_string());
        } else if theme.contains("light") {
          return Ok("light".to_string());
        }
      }
    }

    // Check XFCE window manager theme as backup
    if let Ok(output) = Command::new("xfconf-query")
      .args(["-c", "xfwm4", "-p", "/general/theme"])
      .output()
    {
      if output.status.success() {
        let theme = String::from_utf8_lossy(&output.stdout)
          .trim()
          .to_lowercase();
        if theme.contains("dark") || theme.contains("night") {
          return Ok("dark".to_string());
        } else if theme.contains("light") {
          return Ok("light".to_string());
        }
      }
    }

    Err("Could not detect XFCE theme".into())
  }

  fn detect_gtk_theme() -> Result<String, Box<dyn std::error::Error>> {
    // Try to read GTK3 settings file
    if let Ok(home) = std::env::var("HOME") {
      let gtk3_settings = std::path::Path::new(&home).join(".config/gtk-3.0/settings.ini");
      if gtk3_settings.exists() {
        if let Ok(content) = std::fs::read_to_string(gtk3_settings) {
          for line in content.lines() {
            if line.starts_with("gtk-theme-name=") {
              let theme_name = line.split('=').nth(1).unwrap_or("").trim().to_lowercase();
              if theme_name.contains("dark") || theme_name.contains("night") {
                return Ok("dark".to_string());
              } else if theme_name.contains("light") || theme_name.contains("adwaita") {
                return Ok("light".to_string());
              }
            }
          }
        }
      }

      // Try GTK4 settings
      let gtk4_settings = std::path::Path::new(&home).join(".config/gtk-4.0/settings.ini");
      if gtk4_settings.exists() {
        if let Ok(content) = std::fs::read_to_string(gtk4_settings) {
          for line in content.lines() {
            if line.starts_with("gtk-theme-name=") {
              let theme_name = line.split('=').nth(1).unwrap_or("").trim().to_lowercase();
              if theme_name.contains("dark") || theme_name.contains("night") {
                return Ok("dark".to_string());
              } else if theme_name.contains("light") || theme_name.contains("adwaita") {
                return Ok("light".to_string());
              }
            }
          }
        }
      }
    }

    Err("Could not detect GTK theme".into())
  }

  fn detect_dconf_theme() -> Result<String, Box<dyn std::error::Error>> {
    if !is_command_available("dconf") {
      return Err("dconf not available".into());
    }

    // Try reading color scheme directly from dconf
    if let Ok(output) = Command::new("dconf")
      .args(["read", "/org/gnome/desktop/interface/color-scheme"])
      .output()
    {
      if output.status.success() {
        let scheme = String::from_utf8_lossy(&output.stdout).trim().to_string();
        match scheme.as_str() {
          "'prefer-dark'" => return Ok("dark".to_string()),
          "'prefer-light'" => return Ok("light".to_string()),
          _ => {}
        }
      }
    }

    // Try reading GTK theme from dconf
    if let Ok(output) = Command::new("dconf")
      .args(["read", "/org/gnome/desktop/interface/gtk-theme"])
      .output()
    {
      if output.status.success() {
        let theme_name = String::from_utf8_lossy(&output.stdout)
          .trim()
          .trim_matches('\'')
          .to_lowercase();

        if theme_name.contains("dark") || theme_name.contains("night") {
          return Ok("dark".to_string());
        } else if theme_name.contains("light") || theme_name.contains("adwaita") {
          return Ok("light".to_string());
        }
      }
    }

    Err("Could not detect dconf theme".into())
  }

  fn detect_env_theme() -> Result<String, Box<dyn std::error::Error>> {
    // Check common environment variables
    if let Ok(theme) = std::env::var("GTK_THEME") {
      let theme_lower = theme.to_lowercase();
      if theme_lower.contains("dark") || theme_lower.contains("night") {
        return Ok("dark".to_string());
      } else if theme_lower.contains("light") {
        return Ok("light".to_string());
      }
    }

    if let Ok(theme) = std::env::var("QT_STYLE_OVERRIDE") {
      let theme_lower = theme.to_lowercase();
      if theme_lower.contains("dark") || theme_lower.contains("night") {
        return Ok("dark".to_string());
      } else if theme_lower.contains("light") {
        return Ok("light".to_string());
      }
    }

    Err("Could not detect theme from environment".into())
  }

  fn detect_portal_theme() -> Result<String, Box<dyn std::error::Error>> {
    if !is_command_available("busctl") {
      return Err("busctl not available".into());
    }

    // Try to query the color scheme via org.freedesktop.portal.Settings
    if let Ok(output) = Command::new("busctl")
      .args([
        "--user",
        "call",
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        "Read",
        "ss",
        "org.freedesktop.appearance",
        "color-scheme",
      ])
      .output()
    {
      if output.status.success() {
        let response = String::from_utf8_lossy(&output.stdout);
        // Parse DBus response - look for preference values
        if response.contains(" 1 ") {
          return Ok("dark".to_string());
        } else if response.contains(" 2 ") {
          return Ok("light".to_string());
        }
      }
    }

    Err("Could not detect portal theme".into())
  }

  fn detect_system_files_theme() -> Result<String, Box<dyn std::error::Error>> {
    // Check if we're in a dark terminal (heuristic)
    if let Ok(term) = std::env::var("TERM") {
      let term_lower = term.to_lowercase();
      if term_lower.contains("dark") || term_lower.contains("night") {
        return Ok("dark".to_string());
      }
    }

    // Check if we can determine from desktop session
    if let Ok(desktop) = std::env::var("XDG_CURRENT_DESKTOP") {
      let desktop_lower = desktop.to_lowercase();
      // Some desktops default to dark
      if desktop_lower.contains("i3") || desktop_lower.contains("sway") {
        // Window managers often use dark themes by default
        return Ok("dark".to_string());
      }
    }

    Err("Could not detect theme from system files".into())
  }

  pub fn is_command_available(command: &str) -> bool {
    Command::new("which")
      .arg(command)
      .output()
      .map(|output| output.status.success())
      .unwrap_or(false)
  }
}

#[cfg(target_os = "macos")]
mod macos {
  use super::*;

  pub fn detect_system_theme() -> SystemTheme {
    // macOS theme detection using osascript
    if let Ok(output) = Command::new("osascript")
      .args([
        "-e",
        "tell application \"System Events\" to tell appearance preferences to get dark mode",
      ])
      .output()
    {
      if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout).to_string();
        let result = result.trim();
        match result {
          "true" => {
            return SystemTheme {
              theme: "dark".to_string(),
            }
          }
          "false" => {
            return SystemTheme {
              theme: "light".to_string(),
            }
          }
          _ => {}
        }
      }
    }

    // Fallback method using defaults
    if let Ok(output) = Command::new("defaults")
      .args(["read", "-g", "AppleInterfaceStyle"])
      .output()
    {
      if output.status.success() {
        let style = String::from_utf8_lossy(&output.stdout).to_string();
        let style = style.trim();
        if style.to_lowercase() == "dark" {
          return SystemTheme {
            theme: "dark".to_string(),
          };
        }
      }
    }

    // Default to light if we can't determine
    SystemTheme {
      theme: "light".to_string(),
    }
  }
}

#[cfg(target_os = "windows")]
mod windows {
  use super::*;

  pub fn detect_system_theme() -> SystemTheme {
    // Windows theme detection via registry
    // This is a simplified implementation - you might want to use winreg crate for better registry access
    if let Ok(output) = Command::new("reg")
      .args([
        "query",
        "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
        "/v",
        "AppsUseLightTheme",
      ])
      .output()
    {
      if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout);
        if result.contains("0x0") {
          return SystemTheme {
            theme: "dark".to_string(),
          };
        } else if result.contains("0x1") {
          return SystemTheme {
            theme: "light".to_string(),
          };
        }
      }
    }

    // Default to light if we can't determine
    SystemTheme {
      theme: "light".to_string(),
    }
  }
}

// Command to expose this functionality to the frontend
#[tauri::command]
pub fn get_system_theme() -> SystemTheme {
  let detector = ThemeDetector::instance();
  detector.detect_system_theme()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_theme_detector_creation() {
    let detector = ThemeDetector::instance();

    // Should not panic when creating detector
    assert!(
      std::ptr::eq(detector, ThemeDetector::instance()),
      "Should return same instance (singleton)"
    );
  }

  #[test]
  fn test_detect_system_theme_returns_valid_value() {
    let detector = ThemeDetector::instance();
    let theme = detector.detect_system_theme();

    // Should return a valid theme string
    assert!(
      matches!(theme.theme.as_str(), "light" | "dark" | "unknown"),
      "Theme should be one of: light, dark, unknown. Got: {}",
      theme.theme
    );

    // Theme string should not be empty
    assert!(!theme.theme.is_empty(), "Theme string should not be empty");
  }

  #[test]
  fn test_get_system_theme_command() {
    let theme = get_system_theme();

    assert!(
      matches!(theme.theme.as_str(), "light" | "dark" | "unknown"),
      "Command should return valid theme. Got: {}",
      theme.theme
    );

    // Should be consistent with direct detector call
    let detector = ThemeDetector::instance();
    let direct_theme = detector.detect_system_theme();
    assert_eq!(
      theme.theme, direct_theme.theme,
      "Command and direct call should return same theme"
    );
  }

  #[test]
  fn test_system_theme_serialization() {
    let theme = SystemTheme {
      theme: "dark".to_string(),
    };

    // Test serialization
    let serialized = serde_json::to_string(&theme);
    assert!(
      serialized.is_ok(),
      "Should serialize SystemTheme successfully"
    );

    let json_str = serialized.unwrap();
    assert!(
      json_str.contains("dark"),
      "Serialized JSON should contain theme value"
    );

    // Test deserialization
    let deserialized: Result<SystemTheme, _> = serde_json::from_str(&json_str);
    assert!(
      deserialized.is_ok(),
      "Should deserialize SystemTheme successfully"
    );

    let theme_back = deserialized.unwrap();
    assert_eq!(
      theme_back.theme, "dark",
      "Deserialized theme should match original"
    );
  }

  #[cfg(target_os = "linux")]
  #[test]
  fn test_linux_command_availability_check() {
    use super::linux::is_command_available;

    // Test with a command that should exist on most systems
    let ls_available = is_command_available("ls");
    assert!(ls_available, "ls command should be available on Linux");

    // Test with a command that definitely doesn't exist
    let fake_available = is_command_available("definitely_nonexistent_command_12345");
    assert!(!fake_available, "Fake command should not be available");
  }

  #[test]
  fn test_theme_detector_consistency() {
    let detector = ThemeDetector::instance();

    // Call detect_system_theme multiple times - should be consistent
    let theme1 = detector.detect_system_theme();
    let theme2 = detector.detect_system_theme();
    let theme3 = detector.detect_system_theme();

    assert_eq!(
      theme1.theme, theme2.theme,
      "Multiple calls should return consistent results"
    );
    assert_eq!(
      theme2.theme, theme3.theme,
      "Multiple calls should return consistent results"
    );
  }
}

// Global singleton instance
lazy_static::lazy_static! {
  static ref THEME_DETECTOR: ThemeDetector = ThemeDetector::new();
}

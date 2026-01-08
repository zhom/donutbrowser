//! OS-specific font lists for Camoufox.
//!
//! Provides default system fonts for Windows, macOS, and Linux.

use std::collections::HashMap;

use crate::camoufox::data;

/// Get fonts for the target OS.
pub fn get_fonts_for_os(target_os: &str) -> Vec<String> {
  let fonts_map: HashMap<String, Vec<String>> =
    serde_json::from_str(data::FONTS_JSON).unwrap_or_default();

  let os_key = match target_os {
    "win" | "windows" => "win",
    "mac" | "macos" => "mac",
    "lin" | "linux" => "lin",
    _ => "win", // Default to Windows fonts
  };

  fonts_map.get(os_key).cloned().unwrap_or_default()
}

/// Get fonts for the target OS with additional custom fonts.
pub fn get_fonts_with_custom(target_os: &str, custom_fonts: Option<&[String]>) -> Vec<String> {
  let mut fonts = get_fonts_for_os(target_os);

  if let Some(custom) = custom_fonts {
    // Add custom fonts, avoiding duplicates
    for font in custom {
      if !fonts.contains(font) {
        fonts.push(font.clone());
      }
    }
  }

  fonts
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_get_fonts_for_windows() {
    let fonts = get_fonts_for_os("win");
    assert!(!fonts.is_empty());
    assert!(fonts.contains(&"Arial".to_string()));
    assert!(fonts.contains(&"Calibri".to_string()));
  }

  #[test]
  fn test_get_fonts_for_macos() {
    let fonts = get_fonts_for_os("mac");
    assert!(!fonts.is_empty());
    assert!(fonts.contains(&"Helvetica".to_string()));
  }

  #[test]
  fn test_get_fonts_for_linux() {
    let fonts = get_fonts_for_os("lin");
    assert!(!fonts.is_empty());
  }

  #[test]
  fn test_get_fonts_with_custom() {
    let custom = vec!["MyCustomFont".to_string()];
    let fonts = get_fonts_with_custom("win", Some(&custom));

    assert!(fonts.contains(&"MyCustomFont".to_string()));
    assert!(fonts.contains(&"Arial".to_string()));
  }

  #[test]
  fn test_fonts_no_duplicates() {
    let custom = vec!["Arial".to_string()]; // Arial already exists in Windows fonts
    let fonts = get_fonts_with_custom("win", Some(&custom));

    // Count occurrences of Arial
    let arial_count = fonts.iter().filter(|f| *f == "Arial").count();
    assert_eq!(arial_count, 1);
  }
}

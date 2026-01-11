//! Geolocation support for Camoufox fingerprinting.
//!
//! This module provides IP-based geolocation lookup using the MaxMind GeoLite2 database,
//! and locale generation based on country/territory information.

use crate::camoufox::data;
use crate::geoip_downloader::GeoIPDownloader;
use directories::BaseDirs;
use maxminddb::{geoip2, Reader};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use rand::Rng;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;

// Re-export IP utilities for backward compatibility
pub use crate::ip_utils::{fetch_public_ip, is_ipv4, is_ipv6, validate_ip, IpError};

/// Geolocation error type.
#[derive(Debug, thiserror::Error)]
pub enum GeolocationError {
  #[error("GeoIP database not found. Please download it first.")]
  DatabaseNotFound,

  #[error("Failed to open GeoIP database: {0}")]
  DatabaseOpen(String),

  #[error("Invalid IP address: {0}")]
  InvalidIP(String),

  #[error("IP location not found: {0}")]
  LocationNotFound(String),

  #[error("Unknown territory: {0}")]
  UnknownTerritory(String),

  #[error("No language data for territory: {0}")]
  NoLanguageData(String),

  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  #[error("Network error: {0}")]
  Network(String),

  #[error("IP error: {0}")]
  Ip(#[from] IpError),
}

/// Locale information.
#[derive(Debug, Clone)]
pub struct Locale {
  pub language: String,
  pub region: Option<String>,
  pub script: Option<String>,
}

impl Locale {
  /// Format locale as a string (e.g., "en-US").
  pub fn as_string(&self) -> String {
    if let Some(region) = &self.region {
      format!("{}-{}", self.language, region)
    } else {
      self.language.clone()
    }
  }

  /// Convert to config format for Camoufox.
  pub fn as_config(&self) -> HashMap<String, serde_json::Value> {
    let mut config = HashMap::new();

    if let Some(region) = &self.region {
      config.insert(
        "locale:region".to_string(),
        serde_json::json!(region.to_uppercase()),
      );
    }

    config.insert(
      "locale:language".to_string(),
      serde_json::json!(self.language.clone()),
    );

    if let Some(script) = &self.script {
      config.insert("locale:script".to_string(), serde_json::json!(script));
    }

    config
  }
}

/// Geolocation information.
#[derive(Debug, Clone)]
pub struct Geolocation {
  pub locale: Locale,
  pub longitude: f64,
  pub latitude: f64,
  pub timezone: String,
  pub accuracy: Option<f64>,
}

impl Geolocation {
  /// Convert to config format for Camoufox.
  pub fn as_config(&self) -> HashMap<String, serde_json::Value> {
    let mut config = self.locale.as_config();

    config.insert(
      "geolocation:longitude".to_string(),
      serde_json::json!(self.longitude),
    );
    config.insert(
      "geolocation:latitude".to_string(),
      serde_json::json!(self.latitude),
    );
    config.insert("timezone".to_string(), serde_json::json!(self.timezone));

    if let Some(accuracy) = self.accuracy {
      config.insert(
        "geolocation:accuracy".to_string(),
        serde_json::json!(accuracy),
      );
    }

    config
  }
}

/// Territory language population data.
struct LanguagePopulation {
  language: String,
  population_percent: f64,
}

/// Statistical locale selector based on territory language populations.
pub struct LocaleSelector {
  territories: HashMap<String, Vec<LanguagePopulation>>,
}

impl LocaleSelector {
  /// Create a new locale selector by parsing territory info XML.
  pub fn new() -> Result<Self, GeolocationError> {
    let mut territories: HashMap<String, Vec<LanguagePopulation>> = HashMap::new();

    let mut reader = XmlReader::from_str(data::TERRITORY_INFO_XML);
    reader.config_mut().trim_text(true);

    let mut current_territory: Option<String> = None;
    let mut current_languages: Vec<LanguagePopulation> = Vec::new();

    let mut buf = Vec::new();

    loop {
      match reader.read_event_into(&mut buf) {
        Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
          let name = e.name();
          let name_str = std::str::from_utf8(name.as_ref()).unwrap_or("");

          if name_str == "territory" {
            // Save previous territory if exists
            if let Some(code) = current_territory.take() {
              if !current_languages.is_empty() {
                territories.insert(code, std::mem::take(&mut current_languages));
              }
            }

            // Get territory type attribute
            for attr in e.attributes().flatten() {
              if attr.key.as_ref() == b"type" {
                current_territory = Some(String::from_utf8_lossy(&attr.value).to_uppercase());
              }
            }
          } else if name_str == "languagePopulation" && current_territory.is_some() {
            let mut lang_type = None;
            let mut pop_percent = 0.0;

            for attr in e.attributes().flatten() {
              match attr.key.as_ref() {
                b"type" => {
                  lang_type = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                b"populationPercent" => {
                  pop_percent = String::from_utf8_lossy(&attr.value).parse().unwrap_or(0.0);
                }
                _ => {}
              }
            }

            if let Some(lang) = lang_type {
              current_languages.push(LanguagePopulation {
                language: lang.replace('_', "-"),
                population_percent: pop_percent,
              });
            }
          }
        }
        Ok(Event::End(ref e)) => {
          let name_ref = e.name();
          let name = std::str::from_utf8(name_ref.as_ref()).unwrap_or("");
          if name == "territory" {
            // Save territory
            if let Some(code) = current_territory.take() {
              if !current_languages.is_empty() {
                territories.insert(code, std::mem::take(&mut current_languages));
              }
            }
          }
        }
        Ok(Event::Eof) => break,
        Err(e) => {
          log::warn!("Error parsing territory XML: {}", e);
          break;
        }
        _ => {}
      }
      buf.clear();
    }

    Ok(Self { territories })
  }

  /// Get a locale for a given region/country code.
  pub fn from_region(&self, region: &str) -> Result<Locale, GeolocationError> {
    let region_upper = region.to_uppercase();

    let languages = self
      .territories
      .get(&region_upper)
      .ok_or_else(|| GeolocationError::UnknownTerritory(region.to_string()))?;

    if languages.is_empty() {
      return Err(GeolocationError::NoLanguageData(region.to_string()));
    }

    // Weighted random selection based on population percentage
    let total: f64 = languages.iter().map(|l| l.population_percent).sum();
    let mut rng = rand::rng();
    let target = rng.random::<f64>() * total;
    let mut cumulative = 0.0;

    for lang in languages {
      cumulative += lang.population_percent;
      if cumulative >= target {
        return Ok(normalize_locale(&format!(
          "{}-{}",
          lang.language, region_upper
        )));
      }
    }

    // Fallback to first language
    let first_lang = &languages[0].language;
    Ok(normalize_locale(&format!(
      "{}-{}",
      first_lang, region_upper
    )))
  }
}

impl Default for LocaleSelector {
  fn default() -> Self {
    Self::new().unwrap_or(Self {
      territories: HashMap::new(),
    })
  }
}

/// Normalize a locale string to standard format.
fn normalize_locale(locale: &str) -> Locale {
  let parts: Vec<&str> = locale.split('-').collect();

  let language = parts
    .first()
    .map(|s| s.to_lowercase())
    .unwrap_or_else(|| "en".to_string());

  let region = parts.get(1).map(|s| s.to_uppercase());

  // Determine script based on language if needed
  let script = match language.as_str() {
    "zh" => {
      // Chinese - Traditional for TW/HK, Simplified otherwise
      if region.as_deref() == Some("TW") || region.as_deref() == Some("HK") {
        Some("Hant".to_string())
      } else {
        Some("Hans".to_string())
      }
    }
    "sr" => {
      // Serbian - can be Cyrillic or Latin
      Some("Cyrl".to_string())
    }
    _ => None,
  };

  Locale {
    language,
    region,
    script,
  }
}

/// Get the path to the GeoIP MMDB file.
fn get_mmdb_path() -> Result<PathBuf, GeolocationError> {
  let base_dirs = BaseDirs::new().ok_or(GeolocationError::DatabaseNotFound)?;

  #[cfg(target_os = "windows")]
  let cache_dir = base_dirs
    .data_local_dir()
    .join("camoufox")
    .join("camoufox")
    .join("Cache");

  #[cfg(target_os = "macos")]
  let cache_dir = base_dirs.cache_dir().join("camoufox");

  #[cfg(target_os = "linux")]
  let cache_dir = base_dirs.cache_dir().join("camoufox");

  #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
  let cache_dir = base_dirs.cache_dir().join("camoufox");

  Ok(cache_dir.join("GeoLite2-City.mmdb"))
}

/// Check if the GeoIP database is available.
pub fn is_geoip_available() -> bool {
  GeoIPDownloader::is_geoip_database_available()
}

/// Get geolocation information for an IP address.
pub fn get_geolocation(ip: &str) -> Result<Geolocation, GeolocationError> {
  let mmdb_path = get_mmdb_path()?;

  if !mmdb_path.exists() {
    return Err(GeolocationError::DatabaseNotFound);
  }

  let reader =
    Reader::open_readfile(&mmdb_path).map_err(|e| GeolocationError::DatabaseOpen(e.to_string()))?;

  let ip_addr: IpAddr =
    IpAddr::from_str(ip).map_err(|_| GeolocationError::InvalidIP(ip.to_string()))?;

  let lookup_result = reader
    .lookup(ip_addr)
    .map_err(|e| GeolocationError::LocationNotFound(e.to_string()))?;
  let city: geoip2::City = lookup_result
    .decode()
    .map_err(|e| GeolocationError::LocationNotFound(e.to_string()))?
    .ok_or_else(|| GeolocationError::LocationNotFound(ip.to_string()))?;

  // Extract location data
  let location = &city.location;

  let longitude = location
    .longitude
    .ok_or_else(|| GeolocationError::LocationNotFound("No longitude".to_string()))?;
  let latitude = location
    .latitude
    .ok_or_else(|| GeolocationError::LocationNotFound("No latitude".to_string()))?;
  let timezone = location
    .time_zone
    .ok_or_else(|| GeolocationError::LocationNotFound("No timezone".to_string()))?
    .to_string();

  // Get country code
  let country = &city.country;
  let iso_code = country
    .iso_code
    .ok_or_else(|| GeolocationError::LocationNotFound("No country code".to_string()))?
    .to_uppercase();

  // Get locale from territory data
  let selector = LocaleSelector::new()?;
  let locale = selector.from_region(&iso_code)?;

  Ok(Geolocation {
    locale,
    longitude,
    latitude,
    timezone,
    accuracy: location.accuracy_radius.map(|r| r as f64),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_locale_selector_creation() {
    let selector = LocaleSelector::new();
    assert!(selector.is_ok());
  }

  #[test]
  fn test_locale_from_region() {
    let selector = LocaleSelector::new().unwrap();

    // Test common regions
    let us_locale = selector.from_region("US");
    assert!(us_locale.is_ok());
    let us = us_locale.unwrap();
    assert_eq!(us.region, Some("US".to_string()));

    let de_locale = selector.from_region("DE");
    assert!(de_locale.is_ok());
    let de = de_locale.unwrap();
    assert_eq!(de.region, Some("DE".to_string()));
  }

  #[test]
  fn test_locale_as_string() {
    let locale = Locale {
      language: "en".to_string(),
      region: Some("US".to_string()),
      script: None,
    };
    assert_eq!(locale.as_string(), "en-US");

    let locale_no_region = Locale {
      language: "en".to_string(),
      region: None,
      script: None,
    };
    assert_eq!(locale_no_region.as_string(), "en");
  }

  #[test]
  fn test_normalize_locale() {
    let locale = normalize_locale("en-US");
    assert_eq!(locale.language, "en");
    assert_eq!(locale.region, Some("US".to_string()));
    assert!(locale.script.is_none());

    let zh_tw = normalize_locale("zh-TW");
    assert_eq!(zh_tw.language, "zh");
    assert_eq!(zh_tw.region, Some("TW".to_string()));
    assert_eq!(zh_tw.script, Some("Hant".to_string()));

    let zh_cn = normalize_locale("zh-CN");
    assert_eq!(zh_cn.script, Some("Hans".to_string()));
  }
}

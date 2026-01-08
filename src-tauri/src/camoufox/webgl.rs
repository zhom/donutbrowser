//! WebGL fingerprint sampling from SQLite database.
//!
//! Samples realistic WebGL configurations based on OS-specific probability distributions.

use rand::Rng;
use rusqlite::{Connection, Result as SqliteResult};
use std::collections::HashMap;
use std::io::Write;
use tempfile::NamedTempFile;

use crate::camoufox::data;

/// WebGL fingerprint data.
#[derive(Debug, Clone)]
pub struct WebGLData {
  pub vendor: String,
  pub renderer: String,
  pub config: HashMap<String, serde_json::Value>,
}

/// Error type for WebGL operations.
#[derive(Debug, thiserror::Error)]
pub enum WebGLError {
  #[error("SQLite error: {0}")]
  Sqlite(#[from] rusqlite::Error),

  #[error("JSON parsing error: {0}")]
  Json(#[from] serde_json::Error),

  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  #[error("No WebGL data found for OS: {0}")]
  NoDataForOS(String),

  #[error("Invalid vendor/renderer combination for OS {os}: {vendor}/{renderer}")]
  InvalidCombination {
    os: String,
    vendor: String,
    renderer: String,
  },
}

/// Sample a WebGL configuration for the given OS.
///
/// If `vendor` and `renderer` are provided, returns the specific configuration.
/// Otherwise, randomly samples based on OS-specific probability weights.
pub fn sample_webgl(
  os: &str,
  vendor: Option<&str>,
  renderer: Option<&str>,
) -> Result<WebGLData, WebGLError> {
  // Write embedded database to a temporary file
  let mut temp_file = NamedTempFile::new()?;
  temp_file.write_all(data::WEBGL_DATA_DB)?;
  let db_path = temp_file.path();

  let conn = Connection::open(db_path)?;

  // Validate OS
  let os_column = match os {
    "win" | "windows" => "win",
    "mac" | "macos" => "mac",
    "lin" | "linux" => "lin",
    _ => return Err(WebGLError::NoDataForOS(os.to_string())),
  };

  if let (Some(v), Some(r)) = (vendor, renderer) {
    sample_specific(&conn, os_column, v, r)
  } else {
    sample_random(&conn, os_column)
  }
}

fn sample_specific(
  conn: &Connection,
  os_column: &str,
  vendor: &str,
  renderer: &str,
) -> Result<WebGLData, WebGLError> {
  let query = format!(
    "SELECT vendor, renderer, data, {} FROM webgl_fingerprints WHERE vendor = ?1 AND renderer = ?2",
    os_column
  );

  let mut stmt = conn.prepare(&query)?;
  let mut rows = stmt.query([vendor, renderer])?;

  if let Some(row) = rows.next()? {
    let weight: f64 = row.get(3)?;
    if weight <= 0.0 {
      return Err(WebGLError::InvalidCombination {
        os: os_column.to_string(),
        vendor: vendor.to_string(),
        renderer: renderer.to_string(),
      });
    }

    let data_json: String = row.get(2)?;
    let config: HashMap<String, serde_json::Value> = serde_json::from_str(&data_json)?;

    Ok(WebGLData {
      vendor: vendor.to_string(),
      renderer: renderer.to_string(),
      config,
    })
  } else {
    Err(WebGLError::InvalidCombination {
      os: os_column.to_string(),
      vendor: vendor.to_string(),
      renderer: renderer.to_string(),
    })
  }
}

fn sample_random(conn: &Connection, os_column: &str) -> Result<WebGLData, WebGLError> {
  let query = format!(
    "SELECT vendor, renderer, data, {} FROM webgl_fingerprints WHERE {} > 0",
    os_column, os_column
  );

  let mut stmt = conn.prepare(&query)?;
  let rows: Vec<(String, String, String, f64)> = stmt
    .query_map([], |row| {
      Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, f64>(3)?,
      ))
    })?
    .collect::<SqliteResult<Vec<_>>>()?;

  if rows.is_empty() {
    return Err(WebGLError::NoDataForOS(os_column.to_string()));
  }

  // Calculate total weight
  let total_weight: f64 = rows.iter().map(|(_, _, _, w)| w).sum();

  // Weighted random selection
  let mut rng = rand::rng();
  let threshold = rng.random::<f64>() * total_weight;
  let mut cumulative = 0.0;

  for (vendor, renderer, data_json, weight) in &rows {
    cumulative += *weight;
    if cumulative >= threshold {
      let config: HashMap<String, serde_json::Value> = serde_json::from_str(data_json)?;
      return Ok(WebGLData {
        vendor: vendor.clone(),
        renderer: renderer.clone(),
        config,
      });
    }
  }

  // Fallback to last row
  let (vendor, renderer, data_json, _) = rows.last().unwrap();
  let config: HashMap<String, serde_json::Value> = serde_json::from_str(data_json)?;
  Ok(WebGLData {
    vendor: vendor.clone(),
    renderer: renderer.clone(),
    config,
  })
}

/// Get all possible vendor/renderer pairs for each OS.
pub fn get_possible_pairs() -> Result<HashMap<String, Vec<(String, String)>>, WebGLError> {
  // Write embedded database to a temporary file
  let mut temp_file = NamedTempFile::new()?;
  temp_file.write_all(data::WEBGL_DATA_DB)?;
  let db_path = temp_file.path();

  let conn = Connection::open(db_path)?;
  let mut result = HashMap::new();

  for os in &["win", "mac", "lin"] {
    let query = format!(
      "SELECT DISTINCT vendor, renderer FROM webgl_fingerprints WHERE {} > 0 ORDER BY {} DESC",
      os, os
    );

    let mut stmt = conn.prepare(&query)?;
    let pairs: Vec<(String, String)> = stmt
      .query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
      })?
      .collect::<SqliteResult<Vec<_>>>()?;

    result.insert(os.to_string(), pairs);
  }

  Ok(result)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_sample_webgl_windows() {
    let result = sample_webgl("win", None, None);
    assert!(
      result.is_ok(),
      "Failed to sample WebGL for Windows: {:?}",
      result.err()
    );

    let data = result.unwrap();
    assert!(!data.vendor.is_empty());
    assert!(!data.renderer.is_empty());
    assert!(!data.config.is_empty());
  }

  #[test]
  fn test_sample_webgl_macos() {
    let result = sample_webgl("mac", None, None);
    assert!(
      result.is_ok(),
      "Failed to sample WebGL for macOS: {:?}",
      result.err()
    );
  }

  #[test]
  fn test_sample_webgl_linux() {
    let result = sample_webgl("lin", None, None);
    assert!(
      result.is_ok(),
      "Failed to sample WebGL for Linux: {:?}",
      result.err()
    );
  }

  #[test]
  fn test_get_possible_pairs() {
    let result = get_possible_pairs();
    assert!(
      result.is_ok(),
      "Failed to get possible pairs: {:?}",
      result.err()
    );

    let pairs = result.unwrap();
    assert!(pairs.contains_key("win"));
    assert!(pairs.contains_key("mac"));
    assert!(pairs.contains_key("lin"));
    assert!(!pairs.get("win").unwrap().is_empty());
  }
}

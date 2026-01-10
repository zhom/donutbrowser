use crate::profile::manager::ProfileManager;
use crate::profile::BrowserProfile;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

/// Unified cookie representation that works across both browser types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedCookie {
  pub name: String,
  pub value: String,
  pub domain: String,
  pub path: String,
  pub expires: i64,
  pub is_secure: bool,
  pub is_http_only: bool,
  pub same_site: i32,
  pub creation_time: i64,
  pub last_accessed: i64,
}

/// Cookies grouped by domain for UI display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainCookies {
  pub domain: String,
  pub cookies: Vec<UnifiedCookie>,
  pub cookie_count: usize,
}

/// Result of reading cookies from a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieReadResult {
  pub profile_id: String,
  pub browser_type: String,
  pub domains: Vec<DomainCookies>,
  pub total_count: usize,
}

/// Request to copy specific cookies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieCopyRequest {
  pub source_profile_id: String,
  pub target_profile_ids: Vec<String>,
  pub selected_cookies: Vec<SelectedCookie>,
}

/// Identifies a specific cookie to copy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedCookie {
  pub domain: String,
  pub name: String,
}

/// Result of a copy operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieCopyResult {
  pub target_profile_id: String,
  pub cookies_copied: usize,
  pub cookies_replaced: usize,
  pub errors: Vec<String>,
}

pub struct CookieManager;

impl CookieManager {
  /// Windows epoch offset: seconds between 1601-01-01 and 1970-01-01
  const WINDOWS_EPOCH_DIFF: i64 = 11644473600;

  /// Get the cookie database path for a profile
  fn get_cookie_db_path(profile: &BrowserProfile, profiles_dir: &Path) -> Result<PathBuf, String> {
    let profile_data_path = profile.get_profile_data_path(profiles_dir);

    match profile.browser.as_str() {
      "wayfern" => {
        let path = profile_data_path.join("Default").join("Cookies");
        if path.exists() {
          Ok(path)
        } else {
          Err(format!("Cookie database not found at: {}", path.display()))
        }
      }
      "camoufox" => {
        let path = profile_data_path.join("cookies.sqlite");
        if path.exists() {
          Ok(path)
        } else {
          Err(format!("Cookie database not found at: {}", path.display()))
        }
      }
      _ => Err(format!(
        "Unsupported browser type for cookie operations: {}",
        profile.browser
      )),
    }
  }

  /// Convert Chrome timestamp (Windows epoch, microseconds) to Unix timestamp (seconds)
  fn chrome_time_to_unix(chrome_time: i64) -> i64 {
    if chrome_time == 0 {
      return 0;
    }
    (chrome_time / 1_000_000) - Self::WINDOWS_EPOCH_DIFF
  }

  /// Convert Unix timestamp (seconds) to Chrome timestamp (Windows epoch, microseconds)
  fn unix_to_chrome_time(unix_time: i64) -> i64 {
    if unix_time == 0 {
      return 0;
    }
    (unix_time + Self::WINDOWS_EPOCH_DIFF) * 1_000_000
  }

  /// Read cookies from a Firefox/Camoufox profile
  fn read_firefox_cookies(db_path: &Path) -> Result<Vec<UnifiedCookie>, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

    let mut stmt = conn
      .prepare(
        "SELECT name, value, host, path, expiry, isSecure, isHttpOnly,
                        sameSite, creationTime, lastAccessed
                 FROM moz_cookies",
      )
      .map_err(|e| format!("Failed to prepare statement: {e}"))?;

    let cookies = stmt
      .query_map([], |row| {
        Ok(UnifiedCookie {
          name: row.get(0)?,
          value: row.get(1)?,
          domain: row.get(2)?,
          path: row.get(3)?,
          expires: row.get(4)?,
          is_secure: row.get::<_, i32>(5)? != 0,
          is_http_only: row.get::<_, i32>(6)? != 0,
          same_site: row.get(7)?,
          creation_time: row.get::<_, i64>(8)? / 1_000_000,
          last_accessed: row.get::<_, i64>(9)? / 1_000_000,
        })
      })
      .map_err(|e| format!("Failed to query cookies: {e}"))?
      .collect::<Result<Vec<_>, _>>()
      .map_err(|e| format!("Failed to collect cookies: {e}"))?;

    Ok(cookies)
  }

  /// Read cookies from a Chrome/Wayfern profile
  fn read_chrome_cookies(db_path: &Path) -> Result<Vec<UnifiedCookie>, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

    let mut stmt = conn
      .prepare(
        "SELECT name, value, host_key, path, expires_utc, is_secure,
                        is_httponly, samesite, creation_utc, last_access_utc
                 FROM cookies",
      )
      .map_err(|e| format!("Failed to prepare statement: {e}"))?;

    let cookies = stmt
      .query_map([], |row| {
        Ok(UnifiedCookie {
          name: row.get(0)?,
          value: row.get(1)?,
          domain: row.get(2)?,
          path: row.get(3)?,
          expires: Self::chrome_time_to_unix(row.get(4)?),
          is_secure: row.get::<_, i32>(5)? != 0,
          is_http_only: row.get::<_, i32>(6)? != 0,
          same_site: row.get(7)?,
          creation_time: Self::chrome_time_to_unix(row.get(8)?),
          last_accessed: Self::chrome_time_to_unix(row.get(9)?),
        })
      })
      .map_err(|e| format!("Failed to query cookies: {e}"))?
      .collect::<Result<Vec<_>, _>>()
      .map_err(|e| format!("Failed to collect cookies: {e}"))?;

    Ok(cookies)
  }

  /// Write cookies to a Firefox/Camoufox profile
  fn write_firefox_cookies(
    db_path: &Path,
    cookies: &[UnifiedCookie],
  ) -> Result<(usize, usize), String> {
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

    let mut copied = 0;
    let mut replaced = 0;

    for cookie in cookies {
      let existing: Option<i64> = conn
        .query_row(
          "SELECT id FROM moz_cookies WHERE host = ?1 AND name = ?2 AND path = ?3",
          params![&cookie.domain, &cookie.name, &cookie.path],
          |row| row.get(0),
        )
        .ok();

      if existing.is_some() {
        conn
          .execute(
            "UPDATE moz_cookies SET value = ?1, expiry = ?2, isSecure = ?3,
                     isHttpOnly = ?4, sameSite = ?5, lastAccessed = ?6
                     WHERE host = ?7 AND name = ?8 AND path = ?9",
            params![
              &cookie.value,
              cookie.expires,
              cookie.is_secure as i32,
              cookie.is_http_only as i32,
              cookie.same_site,
              cookie.last_accessed * 1_000_000,
              &cookie.domain,
              &cookie.name,
              &cookie.path,
            ],
          )
          .map_err(|e| format!("Failed to update cookie: {e}"))?;
        replaced += 1;
      } else {
        conn
          .execute(
            "INSERT INTO moz_cookies
                     (originAttributes, name, value, host, path, expiry, lastAccessed,
                      creationTime, isSecure, isHttpOnly, sameSite, rawSameSite, schemeMap)
                     VALUES ('', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 2)",
            params![
              &cookie.name,
              &cookie.value,
              &cookie.domain,
              &cookie.path,
              cookie.expires,
              cookie.last_accessed * 1_000_000,
              cookie.creation_time * 1_000_000,
              cookie.is_secure as i32,
              cookie.is_http_only as i32,
              cookie.same_site,
            ],
          )
          .map_err(|e| format!("Failed to insert cookie: {e}"))?;
        copied += 1;
      }
    }

    Ok((copied, replaced))
  }

  /// Write cookies to a Chrome/Wayfern profile
  fn write_chrome_cookies(
    db_path: &Path,
    cookies: &[UnifiedCookie],
  ) -> Result<(usize, usize), String> {
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

    let mut copied = 0;
    let mut replaced = 0;

    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs() as i64;

    for cookie in cookies {
      let existing: Option<i64> = conn
        .query_row(
          "SELECT rowid FROM cookies WHERE host_key = ?1 AND name = ?2 AND path = ?3",
          params![&cookie.domain, &cookie.name, &cookie.path],
          |row| row.get(0),
        )
        .ok();

      if existing.is_some() {
        conn
          .execute(
            "UPDATE cookies SET value = ?1, expires_utc = ?2, is_secure = ?3,
                     is_httponly = ?4, samesite = ?5, last_access_utc = ?6, last_update_utc = ?7
                     WHERE host_key = ?8 AND name = ?9 AND path = ?10",
            params![
              &cookie.value,
              Self::unix_to_chrome_time(cookie.expires),
              cookie.is_secure as i32,
              cookie.is_http_only as i32,
              cookie.same_site,
              Self::unix_to_chrome_time(cookie.last_accessed),
              Self::unix_to_chrome_time(now),
              &cookie.domain,
              &cookie.name,
              &cookie.path,
            ],
          )
          .map_err(|e| format!("Failed to update cookie: {e}"))?;
        replaced += 1;
      } else {
        conn.execute(
                    "INSERT INTO cookies
                     (creation_utc, host_key, top_frame_site_key, name, value, encrypted_value,
                      path, expires_utc, is_secure, is_httponly, last_access_utc, has_expires,
                      is_persistent, priority, samesite, source_scheme, source_port, source_type,
                      has_cross_site_ancestor, last_update_utc)
                     VALUES (?1, ?2, '', ?3, ?4, X'', ?5, ?6, ?7, ?8, ?9, 1, 1, 1, ?10, 2, -1, 0, 0, ?11)",
                    params![
                        Self::unix_to_chrome_time(cookie.creation_time),
                        &cookie.domain,
                        &cookie.name,
                        &cookie.value,
                        &cookie.path,
                        Self::unix_to_chrome_time(cookie.expires),
                        cookie.is_secure as i32,
                        cookie.is_http_only as i32,
                        Self::unix_to_chrome_time(cookie.last_accessed),
                        cookie.same_site,
                        Self::unix_to_chrome_time(now),
                    ],
                )
                .map_err(|e| format!("Failed to insert cookie: {e}"))?;
        copied += 1;
      }
    }

    Ok((copied, replaced))
  }

  /// Public API: Read cookies from a profile
  pub fn read_cookies(profile_id: &str) -> Result<CookieReadResult, String> {
    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profiles = profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| format!("Profile not found: {profile_id}"))?;

    let db_path = Self::get_cookie_db_path(profile, &profiles_dir)?;

    let cookies = match profile.browser.as_str() {
      "camoufox" => Self::read_firefox_cookies(&db_path)?,
      "wayfern" => Self::read_chrome_cookies(&db_path)?,
      _ => return Err(format!("Unsupported browser type: {}", profile.browser)),
    };

    let mut domain_map: HashMap<String, Vec<UnifiedCookie>> = HashMap::new();

    for cookie in cookies {
      domain_map
        .entry(cookie.domain.clone())
        .or_default()
        .push(cookie);
    }

    let mut domains: Vec<DomainCookies> = domain_map
      .into_iter()
      .map(|(domain, cookies)| DomainCookies {
        domain,
        cookie_count: cookies.len(),
        cookies,
      })
      .collect();

    domains.sort_by(|a, b| a.domain.cmp(&b.domain));

    let total_count = domains.iter().map(|d| d.cookie_count).sum();

    Ok(CookieReadResult {
      profile_id: profile_id.to_string(),
      browser_type: profile.browser.clone(),
      domains,
      total_count,
    })
  }

  /// Public API: Copy cookies between profiles
  pub async fn copy_cookies(
    app_handle: &AppHandle,
    request: CookieCopyRequest,
  ) -> Result<Vec<CookieCopyResult>, String> {
    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profiles = profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let source = profiles
      .iter()
      .find(|p| p.id.to_string() == request.source_profile_id)
      .ok_or_else(|| format!("Source profile not found: {}", request.source_profile_id))?;

    let source_db_path = Self::get_cookie_db_path(source, &profiles_dir)?;
    let all_cookies = match source.browser.as_str() {
      "camoufox" => Self::read_firefox_cookies(&source_db_path)?,
      "wayfern" => Self::read_chrome_cookies(&source_db_path)?,
      _ => return Err(format!("Unsupported browser type: {}", source.browser)),
    };

    let cookies_to_copy: Vec<UnifiedCookie> = if request.selected_cookies.is_empty() {
      all_cookies
    } else {
      all_cookies
        .into_iter()
        .filter(|c| {
          request.selected_cookies.iter().any(|s| {
            if s.name.is_empty() {
              c.domain == s.domain
            } else {
              c.domain == s.domain && c.name == s.name
            }
          })
        })
        .collect()
    };

    let mut results = Vec::new();

    for target_id in &request.target_profile_ids {
      let target = match profiles.iter().find(|p| p.id.to_string() == *target_id) {
        Some(p) => p,
        None => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: 0,
            cookies_replaced: 0,
            errors: vec![format!("Profile not found: {target_id}")],
          });
          continue;
        }
      };

      let is_running = profile_manager
        .check_browser_status(app_handle.clone(), target)
        .await
        .unwrap_or(false);

      if is_running {
        results.push(CookieCopyResult {
          target_profile_id: target_id.clone(),
          cookies_copied: 0,
          cookies_replaced: 0,
          errors: vec![format!("Browser is running for profile: {}", target.name)],
        });
        continue;
      }

      let target_db_path = match Self::get_cookie_db_path(target, &profiles_dir) {
        Ok(p) => p,
        Err(e) => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: 0,
            cookies_replaced: 0,
            errors: vec![e],
          });
          continue;
        }
      };

      let write_result = match target.browser.as_str() {
        "camoufox" => Self::write_firefox_cookies(&target_db_path, &cookies_to_copy),
        "wayfern" => Self::write_chrome_cookies(&target_db_path, &cookies_to_copy),
        _ => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: 0,
            cookies_replaced: 0,
            errors: vec![format!("Unsupported browser: {}", target.browser)],
          });
          continue;
        }
      };

      match write_result {
        Ok((copied, replaced)) => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: copied,
            cookies_replaced: replaced,
            errors: vec![],
          });
        }
        Err(e) => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: 0,
            cookies_replaced: 0,
            errors: vec![e],
          });
        }
      }
    }

    Ok(results)
  }
}

use crate::profile::manager::ProfileManager;
use crate::profile::BrowserProfile;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

/// Chromium cookie decryption support for reading existing encrypted cookies.
/// Writes always go through the plaintext `value` column (see `write_chrome_cookies`),
/// so no encryption path is needed here — Chromium reads plaintext when
/// `encrypted_value` is empty, regardless of what other cookies store.
pub mod chrome_decrypt {
  use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
  use std::path::Path;

  type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

  const PBKDF2_ITERATIONS: u32 = 1;
  const KEY_LEN: usize = 16; // AES-128
  const SALT: &[u8] = b"saltysalt";
  const IV: [u8; 16] = [b' '; 16]; // 16 spaces

  fn derive_key(password: &[u8]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, SALT, PBKDF2_ITERATIONS, &mut key);
    key
  }

  /// Get the encryption key for Chrome cookies.
  /// Wayfern stores os_crypt_key as a file inside the profile's user-data-dir on all platforms.
  /// On macOS/Linux the key is a base64 string used as PBKDF2 password.
  pub fn get_encryption_key(profile_data_path: &Path) -> Option<[u8; KEY_LEN]> {
    let key_file = profile_data_path.join("os_crypt_key");
    if let Ok(contents) = std::fs::read_to_string(&key_file) {
      let contents = contents.trim();
      if !contents.is_empty() {
        return Some(derive_key(contents.as_bytes()));
      }
    }

    // Fallback for macOS: try system Keychain (for profiles created before file-based keys)
    #[cfg(target_os = "macos")]
    {
      let output = std::process::Command::new("security")
        .args([
          "find-generic-password",
          "-w",
          "-s",
          "Chromium Safe Storage",
          "-a",
          "Chromium",
        ])
        .output()
        .ok()?;
      if output.status.success() {
        let password = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !password.is_empty() {
          return Some(derive_key(password.as_bytes()));
        }
      }
    }

    None
  }

  /// Decrypt a Chrome encrypted cookie value.
  /// Chromium prefixes encrypted values with "v10" (macOS) or "v11" (Linux).
  pub fn decrypt(encrypted: &[u8], key: &[u8; KEY_LEN]) -> Option<String> {
    if encrypted.len() < 3 {
      return None;
    }
    // Check for v10/v11 prefix
    let prefix = &encrypted[..3];
    if prefix != b"v10" && prefix != b"v11" {
      return None;
    }
    let ciphertext = &encrypted[3..];
    if ciphertext.is_empty() {
      return Some(String::new());
    }

    let mut buf = ciphertext.to_vec();
    let decrypted = Aes128CbcDec::new(key.into(), &IV.into())
      .decrypt_padded_mut::<Pkcs7>(&mut buf)
      .ok()?;

    String::from_utf8(decrypted.to_vec()).ok()
  }
}

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

/// Result of a cookie import operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieImportResult {
  pub cookies_imported: usize,
  pub cookies_replaced: usize,
  pub errors: Vec<String>,
}

pub struct CookieManager;

impl CookieManager {
  /// Windows epoch offset: seconds between 1601-01-01 and 1970-01-01
  const WINDOWS_EPOCH_DIFF: i64 = 11644473600;

  /// Get the Chrome cookie encryption key for a Wayfern profile
  fn get_chrome_encryption_key(profile: &BrowserProfile, profiles_dir: &Path) -> Option<[u8; 16]> {
    let profile_data_path = profile.get_profile_data_path(profiles_dir);
    chrome_decrypt::get_encryption_key(&profile_data_path)
  }

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

  /// Read cookies from a Chrome/Wayfern profile.
  /// Handles encrypted cookies by decrypting encrypted_value using the profile's encryption key.
  fn read_chrome_cookies(
    db_path: &Path,
    encryption_key: Option<&[u8; 16]>,
  ) -> Result<Vec<UnifiedCookie>, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

    let mut stmt = conn
      .prepare(
        "SELECT name, value, host_key, path, expires_utc, is_secure,
                is_httponly, samesite, creation_utc, last_access_utc, encrypted_value
         FROM cookies",
      )
      .map_err(|e| format!("Failed to prepare statement: {e}"))?;

    let cookies = stmt
      .query_map([], |row| {
        let name: String = row.get(0)?;
        let plaintext_value: String = row.get(1)?;
        let domain: String = row.get(2)?;
        let path: String = row.get(3)?;
        let expires_utc: i64 = row.get(4)?;
        let is_secure: i32 = row.get(5)?;
        let is_httponly: i32 = row.get(6)?;
        let samesite: i32 = row.get(7)?;
        let creation_utc: i64 = row.get(8)?;
        let last_access_utc: i64 = row.get(9)?;
        let encrypted_value: Vec<u8> = row.get(10)?;

        // Use plaintext value if available, otherwise decrypt encrypted_value
        let value = if !plaintext_value.is_empty() {
          plaintext_value
        } else if !encrypted_value.is_empty() {
          encryption_key
            .and_then(|key| chrome_decrypt::decrypt(&encrypted_value, key))
            .unwrap_or_default()
        } else {
          String::new()
        };

        Ok(UnifiedCookie {
          name,
          value,
          domain,
          path,
          expires: Self::chrome_time_to_unix(expires_utc),
          is_secure: is_secure != 0,
          is_http_only: is_httponly != 0,
          same_site: samesite,
          creation_time: Self::chrome_time_to_unix(creation_utc),
          last_accessed: Self::chrome_time_to_unix(last_access_utc),
        })
      })
      .map_err(|e| format!("Failed to query cookies: {e}"))?
      .collect::<Result<Vec<_>, _>>()
      .map_err(|e| format!("Failed to collect cookies: {e}"))?;

    Ok(cookies)
  }

  /// Write cookies to a Firefox/Camoufox profile.
  ///
  /// Firefox's `moz_cookies.expiry` is "seconds since Unix epoch", so `expiry = 0`
  /// is interpreted as 1970-01-01 and purged on read. To let imported session
  /// cookies survive browser restart, we rewrite them to a far-future expiry.
  ///
  /// `schemeMap` is a bitfield (1 = HTTP, 2 = HTTPS, 3 = both). Setting it based
  /// on `is_secure` preserves Firefox's scheme-bound cookie enforcement.
  fn write_firefox_cookies(
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
    // Session cookies get 30 days of persistence so they survive restart.
    let session_cookie_expiry = now + 30 * 86400;

    for cookie in cookies {
      let expiry = if cookie.expires > 0 {
        cookie.expires
      } else {
        session_cookie_expiry
      };
      // schemeMap bitfield: 1 = HTTP, 2 = HTTPS
      let scheme_map: i32 = if cookie.is_secure { 2 } else { 1 };

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
                     isHttpOnly = ?4, sameSite = ?5, rawSameSite = ?5,
                     lastAccessed = ?6, schemeMap = ?7
                     WHERE host = ?8 AND name = ?9 AND path = ?10",
            params![
              &cookie.value,
              expiry,
              cookie.is_secure as i32,
              cookie.is_http_only as i32,
              cookie.same_site,
              cookie.last_accessed * 1_000_000,
              scheme_map,
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
                     VALUES ('', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11)",
            params![
              &cookie.name,
              &cookie.value,
              &cookie.domain,
              &cookie.path,
              expiry,
              cookie.last_accessed * 1_000_000,
              cookie.creation_time * 1_000_000,
              cookie.is_secure as i32,
              cookie.is_http_only as i32,
              cookie.same_site,
              scheme_map,
            ],
          )
          .map_err(|e| format!("Failed to insert cookie: {e}"))?;
        copied += 1;
      }
    }

    Ok((copied, replaced))
  }

  /// Write cookies to a Chrome/Wayfern profile.
  ///
  /// Always writes values as plaintext in the `value` column with an empty
  /// `encrypted_value`. Chromium reads plaintext on a per-row basis when
  /// `encrypted_value` is empty, so this mixes cleanly with any pre-existing
  /// encrypted cookies in the database. We avoid encrypting on write because
  /// the os_crypt key derivation between Wayfern's runtime and an external
  /// writer is not guaranteed to match, and a ciphertext Chromium can't
  /// decrypt silently produces an empty cookie value at runtime.
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
      // Session cookies (no expiry) must have has_expires/is_persistent = 0.
      // Otherwise Chromium interprets expires_utc=0 as 1601-01-01 (expired).
      let has_expires = if cookie.expires > 0 { 1 } else { 0 };
      let is_persistent = has_expires;
      // HTTPS cookies use 443, HTTP uses 80. source_port participates in
      // Chromium's scheme-bound cookie enforcement.
      let source_port: i32 = if cookie.is_secure { 443 } else { 80 };
      let source_scheme: i32 = if cookie.is_secure { 2 } else { 1 };

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
            "UPDATE cookies SET value = ?1, encrypted_value = x'', expires_utc = ?2, is_secure = ?3,
                     is_httponly = ?4, samesite = ?5, last_access_utc = ?6, last_update_utc = ?7,
                     has_expires = ?8, is_persistent = ?9, source_scheme = ?10, source_port = ?11
                     WHERE host_key = ?12 AND name = ?13 AND path = ?14",
            params![
              &cookie.value,
              Self::unix_to_chrome_time(cookie.expires),
              cookie.is_secure as i32,
              cookie.is_http_only as i32,
              cookie.same_site,
              Self::unix_to_chrome_time(cookie.last_accessed),
              Self::unix_to_chrome_time(now),
              has_expires,
              is_persistent,
              source_scheme,
              source_port,
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
            "INSERT INTO cookies
                     (creation_utc, host_key, top_frame_site_key, name, value, encrypted_value,
                      path, expires_utc, is_secure, is_httponly, last_access_utc, has_expires,
                      is_persistent, priority, samesite, source_scheme, source_port, source_type,
                      has_cross_site_ancestor, last_update_utc)
                     VALUES (?1, ?2, '', ?3, ?4, x'', ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?13, ?14, 0, 0, ?15)",
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
              has_expires,
              is_persistent,
              cookie.same_site,
              source_scheme,
              source_port,
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
      "wayfern" => {
        let key = Self::get_chrome_encryption_key(profile, &profiles_dir);
        Self::read_chrome_cookies(&db_path, key.as_ref())?
      }
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
      "wayfern" => {
        let key = Self::get_chrome_encryption_key(source, &profiles_dir);
        Self::read_chrome_cookies(&source_db_path, key.as_ref())?
      }
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

  /// Parse Netscape format cookies from text content
  fn parse_netscape_cookies(content: &str) -> (Vec<UnifiedCookie>, Vec<String>) {
    let mut cookies = Vec::new();
    let mut errors = Vec::new();
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs() as i64;

    for (i, line) in content.lines().enumerate() {
      let line = line.trim();
      if line.is_empty() || line.starts_with('#') {
        continue;
      }

      let fields: Vec<&str> = line.split('\t').collect();
      if fields.len() < 7 {
        errors.push(format!(
          "Line {}: expected 7 tab-separated fields, got {}",
          i + 1,
          fields.len()
        ));
        continue;
      }

      let domain = fields[0].to_string();
      let path = fields[2].to_string();
      let is_secure = fields[3].eq_ignore_ascii_case("TRUE");
      let expires = fields[4].parse::<i64>().unwrap_or(0);
      let name = fields[5].to_string();
      let value = fields[6].to_string();

      cookies.push(UnifiedCookie {
        name,
        value,
        domain,
        path,
        expires,
        is_secure,
        is_http_only: false,
        same_site: 0,
        creation_time: now,
        last_accessed: now,
      });
    }

    (cookies, errors)
  }

  /// Parse JSON format cookies (array of cookie objects, e.g. from browser extensions)
  fn parse_json_cookies(content: &str) -> (Vec<UnifiedCookie>, Vec<String>) {
    let mut cookies = Vec::new();
    let mut errors = Vec::new();
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs() as i64;

    let arr: Vec<Value> = match serde_json::from_str(content) {
      Ok(v) => v,
      Err(e) => {
        errors.push(format!("Failed to parse JSON: {e}"));
        return (cookies, errors);
      }
    };

    for (i, obj) in arr.iter().enumerate() {
      let name = match obj.get("name").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
          errors.push(format!("Cookie {}: missing 'name' field", i + 1));
          continue;
        }
      };
      let value = obj
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
      let domain = match obj.get("domain").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
          errors.push(format!("Cookie {}: missing 'domain' field", i + 1));
          continue;
        }
      };
      let path = obj
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("/")
        .to_string();
      let is_secure = obj.get("secure").and_then(|v| v.as_bool()).unwrap_or(false);
      let is_http_only = obj
        .get("httpOnly")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
      let is_session = obj
        .get("session")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
      let expires = if is_session {
        0
      } else {
        obj
          .get("expirationDate")
          .and_then(|v| v.as_f64())
          .map(|f| f as i64)
          .unwrap_or(0)
      };
      let same_site = obj
        .get("sameSite")
        .and_then(|v| v.as_str())
        .map(|s| match s {
          "lax" => 1,
          "strict" => 2,
          _ => 0, // "no_restriction" or unrecognized
        })
        .unwrap_or(0);

      cookies.push(UnifiedCookie {
        name,
        value,
        domain,
        path,
        expires,
        is_secure,
        is_http_only,
        same_site,
        creation_time: now,
        last_accessed: now,
      });
    }

    (cookies, errors)
  }

  /// Auto-detect cookie format and parse
  fn parse_cookies(content: &str) -> (Vec<UnifiedCookie>, Vec<String>) {
    let trimmed = content.trim();
    if trimmed.starts_with('[') && serde_json::from_str::<Vec<Value>>(trimmed).is_ok() {
      return Self::parse_json_cookies(trimmed);
    }
    Self::parse_netscape_cookies(content)
  }

  /// Format cookies as Netscape TXT
  pub fn format_netscape_cookies(cookies: &[UnifiedCookie]) -> String {
    let mut lines = Vec::new();
    lines.push("# Netscape HTTP Cookie File".to_string());
    for cookie in cookies {
      let flag = if cookie.domain.starts_with('.') {
        "TRUE"
      } else {
        "FALSE"
      };
      let secure = if cookie.is_secure { "TRUE" } else { "FALSE" };
      lines.push(format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        cookie.domain, flag, cookie.path, secure, cookie.expires, cookie.name, cookie.value
      ));
    }
    lines.join("\n")
  }

  /// Format cookies as JSON
  pub fn format_json_cookies(cookies: &[UnifiedCookie]) -> String {
    let arr: Vec<Value> = cookies
      .iter()
      .map(|c| {
        let same_site_str = match c.same_site {
          1 => "lax",
          2 => "strict",
          _ => "no_restriction",
        };
        serde_json::json!({
          "name": c.name,
          "value": c.value,
          "domain": c.domain,
          "path": c.path,
          "secure": c.is_secure,
          "httpOnly": c.is_http_only,
          "sameSite": same_site_str,
          "expirationDate": c.expires,
          "session": c.expires == 0,
          "hostOnly": !c.domain.starts_with('.'),
        })
      })
      .collect();
    serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".to_string())
  }

  /// Public API: Import cookies with auto-format detection
  pub async fn import_cookies(
    app_handle: &AppHandle,
    profile_id: &str,
    content: &str,
  ) -> Result<CookieImportResult, String> {
    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profiles = profile_manager
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| format!("Profile not found: {profile_id}"))?;

    let is_running = profile_manager
      .check_browser_status(app_handle.clone(), profile)
      .await
      .unwrap_or(false);

    if is_running {
      return Err(format!(
        "Cannot import cookies while browser is running for profile: {}",
        profile.name
      ));
    }

    let (cookies, parse_errors) = Self::parse_cookies(content);

    if cookies.is_empty() {
      return Err("No valid cookies found in the file".to_string());
    }

    let db_path = Self::get_cookie_db_path(profile, &profiles_dir)?;

    let write_result = match profile.browser.as_str() {
      "camoufox" => Self::write_firefox_cookies(&db_path, &cookies),
      "wayfern" => Self::write_chrome_cookies(&db_path, &cookies),
      _ => return Err(format!("Unsupported browser type: {}", profile.browser)),
    };

    match write_result {
      Ok((imported, replaced)) => Ok(CookieImportResult {
        cookies_imported: imported,
        cookies_replaced: replaced,
        errors: parse_errors,
      }),
      Err(e) => Err(format!("Failed to write cookies: {e}")),
    }
  }

  /// Public API: Export cookies from a profile in the specified format
  pub fn export_cookies(profile_id: &str, format: &str) -> Result<String, String> {
    let result = Self::read_cookies(profile_id)?;
    let all_cookies: Vec<UnifiedCookie> =
      result.domains.into_iter().flat_map(|d| d.cookies).collect();

    match format {
      "json" => Ok(Self::format_json_cookies(&all_cookies)),
      "netscape" => Ok(Self::format_netscape_cookies(&all_cookies)),
      _ => Err(format!("Unsupported export format: {format}")),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_parse_netscape_cookies_valid() {
    let content = "# Netscape HTTP Cookie File\n\
      .example.com\tTRUE\t/\tTRUE\t1700000000\tsession_id\tabc123\n\
      example.com\tFALSE\t/path\tFALSE\t0\ttoken\txyz";
    let (cookies, errors) = CookieManager::parse_netscape_cookies(content);
    assert_eq!(cookies.len(), 2);
    assert!(errors.is_empty());

    assert_eq!(cookies[0].domain, ".example.com");
    assert_eq!(cookies[0].name, "session_id");
    assert_eq!(cookies[0].value, "abc123");
    assert_eq!(cookies[0].path, "/");
    assert!(cookies[0].is_secure);
    assert_eq!(cookies[0].expires, 1700000000);

    assert_eq!(cookies[1].domain, "example.com");
    assert!(!cookies[1].is_secure);
    assert_eq!(cookies[1].expires, 0);
  }

  #[test]
  fn test_parse_netscape_cookies_skips_comments_and_blanks() {
    let content = "# Comment line\n\n  \n# Another comment\n\
      .test.com\tTRUE\t/\tFALSE\t0\tname\tvalue\n";
    let (cookies, errors) = CookieManager::parse_netscape_cookies(content);
    assert_eq!(cookies.len(), 1);
    assert!(errors.is_empty());
  }

  #[test]
  fn test_parse_netscape_cookies_malformed_lines() {
    let content = "not\tenough\tfields\n\
      .ok.com\tTRUE\t/\tFALSE\t0\tname\tvalue\n";
    let (cookies, errors) = CookieManager::parse_netscape_cookies(content);
    assert_eq!(cookies.len(), 1);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("expected 7 tab-separated fields"));
  }

  #[test]
  fn test_parse_json_cookies_valid() {
    let content = r#"[
      {
        "name": "sid",
        "value": "abc",
        "domain": ".example.com",
        "path": "/",
        "secure": true,
        "httpOnly": true,
        "sameSite": "lax",
        "expirationDate": 1700000000,
        "session": false
      }
    ]"#;
    let (cookies, errors) = CookieManager::parse_json_cookies(content);
    assert_eq!(cookies.len(), 1);
    assert!(errors.is_empty());
    assert_eq!(cookies[0].name, "sid");
    assert_eq!(cookies[0].domain, ".example.com");
    assert!(cookies[0].is_secure);
    assert!(cookies[0].is_http_only);
    assert_eq!(cookies[0].same_site, 1);
    assert_eq!(cookies[0].expires, 1700000000);
  }

  #[test]
  fn test_parse_json_cookies_session() {
    let content = r#"[{"name": "s", "value": "v", "domain": ".d.com", "session": true, "expirationDate": 9999}]"#;
    let (cookies, errors) = CookieManager::parse_json_cookies(content);
    assert_eq!(cookies.len(), 1);
    assert!(errors.is_empty());
    assert_eq!(cookies[0].expires, 0);
  }

  #[test]
  fn test_parse_json_cookies_same_site_mapping() {
    let content = r#"[
      {"name": "a", "value": "", "domain": ".d.com", "sameSite": "no_restriction"},
      {"name": "b", "value": "", "domain": ".d.com", "sameSite": "lax"},
      {"name": "c", "value": "", "domain": ".d.com", "sameSite": "strict"}
    ]"#;
    let (cookies, _) = CookieManager::parse_json_cookies(content);
    assert_eq!(cookies[0].same_site, 0);
    assert_eq!(cookies[1].same_site, 1);
    assert_eq!(cookies[2].same_site, 2);
  }

  #[test]
  fn test_parse_cookies_auto_detect_json() {
    let content = r#"[{"name": "x", "value": "y", "domain": ".test.com"}]"#;
    let (cookies, _) = CookieManager::parse_cookies(content);
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "x");
  }

  #[test]
  fn test_parse_cookies_auto_detect_netscape() {
    let content = ".test.com\tTRUE\t/\tFALSE\t0\tname\tvalue";
    let (cookies, _) = CookieManager::parse_cookies(content);
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "name");
  }

  #[test]
  fn test_format_netscape_cookies() {
    let cookies = vec![UnifiedCookie {
      name: "sid".to_string(),
      value: "abc".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1700000000,
      is_secure: true,
      is_http_only: false,
      same_site: 0,
      creation_time: 0,
      last_accessed: 0,
    }];
    let output = CookieManager::format_netscape_cookies(&cookies);
    assert!(output.contains("# Netscape HTTP Cookie File"));
    assert!(output.contains(".example.com\tTRUE\t/\tTRUE\t1700000000\tsid\tabc"));
  }

  #[test]
  fn test_format_json_cookies() {
    let cookies = vec![UnifiedCookie {
      name: "sid".to_string(),
      value: "abc".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1700000000,
      is_secure: true,
      is_http_only: true,
      same_site: 1,
      creation_time: 0,
      last_accessed: 0,
    }];
    let output = CookieManager::format_json_cookies(&cookies);
    let parsed: Vec<Value> = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["name"], "sid");
    assert_eq!(parsed[0]["sameSite"], "lax");
    assert_eq!(parsed[0]["session"], false);
    assert_eq!(parsed[0]["hostOnly"], false);
  }

  #[test]
  fn test_netscape_roundtrip() {
    let cookies = vec![
      UnifiedCookie {
        name: "a".to_string(),
        value: "1".to_string(),
        domain: ".d.com".to_string(),
        path: "/".to_string(),
        expires: 1700000000,
        is_secure: true,
        is_http_only: false,
        same_site: 0,
        creation_time: 0,
        last_accessed: 0,
      },
      UnifiedCookie {
        name: "b".to_string(),
        value: "2".to_string(),
        domain: "d.com".to_string(),
        path: "/p".to_string(),
        expires: 0,
        is_secure: false,
        is_http_only: false,
        same_site: 0,
        creation_time: 0,
        last_accessed: 0,
      },
    ];
    let formatted = CookieManager::format_netscape_cookies(&cookies);
    let (parsed, errors) = CookieManager::parse_netscape_cookies(&formatted);
    assert!(errors.is_empty());
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].name, "a");
    assert_eq!(parsed[0].domain, ".d.com");
    assert!(parsed[0].is_secure);
    assert_eq!(parsed[1].name, "b");
    assert_eq!(parsed[1].domain, "d.com");
  }

  #[test]
  fn test_json_roundtrip() {
    let cookies = vec![UnifiedCookie {
      name: "tok".to_string(),
      value: "xyz".to_string(),
      domain: ".site.org".to_string(),
      path: "/app".to_string(),
      expires: 1700000000,
      is_secure: false,
      is_http_only: true,
      same_site: 2,
      creation_time: 0,
      last_accessed: 0,
    }];
    let formatted = CookieManager::format_json_cookies(&cookies);
    let (parsed, errors) = CookieManager::parse_json_cookies(&formatted);
    assert!(errors.is_empty());
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "tok");
    assert_eq!(parsed[0].domain, ".site.org");
    assert_eq!(parsed[0].path, "/app");
    assert!(!parsed[0].is_secure);
    assert!(parsed[0].is_http_only);
    assert_eq!(parsed[0].same_site, 2);
    assert_eq!(parsed[0].expires, 1700000000);
  }

  #[test]
  fn test_chrome_time_to_unix() {
    assert_eq!(CookieManager::chrome_time_to_unix(0), 0);
    let chrome_time: i64 = (1700000000 + CookieManager::WINDOWS_EPOCH_DIFF) * 1_000_000;
    assert_eq!(CookieManager::chrome_time_to_unix(chrome_time), 1700000000);
  }

  #[test]
  fn test_unix_to_chrome_time() {
    assert_eq!(CookieManager::unix_to_chrome_time(0), 0);
    let expected = (1700000000 + CookieManager::WINDOWS_EPOCH_DIFF) * 1_000_000;
    assert_eq!(CookieManager::unix_to_chrome_time(1700000000), expected);
  }

  #[test]
  fn test_chrome_time_roundtrip() {
    let unix = 1700000000_i64;
    let chrome = CookieManager::unix_to_chrome_time(unix);
    assert_eq!(CookieManager::chrome_time_to_unix(chrome), unix);
  }

  /// Set up a minimal Chrome cookie SQLite schema for testing writes.
  fn create_chrome_cookies_db(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn
      .execute_batch(
        "CREATE TABLE cookies (
          creation_utc INTEGER NOT NULL,
          host_key TEXT NOT NULL,
          top_frame_site_key TEXT NOT NULL,
          name TEXT NOT NULL,
          value TEXT NOT NULL,
          encrypted_value BLOB NOT NULL DEFAULT '',
          path TEXT NOT NULL,
          expires_utc INTEGER NOT NULL,
          is_secure INTEGER NOT NULL,
          is_httponly INTEGER NOT NULL,
          last_access_utc INTEGER NOT NULL,
          has_expires INTEGER NOT NULL DEFAULT 1,
          is_persistent INTEGER NOT NULL DEFAULT 1,
          priority INTEGER NOT NULL DEFAULT 1,
          samesite INTEGER NOT NULL DEFAULT -1,
          source_scheme INTEGER NOT NULL DEFAULT 0,
          source_port INTEGER NOT NULL DEFAULT -1,
          last_update_utc INTEGER NOT NULL DEFAULT 0,
          source_type INTEGER NOT NULL DEFAULT 0,
          has_cross_site_ancestor INTEGER NOT NULL DEFAULT 0
        );",
      )
      .unwrap();
  }

  /// Set up a minimal Firefox moz_cookies SQLite schema for testing writes.
  fn create_firefox_cookies_db(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn
      .execute_batch(
        "CREATE TABLE moz_cookies (
          id INTEGER PRIMARY KEY,
          originAttributes TEXT NOT NULL DEFAULT '',
          name TEXT,
          value TEXT,
          host TEXT,
          path TEXT,
          expiry INTEGER,
          lastAccessed INTEGER,
          creationTime INTEGER,
          isSecure INTEGER,
          isHttpOnly INTEGER,
          inBrowserElement INTEGER DEFAULT 0,
          sameSite INTEGER DEFAULT 0,
          rawSameSite INTEGER DEFAULT 0,
          schemeMap INTEGER DEFAULT 0,
          CONSTRAINT moz_uniqueid UNIQUE (name, host, path, originAttributes)
        );",
      )
      .unwrap();
  }

  #[test]
  fn test_write_chrome_cookies_stores_plaintext_values() {
    let tmp = std::env::temp_dir().join(format!("donut_cookie_test_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&tmp);

    let cookies = vec![UnifiedCookie {
      name: "c_user".to_string(),
      value: "100012345".to_string(),
      domain: ".facebook.com".to_string(),
      path: "/".to_string(),
      expires: 1800000000,
      is_secure: true,
      is_http_only: true,
      same_site: 0,
      creation_time: 1700000000,
      last_accessed: 1700000000,
    }];

    let (inserted, replaced) = CookieManager::write_chrome_cookies(&tmp, &cookies).unwrap();
    assert_eq!(inserted, 1);
    assert_eq!(replaced, 0);

    let conn = Connection::open(&tmp).unwrap();
    let (value, encrypted, has_expires, is_persistent, source_scheme, source_port): (
      String,
      Vec<u8>,
      i32,
      i32,
      i32,
      i32,
    ) = conn
      .query_row(
        "SELECT value, encrypted_value, has_expires, is_persistent, source_scheme, source_port
         FROM cookies WHERE name = ?1",
        params!["c_user"],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
          ))
        },
      )
      .unwrap();

    // Core fix: plaintext in value, empty encrypted_value
    assert_eq!(value, "100012345");
    assert!(encrypted.is_empty());
    // Persistent cookie since expires > 0
    assert_eq!(has_expires, 1);
    assert_eq!(is_persistent, 1);
    // Secure cookie gets HTTPS scheme + port 443
    assert_eq!(source_scheme, 2);
    assert_eq!(source_port, 443);

    let _ = std::fs::remove_file(&tmp);
  }

  #[test]
  fn test_write_chrome_cookies_session_cookie_not_expired() {
    let tmp = std::env::temp_dir().join(format!("donut_cookie_test_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&tmp);

    let cookies = vec![UnifiedCookie {
      name: "session".to_string(),
      value: "abc".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 0, // session cookie
      is_secure: false,
      is_http_only: false,
      same_site: 0,
      creation_time: 1700000000,
      last_accessed: 1700000000,
    }];

    CookieManager::write_chrome_cookies(&tmp, &cookies).unwrap();

    let conn = Connection::open(&tmp).unwrap();
    let (has_expires, is_persistent, source_scheme, source_port): (i32, i32, i32, i32) = conn
      .query_row(
        "SELECT has_expires, is_persistent, source_scheme, source_port
         FROM cookies WHERE name = ?1",
        params!["session"],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .unwrap();

    // Session cookie must not be persistent — otherwise Chromium treats
    // expires_utc=0 as 1601-01-01 (immediately expired).
    assert_eq!(has_expires, 0);
    assert_eq!(is_persistent, 0);
    // Non-secure cookie uses HTTP scheme + port 80
    assert_eq!(source_scheme, 1);
    assert_eq!(source_port, 80);

    let _ = std::fs::remove_file(&tmp);
  }

  #[test]
  fn test_write_chrome_cookies_replaces_existing() {
    let tmp = std::env::temp_dir().join(format!("donut_cookie_test_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&tmp);

    let cookie = UnifiedCookie {
      name: "token".to_string(),
      value: "v1".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1800000000,
      is_secure: true,
      is_http_only: false,
      same_site: 1,
      creation_time: 1700000000,
      last_accessed: 1700000000,
    };

    let (inserted, _) =
      CookieManager::write_chrome_cookies(&tmp, std::slice::from_ref(&cookie)).unwrap();
    assert_eq!(inserted, 1);

    let mut updated = cookie.clone();
    updated.value = "v2".to_string();
    let (inserted, replaced) =
      CookieManager::write_chrome_cookies(&tmp, std::slice::from_ref(&updated)).unwrap();
    assert_eq!(inserted, 0);
    assert_eq!(replaced, 1);

    let conn = Connection::open(&tmp).unwrap();
    let (value, encrypted): (String, Vec<u8>) = conn
      .query_row(
        "SELECT value, encrypted_value FROM cookies WHERE name = ?1",
        params!["token"],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .unwrap();
    assert_eq!(value, "v2");
    assert!(encrypted.is_empty());

    let _ = std::fs::remove_file(&tmp);
  }

  /// Wayfern → Camoufox: write cookies to a Chrome DB, read them back, and
  /// verify they land in a Firefox DB with values intact, correct schemeMap,
  /// and non-expired timestamps. This is the path exercised by the
  /// "copy cookies between profiles of different browser types" feature.
  #[test]
  fn test_wayfern_cookies_transfer_to_camoufox() {
    let chrome_db =
      std::env::temp_dir().join(format!("donut_xbrowser_chrome_{}.db", uuid::Uuid::new_v4()));
    let ff_db = std::env::temp_dir().join(format!("donut_xbrowser_ff_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&chrome_db);
    create_firefox_cookies_db(&ff_db);

    // Simulate cookies in a Wayfern profile: a persistent cookie and a
    // session cookie, both from a real-world HTTPS site.
    let source_cookies = vec![
      UnifiedCookie {
        name: "c_user".to_string(),
        value: "100012345678".to_string(),
        domain: ".facebook.com".to_string(),
        path: "/".to_string(),
        expires: 1900000000, // persistent, far in the future
        is_secure: true,
        is_http_only: true,
        same_site: 0,
        creation_time: 1700000000,
        last_accessed: 1700000000,
      },
      UnifiedCookie {
        name: "xs".to_string(),
        value: "sessionvalue".to_string(),
        domain: ".facebook.com".to_string(),
        path: "/".to_string(),
        expires: 0, // session cookie
        is_secure: true,
        is_http_only: true,
        same_site: 1,
        creation_time: 1700000000,
        last_accessed: 1700000000,
      },
    ];
    CookieManager::write_chrome_cookies(&chrome_db, &source_cookies).unwrap();

    // Read back from the Chrome DB (as if reading from the Wayfern profile).
    let from_chrome = CookieManager::read_chrome_cookies(&chrome_db, None).unwrap();
    assert_eq!(from_chrome.len(), 2);
    let c_user_src = from_chrome.iter().find(|c| c.name == "c_user").unwrap();
    assert_eq!(c_user_src.value, "100012345678");
    let xs_src = from_chrome.iter().find(|c| c.name == "xs").unwrap();
    assert_eq!(xs_src.value, "sessionvalue");

    // Write them into the Camoufox (Firefox) DB.
    let (inserted, replaced) = CookieManager::write_firefox_cookies(&ff_db, &from_chrome).unwrap();
    assert_eq!(inserted, 2);
    assert_eq!(replaced, 0);

    // Read back from Firefox and verify values survived the round trip.
    let from_ff = CookieManager::read_firefox_cookies(&ff_db).unwrap();
    assert_eq!(from_ff.len(), 2);
    let c_user = from_ff.iter().find(|c| c.name == "c_user").unwrap();
    assert_eq!(c_user.value, "100012345678");
    assert_eq!(c_user.domain, ".facebook.com");
    assert!(c_user.is_secure);
    assert!(c_user.is_http_only);
    let xs = from_ff.iter().find(|c| c.name == "xs").unwrap();
    assert_eq!(xs.value, "sessionvalue");

    // Raw DB checks against the Firefox schema — these would catch the bugs
    // that caused issue #265 on the Chrome path (plaintext, correct expiry,
    // correct schemeMap).
    let conn = Connection::open(&ff_db).unwrap();
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs() as i64;

    let (c_user_expiry, c_user_scheme): (i64, i32) = conn
      .query_row(
        "SELECT expiry, schemeMap FROM moz_cookies WHERE name = ?1",
        params!["c_user"],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .unwrap();
    assert!(
      c_user_expiry > now,
      "persistent cookie must not be expired in firefox (expiry={c_user_expiry}, now={now})"
    );
    assert_eq!(c_user_scheme, 2, "HTTPS cookie must have schemeMap=2");

    let (xs_expiry, xs_scheme): (i64, i32) = conn
      .query_row(
        "SELECT expiry, schemeMap FROM moz_cookies WHERE name = ?1",
        params!["xs"],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .unwrap();
    assert!(
      xs_expiry > now,
      "session cookie must be rewritten to a future expiry (got {xs_expiry}, now={now})"
    );
    assert_eq!(xs_scheme, 2);

    let _ = std::fs::remove_file(&chrome_db);
    let _ = std::fs::remove_file(&ff_db);
  }

  /// Camoufox → Wayfern: the reverse direction. Ensures the Chrome writer
  /// still produces plaintext values / empty encrypted_value when fed cookies
  /// that originated in Firefox.
  #[test]
  fn test_camoufox_cookies_transfer_to_wayfern() {
    let ff_db =
      std::env::temp_dir().join(format!("donut_xbrowser_rev_ff_{}.db", uuid::Uuid::new_v4()));
    let chrome_db = std::env::temp_dir().join(format!(
      "donut_xbrowser_rev_chrome_{}.db",
      uuid::Uuid::new_v4()
    ));
    create_firefox_cookies_db(&ff_db);
    create_chrome_cookies_db(&chrome_db);

    let source_cookies = vec![UnifiedCookie {
      name: "sessionid".to_string(),
      value: "abc123def456".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1900000000,
      is_secure: true,
      is_http_only: false,
      same_site: 1,
      creation_time: 1700000000,
      last_accessed: 1700000000,
    }];
    CookieManager::write_firefox_cookies(&ff_db, &source_cookies).unwrap();

    let from_ff = CookieManager::read_firefox_cookies(&ff_db).unwrap();
    assert_eq!(from_ff.len(), 1);
    assert_eq!(from_ff[0].value, "abc123def456");

    CookieManager::write_chrome_cookies(&chrome_db, &from_ff).unwrap();

    let from_chrome = CookieManager::read_chrome_cookies(&chrome_db, None).unwrap();
    assert_eq!(from_chrome.len(), 1);
    assert_eq!(from_chrome[0].value, "abc123def456");

    // Verify the raw DB state on the Chrome side — plaintext value, empty
    // encrypted_value, persistent, HTTPS.
    let conn = Connection::open(&chrome_db).unwrap();
    let (value, encrypted, is_persistent, source_scheme): (String, Vec<u8>, i32, i32) = conn
      .query_row(
        "SELECT value, encrypted_value, is_persistent, source_scheme
         FROM cookies WHERE name = ?1",
        params!["sessionid"],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
      )
      .unwrap();
    assert_eq!(value, "abc123def456");
    assert!(encrypted.is_empty());
    assert_eq!(is_persistent, 1);
    assert_eq!(source_scheme, 2);

    let _ = std::fs::remove_file(&ff_db);
    let _ = std::fs::remove_file(&chrome_db);
  }
}

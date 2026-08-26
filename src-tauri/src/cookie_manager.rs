use crate::profile::manager::ProfileManager;
use crate::profile::BrowserProfile;
use rusqlite::{params, Connection, OpenFlags};
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
  use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
  use ring::pbkdf2;
  use sha2::{Digest, Sha256};
  use std::num::NonZeroU32;
  use std::path::Path;

  type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

  /// PBKDF2 iteration count for deriving the AES key from the password stored
  /// in `os_crypt_key`. Must match Chromium's `OSCryptImpl` on each platform:
  /// macOS uses 1003 iterations, Linux uses 1. Getting this wrong produces a
  /// different AES key → silent decryption failure → empty cookie values.
  /// See `components/os_crypt/sync/os_crypt_{mac.mm,linux.cc}` in Chromium.
  #[cfg(target_os = "macos")]
  const PBKDF2_ITERATIONS: u32 = 1003;
  #[cfg(not(target_os = "macos"))]
  const PBKDF2_ITERATIONS: u32 = 1;

  const KEY_LEN: usize = 16; // AES-128
  const SALT: &[u8] = b"saltysalt";
  const IV: [u8; 16] = [b' '; 16]; // 16 spaces
  const HOST_HASH_LEN: usize = 32; // SHA-256 output length

  fn derive_key(password: &[u8]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    // Using ring::pbkdf2 instead of the `pbkdf2` crate to avoid digest
    // version conflicts between sha1 0.11 (digest 0.11) and pbkdf2 0.12
    // (digest 0.10). ring's implementation is self-contained.
    pbkdf2::derive(
      pbkdf2::PBKDF2_HMAC_SHA1,
      NonZeroU32::new(PBKDF2_ITERATIONS).expect("iterations must be non-zero"),
      SALT,
      password,
      &mut key,
    );
    key
  }

  pub fn get_encryption_key(profile_data_path: &Path) -> Option<[u8; KEY_LEN]> {
    let key_file = profile_data_path.join("os_crypt_key");
    // Read as raw bytes and do NOT trim — Chromium's `ReadFileToString`
    // passes the exact file contents to `Pbkdf2(file_contents)`. Any
    // normalisation we do here would produce a different derived key.
    let contents = std::fs::read(&key_file).ok()?;
    if contents.is_empty() {
      return None;
    }
    Some(derive_key(&contents))
  }

  /// Decrypt a Chrome encrypted cookie value.
  ///
  /// Chromium prefixes encrypted values with "v10" / "v11" and, since ~M100,
  /// prepends `SHA-256(host_key)` to the plaintext before encryption as an
  /// integrity check. After decryption we verify and strip those 32 bytes
  /// when present. Passing `host_key` is required to do that verification —
  /// without it we'd return 32 bytes of hash noise plus the actual value,
  /// which is not valid UTF-8 and gets thrown away.
  pub fn decrypt(encrypted: &[u8], host_key: &str, key: &[u8; KEY_LEN]) -> Option<String> {
    if encrypted.len() < 3 {
      return None;
    }
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
      .decrypt_padded::<Pkcs7>(&mut buf)
      .ok()?;

    // Strip the SHA-256(host_key) integrity prefix if present. Older cookies
    // (pre-M100) didn't have this prefix, so we fall back to the raw bytes
    // when the first 32 bytes don't match the expected hash.
    if decrypted.len() >= HOST_HASH_LEN {
      let expected: [u8; HOST_HASH_LEN] = Sha256::digest(host_key.as_bytes()).into();
      if decrypted[..HOST_HASH_LEN] == expected {
        return String::from_utf8(decrypted[HOST_HASH_LEN..].to_vec()).ok();
      }
    }

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

/// Lightweight cookie metadata for the profile-info dialog. Computed without
/// decrypting any cookie values, so it stays cheap even for multi-MB Chromium
/// cookie stores and never blocks the runtime for noticeable time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieStats {
  pub profile_id: String,
  pub browser_type: String,
  pub total_count: usize,
  /// Every domain the profile has cookies for, sorted by cookie count desc.
  pub domains: Vec<DomainCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainCount {
  pub domain: String,
  pub count: usize,
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

/// What a write does to cookies the profile already has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CookieWriteMode {
  /// Update the rows the incoming cookies match, insert the rest, delete
  /// nothing.
  #[default]
  Merge,
  /// Clear every cookie the profile holds for the sites named in this write
  /// before applying it, so the result is exactly what was pasted. Subdomains
  /// and every other site are left alone.
  ReplaceMatchingSites,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieWriteCounts {
  pub added: usize,
  pub overwritten: usize,
  pub deleted: usize,
}

/// One accepted cookie, as the paste preview shows it.
///
/// Deliberately has no `value`: the value IS the credential, and a preview that
/// carries it puts every pasted session token into the frontend's state, its
/// logs and any screenshot of the dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastedCookiePreview {
  pub name: String,
  pub domain: String,
  pub path: String,
  pub expires: i64,
  pub is_secure: bool,
  pub is_http_only: bool,
  pub same_site: i32,
}

impl From<&UnifiedCookie> for PastedCookiePreview {
  fn from(cookie: &UnifiedCookie) -> Self {
    Self {
      name: cookie.name.clone(),
      domain: cookie.domain.clone(),
      path: cookie.path.clone(),
      expires: cookie.expires,
      is_secure: cookie.is_secure,
      is_http_only: cookie.is_http_only,
      same_site: cookie.same_site,
    }
  }
}

/// Everything the paste dialog needs to describe a paste before writing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookiePasteAnalysis {
  pub format: Option<crate::cookie_paste::PasteFormat>,
  pub cookies: Vec<PastedCookiePreview>,
  pub issues: Vec<crate::cookie_paste::CookieIssue>,
  pub site_required: bool,
  pub expired_count: usize,
  /// How many stored rows [`CookieWriteMode::ReplaceMatchingSites`] would
  /// delete for the sites this paste names. `None` when the store cannot be
  /// read — a profile that has never launched, a locked database, a browser
  /// with no supported cookie store.
  pub replace_delete_count: Option<usize>,
  /// The profile wipes its browsing data when the browser exits, so an import
  /// here lives exactly one session. Not an error, but silently losing a pasted
  /// login is the worst thing this feature can do.
  pub clears_on_close: bool,
  /// The `{"code":…}` string explaining why an import would be refused right
  /// now, ready for `translateBackendError`. `None` when the import can run.
  pub blocked_by: Option<String>,
}

/// Result of writing a paste into a profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookiePasteImportResult {
  pub added: usize,
  pub overwritten: usize,
  pub deleted: usize,
  /// Cookies that parsed cleanly but were not written, i.e. expired ones the
  /// caller chose to leave out.
  pub skipped: usize,
  pub issues: Vec<crate::cookie_paste::CookieIssue>,
}

pub struct CookieManager;

impl CookieManager {
  /// Windows epoch offset: seconds between 1601-01-01 and 1970-01-01
  const WINDOWS_EPOCH_DIFF: i64 = 11644473600;

  fn get_chrome_encryption_key(profile: &BrowserProfile, profiles_dir: &Path) -> Option<[u8; 16]> {
    let profile_data_path = profile.get_profile_data_path(profiles_dir);
    chrome_decrypt::get_encryption_key(&profile_data_path)
  }

  fn wayfern_cookie_path(profile_data_path: &Path) -> PathBuf {
    let default_dir = profile_data_path.join("Default");
    #[cfg(target_os = "windows")]
    {
      default_dir.join("Network").join("Cookies")
    }
    #[cfg(not(target_os = "windows"))]
    {
      default_dir.join("Cookies")
    }
  }

  /// Get the cookie database path for a profile (read-side: errors if missing).
  fn get_cookie_db_path(profile: &BrowserProfile, profiles_dir: &Path) -> Result<PathBuf, String> {
    let profile_data_path = profile.get_profile_data_path(profiles_dir);

    match profile.browser.as_str() {
      "wayfern" => {
        let path = Self::wayfern_cookie_path(&profile_data_path);
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

  /// Get the cookie database path for a profile, creating an empty
  /// browser-compatible database if it doesn't exist yet. Use this for write
  /// paths (copy / import) so we can populate the cookie store of a profile
  /// that has never been launched.
  fn ensure_cookie_db_path(
    profile: &BrowserProfile,
    profiles_dir: &Path,
  ) -> Result<PathBuf, String> {
    let profile_data_path = profile.get_profile_data_path(profiles_dir);

    match profile.browser.as_str() {
      "wayfern" => {
        let path = Self::wayfern_cookie_path(&profile_data_path);
        if !path.exists() {
          Self::create_empty_chrome_cookies_db(&path)?;
        }
        Ok(path)
      }
      _ => Err(format!(
        "Unsupported browser type for cookie operations: {}",
        profile.browser
      )),
    }
  }

  /// Create an empty Chromium-format Cookies SQLite database at `path`.
  ///
  /// Schema matches what recent Chromium versions write on first launch:
  /// the `cookies` table, the `meta` table with version info, and the
  /// `host_key/top_frame_site_key/name/path` unique index. Chromium's cookie
  /// store migration code will upgrade this forward when Wayfern first
  /// launches the profile.
  fn create_empty_chrome_cookies_db(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create cookie directory: {e}"))?;
    }
    let conn =
      Connection::open(path).map_err(|e| format!("Failed to create cookie database: {e}"))?;
    conn
      .execute_batch(
        "CREATE TABLE cookies(
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
        );
        CREATE UNIQUE INDEX cookies_unique_index
          ON cookies(host_key, top_frame_site_key, name, path);
        CREATE TABLE meta(
          key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY,
          value LONGVARCHAR
        );
        INSERT INTO meta VALUES('version', '23');
        INSERT INTO meta VALUES('last_compatible_version', '23');",
      )
      .map_err(|e| format!("Failed to initialize cookie database schema: {e}"))?;
    Ok(())
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

        // Use plaintext value if available, otherwise decrypt encrypted_value.
        // Decryption needs the host_key (domain) to verify and strip the
        // SHA-256 integrity prefix Chromium prepends before encryption.
        let value = if !plaintext_value.is_empty() {
          plaintext_value
        } else if !encrypted_value.is_empty() {
          encryption_key
            .and_then(|key| chrome_decrypt::decrypt(&encrypted_value, &domain, key))
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

  /// Write cookies to a Chrome/Wayfern profile.
  ///
  /// Always writes values as plaintext in the `value` column with an empty
  /// `encrypted_value`. Chromium reads plaintext on a per-row basis when
  /// `encrypted_value` is empty, so this mixes cleanly with any pre-existing
  /// encrypted cookies in the database. We avoid encrypting on write because
  /// the os_crypt key derivation between Wayfern's runtime and an external
  /// writer is not guaranteed to match, and a ciphertext Chromium can't
  /// decrypt silently produces an empty cookie value at runtime.
  ///
  /// The whole write runs in one transaction, so a row that fails cannot leave
  /// half a paste applied — the half that would matter is the half holding the
  /// login.
  fn write_chrome_cookies(
    db_path: &Path,
    cookies: &[UnifiedCookie],
    mode: CookieWriteMode,
  ) -> Result<CookieWriteCounts, String> {
    let mut conn =
      Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;
    let tx = conn
      .transaction()
      .map_err(|e| format!("Failed to start cookie transaction: {e}"))?;

    let mut counts = CookieWriteCounts::default();

    if mode == CookieWriteMode::ReplaceMatchingSites {
      for host in Self::replace_host_keys(cookies) {
        counts.deleted += tx
          .execute("DELETE FROM cookies WHERE host_key = ?1", params![host])
          .map_err(|e| format!("Failed to clear existing cookies: {e}"))?;
      }
    }

    let now = Self::now_secs();
    // Session cookies get 30 days of persistence so they survive restart.
    // routinely exported as a session cookie; writing it as memory-only
    // (is_persistent = 0) makes Chromium drop it on the next flush, so the
    // imported account silently signs out on relaunch. Persisting it with a real
    // expiry keeps it alive (expires_utc=0 would otherwise mean 1601-01-01).
    let session_cookie_expiry = now + 30 * 86400;

    for cookie in cookies {
      let expires = if cookie.expires > 0 {
        cookie.expires
      } else {
        session_cookie_expiry
      };
      let has_expires = 1;
      let is_persistent = 1;
      // HTTPS cookies use 443, HTTP uses 80. source_port participates in
      // Chromium's scheme-bound cookie enforcement.
      let source_port: i32 = if cookie.is_secure { 443 } else { 80 };
      let source_scheme: i32 = if cookie.is_secure { 2 } else { 1 };

      // Four columns, matching the store's own unique index. A three-column
      // probe collapses every top-frame partition of one host/name/path into a
      // single arbitrary row: the rest keep their old values and the write
      // still reports one replacement.
      let existing: Option<i64> = tx
        .query_row(
          "SELECT rowid FROM cookies
           WHERE host_key = ?1 AND top_frame_site_key = '' AND name = ?2 AND path = ?3",
          params![&cookie.domain, &cookie.name, &cookie.path],
          |row| row.get(0),
        )
        .ok();

      if let Some(rowid) = existing {
        // creation_utc is deliberately not written: Chromium evicts by it, so
        // refreshing a cookie must not make it look brand new.
        tx.execute(
          "UPDATE cookies SET value = ?1, encrypted_value = x'', expires_utc = ?2, is_secure = ?3,
                     is_httponly = ?4, samesite = ?5, last_access_utc = ?6, last_update_utc = ?7,
                     has_expires = ?8, is_persistent = ?9, source_scheme = ?10, source_port = ?11
                     WHERE rowid = ?12",
          params![
            &cookie.value,
            Self::unix_to_chrome_time(expires),
            cookie.is_secure as i32,
            cookie.is_http_only as i32,
            cookie.same_site,
            Self::unix_to_chrome_time(cookie.last_accessed),
            Self::unix_to_chrome_time(now),
            has_expires,
            is_persistent,
            source_scheme,
            source_port,
            rowid,
          ],
        )
        .map_err(|e| format!("Failed to update cookie: {e}"))?;
        counts.overwritten += 1;
      } else {
        tx.execute(
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
              Self::unix_to_chrome_time(expires),
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
        counts.added += 1;
      }
    }

    tx.commit()
      .map_err(|e| format!("Failed to commit cookies: {e}"))?;

    Ok(counts)
  }

  /// Which `host_key` values a replace-mode write clears out first.
  ///
  /// `.example.com` and `example.com` are one site to the person pasting and
  /// two distinct host keys to Chromium, so replacing one without the other
  /// leaves the stale half of the login behind. Subdomains stay untouched:
  /// nothing in the paste says anything about them.
  fn replace_host_keys(cookies: &[UnifiedCookie]) -> Vec<String> {
    let mut hosts = std::collections::BTreeSet::new();
    for cookie in cookies {
      let bare = cookie.domain.trim_start_matches('.');
      if bare.is_empty() {
        continue;
      }
      hosts.insert(bare.to_string());
      hosts.insert(format!(".{bare}"));
    }
    hosts.into_iter().collect()
  }

  /// How many stored rows a replace-mode write of `cookies` would delete.
  ///
  /// Reads the snapshot view so a running browser's lock does not turn the
  /// preview into an error.
  fn count_replaceable_rows(db_path: &Path, cookies: &[UnifiedCookie]) -> Result<usize, String> {
    let conn = Self::open_cookie_db_readonly(db_path)?;
    let mut total = 0usize;
    for host in Self::replace_host_keys(cookies) {
      let count: i64 = conn
        .query_row(
          "SELECT COUNT(*) FROM cookies WHERE host_key = ?1",
          params![host],
          |row| row.get(0),
        )
        .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?;
      total += count as usize;
    }
    Ok(total)
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

  /// Open the cookie SQLite database read-only without acquiring any lock.
  ///
  /// `immutable=1` tells SQLite the file will not change during the read,
  /// which causes it to skip all locking. That lets us read metadata even
  /// while the browser holds an exclusive lock on the cookies database —
  /// the trade-off is that we may see a slightly stale snapshot, which is
  /// acceptable for the badge/preview use cases this powers.
  fn open_cookie_db_readonly(db_path: &Path) -> Result<Connection, String> {
    let path_str = db_path.to_string_lossy();
    if path_str.contains('?') || path_str.contains('#') {
      return Err(
        serde_json::json!({
          "code": "COOKIE_DB_UNAVAILABLE",
          "params": { "detail": "profile path contains a reserved URI character" }
        })
        .to_string(),
      );
    }
    let uri = format!("file:{path_str}?mode=ro&immutable=1");
    Connection::open_with_flags(
      &uri,
      OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| {
      let code = if e.to_string().to_lowercase().contains("locked") {
        "COOKIE_DB_LOCKED"
      } else {
        "COOKIE_DB_UNAVAILABLE"
      };
      serde_json::json!({
        "code": code,
        "params": { "detail": e.to_string() }
      })
      .to_string()
    })
  }

  /// Public API: read lightweight stats (total count + top 5 domains) for a
  /// profile's cookie store. Reads from a snapshot view of the SQLite file
  /// without holding a lock, so this works while the browser is running.
  pub fn read_stats(profile_id: &str) -> Result<CookieStats, String> {
    let profile_manager = ProfileManager::instance();
    let profiles_dir = profile_manager.get_profiles_dir();
    let profiles = profile_manager.list_profiles().map_err(|e| {
      serde_json::json!({
        "code": "COOKIE_DB_UNAVAILABLE",
        "params": { "detail": e.to_string() }
      })
      .to_string()
    })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| serde_json::json!({ "code": "PROFILE_NOT_FOUND" }).to_string())?;

    let db_path = Self::get_cookie_db_path(profile, &profiles_dir).map_err(|e| {
      serde_json::json!({
        "code": "COOKIE_DB_UNAVAILABLE",
        "params": { "detail": e }
      })
      .to_string()
    })?;

    let conn = Self::open_cookie_db_readonly(&db_path)?;

    let (count_sql, domain_sql) = match profile.browser.as_str() {
      "wayfern" => (
        "SELECT COUNT(*) FROM cookies",
        "SELECT host_key, COUNT(*) FROM cookies GROUP BY host_key ORDER BY COUNT(*) DESC, host_key ASC",
      ),
      _ => {
        return Err(
          serde_json::json!({
            "code": "COOKIE_DB_UNAVAILABLE",
            "params": { "detail": format!("unsupported browser: {}", profile.browser) }
          })
          .to_string(),
        )
      }
    };

    let total_count: usize = conn
      .query_row(count_sql, [], |row| row.get::<_, i64>(0))
      .map_err(|e| {
        serde_json::json!({
          "code": "COOKIE_DB_UNAVAILABLE",
          "params": { "detail": e.to_string() }
        })
        .to_string()
      })? as usize;

    let mut stmt = conn.prepare(domain_sql).map_err(|e| {
      serde_json::json!({
        "code": "COOKIE_DB_UNAVAILABLE",
        "params": { "detail": e.to_string() }
      })
      .to_string()
    })?;
    let domains: Vec<DomainCount> = stmt
      .query_map([], |row| {
        Ok(DomainCount {
          domain: row.get::<_, String>(0)?,
          count: row.get::<_, i64>(1)? as usize,
        })
      })
      .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
      .map_err(|e| {
        serde_json::json!({
          "code": "COOKIE_DB_UNAVAILABLE",
          "params": { "detail": e.to_string() }
        })
        .to_string()
      })?;

    Ok(CookieStats {
      profile_id: profile_id.to_string(),
      browser_type: profile.browser.clone(),
      total_count,
      domains,
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

      // Target may be a brand-new profile that has never been launched, so
      // its Cookies DB file doesn't exist yet. Create an empty one on demand.
      let target_db_path = match Self::ensure_cookie_db_path(target, &profiles_dir) {
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
        "wayfern" => {
          Self::write_chrome_cookies(&target_db_path, &cookies_to_copy, CookieWriteMode::Merge)
        }
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
        Ok(counts) => {
          results.push(CookieCopyResult {
            target_profile_id: target_id.clone(),
            cookies_copied: counts.added,
            cookies_replaced: counts.overwritten,
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

  /// Format cookies as Netscape TXT
  ///
  /// http-only cookies carry the `#HttpOnly_` host prefix curl and the
  /// cookies.txt extensions use. The plain format has no field for the flag, so
  /// without the prefix an export of a login cookie re-imports as a cookie any
  /// page script can read.
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
      let host = if cookie.is_http_only {
        format!("#HttpOnly_{}", cookie.domain)
      } else {
        cookie.domain.clone()
      };
      lines.push(format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        host, flag, cookie.path, secure, cookie.expires, cookie.name, cookie.value
      ));
    }
    lines.join("\n")
  }

  /// Format cookies as JSON
  pub fn format_json_cookies(cookies: &[UnifiedCookie]) -> String {
    let arr: Vec<Value> = cookies
      .iter()
      .map(|c| {
        // -1 is Chromium's "unspecified", which is what every cookie that never
        // carried a SameSite attribute stores. Exporting it as
        // "no_restriction" turned it into an explicit SameSite=None on the way
        // back in, and Chromium refuses to send those unless they are also
        // Secure — so a re-imported login simply stopped being sent.
        let same_site_str = match c.same_site {
          0 => "no_restriction",
          1 => "lax",
          2 => "strict",
          _ => "unspecified",
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

  /// The profile a cookie write targets, or the code saying it does not exist.
  fn paste_target(profile_id: &str) -> Result<BrowserProfile, String> {
    ProfileManager::instance()
      .list_profiles()
      .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?
      .into_iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| crate::backend_error("PROFILE_NOT_FOUND"))
  }

  /// Why writing cookies into this profile would be refused right now, as the
  /// `{"code":…}` string the frontend translates. `None` means go ahead.
  async fn paste_blocker(app_handle: &AppHandle, profile: &BrowserProfile) -> Option<String> {
    let is_running = ProfileManager::instance()
      .check_browser_status(app_handle.clone(), profile)
      .await
      .unwrap_or(false);
    if is_running {
      return Some(crate::backend_error("COOKIE_IMPORT_BROWSER_RUNNING"));
    }
    // The profile directory on disk is ciphertext while locked; a plaintext
    // SQLite write into it is not a cookie, it is corruption.
    if profile.password_protected {
      return Some(crate::backend_error("COOKIE_IMPORT_PROFILE_PROTECTED"));
    }
    // A remote session owns this profile until its work has been pulled back.
    // Writing locally makes every local mtime newer, so the next sync uploads
    // the pre-session copy and deletes the session's cookies. See
    // `remote_handoff`.
    if crate::remote_handoff::state_for(&profile.id.to_string()).is_some() {
      return Some(crate::backend_error("COOKIE_IMPORT_REMOTE_SESSION"));
    }
    None
  }

  /// Whether a successful import into this profile would be wiped on browser
  /// exit. Mirrors the condition `browser_runner` actually applies, which lets
  /// ephemeral and password-protected profiles take their own paths first.
  fn clears_cookies_on_close(profile: &BrowserProfile) -> bool {
    profile.clear_on_close && !profile.ephemeral && !profile.password_protected
  }

  /// Public API: describe a pasted blob without writing anything.
  pub async fn analyze_paste(
    app_handle: &AppHandle,
    profile_id: &str,
    content: &str,
    site: Option<&str>,
  ) -> Result<CookiePasteAnalysis, String> {
    let profile = Self::paste_target(profile_id)?;
    let parsed = crate::cookie_paste::parse_paste(content, site);
    let blocked_by = Self::paste_blocker(app_handle, &profile).await;

    // Counted over every accepted cookie, so this is the number of rows the
    // sites in the paste currently hold — the ceiling on what replace mode
    // removes, whatever the caller later decides about expired cookies.
    let replace_delete_count = if parsed.cookies.is_empty() {
      Some(0)
    } else {
      Self::get_cookie_db_path(&profile, &ProfileManager::instance().get_profiles_dir())
        .ok()
        .and_then(|db_path| Self::count_replaceable_rows(&db_path, &parsed.cookies).ok())
    };

    Ok(CookiePasteAnalysis {
      format: parsed.format,
      cookies: parsed
        .cookies
        .iter()
        .map(PastedCookiePreview::from)
        .collect(),
      issues: parsed.issues,
      site_required: parsed.site_required,
      expired_count: parsed.expired_count,
      replace_delete_count,
      clears_on_close: Self::clears_cookies_on_close(&profile),
      blocked_by,
    })
  }

  /// Public API: write a pasted blob into a profile's cookie store.
  pub async fn import_paste(
    app_handle: &AppHandle,
    profile_id: &str,
    content: &str,
    site: Option<&str>,
    mode: CookieWriteMode,
    include_expired: bool,
  ) -> Result<CookiePasteImportResult, String> {
    let profile = Self::paste_target(profile_id)?;
    if let Some(blocker) = Self::paste_blocker(app_handle, &profile).await {
      return Err(blocker);
    }

    let parsed = crate::cookie_paste::parse_paste(content, site);
    if parsed.cookies.is_empty() {
      return Err(crate::backend_error("COOKIE_IMPORT_NO_COOKIES"));
    }

    let now = Self::now_secs();
    let total = parsed.cookies.len();
    let to_write: Vec<UnifiedCookie> = if include_expired {
      parsed.cookies
    } else {
      parsed
        .cookies
        .into_iter()
        .filter(|c| c.expires == 0 || c.expires > now)
        .collect()
    };
    let skipped = total - to_write.len();

    let counts = Self::write_paste(&profile, &to_write, mode).await?;

    Ok(CookiePasteImportResult {
      added: counts.added,
      overwritten: counts.overwritten,
      deleted: counts.deleted,
      skipped,
      issues: parsed.issues,
    })
  }

  /// The SQLite half of a paste write, off the async runtime.
  ///
  /// `check_browser_status` is async and every caller must await it before
  /// getting here; only this synchronous tail belongs on a blocking thread.
  async fn write_paste(
    profile: &BrowserProfile,
    cookies: &[UnifiedCookie],
    mode: CookieWriteMode,
  ) -> Result<CookieWriteCounts, String> {
    if profile.browser.as_str() != "wayfern" {
      return Err(crate::backend_error_with_detail(
        "COOKIE_DB_UNAVAILABLE",
        format!("unsupported browser: {}", profile.browser),
      ));
    }

    // A profile that has never launched has no Cookies file yet.
    let db_path =
      Self::ensure_cookie_db_path(profile, &ProfileManager::instance().get_profiles_dir())
        .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?;
    let owned: Vec<UnifiedCookie> = cookies.to_vec();

    tokio::task::spawn_blocking(move || Self::write_chrome_cookies(&db_path, &owned, mode))
      .await
      .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))?
      .map_err(|e| crate::backend_error_with_detail("COOKIE_DB_UNAVAILABLE", e))
  }

  fn now_secs() -> i64 {
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|d| d.as_secs() as i64)
      .unwrap_or(0)
  }

  /// Public API: Import cookies from a file's contents, with auto-detection.
  ///
  /// The paste parser backs this too, so a `cookies.txt` dragged into the
  /// dialog and one pasted into the textarea are read by the same code.
  pub async fn import_cookies(
    app_handle: &AppHandle,
    profile_id: &str,
    content: &str,
  ) -> Result<CookieImportResult, String> {
    let result = Self::import_paste(
      app_handle,
      profile_id,
      content,
      None,
      CookieWriteMode::Merge,
      true,
    )
    .await?;

    Ok(CookieImportResult {
      cookies_imported: result.added,
      cookies_replaced: result.overwritten,
      // The file dialog has no room for a per-line report, so it gets the codes
      // of the things that went wrong and nothing else.
      errors: result
        .issues
        .into_iter()
        .filter(|i| !matches!(i.severity, crate::cookie_paste::IssueSeverity::Info))
        .map(|i| match i.source {
          Some(source) => format!("{source}: {}", i.code),
          None => i.code,
        })
        .collect(),
    })
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

  #[cfg(target_os = "macos")]
  const SYNTHETIC_COOKIE_HOST: &str = ".example.test";
  #[cfg(target_os = "macos")]
  const SYNTHETIC_COOKIE_VALUE: &str = "synthetic-cookie-value";
  #[cfg(target_os = "macos")]
  const SYNTHETIC_OS_CRYPT_PASSWORD: &[u8] = b"donut-synthetic-cookie-key";
  #[cfg(target_os = "macos")]
  const SYNTHETIC_ENCRYPTED_COOKIE_HEX: &str = "763130d83b9fd3e6d1b1c793769f55251f5e9d1193be72c0c08ea32e2cf068a85d9d0b97d8b2e6deca93a2b3c290e98e1a851f83d5566f9aa9314befe56dc6bdbd423d";

  #[cfg(target_os = "macos")]
  fn synthetic_encrypted_cookie() -> Vec<u8> {
    (0..SYNTHETIC_ENCRYPTED_COOKIE_HEX.len())
      .step_by(2)
      .map(|i| u8::from_str_radix(&SYNTHETIC_ENCRYPTED_COOKIE_HEX[i..i + 2], 16).unwrap())
      .collect()
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
  fn test_format_netscape_cookies_marks_http_only() {
    let cookies = vec![UnifiedCookie {
      name: "sid".to_string(),
      value: "abc".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1700000000,
      is_secure: true,
      is_http_only: true,
      same_site: -1,
      creation_time: 0,
      last_accessed: 0,
    }];
    let output = CookieManager::format_netscape_cookies(&cookies);
    assert!(output.contains("#HttpOnly_.example.com\tTRUE\t/\tTRUE\t1700000000\tsid\tabc"));

    let parsed = crate::cookie_paste::parse_paste(&output, None);
    assert_eq!(parsed.cookies.len(), 1);
    assert_eq!(parsed.cookies[0].domain, ".example.com");
    assert!(parsed.cookies[0].is_http_only);
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
    let parsed = crate::cookie_paste::parse_paste(&formatted, None).cookies;
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
    let parsed = crate::cookie_paste::parse_paste(&formatted, None).cookies;
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "tok");
    assert_eq!(parsed[0].domain, ".site.org");
    assert_eq!(parsed[0].path, "/app");
    assert!(!parsed[0].is_secure);
    assert!(parsed[0].is_http_only);
    assert_eq!(parsed[0].same_site, 2);
    assert_eq!(parsed[0].expires, 1700000000);
  }

  /// Every Netscape import now lands as -1 (unspecified), which used to export
  /// as "no_restriction" and come back as an explicit SameSite=None — a cookie
  /// Chromium then refuses to send unless it is also Secure.
  #[test]
  fn test_json_roundtrip_preserves_unspecified_same_site() {
    let cookies = vec![UnifiedCookie {
      name: "sid".to_string(),
      value: "v".to_string(),
      domain: ".site.org".to_string(),
      path: "/".to_string(),
      expires: 1900000000,
      is_secure: false,
      is_http_only: false,
      same_site: -1,
      creation_time: 0,
      last_accessed: 0,
    }];
    let formatted = CookieManager::format_json_cookies(&cookies);
    assert_eq!(
      serde_json::from_str::<Vec<Value>>(&formatted).unwrap()[0]["sameSite"],
      "unspecified"
    );

    let parsed = crate::cookie_paste::parse_paste(&formatted, None).cookies;
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].same_site, -1);
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

    let counts =
      CookieManager::write_chrome_cookies(&tmp, &cookies, CookieWriteMode::Merge).unwrap();
    assert_eq!(counts.added, 1);
    assert_eq!(counts.overwritten, 0);

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
  fn test_write_chrome_cookies_session_cookie_persisted() {
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

    CookieManager::write_chrome_cookies(&tmp, &cookies, CookieWriteMode::Merge).unwrap();

    let conn = Connection::open(&tmp).unwrap();
    let (has_expires, is_persistent, expires_utc, source_scheme, source_port): (
      i32,
      i32,
      i64,
      i32,
      i32,
    ) = conn
      .query_row(
        "SELECT has_expires, is_persistent, expires_utc, source_scheme, source_port
         FROM cookies WHERE name = ?1",
        params!["session"],
        |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
          ))
        },
      )
      .unwrap();

    // Imported session cookies are promoted to persistent with a far-future
    // expiry so an imported login survives relaunch.
    assert_eq!(has_expires, 1);
    assert_eq!(is_persistent, 1);
    // Must be a real future expiry, not 0 (which Chromium reads as 1601-01-01).
    assert!(expires_utc > 0);
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

    let counts = CookieManager::write_chrome_cookies(
      &tmp,
      std::slice::from_ref(&cookie),
      CookieWriteMode::Merge,
    )
    .unwrap();
    assert_eq!(counts.added, 1);

    let mut updated = cookie.clone();
    updated.value = "v2".to_string();
    let counts = CookieManager::write_chrome_cookies(
      &tmp,
      std::slice::from_ref(&updated),
      CookieWriteMode::Merge,
    )
    .unwrap();
    assert_eq!(counts.added, 0);
    assert_eq!(counts.overwritten, 1);

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

  fn cookie(domain: &str, name: &str, value: &str) -> UnifiedCookie {
    UnifiedCookie {
      name: name.to_string(),
      value: value.to_string(),
      domain: domain.to_string(),
      path: "/".to_string(),
      expires: 1900000000,
      is_secure: true,
      is_http_only: false,
      same_site: -1,
      creation_time: 1700000000,
      last_accessed: 1700000000,
    }
  }

  fn host_keys(db_path: &Path) -> Vec<String> {
    let conn = Connection::open(db_path).unwrap();
    let mut stmt = conn
      .prepare("SELECT host_key || '|' || name FROM cookies ORDER BY host_key, name")
      .unwrap();
    let rows: Vec<String> = stmt
      .query_map([], |row| row.get(0))
      .unwrap()
      .collect::<Result<_, _>>()
      .unwrap();
    rows
  }

  /// Replace mode clears both spellings of the pasted site and nothing else:
  /// not a subdomain, not another site.
  #[test]
  fn test_write_chrome_cookies_replace_matching_sites_scope() {
    let tmp = std::env::temp_dir().join(format!("donut_cookie_test_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&tmp);

    let seeded = vec![
      cookie(".example.com", "dotted", "old"),
      cookie("example.com", "undotted", "old"),
      cookie("app.example.com", "subdomain", "keep"),
      cookie(".other.com", "elsewhere", "keep"),
    ];
    CookieManager::write_chrome_cookies(&tmp, &seeded, CookieWriteMode::Merge).unwrap();

    // A paste that names only the undotted form must still clear the dotted one.
    let counts = CookieManager::write_chrome_cookies(
      &tmp,
      &[cookie("example.com", "fresh", "new")],
      CookieWriteMode::ReplaceMatchingSites,
    )
    .unwrap();
    assert_eq!(counts.deleted, 2);
    assert_eq!(counts.added, 1);
    assert_eq!(counts.overwritten, 0);

    assert_eq!(
      host_keys(&tmp),
      vec![
        ".other.com|elsewhere".to_string(),
        "app.example.com|subdomain".to_string(),
        "example.com|fresh".to_string(),
      ]
    );

    let _ = std::fs::remove_file(&tmp);
  }

  #[test]
  fn test_write_chrome_cookies_merge_deletes_nothing() {
    let tmp = std::env::temp_dir().join(format!("donut_cookie_test_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&tmp);

    CookieManager::write_chrome_cookies(
      &tmp,
      &[cookie(".example.com", "existing", "old")],
      CookieWriteMode::Merge,
    )
    .unwrap();
    let counts = CookieManager::write_chrome_cookies(
      &tmp,
      &[cookie("example.com", "fresh", "new")],
      CookieWriteMode::Merge,
    )
    .unwrap();

    assert_eq!(counts.deleted, 0);
    assert_eq!(counts.added, 1);
    assert_eq!(host_keys(&tmp).len(), 2);

    let _ = std::fs::remove_file(&tmp);
  }

  /// The store's unique index is four columns wide. A three-column probe made
  /// one partitioned cookie stand in for all of them, so the others silently
  /// kept their stale values.
  #[test]
  fn test_write_chrome_cookies_probe_is_partition_aware() {
    let tmp = std::env::temp_dir().join(format!("donut_cookie_test_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&tmp);

    let conn = Connection::open(&tmp).unwrap();
    conn
      .execute(
        "INSERT INTO cookies
           (creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path,
            expires_utc, is_secure, is_httponly, last_access_utc)
         VALUES (1, '.example.com', 'https://partner.example', 'sid', 'partitioned', x'', '/',
                 0, 1, 0, 0)",
        [],
      )
      .unwrap();
    drop(conn);

    let counts = CookieManager::write_chrome_cookies(
      &tmp,
      &[cookie(".example.com", "sid", "unpartitioned")],
      CookieWriteMode::Merge,
    )
    .unwrap();
    assert_eq!(counts.added, 1, "the partitioned row is a different cookie");
    assert_eq!(counts.overwritten, 0);

    let conn = Connection::open(&tmp).unwrap();
    let partitioned: String = conn
      .query_row(
        "SELECT value FROM cookies WHERE top_frame_site_key = 'https://partner.example'",
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(partitioned, "partitioned");

    let _ = std::fs::remove_file(&tmp);
  }

  /// Chromium evicts by creation_utc, so refreshing a cookie must not reset it.
  #[test]
  fn test_write_chrome_cookies_preserves_creation_time() {
    let tmp = std::env::temp_dir().join(format!("donut_cookie_test_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&tmp);

    CookieManager::write_chrome_cookies(
      &tmp,
      &[cookie(".example.com", "sid", "v1")],
      CookieWriteMode::Merge,
    )
    .unwrap();
    let original = CookieManager::unix_to_chrome_time(1700000000);

    let mut refreshed = cookie(".example.com", "sid", "v2");
    refreshed.creation_time = 1800000000;
    CookieManager::write_chrome_cookies(&tmp, &[refreshed], CookieWriteMode::Merge).unwrap();

    let conn = Connection::open(&tmp).unwrap();
    let (creation, value): (i64, String) = conn
      .query_row(
        "SELECT creation_utc, value FROM cookies WHERE name = 'sid'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .unwrap();
    assert_eq!(creation, original);
    assert_eq!(value, "v2");

    let _ = std::fs::remove_file(&tmp);
  }

  /// A write that fails partway must leave the store exactly as it was, and in
  /// replace mode that includes the deletes it had already issued.
  #[test]
  fn test_write_chrome_cookies_rolls_back_on_failure() {
    let tmp = std::env::temp_dir().join(format!("donut_cookie_test_{}.db", uuid::Uuid::new_v4()));
    create_chrome_cookies_db(&tmp);
    CookieManager::write_chrome_cookies(
      &tmp,
      &[cookie(".example.com", "existing", "keep")],
      CookieWriteMode::Merge,
    )
    .unwrap();

    // NOT NULL on `path` is what makes the second insert fail; the first one
    // and the replace-mode deletes have to unwind with it.
    let mut broken = cookie(".example.com", "second", "v");
    broken.path = String::new();
    let conn = Connection::open(&tmp).unwrap();
    conn
      .execute(
        "CREATE UNIQUE INDEX no_empty_path ON cookies(path) WHERE path = ''",
        [],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO cookies
           (creation_utc, host_key, top_frame_site_key, name, value, encrypted_value, path,
            expires_utc, is_secure, is_httponly, last_access_utc)
         VALUES (1, '.blocker.com', '', 'blocker', 'v', x'', '', 0, 0, 0, 0)",
        [],
      )
      .unwrap();
    drop(conn);

    let result = CookieManager::write_chrome_cookies(
      &tmp,
      &[cookie(".example.com", "first", "v"), broken],
      CookieWriteMode::ReplaceMatchingSites,
    );
    assert!(result.is_err());

    assert_eq!(
      host_keys(&tmp),
      vec![
        ".blocker.com|blocker".to_string(),
        ".example.com|existing".to_string(),
      ],
      "neither the delete nor the first insert may survive a failed write"
    );

    let _ = std::fs::remove_file(&tmp);
  }

  #[test]
  #[cfg(target_os = "macos")]
  fn test_decrypt_v10_cookie_with_synthetic_vector() {
    let profile_dir =
      std::env::temp_dir().join(format!("donut_decrypt_vector_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::write(
      profile_dir.join("os_crypt_key"),
      SYNTHETIC_OS_CRYPT_PASSWORD,
    )
    .unwrap();

    let key = chrome_decrypt::get_encryption_key(&profile_dir)
      .expect("should derive key from os_crypt_key file");

    let decrypted =
      chrome_decrypt::decrypt(&synthetic_encrypted_cookie(), SYNTHETIC_COOKIE_HOST, &key)
        .expect("decryption must succeed with correct key and host");
    assert_eq!(decrypted, SYNTHETIC_COOKIE_VALUE);

    let _ = std::fs::remove_dir_all(&profile_dir);
  }

  #[test]
  #[cfg(target_os = "macos")]
  fn test_decrypt_with_wrong_host_returns_none_or_raw() {
    let profile_dir =
      std::env::temp_dir().join(format!("donut_decrypt_wrong_host_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&profile_dir).unwrap();
    std::fs::write(
      profile_dir.join("os_crypt_key"),
      SYNTHETIC_OS_CRYPT_PASSWORD,
    )
    .unwrap();

    let key = chrome_decrypt::get_encryption_key(&profile_dir).unwrap();
    let result =
      chrome_decrypt::decrypt(&synthetic_encrypted_cookie(), ".wrong.example.test", &key);
    assert!(
      result.as_deref() != Some(SYNTHETIC_COOKIE_VALUE),
      "decrypt must not return the cookie value when host_key is wrong"
    );

    let _ = std::fs::remove_dir_all(&profile_dir);
  }

  /// Regression: a brand-new Wayfern profile has no `Default/Cookies` file
  /// yet (Chromium only writes it on first launch). Copying/importing into
  /// such a profile must create the file on demand.
  #[test]
  fn test_create_empty_chrome_cookies_db_then_write() {
    let dir = std::env::temp_dir().join(format!("donut_empty_chrome_{}", uuid::Uuid::new_v4()));
    let db_path = dir.join("Default").join("Cookies");
    assert!(!db_path.exists());

    CookieManager::create_empty_chrome_cookies_db(&db_path).unwrap();
    assert!(db_path.exists());

    // Round-trip: write a cookie into the freshly created DB, read it back.
    let cookies = vec![UnifiedCookie {
      name: "auth".to_string(),
      value: "token123".to_string(),
      domain: ".example.com".to_string(),
      path: "/".to_string(),
      expires: 1900000000,
      is_secure: true,
      is_http_only: true,
      same_site: 0,
      creation_time: 1700000000,
      last_accessed: 1700000000,
    }];
    let counts =
      CookieManager::write_chrome_cookies(&db_path, &cookies, CookieWriteMode::Merge).unwrap();
    assert_eq!(counts.added, 1);
    assert_eq!(counts.overwritten, 0);

    let read = CookieManager::read_chrome_cookies(&db_path, None).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].value, "token123");

    // Schema sanity: `meta` table with version row exists so Chromium's
    // cookie store migration code can upgrade this on first launch.
    let conn = Connection::open(&db_path).unwrap();
    let version: String = conn
      .query_row("SELECT value FROM meta WHERE key = 'version'", [], |row| {
        row.get(0)
      })
      .unwrap();
    assert!(!version.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
  }
}

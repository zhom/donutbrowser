//! Recovering the *source* browser's os_crypt key.
//!
//! Every Chromium-family browser seals cookies, passwords and payment data with
//! a key held outside the profile: the macOS Keychain, a DPAPI blob in
//! `Local State`, or the Freedesktop secret service. Import has to open that
//! lock before it can re-seal anything with Wayfern's portable key
//! ([`super::os_crypt::TargetKey`]).
//!
//! Failure here is never fatal. A declined Keychain prompt or a locked keyring
//! degrades to "everything except the secrets came across", recorded as a
//! warning, because a partial profile is worth far more than a failed import.

#[cfg(target_os = "windows")]
use super::os_crypt::CryptoKey;
use super::os_crypt::SourceKeyring;
#[cfg(target_os = "macos")]
use super::os_crypt::MAC_ITERATIONS;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::os_crypt::{derive_key, CryptoKey};
#[cfg(target_os = "linux")]
use super::os_crypt::{POSIX_FALLBACK_PASSWORD, POSIX_ITERATIONS};
use super::report::warning;
use std::path::Path;

/// Keychain / secret-service identities to try for a source family, most
/// specific first.
///
/// Trying several is safe and costs nothing: a lookup for a service that does
/// not exist fails without prompting, so at most one dialog appears — the one
/// for the item that is actually there. That is what lets a single `chromium`
/// family key cover both Google Chrome and vanilla Chromium, which share a
/// detection entry but not a Keychain item.
// Consulted by the Keychain and secret-service lookups. Windows resolves the
// key through DPAPI against the profile's own Local State, so it never needs
// to guess a brand.
#[allow(dead_code)]
fn brand_candidates(family: &str, source_path: &Path) -> Vec<&'static str> {
  let path = source_path.to_string_lossy();
  let mut brands: Vec<&'static str> = match family {
    "chrome-beta" => vec!["Chrome Beta", "Chrome"],
    "chrome-dev" => vec!["Chrome Dev", "Chrome"],
    "chrome-canary" => vec!["Chrome Canary", "Chrome"],
    "brave" => vec!["Brave", "Brave Browser"],
    "brave-beta" => vec!["Brave Beta", "Brave Browser", "Brave"],
    "brave-nightly" => vec!["Brave Nightly", "Brave Browser", "Brave"],
    "edge" => vec!["Microsoft Edge", "Chromium"],
    "edge-beta" => vec!["Microsoft Edge Beta", "Microsoft Edge"],
    "edge-dev" => vec!["Microsoft Edge Dev", "Microsoft Edge"],
    "vivaldi" => vec!["Vivaldi", "Chromium"],
    "opera" => vec!["Opera", "Chromium"],
    "opera-gx" => vec!["Opera GX", "Opera", "Chromium"],
    "arc" => vec!["Arc", "Chromium"],
    "yandex" => vec!["Yandex", "Yandex Browser", "Chromium"],
    // "chromium" covers both Google Chrome and upstream Chromium; the install
    // path is the only thing that tells them apart.
    _ => vec!["Chrome", "Chromium"],
  };

  if (family.is_empty() || family == "chromium")
    && path.contains("Chromium")
    && !path.contains("Google")
  {
    brands = vec!["Chromium", "Chrome"];
  }

  brands
}

/// Recover whatever key material the source browser used.
///
/// `source_user_data_dir` is the directory holding `Local State` (the parent of
/// the profile directory), which is where Windows keeps its wrapped key. It is
/// `None` when the user pointed at a bare profile folder with no parent we can
/// trust.
pub fn recover_source_keys(
  family: &str,
  source_path: &Path,
  source_user_data_dir: Option<&Path>,
  report: &mut super::report::ProfileImportReport,
) -> SourceKeyring {
  let mut keyring = SourceKeyring::default();

  #[cfg(target_os = "macos")]
  {
    let _ = source_user_data_dir;
    for brand in brand_candidates(family, source_path) {
      match macos_keychain_password(brand) {
        Ok(Some(password)) => {
          keyring.v10 = Some(CryptoKey::Aes128Cbc(derive_key(&password, MAC_ITERATIONS)));
          log::info!("Recovered os_crypt password for '{brand} Safe Storage'");
          break;
        }
        Ok(None) => continue,
        Err(e) => {
          log::warn!("Keychain lookup for '{brand} Safe Storage' failed: {e}");
          break;
        }
      }
    }
  }

  #[cfg(target_os = "windows")]
  {
    let _ = source_path;
    if let Some(dir) = source_user_data_dir {
      match windows_local_state_key(dir) {
        Ok(Some(key)) => keyring.v10 = Some(CryptoKey::Aes256Gcm(key)),
        Ok(None) => {}
        Err(e) => log::warn!("DPAPI key recovery failed: {e}"),
      }
      if windows_has_app_bound_key(dir) {
        // Recorded up front: the cookie store will be full of `v20` records
        // and the user deserves to know why before they see the count.
        report.warn(warning::APP_BOUND_ENCRYPTED);
      }
    }
  }

  #[cfg(target_os = "linux")]
  {
    let _ = source_user_data_dir;
    // A profile can hold both tags at once, so populate both slots rather than
    // choosing one. v10 is always available: it is a hardcoded password.
    keyring.v10 = Some(CryptoKey::Aes128Cbc(derive_key(
      POSIX_FALLBACK_PASSWORD,
      POSIX_ITERATIONS,
    )));
    for brand in brand_candidates(family, source_path) {
      match linux_secret_service_password(brand) {
        Ok(Some(password)) => {
          keyring.v11 = Some(CryptoKey::Aes128Cbc(derive_key(
            &password,
            POSIX_ITERATIONS,
          )));
          log::info!("Recovered os_crypt secret for '{brand} Safe Storage'");
          break;
        }
        Ok(None) => continue,
        Err(e) => {
          log::warn!("Secret service lookup for '{brand} Safe Storage' failed: {e}");
          break;
        }
      }
    }
  }

  if keyring.is_empty() {
    report.warn(warning::SECRETS_NOT_MIGRATED);
  }

  // Silence unused-parameter warnings on platforms that do not use every arg.
  let _ = (family, source_path, source_user_data_dir);
  keyring
}

/// How long to wait on a keyring before giving up.
///
/// Both backends can put a dialog in front of the user — macOS asks whether
/// Donut may read another app's Keychain item, and an unlocked-on-demand
/// keyring prompts on Linux. That is fine interactively, but an import driven
/// over REST or MCP would otherwise wedge forever with nobody at the screen.
/// Long enough for a person to notice and click; short enough that automation
/// recovers into "secrets not migrated", which is merely a partial import.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const KEYRING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run a keyring lookup on its own OS thread, bounded by [`KEYRING_TIMEOUT`].
///
/// Off-thread rather than inline for two reasons: import already runs inside
/// `spawn_blocking`, and zbus's blocking API drives a private tokio runtime, so
/// keeping it off a runtime-owned thread sidesteps any nested-runtime question;
/// and it turns a panic or a stuck IPC call into a recoverable warning instead
/// of a failed import.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_keyring_lookup<F>(what: &str, lookup: F) -> Result<Option<Vec<u8>>, String>
where
  F: FnOnce() -> Result<Option<Vec<u8>>, String> + Send + std::panic::UnwindSafe + 'static,
{
  let (tx, rx) = std::sync::mpsc::channel();
  std::thread::spawn(move || {
    let result =
      std::panic::catch_unwind(lookup).unwrap_or_else(|_| Err("lookup panicked".to_string()));
    let _ = tx.send(result);
  });

  match rx.recv_timeout(KEYRING_TIMEOUT) {
    Ok(result) => result,
    Err(_) => Err(format!("{what} did not respond")),
  }
}

#[cfg(target_os = "macos")]
fn macos_keychain_password(brand: &str) -> Result<Option<Vec<u8>>, String> {
  let brand = brand.to_string();
  run_keyring_lookup("keychain", move || macos_keychain_lookup(&brand))
}

#[cfg(target_os = "macos")]
fn macos_keychain_lookup(brand: &str) -> Result<Option<Vec<u8>>, String> {
  use security_framework::passwords::get_generic_password;

  let service = format!("{brand} Safe Storage");
  match get_generic_password(&service, brand) {
    Ok(password) => Ok(Some(password)),
    Err(e) => {
      // errSecItemNotFound: this brand simply is not installed. Anything else
      // (notably errSecAuthFailed / errSecUserCanceled when the user declines
      // the access dialog) is a real failure worth surfacing.
      if e.code() == -25300 {
        Ok(None)
      } else {
        Err(e.to_string())
      }
    }
  }
}

#[cfg(target_os = "windows")]
fn read_local_state_os_crypt(dir: &Path) -> Option<serde_json::Value> {
  let raw = std::fs::read_to_string(dir.join("Local State")).ok()?;
  let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
  parsed.get("os_crypt").cloned()
}

#[cfg(target_os = "windows")]
fn windows_has_app_bound_key(dir: &Path) -> bool {
  read_local_state_os_crypt(dir)
    .and_then(|v| {
      v.get("app_bound_encrypted_key")
        .and_then(|k| k.as_str().map(str::to_string))
    })
    .is_some_and(|k| !k.is_empty())
}

#[cfg(target_os = "windows")]
fn windows_local_state_key(dir: &Path) -> Result<Option<[u8; 32]>, String> {
  use base64::Engine;

  let Some(os_crypt) = read_local_state_os_crypt(dir) else {
    return Ok(None);
  };
  let Some(encoded) = os_crypt.get("encrypted_key").and_then(|k| k.as_str()) else {
    return Ok(None);
  };

  let decoded = base64::engine::general_purpose::STANDARD
    .decode(encoded)
    .map_err(|e| format!("encrypted_key is not valid base64: {e}"))?;

  // The blob is "DPAPI" || CryptProtectData(key).
  const DPAPI_PREFIX: &[u8] = b"DPAPI";
  if !decoded.starts_with(DPAPI_PREFIX) {
    return Err("encrypted_key is missing the DPAPI header".to_string());
  }

  let unwrapped = dpapi_unprotect(&decoded[DPAPI_PREFIX.len()..])?;
  let key: [u8; 32] = unwrapped
    .as_slice()
    .try_into()
    .map_err(|_| format!("expected a 32-byte AES key, got {} bytes", unwrapped.len()))?;
  Ok(Some(key))
}

#[cfg(target_os = "windows")]
fn dpapi_unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, String> {
  use windows::Win32::Foundation::LocalFree;
  use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

  // `pdatain` is `*const CRYPT_INTEGER_BLOB`: DPAPI only reads the input blob,
  // so a shared reference is what the signature wants.
  let input = CRYPT_INTEGER_BLOB {
    cbData: ciphertext.len() as u32,
    pbData: ciphertext.as_ptr() as *mut u8,
  };
  let mut output = CRYPT_INTEGER_BLOB::default();

  // SAFETY: `input` points at a live slice for the duration of the call, and
  // `output` is freed via LocalFree exactly once below, as the API requires.
  unsafe {
    CryptUnprotectData(&input, None, None, None, None, 0, &mut output)
      .map_err(|e| format!("CryptUnprotectData failed: {e}"))?;

    let plaintext = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
    let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(
      output.pbData as *mut core::ffi::c_void,
    )));
    Ok(plaintext)
  }
}

#[cfg(target_os = "linux")]
fn linux_secret_service_password(brand: &str) -> Result<Option<Vec<u8>>, String> {
  let brand = brand.to_string();
  run_keyring_lookup("secret service", move || {
    linux_secret_service_lookup(&brand)
  })
}

#[cfg(target_os = "linux")]
fn linux_secret_service_lookup(brand: &str) -> Result<Option<Vec<u8>>, String> {
  use secret_service::blocking::SecretService;
  use secret_service::EncryptionType;
  use std::collections::HashMap;

  let service =
    SecretService::connect(EncryptionType::Dh).map_err(|e| format!("no secret service: {e}"))?;
  let collection = service
    .get_default_collection()
    .map_err(|e| format!("no default collection: {e}"))?;
  if collection.is_locked().unwrap_or(true) {
    collection
      .unlock()
      .map_err(|e| format!("keyring is locked: {e}"))?;
  }

  // Match on the item's LABEL, not on its `application` attribute.
  //
  // `freedesktop_secret_key_provider.cc` stores two attributes —
  // `application: kAppName` and `xdg:schema` — and sets the label to
  // `kKeyName`, which is always "<Brand> Safe Storage". `kAppName` is a
  // per-fork branding string ("chrome", "chromium", …) that we cannot derive
  // from a display name: lowercasing "Microsoft Edge" gives "microsoft edge",
  // which matches nothing, and the search would silently return zero items.
  // The label is the one identifier that is the same across every fork and is
  // exactly the string we already build for the macOS Keychain.
  let label = format!("{brand} Safe Storage");

  // The schema attribute narrows the scan to os_crypt secrets; it is shared by
  // every Chromium fork, so it costs nothing in portability.
  let mut attributes = HashMap::new();
  attributes.insert("xdg:schema", "chrome_libsecret_os_crypt_password_v2");
  let mut items = collection
    .search_items(attributes)
    .map_err(|e| format!("search failed: {e}"))?;
  if items.is_empty() {
    // Older Chromium releases used a v1 schema, and some forks omit it.
    items = collection
      .get_all_items()
      .map_err(|e| format!("could not list items: {e}"))?;
  }

  for item in &items {
    if item.get_label().is_ok_and(|found| found == label) {
      return item
        .get_secret()
        .map(Some)
        .map_err(|e| format!("could not read secret: {e}"));
    }
  }

  Ok(None)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  #[test]
  fn chromium_family_disambiguates_chrome_from_chromium_by_path() {
    let chrome = PathBuf::from("/Users/x/Library/Application Support/Google/Chrome/Default");
    assert_eq!(brand_candidates("chromium", &chrome)[0], "Chrome");

    let chromium = PathBuf::from("/Users/x/Library/Application Support/Chromium/Default");
    assert_eq!(brand_candidates("chromium", &chromium)[0], "Chromium");
  }

  #[test]
  fn every_brand_falls_back_to_a_second_candidate() {
    // A single candidate means one wrong guess loses the secrets entirely, so
    // each family must offer a fallback identity.
    for family in [
      "chrome-beta",
      "chrome-dev",
      "chrome-canary",
      "brave",
      "edge",
      "vivaldi",
      "opera",
      "opera-gx",
      "arc",
      "yandex",
      "chromium",
    ] {
      let candidates = brand_candidates(family, Path::new("/tmp/profile"));
      assert!(
        candidates.len() >= 2,
        "{family} needs a fallback brand candidate"
      );
    }
  }

  #[test]
  fn unknown_family_still_yields_candidates() {
    let candidates = brand_candidates("something-new", Path::new("/tmp/profile"));
    assert!(!candidates.is_empty());
  }
}

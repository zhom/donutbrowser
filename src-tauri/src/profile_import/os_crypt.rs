//! Key material for profile import.
//!
//! Wayfern deliberately does not use the OS keyring. Every `os_crypt_async`
//! key provider is patched to read (or mint) `<user-data-dir>/os_crypt_key`
//! instead, so a profile directory is self-contained and portable. See
//! `wayfern/patches/extra/fingerprint/components-os_crypt-async-browser-*`.
//!
//! That portability is exactly why an imported Chrome profile carries nothing:
//! its secrets are sealed with a key held in the macOS Keychain / Windows DPAPI
//! / the Freedesktop secret service, and Wayfern never looks there. Import has
//! to open the source's lock and re-seal everything with Wayfern's.
//!
//! The on-disk format is per-platform and NOT interchangeable, matching the
//! provider that owns each tag in the patched Chromium 151 tree:
//!
//! | Host    | `os_crypt_key`      | Derivation                          | Cipher       | Tag   |
//! |---------|---------------------|-------------------------------------|--------------|-------|
//! | macOS   | `base64(16 bytes)`  | PBKDF2-HMAC-SHA1(saltysalt, 1003)   | AES-128-CBC  | `v10` |
//! | Linux   | `base64(16 bytes)`  | PBKDF2-HMAC-SHA1(saltysalt, 1)      | AES-128-CBC  | `v11` |
//! | Windows | 32 raw bytes        | none, the bytes are the key         | AES-256-GCM  | `v10` |
//!
//! Linux must write `v11`, not `v10`: `PosixKeyProvider` owns `v10` with the
//! hardcoded "peanuts" password and `Encryptor::DecryptData` dispatches on the
//! tag prefix, so a `v10` record on Linux would be decrypted with the wrong key
//! forever.

use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
// Windows stores the raw 32-byte key, so it neither encodes nor decodes
// base64; only the mac/Linux password paths below need the trait in scope.
#[cfg(not(target_os = "windows"))]
use base64::Engine;
use rand::RngExt;
use ring::pbkdf2;
use std::num::NonZeroU32;
use std::path::Path;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// Chromium's fixed PBKDF2 salt for every CBC-based os_crypt provider.
// Only the CBC hosts derive a key; Windows uses the file's bytes directly.
#[allow(dead_code)]
pub const SALT: &[u8] = b"saltysalt";
/// Chromium's fixed CBC IV: sixteen spaces.
pub const CBC_IV: [u8; 16] = [b' '; 16];
/// AES-256-GCM nonce length, prepended to the ciphertext by `Encryptor::Key::Encrypt`.
const GCM_NONCE_LEN: usize = 12;
/// The `os_crypt_key` name, at the root of the user-data dir.
pub const KEY_FILE_NAME: &str = "os_crypt_key";

/// `PBKDF2-HMAC-SHA1(password = "", salt = "saltysalt", iterations = 1)`.
///
/// Chromium retries every failed AES-128-CBC decrypt with this key
/// (`encryptor.cc`, crbug.com/40055416) because profiles created while the
/// keyring was unavailable were sealed with an empty password. Import has to do
/// the same or those records look corrupt.
pub const EMPTY_PASSWORD_KEY: [u8; 16] = [
  0xd0, 0xd0, 0xec, 0x9c, 0x7d, 0x77, 0xd4, 0x3a, 0xc5, 0x41, 0x87, 0xfa, 0x48, 0x18, 0xd1, 0x7f,
];

/// The password Chromium's `PosixKeyProvider` uses when no secret service is
/// available (`--password-store=basic`). Records sealed with it carry `v10`.
// Read on Linux and by the known-answer tests; unreferenced on other hosts.
#[allow(dead_code)]
pub const POSIX_FALLBACK_PASSWORD: &[u8] = b"peanuts";

/// PBKDF2 iteration counts, per the provider that owns each platform.
// Each host only ever derives with its own count, but both are needed to read
// a profile produced on the other one.
#[allow(dead_code)]
pub const MAC_ITERATIONS: u32 = 1003;
#[allow(dead_code)]
pub const POSIX_ITERATIONS: u32 = 1;

/// Derive a 16-byte AES-128 key the way every CBC os_crypt provider does.
///
/// `password` is the raw bytes, never trimmed: Chromium passes the exact
/// `ReadFileToString` result to the KDF, so normalising here would silently
/// produce a different key and every decrypt would fail.
// Called from the mac and Linux branches only: `DPAPIKeyProvider` takes the
// 32 bytes on disk as the AES-256 key with no derivation step at all.
#[allow(dead_code)]
pub fn derive_key(password: &[u8], iterations: u32) -> [u8; 16] {
  let mut key = [0u8; 16];
  // ring rather than the `pbkdf2` crate: sha1 0.11 (digest 0.11) and
  // pbkdf2 0.12 (digest 0.10) cannot coexist. ring is self-contained.
  pbkdf2::derive(
    pbkdf2::PBKDF2_HMAC_SHA1,
    NonZeroU32::new(iterations).expect("iterations must be non-zero"),
    SALT,
    password,
    &mut key,
  );
  key
}

/// One os_crypt cipher, keyed. Which variant applies is decided by the tag the
/// record carries, never by the host platform.
#[derive(Clone)]
pub enum CryptoKey {
  Aes128Cbc([u8; 16]),
  // Only Windows keys with GCM, but the variant has to exist everywhere so the
  // tag dispatch in `SourceKeyring` stays platform-independent.
  #[allow(dead_code)]
  Aes256Gcm([u8; 32]),
}

impl CryptoKey {
  /// Decrypt a *tagless* ciphertext (the caller has already stripped the
  /// 3-byte version prefix).
  pub fn decrypt(&self, ciphertext: &[u8]) -> Option<Vec<u8>> {
    match self {
      Self::Aes128Cbc(key) => {
        if ciphertext.is_empty() {
          return Some(Vec::new());
        }
        let mut buf = ciphertext.to_vec();
        Aes128CbcDec::new(key.into(), &CBC_IV.into())
          .decrypt_padded::<Pkcs7>(&mut buf)
          .ok()
          .map(<[u8]>::to_vec)
      }
      Self::Aes256Gcm(key) => {
        if ciphertext.len() < GCM_NONCE_LEN {
          return None;
        }
        let (nonce, body) = ciphertext.split_at(GCM_NONCE_LEN);
        let nonce: [u8; GCM_NONCE_LEN] = nonce.try_into().ok()?;
        Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key))
          .decrypt(
            &Nonce::from(nonce),
            Payload {
              msg: body,
              aad: &[],
            },
          )
          .ok()
      }
    }
  }

  /// Encrypt to a *tagless* ciphertext. The caller prepends the tag.
  pub fn encrypt(&self, plaintext: &[u8]) -> Option<Vec<u8>> {
    match self {
      Self::Aes128Cbc(key) => {
        let mut buf = vec![0u8; plaintext.len() + 16];
        buf[..plaintext.len()].copy_from_slice(plaintext);
        Aes128CbcEnc::new(key.into(), &CBC_IV.into())
          .encrypt_padded::<Pkcs7>(&mut buf, plaintext.len())
          .ok()
          .map(<[u8]>::to_vec)
      }
      Self::Aes256Gcm(key) => {
        let nonce: [u8; GCM_NONCE_LEN] = rand::rng().random();
        let sealed = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key))
          .encrypt(
            &Nonce::from(nonce),
            Payload {
              msg: plaintext,
              aad: &[],
            },
          )
          .ok()?;
        // The nonce goes at the front, matching `Encryptor::Key::Encrypt`.
        let mut out = Vec::with_capacity(GCM_NONCE_LEN + sealed.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Some(out)
      }
    }
  }
}

/// Wayfern's key for the profile being created.
pub struct TargetKey {
  key: CryptoKey,
  tag: &'static [u8; 3],
}

impl TargetKey {
  /// The tag the host platform's key provider claims.
  pub const fn host_tag() -> &'static [u8; 3] {
    #[cfg(target_os = "linux")]
    {
      b"v11"
    }
    #[cfg(not(target_os = "linux"))]
    {
      b"v10"
    }
  }

  /// Build the key from the raw `os_crypt_key` file contents.
  ///
  /// Returns `None` when the contents cannot key the host cipher — on Windows
  /// that means anything other than exactly 32 bytes, which is what
  /// `DPAPIKeyProvider` requires before it will adopt a portable key.
  fn from_file_contents(contents: &[u8]) -> Option<Self> {
    if contents.is_empty() {
      return None;
    }
    #[cfg(target_os = "windows")]
    {
      let bytes: [u8; 32] = contents.try_into().ok()?;
      Some(Self {
        key: CryptoKey::Aes256Gcm(bytes),
        tag: Self::host_tag(),
      })
    }
    #[cfg(target_os = "macos")]
    {
      Some(Self {
        key: CryptoKey::Aes128Cbc(derive_key(contents, MAC_ITERATIONS)),
        tag: Self::host_tag(),
      })
    }
    #[cfg(target_os = "linux")]
    {
      Some(Self {
        key: CryptoKey::Aes128Cbc(derive_key(contents, POSIX_ITERATIONS)),
        tag: Self::host_tag(),
      })
    }
  }

  /// Fresh key material in the host platform's `os_crypt_key` format.
  fn generate_file_contents() -> Vec<u8> {
    #[cfg(target_os = "windows")]
    {
      // Windows stores the AES-256 key itself, so it must be 32 bytes.
      let key: [u8; 32] = rand::rng().random();
      key.to_vec()
    }
    #[cfg(not(target_os = "windows"))]
    {
      // mac/Linux store a *password* that is fed to PBKDF2. Wayfern mints
      // `base64(16 random bytes)`; match it so the file is indistinguishable
      // from one the browser wrote itself.
      let raw: [u8; 16] = rand::rng().random();
      base64::engine::general_purpose::STANDARD
        .encode(raw)
        .into_bytes()
    }
  }

  /// Read the existing `os_crypt_key`, or mint and persist one.
  ///
  /// Writing eagerly at import time — rather than letting the first launch do
  /// it — is deliberate. The mac and Linux patches have no `else` branch when
  /// the write fails, so the browser would run on an in-memory key that dies
  /// with the process and orphans everything it wrote. Failing here instead
  /// turns that silent data loss into a visible import error.
  pub fn ensure(user_data_dir: &Path) -> Result<Self, String> {
    let key_file = user_data_dir.join(KEY_FILE_NAME);

    if let Ok(existing) = std::fs::read(&key_file) {
      if let Some(key) = Self::from_file_contents(&existing) {
        return Ok(key);
      }
      // Present but unusable (a Windows-format key on macOS, say, or a
      // truncated write). Replacing it is safe only because import always
      // re-encrypts into whatever key we end up with.
      log::warn!(
        "Replacing unusable {KEY_FILE_NAME} ({} bytes) at {}",
        existing.len(),
        key_file.display()
      );
    }

    std::fs::create_dir_all(user_data_dir)
      .map_err(|e| format!("Failed to create profile directory: {e}"))?;
    let contents = Self::generate_file_contents();
    std::fs::write(&key_file, &contents)
      .map_err(|e| format!("Failed to write os_crypt_key: {e}"))?;

    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let _ = std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600));
    }

    // Read back rather than trust the write: a key that did not land is the
    // one failure mode that silently destroys every secret we are about to
    // write with it.
    let written =
      std::fs::read(&key_file).map_err(|e| format!("Failed to verify os_crypt_key: {e}"))?;
    if written != contents {
      return Err("os_crypt_key verification failed after write".to_string());
    }

    Self::from_file_contents(&contents).ok_or_else(|| "Failed to derive os_crypt_key".to_string())
  }

  /// Seal a value the way Wayfern will expect to find it: `tag || ciphertext`.
  pub fn encrypt(&self, plaintext: &[u8]) -> Option<Vec<u8>> {
    let body = self.key.encrypt(plaintext)?;
    let mut out = Vec::with_capacity(3 + body.len());
    out.extend_from_slice(self.tag);
    out.extend_from_slice(&body);
    Some(out)
  }
}

/// What a decrypt attempt produced.
pub enum Decrypted {
  /// Recovered plaintext.
  Value(Vec<u8>),
  /// Already plaintext — no recognised version tag.
  NotEncrypted,
  /// Correctly identified but not openable: no key for the tag (Windows
  /// App-Bound `v20`), or every candidate key failed.
  Unrecoverable,
}

/// The source browser's keys, indexed by the tag the records carry.
///
/// Indexing by tag rather than by platform is not pedantry: a single Linux
/// profile can legitimately hold both `v10` (peanuts) and `v11` (keyring)
/// records, because the available secret service changes between sessions.
#[derive(Default)]
pub struct SourceKeyring {
  pub v10: Option<CryptoKey>,
  pub v11: Option<CryptoKey>,
  /// Seen at least one `v20` (Windows App-Bound) record, which no third party
  /// can open. Tracked so the import report can say so explicitly.
  pub saw_app_bound: std::cell::Cell<bool>,
}

impl SourceKeyring {
  pub fn is_empty(&self) -> bool {
    self.v10.is_none() && self.v11.is_none()
  }

  /// Open one stored value, dispatching on its version tag exactly as
  /// `Encryptor::DecryptData` does.
  pub fn decrypt(&self, stored: &[u8]) -> Decrypted {
    if stored.len() < 3 {
      return if stored.is_empty() {
        Decrypted::Value(Vec::new())
      } else {
        Decrypted::NotEncrypted
      };
    }

    let (tag, body) = stored.split_at(3);
    let key = match tag {
      b"v10" => self.v10.as_ref(),
      b"v11" => self.v11.as_ref(),
      b"v20" => {
        // App-Bound Encryption. The key is wrapped by the SYSTEM-level Chrome
        // Elevation Service, which validates the calling binary. There is no
        // legitimate way for us to unwrap it.
        self.saw_app_bound.set(true);
        return Decrypted::Unrecoverable;
      }
      _ => return Decrypted::NotEncrypted,
    };

    let Some(key) = key else {
      return Decrypted::Unrecoverable;
    };

    if let Some(plaintext) = key.decrypt(body) {
      return Decrypted::Value(plaintext);
    }

    // Chromium's own fallback for CBC records sealed with an empty password.
    if matches!(key, CryptoKey::Aes128Cbc(_)) {
      if let Some(plaintext) = CryptoKey::Aes128Cbc(EMPTY_PASSWORD_KEY).decrypt(body) {
        return Decrypted::Value(plaintext);
      }
    }

    Decrypted::Unrecoverable
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  #[test]
  fn empty_password_key_matches_chromium_constant() {
    // Locks the constant against the value Chromium hardcodes in encryptor.cc.
    assert_eq!(derive_key(b"", POSIX_ITERATIONS), EMPTY_PASSWORD_KEY);
  }

  #[test]
  fn peanuts_key_matches_known_vector() {
    // PBKDF2-HMAC-SHA1("peanuts", "saltysalt", 1, 16). Any drift here silently
    // breaks every Linux `--password-store=basic` import.
    assert_eq!(
      derive_key(POSIX_FALLBACK_PASSWORD, POSIX_ITERATIONS),
      [
        0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27,
        0x78
      ]
    );
  }

  #[test]
  fn cbc_round_trip() {
    let key = CryptoKey::Aes128Cbc(derive_key(b"hunter2", MAC_ITERATIONS));
    let sealed = key.encrypt(b"session-token").expect("encrypt");
    assert_eq!(key.decrypt(&sealed).expect("decrypt"), b"session-token");
  }

  #[test]
  fn cbc_round_trip_empty_plaintext() {
    let key = CryptoKey::Aes128Cbc(derive_key(b"hunter2", MAC_ITERATIONS));
    let sealed = key.encrypt(b"").expect("encrypt");
    // PKCS7 always emits a full padding block, so this must not be empty.
    assert_eq!(sealed.len(), 16);
    assert!(key.decrypt(&sealed).expect("decrypt").is_empty());
  }

  #[test]
  fn gcm_round_trip_with_fresh_nonce_each_time() {
    let key = CryptoKey::Aes256Gcm([7u8; 32]);
    let a = key.encrypt(b"session-token").expect("encrypt");
    let b = key.encrypt(b"session-token").expect("encrypt");
    assert_ne!(a, b, "nonce must be random per call");
    assert_eq!(key.decrypt(&a).expect("decrypt"), b"session-token");
    assert_eq!(key.decrypt(&b).expect("decrypt"), b"session-token");
  }

  #[test]
  fn gcm_rejects_tampered_ciphertext() {
    let key = CryptoKey::Aes256Gcm([7u8; 32]);
    let mut sealed = key.encrypt(b"session-token").expect("encrypt");
    let last = sealed.len() - 1;
    sealed[last] ^= 0xff;
    assert!(key.decrypt(&sealed).is_none());
  }

  #[test]
  fn target_key_is_stable_across_calls() {
    let dir = TempDir::new().unwrap();
    let first = TargetKey::ensure(dir.path()).expect("mint");
    let sealed = first.encrypt(b"value").expect("encrypt");

    let second = TargetKey::ensure(dir.path()).expect("reuse");
    // Re-running import over the same directory must not orphan what the
    // previous run wrote.
    let key_file = std::fs::read(dir.path().join(KEY_FILE_NAME)).unwrap();
    let reloaded = TargetKey::from_file_contents(&key_file).expect("reload");
    assert_eq!(
      reloaded.encrypt(b"probe").map(|v| v[..3].to_vec()),
      second.encrypt(b"probe").map(|v| v[..3].to_vec())
    );

    let mut keyring = SourceKeyring::default();
    let contents = std::fs::read(dir.path().join(KEY_FILE_NAME)).unwrap();
    install_host_key(&mut keyring, &contents);
    match keyring.decrypt(&sealed) {
      Decrypted::Value(v) => assert_eq!(v, b"value"),
      _ => panic!("target key must round-trip through the source keyring"),
    }
  }

  #[test]
  fn minted_key_matches_wayfern_file_format() {
    let dir = TempDir::new().unwrap();
    TargetKey::ensure(dir.path()).expect("mint");
    let contents = std::fs::read(dir.path().join(KEY_FILE_NAME)).unwrap();

    #[cfg(target_os = "windows")]
    assert_eq!(
      contents.len(),
      32,
      "DPAPIKeyProvider only adopts a 32-byte portable key"
    );

    #[cfg(not(target_os = "windows"))]
    {
      // Wayfern writes base64(16 random bytes) = 24 ASCII chars.
      assert_eq!(contents.len(), 24);
      let text = String::from_utf8(contents).expect("ascii");
      assert!(
        base64::engine::general_purpose::STANDARD
          .decode(&text)
          .map(|b| b.len())
          == Ok(16),
        "expected base64 of 16 bytes, got {text}"
      );
    }

    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(dir.path().join(KEY_FILE_NAME))
        .unwrap()
        .permissions()
        .mode();
      assert_eq!(mode & 0o777, 0o600);
    }
  }

  #[test]
  fn unknown_tag_is_treated_as_plaintext_not_as_loss() {
    let keyring = SourceKeyring::default();
    assert!(matches!(
      keyring.decrypt(b"plain cookie value"),
      Decrypted::NotEncrypted
    ));
  }

  #[test]
  fn app_bound_records_are_flagged_unrecoverable() {
    let keyring = SourceKeyring::default();
    let mut sealed = b"v20".to_vec();
    sealed.extend_from_slice(&[0u8; 40]);
    assert!(matches!(keyring.decrypt(&sealed), Decrypted::Unrecoverable));
    assert!(
      keyring.saw_app_bound.get(),
      "v20 must be reported to the user, not silently dropped"
    );
  }

  #[test]
  fn missing_key_for_known_tag_is_unrecoverable() {
    let keyring = SourceKeyring::default();
    let mut sealed = b"v10".to_vec();
    sealed.extend_from_slice(&[0u8; 32]);
    assert!(matches!(keyring.decrypt(&sealed), Decrypted::Unrecoverable));
  }

  #[test]
  fn empty_password_fallback_recovers_the_record() {
    // A record sealed with the empty-password key must still open when the
    // keyring holds a different primary key, mirroring Chromium.
    let sealed_body = CryptoKey::Aes128Cbc(EMPTY_PASSWORD_KEY)
      .encrypt(b"legacy")
      .unwrap();
    let mut stored = b"v10".to_vec();
    stored.extend_from_slice(&sealed_body);

    let keyring = SourceKeyring {
      v10: Some(CryptoKey::Aes128Cbc(derive_key(b"a different key", 1003))),
      ..Default::default()
    };
    match keyring.decrypt(&stored) {
      Decrypted::Value(v) => assert_eq!(v, b"legacy"),
      _ => panic!("empty-password fallback must be attempted"),
    }
  }

  /// Load the host-format key into a keyring under the host tag, for tests
  /// that need to verify what we wrote is what Wayfern will read.
  fn install_host_key(keyring: &mut SourceKeyring, contents: &[u8]) {
    #[cfg(target_os = "windows")]
    {
      let bytes: [u8; 32] = contents.try_into().unwrap();
      keyring.v10 = Some(CryptoKey::Aes256Gcm(bytes));
    }
    #[cfg(target_os = "macos")]
    {
      keyring.v10 = Some(CryptoKey::Aes128Cbc(derive_key(contents, MAC_ITERATIONS)));
    }
    #[cfg(target_os = "linux")]
    {
      keyring.v11 = Some(CryptoKey::Aes128Cbc(derive_key(contents, POSIX_ITERATIONS)));
    }
  }
}

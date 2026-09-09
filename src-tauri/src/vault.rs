//! Sealing of the secrets Donut keeps on this machine.
//!
//! The API and MCP tokens, the cloud session, the sync token and the sync
//! encryption password each live in a small file under the settings folder.
//! They are sealed with AES-256-GCM under a key derived (Argon2id, per-file
//! salt) from this installation's own vault key: 32 random bytes minted on
//! first use and kept in `vault.key`, readable by the owner only.
//!
//! Every build before the per-install key sealed those files under one
//! password compiled into the binary, the same for every install whose build
//! did not set `DONUT_BROWSER_VAULT_PASSWORD`. A file that still carries that
//! seal is opened with the legacy password and re-sealed under the
//! installation key on the spot, so an update keeps every login and token.
//!
//! File layout, unchanged from the earlier per-module copies:
//! `magic (5-byte header + 1 version byte) | salt length | PHC base64 salt |
//! 12-byte nonce | 4-byte little-endian ciphertext length | ciphertext`.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::sync::encryption::{decode_salt, derive_vault_key, encode_salt};

pub const VAULT_KEY_FILE: &str = "vault.key";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// The sealing password of every build before the per-install key. Written
/// by `build.rs` from the build environment, with the historical default when
/// nothing was set. Only ever used to open a file sealed by such a build.
const LEGACY_PASSWORD: &str = include_str!(concat!(env!("OUT_DIR"), "/legacy_vault_password.txt"));

/// The installation key, cached with the file it came from so a test that
/// moves the settings folder never reads a key from the previous one.
static INSTALL_KEY: Mutex<Option<(PathBuf, [u8; KEY_LEN])>> = Mutex::new(None);

fn key_file() -> PathBuf {
  crate::app_dirs::settings_dir().join(VAULT_KEY_FILE)
}

/// This installation's vault key, minted the first time anything needs it.
///
/// A key file of the wrong size is refused rather than replaced: minting a
/// new key over it would silently orphan every file sealed under the old one.
pub fn install_key() -> Result<[u8; KEY_LEN], String> {
  let path = key_file();
  if let Ok(cached) = INSTALL_KEY.lock() {
    if let Some((cached_path, key)) = cached.as_ref() {
      if *cached_path == path {
        return Ok(*key);
      }
    }
  }
  let key = match std::fs::read(&path) {
    Ok(bytes) if bytes.len() == KEY_LEN => <[u8; KEY_LEN]>::try_from(bytes.as_slice())
      .map_err(|_| "The vault key file could not be read whole".to_string())?,
    Ok(bytes) => {
      return Err(format!(
        "The vault key file {} holds {} bytes instead of {KEY_LEN}",
        path.display(),
        bytes.len()
      ));
    }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => mint_key(&path)?,
    Err(e) => return Err(format!("Could not read the vault key: {e}")),
  };
  if let Ok(mut cached) = INSTALL_KEY.lock() {
    *cached = Some((path, key));
  }
  Ok(key)
}

/// Write a fresh key next to the sealed files. Written to a sibling first and
/// renamed into place, so a crash mid-write never leaves a short key behind.
fn mint_key(path: &Path) -> Result<[u8; KEY_LEN], String> {
  let key: [u8; KEY_LEN] = rand::rng().random();
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)
      .map_err(|e| format!("Could not create the settings folder: {e}"))?;
  }
  let staging = path.with_extension("key.tmp");
  std::fs::write(&staging, key).map_err(|e| format!("Could not write the vault key: {e}"))?;
  crate::app_dirs::restrict_to_owner(&staging);
  if let Err(e) = std::fs::rename(&staging, path) {
    let _ = std::fs::remove_file(&staging);
    // Another process minted the key first; theirs is the one to keep.
    if path.exists() {
      return install_key();
    }
    return Err(format!("Could not place the vault key: {e}"));
  }
  crate::app_dirs::restrict_to_owner(path);
  Ok(key)
}

/// Seal `secret` into `file` under this installation's key.
pub fn seal(file: &Path, magic: &[u8; 6], secret: &str) -> Result<(), String> {
  let key = install_key()?;
  seal_with(file, magic, secret, &key)
}

fn seal_with(file: &Path, magic: &[u8; 6], secret: &str, material: &[u8]) -> Result<(), String> {
  if let Some(parent) = file.parent() {
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
  }
  let salt_bytes: [u8; 16] = rand::rng().random();
  let salt = encode_salt(&salt_bytes);
  let key = Key::<Aes256Gcm>::from(derive_vault_key(material, &salt_bytes)?);
  let cipher = Aes256Gcm::new(&key);
  let nonce_bytes: [u8; NONCE_LEN] = rand::rng().random();
  let nonce = Nonce::from(nonce_bytes);
  let ciphertext = cipher
    .encrypt(&nonce, secret.as_bytes())
    .map_err(|e| format!("Encryption failed: {e}"))?;

  let mut data = Vec::new();
  data.extend_from_slice(magic);
  let salt_str = salt.as_str();
  data.push(salt_str.len() as u8);
  data.extend_from_slice(salt_str.as_bytes());
  data.extend_from_slice(&nonce);
  data.extend_from_slice(&(ciphertext.len() as u32).to_le_bytes());
  data.extend_from_slice(&ciphertext);

  std::fs::write(file, data).map_err(|e| format!("Failed to write file: {e}"))?;
  crate::app_dirs::restrict_to_owner(file);
  Ok(())
}

/// The parts of a sealed file, once the layout has been checked.
struct Sealed<'a> {
  salt: Vec<u8>,
  nonce: [u8; NONCE_LEN],
  ciphertext: &'a [u8],
}

/// Take a sealed file apart. A foreign magic or a layout this version does not
/// know reads as "no secret", never as an error.
fn parse<'a>(data: &'a [u8], magic: &[u8; 6]) -> Result<Option<Sealed<'a>>, String> {
  if data.len() < magic.len() + 1 || &data[..magic.len()] != magic {
    return Ok(None);
  }
  let mut offset = magic.len();
  let salt_len = data[offset] as usize;
  offset += 1;
  if offset + salt_len > data.len() {
    return Ok(None);
  }
  let salt_str =
    std::str::from_utf8(&data[offset..offset + salt_len]).map_err(|_| "Invalid salt encoding")?;
  let salt = decode_salt(salt_str)?;
  offset += salt_len;
  if offset + NONCE_LEN > data.len() {
    return Ok(None);
  }
  let nonce: [u8; NONCE_LEN] = data[offset..offset + NONCE_LEN]
    .try_into()
    .map_err(|_| "Invalid nonce length".to_string())?;
  offset += NONCE_LEN;
  if offset + 4 > data.len() {
    return Ok(None);
  }
  let ciphertext_len = u32::from_le_bytes([
    data[offset],
    data[offset + 1],
    data[offset + 2],
    data[offset + 3],
  ]) as usize;
  offset += 4;
  if offset + ciphertext_len > data.len() {
    return Ok(None);
  }
  Ok(Some(Sealed {
    salt,
    nonce,
    ciphertext: &data[offset..offset + ciphertext_len],
  }))
}

fn unseal(sealed: &Sealed<'_>, material: &[u8]) -> Result<Option<String>, String> {
  let key = Key::<Aes256Gcm>::from(derive_vault_key(material, &sealed.salt)?);
  let cipher = Aes256Gcm::new(&key);
  let Ok(plaintext) = cipher.decrypt(&Nonce::from(sealed.nonce), sealed.ciphertext) else {
    return Ok(None);
  };
  Ok(String::from_utf8(plaintext).ok())
}

/// Read back a secret written by `seal`.
///
/// A missing file, a foreign magic or a damaged layout all read as "no
/// secret" so a stale file never blocks the feature it belongs to. A file
/// that opens only under the legacy build password is re-sealed under this
/// installation's key before the secret is returned. A seal that neither key
/// opens is an error: the file is real, and the caller must not mint over it
/// as if it were absent.
pub fn open(file: &Path, magic: &[u8; 6]) -> Result<Option<String>, String> {
  if !file.exists() {
    return Ok(None);
  }
  let data = std::fs::read(file).map_err(|e| format!("Failed to read file: {e}"))?;
  let Some(sealed) = parse(&data, magic)? else {
    return Ok(None);
  };
  let key = install_key()?;
  if let Some(secret) = unseal(&sealed, &key)? {
    return Ok(Some(secret));
  }
  match unseal(&sealed, LEGACY_PASSWORD.trim().as_bytes())? {
    Some(secret) => {
      if let Err(e) = seal_with(file, magic, &secret, &key) {
        log::warn!(
          "Could not re-seal {} under the vault key: {e}",
          file.display()
        );
      }
      Ok(Some(secret))
    }
    None => Err("Decryption failed".to_string()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tempfile::TempDir;

  fn isolated() -> (TempDir, crate::app_dirs::TestDirGuard) {
    let dir = TempDir::new().unwrap();
    let guard = crate::app_dirs::set_test_data_dir(dir.path().to_path_buf());
    (dir, guard)
  }

  #[test]
  fn the_key_is_minted_once_and_reused() {
    let (_dir, _guard) = isolated();
    let first = install_key().unwrap();
    let second = install_key().unwrap();
    assert_eq!(first, second);
    assert_eq!(std::fs::read(key_file()).unwrap().len(), KEY_LEN);
  }

  #[test]
  fn a_seal_round_trips_and_a_foreign_magic_reads_as_nothing() {
    let (dir, _guard) = isolated();
    let file = dir.path().join("secret.dat");
    seal(&file, b"DBTST\x02", "hunter's token").unwrap();
    assert_eq!(
      open(&file, b"DBTST\x02").unwrap().as_deref(),
      Some("hunter's token")
    );
    assert_eq!(open(&file, b"DBOTH\x02").unwrap(), None);
    assert_eq!(
      open(&dir.path().join("missing.dat"), b"DBTST\x02").unwrap(),
      None
    );
  }

  #[test]
  fn a_legacy_seal_opens_once_and_comes_back_under_the_install_key() {
    let (dir, _guard) = isolated();
    let file = dir.path().join("legacy.dat");
    seal_with(
      &file,
      b"DBTST\x02",
      "kept",
      LEGACY_PASSWORD.trim().as_bytes(),
    )
    .unwrap();
    assert_eq!(open(&file, b"DBTST\x02").unwrap().as_deref(), Some("kept"));

    // Re-sealed: the legacy password no longer opens the file, the key does.
    let data = std::fs::read(&file).unwrap();
    let sealed = parse(&data, b"DBTST\x02").unwrap().unwrap();
    assert_eq!(
      unseal(&sealed, LEGACY_PASSWORD.trim().as_bytes()).unwrap(),
      None
    );
    assert_eq!(
      unseal(&sealed, &install_key().unwrap()).unwrap().as_deref(),
      Some("kept")
    );
  }

  #[test]
  fn a_seal_under_an_unknown_key_is_an_error_not_an_absence() {
    let (dir, _guard) = isolated();
    let file = dir.path().join("foreign.dat");
    let other: [u8; KEY_LEN] = rand::rng().random();
    seal_with(&file, b"DBTST\x02", "elsewhere", &other).unwrap();
    assert!(open(&file, b"DBTST\x02").is_err());
  }

  #[test]
  fn a_damaged_key_file_is_refused_rather_than_replaced() {
    let (_dir, _guard) = isolated();
    std::fs::create_dir_all(key_file().parent().unwrap()).unwrap();
    std::fs::write(key_file(), b"short").unwrap();
    assert!(install_key().is_err());
  }
}

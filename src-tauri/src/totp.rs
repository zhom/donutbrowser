//! Time-based one-time passwords (RFC 6238), for the 2FA codes of the
//! accounts a profile signs in to.
//!
//! A key is read from an `otpauth://totp/...` link (what a QR code holds) or
//! from the bare Base32 secret most sites show under "can't scan the code".

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum TotpAlgorithm {
  #[default]
  Sha1,
  Sha256,
  Sha512,
}

/// A parsed key: everything needed to compute a code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotpKey {
  /// The shared secret, Base32 (RFC 4648) without padding.
  pub secret: String,
  #[serde(default)]
  pub algorithm: TotpAlgorithm,
  pub digits: u32,
  pub period: u64,
  #[serde(default)]
  pub issuer: Option<String>,
  #[serde(default)]
  pub account: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TotpCode {
  pub code: String,
  /// Unix seconds at which `code` stops being valid.
  pub valid_until: u64,
  pub period: u64,
}

/// The input is neither a usable `otpauth://totp` link nor a Base32 secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTotpKey;

/// 80 bits: authenticator apps refuse shorter keys, and a word or a typo
/// would otherwise read as a valid secret and give wrong codes.
const MIN_SECRET_BYTES: usize = 10;
const MAX_SECRET_BYTES: usize = 128;

fn decode_base32(input: &str) -> Option<Vec<u8>> {
  let mut bits: u64 = 0;
  let mut bit_count = 0u32;
  let mut out = Vec::with_capacity(input.len() * 5 / 8);
  for c in input.chars() {
    if c == '=' || c == ' ' || c == '-' {
      continue;
    }
    let value = match c.to_ascii_uppercase() {
      c @ 'A'..='Z' => c as u64 - 'A' as u64,
      c @ '2'..='7' => c as u64 - '2' as u64 + 26,
      _ => return None,
    };
    bits = (bits << 5) | value;
    bit_count += 5;
    if bit_count >= 8 {
      bit_count -= 8;
      out.push((bits >> bit_count) as u8);
      bits &= (1 << bit_count) - 1;
    }
  }
  (MIN_SECRET_BYTES..=MAX_SECRET_BYTES)
    .contains(&out.len())
    .then_some(out)
}

fn normalized_secret(raw: &str) -> Option<String> {
  decode_base32(raw)?;
  Some(
    raw
      .chars()
      .filter(|c| !matches!(c, '=' | ' ' | '-'))
      .map(|c| c.to_ascii_uppercase())
      .collect(),
  )
}

impl TotpKey {
  pub fn parse(input: &str) -> Result<Self, InvalidTotpKey> {
    let input = input.trim();
    if input.len() > 2048 {
      return Err(InvalidTotpKey);
    }
    if !input
      .get(..10)
      .is_some_and(|scheme| scheme.eq_ignore_ascii_case("otpauth://"))
    {
      return Ok(Self {
        secret: normalized_secret(input).ok_or(InvalidTotpKey)?,
        algorithm: TotpAlgorithm::Sha1,
        digits: 6,
        period: 30,
        issuer: None,
        account: None,
      });
    }

    let url = url::Url::parse(input).map_err(|_| InvalidTotpKey)?;
    // HOTP counts presses instead of time and needs a counter kept in step
    // with the server; it is not what sites use for 2FA codes.
    if !url
      .host_str()
      .is_some_and(|kind| kind.eq_ignore_ascii_case("totp"))
    {
      return Err(InvalidTotpKey);
    }
    let label = urlencoding::decode(url.path().trim_start_matches('/'))
      .map(|l| l.into_owned())
      .unwrap_or_default();
    let (label_issuer, account) = match label.split_once(':') {
      Some((issuer, account)) => (Some(issuer.trim().to_string()), account.trim().to_string()),
      None => (None, label.trim().to_string()),
    };

    let mut key = Self {
      secret: String::new(),
      algorithm: TotpAlgorithm::Sha1,
      digits: 6,
      period: 30,
      issuer: label_issuer.filter(|i| !i.is_empty()),
      account: Some(account).filter(|a| !a.is_empty()),
    };
    for (name, value) in url.query_pairs() {
      match name.to_ascii_lowercase().as_str() {
        "secret" => key.secret = normalized_secret(&value).ok_or(InvalidTotpKey)?,
        "issuer" if !value.trim().is_empty() => key.issuer = Some(value.trim().to_string()),
        "algorithm" => {
          key.algorithm = match value.to_ascii_uppercase().as_str() {
            "SHA1" => TotpAlgorithm::Sha1,
            "SHA256" => TotpAlgorithm::Sha256,
            "SHA512" => TotpAlgorithm::Sha512,
            _ => return Err(InvalidTotpKey),
          }
        }
        "digits" => {
          key.digits = value.parse().map_err(|_| InvalidTotpKey)?;
        }
        "period" => {
          key.period = value.parse().map_err(|_| InvalidTotpKey)?;
        }
        _ => {}
      }
    }
    if key.secret.is_empty() || !(6..=8).contains(&key.digits) || !(1..=300).contains(&key.period) {
      return Err(InvalidTotpKey);
    }
    Ok(key)
  }

  /// The code for the time step containing `unix_secs`.
  pub fn code_at(&self, unix_secs: u64) -> String {
    let secret = decode_base32(&self.secret).unwrap_or_default();
    let algorithm = match self.algorithm {
      TotpAlgorithm::Sha1 => ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
      TotpAlgorithm::Sha256 => ring::hmac::HMAC_SHA256,
      TotpAlgorithm::Sha512 => ring::hmac::HMAC_SHA512,
    };
    let counter = unix_secs / self.period.max(1);
    let tag = ring::hmac::sign(
      &ring::hmac::Key::new(algorithm, &secret),
      &counter.to_be_bytes(),
    );
    let hash = tag.as_ref();
    let offset = (hash[hash.len() - 1] & 0x0F) as usize;
    let truncated = u32::from_be_bytes([
      hash[offset] & 0x7F,
      hash[offset + 1],
      hash[offset + 2],
      hash[offset + 3],
    ]);
    let code = truncated % 10u32.pow(self.digits);
    format!("{code:0width$}", width = self.digits as usize)
  }

  pub fn code_now(&self) -> TotpCode {
    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_secs())
      .unwrap_or(0);
    let period = self.period.max(1);
    TotpCode {
      code: self.code_at(now),
      valid_until: (now / period + 1) * period,
      period,
    }
  }
}

/// The code for a key the person pastes, without keeping the key anywhere.
#[tauri::command]
pub fn generate_totp_code(key: String) -> Result<TotpCode, String> {
  TotpKey::parse(&key)
    .map(|key| key.code_now())
    .map_err(|_| crate::backend_error("TOTP_KEY_INVALID"))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let mut bits: u64 = 0;
    let mut count = 0;
    for byte in bytes {
      bits = (bits << 8) | u64::from(*byte);
      count += 8;
      while count >= 5 {
        count -= 5;
        out.push(ALPHABET[((bits >> count) & 31) as usize] as char);
      }
    }
    if count > 0 {
      out.push(ALPHABET[((bits << (5 - count)) & 31) as usize] as char);
    }
    out
  }

  fn rfc_key(secret: &[u8], algorithm: TotpAlgorithm) -> TotpKey {
    TotpKey {
      secret: base32(secret),
      algorithm,
      digits: 8,
      period: 30,
      issuer: None,
      account: None,
    }
  }

  /// RFC 6238 appendix B.
  #[test]
  fn codes_match_the_rfc_6238_vectors() {
    let sha1 = rfc_key(b"12345678901234567890", TotpAlgorithm::Sha1);
    let sha256 = rfc_key(b"12345678901234567890123456789012", TotpAlgorithm::Sha256);
    let sha512 = rfc_key(
      b"1234567890123456789012345678901234567890123456789012345678901234",
      TotpAlgorithm::Sha512,
    );
    for (time, a, b, c) in [
      (59, "94287082", "46119246", "90693936"),
      (1111111109, "07081804", "68084774", "25091201"),
      (1111111111, "14050471", "67062674", "99943326"),
      (1234567890, "89005924", "91819424", "93441116"),
      (2000000000, "69279037", "90698825", "38618901"),
      (20000000000, "65353130", "77737706", "47863826"),
    ] {
      assert_eq!(sha1.code_at(time), a, "SHA1 at {time}");
      assert_eq!(sha256.code_at(time), b, "SHA256 at {time}");
      assert_eq!(sha512.code_at(time), c, "SHA512 at {time}");
    }
  }

  #[test]
  fn a_bare_secret_is_read_the_way_sites_print_it() {
    let key = TotpKey::parse(" jbsw y3dp ehpk 3pxp ").unwrap();
    assert_eq!(key.secret, "JBSWY3DPEHPK3PXP");
    assert_eq!((key.digits, key.period), (6, 30));
    assert_eq!(key.algorithm, TotpAlgorithm::Sha1);
    assert_eq!(
      TotpKey::parse("JBSWY3DP-EHPK3PXP===").unwrap().secret,
      "JBSWY3DPEHPK3PXP"
    );
    assert_eq!(key.code_at(59).len(), 6);
  }

  #[test]
  fn an_otpauth_link_keeps_its_issuer_account_and_parameters() {
    let key = TotpKey::parse(
      "otpauth://totp/Example%20Co:alice@example.com?secret=JBSWY3DPEHPK3PXP&issuer=Example&algorithm=SHA256&digits=8&period=60",
    )
    .unwrap();
    assert_eq!(key.issuer.as_deref(), Some("Example"));
    assert_eq!(key.account.as_deref(), Some("alice@example.com"));
    assert_eq!(key.algorithm, TotpAlgorithm::Sha256);
    assert_eq!((key.digits, key.period), (8, 60));

    let no_issuer = TotpKey::parse("otpauth://totp/bob?secret=JBSWY3DPEHPK3PXP").unwrap();
    assert_eq!(no_issuer.issuer, None);
    assert_eq!(no_issuer.account.as_deref(), Some("bob"));
  }

  #[test]
  fn unusable_keys_are_refused() {
    for input in [
      "",
      "   ",
      "not base32!",
      "not a key",
      "JBSWY3DP",
      "otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP&counter=1",
      "otpauth://totp/x",
      "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&digits=4",
      "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&period=0",
      "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP&algorithm=MD5",
      "otpauth://totp/x?secret=0189",
    ] {
      assert_eq!(TotpKey::parse(input), Err(InvalidTotpKey), "{input:?}");
    }
    assert!(
      TotpKey::parse(&"A".repeat(300)).is_err(),
      "longer than any real key"
    );
  }

  #[test]
  fn a_code_is_valid_until_the_end_of_its_step() {
    let code = TotpKey::parse("JBSWY3DPEHPK3PXP").unwrap().code_now();
    assert_eq!(code.valid_until % 30, 0);
    assert_eq!(code.code.len(), 6);
    assert!(generate_totp_code("nope!".to_string()).is_err());
  }
}

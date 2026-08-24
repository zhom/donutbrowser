//! SHA256 helpers shared by the app self-updater and the browser downloader.
//!
//! Both verify a downloaded artifact against a digest published beside it, so
//! the hashing and the `sha256sum` parsing live here instead of in either
//! caller.

use std::path::Path;

/// Stream `path` through SHA256 and return the lowercase hex digest. Reads in
/// 1 MiB blocks so a multi-gigabyte browser archive never lands in memory.
pub fn sha256_file(path: &Path) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
  use sha2::{Digest, Sha256};
  use std::io::Read;
  let mut file = std::fs::File::open(path)?;
  let mut hasher = Sha256::new();
  let mut buf = vec![0u8; 1024 * 1024];
  loop {
    let n = file.read(&mut buf)?;
    if n == 0 {
      break;
    }
    hasher.update(&buf[..n]);
  }
  let digest = hasher.finalize();
  let mut hex = String::with_capacity(digest.len() * 2);
  for byte in digest {
    use std::fmt::Write;
    let _ = write!(hex, "{byte:02x}");
  }
  Ok(hex)
}

/// Extract the hex digest for `filename` from standard `sha256sum` output
/// (`<hex>  <name>`, optionally with the `*` binary-mode marker).
pub fn find_checksum_for_file(checksums_text: &str, filename: &str) -> Option<String> {
  checksums_text.lines().find_map(|line| {
    let (hash, rest) = line.split_once(char::is_whitespace)?;
    let name = rest.trim_start().trim_start_matches('*');
    if name == filename && is_sha256_hex(hash) {
      Some(hash.to_ascii_lowercase())
    } else {
      None
    }
  })
}

/// Digest from a single-asset `<file>.sha256` sidecar.
///
/// Prefers the entry named `filename`, because a name binds the digest to the
/// asset it covers. Falls back to a lone digest only when the sidecar holds
/// exactly one: `sha256sum < file` writes `-` as the name and some publishers
/// emit the bare hash, and neither is ambiguous when it stands alone. A
/// sidecar listing several assets always needs the name to match.
pub fn parse_sidecar_digest(text: &str, filename: &str) -> Option<String> {
  if let Some(named) = find_checksum_for_file(text, filename) {
    return Some(named);
  }

  let mut digests = text
    .lines()
    .filter_map(|line| line.split_whitespace().next())
    .filter(|token| is_sha256_hex(token));
  let only = digests.next()?;
  if digests.next().is_some() {
    return None;
  }
  Some(only.to_ascii_lowercase())
}

fn is_sha256_hex(value: &str) -> bool {
  value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
  use super::*;

  const HELLO_WORLD_SHA256: &str =
    "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

  #[test]
  fn test_find_checksum_for_file() {
    let sums = "\
0e5a4601745092b7d1c93c1e7e1c30d923be3d1e916b661bd53d1c0c9c7f0a11  Donut_0.29.0_aarch64.dmg
ABCDEF01745092B7D1C93C1E7E1C30D923BE3D1E916B661BD53D1C0C9C7F0A22 *Donut_0.29.0_x64.dmg
not-a-hash  Donut_0.29.0_amd64.deb
";

    // Plain entry.
    assert_eq!(
      find_checksum_for_file(sums, "Donut_0.29.0_aarch64.dmg").as_deref(),
      Some("0e5a4601745092b7d1c93c1e7e1c30d923be3d1e916b661bd53d1c0c9c7f0a11")
    );
    // Binary-mode marker is stripped; hash is normalized to lowercase.
    assert_eq!(
      find_checksum_for_file(sums, "Donut_0.29.0_x64.dmg").as_deref(),
      Some("abcdef01745092b7d1c93c1e7e1c30d923be3d1e916b661bd53d1c0c9c7f0a22")
    );
    // Entries with malformed hashes are rejected rather than trusted.
    assert_eq!(find_checksum_for_file(sums, "Donut_0.29.0_amd64.deb"), None);
    // Missing file.
    assert_eq!(find_checksum_for_file(sums, "Donut_0.29.0_arm64.deb"), None);
  }

  #[test]
  fn test_sha256_file_matches_known_digest() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("data.bin");
    std::fs::write(&path, b"hello world").unwrap();
    assert_eq!(sha256_file(&path).unwrap(), HELLO_WORLD_SHA256);
  }

  #[test]
  fn test_parse_sidecar_digest_prefers_the_named_entry() {
    // The real Wayfern sidecar shape: `<hex>  <name>`, one asset per file.
    let sidecar = format!("{HELLO_WORLD_SHA256}  wayfern-151.0.7922.71_windows_x64.zip\n");
    assert_eq!(
      parse_sidecar_digest(&sidecar, "wayfern-151.0.7922.71_windows_x64.zip").as_deref(),
      Some(HELLO_WORLD_SHA256)
    );
  }

  #[test]
  fn test_parse_sidecar_digest_accepts_an_unnamed_lone_digest() {
    // `sha256sum < file` writes `-` as the name, and some publishers emit the
    // bare hash. Both cover the one asset the sidecar sits beside.
    for sidecar in [
      format!("{HELLO_WORLD_SHA256}  -\n"),
      format!("{HELLO_WORLD_SHA256}\n"),
      format!("  {HELLO_WORLD_SHA256}  \n"),
    ] {
      assert_eq!(
        parse_sidecar_digest(&sidecar, "wayfern.zip").as_deref(),
        Some(HELLO_WORLD_SHA256),
        "should accept lone digest in {sidecar:?}"
      );
    }
  }

  #[test]
  fn test_parse_sidecar_digest_normalizes_case() {
    let sidecar = format!("{}  -\n", HELLO_WORLD_SHA256.to_ascii_uppercase());
    assert_eq!(
      parse_sidecar_digest(&sidecar, "wayfern.zip").as_deref(),
      Some(HELLO_WORLD_SHA256)
    );
  }

  #[test]
  fn test_parse_sidecar_digest_rejects_an_ambiguous_multi_entry_sidecar() {
    let sidecar = format!(
      "{HELLO_WORLD_SHA256}  other.zip\n\
       ABCDEF01745092B7D1C93C1E7E1C30D923BE3D1E916B661BD53D1C0C9C7F0A22  another.zip\n"
    );
    // Two candidates and neither is named `wayfern.zip`: guessing would defeat
    // the point of the check.
    assert_eq!(parse_sidecar_digest(&sidecar, "wayfern.zip"), None);
  }

  #[test]
  fn test_parse_sidecar_digest_rejects_junk() {
    assert_eq!(parse_sidecar_digest("", "wayfern.zip"), None);
    assert_eq!(
      parse_sidecar_digest("not-a-hash  wayfern.zip", "wayfern.zip"),
      None
    );
    // An HTML error page served with HTTP 200 must not read as a digest.
    assert_eq!(
      parse_sidecar_digest("<!doctype html><title>404</title>", "wayfern.zip"),
      None
    );
    // Right shape, wrong length.
    assert_eq!(
      parse_sidecar_digest("abc123  wayfern.zip", "wayfern.zip"),
      None
    );
  }
}

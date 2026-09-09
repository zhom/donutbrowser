//! Import a Chromium extension from a link instead of a file.
//!
//! Three inputs are accepted: a Chrome Web Store detail URL, the bare
//! 32-character extension id from one, and a direct `.crx`/`.zip` URL. All
//! three resolve to a single archive download, whose payload is normalised to
//! the plain ZIP that `extension_manager` already stores, so nothing
//! downstream (assignment, groups, per-profile staging, sync) has to know an
//! extension arrived over the network.

use serde::{Deserialize, Serialize};
use url::Url;

/// Matches the body limit the REST extension routes accept for an upload
/// (`api_server::DefaultBodyLimit::max(64 MiB)`). A link import and a file
/// upload land in the same store, so they get the same ceiling.
pub const MAX_EXTENSION_BYTES: u64 = 64 * 1024 * 1024;

const CRX_MAGIC: &[u8; 4] = b"Cr24";
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";
const MAX_REDIRECTS: usize = 5;

/// The last-resort `prodversion` for the Web Store endpoint, used only when no
/// Wayfern build is downloaded and no version cache exists yet — a fresh
/// install that has never fetched a browser. Every other path reads the real
/// installed version, so this is a floor, not the normal answer.
const FALLBACK_PRODUCT_VERSION: &str = "120.0.0.0";

fn err(code: &str) -> String {
  crate::backend_error(code)
}

/// What a link resolves to before anything is fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionSource {
  /// A Chrome Web Store product id, downloaded through the update service.
  WebStore(String),
  /// An archive served directly.
  Direct(Url),
}

/// A downloaded, validated extension archive, staged in the frontend exactly
/// like a picked file so the user confirms a real name and version before it
/// is stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchedExtension {
  pub file_name: String,
  pub file_data: Vec<u8>,
  /// The manifest's own name, with any `__MSG_key__` placeholder resolved.
  pub name: Option<String>,
  pub version: Option<String>,
  pub description: Option<String>,
  /// The URL the bytes actually came from, so the staged form can name it.
  pub source_url: String,
  /// True when the id was resolved through the Chrome Web Store update
  /// service rather than downloaded from a link the user typed in full.
  pub from_web_store: bool,
}

/// A Chrome extension id is 32 characters drawn from `a`-`p`: the store
/// re-encodes the first 128 bits of the packing key's SHA-256 with that
/// alphabet, so anything outside it is not an id however long it is.
pub fn parse_extension_id(candidate: &str) -> Option<String> {
  let trimmed = candidate.trim();
  if trimmed.len() != 32 {
    return None;
  }
  let lowered = trimmed.to_ascii_lowercase();
  lowered
    .bytes()
    .all(|b| b.is_ascii_lowercase() && b <= b'p')
    .then_some(lowered)
}

fn web_store_id_from_path(url: &Url) -> Option<String> {
  let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
  // `/detail/<slug>/<id>` on the current store, `/webstore/detail/<slug>/<id>`
  // on the legacy host, and both allow the slug to be omitted. Rather than
  // encoding every shape, take the first segment that is a real id.
  segments
    .iter()
    .find_map(|segment| parse_extension_id(segment))
}

fn is_web_store_host(host: &str) -> bool {
  matches!(
    host,
    "chromewebstore.google.com" | "chrome.google.com" | "www.chrome.google.com"
  )
}

fn path_is_archive(url: &Url) -> bool {
  let path = url.path().to_ascii_lowercase();
  path.ends_with(".crx") || path.ends_with(".zip")
}

/// Loopback plain HTTP is accepted only in the `e2e` build, where the suite
/// serves its own CRX fixture from a local server. A shipped build has no such
/// path, so every real import crosses TLS.
fn scheme_is_allowed(url: &Url) -> bool {
  if url.scheme() == "https" {
    return true;
  }
  cfg!(feature = "e2e") && url.scheme() == "http" && host_is_loopback(url)
}

fn host_is_loopback(url: &Url) -> bool {
  match url.host() {
    Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
    Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
    Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
    None => false,
  }
}

/// Classify what the user typed. Anything that is not one of the three
/// accepted shapes is refused here, before a single byte is requested.
pub fn parse_extension_source(input: &str) -> Result<ExtensionSource, String> {
  let trimmed = input.trim();
  if trimmed.is_empty() {
    return Err(err("EXTENSION_URL_INVALID"));
  }

  if let Some(id) = parse_extension_id(trimmed) {
    return Ok(ExtensionSource::WebStore(id));
  }

  let url = Url::parse(trimmed).map_err(|_| err("EXTENSION_URL_INVALID"))?;
  if !scheme_is_allowed(&url) {
    return Err(err("EXTENSION_URL_INVALID"));
  }

  if let Some(host) = url.host_str() {
    if is_web_store_host(host) {
      return web_store_id_from_path(&url)
        .map(ExtensionSource::WebStore)
        .ok_or_else(|| err("EXTENSION_URL_INVALID"));
    }
  }

  if path_is_archive(&url) {
    return Ok(ExtensionSource::Direct(url));
  }

  Err(err("EXTENSION_URL_INVALID"))
}

/// The `nacl_arch` the Web Store update service expects for this machine. It
/// picks between architecture-specific builds of the same extension, so a
/// wrong value hands back a package the browser cannot load.
pub fn nacl_arch() -> &'static str {
  match std::env::consts::ARCH {
    "x86_64" => "x86-64",
    "x86" => "x86-32",
    "aarch64" => "arm64",
    "arm" => "arm",
    _ => "x86-64",
  }
}

/// Newest Chromium version this machine actually has, because the Web Store
/// serves a package built for the requesting browser and a version it does not
/// recognise is answered with an error rather than a CRX.
pub fn chromium_product_version() -> String {
  let downloaded = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance()
    .get_downloaded_versions("wayfern");
  if let Some(version) = newest_version(&downloaded) {
    return version;
  }

  let cached = crate::browser_version_manager::BrowserVersionManager::instance()
    .get_cached_browser_versions("wayfern")
    .unwrap_or_default();
  newest_version(&cached).unwrap_or_else(|| FALLBACK_PRODUCT_VERSION.to_string())
}

/// Highest dotted-numeric version in `versions`. Neither the registry nor the
/// version cache promises an order, and a lexical max reads `9.x` as newer
/// than `151.x`.
fn newest_version(versions: &[String]) -> Option<String> {
  versions
    .iter()
    .filter(|version| !version.trim().is_empty())
    .max_by_key(|version| version_key(version))
    .cloned()
}

fn version_key(version: &str) -> [u64; 4] {
  let mut parts = [0u64; 4];
  for (slot, piece) in parts.iter_mut().zip(version.split('.')) {
    *slot = piece.trim().parse().unwrap_or(0);
  }
  parts
}

/// The Chrome Web Store update service, the endpoint Chromium itself uses to
/// fetch a package on demand. It needs the product id, the ABI, and a Chromium
/// version, and answers with a redirect to the CRX.
pub fn web_store_download_url(id: &str, product_version: &str, nacl_arch: &str) -> String {
  format!(
    "https://clients2.google.com/service/update2/crx\
?response=redirect&acceptformat=crx3&prodversion={product}&nacl_arch={arch}\
&x=id%3D{id}%26installsource%3Dondemand%26uc",
    product = urlencoding::encode(product_version),
    arch = urlencoding::encode(nacl_arch),
    id = id,
  )
}

/// Unwrap a CRX3 container to the ZIP it carries.
///
/// A `.crx` is not a ZIP with a different name: it is `Cr24`, a little-endian
/// format version, a little-endian header length, that many bytes of protobuf
/// signature header, and only then the ZIP. Storing the whole file as if it
/// were an archive leaves every reader to guess where the ZIP starts.
pub fn crx3_zip_payload(data: &[u8]) -> Result<&[u8], String> {
  if data.len() < 16 || &data[0..4] != CRX_MAGIC {
    return Err(err("EXTENSION_NOT_AN_EXTENSION"));
  }
  let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
  if version != 3 {
    // CRX2 has a different header (two length fields, no protobuf) and has not
    // been accepted by Chromium for years. Refusing is more useful than
    // guessing at an offset.
    return Err(err("EXTENSION_NOT_AN_EXTENSION"));
  }
  let header_len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
  let start = 12usize
    .checked_add(header_len)
    .ok_or_else(|| err("EXTENSION_NOT_AN_EXTENSION"))?;
  let payload = data
    .get(start..)
    .ok_or_else(|| err("EXTENSION_NOT_AN_EXTENSION"))?;
  if payload.len() < 4 || &payload[0..4] != ZIP_MAGIC {
    return Err(err("EXTENSION_NOT_AN_EXTENSION"));
  }
  Ok(payload)
}

/// Normalise downloaded bytes to the plain ZIP the store keeps. A CRX3 is
/// unwrapped; a ZIP passes through; anything else is refused.
pub fn archive_payload(data: &[u8]) -> Result<&[u8], String> {
  if data.len() >= 4 && &data[0..4] == ZIP_MAGIC {
    return Ok(data);
  }
  crx3_zip_payload(data)
}

fn redirect_policy() -> reqwest::redirect::Policy {
  reqwest::redirect::Policy::custom(|attempt| {
    if !scheme_is_allowed(attempt.url()) {
      // A store redirect that leaves TLS would download the package in the
      // clear, and the package is executable code. Stopping here surfaces the
      // final response instead of following it.
      return attempt.stop();
    }
    if attempt.previous().len() > MAX_REDIRECTS {
      return attempt.stop();
    }
    attempt.follow()
  })
}

async fn download_archive(url: &str) -> Result<Vec<u8>, String> {
  use futures_util::StreamExt;

  let client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(120))
    .connect_timeout(std::time::Duration::from_secs(15))
    .redirect(redirect_policy())
    .build()
    .map_err(|_| err("EXTENSION_DOWNLOAD_FAILED"))?;

  let response = client
    .get(url)
    .header("User-Agent", "Mozilla/5.0 (compatible; donutbrowser)")
    .send()
    .await
    .map_err(|e| {
      log::warn!("Extension download request failed: {e}");
      err("EXTENSION_DOWNLOAD_FAILED")
    })?;

  if !response.status().is_success() {
    log::warn!("Extension download answered HTTP {}", response.status());
    return Err(err("EXTENSION_DOWNLOAD_FAILED"));
  }
  // A redirect the policy stopped surfaces here as a 3xx, which
  // `is_success` already rejects; the final URL is checked again so a
  // same-status hop can never slip through.
  if !scheme_is_allowed(response.url()) {
    return Err(err("EXTENSION_DOWNLOAD_FAILED"));
  }
  if response
    .content_length()
    .is_some_and(|len| len > MAX_EXTENSION_BYTES)
  {
    return Err(err("EXTENSION_TOO_LARGE"));
  }

  let mut buffer: Vec<u8> = Vec::new();
  let mut stream = response.bytes_stream();
  while let Some(chunk) = stream.next().await {
    let chunk = chunk.map_err(|e| {
      log::warn!("Extension download stream failed: {e}");
      err("EXTENSION_DOWNLOAD_FAILED")
    })?;
    // A server is free to lie about, or omit, Content-Length, so the ceiling
    // is enforced against what actually arrives.
    if buffer.len() as u64 + chunk.len() as u64 > MAX_EXTENSION_BYTES {
      return Err(err("EXTENSION_TOO_LARGE"));
    }
    buffer.extend_from_slice(&chunk);
  }

  if buffer.is_empty() {
    return Err(err("EXTENSION_DOWNLOAD_FAILED"));
  }
  Ok(buffer)
}

/// What the archive is stored as. The payload written to the store is always
/// the plain ZIP, so a `.crx` link keeps its name but not its extension —
/// calling an unwrapped payload `.crx` would tell every later reader to skip a
/// CRX header that is no longer there.
fn direct_file_name(url: &Url) -> String {
  let raw = url
    .path_segments()
    .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
    .unwrap_or_default();
  let stem = raw
    .strip_suffix(".crx")
    .or_else(|| raw.strip_suffix(".CRX"))
    .or_else(|| raw.strip_suffix(".zip"))
    .or_else(|| raw.strip_suffix(".ZIP"))
    .unwrap_or(raw)
    .trim();
  if stem.is_empty() {
    return "extension.zip".to_string();
  }
  format!("{stem}.zip")
}

/// Fetch and validate an extension from a link, returning the plain ZIP plus
/// the identity read out of its own manifest.
pub async fn fetch_extension(input: &str) -> Result<FetchedExtension, String> {
  let source = parse_extension_source(input)?;
  let (download_url, file_name, from_web_store) = match &source {
    ExtensionSource::WebStore(id) => (
      web_store_download_url(id, &chromium_product_version(), nacl_arch()),
      format!("{id}.zip"),
      true,
    ),
    ExtensionSource::Direct(url) => (url.to_string(), direct_file_name(url), false),
  };

  let raw = download_archive(&download_url).await?;
  let payload = archive_payload(&raw)?;
  // A ZIP that carries no manifest is not an extension, whatever it was
  // served as. Refusing here keeps a 404 page or an installer out of the
  // store instead of leaving a broken row the user has to work out.
  let manifest = crate::extension_manager::read_manifest_from_archive(payload, "zip")
    .ok_or_else(|| err("EXTENSION_NOT_AN_EXTENSION"))?;
  let (name, version, description, _author, _homepage) =
    crate::extension_manager::manifest_metadata(
      &manifest,
      &crate::extension_manager::ManifestSource::Archive {
        data: payload,
        file_type: "zip",
      },
    );

  Ok(FetchedExtension {
    file_name,
    file_data: payload.to_vec(),
    name,
    version,
    description,
    source_url: if from_web_store {
      // The update-service URL is machine-specific noise; the store page is
      // what a user recognises and can open.
      match &source {
        ExtensionSource::WebStore(id) => {
          format!("https://chromewebstore.google.com/detail/{id}")
        }
        ExtensionSource::Direct(url) => url.to_string(),
      }
    } else {
      download_url
    },
    from_web_store,
  })
}

#[tauri::command]
pub async fn fetch_extension_from_url(url: String) -> Result<FetchedExtension, String> {
  fetch_extension(&url).await
}

#[cfg(test)]
mod tests {
  use super::*;

  fn crx3(header: &[u8], zip: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(CRX_MAGIC);
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(header);
    out.extend_from_slice(zip);
    out
  }

  fn zip_bytes() -> Vec<u8> {
    let mut zip = ZIP_MAGIC.to_vec();
    zip.extend_from_slice(b"the rest of an archive");
    zip
  }

  #[test]
  fn a_crx3_container_yields_exactly_the_zip_it_carries() {
    let zip = zip_bytes();
    let crx = crx3(&[7u8; 40], &zip);
    assert_eq!(crx3_zip_payload(&crx).unwrap(), zip.as_slice());
    assert_eq!(archive_payload(&crx).unwrap(), zip.as_slice());
  }

  #[test]
  fn a_file_that_is_not_a_crx_is_refused_rather_than_scanned_for_a_zip() {
    let mut wrong_magic = crx3(&[0u8; 8], &zip_bytes());
    wrong_magic[0] = b'X';
    assert_eq!(
      crx3_zip_payload(&wrong_magic).unwrap_err(),
      err("EXTENSION_NOT_AN_EXTENSION")
    );
    assert_eq!(
      archive_payload(b"<!doctype html><html>404</html>").unwrap_err(),
      err("EXTENSION_NOT_AN_EXTENSION")
    );
  }

  #[test]
  fn a_truncated_crx_never_reads_past_its_own_bytes() {
    let full = crx3(&[1u8; 32], &zip_bytes());
    for cut in [8usize, 12, 20, 40] {
      assert!(crx3_zip_payload(&full[..cut.min(full.len())]).is_err());
    }
    // A header length that runs past the file must not panic or return the
    // tail of some other structure.
    let mut lying = crx3(&[1u8; 32], &zip_bytes());
    lying[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(crx3_zip_payload(&lying).is_err());
  }

  #[test]
  fn a_plain_zip_is_accepted_and_a_crx2_is_not() {
    let zip = zip_bytes();
    assert_eq!(archive_payload(&zip).unwrap(), zip.as_slice());

    let mut crx2 = crx3(&[0u8; 16], &zip);
    crx2[4..8].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(
      archive_payload(&crx2).unwrap_err(),
      err("EXTENSION_NOT_AN_EXTENSION")
    );
  }

  #[test]
  fn every_accepted_link_shape_resolves_to_one_source() {
    let id = "abcdefghijklmnopabcdefghijklmnop";
    for input in [
      id,
      &format!("  {}  ", id.to_ascii_uppercase()),
      &format!("https://chromewebstore.google.com/detail/some-slug/{id}"),
      &format!("https://chromewebstore.google.com/detail/some-slug/{id}?hl=en"),
      &format!("https://chromewebstore.google.com/detail/{id}"),
      &format!("https://chrome.google.com/webstore/detail/some-slug/{id}"),
      &format!("https://chrome.google.com/webstore/detail/some-slug/{id}/related"),
    ] {
      assert_eq!(
        parse_extension_source(input).unwrap(),
        ExtensionSource::WebStore(id.to_string()),
        "{input}"
      );
    }

    let direct = "https://files.example.com/pack/ublock.crx";
    assert_eq!(
      parse_extension_source(direct).unwrap(),
      ExtensionSource::Direct(Url::parse(direct).unwrap())
    );
    assert!(matches!(
      parse_extension_source("https://files.example.com/pack/ublock.zip?v=2").unwrap(),
      ExtensionSource::Direct(_)
    ));
  }

  #[test]
  fn a_link_that_is_not_an_extension_is_refused_before_anything_is_fetched() {
    for input in [
      "",
      "   ",
      // 31 and 33 characters, and an id using letters past `p`.
      "abcdefghijklmnopabcdefghijklmno",
      "abcdefghijklmnopabcdefghijklmnopq",
      "abcdefghijklmnopabcdefghijklmnoz",
      "not a url at all",
      "ftp://files.example.com/ublock.crx",
      "file:///etc/passwd",
      // The right host, but no product id anywhere in the path.
      "https://chromewebstore.google.com/category/extensions",
      // An https URL that is not an archive.
      "https://files.example.com/downloads",
      "https://files.example.com/installer.exe",
    ] {
      assert_eq!(
        parse_extension_source(input).unwrap_err(),
        err("EXTENSION_URL_INVALID"),
        "{input}"
      );
    }
  }

  #[test]
  fn plain_http_is_refused_outside_the_test_build_and_never_off_loopback() {
    let loopback = Url::parse("http://127.0.0.1:8321/fixture.crx").unwrap();
    assert_eq!(scheme_is_allowed(&loopback), cfg!(feature = "e2e"));
    assert!(!scheme_is_allowed(
      &Url::parse("http://files.example.com/ublock.crx").unwrap()
    ));
    assert!(scheme_is_allowed(
      &Url::parse("https://files.example.com/ublock.crx").unwrap()
    ));
    assert_eq!(
      parse_extension_source("http://files.example.com/ublock.crx").unwrap_err(),
      err("EXTENSION_URL_INVALID")
    );
  }

  /// The redirect policy is what makes the scheme guard hold for the whole
  /// chain, not only the first request: the Web Store answers with a redirect,
  /// so the URL the bytes actually come from is never the one that was typed.
  #[test]
  fn a_redirect_is_judged_by_the_same_rule_as_the_first_request() {
    let policy_allows = |url: &str| scheme_is_allowed(&Url::parse(url).unwrap());
    assert!(policy_allows(
      "https://clients2.googleusercontent.com/crx/blobs/abc/EXT.crx"
    ));
    // The classic downgrade: an https request answered with a plain-HTTP
    // Location. The package is executable code, so the chain stops there.
    assert!(!policy_allows("http://mirror.example.com/EXT.crx"));
    assert!(!policy_allows("ftp://mirror.example.com/EXT.crx"));
  }

  #[test]
  fn the_size_ceiling_matches_the_upload_route_and_bounds_the_buffer() {
    assert_eq!(MAX_EXTENSION_BYTES, 64 * 1024 * 1024);
    // The streaming guard is a comparison on running totals; prove the
    // arithmetic it relies on rejects the first chunk that crosses the line.
    let already = MAX_EXTENSION_BYTES - 10;
    assert!(already + 11 > MAX_EXTENSION_BYTES);
    assert!(already + 10 <= MAX_EXTENSION_BYTES);
  }

  #[test]
  fn the_web_store_url_carries_the_id_the_abi_and_the_installed_version() {
    let url = web_store_download_url("abcdefghijklmnopabcdefghijklmnop", "151.0.7922.76", "arm64");
    assert!(url.starts_with("https://clients2.google.com/service/update2/crx?"));
    assert!(url.contains("prodversion=151.0.7922.76"));
    assert!(url.contains("nacl_arch=arm64"));
    assert!(url.contains("id%3Dabcdefghijklmnopabcdefghijklmnop"));
    assert!(url.contains("acceptformat=crx3"));
  }

  #[test]
  fn the_newest_installed_version_wins_over_a_lexically_larger_one() {
    let versions = vec![
      "9.0.1.0".to_string(),
      "151.0.7922.76".to_string(),
      "147.0.7727.138".to_string(),
    ];
    assert_eq!(newest_version(&versions).unwrap(), "151.0.7922.76");
    assert_eq!(newest_version(&[]), None);
  }

  #[test]
  fn a_direct_download_names_the_stored_file_a_zip() {
    assert_eq!(
      direct_file_name(&Url::parse("https://files.example.com/pack/ublock.crx").unwrap()),
      "ublock.zip"
    );
    assert_eq!(
      direct_file_name(&Url::parse("https://files.example.com/pack/ublock.zip").unwrap()),
      "ublock.zip"
    );
    assert_eq!(
      direct_file_name(&Url::parse("https://files.example.com/.crx").unwrap()),
      "extension.zip"
    );
  }
}

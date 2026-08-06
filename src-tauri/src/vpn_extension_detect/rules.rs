//! Pure classification rules for VPN/proxy extension detection.
//!
//! Deliberately free of crate-internal dependencies (`std` + `serde_json`
//! only): these rules are the heart of the feature and the part most worth
//! testing in isolation, so nothing here may reach for app state, the
//! filesystem, or the network. Enumerating the two extension sources and
//! reading them off disk lives in the parent module.

use serde::{Deserialize, Serialize};

/// Substrings that corroborate a request-blocking extension being a VPN.
/// Matched case-insensitively against name + description.
const KEYWORDS: &[&str] = &[
  "vpn",
  "proxy",
  "tunnel",
  "unblock",
  "wireguard",
  "shadowsocks",
  "socks",
];

/// Matched as a whole token rather than a substring — too short to be safe
/// inside other words ("warped", "warpaint").
const TOKEN_KEYWORDS: &[&str] = &["warp"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedVpnExtension {
  /// Stable acknowledgement identity: `donut:<uuid>` or `crx:<32-char-id>`.
  pub key: String,
  pub name: String,
  pub version: Option<String>,
  /// `"donut"` (managed by Donut) or `"browser"` (installed inside the profile).
  pub source: String,
  /// `"confirmed"` or `"likely"`.
  pub confidence: String,
  /// Why it matched, for the dialog's detail line.
  pub signals: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestSignals {
  pub proxy_permission: bool,
  pub optional_proxy_permission: bool,
  pub declarative_net_request: bool,
  pub web_request_blocking: bool,
  pub broad_host_permissions: bool,
}

fn string_list<'a>(manifest: &'a serde_json::Value, key: &str) -> Vec<&'a str> {
  manifest
    .get(key)
    .and_then(|v| v.as_array())
    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
    .unwrap_or_default()
}

fn is_broad_host(pattern: &str) -> bool {
  matches!(pattern, "<all_urls>" | "*://*/*")
}

pub fn signals_from_manifest(manifest: &serde_json::Value) -> ManifestSignals {
  let permissions = string_list(manifest, "permissions");
  let optional_permissions = string_list(manifest, "optional_permissions");
  let host_permissions = string_list(manifest, "host_permissions");
  let optional_host_permissions = string_list(manifest, "optional_host_permissions");

  let has = |list: &[&str], name: &str| list.contains(&name);

  // MV2 keeps host patterns inside `permissions`; MV3 splits them into
  // `host_permissions`. Look in both so one manifest version isn't silently
  // under-detected.
  let all_hosts: Vec<&str> = permissions
    .iter()
    .chain(host_permissions.iter())
    .chain(optional_host_permissions.iter())
    .copied()
    .collect();
  let broad = all_hosts.iter().any(|p| is_broad_host(p))
    || (all_hosts.contains(&"http://*/*") && all_hosts.contains(&"https://*/*"));

  ManifestSignals {
    proxy_permission: has(&permissions, "proxy"),
    optional_proxy_permission: has(&optional_permissions, "proxy"),
    declarative_net_request: has(&permissions, "declarativeNetRequest")
      || has(&permissions, "declarativeNetRequestWithHostAccess"),
    web_request_blocking: has(&permissions, "webRequest")
      && has(&permissions, "webRequestBlocking"),
    broad_host_permissions: broad,
  }
}

pub fn keyword_hit(name: &str, description: Option<&str>) -> bool {
  let mut haystack = name.to_lowercase();
  if let Some(d) = description {
    haystack.push(' ');
    haystack.push_str(&d.to_lowercase());
  }
  if KEYWORDS.iter().any(|k| haystack.contains(k)) {
    return true;
  }
  haystack
    .split(|c: char| !c.is_alphanumeric())
    .any(|token| TOKEN_KEYWORDS.contains(&token))
}

/// Classify an extension from its manifest signals.
///
/// The `proxy` permission is the only signal that *proves* the capability: it
/// is what Chromium requires to call `chrome.proxy`, and it stays in
/// `permissions` under both manifest versions because it is an API permission,
/// not a host pattern.
///
/// The request-blocking tier additionally requires a keyword, and that
/// corroboration is not optional: `declarativeNetRequest` plus `<all_urls>`
/// describes every content blocker in the ecosystem, so without it the warning
/// fires on uBlock Origin — which would teach users to dismiss the dialog on
/// sight, destroying the value of the mismatch block that shares it.
pub fn classify(signals: &ManifestSignals, keyword: bool) -> Option<&'static str> {
  if signals.proxy_permission {
    return Some("confirmed");
  }
  if signals.optional_proxy_permission {
    return Some("likely");
  }
  if (signals.declarative_net_request || signals.web_request_blocking)
    && signals.broad_host_permissions
    && keyword
  {
    return Some("likely");
  }
  None
}

pub fn signal_labels(signals: &ManifestSignals, keyword: bool) -> Vec<String> {
  let mut out = Vec::new();
  if signals.proxy_permission {
    out.push("permissions:proxy".to_string());
  }
  if signals.optional_proxy_permission {
    out.push("optionalPermissions:proxy".to_string());
  }
  if signals.declarative_net_request {
    out.push("declarativeNetRequest".to_string());
  }
  if signals.web_request_blocking {
    out.push("webRequestBlocking".to_string());
  }
  if signals.broad_host_permissions {
    out.push("broadHostPermissions".to_string());
  }
  if keyword {
    out.push("keyword".to_string());
  }
  out
}

/// `__MSG_someKey__` -> `someKey`.
pub fn message_placeholder_key(value: &str) -> Option<String> {
  value
    .strip_prefix("__MSG_")
    .and_then(|rest| rest.strip_suffix("__"))
    .map(str::to_string)
}

/// Chromium's `messages.json` shape: `{ "key": { "message": "..." } }`, with
/// keys compared case-insensitively.
pub fn lookup_message(messages: &serde_json::Value, key: &str) -> Option<String> {
  let obj = messages.as_object()?;
  obj
    .iter()
    .find(|(k, _)| k.eq_ignore_ascii_case(key))
    .and_then(|(_, v)| v.get("message"))
    .and_then(|v| v.as_str())
    .map(str::to_string)
}

pub fn manifest_str(manifest: &serde_json::Value, key: &str) -> Option<String> {
  manifest
    .get(key)
    .and_then(|v| v.as_str())
    .map(str::to_string)
}

/// Sort key for an extension version directory (`1.10.0_0`), compared
/// numerically so `1.10.0` sorts above `1.9.0` where a lexicographic compare
/// would put it below.
pub fn version_dir_sort_key(name: &str) -> Vec<u64> {
  name
    .split(['.', '_'])
    .map(|part| part.parse::<u64>().unwrap_or(0))
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn signals_of(manifest: serde_json::Value) -> ManifestSignals {
    signals_from_manifest(&manifest)
  }

  #[test]
  fn classify_confirms_on_proxy_permission() {
    let s = signals_of(json!({ "permissions": ["proxy", "storage"] }));
    assert!(s.proxy_permission);
    assert_eq!(classify(&s, false), Some("confirmed"));
  }

  #[test]
  fn classify_confirms_proxy_permission_in_mv2() {
    // `proxy` is an API permission, so MV3's host_permissions split does not
    // move it — the same key works for both manifest versions.
    let s = signals_of(json!({
      "manifest_version": 2,
      "permissions": ["proxy", "<all_urls>", "webRequest"]
    }));
    assert_eq!(classify(&s, false), Some("confirmed"));
  }

  #[test]
  fn classify_likely_on_optional_proxy() {
    let s = signals_of(json!({ "optional_permissions": ["proxy"] }));
    assert_eq!(classify(&s, false), Some("likely"));
  }

  #[test]
  fn classify_ignores_content_blocker() {
    // The regression guard: a content blocker declares exactly these and is
    // not a VPN. Firing here would train users to dismiss the dialog.
    let s = signals_of(json!({
      "permissions": ["declarativeNetRequest"],
      "host_permissions": ["<all_urls>"]
    }));
    assert!(s.declarative_net_request && s.broad_host_permissions);
    assert_eq!(classify(&s, keyword_hit("uBlock Origin", None)), None);
  }

  #[test]
  fn classify_likely_on_dnr_plus_keyword() {
    let s = signals_of(json!({
      "permissions": ["declarativeNetRequest"],
      "host_permissions": ["<all_urls>"]
    }));
    assert_eq!(
      classify(&s, keyword_hit("Free VPN Proxy", None)),
      Some("likely")
    );
  }

  #[test]
  fn classify_ignores_keyword_only() {
    // A name alone proves nothing; without a capability signal this is noise.
    let s = signals_of(json!({ "permissions": ["storage"] }));
    assert_eq!(classify(&s, keyword_hit("VPN Deals Finder", None)), None);
  }

  #[test]
  fn classify_requires_broad_hosts_for_the_blocking_tier() {
    let s = signals_of(json!({
      "permissions": ["declarativeNetRequest"],
      "host_permissions": ["https://example.com/*"]
    }));
    assert_eq!(classify(&s, keyword_hit("Some VPN", None)), None);
  }

  #[test]
  fn broad_hosts_detected_from_split_http_and_https() {
    let s = signals_of(json!({
      "permissions": ["webRequest", "webRequestBlocking"],
      "host_permissions": ["http://*/*", "https://*/*"]
    }));
    assert!(s.broad_host_permissions);
    assert!(s.web_request_blocking);
  }

  #[test]
  fn broad_hosts_detected_from_mv2_permissions_array() {
    // MV2 puts host patterns in `permissions`; the split-out key is absent.
    let s = signals_of(json!({
      "manifest_version": 2,
      "permissions": ["webRequest", "webRequestBlocking", "<all_urls>"]
    }));
    assert!(s.broad_host_permissions);
    assert_eq!(classify(&s, keyword_hit("Turbo VPN", None)), Some("likely"));
  }

  #[test]
  fn keyword_matching_is_substring_but_token_bound_for_short_terms() {
    assert!(keyword_hit("TouchVPN", None));
    assert!(keyword_hit("Unblock Sites", None));
    assert!(keyword_hit("Cloudflare WARP", None));
    // "warp" only matches as a whole token, so this must not hit.
    assert!(!keyword_hit("Time Warped Clock", None));
    assert!(keyword_hit(
      "Anything",
      Some("a fast tunnel for your browser")
    ));
  }

  #[test]
  fn message_placeholder_round_trip() {
    assert_eq!(
      message_placeholder_key("__MSG_appName__").as_deref(),
      Some("appName")
    );
    assert_eq!(message_placeholder_key("Plain Name"), None);
    let messages = json!({ "appName": { "message": "Nord VPN" } });
    assert_eq!(
      lookup_message(&messages, "appName").as_deref(),
      Some("Nord VPN")
    );
    // Chromium compares message keys case-insensitively.
    assert_eq!(
      lookup_message(&messages, "APPNAME").as_deref(),
      Some("Nord VPN")
    );
  }

  #[test]
  fn version_dirs_sort_numerically_not_lexicographically() {
    let mut dirs = ["1.9.0_0", "1.10.0_0", "1.2.0_0"];
    dirs.sort_by_key(|d| version_dir_sort_key(d));
    assert_eq!(dirs.last(), Some(&"1.10.0_0"));
  }

  #[test]
  fn malformed_manifest_yields_no_signals() {
    // Arrays of non-strings, wrong types, and missing keys must not panic.
    let s = signals_of(json!({ "permissions": [1, 2, {"a": "b"}], "host_permissions": "nope" }));
    assert_eq!(s, ManifestSignals::default());
    assert_eq!(classify(&s, true), None);
  }
}

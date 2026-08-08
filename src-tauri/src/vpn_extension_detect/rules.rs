//! Pure classification rules for VPN/proxy extension detection.
//!
//! Deliberately free of crate-internal dependencies (`std` + `serde_json`
//! only): these rules are the heart of the feature and the part most worth
//! testing in isolation, so nothing here may reach for app state, the
//! filesystem, or the network. Enumerating the two extension sources and
//! reading them off disk lives in the parent module.

use serde::{Deserialize, Serialize};

/// Chrome Web Store ids of extensions whose whole purpose is routing the
/// browser somewhere else. Sorted, so membership is a binary search.
///
/// This list is what lets a VPN with an unrevealing name — "Hotspot Shield"
/// says nothing about what it does — be named as one instead of appearing as
/// an anonymous holder of the proxy permission. Every id was verified by
/// downloading the extension and reading its manifest; a wrong id is worse
/// than a missing one, because a stale list only ever loses recall while a
/// wrong one accuses the wrong extension.
const KNOWN_VPN_EXTENSION_IDS: &[&str] = &[
  "adlpodnneegcnbophopdmhedicjbcgco", // Troywell VPN
  "ailoabdmgclmfmhdagmlohpjlbpffblp", // Surfshark
  "akcocjjpkmlniicdeemdceeajlmoabhg", // 1VPN
  "apbcbecdpjefgklcokinpapmmdekecah", // Ninja VPN
  "bihmplhobchoageeokmgbdihknkjbknd", // Touch VPN (delisted 2025, still installed in old profiles)
  "blapeiihifiknfmceddkceklnpopgclm", // Proxy Switcher Pro
  "bnlofglpdlboacepdieejiecfbfpmhlb", // Turbo VPN
  "dookpfaalaaappcdneeahomimbllocnb", // FoxyProxy Basic
  "eppiocemhmnlbhjplcgkofciiegomcon", // Urban VPN
  "fcfhplploccackoneaefokcmbjfbkenj", // 1clickVPN
  "fdcgdnkidjaadafnichfpabhfomcebme", // ZenMate (delisted 2025)
  "ffbkglfijbcbgblgflchnbphjdllaogb", // CyberGhost
  "fgddmllnllkalaagkghckoinaemmogpe", // ExpressVPN
  "fjoaledfpmneenckfbpdfhkmimnjocfa", // NordVPN
  "gcknhkkoolaabfmlnjonogaaifnjlfnp", // FoxyProxy
  "gdpehpfhegefkjelaifkdbppjbhilaom", // Proxy-Cheap Proxy Manager
  "gjakohbhfclfjmhhlenfdkldieofkpjl", // IPRoyal Proxy Manager
  "gjknjjomckknofjidppipffbpoekiipm", // Betternet
  "gkojfkhlekighikafcpjkiklfbnlmeio", // Hola VPN
  "hnmpcagpplmpfojmgmnngilcnanddlhb", // Windscribe
  "jaoafpkngncfpfggjefnekilbkcpjdgp", // uVPN
  "jedieiamjmoflcknjdjhpieklepfglin", // FastestVPN
  "jpadbaildllggkcgibilkeacpcodailn", // Planet VPN lite
  "jplgfhpmjnbigmhklmmbgecoobifkmpa", // Proton VPN
  "jplnlifepflhkbkgonidnobkakhmpnmh", // Private Internet Access
  "kgepmkaldicdcljckhamnhkigddnbcbd", // PACify Proxy Manager
  "kpiecbcckbofpmkkkdibbllpinceiihk", // DotVPN
  "majdfhpaihoncoakbjgbdhglocklcgno", // VeePN
  "nbcojefnccbanplpoffopkoepjmhgdgh", // Hoxx VPN
  "nlbejmccbhkncgokjcmghpfloaajcffj", // Hotspot Shield
  "ohjocgmpmlfahafbipehkhbaacoemojp", // hide.me Proxy
  "omdakjcmkglenbhjadbccaookpfjihpa", // TunnelBear
  "omghfjlpggmjjaagoclmmobgdodcjboh", // Browsec
  "onnfghpihccifgojkpnnncpagjcdbjod", // Proxy Switcher and Manager
  "oofgbpoabipfcfjapgnbbjjaenockbdp", // SetupVPN
  "padekgcemlokbadohgkifijomclgjgif", // Proxy SwitchyOmega
  "pphgdbgldlmicfdkhondlafkiomnelnk", // 1ClickVPN Proxy
];

/// Terms specific enough to name a VPN wherever they appear, including in a
/// 132-character manifest description.
const STRONG_KEYWORDS: &[&str] = &["vpn", "wireguard", "shadowsocks", "openvpn"];

/// Terms that only mean "VPN" in a product's *name*. In a description they are
/// ordinary English — "no proxy setup required", "carpal tunnel", "unblock
/// right click" — and matching them there is where the noise comes from.
const NAME_ONLY_KEYWORDS: &[&str] = &["proxy", "unblock"];

/// Matched as whole tokens rather than substrings, and in the name only. Too
/// short to be safe inside other words ("tussocks", "tunnelling").
const NAME_TOKEN_KEYWORDS: &[&str] = &["socks", "socks5", "tunnel"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedVpnExtension {
  /// Stable acknowledgement identity: `donut:<uuid>` or `crx:<32-char-id>`.
  pub key: String,
  pub name: String,
  pub version: Option<String>,
  /// `"donut"` (managed by Donut) or `"browser"` (installed inside the profile).
  pub source: String,
  /// `"confirmed"` and `"likely"` are claims that this IS a VPN/proxy tool.
  /// `"capability"` claims only that it *could* change the proxy.
  pub confidence: String,
  /// Whether the manifest holds Chromium's `proxy` permission outright, so the
  /// extension can call `chrome.proxy.settings.set` without asking again.
  /// Separate from `confidence`: a download manager reading the browser's
  /// proxy declares the identical permission as a VPN hijacking it.
  pub proxy_control: bool,
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

/// True when this is the id of an extension known to route browser traffic.
pub fn is_known_vpn_extension(extension_id: &str) -> bool {
  KNOWN_VPN_EXTENSION_IDS.binary_search(&extension_id).is_ok()
}

fn has_token(haystack: &str, tokens: &[&str]) -> bool {
  haystack
    .split(|c: char| !c.is_alphanumeric())
    .any(|token| tokens.contains(&token))
}

/// Does the extension describe itself as a VPN or proxy tool?
///
/// The name is weighted far more heavily than the description, because that is
/// where the evidence actually lives: a VPN vendor puts "VPN" in the name — it
/// is how the store surfaces them — while a description is 132 characters of
/// ordinary prose in which "proxy", "tunnel" and "unblock" are all innocent.
/// Matching those three against descriptions is what flags carpal-tunnel
/// reminders, right-click unblockers, and tools whose pitch is that they need
/// *no* proxy setup.
pub fn vpn_keyword_hit(name: &str, description: Option<&str>) -> bool {
  let name = name.to_lowercase();
  if STRONG_KEYWORDS.iter().any(|k| name.contains(k))
    || NAME_ONLY_KEYWORDS.iter().any(|k| name.contains(k))
    || has_token(&name, NAME_TOKEN_KEYWORDS)
  {
    return true;
  }
  description
    .map(str::to_lowercase)
    .is_some_and(|d| STRONG_KEYWORDS.iter().any(|k| d.contains(k)))
}

/// Classify an extension from its id, manifest signals and self-description.
///
/// Two different questions are answered here, and fusing them is what made an
/// ordinary download manager get reported as a VPN. Chromium has no read-only
/// variant of the `proxy` permission: `chrome.proxy.settings.get()` and
/// `.set()` sit behind the same manifest string, so an extension replicating
/// the browser's proxy for its own transfers declares exactly what a VPN
/// hijacking it declares. The permission therefore proves a *capability* and
/// nothing more; naming something a VPN needs separate evidence — a known id,
/// or the extension saying so itself.
///
/// The request-blocking tier's keyword requirement is not optional either:
/// `declarativeNetRequest` plus `<all_urls>` describes every content blocker in
/// the ecosystem, so without it the warning fires on uBlock Origin — which
/// would teach users to dismiss the dialog on sight, destroying the value of
/// the mismatch block that shares it.
///
/// An `optional_permissions` entry the user has never granted is deliberately
/// not a capability at all: the extension cannot call `chrome.proxy` until it
/// asks and is allowed.
pub fn classify(
  extension_id: Option<&str>,
  signals: &ManifestSignals,
  keyword: bool,
) -> Option<&'static str> {
  if extension_id.is_some_and(is_known_vpn_extension) {
    return Some("confirmed");
  }
  if keyword {
    if signals.proxy_permission {
      return Some("confirmed");
    }
    if signals.optional_proxy_permission
      || ((signals.declarative_net_request || signals.web_request_blocking)
        && signals.broad_host_permissions)
    {
      return Some("likely");
    }
  }
  if signals.proxy_permission {
    return Some("capability");
  }
  None
}

pub fn signal_labels(
  extension_id: Option<&str>,
  signals: &ManifestSignals,
  keyword: bool,
) -> Vec<String> {
  let mut out = Vec::new();
  if extension_id.is_some_and(is_known_vpn_extension) {
    out.push("knownVpnExtension".to_string());
  }
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

  fn classify_named(
    manifest: serde_json::Value,
    name: &str,
    description: Option<&str>,
  ) -> Option<&'static str> {
    let s = signals_of(manifest);
    classify(None, &s, vpn_keyword_hit(name, description))
  }

  #[test]
  fn classify_confirms_a_self_described_vpn_holding_the_proxy_permission() {
    let s = signals_of(json!({ "permissions": ["proxy", "storage"] }));
    assert!(s.proxy_permission);
    assert_eq!(
      classify(None, &s, vpn_keyword_hit("Turbo VPN", None)),
      Some("confirmed")
    );
  }

  #[test]
  fn classify_confirms_proxy_permission_in_mv2() {
    // `proxy` is an API permission, so MV3's host_permissions split does not
    // move it — the same key works for both manifest versions.
    let s = signals_of(json!({
      "manifest_version": 2,
      "permissions": ["proxy", "<all_urls>", "webRequest"]
    }));
    assert_eq!(
      classify(None, &s, vpn_keyword_hit("Hoxx VPN Proxy", None)),
      Some("confirmed")
    );
  }

  #[test]
  fn a_download_manager_is_reported_as_a_capability_never_as_a_vpn() {
    // The bug this whole split exists for. IDM Integration Module declares
    // `proxy` so the desktop binary can replicate the browser's route for a
    // handed-off download, and says nothing about VPNs anywhere. Verified
    // against the real published manifest.
    let verdict = classify_named(
      json!({
        "permissions": [
          "scripting", "tabs", "cookies", "contextMenus", "webNavigation",
          "webRequest", "declarativeNetRequest", "downloads", "downloads.shelf",
          "downloads.ui", "management", "storage", "proxy", "nativeMessaging"
        ]
      }),
      "IDM Integration Module",
      Some("Download files with Internet Download Manager"),
    );
    assert_eq!(verdict, Some("capability"));
  }

  #[test]
  fn a_known_vpn_is_confirmed_from_its_id_alone() {
    // Hotspot Shield's name contains no keyword at all, so without the id list
    // the biggest VPN in the store would be indistinguishable from a download
    // manager.
    let s = signals_of(json!({ "permissions": ["proxy"] }));
    let id = "nlbejmccbhkncgokjcmghpfloaajcffj";
    assert_eq!(
      classify(Some(id), &s, vpn_keyword_hit("Hotspot Shield", None)),
      Some("confirmed")
    );
    assert!(signal_labels(Some(id), &s, false).contains(&"knownVpnExtension".to_string()));
  }

  #[test]
  fn the_known_vpn_id_list_is_sorted_and_well_formed() {
    // Membership is a binary search, so an unsorted entry is silently missed.
    assert!(KNOWN_VPN_EXTENSION_IDS.windows(2).all(|w| w[0] < w[1]));
    for id in KNOWN_VPN_EXTENSION_IDS {
      assert_eq!(id.len(), 32, "{id} is not a Chrome extension id");
      assert!(
        id.bytes().all(|b| (b'a'..=b'p').contains(&b)),
        "{id} is not a Chrome extension id"
      );
    }
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
    assert_eq!(
      classify(None, &s, vpn_keyword_hit("uBlock Origin", None)),
      None
    );
  }

  #[test]
  fn classify_likely_on_dnr_plus_keyword() {
    let s = signals_of(json!({
      "permissions": ["declarativeNetRequest"],
      "host_permissions": ["<all_urls>"]
    }));
    assert_eq!(
      classify(None, &s, vpn_keyword_hit("Free VPN Proxy", None)),
      Some("likely")
    );
  }

  #[test]
  fn classify_likely_on_optional_proxy_plus_keyword() {
    // Optional and ungranted is not a capability, so it only matters when the
    // extension also says what it is.
    let s = signals_of(json!({ "optional_permissions": ["proxy"] }));
    assert_eq!(
      classify(None, &s, vpn_keyword_hit("Some VPN", None)),
      Some("likely")
    );
    assert_eq!(
      classify(None, &s, vpn_keyword_hit("Request Interceptor", None)),
      None
    );
  }

  #[test]
  fn classify_ignores_keyword_only() {
    // A name alone proves nothing; without a capability signal this is noise.
    let s = signals_of(json!({ "permissions": ["storage"] }));
    assert_eq!(
      classify(None, &s, vpn_keyword_hit("VPN Deals Finder", None)),
      None
    );
  }

  #[test]
  fn classify_requires_broad_hosts_for_the_blocking_tier() {
    let s = signals_of(json!({
      "permissions": ["declarativeNetRequest"],
      "host_permissions": ["https://example.com/*"]
    }));
    assert_eq!(classify(None, &s, vpn_keyword_hit("Some VPN", None)), None);
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
    assert_eq!(
      classify(None, &s, vpn_keyword_hit("Turbo VPN", None)),
      Some("likely")
    );
  }

  #[test]
  fn keyword_matching_reads_the_name_broadly_and_the_description_narrowly() {
    assert!(vpn_keyword_hit("TouchVPN", None));
    assert!(vpn_keyword_hit("Unblock Sites", None));
    assert!(vpn_keyword_hit("Shadowsocks Client", None));
    // Whole-token terms must not match inside longer words. "socks" in a name
    // is the protocol often enough to keep; "tussocks" and "tunnelling" are
    // exactly why it cannot be a substring.
    assert!(vpn_keyword_hit("SOCKS5 Configurator", None));
    assert!(!vpn_keyword_hit("Tussocks Field Guide", None));
    assert!(!vpn_keyword_hit("Tunnelling Contractors CRM", None));

    // A description says "VPN" only when it means one...
    assert!(vpn_keyword_hit(
      "Anything",
      Some("a free VPN for your browser")
    ));
    // ...but these three are ordinary English and must not promote anything.
    assert!(!vpn_keyword_hit(
      "Requestly",
      Some("Modify HTTP requests, no proxy setup required")
    ));
    assert!(!vpn_keyword_hit(
      "Stretch Reminder",
      Some("Avoid carpal tunnel syndrome while you work")
    ));
    assert!(!vpn_keyword_hit(
      "Absolute Right Click",
      Some("Unblock right click and text selection on any site")
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
    assert_eq!(classify(None, &s, true), None);
  }
}

//! Whether a profile's exit node can be reached from somewhere that is not this
//! machine.
//!
//! Remote execution — an interactive remote session or a Cookie Bot night — runs
//! the browser on a remote host, but the PROFILE (and its proxy, and its VPN
//! config) is pulled from the user's sync namespace. Addresses are not rewritten
//! in transit, so a proxy recorded as `127.0.0.1:8080` arrives meaning *that
//! machine's own loopback*.
//!
//! That is the whole bug this module exists to prevent. A profile with NO exit
//! is already refused (`proxy_required`), because a night browsed without the
//! user's own exit damages an identity rather than building it — but "an exit is
//! configured" and "that exit is reachable from somewhere else" are different
//! questions, and only the first was ever asked. A local proxy satisfied it and
//! failed the second, so the run was accepted, dispatched, and burned an hour
//! either erroring out or (worse) egressing from the remote host's own address:
//! exactly the outcome `proxy_required` exists to stop, reached by the one route
//! it did not check.
//!
//! Local proxies are not an exotic case. A local MITM proxy, an SSH tunnel, a
//! locally-run SOCKS client and Donut's own VLESS support all present to the
//! browser as `127.0.0.1:<port>`.
//!
//! This module is the single answer, shared by every caller, and it FAILS
//! CLOSED: anything it cannot parse is reported as unreachable. Refusing a
//! working setup costs the user one support question; accepting a broken one
//! costs an hour of quota and a damaged profile identity.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Whether a remote host could dial this profile's exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReachability {
  /// No proxy and no VPN. The caller's existing "no exit" refusal applies.
  None,
  /// An address a host elsewhere on the internet can reach.
  Remote,
  /// An address that only means anything on this machine or this LAN.
  LocalOnly {
    /// The offending host, for a message the user can act on.
    host: String,
    /// Which part of the config it came from: "proxy" or "VPN".
    source: &'static str,
  },
  /// A protocol a remote host has no way to speak, whatever address it names.
  ///
  /// Reachability is the wrong question for these: the server in a VLESS URI is
  /// as publicly routable as any other, so the host check passes and the run is
  /// accepted, dispatched, and then refused remotely, because dialling VLESS
  /// needs a sidecar that is not available there. That is a permanent refusal
  /// wearing a transient one's clothes, and every nightly retry pays for it.
  UnsupportedKind {
    /// The protocol, as the user would name it: "VLESS".
    kind: String,
    /// Which part of the config it came from: "proxy" or "VPN".
    source: &'static str,
  },
  /// Configured, but this code could not determine the host.
  ///
  /// Treated as unreachable by [`ExitReachability::is_remote`] — see the
  /// fail-closed note in the module docs.
  Unknown {
    reason: String,
    source: &'static str,
  },
}

impl ExitReachability {
  /// Whether remote execution may proceed.
  pub fn is_remote(&self) -> bool {
    matches!(self, ExitReachability::Remote)
  }

  /// A one-line reason for a refusal, or None when there is nothing to refuse.
  pub fn refusal_detail(&self) -> Option<String> {
    match self {
      ExitReachability::Remote | ExitReachability::None => None,
      ExitReachability::LocalOnly { host, source } => Some(format!(
        "The {source} for this profile points at {host}, which only exists on this computer. \
         Remote runs happen on our hosts and cannot reach it."
      )),
      ExitReachability::UnsupportedKind { kind, source } => Some(format!(
        "The {source} for this profile is {kind}, which needs the Xray sidecar our hosts do not \
         run, so the fleet cannot dial it however reachable its server is. An HTTP, HTTPS, SOCKS \
         or WireGuard exit works."
      )),
      ExitReachability::Unknown { reason, source } => Some(format!(
        "The {source} for this profile could not be read ({reason}), so we cannot confirm a \
         remote host could use it."
      )),
    }
  }
}

/// Whether a hostname or IP literal is reachable from another machine.
///
/// Rejects, in order: empty/whitespace, unparsable-as-either, and every IP
/// range that is scoped to a machine or a private network. Hostnames that are
/// not IP literals are accepted unless they use a name suffix that is
/// definitionally local — a public DNS name cannot be validated here without a
/// lookup, and doing a lookup would make this impure and slow on a hot path.
pub fn host_is_remote_reachable(host: &str) -> bool {
  let host = normalize_host(host);
  if host.is_empty() {
    return false;
  }

  if let Ok(ip) = host.parse::<IpAddr>() {
    return ip_is_remote_reachable(ip);
  }

  let lower = host.to_ascii_lowercase();

  // `localhost` and anything under it resolve to loopback everywhere.
  if lower == "localhost" || lower.ends_with(".localhost") {
    return false;
  }

  // Suffixes reserved for local/private name resolution (RFC 6762 mDNS, RFC
  // 8375, and the names router vendors hand out on a LAN). A remote host
  // resolving one of these gets its own network's answer, not the user's.
  const LOCAL_SUFFIXES: [&str; 7] = [
    ".local",
    ".localdomain",
    ".internal",
    ".home",
    ".home.arpa",
    ".lan",
    ".intranet",
  ];
  if LOCAL_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix)) {
    return false;
  }

  // A bare single-label name ("my-proxy", "router") is only resolvable through
  // a local search domain, so it is no more use to a remote host than `.local`.
  if !lower.contains('.') {
    return false;
  }

  true
}

/// Whether an IP literal is routable from another machine.
fn ip_is_remote_reachable(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => ipv4_is_remote_reachable(v4),
    IpAddr::V6(v6) => ipv6_is_remote_reachable(v6),
  }
}

fn ipv4_is_remote_reachable(ip: Ipv4Addr) -> bool {
  // `is_private`/`is_loopback`/`is_link_local` are stable; the rest are not, so
  // the remaining ranges are spelled out rather than gated behind a nightly
  // feature.
  if ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() {
    return false;
  }
  if ip.is_broadcast() || ip.is_multicast() || ip.is_documentation() {
    return false;
  }
  let [a, b, ..] = ip.octets();
  // 100.64.0.0/10 — carrier-grade NAT (RFC 6598). Reachable inside one
  // carrier's network and nowhere else.
  if a == 100 && (64..128).contains(&b) {
    return false;
  }
  // 0.0.0.0/8 "this network", and 240.0.0.0/4 reserved.
  if a == 0 || a >= 240 {
    return false;
  }
  true
}

fn ipv6_is_remote_reachable(ip: Ipv6Addr) -> bool {
  if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
    return false;
  }
  // An IPv4 address wearing an IPv6 hat is still that IPv4 address — classify
  // it as one, or `::ffff:127.0.0.1` walks straight through.
  if let Some(v4) = ip.to_ipv4_mapped() {
    return ipv4_is_remote_reachable(v4);
  }
  if let Some(v4) = ip.to_ipv4() {
    return ipv4_is_remote_reachable(v4);
  }
  let segments = ip.segments();
  // fc00::/7 unique-local, fe80::/10 link-local.
  if (segments[0] & 0xfe00) == 0xfc00 {
    return false;
  }
  if (segments[0] & 0xffc0) == 0xfe80 {
    return false;
  }
  true
}

/// Strip the decoration a host can arrive wrapped in: whitespace, `[...]`
/// around an IPv6 literal, a trailing dot on an FQDN, and any `user@` or
/// `:port` that came along from a URI.
fn normalize_host(raw: &str) -> String {
  let mut host = raw.trim();
  if host.is_empty() {
    return String::new();
  }

  // `user:pass@host` — take what follows the LAST '@', since a password may
  // itself contain one.
  if let Some(at) = host.rfind('@') {
    host = &host[at + 1..];
  }

  // Bracketed IPv6, optionally with a port: `[::1]:1080`.
  if let Some(stripped) = host.strip_prefix('[') {
    if let Some(end) = stripped.find(']') {
      return stripped[..end].trim().to_string();
    }
    return stripped.trim().to_string();
  }

  // `host:port`, but only when there is exactly one colon — more than one means
  // a bare IPv6 literal, whose colons are part of the address.
  if host.matches(':').count() == 1 {
    if let Some((left, _port)) = host.split_once(':') {
      host = left;
    }
  }

  host.trim().trim_end_matches('.').to_string()
}

/// The host a VLESS URI actually dials.
///
/// Load-bearing because of an asymmetry that is easy to get backwards: a VLESS
/// proxy presents to the browser as `127.0.0.1:<port>` — Donut runs a local xray
/// worker and points the browser at it — but the address that decides whether
/// anyone else could use this config is the SERVER inside the URI. The local
/// port is an implementation detail of this machine; the URI is the exit.
pub fn vless_uri_host(uri: &str) -> Option<String> {
  let rest = uri.trim().strip_prefix("vless://")?;
  // Cut the fragment (`#label`) and query (`?type=...`) before looking for the
  // authority — either may contain '@' or ':'.
  let rest = rest.split('#').next()?;
  let rest = rest.split('?').next()?;
  // `uuid@host:port/...`
  let authority = rest.split('/').next()?;
  let host_port = authority
    .rsplit_once('@')
    .map(|(_, h)| h)
    .unwrap_or(authority);
  let host = normalize_host(host_port);
  if host.is_empty() {
    None
  } else {
    Some(host)
  }
}

/// The exit host a stored proxy represents, as a remote host would have to dial
/// it.
pub fn proxy_exit_host(settings: &crate::browser::ProxySettings) -> Result<String, String> {
  if settings.proxy_type.eq_ignore_ascii_case("vless") {
    let uri = settings
      .vless_uri
      .as_deref()
      .filter(|uri| !uri.trim().is_empty())
      .ok_or_else(|| "VLESS proxy has no server URI".to_string())?;
    return vless_uri_host(uri).ok_or_else(|| "VLESS server URI is malformed".to_string());
  }

  let host = normalize_host(&settings.host);
  if host.is_empty() {
    return Err("proxy has no host".to_string());
  }
  Ok(host)
}

/// Whether a stored proxy speaks a protocol no remote host can dial.
///
/// Reads the two facts that both mean VLESS, because either one alone is a real
/// record: the picker writes `proxy_type = "vless"`, and a config pasted as a
/// bare URI carries the protocol in `vless_uri` before the type is normalised.
/// Taking only the first would let the second through to the host that refuses
/// it.
fn unsupported_remote_kind(settings: &crate::browser::ProxySettings) -> Option<String> {
  let has_uri = settings
    .vless_uri
    .as_deref()
    .is_some_and(|uri| !uri.trim().is_empty());
  if settings.proxy_type.eq_ignore_ascii_case("vless") || has_uri {
    return Some("VLESS".to_string());
  }
  None
}

/// Classify a stored proxy.
///
/// Protocol first, address second. A VLESS config names a perfectly routable
/// server, so asking the address question first answers `Remote` for an exit no
/// remote host can use; the kind has to disqualify it before the host is looked
/// at. (`proxy_exit_host` still resolves a VLESS server, because "which machine
/// does this dial" remains a real question for a log line.)
pub fn classify_proxy(settings: &crate::browser::ProxySettings) -> ExitReachability {
  if let Some(kind) = unsupported_remote_kind(settings) {
    return ExitReachability::UnsupportedKind {
      kind,
      source: "proxy",
    };
  }

  match proxy_exit_host(settings) {
    Err(reason) => ExitReachability::Unknown {
      reason,
      source: "proxy",
    },
    Ok(host) => {
      if host_is_remote_reachable(&host) {
        ExitReachability::Remote
      } else {
        ExitReachability::LocalOnly {
          host,
          source: "proxy",
        }
      }
    }
  }
}

/// Classify a WireGuard peer endpoint (`host:port`).
pub fn classify_wireguard_endpoint(peer_endpoint: &str) -> ExitReachability {
  let host = normalize_host(peer_endpoint);
  if host.is_empty() {
    return ExitReachability::Unknown {
      reason: "VPN config has no peer endpoint".to_string(),
      source: "VPN",
    };
  }
  if host_is_remote_reachable(&host) {
    ExitReachability::Remote
  } else {
    ExitReachability::LocalOnly {
      host,
      source: "VPN",
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::browser::ProxySettings;

  fn proxy(proxy_type: &str, host: &str) -> ProxySettings {
    ProxySettings {
      proxy_type: proxy_type.to_string(),
      host: host.to_string(),
      port: 8080,
      username: None,
      password: None,
      vless_uri: None,
    }
  }

  #[test]
  fn loopback_in_every_spelling_is_local() {
    // The literal case the bug was reported for, plus the spellings that reach
    // the same place. `::ffff:127.0.0.1` is the one a naive IPv6 check misses.
    for host in [
      "127.0.0.1",
      "127.1.2.3",
      "localhost",
      "LOCALHOST",
      "foo.localhost",
      "::1",
      "[::1]",
      "::ffff:127.0.0.1",
      "0.0.0.0",
      "::",
    ] {
      assert!(
        !host_is_remote_reachable(host),
        "{host} should not be remote-reachable"
      );
    }
  }

  #[test]
  fn private_and_carrier_ranges_are_local() {
    for host in [
      "10.0.0.1",
      "192.168.1.1",
      "172.16.0.1",
      "172.31.255.254",
      "169.254.1.1", // link-local / APIPA
      "100.64.0.1",  // CGNAT
      "100.127.255.1",
      "fd00::1", // unique-local
      "fe80::1", // link-local
      "240.0.0.1",
      "0.1.2.3",
    ] {
      assert!(
        !host_is_remote_reachable(host),
        "{host} should not be remote-reachable"
      );
    }
  }

  #[test]
  fn public_addresses_and_names_are_reachable() {
    for host in [
      "1.1.1.1",
      "8.8.8.8",
      "172.15.0.1", // just outside 172.16/12
      "172.32.0.1",
      "100.63.255.255", // just outside 100.64/10
      "100.128.0.1",
      "2606:4700:4700::1111",
      "proxy.example.com",
      "gate.smartproxy.net.",
      "residential.example.co.uk",
    ] {
      assert!(
        host_is_remote_reachable(host),
        "{host} should be remote-reachable"
      );
    }
  }

  #[test]
  fn lan_only_names_are_local() {
    // A remote host resolving these gets ITS network's answer, not the user's —
    // which is worse than failing, because it may well succeed against
    // something unrelated.
    for host in [
      "my-proxy", // single label: needs a search domain
      "router.local",
      "nas.home.arpa",
      "proxy.lan",
      "box.internal",
      "server.localdomain",
      "gateway.intranet",
    ] {
      assert!(
        !host_is_remote_reachable(host),
        "{host} should not be remote-reachable"
      );
    }
  }

  #[test]
  fn host_port_and_credentials_are_stripped_before_classifying() {
    assert!(!host_is_remote_reachable("127.0.0.1:8080"));
    assert!(!host_is_remote_reachable("user:pass@127.0.0.1:8080"));
    assert!(!host_is_remote_reachable("[::1]:1080"));
    assert!(host_is_remote_reachable("user:p@ss@proxy.example.com:8080"));
  }

  #[test]
  fn a_vless_proxy_is_refused_however_public_its_server_is() {
    // This URI names a routable server, so the address check says Remote and
    // the enrolment is accepted; every night after that the run is refused
    // remotely, because dialling VLESS needs a sidecar that is not available
    // there. The protocol has to disqualify the exit here, where no remote hour
    // has been spent yet.
    let mut settings = proxy("vless", "127.0.0.1");
    settings.vless_uri =
      Some("vless://6d6e21a1-4829-4d2b-bc7f-1b25707b61e4@vpn.example.com:443?type=tcp#node".into());

    assert_eq!(
      classify_proxy(&settings),
      ExitReachability::UnsupportedKind {
        kind: "VLESS".to_string(),
        source: "proxy",
      }
    );
    assert!(!classify_proxy(&settings).is_remote());
    // The refusal has to name the protocol and the exits that do work, or it
    // reads as "your proxy is broken" for a proxy that is fine everywhere else.
    let detail = classify_proxy(&settings).refusal_detail().unwrap();
    assert!(detail.contains("VLESS"), "{detail}");
    assert!(detail.contains("Xray"), "{detail}");
    assert!(detail.contains("SOCKS"), "{detail}");
  }

  #[test]
  fn a_vless_uri_is_refused_even_when_the_type_field_disagrees() {
    // A config pasted as a bare URI can land with the type still unnormalised.
    // Reading only `proxy_type` would classify this one on `settings.host` —
    // which for VLESS is the local xray worker, so it would come back LocalOnly
    // and tell the user to swap a proxy whose real problem is its protocol.
    let mut settings = proxy("socks5", "1.2.3.4");
    settings.vless_uri = Some("vless://uuid@vpn.example.com:443?type=tcp".into());

    assert_eq!(
      classify_proxy(&settings),
      ExitReachability::UnsupportedKind {
        kind: "VLESS".to_string(),
        source: "proxy",
      }
    );
  }

  #[test]
  fn a_vless_uri_pointing_at_loopback_is_refused_on_its_kind() {
    // Local AND unsupported. Either verdict blocks the run, but the kind is the
    // one the user has to act on: fixing the address still leaves an exit no
    // remote host can dial.
    let mut settings = proxy("vless", "127.0.0.1");
    settings.vless_uri = Some("vless://uuid@127.0.0.1:443?type=tcp".into());

    assert_eq!(
      classify_proxy(&settings),
      ExitReachability::UnsupportedKind {
        kind: "VLESS".to_string(),
        source: "proxy",
      }
    );
  }

  #[test]
  fn vless_host_parsing_survives_query_and_fragment() {
    assert_eq!(
      vless_uri_host("vless://uuid@example.com:443?sni=a@b.com&x=1#my@label"),
      Some("example.com".to_string())
    );
    assert_eq!(
      vless_uri_host("vless://uuid@[2606:4700::1111]:443?type=ws"),
      Some("2606:4700::1111".to_string())
    );
    assert_eq!(vless_uri_host("not-a-vless-uri"), None);
  }

  #[test]
  fn an_unreadable_config_fails_closed() {
    // Unknown must never be treated as usable: the point of the check is that
    // we could not confirm reachability, and guessing "yes" reintroduces the
    // exact failure it prevents.
    let verdict = classify_proxy(&proxy("socks5", "   "));

    assert!(matches!(verdict, ExitReachability::Unknown { .. }));
    assert!(!verdict.is_remote());
    assert!(verdict.refusal_detail().is_some());
  }

  #[test]
  fn ordinary_proxies_are_classified_by_host() {
    assert_eq!(
      classify_proxy(&proxy("socks5", "gate.example.com")),
      ExitReachability::Remote
    );
    assert_eq!(
      classify_proxy(&proxy("http", "192.168.0.10")),
      ExitReachability::LocalOnly {
        host: "192.168.0.10".to_string(),
        source: "proxy",
      }
    );
  }

  #[test]
  fn wireguard_endpoints_are_classified_by_their_peer() {
    assert_eq!(
      classify_wireguard_endpoint("vpn.example.com:51820"),
      ExitReachability::Remote
    );
    assert_eq!(
      classify_wireguard_endpoint("10.0.0.1:51820"),
      ExitReachability::LocalOnly {
        host: "10.0.0.1".to_string(),
        source: "VPN",
      }
    );
    assert!(matches!(
      classify_wireguard_endpoint("  "),
      ExitReachability::Unknown { .. }
    ));
  }

  #[test]
  fn only_remote_permits_a_run() {
    assert!(ExitReachability::Remote.is_remote());
    assert!(!ExitReachability::None.is_remote());
    assert!(!ExitReachability::LocalOnly {
      host: "127.0.0.1".into(),
      source: "proxy"
    }
    .is_remote());
    assert!(!ExitReachability::UnsupportedKind {
      kind: "VLESS".into(),
      source: "proxy"
    }
    .is_remote());
    // `None` has no detail: the caller's existing "no exit at all" refusal is
    // the better message, and two refusals for one condition read as a bug.
    assert!(ExitReachability::None.refusal_detail().is_none());
  }
}

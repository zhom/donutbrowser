/**
 * Reading a proxy out of a pasted line.
 *
 * The parser itself is Rust's `parse_txt_proxies`
 * (`src-tauri/src/proxy_manager.rs`); both the import dialog and the add/edit
 * form hand their clipboard text to it rather than re-implementing the format
 * zoo. What is left for the frontend is the part the backend deliberately
 * refuses to decide: `a:b:c:d` is either `host:port:username:password` or
 * `username:password:host:port`, and when both middle fields parse as a port
 * only the user knows which. The backend reports that as `ambiguous`; the
 * functions below turn the user's answer back into a proxy.
 *
 * Kept free of runtime imports so `proxy-string.test.mjs` can load it directly.
 */

import type { ParsedProxyLine, ProxyParseResult } from "@/types";

/**
 * URL schemes the Rust parser accepts, mapped onto the stored proxy type.
 *
 * A Map rather than an object literal, and for the same reason as
 * PROTOCOL_TOKENS below: the key comes from text the user pasted, and an object
 * lookup answers for the whole prototype chain. `__proto__://a:1:b:2` resolved
 * to Object.prototype and `constructor://...` to a function, both truthy, so
 * `splitProxyScheme` handed a non-string `proxy_type` to the import preview.
 */
const PROXY_SCHEMES = new Map<string, string>([
  ["http", "http"],
  ["https", "https"],
  ["httpstls", "httpstls"],
  ["socks", "socks5"],
  ["socks4", "socks4"],
  ["socks5", "socks5"],
  ["ss", "ss"],
  ["shadowsocks", "ss"],
  ["vless", "vless"],
]);

/**
 * Protocol identifiers, shortened where the raw stored type would not fit the
 * management table's column. Not translated: these are wire protocol names,
 * like the raw type that cell printed before. What the protocol MEANS for the
 * user belongs in the tooltip, which is translated.
 */
const PROTOCOL_TOKENS = new Map<string, string>([["httpstls", "HTTP/TLS"]]);

/**
 * Trim and case-fold a stored `proxy_type` before anything compares it.
 *
 * The first two steps of `canonicalProxyType` (`proxy-type.ts`), and every
 * reader of a stored type in this file goes through it. Skipping the trim here
 * while the form trimmed for its labels is what let one REST-stored `"ss "` be
 * Shadowsocks to the cipher field and an unencrypted hop to the warning printed
 * beside it, a false alarm about a proxy carrying a real cipher, and the
 * cheapest way to teach a user to ignore this warning. The two files cannot
 * import each other (both are loaded straight by `node --test`), so
 * `proxy-string.test.mjs` imports both and pins them in lockstep.
 */
function normalizeProxyType(proxyType: string): string {
  return proxyType.trim().toLowerCase();
}

/**
 * The protocol label for a stored proxy: the short token when there is one,
 * otherwise the stored type verbatim. Empty only when the stored type is blank,
 * which is the one case the caller has to fill with a translated placeholder.
 *
 * A Map, not an object literal. `proxy_type` is free text, POST /v1/proxies
 * and `import_proxies_json` store whatever they are given, unvalidated, so an
 * object lookup would answer from Object.prototype: `__proto__` yielded an
 * object, which React refuses as a child, and rendering it threw and unmounted
 * the ENTIRE proxy table, hiding every proxy and every first-hop warning;
 * `constructor`, `toString` and `valueOf` yielded functions, which React drops,
 * leaving the cell blank. A Map only ever answers for keys actually put in it.
 */
export function proxyProtocolToken(proxyType: string): string {
  return PROTOCOL_TOKENS.get(normalizeProxyType(proxyType)) ?? proxyType.trim();
}

/** What a line carrying no scheme is assumed to be. */
export const DEFAULT_PROXY_TYPE = "http";

/**
 * Proxy types whose hop from this machine to the proxy is encrypted.
 *
 * `https` is deliberately absent. It is a provider label on a plaintext CONNECT
 * endpoint, and Donut dials it byte-for-byte like `http`
 * (`proxy_server.rs::connect_to_target_via_upstream`). Listing it here would
 * make the UI state something untrue about the wire.
 */
const ENCRYPTED_FIRST_HOP = new Set([
  "httpstls",
  "ss",
  // The REST API stores proxy_type verbatim, so a proxy created with
  // "shadowsocks" reaches the UI with that spelling and the backend tunnels it
  // (proxy_server.rs matches "ss" | "shadowsocks"). Without it here the guard
  // below returned false and the UI told the user an ENCRYPTED hop was in the
  // clear, the same class of wrong claim this feature exists to remove, only
  // inverted.
  "shadowsocks",
  "vless",
]);

/**
 * Whether the leg from this machine to the proxy is encrypted.
 *
 * Says nothing about what the proxy operator can see: they terminate the
 * tunnel either way and still observe every hostname, port and timing, plus the
 * full contents of anything not carried over end-to-end TLS.
 */
const NULL_CIPHERS = new Set(["none", "plain"]);

/**
 * Whether the hop from this machine to the proxy is encrypted.
 *
 * Shadowsocks takes the cipher as free text, and shadowsocks-crypto maps both
 * "none" and "plain" to CipherKind::NONE, a no-op. Keying off the type alone
 * therefore labelled such a proxy "First hop encrypted", coloured it safe, and
 * suppressed the credentials-in-clear warning, for a connection carrying
 * everything in the clear. A claim of safety that is wrong is worse than no
 * claim, so the cipher decides for Shadowsocks.
 */
export function isFirstHopEncrypted(
  proxyType: string,
  cipher?: string | null,
): boolean {
  const type = normalizeProxyType(proxyType);
  if (!ENCRYPTED_FIRST_HOP.has(type)) return false;
  if (type === "ss" || type === "shadowsocks") {
    // Fail closed. An absent cipher is not evidence of encryption, and the
    // asymmetry matters: wrongly saying "in the clear" costs a needless
    // warning, wrongly saying "encrypted" costs the user the one thing this
    // label exists to tell them.
    const normalized = (cipher ?? "").trim().toLowerCase();
    return normalized.length > 0 && !NULL_CIPHERS.has(normalized);
  }
  return true;
}

export const HOST_FIRST_FORMAT = "host:port:username:password";
export const CREDENTIALS_FIRST_FORMAT = "username:password:host:port";

/**
 * Separates `socks5://1.2.3.4:1080` into its scheme and body. An unknown or
 * absent scheme leaves the body untouched and falls back to HTTP, which is what
 * the backend does with a bare `host:port`.
 */
export function splitProxyScheme(line: string): {
  proxyType: string;
  rest: string;
} {
  const separator = line.indexOf("://");
  if (separator === -1) {
    return { proxyType: DEFAULT_PROXY_TYPE, rest: line };
  }

  const proxyType = PROXY_SCHEMES.get(line.slice(0, separator).toLowerCase());
  return proxyType
    ? { proxyType, rest: line.slice(separator + 3) }
    : { proxyType: DEFAULT_PROXY_TYPE, rest: line };
}

/**
 * Builds a proxy from a four-part line once the user has said which of the two
 * orderings it uses. Returns null when the chosen ordering doesn't actually fit
 * the line, so a stale selection can't produce a proxy pointing at a password.
 */
export function resolveAmbiguousProxyLine(
  line: string,
  format: string,
): ParsedProxyLine | null {
  const trimmed = line.trim();
  const { proxyType, rest } = splitProxyScheme(trimmed);
  const parts = rest.split(":");
  if (parts.length !== 4) {
    return null;
  }

  const hostFirst = format === HOST_FIRST_FORMAT;
  if (!hostFirst && format !== CREDENTIALS_FIRST_FORMAT) {
    return null;
  }

  const host = hostFirst ? parts[0] : parts[2];
  const port = Number.parseInt(hostFirst ? parts[1] : parts[3], 10);
  if (!host || !Number.isInteger(port) || port < 1 || port > 65535) {
    return null;
  }

  return {
    proxy_type: proxyType,
    host,
    port,
    username: hostFirst ? parts[2] : parts[0],
    password: hostFirst ? parts[3] : parts[1],
    original_line: trimmed,
  };
}

/**
 * Picks the proxy to use out of a parse of pasted text. Only the first usable
 * line matters: the form holds one proxy, and a paste that happens to carry a
 * whole list should still fill it in rather than do nothing.
 *
 * Ambiguous lines resolve as `host:port:username:password`, the ordering the
 * import dialog offers first and the one vendors overwhelmingly ship.
 */
export function pickParsedProxy(
  results: ProxyParseResult[],
): ParsedProxyLine | null {
  for (const result of results) {
    if (result.status === "parsed") {
      return {
        proxy_type: result.proxy_type,
        host: result.host,
        port: result.port,
        username: result.username,
        password: result.password,
        vless_uri: result.vless_uri,
        original_line: result.original_line,
      };
    }
    if (result.status === "ambiguous") {
      const resolved = resolveAmbiguousProxyLine(
        result.line,
        HOST_FIRST_FORMAT,
      );
      if (resolved) {
        return resolved;
      }
    }
  }
  return null;
}

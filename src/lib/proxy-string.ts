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

/** URL schemes the Rust parser accepts, mapped onto the stored proxy type. */
const PROXY_SCHEMES: Record<string, string> = {
  http: "http",
  https: "https",
  socks: "socks5",
  socks4: "socks4",
  socks5: "socks5",
  ss: "ss",
  shadowsocks: "ss",
  vless: "vless",
};

/** What a line carrying no scheme is assumed to be. */
export const DEFAULT_PROXY_TYPE = "http";

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

  const proxyType = PROXY_SCHEMES[line.slice(0, separator).toLowerCase()];
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

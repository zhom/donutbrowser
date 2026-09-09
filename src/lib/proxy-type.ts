/**
 * One answer to "what kind of proxy is this?".
 *
 * `proxy_type` is stored verbatim. `POST /v1/proxies` and `import_proxies_json`
 * write whatever the caller sent, and only VLESS is rewritten on the way in
 * (`proxy_manager.rs::normalize_proxy_settings`). So the same Shadowsocks proxy
 * reaches the UI as `ss` or as `shadowsocks`, `proxy_server.rs` tunnels both -
 * and in any case, because the Rust side compares through `Url::scheme()`,
 * which lowercases for it.
 *
 * Comparing the stored string to a single spelling is what let the edit dialog
 * label a Shadowsocks cipher "Username", skip that field's required check, and
 * render its type Select on the placeholder as though the proxy had no type at
 * all. Ask this instead of writing `proxy_type === "ss"`.
 *
 * Kept free of runtime imports so `proxy-type.test.mjs` can load it directly.
 */

/**
 * Spellings that name a type the UI already knows under another name.
 *
 * A Map, not an object literal, and for the same reason `PROXY_SCHEMES` in
 * `proxy-string.ts` is one: the key is free text, `POST /v1/proxies` and
 * `import_proxies_json` store whatever they are handed, and an object lookup
 * answers for the whole prototype chain. `canonicalProxyType("__proto__")`
 * returned Object.prototype and `("constructor")` returned a function; both are
 * truthy, so the `??` never fired and the declared `: string` return was a lie
 * TypeScript cannot catch. Every caller branches on this value and one of them
 * puts it in a React child, which throws on an object and drops a function. A
 * Map only ever answers for keys actually put into it.
 *
 * Only aliases the backend genuinely treats as the same protocol belong here. A
 * scheme the Rust worker would not match (`socks` is not `socks5` to
 * `Url::scheme()`) must stay distinct, or the UI describes a wire that does not
 * exist.
 */
const TYPE_ALIASES = new Map<string, string>([["shadowsocks", "ss"]]);

/**
 * The spelling the UI branches on, for a `proxy_type` that may carry any of
 * them. Returns the input trimmed and lowercased when it has no alias, so an
 * unknown type still compares predictably rather than throwing the caller into
 * a default.
 *
 * Trimming and case-folding here obliges everything else that reads a stored
 * `proxy_type` to do the same. `isFirstHopEncrypted` (`proxy-string.ts`) did
 * not, so one REST-stored `"ss "` was Shadowsocks to the field labels and an
 * unencrypted hop to the warning beside them. Both files are loaded straight by
 * `node --test`, which resolves neither `@/` nor an extensionless relative
 * specifier, so they cannot import each other; `proxy-string.test.mjs` imports
 * both and pins them in lockstep instead.
 */
export function canonicalProxyType(proxyType: string): string {
  const normalized = proxyType.trim().toLowerCase();
  return TYPE_ALIASES.get(normalized) ?? normalized;
}

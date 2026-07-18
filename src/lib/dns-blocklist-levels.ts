/**
 * The DNS blocklist levels the backend accepts, mirroring
 * `BlocklistLevel::as_str` in `src-tauri/src/dns_blocklist.rs`. Ordered from
 * least to most restrictive, with `custom` (the user's own sources/rules) last.
 *
 * Every level picker reads from this list. Keeping it in one place is what
 * stops a new level from reaching some surfaces and not others — `custom` was
 * previously missing from two pickers, so a profile already set to it rendered
 * with nothing selected and could not be restored.
 *
 * Labels are translation keys, never the backend's `display_name`: that field
 * is hardcoded English and renders untranslated to every locale.
 */
export const DNS_BLOCKLIST_LEVELS = [
  { value: "light", labelKey: "dnsBlocklist.light" },
  { value: "normal", labelKey: "dnsBlocklist.normal" },
  { value: "pro", labelKey: "dnsBlocklist.pro" },
  { value: "pro_plus", labelKey: "dnsBlocklist.proPlus" },
  { value: "ultimate", labelKey: "dnsBlocklist.ultimate" },
  { value: "custom", labelKey: "dnsBlocklist.customLevel" },
] as const;

export type DnsBlocklistLevel = (typeof DNS_BLOCKLIST_LEVELS)[number]["value"];

/**
 * Translation key for a level slug. A null/empty level means no filtering, and
 * an unrecognised one (a level added backend-first) falls back to the same,
 * which is the honest reading of "we don't know this level".
 */
export function dnsBlocklistLabelKey(level: string | null | undefined): string {
  if (!level) {
    return "dnsBlocklist.none";
  }
  return (
    DNS_BLOCKLIST_LEVELS.find((l) => l.value === level)?.labelKey ??
    "dnsBlocklist.none"
  );
}

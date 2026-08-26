import assert from "node:assert/strict";
import test from "node:test";
import {
  matchesProfile,
  PROFILE_SEARCH_FIELDS,
  parseProfileSearch,
} from "./profile-search.ts";

/**
 * What is pinned here is the promise the table depends on: the box behaves
 * exactly as it did before for a bare word, a query being typed never blanks
 * the list, and every field resolves the name a user can see rather than the id
 * the profile stores.
 */

const NOW = Date.parse("2026-06-15T12:00:00Z");
const HOUR = 3600;
const DAY = 86400;

const ctx = {
  groupNames: new Map([["g1", "Client A"]]),
  proxyNames: new Map([["p1", "Frankfurt residential"]]),
  vpnNames: new Map([["v1", "Office WireGuard"]]),
  extensionGroupNames: new Map([["e1", "Ad blockers"]]),
  runningProfiles: new Set(["shop"]),
  now: NOW,
};

function profile(overrides = {}) {
  return {
    id: "a1b2c3d4-1111-2222-3333-444455556666",
    name: "Shopify EU",
    browser: "wayfern",
    version: "140.0.3",
    release_type: "stable",
    ...overrides,
  };
}

/** Convenience: does this raw query match this profile? */
function hit(query, target, context = ctx) {
  return matchesProfile(target, parseProfileSearch(query), context);
}

function names(query, targets) {
  const parsed = parseProfileSearch(query);
  return targets
    .filter((p) => matchesProfile(p, parsed, ctx))
    .map((p) => p.name);
}

test("an empty query matches everything", () => {
  for (const raw of ["", "   ", '""', "\t\n"]) {
    const parsed = parseProfileSearch(raw);
    assert.equal(parsed.isEmpty, true, `expected ${JSON.stringify(raw)} empty`);
    assert.equal(hit(raw, profile()), true);
  }
});

test("plain text still searches name, note and tags", () => {
  assert.equal(hit("shopify", profile()), true);
  assert.equal(hit("SHOPIFY", profile()), true);
  assert.equal(hit("amazon", profile()), false);
  assert.equal(hit("renew", profile({ note: "Renew the card" })), true);
  assert.equal(hit("ads", profile({ tags: ["paid-ads", "eu"] })), true);
});

test("plain text also matches the id by prefix, so the table's short id works", () => {
  assert.equal(hit("a1b2c3d4", profile()), true);
  assert.equal(hit("A1B2C3D4", profile()), true);
  assert.equal(hit("a1b2c3d4-1111-2222-3333-444455556666", profile()), true);
  // A slice from the middle is not a prefix and must not match.
  assert.equal(hit("2222", profile()), false);
});

test("a colon inside an ordinary word stays free text", () => {
  const noted = profile({
    note: "check https://shop.example.com:8080 at 12:30",
  });
  assert.equal(hit("https://shop.example.com:8080", noted), true);
  assert.equal(hit("12:30", noted), true);
  // An unknown field name is free text too, never a filter that matches nothing.
  assert.equal(hit("warmup:done", profile({ note: "warmup:done" })), true);
  assert.equal(hit("warmup:done", profile()), false);
});

test("field terms match on the resolved name, not the stored id", () => {
  const target = profile({
    group_id: "g1",
    proxy_id: "p1",
    vpn_id: "v1",
    extension_group_id: "e1",
  });
  assert.equal(hit("group:client", target), true);
  assert.equal(hit('group:"Client A"', target), true);
  assert.equal(hit("group:g1", target), false);
  assert.equal(hit("proxy:frankfurt", target), true);
  assert.equal(hit("vpn:office", target), true);
  assert.equal(hit("ext:blockers", target), true);
  assert.equal(hit("folder:client", target), true, "alias");
  assert.equal(hit("extension:blockers", target), true, "alias");
});

test("name, note, tag, id, browser, version and email fields", () => {
  const target = profile({
    note: "Renew the card",
    tags: ["prod", "eu"],
    created_by_email: "ops@example.com",
  });
  assert.equal(hit("name:shop", target), true);
  assert.equal(hit("name:renew", target), false, "name must not read the note");
  assert.equal(hit("note:card", target), true);
  assert.equal(hit("notes:card", target), true, "alias");
  assert.equal(hit("tag:prod", target), true);
  assert.equal(hit("tags:eu", target), true, "alias");
  assert.equal(hit("id:a1b2c3d4", target), true);
  assert.equal(hit("id:b2c3", target), false, "id matches by prefix only");
  assert.equal(hit("browser:wayfern", target), true);
  assert.equal(hit("version:140", target), true);
  assert.equal(hit("email:ops@example.com", target), true);
  assert.equal(hit("owner:ops", target), true, "alias");
});

test("enum fields match a slug by prefix, so a half-typed value narrows", () => {
  const running = profile({ id: "shop", name: "Live" });
  assert.equal(hit("status:running", running), true);
  assert.equal(hit("status:run", running), true);
  assert.equal(hit("status:stopped", running), false);
  assert.equal(hit("status:stopped", profile()), true);
  assert.equal(hit("os:macos", profile({ host_os: "macos" })), true);
  assert.equal(
    hit("os:windows", profile({ wayfern_config: { os: "windows" } })),
    true,
    "falls back to the fingerprint OS",
  );
  assert.equal(
    hit("dns:pro_plus", profile({ dns_blocklist: "pro_plus" })),
    true,
  );
  assert.equal(
    hit("sync:encrypted", profile({ sync_mode: "Encrypted" })),
    true,
  );
  assert.equal(
    hit("sync:disabled", profile()),
    true,
    "unset reads as disabled",
  );
});

test("boolean fields take yes and no", () => {
  assert.equal(hit("locked:yes", profile({ password_protected: true })), true);
  assert.equal(hit("locked:no", profile({ password_protected: true })), false);
  assert.equal(hit("password:no", profile()), true, "alias, unset is false");
  assert.equal(hit("ephemeral:yes", profile({ ephemeral: true })), true);
  assert.equal(
    hit("locked:maybe", profile({ password_protected: true })),
    false,
    "an unusable value matches nothing rather than everything",
  );
});

test("none and any answer the empty question on every relation", () => {
  const bare = profile();
  const wired = profile({ proxy_id: "p1", group_id: "g1", tags: ["eu"] });
  assert.equal(hit("proxy:none", bare), true);
  assert.equal(hit("proxy:none", wired), false);
  assert.equal(hit("proxy:any", wired), true);
  assert.equal(hit("group:none", bare), true);
  assert.equal(hit("tag:none", bare), true);
  assert.equal(hit("tag:any", wired), true);
  assert.equal(hit("note:any", profile({ note: "x" })), true);
  // A quoted value is the literal word, so a tag really called "none" is findable.
  assert.equal(hit('tag:"none"', profile({ tags: ["none"] })), true);
  assert.equal(hit('tag:"none"', bare), false);
  // A proxy whose stored name no longer resolves still counts as having one.
  assert.equal(hit("proxy:none", profile({ proxy_id: "gone" })), false);
  assert.equal(hit("proxy:any", profile({ proxy_id: "gone" })), true);
});

test("negation inverts a term, on both kinds", () => {
  const tagged = profile({ tags: ["banned"] });
  assert.equal(hit("-tag:banned", tagged), false);
  assert.equal(hit("-tag:banned", profile()), true);
  assert.equal(hit("!tag:banned", tagged), false, "! is an alias for -");
  assert.equal(hit("tag!=banned", tagged), false);
  assert.equal(hit("tag!=banned", profile()), true);
  assert.equal(hit("-shopify", profile()), false);
  assert.equal(hit("-amazon", profile()), true);
});

test("quotes hold a value together and keep separators literal", () => {
  const spaced = profile({ tags: ["black friday"], note: "a, b" });
  assert.equal(hit('tag:"black friday"', spaced), true);
  assert.equal(
    hit("tag:black friday", profile({ tags: ["black"] })),
    false,
    "unquoted is two terms, and nothing here matches the second",
  );
  assert.equal(hit('note:"a, b"', spaced), true, "a quoted comma is literal");
  assert.equal(hit('"-lead"', profile({ name: "-lead" })), true);
});

test("several terms combine with AND", () => {
  const target = profile({ tags: ["prod"], group_id: "g1", note: "vat" });
  assert.equal(hit("tag:prod group:client", target), true);
  assert.equal(hit("tag:prod group:other", target), false);
  assert.equal(hit("shopify tag:prod note:vat status:stopped", target), true);
  assert.equal(hit("shopify tag:prod -note:vat", target), false);
});

test("or joins two terms, and binds tighter than the implicit and", () => {
  const rows = [
    profile({ name: "A", tags: ["ads"], id: "shop" }),
    profile({ name: "B", tags: ["seo"] }),
    profile({ name: "C", tags: ["other"] }),
  ];
  assert.deepEqual(names("tag:ads or tag:seo", rows), ["A", "B"]);
  assert.deepEqual(names("tag:ads or tag:seo status:running", rows), ["A"]);
  assert.deepEqual(names("tag:ads,seo", rows), ["A", "B"], "comma is or");
  assert.deepEqual(
    names("OR tag:ads or", rows),
    ["A"],
    "a dangling or is ignored",
  );
});

test("an = prefix forces a whole-value match", () => {
  const long = profile({ tags: ["production"] });
  assert.equal(hit("tag:prod", long), true);
  assert.equal(hit("tag:=prod", long), false);
  assert.equal(hit("tag:=production", long), true);
  assert.equal(hit('group:="Client A"', profile({ group_id: "g1" })), true);
  assert.equal(
    hit("name:=shopify", profile()),
    false,
    "the real name is longer",
  );
});

test("dates take relative durations, read the way the question is asked", () => {
  const fresh = profile({ last_launch: NOW / 1000 - 2 * DAY });
  const cold = profile({ last_launch: NOW / 1000 - 90 * DAY });
  assert.equal(hit("launched:<7d", fresh), true);
  assert.equal(hit("launched:<7d", cold), false);
  assert.equal(
    hit("launched:>30d", cold),
    true,
    "not launched for over 30 days",
  );
  assert.equal(hit("launched:>30d", fresh), false);
  assert.equal(hit("launched:7d", fresh), true, "bare means within");
  assert.equal(
    hit("launched:<12h", profile({ last_launch: NOW / 1000 - HOUR })),
    true,
  );
  assert.equal(hit("launched:never", profile()), true);
  assert.equal(hit("launched:never", fresh), false);
  assert.equal(hit("launched:any", fresh), true);
  assert.equal(hit("lastlaunch:<7d", fresh), true, "alias");
});

test("dates take absolute days, months and years", () => {
  const made = profile({
    created_at: Date.parse("2026-03-04T10:00:00") / 1000,
  });
  assert.equal(hit("created:2026-03-04", made), true);
  assert.equal(hit("created:2026-03-05", made), false);
  assert.equal(hit("created:2026-03", made), true);
  assert.equal(hit("created:2026", made), true);
  assert.equal(hit("created:>=2026-01-01", made), true);
  assert.equal(hit("created:<2026-01-01", made), false);
  assert.equal(
    hit("created:>2026-03", made),
    false,
    "March is not after March",
  );
  assert.equal(
    hit("created:none", profile()),
    true,
    "legacy profiles have none",
  );
});

test("version comparisons run segment by segment", () => {
  assert.equal(hit("version:>140", profile()), true);
  assert.equal(hit("version:>=140.0", profile()), true);
  assert.equal(hit("version:<140", profile()), false);
  assert.equal(hit("version:>141", profile()), false);
  assert.equal(
    hit("version:>9", profile({ version: "10.0.1" })),
    true,
    "not lexical",
  );
});

test("a query being typed never throws and never blanks the list", () => {
  const target = profile({ tags: ["prod"], note: 'say "hello"' });
  const halves = [
    'name:"unclosed',
    "name:",
    "tag:",
    "-",
    "!",
    ":",
    '"',
    '""',
    "or",
    "or or or",
    "tag:,,,",
    "created:>",
    "created:>notadate",
    "launched:<abc",
    "tag:prod created:notadate",
    "=",
    "tag:=",
    ">=<:",
    "name:>shop",
  ];
  for (const raw of halves) {
    assert.doesNotThrow(() => parseProfileSearch(raw), raw);
    assert.doesNotThrow(() => hit(raw, target), raw);
  }
  assert.equal(hit('name:"unclosed', profile({ name: "unclosed" })), true);
  assert.equal(hit("tag:", target), true, "a bare field filters nothing");
  assert.equal(
    hit("tag:prod created:notadate", target),
    true,
    "bad date drops itself",
  );
  assert.equal(
    hit("name:>shop", profile()),
    false,
    "a bad operator falls to text",
  );
  assert.equal(hit("name:>shop", profile({ note: "name:>shop" })), true);
});

test("unicode values compare case-insensitively", () => {
  const cyrillic = profile({ name: "Профиль Магазин", tags: ["Реклама"] });
  assert.equal(hit("магазин", cyrillic), true);
  assert.equal(hit("МАГАЗИН", cyrillic), true);
  assert.equal(hit("tag:реклама", cyrillic), true);
  assert.equal(hit("name:профиль", cyrillic), true);

  const cjk = profile({ name: "東京プロファイル", note: "測試" });
  assert.equal(hit("東京", cjk), true);
  assert.equal(hit("note:測試", cjk), true);

  const emoji = profile({ name: "Store 🛒 EU", tags: ["🔥 hot"] });
  assert.equal(hit("🛒", emoji), true);
  assert.equal(hit('tag:"🔥 hot"', emoji), true);
  assert.equal(hit("straße", profile({ name: "Straße Berlin" })), true);
});

test("every field carries a translation key and unique tokens", () => {
  const seen = new Set();
  for (const field of PROFILE_SEARCH_FIELDS) {
    assert.match(field.labelKey, /^search\.fields\./, field.key);
    for (const token of [field.key, ...field.aliases]) {
      assert.equal(seen.has(token), false, `duplicate token ${token}`);
      seen.add(token);
    }
  }
});

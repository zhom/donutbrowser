import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

/**
 * A cookie bot run that refuses is RECORDED rather than thrown: the fleet
 * writes an `outcome_code` onto the run row so the user reads it in their
 * history. `OUTCOME_KEYS` in cookie-bot-shared.tsx turns that code into a
 * sentence, and `outcomeLabel` falls back to `cookieBot.outcome.unknown`, which
 * prints the raw token. So a code the server sends and this map has never heard
 * of does not crash, does not warn, and does not fail a build. It renders
 * `browsing_incomplete` to a paying customer and stays that way for as long as
 * nobody happens to look.
 *
 * That is not hypothetical: five codes had already drifted apart when this was
 * written, `proxy_local_only` and `browsing_incomplete` among them, the latter
 * being the commonest real outcome there is. Both sides were correct in
 * isolation. Nothing compared them.
 *
 * This file is that comparison, in two halves. The committed snapshot runs
 * everywhere, including CI, where no API source is available; the cross-repo
 * check runs only where that source is and exists to say when the snapshot has
 * stopped being a mirror of the producer. Neither half is sufficient alone: a
 * snapshot with no live check rots, and a live check with no snapshot is inert
 * exactly where regressions get caught.
 */

const HERE = path.dirname(fileURLToPath(import.meta.url));
const LOCALES = path.join(HERE, "..", "i18n", "locales");
const SHARED = path.join(HERE, "..", "components", "cookie-bot-shared.tsx");

/**
 * Every value of the server's `CookieBotOutcomeCode` union, snapshotted from
 * the cloud API.
 *
 * Sorted, because the union's source order carries no meaning and an
 * alphabetical list makes a diff here readable.
 */
const SERVER_OUTCOME_CODES = [
  "browsing_incomplete",
  "budget_exceeded",
  "cancelled_by_user",
  "encrypted_sync",
  "manager_error",
  "no_capacity",
  "no_sites",
  "not_entitled",
  "platform_unsupported",
  "profile_locked",
  "profile_not_synced",
  "profile_os_mismatch",
  "proxy_local_only",
  "proxy_required",
  "proxy_unsupported",
  "quota_exhausted",
  "sync_disabled",
  "touch_fingerprint",
];

/** The declaration the live half looks for, and reads the union out of. */
const UNION_DECLARATION = "export type CookieBotOutcomeCode";

/**
 * A checkout of the API source to read the live union from, when there is one.
 *
 * No default and no assumed location: the cross-repo half runs only where
 * `DONUT_API_SOURCE_DIR` points at that source, and is skipped everywhere else
 * — CI included — exactly as it was skipped before when nothing was available
 * to read.
 */
const API_SOURCE_DIR = process.env.DONUT_API_SOURCE_DIR ?? null;

/** Nothing worth reading lives in these, and walking them is slow. */
const UNREAD_DIRS = new Set([
  "node_modules",
  "dist",
  "build",
  ".git",
  ".next",
  ".turbo",
]);

/**
 * The file under `dir` that declares the union, found by the declaration
 * itself rather than by a path this repo has no business knowing. Null when
 * the tree holds no such file.
 */
function findUnionSource(dir, depth = 0) {
  if (depth > 8) return null;
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return null;
  }
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (UNREAD_DIRS.has(entry.name)) continue;
      const found = findUnionSource(full, depth + 1);
      if (found !== null) return found;
      continue;
    }
    if (!entry.name.endsWith(".ts")) continue;
    try {
      if (readFileSync(full, "utf8").includes(UNION_DECLARATION)) return full;
    } catch {
      // Unreadable, and so not the file being looked for either way.
    }
  }
  return null;
}

/**
 * Drop `//` and block comments so a sentence inside one cannot be read as code.
 *
 * Both regions this is used on hold only identifiers and single-line string
 * literals, so there is no string that could contain a comment opener and be
 * mangled by this. Applying it to a whole file would not be safe.
 */
const stripComments = (source) =>
  source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "");

/** The body of the first `{ ... };` block after `marker`, comments removed. */
function blockAfter(source, marker, file) {
  const start = source.indexOf(marker);
  assert.notEqual(
    start,
    -1,
    `${file} no longer contains \`${marker}\`; this test has drifted from the ` +
      "file it exists to read, so do not treat a pass here as coverage",
  );
  const open = source.indexOf("{", start);
  const close = source.indexOf("\n};", open);
  assert.ok(
    open !== -1 && close > open,
    `could not find the end of \`${marker}\` in ${file}`,
  );
  return stripComments(source.slice(open + 1, close));
}

/** The real `OUTCOME_KEYS` map, read out of the component's source text. */
function outcomeKeys() {
  const body = blockAfter(
    readFileSync(SHARED, "utf8"),
    "export const OUTCOME_KEYS",
    "cookie-bot-shared.tsx",
  );
  const entries = [...body.matchAll(/([A-Za-z_][\w]*)\s*:\s*"([^"]+)"/g)].map(
    (m) => [m[1], m[2]],
  );
  // A regex that silently matches nothing would make every assertion below
  // vacuous, and this whole file would report green over an empty map. The
  // floor is deliberately well under the real count so a legitimate removal
  // fails on the comparison that follows rather than here.
  assert.ok(
    entries.length >= 15,
    `parsed only ${entries.length} entries out of OUTCOME_KEYS; the map's ` +
      "shape has changed and this parser is no longer reading it",
  );
  return new Map(entries);
}

/** The server's `CookieBotOutcomeCode` union, or null when it is not on disk. */
function serverUnion() {
  if (API_SOURCE_DIR === null) return null;
  const file = findUnionSource(API_SOURCE_DIR);
  if (file === null) return null;
  const source = readFileSync(file, "utf8");
  const start = source.indexOf(UNION_DECLARATION);
  assert.notEqual(
    start,
    -1,
    `${file} exists but declares no CookieBotOutcomeCode; the union ` +
      "has been renamed or moved, and this check is reading the wrong file",
  );
  // Strip first, then cut at the terminating semicolon: the union's members are
  // interleaved with JSDoc prose, and prose is exactly where a stray `;` lives.
  const declaration = stripComments(source.slice(start));
  const end = declaration.indexOf(";");
  assert.ok(end > 0, "the CookieBotOutcomeCode union is not terminated");
  const codes = [...declaration.slice(0, end).matchAll(/'([a-z_]+)'/g)].map(
    (m) => m[1],
  );
  assert.ok(
    codes.length >= 15,
    `parsed only ${codes.length} members out of CookieBotOutcomeCode; the ` +
      "union's shape has changed and this parser is no longer reading it",
  );
  return codes.sort();
}

function localeBundles() {
  const files = readdirSync(LOCALES).filter((name) => name.endsWith(".json"));
  return files.map((name) => [
    name.replace(/\.json$/, ""),
    JSON.parse(readFileSync(path.join(LOCALES, name), "utf8")),
  ]);
}

const lookup = (bundle, key) =>
  key
    .split(".")
    .reduce((node, part) => (node == null ? undefined : node[part]), bundle);

test("the snapshot covers exactly the outcome codes the map handles", () => {
  // Both directions on purpose. Missing entries are the bug that already
  // shipped: the server sends a code, the map has no sentence, and the run
  // history prints `browsing_incomplete` at the customer. Extra entries are
  // the quieter half — a mapping for a code the server stopped sending keeps
  // "passing" forever and makes the map stop being a mirror of the producer,
  // which is the only thing that makes checking it worthwhile.
  const map = outcomeKeys();
  assert.deepEqual(
    [...map.keys()].sort(),
    [...SERVER_OUTCOME_CODES].sort(),
    "OUTCOME_KEYS and the snapshotted server union disagree; a code the " +
      "server can send would render as its raw token in the run history",
  );
});

test("every outcome sentence exists, non-empty, in every locale", () => {
  const map = outcomeKeys();
  const bundles = localeBundles();
  assert.ok(
    bundles.length >= 10,
    `found ${bundles.length} locale files, expected at least ten`,
  );

  // `outcomeLabel` routes an unmapped code through this one, so it is as
  // load-bearing as any mapped key and is missed by a loop over the map.
  const keys = [...map.values(), "cookieBot.outcome.unknown"];

  for (const [locale, bundle] of bundles) {
    for (const key of keys) {
      const value = lookup(bundle, key);
      assert.equal(
        typeof value,
        "string",
        `${locale} has no string at ${key}; that outcome renders untranslated`,
      );
      assert.notEqual(
        value.trim(),
        "",
        `${locale} has an empty string at ${key}; the run history would show ` +
          "a blank reason, which reads as no reason at all",
      );
    }
  }
});

test("no two outcomes share one sentence, in any locale", () => {
  // Each code exists because its fix differs: finish a sync, attach a public
  // proxy, use a proxy the fleet can dial, re-record the profile on this OS.
  // Collapsing two onto one sentence sends a customer to do the wrong thing
  // about a problem the product already knew how to name.
  const values = [...outcomeKeys().values()];
  for (const [locale, bundle] of localeBundles()) {
    const rendered = values.map((key) => lookup(bundle, key));
    assert.equal(
      new Set(rendered).size,
      values.length,
      `${locale} reuses one sentence for two different cookie bot outcomes`,
    );
  }
});

test("the snapshot still matches the server's real union", (t) => {
  const codes = serverUnion();
  if (codes === null) {
    t.skip(
      "no API source is available, so the snapshot above ran against itself " +
        "only: whether SERVER_OUTCOME_CODES still equals the server's " +
        "CookieBotOutcomeCode union is UNCHECKED here. Set " +
        "DONUT_API_SOURCE_DIR to a checkout of the API source to check it",
    );
    return;
  }

  // Equality, not containment, and for the same reason as the map comparison
  // above: a code the server has REMOVED must fail here too, or the snapshot
  // grows stale entries that keep the whole file green while it mirrors
  // nothing.
  assert.deepEqual(
    codes,
    [...SERVER_OUTCOME_CODES].sort(),
    "the server's CookieBotOutcomeCode union has changed; update " +
      "SERVER_OUTCOME_CODES, OUTCOME_KEYS, and every locale file together",
  );
});

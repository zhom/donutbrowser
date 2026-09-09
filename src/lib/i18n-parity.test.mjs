import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

/**
 * The project rule is that a new user-facing string lands in EVERY locale file
 * in the same change. Nothing enforced it: `pnpm lint` runs biome, tsc, clippy
 * and typos, and `pnpm test` runs ten suites, none of which reads more than one
 * locale. A review found four `backendErrors.wayfern*` keys, all of them
 * emitted by Rust and rendered by the frontend, missing from all nine
 * non-English locales, so a Russian user saw English for a real error. They had
 * been missing long enough that nobody could say when.
 *
 * This is the gate. It reads whatever locale files are on disk rather than a
 * hardcoded list, because a newly added locale is exactly what a hardcoded list
 * skips.
 */

const HERE = path.dirname(fileURLToPath(import.meta.url));
const LOCALES = path.join(HERE, "..", "i18n", "locales");

/** i18next appends these to a base key for languages with more plural forms. */
const PLURAL_SUFFIX = /_(zero|one|two|few|many|other)$/;

function flatten(value, prefix = "") {
  const out = new Map();
  for (const [key, child] of Object.entries(value)) {
    const full = prefix ? `${prefix}.${key}` : key;
    if (child !== null && typeof child === "object" && !Array.isArray(child)) {
      for (const [k, v] of flatten(child, full)) out.set(k, v);
    } else {
      out.set(full, child);
    }
  }
  return out;
}

function load() {
  const files = readdirSync(LOCALES).filter((name) => name.endsWith(".json"));
  return files.map((name) => [
    name.replace(/\.json$/, ""),
    flatten(JSON.parse(readFileSync(path.join(LOCALES, name), "utf8"))),
  ]);
}

/** `{{name}}` placeholders, which must survive translation or interpolation breaks. */
const placeholders = (value) =>
  new Set(
    typeof value === "string"
      ? [...value.matchAll(/\{\{\s*([\w.]+)\s*\}\}/g)].map((m) => m[1])
      : [],
  );

const bundles = load();
const english = bundles.find(([code]) => code === "en")?.[1];

test("the locale files are actually there and actually loaded", () => {
  // Without this, every assertion below is vacuously true the moment the glob
  // stops matching, the exact way a directory rename turns this whole file
  // green while it checks nothing.
  assert.ok(english, "en.json must exist");
  assert.ok(
    english.size > 1000,
    `en.json flattened to only ${english?.size} keys; it has been truncated ` +
      "or the flattener is broken",
  );
  assert.ok(
    bundles.length >= 10,
    `found ${bundles.length} locale files, expected at least ten`,
  );
});

test("every locale carries exactly the English key set", () => {
  for (const [code, bundle] of bundles) {
    if (code === "en") continue;

    const missing = [...english.keys()].filter((key) => !bundle.has(key));
    assert.deepEqual(
      missing,
      [],
      `${code}.json is missing ${missing.length} key(s) that en.json has, so ` +
        `those strings render in English to ${code} users: ${missing
          .slice(0, 8)
          .join(", ")}`,
    );

    // An extra key is only legitimate when it is another PLURAL form of a key
    // English already has: ru and pl carry _few/_many that en does not need.
    const extra = [...bundle.keys()].filter((key) => {
      if (english.has(key)) return false;
      const base = key.replace(PLURAL_SUFFIX, "");
      return !english.has(`${base}_other`) && !english.has(`${base}_one`);
    });
    assert.deepEqual(
      extra,
      [],
      `${code}.json has ${extra.length} key(s) en.json does not, and they are ` +
        `not plural forms of an English key, dead weight or a typo: ${extra
          .slice(0, 8)
          .join(", ")}`,
    );
  }
});

test("no locale fakes a translation with an empty string", () => {
  // An empty value satisfies a key-set check and renders as NOTHING: a blank
  // button, a dialog with no title. If a phrase does not apply in a language,
  // the fix is one interpolated key, not a blank.
  for (const [code, bundle] of bundles) {
    const blank = [...bundle]
      .filter(([, value]) => typeof value === "string" && value.trim() === "")
      .map(([key]) => key);
    assert.deepEqual(
      blank,
      [],
      `${code}.json has ${blank.length} empty string(s), which render as ` +
        `nothing at all: ${blank.slice(0, 8).join(", ")}`,
    );
  }
});

test("interpolation placeholders survive every translation", () => {
  // A translator dropping {{detail}} does not fail a key-set check, and the
  // sentence silently loses the only part that says what went wrong.
  for (const [code, bundle] of bundles) {
    if (code === "en") continue;
    const broken = [];
    for (const [key, value] of english) {
      const expected = placeholders(value);
      if (expected.size === 0) continue;
      const actual = placeholders(bundle.get(key));
      if (
        expected.size !== actual.size ||
        [...expected].some((name) => !actual.has(name))
      ) {
        broken.push(`${key} (expected ${[...expected].join(",")})`);
      }
    }
    assert.deepEqual(
      broken,
      [],
      `${code}.json changes or drops interpolation placeholders: ${broken
        .slice(0, 8)
        .join("; ")}`,
    );
  }
});

/** Comments may legally hold a `;` or a quoted CODE, so they come out first. */
const stripComments = (source) =>
  source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "");

test("every backend error code the frontend knows has a sentence in every locale", () => {
  // The union is the list of codes Rust can send. A code that reaches
  // translateBackendError with no key renders the raw identifier to the user.
  const source = readFileSync(path.join(HERE, "backend-errors.ts"), "utf8");

  // Comments are stripped BEFORE the terminating `;` is located. A doc comment
  // inside the union contains the prose "...precondition; without a case here",
  // and that semicolon used to end the slice early, so the final thirteen codes
  //, every Wayfern fingerprint failure, LAUNCH_CONSENT_EXPIRED, INTERNAL_ERROR
  //, were silently never checked by the gate that exists to check them.
  const declaration = stripComments(
    source.slice(source.indexOf("export type BackendErrorCode")),
  );
  const union = declaration.slice(0, declaration.indexOf(";"));
  const codes = [...union.matchAll(/"([A-Z0-9_]+)"/g)].map((m) => m[1]);

  // Resolve each `case` to the key(s) it actually returns instead of assuming
  // camelCase. Three codes deliberately share or rename a key
  // (COOKIE_BOT_REQUIRES_PROXY reuses the EXIT_NODE sentence, INTERNAL_ERROR
  // uses `internal`, IMPORT_SOURCE_NOT_CHROMIUM picks a `Named` variant when it
  // has the family), and a camelCase guess reports those as false alarms while
  // proving nothing about the key the user actually gets.
  const body = stripComments(
    source.slice(source.indexOf("switch (parsed.code)")),
  );
  const cases = body.slice(0, body.indexOf("default:"));
  const keysByCode = new Map();
  let pending = [];
  let seenKey = false;
  for (const match of cases.matchAll(
    /case\s+"([A-Z0-9_]+)"\s*:|\bt\(\s*"([\w.]+)"/g,
  )) {
    if (match[1]) {
      // A key since the last label means this starts a fresh group; without the
      // reset, stacked labels would keep absorbing every later case's keys.
      if (seenKey) {
        pending = [];
        seenKey = false;
      }
      pending.push(match[1]);
      if (!keysByCode.has(match[1])) keysByCode.set(match[1], []);
      continue;
    }
    if (pending.length === 0) continue;
    seenKey = true;
    for (const code of pending) keysByCode.get(code).push(match[2]);
  }

  // Cross-checking both directions is what makes a magic minimum unnecessary:
  // if either parse silently stops reading, the two sets stop agreeing.
  const unhandled = codes.filter((code) => !keysByCode.has(code));
  assert.deepEqual(
    unhandled,
    [],
    `${unhandled.length} code(s) in BackendErrorCode have no case in ` +
      `translateBackendError, so they reach the user as the raw identifier: ` +
      unhandled.slice(0, 8).join(", "),
  );
  const undeclared = [...keysByCode.keys()].filter(
    (code) => !codes.includes(code),
  );
  assert.deepEqual(
    undeclared,
    [],
    `${undeclared.length} case(s) in translateBackendError are absent from the ` +
      `union, so either the union parse truncated or the code is dead: ` +
      undeclared.slice(0, 8).join(", "),
  );

  const keys = [...new Set([...keysByCode.values()].flat())];
  for (const [locale, bundle] of bundles) {
    const missing = keys.filter((key) => !bundle.has(key));
    assert.deepEqual(
      missing,
      [],
      `${locale}.json has no backendErrors entry for ${missing.length} key(s) ` +
        `translateBackendError can ask for: ${missing.slice(0, 8).join(", ")}`,
    );
  }
});

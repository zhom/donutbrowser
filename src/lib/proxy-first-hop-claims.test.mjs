import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { isFirstHopEncrypted } from "./proxy-string.ts";

/**
 * Three screens describe the same stored proxy's first hop, and they must not
 * disagree about it.
 *
 * Shadowsocks is the only type whose hop is decided by something other than the
 * type: its cipher. A proxy can reach any of these screens with no cipher
 * recorded at all, `proxy_manager.rs` parses `ss://host:8388` (no `@`) into
 * username None, and `import_proxies_json` writes proxy_type and username
 * verbatim with no validation, and with none recorded the honest answer is
 * "undecided", not "in the clear".
 *
 * The add/edit form was taught that. The management table and the import
 * preview were not, so for one proxy the app said "the cipher decides" on one
 * screen and "the connection to this proxy is not encrypted" on the other.
 * Nothing caught it: the three surfaces each phrase the claim themselves, and
 * no test read more than one of them.
 *
 * This is that gate. It reads the component sources, because the claim lives in
 * which translation key each branch reaches for, and there is no React test
 * harness here to render them.
 */

const HERE = path.dirname(fileURLToPath(import.meta.url));
const COMPONENTS = path.join(HERE, "..", "components");
const LOCALES = path.join(HERE, "..", "i18n", "locales");

/**
 * Every screen that can call a first hop unencrypted, with the key it uses for
 * that claim and the key it must use for the undecided one.
 */
const SURFACES = [
  {
    file: "proxy-form-dialog.tsx",
    inTheClear: "proxies.form.firstHopPlaintextNote",
    undecided: "proxies.form.firstHopCipherNote",
  },
  {
    file: "proxy-management-dialog.tsx",
    inTheClear: "proxies.management.firstHopPlaintextTooltip",
    undecided: "proxies.management.firstHopCipherTooltip",
  },
  {
    file: "proxy-import-dialog.tsx",
    inTheClear: "proxies.importDialog.plaintextFirstHopCount",
    undecided: "proxies.importDialog.cipherUndecidedCount",
  },
];

/**
 * Block comments go first, then whole-line `//` ones. A trailing-`//` rule would
 * eat the `://` this dialog prints after a scheme, and the prose in these files
 * names the very keys being searched for, a comment mentioning a key must not
 * pass for a branch that reaches it.
 */
const stripComments = (source) =>
  source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/[^\n]*$/gm, "");

const sources = SURFACES.map((surface) => ({
  ...surface,
  source: stripComments(
    readFileSync(path.join(COMPONENTS, surface.file), "utf8"),
  ),
}));

test("the surfaces are actually there and actually read", () => {
  // Without this every assertion below is vacuously true the moment a file is
  // renamed or the comment stripper eats the whole body, the exact way a gate
  // like this goes green while checking nothing.
  assert.equal(sources.length, 3);
  for (const { file, source } of sources) {
    assert.ok(
      source.length > 2000,
      `${file} came back as ${source.length} chars`,
    );
  }
});

test("every screen that can call a first hop unencrypted still makes that claim", () => {
  // The pairing below only means something while both halves are live. If a
  // screen stops saying "in the clear" at all, this file is pinning a claim
  // nobody makes any more and must be updated rather than left to pass.
  for (const { file, inTheClear, source } of sources) {
    assert.ok(
      source.includes(inTheClear),
      `${file} no longer reaches for ${inTheClear}; this gate is now vacuous ` +
        "for that screen",
    );
  }
});

test("no screen can call a first hop unencrypted without being able to call it undecided", () => {
  for (const { file, inTheClear, undecided, source } of sources) {
    assert.ok(
      source.includes(undecided),
      `${file} reaches for ${inTheClear} but never for ${undecided}, so a ` +
        "Shadowsocks proxy with no cipher recorded is reported there as " +
        "plaintext while the add/edit form reports the same proxy as " +
        "undecided, the app contradicting itself about one proxy",
    );
  }
});

test("the undecided branch is keyed off the canonical type, not a bare spelling", () => {
  // `POST /v1/proxies` and `import_proxies_json` store proxy_type verbatim, so
  // the same Shadowsocks proxy arrives as `ss`, `shadowsocks`, or `"ss "`. A
  // screen comparing the raw string answers for one spelling and quietly calls
  // the others plaintext again.
  for (const { file, source } of sources) {
    assert.ok(
      source.includes("canonicalProxyType"),
      `${file} decides the undecided branch without canonicalProxyType`,
    );
    // The form canonicalises once into a local and compares that, the other two
    // call it inline; both are correct, and only a raw stored `proxy_type` on
    // the left is not.
    const canonicalised = [
      ...source.matchAll(/(?:const|let)\s+(\w+)\s*=\s*canonicalProxyType\(/g),
    ].map((match) => match[1]);
    const bare = source.split("\n").filter((line) => {
      const comparison = line.search(/===\s*"ss"/);
      if (comparison === -1) return false;
      const left = line.slice(0, comparison);
      return (
        !left.includes("canonicalProxyType(") &&
        !canonicalised.some((name) => new RegExp(`\\b${name}\\b`).test(left))
      );
    });
    assert.deepEqual(
      bare,
      [],
      `${file} compares a stored proxy_type to "ss" without canonicalising ` +
        "it first, so the `shadowsocks` spelling misses the branch",
    );
  }
});

test("both keys of every pair have a sentence in every locale", () => {
  // A branch reaching for a key no locale carries renders the raw identifier.
  // i18n-parity checks the locales against each other; nothing checks that a
  // key a component actually asks for exists in the first place.
  const wanted = SURFACES.flatMap(({ inTheClear, undecided }) => [
    inTheClear,
    undecided,
  ]);
  const codes = ["en", "es", "fr", "ja", "ko", "pt", "ru", "tr", "vi", "zh"];
  for (const code of codes) {
    const bundle = JSON.parse(
      readFileSync(path.join(LOCALES, `${code}.json`), "utf8"),
    );
    const missing = wanted.filter((key) => {
      const value = key
        .split(".")
        .reduce((node, part) => (node == null ? node : node[part]), bundle);
      return typeof value !== "string" || value.trim() === "";
    });
    assert.deepEqual(
      missing,
      [],
      `${code}.json has no sentence for ${missing.join(", ")}`,
    );
  }
});

test("an undecided cipher still fails closed", () => {
  // This change is about what the UI CLAIMS. The guard every screen colours and
  // branches on is unchanged: absent evidence of encryption is never treated as
  // encryption, whatever the sentence beside it now says.
  for (const spelling of ["ss", "shadowsocks", "SS", " ss "]) {
    for (const cipher of [undefined, null, "", "   ", "none", "plain"]) {
      assert.equal(
        isFirstHopEncrypted(spelling, cipher),
        false,
        `isFirstHopEncrypted(${spelling}, ${String(cipher)}) must stay false`,
      );
    }
    assert.equal(isFirstHopEncrypted(spelling, "aes-256-gcm"), true);
  }
});

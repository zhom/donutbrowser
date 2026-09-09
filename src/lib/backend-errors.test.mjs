import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { translateBackendError } from "./backend-errors.ts";

/**
 * The remote-control bridge reports failures as codes, not sentences.
 *
 * `mcp_remote.rs` used to put its own English prose into `McpRemoteStatus`, and
 * some of that prose was the RELAY'S close reason: server-authored English
 * rendered verbatim under a UI that ships in ten languages. It now sends a
 * `{"code": …}` envelope instead, which only works if three things line up: the
 * code Rust emits, a `case` in the switch, and a key in every locale file.
 *
 * A Rust test checks the first two structurally, by grepping for the `case`.
 * Nothing checked that the chain actually PRODUCES A SENTENCE, and the failure
 * mode is silent, because `translateBackendError` falls back to `String(err)`,
 * so a missing case renders the raw JSON envelope to the customer rather than
 * throwing. This runs the real translator against the real locale files.
 */

const LOCALES = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "i18n",
  "locales",
);

/** The codes `BridgeError::code()` in src-tauri/src/mcp_remote.rs can emit. */
const BRIDGE_CODES = [
  "MCP_REMOTE_UNAUTHORIZED",
  "MCP_REMOTE_SLOT_TAKEN",
  "MCP_REMOTE_NOT_ENTITLED",
  "MCP_REMOTE_UNREACHABLE",
  "MCP_REMOTE_REQUIRES_SIGN_IN",
];

/**
 * The codes the remote credential path emits: `cloud_auth.rs` for a refused
 * mint, `lib.rs::mcp_target_for` for an install with nothing stored. They
 * reach the same dialog through the same translator, so they get the same
 * guarantee.
 */
const CREDENTIAL_CODES = [
  "MCP_REMOTE_KEY_MISSING",
  "MCP_REMOTE_KEY_LIMIT",
  "MCP_REMOTE_KEY_UNAVAILABLE",
];

const REMOTE_CODES = [...BRIDGE_CODES, ...CREDENTIAL_CODES];

function localeFiles() {
  return readdirSync(LOCALES)
    .filter((name) => name.endsWith(".json"))
    .map((name) => [
      name.replace(/\.json$/, ""),
      JSON.parse(readFileSync(path.join(LOCALES, name), "utf8")),
    ]);
}

/** A `t` that resolves against a real locale, and refuses to invent anything. */
function translatorFor(bundle, locale) {
  return (key, params) => {
    const value = key
      .split(".")
      .reduce((node, part) => (node == null ? undefined : node[part]), bundle);
    assert.equal(
      typeof value,
      "string",
      `${locale} has no string at ${key}; the UI would render the raw code`,
    );
    if (!params) return value;
    return Object.entries(params).reduce(
      (text, [name, replacement]) =>
        text.replaceAll(`{{${name}}}`, String(replacement)),
      value,
    );
  };
}

test("every remote MCP error code resolves to a sentence in every locale", () => {
  const locales = localeFiles();
  assert.ok(locales.length >= 10, "expected at least ten locale files");

  for (const [locale, bundle] of locales) {
    const t = translatorFor(bundle, locale);
    for (const code of REMOTE_CODES) {
      // Exactly what `set_state` puts on the wire: `crate::backend_error(code)`.
      const envelope = JSON.stringify({ code });
      const rendered = translateBackendError(t, envelope);

      assert.ok(
        rendered.length > 0,
        `${locale}/${code} rendered an empty string`,
      );
      // The fallback is `String(err)`, so an unhandled code shows the customer
      // the JSON envelope. That is the exact silent failure being guarded.
      assert.ok(
        !rendered.includes(code),
        `${locale}/${code} fell through to the raw envelope: ${rendered}`,
      );
      assert.ok(
        !rendered.startsWith("{"),
        `${locale}/${code} rendered JSON: ${rendered}`,
      );
    }
  }
});

test("each remote MCP code says something DIFFERENT, in every locale", () => {
  // Each code exists because its situation needs its own answer: sign in
  // again, close the other copy of Donut, upgrade the plan, just wait, create
  // a credential first, revoke one on the account page. Mapping two of them
  // onto one sentence would leave a customer doing the wrong thing about a
  // problem the app already knew how to name.
  for (const [locale, bundle] of localeFiles()) {
    const t = translatorFor(bundle, locale);
    const rendered = REMOTE_CODES.map((code) =>
      translateBackendError(t, JSON.stringify({ code })),
    );
    assert.equal(
      new Set(rendered).size,
      REMOTE_CODES.length,
      `${locale} reuses one sentence for two different remote MCP failures: ${JSON.stringify(rendered)}`,
    );
  }
});

test("an unknown code still renders something rather than throwing", () => {
  const [, bundle] = localeFiles()[0];
  const t = translatorFor(bundle, "en");
  // A desktop newer than the UI can send a code this build has never heard of.
  // Falling back to the envelope is ugly, but it must not crash the dialog.
  const rendered = translateBackendError(t, '{"code":"SOMETHING_NEWER"}');
  assert.equal(typeof rendered, "string");
  assert.ok(rendered.length > 0);
});

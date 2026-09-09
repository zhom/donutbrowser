import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { canonicalProxyType } from "./proxy-type.ts";

/**
 * `proxy_type` is stored verbatim, so the UI meets both Shadowsocks spellings.
 * The unit tests below pin the helper; the source tests after them pin the
 * thing the helper exists for, because the add/edit form is a `.tsx` that
 * `node --test` cannot import and every one of these defects lived in a
 * comparison, not in a computation.
 */

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIALOG = path.join(HERE, "..", "components", "proxy-form-dialog.tsx");
const source = readFileSync(DIALOG, "utf8");

test("both Shadowsocks spellings answer as one type", () => {
  // `POST /v1/proxies` and `import_proxies_json` both accept `shadowsocks` and
  // store it as sent, and `proxy_server.rs` tunnels it: matching only `ss` in
  // the UI describes a proxy the backend is perfectly happy to run.
  assert.equal(canonicalProxyType("ss"), "ss");
  assert.equal(canonicalProxyType("shadowsocks"), "ss");
  assert.equal(canonicalProxyType("Shadowsocks"), "ss");
  assert.equal(canonicalProxyType("  SS  "), "ss");
});

test("every other type keeps its own identity", () => {
  for (const type of ["http", "https", "httpstls", "socks4", "socks5", "vless"])
    assert.equal(canonicalProxyType(type), type);
  assert.equal(canonicalProxyType("HTTPSTLS"), "httpstls");
  assert.equal(canonicalProxyType("VLESS"), "vless");
  // `socks` is not `socks5` to `Url::scheme()`, so folding it here would have
  // the UI describe a hop the Rust worker never dials.
  assert.equal(canonicalProxyType("socks"), "socks");
  assert.equal(canonicalProxyType("nonsense"), "nonsense");
  assert.equal(canonicalProxyType(""), "");
});

test("a type that names a prototype member is answered as text, not as a prototype", () => {
  // `proxy_type` is free text: POST /v1/proxies and import_proxies_json store
  // whatever they are handed. An object-literal alias table answered for the
  // whole prototype chain, so `__proto__` canonicalised to Object.prototype and
  // `constructor` to a function, both truthy, so the `??` never fired and the
  // declared `: string` return was a lie TypeScript cannot catch. The form
  // branches on this value, and the management table renders the type it picks;
  // React throws on an object child (which unmounts the whole table) and
  // silently drops a function.
  for (const hostile of [
    "__proto__",
    "constructor",
    "toString",
    "valueOf",
    "hasOwnProperty",
    "isPrototypeOf",
  ]) {
    const canonical = canonicalProxyType(hostile);
    assert.equal(
      typeof canonical,
      "string",
      `${hostile} must canonicalise to a string, not to whatever the ` +
        "prototype chain holds under that name",
    );
    // Lowercased like any other unknown type, and otherwise untouched: an
    // unrecognised spelling still compares predictably instead of vanishing
    // into a default.
    assert.equal(canonical, hostile.toLowerCase());
  }

  // The same through the trim/case-fold path, which is where a hostile value
  // arrives from a stored proxy rather than from a literal.
  assert.equal(canonicalProxyType("  __PROTO__  "), "__proto__");
  assert.notEqual(canonicalProxyType("__proto__"), canonicalProxyType("ss"));
});

test("the proxy form never compares a stored proxy_type to one spelling", () => {
  // The whole defect class in one assertion. `form.proxy_type === "ss"` typed
  // the cipher field as a username, skipped its required check and left the
  // type Select on its placeholder for any proxy stored as `shadowsocks`; the
  // same shape guarded the TLS hint and the VLESS branch. Reverting any of them
  // to a literal comparison fails here.
  const comparisons = [
    ...source.matchAll(/proxy_type\s*[!=]==\s*"[^"]*"/g),
  ].map((match) => match[0]);
  assert.deepEqual(
    comparisons,
    [],
    `proxy-form-dialog.tsx compares a stored proxy_type to a literal ` +
      `${comparisons.length} time(s); route it through canonicalProxyType so ` +
      `an alias spelling cannot slip past: ${comparisons.join(", ")}`,
  );
  assert.ok(
    source.includes("canonicalProxyType(form.proxy_type)"),
    "the form must derive its type through canonicalProxyType",
  );
});

test("the type Select offers the spelling the proxy is actually stored as", () => {
  // Radix matches an item by value. With every item hardcoded to the canonical
  // spelling, opening a `shadowsocks` proxy showed the placeholder, and the
  // only way out of that was picking an item that retyped the proxy to `ss`.
  assert.match(
    source,
    /const typeItemValue = \(type: string\) =>\s*type === canonicalType \? form\.proxy_type : type;/,
    "the item standing for the current proxy must carry the proxy's own " +
      "proxy_type, so selecting it cannot rewrite the stored type",
  );
  const items = [
    ...source.matchAll(/<SelectItem key=\{type\} value=\{([^}]+)\}/g),
  ].map((match) => match[1].trim());
  assert.ok(items.length > 0, "the type Select must render items at all");
  assert.deepEqual(
    [...new Set(items)],
    ["typeItemValue(type)"],
    "every type group must render through typeItemValue, however the groups " +
      "are arranged",
  );
});

test("Shadowsocks is not filed under a heading that promises encryption", () => {
  // `ss` sat in the encrypted group while the note 40 pixels below read "not
  // encrypted", which it always did the moment the type was picked, because
  // the cipher that decides is empty until the user types one. The heading and
  // the note contradicted each other in a single glance, about the one field
  // this whole panel exists to explain.
  const group = (name) =>
    source.match(new RegExp(`const ${name} = \\[([^\\]]*)\\]`))?.[1];
  const always = group("ALWAYS_ENCRYPTED_FIRST_HOP_TYPES");
  const dependent = group("CIPHER_DEPENDENT_FIRST_HOP_TYPES");
  assert.ok(
    always && dependent,
    "the type list must separate the types that always encrypt the first hop " +
      "from the ones whose cipher decides",
  );
  assert.ok(
    !/"ss"/.test(always),
    `the unconditional "First hop encrypted" group must not claim Shadowsocks: ${always}`,
  );
  assert.match(dependent, /"ss"/);
  assert.ok(
    source.includes("proxies.form.firstHopGroupCipher"),
    "the cipher-dependent group needs its own translated heading",
  );
});

test("an empty cipher is described as undecided, and still fails closed", () => {
  assert.match(
    source,
    /const cipherUndecided =\s*isShadowsocks && form\.username\.trim\(\)\.length === 0;/,
    "the form must know the difference between a cipher that is empty and one " +
      "that encrypts nothing",
  );

  const note = source.slice(source.indexOf('id="proxy-type-first-hop"'));
  const body = note.slice(0, note.indexOf("</p>"));
  assert.ok(
    body.includes("proxies.form.firstHopCipherNote"),
    "the note must have a sentence for the cipher nobody has chosen yet",
  );
  assert.ok(
    body.indexOf("cipherUndecided") < body.indexOf("firstHopPlaintextNote"),
    "the undecided-cipher sentence must be reached BEFORE the flat claim that " +
      "the hop is not encrypted, or the contradiction stands",
  );

  // The fail-closed half. `cipherUndecided` softens one SENTENCE and nothing
  // else: `firstHopEncrypted` stays false for an empty cipher, so the warning
  // colour, the exposure panel and the submit guard are all untouched. Reading
  // it anywhere else would turn "we do not know yet" into "it is fine".
  const stripped = source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/[^\n]*/g, "");
  assert.equal(
    [...stripped.matchAll(/cipherUndecided/g)].length,
    2,
    "cipherUndecided may only be declared and used to pick that one sentence",
  );
  assert.ok(
    stripped.includes("isFirstHopEncrypted(canonicalType, form.username)"),
    "the encryption answer must come from the same canonical type the field " +
      "labels use, so a padded stored type cannot make them disagree",
  );
});

test("the plaintext warning tells a Shadowsocks user about the cipher, not credentials", () => {
  // With cipher `none` the first hop is unencrypted and the cipher field is
  // non-empty, so the panel fired and claimed a username and password were
  // crossing the network readable. Shadowsocks never puts either on the wire.
  // The exposure is the destination and the payload, and the fix a user should
  // reach for is a real cipher.
  const panel = source.slice(source.indexOf("showPlaintextExposure &&"));
  const body = panel.slice(0, panel.indexOf("</div>"));
  assert.ok(
    body.includes("isShadowsocks"),
    "the panel must pick its sentence from the type; a single sentence is " +
      "wrong for one of the two protocols that can reach it",
  );
  for (const key of ["nullCipherHeading", "nullCipherBody"]) {
    assert.ok(
      body.includes(`proxies.form.${key}`),
      `the Shadowsocks branch must use proxies.form.${key}`,
    );
  }
  for (const key of ["credentialsInClearHeading", "credentialsInClearBody"]) {
    assert.ok(
      body.includes(`proxies.form.${key}`),
      `the credentialed branch must keep proxies.form.${key}`,
    );
  }
});

test("the Shadowsocks sentences name the cipher, not the credentials", () => {
  // A key that exists but repeats the credentials claim would pass every check
  // above while telling the user exactly the wrong thing to change.
  const locales = path.join(HERE, "..", "i18n", "locales", "en.json");
  const form = JSON.parse(readFileSync(locales, "utf8")).proxies.form;
  assert.match(form.nullCipherHeading, /cipher/i);
  assert.match(form.nullCipherBody, /cipher/i);
  assert.ok(
    !/username/i.test(form.nullCipherBody),
    "Shadowsocks has no username to expose; saying so sends the user to the " +
      "wrong field",
  );

  // The undecided-cipher sentences replace a claim the form could not support.
  // A key that exists but repeats "the connection is not encrypted" would pass
  // every structural check above and leave the contradiction exactly where it
  // was, so the sentence itself is pinned: it must name the cipher, and it must
  // not settle the question.
  assert.match(form.firstHopGroupCipher, /cipher/i);
  assert.match(form.firstHopCipherNote, /cipher/i);
  assert.ok(
    !/is not encrypted/i.test(form.firstHopCipherNote),
    "the sentence for an empty cipher must not assert the hop is in the clear; " +
      "the cipher has not been chosen yet",
  );
  // It must still say what an unset or null cipher means, or softening the
  // claim would have cost the user the warning.
  assert.match(form.firstHopCipherNote, /empty|none|null/i);
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  CREDENTIALS_FIRST_FORMAT,
  HOST_FIRST_FORMAT,
  isFirstHopEncrypted,
  pickParsedProxy,
  proxyProtocolToken,
  resolveAmbiguousProxyLine,
  splitProxyScheme,
} from "./proxy-string.ts";
import { canonicalProxyType } from "./proxy-type.ts";

/**
 * The formats themselves are exercised in Rust
 * (`proxy_manager::tests::test_proxy_txt_parsing_various_formats`). What is
 * pinned here is the frontend's half: the scheme survives an ambiguous line,
 * and a resolution that doesn't fit the line is refused rather than turned into
 * a proxy pointing at somebody's password.
 */

test("a bare line is HTTP", () => {
  assert.deepEqual(splitProxyScheme("1.2.3.4:8080"), {
    proxyType: "http",
    rest: "1.2.3.4:8080",
  });
});

test("known schemes are recognised and normalised", () => {
  assert.deepEqual(splitProxyScheme("SOCKS://1.2.3.4:1080"), {
    proxyType: "socks5",
    rest: "1.2.3.4:1080",
  });
  assert.equal(splitProxyScheme("shadowsocks://host:8388").proxyType, "ss");
});

test("an unknown scheme is left in the body rather than guessed at", () => {
  assert.deepEqual(splitProxyScheme("ftp://1.2.3.4:21"), {
    proxyType: "http",
    rest: "ftp://1.2.3.4:21",
  });
});

test("a scheme that names a prototype member is still an unknown scheme", () => {
  // The scheme comes from text the user pasted. An object-literal table
  // answered for the whole prototype chain, so `__proto__` matched with
  // Object.prototype and `constructor` with a function, both truthy, so the
  // body was stripped and a non-string proxy_type reached the import preview,
  // where React refuses an object as a child.
  for (const hostile of [
    "__proto__",
    "constructor",
    "toString",
    "valueOf",
    "hasOwnProperty",
  ]) {
    const line = `${hostile}://1.2.3.4:8080`;
    assert.deepEqual(splitProxyScheme(line), { proxyType: "http", rest: line });
  }

  // ...so the line still carries its five colon-parts and cannot be read as a
  // four-part ambiguous one. Before, the scheme was stripped and this minted a
  // proxy whose proxy_type was Object.prototype.
  assert.equal(
    resolveAmbiguousProxyLine(
      "__proto__://1.2.3.4:8080:u:p",
      HOST_FIRST_FORMAT,
    ),
    null,
  );
});

test("the protocol label shortens what needs it and prints the rest verbatim", () => {
  assert.equal(proxyProtocolToken("httpstls"), "HTTP/TLS");
  assert.equal(proxyProtocolToken("HTTPSTLS"), "HTTP/TLS");
  assert.equal(proxyProtocolToken("socks5"), "socks5");
  // An unrecognised type is the point: `proxy_type` is free text that
  // POST /v1/proxies and import_proxies_json store verbatim, and the column
  // must show what was actually stored rather than nothing.
  assert.equal(proxyProtocolToken("weird-vendor-type"), "weird-vendor-type");
  // Blank is the one empty answer, and it is the caller's cue to render a
  // translated placeholder instead of an empty cell.
  assert.equal(proxyProtocolToken("   "), "");
});

test("the protocol label answers for protocols only, never for a prototype", () => {
  // `PROTOCOL_TOKENS[proxyType]` walked the prototype chain: "__proto__" gave
  // an object, which React refuses as a child, and the throw unmounted the
  // WHOLE proxy table, every proxy and every first-hop warning gone.
  // "constructor", "toString" and "valueOf" gave functions, which React drops,
  // leaving the protocol cell blank.
  for (const hostile of [
    "__proto__",
    "constructor",
    "toString",
    "valueOf",
    "hasOwnProperty",
    "isPrototypeOf",
  ]) {
    const label = proxyProtocolToken(hostile);
    assert.equal(typeof label, "string", `${hostile} must render as text`);
    assert.equal(label, hostile);
  }
});

test("host-first resolution keeps the scheme", () => {
  assert.deepEqual(
    resolveAmbiguousProxyLine(
      "socks5://1234:5678:9012:3456",
      HOST_FIRST_FORMAT,
    ),
    {
      proxy_type: "socks5",
      host: "1234",
      port: 5678,
      username: "9012",
      password: "3456",
      original_line: "socks5://1234:5678:9012:3456",
    },
  );
});

test("credentials-first resolution reads the tail as the endpoint", () => {
  assert.deepEqual(
    resolveAmbiguousProxyLine("1234:5678:9012:3456", CREDENTIALS_FIRST_FORMAT),
    {
      proxy_type: "http",
      host: "9012",
      port: 3456,
      username: "1234",
      password: "5678",
      original_line: "1234:5678:9012:3456",
    },
  );
});

test("a format that doesn't fit the line resolves to nothing", () => {
  // 70000 is past the port range, so this ordering cannot be the right one.
  assert.equal(
    resolveAmbiguousProxyLine("host:70000:user:pass", HOST_FIRST_FORMAT),
    null,
  );
  assert.equal(resolveAmbiguousProxyLine("host:8080", HOST_FIRST_FORMAT), null);
  assert.equal(
    resolveAmbiguousProxyLine("a:1:b:2", "host:port:user:password"),
    null,
  );
});

test("the first parsed line of a multi-line paste wins", () => {
  const parsed = pickParsedProxy([
    { status: "invalid", line: "notaproxy", reason: "nope" },
    {
      status: "parsed",
      proxy_type: "socks5",
      host: "1.2.3.4",
      port: 1080,
      username: "u",
      password: "p",
      original_line: "socks5://u:p@1.2.3.4:1080",
    },
    {
      status: "parsed",
      proxy_type: "http",
      host: "5.6.7.8",
      port: 80,
      original_line: "5.6.7.8:80",
    },
  ]);
  assert.equal(parsed?.host, "1.2.3.4");
  assert.equal(parsed?.proxy_type, "socks5");
});

test("an ambiguous paste falls back to host:port:username:password", () => {
  const parsed = pickParsedProxy([
    {
      status: "ambiguous",
      line: "1234:5678:9012:3456",
      possible_formats: [HOST_FIRST_FORMAT, CREDENTIALS_FIRST_FORMAT],
    },
  ]);
  assert.equal(parsed?.host, "1234");
  assert.equal(parsed?.port, 5678);
});

test("nothing usable yields null so the plain paste stands", () => {
  assert.equal(
    pickParsedProxy([
      { status: "invalid", line: "proxy.example.com", reason: "" },
    ]),
    null,
  );
  assert.equal(pickParsedProxy([]), null);
});

test("the TLS-wrapped scheme survives a paste", () => {
  assert.deepEqual(splitProxyScheme("httpstls://proxy.example.com:443"), {
    proxyType: "httpstls",
    rest: "proxy.example.com:443",
  });
  assert.equal(
    splitProxyScheme("HTTPSTLS://proxy.example.com:443").proxyType,
    "httpstls",
  );
});

test("only the types that actually encrypt the first hop say they do", () => {
  // `ss` is deliberately NOT in this list: its answer depends on the cipher,
  // and a type-only query carries no cipher, so it cannot claim encryption.
  for (const encrypted of ["httpstls", "vless", "HTTPSTLS", "VLESS"]) {
    assert.equal(isFirstHopEncrypted(encrypted), true, encrypted);
  }
  assert.equal(isFirstHopEncrypted("ss", "aes-256-gcm"), true);
  assert.equal(isFirstHopEncrypted("SS", "chacha20-ietf-poly1305"), true);
  // The REST API stores the type verbatim, so both spellings arrive, and both
  // must answer identically or the UI contradicts the wire.
  assert.equal(isFirstHopEncrypted("shadowsocks", "aes-256-gcm"), true);
  assert.equal(isFirstHopEncrypted("shadowsocks", "none"), false);
  assert.equal(isFirstHopEncrypted("shadowsocks"), false);

  // `https` is the one that matters. It is a provider label on a plaintext
  // CONNECT endpoint, dialed byte-for-byte like `http`, so the UI must not
  // present it as encrypted.
  for (const plaintext of ["http", "https", "HTTPS", "socks4", "socks5", ""]) {
    assert.equal(isFirstHopEncrypted(plaintext), false, plaintext);
  }

  // Shadowsocks takes its cipher as free text and shadowsocks-crypto maps
  // "none"/"plain" to a no-op cipher. Such a proxy was labelled "First hop
  // encrypted", coloured safe, and had its credentials-in-clear warning
  // suppressed, while carrying everything in the clear.
  for (const nullCipher of ["none", "plain", "NONE", " None "]) {
    assert.equal(
      isFirstHopEncrypted("ss", nullCipher),
      false,
      `ss with cipher ${nullCipher} is not encrypted`,
    );
  }
  for (const real of ["aes-256-gcm", "chacha20-ietf-poly1305"]) {
    assert.equal(isFirstHopEncrypted("ss", real), true, real);
  }
  // An unspecified cipher is not a claim of safety either.
  assert.equal(isFirstHopEncrypted("ss", null), false);
  // Types whose encryption does not depend on a cipher are unaffected.
  assert.equal(isFirstHopEncrypted("vless", null), true);
});

test("a padded stored type answers the same as an unpadded one", () => {
  // `proxy_type` is stored verbatim, so `"ss "` is a value a proxy really
  // arrives with from POST /v1/proxies or import_proxies_json. This helper
  // lowercased but did not trim, while canonicalProxyType trimmed, so one such
  // proxy was Shadowsocks to the form, which labelled its field "Cipher" -
  // and an UNENCRYPTED first hop to the warning printed beside that field,
  // about a proxy configured with aes-256-gcm. A warning that fires on a
  // correct configuration is how a user learns to ignore this warning.
  assert.equal(isFirstHopEncrypted("ss ", "aes-256-gcm"), true);
  assert.equal(isFirstHopEncrypted(" shadowsocks", "aes-256-gcm"), true);
  assert.equal(isFirstHopEncrypted("\thttpstls\n"), true);
  assert.equal(isFirstHopEncrypted(" VLESS "), true);
  // Padding does not buy encryption either: the cipher still decides, and the
  // types that never encrypt still never do.
  assert.equal(isFirstHopEncrypted(" ss ", ""), false);
  assert.equal(isFirstHopEncrypted(" ss ", " none "), false);
  assert.equal(isFirstHopEncrypted(" https "), false);
  assert.equal(proxyProtocolToken(" httpstls "), "HTTP/TLS");
});

test("the first-hop answer never disagrees with the canonical type", () => {
  // The lockstep the two helpers cannot get from a shared import: both files
  // are loaded straight by `node --test`, which resolves neither `@/` nor an
  // extensionless relative specifier, so neither may import the other. This
  // test can import both, and it is what keeps their normalisation identical.
  // Any two spellings of one type must get one answer, and it must be the
  // answer the canonical spelling gets.
  const spellings = [
    "ss",
    "SS",
    " ss ",
    "shadowsocks",
    " Shadowsocks ",
    "httpstls",
    " HTTPSTLS ",
    "vless",
    " VLess ",
    "http",
    " HTTPS ",
    "socks5",
    " socks4 ",
    "__proto__",
    "",
    "   ",
  ];
  const answers = new Map();
  for (const spelling of spellings) {
    const canonical = canonicalProxyType(spelling);
    const answer = isFirstHopEncrypted(spelling, "aes-256-gcm");
    assert.equal(
      isFirstHopEncrypted(canonical, "aes-256-gcm"),
      answer,
      `${JSON.stringify(spelling)} and its canonical form ${JSON.stringify(
        canonical,
      )} describe different wires`,
    );
    if (answers.has(canonical)) {
      assert.equal(
        answer,
        answers.get(canonical),
        `${JSON.stringify(spelling)} answers differently from another ` +
          `spelling of ${JSON.stringify(canonical)}`,
      );
    } else {
      answers.set(canonical, answer);
    }
  }
  // The corpus has to have covered both answers, or the loop proves nothing.
  assert.equal(answers.get("ss"), true);
  assert.equal(answers.get("httpstls"), true);
  assert.equal(answers.get("http"), false);
  assert.equal(answers.get("https"), false);
  assert.equal(answers.get("__proto__"), false);
});

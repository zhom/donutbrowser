import assert from "node:assert/strict";
import test from "node:test";
import {
  CREDENTIALS_FIRST_FORMAT,
  HOST_FIRST_FORMAT,
  pickParsedProxy,
  resolveAmbiguousProxyLine,
  splitProxyScheme,
} from "./proxy-string.ts";

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

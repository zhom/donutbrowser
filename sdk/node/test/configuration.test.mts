/** Where the token and the port come from, and in what order. */

import assert from "node:assert/strict";
import { test } from "node:test";

import { DEFAULT_HOST, DEFAULT_PORT, DonutClient, DonutError } from "../src/index.mts";
import { FakeDonut } from "./fake-donut.mts";

test("arguments are used as given", () => {
  const client = new DonutClient({ token: "from-argument", port: 12345, env: {} });
  assert.equal(client.token, "from-argument");
  assert.equal(client.port, 12345);
  assert.equal(client.host, DEFAULT_HOST);
  assert.equal(client.baseUrl, "http://127.0.0.1:12345");
});

test("the environment fills in what was not passed", () => {
  const client = new DonutClient({
    env: { DONUT_API_TOKEN: "from-env", DONUT_API_PORT: "13579" },
  });
  assert.equal(client.token, "from-env");
  assert.equal(client.port, 13579);
});

test("arguments win over the environment", () => {
  const client = new DonutClient({
    token: "from-argument",
    port: 111,
    env: { DONUT_API_TOKEN: "from-env", DONUT_API_PORT: "222" },
  });
  assert.equal(client.token, "from-argument");
  assert.equal(client.port, 111);
});

test("the port falls back to the app default", () => {
  const client = new DonutClient({ env: { DONUT_API_TOKEN: "t" } });
  assert.equal(client.port, DEFAULT_PORT);
  assert.equal(DEFAULT_PORT, 10108);
});

test("a baseUrl overrides host and port", () => {
  const client = new DonutClient({
    baseUrl: "http://127.0.0.1:9999/donut",
    token: "t",
    env: { DONUT_API_PORT: "222" },
  });
  assert.equal(client.port, 9999);
  assert.equal(client.baseUrl, "http://127.0.0.1:9999/donut");
});

test("a baseUrl prefix is kept on every path", async () => {
  const fake = await new FakeDonut().start();
  try {
    const client = new DonutClient({
      baseUrl: `http://127.0.0.1:${fake.port}/donut`,
      token: "t",
      timeoutMs: 5_000,
      env: {},
    });
    await client.listProfiles();
    assert.equal(fake.last.path, "/donut/v1/profiles");
  } finally {
    await fake.stop();
  }
});

test("an unusable port in the environment is reported", () => {
  assert.throws(
    () => new DonutClient({ env: { DONUT_API_TOKEN: "t", DONUT_API_PORT: "not-a-number" } }),
    /DONUT_API_PORT/,
  );
});

test("an unsupported scheme is refused", () => {
  assert.throws(
    () => new DonutClient({ baseUrl: "ftp://127.0.0.1:9999", token: "t", env: {} }),
    DonutError,
  );
});

test("the websocket address is built from the same base", () => {
  const client = new DonutClient({ token: "t", port: 10108, env: {} });
  assert.equal(
    client.remoteSessionCdpUrl("s 1"),
    "ws://127.0.0.1:10108/v1/remote-sessions/s%201/cdp",
  );
});

test("an https base gives a wss websocket address", () => {
  const client = new DonutClient({ baseUrl: "https://127.0.0.1:8443", token: "t", env: {} });
  assert.equal(
    client.remoteSessionCdpUrl("s1"),
    "wss://127.0.0.1:8443/v1/remote-sessions/s1/cdp",
  );
});

test("a supplied fetch is the one that is used", async () => {
  const seen: string[] = [];
  const client = new DonutClient({
    token: "t",
    env: {},
    fetch: async (input) => {
      seen.push(String(input));
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    },
  });
  assert.deepEqual(await client.listTags(), []);
  assert.deepEqual(seen, ["http://127.0.0.1:10108/v1/tags"]);
});

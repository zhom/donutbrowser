/** Each status the app documents throws its own error. */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  BadGateway,
  Conflict,
  DonutApiError,
  DonutClient,
  DonutConnectionError,
  DonutError,
  Forbidden,
  NotFound,
  PaymentRequired,
  RateLimited,
  RequestTimeout,
  ServerError,
  ServiceUnavailable,
  Unauthorized,
  ValidationError,
} from "../src/index.mts";
import { FakeDonut } from "./fake-donut.mts";
import { withClient } from "./support.mts";

const STATUS_TO_ERROR: [number, new (...args: never[]) => DonutApiError][] = [
  [400, ValidationError],
  [401, Unauthorized],
  [402, PaymentRequired],
  [403, Forbidden],
  [404, NotFound],
  [408, RequestTimeout],
  [409, Conflict],
  [429, RateLimited],
  [500, ServerError],
  [502, BadGateway],
  [503, ServiceUnavailable],
];

for (const [status, expected] of STATUS_TO_ERROR) {
  test(`${status} maps to ${expected.name}`, async () => {
    await withClient(async (client, fake) => {
      fake.enqueueError(status, "something went wrong");
      const thrown = await client.listProfiles().then(
        () => null,
        (error: unknown) => error,
      );
      assert.ok(thrown instanceof expected, `expected ${expected.name}, got ${String(thrown)}`);
      assert.equal(thrown.status, status);
      assert.equal(thrown.body, "something went wrong");
      assert.equal(thrown.method, "GET");
      assert.equal(thrown.path, "/v1/profiles");
    });
  });
}

test("every error is a DonutError", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(404, "PROFILE_NOT_FOUND");
    await assert.rejects(client.getProfile("nope"), DonutError);
  });
});

test("the five hundreds share one base", async () => {
  await withClient(async (client, fake) => {
    for (const status of [500, 502, 503]) {
      fake.enqueueError(status, "upstream");
      await assert.rejects(client.listProfiles(), ServerError);
    }
  });
});

test("rate limited carries retryAfter", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(429, "automation request rate limit exceeded", { "Retry-After": "42" });
    const thrown = await client.runProfile("p1").then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof RateLimited);
    assert.equal(thrown.retryAfter, 42);
  });
});

test("rate limited without the header is still thrown", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(429, "slow down");
    const thrown = await client.runProfile("p1").then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof RateLimited);
    assert.equal(thrown.retryAfter, null);
  });
});

test("an unreadable Retry-After does not break the error", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(429, "slow down", { "Retry-After": "Wed, 21 Oct 2026 07:28:00 GMT" });
    const thrown = await client.runProfile("p1").then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof RateLimited);
    assert.equal(thrown.retryAfter, null);
  });
});

test("a structured code body is decoded", async () => {
  // The app shares `{"code": ...}` strings with its own frontend.
  await withClient(async (client, fake) => {
    fake.enqueueError(400, JSON.stringify({ code: "NAME_CANNOT_BE_EMPTY" }));
    const thrown = await client.createGroup("").then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof ValidationError);
    assert.equal(thrown.code, "NAME_CANNOT_BE_EMPTY");
    assert.deepEqual(thrown.params, {});
  });
});

test("a structured code body keeps its params", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(
      409,
      JSON.stringify({ code: "PROFILE_LOCKED_BY_MEMBER", params: { n: "5" } }),
    );
    const thrown = await client.runProfile("p1").then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof Conflict);
    assert.equal(thrown.code, "PROFILE_LOCKED_BY_MEMBER");
    assert.deepEqual(thrown.params, { n: "5" });
  });
});

test("a plain text body leaves code unset", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(400, "invalid browser");
    const thrown = await client.createProfile({ name: "x", browser: "chromium" }).then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof ValidationError);
    assert.equal(thrown.code, null);
    assert.equal(thrown.body, "invalid browser");
  });
});

test("an undocumented status still throws something catchable", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(418, "teapot");
    const thrown = await client.listProfiles().then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof DonutApiError);
    assert.equal(thrown.status, 418);
  });
});

test("an undocumented server status is a ServerError", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(504, "gateway timeout");
    await assert.rejects(client.listProfiles(), ServerError);
  });
});

test("the message names the call", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(404, "Profile not found");
    const thrown = await client.getProfile("missing").then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof NotFound);
    assert.match(thrown.message, /404/);
    assert.match(thrown.message, /GET \/v1\/profiles\/missing/);
  });
});

test("errors keep their class name", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(404, "gone");
    const thrown = await client.listProfiles().then(
      () => null,
      (error: unknown) => error,
    );
    assert.ok(thrown instanceof NotFound);
    assert.equal(thrown.name, "NotFound");
  });
});

test("an unreachable app is not an API error", async () => {
  const fake = await new FakeDonut().start();
  const port = fake.port;
  await fake.stop();

  const client = new DonutClient({ token: "t", port, timeoutMs: 2_000, env: {} });
  const thrown = await client.listProfiles().then(
    () => null,
    (error: unknown) => error,
  );
  assert.ok(thrown instanceof DonutConnectionError);
  assert.match(thrown.message, /Local API/);
});

test("a missing token fails before any request", () => {
  assert.throws(() => new DonutClient({ env: {} }), /DONUT_API_TOKEN/);
});

test("a non-JSON answer is reported as such", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueRaw(200, "<html>nope</html>");
    await assert.rejects(client.listProfiles(), /not\s+JSON/);
  });
});

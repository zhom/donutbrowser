/** `withProfile` launches, hands over the CDP endpoint, and stops. */

import assert from "node:assert/strict";
import { test } from "node:test";

import { Conflict, DonutError, RunSession } from "../src/index.mts";
import { withClient } from "./support.mts";

const RUN_BODY = { profile_id: "p1", remote_debugging_port: 9222, headless: true };

test("the callback gets the CDP endpoint", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueJson(RUN_BODY);
    fake.enqueueEmpty(204);

    const seen = await client.withProfile(
      "p1",
      { url: "https://example.com", headless: true },
      (session) => {
        assert.ok(session instanceof RunSession);
        assert.equal(session.remoteDebuggingPort, 9222);
        assert.equal(session.headless, true);
        assert.equal(session.cdpUrl, "http://127.0.0.1:9222");
        assert.deepEqual(session.response, RUN_BODY);
        return session.cdpUrl;
      },
    );

    assert.equal(seen, "http://127.0.0.1:9222");
    assert.deepEqual(
      fake.requests.map((sent) => `${sent.method} ${sent.path}`),
      ["POST /v1/profiles/p1/run", "POST /v1/profiles/p1/kill"],
    );
    assert.deepEqual(fake.requests[0]?.json, { url: "https://example.com", headless: true });
  });
});

test("the browser is stopped when the callback throws", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueJson(RUN_BODY);
    fake.enqueueEmpty(204);

    await assert.rejects(
      client.withProfile("p1", {}, () => {
        throw new RangeError("the body failed");
      }),
      RangeError,
    );

    assert.deepEqual(
      fake.requests.map((sent) => sent.path),
      ["/v1/profiles/p1/run", "/v1/profiles/p1/kill"],
    );
  });
});

test("a failed stop never hides why the callback failed", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueJson(RUN_BODY);
    fake.enqueueError(409, "PROFILE_LOCKED_ELSEWHERE");

    let captured: RunSession | undefined;
    await assert.rejects(
      client.withProfile("p1", {}, (session) => {
        captured = session;
        throw new RangeError("the body failed");
      }),
      RangeError,
    );

    assert.ok(captured?.cleanupError instanceof Conflict);
  });
});

test("a failed stop is thrown when the callback was fine", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueJson(RUN_BODY);
    fake.enqueueError(503, "the fleet could not be reached");

    await assert.rejects(
      client.withProfile("p1", {}, () => "done"),
      DonutError,
    );
  });
});

test("a failed launch never runs the callback and stops nothing", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueError(409, "PROFILE_RUNNING");

    await assert.rejects(
      client.withProfile("p1", {}, () => {
        throw new Error("the callback must not run when the launch failed");
      }),
      Conflict,
    );

    assert.deepEqual(
      fake.requests.map((sent) => sent.path),
      ["/v1/profiles/p1/run"],
    );
  });
});

test("an async callback is awaited before the browser is stopped", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueJson(RUN_BODY);
    fake.enqueueJson({ profiles: [], total: 0 });
    fake.enqueueEmpty(204);

    await client.withProfile("p1", {}, async () => {
      await client.listProfiles();
    });

    assert.deepEqual(
      fake.requests.map((sent) => sent.path),
      ["/v1/profiles/p1/run", "/v1/profiles", "/v1/profiles/p1/kill"],
    );
  });
});

test("a session also disposes itself", async () => {
  // `withProfile` is the portable form, but a runtime with `await using` can
  // hold a RunSession directly.
  await withClient(async (client, fake) => {
    fake.enqueueEmpty(204);
    const session = new RunSession(client, "p1", RUN_BODY);
    await session[Symbol.asyncDispose]();
    assert.deepEqual(
      fake.requests.map((sent) => sent.path),
      ["/v1/profiles/p1/kill"],
    );
  });
});

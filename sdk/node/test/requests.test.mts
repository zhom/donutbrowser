/**
 * Every client method sends exactly the request the app documents.
 *
 * The table below is the whole public surface. Each row names a method, the
 * arguments to call it with, and the request that must appear on the wire: the
 * verb, the concrete path, the query string and the JSON body. `operation` is
 * the path template the app publishes, which ties this file to
 * `OPERATIONS` and, through it, to `sdk/api-paths.json`.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { OPERATIONS } from "../src/index.mts";
import { withClient } from "./support.mts";

interface Case {
  method: string;
  args: unknown[];
  verb: string;
  path: string;
  body: unknown;
  query?: Record<string, string>;
  operation: string;
}

const LOCATOR = { role: "button", name: "Sign in" };

const CASES: Case[] = [
  // -- profiles ------------------------------------------------------------
  {
    method: "listProfiles",
    args: [],
    verb: "GET",
    path: "/v1/profiles",
    body: null,
    operation: "GET /v1/profiles",
  },
  {
    method: "getProfile",
    args: ["p1"],
    verb: "GET",
    path: "/v1/profiles/p1",
    body: null,
    operation: "GET /v1/profiles/{id}",
  },
  {
    method: "createProfile",
    args: [{ name: "Shopper", browser: "wayfern", tags: ["eu"], ephemeral: true }],
    verb: "POST",
    path: "/v1/profiles",
    body: { name: "Shopper", browser: "wayfern", tags: ["eu"], ephemeral: true },
    operation: "POST /v1/profiles",
  },
  {
    method: "createProfile",
    args: [{ name: "Bare", browser: "wayfern", version: undefined }],
    verb: "POST",
    path: "/v1/profiles",
    body: { name: "Bare", browser: "wayfern" },
    operation: "POST /v1/profiles",
  },
  {
    method: "updateProfile",
    args: ["p1", { name: "Renamed", proxy_id: "", clear_on_close: false }],
    verb: "PUT",
    path: "/v1/profiles/p1",
    body: { name: "Renamed", proxy_id: "", clear_on_close: false },
    operation: "PUT /v1/profiles/{id}",
  },
  {
    method: "deleteProfile",
    args: ["p1"],
    verb: "DELETE",
    path: "/v1/profiles/p1",
    body: null,
    operation: "DELETE /v1/profiles/{id}",
  },
  {
    method: "runProfile",
    args: ["p1", { url: "https://example.com", headless: true }],
    verb: "POST",
    path: "/v1/profiles/p1/run",
    body: { url: "https://example.com", headless: true },
    operation: "POST /v1/profiles/{id}/run",
  },
  {
    method: "runProfileRemote",
    args: ["p1", { url: "https://example.com" }],
    verb: "POST",
    path: "/v1/profiles/p1/run-remote",
    body: { url: "https://example.com" },
    operation: "POST /v1/profiles/{id}/run-remote",
  },
  {
    method: "setProfileCloudSync",
    args: ["p1", "Regular"],
    verb: "POST",
    path: "/v1/profiles/p1/cloud-sync",
    body: { mode: "Regular" },
    operation: "POST /v1/profiles/{id}/cloud-sync",
  },
  {
    method: "openUrl",
    args: ["p1", "https://example.com/page"],
    verb: "POST",
    path: "/v1/profiles/p1/open-url",
    body: { url: "https://example.com/page" },
    operation: "POST /v1/profiles/{id}/open-url",
  },
  {
    method: "killProfile",
    args: ["p1"],
    verb: "POST",
    path: "/v1/profiles/p1/kill",
    body: null,
    operation: "POST /v1/profiles/{id}/kill",
  },
  {
    method: "batchRunProfiles",
    args: [["p1", "p2"], { headless: false }],
    verb: "POST",
    path: "/v1/profiles/batch/run",
    body: { profile_ids: ["p1", "p2"], headless: false },
    operation: "POST /v1/profiles/batch/run",
  },
  {
    method: "batchStopProfiles",
    args: [["p1", "p2"]],
    verb: "POST",
    path: "/v1/profiles/batch/stop",
    body: { profile_ids: ["p1", "p2"] },
    operation: "POST /v1/profiles/batch/stop",
  },
  {
    method: "distributeProxies",
    args: [
      [
        { profile_id: "p1", proxy_id: "x1" },
        { profile_id: "p2", proxy_id: "x2" },
      ],
    ],
    verb: "POST",
    path: "/v1/profiles/distribute-proxies",
    body: {
      pairs: [
        { profile_id: "p1", proxy_id: "x1" },
        { profile_id: "p2", proxy_id: "x2" },
      ],
    },
    operation: "POST /v1/profiles/distribute-proxies",
  },
  {
    method: "detectImportProfiles",
    args: [{ folder: "/Users/x/Chrome" }],
    verb: "GET",
    path: "/v1/profiles/import/detect",
    body: null,
    query: { folder: "/Users/x/Chrome" },
    operation: "GET /v1/profiles/import/detect",
  },
  {
    method: "detectImportProfiles",
    args: [],
    verb: "GET",
    path: "/v1/profiles/import/detect",
    body: null,
    operation: "GET /v1/profiles/import/detect",
  },
  {
    method: "importProfiles",
    args: [
      [{ source_path: "/tmp/src", new_profile_name: "Imported" }],
      { duplicate_strategy: "skip" },
    ],
    verb: "POST",
    path: "/v1/profiles/import",
    body: {
      items: [{ source_path: "/tmp/src", new_profile_name: "Imported" }],
      duplicate_strategy: "skip",
    },
    operation: "POST /v1/profiles/import",
  },
  {
    method: "importProfileCookies",
    args: ["p1", "[]"],
    verb: "POST",
    path: "/v1/profiles/p1/cookies/import",
    body: { content: "[]" },
    operation: "POST /v1/profiles/{id}/cookies/import",
  },
  // -- agent ---------------------------------------------------------------
  {
    method: "agentPerceive",
    args: ["p1", { viewport_only: true, max_bytes: 2048 }],
    verb: "POST",
    path: "/v1/profiles/p1/agent/perceive",
    body: { viewport_only: true, max_bytes: 2048 },
    operation: "POST /v1/profiles/{id}/agent/perceive",
  },
  {
    method: "agentPerceive",
    args: ["p1"],
    verb: "POST",
    path: "/v1/profiles/p1/agent/perceive",
    body: {},
    operation: "POST /v1/profiles/{id}/agent/perceive",
  },
  {
    method: "agentResolveLocator",
    args: ["p1", { locator: LOCATOR, candidate_limit: 5 }],
    verb: "POST",
    path: "/v1/profiles/p1/agent/resolve-locator",
    body: { locator: LOCATOR, candidate_limit: 5 },
    operation: "POST /v1/profiles/{id}/agent/resolve-locator",
  },
  {
    method: "agentClick",
    args: ["p1", { locator: LOCATOR, button: "right", click_count: 2 }],
    verb: "POST",
    path: "/v1/profiles/p1/agent/click",
    body: { locator: LOCATOR, button: "right", click_count: 2 },
    operation: "POST /v1/profiles/{id}/agent/click",
  },
  {
    method: "agentType",
    args: ["p1", { locator: LOCATOR, text: "hello", clear_first: false, wpm: 55 }],
    verb: "POST",
    path: "/v1/profiles/p1/agent/type",
    body: { locator: LOCATOR, text: "hello", clear_first: false, wpm: 55 },
    operation: "POST /v1/profiles/{id}/agent/type",
  },
  {
    method: "agentExtract",
    args: [
      "p1",
      {
        container: { role: "listitem" },
        field_map: [{ key: "title", locator: { role: "heading" }, source: "text" }],
        max_pages: 3,
      },
    ],
    verb: "POST",
    path: "/v1/profiles/p1/agent/extract",
    body: {
      container: { role: "listitem" },
      field_map: [{ key: "title", locator: { role: "heading" }, source: "text" }],
      max_pages: 3,
    },
    operation: "POST /v1/profiles/{id}/agent/extract",
  },
  {
    method: "agentPick",
    args: ["p1", { timeout_ms: 15000 }],
    verb: "POST",
    path: "/v1/profiles/p1/agent/pick",
    body: { timeout_ms: 15000 },
    operation: "POST /v1/profiles/{id}/agent/pick",
  },
  // -- remote sessions -----------------------------------------------------
  {
    method: "listRemoteSessions",
    args: [],
    verb: "GET",
    path: "/v1/remote-sessions",
    body: null,
    operation: "GET /v1/remote-sessions",
  },
  {
    method: "getRemoteSession",
    args: ["s1"],
    verb: "GET",
    path: "/v1/remote-sessions/s1",
    body: null,
    operation: "GET /v1/remote-sessions/{id}",
  },
  {
    method: "stopRemoteSession",
    args: ["s1"],
    verb: "DELETE",
    path: "/v1/remote-sessions/s1",
    body: null,
    operation: "DELETE /v1/remote-sessions/{id}",
  },
  {
    method: "getRemoteHours",
    args: [],
    verb: "GET",
    path: "/v1/remote-hours",
    body: null,
    operation: "GET /v1/remote-hours",
  },
  // -- cookie bot ----------------------------------------------------------
  {
    method: "listCookieBotSchedules",
    args: [{ scope: "team" }],
    verb: "GET",
    path: "/v1/cookie-bot/schedules",
    body: null,
    query: { scope: "team" },
    operation: "GET /v1/cookie-bot/schedules",
  },
  {
    method: "getCookieBotSchedule",
    args: ["p1"],
    verb: "GET",
    path: "/v1/cookie-bot/schedules/p1",
    body: null,
    operation: "GET /v1/cookie-bot/schedules/{profile_id}",
  },
  {
    method: "setCookieBotSchedule",
    args: [
      "p1",
      {
        enabled: true,
        run_at_minute: 120,
        days_mask: 31,
        timezone: "Europe/Berlin",
        preset: "steady",
        max_minutes: 45,
        sites: ["https://example.com"],
        acknowledge_conflict: true,
      },
    ],
    verb: "PUT",
    path: "/v1/cookie-bot/schedules/p1",
    body: {
      enabled: true,
      run_at_minute: 120,
      days_mask: 31,
      timezone: "Europe/Berlin",
      preset: "steady",
      max_minutes: 45,
      sites: ["https://example.com"],
      acknowledge_conflict: true,
    },
    operation: "PUT /v1/cookie-bot/schedules/{profile_id}",
  },
  {
    method: "deleteCookieBotSchedule",
    args: ["p1"],
    verb: "DELETE",
    path: "/v1/cookie-bot/schedules/p1",
    body: null,
    operation: "DELETE /v1/cookie-bot/schedules/{profile_id}",
  },
  {
    method: "getCookieBotConflicts",
    args: ["p1", { run_at_minute: 90, timezone: "UTC", days_mask: 7 }],
    verb: "GET",
    path: "/v1/cookie-bot/conflicts",
    body: null,
    query: { profile_id: "p1", run_at_minute: "90", timezone: "UTC", days_mask: "7" },
    operation: "GET /v1/cookie-bot/conflicts",
  },
  {
    method: "listCookieBotRuns",
    args: [{ profile_id: "p1", limit: 10, before: "cursor-1" }],
    verb: "GET",
    path: "/v1/cookie-bot/runs",
    body: null,
    query: { profile_id: "p1", limit: "10", before: "cursor-1" },
    operation: "GET /v1/cookie-bot/runs",
  },
  {
    method: "startCookieBotRun",
    args: [{ profile_id: "p1", max_minutes: 30 }],
    verb: "POST",
    path: "/v1/cookie-bot/runs",
    body: { profile_id: "p1", max_minutes: 30 },
    operation: "POST /v1/cookie-bot/runs",
  },
  {
    method: "cancelCookieBotRun",
    args: ["r1"],
    verb: "DELETE",
    path: "/v1/cookie-bot/runs/r1",
    body: null,
    operation: "DELETE /v1/cookie-bot/runs/{run_id}",
  },
  {
    method: "listCookieBotPresets",
    args: [],
    verb: "GET",
    path: "/v1/cookie-bot/presets",
    body: null,
    operation: "GET /v1/cookie-bot/presets",
  },
  {
    method: "getCookieBotUsage",
    args: [{ period: "2026-08" }],
    verb: "GET",
    path: "/v1/cookie-bot/usage",
    body: null,
    query: { period: "2026-08" },
    operation: "GET /v1/cookie-bot/usage",
  },
  // -- groups and tags -----------------------------------------------------
  {
    method: "listGroups",
    args: [],
    verb: "GET",
    path: "/v1/groups",
    body: null,
    operation: "GET /v1/groups",
  },
  {
    method: "getGroup",
    args: ["g1"],
    verb: "GET",
    path: "/v1/groups/g1",
    body: null,
    operation: "GET /v1/groups/{id}",
  },
  {
    method: "createGroup",
    args: ["Retail"],
    verb: "POST",
    path: "/v1/groups",
    body: { name: "Retail" },
    operation: "POST /v1/groups",
  },
  {
    method: "updateGroup",
    args: ["g1", "Retail EU"],
    verb: "PUT",
    path: "/v1/groups/g1",
    body: { name: "Retail EU" },
    operation: "PUT /v1/groups/{id}",
  },
  {
    method: "deleteGroup",
    args: ["g1"],
    verb: "DELETE",
    path: "/v1/groups/g1",
    body: null,
    operation: "DELETE /v1/groups/{id}",
  },
  {
    method: "listTags",
    args: [],
    verb: "GET",
    path: "/v1/tags",
    body: null,
    operation: "GET /v1/tags",
  },
  // -- proxies -------------------------------------------------------------
  {
    method: "listProxies",
    args: [],
    verb: "GET",
    path: "/v1/proxies",
    body: null,
    operation: "GET /v1/proxies",
  },
  {
    method: "getProxy",
    args: ["x1"],
    verb: "GET",
    path: "/v1/proxies/x1",
    body: null,
    operation: "GET /v1/proxies/{id}",
  },
  {
    method: "createProxy",
    args: [{ name: "EU", proxy_settings: { proxy_type: "http", host: "h", port: 8080 } }],
    verb: "POST",
    path: "/v1/proxies",
    body: { name: "EU", proxy_settings: { proxy_type: "http", host: "h", port: 8080 } },
    operation: "POST /v1/proxies",
  },
  {
    method: "updateProxy",
    args: ["x1", { name: "EU 2" }],
    verb: "PUT",
    path: "/v1/proxies/x1",
    body: { name: "EU 2" },
    operation: "PUT /v1/proxies/{id}",
  },
  {
    method: "deleteProxy",
    args: ["x1"],
    verb: "DELETE",
    path: "/v1/proxies/x1",
    body: null,
    operation: "DELETE /v1/proxies/{id}",
  },
  {
    method: "importProxies",
    args: [{ format: "txt", content: "h:1:u:p", name_prefix: "EU" }],
    verb: "POST",
    path: "/v1/proxies/import",
    body: { format: "txt", content: "h:1:u:p", name_prefix: "EU" },
    operation: "POST /v1/proxies/import",
  },
  // -- vpns ----------------------------------------------------------------
  {
    method: "listVpns",
    args: [],
    verb: "GET",
    path: "/v1/vpns",
    body: null,
    operation: "GET /v1/vpns",
  },
  {
    method: "getVpn",
    args: ["v1"],
    verb: "GET",
    path: "/v1/vpns/v1",
    body: null,
    operation: "GET /v1/vpns/{id}",
  },
  {
    method: "exportVpn",
    args: ["v1"],
    verb: "GET",
    path: "/v1/vpns/v1/export",
    body: null,
    operation: "GET /v1/vpns/{id}/export",
  },
  {
    method: "importVpn",
    args: [{ content: "[Interface]", filename: "eu.conf" }],
    verb: "POST",
    path: "/v1/vpns/import",
    body: { content: "[Interface]", filename: "eu.conf" },
    operation: "POST /v1/vpns/import",
  },
  {
    method: "createVpn",
    args: [{ name: "EU", vpn_type: "WireGuard", config_data: "[Interface]" }],
    verb: "POST",
    path: "/v1/vpns",
    body: { name: "EU", vpn_type: "WireGuard", config_data: "[Interface]" },
    operation: "POST /v1/vpns",
  },
  {
    method: "updateVpn",
    args: ["v1", "EU 2"],
    verb: "PUT",
    path: "/v1/vpns/v1",
    body: { name: "EU 2" },
    operation: "PUT /v1/vpns/{id}",
  },
  {
    method: "deleteVpn",
    args: ["v1"],
    verb: "DELETE",
    path: "/v1/vpns/v1",
    body: null,
    operation: "DELETE /v1/vpns/{id}",
  },
  // -- extensions ----------------------------------------------------------
  {
    method: "listExtensions",
    args: [],
    verb: "GET",
    path: "/v1/extensions",
    body: null,
    operation: "GET /v1/extensions",
  },
  {
    method: "getExtension",
    args: ["e1"],
    verb: "GET",
    path: "/v1/extensions/e1",
    body: null,
    operation: "GET /v1/extensions/{id}",
  },
  {
    method: "createExtension",
    args: [{ name: "Blocker", file_name: "b.crx", file_data_base64: "AAAA" }],
    verb: "POST",
    path: "/v1/extensions",
    body: { name: "Blocker", file_name: "b.crx", file_data_base64: "AAAA" },
    operation: "POST /v1/extensions",
  },
  {
    method: "updateExtension",
    args: ["e1", { name: "Blocker 2", link: true }],
    verb: "PUT",
    path: "/v1/extensions/e1",
    body: { name: "Blocker 2", link: true },
    operation: "PUT /v1/extensions/{id}",
  },
  {
    method: "deleteExtension",
    args: ["e1"],
    verb: "DELETE",
    path: "/v1/extensions/e1",
    body: null,
    operation: "DELETE /v1/extensions/{id}",
  },
  {
    method: "listExtensionGroups",
    args: [],
    verb: "GET",
    path: "/v1/extension-groups",
    body: null,
    operation: "GET /v1/extension-groups",
  },
  {
    method: "getExtensionGroup",
    args: ["eg1"],
    verb: "GET",
    path: "/v1/extension-groups/eg1",
    body: null,
    operation: "GET /v1/extension-groups/{id}",
  },
  {
    method: "createExtensionGroup",
    args: ["Adblock set"],
    verb: "POST",
    path: "/v1/extension-groups",
    body: { name: "Adblock set" },
    operation: "POST /v1/extension-groups",
  },
  {
    method: "updateExtensionGroup",
    args: ["eg1", { extension_ids: ["e1", "e2"] }],
    verb: "PUT",
    path: "/v1/extension-groups/eg1",
    body: { extension_ids: ["e1", "e2"] },
    operation: "PUT /v1/extension-groups/{id}",
  },
  {
    method: "deleteExtensionGroup",
    args: ["eg1"],
    verb: "DELETE",
    path: "/v1/extension-groups/eg1",
    body: null,
    operation: "DELETE /v1/extension-groups/{id}",
  },
  {
    method: "addExtensionToGroup",
    args: ["eg1", "e1"],
    verb: "POST",
    path: "/v1/extension-groups/eg1/extensions/e1",
    body: null,
    operation: "POST /v1/extension-groups/{id}/extensions/{extension_id}",
  },
  {
    method: "removeExtensionFromGroup",
    args: ["eg1", "e1"],
    verb: "DELETE",
    path: "/v1/extension-groups/eg1/extensions/e1",
    body: null,
    operation: "DELETE /v1/extension-groups/{id}/extensions/{extension_id}",
  },
  // -- browsers ------------------------------------------------------------
  {
    method: "downloadBrowser",
    args: [{ browser: "wayfern", version: "152.0.1" }],
    verb: "POST",
    path: "/v1/browsers/download",
    body: { browser: "wayfern", version: "152.0.1" },
    operation: "POST /v1/browsers/download",
  },
  {
    method: "listBrowserVersions",
    args: ["wayfern"],
    verb: "GET",
    path: "/v1/browsers/wayfern/versions",
    body: null,
    operation: "GET /v1/browsers/{browser}/versions",
  },
  {
    method: "isBrowserDownloaded",
    args: ["wayfern", "152.0.1"],
    verb: "GET",
    path: "/v1/browsers/wayfern/versions/152.0.1/downloaded",
    body: null,
    operation: "GET /v1/browsers/{browser}/versions/{version}/downloaded",
  },
];

for (const [index, expected] of CASES.entries()) {
  test(`${expected.method} sends the documented request [${index}]`, async () => {
    await withClient(async (client, fake) => {
      const callable = (client as unknown as Record<string, (...args: unknown[]) => Promise<unknown>>)[
        expected.method
      ];
      assert.equal(typeof callable, "function", `${expected.method} is not a method`);
      await callable.call(client, ...expected.args);

      const sent = fake.last;
      assert.equal(sent.method, expected.verb);
      assert.equal(sent.path, expected.path);
      assert.deepEqual(sent.query, expected.query ?? {});
      assert.deepEqual(sent.json, expected.body);
      assert.equal(OPERATIONS.get(expected.operation), expected.method);
    });
  });
}

test("every wrapped operation has a request test", () => {
  const covered = new Set(CASES.map((entry) => entry.method));
  const missing = [...OPERATIONS.values()].filter((name) => !covered.has(name)).sort();
  assert.deepEqual(missing, [], `these wrapped operations have no request test: ${missing}`);
});

test("the token travels as a bearer header", async () => {
  await withClient(async (client, fake) => {
    await client.listProfiles();
    assert.equal(fake.last.headers.authorization, "Bearer test-token-abc123");
    assert.equal(fake.last.headers.accept, "application/json");
    assert.equal(
      fake.last.headers["content-type"],
      undefined,
      "a GET must not claim to carry JSON",
    );
  });
});

test("a body is sent as JSON", async () => {
  await withClient(async (client, fake) => {
    await client.createGroup("Retail");
    assert.equal(fake.last.headers["content-type"], "application/json");
    assert.equal(fake.last.rawBody, '{"name":"Retail"}');
  });
});

test("path ids are escaped", async () => {
  await withClient(async (client, fake) => {
    await client.getProfile("a/b c?d");
    assert.equal(fake.last.path, "/v1/profiles/a%2Fb%20c%3Fd");
  });
});

test("undefined arguments are left out of the body", async () => {
  await withClient(async (client, fake) => {
    await client.updateProfile("p1", { name: "Only this", version: undefined });
    assert.deepEqual(fake.last.json, { name: "Only this" });
  });
});

test("an empty string still reaches the app", async () => {
  // `proxy_id: ""` is how the app is told to detach a proxy, so it must survive.
  await withClient(async (client, fake) => {
    await client.updateProfile("p1", { proxy_id: "" });
    assert.deepEqual(fake.last.json, { proxy_id: "" });
  });
});

test("a no-content answer becomes undefined", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueEmpty(204);
    assert.equal(await client.deleteProfile("p1"), undefined);
  });
});

test("a JSON answer is returned as sent", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueJson({ profiles: [{ id: "p1", name: "Shopper" }], total: 1 });
    assert.deepEqual(await client.listProfiles(), {
      profiles: [{ id: "p1", name: "Shopper" }],
      total: 1,
    });
  });
});

test("a bare boolean answer is returned", async () => {
  await withClient(async (client, fake) => {
    fake.enqueueJson(true);
    assert.equal(await client.isBrowserDownloaded("wayfern", "152.0.1"), true);
  });
});

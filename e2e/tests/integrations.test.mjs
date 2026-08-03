import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { withApp } from "../lib/app.mjs";

const VLESS_URI =
  "vless://6d6e21a1-4829-4d2b-bc7f-1b25707b61e4@127.0.0.1:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.example.com&fp=chrome&pbk=BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc&sid=0123456789abcdef&spx=%2F&type=tcp&headerType=none#MCP";

async function jsonRequest(
  url,
  { method = "GET", token, body, headers = {} } = {},
) {
  const response = await fetch(url, {
    method,
    headers: {
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...(body === undefined ? {} : { "content-type": "application/json" }),
      ...headers,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let value = null;
  if (text) {
    try {
      value = JSON.parse(text);
    } catch {
      value = text;
    }
  }
  return { response, value };
}

async function seedTerms(app) {
  const home = path.join(app.root, "home");
  const directory =
    process.platform === "darwin"
      ? path.join(home, "Library", "Application Support", "Wayfern")
      : process.platform === "win32"
        ? path.join(app.root, "windows", "roaming", "Wayfern")
        : path.join(app.root, "xdg", "config", "Wayfern");
  await mkdir(directory, { recursive: true });
  await writeFile(
    path.join(directory, "license-accepted"),
    String(Math.floor(Date.now() / 1000)),
  );
}

async function invokeContract(app, command, args = {}) {
  try {
    return { ok: true, value: await app.invoke(command, args) };
  } catch (error) {
    return { ok: false, error: String(error) };
  }
}

async function assertCommandErrorCode(app, command, code, args = {}) {
  const error = await app.invokeError(command, args);
  assert.match(error, new RegExp(`"code":"${code}"`));
}

test("authenticated REST API serves its complete OpenAPI contract and CRUD lifecycle", async () => {
  await withApp("integrations-rest", async (app) => {
    await seedTerms(app);
    const settings = await app.invoke("get_app_settings");
    const saved = await app.invoke("save_app_settings", {
      settings: {
        ...settings,
        api_enabled: true,
        api_port: 0,
        api_token: null,
        onboarding_completed: true,
      },
    });
    assert.ok(saved.api_token?.length >= 32);
    const port = await app.invoke("start_api_server", { port: 0 });
    assert.equal(await app.invoke("get_api_server_status"), port);
    const base = `http://127.0.0.1:${port}`;

    const openapi = await jsonRequest(`${base}/openapi.json`);
    assert.equal(openapi.response.status, 200);
    assert.equal(openapi.value.openapi.startsWith("3."), true);
    const paths = Object.keys(openapi.value.paths);
    for (const required of [
      "/v1/profiles",
      "/v1/profiles/{id}/run",
      "/v1/groups",
      "/v1/proxies",
      "/v1/vpns/{id}/export",
      "/v1/extensions",
      "/v1/browsers/{browser}/versions",
    ]) {
      assert.ok(paths.includes(required), `OpenAPI is missing ${required}`);
    }

    const unauthorized = await jsonRequest(`${base}/v1/profiles`);
    assert.equal(unauthorized.response.status, 401);
    const wrongToken = await jsonRequest(`${base}/v1/profiles`, {
      token: "wrong",
    });
    assert.equal(wrongToken.response.status, 401);

    const groupsInitially = await jsonRequest(`${base}/v1/groups`, {
      token: saved.api_token,
    });
    assert.equal(groupsInitially.response.status, 200);
    assert.deepEqual(groupsInitially.value, []);
    const createdGroup = await jsonRequest(`${base}/v1/groups`, {
      method: "POST",
      token: saved.api_token,
      body: { name: "REST Group" },
    });
    assert.equal(createdGroup.response.status, 200);
    assert.equal(createdGroup.value.name, "REST Group");
    const groupId = createdGroup.value.id;
    const updatedGroup = await jsonRequest(`${base}/v1/groups/${groupId}`, {
      method: "PUT",
      token: saved.api_token,
      body: { name: "REST Group Updated" },
    });
    assert.equal(updatedGroup.value.name, "REST Group Updated");

    const createdProxy = await jsonRequest(`${base}/v1/proxies`, {
      method: "POST",
      token: saved.api_token,
      body: {
        name: "REST Proxy",
        proxy_settings: {
          proxy_type: "http",
          host: "127.0.0.1",
          port: 8080,
          username: null,
          password: null,
        },
      },
    });
    assert.equal(createdProxy.response.status, 200);
    assert.equal(createdProxy.value.proxy_settings.port, 8080);
    const proxyId = createdProxy.value.id;
    const fetchedProxy = await jsonRequest(`${base}/v1/proxies/${proxyId}`, {
      token: saved.api_token,
    });
    assert.equal(fetchedProxy.value.name, "REST Proxy");
    const createdVless = await jsonRequest(`${base}/v1/proxies`, {
      method: "POST",
      token: saved.api_token,
      body: {
        name: "REST VLESS Reality",
        proxy_settings: {
          proxy_type: "vless",
          host: "127.0.0.1",
          port: 443,
          username: null,
          password: null,
          vless_uri: VLESS_URI,
        },
      },
    });
    assert.equal(createdVless.response.status, 200);
    assert.equal(createdVless.value.proxy_settings.vless_uri, VLESS_URI);
    assert.equal(createdVless.value.proxy_settings.host, "127.0.0.1");
    assert.equal(createdVless.value.proxy_settings.port, 443);
    const vlessProxyId = createdVless.value.id;
    const invalidVless = await jsonRequest(
      `${base}/v1/proxies/${vlessProxyId}`,
      {
        method: "PUT",
        token: saved.api_token,
        body: {
          proxy_settings: {
            ...createdVless.value.proxy_settings,
            vless_uri: VLESS_URI.replace("security=reality", "security=tls"),
          },
        },
      },
    );
    assert.equal(invalidVless.response.status, 400);
    assert.match(JSON.stringify(invalidVless.value), /VLESS_CONFIG_INVALID/);
    assert.equal(
      (
        await jsonRequest(`${base}/v1/proxies/${vlessProxyId}`, {
          token: saved.api_token,
        })
      ).value.proxy_settings.vless_uri,
      VLESS_URI,
    );
    const imported = await jsonRequest(`${base}/v1/proxies/import`, {
      method: "POST",
      token: saved.api_token,
      body: {
        format: "txt",
        content: "http://127.0.0.1:8081",
        name_prefix: "API",
      },
    });
    assert.equal(imported.response.status, 200);
    assert.equal(imported.value.imported_count, 1);

    const missing = await jsonRequest(`${base}/v1/groups/missing`, {
      token: saved.api_token,
    });
    assert.equal(missing.response.status, 404);
    const invalidProfile = await jsonRequest(`${base}/v1/profiles`, {
      method: "POST",
      token: saved.api_token,
      body: { name: "Bad", browser: "unsupported", version: "latest" },
    });
    assert.equal(invalidProfile.response.status, 400);

    assert.equal(
      (
        await jsonRequest(`${base}/v1/proxies/${proxyId}`, {
          method: "DELETE",
          token: saved.api_token,
        })
      ).response.status,
      204,
    );
    assert.equal(
      (
        await jsonRequest(`${base}/v1/proxies/${vlessProxyId}`, {
          method: "DELETE",
          token: saved.api_token,
        })
      ).response.status,
      204,
    );
    for (const importedProxy of imported.value.proxies) {
      await jsonRequest(`${base}/v1/proxies/${importedProxy.id}`, {
        method: "DELETE",
        token: saved.api_token,
      });
    }
    assert.equal(
      (
        await jsonRequest(`${base}/v1/groups/${groupId}`, {
          method: "DELETE",
          token: saved.api_token,
        })
      ).response.status,
      204,
    );
    await app.invoke("stop_api_server");
    assert.equal(await app.invoke("get_api_server_status"), null);
  });
});

test("MCP Streamable HTTP initialization, auth, discovery, calls, and isolated agent install", async () => {
  await withApp("integrations-mcp", async (app) => {
    await seedTerms(app);
    await assertCommandErrorCode(
      app,
      "stop_mcp_server",
      "MCP_SERVER_NOT_RUNNING",
    );
    const port = await app.invoke("start_mcp_server");
    await assertCommandErrorCode(
      app,
      "start_mcp_server",
      "MCP_SERVER_ALREADY_RUNNING",
    );
    assert.equal(await app.invoke("get_mcp_server_status"), true);
    const config = await app.invoke("get_mcp_config");
    assert.equal(config.port, port);
    assert.ok(config.token.length >= 32);
    const base = `http://127.0.0.1:${port}`;
    assert.equal((await fetch(`${base}/health`)).status, 200);
    assert.equal(
      (
        await jsonRequest(`${base}/mcp`, {
          method: "POST",
          body: { jsonrpc: "2.0", id: 1, method: "initialize", params: {} },
        })
      ).response.status,
      401,
    );

    const initialized = await jsonRequest(`${base}/mcp/${config.token}`, {
      method: "POST",
      body: {
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: {
          protocolVersion: "2025-11-25",
          capabilities: {},
          clientInfo: { name: "donut-e2e", version: "1" },
        },
      },
    });
    assert.equal(initialized.response.status, 200);
    assert.equal(initialized.value.result.serverInfo.name, "donut-browser");
    const sessionId = initialized.response.headers.get("mcp-session-id");
    assert.ok(sessionId);
    const mcpHeaders = { "mcp-session-id": sessionId };
    const notification = await jsonRequest(`${base}/mcp/${config.token}`, {
      method: "POST",
      headers: mcpHeaders,
      body: { jsonrpc: "2.0", method: "notifications/initialized" },
    });
    assert.equal(notification.response.status, 202);
    const tools = await jsonRequest(`${base}/mcp/${config.token}`, {
      method: "POST",
      headers: mcpHeaders,
      body: { jsonrpc: "2.0", id: 2, method: "tools/list", params: {} },
    });
    assert.equal(tools.response.status, 200);
    const names = tools.value.result.tools.map((tool) => tool.name);
    for (const name of [
      "list_profiles",
      "create_profile",
      "run_profile",
      "list_proxies",
      "create_proxy",
      "update_proxy",
      "get_page_content",
      "get_interactive_elements",
    ]) {
      assert.ok(names.includes(name), `MCP is missing ${name}`);
    }
    const listed = await jsonRequest(`${base}/mcp/${config.token}`, {
      method: "POST",
      headers: mcpHeaders,
      body: {
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: { name: "list_profiles", arguments: {} },
      },
    });
    assert.equal(listed.response.status, 200);
    assert.equal(listed.value.error, undefined);
    assert.ok(listed.value.result);

    const createdVless = await jsonRequest(`${base}/mcp/${config.token}`, {
      method: "POST",
      headers: mcpHeaders,
      body: {
        jsonrpc: "2.0",
        id: 4,
        method: "tools/call",
        params: {
          name: "create_proxy",
          arguments: {
            name: "MCP VLESS Reality",
            proxy_type: "vless",
            vless_uri: VLESS_URI,
          },
        },
      },
    });
    assert.equal(createdVless.response.status, 200);
    assert.equal(createdVless.value.error, undefined);
    let vlessProxy = (await app.invoke("get_stored_proxies")).find(
      (proxy) => proxy.name === "MCP VLESS Reality",
    );
    assert.ok(vlessProxy);
    assert.equal(vlessProxy.proxy_settings.proxy_type, "vless");
    assert.equal(vlessProxy.proxy_settings.vless_uri, VLESS_URI);

    const updatedVless = await jsonRequest(`${base}/mcp/${config.token}`, {
      method: "POST",
      headers: mcpHeaders,
      body: {
        jsonrpc: "2.0",
        id: 5,
        method: "tools/call",
        params: {
          name: "update_proxy",
          arguments: {
            proxy_id: vlessProxy.id,
            name: "MCP VLESS Updated",
            vless_uri: VLESS_URI,
          },
        },
      },
    });
    assert.equal(updatedVless.value.error, undefined);
    vlessProxy = (await app.invoke("get_stored_proxies")).find(
      (proxy) => proxy.id === vlessProxy.id,
    );
    assert.equal(vlessProxy.name, "MCP VLESS Updated");

    const invalidVless = await jsonRequest(`${base}/mcp/${config.token}`, {
      method: "POST",
      headers: mcpHeaders,
      body: {
        jsonrpc: "2.0",
        id: 6,
        method: "tools/call",
        params: {
          name: "update_proxy",
          arguments: {
            proxy_id: vlessProxy.id,
            vless_uri: VLESS_URI.replace("security=reality", "security=tls"),
          },
        },
      },
    });
    assert.match(invalidVless.value.error.message, /VLESS_CONFIG_INVALID/);
    assert.equal(
      (await app.invoke("get_stored_proxies")).find(
        (proxy) => proxy.id === vlessProxy.id,
      ).proxy_settings.vless_uri,
      VLESS_URI,
    );
    await app.invoke("delete_stored_proxy", { proxyId: vlessProxy.id });

    const agents = await app.invoke("list_mcp_agents");
    assert.ok(agents.some((agent) => agent.id === "cursor"));
    await assertCommandErrorCode(app, "add_mcp_to_agent", "MCP_AGENT_UNKNOWN", {
      agentId: "missing-e2e-agent",
    });
    await app.invoke("add_mcp_to_agent", { agentId: "cursor" });
    assert.equal(
      (await app.invoke("list_mcp_agents")).find(
        (agent) => agent.id === "cursor",
      ).connected,
      true,
    );
    await app.invoke("remove_mcp_from_agent", { agentId: "cursor" });
    assert.equal(
      (await app.invoke("list_mcp_agents")).find(
        (agent) => agent.id === "cursor",
      ).connected,
      false,
    );
    assert.equal(
      (
        await jsonRequest(`${base}/mcp/${config.token}`, {
          method: "DELETE",
          headers: mcpHeaders,
        })
      ).response.status,
      200,
    );
    await app.invoke("stop_mcp_server");
    assert.equal(await app.invoke("get_mcp_server_status"), false);
  });
});

test("REST and MCP share the browser automation rate limit", async () => {
  await withApp(
    "integrations-rate-limit",
    async (app) => {
      await seedTerms(app);
      const settings = await app.invoke("get_app_settings");
      const saved = await app.invoke("save_app_settings", {
        settings: {
          ...settings,
          api_enabled: true,
          api_port: 0,
          api_token: null,
          onboarding_completed: true,
        },
      });

      const apiPort = await app.invoke("start_api_server", { port: 0 });
      const mcpPort = await app.invoke("start_mcp_server");
      const mcpConfig = await app.invoke("get_mcp_config");
      const apiBase = `http://127.0.0.1:${apiPort}`;
      const mcpUrl = `http://127.0.0.1:${mcpPort}/mcp/${mcpConfig.token}`;

      const initialized = await jsonRequest(mcpUrl, {
        method: "POST",
        body: {
          jsonrpc: "2.0",
          id: 1,
          method: "initialize",
          params: {
            protocolVersion: "2025-11-25",
            capabilities: {},
            clientInfo: { name: "donut-e2e-rate-limit", version: "1" },
          },
        },
      });
      assert.equal(initialized.response.status, 200);
      const mcpHeaders = {
        "mcp-session-id": initialized.response.headers.get("mcp-session-id"),
      };

      const missingProfileId = "00000000-0000-0000-0000-000000000000";
      const first = await jsonRequest(
        `${apiBase}/v1/profiles/${missingProfileId}/run`,
        {
          method: "POST",
          token: saved.api_token,
          body: {},
        },
      );
      assert.equal(first.response.status, 404);

      const second = await jsonRequest(mcpUrl, {
        method: "POST",
        headers: mcpHeaders,
        body: {
          jsonrpc: "2.0",
          id: 2,
          method: "tools/call",
          params: {
            name: "run_profile",
            arguments: { profile_id: missingProfileId },
          },
        },
      });
      assert.equal(second.response.status, 200);
      assert.equal(second.value.error.code, -32000);

      const restLimited = await jsonRequest(
        `${apiBase}/v1/profiles/${missingProfileId}/run`,
        {
          method: "POST",
          token: saved.api_token,
          body: {},
        },
      );
      assert.equal(restLimited.response.status, 429);
      assert.ok(Number(restLimited.response.headers.get("retry-after")) > 0);

      const mcpLimited = await jsonRequest(mcpUrl, {
        method: "POST",
        headers: mcpHeaders,
        body: {
          jsonrpc: "2.0",
          id: 3,
          method: "tools/call",
          params: {
            name: "run_profile",
            arguments: { profile_id: missingProfileId },
          },
        },
      });
      assert.equal(mcpLimited.response.status, 429);
      assert.ok(Number(mcpLimited.response.headers.get("retry-after")) > 0);

      const freeCall = await jsonRequest(mcpUrl, {
        method: "POST",
        headers: mcpHeaders,
        body: {
          jsonrpc: "2.0",
          id: 4,
          method: "tools/call",
          params: { name: "list_profiles", arguments: {} },
        },
      });
      assert.equal(freeCall.response.status, 200);
      assert.equal(freeCall.value.error, undefined);

      await app.invoke("stop_mcp_server");
      await app.invoke("stop_api_server");
    },
    {
      extraEnv: {
        DONUT_E2E_REQUESTS_PER_HOUR: "2",
        WAYFERN_TEST_TOKEN: "donut-e2e-rate-limit",
      },
    },
  );
});

test("offline cloud, update, team-lock, trial, and synchronizer contracts are deterministic", async () => {
  await withApp(
    "integrations-contracts",
    async (app) => {
      await assertCommandErrorCode(
        app,
        "start_mcp_server",
        "WAYFERN_TERMS_REQUIRED",
      );
      assert.equal(await app.invoke("cloud_get_user"), null);
      assert.equal(await app.invoke("cloud_get_proxy_usage"), null);
      assert.ok(await app.invoke("cloud_get_wayfern_token"));
      assert.deepEqual(await app.invoke("get_team_locks"), []);
      assert.equal(
        await app.invoke("get_team_lock_status", {
          profileId: "00000000-0000-0000-0000-000000000000",
        }),
        null,
      );
      assert.deepEqual(await app.invoke("get_sync_sessions"), []);
      const startResult = await invokeContract(app, "start_sync_session", {
        leaderProfileId: "00000000-0000-0000-0000-000000000001",
        followerProfileIds: ["00000000-0000-0000-0000-000000000002"],
      });
      assert.equal(startResult.ok, false);
      const stopError = await app.invokeError("stop_sync_session", {
        sessionId: "missing",
      });
      assert.match(stopError, /not found|session/i);
      const removeError = await app.invokeError("remove_sync_follower", {
        sessionId: "missing",
        followerProfileId: "missing",
      });
      assert.match(removeError, /not found|session/i);

      assert.equal(await app.invoke("check_for_app_updates"), null);
      assert.equal(await app.invoke("check_for_app_updates_manual"), null);
      assert.ok(
        await invokeContract(app, "cloud_exchange_device_code", {
          code: "DONUT-E2E-INVALID-CODE",
        }),
      );
      assert.ok(await invokeContract(app, "cloud_refresh_profile"));
      assert.ok(await invokeContract(app, "cloud_get_countries"));
      assert.ok(
        await invokeContract(app, "create_cloud_location_proxy", {
          name: "E2E unavailable cloud proxy",
          country: "ZZ",
          region: null,
          city: null,
          isp: null,
        }),
      );
      assert.ok(await invokeContract(app, "cloud_refresh_wayfern_token"));

      assert.ok(await invokeContract(app, "trigger_manual_version_update"));
      assert.ok(
        await invokeContract(app, "clear_all_version_cache_and_refetch"),
      );
      assert.ok(await invokeContract(app, "check_for_browser_updates"));
      await app.invoke("dismiss_update_notification", {
        notificationId: "missing-e2e-notification",
      });
      assert.deepEqual(
        await app.invoke("complete_browser_update_with_auto_update", {
          browser: "wayfern",
          newVersion: "150.0.7871.100",
        }),
        [],
      );
      const prepareError = await app.invokeError(
        "download_and_prepare_app_update",
        {
          updateInfo: {
            current_version: "0.0.0",
            new_version: "0.0.1-e2e",
            release_notes: "E2E invalid update contract",
            download_url: `${process.env.DONUT_E2E_FIXTURE_URL}/invalid-update.zip`,
            is_nightly: false,
            published_at: "2026-01-01T00:00:00Z",
            manual_update_required: false,
            release_page_url: null,
            repo_update: false,
            checksums_url: null,
            asset_digest: null,
          },
        },
      );
      assert.match(prepareError, /checksum|verif|Failed to download/i);
      const versionStatus = await app.invoke("get_version_update_status");
      assert.ok(versionStatus && typeof versionStatus === "object");
      assert.equal(typeof (await app.invoke("is_default_browser")), "boolean");

      // Remote sessions and the cookie bot are brokered by the cloud backend.
      // Signed out, every one of them must fail as a code the UI can
      // translate — a raw English string from the transport would reach the
      // user untranslated, which is what the {"code":…} convention prevents.
      const notSignedIn = /"code":"CLOUD_NOT_SIGNED_IN"/;
      const missingProfileId = "00000000-0000-0000-0000-0000000000ff";
      assert.match(await app.invokeError("list_remote_sessions"), notSignedIn);
      assert.match(
        await app.invokeError("get_remote_session", {
          sessionId: "missing-e2e-session",
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("stop_remote_session", {
          sessionId: "missing-e2e-session",
        }),
        notSignedIn,
      );

      // The transition stream is what the desktop uses instead of polling, so
      // its subscriber has to start, report itself, and stop on demand. Both
      // calls are repeated: a second start must not open a second socket, and
      // a second stop must not fail.
      assert.equal(await app.invoke("get_remote_session_events_status"), false);
      await app.invoke("start_remote_session_events");
      assert.equal(await app.invoke("get_remote_session_events_status"), true);
      await app.invoke("start_remote_session_events");
      assert.equal(await app.invoke("get_remote_session_events_status"), true);
      await app.invoke("stop_remote_session_events");
      assert.equal(await app.invoke("get_remote_session_events_status"), false);
      await app.invoke("stop_remote_session_events");
      assert.equal(await app.invoke("get_remote_session_events_status"), false);

      assert.match(
        await app.invokeError("get_cookie_bot_schedules", { scope: "mine" }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_cookie_bot_schedule", {
          profileId: missingProfileId,
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("delete_cookie_bot_schedule", {
          profileId: missingProfileId,
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("check_cookie_bot_conflicts", {
          profileId: missingProfileId,
          runAtMinute: 120,
          daysMask: 127,
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_cookie_bot_runs", { limit: 10 }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("cancel_cookie_bot_run", {
          runId: "missing-e2e-run",
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_cookie_bot_presets"),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_remote_hours_quota"),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_cookie_bot_usage", { period: "2026-01" }),
        notSignedIn,
      );

      // Enrolling and running act on a profile this machine holds: both are
      // refused before any network call when it does not exist, so a bad id
      // can never reach a leased host or an hour of the pooled budget.
      assert.match(
        await app.invokeError("save_cookie_bot_schedule", {
          profileId: missingProfileId,
          schedule: {
            profile_name: "E2E missing profile",
            platform: "windows",
            enabled: true,
            run_at_minute: 120,
            days_mask: 127,
            timezone: "UTC",
            preset: "balanced",
            max_minutes: 60,
            sites: ["https://example.com"],
          },
          acknowledgeConflict: false,
        }),
        /"code":"PROFILE_NOT_FOUND"/,
      );
      assert.match(
        await app.invokeError("run_cookie_bot_now", {
          profileId: missingProfileId,
          maxMinutes: 30,
        }),
        /"code":"PROFILE_NOT_FOUND"/,
      );

      const trial = await app.invoke("get_commercial_trial_status");
      assert.ok(trial && typeof trial === "object");
      await app.invoke("acknowledge_trial_expiration");
      assert.equal(await app.invoke("has_acknowledged_trial_expiration"), true);
      await app.invoke("cloud_logout");
      assert.equal(await app.invoke("cloud_get_user"), null);
    },
    { wayfernTermsAccepted: false },
  );
});

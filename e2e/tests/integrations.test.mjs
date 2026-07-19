import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { withApp } from "../lib/app.mjs";

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
    const port = await app.invoke("start_mcp_server");
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

    const agents = await app.invoke("list_mcp_agents");
    assert.ok(agents.some((agent) => agent.id === "cursor"));
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

test("offline cloud, update, team-lock, trial, and synchronizer contracts are deterministic", async () => {
  await withApp("integrations-contracts", async (app) => {
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
      await invokeContract(app, "cloud_get_regions", {
        country: "ZZ",
      }),
    );
    assert.ok(
      await invokeContract(app, "cloud_get_cities", {
        country: "ZZ",
        region: null,
      }),
    );
    assert.ok(
      await invokeContract(app, "cloud_get_isps", {
        country: "ZZ",
        region: null,
        city: null,
      }),
    );
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
    assert.ok(await invokeContract(app, "clear_all_version_cache_and_refetch"));
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

    const trial = await app.invoke("get_commercial_trial_status");
    assert.ok(trial && typeof trial === "object");
    await app.invoke("acknowledge_trial_expiration");
    assert.equal(await app.invoke("has_acknowledged_trial_expiration"), true);
    await app.invoke("cloud_logout");
    assert.equal(await app.invoke("cloud_get_user"), null);
  });
});

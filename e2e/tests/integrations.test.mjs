import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { withApp } from "../lib/app.mjs";
import {
  extensionZipBase64,
  LOCALIZED_EXTENSION_MESSAGES,
  localizedExtensionZipBase64,
  OVERSIZED_EXTENSION_NAME,
  oversizedExtensionZipBase64,
  writeUnpackedExtension,
} from "../lib/fixtures.mjs";

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
    // The served spec comes from the hand-maintained ApiDoc derive, not from
    // the router, so an extension route can answer requests while being absent
    // from the contract an agent generates its client from.
    for (const [route, methods] of [
      ["/v1/extensions", ["get", "post"]],
      ["/v1/extensions/{id}", ["get", "put", "delete"]],
      ["/v1/extension-groups", ["get", "post"]],
      ["/v1/extension-groups/{id}", ["get", "put", "delete"]],
      [
        "/v1/extension-groups/{id}/extensions/{extension_id}",
        ["post", "delete"],
      ],
    ]) {
      assert.ok(paths.includes(route), `OpenAPI is missing ${route}`);
      for (const method of methods) {
        assert.ok(
          openapi.value.paths[route][method],
          `OpenAPI is missing ${method.toUpperCase()} ${route}`,
        );
      }
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

    // Extensions arrive either as an inline payload or as a path the app can
    // read, and the folder form is the whole point: it is how an agent reaches
    // the "load unpacked" flow that the desktop offers through a file picker.
    const archiveExtension = await jsonRequest(`${base}/v1/extensions`, {
      method: "POST",
      token: saved.api_token,
      body: {
        name: "REST Archive Extension",
        file_name: "fixture.zip",
        file_data_base64: extensionZipBase64(),
      },
    });
    assert.equal(
      archiveExtension.response.status,
      201,
      JSON.stringify(archiveExtension.value),
    );
    assert.equal(archiveExtension.value.name, "Donut E2E Fixture");
    assert.equal(archiveExtension.value.source_kind, "archive");
    assert.equal(archiveExtension.value.linked_path, null);

    const unpackedDir = await writeUnpackedExtension(
      path.join(app.root, "fixtures", "rest-unpacked-extension"),
      { name: "Donut REST Unpacked", version: "1.2.0" },
    );
    const folderExtension = await jsonRequest(`${base}/v1/extensions`, {
      method: "POST",
      token: saved.api_token,
      body: { name: "REST Folder Extension", source_path: unpackedDir },
    });
    assert.equal(
      folderExtension.response.status,
      201,
      JSON.stringify(folderExtension.value),
    );
    assert.equal(folderExtension.value.name, "Donut REST Unpacked");
    assert.equal(folderExtension.value.version, "1.2.0");
    assert.equal(folderExtension.value.source_kind, "unpacked");
    assert.equal(folderExtension.value.linked_path, null);

    // Two sources in one request have no defined winner, so the request is
    // refused rather than silently resolved.
    const ambiguousSource = await jsonRequest(`${base}/v1/extensions`, {
      method: "POST",
      token: saved.api_token,
      body: {
        name: "REST Ambiguous Extension",
        file_name: "fixture.zip",
        file_data_base64: extensionZipBase64(),
        source_path: unpackedDir,
      },
    });
    assert.equal(ambiguousSource.response.status, 400);
    assert.match(
      JSON.stringify(ambiguousSource.value),
      /EXTENSION_SOURCE_REQUIRED/,
    );
    const sourcelessExtension = await jsonRequest(`${base}/v1/extensions`, {
      method: "POST",
      token: saved.api_token,
      body: { name: "REST Sourceless Extension" },
    });
    assert.equal(sourcelessExtension.response.status, 400);
    assert.match(
      JSON.stringify(sourcelessExtension.value),
      /EXTENSION_SOURCE_REQUIRED/,
    );
    // An archive has no folder to keep loading from, so linking one is refused
    // rather than quietly stored as a copy.
    const linkedArchive = await jsonRequest(`${base}/v1/extensions`, {
      method: "POST",
      token: saved.api_token,
      body: {
        name: "REST Linked Archive",
        file_name: "fixture.zip",
        file_data_base64: extensionZipBase64(),
        link: true,
      },
    });
    assert.equal(linkedArchive.response.status, 400);
    assert.match(
      JSON.stringify(linkedArchive.value),
      /EXTENSION_LINK_REQUIRES_DIRECTORY/,
    );

    const extensionId = folderExtension.value.id;
    assert.equal(
      (
        await jsonRequest(`${base}/v1/extensions/${extensionId}`, {
          token: saved.api_token,
        })
      ).value.id,
      extensionId,
    );
    const renamedExtension = await jsonRequest(
      `${base}/v1/extensions/${extensionId}`,
      {
        method: "PUT",
        token: saved.api_token,
        body: { name: "REST Renamed Extension" },
      },
    );
    assert.equal(
      renamedExtension.response.status,
      200,
      JSON.stringify(renamedExtension.value),
    );
    assert.equal(renamedExtension.value.name, "REST Renamed Extension");
    assert.equal(
      (await jsonRequest(`${base}/v1/extensions`, { token: saved.api_token }))
        .value.length,
      2,
    );

    // Axum's default body limit is 2 MiB, which plenty of real `.crx` files
    // exceed: every one of them was refused before the handler ran until the
    // extension payload routes got a limit of their own. The fixture below is
    // stored rather than deflated, so the body genuinely stays over the
    // default and a 201 can only come from the raised limit.
    const oversizedBody = {
      name: "REST Oversized Extension",
      file_name: "oversized.zip",
      file_data_base64: oversizedExtensionZipBase64(),
    };
    assert.ok(
      Buffer.byteLength(JSON.stringify(oversizedBody)) > 2 * 1024 * 1024,
      "the oversized fixture must exceed the default body limit it tests",
    );
    const oversized = await jsonRequest(`${base}/v1/extensions`, {
      method: "POST",
      token: saved.api_token,
      body: oversizedBody,
    });
    assert.equal(
      oversized.response.status,
      201,
      JSON.stringify(oversized.value),
    );
    // Read out of the archive that arrived, so the payload landed whole rather
    // than merely being accepted.
    assert.equal(oversized.value.name, OVERSIZED_EXTENSION_NAME);
    assert.equal(oversized.value.version, "1.0.0");
    assert.equal(oversized.value.file_type, "zip");

    // The raised limit is scoped to the two paths that carry a payload. A
    // group name is never megabytes long, so a route that accepted one would
    // mean the layer had been attached to the whole router.
    const oversizedGroupBody = { name: "G".repeat(3 * 1024 * 1024) };
    assert.ok(
      Buffer.byteLength(JSON.stringify(oversizedGroupBody)) > 2 * 1024 * 1024,
    );
    const oversizedGroup = await jsonRequest(`${base}/v1/extension-groups`, {
      method: "POST",
      token: saved.api_token,
      body: oversizedGroupBody,
    });
    assert.equal(
      oversizedGroup.response.status,
      413,
      JSON.stringify(oversizedGroup.value),
    );
    assert.deepEqual(
      (
        await jsonRequest(`${base}/v1/extension-groups`, {
          token: saved.api_token,
        })
      ).value,
      [],
      "the refused group request must not have stored anything",
    );

    // Chrome Web Store extensions overwhelmingly localize their manifest: the
    // name a user recognizes sits in `_locales/<default_locale>/messages.json`
    // and the manifest holds `__MSG_extName__`. Storing the manifest verbatim
    // is what puts a raw placeholder in the extension list.
    const localized = await jsonRequest(`${base}/v1/extensions`, {
      method: "POST",
      token: saved.api_token,
      body: {
        name: "REST Localized Extension",
        file_name: "localized.zip",
        file_data_base64: localizedExtensionZipBase64(),
      },
    });
    assert.equal(
      localized.response.status,
      201,
      JSON.stringify(localized.value),
    );
    assert.equal(localized.value.name, LOCALIZED_EXTENSION_MESSAGES.extName);
    assert.equal(
      localized.value.description,
      LOCALIZED_EXTENSION_MESSAGES.extDescription,
    );
    assert.equal(
      localized.value.author,
      LOCALIZED_EXTENSION_MESSAGES.extAuthor,
    );
    assert.doesNotMatch(JSON.stringify(localized.value), /__MSG_/);
    // The resolved strings have to be what was persisted, not something the
    // create response computed on its way out.
    assert.equal(
      (
        await jsonRequest(`${base}/v1/extensions/${localized.value.id}`, {
          token: saved.api_token,
        })
      ).value.name,
      LOCALIZED_EXTENSION_MESSAGES.extName,
    );

    // A placeholder the locale file cannot resolve falls back to the name the
    // caller sent. What it must never do is store `__MSG_extName__` itself.
    const unresolved = await jsonRequest(`${base}/v1/extensions`, {
      method: "POST",
      token: saved.api_token,
      body: {
        name: "REST Unresolved Placeholder",
        file_name: "unresolved.zip",
        file_data_base64: localizedExtensionZipBase64({ messages: {} }),
      },
    });
    assert.equal(
      unresolved.response.status,
      201,
      JSON.stringify(unresolved.value),
    );
    assert.equal(unresolved.value.name, "REST Unresolved Placeholder");
    assert.equal(unresolved.value.description, null);
    assert.equal(unresolved.value.author, null);
    assert.doesNotMatch(JSON.stringify(unresolved.value), /__MSG_/);

    const extensionGroup = await jsonRequest(`${base}/v1/extension-groups`, {
      method: "POST",
      token: saved.api_token,
      body: { name: "REST Extension Group" },
    });
    assert.equal(
      extensionGroup.response.status,
      201,
      JSON.stringify(extensionGroup.value),
    );
    assert.equal(extensionGroup.value.name, "REST Extension Group");
    assert.deepEqual(extensionGroup.value.extension_ids, []);
    const extensionGroupId = extensionGroup.value.id;
    const renamedExtensionGroup = await jsonRequest(
      `${base}/v1/extension-groups/${extensionGroupId}`,
      {
        method: "PUT",
        token: saved.api_token,
        body: { name: "REST Extension Group Updated" },
      },
    );
    assert.equal(
      renamedExtensionGroup.response.status,
      200,
      JSON.stringify(renamedExtensionGroup.value),
    );
    assert.equal(
      renamedExtensionGroup.value.name,
      "REST Extension Group Updated",
    );

    const membershipUrl = `${base}/v1/extension-groups/${extensionGroupId}/extensions/${extensionId}`;
    const joined = await jsonRequest(membershipUrl, {
      method: "POST",
      token: saved.api_token,
    });
    assert.equal(joined.response.status, 200, JSON.stringify(joined.value));
    assert.deepEqual(joined.value.extension_ids, [extensionId]);
    assert.deepEqual(
      (
        await jsonRequest(`${base}/v1/extension-groups/${extensionGroupId}`, {
          token: saved.api_token,
        })
      ).value.extension_ids,
      [extensionId],
    );
    const left = await jsonRequest(membershipUrl, {
      method: "DELETE",
      token: saved.api_token,
    });
    assert.equal(left.response.status, 200, JSON.stringify(left.value));
    assert.deepEqual(left.value.extension_ids, []);
    assert.deepEqual(
      (
        await jsonRequest(`${base}/v1/extension-groups/${extensionGroupId}`, {
          token: saved.api_token,
        })
      ).value.extension_ids,
      [],
    );

    // The whole path an automation client takes: an extension, a group holding
    // it, and a profile that will load that group the next time it launches.
    // Each piece already had coverage; the sequence did not, and it is the
    // sequence that has to work for extensions to be usable over REST at all.
    const launchProfile = await app.invoke("create_browser_profile_new", {
      name: "REST Extension Profile",
      browserStr: "wayfern",
      version: "150.0.7871.100",
      releaseType: "stable",
      proxyId: null,
      vpnId: null,
      // A stored fingerprint keeps this suite off the real browser; the
      // browser suite covers generation.
      wayfernConfig: { fingerprint: "{}" },
      groupId: null,
      ephemeral: false,
      dnsBlocklist: null,
      launchHook: null,
    });
    const launchGroup = await jsonRequest(`${base}/v1/extension-groups`, {
      method: "POST",
      token: saved.api_token,
      body: { name: "REST Launch Extension Group" },
    });
    assert.equal(
      launchGroup.response.status,
      201,
      JSON.stringify(launchGroup.value),
    );
    const launchGroupId = launchGroup.value.id;
    assert.deepEqual(
      (
        await jsonRequest(
          `${base}/v1/extension-groups/${launchGroupId}/extensions/${archiveExtension.value.id}`,
          { method: "POST", token: saved.api_token },
        )
      ).value.extension_ids,
      [archiveExtension.value.id],
    );
    const assigned = await jsonRequest(
      `${base}/v1/profiles/${launchProfile.id}`,
      {
        method: "PUT",
        token: saved.api_token,
        body: { extension_group_id: launchGroupId },
      },
    );
    assert.equal(assigned.response.status, 200, JSON.stringify(assigned.value));
    assert.equal(assigned.value.profile.id, launchProfile.id);
    // `ApiProfile` carries no `extension_group_id`, so the assignment can only
    // be read back through the surface the launcher itself resolves.
    const assignedGroup = () =>
      app.invoke("get_extension_group_for_profile", {
        profileId: launchProfile.id,
      });
    assert.equal((await assignedGroup()).id, launchGroupId);
    assert.deepEqual((await assignedGroup()).extension_ids, [
      archiveExtension.value.id,
    ]);

    // A group that does not exist used to be stored anyway and fail at launch,
    // far from the request that caused it.
    const missingExtensionGroup = await jsonRequest(
      `${base}/v1/profiles/${launchProfile.id}`,
      {
        method: "PUT",
        token: saved.api_token,
        body: { extension_group_id: "00000000-0000-0000-0000-0000000000ee" },
      },
    );
    assert.equal(
      missingExtensionGroup.response.status,
      404,
      JSON.stringify(missingExtensionGroup.value),
    );
    assert.equal(
      (await assignedGroup()).id,
      launchGroupId,
      "a refused assignment must leave the previous one in place",
    );

    assert.equal(
      (
        await jsonRequest(`${base}/v1/profiles/${launchProfile.id}`, {
          method: "PUT",
          token: saved.api_token,
          body: { extension_group_id: "" },
        })
      ).response.status,
      200,
    );
    assert.equal(await assignedGroup(), null);
    assert.equal(
      (
        await jsonRequest(`${base}/v1/extension-groups/${launchGroupId}`, {
          method: "DELETE",
          token: saved.api_token,
        })
      ).response.status,
      204,
    );
    await app.invoke("delete_profile", { profileId: launchProfile.id });

    for (const id of [
      extensionId,
      archiveExtension.value.id,
      oversized.value.id,
      localized.value.id,
      unresolved.value.id,
    ]) {
      assert.equal(
        (
          await jsonRequest(`${base}/v1/extensions/${id}`, {
            method: "DELETE",
            token: saved.api_token,
          })
        ).response.status,
        204,
      );
    }
    assert.equal(
      (
        await jsonRequest(`${base}/v1/extension-groups/${extensionGroupId}`, {
          method: "DELETE",
          token: saved.api_token,
        })
      ).response.status,
      204,
    );

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
      // The remote loop has to be complete from MCP alone: start a session,
      // watch it become usable, drive it with the interaction tools above, stop
      // it. Any one of these missing leaves an agent able to lease a host it
      // cannot use, or unable to lease one at all.
      "run_profile_remote",
      "get_remote_session",
      "stop_remote_session",
      // Extension management is only usable from an agent if importing and
      // grouping are reachable, not just listing and deleting.
      "add_extension",
      "update_extension",
      "add_extension_to_group",
      "remove_extension_from_group",
      "update_extension_group",
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

    let toolCallId = 7;
    const callTool = (name, args) =>
      jsonRequest(`${base}/mcp/${config.token}`, {
        method: "POST",
        headers: mcpHeaders,
        body: {
          jsonrpc: "2.0",
          id: toolCallId++,
          method: "tools/call",
          params: { name, arguments: args },
        },
      });

    const unpackedDir = await writeUnpackedExtension(
      path.join(app.root, "fixtures", "mcp-unpacked-extension"),
      { name: "Donut MCP Unpacked", version: "1.0.0" },
    );
    const addedExtension = await callTool("add_extension", {
      path: unpackedDir,
      name: "MCP Folder Extension",
    });
    assert.equal(addedExtension.response.status, 200);
    const subscriptionGated = /subscription/i.test(
      addedExtension.value.error?.message ?? "",
    );
    // The e2e build overrides the paid-plan gate whenever a Wayfern test token
    // is present, so with one in the environment a gated answer means the
    // override stopped working and everything below it silently stopped
    // running.
    assert.ok(
      !subscriptionGated || !process.env.WAYFERN_TEST_TOKEN,
      `the e2e paid-plan override did not apply: ${addedExtension.value.error?.message}`,
    );
    if (subscriptionGated) {
      // Every extension tool is gated on an active paid plan and this session
      // is signed out, so the call path is unreachable here. The tool list
      // above still proves the tools are published.
      console.warn(
        "Skipping the MCP extension tool calls: this session has no paid entitlement",
      );
    } else {
      assert.equal(addedExtension.value.error, undefined);
      const stored = (await app.invoke("list_extensions")).find(
        (item) => item.name === "Donut MCP Unpacked",
      );
      assert.ok(stored, "the MCP import must produce a stored extension");
      assert.equal(stored.source_kind, "unpacked");
      assert.equal(stored.linked_path, null);

      const renamedExtension = await callTool("update_extension", {
        extension_id: stored.id,
        name: "MCP Renamed Extension",
      });
      assert.equal(renamedExtension.value.error, undefined);
      assert.equal(
        (await app.invoke("list_extensions")).find(
          (item) => item.id === stored.id,
        ).name,
        "MCP Renamed Extension",
      );

      const extensionGroup = await app.invoke("create_extension_group", {
        name: "MCP Extension Group",
      });
      const joined = await callTool("add_extension_to_group", {
        group_id: extensionGroup.id,
        extension_id: stored.id,
      });
      assert.equal(joined.value.error, undefined);
      const readGroup = async () =>
        (await app.invoke("list_extension_groups")).find(
          (item) => item.id === extensionGroup.id,
        );
      assert.deepEqual((await readGroup()).extension_ids, [stored.id]);

      const renamedGroup = await callTool("update_extension_group", {
        group_id: extensionGroup.id,
        name: "MCP Extension Group Updated",
      });
      assert.equal(renamedGroup.value.error, undefined);
      assert.equal((await readGroup()).name, "MCP Extension Group Updated");

      const removed = await callTool("remove_extension_from_group", {
        group_id: extensionGroup.id,
        extension_id: stored.id,
      });
      assert.equal(removed.value.error, undefined);
      assert.deepEqual((await readGroup()).extension_ids, []);

      await app.invoke("delete_extension", { extensionId: stored.id });
      await app.invoke("delete_extension_group", {
        groupId: extensionGroup.id,
      });
    }

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
      // The local-launch gate. Nothing has run remotely in this session, so it
      // is empty — but it must answer, because a UI that cannot read it shows
      // an enabled Run button over a profile the backend will refuse.
      const handoff = await app.invoke("get_remote_handoff_states");
      assert.ok(
        handoff && typeof handoff === "object" && !Array.isArray(handoff),
        "the handoff gate must answer with a profile-keyed object",
      );
      assert.equal(Object.keys(handoff).length, 0);

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
      // Saved site lists are cloud-backed like the schedules above, so they
      // must refuse the same way rather than appearing to work offline.
      assert.match(
        await app.invokeError("get_cookie_bot_user_templates", {}),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("create_cookie_bot_user_template", {
          name: "e2e list",
          sites: ["example.com"],
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("update_cookie_bot_user_template", {
          id: "00000000-0000-0000-0000-000000000000",
          name: "renamed",
          sites: null,
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("delete_cookie_bot_user_template", {
          id: "00000000-0000-0000-0000-000000000000",
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

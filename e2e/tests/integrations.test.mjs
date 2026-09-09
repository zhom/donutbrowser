import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
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

/**
 * Evidence that a command's BODY ran, not merely that it was invoked.
 *
 * `assert.ok(await invokeContract(...))` was the pattern here, and it cannot
 * fail: `invokeContract` always resolves to an object, and every object is
 * truthy. It passed whether the command succeeded, refused, or did not exist at
 * all, so eight commands whose only coverage-map evidence was that line could
 * have had their entire bodies deleted with every suite still green.
 *
 * Each caller now states which of the two outcomes it expects and pins it, so
 * the assertion fails if the command stops reaching its real logic. The two
 * outcomes are both legitimate here: these are cloud and update commands, and a
 * hermetic E2E run has no session, so a refusal FOR THE RIGHT REASON is exactly
 * as much proof that the body ran as a success is.
 */
async function assertContract(app, command, expected, args = {}) {
  const result = await invokeContract(app, command, args);
  if (expected.refusedWith) {
    assert.equal(
      result.ok,
      false,
      `${command} was expected to refuse, but returned ${JSON.stringify(result.value)}`,
    );
    assert.match(
      result.error,
      expected.refusedWith,
      `${command} refused for a different reason than the one that proves its body ran`,
    );
    return result;
  }
  assert.equal(result.ok, true, `${command} failed: ${result.error}`);
  expected.answers(result.value);
  return result;
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
    const diagnostic = await app.invoke("check_integration_connection", {
      target: "api",
    });
    assert.equal(diagnostic.configured, true);
    assert.equal(diagnostic.reachable, true);
    assert.equal(diagnostic.authorized, true);
    assert.equal(diagnostic.http_status, 200);
    assert.ok(diagnostic.checked_at > 0);
    assert.deepEqual(Object.keys(diagnostic).sort(), [
      "authorized",
      "checked_at",
      "configured",
      "http_status",
      "reachable",
    ]);
    const unconfigured = await app.invoke("check_integration_connection", {
      target: "remote",
    });
    assert.equal(unconfigured.configured, false);
    assert.equal(unconfigured.authorized, null);
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
    // A caller that can set the assignment must be able to read it back without
    // dropping to the desktop app.
    assert.equal(
      assigned.value.profile.extension_group_id,
      launchGroupId,
      "the update response must echo the assignment it just made",
    );
    assert.equal(
      (
        await jsonRequest(`${base}/v1/profiles/${launchProfile.id}`, {
          token: saved.api_token,
        })
      ).value.profile.extension_group_id,
      launchGroupId,
      "a fresh GET must report the assignment",
    );
    // Also read it back through the surface the launcher itself resolves, so
    // the REST field and the launch path cannot drift apart.
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
        await jsonRequest(`${base}/v1/profiles/${launchProfile.id}`, {
          token: saved.api_token,
        })
      ).value.profile.extension_group_id,
      null,
      "clearing the assignment must be visible over REST too",
    );
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
    const stoppedDiagnostic = await app.invoke("check_integration_connection", {
      target: "api",
    });
    assert.equal(stoppedDiagnostic.configured, true);
    assert.equal(stoppedDiagnostic.reachable, false);
    assert.equal(stoppedDiagnostic.authorized, null);
  });
});

test("local MCP is removed: enabling it and installing a local client are refused", async () => {
  await withApp("integrations-mcp", async (app) => {
    await seedTerms(app);
    // Enabling the local server is refused with the removal code (it used to
    // start a loopback MCP server), so nothing is left listening as a result.
    await assertCommandErrorCode(app, "start_mcp_server", "MCP_LOCAL_REMOVED");
    assert.equal(await app.invoke("get_mcp_server_status"), false);
    // Nothing is configured for a server that no longer exists.
    assert.equal(await app.invoke("get_mcp_config"), null);
    // stop is a no-op when nothing is running.
    await assertCommandErrorCode(
      app,
      "stop_mcp_server",
      "MCP_SERVER_NOT_RUNNING",
    );

    // The client roster still resolves, so the Integrations page can offer the
    // remote endpoint and show which clients are still on the removed local one.
    const agents = await app.invoke("list_mcp_agents");
    assert.ok(agents.some((agent) => agent.id === "cursor"));
    // fx cannot take the bearer from its config file, so the page tells the
    // user which variable to export; that name travels with the row.
    assert.equal(
      agents.find((agent) => agent.id === "fx").token_env,
      "DONUT_MCP_TOKEN",
    );
    assert.equal(agents.find((agent) => agent.id === "cursor").token_env, null);

    // An unknown agent and an unknown target keep their own distinct errors.
    await assertCommandErrorCode(app, "add_mcp_to_agent", "MCP_AGENT_UNKNOWN", {
      agentId: "missing-e2e-agent",
      target: "remote",
    });
    await assertCommandErrorCode(
      app,
      "remove_mcp_from_agent",
      "MCP_AGENT_UNKNOWN",
      { agentId: "missing-e2e-agent" },
    );
    await assertCommandErrorCode(app, "add_mcp_to_agent", "INTERNAL_ERROR", {
      agentId: "cursor",
      target: "bogus",
    });

    // Installing a client to the LOCAL endpoint is refused now (it used to
    // write a local config), and nothing is written.
    await assertCommandErrorCode(app, "add_mcp_to_agent", "MCP_LOCAL_REMOVED", {
      agentId: "cursor",
      target: "local",
    });
    assert.equal(
      (await app.invoke("list_mcp_agents")).find(
        (agent) => agent.id === "cursor",
      ).connected,
      false,
      "a refused local install must not have written an entry",
    );

    // Remote MCP with no stored `dmk_` credential is refused with a code the UI
    // can explain, and writes nothing into the client's config.
    await assertCommandErrorCode(
      app,
      "add_mcp_to_agent",
      "MCP_REMOTE_KEY_MISSING",
      { agentId: "cursor", target: "remote" },
    );
    assert.equal(
      (await app.invoke("list_mcp_agents")).find(
        (agent) => agent.id === "cursor",
      ).connected,
      false,
      "a refused remote install must not have written an entry",
    );
  });
});

test("the remote-control bridge refuses a signed-out desktop and stays off", async () => {
  await withApp("integrations-mcp-remote", async (app) => {
    await seedTerms(app);

    // Off by default, and it must stay that way without an explicit opt-in: the
    // bridge hands Donut cloud the ability to drive this browser, which is not
    // something to switch on for somebody because their plan allows it.
    const settings = await app.invoke("get_app_settings");
    assert.equal(settings.mcp_remote_enabled, false);

    const initial = await app.invoke("get_mcp_remote_status");
    assert.equal(initial.enabled, false);
    assert.equal(initial.connected, false);
    assert.equal(initial.lastError, null);
    assert.ok(initial.instanceId.length >= 8);

    // Stable across calls, and persisted: the id is how this desktop reclaims
    // its own connection after a blip, so a desktop that renamed itself on
    // every read could never do so.
    assert.equal(
      (await app.invoke("get_mcp_remote_status")).instanceId,
      initial.instanceId,
    );

    // A signed-out desktop has no credential to authenticate the socket with,
    // so it is refused here rather than allowed to open one and be dropped.
    assert.match(
      await app.invokeError("start_mcp_remote_bridge"),
      /"code":"MCP_REMOTE_REQUIRES_SIGN_IN"/,
    );
    // The bridge must not merely be un-enabled: it must not have been STARTED.
    // The sign-in gate sits above `mcp_remote::start`, and `enabled` is the only
    // half that says so: it is `is_running()`, which `start` flips synchronously
    // before it spawns the reconnect loop. `connected` is not a substitute:
    // it only goes true once the socket authenticates, which a signed-out
    // desktop never manages, so it reads false whether or not the bridge was
    // started and is dialling donutbrowser.com in the background.
    const afterRefusedStart = await app.invoke("get_mcp_remote_status");
    assert.equal(
      afterRefusedStart.enabled,
      false,
      "a refused start must not have started the bridge task",
    );
    assert.equal(
      afterRefusedStart.connected,
      false,
      "a refused start must not have opened a socket",
    );
    assert.equal(
      (await app.invoke("get_app_settings")).mcp_remote_enabled,
      false,
      "a refused start must not leave the setting on, or the next launch dials a socket the user never enabled",
    );

    // Stopping something that is not running is a no-op, not an error: the
    // desktop calls this on sign-out and on quit, and both must be safe.
    const stoppedWhileOff = await app.invoke("stop_mcp_remote_bridge");
    assert.equal(stoppedWhileOff.enabled, false);
    assert.equal(stoppedWhileOff.connected, false);
    assert.equal(stoppedWhileOff.instanceId, initial.instanceId);

    // The no-op case says nothing about the PERSISTED effect, and that is the
    // half that matters: `ensure_remote_bridge` re-opens the bridge on the next
    // launch (and on the next sign-in, and on the ten-minute reconnect tick)
    // from `mcp_remote_enabled` alone. So put the flag on disk the way a
    // previously opted-in session would have left it, and stop again.
    //
    // Every field of the returned status is computed from in-memory bridge
    // state (`enabled` is `is_running()`, `connected` is `is_connected()`), so
    // dropping the settings write inside `stop_mcp_remote_bridge` leaves all
    // three assertions above green while the internet-facing bridge comes back
    // by itself. Only the on-disk flag tells a real stop from `Ok(status())`.
    //
    // `save_app_settings` is NOT how the flag gets there. It belongs to the
    // start and stop commands alone: a settings save that carried it would
    // switch the internet-facing bridge on for the next launch without ever
    // passing the sign-in and terms gates those commands enforce. Prove the
    // save cannot flip it, then seed the file directly.
    const beforeSeed = await app.invoke("get_app_settings");
    const saved = await app.invoke("save_app_settings", {
      settings: { ...beforeSeed, mcp_remote_enabled: true },
    });
    assert.equal(
      saved.mcp_remote_enabled,
      false,
      "a settings save must not be able to switch remote control on",
    );
    assert.equal(
      (await app.invoke("get_app_settings")).mcp_remote_enabled,
      false,
      "nor may it reach disk through the save",
    );

    const settingsFile = path.join(
      app.dataRoot,
      "data",
      "settings",
      "app_settings.json",
    );
    const onDisk = JSON.parse(await readFile(settingsFile, "utf8"));
    await writeFile(
      settingsFile,
      `${JSON.stringify({ ...onDisk, mcp_remote_enabled: true }, null, 2)}\n`,
    );
    assert.equal(
      (await app.invoke("get_app_settings")).mcp_remote_enabled,
      true,
      "the seeded opt-in must reach disk, or the stop below proves nothing",
    );

    const stopped = await app.invoke("stop_mcp_remote_bridge");
    assert.equal(stopped.enabled, false);
    assert.equal(stopped.connected, false);
    assert.equal(stopped.instanceId, initial.instanceId);
    assert.equal(
      (await app.invoke("get_app_settings")).mcp_remote_enabled,
      false,
      "stopping must clear the persisted opt-in, or the next launch re-opens the bridge the user just switched off",
    );

    // Entitlement is asked of the SERVER, because neither of the local answers
    // is right: the cached entitlement is per-account, so an entitled
    // enterprise team MEMBER reads as unentitled, and an open socket only
    // proves the plan is active, not that remote control is allowed.
    //
    // Signed out there is no credential to ask with, and the honest outcome is
    // a clean error the dialog swallows to leave the local cache in charge:
    // never a crash, and never a fabricated `true`.
    const entitlementError = await app.invokeError(
      "get_remote_control_entitlement",
    );
    assert.match(entitlementError, /INTERNAL_ERROR|Not logged in/);

    // The remote MCP credential, the `dmk_` key agents present to the remote
    // endpoint. A fresh desktop holds none, and reports exactly that: the
    // shape is `{ present, token_prefix }` with no plaintext anywhere in it.
    assert.deepEqual(await app.invoke("get_mcp_remote_credential"), {
      present: false,
      token_prefix: null,
    });

    // Minting needs the session, so a signed-out desktop is refused with the
    // same code as the bridge, and refused BEFORE anything is stored.
    //
    // Signed in, the answer is `{ token_prefix, failed_clients }`: the key is
    // stored and its predecessor revoked before any client is rewritten, so a
    // client that could not be rewritten is named there rather than turning
    // the rotation into an error the dialog would answer by minting again.
    assert.match(
      await app.invokeError("rotate_mcp_remote_credential"),
      /"code":"MCP_REMOTE_REQUIRES_SIGN_IN"/,
    );
    assert.equal(
      (await app.invoke("get_mcp_remote_credential")).present,
      false,
      "a refused rotation must not have stored a credential",
    );

    // Forgetting nothing is a no-op, not an error: sign-out and the
    // Integrations page both reach it without checking first.
    await app.invoke("forget_mcp_remote_credential");
    assert.deepEqual(await app.invoke("get_mcp_remote_credential"), {
      present: false,
      token_prefix: null,
    });

    // The local MCP server is a separate transport and is unaffected either way.
    assert.equal(await app.invoke("get_mcp_server_status"), false);
  });
});

test("the remote-control bridge refuses before the terms are accepted", async () => {
  // `start_mcp_remote_bridge` has TWO gates (the Wayfern terms, then the
  // signed-in check), and every other test seeds the terms first, so only the
  // second one was ever reached. The first could have been deleted with the
  // whole suite green, which would let a desktop that never accepted the terms
  // open an internet-facing hook into itself.
  //
  // `wayfernTermsAccepted: false` is what makes this session different: the
  // harness seeds the acceptance file by DEFAULT, so merely omitting the
  // explicit `seedTerms` call leaves the terms accepted and this test proves
  // nothing (it first ran that way and hit the sign-in gate instead).
  await withApp(
    "integrations-mcp-remote-terms",
    async (app) => {
      assert.match(
        await app.invokeError("start_mcp_remote_bridge"),
        /"code":"WAYFERN_TERMS_REQUIRED"/,
      );

      const status = await app.invoke("get_mcp_remote_status");
      // `enabled` first, for the same reason as the sign-in gate: it is
      // `is_running()` and flips inside `mcp_remote::start`, so it is what
      // catches a gate that stopped sitting above the start. `connected` alone
      // would stay false on a bridge that was started and merely never got a
      // socket up.
      assert.equal(
        status.enabled,
        false,
        "the bridge may not have been started",
      );
      assert.equal(status.connected, false, "no socket may have been opened");
      assert.equal(
        (await app.invoke("get_app_settings")).mcp_remote_enabled,
        false,
        "a terms refusal must not leave the setting on either",
      );
    },
    { wayfernTermsAccepted: false },
  );
});

test("REST browser automation requests hit the shared automation rate limit", async () => {
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
      const apiBase = `http://127.0.0.1:${apiPort}`;
      const missingProfileId = "00000000-0000-0000-0000-000000000000";

      // The shared automation limiter sits innermost, past auth, so an
      // authenticated automation call consumes a token even when the profile
      // is missing (404). With the window set to 2/hour below the contract is
      // exact: 404, 404, then 429 with a Retry-After. The limiter is shared
      // with the MCP tool engine, whose branch has no automated coverage now
      // that the loopback endpoint is gone: it needs the e2e-only override
      // that `cargo test --lib` does not compile.
      const run = () =>
        jsonRequest(`${apiBase}/v1/profiles/${missingProfileId}/run`, {
          method: "POST",
          token: saved.api_token,
          body: {},
        });
      assert.equal((await run()).response.status, 404);
      assert.equal((await run()).response.status, 404);
      const limited = await run();
      assert.equal(limited.response.status, 429);
      assert.ok(Number(limited.response.headers.get("retry-after")) > 0);
      // Only automation calls spend the budget: a plain read still answers.
      const listed = await jsonRequest(`${apiBase}/v1/profiles`, {
        method: "GET",
        token: saved.api_token,
      });
      assert.equal(listed.response.status, 200);

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
      // Local MCP is removed: the enable command refuses uniformly with the
      // removal code, regardless of whether the terms have been accepted.
      await assertCommandErrorCode(
        app,
        "start_mcp_server",
        "MCP_LOCAL_REMOVED",
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

      // The controls a person uses on a live session. Without one to act on,
      // each has to refuse by its own code rather than pretend it worked: the
      // panel reads these back, and a silent success would leave a button
      // claiming a state the backend never entered.
      assert.match(
        await app.invokeError("set_sync_session_paused", {
          sessionId: "missing",
          paused: true,
        }),
        /SYNC_SESSION_NOT_FOUND/,
      );
      assert.match(
        await app.invokeError("set_sync_follower_held", {
          sessionId: "missing",
          followerProfileId: "missing",
          held: true,
        }),
        /SYNC_SESSION_NOT_FOUND/,
      );
      assert.match(
        await app.invokeError("arrange_sync_windows", {
          sessionId: "missing",
          layout: "grid",
        }),
        /SYNC_SESSION_NOT_FOUND/,
      );
      // An unknown layout never reaches the display: it fails to deserialise.
      assert.ok(
        await app.invokeError("arrange_sync_windows", {
          sessionId: "missing",
          layout: "diagonal",
        }),
      );

      assert.equal(await app.invoke("check_for_app_updates"), null);
      assert.equal(await app.invoke("check_for_app_updates_manual"), null);
      await assertContract(
        app,
        "cloud_exchange_device_code",
        {
          // Any of these proves the command's BODY ran and reached its network
          // layer, which is what the evidence is for. Pinning only the server's
          // "invalid or expired login code" sentence made a test named
          // "offline ... deterministic" depend on a live round-trip to
          // api.donutbrowser.com: red offline, behind a proxy, when the
          // unauthenticated challenge is rate-limited, or the day the backend
          // rewords it, with no signal that the desktop is fine.
          refusedWith:
            /invalid or expired login code|failed to fetch challenge|challenge request failed/i,
        },
        { code: "DONUT-E2E-INVALID-CODE" },
      );
      await assertContract(app, "cloud_refresh_profile", {
        refusedWith: /not logged in/i,
      });
      await assertContract(app, "cloud_get_countries", {
        refusedWith: /not logged in/i,
      });
      await assertContract(
        app,
        "create_cloud_location_proxy",
        { refusedWith: /no cloud proxy available/i },
        {
          name: "E2E unavailable cloud proxy",
          country: "ZZ",
          region: null,
          city: null,
          isp: null,
        },
      );
      await assertContract(app, "cloud_refresh_wayfern_token", {
        // Compared against the value the harness injected, NOT matched against
        // a hex shape. Under the `e2e` feature this command returns
        // WAYFERN_TEST_TOKEN, so a /^[0-9a-f]{32,}$/ assertion only proved the
        // harness's own env var looks like a token: it validated the fixture
        // and would have passed with the command's body deleted. Equality
        // proves the command actually reached the token and returned it.
        answers: (token) =>
          assert.equal(
            String(token),
            process.env.WAYFERN_TEST_TOKEN,
            "the command must return the token it was given, not a different value",
          ),
      });

      await assertContract(app, "trigger_manual_version_update", {
        answers: (report) => {
          assert.ok(
            Array.isArray(report),
            "the update run must report per browser",
          );
          // Non-empty, or the per-entry loop below asserts nothing at all: an
          // `[]` satisfied every check while the command did no work.
          assert.ok(
            report.length > 0,
            `the update run must report at least one browser, got ${JSON.stringify(report)}`,
          );
          for (const entry of report) {
            assert.ok(
              typeof entry.browser === "string" && entry.browser.length > 0,
              `every entry names its browser: ${JSON.stringify(entry)}`,
            );
            assert.equal(typeof entry.updated_successfully, "boolean");
          }
        },
      });
      await assertContract(app, "clear_all_version_cache_and_refetch", {
        answers: (value) => assert.equal(value, null),
      });
      await assertContract(app, "check_for_browser_updates", {
        answers: (updates) =>
          assert.ok(
            Array.isArray(updates),
            `the update check must answer with a list, got ${JSON.stringify(updates)}`,
          ),
      });
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

      // The agent plane is brokered by the same cloud. Signed out, every read
      // and write must refuse as a translatable code, and the two things the
      // desktop can judge for itself — is this profile here, does the goal say
      // anything — must be judged BEFORE any of that, so a bad request never
      // becomes a model bill.
      assert.match(
        await app.invokeError("get_agent_runs", { limit: 5 }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_agent_run", { runId: "missing-e2e-run" }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("cancel_agent_run", { runId: "missing-e2e-run" }),
        notSignedIn,
      );
      assert.match(await app.invokeError("get_agent_recipes"), notSignedIn);
      assert.match(
        await app.invokeError("create_agent_recipe", {
          name: "E2E recipe",
          steps: [{ type: "navigate", url: "https://example.com" }],
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("update_agent_recipe", {
          id: "00000000-0000-0000-0000-000000000000",
          name: "E2E recipe",
          steps: [{ type: "navigate", url: "https://example.com" }],
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("delete_agent_recipe", {
          id: "00000000-0000-0000-0000-000000000000",
        }),
        notSignedIn,
      );

      // A goal is judged before the profile is even looked up: an empty goal is
      // the one refusal that costs nothing to make locally, and it must not
      // depend on being signed in.
      assert.match(
        await app.invokeError("start_agent_run", {
          input: {
            profileId: missingProfileId,
            target: "desktop",
            goal: "   ",
          },
        }),
        /"code":"AGENT_GOAL_INVALID"/,
      );
      assert.match(
        await app.invokeError("start_agent_run", {
          input: {
            profileId: missingProfileId,
            target: "desktop",
            goal: "Open the dashboard and export last week's report",
          },
        }),
        /"code":"PROFILE_NOT_FOUND"/,
      );
      // Recipes are validated the same way, and with the code the rest of the
      // app already uses for a blank name rather than an agent-specific one.
      assert.match(
        await app.invokeError("create_agent_recipe", {
          name: "  ",
          steps: [{ type: "navigate", url: "https://example.com" }],
        }),
        /"code":"NAME_CANNOT_BE_EMPTY"/,
      );
      // A step is an object the API validates, never a line of prose: the old
      // string shape is refused here rather than sent and rejected by the
      // server. So is a step that names an element without saying which.
      for (const steps of [
        [],
        ["open the dashboard"],
        [{ url: "https://example.com" }],
        [{ type: "teleport" }],
        [{ type: "click" }],
        [{ type: "click", selector: "#buy", locator: { role: "button" } }],
      ]) {
        assert.match(
          await app.invokeError("create_agent_recipe", {
            name: "E2E recipe",
            steps,
          }),
          /"code":"AGENT_RECIPE_INVALID"/,
          `${JSON.stringify(steps)} must be refused before any network call`,
        );
      }

      // The step stream is how the run panel fills; without it a page opened
      // during a run shows a goal and nothing else. It has to start, name the
      // run it is watching, follow a switch to another run, and stop on demand.
      assert.equal(await app.invoke("get_agent_run_events_status"), null);
      await app.invoke("start_agent_run_events", { runId: "e2e-agent-run-1" });
      assert.equal(
        await app.invoke("get_agent_run_events_status"),
        "e2e-agent-run-1",
      );
      // A second start for the same run is a no-op, not a second socket.
      await app.invoke("start_agent_run_events", { runId: "e2e-agent-run-1" });
      assert.equal(
        await app.invoke("get_agent_run_events_status"),
        "e2e-agent-run-1",
      );
      // Opening a different run replaces the stream: the panel shows one run.
      await app.invoke("start_agent_run_events", { runId: "e2e-agent-run-2" });
      assert.equal(
        await app.invoke("get_agent_run_events_status"),
        "e2e-agent-run-2",
      );
      await app.invoke("stop_agent_run_events");
      assert.equal(await app.invoke("get_agent_run_events_status"), null);
      // A second stop must not fail.
      await app.invoke("stop_agent_run_events");
      assert.equal(await app.invoke("get_agent_run_events_status"), null);

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

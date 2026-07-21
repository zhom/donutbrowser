import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { isIP } from "node:net";
import path from "node:path";
import test from "node:test";
import { appFromEnvironment } from "../lib/app.mjs";
import { CdpClient } from "../lib/cdp.mjs";
import {
  extensionZipBase64,
  prepareWayfern,
  wireGuardFixture,
} from "../lib/fixtures.mjs";

function proxySettings(raw, expectedKind) {
  assert.ok(raw, `${expectedKind} residential proxy URL is required`);
  const url = new URL(raw);
  const rawType = url.protocol.slice(0, -1).toLowerCase();
  const proxyType =
    rawType === "socks" || rawType === "socks5h" ? "socks5" : rawType;
  if (expectedKind === "HTTP") {
    assert.ok(
      proxyType === "http" || proxyType === "https",
      `Expected an HTTP proxy URL, got ${rawType}`,
    );
  } else {
    assert.equal(proxyType, "socks5");
  }
  const port = Number(url.port);
  assert.ok(url.hostname && port > 0 && port <= 65535);
  return {
    proxy_type: proxyType,
    host: url.hostname,
    port,
    username: url.username ? decodeURIComponent(url.username) : null,
    password: url.password ? decodeURIComponent(url.password) : null,
  };
}

function wireGuardFields(config) {
  let section = "";
  const fields = new Map();
  for (const rawLine of config.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    if (line === "[Interface]") {
      section = "interface";
      continue;
    }
    if (line === "[Peer]") {
      section = "peer";
      continue;
    }
    const separator = line.indexOf("=");
    if (separator === -1) continue;
    fields.set(
      `${section}.${line.slice(0, separator).trim()}`,
      line.slice(separator + 1).trim(),
    );
  }
  return {
    privateKey: fields.get("interface.PrivateKey"),
    address: fields.get("interface.Address"),
    dns: fields.get("interface.DNS") ?? "",
    peerPublicKey: fields.get("peer.PublicKey"),
    peerEndpoint: fields.get("peer.Endpoint"),
    allowedIps: fields.get("peer.AllowedIPs") ?? "0.0.0.0/0",
    persistentKeepalive: fields.get("peer.PersistentKeepalive") ?? "",
    presharedKey: fields.get("peer.PresharedKey") ?? "",
  };
}

async function request(url, { method = "GET", token, body } = {}) {
  const response = await fetch(url, {
    method,
    headers: {
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      ...(body === undefined ? {} : { "content-type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let value = text;
  if (text) {
    try {
      value = JSON.parse(text);
    } catch {
      // Plain-text responses are intentional for some endpoints.
    }
  }
  return { response, value };
}

async function createGroupThroughUi(app) {
  await app.clickSelector('[aria-label="Groups"]');
  await app.waitForText("Profile groups");
  await app.clickSelector('[aria-label="Create"]');
  await app.waitForText("Create New Group");
  await app.fillSelector("#group-name", "Visible UI Group");
  await app.clickTextIn('[role="dialog"]', "Create", { roles: ["button"] });
  await app.waitForText("Visible UI Group");
  const groups = await app.invoke("get_profile_groups");
  return groups.find((group) => group.name === "Visible UI Group");
}

async function createProxyThroughUi(app, settings) {
  await app.clickSelector('[aria-label="Network"]');
  await app.waitForText("New proxy");
  await app.clickSelector('[aria-label="New proxy"]');
  await app.waitForText("Add Proxy");
  await app.fillSelector("#proxy-name", "Visible Residential HTTP");
  await app.fillSelector("#proxy-host", settings.host);
  await app.fillSelector("#proxy-port", String(settings.port));
  if (settings.username)
    await app.fillSelector("#proxy-username", settings.username);
  if (settings.password)
    await app.fillSelector("#proxy-password", settings.password);
  await app.clickTextIn('[role="dialog"]', "Add Proxy", {
    roles: ["button"],
  });
  await app.waitForText("Visible Residential HTTP");
  const proxies = await app.invoke("get_stored_proxies");
  return proxies.find((proxy) => proxy.name === "Visible Residential HTTP");
}

async function createVpnThroughUi(app, config) {
  const fields = wireGuardFields(config);
  assert.ok(
    fields.privateKey &&
      fields.address &&
      fields.peerPublicKey &&
      fields.peerEndpoint,
    "WireGuard fixture is missing required fields",
  );
  await app.clickText("VPNs", { exact: false, roles: ["tab"] });
  await app.clickSelector('[aria-label="New VPN"]');
  await app.waitForText("Create WireGuard VPN");
  await app.fillSelector("#wg-name", "Visible Local WireGuard");
  await app.fillSelector("#wg-private-key", fields.privateKey);
  await app.fillSelector("#wg-address", fields.address);
  if (fields.dns) await app.fillSelector("#wg-dns", fields.dns);
  await app.fillSelector("#wg-peer-public-key", fields.peerPublicKey);
  await app.fillSelector("#wg-peer-endpoint", fields.peerEndpoint);
  await app.fillSelector("#wg-allowed-ips", fields.allowedIps);
  if (fields.persistentKeepalive) {
    await app.fillSelector("#wg-keepalive", fields.persistentKeepalive);
  }
  if (fields.presharedKey) {
    await app.fillSelector("#wg-preshared-key", fields.presharedKey);
  }
  await app.clickTextIn('[role="dialog"]', "Create VPN", {
    roles: ["button"],
  });
  await app.waitForText("Visible Local WireGuard");
  const vpns = await app.invoke("list_vpn_configs");
  return vpns.find((vpn) => vpn.name === "Visible Local WireGuard");
}

async function createExtensionsThroughUi(app) {
  const extensionFile = path.join(app.root, "visible-extension.zip");
  await writeFile(extensionFile, Buffer.from(extensionZipBase64(), "base64"));
  await app.clickSelector('[aria-label="Extensions"]');
  await app.waitForText("Upload");

  await app.execute(`
    const input = document.querySelector("#ext-file-input");
    input.classList.remove("hidden");
    input.style.position = "fixed";
    input.style.left = "12px";
    input.style.bottom = "12px";
  `);
  const input = await app.session.findCss("#ext-file-input");
  await app.session.sendKeys(input, extensionFile);
  await app.waitForText("visible-extension.zip");
  await app.fillSelector(
    'input[placeholder="Extension name"]',
    "Visible UI Extension",
  );
  await app.clickText("Add", { roles: ["button"] });
  await app.waitForText("Donut E2E Fixture");

  await app.clickText("Groups", { exact: false, roles: ["tab"] });
  await app.clickSelector('[aria-label="New group"]');
  await app.fillSelector(
    'input[placeholder="Group name"]',
    "Visible Extension Group",
  );
  await app.clickText("Create", { roles: ["button"] });
  await app.waitForText("Visible Extension Group");

  let [extensions, groups] = await Promise.all([
    app.invoke("list_extensions"),
    app.invoke("list_extension_groups"),
  ]);
  const extension = extensions.find(
    (item) => item.name === "Donut E2E Fixture",
  );
  let group = groups.find((item) => item.name === "Visible Extension Group");
  assert.ok(extension && group);

  const editButton = await app.execute(
    `
      const row = [...document.querySelectorAll("tr")].find((candidate) =>
        (candidate.innerText || "").includes(arguments[0])
      );
      return row?.querySelector("td:last-child button") ?? null;
    `,
    [group.name],
  );
  assert.ok(editButton, "Extension group edit control was not visible");
  await app.session.click(editButton);
  await app.waitForText("Edit Group");
  const extensionPicker = await app.execute(`
    const dialogs = [...document.querySelectorAll('[role="dialog"]')];
    return dialogs.reverse().find(
      (dialog) => (dialog.innerText || "").includes("Edit Group")
    )?.querySelector('[role="combobox"]') ?? null;
  `);
  assert.ok(extensionPicker, "Extension picker was not visible");
  await app.session.click(extensionPicker);
  await app.clickText(extension.name, { roles: ["option"] });
  await app.clickTextIn('[role="dialog"]', "Save", { roles: ["button"] });
  await app.waitFor(
    async () => {
      groups = await app.invoke("list_extension_groups");
      group = groups.find((item) => item.name === "Visible Extension Group");
      return group?.extension_ids.includes(extension.id);
    },
    { description: "uploaded extension added to visible extension group" },
  );

  return {
    extension,
    group,
  };
}

async function createProfileThroughUi(app, groupName) {
  await app.clickSelector('[aria-label="Profiles"]');
  await app.clickText(groupName, { exact: false, roles: ["button"] });
  await app.clickText("New", { roles: ["button"] });
  await app.waitFor(
    async () => {
      const text = await app.bodyText();
      return (
        text.includes("Create New Profile") ||
        text.includes("Create New Chromium Profile")
      );
    },
    { description: "profile creation dialog" },
  );
  if (!(await app.visibleTextIncludes("Create New Chromium Profile"))) {
    await app.clickText("Chromium", { exact: false, roles: ["button"] });
    await app.waitForText("Create New Chromium Profile");
  }
  await app.fillSelector("#profile-name", "Visible Network Profile");
  await app.clickTextIn('[role="dialog"]', "Create", { roles: ["button"] });
  await app.waitForText("Visible Network Profile", 60_000);
  await app.waitFor(
    async () =>
      !(await app.execute(`
        return [...document.querySelectorAll('[role="dialog"]')].some(
          (dialog) =>
            (dialog.innerText || "").includes("Create New Chromium Profile")
        );
      `)),
    { description: "profile creation dialog to unmount" },
  );
  const profiles = await app.invoke("list_browser_profiles");
  return profiles.find((profile) => profile.name === "Visible Network Profile");
}

async function assignNetworkThroughUi(app, profileName, currentName, newName) {
  const trigger = await app.execute(
    `
      const row = [...document.querySelectorAll("tr")].find((candidate) =>
        (candidate.innerText || "").includes(arguments[0])
      );
      const expected = arguments[1].toLocaleLowerCase();
      return [...(row?.querySelectorAll('[aria-haspopup="dialog"]') ?? [])].find(
        (trigger) => (trigger.innerText || trigger.textContent || "")
          .toLocaleLowerCase()
          .includes(expected)
      ) ?? null;
    `,
    [profileName, currentName],
  );
  assert.ok(trigger, `Network selector for ${profileName} was not visible`);
  await app.session.click(trigger);
  await app.clickText(newName, { exact: false, roles: ["option"] });
  await app.waitFor(
    () =>
      app.execute(
        `
          return ![...document.querySelectorAll('[data-slot="popover-content"]')]
            .some((content) => (content.innerText || "").includes(arguments[0]));
        `,
        [newName],
      ),
    { description: `${newName} network picker to unmount` },
  );
}

async function assignExtensionGroupThroughUi(
  app,
  profileName,
  currentName,
  newName,
) {
  const trigger = await app.execute(
    `
      const row = [...document.querySelectorAll("tr")].find((candidate) =>
        (candidate.innerText || "").includes(arguments[0])
      );
      return [...(row?.querySelectorAll("button") ?? [])].find(
        (button) => (button.innerText || button.textContent || "")
          .trim()
          .includes(arguments[1])
      ) ?? null;
    `,
    [profileName, currentName],
  );
  assert.ok(trigger, `Extension selector for ${profileName} was not visible`);
  await app.session.click(trigger);
  await app.clickText(newName, { exact: false, roles: ["option"] });
  await app.waitFor(
    () =>
      app.execute(
        `
          return ![...document.querySelectorAll('[data-slot="popover-content"]')]
            .some((content) => (content.innerText || "").includes(arguments[0]));
        `,
        [newName],
      ),
    { description: `${newName} extension picker to unmount` },
  );
}

async function runProfile(_app, base, token, profileId, url) {
  const launched = await request(`${base}/v1/profiles/${profileId}/run`, {
    method: "POST",
    token,
    body: { url, headless: true },
  });
  assert.equal(launched.response.status, 200, JSON.stringify(launched.value));
  const cdp = await CdpClient.connect(launched.value.remote_debugging_port);
  return { launched: launched.value, cdp };
}

async function stopProfile(app, base, token, profileId, cdp) {
  cdp.close();
  const stopped = await request(`${base}/v1/profiles/${profileId}/kill`, {
    method: "POST",
    token,
  });
  assert.equal(stopped.response.status, 204);
  await app.waitFor(
    async () => {
      const profile = (await app.invoke("list_browser_profiles")).find(
        (item) => item.id === profileId,
      );
      return !profile?.process_id;
    },
    { timeoutMs: 20_000, description: "network profile process cleanup" },
  );
}

async function assertProxyWorkerLogsRedacted(app, settings) {
  const files = (await readdir(path.join(app.root, "tmp"))).filter(
    (file) => file.startsWith("donut-proxy-") && file.endsWith(".log"),
  );
  assert.ok(files.length > 0, "No proxy worker diagnostic logs were created");
  const contents = (
    await Promise.all(
      files.map((file) => readFile(path.join(app.root, "tmp", file), "utf8")),
    )
  ).join("\n");
  for (const item of settings) {
    if (!item.username) continue;
    const rawAuth = `${item.username}:${item.password ?? ""}@`;
    const encodedAuth = `${encodeURIComponent(item.username)}:${encodeURIComponent(item.password ?? "")}@`;
    assert.equal(
      contents.includes(rawAuth) || contents.includes(encodedAuth),
      false,
      "Proxy worker logs exposed upstream credentials",
    );
  }
}

function wireGuardTargetWasReached() {
  const container = process.env.DONUT_E2E_WIREGUARD_CONTAINER;
  assert.ok(container, "WireGuard fixture container name is required");
  return (
    spawnSync(
      "docker",
      [
        "exec",
        container,
        "grep",
        "-q",
        "GET /donut-e2e-wireguard ",
        "/tmp/donut-e2e-target-requests",
      ],
      { stdio: "ignore", timeout: 2_000 },
    ).status === 0
  );
}

test("visible UI creates and assigns profiles, groups, proxies, VPNs, extensions, and extension groups", async () => {
  const httpSettings = proxySettings(
    process.env.RESIDENTIAL_PROXY_URL_ONE_HTTP,
    "HTTP",
  );
  const socksSettings = proxySettings(
    process.env.RESIDENTIAL_PROXY_URL_ONE_SOCKS,
    "SOCKS",
  );
  const realWireGuardConfig = process.env.DONUT_E2E_WIREGUARD_CONFIG_BASE64
    ? Buffer.from(
        process.env.DONUT_E2E_WIREGUARD_CONFIG_BASE64,
        "base64",
      ).toString("utf8")
    : null;
  const app = appFromEnvironment("network-visible-ui", {
    wayfernTermsAccepted: false,
  });
  let apiPort;
  let activeCdp;
  let activeVpnId;
  try {
    const prepared = await prepareWayfern(
      app,
      process.env.DONUT_E2E_PROJECT_ROOT,
    );
    if (!app.session) await app.start();
    if (!(await app.invoke("check_wayfern_terms_accepted"))) {
      await app.invoke("accept_wayfern_terms");
      await app.restart();
    }
    assert.equal(
      await app.visibleTextIncludes("Welcome to Donut Browser"),
      false,
      "completed test sessions must not leave the Welcome dialog over the UI",
    );
    const group = await createGroupThroughUi(app);
    assert.ok(group);
    await app.capture("01-profile-group-created");

    const httpProxy = await createProxyThroughUi(app, httpSettings);
    assert.ok(httpProxy);
    assert.equal(
      httpProxy.proxy_settings.proxy_type === httpSettings.proxy_type &&
        httpProxy.proxy_settings.host === httpSettings.host &&
        httpProxy.proxy_settings.port === httpSettings.port &&
        httpProxy.proxy_settings.username === httpSettings.username &&
        httpProxy.proxy_settings.password === httpSettings.password,
      true,
      "The HTTP proxy created through the UI did not preserve its settings",
    );
    const vpn = await createVpnThroughUi(
      app,
      realWireGuardConfig ?? wireGuardFixture(),
    );
    assert.ok(vpn);
    activeVpnId = vpn.id;
    await app.capture("02-proxy-and-vpn-created");

    const extensionEntities = await createExtensionsThroughUi(app);
    assert.ok(extensionEntities.extension);
    assert.ok(extensionEntities.group);
    await app.capture("03-extension-and-group-created");

    const profile = await createProfileThroughUi(app, group.name);
    assert.ok(profile);
    assert.equal(profile.version, prepared.version);
    assert.equal(profile.group_id, group.id);
    await assignExtensionGroupThroughUi(
      app,
      profile.name,
      "Default",
      extensionEntities.group.name,
    );
    await app.waitFor(
      async () =>
        (await app.invoke("list_browser_profiles")).find(
          (item) => item.id === profile.id,
        )?.extension_group_id === extensionEntities.group.id,
      { description: "extension group assignment persisted" },
    );
    await app.capture("04-profile-created");

    const socksProxy = await app.invoke("create_stored_proxy", {
      name: "Residential SOCKS5",
      proxySettings: socksSettings,
    });
    const [httpCheck, socksCheck] = await Promise.all([
      app.invoke("check_proxy_validity", {
        proxyId: httpProxy.id,
        proxySettings: null,
      }),
      app.invoke("check_proxy_validity", {
        proxyId: socksProxy.id,
        proxySettings: null,
      }),
    ]);
    assert.equal(httpCheck.is_valid, true);
    assert.equal(socksCheck.is_valid, true);
    assert.ok(isIP(httpCheck.ip));
    assert.ok(isIP(socksCheck.ip));

    await assignNetworkThroughUi(
      app,
      profile.name,
      "Not selected",
      httpProxy.name,
    );
    await app.waitFor(
      async () =>
        (await app.invoke("list_browser_profiles")).find(
          (item) => item.id === profile.id,
        )?.proxy_id === httpProxy.id,
      { description: "HTTP proxy assignment persisted" },
    );

    const settings = await app.invoke("get_app_settings");
    const saved = await app.invoke("save_app_settings", {
      settings: {
        ...settings,
        api_enabled: true,
        api_port: 0,
        api_token: null,
      },
    });
    apiPort = await app.invoke("start_api_server", { port: 0 });
    const base = `http://127.0.0.1:${apiPort}`;

    const proxied = await runProfile(
      app,
      base,
      saved.api_token,
      profile.id,
      "https://api.ipify.org/",
    );
    activeCdp = proxied.cdp;
    const browserExitIp = await activeCdp.waitFor(
      `(() => {
        const value = document.body?.innerText?.trim() ?? "";
        return /^[0-9a-f:.]+$/i.test(value) ? value : false;
      })()`,
      { timeoutMs: 30_000, description: "Wayfern residential proxy exit IP" },
    );
    assert.ok(isIP(browserExitIp));
    await stopProfile(app, base, saved.api_token, profile.id, activeCdp);
    activeCdp = null;
    await assertProxyWorkerLogsRedacted(app, [httpSettings, socksSettings]);

    await assignNetworkThroughUi(app, profile.name, httpProxy.name, vpn.name);
    await app.waitFor(
      async () =>
        (await app.invoke("list_browser_profiles")).find(
          (item) => item.id === profile.id,
        )?.vpn_id === vpn.id,
      { description: "WireGuard assignment persisted" },
    );
    await app.capture("05-proxy-and-vpn-assigned");

    if (realWireGuardConfig) {
      const tunneled = await runProfile(
        app,
        base,
        saved.api_token,
        profile.id,
        process.env.DONUT_E2E_WIREGUARD_TARGET_URL,
      );
      activeCdp = tunneled.cdp;
      await app.waitFor(wireGuardTargetWasReached, {
        timeoutMs: 30_000,
        description: "Wayfern GET through local WireGuard peer",
      });
      await stopProfile(app, base, saved.api_token, profile.id, activeCdp);
      activeCdp = null;
    }
  } catch (error) {
    await app.capture("failure");
    throw error;
  } finally {
    activeCdp?.close();
    if (app.session) {
      if (apiPort) await app.invoke("stop_api_server").catch(() => {});
      for (const profile of await app
        .invoke("list_browser_profiles")
        .catch(() => [])) {
        if (profile.process_id) {
          await app.invoke("kill_browser_profile", { profile }).catch(() => {});
        }
      }
      if (activeVpnId) {
        await app
          .invoke("disconnect_vpn", { vpnId: activeVpnId })
          .catch(() => {});
      }
    }
    await app.close();
  }
});

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readdir, readFile, stat, writeFile } from "node:fs/promises";
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

function processIsRunning(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

async function createXrayProfile(app, version) {
  return app.invoke("create_browser_profile_new", {
    name: "Xray Reality Profile",
    browserStr: "wayfern",
    version,
    releaseType: "stable",
    proxyId: null,
    vpnId: null,
    wayfernConfig: {
      fingerprint: null,
      randomize_fingerprint_on_launch: false,
      geoip: false,
    },
    groupId: null,
    ephemeral: false,
    dnsBlocklist: null,
    launchHook: null,
  });
}

// Matches a whole Xray-core access log line whose destination is exactly
// api.ipify.org:443 ("... accepted tcp:api.ipify.org:443 [outbound]"). The
// leading delimiter keeps a lookalike host such as notapi.ipify.org:443 from
// passing the assertion.
const XRAY_ACCESS_LOG_TARGET = /^.*[\s:]api\.ipify\.org:443(?:\s.*)?$/;

test("VLESS Reality persists, imports, routes through Xray-core, records traffic, and cleans up", async () => {
  const vlessUri = process.env.DONUT_E2E_VLESS_URI;
  const accessLog = process.env.DONUT_E2E_XRAY_ACCESS_LOG;
  assert.ok(vlessUri, "The network harness must provide a VLESS Reality URI");
  assert.ok(accessLog, "The network harness must provide an Xray access log");

  const app = appFromEnvironment("network-xray-reality", {
    seedVersionCache: false,
    wayfernTermsAccepted: false,
  });
  let apiPort;
  let activeCdp;
  let profile;
  let worker;
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

    const invalidUri = vlessUri.replace("security=reality", "security=tls");
    const invalidCreate = await app.invokeError("create_stored_proxy", {
      name: "Invalid VLESS",
      proxySettings: {
        proxy_type: "vless",
        host: "ignored.invalid",
        port: 1,
        username: "must-be-cleared",
        password: "must-be-cleared",
        vless_uri: invalidUri,
      },
    });
    assert.match(invalidCreate, /VLESS_CONFIG_INVALID/);

    const created = await app.invoke("create_stored_proxy", {
      name: "Local VLESS Reality",
      proxySettings: {
        proxy_type: "VLESS",
        host: "ignored.invalid",
        port: 1,
        username: "must-be-cleared",
        password: "must-be-cleared",
        vless_uri: vlessUri,
      },
    });
    assert.equal(created.proxy_settings.proxy_type, "vless");
    assert.equal(created.proxy_settings.host, "127.0.0.1");
    assert.equal(created.proxy_settings.port, Number(new URL(vlessUri).port));
    assert.equal(created.proxy_settings.username, null);
    assert.equal(created.proxy_settings.password, null);
    assert.equal(created.proxy_settings.vless_uri, vlessUri);

    const updated = await app.invoke("update_stored_proxy", {
      proxyId: created.id,
      name: "Updated VLESS Reality",
      proxySettings: {
        proxy_type: "vless",
        host: "still-ignored.invalid",
        port: 2,
        username: "still-cleared",
        password: "still-cleared",
        vless_uri: vlessUri,
      },
    });
    assert.equal(updated.name, "Updated VLESS Reality");
    assert.equal(updated.proxy_settings.host, "127.0.0.1");
    assert.equal(updated.proxy_settings.username, null);
    assert.equal(updated.proxy_settings.password, null);

    const exportedJson = await app.invoke("export_proxies", {
      format: "json",
    });
    const exported = JSON.parse(exportedJson);
    assert.equal(exported.proxies.length, 1);
    assert.deepEqual(exported.proxies[0], {
      name: "Updated VLESS Reality",
      type: "vless",
      host: "127.0.0.1",
      port: Number(new URL(vlessUri).port),
      vless_uri: vlessUri,
    });
    assert.equal(
      await app.invoke("export_proxies", { format: "txt" }),
      vlessUri,
    );

    const parsed = await app.invoke("parse_txt_proxies", {
      content: `${vlessUri}\n${invalidUri}\n`,
    });
    assert.equal(parsed.length, 2);
    assert.equal(parsed[0].status, "parsed");
    assert.equal(parsed[0].proxy_type, "vless");
    assert.equal(parsed[0].vless_uri, vlessUri);
    assert.equal(parsed[1].status, "invalid");

    await app.restart();
    const persisted = (await app.invoke("get_stored_proxies")).find(
      (candidate) => candidate.id === created.id,
    );
    assert.ok(persisted, "VLESS proxy did not survive an app restart");
    assert.equal(persisted.proxy_settings.vless_uri, vlessUri);

    await app.invoke("delete_stored_proxy", { proxyId: created.id });
    const imported = await app.invoke("import_proxies_json", {
      content: exportedJson,
    });
    assert.equal(imported.imported_count, 1);
    assert.equal(imported.skipped_count, 0);
    assert.deepEqual(imported.errors, []);
    assert.equal(imported.proxies[0].proxy_settings.vless_uri, vlessUri);
    const proxy = imported.proxies[0];

    const invalidImport = await app.invoke("import_proxies_json", {
      content: JSON.stringify({
        version: "1.0",
        source: "DonutBrowser",
        exported_at: new Date().toISOString(),
        proxies: [
          {
            name: "Rejected VLESS",
            type: "vless",
            host: "127.0.0.1",
            port: 443,
            vless_uri: invalidUri,
          },
        ],
      }),
    });
    assert.equal(invalidImport.imported_count, 0);
    assert.equal(invalidImport.errors.length, 1);
    assert.match(invalidImport.errors[0], /VLESS_CONFIG_INVALID/);

    profile = await createXrayProfile(app, prepared.version);
    await app.invoke("update_profile_proxy", {
      profileId: profile.id,
      proxyId: proxy.id,
    });
    const assigned = (await app.invoke("list_browser_profiles")).find(
      (candidate) => candidate.id === profile.id,
    );
    assert.equal(assigned.proxy_id, proxy.id);

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
    const launched = await runProfile(
      app,
      base,
      saved.api_token,
      profile.id,
      "https://api.ipify.org/",
    );
    activeCdp = launched.cdp;
    const exitIp = await activeCdp.waitFor(
      `(() => {
        const value = document.body?.innerText?.trim() ?? "";
        return /^[0-9a-f:.]+$/i.test(value) ? value : false;
      })()`,
      { timeoutMs: 30_000, description: "VLESS Reality exit IP" },
    );
    assert.ok(isIP(exitIp), `Unexpected exit IP response: ${exitIp}`);

    const workerDirectory = path.join(app.dataRoot, "cache", "proxy_workers");
    await app.waitFor(
      async () => {
        const workerFiles = await readdir(workerDirectory).catch(() => []);
        const workerFile = workerFiles.find(
          (file) => file.startsWith("xray_worker_") && file.endsWith(".json"),
        );
        if (!workerFile) return false;
        worker = JSON.parse(
          await readFile(path.join(workerDirectory, workerFile), "utf8"),
        );
        return worker.profile_id === profile.id;
      },
      { description: "profile-scoped Xray worker configuration" },
    );
    assert.ok(processIsRunning(worker.pid), "Xray supervisor is not running");
    assert.ok(processIsRunning(worker.xray_pid), "Xray-core is not running");
    assert.equal(worker.vless_uri, vlessUri);

    const workerPath = path.join(
      workerDirectory,
      `xray_worker_${worker.id}.json`,
    );
    const runtimePath = path.join(
      workerDirectory,
      `xray_runtime_${worker.id}.json`,
    );
    if (process.platform !== "win32") {
      assert.equal((await stat(workerPath)).mode & 0o777, 0o600);
      assert.equal((await stat(runtimePath)).mode & 0o777, 0o600);
    }
    const runtime = JSON.parse(await readFile(runtimePath, "utf8"));
    assert.equal(runtime.inbounds[0].protocol, "socks");
    assert.equal(runtime.inbounds[0].listen, "127.0.0.1");
    assert.equal(runtime.outbounds[0].protocol, "vless");
    assert.equal(runtime.outbounds[0].streamSettings.security, "reality");
    assert.equal(
      runtime.outbounds[0].settings.vnext[0].users[0].flow,
      "xtls-rprx-vision",
    );

    await app.waitFor(
      async () =>
        (await readFile(accessLog, "utf8").catch(() => ""))
          .split("\n")
          .some((line) => XRAY_ACCESS_LOG_TARGET.test(line)),
      {
        timeoutMs: 15_000,
        description: "request in Xray-core server access log",
      },
    );
    const liveTraffic = await app.waitFor(
      async () => {
        const snapshot = await app.invoke("get_profile_traffic_snapshot", {
          profileId: profile.id,
        });
        return snapshot?.total_bytes_sent > 0 &&
          snapshot?.total_bytes_received > 0
          ? snapshot
          : false;
      },
      {
        timeoutMs: 15_000,
        description: "local traffic snapshot for VLESS profile",
      },
    );
    assert.ok(liveTraffic.total_requests > 0);

    const supervisorPid = worker.pid;
    const xrayPid = worker.xray_pid;
    await stopProfile(app, base, saved.api_token, profile.id, activeCdp);
    activeCdp = null;
    await app.waitFor(
      async () => {
        const files = await readdir(workerDirectory).catch(() => []);
        return (
          !files.includes(`xray_worker_${worker.id}.json`) &&
          !processIsRunning(supervisorPid) &&
          !processIsRunning(xrayPid)
        );
      },
      {
        timeoutMs: 15_000,
        description: "Xray worker process and configuration cleanup",
      },
    );
    await assert.rejects(readFile(runtimePath), { code: "ENOENT" });

    const persistedTraffic = await app.invoke("get_profile_traffic_snapshot", {
      profileId: profile.id,
    });
    assert.ok(
      persistedTraffic.total_bytes_sent >= liveTraffic.total_bytes_sent,
    );
    assert.ok(
      persistedTraffic.total_bytes_received >= liveTraffic.total_bytes_received,
    );
  } catch (error) {
    await app.capture("failure");
    throw error;
  } finally {
    activeCdp?.close();
    if (app.session) {
      if (apiPort) await app.invoke("stop_api_server").catch(() => {});
      if (profile) {
        const latest = (
          await app.invoke("list_browser_profiles").catch(() => [])
        ).find((candidate) => candidate.id === profile.id);
        if (latest?.process_id) {
          await app
            .invoke("kill_browser_profile", { profile: latest })
            .catch(() => {});
        }
      }
    }
    await app.close();
  }
});

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
    seedVersionCache: false,
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

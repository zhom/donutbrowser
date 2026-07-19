import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { appFromEnvironment } from "../lib/app.mjs";
import { CdpClient } from "../lib/cdp.mjs";
import {
  defaultWayfernPath,
  inspectWayfern,
  seedWayfern,
} from "../lib/fixtures.mjs";

const fixtureUrl = process.env.DONUT_E2E_FIXTURE_URL;

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

async function prepareWayfern(app) {
  const localBundle = defaultWayfernPath(process.env.DONUT_E2E_PROJECT_ROOT);
  if (existsSync(localBundle)) {
    const wayfern = inspectWayfern(localBundle);
    await seedWayfern(app.dataRoot, wayfern);
    return { version: wayfern.version, source: "local fixture" };
  }

  await app.start();
  const current = await app.invoke("fetch_browser_versions_with_count", {
    browserStr: "wayfern",
  });
  assert.ok(
    current.versions.length > 0,
    "No Wayfern build is published for this platform",
  );
  const version = current.versions[0];
  await app.invoke("download_browser", {
    browserStr: "wayfern",
    version,
  });
  return { version, source: "published download" };
}

function processExists(pid) {
  if (!pid) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function waitForProcessExit(app, pid) {
  await app.waitFor(() => !processExists(pid), {
    timeoutMs: 20_000,
    description: `Wayfern process ${pid} to exit`,
  });
}

function assertIdleResourceBounds(pid) {
  if (process.platform === "win32") return;
  const output = execFileSync("ps", ["-o", "rss=,%cpu=", "-p", String(pid)], {
    encoding: "utf8",
  }).trim();
  const [rssText, cpuText] = output.split(/\s+/);
  const rssKiB = Number(rssText);
  const cpuPercent = Number(cpuText);
  assert.ok(
    rssKiB > 0 && rssKiB < 2_000_000,
    `Wayfern main process RSS is ${rssKiB} KiB`,
  );
  assert.ok(
    cpuPercent >= 0 && cpuPercent < 200,
    `Wayfern main process CPU is ${cpuPercent}%`,
  );
}

function realWayfernTermsPath() {
  if (process.platform === "darwin") {
    return path.join(
      os.homedir(),
      "Library",
      "Application Support",
      "Wayfern",
      "license-accepted",
    );
  }
  if (process.platform === "win32") {
    return path.join(
      process.env.APPDATA ?? path.join(os.homedir(), "AppData", "Roaming"),
      "Wayfern",
      "license-accepted",
    );
  }
  return path.join(
    process.env.XDG_CONFIG_HOME ?? path.join(os.homedir(), ".config"),
    "Wayfern",
    "license-accepted",
  );
}

async function snapshotFile(file) {
  try {
    const [contents, metadata] = await Promise.all([
      readFile(file),
      stat(file, { bigint: true }),
    ]);
    return {
      exists: true,
      contents: contents.toString("base64"),
      size: metadata.size.toString(),
      mtime: metadata.mtimeNs.toString(),
    };
  } catch (error) {
    if (error.code === "ENOENT") return { exists: false };
    throw error;
  }
}

async function createRealProfile(app, version, name, fingerprint = null) {
  return app.invoke("create_browser_profile_new", {
    name,
    browserStr: "wayfern",
    version,
    releaseType: "stable",
    proxyId: null,
    vpnId: null,
    wayfernConfig: {
      fingerprint,
      randomize_fingerprint_on_launch: false,
      geoip: false,
    },
    groupId: null,
    ephemeral: false,
    dnsBlocklist: null,
    launchHook: null,
  });
}

test("real Wayfern fingerprinting, terms, API automation, CDP, cookies, and process cleanup", async () => {
  assert.ok(process.env.WAYFERN_TEST_TOKEN, "WAYFERN_TEST_TOKEN is required");
  const realTermsFile = realWayfernTermsPath();
  const realTermsBefore = await snapshotFile(realTermsFile);
  const hasLocalWayfern = existsSync(
    defaultWayfernPath(process.env.DONUT_E2E_PROJECT_ROOT),
  );
  const app = appFromEnvironment("browser-wayfern", {
    seedVersionCache: hasLocalWayfern,
  });
  let cdp;
  let browserPid;
  try {
    const prepared = await prepareWayfern(app);
    if (!app.session) await app.start();

    assert.equal(await app.invoke("check_wayfern_downloaded"), true);
    assert.equal(await app.invoke("check_wayfern_terms_accepted"), false);
    await app.invoke("accept_wayfern_terms");
    assert.equal(await app.invoke("check_wayfern_terms_accepted"), true);
    assert.ok(
      (
        await app.invoke("get_downloaded_browser_versions", {
          browserStr: "wayfern",
        })
      ).includes(prepared.version),
    );
    assert.equal(
      await app.invoke("check_browser_exists", {
        browserStr: "wayfern",
        version: prepared.version,
      }),
      true,
    );
    assert.deepEqual(await app.invoke("check_missing_binaries"), []);
    assert.deepEqual(await app.invoke("ensure_all_binaries_exist"), []);
    assert.deepEqual(await app.invoke("ensure_active_browsers_downloaded"), []);
    assert.deepEqual(await app.invoke("get_supported_browsers"), ["wayfern"]);
    assert.equal(
      await app.invoke("is_browser_supported_on_platform", {
        browserStr: "wayfern",
      }),
      true,
    );
    assert.ok(
      (
        await app.invoke("fetch_browser_versions_cached_first", {
          browserStr: "wayfern",
        })
      ).some((item) => item.version === prepared.version),
    );
    assert.ok(
      (
        await app.invoke("fetch_browser_versions_with_count_cached_first", {
          browserStr: "wayfern",
        })
      ).versions.includes(prepared.version),
    );
    assert.equal(
      (await app.invoke("get_browser_release_types", { browserStr: "wayfern" }))
        .stable,
      prepared.version,
    );
    assert.match(
      await app.invokeError("cancel_download", {
        browserStr: "wayfern",
        version: prepared.version,
      }),
      /No active download/,
    );

    const sample = await app.invoke("generate_sample_fingerprint", {
      browser: "wayfern",
      version: prepared.version,
      configJson: JSON.stringify({ geoip: false }),
    });
    const fingerprint = JSON.parse(sample);
    assert.ok(
      Object.keys(fingerprint).length >= 10,
      "Wayfern returned an incomplete fingerprint",
    );

    const profile = await createRealProfile(
      app,
      prepared.version,
      `Real Wayfern (${prepared.source})`,
    );
    assert.ok(profile.wayfern_config.fingerprint);
    assert.ok(
      Object.keys(JSON.parse(profile.wayfern_config.fingerprint)).length >= 10,
    );
    assert.equal(await app.invoke("check_missing_geoip_database"), true);
    assert.equal(await app.invoke("is_geoip_database_available"), false);
    await app.invoke("download_geoip_database");
    assert.equal(await app.invoke("is_geoip_database_available"), true);
    assert.equal(await app.invoke("check_missing_geoip_database"), false);
    await app.invoke("update_wayfern_config", {
      profileId: profile.id,
      config: profile.wayfern_config,
    });
    await app.invoke("match_profile_fingerprint_to_exit", {
      profileId: profile.id,
      exitIp: "8.8.8.8",
    });
    const consistency = await app.invoke(
      "check_profile_fingerprint_consistency",
      {
        profileId: profile.id,
      },
    );
    assert.equal(typeof consistency, "object");

    const directProfile = (await app.invoke("list_browser_profiles")).find(
      (item) => item.id === profile.id,
    );
    const directLaunch = await app.invoke("launch_browser_profile", {
      profile: directProfile,
      url: `${fixtureUrl}/direct-command`,
    });
    assert.ok(directLaunch.process_id);
    await app.invoke("open_url_with_profile", {
      profileId: profile.id,
      url: `${fixtureUrl}/direct-open`,
    });
    await app.invoke("kill_browser_profile", { profile: directLaunch });
    await waitForProcessExit(app, directLaunch.process_id);

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
    const port = await app.invoke("start_api_server", { port: 0 });
    const base = `http://127.0.0.1:${port}`;
    const launched = await request(`${base}/v1/profiles/${profile.id}/run`, {
      method: "POST",
      token: saved.api_token,
      body: { url: `${fixtureUrl}/wayfern`, headless: true },
    });
    assert.equal(launched.response.status, 200, JSON.stringify(launched.value));
    assert.equal(launched.value.headless, true);

    cdp = await CdpClient.connect(launched.value.remote_debugging_port);
    await cdp.waitFor(`document.title === "Donut E2E Browser Fixture"`, {
      description: "fixture page title",
    });
    assert.equal(
      await cdp.evaluate("document.querySelector('#path').textContent"),
      "/wayfern",
    );
    assert.equal(
      await cdp.evaluate(
        "document.querySelector('#fixture-button').click(); document.querySelector('#fixture-button').dataset.clicked",
      ),
      "yes",
    );
    const echo = await cdp.evaluate(
      `fetch(${JSON.stringify(`${fixtureUrl}/api/echo`)}, {
        method: "POST",
        body: "wayfern-cdp-body"
      }).then((response) => response.json())`,
    );
    assert.equal(echo.method, "POST");
    assert.equal(echo.body, "wayfern-cdp-body");
    assert.ok(echo.userAgent.length > 20);
    assert.match(await cdp.evaluate("document.cookie"), /donut_e2e=browser-ok/);

    const runningProfile = (await app.invoke("list_browser_profiles")).find(
      (item) => item.id === profile.id,
    );
    browserPid = runningProfile.process_id;
    assert.equal(
      await app.invoke("check_browser_status", { profile: runningProfile }),
      true,
    );
    assertIdleResourceBounds(browserPid);
    if (process.platform !== "win32") {
      const command = execFileSync(
        "ps",
        ["-ww", "-o", "command=", "-p", String(browserPid)],
        {
          encoding: "utf8",
        },
      );
      assert.match(
        command,
        new RegExp(app.dataRoot.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")),
      );
    }

    const opened = await request(`${base}/v1/profiles/${profile.id}/open-url`, {
      method: "POST",
      token: saved.api_token,
      body: { url: `${fixtureUrl}/opened-via-api` },
    });
    assert.equal(opened.response.status, 200);
    await app.waitFor(
      async () => {
        const targets = await fetch(
          `http://127.0.0.1:${launched.value.remote_debugging_port}/json`,
        ).then((response) => response.json());
        return targets.some((target) => target.url.includes("/opened-via-api"));
      },
      { timeoutMs: 20_000, description: "API-opened Wayfern target" },
    );

    const killed = await request(`${base}/v1/profiles/${profile.id}/kill`, {
      method: "POST",
      token: saved.api_token,
    });
    assert.equal(killed.response.status, 204);
    cdp.close();
    cdp = null;
    await waitForProcessExit(app, browserPid);
    const stoppedProfile = (await app.invoke("list_browser_profiles")).find(
      (item) => item.id === profile.id,
    );
    assert.equal(
      await app.invoke("check_browser_status", { profile: stoppedProfile }),
      false,
    );

    const batchProfile = await createRealProfile(
      app,
      prepared.version,
      "Wayfern Batch Automation",
      sample,
    );
    const batchRun = await request(`${base}/v1/profiles/batch/run`, {
      method: "POST",
      token: saved.api_token,
      body: {
        profile_ids: [batchProfile.id],
        url: `${fixtureUrl}/batch`,
        headless: true,
      },
    });
    assert.equal(batchRun.response.status, 200);
    assert.equal(
      batchRun.value.results[0].ok,
      true,
      batchRun.value.results[0].error,
    );
    const batchCdp = await CdpClient.connect(
      batchRun.value.results[0].remote_debugging_port,
    );
    assert.equal(
      await batchCdp.waitFor("window.__fixtureReady === true"),
      true,
    );
    batchCdp.close();
    const batchStop = await request(`${base}/v1/profiles/batch/stop`, {
      method: "POST",
      token: saved.api_token,
      body: { profile_ids: [batchProfile.id] },
    });
    assert.equal(batchStop.response.status, 200);
    assert.equal(
      batchStop.value.results[0].ok,
      true,
      batchStop.value.results[0].error,
    );

    await app.invoke("stop_api_server");
    await app.invoke("delete_profile", { profileId: profile.id });
    await app.invoke("delete_profile", { profileId: batchProfile.id });
  } catch (error) {
    await app.capture("failure");
    throw error;
  } finally {
    cdp?.close();
    if (app.session && browserPid && processExists(browserPid)) {
      const profile = (
        await app.invoke("list_browser_profiles").catch(() => [])
      ).find((item) => item.process_id === browserPid);
      if (profile)
        await app.invoke("kill_browser_profile", { profile }).catch(() => {});
    }
    await app.close();
    assert.deepEqual(
      await snapshotFile(realTermsFile),
      realTermsBefore,
      "the browser suite modified the real Wayfern terms marker",
    );
  }
});

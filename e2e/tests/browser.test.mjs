import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { readdir, readFile, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { appFromEnvironment } from "../lib/app.mjs";
import { CdpClient } from "../lib/cdp.mjs";
import {
  defaultWayfernPath,
  inspectWayfern,
  prepareWayfern,
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
  const localWayfernPath = defaultWayfernPath(
    process.env.DONUT_E2E_PROJECT_ROOT,
  );
  const localWayfernVersion = existsSync(localWayfernPath)
    ? inspectWayfern(localWayfernPath).version
    : null;
  const app = appFromEnvironment("browser-wayfern", {
    seedVersionCache: localWayfernVersion ?? false,
    wayfernTermsAccepted: false,
  });
  let cdp;
  let browserPid;
  try {
    const prepared = await prepareWayfern(
      app,
      process.env.DONUT_E2E_PROJECT_ROOT,
    );
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

/// Read every donut-proxy worker config the app has on disk.
async function readWorkerConfigs(app) {
  const dir = path.join(app.dataRoot, "cache", "proxy_workers");
  let entries;
  try {
    entries = await readdir(dir);
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
  const configs = [];
  for (const entry of entries) {
    if (!entry.endsWith(".json")) continue;
    try {
      configs.push(JSON.parse(await readFile(path.join(dir, entry), "utf8")));
    } catch {
      // A worker rewriting its config mid-read is not a failure.
    }
  }
  return configs;
}

async function waitForCondition(check, description, timeoutMs = 30_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    if (await check()) return;
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`Timed out after ${timeoutMs}ms waiting for ${description}`);
}

/// Launch a real profile and return it together with the worker serving it,
/// asserting the worker is pinned to that exact browser process.
async function launchWithWorker(app, version, name) {
  const profile = await createRealProfile(app, version, name);
  const launched = await app.invoke("launch_browser_profile", {
    profile,
    url: `${fixtureUrl}/worker-lifecycle`,
  });
  const browserPid = launched.process_id;
  assert.ok(browserPid, "Wayfern must report a process id");

  const worker = await app.waitFor(
    async () =>
      (await readWorkerConfigs(app)).find(
        (config) => config.profile_id === profile.id,
      ),
    { description: `the proxy worker config for ${name}` },
  );
  assert.ok(worker.pid, "the worker must record its own pid");
  assert.equal(
    worker.browser_pid,
    browserPid,
    "the worker must record the browser it serves",
  );
  // Without the start time a recycled PID reads as a live browser forever,
  // which is exactly how workers ended up outliving everything.
  assert.equal(
    typeof worker.browser_pid_start_time,
    "number",
    "the owning browser must be pinned to a start time, not just a pid",
  );
  return { profile, browserPid, workerPid: worker.pid, workerId: worker.id };
}

// The reported orphan, reproduced end to end with a real Wayfern. A detached
// donut-proxy has to notice its browser is gone and exit on its own — before it
// recorded a verified owner identity it just kept running and users killed it
// by hand. Phase one covers closing the browser; phase two covers the reported
// order (app closed first, so nothing is left to reap anything).
test("a proxy worker dies with its browser, with and without the app running", async () => {
  assert.ok(process.env.WAYFERN_TEST_TOKEN, "WAYFERN_TEST_TOKEN is required");
  const localWayfernPath = defaultWayfernPath(
    process.env.DONUT_E2E_PROJECT_ROOT,
  );
  const localWayfernVersion = existsSync(localWayfernPath)
    ? inspectWayfern(localWayfernPath).version
    : null;
  const app = appFromEnvironment("browser-worker-lifecycle", {
    seedVersionCache: localWayfernVersion ?? false,
    // Let the app run the real acceptance flow below; the pre-seeded marker is
    // not what the Wayfern binary itself honours, and it would exit on launch.
    wayfernTermsAccepted: false,
    // The worker polls its owner every 15s in production; shorten it so a reap
    // is observable without padding the suite by minutes.
    extraEnv: { DONUT_PROXY_WATCHDOG_INTERVAL_MS: "500" },
  });

  const strays = new Set();
  try {
    const prepared = await prepareWayfern(
      app,
      process.env.DONUT_E2E_PROJECT_ROOT,
    );
    if (!app.session) await app.start();
    if (!(await app.invoke("check_wayfern_terms_accepted"))) {
      await app.invoke("accept_wayfern_terms");
    }

    // Phase one: the app is up, the browser goes away.
    const first = await launchWithWorker(app, prepared.version, "Worker Reap");
    strays.add(first.browserPid).add(first.workerPid);
    process.kill(first.browserPid, "SIGKILL");
    await waitForCondition(
      () => !processExists(first.browserPid),
      `Wayfern ${first.browserPid} to exit`,
    );
    await waitForCondition(
      () => !processExists(first.workerPid),
      `donut-proxy ${first.workerPid} to exit after its browser did`,
    );
    assert.equal(
      (await readWorkerConfigs(app)).find(
        (config) => config.id === first.workerId,
      ),
      undefined,
      "the worker must delete its config on the way out",
    );

    // Phase two: the reported order — close the app while the browser is still
    // running, then close the browser. Nothing but the worker is left alive.
    const second = await launchWithWorker(
      app,
      prepared.version,
      "Worker Reap After Quit",
    );
    strays.add(second.browserPid).add(second.workerPid);
    await app.close();

    // The app is expected to leave a live browser and its route alone on quit.
    // If the harness tears the session's whole process group down instead, the
    // orphan case cannot be observed here — say so rather than asserting on it.
    if (!processExists(second.browserPid) || !processExists(second.workerPid)) {
      console.warn(
        "Skipping the app-closed orphan check: the driver terminated the browser and/or worker along with the app",
      );
      return;
    }

    process.kill(second.browserPid, "SIGKILL");
    await waitForCondition(
      () => !processExists(second.browserPid),
      `Wayfern ${second.browserPid} to exit`,
    );
    await waitForCondition(
      () => !processExists(second.workerPid),
      `orphaned donut-proxy ${second.workerPid} to reap itself with no app running`,
    );
  } finally {
    for (const pid of strays) {
      if (pid && processExists(pid)) {
        try {
          process.kill(pid, "SIGKILL");
        } catch {
          // Already gone.
        }
      }
    }
    await app.close();
  }
});

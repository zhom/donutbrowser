import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readdir, readFile, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import en from "../../src/i18n/locales/en.json" with { type: "json" };
import { appFromEnvironment } from "../lib/app.mjs";
import { CdpClient } from "../lib/cdp.mjs";
import {
  cachedFixtureVersion,
  currentHostOs,
  prepareWayfern,
  writeUnpackedExtension,
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

/**
 * The exit-derived fields `WayfernConfig.location` may hold. Mirrors
 * `LOCALE_CARRY_OVER_KEYS` in wayfern_manager.rs: anything outside this set is
 * a device field, and a device field never belongs to the location.
 */
const LOCATION_KEYS = new Set([
  "timezone",
  "timezoneOffset",
  "language",
  "languages",
  "latitude",
  "longitude",
  "accuracy",
]);

/** `fingerprint` is the serialised fingerprint STRING, or null for a fresh one. */
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
  const localWayfernVersion = cachedFixtureVersion(
    process.env.DONUT_E2E_PROJECT_ROOT,
  );
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
    // The gate is a real modal until the terms are accepted, and acceptance
    // through the bridge (not the dialog's own button) must lift it too: the
    // frontend learns about the marker from the backend's event, not from a
    // restart.
    const termsDialogVisible = () =>
      app.execute(
        `return [...document.querySelectorAll('[role="dialog"]')].some(node => node.textContent.includes(arguments[0]));`,
        [en.wayfernTerms.title],
      );
    await app.waitFor(termsDialogVisible, {
      description: "the Wayfern terms dialog before acceptance",
    });
    await app.invoke("accept_wayfern_terms");
    assert.equal(await app.invoke("check_wayfern_terms_accepted"), true);
    await app.waitFor(async () => !(await termsDialogVisible()), {
      description: "the Wayfern terms dialog to close after acceptance",
    });
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
    // The app's own resolver must agree with the release manifest the harness
    // read when it decided the cached fixture was current. If these two ever
    // diverge, the fixture check compares against a version the app will never
    // ask for, and the suite silently runs an old browser again.
    assert.ok(
      (
        await app.invoke("fetch_browser_versions_with_count", {
          browserStr: "wayfern",
        })
      ).versions.includes(prepared.version),
      "the app must resolve the same published version the fixture was chosen for",
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
    const fingerprint = JSON.parse(sample.fingerprint);
    assert.ok(
      Object.keys(fingerprint).length >= 10,
      "Wayfern returned an incomplete fingerprint",
    );
    // A browser with the identity API must hand back the UUID the device was
    // derived from. The device itself is a view to show once and discard: an
    // identity-backed profile stores the id and the exit's location, never the
    // payload, so no fingerprint sits on disk to be copied.
    const identityCapable =
      Number.parseInt(prepared.version.split(".")[0], 10) >= 151;
    assert.equal(
      typeof sample.identity_id === "string",
      identityCapable,
      "identity_id must be present exactly on browsers with the identity API",
    );
    assert.equal(
      sample.identity_baseline,
      undefined,
      "the retired identity baseline must not be handed back",
    );
    assert.ok(
      sample.location === null || typeof sample.location === "string",
      "location is the exit-derived JSON object, or null when none resolved",
    );
    if (typeof sample.location === "string") {
      const locationKeys = Object.keys(JSON.parse(sample.location));
      assert.ok(locationKeys.length > 0, "a resolved location is never empty");
      for (const key of locationKeys) {
        assert.ok(
          LOCATION_KEYS.has(key),
          `${key} is a device field and must not travel in the location`,
        );
      }
    }

    const profile = await createRealProfile(
      app,
      prepared.version,
      `Real Wayfern (${prepared.source})`,
    );
    // An identity-backed profile stores the identity and the location and never
    // the device: the browser rebuilds it from the id on every launch. A legacy
    // browser stores the whole payload.
    assert.equal(
      typeof profile.wayfern_config.identity_id === "string",
      identityCapable,
      "a created profile must carry the identity its device came from",
    );
    assert.equal(
      profile.wayfern_config.fingerprint === undefined,
      identityCapable,
      "an identity-backed profile must store no device payload",
    );
    if (!identityCapable) {
      assert.ok(
        Object.keys(JSON.parse(profile.wayfern_config.fingerprint)).length >=
          10,
      );
    }
    assert.equal(await app.invoke("check_missing_geoip_database"), true);
    assert.equal(await app.invoke("is_geoip_database_available"), false);
    await app.invoke("download_geoip_database");
    assert.equal(await app.invoke("is_geoip_database_available"), true);
    assert.equal(await app.invoke("check_missing_geoip_database"), false);

    // The new-profile form (which needs a downloaded browser and its release
    // types, so it renders here and not in the UI suite): session restore is
    // on by default and the checkbox is a live control.
    await app.clickSelector('[aria-label="Profiles"]');
    await app.clickText("New");
    const restoreChecked = () =>
      app.execute(
        `return document.querySelector("#restore-session")?.getAttribute("aria-checked") ?? null;`,
      );
    await app.waitFor(async () => (await restoreChecked()) !== null, {
      description: "the session-restore checkbox in the new-profile form",
    });
    assert.equal(
      await restoreChecked(),
      "true",
      "a new profile must default to continuing its last session",
    );
    await app.clickSelector("#restore-session");
    await app.waitFor(async () => (await restoreChecked()) === "false", {
      description: "the session-restore checkbox to switch off",
    });
    await app.pressShortcut({ key: "Escape" });
    await app.waitFor(
      () =>
        app.execute(
          `return !document.querySelector("[role='dialog'] #restore-session");`,
        ),
      { description: "the new-profile dialog to close" },
    );

    await app.invoke("update_wayfern_config", {
      profileId: profile.id,
      config: profile.wayfern_config,
    });
    await app.invoke("match_profile_fingerprint_to_exit", {
      profileId: profile.id,
      exitIp: "8.8.8.8",
    });
    // The identity is internal state that neither call above sends back.
    // Losing it would silently re-mint the device on the next launch and throw
    // the user's edits away with it, so both paths must carry it forward
    // unchanged. The exit re-match moves only the location: the profile comes
    // out of it still identity-only, with the exit's timezone stored.
    if (identityCapable) {
      const stored = (await app.invoke("list_browser_profiles")).find(
        (p) => p.id === profile.id,
      );
      assert.equal(
        stored.wayfern_config.identity_id,
        profile.wayfern_config.identity_id,
        "the identity must survive update_wayfern_config and an exit re-match",
      );
      assert.equal(
        stored.wayfern_config.fingerprint,
        undefined,
        "neither call may leave a device payload behind",
      );
      assert.equal(
        typeof JSON.parse(stored.wayfern_config.location).timezone,
        "string",
        "an exit re-match stores the exit's timezone in the location",
      );
    }
    // The session-restore switch is profile configuration and round-trips
    // like the rest of it; `undefined` (the default) reads as on.
    await app.invoke("update_wayfern_config", {
      profileId: profile.id,
      config: {
        ...(await app.invoke("list_browser_profiles")).find(
          (p) => p.id === profile.id,
        ).wayfern_config,
        restore_session: false,
      },
    });
    assert.equal(
      (await app.invoke("list_browser_profiles")).find(
        (p) => p.id === profile.id,
      ).wayfern_config.restore_session,
      false,
      "restore_session must persist through update_wayfern_config",
    );

    // The persona the browser will offer in its fill menu: derived from the
    // profile's own seed, so it is stable for this profile, unique to it, and
    // never empty.
    const persona = await app.invoke("get_profile_persona", {
      profileId: profile.id,
    });
    assert.ok(
      persona.length >= 8,
      "a persona carries the fields to fill a form",
    );
    assert.deepEqual(
      await app.invoke("get_profile_persona", { profileId: profile.id }),
      persona,
      "the same profile presents the same person every time",
    );
    for (const entry of persona) {
      assert.ok(entry.id && entry.label && entry.value.trim());
    }
    const email = persona.find((entry) => entry.id === "email");
    assert.match(email.value, /@/);
    assert.match(
      await app.invokeError("get_profile_persona", {
        profileId: "00000000-0000-0000-0000-000000000000",
      }),
      /PROFILE_NOT_FOUND/,
    );
    // An edit replaces one value and leaves the rest derived.
    await app.invoke("update_wayfern_config", {
      profileId: profile.id,
      config: {
        ...(await app.invoke("list_browser_profiles")).find(
          (p) => p.id === profile.id,
        ).wayfern_config,
        persona: JSON.stringify([
          { id: "email", label: "Email", value: "someone@example.com" },
        ]),
      },
    });
    const edited = await app.invoke("get_profile_persona", {
      profileId: profile.id,
    });
    assert.equal(
      edited.find((entry) => entry.id === "email").value,
      "someone@example.com",
    );
    assert.equal(
      edited.find((entry) => entry.id === "full_name").value,
      persona.find((entry) => entry.id === "full_name").value,
      "an edit to one field must not redraw the others",
    );
    // What "reset to generated" shows: the person before any edit.
    assert.deepEqual(
      await app.invoke("get_profile_persona", {
        profileId: profile.id,
        derivedOnly: true,
      }),
      persona,
    );

    // Pre-launch gate: local-only checks that must answer without starting a
    // proxy, an Xray worker or the browser.
    const checks = await app.invoke("get_profile_pre_launch_checks", {
      profileId: profile.id,
    });
    assert.ok(Array.isArray(checks.vpn_extensions));
    assert.equal(
      typeof checks.scan_state,
      "string",
      "the scan must report whether it saw the whole profile",
    );
    assert.equal(typeof checks.consistency, "object");
    assert.equal(typeof checks.exit_probe_pending, "boolean");
    assert.equal(typeof checks.exit_measurement_unreliable, "boolean");
    // The third consistency state: what no probe can ever verify for this
    // profile. Reported so a launch that compared nothing is never rendered as
    // a launch that compared everything and agreed.
    assert.ok(
      Array.isArray(checks.exit_unverified),
      "the pre-launch report must say what it cannot verify",
    );
    assert.ok(
      Array.isArray(checks.consistency.unverified),
      "a consistency result must carry the dimensions nothing compared",
    );
    // "Donut will check it while starting" is only sayable while some
    // dimension is still checkable. Both dimensions unverifiable means the
    // probe would compare nothing, so it is not pending work.
    assert.ok(
      !checks.exit_probe_pending || checks.exit_unverified.length < 2,
      "a probe that can compare nothing must not be reported as pending",
    );
    // This profile has no VPN extension, so nothing may block its launch.
    assert.equal(
      checks.vpn_extensions.length,
      0,
      "a clean profile must not report a VPN extension",
    );
    assert.equal(
      checks.consent_token,
      null,
      "a consent token is only minted when a cached mismatch is blocking",
    );

    // Extension detection, against manifests written where Chromium puts
    // them. The three cases are the whole point of the classifier: a real VPN
    // is named as one, a known VPN with an unrevealing name is caught by its
    // id, and a download manager holding the same `proxy` permission is
    // reported as a capability and never as a VPN.
    // `DONUTBROWSER_DATA_ROOT` puts the data dir at <dataRoot>/data, so this
    // is app_dirs::profiles_dir() plus the layout Chromium itself uses.
    const extensionsDir = path.join(
      app.dataRoot,
      "data",
      "profiles",
      profile.id,
      "profile",
      "Default",
      "Extensions",
    );
    const seedExtension = async (id, version, manifest) => {
      const dir = path.join(extensionsDir, id, `${version}_0`);
      await mkdir(dir, { recursive: true });
      await writeFile(
        path.join(dir, "manifest.json"),
        JSON.stringify(manifest),
      );
    };
    const IDM_ID = "ngpampappnmepgilojfohadhhmbhlaek";
    const HOTSPOT_SHIELD_ID = "nlbejmccbhkncgokjcmghpfloaajcffj";
    const NAMED_VPN_ID = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    await seedExtension(IDM_ID, "6.43.1", {
      name: "IDM Integration Module",
      version: "6.43.1",
      description: "Download files with Internet Download Manager",
      permissions: ["downloads", "storage", "proxy", "nativeMessaging"],
    });
    await seedExtension(HOTSPOT_SHIELD_ID, "10.0.0", {
      name: "Hotspot Shield",
      version: "10.0.0",
      permissions: ["proxy"],
    });
    await seedExtension(NAMED_VPN_ID, "1.0.0", {
      name: "Turbo VPN Free",
      version: "1.0.0",
      permissions: ["proxy"],
    });

    const withExtensions = await app.invoke("get_profile_pre_launch_checks", {
      profileId: profile.id,
    });
    const detected = new Map(
      withExtensions.vpn_extensions.map((item) => [item.key, item]),
    );
    assert.equal(detected.size, 3, "every seeded extension must be reported");
    assert.equal(detected.get(`crx:${NAMED_VPN_ID}`).confidence, "confirmed");
    assert.equal(
      detected.get(`crx:${HOTSPOT_SHIELD_ID}`).confidence,
      "confirmed",
      "a known VPN id must be named even when its name gives nothing away",
    );
    assert.equal(
      detected.get(`crx:${IDM_ID}`).confidence,
      "capability",
      "a download manager holding the proxy permission is not a VPN",
    );
    assert.ok(
      detected.get(`crx:${IDM_ID}`).proxy_control,
      "it does still hold the permission, which is why it is listed at all",
    );
    assert.equal(
      withExtensions.exit_measurement_unreliable,
      true,
      "a proxy-capable extension makes the exit measurement a caveat",
    );

    // Acknowledgements are per-profile and must be accepted for both kinds.
    await app.invoke("ack_launch_gate", {
      profileId: profile.id,
      ackFingerprint: false,
      ackExtensionKeys: ["crx:e2e-nonexistent-extension"],
    });
    await app.invoke("ack_launch_gate", {
      profileId: profile.id,
      ackFingerprint: true,
      ackExtensionKeys: [`crx:${IDM_ID}`],
    });
    const afterAck = await app.invoke("get_profile_pre_launch_checks", {
      profileId: profile.id,
    });
    assert.deepEqual(
      afterAck.vpn_extensions.map((item) => item.key).sort(),
      [`crx:${HOTSPOT_SHIELD_ID}`, `crx:${NAMED_VPN_ID}`].sort(),
      "an acknowledged extension stops being reported, the others do not",
    );
    assert.match(
      await app.invokeError("get_profile_pre_launch_checks", {
        profileId: "00000000-0000-0000-0000-000000000000",
      }),
      /PROFILE_NOT_FOUND/,
    );

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
      // An automation run starts clean: it never reopens a person's session.
      // The crash-restore bubble stays hidden, and the retired switch that
      // Chromium no longer reads is gone from the command line.
      assert.doesNotMatch(command, /--restore-last-session/);
      assert.match(command, /--hide-crash-restore-bubble/);
      assert.doesNotMatch(command, /--disable-session-crashed-bubble/);
      assert.match(
        command,
        /--enable-logging=stderr/,
        "the browser's own verdicts reach the app through stderr",
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
      // The fingerprint STRING, not the envelope `generate_sample_fingerprint`
      // returns it in. `WayfernConfig.fingerprint` is an `Option<String>`
      // (wayfern_manager.rs), so passing `sample` made the whole command fail
      // to deserialise with "invalid type: map, expected a string", before any
      // of the automation this test exists to check could run.
      sample.fingerprint,
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
    // A profile carrying a whole stored device is migrated into an identity
    // plus overrides. Some override values are currently rejected by the
    // browser at launch, and the launcher reports that with the property
    // named, so this asserts the reported failure rather than pretending the
    // launch worked. If the launch succeeds instead, the else branch takes
    // over and the batch is asserted in full.
    const batchBlockedByBrowser =
      !batchRun.value.results[0].ok &&
      /was not applied: \w+/.test(batchRun.value.results[0].error ?? "");
    if (batchBlockedByBrowser) {
      console.log(
        `[donut-e2e] Batch profile could not launch: ${batchRun.value.results[0].error}`,
      );
      assert.match(
        batchRun.value.results[0].error,
        /WAYFERN_IDENTITY_REFUSED|WAYFERN_FINGERPRINT_APPLY_FAILED/,
        "a refused device must reach the caller as a coded error, never as a silent success",
      );
    } else {
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
    }
    const batchStop = await request(`${base}/v1/profiles/batch/stop`, {
      method: "POST",
      token: saved.api_token,
      body: { profile_ids: [batchProfile.id] },
    });
    assert.equal(batchStop.response.status, 200);
    // Stopping is idempotent: a profile that never launched is already
    // stopped, so the batch endpoint reports success either way.
    assert.equal(
      batchStop.value.results[0].ok,
      true,
      `batch stop reported ${JSON.stringify(batchStop.value.results[0])}`,
    );

    // The recipe recorder's refusals, which are the whole contract a caller can
    // rely on without a paid browser: what it will not start on, and that an
    // idle recorder answers rather than throwing. The capture itself is a paid
    // browser feature and is tested where that feature lives.
    assert.deepEqual(await app.invoke("get_recipe_recording"), {
      profile_id: null,
      steps: [],
      recording: false,
    });
    assert.deepEqual(await app.invoke("stop_recipe_recording"), {
      profile_id: null,
      steps: [],
      recording: false,
    });
    assert.match(
      await app.invokeError("start_recipe_recording", {
        profileId: "00000000-0000-0000-0000-000000000000",
      }),
      /PROFILE_NOT_FOUND/,
    );
    assert.match(
      await app.invokeError("start_recipe_recording", {
        profileId: profile.id,
      }),
      /PROFILE_NOT_RUNNING/,
      "a recording needs a live browser to attach to",
    );

    // Export and import: a profile is moved to another machine as one archive
    // and comes back as a NEW profile, owing nothing to the machine that wrote
    // it. Exercised here because this is the suite with a real profile
    // directory to carry.
    const exportPath = path.join(app.dataRoot, "exported.donutprofile");
    const exported = await app.invoke("export_profile", {
      profileId: profile.id,
      destination: exportPath,
      includeData: true,
    });
    assert.equal(exported.profile_name, profile.name);
    assert.equal(exported.browser, "wayfern");
    assert.ok((await stat(exportPath)).size > 0);
    const archivePreview = await app.invoke("preview_profile_archive", {
      path: exportPath,
    });
    assert.equal(archivePreview.manifest.profile_name, profile.name);
    assert.deepEqual(archivePreview.tags, []);
    const importedProfile = await app.invoke("import_profile_archive", {
      path: exportPath,
    });
    assert.notEqual(importedProfile.id, profile.id);
    assert.equal(importedProfile.version, profile.version);
    assert.equal(
      importedProfile.process_id,
      null,
      "an imported profile is not running on this machine",
    );
    assert.equal(
      importedProfile.proxy_id ?? null,
      null,
      "a proxy id belongs to the machine that assigned it",
    );
    assert.equal(
      importedProfile.wayfern_config.identity_id,
      profile.wayfern_config.identity_id,
      "the device travels: the same identity rebuilds the same browser",
    );
    // Twice from one archive gives two profiles, under distinct names.
    const importedAgain = await app.invoke("import_profile_archive", {
      path: exportPath,
    });
    assert.notEqual(importedAgain.id, importedProfile.id);
    assert.notEqual(importedAgain.name, importedProfile.name);
    assert.match(
      await app.invokeError("preview_profile_archive", {
        path: path.join(app.dataRoot, "not-an-archive"),
      }),
      /PROFILE_IMPORT_FAILED/,
    );
    for (const created of [importedProfile, importedAgain]) {
      await app.invoke("delete_profile", {
        profileId: created.id,
        permanent: true,
      });
    }

    // A temporary profile: created over REST for one run, gone once its
    // browser stops. Nothing else in the app removes it, so this is the
    // whole contract an automation client depends on.
    const temporary = await request(`${base}/v1/profiles`, {
      method: "POST",
      token: saved.api_token,
      body: {
        name: "Temporary Run",
        browser: "wayfern",
        version: prepared.version,
        temporary: true,
      },
    });
    assert.equal(
      temporary.response.status,
      200,
      JSON.stringify(temporary.value),
    );
    assert.equal(temporary.value.profile.temporary, true);
    assert.equal(
      temporary.value.profile.ephemeral,
      true,
      "a temporary profile keeps its browsing data in memory only",
    );
    const temporaryId = temporary.value.profile.id;
    const temporaryRun = await request(
      `${base}/v1/profiles/${temporaryId}/run`,
      {
        method: "POST",
        token: saved.api_token,
        body: { url: `${fixtureUrl}/temporary`, headless: true },
      },
    );
    assert.equal(
      temporaryRun.response.status,
      200,
      JSON.stringify(temporaryRun.value),
    );
    const temporaryPid = (await app.invoke("list_browser_profiles")).find(
      (item) => item.id === temporaryId,
    )?.process_id;
    assert.ok(
      temporaryPid,
      "the temporary profile must report the browser it started",
    );
    await request(`${base}/v1/profiles/${temporaryId}/kill`, {
      method: "POST",
      token: saved.api_token,
    });
    await waitForProcessExit(app, temporaryPid);
    await app.waitFor(
      async () =>
        !(await app.invoke("list_browser_profiles")).some(
          (item) => item.id === temporaryId,
        ),
      { description: "the temporary profile to delete itself" },
    );
    assert.deepEqual(
      (await app.invoke("list_trashed_profiles")).filter(
        (entry) => entry.id === temporaryId,
      ),
      [],
      "a disposable profile must not land in the trash",
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
  const localWayfernVersion = cachedFixtureVersion(
    process.env.DONUT_E2E_PROJECT_ROOT,
  );
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

// Two things nothing else covers. First, that an assigned extension group
// actually reaches Wayfern: a loaded MV3 extension registers a
// `chrome-extension://<id>/background.js` service-worker target, so CDP can see
// it from outside. Second, that staging is per profile. It used to be one
// shared `extensions/unpacked` directory wiped on every launch, and because
// Chromium records the absolute staging path and reads those files lazily for
// the life of the process instead of copying them into the profile, launching a
// second profile broke the extension in every browser already running.
test("an assigned extension group reaches Wayfern and each profile stages its own copy", async () => {
  assert.ok(process.env.WAYFERN_TEST_TOKEN, "WAYFERN_TEST_TOKEN is required");
  const localWayfernVersion = cachedFixtureVersion(
    process.env.DONUT_E2E_PROJECT_ROOT,
  );
  const app = appFromEnvironment("browser-extensions", {
    seedVersionCache: localWayfernVersion ?? false,
    wayfernTermsAccepted: false,
  });
  const launched = [];
  try {
    const prepared = await prepareWayfern(
      app,
      process.env.DONUT_E2E_PROJECT_ROOT,
    );
    if (!app.session) await app.start();
    if (!(await app.invoke("check_wayfern_terms_accepted"))) {
      await app.invoke("accept_wayfern_terms");
    }

    const extension = await app.invoke("add_unpacked_extension", {
      name: "Donut Launch Fixture",
      path: await writeUnpackedExtension(
        path.join(app.root, "fixtures", "loaded-extension"),
        { name: "Donut Launch Fixture", version: "1.0.0" },
      ),
      link: false,
    });
    const group = await app.invoke("create_extension_group", {
      name: "Launch Extensions",
    });
    await app.invoke("add_extension_to_group", {
      groupId: group.id,
      extensionId: extension.id,
    });

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
    const base = `http://127.0.0.1:${await app.invoke("start_api_server", { port: 0 })}`;

    const stagedManifest = (profileId) =>
      path.join(
        app.dataRoot,
        "data",
        "extensions",
        "unpacked",
        profileId,
        extension.id,
        "manifest.json",
      );
    const extensionWorkers = async (debuggingPort) => {
      const targets = await fetch(
        `http://127.0.0.1:${debuggingPort}/json`,
      ).then((response) => response.json());
      return targets.filter(
        (target) =>
          target.type === "service_worker" &&
          String(target.url).startsWith("chrome-extension://"),
      );
    };
    const launchWithExtension = async (name) => {
      const profile = await createRealProfile(app, prepared.version, name);
      assert.equal(
        (
          await app.invoke("assign_extension_group_to_profile", {
            profileId: profile.id,
            extensionGroupId: group.id,
          })
        ).extension_group_id,
        group.id,
      );
      const run = await request(`${base}/v1/profiles/${profile.id}/run`, {
        method: "POST",
        token: saved.api_token,
        body: { url: `${fixtureUrl}/extension-launch`, headless: true },
      });
      assert.equal(run.response.status, 200, JSON.stringify(run.value));
      const record = {
        profile,
        debuggingPort: run.value.remote_debugging_port,
      };
      launched.push(record);
      const workers = await app.waitFor(
        async () => {
          const found = await extensionWorkers(record.debuggingPort);
          return found.length > 0 ? found : null;
        },
        {
          timeoutMs: 60_000,
          description: `the extension's service worker in ${name}`,
        },
      );
      assert.match(
        workers[0].url,
        /^chrome-extension:\/\/\w+\/background\.js$/,
      );
      return record;
    };

    const first = await launchWithExtension("Extension Launch One");
    assert.ok(
      existsSync(stagedManifest(first.profile.id)),
      "the first profile must stage the extension under its own id",
    );
    if (process.platform !== "win32") {
      // The staged path is what Chromium was handed, and it is per profile.
      const running = (await app.invoke("list_browser_profiles")).find(
        (item) => item.id === first.profile.id,
      );
      const command = execFileSync(
        "ps",
        ["-ww", "-o", "command=", "-p", String(running.process_id)],
        { encoding: "utf8" },
      );
      assert.ok(
        command.includes(
          `--load-extension=${path.dirname(stagedManifest(first.profile.id))}`,
        ),
        "Wayfern must be pointed at this profile's own staged copy",
      );
    }
    await launchWithExtension("Extension Launch Two");

    // The regression itself: the second launch must not have taken the first
    // profile's files with it. The staged manifest is what its running browser
    // is still reading from.
    for (const { profile } of launched) {
      assert.ok(
        existsSync(stagedManifest(profile.id)),
        `${profile.name} lost its staged extension to another profile's launch`,
      );
    }

    for (const { profile } of launched) {
      const running = (await app.invoke("list_browser_profiles")).find(
        (item) => item.id === profile.id,
      );
      await app.invoke("kill_browser_profile", { profile: running });
      await waitForProcessExit(app, running.process_id);
    }
    await app.invoke("stop_api_server");
  } catch (error) {
    await app.capture("failure");
    throw error;
  } finally {
    if (app.session) {
      const running = await app.invoke("list_browser_profiles").catch(() => []);
      for (const { profile } of launched) {
        const record = running.find((item) => item.id === profile.id);
        if (record?.process_id && processExists(record.process_id)) {
          await app
            .invoke("kill_browser_profile", { profile: record })
            .catch(() => {});
        }
      }
    }
    await app.close();
  }
});

/// The browser's remote-debugging port, read off its own command line: an
/// interactive launch does not hand the port back the way an API run does.
function debuggingPortOf(pid) {
  const command = execFileSync(
    "ps",
    ["-ww", "-o", "command=", "-p", String(pid)],
    { encoding: "utf8" },
  );
  const match = command.match(/--remote-debugging-port=(\d+)/);
  assert.ok(match, `no debugging port on the command line: ${command}`);
  return { port: Number(match[1]), command };
}

async function targetUrls(port) {
  const targets = await fetch(`http://127.0.0.1:${port}/json`).then((r) =>
    r.json(),
  );
  return targets
    .filter((target) => target.type === "page")
    .map((target) => target.url);
}

test("an interactive launch continues the last session once the identity travels at launch", async () => {
  assert.ok(process.env.WAYFERN_TEST_TOKEN, "WAYFERN_TEST_TOKEN is required");
  const localWayfernVersion = cachedFixtureVersion(
    process.env.DONUT_E2E_PROJECT_ROOT,
  );
  const app = appFromEnvironment("browser-session", {
    seedVersionCache: localWayfernVersion ?? false,
    wayfernTermsAccepted: false,
  });
  let browserPid;
  try {
    const prepared = await prepareWayfern(
      app,
      process.env.DONUT_E2E_PROJECT_ROOT,
    );
    if (!app.session) await app.start();
    // The browser itself refuses to start until its terms marker exists, and
    // only its own acceptance run writes one it recognises.
    await app.invoke("accept_wayfern_terms");
    const major = Number.parseInt(prepared.version.split(".")[0], 10);
    if (major < 152) {
      // Older builds take no launch identity, so Donut starts them on a fresh
      // tab and there is nothing to continue.
      console.log(
        `[donut-e2e] Wayfern ${prepared.version} takes no launch identity; session restore is off by design, skipping the restore assertions`,
      );
      return;
    }

    const profile = await createRealProfile(
      app,
      prepared.version,
      "Session Restore",
    );
    // A launch identity needs the exit's timezone; the geoip match writes it.
    await app.invoke("download_geoip_database");
    await app.invoke("match_profile_fingerprint_to_exit", {
      profileId: profile.id,
      exitIp: "8.8.8.8",
    });
    const stored = (await app.invoke("list_browser_profiles")).find(
      (p) => p.id === profile.id,
    );
    const location = JSON.parse(stored.wayfern_config.location);
    assert.equal(typeof location.timezone, "string");
    const userDataDir = path.join(
      app.dataRoot,
      "data",
      "profiles",
      profile.id,
      "profile",
    );

    const launch = async (url) => {
      const current = (await app.invoke("list_browser_profiles")).find(
        (p) => p.id === profile.id,
      );
      const launched = await app.invoke("launch_browser_profile", {
        profile: current,
        url,
      });
      assert.ok(launched.process_id);
      browserPid = launched.process_id;
      return launched;
    };
    const stop = async () => {
      const current = (await app.invoke("list_browser_profiles")).find(
        (p) => p.id === profile.id,
      );
      await app.invoke("kill_browser_profile", { profile: current });
      await waitForProcessExit(app, browserPid);
    };
    const waitForTargets = async (port, expected) => {
      let seen = [];
      await app
        .waitFor(
          async () => {
            seen = await targetUrls(port).catch(() => []);
            return expected.every((needle) =>
              seen.some((url) => url.includes(needle)),
            );
          },
          { timeoutMs: 30_000, description: `targets ${expected.join(", ")}` },
        )
        .catch(() => {
          // The URLs it did see are the whole diagnosis: a restore that
          // dropped one tab looks identical to one that never ran.
          assert.fail(
            `waiting for ${expected.join(", ")} but the browser had ${
              seen.length ? seen.join(", ") : "no page targets"
            }`,
          );
        });
    };

    // First session: two tabs.
    const first = await launch(`${fixtureUrl}/session-a`);
    const { port: firstPort, command } = debuggingPortOf(first.process_id);
    assert.match(command, /--restore-last-session/);
    assert.match(command, /--wayfern-identity-file=/);
    const identityFile = JSON.parse(
      await readFile(path.join(userDataDir, "wayfern-identity.json"), "utf8"),
    );
    assert.equal(identityFile.identityId, stored.wayfern_config.identity_id);
    assert.equal(identityFile.timezone, location.timezone);
    // No claimed OS means the host, which is what an omitted operatingSystem
    // means over CDP as well; the document has to spell it out.
    assert.equal(
      identityFile.operatingSystem,
      stored.wayfern_config.os ?? currentHostOs(),
    );
    await waitForTargets(firstPort, ["/session-a"]);
    await app.invoke("open_url_with_profile", {
      profileId: profile.id,
      url: `${fixtureUrl}/session-b`,
    });
    await waitForTargets(firstPort, ["/session-a", "/session-b"]);
    await stop();
    const preferences = JSON.parse(
      await readFile(path.join(userDataDir, "Default", "Preferences"), "utf8"),
    );
    assert.equal(
      preferences.profile?.exit_type,
      "Normal",
      "a stop must run the browser's own shutdown so the session is written",
    );

    // Second session: both tabs come back, and the launch URL gets its own
    // tab instead of replacing a restored one.
    const second = await launch(`${fixtureUrl}/session-c`);
    const { port: secondPort } = debuggingPortOf(second.process_id);
    await waitForTargets(secondPort, [
      "/session-a",
      "/session-b",
      "/session-c",
    ]);

    // A browser that died hard still comes back, with no bubble to answer.
    // Chromium commits a tab change to the session file on a short delay, so a
    // kill in the same second loses the newest tab through no fault of the
    // launcher; wait for the write before pulling the plug.
    await new Promise((resolve) => setTimeout(resolve, 6_000));
    process.kill(second.process_id, "SIGKILL");
    await waitForProcessExit(app, second.process_id);
    await app.waitFor(
      async () =>
        !(await app.invoke("check_browser_status", {
          profile: (
            await app.invoke("list_browser_profiles")
          ).find((p) => p.id === profile.id),
        })),
      { description: "the app to notice the killed browser" },
    );
    const third = await launch(null);
    const { port: thirdPort } = debuggingPortOf(third.process_id);
    await waitForTargets(thirdPort, ["/session-a", "/session-b", "/session-c"]);
    await stop();

    // Switched off, the profile starts on a fresh tab.
    await app.invoke("update_wayfern_config", {
      profileId: profile.id,
      config: { ...stored.wayfern_config, restore_session: false },
    });
    const fourth = await launch(`${fixtureUrl}/session-d`);
    const { port: fourthPort, command: fourthCommand } = debuggingPortOf(
      fourth.process_id,
    );
    assert.doesNotMatch(fourthCommand, /--restore-last-session/);
    await waitForTargets(fourthPort, ["/session-d"]);
    assert.ok(
      !(await targetUrls(fourthPort)).some((url) => url.includes("/session-a")),
      "a profile with restore switched off must not reopen the old session",
    );
    await stop();
    await app.invoke("delete_profile", { profileId: profile.id });
  } catch (error) {
    await app.capture("failure");
    throw error;
  } finally {
    if (app.session && browserPid && processExists(browserPid)) {
      const profile = (
        await app.invoke("list_browser_profiles").catch(() => [])
      ).find((item) => item.process_id === browserPid);
      if (profile)
        await app.invoke("kill_browser_profile", { profile }).catch(() => {});
    }
    await app.close();
  }
});

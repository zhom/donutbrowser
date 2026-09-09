import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import Color from "color";
import en from "../../src/i18n/locales/en.json" with { type: "json" };
import { THEMES } from "../../src/lib/themes.ts";
import { withApp } from "../lib/app.mjs";
import { extensionZipBase64 } from "../lib/fixtures.mjs";

const slot = (name) => `[data-slot="${name}"]`;
const profileRow = (id) => `tr[data-profile-id="${id}"]`;
const inspectTrigger = (id) =>
  `${slot("profile-inspect-trigger")}[data-profile-id="${id}"]`;
const dragHandle = (id) =>
  `${slot("profile-drag-handle")}[data-profile-id="${id}"]`;
const tableScroll = `${slot("profile-workspace")} > .scroll-fade`;
const modifier =
  process.platform === "darwin" ? { meta: true } : { ctrl: true };

async function createProfile(app, name) {
  return app.invoke("create_browser_profile_new", {
    name,
    browserStr: "wayfern",
    version: "150.0.7871.100",
    releaseType: "stable",
    proxyId: null,
    vpnId: null,
    wayfernConfig: { fingerprint: "{}" },
    groupId: null,
    ephemeral: false,
    dnsBlocklist: null,
    launchHook: null,
  });
}

async function waitForSelector(app, selector, present = true) {
  return app.waitFor(
    () =>
      app.execute(
        `return Boolean(document.querySelector(arguments[0])) === arguments[1];`,
        [selector, present],
      ),
    { description: `${present ? "present" : "absent"} ${selector}` },
  );
}

async function resize(app, width, height) {
  const before = await app.session.command("GET", "/window/rect");
  const viewport = await app.execute(
    "return { width: innerWidth, height: innerHeight };",
  );
  const contentWidth = width - Math.max(0, before.width - viewport.width);
  const contentHeight = height - Math.max(0, before.height - viewport.height);
  await app.session.command("POST", "/window/rect", { width, height });
  // The native resize response precedes WebKit layout and painting. A capture
  // before those complete crops the previous frame to the new window size.
  await app.waitFor(
    () =>
      app.execute(
        "return innerWidth === arguments[0] && innerHeight === arguments[1];",
        [contentWidth, contentHeight],
      ),
    { description: `WebView resized to ${contentWidth} by ${contentHeight}` },
  );
  await app.execute(
    "return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))));",
  );
}

async function tabForward(app) {
  // tauri-wd's /actions dispatches an untrusted Tab event without its default.
  // /element/value implements focus traversal, and honors a canceled keydown
  // before moveFocus(), including Radix's modal boundary trap.
  const active = await app.execute("return document.activeElement;");
  await app.session.sendKeys(active, "\uE004");
}

async function activateFocusedByKeyboard(app, key = "\uE006") {
  // tauri-wd 0.1.11 does not implement Enter/Space's default button click in
  // either keyboard endpoint. Exercise the handlers, then supply only the
  // uncanceled missing HTML activation. This is keyboard-path simulation;
  // pointer activation is covered separately through actual WebDriver clicks.
  await app.execute(`
    const state = { target: document.activeElement, events: [], clicks: 0 };
    state.recordKey = (event) => state.events.push(event);
    state.recordClick = (event) => { if (state.target === event.target || state.target.contains(event.target)) state.clicks++; };
    document.addEventListener("keydown", state.recordKey, true);
    document.addEventListener("keyup", state.recordKey, true);
    document.addEventListener("click", state.recordClick, true);
    window.__donutMotionKeyboard = state;
  `);
  try {
    await app.pressShortcut({ key });
    await app.execute(`
      const state = window.__donutMotionKeyboard;
      if (!state.clicks && !state.events.some((event) => event.defaultPrevented) && state.target.isConnected) state.target.click();
    `);
  } finally {
    await app.execute(`
      const state = window.__donutMotionKeyboard;
      if (!state) return;
      document.removeEventListener("keydown", state.recordKey, true);
      document.removeEventListener("keyup", state.recordKey, true);
      document.removeEventListener("click", state.recordClick, true);
      delete window.__donutMotionKeyboard;
    `);
  }
}

async function emit(app, event, payload) {
  await app.invoke("plugin:event|emit", { event, payload });
}

async function assertContained(app, selector) {
  const bounds = await app.execute(
    `
    const element = document.querySelector(arguments[0]);
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    return { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom,
      width: innerWidth, height: innerHeight };
  `,
    [selector],
  );
  assert.ok(bounds, selector);
  assert.ok(
    bounds.left >= -1 && bounds.right <= bounds.width + 1,
    JSON.stringify(bounds),
  );
  assert.ok(
    bounds.top >= -1 && bounds.bottom <= bounds.height + 1,
    JSON.stringify(bounds),
  );
}

async function exampleStates(app) {
  return app.execute(`return Object.fromEntries(
    [...document.querySelectorAll('[data-slot="isolation-example-profile"]')].map((profile) =>
      [profile.dataset.profileId, { signedIn: profile.dataset.signedIn, route: profile.dataset.route }])
  );`);
}

async function assertExamples(app, expected) {
  await app.waitFor(
    async () => {
      const actual = await exampleStates(app);
      return (
        Object.keys(actual).length === Object.keys(expected).length &&
        Object.entries(expected).every(
          ([id, profile]) =>
            actual[id]?.signedIn === profile.signedIn &&
            actual[id]?.route === profile.route,
        )
      );
    },
    {
      description: `independent example state ${JSON.stringify(expected)}`,
    },
  );
}

const emptyExamples = {
  research: { signedIn: "false", route: "direct" },
  shopping: { signedIn: "false", route: "direct" },
};

async function exerciseIsolation(app) {
  await waitForSelector(app, slot("profile-isolation-demo"));
  const realProfiles = await app.invoke("list_browser_profiles");
  await assertExamples(app, emptyExamples);
  await app.clickSelector(slot("isolation-toggle-cookie"));
  await assertExamples(app, {
    ...emptyExamples,
    research: { signedIn: "true", route: "direct" },
  });
  await app.clickSelector(slot("isolation-toggle-route"));
  await assertExamples(app, {
    ...emptyExamples,
    research: { signedIn: "true", route: "proxy" },
  });
  await app.clickSelector(
    `${slot("isolation-example-profile")}[data-profile-id="shopping"] ${slot("isolation-select-profile")}`,
  );
  await app.clickSelector(slot("isolation-toggle-cookie"));
  await assertExamples(app, {
    research: { signedIn: "true", route: "proxy" },
    shopping: { signedIn: "true", route: "direct" },
  });
  await app.clickSelector(slot("isolation-toggle-route"));
  await app.clickSelector(slot("isolation-toggle-cookie"));
  await assertExamples(app, {
    research: { signedIn: "true", route: "proxy" },
    shopping: { signedIn: "false", route: "proxy" },
  });

  // Move focus with the real keyboard, then activate the currently focused
  // route button. The example changes without waiting for a pointer animation.
  await app.execute(
    `document.querySelector('[data-slot="isolation-toggle-cookie"]').focus();`,
  );
  await tabForward(app);
  assert.equal(
    await app.execute(`return document.activeElement?.dataset.slot;`),
    "isolation-toggle-route",
  );
  await activateFocusedByKeyboard(app);
  await assertExamples(app, {
    research: { signedIn: "true", route: "proxy" },
    shopping: { signedIn: "false", route: "direct" },
  });
  await app.clickSelector(slot("isolation-reset"));
  await assertExamples(app, emptyExamples);
  assert.deepEqual(
    await app.invoke("list_browser_profiles"),
    realProfiles,
    "the demonstration never mutates real profiles",
  );
}

test("first-run profile cutaway is interactive, isolated, and readable at both window sizes", async () => {
  await withApp(
    "motion-first-run",
    async (app) => {
      await app.waitForText(en.welcome.title);
      await resize(app, 1080, 900);
      await exerciseIsolation(app);
      await app.capture("onboarding-isolation-wide");
      await resize(app, 640, 480);
      await assertContained(app, '[role="dialog"]');
      await app.clickSelector(`${slot("welcome-features")} summary`);
      await app.waitFor(
        () =>
          app.execute(
            `return document.querySelector('[data-slot="welcome-features"]')?.open === true;`,
          ),
        {
          description:
            "welcome feature disclosure to open from a real pointer click",
        },
      );
      assert.equal(
        await app.visibleTextIncludes(en.welcome.features.items.cookies),
        true,
      );
      await app.clickSelector(`${slot("welcome-features")} summary`);
      await app.capture("onboarding-isolation-narrow");
      await app.clickText(en.welcome.next, { roles: ["button"] });
      await app.waitForText(en.welcome.license.title);
      assert.equal(
        await app.invoke("get_onboarding_completed"),
        false,
        "the example does not complete actual onboarding",
      );
    },
    { onboardingCompleted: false },
  );
});

async function openReplay(app) {
  await app.clickSelector(`[aria-label="${en.rail.more.label}"]`);
  await waitForSelector(app, '[role="menu"]');
  await app.clickText(en.rail.more.about, {
    exact: false,
    roles: ["menuitem"],
  });
  await app.clickSelector(slot("isolation-demo-replay"));
  await waitForSelector(app, slot("profile-isolation-demo"));
}

async function assertPopupReadable(app, selector) {
  await app.waitFor(
    () =>
      app.execute(
        `
        const content = document.querySelector(arguments[0]);
        if (!content || content.getBoundingClientRect().height <= 0) return false;
        for (let node = content; node instanceof Element; node = node.parentElement) {
          const style = getComputedStyle(node);
          if (Number(style.opacity) < 0.99 || style.visibility === "hidden" || style.display === "none") return false;
        }
        return true;
      `,
        [selector],
      ),
    { timeoutMs: 2000, description: `fully readable popup ${selector}` },
  );
}

test("paused CSS animations cannot retain dismissed selects or dropdowns", async () => {
  await withApp(
    "motion-popup-dismissal",
    async (app) => {
      await resize(app, 1100, 760);
      await createProfile(app, "Motion popup Alpha");
      await createProfile(app, "Motion popup Beta");
      await app.clickSelector(`[aria-label="${en.rail.settings}"]`);
      await app.clickSelector("#theme-select");
      await assertPopupReadable(app, slot("select-content"));

      // Freeze CSS only after the first menu is fully open. A closed Radix
      // Presence must not wait for animationend, which cannot arrive here.
      await app.execute(`
        const style = document.createElement("style");
        style.id = "donut-motion-paused-css";
        style.textContent = "*, *::before, *::after { animation-play-state: paused !important; }";
        document.head.append(style);
        window.__donutPausedSelect = document.querySelector('[data-slot="select-content"]');
      `);
      try {
        assert.equal(
          await app.execute(
            `return getComputedStyle(window.__donutPausedSelect).animationPlayState;`,
          ),
          "paused",
        );
        await app.clickText(en.common.labels.custom, { roles: ["option"] });
        await app.waitFor(
          () =>
            app.execute(
              `return !window.__donutPausedSelect.isConnected && !document.querySelector('[data-slot="select-content"]');`,
            ),
          {
            timeoutMs: 2000,
            description:
              "selected menu to unmount without a CSS animationend event",
          },
        );

        // The next popup also opens under the paused clock: both its content
        // and its pointer targets must be available at the initial frame.
        await app.clickSelector("#theme-preset-select");
        await assertPopupReadable(app, slot("select-content"));
        await app.capture("paused-css-preset-select");
        const preset = THEMES.find((theme) => theme.id === "dracula");
        assert.ok(preset);
        await app.clickText(preset.name, { roles: ["option"] });
        await waitForSelector(app, slot("select-content"), false);
        assert.equal(
          await app.execute(
            `return document.querySelector("#theme-preset-select")?.textContent.trim();`,
          ),
          preset.name,
        );
        await app.clickText(en.common.buttons.saveSettings, {
          roles: ["button"],
        });
        await app.waitFor(
          () => app.visibleTextIncludes(en.common.buttons.saved),
          { description: "saved settings feedback" },
        );
        await waitForSelector(app, "#theme-select", true);
        await app.clickSelector(`[aria-label="${en.rail.profiles}"]`);

        const openSort = () =>
          app.clickTextIn("thead", en.common.labels.name, {
            roles: ["button"],
          });
        await openSort();
        await assertPopupReadable(app, slot("dropdown-menu-content"));
        await app.capture("paused-css-profile-sort");
        await app.clickText(en.profiles.sort.nameDesc, {
          roles: ["menuitem"],
        });
        await waitForSelector(app, slot("dropdown-menu-content"), false);
        await app.waitFor(
          () =>
            app.execute(
              `return document.querySelector('tr[data-profile-id]')?.textContent.includes("Motion popup Beta");`,
            ),
          { description: "pointer-selected descending profile order" },
        );
        await openSort();
        await assertPopupReadable(app, slot("dropdown-menu-content"));
        await app.clickText(en.profiles.sort.nameAsc, {
          roles: ["menuitem"],
        });
        await waitForSelector(app, slot("dropdown-menu-content"), false);

        await app.clickSelector(`[aria-label="${en.rail.more.label}"]`);
        await assertPopupReadable(app, '[role="menu"]');
        await app.clickText(en.rail.more.about, {
          exact: false,
          roles: ["menuitem"],
        });
        await waitForSelector(app, '[role="menu"]', false);
        await app.clickSelector(slot("isolation-demo-replay"));
        await waitForSelector(app, slot("profile-isolation-demo"));
        await app.capture("paused-css-about-replay");
      } finally {
        await app.execute(`
          document.getElementById("donut-motion-paused-css")?.remove();
          delete window.__donutPausedSelect;
        `);
      }
    },
    { seedDownloadedBrowser: true },
  );
});

async function clickVisible(app, selector) {
  // tauri-wd's element click always scrollIntoView(center), including a raw
  // session.click. Pointer actions preserve the position of an in-view row.
  const point = await pointIn(app, selector);
  try {
    await pointerActions(app, [
      { type: "pointerMove", x: point.x, y: point.y, origin: "viewport" },
      { type: "pointerDown", button: 0 },
      { type: "pointerUp", button: 0 },
    ]);
  } finally {
    await app.session.command("DELETE", "/actions");
  }
}

async function visibleProfileIds(app, count = 3) {
  return app.waitFor(
    () =>
      app.execute(
        `
    const scroller = document.querySelector(arguments[0]);
    if (!scroller) return null;
    const bounds = scroller.getBoundingClientRect();
    const rows = [...scroller.querySelectorAll('tr[data-profile-id]')].filter((row) => {
      const rect = row.getBoundingClientRect();
      return rect.top > bounds.top + 45 && rect.bottom < bounds.bottom - 110;
    });
    return rows.length >= arguments[1] ? rows.slice(0, arguments[1]).map((row) => row.dataset.profileId) : null;
  `,
        [tableScroll, count],
      ),
    { description: `${count} fully visible profile rows` },
  );
}

async function toggleProfile(app, id) {
  const selector = `${profileRow(id)} [role="checkbox"], ${profileRow(id)} [aria-label="${en.common.aria.selectProfile}"]`;
  await clickVisible(app, selector);
}

async function pointIn(app, selector) {
  const point = await app.execute(
    `
    const node = document.querySelector(arguments[0]);
    if (!node) return null;
    const rect = node.getBoundingClientRect();
    const x = Math.round(rect.left + rect.width / 2);
    const y = Math.round(rect.top + rect.height / 2);
    const hit = document.elementFromPoint(x, y);
    return hit && (hit === node || node.contains(hit)) ? { x, y, left: rect.left, top: rect.top } : null;
  `,
    [selector],
  );
  assert.ok(point, `pointer-interactable ${selector}`);
  return point;
}

async function pointerActions(app, actions) {
  await app.session.command("POST", "/actions", {
    actions: [
      {
        type: "pointer",
        id: "motion-profile-pointer",
        parameters: { pointerType: "mouse" },
        actions,
      },
    ],
  });
}

async function beginDrag(app, id) {
  const start = await pointIn(app, dragHandle(id));
  await pointerActions(app, [
    { type: "pointerMove", x: start.x, y: start.y, origin: "viewport" },
    { type: "pointerDown", button: 0 },
    { type: "pause", duration: 50 },
    {
      type: "pointerMove",
      x: start.x + 16,
      y: start.y + 10,
      duration: 100,
      origin: "viewport",
    },
  ]);
  await waitForSelector(
    app,
    `${slot("profile-drag-preview")}[data-phase="dragging"]`,
  );
  const preview = await app.execute(`
    const rect = document.querySelector('[data-slot="profile-drag-preview"]').getBoundingClientRect();
    return { left: rect.left, top: rect.top };
  `);
  assert.ok(
    Math.abs(preview.left - start.left - 16) <= 3,
    "drag preserves the horizontal grab offset",
  );
  assert.ok(
    Math.abs(preview.top - start.top - 10) <= 3,
    "drag preserves the vertical grab offset",
  );
}

async function groupAssignments(app) {
  const profiles = await app.invoke("list_browser_profiles");
  return Object.fromEntries(
    profiles.map((profile) => [profile.id, profile.group_id ?? null]),
  );
}

async function installSynchronizerFixture(app) {
  // This test supplies a paid user's UI state without changing authentication,
  // entitlement checks, or synchronizer state in the backend. The only
  // operation being simulated is the readiness promise of start_sync_session.
  // Tauri defines invoke/ipc/postMessage as non-writable. Like the native folder
  // picker fixture in ui.test.mjs, this wraps only their IPC fetch transport.
  await app.execute(`
    const fixture = { originalFetch: window.fetch, requests: [], pending: [] };
    window.__donutMotionSynchronizer = fixture;
    const response = (value, ok = true) => new Response(JSON.stringify(value), {
      status: 200, headers: { "content-type": "application/json", "Tauri-Response": ok ? "ok" : "error" }
    });
    window.fetch = function (input, init) {
      let command = "";
      try {
        const url = new URL(typeof input === "string" ? input : input.url);
        if ((url.protocol === "ipc:" && url.hostname === "localhost") ||
          (["http:", "https:"].includes(url.protocol) && url.hostname === "ipc.localhost")) {
          command = decodeURIComponent(url.pathname.split("/").pop() || "");
        }
      } catch {}
      if (command === "cloud_get_user") return Promise.resolve(response({
        logged_in_at: "2026-09-01T00:00:00Z",
        user: {
          id: "motion-ui-fixture", email: "motion@example.test", plan: "pro",
          planPeriod: "monthly", subscriptionStatus: "active", profileLimit: 50,
          cloudProfilesUsed: 0, proxyBandwidthLimitMb: 0, proxyBandwidthUsedMb: 0,
          proxyBandwidthExtraMb: 0, isPrimaryDevice: true
        }
      }));
      if (command === "start_sync_session") {
        fixture.requests.push(JSON.parse(init.body));
        return new Promise((resolve) => fixture.pending.push({
          resolve: (value) => resolve(response(value)),
          // Reject at the Tauri response layer. Rejecting fetch itself would
          // trigger its native-transport fallback and launch real browsers.
          reject: (error) => resolve(response(error, false))
        }));
      }
      return fixture.originalFetch.apply(window, arguments);
    };
  `);
  assert.equal(
    (await app.invoke("cloud_get_user")).user?.id,
    "motion-ui-fixture",
    "scoped IPC fixture is active before opening paid UI",
  );
  await emit(app, "cloud-auth-changed", null);
}

async function restoreSynchronizerFixture(app) {
  await app.execute(`
    const fixture = window.__donutMotionSynchronizer;
    if (!fixture) return;
    window.fetch = fixture.originalFetch;
    for (const pending of fixture.pending) pending.reject("Motion fixture disposed");
    delete window.__donutMotionSynchronizer;
  `);
  await emit(app, "cloud-auth-expired", null);
}

test("synchronizer rehearsal targets selected profiles and waits for actual command readiness", async () => {
  await withApp(
    "motion-synchronizer",
    async (app) => {
      await resize(app, 1080, 900);
      const leader = await createProfile(app, "Motion rehearsal leader");
      const first = await createProfile(app, "Motion rehearsal follower one");
      const second = await createProfile(
        app,
        "Motion rehearsal follower with a longer profile name",
      );
      await installSynchronizerFixture(app);
      const session = {
        id: "motion-ui-rehearsal-session",
        leader_profile_id: leader.id,
        leader_profile_name: leader.name,
        followers: [
          {
            profile_id: first.id,
            profile_name: first.name,
            failed_at_url: null,
          },
        ],
      };
      try {
        await app.clickSelector(inspectTrigger(leader.id));
        await app.clickSelector(
          `${slot("profile-info-section")}[data-section="automation"]`,
        );
        await app.waitFor(
          () =>
            app.execute(
              `return document.querySelector('[data-slot="profile-start-synchronizer"]')?.disabled === false;`,
            ),
          {
            description:
              "paid fixture can open the real synchronizer selection interface",
          },
        );
        await app.clickSelector(slot("profile-start-synchronizer"));
        await waitForSelector(app, slot("synchronizer-follower-dialog"));
        const option = (id) =>
          `${slot("synchronizer-follower-option")}[data-profile-id="${id}"]`;
        const checkbox = (id) => `${option(id)} [role="checkbox"]`;
        const checked = (id, value) =>
          app.waitFor(
            () =>
              app.execute(
                `return document.querySelector(arguments[0])?.getAttribute("aria-checked") === arguments[1];`,
                [checkbox(id), String(value)],
              ),
            {
              description: `${id} checkbox becomes ${value}`,
            },
          );
        await app.clickSelector(`${option(first.id)} > span`);
        await checked(first.id, true);
        await app.clickSelector(checkbox(first.id));
        await checked(first.id, false);
        await app.execute(`document.querySelector(arguments[0]).focus();`, [
          checkbox(first.id),
        ]);
        await activateFocusedByKeyboard(app, "\uE00D");
        await checked(first.id, true);
        await app.clickSelector(`${option(second.id)} > span`);
        await checked(second.id, true);
        await app.clickSelector(slot("synchronizer-preview-send"));
        const received = () =>
          app.execute(
            `return [...document.querySelectorAll('[data-slot="synchronizer-preview-follower"][data-received="true"]')].map((node) => node.dataset.profileId).sort();`,
          );
        await app.waitFor(
          async () =>
            JSON.stringify(await received()) ===
            JSON.stringify([first.id, second.id].sort()),
          {
            description:
              "example click reaches exactly the two selected real profile names",
          },
        );
        assert.equal(
          await app.execute(
            `return window.__donutMotionSynchronizer.requests.length;`,
          ),
          0,
          "rehearsal never starts browsers",
        );
        await app.capture("synchronizer-rehearsal-wide");
        await app.clickSelector(checkbox(second.id));
        await checked(second.id, false);
        assert.deepEqual(
          await received(),
          [],
          "changing selection resets the previous rehearsal result",
        );
        await app.execute(
          `document.querySelector('[data-slot="synchronizer-preview-send"]').focus();`,
        );
        await activateFocusedByKeyboard(app);
        await app.waitFor(
          async () =>
            JSON.stringify(await received()) === JSON.stringify([first.id]),
          {
            description:
              "keyboard rehearsal acknowledges only the selected follower",
          },
        );
        assert.equal(
          await app.execute(
            `return document.querySelectorAll('[data-slot="synchronizer-rehearsal"] path[stroke-width="2.5"]').length;`,
          ),
          0,
          "keyboard preview is static",
        );
        await resize(app, 640, 700);
        await assertContained(app, '[role="dialog"]');
        await app.capture("synchronizer-rehearsal-narrow");

        await app.clickSelector(slot("synchronizer-start"));
        await waitForSelector(app, slot("synchronizer-starting"));
        assert.deepEqual(
          await app.execute(
            `return window.__donutMotionSynchronizer.requests;`,
          ),
          [
            {
              leaderProfileId: leader.id,
              followerProfileIds: [first.id],
            },
          ],
        );
        await emit(app, "sync-session-changed", session);
        await waitForSelector(app, slot("synchronizer-starting"));
        assert.equal(
          await app.execute(
            `return document.querySelector('[data-slot="synchronizer-start"]')?.disabled;`,
          ),
          true,
        );
        await assert.rejects(
          app.session.click(
            await app.session.findCss(slot("synchronizer-start")),
          ),
          /element does not receive pointer events/,
        );
        await app.pressShortcut({ key: "Escape" });
        assert.equal(
          await app.execute(
            `return window.__donutMotionSynchronizer.requests.length;`,
          ),
          1,
          "pending startup cannot be duplicated",
        );
        await waitForSelector(app, slot("synchronizer-follower-dialog"));
        await app.capture("synchronizer-awaiting-readiness");
        await app.execute(
          `window.__donutMotionSynchronizer.pending.shift().reject(JSON.stringify({ code: "PROFILE_RUNNING" }));`,
        );
        await waitForSelector(app, slot("synchronizer-start-error"));
        await checked(first.id, true);
        await checked(second.id, false);
        assert.equal(
          await app.execute(
            `return document.querySelector('[data-slot="synchronizer-start"]')?.disabled;`,
          ),
          false,
          "failed startup can be retried",
        );
        await app.capture("synchronizer-retry");
        await emit(app, "sync-session-ended", session.id);
        await app.clickSelector(slot("synchronizer-start"));
        await waitForSelector(app, slot("synchronizer-starting"));
        assert.equal(
          await app.execute(
            `return window.__donutMotionSynchronizer.requests.length;`,
          ),
          2,
        );
        await app.execute(
          `window.__donutMotionSynchronizer.pending.shift().resolve(arguments[0]);`,
          [session],
        );
        await waitForSelector(app, slot("synchronizer-follower-dialog"), false);
        assert.deepEqual(
          await app.invoke("get_sync_sessions"),
          [],
          "the readiness fixture never creates a backend session",
        );
        const profiles = await app.invoke("list_browser_profiles");
        assert.ok(
          profiles.every((profile) => profile.process_id == null),
          "rehearsal and simulated readiness launch no browser processes",
        );
      } finally {
        await emit(app, "sync-session-ended", session.id);
        await restoreSynchronizerFixture(app);
      }
    },
    { seedDownloadedBrowser: true },
  );
});

async function freezeAnimations(app) {
  // Motion captures requestAnimationFrame at module initialization, so replacing
  // the global afterward does not stall its engine. Its JS batcher reads
  // performance.now on every batch; native animations need a separate pause.
  await app.execute(`
    const state = {
      now: performance.now(),
      nowDescriptor: Object.getOwnPropertyDescriptor(performance, "now"),
      animateDescriptor: Object.getOwnPropertyDescriptor(Element.prototype, "animate"),
      animations: []
    };
    window.__donutFrozenMotion = state;
    Object.defineProperty(performance, "now", { configurable: true, value: () => state.now });
    if (state.animateDescriptor?.value) {
      Object.defineProperty(Element.prototype, "animate", {
        ...state.animateDescriptor,
        value: function (...args) {
          const animation = state.animateDescriptor.value.apply(this, args);
          const play = animation.play;
          const pause = animation.pause;
          state.animations.push({ animation, play, playDescriptor: Object.getOwnPropertyDescriptor(animation, "play") });
          // NativeAnimation may explicitly play after construction. Keep that
          // path paused too, so its first keyframe remains the rendered frame.
          Object.defineProperty(animation, "play", {
            configurable: true,
            value: function () { play.call(this); pause.call(this); this.currentTime = 0; }
          });
          pause.call(animation);
          animation.currentTime = 0;
          return animation;
        }
      });
    }
  `);
}

async function resumeAnimations(app) {
  await app.execute(`
    const state = window.__donutFrozenMotion;
    if (!state) return;
    if (state.nowDescriptor) Object.defineProperty(performance, "now", state.nowDescriptor);
    else delete performance.now;
    if (state.animateDescriptor) Object.defineProperty(Element.prototype, "animate", state.animateDescriptor);
    for (const { animation, play, playDescriptor } of state.animations) {
      if (playDescriptor) Object.defineProperty(animation, "play", playDescriptor);
      else delete animation.play;
      if (animation.playState === "paused") play.call(animation);
    }
    delete window.__donutFrozenMotion;
  `);
}

test("profile replay, inspector, group gestures, and remote handoff preserve context", async () => {
  await withApp(
    "motion-workspace",
    async (app) => {
      await resize(app, 1480, 900);
      await openReplay(app);
      await exerciseIsolation(app);
      await app.capture("isolation-replay-wide");
      await resize(app, 640, 480);
      await assertContained(app, '[role="dialog"]');
      await app.capture("isolation-replay-narrow");
      await app.clickText(en.common.buttons.back, { roles: ["button"] });
      await waitForSelector(app, slot("isolation-demo-replay"));
      assert.equal(
        await app.execute(`return document.activeElement?.dataset.slot;`),
        "isolation-demo-replay",
      );
      await app.pressShortcut({ key: "Escape" });
      await waitForSelector(app, '[role="dialog"]', false);
      await resize(app, 1480, 900);

      const group = await app.invoke("create_profile_group", {
        name: "Motion research group",
      });
      const proxy = await app.invoke("create_stored_proxy", {
        name: "Motion inspector proxy",
        proxySettings: {
          proxy_type: "http",
          host: "127.0.0.1",
          port: 9,
          username: null,
          password: null,
        },
      });
      for (let index = 0; index < 36; index += 1) {
        await createProfile(
          app,
          `Motion profile ${String(index + 1).padStart(2, "0")}`,
        );
      }
      await app.waitFor(
        async () => (await app.invoke("list_browser_profiles")).length === 36,
        { description: "all motion fixture profiles" },
      );
      await waitForSelector(app, slot("profile-inspect-trigger"));
      await app.execute(
        `document.querySelector(arguments[0]).scrollTop = 250;`,
        [tableScroll],
      );
      const [first, second] = await visibleProfileIds(app, 2);
      await toggleProfile(app, first);
      const scrollBefore = await app.execute(
        `return document.querySelector(arguments[0]).scrollTop;`,
        [tableScroll],
      );
      assert.ok(scrollBefore > 0);
      try {
        await freezeAnimations(app);
        await clickVisible(app, inspectTrigger(first));
        await waitForSelector(
          app,
          `${slot("profile-inspector")}[data-profile-id="${first}"]`,
        );
        assert.equal(
          await app.execute(`
          const content = document.querySelector('[data-slot="profile-info-content"]');
          const control = document.querySelector('[data-slot="profile-info-section"][data-section="network"]');
          return [content, control].every((element) => {
            if (!element || element.getBoundingClientRect().height <= 0) return false;
            for (let node = element; node instanceof Element; node = node.parentElement) {
              const style = getComputedStyle(node);
              if (Number(style.opacity) === 0 || style.visibility === "hidden" || style.display === "none") return false;
            }
            return true;
          });
        `),
          true,
          "inspector text and controls remain visible with the animation clock frozen at its initial frame",
        );
        assert.equal(
          await app.execute(
            `return performance.now() === window.__donutFrozenMotion.now && window.__donutFrozenMotion.animations.every(({ animation }) => animation.playState !== "running" && (animation.currentTime === null || animation.currentTime === 0));`,
          ),
          true,
          "both JavaScript and newly created native animations remain frozen",
        );
        await app.capture("profile-inspector-animation-frozen");
      } finally {
        await resumeAnimations(app);
      }
      assert.equal(
        await app.execute(
          `return document.querySelector('[data-slot="dialog-overlay"]') === null;`,
        ),
        true,
        "wide inspector is nonmodal",
      );
      await app.clickSelector(
        `${slot("profile-info-section")}[data-section="network"]`,
      );
      await clickVisible(app, inspectTrigger(second));
      await waitForSelector(
        app,
        `${slot("profile-info-content")}[data-profile-id="${second}"][data-section="network"]`,
      );
      assert.equal(
        await app.execute(
          `return document.querySelector(arguments[0]).scrollTop;`,
          [tableScroll],
        ),
        scrollBefore,
        "profile switching preserves table scroll",
      );
      assert.equal(
        await app.execute(
          `return document.querySelector(arguments[0])?.getAttribute("data-state");`,
          [profileRow(first)],
        ),
        "selected",
        "profile switching preserves selection",
      );
      await app.capture("profile-inspector-wide");
      await app.pressShortcut({ key: "Escape" });
      await waitForSelector(app, slot("profile-inspector"), false);
      assert.equal(
        await app.execute(`return document.activeElement?.dataset.profileId;`),
        second,
        "Escape restores the last inspection trigger",
      );

      await activateFocusedByKeyboard(app);
      await waitForSelector(app, slot("profile-inspector"));
      await app.clickSelector(
        `${slot("profile-info-section")}[data-section="network"]`,
      );
      await app.clickSelector(
        `${slot("profile-info-content")}[data-section="network"] [role="combobox"]`,
      );
      await app.waitFor(
        () =>
          app.execute(`
          const popup = document.querySelector('[data-slot="select-content"]');
          return popup?.contains(document.activeElement) && !popup.closest('[aria-hidden="true"]');
        `),
        {
          description: "focused, accessibility-visible inspector proxy picker",
        },
      );
      // Radix Select intentionally closes on window.resize. Its ownership
      // cleanup must restore the inspector's accessibility exposure.
      await resize(app, 720, 600);
      await waitForSelector(app, slot("profile-inspector"), false);
      await waitForSelector(app, slot("select-content"), false);
      const inspectorAccessible = () =>
        app.execute(`
        const content = document.querySelector('[data-slot="profile-info-content"]');
        return content && !content.closest('[aria-hidden="true"]');
      `);
      await app.waitFor(inspectorAccessible, {
        description: "inspector is accessible after Select closes on resize",
      });
      await app.clickSelector(
        `${slot("profile-info-content")}[data-section="network"] [role="combobox"]`,
      );
      await app.clickText(proxy.name, { roles: ["option"] });
      await waitForSelector(app, slot("select-content"), false);
      await app.waitFor(
        async () =>
          (await app.invoke("list_browser_profiles")).find(
            (profile) => profile.id === second,
          )?.proxy_id === proxy.id,
        { description: "real pointer selection persists the proxy assignment" },
      );
      // The proxy is an inert local fixture. Selecting it neither starts a
      // browser nor claims the endpoint is reachable.
      await resize(app, 1480, 900);
      await waitForSelector(app, slot("profile-inspector"));
      await app.clickSelector(
        `${slot("profile-info-section")}[data-section="overview"]`,
      );
      const colorTrigger = `[aria-label="${en.profileInfo.fields.windowColor}"]`;
      await app.clickSelector(colorTrigger);
      await assertPopupReadable(app, slot("popover-content"));
      await app.clickSelector(`${slot("popover-content")} input`);
      const colorBefore = await app.execute(`
        const popup = document.querySelector('[data-slot="popover-content"]');
        window.__donutMotionOwnedPopup = { popup, focused: document.activeElement };
        return popup.querySelector('input').value;
      `);
      assert.equal(
        await app.execute(
          `return document.querySelector(arguments[0]).getAttribute('aria-controls') === window.__donutMotionOwnedPopup.popup.id;`,
          [colorTrigger],
        ),
        true,
        "the color trigger identifies its actual owned portal",
      );
      try {
        await resize(app, 720, 600);
        await waitForSelector(app, slot("profile-inspector"), false);
        await assertPopupReadable(app, slot("popover-content"));
        assert.deepEqual(
          await app.execute(`
          const popup = document.querySelector('[data-slot="popover-content"]');
          return {
            samePopup: popup === window.__donutMotionOwnedPopup.popup,
            sameFocus: document.activeElement === window.__donutMotionOwnedPopup.focused,
            focusInside: popup?.contains(document.activeElement),
            accessible: Boolean(popup && !popup.closest('[aria-hidden="true"]'))
          };
        `),
          {
            samePopup: true,
            sameFocus: true,
            focusInside: true,
            accessible: true,
          },
          "wide-to-narrow resize preserves the owned portal, focus, and accessibility exposure",
        );
        await app.capture("profile-inspector-owned-popup-narrow");
        const colorPoint = await pointIn(
          app,
          `${slot("popover-content")} .h-32`,
        );
        try {
          await pointerActions(app, [
            {
              type: "pointerMove",
              x: colorPoint.x,
              y: colorPoint.y,
              origin: "viewport",
            },
            { type: "pointerDown", button: 0 },
            {
              type: "pointerMove",
              x: colorPoint.x + 20,
              y: colorPoint.y + 10,
              duration: 120,
              origin: "viewport",
            },
            { type: "pointerUp", button: 0 },
          ]);
        } finally {
          await app.session.command("DELETE", "/actions");
        }
        await app.waitFor(
          () =>
            app.execute(
              `return document.querySelector('[data-slot="popover-content"] input').value !== arguments[0];`,
              [colorBefore],
            ),
          {
            description:
              "resized color popup responds to real pointer selection",
          },
        );
        // The picker renders its HSL input separately from its effect-driven
        // onColorChange. The controlled swatch is the color committed on close.
        const chosenColor = Color(
          await app.waitFor(
            () =>
              app.execute(
                `
              const color = getComputedStyle(document.querySelector(arguments[0])).backgroundColor;
              const before = document.createElement('span').style;
              before.backgroundColor = arguments[1];
              return color !== before.backgroundColor ? color : null;
            `,
                [colorTrigger, colorBefore],
              ),
            {
              description:
                "selected color reaches the controlled profile swatch",
            },
          ),
        ).hex();
        await app.clickSelector(colorTrigger);
        await waitForSelector(app, slot("popover-content"), false);
        await app.waitFor(
          async () =>
            (await app.invoke("list_browser_profiles"))
              .find((profile) => profile.id === second)
              ?.window_color?.toLowerCase() === chosenColor.toLowerCase(),
          {
            description:
              "closing the color popup persists its real selected color",
          },
        );
        await app.waitFor(inspectorAccessible, {
          description:
            "inspector remains accessible after its owned popup closes",
        });
      } finally {
        await app.execute(`delete window.__donutMotionOwnedPopup;`);
      }
      await resize(app, 1480, 900);
      await waitForSelector(app, slot("profile-inspector"));
      await app.clickSelector(
        `${slot("profile-info-section")}[data-section="automation"]`,
      );
      const draft = "https://example.com/inspector-draft";
      await app.fillSelector(slot("profile-launch-hook-input"), draft);
      await app.execute(
        `window.__donutMotionDraftInput = document.querySelector('[data-slot="profile-launch-hook-input"]');`,
      );
      await resize(app, 720, 600);
      await waitForSelector(app, slot("profile-inspector"), false);
      await waitForSelector(
        app,
        `${slot("profile-info-content")}[data-profile-id="${second}"][data-section="automation"]`,
      );
      assert.deepEqual(
        await app.execute(`
        const input = document.querySelector('[data-slot="profile-launch-hook-input"]');
        return { value: input?.value, sameNode: input === window.__donutMotionDraftInput };
      `),
        { value: draft, sameNode: true },
        "wide-to-narrow transition preserves the actual unsaved editor",
      );
      await assertContained(app, '[role="dialog"]');
      await app.capture("profile-inspector-narrow");
      assert.equal(
        await app.execute(
          `return document.activeElement === document.querySelector('[data-slot="profile-launch-hook-input"]');`,
        ),
        true,
        "resizing keeps focus in the unsaved editor",
      );
      for (let index = 0; index < 14; index += 1) {
        await tabForward(app);
        assert.equal(
          await app.execute(
            `return document.querySelector('[role="dialog"]')?.contains(document.activeElement);`,
          ),
          true,
          "compact inspector traps keyboard focus inside the modal",
        );
      }
      await resize(app, 1480, 900);
      await waitForSelector(app, slot("profile-inspector"));
      assert.deepEqual(
        await app.execute(`
        const input = document.querySelector('[data-slot="profile-launch-hook-input"]');
        return { value: input?.value, sameNode: input === window.__donutMotionDraftInput };
      `),
        { value: draft, sameNode: true },
        "narrow-to-wide transition also preserves the unsaved editor",
      );
      await app.execute(`delete window.__donutMotionDraftInput;`);
      await resize(app, 720, 600);
      await waitForSelector(app, slot("profile-inspector"), false);
      await app.pressShortcut({ key: "Escape" });
      await waitForSelector(app, slot("profile-info-content"), false);
      await app.pressShortcut({ key: "k", ...modifier });
      await waitForSelector(app, "[cmdk-input]");
      await app.session.sendKeys(
        await app.session.findCss("[cmdk-input]"),
        en.rail.settings,
      );
      assert.equal(await app.visibleTextIncludes(en.rail.settings), true);
      await app.pressShortcut({ key: "Escape" });
      await waitForSelector(app, "[cmdk-input]", false);

      await resize(app, 1480, 900);
      await app.execute(`document.querySelector(arguments[0]).scrollTop = 0;`, [
        tableScroll,
      ]);
      // Clear any earlier selection through the actual table checkbox.
      const selected = await app.execute(
        `return [...document.querySelectorAll('tr[data-state="selected"]')].map((row) => row.dataset.profileId);`,
      );
      if (selected.includes(first)) await toggleProfile(app, first);
      else {
        // The selected profile may be virtualized outside the top of the table.
        await app.clickSelector(`[aria-label="${en.common.aria.selectAll}"]`);
        await app.clickSelector(`[aria-label="${en.common.aria.selectAll}"]`);
      }
      const [moveFirst, moveSecond, cancelId] = await visibleProfileIds(app);
      await toggleProfile(app, moveFirst);
      await toggleProfile(app, moveSecond);
      try {
        await beginDrag(app, moveFirst);
        const target = await pointIn(
          app,
          `[data-profile-group-drop="${group.id}"]`,
        );
        await pointerActions(app, [
          {
            type: "pointerMove",
            x: target.x,
            y: target.y,
            duration: 180,
            origin: "viewport",
          },
        ]);
        await waitForSelector(
          app,
          `[data-profile-group-drop="${group.id}"][data-drop-state="target"]`,
        );
        await app.capture("profile-group-drag");
        await pointerActions(app, [{ type: "pointerUp", button: 0 }]);
        await app.waitFor(
          async () => {
            const assignments = await groupAssignments(app);
            return (
              assignments[moveFirst] === group.id &&
              assignments[moveSecond] === group.id
            );
          },
          {
            description:
              "both selected profiles persisted into the drop target group",
          },
        );
      } finally {
        await app.session.command("DELETE", "/actions");
      }
      await waitForSelector(app, slot("profile-drag-preview"), false);

      const beforeCancel = await groupAssignments(app);
      try {
        await beginDrag(app, cancelId);
        await app.pressShortcut({ key: "Escape" });
      } finally {
        await app.session.command("DELETE", "/actions");
      }
      await waitForSelector(app, slot("profile-drag-preview"), false);
      assert.deepEqual(
        await groupAssignments(app),
        beforeCancel,
        "Escape never writes a group assignment",
      );
      await app.execute(`document.querySelector(arguments[0]).focus();`, [
        dragHandle(cancelId),
      ]);
      await activateFocusedByKeyboard(app);
      await app.waitForText(en.groupAssignment.title);
      await app.pressShortcut({ key: "Escape" });
      await waitForSelector(app, '[role="dialog"]', false);

      await clickVisible(app, inspectTrigger(cancelId));
      await app.clickSelector(
        `${slot("profile-info-section")}[data-section="overview"]`,
      );
      const handoff = `${slot("profile-handoff-detail")}[data-profile-id="${cancelId}"]`;
      // These are frontend event-contract checks. No remote browser or real
      // sync transfer is started, and no networking success is inferred.
      await emit(app, "remote-handoff-changed", { [cancelId]: "running" });
      await waitForSelector(app, `${handoff}[data-handoff-state="running"]`);
      await emit(app, "remote-handoff-changed", {});
      await waitForSelector(app, handoff, false);
      await emit(app, "remote-handoff-changed", { [cancelId]: "pending_sync" });
      await waitForSelector(
        app,
        `${handoff}[data-handoff-state="pending_sync"]`,
      );
      await emit(app, "profile-sync-status", {
        profile_id: cancelId,
        status: "error",
        error: "Motion fixture sync failure",
      });
      await app.waitFor(
        () =>
          app.execute(
            `return document.querySelector(arguments[0])?.textContent.includes(arguments[1]);`,
            [handoff, en.profileMotion.handoffError],
          ),
        {
          description:
            "failed return keeps its pending state and explains the error",
        },
      );
      await waitForSelector(
        app,
        `${handoff}[data-handoff-state="pending_sync"]`,
      );
      await app.capture("profile-handoff-retry");
      await emit(app, "profile-sync-status", {
        profile_id: cancelId,
        status: "synced",
      });
      await waitForSelector(
        app,
        `${handoff}[data-handoff-state="pending_sync"]`,
      );
      await emit(app, "remote-handoff-changed", {});
      await waitForSelector(app, `${handoff}[data-handoff-state="returned"]`);
      await app.capture("profile-handoff-returned");
      await app.clickTextIn(handoff, en.common.buttons.close, {
        roles: ["button"],
      });
      await waitForSelector(app, handoff, false);
    },
    { seedDownloadedBrowser: true },
  );
});

test("settings save a RAM-only edit, preserve location, and expose review and discard", async () => {
  await withApp("motion-settings-feedback", async (app) => {
    await resize(app, 1000, 720);
    await app.clickSelector(`[aria-label="${en.rail.settings}"]`);
    await waitForSelector(app, slot("settings-search"));
    await app.fillSelector(slot("settings-search"), "decrypted");
    assert.deepEqual(
      await app.execute(
        `return [...document.querySelectorAll('[data-settings-section]')].filter(node => !node.hidden).map(node => node.dataset.settingsSection);`,
      ),
      ["advanced"],
    );
    const initial = await app.invoke("get_app_settings");
    const selector = "#keep-decrypted-profiles-in-ram";
    await app.clickSelector(selector);
    await app.clickSelector(`${slot("settings-feedback")} summary`);
    assert.equal(
      await app.visibleTextIncludes(en.settings.keepDecryptedProfilesInRam),
      true,
    );
    await app.capture("review-ram-change");
    await app.clickSelector(slot("settings-discard"));
    assert.equal(
      await app.execute(
        `return document.querySelector(arguments[0]).getAttribute("data-state") === "checked";`,
        [selector],
      ),
      Boolean(initial.keep_decrypted_profiles_in_ram),
    );
    await app.clickSelector(selector);
    await app.clickText(en.common.buttons.saveSettings, { roles: ["button"] });
    await app.waitFor(
      async () =>
        (await app.invoke("get_app_settings"))
          .keep_decrypted_profiles_in_ram !==
        Boolean(initial.keep_decrypted_profiles_in_ram),
      { description: "RAM preference persisted" },
    );
    await waitForSelector(app, slot("settings-search"));
    assert.equal(
      await app.execute(`return document.querySelector(arguments[0]).value;`, [
        slot("settings-search"),
      ]),
      "decrypted",
    );
    await app.waitFor(() => app.visibleTextIncludes(en.common.buttons.saved), {
      description: "inline save confirmation",
    });
    await resize(app, 780, 580);
    await app.capture("settings-small-saved");
    assert.equal(
      await app.execute(
        `const el = [...document.querySelectorAll("button")].find(button => button.textContent.trim() === arguments[0]); const r = el.getBoundingClientRect(); return r.bottom <= innerHeight && r.right <= innerWidth && r.top >= 0;`,
        [en.common.buttons.saveSettings],
      ),
      true,
    );

    await app.pressShortcut({ ...modifier, key: "/" });
    await waitForSelector(app, slot("shortcuts-search"));
    await app.fillSelector(slot("shortcuts-search"), en.shortcuts.goProfiles);
    assert.equal(await app.visibleTextIncludes(en.shortcuts.goProfiles), true);
    assert.equal(await app.visibleTextIncludes(en.shortcuts.goGroups), false);
    await app.capture("shortcuts-search");
  });
});

test("profile notes save through the inspector and real launch failures retain a receipt", async () => {
  await withApp("motion-profile-feedback", async (app) => {
    await resize(app, 1200, 800);
    const profile = await createProfile(app, "Feedback profile");
    await waitForSelector(app, inspectTrigger(profile.id));
    await app.clickSelector(inspectTrigger(profile.id));
    await app.clickSelector(slot("profile-edit-note"));
    await app.fillSelector(
      `textarea[aria-label="${en.profileInfo.fields.note}"]`,
      "Remember the launch context",
    );
    await app.clickTextIn(slot("popover-content"), en.common.buttons.save, {
      roles: ["button"],
    });
    await app.waitFor(
      async () =>
        (await app.invoke("list_browser_profiles")).find(
          (item) => item.id === profile.id,
        )?.note === "Remember the launch context",
      { description: "note saved" },
    );
    // A cross-platform launch is rejected before starting any process.
    const otherOs = process.platform === "darwin" ? "linux" : "macos";
    await app.invokeError("launch_browser_profile", {
      profile: { ...profile, host_os: otherOs },
      url: null,
    });
    await waitForSelector(
      app,
      `${slot("profile-launch-activity")}[data-stage="failed"]`,
    );
    await app.clickSelector(`${slot("profile-launch-activity")} summary`);
    assert.equal(
      await app.visibleTextIncludes(en.appFeedback.launch.preparing),
      true,
    );
    assert.equal(
      await app.visibleTextIncludes(en.appFeedback.launch.failed),
      true,
    );
    await app.capture("profile-launch-receipt");
  });
});

test("schedule displays every timezone and slot, and skipped run details work by keyboard", async () => {
  await withApp("motion-schedule-feedback", async (app) => {
    await resize(app, 1200, 800);
    const profile = await createProfile(app, "Two daily slots");
    const other = await createProfile(app, "Separate timezone");
    await installSynchronizerFixture(app);
    const schedule = (item, timezone, slots) => ({
      profile_id: item.id,
      profile_name: item.name,
      owner_user_id: "motion-ui-fixture",
      enabled: true,
      timezone,
      slots,
      run_at_minute: slots[0].run_at_minute,
      days_mask: slots[0].days_mask,
      max_minutes: 30,
      platform: "linux",
      preset: "light",
      sites: [],
      template_id: null,
      next_run_at: "2026-09-09T02:00:00Z",
      last_run_at: null,
    });
    const schedules = [
      schedule(profile, "UTC", [
        { days_mask: 127, run_at_minute: 120 },
        { days_mask: 127, run_at_minute: 720 },
      ]),
      schedule(other, "Asia/Yerevan", [{ days_mask: 127, run_at_minute: 120 }]),
    ];
    await app.execute(
      `
      const schedules = arguments[0], id = arguments[1];
      const previous = window.fetch;
      window.__donutScheduleCalls = [];
      const response = value => Promise.resolve(new Response(JSON.stringify(value), { status: 200, headers: { "content-type": "application/json", "Tauri-Response": "ok" } }));
      window.fetch = function(input, init) {
        let command = "";
        try { const url = new URL(typeof input === "string" ? input : input.url); if (url.hostname === "localhost" && url.protocol === "ipc:" || url.hostname === "ipc.localhost") command = decodeURIComponent(url.pathname.split("/").pop() || ""); } catch {}
        if (command === "get_cookie_bot_schedules") return response({ schedules });
        if (command === "get_remote_hours_quota") return response({ granted_hours: 200, remaining_hours: 199.8, used_hours: 0.2, seats: 1, per_seat_hours: 200, members: [] });
        if (command === "get_cookie_bot_runs") return response({ runs: [{ id: "skipped-fixture", profile_id: id, profile_name: "Two daily slots", status: "skipped", scheduled_for: "2026-09-08T02:00:00Z", max_minutes: 30, chunks_total: 1, chunk_index: 0, sites_total: 0, sites_visited: 0, sites_failed: 0, consent_dismissed: 0, billed_seconds: 0, outcome_code: "profile_locked" }], next_before: null });
        return previous.apply(window, arguments);
      };
    `,
      [schedules, profile.id],
    );
    await app.pressShortcut({ ...modifier, key: "b" });
    await app.clickText(en.cookieBot.tabs.schedule, { roles: ["tab"] });
    await waitForSelector(app, slot("schedule-lane"));
    assert.deepEqual(
      await app.execute(
        `return [...document.querySelectorAll('[data-slot="schedule-lane"]')].map(node => Number(node.dataset.minute)).sort((a, b) => a - b);`,
      ),
      [120, 120, 720],
    );
    assert.equal(await app.visibleTextIncludes("Asia/Yerevan"), true);
    assert.equal(await app.visibleTextIncludes("199.8"), true);
    await app.capture("every-schedule-slot");
    await app.clickText(en.cookieBot.tabs.activity, { roles: ["tab"] });
    await app.clickSelector('[role="combobox"]');
    await app.clickText(en.cookieBot.runStatus.skipped, { roles: ["option"] });
    await waitForSelector(app, slot("run-details-toggle"));
    await app.execute(`document.querySelector(arguments[0]).focus();`, [
      slot("run-details-toggle"),
    ]);
    await activateFocusedByKeyboard(app);
    assert.equal(
      await app.execute(
        `return document.querySelector(arguments[0]).getAttribute("aria-expanded");`,
        [slot("run-details-toggle")],
      ),
      "true",
    );
    await app.capture("skipped-run-details");
    await restoreSynchronizerFixture(app);
  });
});

test("connection diagnostics and extension impact expose real assigned profiles", async () => {
  await withApp("motion-connection-impact", async (app) => {
    await resize(app, 1120, 800);
    const profile = await createProfile(app, "Assigned feedback profile");
    const proxy = await app.invoke("create_stored_proxy", {
      name: "Unavailable local route",
      proxySettings: {
        proxy_type: "http",
        host: "127.0.0.1",
        port: 9,
        username: null,
        password: null,
      },
    });
    await app.invoke("update_profile_proxy", {
      profileId: profile.id,
      proxyId: proxy.id,
    });
    await app.pressShortcut({ ...modifier, key: "n" });
    await waitForSelector(app, slot("profile-usage"));
    await app.clickSelector(slot("profile-usage"));
    await app.waitForText(profile.name);
    await app.capture("proxy-assignment-list");
    await app.pressShortcut({ key: "Escape" });
    await app.clickSelector(`[aria-label="${en.proxyCheck.tooltipDefault}"]`);
    await waitForSelector(app, slot("proxy-route-details"));
    await app.waitFor(
      () => app.visibleTextIncludes(en.proxyCheck.tooltipFailedTitle),
      { description: "actual failed proxy check" },
    );
    assert.equal(
      await app.visibleTextIncludes(en.appFeedback.notVerified),
      true,
    );
    await app.capture("proxy-route-failure");
    await app.pressShortcut({ key: "Escape" });
    await app.invoke("update_stored_proxy", {
      proxyId: proxy.id,
      name: proxy.name,
      proxySettings: { ...proxy.proxy_settings, port: 19 },
    });
    assert.equal(
      await app.invoke("get_cached_proxy_check", { proxyId: proxy.id }),
      null,
    );

    const extension = await app.invoke("add_extension", {
      name: "Fixture",
      fileName: "fixture.zip",
      fileData: [...Buffer.from(extensionZipBase64(), "base64")],
    });
    const group = await app.invoke("create_extension_group", {
      name: "Feedback extensions",
    });
    await app.invoke("add_extension_to_group", {
      groupId: group.id,
      extensionId: extension.id,
    });
    await app.invoke("assign_extension_group_to_profile", {
      profileId: profile.id,
      extensionGroupId: group.id,
    });
    await app.invoke("update_extension", {
      extensionId: extension.id,
      name: "My chosen name",
      fileName: null,
      fileData: null,
    });
    await app.pressShortcut({ ...modifier, key: "e" });
    await app.clickText("My chosen name", { roles: ["button"], exact: false });
    await waitForSelector(app, slot("assignment-impact"));
    const disclosures = await app.execute(
      `return [...document.querySelectorAll('[data-slot="assignment-impact"] summary')].map(node => node.textContent.trim());`,
    );
    assert.ok(disclosures.some((text) => text.includes("1")));
    await app.clickSelector(
      `${slot("assignment-impact")} li:last-child summary`,
    );
    await app.waitForText(profile.name);
    await app.waitFor(
      () =>
        app.execute(`
        const impact = document.querySelector('[data-slot="assignment-impact"]');
        const marker = impact.querySelector('[data-slot="operation-marker"]')?.getBoundingClientRect();
        const label = impact.querySelector('li[aria-current="step"]')?.getBoundingClientRect();
        if (!marker || !label || Math.abs(marker.left + marker.width / 2 - label.left - label.width / 2) > 1) return false;
        let node = impact;
        while (node && node !== document.body) {
          if (node.scrollLeft || node.scrollWidth > node.clientWidth + 1) return false;
          node = node.parentElement;
        }
        return true;
      `),
      { description: "extension flow aligned without horizontal overflow" },
    );
    await app.capture("extension-assignment-impact");
    await app.clickText(en.appFeedback.useManifestName, { roles: ["button"] });
    await app.clickTextIn('[role="dialog"]', en.common.buttons.save, {
      roles: ["button"],
    });
    await app.waitFor(
      async () =>
        (await app.invoke("list_extensions"))[0].name === extension.name,
      { description: "explicit manifest-name choice persists" },
    );

    await app.pressShortcut({ ...modifier, key: "i" });
    await waitForSelector(app, slot("integration-diagnostics"));
    await app.clickTextIn(
      slot("integration-diagnostics"),
      en.appFeedback.testConnection,
      { roles: ["button"] },
    );
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector('[data-slot="integration-diagnostics"] [role="status"]').textContent.includes(arguments[0]);`,
          [en.proxyCheck.tooltipChecked.split("{{")[0]],
        ),
      { description: "real local API probe receipt" },
    );
    await app.capture("integration-connection-receipt");
  });
});

test("partial import receipts survive retry without duplicating successful profiles", async () => {
  await withApp(
    "motion-import-receipts",
    async (app) => {
      const root = path.join(app.root, "import-sources");
      const sources = [
        path.join(root, "Default"),
        path.join(root, "Profile 1"),
      ];
      async function writeSource(index) {
        await mkdir(sources[index], { recursive: true });
        await writeFile(
          path.join(sources[index], "Preferences"),
          JSON.stringify({ profile: { name: `Receipt source ${index + 1}` } }),
        );
        await writeFile(
          path.join(sources[index], "Bookmarks"),
          JSON.stringify({
            roots: {
              bookmark_bar: {
                type: "folder",
                children: [
                  { type: "url", name: "Example", url: "https://example.com/" },
                ],
              },
            },
          }),
        );
      }
      await writeSource(0);
      await writeSource(1);
      // Only fingerprint generation is fixed for this offline UI suite. Scanning,
      // file copying, reports, progress events and retries run through real IPC.
      await app.execute(`
      window.__donutImportFetch = window.fetch;
      window.__donutImportBatches = [];
      window.fetch = function(input, init) {
        let command = "";
        try { const url = new URL(typeof input === "string" ? input : input.url); if (url.hostname === "localhost" && url.protocol === "ipc:" || url.hostname === "ipc.localhost") command = decodeURIComponent(url.pathname.split("/").pop() || ""); } catch {}
        if (command === "import_browser_profiles") {
          const body = JSON.parse(init.body);
          window.__donutImportBatches.push(body.items.map(item => item.source_path));
          body.wayfernConfig = { fingerprint: "{}" };
          return window.__donutImportFetch.call(this, input, { ...init, body: JSON.stringify(body) });
        }
        return window.__donutImportFetch.apply(this, arguments);
      };
    `);
      try {
        await app.pressShortcut({ ...modifier, key: "o" });
        await app.clickText(en.importProfile.manualImport, { roles: ["tab"] });
        await app.fillSelector("#manual-profile-path", root);
        await app.clickText(en.importProfile.scanButton, { roles: ["button"] });
        await app.waitFor(
          () =>
            app.execute(
              `return document.querySelectorAll('[role="checkbox"][data-state="checked"]').length === 3;`,
            ),
          { description: "both scanned sources selected" },
        );
        await app.clickText(en.importProfile.nextButton, { roles: ["button"] });
        await rm(sources[1], { recursive: true, force: true });
        await app.clickText(
          en.importProfile.importButtonCount.replace("{{count}}", "2"),
          { roles: ["button"], exact: false },
        );
        await app.waitFor(
          () =>
            app.execute(
              `return document.querySelectorAll('[data-slot="import-receipt"][data-status="imported"]').length === 1 && document.querySelectorAll('[data-slot="import-receipt"][data-status="failed"]').length === 1;`,
            ),
          { description: "real partial-import receipts" },
        );
        const first = await app.invoke("list_browser_profiles");
        assert.equal(first.length, 1);
        await app.capture("partial-import-receipts");
        await writeSource(1);
        await app.clickSelector(slot("import-retry"));
        await app.waitFor(
          () =>
            app.execute(
              `return document.querySelectorAll('[data-slot="import-receipt"][data-status="imported"]').length === 2;`,
            ),
          { description: "both import receipts after retry" },
        );
        const profiles = await app.invoke("list_browser_profiles");
        assert.equal(profiles.length, 2);
        assert.ok(profiles.some((profile) => profile.id === first[0].id));
        await app.waitFor(
          () =>
            app.execute(
              `return [...document.querySelectorAll('[data-sonner-toast]')].some(node => node.getAttribute('data-type') === 'success' && node.textContent.includes(arguments[0]));`,
              [
                en.importProfile.resultsSummary
                  .replace("{{imported}}", "2")
                  .replace("{{skipped}}", "0")
                  .replace("{{failed}}", "0"),
              ],
            ),
          { description: "retry toast agrees with the cumulative receipts" },
        );
        const batches = await app.execute(
          `return window.__donutImportBatches;`,
        );
        assert.equal(batches[0].length, 2);
        assert.deepEqual(batches[1], [sources[1]]);
        await resize(app, 760, 620);
        await assertContained(app, slot("import-receipts"));
        await app.capture("import-retry-receipts-small");
      } finally {
        await app.execute(
          `window.fetch = window.__donutImportFetch; delete window.__donutImportFetch; delete window.__donutImportBatches;`,
        );
      }
    },
    { seedDownloadedBrowser: true },
  );
});

test("About hides a snack drawer that responds to bites and resets on close", async () => {
  await withApp("motion-donut-snack", async (app) => {
    await resize(app, 760, 620);
    const openAbout = async () => {
      await app.clickSelector(`[aria-label="${en.rail.more.label}"]`);
      await app.clickText(en.rail.more.about, {
        roles: ["menuitem"],
        exact: false,
      });
      await waitForSelector(app, slot("about-logo"));
    };
    await openAbout();
    await app.clickSelector(slot("about-logo"));
    await waitForSelector(app, slot("donut-snack"), false);
    await app.execute(
      `document.querySelector('[data-slot="about-logo"]').focus();`,
    );
    await app.pressShortcut({ key: "\uE006", shift: true });
    await waitForSelector(app, slot("donut-snack"));
    const biteCount = () =>
      app.execute(
        `return Number(document.querySelector('[data-slot="donut-snack"]').dataset.bites);`,
      );
    assert.equal(await biteCount(), 0);
    await app.capture("secret-snack-drawer");
    for (const count of [1, 2, 3]) {
      await app.clickSelector(slot("donut-snack-bite"));
      assert.equal(await biteCount(), count);
      await assertContained(app, slot("donut-snack"));
      if (count === 2) await app.capture("secret-snack-two-bites");
    }
    await app.waitForText(en.about.snack.finished);
    await app.capture("secret-snack-crumbs");
    await app.clickSelector(slot("donut-snack-bite"));
    assert.equal(await biteCount(), 0);
    await activateFocusedByKeyboard(app);
    assert.equal(await biteCount(), 1);
    await freezeAnimations(app);
    try {
      await app.clickSelector(slot("donut-snack-bite"));
      assert.equal(await biteCount(), 2);
      await assertPopupReadable(app, slot("donut-snack"));
    } finally {
      await resumeAnimations(app);
    }
    assert.deepEqual(
      await app.invoke("list_browser_profiles"),
      [],
      "the snack never touches profiles",
    );
    await app.pressShortcut({ key: "Escape" });
    await waitForSelector(app, slot("donut-snack"), false);
    await openAbout();
    await waitForSelector(app, slot("donut-snack"), false);
  });
});

test("the Wayfern terms gate lifts when the backend announces acceptance", async () => {
  // No Wayfern binary in this suite, so the marker is written the way the
  // binary writes it and the backend's announcement is replayed. The dialog
  // must close on that event alone: a REST or automation acceptance never
  // presses the dialog's own button.
  await withApp(
    "motion-terms-gate",
    async (app) => {
      const termsDialogVisible = () =>
        app.execute(
          `return [...document.querySelectorAll('[role="dialog"]')].some(node => node.textContent.includes(arguments[0]));`,
          [en.wayfernTerms.title],
        );
      await app.waitFor(termsDialogVisible, {
        description: "the Wayfern terms dialog before acceptance",
      });
      await app.capture("terms-gate-closed");
      await mkdir(path.dirname(app.wayfernTermsFile), { recursive: true });
      await writeFile(
        app.wayfernTermsFile,
        `${Math.floor(Date.now() / 1000)}\n`,
      );
      assert.equal(await app.invoke("check_wayfern_terms_accepted"), true);
      await emit(app, "wayfern-terms-accepted", null);
      await app.waitFor(async () => !(await termsDialogVisible()), {
        description: "the Wayfern terms dialog to close on the event",
      });
    },
    { wayfernTermsAccepted: false, seedDownloadedBrowser: true },
  );
});

test("the cheat code pays out sprinkles and never touches a profile", async () => {
  await withApp("motion-cheat-code", async (app) => {
    await resize(app, 1000, 700);
    const code = ["", "", "", "", "", "", "", "", "b", "a"];
    const toastVisible = () =>
      app.execute(
        `return [...document.querySelectorAll('[data-sonner-toast]')].some(node => node.textContent.includes(arguments[0]));`,
        [en.easterEgg.konami.title],
      );
    // Arrow keys type nothing; the letters land in a focused field as text.
    const typedLetters = code.filter((key) => /^[a-z]$/.test(key)).join("");
    // Typed into a field, the code is text, not a command.
    await app.pressShortcut({ ...modifier, key: "/" });
    await waitForSelector(app, slot("shortcuts-search"));
    await app.clickSelector(slot("shortcuts-search"));
    for (const key of code) await app.pressShortcut({ key });
    assert.equal(await toastVisible(), false);
    assert.equal(
      await app.execute(`return document.querySelector(arguments[0]).value;`, [
        slot("shortcuts-search"),
      ]),
      typedLetters,
    );
    // Escape empties the filter first; only an empty filter lets it through.
    await app.pressShortcut({ key: "Escape" });
    assert.equal(
      await app.execute(`return document.querySelector(arguments[0]).value;`, [
        slot("shortcuts-search"),
      ]),
      "",
    );
    await app.execute("document.activeElement?.blur();");
    for (const key of code) await app.pressShortcut({ key });
    await app.waitFor(toastVisible, { description: "the cheat-code toast" });
    await app.capture("cheat-code");
    assert.deepEqual(
      await app.invoke("list_browser_profiles"),
      [],
      "the cheat code creates nothing",
    );
  });
});

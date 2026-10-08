import assert from "node:assert/strict";
import { mkdir, realpath, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import Color from "color";
import en from "../../src/i18n/locales/en.json" with { type: "json" };
import {
  curlSnippet,
  javascriptSnippet,
  LOCAL_API_EXAMPLES,
  maskToken,
  oneLine,
} from "../../src/lib/api-examples.ts";
import { getDerivedThemeColors, THEMES } from "../../src/lib/themes.ts";
import { withApp } from "../lib/app.mjs";
import {
  CRX_EXTENSION_NAME,
  CRX_EXTENSION_VERSION,
  extensionZipBase64,
  wireGuardFixture,
  writeUnpackedExtension,
} from "../lib/fixtures.mjs";

const THEME_VARIABLES = [
  "--background",
  "--foreground",
  "--card",
  "--card-foreground",
  "--popover",
  "--popover-foreground",
  "--primary",
  "--primary-foreground",
  "--secondary",
  "--secondary-foreground",
  "--muted",
  "--muted-foreground",
  "--accent",
  "--accent-foreground",
  "--destructive",
  "--destructive-foreground",
  "--success",
  "--success-foreground",
  "--warning",
  "--warning-foreground",
  "--border",
  "--chart-1",
  "--chart-2",
  "--chart-3",
  "--chart-4",
  "--chart-5",
];

const DRACULA_THEME = THEMES.find((theme) => theme.id === "dracula").colors;
const AYU_LIGHT_THEME = THEMES.find((theme) => theme.id === "ayu-light").colors;

async function tableRows(app, tableId) {
  return app.execute(
    `return [...document.querySelectorAll('[data-table-id="' + arguments[0] + '"] tr[data-table-row]')].map(row => row.dataset.tableRow);`,
    [tableId],
  );
}

/** Selection checkboxes, and the browser icons that stand in for them, that sit
 * more than half a pixel off the center of their cell. */
async function offCenterSelectionControls(app, tableSelector) {
  return app.execute(
    `return [...document.querySelectorAll(arguments[0] + ' :is(th, td) :is([role="checkbox"], [role="checkbox"] + [aria-hidden="true"] > svg)')]
       .map((node) => {
         const cell = node.closest("th, td").getBoundingClientRect();
         const rect = node.getBoundingClientRect();
         return {
           control: node.getAttribute("aria-label") ?? "icon",
           x: rect.left + rect.width / 2 - (cell.left + cell.width / 2),
           y: rect.top + rect.height / 2 - (cell.top + cell.height / 2),
         };
       })
       .filter(({ x, y }) => Math.abs(x) > 0.5 || Math.abs(y) > 0.5);`,
    [tableSelector],
  );
}

async function tableShiftClick(app, selector) {
  // tauri-wd 0.2.1 omits modifiers from pointer events. Send the click with
  // Shift through the native WebView; the real checkbox handler selects rows.
  await app.execute(
    `const target = document.querySelector(arguments[0]);
     target.focus();
     target.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }));`,
    [selector],
  );
}

async function openProfileCreation(app) {
  await app.clickText("New", { roles: ["button"] });
  await app.waitForText(en.createProfile.title);
  await app.waitFor(
    () =>
      app.execute(`return Boolean(document.querySelector('#profile-name'));`),
    { description: "profile name field" },
  );
}

async function submitProfileCreation(app, name) {
  await app.clickTextIn('[role="dialog"]', en.common.buttons.create, {
    roles: ["button"],
  });
  await app.waitFor(
    () => app.execute(`return !document.querySelector('#profile-name');`),
    { description: `created profile ${name}` },
  );
  return (await app.invoke("list_browser_profiles")).find(
    (profile) => profile.name === name,
  );
}

test("profile creation stays compact and saves basic, protected, and ephemeral profiles", async () => {
  await withApp(
    "ui-profile-creation",
    async (app) => {
      // The UI suite has no browser binary or live network routes. Record
      // the chosen route, then create locally with a fixed device. The real
      // backend still saves the options and encrypts the profile. The network
      // suite covers creation with a real browser and working proxy.
      await stubCommand(app, "is_geoip_database_available", true);
      await app.execute(`
        const originalFetch = window.fetch;
        window.__donutProfileRequests = [];
        window.fetch = function(input, init) {
          const url = typeof input === 'string' ? input : input?.url ?? '';
          if (url.endsWith('/create_browser_profile_new')) {
            const payload = JSON.parse(init.body);
            window.__donutProfileRequests.push(structuredClone(payload));
            payload.proxyId = null;
            payload.vpnId = null;
            payload.wayfernConfig = { ...payload.wayfernConfig, fingerprint: '{}' };
            return originalFetch.call(window, input, { ...init, body: JSON.stringify(payload) });
          }
          return originalFetch.apply(window, arguments);
        };
      `);
      await app.session.command("POST", "/window/rect", {
        width: 1280,
        height: 900,
      });
      await openProfileCreation(app);
      await app.fillSelector("#profile-name", "Simple profile");
      await app.waitFor(
        () =>
          app.execute(
            `return [...document.querySelectorAll('[role="dialog"] button')].some(button => button.textContent.trim() === arguments[0] && !button.disabled);`,
            [en.common.buttons.create],
          ),
        { description: "creation is available" },
      );
      const basic = await app.execute(`
      const dialog = document.querySelector('#profile-name').closest('[role="dialog"]');
      const rect = dialog.getBoundingClientRect();
      return {
        width: rect.width, height: rect.height,
        text: dialog.innerText,
        tabs: dialog.querySelectorAll('[role="tab"]').length,
        fields: [...dialog.querySelectorAll('input:not([type="hidden"])')].map(input => input.id),
        advancedVisible: Boolean(dialog.querySelector('#launch-hook-url')),
        checkboxes: [...dialog.querySelectorAll('[role="checkbox"]')].map(input => input.id),
      };
    `);
      assert.ok(
        basic.width <= 512 && basic.height < 450,
        JSON.stringify(basic),
      );
      assert.equal(basic.tabs, 0);
      assert.deepEqual(basic.fields, ["profile-name"]);
      assert.deepEqual(basic.checkboxes, ["enable-password", "ephemeral"]);
      assert.equal(basic.advancedVisible, false);
      assert.ok(!basic.text.includes(en.profiles.ephemeralDescription));
      assert.ok(
        !basic.text.includes(en.createProfile.passwordProtect.description),
      );
      const chrome = await app.execute(`
      const dialog = document.querySelector('#profile-name').closest('[role="dialog"]');
      const border = (node) => getComputedStyle(node).borderTopWidth;
      return {
        name: border(document.querySelector('#profile-name')),
        options: [...dialog.querySelectorAll('#profile-proxy, [role="checkbox"]')].map(border),
        proxyName: document.querySelector('#profile-proxy').getAttribute('aria-labelledby')
          .split(' ').map((id) => document.getElementById(id)?.textContent.trim()),
      };
    `);
      assert.equal(chrome.name, "0px", "the name field has no box");
      assert.deepEqual(chrome.options, ["0px", "0px", "0px"]);
      assert.deepEqual(chrome.proxyName, [
        en.createProfile.proxy.title,
        en.createProfile.proxy.noProxy,
      ]);
      await app.capture("profile-create-compact-wide");
      // Adding a route is part of the route list, and stays there when the
      // search finds nothing.
      await app.clickSelector("#profile-proxy");
      await app.fillSelector("[cmdk-input]", "no such route");
      await app.waitForText(en.createProfile.proxy.notFound);
      await app.clickText(en.createProfile.proxy.addProxy, {
        roles: ["option"],
      });
      await app.waitFor(
        () =>
          app.execute(`return Boolean(document.querySelector('#proxy-name'));`),
        { description: "proxy form from the route list" },
      );
      await app.pressShortcut({ key: "Escape" });
      await app.waitFor(
        () =>
          app.execute(
            `return !document.querySelector('#proxy-name') && document.querySelector('#profile-name')?.value === arguments[0];`,
            ["Simple profile"],
          ),
        { description: "creation dialog kept after the proxy form closes" },
      );
      await app.clickSelector("#profile-proxy");
      await app.clickText(en.common.labels.none, { roles: ["option"] });
      const simple = await submitProfileCreation(app, "Simple profile");
      assert.ok(simple);
      assert.equal(simple.ephemeral, false);
      assert.equal(simple.password_protected, false);
      assert.equal(simple.proxy_id, null);
      assert.equal(simple.vpn_id, null);
      assert.ok(simple.wayfern_config.screen_max_width > 0);
      assert.ok(simple.wayfern_config.screen_max_height > 0);

      const proxy = await app.invoke("create_stored_proxy", {
        name: "Creation proxy",
        proxySettings: {
          proxy_type: "http",
          host: "127.0.0.1",
          port: 9,
          username: null,
          password: null,
        },
      });
      const vpn = await app.invoke("create_vpn_config_manual", {
        name: "Creation VPN",
        vpnType: "WireGuard",
        configData: wireGuardFixture(),
      });
      await app.invoke("plugin:event|emit", {
        event: "vpn-configs-changed",
        payload: null,
      });
      const extensions = await app.invoke("create_extension_group", {
        name: "Creation extensions",
      });
      await openProfileCreation(app);
      await app.fillSelector("#profile-name", "Protected profile");
      await app.clickSelector("#profile-proxy");
      await app.fillSelector("[cmdk-input]", "Creation proxy");
      await app.clickText(proxy.name, { roles: ["option"] });
      await app.clickSelector("#enable-password");
      await app.fillSelector("#profile-password", "short");
      await app.clickTextIn('[role="dialog"]', en.common.buttons.create, {
        roles: ["button"],
      });
      await app.waitForText(
        en.profilePassword.errors.tooShort.replace("{{min}}", "8"),
      );
      await app.fillSelector("#profile-password", "creation test password");
      await app.fillSelector("#profile-password-confirm", "different password");
      await app.clickTextIn('[role="dialog"]', en.common.buttons.create, {
        roles: ["button"],
      });
      await app.waitForText(en.profilePassword.errors.mismatch);
      await app.fillSelector(
        "#profile-password-confirm",
        "creation test password",
      );
      await app.clickText(en.createProfile.advancedOptions, {
        roles: ["button"],
      });
      await app.fillSelector(
        "#launch-hook-url",
        "https://example.com/hooks/created",
      );
      await chooseSelectOption(
        app,
        "#profile-dns-blocklist",
        en.dnsBlocklist.normal,
      );
      await chooseSelectOption(
        app,
        "#profile-extension-group",
        `${extensions.name} (0)`,
      );
      await app.clickSelector("#restore-session");
      assert.equal(
        await app.execute(
          `return document.querySelector('#profile-name').closest('[role="dialog"]').querySelectorAll('[role="tab"]').length;`,
        ),
        0,
      );
      await app.execute(
        `document.querySelector('[data-slot="profile-create-fields"]').scrollTop = 0;`,
      );
      await app.capture("profile-create-advanced-wide");
      await app.clickText(en.createProfile.advancedOptions, {
        roles: ["button"],
      });
      assert.equal(
        await app.execute(
          `return document.querySelector('#profile-name').value;`,
        ),
        "Protected profile",
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('#profile-password').value;`,
        ),
        "creation test password",
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('#profile-proxy').textContent.trim();`,
        ),
        proxy.name,
      );
      await app.clickText(en.createProfile.advancedOptions, {
        roles: ["button"],
      });
      assert.equal(
        await app.execute(
          `return document.querySelector('#launch-hook-url').value;`,
        ),
        "https://example.com/hooks/created",
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('#restore-session').getAttribute('aria-checked');`,
        ),
        "false",
      );
      await app.clickText(en.createProfile.advancedOptions, {
        roles: ["button"],
      });
      await app.session.command("POST", "/window/rect", {
        width: 480,
        height: 700,
      });
      await app.capture("profile-create-password-narrow");
      const fit = await app.execute(`
      const dialog = document.querySelector('#profile-name').closest('[role="dialog"]');
      const rect = dialog.getBoundingClientRect();
      const footer = dialog.querySelector('[data-slot="dialog-footer"]').getBoundingClientRect();
      return rect.left >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight &&
        dialog.scrollWidth <= dialog.clientWidth && footer.bottom <= rect.bottom;
    `);
      assert.equal(fit, true);
      const protectedProfile = await submitProfileCreation(
        app,
        "Protected profile",
      );
      assert.ok(protectedProfile);
      assert.equal(protectedProfile.password_protected, true);
      assert.equal(protectedProfile.ephemeral, false);
      const protectedRequest = await app.execute(
        `return window.__donutProfileRequests.at(-1);`,
      );
      assert.equal(protectedRequest.proxyId, proxy.id);
      assert.equal(protectedRequest.vpnId, undefined);
      assert.equal(protectedProfile.dns_blocklist, "normal");
      assert.equal(
        protectedProfile.launch_hook,
        "https://example.com/hooks/created",
      );
      assert.equal(protectedProfile.extension_group_id, extensions.id);
      assert.equal(protectedProfile.wayfern_config.restore_session, false);
      await app.invoke("verify_profile_password", {
        profileId: protectedProfile.id,
        password: "creation test password",
      });

      await openProfileCreation(app);
      await app.fillSelector("#profile-name", "Ephemeral profile");
      assert.equal(
        await app.execute(
          `return document.querySelector('#enable-password').getAttribute('aria-checked');`,
        ),
        "false",
      );
      await app.capture("profile-create-compact-narrow");
      await app.clickText(en.createProfile.advancedOptions, {
        roles: ["button"],
      });
      assert.equal(
        await app.execute(
          `return document.querySelector('#launch-hook-url').value;`,
        ),
        "",
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('#profile-dns-blocklist').textContent.trim();`,
        ),
        en.dnsBlocklist.none,
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('#profile-extension-group').textContent.trim();`,
        ),
        en.profileInfo.values.none,
      );
      await app.capture("profile-create-advanced-narrow");
      await app.clickText(en.createProfile.advancedOptions, {
        roles: ["button"],
      });
      await app.clickSelector("#profile-proxy");
      await app.fillSelector("[cmdk-input]", "Creation VPN");
      await app.clickText(vpn.name, { exact: false, roles: ["option"] });
      await app.clickSelector("#enable-password");
      await app.fillSelector("#profile-password", "discarded password");
      await app.clickSelector("#ephemeral");
      assert.equal(
        await app.execute(
          `return document.querySelector('#enable-password').getAttribute('aria-checked');`,
        ),
        "false",
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('#profile-password') === null;`,
        ),
        true,
      );
      await app.clickSelector("#enable-password");
      assert.equal(
        await app.execute(
          `return document.querySelector('#ephemeral').getAttribute('aria-checked');`,
        ),
        "false",
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('#profile-password').value;`,
        ),
        "",
      );
      await app.clickSelector("#ephemeral");
      const ephemeral = await submitProfileCreation(app, "Ephemeral profile");
      assert.ok(ephemeral);
      assert.equal(ephemeral.ephemeral, true);
      assert.equal(ephemeral.password_protected, false);
      const ephemeralRequest = await app.execute(
        `return window.__donutProfileRequests.at(-1);`,
      );
      assert.equal(ephemeralRequest.proxyId, undefined);
      assert.equal(ephemeralRequest.vpnId, vpn.id);
      assert.equal(ephemeral.launch_hook, null);
    },
    { seedDownloadedBrowser: true },
  );
});

test("profile table filters, range selection, view settings, and keyboard edits work together", async () => {
  await withApp(
    "ui-table-controls",
    async (app) => {
      await app.session.command("POST", "/window/rect", {
        width: 1480,
        height: 900,
      });
      const profiles = [];
      for (let i = 0; i < 32; i++)
        profiles.push(
          await createUiProfile(app, `Table ${String(i + 1).padStart(2, "0")}`),
        );
      for (const profile of profiles.slice(0, 2))
        await app.invoke("update_profile_tags", {
          profileId: profile.id,
          tags: ["table-work"],
        });
      await app.waitFor(
        async () => (await tableRows(app, "profiles")).length >= 4,
        { description: "profile table rows" },
      );
      const chrome = await app.execute(
        `
        const root = document.querySelector('[data-table-id="profiles"]');
        const border = (node) => getComputedStyle(node).borderTopWidth;
        const filters = [...root.querySelectorAll('[data-slot="table-filters"] button')]
          .find((button) => button.textContent.trim() === arguments[0]);
        return {
          controls: [
            filters,
            root.querySelector('[data-slot="table-sort-trigger"]'),
            root.querySelector('[data-slot="table-view-trigger"]'),
          ].map(border),
          rowRules: [...root.querySelectorAll('tr[data-table-row] > td')]
            .filter((cell) => parseFloat(getComputedStyle(cell).borderBottomWidth) > 0).length,
        };
      `,
        [en.tables.filters],
      );
      assert.deepEqual(chrome.controls, ["0px", "0px", "0px"]);
      assert.equal(chrome.rowRules, 0, "rows are not divided by rules");
      assert.deepEqual(
        await offCenterSelectionControls(app, '[data-table-id="profiles"]'),
        [],
      );
      const row = (index) => `tr[data-table-row="${profiles[index].id}"]`;
      await app.clickSelector(`${row(0)} [role="checkbox"]`);
      await tableShiftClick(app, `${row(3)} [role="checkbox"]`);
      await app.waitFor(
        () =>
          app.execute(
            `return document.querySelectorAll('tr[data-table-row][aria-selected="true"]').length === 4;`,
          ),
        { description: "Shift-click selects four rows" },
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('[data-table-id="profiles"] thead [role="checkbox"]').getAttribute('data-state');`,
        ),
        "indeterminate",
      );
      assert.match(
        await app.execute(
          `return document.querySelector('[data-slot="table-action-bar"]').textContent;`,
        ),
        /4/,
      );
      const searchSelector = `input[placeholder="${en.header.searchPlaceholder}"]`;
      // Quoted: an unquoted "02" is its own term, which also prefix-matches
      // any profile whose random id starts with 02.
      await app.fillSelector(searchSelector, '"Table 02"');
      await app.waitFor(
        async () =>
          JSON.stringify(await tableRows(app, "profiles")) ===
          JSON.stringify([profiles[1].id]),
        { description: "profile text search" },
      );
      assert.equal(
        await app.execute(
          `return Boolean(document.querySelector('[data-slot="table-action-bar"]'));`,
        ),
        false,
        "search clears all action targets",
      );
      await app.clickSelector(`[aria-label="${en.header.clearSearch}"]`);
      await app.clickTextIn(
        '[data-table-id="profiles"] [data-slot="table-filters"]',
        en.tables.filters,
        { roles: ["button"] },
      );
      await app.clickSelector(
        `[cmdk-item][data-value="${en.profileTable.tagsHeader} table-work"]`,
      );
      await app.pressShortcut({ key: "Escape" });
      await app.waitFor(
        async () => (await tableRows(app, "profiles")).length === 2,
        { description: "visible tag filter" },
      );
      assert.match(
        await app.execute(
          `return document.querySelector('[data-slot="table-result-count"]').textContent;`,
        ),
        /2.*32/,
      );
      await app.clickTextIn(
        '[data-table-id="profiles"] [data-slot="table-filters"]',
        en.tables.clearFilters,
        { roles: ["button"] },
      );

      await app.clickSelector(
        '[data-table-id="profiles"] [data-slot="table-sort-trigger"]',
      );
      await app.clickText(en.tables.addSort, { roles: ["button"] });
      await app.clickSelector(
        `[data-sort-index="1"] button[aria-label="${en.tables.moveSortUp}"]`,
      );
      await app.pressShortcut({ key: "Escape" });
      await app.waitFor(
        async () =>
          (await app.invoke("get_table_preferences", { tableId: "profiles" }))
            .sorting[0].id === "created_at",
        { description: "ordered sort rules saved" },
      );
      assert.match(
        await app.execute(
          `return document.querySelector('[data-slot="table-sort-trigger"]').textContent;`,
        ),
        new RegExp(en.search.fields.created),
      );

      await app.clickSelector(
        '[data-table-id="profiles"] [data-slot="table-view-trigger"]',
      );
      await app.clickSelector("#table-visible-profiles-note");
      await app.clickSelector(`[aria-label="${en.tables.rowHeight}"]`);
      await app.clickText(en.tables.comfortable, { roles: ["option"] });
      await app.pressShortcut({ key: "Escape" });
      await app.waitFor(
        async () =>
          (await app.invoke("get_table_preferences", { tableId: "profiles" }))
            .density === "comfortable",
        { description: "comfortable rows saved" },
      );
      assert.equal(
        await app.execute(
          `return Boolean(document.querySelector('[data-table-column="note"]'));`,
        ),
        false,
      );
      assert.equal(
        await app.execute(
          `return Math.round(document.querySelector('tr[data-table-row]').getBoundingClientRect().height);`,
        ),
        48,
      );
      const resizeName = `[data-table-id="profiles"] [role="separator"][aria-label="${en.tables.resizeColumn.replace("{{column}}", en.common.labels.name)}"]`;
      const columnWidth = () =>
        app.execute(
          `return document.querySelector(arguments[0]).parentElement.getBoundingClientRect().width;`,
          [resizeName],
        );
      const initialWidth = await columnWidth();
      await app.execute(`document.querySelector(arguments[0]).focus();`, [
        resizeName,
      ]);
      await app.pressShortcut({ key: "\uE014" });
      await app.waitFor(
        async () =>
          (await app.invoke("get_table_preferences", { tableId: "profiles" }))
            .sizing.name > 48,
        { description: "keyboard column resize saved" },
      );
      const saved = await app.invoke("get_table_preferences", {
        tableId: "profiles",
      });
      assert.ok(Math.abs((await columnWidth()) - initialWidth - 16) < 2);
      assert.ok(Math.abs((await columnWidth()) - saved.sizing.name) < 2);
      await app.capture("table-controls-wide");
      await app.restart();
      await app.waitFor(
        () =>
          app.execute(
            `return document.querySelector('tr[data-table-row]')?.getBoundingClientRect().height === 48;`,
          ),
        { description: "saved view restored after restart" },
      );
      assert.deepEqual(
        await app.invoke("get_table_preferences", { tableId: "profiles" }),
        saved,
      );
      assert.ok(Math.abs((await columnWidth()) - saved.sizing.name) < 2);

      await app.clickSelector(
        '[data-table-id="profiles"] [data-slot="table-view-trigger"]',
      );
      await app.clickText(en.tables.resetView, { roles: ["button"] });
      await app.pressShortcut({ key: "Escape" });
      await app.waitFor(
        () =>
          app.execute(
            `return document.querySelector('tr[data-table-row]')?.getBoundingClientRect().height === 36;`,
          ),
        { description: "default view restored" },
      );
      await app.session.command("POST", "/window/rect", {
        width: 1280,
        height: 500,
      });
      await app.waitFor(
        () =>
          app.execute(`return !document.querySelector(arguments[0]);`, [
            row(24),
          ]),
        { description: "keyboard target starts outside the rendered rows" },
      );
      await app.execute(`document.querySelector(arguments[0]).focus();`, [
        `${row(0)} [data-table-column="name"]`,
      ]);
      for (let i = 0; i < 24; i++) {
        await app.pressShortcut({ key: "\uE015" });
        await app.waitFor(
          () =>
            app.execute(
              `return document.activeElement?.closest('tr')?.dataset.tableRow === arguments[0];`,
              [profiles[i + 1].id],
            ),
          { description: `keyboard row ${i + 2}` },
        );
      }
      await app.waitFor(
        () =>
          app.execute(
            `return document.activeElement?.closest('tr')?.dataset.tableRow === arguments[0];`,
            [profiles[24].id],
          ),
        { description: "keyboard crosses the virtual row boundary" },
      );
      await app.pressShortcut({ key: "\uE032" });
      await app.fillSelector(`${row(24)} input`, "Table renamed by keyboard");
      await app.pressShortcut({ key: "\uE006" });
      await app.waitFor(
        async () =>
          (await app.invoke("list_browser_profiles")).find(
            (profile) => profile.id === profiles[24].id,
          )?.name === "Table renamed by keyboard",
        { description: "F2 edit saved through the backend" },
      );
      await app.session.command("POST", "/window/rect", {
        width: 640,
        height: 600,
      });
      await app.capture("table-controls-narrow");
      assert.equal(
        await app.execute(
          `return document.documentElement.scrollWidth <= innerWidth;`,
        ),
        true,
        "controls do not widen the window",
      );
      await app.clickSelector(
        '[data-table-id="profiles"] thead [role="checkbox"]',
      );
      await app.waitFor(
        () =>
          app.execute(
            `
            const scroll = document.querySelector('[data-table-id="profiles"] .scroll-fade');
            scroll.scrollTop = scroll.scrollHeight;
            const rows = [...scroll.querySelectorAll('tr[data-table-row]')];
            const bar = document.querySelector('[data-slot="table-action-bar"]');
            return rows.at(-1)?.dataset.tableRow === arguments[0] &&
              rows.at(-1).getBoundingClientRect().bottom <= bar?.getBoundingClientRect().top - 4;
          `,
            [profiles[24].id],
          ),
        { description: "last row clears the wrapped bulk action bar" },
      );
      await app.capture("table-controls-narrow-selection");
    },
    { seedDownloadedBrowser: true },
  );
});

test("management tables share search, filters, selection, and saved views", async () => {
  await withApp("ui-management-tables", async (app) => {
    const proxies = [];
    for (const [name, protocol] of [
      ["Table proxy Alpha", "http"],
      ["Table proxy Beta", "socks5"],
      ["Table proxy Gamma", "http"],
    ])
      proxies.push(
        await app.invoke("create_stored_proxy", {
          name,
          proxySettings: {
            proxy_type: protocol,
            host: "127.0.0.1",
            port: 9,
            username: null,
            password: null,
          },
        }),
      );
    await app.invoke("create_profile_group", { name: "Table group Alpha" });
    await app.invoke("create_profile_group", { name: "Table group Beta" });
    await app.pressShortcut({
      key: "n",
      ...(process.platform === "darwin" ? { meta: true } : { ctrl: true }),
    });
    await app.waitFor(
      async () => (await tableRows(app, "proxies")).length === 3,
      { description: "proxy table" },
    );
    assert.deepEqual(
      await offCenterSelectionControls(app, '[data-table-id="proxies"]'),
      [],
    );
    const proxyRowHeight = () =>
      app.execute(
        `return document.querySelector('[data-table-id="proxies"] tr[data-table-row]')?.getBoundingClientRect().height;`,
      );
    await app.clickSelector(
      `[data-table-id="proxies"] tr[data-table-row="${proxies[0].id}"] [role="checkbox"]`,
    );
    await tableShiftClick(
      app,
      `[data-table-id="proxies"] tr[data-table-row="${proxies[2].id}"] [role="checkbox"]`,
    );
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelectorAll('[data-table-id="proxies"] tr[aria-selected="true"]').length === 3;`,
        ),
      { description: "management range selection" },
    );
    await app.fillSelector(
      `[data-table-id="proxies"] input[aria-label="${en.tables.search}"]`,
      "Beta",
    );
    await app.waitFor(
      async () => (await tableRows(app, "proxies")).length === 1,
      { description: "proxy search" },
    );
    assert.equal(
      await app.execute(
        `return Boolean(document.querySelector('[data-slot="table-action-bar"]'));`,
      ),
      false,
    );
    await app.fillSelector(
      `[data-table-id="proxies"] input[aria-label="${en.tables.search}"]`,
      "no such row",
    );
    await app.waitForText(en.tables.noResults);
    await app.clickTextIn(
      '[data-table-id="proxies"] tbody',
      en.tables.clearFilters,
      { roles: ["button"] },
    );
    assert.equal((await tableRows(app, "proxies")).length, 3);
    await app.clickTextIn(
      '[data-table-id="proxies"] [data-slot="table-filters"]',
      en.tables.filters,
      { roles: ["button"] },
    );
    await app.clickSelector(
      `[cmdk-item][data-value="${en.proxies.management.protocolCol} HTTP"]`,
    );
    await app.pressShortcut({ key: "Escape" });
    await app.waitFor(
      async () => (await tableRows(app, "proxies")).length === 2,
      { description: "proxy protocol facet" },
    );
    await app.clickSelector(
      '[data-table-id="proxies"] [data-slot="table-view-trigger"]',
    );
    await app.clickSelector("#table-visible-proxies-hostPort");
    await app.pressShortcut({ key: "Escape" });
    await app.waitFor(
      async () =>
        (await app.invoke("get_table_preferences", { tableId: "proxies" }))
          .visibility.hostPort === false,
      { description: "management columns saved" },
    );
    assert.equal(await proxyRowHeight(), 36);
    await app.clickSelector(
      '[data-table-id="proxies"] [data-slot="table-view-trigger"]',
    );
    await app.clickSelector(`[aria-label="${en.tables.rowHeight}"]`);
    await app.clickText(en.tables.comfortable, { roles: ["option"] });
    await app.pressShortcut({ key: "Escape" });
    await app.waitFor(async () => (await proxyRowHeight()) === 48, {
      description: "management row height changes",
    });
    await app.capture("management-table-controls");
    await app.clickSelector(`[aria-label="${en.rail.groups}"]`);
    await app.waitFor(
      async () => (await tableRows(app, "groups")).length === 2,
      { description: "group table" },
    );
    await app.fillSelector(
      `[data-table-id="groups"] input[aria-label="${en.tables.search}"]`,
      "Beta",
    );
    await app.waitFor(
      async () => (await tableRows(app, "groups")).length === 1,
      { description: "group search" },
    );
    await app.invoke("create_vpn_config_manual", {
      name: "Table VPN Alpha",
      vpnType: "WireGuard",
      configData: wireGuardFixture(),
    });
    await app.invoke("create_vpn_config_manual", {
      name: "Table VPN Beta",
      vpnType: "WireGuard",
      configData: wireGuardFixture(),
    });
    await app.clickSelector(`[aria-label="${en.rail.network}"]`);
    await app.clickText(en.proxies.management.tabVpns, {
      roles: ["tab"],
      exact: false,
    });
    await app.waitFor(async () => (await tableRows(app, "vpns")).length === 2, {
      description: "VPN table",
    });
    await app.fillSelector(
      `[data-table-id="vpns"] input[aria-label="${en.tables.search}"]`,
      "Beta",
    );
    await app.waitFor(async () => (await tableRows(app, "vpns")).length === 1, {
      description: "VPN search",
    });
    await app.invoke("add_extension", {
      name: "Table extension",
      fileName: "fixture.zip",
      fileData: [...Buffer.from(extensionZipBase64(), "base64")],
    });
    await app.invoke("create_extension_group", {
      name: "Table extension group Alpha",
    });
    await app.invoke("create_extension_group", {
      name: "Table extension group Beta",
    });
    await app.clickSelector(`[aria-label="${en.rail.extensions}"]`);
    await app.waitFor(
      async () => (await tableRows(app, "extensions")).length === 1,
      { description: "extension table" },
    );
    await app.fillSelector(
      `[data-table-id="extensions"] input[aria-label="${en.tables.search}"]`,
      "no such extension",
    );
    await app.waitForText(en.tables.noResults);
    await app.clickTextIn(
      '[data-table-id="extensions"] tbody',
      en.tables.clearFilters,
      { roles: ["button"] },
    );
    await app.clickText(en.extensions.groupsTab, {
      roles: ["tab"],
      exact: false,
    });
    await app.waitFor(
      async () => (await tableRows(app, "extensionGroups")).length === 2,
      { description: "extension group table" },
    );
    await app.fillSelector(
      `[data-table-id="extensionGroups"] input[aria-label="${en.tables.search}"]`,
      "Beta",
    );
    await app.waitFor(
      async () => (await tableRows(app, "extensionGroups")).length === 1,
      { description: "extension group search" },
    );
  });
});

async function dismissSurface(app) {
  await app.pressShortcut({ key: "Escape" });
  await new Promise((resolve) => setTimeout(resolve, 100));
}

async function themeSnapshot(app) {
  return app.execute(
    `
      const root = document.documentElement;
      const rootStyle = getComputedStyle(root);
      const bodyStyle = getComputedStyle(document.body);
      const variables = arguments[0];
      return {
        mode: root.classList.contains("light")
          ? "light"
          : root.classList.contains("dark")
            ? "dark"
            : "unset",
        inline: Object.fromEntries(
          variables.map((key) => [key, root.style.getPropertyValue(key).trim()])
        ),
        resolved: Object.fromEntries(
          variables.map((key) => [key, rootStyle.getPropertyValue(key).trim()])
        ),
        bodyBackground: bodyStyle.backgroundColor,
        bodyForeground: bodyStyle.color,
      };
    `,
    [THEME_VARIABLES],
  );
}

async function waitForTheme(app, predicate, description) {
  return app.waitFor(
    async () => {
      const snapshot = await themeSnapshot(app);
      return predicate(snapshot) ? snapshot : false;
    },
    { description },
  );
}

function themeVariablesEqual(actual, expected) {
  return THEME_VARIABLES.every(
    (key) => actual[key]?.toLowerCase() === expected[key]?.toLowerCase(),
  );
}

async function applyThemeForContrastAudit(app, theme) {
  await app.execute(
    `
      // This audit reads settled colour tokens, not the animation between
      // them. Tab triggers carry "transition-colors duration-150" and start
      // from --muted-foreground, so a computed style sampled mid-transition
      // returns an intermediate colour and the assertion fails on whichever
      // theme the machine happened to be slow on. Kill transitions for the
      // duration of the audit rather than racing them with a fixed sleep.
      let freeze = document.getElementById("donut-e2e-freeze-transitions");
      if (!freeze) {
        freeze = document.createElement("style");
        freeze.id = "donut-e2e-freeze-transitions";
        freeze.textContent =
          "*, *::before, *::after { transition: none !important; animation: none !important; }";
        document.head.appendChild(freeze);
      }

      const [colors, derived, mode] = arguments;
      const root = document.documentElement;
      root.classList.remove("light", "dark");
      root.classList.add(mode);
      for (const [key, value] of Object.entries({ ...colors, ...derived })) {
        root.style.setProperty(key, value, "important");
      }
    `,
    [theme.colors, getDerivedThemeColors(theme.colors), theme.mode],
  );
  // One frame is enough once transitions are off; the value cannot drift after
  // style recalculation.
  await app.execute(
    `return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))));`,
  );
}

async function animatedTabContrastSnapshot(app) {
  return app.execute(`
    const rootStyle = getComputedStyle(document.documentElement);
    const triggers = [
      ...document.querySelectorAll('[data-slot="animated-tabs-trigger"]'),
    ];
    const active = triggers.find(
      (trigger) => trigger.getAttribute("data-state") === "active",
    );
    const inactive = triggers.find(
      (trigger) => trigger.getAttribute("data-state") === "inactive",
    );
    const content = (trigger) =>
      [...(trigger?.children ?? [])].filter(
        (child) =>
          child.getAttribute("data-slot") !== "animated-tabs-indicator",
      );
    const activeContent = content(active);
    const inactiveContent = content(inactive);
    const indicator = active?.querySelector(
      '[data-slot="animated-tabs-indicator"]',
    );
    return {
      mode: document.documentElement.classList.contains("light")
        ? "light"
        : "dark",
      background: rootStyle.getPropertyValue("--background").trim(),
      accent: rootStyle.getPropertyValue("--accent").trim(),
      accentForeground: rootStyle
        .getPropertyValue("--accent-foreground")
        .trim(),
      mutedForeground: rootStyle
        .getPropertyValue("--muted-foreground")
        .trim(),
      activeTitle: activeContent[0]
        ? getComputedStyle(activeContent[0]).color
        : null,
      activeCount: activeContent[1]
        ? getComputedStyle(activeContent[1]).color
        : null,
      activeBackground: indicator
        ? getComputedStyle(indicator).backgroundColor
        : null,
      inactiveTitle: inactiveContent[0]
        ? getComputedStyle(inactiveContent[0]).color
        : null,
      inactiveCount: inactiveContent[1]
        ? getComputedStyle(inactiveContent[1]).color
        : null,
    };
  `);
}

function assertColorEquals(actual, expected, description) {
  assert.ok(actual, `${description} is missing`);
  assert.equal(Color(actual).hex(), Color(expected).hex(), description);
}

function assertContrast(foreground, background, minimum, description) {
  assert.ok(foreground, `${description} foreground is missing`);
  assert.ok(background, `${description} background is missing`);
  const ratio = Color(foreground).contrast(Color(background));
  assert.ok(
    ratio >= minimum,
    `${description} is ${ratio.toFixed(2)}:1; expected at least ${minimum}:1`,
  );
}

async function chooseSelectOption(app, triggerSelector, option) {
  await app.waitFor(
    () =>
      app.execute(
        `const trigger = document.querySelector(arguments[0]);
         if (!trigger || trigger.matches(":disabled")) return false;
         trigger.focus();
         return document.activeElement === trigger;`,
        [triggerSelector],
      ),
    { description: `enabled select ${triggerSelector}` },
  );
  await app.pressShortcut({ key: "\uE007" });
  await app.waitFor(
    () =>
      app.execute(
        `const option = [...document.querySelectorAll('[role="option"]')].find(node => node.textContent.trim() === arguments[0]);
         option?.focus();
         return option && document.activeElement === option;`,
        [option],
      ),
    { description: `select option ${option}` },
  );
  await app.pressShortcut({ key: "\uE007" });
  await app.waitFor(
    () =>
      app.execute(
        `return document.querySelector(arguments[0])?.textContent.includes(arguments[1]);`,
        [triggerSelector, option],
      ),
    { description: `selected value ${option}` },
  );
}

async function saveSettings(app) {
  await app.clickText("Save Settings", { roles: ["button"] });
  await app.waitFor(
    () =>
      app.execute(
        `return document.querySelector('[data-slot="settings-feedback"]')?.textContent.includes("Saved");`,
      ),
    { description: "Settings to confirm the saved values" },
  );
}

async function assertThemeAcrossNavigation(app, expected) {
  for (const surface of ["Network", "Extensions", "Profiles"]) {
    await app.clickSelector(`[aria-label="${surface}"]`);
    await app.waitFor(
      async () =>
        JSON.stringify(await themeSnapshot(app)) === JSON.stringify(expected),
      { description: `theme to remain unchanged on ${surface}` },
    );
  }
}

async function dragBackgroundColorPicker(app) {
  await app.clickSelector('[aria-label="Background"]');
  const drag = await app.waitFor(
    () =>
      app.execute(`
        const popover = document.querySelector('[data-slot="popover-content"]');
        const selection = [...(popover?.querySelectorAll("div") ?? [])].find(
          (node) => node.style.background.includes("linear-gradient")
        );
        if (!selection) return null;
        const rect = selection.getBoundingClientRect();
        const points = [];
        for (const yf of [0.2, 0.4, 0.6, 0.8]) {
          for (const xf of [0.2, 0.4, 0.6, 0.8]) {
            const point = {
              x: Math.round(rect.left + rect.width * xf),
              y: Math.round(rect.top + rect.height * yf),
            };
            const hit = document.elementFromPoint(point.x, point.y);
            if (hit === selection || selection.contains(hit)) points.push(point);
          }
        }
        return points.length >= 2
          ? { start: points[0], end: points[points.length - 1] }
          : null;
      `),
    { description: "two pointer-interactive background color picker points" },
  );
  await app.execute(`
    window.__donutE2eThemePointerEvents = [];
    for (const type of ["pointermove", "pointerdown", "pointerup"]) {
      window.addEventListener(type, (event) => {
        window.__donutE2eThemePointerEvents.push({
          type,
          x: event.clientX,
          y: event.clientY,
          buttons: event.buttons,
          target: event.target?.className ?? event.target?.tagName ?? "",
        });
      }, true);
    }
  `);
  await app.session.command("POST", "/actions", {
    actions: [
      {
        type: "pointer",
        id: "theme-color-pointer",
        actions: [
          {
            type: "pointerMove",
            x: drag.start.x,
            y: drag.start.y,
            origin: "viewport",
          },
          { type: "pointerDown", button: 0 },
          { type: "pause", duration: 150 },
          {
            type: "pointerMove",
            x: drag.end.x,
            y: drag.end.y,
            duration: 100,
            origin: "viewport",
          },
          { type: "pointerUp", button: 0 },
        ],
      },
    ],
  });
  const pointerEvents = await app.execute(
    `return window.__donutE2eThemePointerEvents ?? [];`,
  );
  assert.equal(pointerEvents[0]?.type, "pointermove");
  assert.equal(pointerEvents[1]?.type, "pointerdown");
  assert.equal(pointerEvents.at(-1)?.type, "pointerup");
  const dragMoves = pointerEvents.slice(2, -1);
  assert.ok(dragMoves.length >= 1);
  assert.ok(dragMoves.every((event) => event.type === "pointermove"));
  assert.match(pointerEvents[1].target, /cursor-pointer/);
  assert.match(dragMoves.at(-1).target, /cursor-pointer/);
  assert.equal(dragMoves.at(-1).buttons, 1);
  await app.waitFor(
    () =>
      app.execute(
        `return document.querySelector("#theme-preset-select")?.textContent?.includes("Your Own") === true;`,
      ),
    {
      description: `customized theme to be marked as Your Own after ${JSON.stringify(pointerEvents)}`,
    },
  );
  await app.clickSelector('[aria-label="Background"]');
  await app.waitFor(
    () =>
      app.execute(
        `return document.querySelector('[data-slot="popover-content"]') === null;`,
      ),
    { description: "color picker to close" },
  );
}

async function paintedColors(app, probes) {
  return app.execute(
    `
      // Computed tints come back as color-mix() results with alpha. Paint
      // them over the page on a 1px canvas to read the colour a person sees.
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 1;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      const sentinel = "#010203";
      const flatten = (layers) => {
        context.clearRect(0, 0, 1, 1);
        for (const layer of layers) {
          context.fillStyle = sentinel;
          context.fillStyle = layer;
          if (context.fillStyle === sentinel) return null;
          context.fillRect(0, 0, 1, 1);
        }
        const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
        return "rgb(" + r + ", " + g + ", " + b + ")";
      };
      const page = getComputedStyle(document.body).backgroundColor;
      const result = { page: flatten([page]) };
      for (const [name, [selector, text]] of Object.entries(arguments[0])) {
        const node = [...document.querySelectorAll(selector)].find(
          (candidate) => !text || candidate.textContent.trim() === text,
        );
        if (!node) {
          result[name] = null;
          continue;
        }
        const style = getComputedStyle(node);
        result[name] = {
          fill: flatten([page, style.backgroundColor]),
          text: flatten([page, style.backgroundColor, style.color]),
        };
      }
      return result;
    `,
    [probes],
  );
}

function assertQuietTint(painted, page, maximum, description) {
  assert.ok(painted, `${description} is missing`);
  const ratio = Color(painted.fill).contrast(Color(page));
  assert.ok(
    ratio >= 1.03 && ratio <= maximum,
    `${description} tint is ${ratio.toFixed(2)}:1 against the page; expected 1.03 to ${maximum}`,
  );
  assertContrast(painted.text, painted.fill, 4.5, `${description} text`);
}

test("soft table and creation surfaces stay quiet and readable in every theme", async () => {
  await withApp(
    "ui-soft-surfaces",
    async (app) => {
      await app.session.command("POST", "/window/rect", {
        width: 1280,
        height: 860,
      });
      const tagged = await createUiProfile(app, "Theme Alpha");
      await createUiProfile(app, "Theme Beta");
      await app.invoke("update_profile_tags", {
        profileId: tagged.id,
        tags: ["theme-tag"],
      });
      await app.waitFor(
        async () => (await tableRows(app, "profiles")).length === 2,
        { description: "theme table rows" },
      );
      await app.clickSelector(
        `tr[data-table-row="${tagged.id}"] [role="checkbox"]`,
      );
      const table = '[data-table-id="profiles"]';
      const captured = new Set(["dracula", "tokyo-night", "ayu-light"]);
      for (const theme of THEMES) {
        await applyThemeForContrastAudit(app, theme);
        const colors = await paintedColors(app, {
          header: [`${table} thead th`, en.profileTable.tagsHeader],
          selected: [
            `${table} tr[data-state="selected"] > td[data-table-column="name"]`,
          ],
          button: [`${table} [data-slot="table-view-trigger"]`],
          tag: [`${table} tbody [data-slot="badge"]`, "theme-tag"],
        });
        assert.ok(colors.page, `${theme.id} page colour`);
        assertQuietTint(colors.header, colors.page, 1.35, `${theme.id} header`);
        assertQuietTint(
          colors.selected,
          colors.page,
          1.6,
          `${theme.id} selected row`,
        );
        assertQuietTint(colors.button, colors.page, 1.35, `${theme.id} button`);
        assertQuietTint(colors.tag, colors.page, 1.4, `${theme.id} tag`);
        if (captured.has(theme.id)) await app.capture(`table-${theme.id}`);
      }

      await openProfileCreation(app);
      await app.fillSelector("#profile-name", "Theme profile");
      await app.clickSelector("#enable-password");
      await app.fillSelector("#profile-password", "theme password");
      for (const theme of THEMES) {
        await applyThemeForContrastAudit(app, theme);
        const colors = await paintedColors(app, {
          off: ["#ephemeral"],
          on: ["#enable-password"],
          name: ['label[for="profile-name"]'],
          password: ["#profile-password"],
        });
        assertQuietTint(colors.off, colors.page, 1.35, `${theme.id} option`);
        assertQuietTint(
          colors.on,
          colors.page,
          1.6,
          `${theme.id} chosen option`,
        );
        assertQuietTint(colors.name, colors.page, 1.35, `${theme.id} name`);
        assertQuietTint(
          colors.password,
          colors.page,
          1.35,
          `${theme.id} password field`,
        );
        if (captured.has(theme.id)) await app.capture(`creation-${theme.id}`);
      }
    },
    { seedDownloadedBrowser: true },
  );
});
test("the integrations page ships only the Local API tab and never names remote control for a regular desktop", async () => {
  await withApp("ui-integrations", async (app) => {
    const modifier =
      process.platform === "darwin" ? { meta: true } : { ctrl: true };
    await app.clickSelector('[aria-label="Integrations"]');
    await app.waitForText(en.integrations.tabApi);

    // Local MCP is gone, and remote control is an Enterprise feature that a
    // desktop without the entitlement is never told about. This session is
    // signed out, so a second tab here means the gate opened for everyone.
    const tabs = async () =>
      app.execute(
        `return [...document.querySelectorAll('[role="tab"]')].map((node) => node.textContent.trim());`,
      );
    assert.deepEqual(await tabs(), [en.integrations.tabApi]);
    // The whole document, not just the painted text: a hidden trigger or an
    // unmounted-looking panel is still a mention.
    const html = await app.html();
    for (const phrase of [
      en.integrations.tabRemote,
      en.integrations.remote.enableLabel,
      en.integrations.remote.signInRequired,
      en.integrations.remote.endpointLabel,
    ]) {
      assert.ok(
        !html.includes(phrase),
        `remote control must stay out of sight: ${JSON.stringify(phrase)}`,
      );
    }
    assert.equal(
      (await app.invoke("get_mcp_remote_status")).enabled,
      false,
      "opening the page must not open the bridge",
    );

    // Mod+I flips to Remote MCP only for an entitled account, so here it
    // never lands on a tab that is not offered.
    const activeTab = async () =>
      app.execute(
        `return document.querySelector('[role="tab"][data-state="active"]')?.textContent.trim() ?? null;`,
      );
    assert.equal(await activeTab(), en.integrations.tabApi);
    await app.pressShortcut({ key: "i", ...modifier });
    await new Promise((resolve) => setTimeout(resolve, 500));
    assert.equal(await activeTab(), en.integrations.tabApi);
    assert.deepEqual(await tabs(), [en.integrations.tabApi]);

    await dismissSurface(app);
  });
});

test("the local API page shows a one-line example and opens runnable examples in a dialog", async () => {
  await withApp("ui-integrations-examples", async (app) => {
    const settings = await app.invoke("get_app_settings");
    const saved = await app.invoke("save_app_settings", {
      settings: {
        ...settings,
        api_enabled: true,
        api_port: 0,
        api_token: null,
      },
    });
    const token = saved.api_token;
    assert.ok(token?.length >= 32);
    const port = await app.invoke("start_api_server", { port: 0 });
    const baseUrl = `http://127.0.0.1:${port}`;

    await app.clickSelector('[aria-label="Integrations"]');
    // The section label is set in capitals, so innerText reads it that way.
    await app.waitForText(en.integrations.apiExampleRequest.toUpperCase());
    await app.execute(
      `window.__copied = [];
       navigator.clipboard.writeText = async (text) => { window.__copied.push(text); };`,
    );
    const copied = () => app.execute("return window.__copied;");

    // The page shows only the ends of the token, and the copy carries all of
    // it, so a pasted command runs as is.
    const quickStart = '[data-testid="integrations-api-example"]';
    const shown = await app.execute(
      "return document.querySelector(arguments[0]).innerText;",
      [quickStart],
    );
    assert.ok(!shown.includes(token), "the full token is never on screen");
    assert.ok(shown.includes(maskToken(token)));
    assert.ok(shown.includes(`${baseUrl}/v1/profiles`));
    await app.clickSelector(
      `[data-slot="code-snippet"]:has(${quickStart}) button`,
    );
    await app.waitFor(async () => (await copied()).length === 1, {
      description: "the quick start was copied",
    });
    assert.equal(
      (await copied())[0],
      oneLine(curlSnippet(LOCAL_API_EXAMPLES[0].request, { baseUrl, token })),
    );
    await app.capture("integrations-local-api");

    await app.clickText(en.integrations.examples.open);
    await app.waitForText(en.integrations.examples.apiTitle);
    for (const example of LOCAL_API_EXAMPLES) {
      await app.clickSelector(`[data-testid="code-example-${example.id}"]`);
      await app.waitFor(
        async () =>
          (
            (await dialogText(app, en.integrations.examples.apiTitle)) ?? ""
          ).includes(en.integrations.examples.items[example.id].description),
        { description: `${example.id} example` },
      );
    }

    // The JavaScript the dialog hands over is real code against the real
    // server: run the copy and read the profiles it prints.
    await app.clickSelector('[data-testid="code-example-listProfiles"]');
    await app.clickText(en.integrations.examples.languages.javascript, {
      roles: ["tab"],
    });
    await app.clickSelector(
      '[data-testid="code-examples-dialog"] [data-slot="code-snippet"] button',
    );
    await app.waitFor(async () => (await copied()).length === 2, {
      description: "the JavaScript example was copied",
    });
    const javascript = (await copied())[1];
    assert.equal(
      javascript,
      javascriptSnippet(LOCAL_API_EXAMPLES[0].request, { baseUrl, token }),
    );
    const printed = [];
    await new (async () => {}).constructor("fetch", "console", javascript)(
      fetch,
      { log: (value) => printed.push(value) },
    );
    assert.equal(printed.length, 1);
    assert.ok(Array.isArray(printed[0].profiles), JSON.stringify(printed[0]));

    // The language is the reader's choice and stays as they move on.
    await app.clickSelector('[data-testid="code-example-createProxy"]');
    await app.waitFor(
      async () =>
        (
          (await dialogText(app, en.integrations.examples.apiTitle)) ?? ""
        ).includes("JSON.stringify({"),
      { description: "JavaScript stays selected" },
    );
    await app.capture("integrations-local-api-examples");
    await dismissSurface(app);
    await dismissSurface(app);

    await app.clickSelector(`[aria-label="${en.rail.account}"]`);
    await app.waitForText(en.account.signedOutDescription);
    await app.capture("account-signed-out");
    await dismissSurface(app);
  });
});

test("all primary navigation buttons and sub-page tabs render and remain interactive", async () => {
  await withApp("ui-navigation", async (app) => {
    const surfaces = [
      ["Settings", /General|Appearance|Sync/i],
      ["Network", /Proxies|VPNs|DNS/i],
      ["Extensions", /Extensions|Groups/i],
      ["Integrations", /API|MCP/i],
      ["Account", /Account|Sign in/i],
    ];
    for (const [label, expected] of surfaces) {
      await app.clickSelector(`[aria-label="${label}"]`);
      await app.waitFor(async () => expected.test(await app.bodyText()), {
        description: `${label} surface`,
      });
      assert.match(await app.bodyText(), expected);
      await dismissSurface(app);
    }

    await app.clickSelector('[aria-label="Groups"]');
    await app.waitForText("Create");
    await dismissSurface(app);

    await app.clickSelector('[aria-label="More"]');
    await app.waitFor(
      () =>
        app.execute(`return Boolean(document.querySelector("[role='menu']"));`),
      { description: "More menu" },
    );
    await dismissSurface(app);

    await app.clickSelector('[aria-label="Profiles"]');
    await app.clickText("New");
    await app.waitFor(
      () =>
        app.execute(
          `return Boolean(document.querySelector("[role='dialog']"));`,
        ),
      { description: "new profile dialog" },
    );
    assert.match(await app.bodyText(), /profile/i);
    await dismissSurface(app);
  });
});

test("every custom theme keeps tabs, counts, group pills, and rail states readable", async () => {
  await withApp("ui-theme-contrast", async (app) => {
    await app.clickSelector('[aria-label="Extensions"]');
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelectorAll('[data-slot="animated-tabs-trigger"]').length === 2;`,
        ),
      { description: "Extension tabs" },
    );

    for (const theme of THEMES) {
      await applyThemeForContrastAudit(app, theme);
      const snapshot = await animatedTabContrastSnapshot(app);
      assert.equal(snapshot.mode, theme.mode, `${theme.id} appearance mode`);
      assertColorEquals(
        snapshot.activeBackground,
        snapshot.accent,
        `${theme.id} active tab background`,
      );
      for (const [label, color] of [
        ["active tab title", snapshot.activeTitle],
        ["active tab count", snapshot.activeCount],
      ]) {
        assertColorEquals(
          color,
          snapshot.accentForeground,
          `${theme.id} ${label} token`,
        );
        assertContrast(
          color,
          snapshot.activeBackground,
          4.5,
          `${theme.id} ${label}`,
        );
      }
      for (const [label, color] of [
        ["inactive tab title", snapshot.inactiveTitle],
        ["inactive tab count", snapshot.inactiveCount],
      ]) {
        assertColorEquals(
          color,
          snapshot.mutedForeground,
          `${theme.id} ${label} token`,
        );
        assertContrast(color, snapshot.background, 4.5, `${theme.id} ${label}`);
      }

      await app.clickSelector(
        '[data-slot="animated-tabs-trigger"][data-state="inactive"]',
      );
      await app.waitFor(
        () =>
          app.execute(
            `return Boolean(document.querySelector(
              '[data-slot="animated-tabs-trigger"][data-state="active"] [data-slot="animated-tabs-indicator"]'
            ));`,
          ),
        { description: `${theme.id} tab indicator after switching` },
      );
    }

    await app.clickSelector('[aria-label="Groups"]');
    await app.waitFor(
      () =>
        app.execute(
          `return Boolean(document.querySelector('[data-slot="group-summary-pill"]'));`,
        ),
      { description: "Profile groups summary pill" },
    );

    for (const theme of THEMES) {
      await applyThemeForContrastAudit(app, theme);
      const snapshot = await app.execute(`
        const rootStyle = getComputedStyle(document.documentElement);
        const pill = document.querySelector(
          '[data-slot="group-summary-pill"]',
        );
        const count = pill?.querySelector(
          '[data-slot="group-summary-count"]',
        );
        const title = pill?.firstElementChild;
        const rail = document.querySelector('nav [aria-current="page"]');
        return {
          pillBackground: pill ? getComputedStyle(pill).backgroundColor : null,
          pillTitle: title ? getComputedStyle(title).color : null,
          pillCount: count ? getComputedStyle(count).color : null,
          railBackground: rail
            ? getComputedStyle(rail).backgroundColor
            : null,
          railForeground: rail ? getComputedStyle(rail).color : null,
          accent: rootStyle.getPropertyValue("--accent").trim(),
          accentForeground: rootStyle
            .getPropertyValue("--accent-foreground")
            .trim(),
        };
      `);
      assertColorEquals(
        snapshot.pillBackground,
        snapshot.accent,
        `${theme.id} group pill background`,
      );
      for (const [label, color] of [
        ["group pill title", snapshot.pillTitle],
        ["group pill count", snapshot.pillCount],
      ]) {
        assertColorEquals(
          color,
          snapshot.accentForeground,
          `${theme.id} ${label} token`,
        );
        assertContrast(
          color,
          snapshot.pillBackground,
          4.5,
          `${theme.id} ${label}`,
        );
      }
      assertContrast(
        snapshot.railForeground,
        snapshot.railBackground,
        3,
        `${theme.id} selected rail icon`,
      );
    }
  });
});

test("Xray proxy form keeps the share link as one clear, validated input", async () => {
  await withApp("ui-vless-proxy-form", async (app) => {
    const uri =
      "vless://6d6e21a1-4829-4d2b-bc7f-1b25707b61e4@vpn.example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.example.com&fp=chrome&pbk=BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc&sid=0123456789abcdef&type=tcp#E2E";

    await app.clickSelector('[aria-label="Network"]');
    await app.waitForText("New proxy");
    await app.clickSelector('[aria-label="New proxy"]');
    await app.waitForText("Add Proxy");
    await app.fillSelector("#proxy-name", "E2E VLESS");
    // chooseSelectOption matches the label exactly.
    await chooseSelectOption(app, "#proxy-type", "VLESS");

    assert.equal(
      await app.execute(
        `return document.querySelector("#proxy-vless-uri") instanceof HTMLTextAreaElement;`,
      ),
      true,
    );
    assert.equal(
      await app.execute(
        `return document.querySelector("#proxy-host") === null &&
          document.querySelector("#proxy-port") === null &&
          document.querySelector("#proxy-username") === null &&
          document.querySelector("#proxy-password") === null;`,
      ),
      true,
    );

    await app.fillSelector(
      "#proxy-vless-uri",
      "vless://6d6e21a1-4829-4d2b-bc7f-1b25707b61e4@vpn.example.com",
    );
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector("#proxy-vless-uri")?.getAttribute("aria-invalid") === "true";`,
        ),
      { description: "invalid VLESS endpoint feedback" },
    );
    assert.equal(
      await app.execute(
        `return [...document.querySelectorAll("[role='dialog'] button")]
          .find((button) => button.textContent?.trim() === "Add Proxy")
          ?.disabled === true;`,
      ),
      true,
    );

    // A well-formed link for a setup Donut cannot use must say WHICH part is
    // unsupported, rather than implying the user mistyped it.
    await app.fillSelector(
      "#proxy-vless-uri",
      `${uri.replace("type=tcp", "type=kcp")}&seed=x`,
    );
    // The hint below the field also says "transport", so only the backend's
    // refusal, which marks the field invalid, counts here.
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector("#proxy-vless-uri")?.getAttribute("aria-invalid") === "true" &&
            /uses a transport/i.test(
              document.querySelector("#proxy-vless-uri-help")?.textContent || ""
            );`,
        ),
      { description: "transport-specific unsupported message" },
    );

    // The link names its protocol, and the backend stores the proxy under it,
    // so the form follows the link before anything is saved.
    await app.fillSelector(
      "#proxy-vless-uri",
      "trojan://pw@tj.example.com:443?sni=tj.example.com",
    );
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector("#proxy-type")?.textContent?.trim() === "Trojan";`,
        ),
      { description: "type follows a Trojan link" },
    );

    await app.fillSelector("#proxy-vless-uri", uri);
    // The field is not marked invalid while the backend is still reading the
    // link, so the enabled button is what proves it was accepted.
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector("#proxy-vless-uri")?.getAttribute("aria-invalid") === "false" &&
            [...document.querySelectorAll("[role='dialog'] button")]
              .find((button) => button.textContent?.trim() === "Add Proxy")
              ?.disabled === false;`,
        ),
      { description: "valid VLESS endpoint feedback" },
    );
    await app.clickTextIn('[role="dialog"]', "Add Proxy", {
      roles: ["button"],
    });
    await app.waitForText("E2E VLESS");

    const proxy = (await app.invoke("get_stored_proxies")).find(
      (item) => item.name === "E2E VLESS",
    );
    assert.ok(proxy);
    assert.equal(proxy.proxy_settings.proxy_type, "vless");
    assert.equal(proxy.proxy_settings.host, "vpn.example.com");
    assert.equal(proxy.proxy_settings.port, 443);
    assert.equal(proxy.proxy_settings.username, null);
    assert.equal(proxy.proxy_settings.password, null);
    assert.match(proxy.proxy_settings.vless_uri, /^vless:\/\//);

    await app.invoke("delete_stored_proxy", { proxyId: proxy.id });
  });
});

test("pasting a proxy string into the form fills every field", async () => {
  await withApp("ui-proxy-form-paste", async (app) => {
    // Dispatched rather than typed: the point is that the paste is spread
    // across the form instead of landing whole in the field it was dropped on,
    // and only a real ClipboardEvent carries the text the handler reads.
    const paste = (selector, text) =>
      app.execute(
        `const field = document.querySelector(arguments[0]);
         const data = new DataTransfer();
         data.setData("text/plain", arguments[1]);
         field.focus();
         return field.dispatchEvent(
           new ClipboardEvent("paste", {
             bubbles: true,
             cancelable: true,
             clipboardData: data,
           }),
         );`,
        [selector, text],
      );
    const fieldValues = () =>
      app.execute(
        `return ["#proxy-name", "#proxy-host", "#proxy-port", "#proxy-username", "#proxy-password"]
           .map((selector) => document.querySelector(selector)?.value ?? null);`,
      );

    await app.clickSelector('[aria-label="Network"]');
    await app.waitForText("New proxy");
    await app.clickSelector('[aria-label="New proxy"]');
    await app.waitForText("Add Proxy");

    await paste("#proxy-host", "socks5://carol:s3cret@1.2.3.4:1080");
    await app.waitFor(async () => (await fieldValues())[1] === "1.2.3.4", {
      description: "host filled from the pasted proxy",
    });
    assert.deepEqual(await fieldValues(), [
      "1.2.3.4:1080",
      "1.2.3.4",
      "1080",
      "carol",
      "s3cret",
    ]);
    assert.equal(
      await app.execute(
        `return document.querySelector("#proxy-type")?.textContent?.trim();`,
      ),
      "SOCKS5",
    );

    // No scheme in the line, so the type falls back to HTTP.
    await paste("#proxy-name", "5.6.7.8:8080:dave:hunter2");
    await app.waitFor(async () => (await fieldValues())[1] === "5.6.7.8", {
      description: "scheme-less proxy string parsed",
    });
    assert.deepEqual((await fieldValues()).slice(1), [
      "5.6.7.8",
      "8080",
      "dave",
      "hunter2",
    ]);
    assert.equal(
      await app.execute(
        `return document.querySelector("#proxy-type")?.textContent?.trim();`,
      ),
      "HTTP",
    );

    // A bare hostname is not a proxy string, so the form is left alone and the
    // browser's own paste stands.
    await paste("#proxy-host", "proxy.example.com");
    // Settle the parse round-trip, so "nothing changed" isn't just "nothing
    // has come back yet".
    await app.invoke("get_stored_proxies");
    assert.deepEqual((await fieldValues()).slice(1), [
      "5.6.7.8",
      "8080",
      "dave",
      "hunter2",
    ]);
  });
});

test("About exposes a searchable, responsive third-party license inventory", async () => {
  await withApp("ui-about-licenses", async (app) => {
    await app.clickSelector('[aria-label="More"]');
    await app.waitFor(
      () =>
        app.execute(`return Boolean(document.querySelector("[role='menu']"));`),
      { description: "More menu" },
    );
    await app.clickText("About Donut Browser", {
      exact: false,
      roles: ["menuitem"],
    });
    await app.waitForText("Open-source anti-detect browser.");

    const aboutText = await app.execute(
      `return document.querySelector("[role='dialog']")?.textContent ?? "";`,
    );
    assert.doesNotMatch(aboutText, /AGPL-3\.0|licensed under/i);

    await app.clickText("Licenses", { roles: ["button"] });
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector("[role='dialog'] [data-slot='dialog-title']")?.textContent === "Licenses";`,
        ),
      { description: "Licenses view" },
    );

    await app.session.command("POST", "/window/rect", {
      width: 640,
      height: 400,
    });
    const inventory = await app.execute(`
      const dialog = document.querySelector("[role='dialog']");
      const list = dialog?.querySelector(".scroll-fade");
      const search = dialog?.querySelector('input[type="search"]');
      const rows = [...(dialog?.querySelectorAll("li") ?? [])].map((row) =>
        [...row.children].map((child) => (child.textContent || "").trim())
      );
      const rect = dialog?.getBoundingClientRect();
      return {
        activeSearch: document.activeElement === search,
        rows,
        scrollable: Boolean(list && list.scrollHeight > list.clientHeight),
        bounds: rect
          ? {
              left: rect.left,
              top: rect.top,
              right: rect.right,
              bottom: rect.bottom,
              viewportWidth: innerWidth,
              viewportHeight: innerHeight,
            }
          : null,
      };
    `);
    assert.equal(inventory.activeSearch, true);
    assert.equal(inventory.scrollable, true);
    assert.ok(inventory.bounds);
    assert.ok(inventory.bounds.left >= 0);
    assert.ok(inventory.bounds.top >= 0);
    assert.ok(inventory.bounds.right <= inventory.bounds.viewportWidth);
    assert.ok(inventory.bounds.bottom <= inventory.bounds.viewportHeight);
    assert.ok(
      inventory.rows.some(
        ([name, license]) => name === "Xray-core" && license === "MPL-2.0",
      ),
    );
    assert.ok(
      inventory.rows.some(
        ([name, license]) =>
          name === "Donut Browser" && license === "AGPL-3.0-only",
      ),
    );
    assert.ok(
      inventory.rows.some(
        ([name, license]) =>
          name === "tauri-plugin-opener" && license === "Apache-2.0 OR MIT",
      ),
    );
    assert.ok(inventory.rows.every((row) => row.length === 2));

    const search = await app.session.findCss('input[type="search"]');
    await app.session.sendKeys(search, "xray");
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelectorAll("[role='dialog'] li").length === 1;`,
        ),
      { description: "license search result" },
    );
    assert.match(
      await app.execute(
        `return document.querySelector("[role='dialog'] li")?.textContent ?? "";`,
      ),
      /Xray-core.*MPL-2\.0/s,
    );

    await app.clickSelector('button[aria-label="Back"]');
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector("[role='dialog'] [data-slot='dialog-title']")?.textContent === "About";`,
        ),
      { description: "About view after returning from licenses" },
    );
    assert.equal(
      await app.execute(
        `return document.activeElement?.textContent?.trim() === "Licenses";`,
      ),
      true,
    );
  });
});

test("settings tabs, command palette filtering, and responsive layout survive resize", async () => {
  await withApp("ui-settings-responsive", async (app) => {
    await app.clickSelector('[aria-label="Settings"]');
    await app.waitForText("Appearance");
    for (const tab of ["Appearance", "Sync", "Encryption"]) {
      const exists = await app.execute(
        `return [...document.querySelectorAll("[role='tab']")].some(
          (node) => (node.textContent || "").trim() === arguments[0]
        );`,
        [tab],
      );
      if (exists) {
        await app.clickText(tab, { roles: ["tab"] });
        await app.waitFor(
          () =>
            app.execute(
              `return [...document.querySelectorAll("[role='tab']")].some(
                (node) => (node.textContent || "").trim() === arguments[0] &&
                  node.getAttribute("data-state") === "active"
              );`,
              [tab],
            ),
          { description: `${tab} settings tab` },
        );
      }
    }
    await dismissSurface(app);

    const modifier =
      process.platform === "darwin" ? { meta: true } : { ctrl: true };
    await app.pressShortcut({ key: "k", ...modifier });
    await app.waitFor(
      () =>
        app.execute(`return Boolean(document.querySelector("[cmdk-input]"));`),
      { description: "command palette" },
    );
    const input = await app.session.findCss("[cmdk-input]");
    await app.session.sendKeys(input, "proxy vpn");
    assert.match(await app.bodyText(), /Network|Proxy|VPN/i);
    await dismissSurface(app);

    // The native driver owns the top-level window. Resize through the WebDriver
    // protocol and assert the app still has usable controls at the minimum size.
    await app.session.command("POST", "/window/rect", {
      width: 640,
      height: 400,
    });
    const viewport = await app.execute(
      "return { width: innerWidth, height: innerHeight };",
    );
    assert.ok(viewport.width >= 600);
    assert.ok(viewport.height >= 350);
    assert.equal(
      await app.execute(
        `return document.querySelector('[aria-label="Settings"]').getBoundingClientRect().width > 0;`,
      ),
      true,
    );
  });
});

test("first-run onboarding stays recoverable, responsive, and platform-aware", async () => {
  await withApp(
    "ui-onboarding",
    async (app) => {
      await app.waitForText("Welcome to Donut Browser");
      assert.equal(await app.invoke("get_onboarding_completed"), false);

      await app.session.command("POST", "/window/rect", {
        width: 640,
        height: 400,
      });
      const layout = await app.execute(`
        const dialog = document.querySelector("[role='dialog']");
        const progress = dialog?.querySelector("[role='progressbar']");
        if (!dialog || !progress) return null;
        const rect = dialog.getBoundingClientRect();
        return {
          left: rect.left,
          top: rect.top,
          right: rect.right,
          bottom: rect.bottom,
          viewportWidth: innerWidth,
          viewportHeight: innerHeight,
          progressNow: progress.getAttribute("aria-valuenow"),
        };
      `);
      assert.ok(layout);
      assert.ok(layout.left >= 0 && layout.right <= layout.viewportWidth);
      assert.ok(layout.top >= 0 && layout.bottom <= layout.viewportHeight);
      assert.equal(layout.progressNow, "1");

      await app.clickText("Next", { roles: ["button"] });
      await app.waitForText("Licensing");
      await app.clickText("I understand", { roles: ["button"] });

      await app.waitFor(
        async () =>
          /Allow microphone & camera|Setting things up|Setup failed/.test(
            await app.bodyText(),
          ),
        { description: "platform-appropriate onboarding step" },
      );
      const body = await app.bodyText();
      if (process.platform !== "darwin") {
        assert.doesNotMatch(body, /Allow microphone & camera/);
        assert.match(body, /Setting things up|Setup failed/);
      }

      // Setup and the optional product tour are still unfinished, so a crash
      // or restart must be able to resume onboarding.
      assert.equal(await app.invoke("get_onboarding_completed"), false);
    },
    { onboardingCompleted: false },
  );
});

test("predefined theme remains rendered across navigation and restart", async () => {
  await withApp("ui-theme-predefined", async (app) => {
    await app.clickSelector('[aria-label="Settings"]');
    await app.waitForText("Appearance");
    await chooseSelectOption(app, "#theme-select", "Light");
    await saveSettings(app);

    const persisted = await app.invoke("get_app_settings");
    assert.equal(persisted.theme, "light");
    const selected = await waitForTheme(
      app,
      (snapshot) =>
        snapshot.mode === "light" &&
        Object.values(snapshot.inline).every((value) => value === ""),
      "predefined light theme to render without custom variables",
    );
    assert.notEqual(selected.bodyBackground, "");
    assert.notEqual(selected.bodyForeground, "");
    await assertThemeAcrossNavigation(app, selected);

    await app.restart();
    assert.equal((await app.invoke("get_app_settings")).theme, "light");
    await app.waitFor(
      async () =>
        JSON.stringify(await themeSnapshot(app)) === JSON.stringify(selected),
      { description: "predefined light theme after restart" },
    );
  });
});

test("preset and manually customized themes survive navigation and restart", async () => {
  await withApp("ui-theme-custom", async (app) => {
    await app.clickSelector('[aria-label="Settings"]');
    await app.waitForText("Appearance");
    await chooseSelectOption(app, "#theme-select", "Custom");
    await chooseSelectOption(app, "#theme-preset-select", "Dracula");
    await saveSettings(app);

    const presetSettings = await app.invoke("get_app_settings");
    assert.equal(presetSettings.theme, "custom");
    assert.deepEqual(presetSettings.custom_theme, DRACULA_THEME);
    const preset = await waitForTheme(
      app,
      (snapshot) =>
        snapshot.mode === "dark" &&
        themeVariablesEqual(snapshot.inline, DRACULA_THEME) &&
        themeVariablesEqual(snapshot.resolved, DRACULA_THEME),
      "Dracula preset variables to render",
    );
    await assertThemeAcrossNavigation(app, preset);

    await app.restart();
    assert.deepEqual(
      (await app.invoke("get_app_settings")).custom_theme,
      DRACULA_THEME,
    );
    await app.waitFor(
      async () =>
        JSON.stringify(await themeSnapshot(app)) === JSON.stringify(preset),
      { description: "Dracula preset after restart" },
    );

    await app.clickSelector('[aria-label="Settings"]');
    await app.waitForText("Appearance");
    assert.equal(
      await app.execute(
        `return document.querySelector("#theme-select")?.textContent?.trim();`,
      ),
      "Custom",
    );
    assert.equal(
      await app.execute(
        `return document.querySelector("#theme-preset-select")?.textContent?.trim();`,
      ),
      "Dracula",
    );
    await dragBackgroundColorPicker(app);
    await saveSettings(app);

    const customizedSettings = await app.invoke("get_app_settings");
    assert.equal(customizedSettings.theme, "custom");
    assert.notEqual(
      customizedSettings.custom_theme["--background"].toLowerCase(),
      DRACULA_THEME["--background"],
    );
    assert.deepEqual(
      Object.keys(customizedSettings.custom_theme).sort(),
      [...THEME_VARIABLES].sort(),
    );
    const customized = await waitForTheme(
      app,
      (snapshot) =>
        snapshot.mode === "dark" &&
        themeVariablesEqual(snapshot.inline, customizedSettings.custom_theme) &&
        themeVariablesEqual(snapshot.resolved, customizedSettings.custom_theme),
      "manually customized variables to render",
    );
    assert.notEqual(customized.bodyBackground, preset.bodyBackground);
    await assertThemeAcrossNavigation(app, customized);

    await app.restart();
    assert.deepEqual(
      (await app.invoke("get_app_settings")).custom_theme,
      customizedSettings.custom_theme,
    );
    await app.waitFor(
      async () =>
        JSON.stringify(await themeSnapshot(app)) === JSON.stringify(customized),
      { description: "manually customized theme after restart" },
    );
  });
});

test("a light custom preset keeps light component behavior after restart", async () => {
  await withApp("ui-theme-custom-light", async (app) => {
    await app.clickSelector('[aria-label="Settings"]');
    await app.waitForText("Appearance");
    await chooseSelectOption(app, "#theme-select", "Custom");
    await chooseSelectOption(app, "#theme-preset-select", "Ayu Light");
    await saveSettings(app);

    const selected = await waitForTheme(
      app,
      (snapshot) =>
        snapshot.mode === "light" &&
        themeVariablesEqual(snapshot.inline, AYU_LIGHT_THEME) &&
        themeVariablesEqual(snapshot.resolved, AYU_LIGHT_THEME),
      "Ayu Light variables and light mode to render",
    );
    await assertThemeAcrossNavigation(app, selected);

    await app.restart();
    assert.deepEqual(
      (await app.invoke("get_app_settings")).custom_theme,
      AYU_LIGHT_THEME,
    );
    await app.waitFor(
      async () =>
        JSON.stringify(await themeSnapshot(app)) === JSON.stringify(selected),
      { description: "Ayu Light preset after restart" },
    );
  });
});

const EXTENSION_STRINGS = en.extensions;

/**
 * Answer the native directory picker from inside the webview.
 *
 * "Load unpacked" calls `open({ directory: true })` from
 * `@tauri-apps/plugin-dialog`, which puts an OS window on screen that no
 * WebDriver can reach. The call leaves the page as the `plugin:dialog|open` IPC
 * command, but Tauri locks its own entry points down: `invoke`, `ipc` and
 * `postMessage` are all installed with
 * `Object.defineProperty(window.__TAURI_INTERNALS__, name, { value })`, so they
 * are non-writable and cannot be wrapped. The seam underneath them is the
 * transport, which POSTs the command through `fetch` to
 * `ipc://localhost/<command>`. Answering that one request with the shape Tauri
 * expects (`Tauri-Response: ok` plus a JSON body) resolves the picker with a
 * folder and needs no test-only hook in the production component. Every other
 * command still reaches the real backend.
 */
async function stubFolderPicker(app, folder) {
  await app.execute(
    `const folder = arguments[0];
     if (!window.__donutOriginalFetch) {
       window.__donutOriginalFetch = window.fetch;
     }
     window.__donutFolderPickerCalls = [];
     window.fetch = function (input, init) {
       const url = String(
         typeof input === "string" ? input : (input && input.url) || "",
       );
       let command = "";
       try {
         command = decodeURIComponent(url.split("/").pop() || "");
       } catch (_error) {
         command = "";
       }
       if (command === "plugin:dialog|open") {
         let payload = null;
         try {
           payload = JSON.parse((init && init.body) || "null");
         } catch (_error) {
           payload = null;
         }
         window.__donutFolderPickerCalls.push(payload);
         return Promise.resolve(
           new Response(JSON.stringify(folder), {
             status: 200,
             headers: {
               "content-type": "application/json",
               "Tauri-Response": "ok",
             },
           }),
         );
       }
       return window.__donutOriginalFetch.apply(window, arguments);
     };
     return true;`,
    [folder],
  );
}

/** Restores the real transport and returns what the picker was asked for. */
async function restoreFolderPicker(app) {
  return app.execute(
    `const calls = window.__donutFolderPickerCalls ?? [];
     if (window.__donutOriginalFetch) {
       window.fetch = window.__donutOriginalFetch;
       delete window.__donutOriginalFetch;
     }
     delete window.__donutFolderPickerCalls;
     return calls;`,
  );
}

async function openExtensionsPage(app) {
  await app.clickSelector('[aria-label="Extensions"]');
  await app.waitFor(
    () =>
      app.execute(`return Boolean(document.querySelector(arguments[0]));`, [
        `[aria-label="${EXTENSION_STRINGS.loadUnpacked}"]`,
      ]),
    { description: "extension management page" },
  );
}

async function stageUnpackedFolder(app, folder) {
  await stubFolderPicker(app, folder);
  await app.clickSelector(`[aria-label="${EXTENSION_STRINGS.loadUnpacked}"]`);
  await app.waitFor(
    () =>
      app.execute(
        `return Boolean(document.querySelector("#ext-link-folder"));`,
      ),
    {
      description:
        "staged folder import form (the intercepted directory picker has to resolve)",
    },
  );
}

async function uploadArchiveThroughUi(app, archivePath, typedName) {
  // The real control is a hidden file input a button clicks for the user;
  // WebDriver can only type a path into an input it can see.
  await app.execute(`
    const input = document.querySelector("#ext-file-input");
    input.classList.remove("hidden");
    input.style.position = "fixed";
    input.style.left = "12px";
    input.style.bottom = "12px";
  `);
  const input = await app.session.findCss("#ext-file-input");
  await app.session.sendKeys(input, archivePath);
  await app.waitForText(path.basename(archivePath));
  await app.execute(`
    const input = document.querySelector("#ext-file-input");
    input.classList.add("hidden");
    input.removeAttribute("style");
  `);
  await app.fillSelector(
    `input[placeholder="${EXTENSION_STRINGS.namePlaceholder}"]`,
    typedName,
  );
  await app.clickText(en.common.buttons.add, { roles: ["button"] });
}

/** The link checkbox plus the copy that is supposed to explain it. */
async function linkCheckboxState(app, id) {
  return app.execute(
    `const checkbox = document.querySelector("#" + arguments[0]);
     const label = document.querySelector('label[for="' + arguments[0] + '"]');
     const help = label?.parentElement?.querySelector("p");
     return checkbox
       ? {
           checked: checkbox.getAttribute("data-state") === "checked",
           label: (label?.innerText ?? "").trim(),
           help: (help?.innerText ?? "").trim(),
         }
       : null;`,
    [id],
  );
}

function extensionRowScript(body) {
  return `const wanted = arguments[0];
     const row = [...document.querySelectorAll("tbody tr")].find((candidate) => {
       const cells = [...candidate.querySelectorAll("td")];
       return cells.length >= 7 && (cells[2].querySelector("button > span")?.textContent || cells[2].innerText || "").trim() === wanted;
     });
     ${body}`;
}

async function extensionRow(app, name) {
  return app.execute(
    extensionRowScript(`if (!row) return null;
     const cells = [...row.querySelectorAll("td")];
     const sync = row.querySelector('[data-slot="animated-switch"]');
     return {
       name: (cells[2].querySelector("button > span")?.textContent || cells[2].innerText || "").trim(),
       source: (cells[4].innerText || "").trim(),
       syncChecked: sync ? sync.getAttribute("data-state") === "checked" : null,
       syncDisabled: sync ? sync.disabled === true : null,
     };`),
    [name],
  );
}

async function extensionEditButton(app, name) {
  return app.execute(
    extensionRowScript(
      `return row ? row.querySelector("td:last-child button") : null;`,
    ),
    [name],
  );
}

async function dialogText(app, title) {
  return app.execute(
    `const wanted = arguments[0];
     const dialog = [...document.querySelectorAll("[role='dialog']")]
       .reverse()
       .find((node) =>
         [...node.querySelectorAll("[data-slot='dialog-title']")].some(
           (heading) => (heading.textContent || "").trim() === wanted,
         ),
       );
     return dialog ? (dialog.innerText || "").trim() : null;`,
    [title],
  );
}

async function toastTexts(app) {
  return app.execute(
    `return [...document.querySelectorAll("[data-sonner-toast]")]
       .map((toast) => (toast.innerText || "").trim())
       .filter(Boolean);`,
  );
}

test("an uploaded archive and a loaded folder both import, each under its own source", async () => {
  await withApp("ui-extension-import-sources", async (app) => {
    const archivePath = path.join(app.root, "ui-archive-extension.zip");
    await writeFile(archivePath, Buffer.from(extensionZipBase64(), "base64"));
    const folder = await writeUnpackedExtension(
      path.join(app.root, "fixtures", "ui-copied-extension"),
      { name: "Donut UI Copied Folder" },
    );

    await openExtensionsPage(app);
    await uploadArchiveThroughUi(
      app,
      archivePath,
      "Overridden By The Manifest",
    );
    await app.waitForText("Donut E2E Fixture");

    await stageUnpackedFolder(app, folder);
    assert.ok(await app.visibleTextIncludes(EXTENSION_STRINGS.selectedFolder));
    assert.ok(
      await app.visibleTextIncludes(folder),
      "the staged import has to name the folder it is about to read",
    );
    const staged = await linkCheckboxState(app, "ext-link-folder");
    assert.equal(staged?.checked, false, "linking a folder has to be opt-in");
    assert.equal(staged.help, EXTENSION_STRINGS.linkFolderOff);
    await app.clickText(en.common.buttons.add, { roles: ["button"] });
    await app.waitForText("Donut UI Copied Folder");

    const pickerCalls = await restoreFolderPicker(app);
    assert.equal(pickerCalls.length, 1, "Load unpacked has to open the picker");
    assert.equal(pickerCalls[0].options.directory, true);
    assert.equal(pickerCalls[0].options.multiple, false);
    assert.equal(
      pickerCalls[0].options.title,
      EXTENSION_STRINGS.selectFolderTitle,
    );

    assert.notEqual(
      EXTENSION_STRINGS.source.archive,
      EXTENSION_STRINGS.source.unpacked,
    );
    assert.equal(
      (await extensionRow(app, "Donut E2E Fixture"))?.source,
      EXTENSION_STRINGS.source.archive,
    );
    assert.equal(
      (await extensionRow(app, "Donut UI Copied Folder"))?.source,
      EXTENSION_STRINGS.source.unpacked,
    );

    const extensions = await app.invoke("list_extensions");
    assert.equal(extensions.length, 2);
    const copied = extensions.find(
      (extension) => extension.name === "Donut UI Copied Folder",
    );
    assert.equal(copied.source_kind, "unpacked");
    assert.equal(
      copied.linked_path,
      null,
      "an unlinked folder import is copied into the store, not pointed at",
    );
    assert.equal(copied.file_type, "zip");
    const archive = extensions.find(
      (extension) => extension.name === "Donut E2E Fixture",
    );
    assert.equal(archive.source_kind, "archive");
    assert.equal(archive.linked_path, null);
  });
});

test("linking a folder says what it costs, records the path, and locks that row's sync off", async () => {
  await withApp("ui-extension-linked-folder", async (app) => {
    await app.invoke("add_extension", {
      name: "Copied Neighbour",
      fileName: "ui-neighbour-extension.zip",
      fileData: [...Buffer.from(extensionZipBase64(), "base64")],
    });
    const folder = await writeUnpackedExtension(
      path.join(app.root, "fixtures", "ui-linked-extension"),
      { name: "Donut UI Linked Folder" },
    );

    await openExtensionsPage(app);
    await app.waitForText("Donut E2E Fixture");
    await stageUnpackedFolder(app, folder);

    const off = await linkCheckboxState(app, "ext-link-folder");
    assert.equal(off?.checked, false);
    assert.equal(off.label, EXTENSION_STRINGS.linkFolder);
    assert.equal(off.help, EXTENSION_STRINGS.linkFolderOff);

    await app.clickSelector("#ext-link-folder");
    const on = await app.waitFor(
      async () => {
        const state = await linkCheckboxState(app, "ext-link-folder");
        return state?.checked ? state : false;
      },
      { description: "link checkbox to turn on" },
    );
    assert.equal(on.help, EXTENSION_STRINGS.linkFolderOn);
    assert.notEqual(
      on.help,
      off.help,
      "the checkbox has to say what turning it on changes",
    );

    await app.clickText(en.common.buttons.add, { roles: ["button"] });
    await app.waitForText("Donut UI Linked Folder");
    assert.equal((await restoreFolderPicker(app)).length, 1);

    const linkedRow = await extensionRow(app, "Donut UI Linked Folder");
    assert.equal(linkedRow?.source, EXTENSION_STRINGS.source.linked);
    assert.equal(linkedRow.syncChecked, false);
    assert.equal(linkedRow.syncDisabled, true);
    assert.equal(
      (await extensionRow(app, "Donut E2E Fixture"))?.syncDisabled,
      false,
      "only the linked row loses its sync control",
    );

    const linked = (await app.invoke("list_extensions")).find(
      (extension) => extension.name === "Donut UI Linked Folder",
    );
    assert.equal(linked.linked_path, await realpath(folder));
    assert.equal(linked.file_type, "unpacked");
    assert.equal(linked.sync_enabled, false);
  });
});

test("the edit dialog replaces an extension's payload from a folder", async () => {
  await withApp("ui-extension-replace-from-folder", async (app) => {
    const original = await app.invoke("add_extension", {
      name: "Replaced Later",
      fileName: "ui-original-extension.zip",
      fileData: [...Buffer.from(extensionZipBase64(), "base64")],
    });
    assert.equal(original.version, "1.0.0");
    const folder = await writeUnpackedExtension(
      path.join(app.root, "fixtures", "ui-replacement-extension"),
      { name: "Donut UI Replacement", version: "3.1.4" },
    );

    await openExtensionsPage(app);
    await app.waitForText("Donut E2E Fixture");
    const editButton = await extensionEditButton(app, "Donut E2E Fixture");
    assert.ok(editButton, "the extension row's edit control was not visible");
    await app.clickElement(editButton, "extension edit button");
    const beforeReplace = await app.waitFor(
      () => dialogText(app, EXTENSION_STRINGS.editExtension),
      { description: "extension edit dialog" },
    );
    assert.ok(beforeReplace.includes(EXTENSION_STRINGS.source.label));
    assert.ok(beforeReplace.includes(EXTENSION_STRINGS.source.archive));

    await stubFolderPicker(app, folder);
    await app.clickTextIn('[role="dialog"]', EXTENSION_STRINGS.selectFolder, {
      roles: ["button"],
    });
    await app.waitFor(
      async () =>
        (await dialogText(app, EXTENSION_STRINGS.editExtension))?.includes(
          folder,
        ),
      { description: "chosen replacement folder" },
    );
    const replaceLink = await linkCheckboxState(app, "ext-edit-link-folder");
    assert.equal(replaceLink?.checked, false);
    assert.equal(replaceLink.help, EXTENSION_STRINGS.linkFolderOff);

    await app.clickTextIn('[role="dialog"]', en.common.buttons.save, {
      roles: ["button"],
    });
    await app.waitFor(
      async () =>
        (await toastTexts(app)).some((text) =>
          text.includes(EXTENSION_STRINGS.updateSuccess),
        ),
      { description: "extension update confirmation" },
    );
    assert.equal((await restoreFolderPicker(app)).length, 1);

    const extensions = await app.invoke("list_extensions");
    assert.equal(
      extensions.length,
      1,
      "replacing a payload must not add a second extension",
    );
    const [updated] = extensions;
    assert.equal(updated.id, original.id);
    assert.equal(updated.source_kind, "unpacked");
    assert.equal(updated.linked_path, null);
    assert.equal(updated.file_name, "ui-replacement-extension.zip");
    assert.equal(updated.version, "3.1.4");
    // The dialog's own name field stays authoritative, so the row keeps its
    // name while the payload underneath it is swapped.
    assert.equal(updated.name, "Donut E2E Fixture");

    await app.waitFor(
      async () =>
        (await extensionRow(app, "Donut E2E Fixture"))?.source ===
        EXTENSION_STRINGS.source.unpacked,
      { description: "replaced row to report its new source" },
    );
  });
});

test("a folder with no manifest fails with the translated reason, not a raw code", async () => {
  await withApp("ui-extension-manifest-missing", async (app) => {
    const folder = path.join(app.root, "fixtures", "ui-not-an-extension");
    await mkdir(folder, { recursive: true });
    await writeFile(path.join(folder, "readme.txt"), "no manifest here\n");

    await openExtensionsPage(app);
    await stageUnpackedFolder(app, folder);
    await app.clickText(en.common.buttons.add, { roles: ["button"] });

    const expected = en.backendErrors.extensionManifestMissing;
    await app.waitFor(
      async () =>
        (await toastTexts(app)).some((text) => text.includes(expected)),
      { description: "translated manifest-missing toast" },
    );
    const toasts = await toastTexts(app);
    assert.ok(
      toasts.every((text) => !text.includes("EXTENSION_MANIFEST_MISSING")),
      `a raw backend code reached the user: ${JSON.stringify(toasts)}`,
    );
    assert.ok(
      toasts.every((text) => !text.includes(EXTENSION_STRINGS.uploadFailed)),
      "the generic fallback would hide which folder problem this was",
    );
    assert.deepEqual(await app.invoke("list_extensions"), []);
    assert.equal((await restoreFolderPicker(app)).length, 1);
  });
});

test("the agent page guides setup, then shows agents, requests, notes, take-overs and the pause live", async () => {
  await withApp(
    "ui-agent",
    async (app) => {
      await app.waitForText("No profiles yet");
      const painted = (testId) =>
        app.execute(
          `const el = document.querySelector('[data-testid="' + arguments[0] + '"]');
         if (!el) return null;
         const rect = el.getBoundingClientRect();
         const style = getComputedStyle(el);
         return rect.width > 0 && rect.height > 0 && Number(style.opacity) === 1 && style.visibility === "visible"
           ? el.innerText.trim()
           : null;`,
          [testId],
        );
      const waitPainted = (testId) =>
        app.waitFor(() => painted(testId), { description: testId });
      const absent = (testId) =>
        app.waitFor(
          () =>
            app.execute(
              `return !document.querySelector('[data-testid="' + arguments[0] + '"]');`,
              [testId],
            ),
          { description: `${testId} gone` },
        );

      // Signed out, the page says what is missing instead of showing controls.
      await app.clickSelector(`[aria-label="${en.rail.agent}"]`);
      const signedOut = await waitPainted("agent-setup-signed-out");
      assert.match(signedOut, new RegExp(en.agent.setup.signedOutTitle));
      assert.equal(await painted("agent-pause-switch"), null);
      // Leaving the page unmounts it, so the next visit reads the stubs below.
      await app.clickSelector(`[aria-label="${en.rail.profiles}"]`);
      await absent("agent-page");

      const profile = await createUiProfile(app, "Agent UI Profile");
      const now = Date.now();
      try {
        await stubCommand(app, "cloud_get_user", {
          logged_in_at: new Date().toISOString(),
          user: {
            id: "ui-agent",
            email: "agent@example.test",
            plan: "pro",
            planPeriod: "monthly",
            subscriptionStatus: "active",
            profileLimit: 50,
            cloudProfilesUsed: 0,
            proxyBandwidthLimitMb: 0,
            proxyBandwidthUsedMb: 0,
            proxyBandwidthExtraMb: 0,
            isPrimaryDevice: true,
          },
        });
        await stubCommand(app, "get_mcp_remote_status", {
          enabled: true,
          connected: true,
          instanceId: "ui-agent-instance",
          lastError: null,
        });
        const base = {
          session_id: "ui-s1",
          profile_id: null,
          choices: [],
          answer: null,
          answered_at: null,
          delivered_to: [],
          done: null,
          total: null,
        };
        const question = {
          ...base,
          id: 9001,
          at: now - 5000,
          kind: "question",
          text: "Which shipping address should I use?",
          choices: ["Home", "Office"],
          state: "open",
        };
        const help = {
          ...base,
          id: 9002,
          at: now - 4000,
          kind: "help",
          text: "Solve the verification on the sign-in page",
          profile_id: profile.id,
          state: "open",
        };
        await stubCommand(app, "get_agent_console", {
          sessions: [
            {
              session_id: "ui-s1",
              client_name: "Claude Code",
              client_version: "2.1.0",
              connected_at: now - 60_000,
              last_seen_at: now - 1000,
              calls: 3,
              errors: 1,
              ended: false,
              status: {
                message: "Checking orders",
                done: 2,
                total: 5,
                profile_id: null,
                updated_at: now - 2000,
              },
            },
          ],
          activity: [
            {
              id: 9100,
              at: now - 3000,
              session_id: "ui-s1",
              tool: "navigate",
              profile_id: profile.id,
              profile_count: null,
              ok: true,
              error_code: null,
              duration_ms: 420,
              detail: "shop.example.com",
            },
            {
              id: 9101,
              at: now - 2500,
              session_id: "ui-s1",
              tool: "click_locator",
              profile_id: profile.id,
              profile_count: null,
              ok: false,
              error_code: "LOCATOR_NO_MATCH",
              duration_ms: 90,
              detail: null,
            },
          ],
          thread: [question, help],
          holds: [
            {
              profile_id: profile.id,
              since: now - 4000,
              note: null,
              request_id: 9002,
            },
          ],
          paused: null,
          quota: { limit: 2000, used: 240, resets_in_secs: 1200 },
        });
        await stubCommand(app, "answer_agent_request", {
          ...question,
          state: "answered",
          answer: "Office",
          answered_at: now,
        });
        await app.invoke("plugin:event|emit", {
          event: "cloud-auth-changed",
          payload: null,
        });
        await app.invoke("plugin:event|emit", {
          event: "agent-console-cleared",
          payload: null,
        });

        // Two open requests reach the rail before the page is opened.
        await app.waitFor(
          async () => (await painted("agent-rail-badge")) === "2",
          { description: "the rail badge counts open requests" },
        );
        const waitingLabel = en.rail.agentWaiting.replace("{{count}}", "2");
        await app.clickSelector(`[aria-label="${waitingLabel}"]`);
        await waitPainted("agent-needs-you");
        // The page holds the seeded snapshot now; later reads go to the backend.
        await app.execute(`delete window.__donutStubs.get_agent_console;`);
        await app.capture("agent-conversation");
        // The Conversation tab carries the open-request count after its label.
        assert.deepEqual(
          await app.execute(
            `return [...document.querySelectorAll('[role="tab"]')].map((node) => node.textContent.trim().replace(/\\d+$/, ""));`,
          ),
          [
            en.agent.tabs.conversation,
            en.agent.tabs.profiles,
            en.agent.tabs.activity,
            en.agent.tabs.recipes,
          ],
        );
        assert.match(await painted("agent-quota"), /1[,.\s ]?760/);
        assert.match(await painted("agent-session-ui-s1"), /Claude Code/);
        assert.match(await painted("agent-session-ui-s1"), /Checking orders/);
        assert.match(
          await painted("agent-request-9001"),
          /Which shipping address should I use\?/,
        );
        assert.match(
          await painted("agent-request-9002"),
          /Solve the verification on the sign-in page/,
        );
        assert.equal(
          await app.execute(
            `return document.querySelectorAll('[data-testid="agent-request-9001"] [data-testid="agent-choice"]').length;`,
          ),
          2,
        );

        // A suggested answer is one click, and the card leaves the list.
        await app.clickText("Office", { roles: ["button"] });
        await absent("agent-request-9001");
        const answered = await app.execute(
          `return (window.__donutStubbedCalls ?? []).filter((c) => c.command === "answer_agent_request").map((c) => c.payload);`,
        );
        assert.deepEqual(answered, [{ requestId: 9001, answer: "Office" }]);

        // Real backend events update the open page: a note, then the pause.
        await app.invoke("send_agent_note", {
          text: "Use the EU shipping profile today",
        });
        await app.waitFor(
          async () =>
            ((await painted("agent-thread")) ?? "").includes(
              "Use the EU shipping profile today",
            ),
          { description: "the note in the thread" },
        );
        await app.invoke("set_agents_paused", {
          paused: true,
          note: "Changing proxies",
        });
        assert.match(
          await waitPainted("agent-paused-banner"),
          /Changing proxies/,
        );
        await app.clickSelector('[data-testid="agent-resume"]');
        await absent("agent-paused-banner");
        assert.equal((await app.invoke("get_agent_console")).paused, null);

        // Profiles: the agent's profile is listed with its last error, and the
        // person can take it over and hand it back.
        await app.clickSelector('[data-testid="agent-tab-profiles"]');
        const row = await waitPainted(`agent-profile-${profile.id}`);
        assert.match(row, /Agent UI Profile/);
        assert.match(row, /LOCATOR_NO_MATCH/);
        assert.deepEqual(
          await offCenterSelectionControls(
            app,
            '[data-testid="agent-profiles"]',
          ),
          [],
        );
        await app.clickSelector(
          `[data-testid="agent-profile-${profile.id}"] [data-testid="agent-take-over"]`,
        );
        await app.fillSelector(
          '[data-testid="agent-take-over-note"]',
          "Fixing the address",
        );
        await app.clickSelector('[data-testid="agent-take-over-confirm"]');
        await app.waitFor(
          async () =>
            (await app.invoke("get_agent_console")).holds.some(
              (hold) =>
                hold.profile_id === profile.id &&
                hold.note === "Fixing the address",
            ),
          { description: "the take-over reached the backend" },
        );
        await app.waitFor(
          () =>
            app.execute(
              `return Boolean(document.querySelector('[data-testid="agent-profile-' + arguments[0] + '"] [data-testid="agent-hand-back"]'));`,
              [profile.id],
            ),
          { description: "the row offers hand back" },
        );
        await app.clickSelector(
          `[data-testid="agent-profile-${profile.id}"] [data-testid="agent-hand-back"]`,
        );
        await app.clickSelector('[data-testid="agent-hand-back-confirm"]');
        await app.waitFor(
          async () =>
            (await app.invoke("get_agent_console")).holds.length === 0,
          { description: "the hand-back reached the backend" },
        );

        await app.clickSelector('[data-testid="agent-tab-activity"]');
        const activity = await waitPainted("agent-activity-list");
        assert.match(activity, /shop\.example\.com/);
        assert.match(activity, /LOCATOR_NO_MATCH/);
        await app.capture("agent-page");
        await dismissSurface(app);

        // The palette reaches the same page.
        const modifier =
          process.platform === "darwin" ? { meta: true } : { ctrl: true };
        await app.pressShortcut({ key: "k", ...modifier });
        await app.waitFor(
          () =>
            app.execute(
              `return Boolean(document.querySelector("[cmdk-input]"));`,
            ),
          { description: "command palette" },
        );
        const input = await app.session.findCss("[cmdk-input]");
        await app.session.sendKeys(input, en.shortcuts.goAgent);
        await app.clickText(en.shortcuts.goAgent, {
          exact: false,
          roles: ["option", "button", "menuitem"],
        });
        await waitPainted("agent-page");
        await dismissSurface(app);
      } finally {
        await restoreStubs(app);
      }
    },
    { settings: { paid_welcome_seen_for: ["ui-agent"] } },
  );
});

async function createUiProfile(app, name) {
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

async function distributionSummary(app) {
  return app.execute(
    `return document.querySelector('[data-testid="distribute-summary"]')?.innerText ?? "";`,
  );
}

test("the distribute-proxies dialog opens from the action bar, counts the pairing, and answers every control", async () => {
  await withApp(
    "ui-proxy-distribution",
    async (app) => {
      for (const name of ["Fleet One", "Fleet Two", "Fleet Three"]) {
        await createUiProfile(app, name);
      }
      for (const [index, name] of ["Exit One", "Exit Two"].entries()) {
        await app.invoke("create_stored_proxy", {
          name,
          proxySettings: {
            proxy_type: "http",
            host: "127.0.0.1",
            port: 9101 + index,
            username: null,
            password: null,
          },
        });
      }
      await app.waitForText("Fleet Three");

      await app.clickSelector(`[aria-label="${en.common.aria.selectAll}"]`);
      await app.clickText(en.tables.moreActions, { roles: ["button"] });
      await app.clickText(en.profiles.actionBar.distributeProxies, {
        roles: ["menuitem"],
      });
      await app.waitForText(en.proxyDistribution.title);

      // Two proxies for three profiles: the dialog has to say so up front, and
      // it must never pretend the third profile is covered.
      await app.waitFor(
        async () => (await distributionSummary(app)).includes("2 of 3"),
        { description: "distribution summary" },
      );
      const summary = await distributionSummary(app);
      assert.match(summary, /2 of 3/);
      // The remainder is never silently dropped: the one profile no proxy was
      // left for is named. Which one depends on the table's order, so assert
      // that exactly one of the three is named rather than pinning the sort.
      const named = ["Fleet One", "Fleet Two", "Fleet Three"].filter((name) =>
        summary.includes(name),
      );
      assert.deepEqual(
        named.length,
        1,
        `the unpaired profile must be named exactly once, got: ${summary}`,
      );

      // Every control answers a click. Dropping the proxies empties the plan...
      await app.clickSelector('[data-testid="distribute-toggle-proxies"]');
      await app.waitFor(
        async () => (await distributionSummary(app)).includes("0 of 3"),
        { description: "summary after clearing the proxies" },
      );
      // ...and putting them back restores it.
      await app.clickSelector('[data-testid="distribute-toggle-proxies"]');
      await app.waitFor(
        async () => (await distributionSummary(app)).includes("2 of 3"),
        { description: "summary after reselecting the proxies" },
      );

      const switchState = () =>
        app.execute(
          `return document.querySelector('[aria-label="${en.proxyDistribution.allowSharingLabel}"]')?.getAttribute("aria-checked") ?? null;`,
        );
      assert.equal(await switchState(), "false", "sharing is off by default");
      await app.clickSelector(
        `[aria-label="${en.proxyDistribution.allowSharingLabel}"]`,
      );
      await app.waitFor(async () => (await switchState()) === "true", {
        description: "sharing switch turning on",
      });

      // Dropping every profile leaves nothing to do, and the primary action
      // must not offer to do it.
      await app.clickSelector('[data-testid="distribute-toggle-profiles"]');
      await app.waitFor(
        async () => (await distributionSummary(app)).includes("0 of 0"),
        { description: "summary after clearing the profiles" },
      );
      assert.equal(
        await app.execute(
          `const nodes = [...document.querySelectorAll("button")];
           const node = nodes.find((n) => (n.innerText ?? "").trim().includes(arguments[0]));
           return node ? node.disabled : null;`,
          [en.proxyDistribution.distributeButton],
        ),
        true,
        "an empty plan must not offer a Distribute button that does nothing",
      );

      await dismissSurface(app);
    },
    { seedDownloadedBrowser: true },
  );
});

test("the group bookmark editor adds, reorders and removes a row", async () => {
  await withApp("ui-group-bookmarks", async (app) => {
    const group = await app.invoke("create_profile_group", { name: "Client" });

    await app.clickSelector('[aria-label="Groups"]');
    await app.waitForText("Client");
    await app.clickSelector('[data-testid="group-bookmarks-button"]');
    await app.waitForText(en.groupBookmarks.description);
    assert.ok(await app.visibleTextIncludes(en.groupBookmarks.empty));

    const rowCount = () =>
      app.execute(
        `return document.querySelectorAll('[data-testid="group-bookmark-row"]').length;`,
      );
    const titles = () =>
      app.execute(
        `return [...document.querySelectorAll('[data-testid="group-bookmark-title"]')].map((n) => n.value);`,
      );
    const rowSelector = (index, testid) =>
      `[data-testid="group-bookmark-rows"] > div:nth-child(${index}) [data-testid="${testid}"]`;

    await app.clickSelector('[data-testid="group-bookmark-add"]');
    await app.waitFor(async () => (await rowCount()) === 1, {
      description: "the first bookmark row",
    });
    await app.clickSelector('[data-testid="group-bookmark-add"]');
    await app.waitFor(async () => (await rowCount()) === 2, {
      description: "the second bookmark row",
    });
    await app.clickSelector('[data-testid="group-bookmark-add"]');
    await app.waitFor(async () => (await rowCount()) === 3, {
      description: "the third bookmark row",
    });

    await app.fillSelector(rowSelector(1, "group-bookmark-title"), "Support");
    await app.fillSelector(
      rowSelector(1, "group-bookmark-url"),
      "https://support.example",
    );
    await app.fillSelector(rowSelector(2, "group-bookmark-title"), "Console");
    await app.fillSelector(
      rowSelector(2, "group-bookmark-url"),
      "https://console.example",
    );
    await app.fillSelector(rowSelector(2, "group-bookmark-folder"), "Ops");
    await app.fillSelector(rowSelector(3, "group-bookmark-title"), "Scratch");
    await app.fillSelector(
      rowSelector(3, "group-bookmark-url"),
      "https://scratch.example",
    );
    assert.deepEqual(await titles(), ["Support", "Console", "Scratch"]);

    // Reorder: the second row moves above the first.
    await app.clickSelector(rowSelector(2, "group-bookmark-move-up"));
    await app.waitFor(async () => (await titles())[0] === "Console", {
      description: "the reordered first row",
    });
    assert.deepEqual(await titles(), ["Console", "Support", "Scratch"]);

    // Remove: the last row goes away and nothing else moves.
    await app.clickSelector(rowSelector(3, "group-bookmark-remove"));
    await app.waitFor(async () => (await rowCount()) === 2, {
      description: "the removed row",
    });
    assert.deepEqual(await titles(), ["Console", "Support"]);

    await app.clickText(en.common.buttons.save);
    await app.waitFor(
      async () =>
        (await app.invoke("get_group_bookmarks", { groupId: group.id }))
          .length === 2,
      { description: "the saved bookmark list" },
    );
    assert.deepEqual(
      await app.invoke("get_group_bookmarks", { groupId: group.id }),
      [
        { title: "Console", url: "https://console.example", folder: "Ops" },
        { title: "Support", url: "https://support.example" },
      ],
    );

    await dismissSurface(app);
  });
});

test("importing from a link validates the input and names the extension before it is saved", async () => {
  await withApp("ui-extension-from-link", async (app) => {
    const fixtureBase = process.env.DONUT_E2E_FIXTURE_URL;
    assert.ok(fixtureBase, "the fixture server URL has to reach the suite");

    await openExtensionsPage(app);
    await app.clickSelector(`[aria-label="${EXTENSION_STRINGS.fromUrl}"]`);
    await app.waitFor(
      () =>
        app.execute(
          `return Boolean(document.querySelector("#ext-url-input"));`,
        ),
      { description: "the link import form" },
    );

    // Nothing to fetch yet, so the action is not offered.
    assert.equal(
      await app.execute(
        `return [...document.querySelectorAll("button")]
           .find((button) => (button.textContent || "").trim() === arguments[0])
           ?.disabled ?? null;`,
        [EXTENSION_STRINGS.fetchExtension],
      ),
      true,
      "Fetch has to stay disabled until there is something to fetch",
    );

    // A link that is not an extension source is refused, in the user's
    // language, and nothing is staged.
    await app.fillSelector("#ext-url-input", "https://example.invalid/page");
    await app.clickText(EXTENSION_STRINGS.fetchExtension, {
      roles: ["button"],
    });
    await app.waitFor(
      async () =>
        (await toastTexts(app)).some((text) =>
          text.includes(en.backendErrors.extensionUrlInvalid),
        ),
      { description: "the refusal for a link that is not an extension" },
    );
    assert.equal(
      await app.execute(
        `return document.querySelector('[data-slot="extension-fetched-identity"]') === null;`,
      ),
      true,
      "a refused link must not stage anything",
    );

    // The real thing: a CRX3 the fixture server serves. The staged form has to
    // show the identity read out of the archive's own manifest, not the file
    // name, before the user commits to storing it.
    await app.fillSelector("#ext-url-input", `${fixtureBase}/extension.crx`);
    await app.clickText(EXTENSION_STRINGS.fetchExtension, {
      roles: ["button"],
    });
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector('[data-slot="extension-fetched-identity"]')?.innerText?.trim() ?? null;`,
        ),
      { description: "the parsed identity of the downloaded extension" },
    );
    const identity = await app.execute(
      `return document.querySelector('[data-slot="extension-fetched-identity"]').innerText.trim();`,
    );
    assert.ok(
      identity.includes(CRX_EXTENSION_NAME),
      `the staged identity has to name the extension, got: ${identity}`,
    );
    assert.ok(
      identity.includes(CRX_EXTENSION_VERSION),
      `the staged identity has to carry the version, got: ${identity}`,
    );
    assert.ok(
      await app.visibleTextIncludes(`${fixtureBase}/extension.crx`),
      "the staged import has to name where the package came from",
    );
    assert.deepEqual(
      await app.invoke("list_extensions"),
      [],
      "fetching stages the archive; it must not store it",
    );

    await app.clickText(en.common.buttons.add, { roles: ["button"] });
    await app.waitForText(CRX_EXTENSION_NAME);
    const stored = await app.invoke("list_extensions");
    assert.equal(stored.length, 1);
    assert.equal(stored[0].name, CRX_EXTENSION_NAME);
    assert.equal(stored[0].version, CRX_EXTENSION_VERSION);
    assert.equal(stored[0].file_type, "zip");
    assert.equal(stored[0].source_kind, "archive");
  });
});

test("a checked proxy shows its UDP verdict in the table and its check trail in the details", async () => {
  await withApp("ui-proxy-check-trail", async (app) => {
    const proxy = await app.invoke("create_stored_proxy", {
      name: "Trail HTTP Proxy",
      proxySettings: {
        proxy_type: "http",
        host: "127.0.0.1",
        // Discard port: the check fails fast, which is a real check outcome
        // and exactly what the trail has to be able to show.
        port: 9,
        username: null,
        password: null,
      },
    });
    await app.invokeError("check_proxy_validity", {
      proxyId: proxy.id,
      proxySettings: null,
    });

    await app.clickSelector('[aria-label="Network"]');
    await app.waitForText(proxy.name);

    // An HTTP proxy cannot carry a datagram, so the table says so without
    // anyone opening anything.
    await app.waitFor(
      async () =>
        (await app.execute(
          `return document.querySelector('[data-slot="proxy-udp-verdict"]')?.dataset?.udp ?? null;`,
        )) === "no",
      { description: "the UDP verdict cell" },
    );
    assert.equal(
      (
        await app.execute(
          `return document.querySelector('[data-slot="proxy-udp-verdict"]').innerText.trim();`,
        )
      ).toLowerCase(),
      en.proxyCheck.udpNo.toLowerCase(),
    );

    await app.clickSelector(`[aria-label="${en.appFeedback.routeDetails}"]`);
    await app.waitFor(
      () =>
        app.execute(
          `return Boolean(document.querySelector('[data-slot="proxy-check-history"]'));`,
        ),
      { description: "the check trail" },
    );
    await app.waitFor(
      async () =>
        (await app.execute(
          `return document.querySelectorAll('[data-slot="proxy-check-history-entry"]').length;`,
        )) >= 1,
      { description: "at least one remembered check" },
    );

    const trail = await app.execute(
      `return [...document.querySelectorAll('[data-slot="proxy-check-history-entry"]')]
         .map((entry) => entry.innerText.replace(/\\s+/g, " ").trim());`,
    );
    assert.ok(trail.length >= 1);
    assert.ok(
      trail[0].includes(en.proxyCheck.historyFailed),
      `the newest line has to report the failure, got: ${trail[0]}`,
    );
    assert.ok(
      await app.visibleTextIncludes(en.proxyCheck.historyTitle),
      "the trail needs a heading that says what it is",
    );
  });
});

/**
 * Answer one Tauri command from inside the webview.
 *
 * Same seam as {@link stubFolderPicker}: the synchroniser panel needs a live
 * session to control, and a real one launches browsers. Every other command
 * still reaches the real backend, so the panel is exercised as it ships.
 */
async function stubCommand(app, command, reply) {
  await app.execute(
    `const wanted = arguments[0];
     const reply = arguments[1];
     if (!window.__donutOriginalFetch) {
       window.__donutOriginalFetch = window.fetch;
     }
     window.__donutStubbedCalls = window.__donutStubbedCalls ?? [];
     window.__donutStubs = window.__donutStubs ?? {};
     window.__donutStubs[wanted] = reply;
     if (!window.__donutStubInstalled) {
       window.__donutStubInstalled = true;
       window.fetch = function (input, init) {
         const url = String(
           typeof input === "string" ? input : (input && input.url) || "",
         );
         let name = "";
         try {
           name = decodeURIComponent(url.split("/").pop() || "");
         } catch (_error) {
           name = "";
         }
         if (window.__donutStubs[name] !== undefined) {
           let payload = null;
           try {
             payload = JSON.parse((init && init.body) || "null");
           } catch (_error) {
             payload = null;
           }
           window.__donutStubbedCalls.push({ command: name, payload });
           return Promise.resolve(
             new Response(JSON.stringify(window.__donutStubs[name]), {
               status: 200,
               headers: {
                 "content-type": "application/json",
                 "Tauri-Response": "ok",
               },
             }),
           );
         }
         return window.__donutOriginalFetch.apply(window, arguments);
       };
     }
     return true;`,
    [command, reply],
  );
}

async function restoreStubs(app) {
  return app.execute(
    `const calls = window.__donutStubbedCalls ?? [];
     if (window.__donutOriginalFetch) {
       window.fetch = window.__donutOriginalFetch;
       delete window.__donutOriginalFetch;
     }
     delete window.__donutStubbedCalls;
     delete window.__donutStubs;
     delete window.__donutStubInstalled;
     return calls;`,
  );
}

const SYNC = en.profiles.synchronizer;

test("the synchroniser panel lists a live session and its controls act on the real state", async () => {
  await withApp("ui-synchronizer-panel", async (app) => {
    const session = {
      id: "ui-panel-session",
      leader_profile_id: "leader-id",
      leader_profile_name: "Panel leader",
      paused: false,
      followers: [
        {
          profile_id: "follower-one",
          profile_name: "Panel follower one",
          failed_at_url: null,
          held: false,
        },
        {
          profile_id: "follower-two",
          profile_name: "Panel follower two",
          failed_at_url: "https://example.invalid/lost",
          held: false,
        },
      ],
    };
    const panel = '[data-slot="synchronizer-panel"]';
    const followerState = (id) =>
      app.execute(
        `const row = document.querySelector('[data-slot="synchronizer-panel-follower"][data-profile-id="' + arguments[0] + '"]');
         if (!row) return null;
         return {
           held: row.dataset.held === "true",
           badge: row.querySelector('[data-slot="synchronizer-panel-follower-state"]')?.innerText?.trim() ?? null,
           holdLabel: row.querySelector('[data-slot="synchronizer-panel-hold"]')?.innerText?.trim() ?? null,
         };`,
        [id],
      );

    try {
      // The profiles page has to be mounted before any of this means
      // anything: an event emitted before the hook subscribes is simply gone.
      await app.waitFor(
        () =>
          app.execute(
            `return Boolean(document.querySelector('[data-slot="profile-workspace"]'));`,
          ),
        { description: "the profiles page" },
      );
      assert.equal(
        await app.execute(
          `return Boolean(document.querySelector(arguments[0]));`,
          [panel],
        ),
        false,
        "no session, no panel",
      );

      // Re-emitted until it lands, because subscribing is asynchronous and a
      // missed event is indistinguishable from a broken panel.
      await app.waitFor(
        async () => {
          await app.invoke("plugin:event|emit", {
            event: "sync-session-changed",
            payload: session,
          });
          return app.execute(
            `return Boolean(document.querySelector(arguments[0]));`,
            [panel],
          );
        },
        { description: "the session panel" },
      );

      assert.equal(
        await app.execute(
          `return document.querySelector('[data-slot="synchronizer-panel-leader"]').innerText.trim();`,
        ),
        session.leader_profile_name,
      );
      assert.deepEqual(
        await app.execute(
          `return [...document.querySelectorAll('[data-slot="synchronizer-panel-follower"]')]
             .map((row) => row.dataset.profileId);`,
        ),
        ["follower-one", "follower-two"],
        "followers stay in the order they were chosen",
      );
      assert.deepEqual(await followerState("follower-one"), {
        held: false,
        badge: SYNC.stateMirroring,
        holdLabel: SYNC.holdOut,
      });
      // The desynced follower reports the failure instead of a state badge.
      assert.equal(await app.visibleTextIncludes(SYNC.stateDesynced), true);
      await app.capture("synchronizer-panel");

      // The backend has no such session, so pausing must fail and the panel
      // must keep telling the truth rather than flipping hopefully.
      await app.clickSelector('[data-slot="synchronizer-panel-pause"]');
      await app.waitFor(
        () => app.visibleTextIncludes(en.backendErrors.syncSessionNotFound),
        { description: "the refusal surfaced to the user" },
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('[data-slot="synchronizer-panel-pause"]').innerText.trim();`,
        ),
        SYNC.pauseMirroring,
        "a refused pause must not read as paused",
      );
      assert.equal(
        await app.execute(
          `return Boolean(document.querySelector('[data-slot="synchronizer-panel-paused-note"]'));`,
        ),
        false,
      );

      // With the backend agreeing, the same click has to land.
      await stubCommand(app, "set_sync_session_paused", {
        ...session,
        paused: true,
      });
      await app.clickSelector('[data-slot="synchronizer-panel-pause"]');
      await app.waitFor(
        () =>
          app.execute(
            `return Boolean(document.querySelector('[data-slot="synchronizer-panel-paused-note"]'));`,
          ),
        { description: "the paused state" },
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('[data-slot="synchronizer-panel-pause"]').innerText.trim();`,
        ),
        SYNC.resumeMirroring,
      );
      assert.equal(
        (await followerState("follower-one")).badge,
        SYNC.statePaused,
      );
      await app.capture("synchronizer-panel-paused");

      // Holding one follower out leaves the other exactly as it was.
      await stubCommand(app, "set_sync_follower_held", {
        ...session,
        followers: [
          { ...session.followers[0], held: true },
          session.followers[1],
        ],
      });
      await app.clickSelector(
        '[data-slot="synchronizer-panel-follower"][data-profile-id="follower-one"] [data-slot="synchronizer-panel-hold"]',
      );
      await app.waitFor(
        async () => (await followerState("follower-one")).held === true,
        { description: "the held-out follower" },
      );
      assert.deepEqual(await followerState("follower-one"), {
        held: true,
        badge: SYNC.stateHeld,
        holdLabel: SYNC.rejoin,
      });

      // The layout the user picked is the layout the backend is asked for.
      await stubCommand(app, "arrange_sync_windows", session);
      await app.clickSelector('[data-slot="synchronizer-panel-layout"]');
      await app.clickText(SYNC.layout.cascade, {
        roles: ["option", "menuitem", "button"],
      });
      await app.waitFor(
        async () =>
          (await app.execute(
            `return document.querySelector('[data-slot="synchronizer-panel-layout"]').innerText.trim();`,
          )) === SYNC.layout.cascade,
        { description: "the chosen layout" },
      );
      await app.clickSelector('[data-slot="synchronizer-panel-arrange"]');
      await app.waitFor(
        async () =>
          (await app.execute(
            `return (window.__donutStubbedCalls ?? []).filter((call) => call.command === "arrange_sync_windows").length;`,
          )) === 1,
        { description: "the arrange request" },
      );
      const calls = await app.execute(
        `return window.__donutStubbedCalls.map((call) => call.command + ":" + JSON.stringify(call.payload));`,
      );
      const arrange = calls.find((call) =>
        call.startsWith("arrange_sync_windows"),
      );
      assert.match(arrange, /"layout":"cascade"/);
      assert.match(arrange, /"sessionId":"ui-panel-session"/);
      await app.capture("synchronizer-panel-arranged");

      await app.waitFor(
        async () => {
          await app.invoke("plugin:event|emit", {
            event: "sync-session-ended",
            payload: session.id,
          });
          return (
            (await app.execute(
              `return Boolean(document.querySelector(arguments[0]));`,
              [panel],
            )) === false
          );
        },
        { description: "the panel leaving with the session" },
      );
    } finally {
      await restoreStubs(app);
    }
  });
});

const TIPS_DIALOG = '[data-slot="tips-dialog"]';

async function openTipsFromRail(app) {
  await app.clickSelector('[aria-label="More"]');
  await app.waitFor(
    () =>
      app.execute(`return Boolean(document.querySelector("[role='menu']"));`),
    { description: "More menu" },
  );
  await app.clickSelector('[data-slot="rail-open-tips"]');
  await app.waitFor(
    () =>
      app.execute(
        `return document.querySelector(arguments[0])?.dataset.mode === "browse";`,
        [TIPS_DIALOG],
      ),
    { description: "the tips catalog" },
  );
}

test("tips open from the rail, walk the catalog, and deep-link into the feature", async () => {
  await withApp("ui-tips-browse", async (app) => {
    await openTipsFromRail(app);
    assert.ok(await app.visibleTextIncludes(en.tips.items.dnsBlocklist.title));

    // Every essential is listed; a plan tip needs a plan, and there is none.
    const listed = await app.execute(
      `return [...document.querySelectorAll('[data-slot="tips-list-item"]')].map((node) => node.dataset.tipId);`,
    );
    assert.ok(listed.includes("dnsBlocklist"));
    assert.ok(listed.includes("trash"));
    assert.ok(!listed.includes("cookieBot"));
    assert.ok(!listed.includes("team"));

    // The drawing is live SVG in the scene panel, not a picture.
    assert.equal(
      await app.execute(
        `return document.querySelectorAll('[data-slot="tip-scene-panel"] svg[data-slot="tip-scene"]').length;`,
      ),
      1,
    );

    await app.clickSelector('[data-slot="tip-next"]');
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector('[data-slot="tip-detail"]')?.dataset.tipId === "proxyCheck";`,
        ),
      { description: "the second tip" },
    );
    assert.ok(await app.visibleTextIncludes(en.tips.items.proxyCheck.title));

    // Picking from the catalog moves the sliding indicator onto that entry.
    await app.clickSelector(
      '[data-slot="tips-list-item"][data-tip-id="trash"]',
    );
    await app.waitFor(
      () =>
        app.execute(
          `return Boolean(document.querySelector('[data-slot="tips-list-item"][data-tip-id="trash"] [data-slot="tips-list-indicator"]'));`,
        ),
      { description: "the indicator on the chosen tip" },
    );
    assert.ok(await app.visibleTextIncludes(en.tips.items.trash.title));
    assert.equal(
      await app.execute(
        `return document.querySelectorAll('[data-slot="tips-list-indicator"]').length;`,
      ),
      1,
      "exactly one entry is marked current",
    );
    await app.capture("tips-browse");

    // Seen tips are remembered, so the automatic flow never repeats them.
    await app.waitFor(
      async () => {
        const state = await app.invoke("get_tips_state");
        return ["dnsBlocklist", "proxyCheck", "trash"].every((id) =>
          state.seen.includes(id),
        );
      },
      { description: "seen tips persisted" },
    );

    // The action lands inside the feature: the DNS tip opens settings on
    // its DNS section, and the dialog is gone by then.
    await app.clickSelector(
      '[data-slot="tips-list-item"][data-tip-id="dnsBlocklist"]',
    );
    await app.clickSelector('[data-slot="tip-action"]');
    await app.waitFor(
      () =>
        app.execute(
          `return document.activeElement?.dataset?.settingsSection === "dns";`,
        ),
      { description: "the DNS settings section focused" },
    );
    assert.equal(
      await app.execute(
        `return Boolean(document.querySelector(arguments[0]));`,
        [TIPS_DIALOG],
      ),
      false,
    );

    // The chord opens the catalog too, and the switch turns the automatic
    // flow off and persists that.
    await app.pressShortcut({
      ...(process.platform === "darwin" ? { meta: true } : { ctrl: true }),
      shift: true,
      key: "h",
    });
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector(arguments[0])?.dataset.mode === "browse";`,
          [TIPS_DIALOG],
        ),
      { description: "the tips catalog from the keyboard" },
    );
    assert.equal(
      await app.execute(
        `return document.querySelector('[data-slot="tips-auto-show"]').getAttribute("data-state");`,
      ),
      "unchecked",
      "the harness seeds the automatic flow off",
    );
    await app.clickSelector('[data-slot="tips-auto-show"]');
    await app.waitFor(
      async () => (await app.invoke("get_tips_state")).auto_show === true,
      { description: "the preference persisted" },
    );
    await dismissSurface(app);
    await app.waitFor(
      () =>
        app.execute(`return !document.querySelector(arguments[0]);`, [
          TIPS_DIALOG,
        ]),
      { description: "the dialog closed" },
    );
  });
});

test("one tip for an unused feature opens by itself as a single card, then waits 2 to 5 days", async () => {
  // Every tip but two is seen already, and the only profile blocks ads, so
  // the DNS tip is in use and the trash tip is the one the automatic flow
  // may open. The flow is switched on only after that profile exists.
  const unseen = ["dnsBlocklist", "trash"];
  const seen = Object.keys(en.tips.items).filter((id) => !unseen.includes(id));
  await withApp(
    "ui-tips-auto",
    async (app) => {
      const profile = await createUiProfile(app, "Tips Blocker");
      await app.invoke("update_profile_dns_blocklist", {
        profileId: profile.id,
        dnsBlocklist: "light",
      });
      await app.invoke("set_tips_auto_show", { enabled: true });
      await app.restart();

      await app.waitFor(
        () =>
          app.execute(
            `return document.querySelector(arguments[0])?.dataset.mode === "single";`,
            [TIPS_DIALOG],
          ),
        { description: "the automatic tip", timeoutMs: 30_000 },
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('[data-slot="tip-detail"]')?.dataset.tipId;`,
        ),
        "trash",
        "the DNS tip is skipped because a profile already blocks ads",
      );
      assert.ok(await app.visibleTextIncludes(en.tips.items.trash.title));
      assert.deepEqual(
        await app.execute(
          `return ['tips-list-item', 'tip-previous', 'tip-next'].map((slot) => document.querySelectorAll('[data-slot="' + slot + '"]').length);`,
        ),
        [0, 0, 0],
        "one tip at a time: no catalog and no pager",
      );
      assert.equal(
        await app.execute(
          `return document.querySelector('[data-slot="tip-advance"]')?.textContent.trim();`,
        ),
        en.tips.done,
      );
      await app.capture("tips-auto");

      const state = await app.waitFor(
        async () => {
          const current = await app.invoke("get_tips_state");
          return current.seen.includes("trash") && current.auto_due === false
            ? current
            : null;
        },
        { description: "the automatic tip recorded" },
      );
      assert.ok(
        !state.seen.includes("dnsBlocklist"),
        "the skipped tip stays unseen",
      );
      const gap = state.next_auto_show_at - state.last_auto_shown_at;
      assert.ok(
        gap >= 2 * 24 * 60 * 60 && gap <= 5 * 24 * 60 * 60,
        `the next automatic tip waits 2 to 5 days, not ${gap} seconds`,
      );

      // Done closes the card instead of walking to another tip.
      await app.clickSelector('[data-slot="tip-advance"]');
      await app.waitFor(
        () =>
          app.execute(`return !document.querySelector(arguments[0]);`, [
            TIPS_DIALOG,
          ]),
        { description: "the dialog closed" },
      );

      // A restart inside the gap shows nothing, and keeps the same wait.
      await app.restart();
      await app.waitForText("Tips Blocker");
      await new Promise((resolve) => setTimeout(resolve, 4_500));
      assert.equal(
        await app.execute(
          `return Boolean(document.querySelector(arguments[0]));`,
          [TIPS_DIALOG],
        ),
        false,
      );
      assert.equal(
        (await app.invoke("get_tips_state")).next_auto_show_at,
        state.next_auto_show_at,
      );
    },
    { settings: { tips_seen: seen } },
  );
});

test("a freshly paid account is welcomed once and walked to its plan tips", async () => {
  await withApp("ui-paid-welcome", async (app) => {
    await app.waitForText("No profiles yet");
    try {
      // A pro account that signed in a moment ago, as the desktop would hold
      // it after a device-code login. Only the IPC read of the cached user is
      // stubbed; the plan observation and the tips state run for real.
      await stubCommand(app, "cloud_get_user", {
        logged_in_at: new Date().toISOString(),
        user: {
          id: "ui-paid-welcome",
          email: "paid@example.test",
          plan: "pro",
          planPeriod: "monthly",
          subscriptionStatus: "active",
          profileLimit: 50,
          cloudProfilesUsed: 0,
          proxyBandwidthLimitMb: 0,
          proxyBandwidthUsedMb: 0,
          proxyBandwidthExtraMb: 0,
          isPrimaryDevice: true,
        },
      });
      await app.invoke("plugin:event|emit", {
        event: "cloud-auth-changed",
        payload: null,
      });
      await app.waitFor(
        () =>
          app.execute(
            `return Boolean(document.querySelector('[data-slot="paid-welcome"]'));`,
          ),
        { description: "the paid welcome" },
      );
      assert.ok(
        await app.visibleTextIncludes(
          en.paidWelcome.title.replace("{{plan}}", "Pro"),
        ),
      );
      assert.deepEqual(
        await app.execute(
          `return [...document.querySelectorAll('[data-slot="paid-welcome-item"]')].map((node) => node.dataset.tipId);`,
        ),
        [
          "cloudBackup",
          "cookieBot",
          "crossOs",
          "automation",
          "agent",
          "remoteControl",
        ],
        "every capability the plan grants, in catalog order, and nothing it lacks",
      );
      await app.capture("paid-welcome");

      // A row opens the catalog on that tip, with the plan tips now listed.
      await app.clickSelector(
        '[data-slot="paid-welcome-item"][data-tip-id="cookieBot"]',
      );
      await app.waitFor(
        () =>
          app.execute(
            `return document.querySelector('[data-slot="tip-detail"]')?.dataset.tipId === "cookieBot";`,
          ),
        { description: "the Cookie Bot tip" },
      );
      assert.equal(
        await app.execute(
          `return Boolean(document.querySelector('[data-slot="paid-welcome"]'));`,
        ),
        false,
      );
      const listed = await app.execute(
        `return [...document.querySelectorAll('[data-slot="tips-list-item"]')].map((node) => node.dataset.tipId);`,
      );
      assert.ok(listed.includes("cookieBot") && listed.includes("agent"));
      assert.ok(listed.includes("dnsBlocklist"));
      await app.capture("tips-plan-catalog");
      await dismissSurface(app);
      await app.waitFor(
        () =>
          app.execute(`return !document.querySelector(arguments[0]);`, [
            TIPS_DIALOG,
          ]),
        { description: "the catalog closed" },
      );

      // Greeted once: the same account signing in again is not welcomed twice.
      await app.invoke("plugin:event|emit", {
        event: "cloud-auth-changed",
        payload: null,
      });
      await new Promise((resolve) => setTimeout(resolve, 1_500));
      assert.equal(
        await app.execute(
          `return Boolean(document.querySelector('[data-slot="paid-welcome"]'));`,
        ),
        false,
      );
      assert.equal(
        await app.invoke("observe_cloud_plan", {
          userId: "ui-paid-welcome",
          paid: true,
          freshLogin: true,
        }),
        false,
      );
    } finally {
      await restoreStubs(app);
    }
  });
});

test("Pro users can edit and save launch arguments in each profile", async () => {
  await withApp(
    "ui-launch-args",
    async (app) => {
      const profile = await createUiProfile(app, "Profile Launch Arguments");
      await app.waitForText(profile.name);
      const openEditor = async () => {
        await app.clickSelector(
          `[data-slot="profile-inspect-trigger"][data-profile-id="${profile.id}"]`,
        );
        await app.clickSelector(
          '[data-slot="profile-info-section"][data-section="fingerprint"]',
        );
        await app.waitForText(en.fingerprint.launchArgs);
      };
      await openEditor();
      assert.equal(
        await app.execute(
          'return document.querySelector("#wayfern-launch-args").disabled;',
        ),
        true,
      );
      await app.clickSelector('[data-slot="profile-info-close"]');
      try {
        await stubCommand(app, "cloud_get_user", {
          logged_in_at: "2020-01-01T00:00:00Z",
          user: {
            id: "launch-args-pro",
            email: "launch-args@example.test",
            plan: "pro",
            planPeriod: "monthly",
            subscriptionStatus: "active",
            profileLimit: 50,
            isPrimaryDevice: true,
          },
        });
        await app.invoke("plugin:event|emit", {
          event: "cloud-auth-changed",
          payload: null,
        });
        await openEditor();
        await app.waitFor(
          () =>
            app.execute(
              'return !document.querySelector("#wayfern-launch-args").disabled;',
            ),
          { description: "Pro launch arguments editor" },
        );
        const input = "--disable-gpu\n--renderer-process-limit=4";
        await app.fillSelector("#wayfern-launch-args", input);
        assert.equal(
          await app.execute(
            'return document.querySelector("#wayfern-launch-args").value;',
          ),
          input,
        );
        await app.capture("launch-arguments-pro");
        await app.clickText(en.common.buttons.save, { exact: true });
        await app.waitFor(
          async () =>
            (await app.invoke("list_browser_profiles"))[0].wayfern_config
              .launch_args?.length === 2,
          { description: "saved launch arguments" },
        );
        await openEditor();
        assert.equal(
          await app.execute(
            'return document.querySelector("#wayfern-launch-args").value;',
          ),
          input,
        );
        await app.fillSelector(
          "#wayfern-launch-args",
          "--user-data-dir=/tmp/other",
        );
        await app.clickText(en.common.buttons.save, { exact: true });
        await app.waitForText(
          en.backendErrors.wayfernLaunchArgReserved.replace(
            "{{argument}}",
            "--user-data-dir",
          ),
        );
        assert.deepEqual(
          (await app.invoke("list_browser_profiles"))[0].wayfern_config
            .launch_args,
          input.split("\n"),
        );
        await app.clickSelector('[data-slot="profile-info-close"]');
        await restoreStubs(app);
        await app.invoke("plugin:event|emit", {
          event: "cloud-auth-changed",
          payload: null,
        });
        await openEditor();
        await app.waitFor(
          () =>
            app.execute(
              'return document.querySelector("#wayfern-launch-args").disabled;',
            ),
          { description: "launch arguments locked after sign out" },
        );
        await app.clickText(en.common.buttons.clear, { exact: true });
        await app.clickText(en.common.buttons.save, { exact: true });
        await app.waitFor(
          async () =>
            !(await app.invoke("list_browser_profiles"))[0].wayfern_config
              .launch_args?.length,
          { description: "cleared launch arguments" },
        );
      } finally {
        await restoreStubs(app);
      }
    },
    { extraEnv: { WAYFERN_TEST_TOKEN: "e2e-local-launch-args" } },
  );
});

test("the create dialog warns a free user who creates quickly and holds a capped plan at its hourly limit", async () => {
  await withApp(
    "ui-profile-hourly-limit",
    async (app) => {
      const createDisabled = () =>
        app.execute(
          `const dialog = document.querySelector('[role="dialog"]');
         const button = [...(dialog?.querySelectorAll('button') ?? [])]
           .find((node) => node.textContent.trim() === arguments[0]);
         return button ? button.disabled : null;`,
          [en.common.buttons.create],
        );

      // Free: past the allowance the dialog warns, and Create still works.
      await stubCommand(app, "get_profile_creation_allowance", {
        enforcement: "soft",
        per_hour: 8,
        used: 8,
        retry_after_secs: 1_200,
      });
      await openProfileCreation(app);
      await app.waitForText(en.createProfile.hourlyLimit.warningTitle);
      await app.waitForText(en.createProfile.hourlyLimit.warning);
      await app.fillSelector("#profile-name", "Quick profile");
      assert.equal(await createDisabled(), false);
      await app.clickTextIn('[role="dialog"]', en.common.buttons.cancel, {
        roles: ["button"],
      });
      await app.waitFor(
        () => app.execute(`return !document.querySelector('#profile-name');`),
        { description: "create dialog closed" },
      );

      // Solo at its cap: Create is disabled and its tooltip says why and when.
      await stubCommand(app, "get_profile_creation_allowance", {
        enforcement: "hard",
        per_hour: 10,
        used: 10,
        retry_after_secs: 1_200,
      });
      await openProfileCreation(app);
      await app.fillSelector("#profile-name", "Capped profile");
      await app.waitFor(async () => (await createDisabled()) === true, {
        description: "Create disabled at the hourly cap",
      });
      assert.equal(
        await app.execute(
          `return document.body.innerText.includes(arguments[0]);`,
          [en.createProfile.hourlyLimit.warningTitle],
        ),
        false,
        "a capped plan is refused, not warned",
      );

      const target = await app.execute(
        `const rect = document.querySelector('[data-slot="hourly-limit"]').getBoundingClientRect();
       return { x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) };`,
      );
      try {
        await app.session.command("POST", "/actions", {
          actions: [
            {
              type: "pointer",
              id: "hourly-limit-pointer",
              actions: [
                {
                  type: "pointerMove",
                  x: target.x,
                  y: target.y,
                  origin: "viewport",
                },
              ],
            },
          ],
        });
      } finally {
        await app.session.command("DELETE", "/actions");
      }
      await app.waitForText(
        en.createProfile.hourlyLimit.reached
          .replace("{{limit}}", "10")
          .replace("{{minutes}}", "20"),
      );
    },
    { seedDownloadedBrowser: true },
  );
});

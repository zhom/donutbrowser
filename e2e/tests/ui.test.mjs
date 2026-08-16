import assert from "node:assert/strict";
import { mkdir, realpath, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import Color from "color";
import en from "../../src/i18n/locales/en.json" with { type: "json" };
import { getDerivedThemeColors, THEMES } from "../../src/lib/themes.ts";
import { withApp } from "../lib/app.mjs";
import {
  extensionZipBase64,
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
  await app.clickSelector(triggerSelector);
  await app.clickText(option, { roles: ["option"] });
}

async function saveSettings(app) {
  await app.clickText("Save Settings", { roles: ["button"] });
  await app.waitFor(
    () =>
      app.execute(`return document.querySelector("#theme-select") === null;`),
    { description: "Settings to close after saving" },
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

test("VLESS proxy form keeps the share URI as one clear, validated input", async () => {
  await withApp("ui-vless-proxy-form", async (app) => {
    const uri =
      "vless://6d6e21a1-4829-4d2b-bc7f-1b25707b61e4@vpn.example.com:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.example.com&fp=chrome&pbk=BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc&sid=0123456789abcdef&type=tcp#E2E";

    await app.clickSelector('[aria-label="Network"]');
    await app.waitForText("New proxy");
    await app.clickSelector('[aria-label="New proxy"]');
    await app.waitForText("Add Proxy");
    await app.fillSelector("#proxy-name", "E2E VLESS");
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

    // A well-formed URI for a setup Donut cannot use must say WHICH part is
    // unsupported, rather than implying the user mistyped it.
    await app.fillSelector(
      "#proxy-vless-uri",
      `${uri.replace("type=tcp", "type=ws")}&path=%2Fray`,
    );
    await app.waitFor(
      () =>
        app.execute(
          `return /only|TCP|transport/i.test(
             document.querySelector("#proxy-vless-uri-help")?.textContent || ""
           );`,
        ),
      { description: "transport-specific unsupported message" },
    );

    await app.fillSelector("#proxy-vless-uri", uri);
    await app.waitFor(
      () =>
        app.execute(
          `return document.querySelector("#proxy-vless-uri")?.getAttribute("aria-invalid") === "false";`,
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
       return cells.length >= 7 && (cells[2].innerText || "").trim() === wanted;
     });
     ${body}`;
}

async function extensionRow(app, name) {
  return app.execute(
    extensionRowScript(`if (!row) return null;
     const cells = [...row.querySelectorAll("td")];
     const sync = row.querySelector('[data-slot="animated-switch"]');
     return {
       name: (cells[2].innerText || "").trim(),
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

import assert from "node:assert/strict";
import test from "node:test";
import { withApp } from "../lib/app.mjs";

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

const DRACULA_THEME = {
  "--background": "#282a36",
  "--foreground": "#f8f8f2",
  "--card": "#44475a",
  "--card-foreground": "#f8f8f2",
  "--popover": "#44475a",
  "--popover-foreground": "#f8f8f2",
  "--primary": "#bd93f9",
  "--primary-foreground": "#282a36",
  "--secondary": "#8be9fd",
  "--secondary-foreground": "#282a36",
  "--muted": "#6272a4",
  "--muted-foreground": "#f8f8f2",
  "--accent": "#ff79c6",
  "--accent-foreground": "#282a36",
  "--destructive": "#ff5555",
  "--destructive-foreground": "#f8f8f2",
  "--success": "#50fa7b",
  "--success-foreground": "#282a36",
  "--warning": "#ffb86c",
  "--warning-foreground": "#282a36",
  "--border": "#6272a4",
  "--chart-1": "#bd93f9",
  "--chart-2": "#50fa7b",
  "--chart-3": "#ff79c6",
  "--chart-4": "#8be9fd",
  "--chart-5": "#ffb86c",
};

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
  assert.deepEqual(
    pointerEvents.map((event) => event.type),
    ["pointermove", "pointerdown", "pointermove", "pointerup"],
  );
  assert.match(pointerEvents[1].target, /cursor-pointer/);
  assert.match(pointerEvents[2].target, /cursor-pointer/);
  assert.equal(pointerEvents[2].buttons, 1);
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

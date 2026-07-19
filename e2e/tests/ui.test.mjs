import assert from "node:assert/strict";
import test from "node:test";
import { withApp } from "../lib/app.mjs";

async function dismissSurface(app) {
  await app.pressShortcut({ key: "Escape" });
  await new Promise((resolve) => setTimeout(resolve, 100));
}

test("all primary navigation buttons and sub-page tabs render and remain interactive", async () => {
  await withApp("ui-navigation", async (app) => {
    await app.invoke("complete_onboarding");
    await app.restart();
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
    await app.invoke("complete_onboarding");
    await app.restart();
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

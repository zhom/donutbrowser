import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { appFromEnvironment, withApp } from "../lib/app.mjs";

test("fresh app renders, completes onboarding, persists settings, and never touches real app roots", async () => {
  await withApp(
    "smoke-fresh",
    async (app) => {
      assert.equal(typeof (await app.session.title()), "string");
      assert.match(await app.bodyText(), /New/);
      await app.waitForText("No profiles yet");

      const initial = await app.invoke("get_app_settings");
      assert.equal(typeof initial.onboarding_completed, "boolean");
      await app.invoke("complete_onboarding");
      assert.equal(await app.invoke("get_onboarding_completed"), true);
      await app.invoke("dismiss_window_resize_warning");
      assert.equal(
        await app.invoke("get_window_resize_warning_dismissed"),
        true,
      );

      const saved = await app.invoke("save_app_settings", {
        settings: {
          ...initial,
          theme: "dark",
          language: "en",
          onboarding_completed: true,
          disable_auto_updates: true,
        },
      });
      assert.equal(saved.theme, "dark");
      assert.equal(saved.language, "en");

      await app.invoke("save_table_sorting_settings", {
        sorting: { column: "browser", direction: "desc" },
      });
      assert.deepEqual(await app.invoke("get_table_sorting_settings"), {
        column: "browser",
        direction: "desc",
      });
      assert.ok((await app.invoke("get_system_language")).length >= 2);
      const system = await app.invoke("get_system_info");
      assert.ok(system && typeof system === "object");
      assert.equal(typeof (await app.invoke("read_log_files")), "string");

      await app.restart();
      const afterRestart = await app.invoke("get_app_settings");
      assert.equal(afterRestart.theme, "dark");
      assert.equal(afterRestart.language, "en");
      assert.equal(afterRestart.onboarding_completed, true);

      const settingsFile = path.join(
        app.dataRoot,
        "data",
        "settings",
        "app_settings.json",
      );
      await access(settingsFile);
      const persisted = JSON.parse(await readFile(settingsFile, "utf8"));
      assert.equal(persisted.api_token, null);
      assert.equal(persisted.mcp_token, null);
    },
    { onboardingCompleted: false },
  );
});

test("two isolated sessions run concurrently and do not share frontend or backend state", async () => {
  const first = appFromEnvironment("smoke-isolation-a");
  const second = appFromEnvironment("smoke-isolation-b");
  try {
    await Promise.all([first.start(), second.start()]);
    const firstSettings = await first.invoke("get_app_settings");
    await first.invoke("save_app_settings", {
      settings: { ...firstSettings, theme: "dark", onboarding_completed: true },
    });
    const secondSettings = await second.invoke("get_app_settings");
    assert.equal(secondSettings.theme, "system");
    assert.notEqual(secondSettings.theme, "dark");

    await first.execute("localStorage.setItem('donut-e2e-only-a', 'yes');");
    assert.equal(
      await second.execute("return localStorage.getItem('donut-e2e-only-a');"),
      null,
      "native WebView data leaked across sessions",
    );
  } catch (error) {
    await Promise.all([first.capture("failure"), second.capture("failure")]);
    throw error;
  } finally {
    await Promise.all([first.close(), second.close()]);
  }
});

test("keyboard command palette and major navigation surfaces are operable through native WebDriver", async () => {
  await withApp("smoke-ui", async (app) => {
    const modifier =
      process.platform === "darwin" ? { meta: true } : { ctrl: true };
    await app.waitFor(
      async () => {
        await app.pressShortcut({ key: "k", ...modifier });
        return app.execute(
          `return Boolean(document.querySelector("[cmdk-input][placeholder='Type a command or search...']"));`,
        );
      },
      { description: "open command palette" },
    );

    const input = await app.session.findCss("[cmdk-input]");
    await app.session.sendKeys(input, "settings");
    const body = await app.bodyText();
    assert.match(body, /Settings/i);

    // Exercise native WebDriver element marshalling and click, not just script execution.
    const close = await app.execute(
      `return [...document.querySelectorAll("button")].find(
        (button) => /close/i.test(button.getAttribute("aria-label") || button.textContent || "")
      ) ?? null;`,
    );
    if (close) {
      await app.session.click(close);
    } else {
      await app.pressShortcut({ key: "Escape" });
    }
  });
});

test("tray labels, hide-to-tray, and confirmed quit follow the native lifecycle", async () => {
  const app = appFromEnvironment("smoke-lifecycle");
  try {
    await app.start();
    await app.invoke("update_tray_menu", {
      showLabel: "Show Donut E2E",
      quitLabel: "Quit Donut E2E",
    });
    await app.invoke("hide_to_tray");
    assert.equal(
      typeof (await app.invoke("get_onboarding_completed")),
      "boolean",
    );

    await app.restart();
    const exitingSession = app.session;
    await app
      .execute(
        `window.__TAURI_INTERNALS__.invoke("confirm_quit").catch(() => {});
         return true;`,
      )
      .catch(() => {});
    await app.waitFor(
      async () => {
        try {
          await exitingSession.title();
          return false;
        } catch {
          return true;
        }
      },
      { timeoutMs: 10_000, description: "confirmed app exit" },
    );
    app.session = null;
    await exitingSession.close().catch(() => {});
  } catch (error) {
    await app.capture("failure");
    throw error;
  } finally {
    await app.close();
  }
});

import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { WebDriverClient } from "./webdriver.mjs";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function isolatedEnvironment(root, extra = {}) {
  const home = path.join(root, "home");
  const temp = path.join(root, "tmp");
  return {
    DONUTBROWSER_DATA_ROOT: path.join(root, "donut"),
    HOME: home,
    USERPROFILE: home,
    ...(process.platform === "darwin" ? { CFFIXED_USER_HOME: home } : {}),
    TMPDIR: temp,
    TMP: temp,
    TEMP: temp,
    XDG_CONFIG_HOME: path.join(root, "xdg", "config"),
    XDG_CACHE_HOME: path.join(root, "xdg", "cache"),
    XDG_DATA_HOME: path.join(root, "xdg", "data"),
    APPDATA: path.join(root, "windows", "roaming"),
    LOCALAPPDATA: path.join(root, "windows", "local"),
    LANG: "en_US.UTF-8",
    LC_ALL: "en_US.UTF-8",
    NO_PROXY: "127.0.0.1,localhost",
    no_proxy: "127.0.0.1,localhost",
    HTTP_PROXY: "",
    HTTPS_PROXY: "",
    ALL_PROXY: "",
    http_proxy: "",
    https_proxy: "",
    all_proxy: "",
    RUST_BACKTRACE: "1",
    ...extra,
  };
}

export class AppSession {
  constructor({
    name,
    root,
    application,
    driverUrl,
    cwd,
    token,
    extraEnv = {},
    args = [],
    seedVersionCache = true,
    onboardingCompleted = true,
    wayfernTermsAccepted = true,
  }) {
    this.name = name;
    this.root = root;
    this.application = application;
    this.driver = new WebDriverClient(driverUrl);
    this.cwd = cwd;
    this.token = token;
    this.extraEnv = extraEnv;
    this.args = args;
    this.seedVersionCache = seedVersionCache;
    this.onboardingCompleted = onboardingCompleted;
    this.wayfernTermsAccepted = wayfernTermsAccepted;
    this.session = null;
  }

  get dataRoot() {
    return path.join(this.root, "donut");
  }

  async start() {
    await Promise.all([
      mkdir(path.join(this.root, "home"), { recursive: true }),
      mkdir(path.join(this.root, "tmp"), { recursive: true }),
      mkdir(path.join(this.root, "artifacts"), { recursive: true }),
    ]);
    if (this.onboardingCompleted) {
      const settingsFile = path.join(
        this.dataRoot,
        "data",
        "settings",
        "app_settings.json",
      );
      await mkdir(path.dirname(settingsFile), { recursive: true });
      await writeFile(
        settingsFile,
        `${JSON.stringify(
          {
            language: "en",
            onboarding_completed: true,
            commercial_trial_acknowledged: true,
            window_resize_warning_dismissed: true,
            disable_auto_updates: true,
          },
          null,
          2,
        )}\n`,
        { flag: "wx" },
      ).catch((error) => {
        if (error.code !== "EEXIST") {
          throw error;
        }
      });
    }
    if (this.wayfernTermsAccepted) {
      const termsFile =
        process.platform === "darwin"
          ? path.join(
              this.root,
              "home",
              "Library",
              "Application Support",
              "Wayfern",
              "license-accepted",
            )
          : process.platform === "win32"
            ? path.join(
                this.root,
                "windows",
                "roaming",
                "Wayfern",
                "license-accepted",
              )
            : path.join(
                this.root,
                "xdg",
                "config",
                "Wayfern",
                "license-accepted",
              );
      await mkdir(path.dirname(termsFile), { recursive: true });
      await writeFile(termsFile, `${Math.floor(Date.now() / 1000)}\n`, {
        flag: "wx",
      }).catch((error) => {
        if (error.code !== "EEXIST") {
          throw error;
        }
      });
    }
    if (this.seedVersionCache) {
      const versionCache = path.join(
        this.root,
        "donut",
        "cache",
        "version_cache",
        "wayfern_versions.json",
      );
      await mkdir(path.dirname(versionCache), { recursive: true });
      await writeFile(
        versionCache,
        `${JSON.stringify({
          releases: [{ version: "150.0.7871.100", date: "2026-07-01" }],
          timestamp: Math.floor(Date.now() / 1000),
        })}\n`,
        { flag: "wx" },
      ).catch((error) => {
        if (error.code !== "EEXIST") {
          throw error;
        }
      });
    }
    const env = isolatedEnvironment(this.root, {
      DONUT_E2E_DISABLE_STARTUP_NETWORK: "1",
      ...(process.env.DONUT_E2E_FIXTURE_URL
        ? {
            DONUT_E2E_DNS_BLOCKLIST_BASE_URL: `${process.env.DONUT_E2E_FIXTURE_URL}/dns`,
            ...(process.env.DONUT_E2E_GEOIP_FIXTURE_READY === "1"
              ? {
                  DONUT_E2E_GEOIP_DOWNLOAD_URL: `${process.env.DONUT_E2E_FIXTURE_URL}/geoip.mmdb`,
                }
              : {}),
          }
        : {}),
      ...(this.token ? { WAYFERN_TEST_TOKEN: this.token } : {}),
      ...this.extraEnv,
    });
    this.session = await this.driver.createSession({
      application: this.application,
      args: this.args,
      env,
      cwd: this.cwd,
      startupTimeout: 120_000,
    });
    await this.session.setTimeouts();
    await this.waitFor(
      async () => {
        const ready = await this.execute(
          "return document.readyState === 'complete' && Boolean(window.__TAURI_INTERNALS__);",
        );
        return ready === true;
      },
      {
        description: `${this.name} frontend and Tauri bridge`,
        timeoutMs: 60_000,
      },
    );
    return this;
  }

  async restart() {
    await this.close();
    return this.start();
  }

  async execute(script, args = []) {
    assert.ok(this.session, `${this.name} is not started`);
    return this.session.execute(script, args);
  }

  async invoke(command, args = {}) {
    assert.ok(this.session, `${this.name} is not started`);
    const result = await this.session.executeAsync(
      `
        const done = arguments[arguments.length - 1];
        const command = arguments[0];
        const args = arguments[1];
        window.__TAURI_INTERNALS__.invoke(command, args)
          .then((value) => done({ ok: true, value }))
          .catch((error) => done({
            ok: false,
            error: typeof error === "string" ? error : (error?.message ?? JSON.stringify(error))
          }));
      `,
      [command, args],
    );
    if (!result?.ok) {
      throw new Error(
        `Tauri command ${command} failed: ${result?.error ?? "unknown error"}`,
      );
    }
    return result.value;
  }

  async invokeError(command, args = {}) {
    try {
      await this.invoke(command, args);
    } catch (error) {
      return String(error);
    }
    throw new Error(`Expected Tauri command ${command} to fail`);
  }

  async bodyText() {
    return this.execute("return document.body?.innerText ?? '';");
  }

  async html() {
    return this.execute("return document.documentElement?.outerHTML ?? '';");
  }

  async visibleTextIncludes(text) {
    return this.execute(
      `
        const wanted = arguments[0];
        return [...document.querySelectorAll("body *")].some((node) => {
          const style = getComputedStyle(node);
          const rect = node.getBoundingClientRect();
          return style.visibility !== "hidden" && style.display !== "none" &&
            rect.width > 0 && rect.height > 0 &&
            (node.innerText ?? "").trim().includes(wanted);
        });
      `,
      [text],
    );
  }

  async waitFor(
    check,
    { timeoutMs = 20_000, intervalMs = 100, description = "condition" } = {},
  ) {
    const started = Date.now();
    let lastError;
    while (Date.now() - started < timeoutMs) {
      try {
        const value = await check();
        if (value) {
          return value;
        }
      } catch (error) {
        lastError = error;
      }
      await sleep(intervalMs);
    }
    throw new Error(
      `Timed out after ${timeoutMs}ms waiting for ${description}${lastError ? `: ${lastError}` : ""}`,
    );
  }

  async waitForText(text, timeoutMs = 20_000) {
    return this.waitFor(() => this.visibleTextIncludes(text), {
      timeoutMs,
      description: `visible text ${JSON.stringify(text)}`,
    });
  }

  async clickText(
    text,
    { exact = true, roles = ["button", "tab", "menuitem", "link"] } = {},
  ) {
    const element = await this.execute(
      `
        const wanted = arguments[0];
        const exact = arguments[1];
        const roles = new Set(arguments[2]);
        const candidates = [...document.querySelectorAll("button, a, [role], [data-slot='button']")];
        const visible = (node) => {
          const style = getComputedStyle(node);
          const rect = node.getBoundingClientRect();
          return style.visibility !== "hidden" && style.display !== "none" &&
            rect.width > 0 && rect.height > 0;
        };
        return candidates.find((node) => {
          const role = node.getAttribute("role") || (node.tagName === "A" ? "link" : "button");
          const label = (node.getAttribute("aria-label") || node.innerText || node.textContent || "").trim();
          return roles.has(role) && visible(node) && (exact ? label === wanted : label.includes(wanted));
        }) ?? null;
      `,
      [text, exact, roles],
    );
    assert.ok(
      element,
      `No visible interactive element matched ${JSON.stringify(text)}`,
    );
    await this.session.click(element);
  }

  async clickTextIn(
    containerSelector,
    text,
    { exact = true, roles = ["button", "tab", "menuitem", "link"] } = {},
  ) {
    const element = await this.execute(
      `
        const containers = [...document.querySelectorAll(arguments[0])];
        const wanted = arguments[1];
        const exact = arguments[2];
        const roles = new Set(arguments[3]);
        const visible = (node) => {
          const style = getComputedStyle(node);
          const rect = node.getBoundingClientRect();
          return style.visibility !== "hidden" && style.display !== "none" &&
            rect.width > 0 && rect.height > 0;
        };
        for (const container of containers.reverse()) {
          if (!visible(container)) continue;
          const candidates = [...container.querySelectorAll("button, a, [role], [data-slot='button']")];
          const match = candidates.find((node) => {
            const role = node.getAttribute("role") || (node.tagName === "A" ? "link" : "button");
            const label = (node.getAttribute("aria-label") || node.innerText || node.textContent || "").trim();
            return roles.has(role) && visible(node) && (exact ? label === wanted : label.includes(wanted));
          });
          if (match) return match;
        }
        return null;
      `,
      [containerSelector, text, exact, roles],
    );
    assert.ok(
      element,
      `No visible interactive element inside ${containerSelector} matched ${JSON.stringify(text)}`,
    );
    await this.session.click(element);
  }

  async clickSelector(selector) {
    const element = await this.waitFor(
      () =>
        this.execute(
          `
            const node = document.querySelector(arguments[0]);
            if (!node) return null;
            const style = getComputedStyle(node);
            const rect = node.getBoundingClientRect();
            return style.visibility !== "hidden" && style.display !== "none" &&
              rect.width > 0 && rect.height > 0 ? node : null;
          `,
          [selector],
        ),
      { description: `visible selector ${selector}` },
    );
    await this.session.click(element);
  }

  async fillSelector(selector, value) {
    const element = await this.waitFor(
      () =>
        this.execute("return document.querySelector(arguments[0]);", [
          selector,
        ]),
      { description: `selector ${selector}` },
    );
    await this.session.clear(element);
    await this.session.sendKeys(element, value);
  }

  async pressShortcut({
    key,
    meta = false,
    ctrl = false,
    alt = false,
    shift = false,
  }) {
    await this.execute(
      `
        window.dispatchEvent(new KeyboardEvent("keydown", {
          key: arguments[0],
          code: arguments[1],
          metaKey: arguments[2],
          ctrlKey: arguments[3],
          altKey: arguments[4],
          shiftKey: arguments[5],
          bubbles: true,
          cancelable: true
        }));
      `,
      [
        key,
        key.length === 1 ? `Key${key.toUpperCase()}` : key,
        meta,
        ctrl,
        alt,
        shift,
      ],
    );
  }

  async capture(label) {
    if (!this.session) {
      return;
    }
    const safe = label.replace(/[^a-z0-9_.-]+/gi, "-");
    try {
      const png = await this.session.screenshot();
      await writeFile(
        path.join(this.root, "artifacts", `${safe}.png`),
        Buffer.from(png, "base64"),
      );
    } catch {
      // Best-effort diagnostics must never hide the original test failure.
    }
    try {
      await writeFile(
        path.join(this.root, "artifacts", `${safe}.html`),
        await this.html(),
      );
    } catch {
      // Best-effort diagnostics must never hide the original test failure.
    }
  }

  async close() {
    if (!this.session) {
      return;
    }
    const session = this.session;
    this.session = null;
    await session.close();
  }
}

export function appFromEnvironment(name, options = {}) {
  const runRoot = process.env.DONUT_E2E_RUN_ROOT;
  assert.ok(runRoot, "DONUT_E2E_RUN_ROOT is required");
  return new AppSession({
    name,
    root: options.root ?? path.join(runRoot, "sessions", name),
    application: process.env.DONUT_E2E_APP,
    driverUrl: process.env.DONUT_E2E_DRIVER_URL,
    cwd: process.env.DONUT_E2E_PROJECT_ROOT,
    token: process.env.WAYFERN_TEST_TOKEN,
    extraEnv: options.extraEnv,
    args: options.args,
    seedVersionCache: options.seedVersionCache,
    onboardingCompleted: options.onboardingCompleted,
    wayfernTermsAccepted: options.wayfernTermsAccepted,
  });
}

export async function withApp(name, callback, options = {}) {
  const app = appFromEnvironment(name, options);
  try {
    await app.start();
    return await callback(app);
  } catch (error) {
    await app.capture("failure");
    throw error;
  } finally {
    await app.close();
  }
}

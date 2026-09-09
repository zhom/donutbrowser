import assert from "node:assert/strict";
import http from "node:http";

export const ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf";

/**
 * One HTTP exchange with the driver, over `node:http` rather than `fetch`.
 *
 * `fetch` is undici, and undici gives every request a 300 s headers timeout
 * of its own. A long `execute/async` sends no headers until the script
 * completes, so a `download_browser` that pulls a 1 GB Wayfern build over a
 * slow link died at 300 s whatever `timeoutMs` asked for. `node:http` has no
 * such default, which leaves `timeoutMs` as the only clock.
 */
function exchange(method, url, body, timeoutMs) {
  return new Promise((resolve, reject) => {
    const payload = body === undefined ? undefined : JSON.stringify(body);
    const request = http.request(
      url,
      {
        method,
        headers:
          payload === undefined
            ? {}
            : {
                "content-type": "application/json",
                "content-length": Buffer.byteLength(payload),
              },
        signal: AbortSignal.timeout(timeoutMs),
      },
      (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("error", reject);
        response.on("end", () =>
          resolve({
            status: response.statusCode ?? 0,
            text: Buffer.concat(chunks).toString("utf8"),
          }),
        );
      },
    );
    request.on("error", (error) => {
      const timedOut =
        error?.name === "AbortError" || error?.name === "TimeoutError";
      reject(
        timedOut
          ? new Error(
              `WebDriver ${method} ${url} gave no response within ${timeoutMs}ms`,
              { cause: error },
            )
          : error,
      );
    });
    request.end(payload);
  });
}

export class WebDriverClient {
  constructor(baseUrl) {
    this.baseUrl = baseUrl.replace(/\/$/, "");
  }

  async request(method, pathname, body, timeoutMs = 330_000) {
    const { status, text } = await exchange(
      method,
      `${this.baseUrl}${pathname}`,
      body,
      timeoutMs,
    );
    let payload = null;
    if (text) {
      try {
        payload = JSON.parse(text);
      } catch {
        throw new Error(
          `WebDriver ${method} ${pathname} returned non-JSON HTTP ${status}: ${text.slice(0, 500)}`,
        );
      }
    }
    const error = payload?.value?.error;
    if (status < 200 || status >= 300) {
      const message = payload?.value?.message ?? text ?? `HTTP ${status}`;
      throw new Error(
        `WebDriver ${method} ${pathname} failed (${error ?? status}): ${message}`,
      );
    }
    return payload?.value;
  }

  async status() {
    return this.request("GET", "/status");
  }

  async createSession({
    application,
    args = [],
    env = {},
    cwd,
    startupTimeout = 90_000,
    headless = false,
  }) {
    const options = { application, args, env, startupTimeout };
    if (cwd) {
      options.cwd = cwd;
    }
    // Only sent when asked, so a driver build without the capability is not
    // handed an option it would reject.
    if (headless) {
      options.headless = true;
    }
    const value = await this.request(
      "POST",
      "/session",
      {
        capabilities: {
          alwaysMatch: {
            "tauri:options": options,
          },
        },
      },
      startupTimeout + 10_000,
    );
    assert.ok(value?.sessionId, "WebDriver did not return a session id");
    return new WebDriverSession(
      this,
      value.sessionId,
      value.capabilities ?? {},
    );
  }
}

export class WebDriverSession {
  constructor(client, id, capabilities) {
    this.client = client;
    this.id = id;
    this.capabilities = capabilities;
    this.closed = false;
  }

  path(suffix = "") {
    return `/session/${encodeURIComponent(this.id)}${suffix}`;
  }

  async command(method, suffix, body, timeoutMs) {
    return this.client.request(method, this.path(suffix), body, timeoutMs);
  }

  async execute(script, args = []) {
    return this.command("POST", "/execute/sync", { script, args });
  }

  async executeAsync(script, args = [], timeoutMs = 330_000) {
    return this.command("POST", "/execute/async", { script, args }, timeoutMs);
  }

  async setTimeouts({
    implicit = 0,
    pageLoad = 300_000,
    script = 300_000,
  } = {}) {
    await this.command("POST", "/timeouts", { implicit, pageLoad, script });
  }

  async find(using, value) {
    const element = await this.command("POST", "/element", { using, value });
    assert.ok(
      element?.[ELEMENT_KEY],
      `Element not found using ${using}: ${value}`,
    );
    return element;
  }

  async findCss(selector) {
    return this.find("css selector", selector);
  }

  async findXpath(xpath) {
    return this.find("xpath", xpath);
  }

  async click(element) {
    await this.command(
      "POST",
      `/element/${encodeURIComponent(element[ELEMENT_KEY])}/click`,
      {},
    );
  }

  async sendKeys(element, text) {
    const chars = [...String(text)];
    await this.command(
      "POST",
      `/element/${encodeURIComponent(element[ELEMENT_KEY])}/value`,
      {
        text: String(text),
        value: chars,
      },
    );
  }

  async clear(element) {
    await this.command(
      "POST",
      `/element/${encodeURIComponent(element[ELEMENT_KEY])}/clear`,
      {},
    );
  }

  async title() {
    return this.command("GET", "/title");
  }

  async screenshot() {
    return this.command("GET", "/screenshot");
  }

  async close() {
    if (this.closed) {
      return;
    }
    this.closed = true;
    try {
      await this.command("DELETE", "");
    } catch (error) {
      if (!String(error).includes("invalid session id")) {
        throw error;
      }
    }
  }
}

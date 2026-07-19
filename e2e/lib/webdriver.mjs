import assert from "node:assert/strict";

export const ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecf";

function abortAfter(timeoutMs) {
  return AbortSignal.timeout(timeoutMs);
}

export class WebDriverClient {
  constructor(baseUrl) {
    this.baseUrl = baseUrl.replace(/\/$/, "");
  }

  async request(method, pathname, body, timeoutMs = 330_000) {
    const response = await fetch(`${this.baseUrl}${pathname}`, {
      method,
      headers:
        body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: abortAfter(timeoutMs),
    });
    const text = await response.text();
    let payload = null;
    if (text) {
      try {
        payload = JSON.parse(text);
      } catch {
        throw new Error(
          `WebDriver ${method} ${pathname} returned non-JSON HTTP ${response.status}: ${text.slice(0, 500)}`,
        );
      }
    }
    const error = payload?.value?.error;
    if (!response.ok) {
      const message =
        payload?.value?.message ?? text ?? `HTTP ${response.status}`;
      throw new Error(
        `WebDriver ${method} ${pathname} failed (${error ?? response.status}): ${message}`,
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
  }) {
    const options = { application, args, env, startupTimeout };
    if (cwd) {
      options.cwd = cwd;
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

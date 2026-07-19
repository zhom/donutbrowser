import assert from "node:assert/strict";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export class CdpClient {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data));
      if (message.id === undefined) return;
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) {
        pending.reject(
          new Error(
            `CDP ${pending.method} failed: ${JSON.stringify(message.error)}`,
          ),
        );
      } else {
        pending.resolve(message.result ?? {});
      }
    });
    socket.addEventListener("close", () => {
      for (const pending of this.pending.values()) {
        pending.reject(
          new Error(`CDP socket closed while waiting for ${pending.method}`),
        );
      }
      this.pending.clear();
    });
  }

  static async connect(port, { timeoutMs = 30_000 } = {}) {
    assert.equal(
      typeof WebSocket,
      "function",
      "This E2E suite requires Node.js 22+ WebSocket",
    );
    const started = Date.now();
    let lastError;
    while (Date.now() - started < timeoutMs) {
      try {
        const response = await fetch(`http://127.0.0.1:${port}/json`, {
          signal: AbortSignal.timeout(1_000),
        });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const targets = await response.json();
        const target = targets.find(
          (item) => item.type === "page" && item.webSocketDebuggerUrl,
        );
        if (!target) throw new Error("no debuggable page target");
        const socket = new WebSocket(target.webSocketDebuggerUrl);
        await new Promise((resolve, reject) => {
          const timeout = setTimeout(
            () => reject(new Error("CDP WebSocket open timed out")),
            5_000,
          );
          socket.addEventListener(
            "open",
            () => {
              clearTimeout(timeout);
              resolve();
            },
            { once: true },
          );
          socket.addEventListener(
            "error",
            () => {
              clearTimeout(timeout);
              reject(new Error("CDP WebSocket failed to open"));
            },
            { once: true },
          );
        });
        return new CdpClient(socket);
      } catch (error) {
        lastError = error;
        await sleep(100);
      }
    }
    throw new Error(
      `Timed out connecting to Wayfern CDP on ${port}: ${lastError}`,
    );
  }

  command(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate(expression) {
    const result = await this.command("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    });
    if (result.exceptionDetails) {
      throw new Error(
        `CDP evaluation failed: ${JSON.stringify(result.exceptionDetails)}`,
      );
    }
    return result.result?.value;
  }

  async waitFor(
    expression,
    { timeoutMs = 20_000, description = expression } = {},
  ) {
    const started = Date.now();
    let lastError;
    while (Date.now() - started < timeoutMs) {
      try {
        const value = await this.evaluate(expression);
        if (value) return value;
      } catch (error) {
        lastError = error;
      }
      await sleep(100);
    }
    throw new Error(
      `Timed out waiting for ${description}${lastError ? `: ${lastError}` : ""}`,
    );
  }

  close() {
    this.socket.close();
  }
}

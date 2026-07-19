import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import test from "node:test";
import { allCoveredCommands, commandCoverage } from "../coverage-map.mjs";
import { WebDriverClient } from "../lib/webdriver.mjs";

function registeredCommands(source) {
  const match = source.match(
    /invoke_handler\(tauri::generate_handler!\[(.*?)\]\)/s,
  );
  assert.ok(match, "Could not locate Tauri generate_handler! command registry");
  const withoutComments = match[1].replace(/\/\/[^\n]*/g, "");
  return [
    ...withoutComments.matchAll(/([A-Za-z_]\w*(?:::[A-Za-z_]\w*)*)\s*,/g),
  ].map((item) => item[1]);
}

function commandHasExecutableEvidence(source, command) {
  const name = command
    .split("::")
    .at(-1)
    .replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(
    `(?:invoke|invokeError)\\(\\s*["']${name}["']|invokeContract\\(\\s*\\w+\\s*,\\s*["']${name}["']`,
  ).test(source);
}

test("every Tauri command has exactly one E2E owner and evidence level", async () => {
  const root =
    process.env.DONUT_E2E_PROJECT_ROOT ??
    path.resolve(import.meta.dirname, "../..");
  const source = await readFile(
    path.join(root, "src-tauri", "src", "lib.rs"),
    "utf8",
  );
  const registered = registeredCommands(source);
  const covered = allCoveredCommands();
  assert.deepEqual(
    [...new Set(covered)].sort(),
    covered.slice().sort(),
    "The E2E coverage map contains duplicate command ownership",
  );
  assert.deepEqual(covered.slice().sort(), registered.slice().sort());

  for (const [name, entry] of Object.entries(commandCoverage)) {
    assert.ok(
      ["integration", "contract", "host-mutating"].includes(entry.level),
      name,
    );
    assert.ok(entry.commands.length > 0, `${name} has no commands`);
    if (entry.level === "host-mutating") {
      assert.ok(
        entry.reason?.length > 80,
        `${name} needs an explicit safety reason`,
      );
      continue;
    }

    const suiteSource = await readFile(
      path.join(root, "e2e", "tests", `${entry.suite}.test.mjs`),
      "utf8",
    );
    for (const command of entry.commands) {
      assert.equal(
        commandHasExecutableEvidence(suiteSource, command),
        true,
        `${command} is assigned to ${entry.suite} but has no executable invoke evidence`,
      );
    }
  }
});

test("WebDriver client preserves application values that contain an error field", async () => {
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(
      JSON.stringify({ value: { ok: false, error: "application error" } }),
    );
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  try {
    const address = server.address();
    const client = new WebDriverClient(`http://127.0.0.1:${address.port}`);
    assert.deepEqual(await client.request("GET", "/value"), {
      ok: false,
      error: "application error",
    });
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});

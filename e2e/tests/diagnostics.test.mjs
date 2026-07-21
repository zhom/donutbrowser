import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { after, test } from "node:test";
import { redactIssueBody } from "../../scripts/redact-sensitive-text.mjs";
import { createSafeDiagnostics } from "../lib/diagnostics.mjs";

const roots = [];
after(async () => {
  await Promise.all(
    roots.map((root) => rm(root, { recursive: true, force: true })),
  );
});

test("shared E2E diagnostics contain only redacted text logs", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "donut-diagnostics-test-"));
  roots.push(root);
  const secretUrl = "http://real-user:real-password@proxy.example:8080";
  const token = ["github", "pat", "example", "token", "0123456789"].join("_");
  const logText = [
    `proxy=${secretUrl}`,
    `Authorization: Bearer ${token}`,
    "visited https://example.com/callback?code=private-code",
    "exit IP 203.0.113.42",
    "home /Users/private-person/Library/Application Support",
    "email private.person@example.com",
    "PrivateKey = wireguard-private-key",
  ].join("\n");

  await Promise.all([
    mkdir(path.join(root, "logs"), { recursive: true }),
    mkdir(path.join(root, "sessions", "network", "donut", "logs"), {
      recursive: true,
    }),
    mkdir(path.join(root, "sessions", "network", "donut", "data", "proxies"), {
      recursive: true,
    }),
    mkdir(path.join(root, "sessions", "network", "artifacts"), {
      recursive: true,
    }),
  ]);
  await Promise.all([
    writeFile(path.join(root, "logs", "driver.log"), logText),
    writeFile(
      path.join(root, "sessions", "network", "donut", "logs", "app.log"),
      logText,
    ),
    writeFile(
      path.join(
        root,
        "sessions",
        "network",
        "donut",
        "data",
        "proxies",
        "real.json",
      ),
      JSON.stringify({ upstream_url: secretUrl, token }),
    ),
    writeFile(
      path.join(root, "sessions", "network", "artifacts", "page.html"),
      `<html>${secretUrl}</html>`,
    ),
  ]);

  const diagnostics = await createSafeDiagnostics(root, {
    suite: "network",
    failed: true,
    sensitiveValues: [secretUrl, token],
  });
  const files = await readdir(diagnostics);
  assert.deepEqual(files.sort(), ["001.log", "002.log", "summary.json"]);
  const combined = (
    await Promise.all(
      files.map((file) => readFile(path.join(diagnostics, file), "utf8")),
    )
  ).join("\n");
  for (const value of [
    secretUrl,
    "real-user",
    "real-password",
    "proxy.example",
    token,
    "private-code",
    "203.0.113.42",
    "private-person",
    "private.person@example.com",
    "wireguard-private-key",
  ]) {
    assert.ok(!combined.includes(value), `diagnostics leaked ${value}`);
  }
  assert.ok(
    !files.some(
      (file) => /\.(?:html|json)$/u.test(file) && file !== "summary.json",
    ),
  );
});

test("automated issue processing omits the complete log field", () => {
  const safe = redactIssueBody(
    `### What happened?\nA failure at user@example.com\n\n### Error logs or screenshots\nARBITRARY_PRIVATE_LOG_CONTENT\npassword=hunter2\n\n### Operating System\nLinux`,
  );
  assert.ok(!safe.includes("ARBITRARY_PRIVATE_LOG_CONTENT"));
  assert.ok(!safe.includes("hunter2"));
  assert.ok(!safe.includes("user@example.com"));
  assert.match(safe, /omitted from automated processing/u);
  assert.match(safe, /Operating System\nLinux/u);
});

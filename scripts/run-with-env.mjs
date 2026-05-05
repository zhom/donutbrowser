#!/usr/bin/env node
// Wrapper that loads `.env` into process.env (without overwriting anything
// already in the environment) and execs the given command. Used by the
// `tauri` npm script so `pnpm tauri build` picks up APPLE_SIGNING_IDENTITY,
// APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID etc. without requiring direnv.
//
// Plain shell `source .env` works on macOS/Linux but not Windows; this
// wrapper is platform-agnostic.

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const envPath = resolve(projectRoot, ".env");

if (existsSync(envPath)) {
  const content = readFileSync(envPath, "utf8");
  for (const rawLine of content.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq === -1) continue;
    const key = line.slice(0, eq).trim();
    let val = line.slice(eq + 1).trim();
    if (
      (val.startsWith('"') && val.endsWith('"')) ||
      (val.startsWith("'") && val.endsWith("'"))
    ) {
      val = val.slice(1, -1);
    }
    // Don't overwrite values already exported by the parent shell — direnv
    // / CI secrets / one-off `FOO=bar pnpm tauri ...` invocations win.
    if (process.env[key] === undefined) {
      process.env[key] = val;
    }
  }
}

const [, , cmd, ...args] = process.argv;
if (!cmd) {
  console.error("usage: run-with-env.mjs <command> [args...]");
  process.exit(2);
}

const child = spawn(cmd, args, { stdio: "inherit", shell: false });
child.on("error", (err) => {
  console.error(`Failed to spawn ${cmd}:`, err.message);
  process.exit(1);
});
child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 1);
  }
});

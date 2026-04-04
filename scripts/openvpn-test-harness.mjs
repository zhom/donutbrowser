#!/usr/bin/env node
/**
 * OpenVPN E2E Test Harness
 *
 * This script:
 * 1. Skips unless explicitly enabled via DONUTBROWSER_RUN_OPENVPN_E2E=1
 * 2. Builds the Rust vpn_integration test binary without running it
 * 3. Runs the OpenVPN e2e test binary under sudo
 *
 * Usage: DONUTBROWSER_RUN_OPENVPN_E2E=1 node scripts/openvpn-test-harness.mjs
 */

import { spawn } from "child_process";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT_DIR = path.resolve(__dirname, "..");
const SRC_TAURI_DIR = path.join(ROOT_DIR, "src-tauri");
const TEST_NAME = "test_openvpn_traffic_flows_through_donut_proxy";

function log(message) {
  console.log(`[openvpn-harness] ${message}`);
}

function error(message) {
  console.error(`[openvpn-harness] ERROR: ${message}`);
}

function shouldRun() {
  if (process.env.DONUTBROWSER_RUN_OPENVPN_E2E !== "1") {
    log("Skipping OpenVPN e2e test because DONUTBROWSER_RUN_OPENVPN_E2E is not set");
    return false;
  }

  if (process.platform !== "linux") {
    log(`Skipping OpenVPN e2e test on unsupported platform: ${process.platform}`);
    return false;
  }

  return true;
}

async function buildTestBinary() {
  log("Building OpenVPN e2e test binary...");

  return new Promise((resolve, reject) => {
    let executablePath = "";
    let stdoutBuffer = "";

    const proc = spawn(
      "cargo",
      [
        "test",
        "--test",
        "vpn_integration",
        TEST_NAME,
        "--no-run",
        "--message-format=json",
      ],
      {
        cwd: SRC_TAURI_DIR,
        env: process.env,
        stdio: ["ignore", "pipe", "pipe"],
      }
    );

    const parseBuffer = (flush = false) => {
      const lines = stdoutBuffer.split("\n");
      const completeLines = flush ? lines : lines.slice(0, -1);
      stdoutBuffer = flush ? "" : lines.at(-1) ?? "";

      for (const line of completeLines.filter(Boolean)) {
        try {
          const message = JSON.parse(line);
          if (message.reason === "compiler-artifact" && message.executable) {
            executablePath = message.executable;
          }
        } catch {
          // Ignore non-JSON lines.
        }
      }
    };

    proc.stdout.on("data", (data) => {
      stdoutBuffer += data.toString();
      parseBuffer();
    });

    proc.stderr.on("data", (data) => {
      process.stderr.write(data);
    });

    proc.on("error", (err) => {
      reject(err);
    });

    proc.on("close", (code) => {
      parseBuffer(true);

      if (code !== 0) {
        reject(new Error(`cargo test --no-run exited with code ${code}`));
        return;
      }

      if (!executablePath) {
        reject(new Error("Could not determine the vpn_integration test binary path"));
        return;
      }

      resolve(path.isAbsolute(executablePath) ? executablePath : path.resolve(SRC_TAURI_DIR, executablePath));
    });
  });
}

async function runOpenVpnE2e(executablePath) {
  log("Running OpenVPN e2e test under sudo...");

  return new Promise((resolve, reject) => {
    const proc = spawn(
      "sudo",
      [
        "--preserve-env=CI,GITHUB_ACTIONS,VPN_TEST_OVPN_HOST,VPN_TEST_OVPN_PORT,DONUTBROWSER_RUN_OPENVPN_E2E",
        executablePath,
        TEST_NAME,
        "--exact",
        "--nocapture",
      ],
      {
        cwd: SRC_TAURI_DIR,
        env: process.env,
        stdio: "inherit",
      }
    );

    proc.on("error", (err) => {
      reject(err);
    });

    proc.on("close", (code) => {
      resolve(code ?? 1);
    });
  });
}

async function main() {
  if (!shouldRun()) {
    process.exit(0);
  }

  try {
    const executablePath = await buildTestBinary();
    const exitCode = await runOpenVpnE2e(executablePath);
    process.exit(exitCode);
  } catch (err) {
    error(err instanceof Error ? err.message : String(err));
    process.exit(1);
  }
}

main();

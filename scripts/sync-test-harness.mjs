#!/usr/bin/env node
/**
 * Sync E2E Test Harness
 *
 * This script:
 * 1. Downloads and starts MinIO (S3-compatible storage)
 * 2. Builds and starts donut-sync server
 * 3. Runs the Rust sync e2e tests
 * 4. Cleans up all processes
 *
 * Usage: node scripts/sync-test-harness.mjs
 */

import { spawn, execSync } from "child_process";
import { createWriteStream, existsSync, mkdirSync, chmodSync } from "fs";
import { mkdir, rm, writeFile } from "fs/promises";
import http from "http";
import https from "https";
import os from "os";
import path from "path";
import { pipeline } from "stream/promises";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT_DIR = path.resolve(__dirname, "..");
const CACHE_DIR = path.join(ROOT_DIR, ".cache", "sync-test");

const MINIO_PORT = 9876;
const MINIO_CONSOLE_PORT = 9877;
const SYNC_PORT = 3456;
const SYNC_TOKEN = "test-sync-token";

const processes = [];

function log(msg) {
  console.log(`[sync-harness] ${msg}`);
}

function error(msg) {
  console.error(`[sync-harness] ERROR: ${msg}`);
}

async function downloadFile(url, dest) {
  return new Promise((resolve, reject) => {
    const file = createWriteStream(dest);
    const protocol = url.startsWith("https") ? https : http;

    protocol
      .get(url, (response) => {
        if (response.statusCode === 302 || response.statusCode === 301) {
          downloadFile(response.headers.location, dest)
            .then(resolve)
            .catch(reject);
          return;
        }

        if (response.statusCode !== 200) {
          reject(new Error(`Failed to download: ${response.statusCode}`));
          return;
        }

        pipeline(response, file).then(resolve).catch(reject);
      })
      .on("error", reject);
  });
}

function getMinioUrl() {
  const platform = os.platform();
  const arch = os.arch();

  if (platform === "darwin") {
    if (arch === "arm64") {
      return "https://dl.min.io/server/minio/release/darwin-arm64/minio";
    }
    return "https://dl.min.io/server/minio/release/darwin-amd64/minio";
  } else if (platform === "linux") {
    if (arch === "arm64") {
      return "https://dl.min.io/server/minio/release/linux-arm64/minio";
    }
    return "https://dl.min.io/server/minio/release/linux-amd64/minio";
  } else if (platform === "win32") {
    return "https://dl.min.io/server/minio/release/windows-amd64/minio.exe";
  }

  throw new Error(`Unsupported platform: ${platform}-${arch}`);
}

async function ensureMinioBinary() {
  const isWindows = os.platform() === "win32";
  const minioBin = path.join(CACHE_DIR, isWindows ? "minio.exe" : "minio");

  if (existsSync(minioBin)) {
    log("MinIO binary already cached");
    return minioBin;
  }

  log("Downloading MinIO binary...");
  mkdirSync(CACHE_DIR, { recursive: true });

  const url = getMinioUrl();
  await downloadFile(url, minioBin);
  if (!isWindows) {
    chmodSync(minioBin, 0o755);
  }

  log("MinIO binary downloaded");
  return minioBin;
}

async function startMinio(minioBin) {
  const dataDir = path.join(CACHE_DIR, "minio-data");
  await mkdir(dataDir, { recursive: true });

  log(`Starting MinIO on port ${MINIO_PORT}...`);

  const proc = spawn(
    minioBin,
    ["server", dataDir, "--address", `:${MINIO_PORT}`, "--console-address", `:${MINIO_CONSOLE_PORT}`],
    {
      env: {
        ...process.env,
        MINIO_ROOT_USER: "minioadmin",
        MINIO_ROOT_PASSWORD: "minioadmin",
      },
      stdio: ["ignore", "pipe", "pipe"],
    }
  );

  processes.push(proc);

  proc.stdout.on("data", (data) => {
    if (process.env.VERBOSE) {
      console.log(`[minio] ${data.toString().trim()}`);
    }
  });

  proc.stderr.on("data", (data) => {
    if (process.env.VERBOSE) {
      console.error(`[minio] ${data.toString().trim()}`);
    }
  });

  proc.on("error", (err) => {
    error(`MinIO error: ${err.message}`);
  });

  await waitForHealth(`http://localhost:${MINIO_PORT}/minio/health/live`, 30000);
  log("MinIO is ready");

  return proc;
}

async function buildDonutSync() {
  log("Building donut-sync...");
  execSync("pnpm build", {
    cwd: path.join(ROOT_DIR, "donut-sync"),
    stdio: process.env.VERBOSE ? "inherit" : "ignore",
  });
  log("donut-sync built");
}

async function startDonutSync() {
  log(`Starting donut-sync on port ${SYNC_PORT}...`);

  const proc = spawn("node", ["dist/main.js"], {
    cwd: path.join(ROOT_DIR, "donut-sync"),
    env: {
      ...process.env,
      PORT: String(SYNC_PORT),
      SYNC_TOKEN,
      S3_ENDPOINT: `http://localhost:${MINIO_PORT}`,
      S3_ACCESS_KEY_ID: "minioadmin",
      S3_SECRET_ACCESS_KEY: "minioadmin",
      S3_BUCKET: "donut-sync-test",
      S3_FORCE_PATH_STYLE: "true",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  processes.push(proc);

  proc.stdout.on("data", (data) => {
    if (process.env.VERBOSE) {
      console.log(`[donut-sync] ${data.toString().trim()}`);
    }
  });

  proc.stderr.on("data", (data) => {
    if (process.env.VERBOSE) {
      console.error(`[donut-sync] ${data.toString().trim()}`);
    }
  });

  proc.on("error", (err) => {
    error(`donut-sync error: ${err.message}`);
  });

  await waitForHealth(`http://localhost:${SYNC_PORT}/health`, 30000);
  log("donut-sync is ready");

  return proc;
}

async function waitForHealth(url, timeoutMs) {
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    try {
      await new Promise((resolve, reject) => {
        http
          .get(url, (res) => {
            if (res.statusCode === 200) {
              resolve();
            } else {
              reject(new Error(`Status ${res.statusCode}`));
            }
          })
          .on("error", reject);
      });
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 500));
    }
  }

  throw new Error(`Timeout waiting for ${url}`);
}

async function runTests() {
  log("Running Rust sync e2e tests...");

  return new Promise((resolve) => {
    const proc = spawn("cargo", ["test", "--test", "sync_e2e", "--", "--test-threads=1"], {
      cwd: path.join(ROOT_DIR, "src-tauri"),
      env: {
        ...process.env,
        SYNC_SERVER_URL: `http://localhost:${SYNC_PORT}`,
        SYNC_TOKEN,
      },
      stdio: "inherit",
    });

    proc.on("close", (code) => {
      resolve(code || 0);
    });
  });
}

function cleanup() {
  log("Cleaning up...");

  for (const proc of processes) {
    try {
      if (os.platform() === "win32") {
        // On Windows, SIGTERM is not supported; use taskkill for reliable cleanup
        try {
          execSync(`taskkill /F /T /PID ${proc.pid}`, { stdio: "ignore" });
        } catch {
          // Process may already be dead
        }
      } else {
        proc.kill("SIGTERM");
      }
    } catch {
      // Already dead
    }
  }
}

async function main() {
  process.on("SIGINT", () => {
    cleanup();
    process.exit(130);
  });

  process.on("SIGTERM", () => {
    cleanup();
    process.exit(143);
  });

  try {
    const minioBin = await ensureMinioBinary();
    await startMinio(minioBin);
    await buildDonutSync();
    await startDonutSync();

    const exitCode = await runTests();

    cleanup();
    process.exit(exitCode);
  } catch (err) {
    error(err.message);
    cleanup();
    process.exit(1);
  }
}

main();


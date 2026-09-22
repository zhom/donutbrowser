#!/usr/bin/env node
/**
 * Sync E2E Test Harness
 *
 * This script:
 * 1. Downloads and starts rclone's S3 server (S3-compatible storage)
 * 2. Builds and starts donut-sync server
 * 3. Runs the Rust sync e2e tests
 * 4. Cleans up all processes
 *
 * Usage: node scripts/sync-test-harness.mjs
 */

import { spawn, spawnSync, execSync } from "child_process";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  chmodSync,
  renameSync,
} from "fs";
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

const S3_PORT = 9876;
const S3_ACCESS_KEY = "donuttestaccesskey";
const S3_SECRET_KEY = "donuttestsecretkey";
// Pinned so a fresh rclone release cannot change what CI runs underneath us.
const RCLONE_VERSION = "v1.75.1";
const SYNC_PORT = 3456;
// Must be >= 24 chars and not a known default — the server's validateEnv()
// rejects short/placeholder tokens and exits at startup otherwise.
const SYNC_TOKEN = "test-sync-token-0123456789abcdef";

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
          file.close();
          downloadFile(response.headers.location, dest)
            .then(resolve)
            .catch(reject);
          return;
        }

        if (response.statusCode !== 200) {
          file.close();
          reject(new Error(`Failed to download: ${response.statusCode}`));
          return;
        }

        // pipeline() resolves once the source ends but doesn't await the
        // destination fd closing. Linux refuses to exec a file whose write
        // fd is still open (ETXTBSY), so explicitly wait for 'close'.
        pipeline(response, file)
          .then(
            () =>
              new Promise((res, rej) => {
                if (file.closed) {
                  res();
                } else {
                  file.once("close", res);
                  file.once("error", rej);
                }
              }),
          )
          .then(resolve)
          .catch(reject);
      })
      .on("error", reject);
  });
}

function getRcloneAsset() {
  const platform = os.platform();
  const arch = os.arch();

  if (platform === "darwin") {
    return `rclone-${RCLONE_VERSION}-osx-${arch === "arm64" ? "arm64" : "amd64"}`;
  } else if (platform === "linux") {
    return `rclone-${RCLONE_VERSION}-linux-${arch === "arm64" ? "arm64" : "amd64"}`;
  } else if (platform === "win32") {
    return `rclone-${RCLONE_VERSION}-windows-amd64`;
  }

  throw new Error(`Unsupported platform: ${platform}-${arch}`);
}

// Mirrors src-tauri/download-xray.mjs: PowerShell owns zip extraction on
// Windows, `unzip` everywhere else. Both are present on every runner this
// harness targets.
function extractZip(archive, destinationDir) {
  if (os.platform() === "win32") {
    const result = spawnSync(
      "powershell",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Expand-Archive -LiteralPath $env:DONUT_RCLONE_ARCHIVE -DestinationPath $env:DONUT_RCLONE_DESTINATION -Force",
      ],
      {
        stdio: "inherit",
        env: {
          ...process.env,
          DONUT_RCLONE_ARCHIVE: archive,
          DONUT_RCLONE_DESTINATION: destinationDir,
        },
      },
    );
    if (result.status !== 0) {
      throw new Error("Failed to extract the rclone archive");
    }
    return;
  }

  const result = spawnSync(
    "unzip",
    ["-qq", "-j", "-o", archive, "*/rclone", "-d", destinationDir],
    { stdio: "inherit" },
  );
  if (result.status !== 0) {
    throw new Error("Failed to extract the rclone archive");
  }
}

async function ensureS3Binary() {
  const isWindows = os.platform() === "win32";
  const binName = isWindows ? "rclone.exe" : "rclone";
  const bin = path.join(CACHE_DIR, binName);

  if (existsSync(bin)) {
    log("rclone binary already cached");
    return bin;
  }

  log("Downloading rclone binary...");
  mkdirSync(CACHE_DIR, { recursive: true });

  const asset = getRcloneAsset();
  const archive = path.join(CACHE_DIR, `${asset}.zip`);
  await downloadFile(
    `https://github.com/rclone/rclone/releases/download/${RCLONE_VERSION}/${asset}.zip`,
    archive,
  );
  extractZip(archive, CACHE_DIR);

  if (isWindows) {
    // Expand-Archive keeps the archive's own directory, so lift the binary out.
    const nested = path.join(CACHE_DIR, asset, binName);
    if (existsSync(nested)) {
      renameSync(nested, bin);
    }
  }
  if (!existsSync(bin)) {
    throw new Error(`rclone archive did not contain ${binName}`);
  }
  if (!isWindows) {
    chmodSync(bin, 0o755);
  }

  log("rclone binary downloaded");
  return bin;
}

// MinIO used to play this part, but MinIO withdrew its community binaries and
// every dl.min.io path now answers 410, which took this job down. rclone still
// publishes a single binary per platform, and `rclone serve s3` speaks enough
// of the API for these tests: path-style addressing, a static key pair, and
// bucket creation (a bucket is a directory under the served root).
async function startS3(bin) {
  const dataDir = path.join(CACHE_DIR, "s3-data");
  await mkdir(dataDir, { recursive: true });

  log(`Starting rclone serve s3 on port ${S3_PORT}...`);

  const proc = spawn(
    bin,
    [
      "serve",
      "s3",
      dataDir,
      "--addr",
      `127.0.0.1:${S3_PORT}`,
      "--auth-key",
      `${S3_ACCESS_KEY},${S3_SECRET_KEY}`,
      "--force-path-style",
      "--vfs-cache-mode",
      "writes",
    ],
    {
      env: { ...process.env },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );

  processes.push(proc);

  proc.stdout.on("data", (data) => {
    if (process.env.VERBOSE) {
      console.log(`[s3] ${data.toString().trim()}`);
    }
  });

  let lastStderr = "";
  proc.stderr.on("data", (data) => {
    lastStderr = data.toString();
    if (process.env.VERBOSE) {
      console.error(`[s3] ${lastStderr.trim()}`);
    }
  });

  proc.on("error", (err) => {
    error(`rclone error: ${err.message}`);
  });

  // Without this, a server that refuses to start (a busy port, most often)
  // surfaces as a bare 30s timeout with the reason buried behind VERBOSE.
  let exitReason = null;
  proc.on("exit", (code) => {
    exitReason = `rclone exited with code ${code}: ${lastStderr.trim() || "no output"}`;
  });

  // No health endpoint: an unauthenticated list is answered once the listener
  // is up, and any HTTP status proves that much.
  await waitForListening(
    `http://127.0.0.1:${S3_PORT}/`,
    30000,
    () => exitReason,
  );
  log("S3 storage is ready");

  return proc;
}

async function buildDonutSync() {
  log("Building donut-sync...");
  // `nest build` runs incremental tsc, which silently skips emit when
  // tsconfig.build.tsbuildinfo says nothing changed — even if dist/ was
  // wiped. Drop the cache so we always produce a fresh dist.
  const syncDir = path.join(ROOT_DIR, "donut-sync");
  await rm(path.join(syncDir, "tsconfig.build.tsbuildinfo"), {
    force: true,
  });
  await rm(path.join(syncDir, "dist"), { recursive: true, force: true });
  execSync("pnpm build", {
    cwd: syncDir,
    stdio: process.env.VERBOSE ? "inherit" : "ignore",
  });
  if (!existsSync(path.join(syncDir, "dist", "main.js"))) {
    throw new Error("donut-sync build did not produce dist/main.js");
  }
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
      S3_ENDPOINT: `http://127.0.0.1:${S3_PORT}`,
      S3_ACCESS_KEY_ID: S3_ACCESS_KEY,
      S3_SECRET_ACCESS_KEY: S3_SECRET_KEY,
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

async function waitForListening(url, timeoutMs, failureReason = () => null) {
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    const reason = failureReason();
    if (reason) {
      throw new Error(reason);
    }
    try {
      await new Promise((resolve, reject) => {
        // A per-attempt timeout matters: something else holding the port can
        // accept the connection and then never answer, and without this the
        // loop would wait on that one request forever instead of noticing
        // that our own server is gone.
        const req = http.get(url, (res) => {
          res.resume();
          resolve();
        });
        req.setTimeout(2000, () => {
          req.destroy(new Error("probe timed out"));
        });
        req.on("error", reject);
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
    const s3Bin = await ensureS3Binary();
    await startS3(s3Bin);
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


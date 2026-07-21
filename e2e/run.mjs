#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import {
  createReadStream,
  createWriteStream,
  existsSync,
  statSync,
} from "node:fs";
import {
  chmod,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rename,
  rm,
} from "node:fs/promises";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import { createSafeDiagnostics } from "./lib/diagnostics.mjs";

const dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(dirname, "..");
const webdriverRoot = path.resolve(
  projectRoot,
  "../tauri-cross-platform-webdriver",
);
const isWindows = process.platform === "win32";
const executableSuffix = isWindows ? ".exe" : "";
const appBinary = path.join(
  projectRoot,
  "e2e",
  "app",
  "target",
  "debug",
  `donutbrowser-e2e${executableSuffix}`,
);
const driverBinary = path.join(
  webdriverRoot,
  "target",
  "debug",
  `tauri-wd${executableSuffix}`,
);

const suiteFiles = {
  smoke: ["diagnostics.test.mjs", "smoke.test.mjs", "coverage.test.mjs"],
  ui: ["ui.test.mjs"],
  entities: ["entities.test.mjs"],
  network: ["network.test.mjs"],
  integrations: ["integrations.test.mjs"],
  sync: ["sync.test.mjs"],
  browser: ["browser.test.mjs"],
  full: [
    "diagnostics.test.mjs",
    "coverage.test.mjs",
    "smoke.test.mjs",
    "ui.test.mjs",
    "entities.test.mjs",
    "network.test.mjs",
    "integrations.test.mjs",
    "sync.test.mjs",
    "browser.test.mjs",
  ],
};

function parseArgs(argv) {
  const options = {
    suite: "full",
    build: true,
    keep: process.env.DONUT_E2E_KEEP_ARTIFACTS === "1",
    verbose: process.env.DONUT_E2E_VERBOSE === "1",
  };
  for (const arg of argv) {
    if (arg.startsWith("--suite=")) {
      options.suite = arg.slice("--suite=".length);
    } else if (arg === "--no-build") {
      options.build = false;
    } else if (arg === "--keep") {
      options.keep = true;
    } else if (arg === "--verbose") {
      options.verbose = true;
    } else {
      throw new Error(`Unknown E2E option: ${arg}`);
    }
  }
  if (!suiteFiles[options.suite]) {
    throw new Error(
      `Unknown suite ${options.suite}; expected ${Object.keys(suiteFiles).join(", ")}`,
    );
  }
  return options;
}

function log(message) {
  process.stdout.write(`[donut-e2e] ${message}\n`);
}

function run(command, args, cwd, env = process.env) {
  log(`${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd, env, stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
}

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close(() => resolve(address.port));
    });
  });
}

async function waitForUrl(url, timeoutMs, processRecord) {
  const started = Date.now();
  let lastError;
  while (Date.now() - started < timeoutMs) {
    if (processRecord?.process.exitCode !== null) {
      throw new Error(
        `${processRecord.name} exited early with ${processRecord.process.exitCode}; see ${processRecord.logPath}`,
      );
    }
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1_000) });
      if (response.ok) {
        return;
      }
      lastError = new Error(`HTTP ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Timed out waiting for ${url}: ${lastError}`);
}

function startProcess(name, command, args, { cwd, env, runRoot, verbose }) {
  const logPath = path.join(runRoot, "logs", `${name}.log`);
  const stream = createWriteStream(logPath, { flags: "a" });
  const child = spawn(command, args, {
    cwd,
    env,
    detached: !isWindows,
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.pipe(stream, { end: false });
  child.stderr.pipe(stream, { end: false });
  if (verbose) {
    child.stdout.on("data", (chunk) =>
      process.stdout.write(`[${name}] ${chunk}`),
    );
    child.stderr.on("data", (chunk) =>
      process.stderr.write(`[${name}] ${chunk}`),
    );
  }
  child.on("error", (error) => {
    process.stderr.write(`[donut-e2e] ${name} process error: ${error}\n`);
  });
  return { name, process: child, stream, logPath };
}

async function stopProcess(record) {
  if (!record || record.process.exitCode !== null) {
    record?.stream.end();
    return;
  }
  if (isWindows) {
    spawnSync("taskkill", ["/PID", String(record.process.pid), "/T", "/F"], {
      stdio: "ignore",
    });
  } else {
    try {
      process.kill(-record.process.pid, "SIGTERM");
    } catch {
      // The process group may already be gone.
    }
  }
  await Promise.race([
    new Promise((resolve) => record.process.once("exit", resolve)),
    new Promise((resolve) => setTimeout(resolve, 5_000)),
  ]);
  if (record.process.exitCode === null && !isWindows) {
    try {
      process.kill(-record.process.pid, "SIGKILL");
    } catch {
      // The process group may already be gone.
    }
  }
  record.stream.end();
}

function unquoteEnvValue(value) {
  const trimmed = value.trim();
  const quote = trimmed[0];
  if ((quote === '"' || quote === "'") && trimmed.at(-1) === quote) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

async function loadLocalValues(names) {
  const values = Object.fromEntries(
    names
      .filter((name) => process.env[name])
      .map((name) => [name, process.env[name]]),
  );
  for (const file of [path.join(projectRoot, ".env")]) {
    try {
      const content = await readFile(file, "utf8");
      for (const name of names) {
        if (values[name]) continue;
        const escapedName = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const match = content.match(
          new RegExp(
            `^\\s*(?:export\\s+)?${escapedName}\\s*=\\s*(.+?)\\s*$`,
            "m",
          ),
        );
        if (match) {
          values[name] = unquoteEnvValue(match[1]);
        }
      }
    } catch {
      // Individual suites validate the secrets they require.
    }
  }
  return values;
}

function buildAll() {
  if (!existsSync(webdriverRoot)) {
    throw new Error(`Missing sibling webdriver repository: ${webdriverRoot}`);
  }
  run("pnpm", ["build"], projectRoot);
  run("pnpm", ["copy-proxy-binary"], projectRoot);
  run(
    "cargo",
    ["build", "--locked", "--manifest-path", "e2e/app/Cargo.toml"],
    projectRoot,
  );
  run(
    "cargo",
    ["build", "--package", "tauri-cross-platform-webdriver"],
    webdriverRoot,
  );
}

function startFixtureServer(geoIpFixture) {
  const server = http.createServer((request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    if (url.pathname === "/health") {
      response.writeHead(200, { "content-type": "text/plain" });
      response.end("ok");
      return;
    }
    if (url.pathname === "/api/echo") {
      const chunks = [];
      request.on("data", (chunk) => chunks.push(chunk));
      request.on("end", () => {
        response.writeHead(200, {
          "content-type": "application/json",
          "set-cookie": "donut_e2e=browser-ok; Path=/; SameSite=Lax",
        });
        response.end(
          JSON.stringify({
            method: request.method,
            body: Buffer.concat(chunks).toString("utf8"),
            userAgent: request.headers["user-agent"],
          }),
        );
      });
      return;
    }
    if (url.pathname.startsWith("/dns/")) {
      response.writeHead(200, {
        "content-type": "text/plain; charset=utf-8",
        "cache-control": "no-store",
      });
      response.end("ads.e2e.invalid\ntracker.e2e.invalid\n");
      return;
    }
    if (url.pathname === "/geoip.mmdb" && geoIpFixture) {
      response.writeHead(200, {
        "content-type": "application/octet-stream",
        "content-length": String(statSync(geoIpFixture).size),
      });
      createReadStream(geoIpFixture).pipe(response);
      return;
    }
    response.writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
    });
    response.end(`<!doctype html>
      <html>
        <head><title>Donut E2E Browser Fixture</title></head>
        <body>
          <h1 id="fixture-title">Donut E2E Browser Fixture</h1>
          <p id="path">${url.pathname}</p>
          <button id="fixture-button" onclick="this.dataset.clicked='yes'; this.textContent='Clicked'">Click fixture</button>
          <script>window.__fixtureReady = true;</script>
        </body>
      </html>`);
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

async function ensureGeoIpFixture() {
  if (process.env.DONUT_E2E_GEOIP_FIXTURE) {
    const fixture = path.resolve(process.env.DONUT_E2E_GEOIP_FIXTURE);
    if (!existsSync(fixture)) {
      throw new Error(`DONUT_E2E_GEOIP_FIXTURE does not exist: ${fixture}`);
    }
    return fixture;
  }
  const toolsDir = path.join(os.tmpdir(), "donut-e2e-tools");
  const fixture = path.join(toolsDir, "GeoLite2-City.mmdb");
  await mkdir(toolsDir, { recursive: true });
  if (existsSync(fixture)) return fixture;

  log("Downloading GeoLite City E2E dependency");
  const releases = await fetch(
    "https://api.github.com/repos/P3TERX/GeoLite.mmdb/releases",
    {
      headers: { "user-agent": "donut-browser-e2e" },
      signal: AbortSignal.timeout(30_000),
    },
  ).then((response) => {
    if (!response.ok) {
      throw new Error(
        `GeoLite release lookup failed with HTTP ${response.status}`,
      );
    }
    return response.json();
  });
  const url = releases
    .flatMap((release) => release.assets ?? [])
    .find((asset) => asset.name.endsWith("-City.mmdb"))?.browser_download_url;
  if (!url) throw new Error("No GeoLite City MMDB asset was found");
  const temporary = `${fixture}.${process.pid}.tmp`;
  await download(url, temporary);
  await rename(temporary, fixture);
  return fixture;
}

function minioUrl() {
  const arch = os.arch() === "arm64" ? "arm64" : "amd64";
  if (process.platform === "darwin") {
    return `https://dl.min.io/server/minio/release/darwin-${arch}/minio`;
  }
  if (process.platform === "linux") {
    return `https://dl.min.io/server/minio/release/linux-${arch}/minio`;
  }
  if (process.platform === "win32") {
    return "https://dl.min.io/server/minio/release/windows-amd64/minio.exe";
  }
  throw new Error(
    `Unsupported MinIO platform ${process.platform}-${os.arch()}`,
  );
}

async function download(url, destination) {
  const transport = url.startsWith("https:") ? https : http;
  await new Promise((resolve, reject) => {
    transport
      .get(url, (response) => {
        if ([301, 302, 307, 308].includes(response.statusCode)) {
          response.resume();
          download(
            new URL(response.headers.location, url).href,
            destination,
          ).then(resolve, reject);
          return;
        }
        if (response.statusCode !== 200) {
          response.resume();
          reject(
            new Error(`Failed to download ${url}: HTTP ${response.statusCode}`),
          );
          return;
        }
        const output = createWriteStream(destination, { mode: 0o755 });
        pipeline(response, output).then(resolve, reject);
      })
      .on("error", reject);
  });
}

async function ensureMinio() {
  if (process.env.DONUT_E2E_MINIO_BIN) {
    return path.resolve(process.env.DONUT_E2E_MINIO_BIN);
  }
  const existingHarnessBinary = path.join(
    projectRoot,
    ".cache",
    "sync-test",
    isWindows ? "minio.exe" : "minio",
  );
  if (existsSync(existingHarnessBinary)) {
    return existingHarnessBinary;
  }
  const toolsDir = path.join(os.tmpdir(), "donut-e2e-tools");
  const binary = path.join(
    toolsDir,
    `minio-${process.platform}-${os.arch()}${executableSuffix}`,
  );
  await mkdir(toolsDir, { recursive: true });
  if (!existsSync(binary)) {
    log("Downloading isolated MinIO test dependency");
    const temporary = `${binary}.${process.pid}.tmp`;
    await download(minioUrl(), temporary);
    await chmod(temporary, 0o755);
    await rm(binary, { force: true });
    await import("node:fs/promises").then(({ rename }) =>
      rename(temporary, binary),
    );
  }
  return binary;
}

async function startSyncInfrastructure(runRoot, options, records) {
  const minioBinary = await ensureMinio();
  const minioPort = await freePort();
  const minioConsolePort = await freePort();
  const syncPort = await freePort();
  const syncToken = "donut-e2e-sync-token-0123456789abcdef";
  const minio = startProcess(
    "minio",
    minioBinary,
    [
      "server",
      path.join(runRoot, "minio-data"),
      "--address",
      `127.0.0.1:${minioPort}`,
      "--console-address",
      `127.0.0.1:${minioConsolePort}`,
    ],
    {
      cwd: projectRoot,
      runRoot,
      verbose: options.verbose,
      env: {
        ...process.env,
        MINIO_ROOT_USER: "minioadmin",
        MINIO_ROOT_PASSWORD: "minioadmin",
        MINIO_BROWSER: "off",
      },
    },
  );
  records.push(minio);
  await waitForUrl(
    `http://127.0.0.1:${minioPort}/minio/health/live`,
    30_000,
    minio,
  );

  const syncRoot = path.join(projectRoot, "donut-sync");
  await rm(path.join(syncRoot, "tsconfig.build.tsbuildinfo"), { force: true });
  await rm(path.join(syncRoot, "dist"), { recursive: true, force: true });
  run("pnpm", ["build"], syncRoot);
  const sync = startProcess("donut-sync", "node", ["dist/main.js"], {
    cwd: syncRoot,
    runRoot,
    verbose: options.verbose,
    env: {
      ...process.env,
      PORT: String(syncPort),
      SYNC_TOKEN: syncToken,
      S3_ENDPOINT: `http://127.0.0.1:${minioPort}`,
      S3_REGION: "us-east-1",
      S3_ACCESS_KEY_ID: "minioadmin",
      S3_SECRET_ACCESS_KEY: "minioadmin",
      S3_BUCKET: `donut-e2e-${process.pid}`,
      S3_FORCE_PATH_STYLE: "true",
    },
  });
  records.push(sync);
  await waitForUrl(`http://127.0.0.1:${syncPort}/health`, 30_000, sync);
  return {
    minioUrl: `http://127.0.0.1:${minioPort}`,
    syncUrl: `http://127.0.0.1:${syncPort}`,
    syncToken,
  };
}

function runDocker(args, { allowFailure = false } = {}) {
  const result = spawnSync("docker", args, {
    encoding: "utf8",
    timeout: 120_000,
    maxBuffer: 10 * 1024 * 1024,
  });
  if (!allowFailure && (result.error || result.status !== 0)) {
    throw new Error(
      `docker ${args[0]} failed: ${result.error?.message ?? result.stderr?.trim() ?? `exit ${result.status}`}`,
    );
  }
  return result;
}

function dockerAvailable() {
  const result = runDocker(["version"], { allowFailure: true });
  return !result.error && result.status === 0;
}

async function startWireGuardInfrastructure() {
  if (!dockerAvailable()) {
    throw new Error(
      "The network E2E suite requires a running Docker daemon for its local WireGuard peer",
    );
  }

  const port = await freePort();
  const name = `donut-wg-e2e-${process.pid}-${Date.now()}`;
  const image =
    process.env.DONUT_E2E_WIREGUARD_IMAGE ??
    "lscr.io/linuxserver/wireguard:latest";
  log("Starting isolated local WireGuard peer");
  runDocker([
    "run",
    "-d",
    "--name",
    name,
    "--cap-add=NET_ADMIN",
    "-p",
    `${port}:51820/udp`,
    "-e",
    "PEERS=1",
    "-e",
    "SERVERURL=127.0.0.1",
    "-e",
    "SERVERPORT=51820",
    "-e",
    "PEERDNS=auto",
    "-e",
    "INTERNAL_SUBNET=10.64.0.0",
    image,
  ]);

  try {
    const deadline = Date.now() + 45_000;
    let config = "";
    while (Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 1_000));
      const configResult = runDocker(
        ["exec", name, "cat", "/config/peer1/peer1.conf"],
        { allowFailure: true },
      );
      const statusResult = runDocker(["exec", name, "wg", "show"], {
        allowFailure: true,
      });
      if (
        configResult.status === 0 &&
        statusResult.status === 0 &&
        statusResult.stdout.includes("listening port")
      ) {
        config = configResult.stdout;
        break;
      }
    }
    if (!config) {
      throw new Error("Local WireGuard peer did not become ready within 45s");
    }

    const server = runDocker([
      "exec",
      "-d",
      name,
      "sh",
      "-c",
      'while true; do printf "HTTP/1.1 200 OK\\r\\nContent-Length: 13\\r\\nConnection: close\\r\\n\\r\\nWG-TUNNEL-OK\\n" | nc -l -p 8080 >> /tmp/donut-e2e-target-requests 2>/dev/null; done',
    ]);
    if (server.status !== 0) {
      throw new Error("Failed to start the WireGuard tunnel target server");
    }
    config = config.replace(
      /^Endpoint\s*=.*$/m,
      `Endpoint = 127.0.0.1:${port}`,
    );
    return {
      name,
      config,
      targetUrl: "http://10.64.0.1:8080/donut-e2e-wireguard",
    };
  } catch (error) {
    runDocker(["rm", "-f", name], { allowFailure: true });
    throw error;
  }
}

async function prepareRetainedArtifacts(
  runRoot,
  { suite, failed, sensitiveValues },
) {
  const sessionsRoot = path.join(runRoot, "sessions");
  const sessions = await readdir(sessionsRoot, {
    withFileTypes: true,
  }).catch(() => []);
  await Promise.all(
    sessions
      .filter((entry) => entry.isDirectory())
      .map((entry) =>
        rm(path.join(sessionsRoot, entry.name, "donut", "data", "binaries"), {
          recursive: true,
          force: true,
        }),
      ),
  );
  await createSafeDiagnostics(runRoot, { suite, failed, sensitiveValues });
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const runRoot = await mkdtemp(path.join(os.tmpdir(), "donut-e2e-"));
  await mkdir(path.join(runRoot, "logs"), { recursive: true });
  const records = [];
  let fixture;
  let wireGuard;
  let failed = false;
  const sensitiveValues = [
    "donut-e2e-sync-token-0123456789abcdef",
    "minioadmin",
  ];
  const cleanup = async () => {
    await Promise.all(records.reverse().map(stopProcess));
    if (fixture) {
      await new Promise((resolve) => fixture.server.close(resolve));
    }
    if (wireGuard) {
      runDocker(["rm", "-f", wireGuard.name], { allowFailure: true });
    }
    if (!options.keep && !failed) {
      await rm(runRoot, { recursive: true, force: true });
    } else {
      await prepareRetainedArtifacts(runRoot, {
        suite: options.suite,
        failed,
        sensitiveValues,
      });
      log(`Artifacts retained at ${runRoot}`);
    }
  };
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.once(signal, () => {
      failed = true;
      cleanup().finally(() => process.exit(signal === "SIGINT" ? 130 : 143));
    });
  }

  try {
    log(`Run root: ${runRoot}`);
    if (options.build) {
      buildAll();
    } else if (!existsSync(appBinary) || !existsSync(driverBinary)) {
      throw new Error(
        "--no-build requested but the E2E app or driver binary is missing",
      );
    }

    const driverPort = await freePort();
    const driver = startProcess(
      "tauri-wd",
      driverBinary,
      [
        "--port",
        String(driverPort),
        "--max-sessions",
        "4",
        "--startup-timeout",
        "120",
        "--command-timeout",
        "330",
        "--log",
        options.verbose ? "debug" : "info",
      ],
      {
        cwd: webdriverRoot,
        env: process.env,
        runRoot,
        verbose: options.verbose,
      },
    );
    records.push(driver);
    await waitForUrl(`http://127.0.0.1:${driverPort}/status`, 15_000, driver);

    const needsBrowser =
      options.suite === "browser" ||
      options.suite === "network" ||
      options.suite === "full";
    const networkEnabled =
      (options.suite === "network" || options.suite === "full") &&
      process.env.DONUT_E2E_SKIP_NETWORK_TEST !== "1";
    const geoIpFixture = needsBrowser ? await ensureGeoIpFixture() : null;
    fixture = await startFixtureServer(geoIpFixture);
    let sync = {};
    if (options.suite === "sync" || options.suite === "full") {
      sync = await startSyncInfrastructure(runRoot, options, records);
    }
    if (networkEnabled && process.env.DONUT_E2E_SKIP_VPN_TUNNEL !== "1") {
      wireGuard = await startWireGuardInfrastructure();
    }

    const localValues = await loadLocalValues([
      "WAYFERN_TEST_TOKEN",
      "RESIDENTIAL_PROXY_URL_ONE_SOCKS",
      "RESIDENTIAL_PROXY_URL_ONE_HTTP",
    ]);
    sensitiveValues.push(...Object.values(localValues));
    const token = localValues.WAYFERN_TEST_TOKEN ?? "";
    if (needsBrowser && !token) {
      throw new Error("WAYFERN_TEST_TOKEN is required by the browser suite");
    }
    if (wireGuard) {
      sensitiveValues.push(
        wireGuard.config,
        Buffer.from(wireGuard.config).toString("base64"),
      );
    }

    const files = suiteFiles[options.suite]
      .filter((file) => networkEnabled || file !== "network.test.mjs")
      .map((file) => path.join(dirname, "tests", file));
    const testArgs = [
      "--test",
      "--test-concurrency=1",
      "--test-reporter=spec",
      ...files,
    ];
    const child = spawn(process.execPath, testArgs, {
      cwd: projectRoot,
      env: {
        ...process.env,
        DONUT_E2E_RUN_ROOT: runRoot,
        DONUT_E2E_PROJECT_ROOT: projectRoot,
        DONUT_E2E_WEBDRIVER_ROOT: webdriverRoot,
        DONUT_E2E_APP: appBinary,
        DONUT_E2E_DRIVER_URL: `http://127.0.0.1:${driverPort}`,
        DONUT_E2E_FIXTURE_URL: `http://127.0.0.1:${fixture.port}`,
        DONUT_E2E_GEOIP_FIXTURE_READY: geoIpFixture ? "1" : "0",
        WAYFERN_TEST_TOKEN: token,
        RESIDENTIAL_PROXY_URL_ONE_SOCKS:
          localValues.RESIDENTIAL_PROXY_URL_ONE_SOCKS ?? "",
        RESIDENTIAL_PROXY_URL_ONE_HTTP:
          localValues.RESIDENTIAL_PROXY_URL_ONE_HTTP ?? "",
        DONUT_E2E_SYNC_URL: sync.syncUrl ?? "",
        DONUT_E2E_SYNC_TOKEN: sync.syncToken ?? "",
        DONUT_E2E_MINIO_URL: sync.minioUrl ?? "",
        DONUT_E2E_WIREGUARD_CONFIG_BASE64: wireGuard
          ? Buffer.from(wireGuard.config).toString("base64")
          : "",
        DONUT_E2E_WIREGUARD_TARGET_URL: wireGuard?.targetUrl ?? "",
        DONUT_E2E_WIREGUARD_CONTAINER: wireGuard?.name ?? "",
      },
      stdio: "inherit",
    });
    const exitCode = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code, signal) => {
        resolve(code ?? (signal ? 1 : 0));
      });
    });
    if (exitCode !== 0) {
      throw new Error(
        `E2E suite ${options.suite} failed with status ${exitCode}`,
      );
    }
    log(`Suite ${options.suite} passed`);
  } catch (error) {
    failed = true;
    process.stderr.write(`[donut-e2e] ERROR: ${error.stack ?? error}\n`);
    process.exitCode = 1;
  } finally {
    await cleanup();
  }
}

await main();

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import {
  chmod,
  copyFile,
  cp,
  mkdir,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";

export const TEST_BROWSER_VERSION = "150.0.7871.100";

export function defaultWayfernPath(projectRoot) {
  if (process.env.DONUT_E2E_WAYFERN_PATH) {
    return path.resolve(process.env.DONUT_E2E_WAYFERN_PATH);
  }
  const fixtureRoot = path.join(projectRoot, ".cache", "e2e-wayfern-fixture");
  return process.platform === "darwin"
    ? path.join(fixtureRoot, "Wayfern.app")
    : path.join(
        fixtureRoot,
        process.platform === "win32" ? "Wayfern.exe" : "wayfern",
      );
}

export function wayfernExecutable(bundlePath) {
  if (process.platform === "darwin") {
    return path.join(bundlePath, "Contents", "MacOS", "Wayfern");
  }
  return bundlePath;
}

export function inspectWayfern(bundlePath) {
  const executable = wayfernExecutable(bundlePath);
  assert.ok(
    existsSync(executable),
    `Wayfern executable is missing: ${executable}`,
  );
  const output =
    process.platform === "darwin"
      ? execFileSync(
          "/usr/bin/plutil",
          [
            "-extract",
            "CFBundleShortVersionString",
            "raw",
            "-o",
            "-",
            path.join(bundlePath, "Contents", "Info.plist"),
          ],
          { encoding: "utf8" },
        ).trim()
      : execFileSync(executable, ["--version"], {
          encoding: "utf8",
          timeout: 15_000,
        }).trim();
  const match = output.match(/(\d+\.\d+\.\d+\.\d+)/);
  assert.ok(match, `Could not parse Wayfern version from: ${output}`);
  return { bundlePath, executable, version: match[1], output };
}

async function cloneAppBundle(source, destination) {
  await mkdir(path.dirname(destination), { recursive: true });
  try {
    execFileSync("/bin/cp", ["-cR", source, destination]);
  } catch (_error) {
    await cp(source, destination, {
      recursive: true,
      preserveTimestamps: true,
      errorOnExist: true,
    });
  }
}

async function cacheDownloadedWayfern(app, projectRoot, version) {
  if (process.env.DONUT_E2E_WAYFERN_PATH) return;
  const destination = defaultWayfernPath(projectRoot);
  if (existsSync(destination)) return;

  const installDir = path.join(
    app.dataRoot,
    "data",
    "binaries",
    "wayfern",
    version,
  );
  const source =
    process.platform === "darwin"
      ? path.join(installDir, "Wayfern.app")
      : path.join(
          installDir,
          process.platform === "win32" ? "wayfern.exe" : "wayfern",
        );
  const staging = `${destination}.tmp-${process.pid}`;
  await rm(staging, { recursive: true, force: true });
  try {
    if (process.platform === "darwin") {
      await cloneAppBundle(source, staging);
    } else {
      await mkdir(path.dirname(staging), { recursive: true });
      await copyFile(source, staging);
      if (process.platform !== "win32") await chmod(staging, 0o755);
    }
    await rename(staging, destination);
  } catch (error) {
    await rm(staging, { recursive: true, force: true });
    if (!existsSync(destination)) throw error;
  }
}

export async function seedWayfern(dataRoot, wayfern) {
  const installDir = path.join(
    dataRoot,
    "data",
    "binaries",
    "wayfern",
    wayfern.version,
  );
  await mkdir(installDir, { recursive: true });
  if (process.platform === "darwin") {
    await cloneAppBundle(
      wayfern.bundlePath,
      path.join(installDir, "Wayfern.app"),
    );
  } else {
    const name = process.platform === "win32" ? "wayfern.exe" : "wayfern";
    const destination = path.join(installDir, name);
    await copyFile(wayfern.executable, destination);
    if (process.platform !== "win32") {
      await chmod(destination, 0o755);
    }
  }
  const registry = {
    browsers: {
      wayfern: {
        [wayfern.version]: {
          browser: "wayfern",
          version: wayfern.version,
          file_path: installDir,
        },
      },
    },
  };
  const registryPath = path.join(
    dataRoot,
    "data",
    "data",
    "downloaded_browsers.json",
  );
  await mkdir(path.dirname(registryPath), { recursive: true });
  await writeFile(registryPath, `${JSON.stringify(registry, null, 2)}\n`);
  return installDir;
}

export async function prepareWayfern(app, projectRoot) {
  const localBundle = defaultWayfernPath(projectRoot);
  if (existsSync(localBundle)) {
    const wayfern = inspectWayfern(localBundle);
    await seedWayfern(app.dataRoot, wayfern);
    return { version: wayfern.version, source: "local fixture" };
  }

  if (!app.session) await app.start();
  const current = await app.invoke("fetch_browser_versions_with_count", {
    browserStr: "wayfern",
  });
  assert.ok(
    current.versions.length > 0,
    "No Wayfern build is published for this platform",
  );
  const version = current.versions[0];
  await app.session.setTimeouts({ script: 600_000 });
  try {
    await app.invoke(
      "download_browser",
      {
        browserStr: "wayfern",
        version,
      },
      620_000,
    );
  } finally {
    await app.session.setTimeouts();
  }
  await cacheDownloadedWayfern(app, projectRoot, version);
  return { version, source: "published download" };
}

export function wireGuardFixture() {
  return [
    "[Interface]",
    "PrivateKey = AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
    "Address = 10.88.0.2/32",
    "DNS = 1.1.1.1",
    "",
    "[Peer]",
    "PublicKey = AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
    "Endpoint = 127.0.0.1:51820",
    "AllowedIPs = 0.0.0.0/0",
    "PersistentKeepalive = 25",
    "",
  ].join("\n");
}

export function extensionZipBase64() {
  // A deterministic Manifest V3 ZIP containing only manifest.json. Generated
  // once and kept inline so the suite has no archiver dependency.
  return "UEsDBBQAAAAAAE8K9Fxo1IfNawAAAGsAAAANAAAAbWFuaWZlc3QuanNvbnsibWFuaWZlc3RfdmVyc2lvbiI6MywibmFtZSI6IkRvbnV0IEUyRSBGaXh0dXJlIiwidmVyc2lvbiI6IjEuMC4wIiwiZGVzY3JpcHRpb24iOiJJc29sYXRlZCB0ZXN0IGV4dGVuc2lvbiJ9UEsBAhQDFAAAAAAATwr0XGjUh81rAAAAawAAAA0AAAAAAAAAAAAAAIABAAAAAG1hbmlmZXN0Lmpzb25QSwUGAAAAAAEAAQA7AAAAlgAAAAAA";
}

export function currentHostOs() {
  return os.platform() === "darwin"
    ? "macos"
    : os.platform() === "win32"
      ? "windows"
      : "linux";
}

/**
 * Write a Chromium cookie store at schema version 24 with plaintext values.
 *
 * Plaintext is deliberate: it is what a store looks like when the source
 * browser could not reach its keyring, and it lets the suite assert that
 * import seals every row with the target profile's key. Chromium reads a row
 * whose `encrypted_value` is empty, and drops any row where both columns are
 * set, so "value cleared and encrypted_value populated" is the only shape that
 * actually loads.
 */
export function writeChromiumCookies(dbPath, cookies) {
  const db = new DatabaseSync(dbPath);
  db.exec(`
    CREATE TABLE cookies(
      creation_utc INTEGER NOT NULL,
      host_key TEXT NOT NULL,
      top_frame_site_key TEXT NOT NULL,
      name TEXT NOT NULL,
      value TEXT NOT NULL,
      encrypted_value BLOB NOT NULL DEFAULT '',
      path TEXT NOT NULL,
      expires_utc INTEGER NOT NULL,
      is_secure INTEGER NOT NULL,
      is_httponly INTEGER NOT NULL,
      last_access_utc INTEGER NOT NULL,
      has_expires INTEGER NOT NULL DEFAULT 1,
      is_persistent INTEGER NOT NULL DEFAULT 1,
      priority INTEGER NOT NULL DEFAULT 1,
      samesite INTEGER NOT NULL DEFAULT -1,
      source_scheme INTEGER NOT NULL DEFAULT 0,
      source_port INTEGER NOT NULL DEFAULT -1,
      last_update_utc INTEGER NOT NULL DEFAULT 0,
      source_type INTEGER NOT NULL DEFAULT 0,
      has_cross_site_ancestor INTEGER NOT NULL DEFAULT 0
    );
    CREATE UNIQUE INDEX cookies_unique_index
      ON cookies(host_key, top_frame_site_key, name, path);
    CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
    INSERT INTO meta VALUES('version', '24');
    INSERT INTO meta VALUES('last_compatible_version', '24');
  `);
  const insert = db.prepare(
    `INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value,
       encrypted_value, path, expires_utc, is_secure, is_httponly, last_access_utc)
     VALUES(?, ?, '', ?, ?, ?, '/', 0, 0, 0, 0)`,
  );
  // `encrypted` cookies are written the way Chromium's v23->v24 migration
  // does: BindString into a BLOB column, which leaves the storage class as
  // TEXT. Reading that as a strict blob returns empty and silently blanks the
  // cookie, so the suite has to reproduce it rather than only binding blobs.
  const insertAsText = db.prepare(
    `INSERT INTO cookies(creation_utc, host_key, top_frame_site_key, name, value,
       encrypted_value, path, expires_utc, is_secure, is_httponly, last_access_utc)
     VALUES(?, ?, '', ?, '', CAST(? AS TEXT), '/', 0, 0, 0, 0)`,
  );
  let creation = 13000000000000000;
  for (const cookie of cookies) {
    if (cookie.encryptedValueText === undefined) {
      insert.run(creation++, cookie.host, cookie.name, cookie.value, "");
    } else {
      insertAsText.run(
        creation++,
        cookie.host,
        cookie.name,
        cookie.encryptedValueText,
      );
    }
  }
  db.close();
}

/** Write a Chromium History database holding the given URLs. */
export function writeChromiumHistory(dbPath, urls) {
  const db = new DatabaseSync(dbPath);
  db.exec(`
    CREATE TABLE urls(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      url LONGVARCHAR,
      title LONGVARCHAR,
      visit_count INTEGER DEFAULT 0 NOT NULL,
      typed_count INTEGER DEFAULT 0 NOT NULL,
      last_visit_time INTEGER NOT NULL,
      hidden INTEGER DEFAULT 0 NOT NULL
    );
    CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
    INSERT INTO meta VALUES('version', '69');
    INSERT INTO meta VALUES('last_compatible_version', '16');
  `);
  const insert = db.prepare(
    "INSERT INTO urls(url, title, visit_count, typed_count, last_visit_time, hidden) VALUES(?, ?, 1, 0, ?, 0)",
  );
  let visit = 13000000000000000;
  for (const url of urls) {
    insert.run(url, url, visit++);
  }
  db.close();
}

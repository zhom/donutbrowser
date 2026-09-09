import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
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
import { crc32 } from "node:zlib";
import {
  WAYFERN_DOWNLOAD_CLIENT_TIMEOUT_MS,
  WAYFERN_DOWNLOAD_TIMEOUT_MS,
} from "./limits.mjs";

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

/**
 * Where the cache fixture records which PUBLISHED version it was installed for.
 *
 * The bundle's own `CFBundleShortVersionString` cannot answer that question: a
 * published version and the version stamped inside the bundle it serves do not
 * always agree, and the app keys everything (download registry, profile
 * `version`, release types) off the PUBLISHED string. Comparing the bundle's
 * own version against the published one would therefore call an up-to-date
 * fixture stale and re-download 1 GB on every single run.
 */
function fixtureStampPath(projectRoot) {
  return path.join(
    path.dirname(defaultWayfernPath(projectRoot)),
    "published-version.txt",
  );
}

/**
 * The published version the cache fixture stands for, or `null` when there is
 * no fixture.
 *
 * Falls back to the bundle's own version when no stamp is present, which is
 * what a hand-installed fixture looks like: it is only right when the two
 * agree, and when they do not the fixture is replaced, which is the safe way
 * to be wrong.
 */
export function cachedFixtureVersion(projectRoot) {
  const bundle = defaultWayfernPath(projectRoot);
  if (!existsSync(bundle)) return null;
  const stamp = fixtureStampPath(projectRoot);
  if (existsSync(stamp)) {
    const recorded = readFileSync(stamp, "utf8").trim();
    if (recorded) return recorded;
  }
  return inspectWayfern(bundle).version;
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

/** Where the app itself resolves the current Wayfern build (api_client.rs). */
const WAYFERN_RELEASE_URL = "https://donutbrowser.com/wayfern.json";

/**
 * The newest published Wayfern version, read from the same manifest the app
 * reads.
 *
 * Deliberately NOT asked of a running app session. Seeding a browser into a
 * session's data root only works before that session starts: a running app
 * runs `cleanup_unused_binaries`, which deletes any binary directory no
 * profile references, and a just-seeded fixture is exactly that. Resolving the
 * version over plain HTTP keeps the seed ahead of app startup.
 */
async function publishedWayfernVersion() {
  const response = await fetch(WAYFERN_RELEASE_URL, {
    signal: AbortSignal.timeout(30_000),
  });
  assert.ok(
    response.ok,
    `Could not read ${WAYFERN_RELEASE_URL}: HTTP ${response.status}`,
  );
  const manifest = await response.json();
  assert.ok(
    typeof manifest.version === "string" && manifest.version,
    `No Wayfern version published at ${WAYFERN_RELEASE_URL}`,
  );
  return manifest.version;
}

async function downloadWayfern(app, version) {
  await app.session.setTimeouts({ script: WAYFERN_DOWNLOAD_TIMEOUT_MS });
  try {
    await app.invoke(
      "download_browser",
      { browserStr: "wayfern", version },
      WAYFERN_DOWNLOAD_CLIENT_TIMEOUT_MS,
    );
  } finally {
    await app.session.setTimeouts();
  }
}

/**
 * Put the build this session just downloaded into the cache fixture, in place
 * of whatever build the cache held before. The swap goes through a staging
 * copy and renames, so a suite that dies mid-copy leaves the old fixture or
 * the new one on disk, never a half-written bundle.
 */
async function cacheDownloadedWayfern(app, projectRoot, version) {
  if (process.env.DONUT_E2E_WAYFERN_PATH) return;
  const destination = defaultWayfernPath(projectRoot);

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
  const retired = `${destination}.stale-${process.pid}`;
  await rm(staging, { recursive: true, force: true });
  await rm(retired, { recursive: true, force: true });
  try {
    if (process.platform === "darwin") {
      await cloneAppBundle(source, staging);
    } else {
      await mkdir(path.dirname(staging), { recursive: true });
      await copyFile(source, staging);
      if (process.platform !== "win32") await chmod(staging, 0o755);
    }
    if (existsSync(destination)) await rename(destination, retired);
    await rename(staging, destination);
    // Stamped only after the bundle is in place, so an interrupted swap can
    // never leave a stamp claiming a version the fixture does not hold.
    await writeFile(fixtureStampPath(projectRoot), `${version}\n`);
  } catch (error) {
    await rm(staging, { recursive: true, force: true });
    if (!existsSync(destination) && existsSync(retired)) {
      await rename(retired, destination);
    }
    if (!existsSync(destination)) throw error;
    // The session itself runs the build it downloaded; only the cache is
    // behind, and the next run resolves the published version again and
    // replaces it then.
    console.warn(
      `[donut-e2e] Could not refresh the Wayfern fixture cache: ${error}`,
    );
  } finally {
    await rm(retired, { recursive: true, force: true });
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

/**
 * Make the newest published Wayfern available to `app` and report the version
 * it will run.
 *
 * `DONUT_E2E_WAYFERN_PATH` pins an explicit bundle and is used as given: that
 * is how a locally built browser gets under test. Without it the suite runs
 * the build the product would offer today, always. The ignored cache fixture
 * only ever saves the download: it is used when it holds exactly that build
 * and replaced when it holds any other, so a cache filled months ago can never
 * quietly keep an old browser under test.
 */
export async function prepareWayfern(app, projectRoot) {
  const localBundle = defaultWayfernPath(projectRoot);
  if (process.env.DONUT_E2E_WAYFERN_PATH) {
    const wayfern = inspectWayfern(localBundle);
    await seedWayfern(app.dataRoot, wayfern);
    return { version: wayfern.version, source: "pinned fixture" };
  }

  const version = await publishedWayfernVersion();
  const cachedVersion = cachedFixtureVersion(projectRoot);
  if (cachedVersion === version) {
    // Seeded under the PUBLISHED version, not the bundle's own, because that
    // is the string the app itself would have registered had it downloaded
    // this build, and what every later `version` assertion compares against.
    // Seeded BEFORE the app starts, or its unused-binary cleanup deletes it.
    await seedWayfern(app.dataRoot, {
      ...inspectWayfern(localBundle),
      version,
    });
    return { version, source: "cached fixture" };
  }
  if (cachedVersion) {
    console.log(
      `[donut-e2e] Cached Wayfern fixture ${cachedVersion} is not the published ${version}; replacing it`,
    );
  }

  if (!app.session) await app.start();
  await downloadWayfern(app, version);
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

// 1980-01-01 00:00, the earliest timestamp the ZIP format can carry. Fixed so
// two calls with the same entries produce byte-identical archives.
const DOS_TIME = 0;
const DOS_DATE = 0x0021;

/**
 * Build a ZIP archive from `entries` (`{ name, data }`) with every member
 * stored, not deflated.
 *
 * Stored is what the inline fixture above already is, and it is load-bearing
 * for the oversized fixture below: the assertion is about a request body that
 * has to stay over the limit under test, so nothing in the archive may shrink
 * the padding back under it.
 */
export function buildStoredZip(entries) {
  const locals = [];
  const central = [];
  let offset = 0;

  for (const { name, data } of entries) {
    const nameBytes = Buffer.from(name, "utf8");
    const body = Buffer.isBuffer(data) ? data : Buffer.from(data);
    const checksum = crc32(body);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(DOS_TIME, 10);
    local.writeUInt16LE(DOS_DATE, 12);
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(body.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    locals.push(local, nameBytes, body);

    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE(20, 4);
    entry.writeUInt16LE(20, 6);
    entry.writeUInt16LE(DOS_TIME, 12);
    entry.writeUInt16LE(DOS_DATE, 14);
    entry.writeUInt32LE(checksum, 16);
    entry.writeUInt32LE(body.length, 20);
    entry.writeUInt32LE(body.length, 24);
    entry.writeUInt16LE(nameBytes.length, 28);
    entry.writeUInt32LE(offset, 42);
    central.push(entry, nameBytes);

    offset += local.length + nameBytes.length + body.length;
  }

  const directory = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);

  return Buffer.concat([...locals, directory, end]);
}

export const OVERSIZED_EXTENSION_NAME = "Donut E2E Oversized Fixture";

/**
 * A valid Manifest V3 ZIP padded past the 2 MiB body limit axum applies by
 * default, so the raised limit on the extension routes is the only reason a
 * request carrying it can succeed.
 *
 * The padding is random bytes, and the archive stores rather than deflates
 * them, so neither the fixture nor the transport can quietly shrink the body
 * back under the limit and turn the assertion into a tautology.
 */
export function oversizedExtensionZipBase64(paddingBytes = 3 * 1024 * 1024) {
  return buildStoredZip([
    {
      name: "manifest.json",
      data: `${JSON.stringify(
        {
          manifest_version: 3,
          name: OVERSIZED_EXTENSION_NAME,
          version: "1.0.0",
          description: "Isolated oversized test extension",
        },
        null,
        2,
      )}\n`,
    },
    { name: "payload.bin", data: randomBytes(paddingBytes) },
  ]).toString("base64");
}

// What `_locales/<default_locale>/messages.json` resolves the manifest's
// placeholders to. Deliberately free of the `__MSG_` marker so a test can
// assert the stored record carries no placeholder anywhere.
export const LOCALIZED_EXTENSION_MESSAGES = {
  extName: "Donut E2E Localized Blocker",
  extDescription: "Resolved from the default locale, not the manifest",
  extAuthor: "Donut E2E Localization",
};

/**
 * A Manifest V3 ZIP shaped the way Chrome Web Store extensions actually ship:
 * `name`, `description` and `author` are `__MSG_key__` placeholders and the
 * real strings live in `_locales/<default_locale>/messages.json`. uBlock Origin
 * Lite is exactly this, which is why an importer that stores the manifest
 * verbatim shows users `__MSG_extName__`.
 *
 * Pass `messages: {}` for a locale file that resolves none of the placeholders,
 * or `messages: null` to omit the locale file entirely.
 */
export function localizedExtensionZipBase64({
  defaultLocale = "en",
  messages = LOCALIZED_EXTENSION_MESSAGES,
} = {}) {
  const entries = [
    {
      name: "manifest.json",
      data: `${JSON.stringify(
        {
          manifest_version: 3,
          name: "__MSG_extName__",
          version: "2.4.0",
          description: "__MSG_extDescription__",
          author: "__MSG_extAuthor__",
          default_locale: defaultLocale,
        },
        null,
        2,
      )}\n`,
    },
  ];
  if (messages) {
    entries.push({
      name: `_locales/${defaultLocale}/messages.json`,
      data: `${JSON.stringify(
        Object.fromEntries(
          Object.entries(messages).map(([key, message]) => [key, { message }]),
        ),
        null,
        2,
      )}\n`,
    });
  }
  return buildStoredZip(entries).toString("base64");
}

// A 1x1 PNG, inline for the same reason the ZIP above is: no encoder
// dependency, and the exact bytes are what the icon assertions compare.
const EXTENSION_ICON_PNG_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

export function extensionIconPngBase64() {
  return EXTENSION_ICON_PNG_BASE64;
}

/**
 * Write a real unpacked Manifest V3 extension at `directory` and return its
 * absolute path.
 *
 * Unlike the ZIP fixture this one declares `icons` and ships the file they
 * point at, so importing the folder exercises icon extraction for both import
 * modes: linking reads the icon straight out of the folder, copying reads it
 * back out of the ZIP the importer builds. The background service worker is
 * what makes a loaded copy observable over CDP, which registers a
 * `chrome-extension://<id>/background.js` target.
 */
export async function writeUnpackedExtension(
  directory,
  { name = "Donut E2E Unpacked", version = "1.0.0" } = {},
) {
  const absolute = path.resolve(directory);
  await mkdir(path.join(absolute, "icons"), { recursive: true });
  await writeFile(
    path.join(absolute, "manifest.json"),
    `${JSON.stringify(
      {
        manifest_version: 3,
        name,
        version,
        description: "Isolated unpacked test extension",
        icons: { 16: "icons/icon-16.png", 48: "icons/icon-48.png" },
        background: { service_worker: "background.js" },
      },
      null,
      2,
    )}\n`,
  );
  await writeFile(
    path.join(absolute, "background.js"),
    [
      "globalThis.__donutE2eExtension = chrome.runtime.id;",
      "chrome.runtime.onInstalled.addListener(() => {",
      "  console.log('donut e2e extension installed');",
      "});",
      "",
    ].join("\n"),
  );
  const icon = Buffer.from(EXTENSION_ICON_PNG_BASE64, "base64");
  for (const size of [16, 48]) {
    await writeFile(path.join(absolute, "icons", `icon-${size}.png`), icon);
  }
  return absolute;
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

/** The name and version the CRX fixture's own manifest declares. */
export const CRX_EXTENSION_NAME = "Donut E2E Web Extension";
export const CRX_EXTENSION_VERSION = "3.2.1";

/**
 * Wrap `zip` in a CRX3 container, the shape the Chrome Web Store actually
 * serves: `Cr24`, a little-endian format version of 3, a little-endian header
 * length, that many bytes of signature header, and only then the ZIP.
 *
 * The header bytes are filler — nothing in Donut verifies the signature, and a
 * real one would need a packing key. What a test built on this proves is that
 * the importer reads the ZIP at the offset the header declares instead of
 * scanning the file for a `PK` marker, which is the bug the format invites.
 */
export function buildCrx3(zip, headerBytes = 137) {
  const prefix = Buffer.alloc(12);
  prefix.write("Cr24", 0, "ascii");
  prefix.writeUInt32LE(3, 4);
  prefix.writeUInt32LE(headerBytes, 8);
  return Buffer.concat([prefix, Buffer.alloc(headerBytes, 0x42), zip]);
}

/** A CRX3 whose payload is a real Manifest V3 archive. */
export function extensionCrx3({
  name = CRX_EXTENSION_NAME,
  version = CRX_EXTENSION_VERSION,
} = {}) {
  return buildCrx3(
    buildStoredZip([
      {
        name: "manifest.json",
        data: `${JSON.stringify(
          {
            manifest_version: 3,
            name,
            version,
            description: "Isolated test extension served over a link",
          },
          null,
          2,
        )}\n`,
      },
    ]),
  );
}

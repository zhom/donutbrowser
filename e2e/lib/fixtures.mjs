import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, copyFile, mkdir, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

export const TEST_BROWSER_VERSION = "150.0.7871.100";

export function defaultWayfernPath(projectRoot) {
  if (process.env.DONUT_E2E_WAYFERN_PATH) {
    return path.resolve(process.env.DONUT_E2E_WAYFERN_PATH);
  }
  const sibling = path.resolve(
    projectRoot,
    "../wayfern-test/test_extracted_app",
  );
  if (process.platform === "darwin") {
    return path.join(sibling, "Wayfern.app");
  }
  if (process.platform === "win32") {
    return path.join(sibling, "Wayfern.exe");
  }
  return path.join(sibling, "wayfern");
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

async function linkOrCopy(source, destination) {
  await mkdir(path.dirname(destination), { recursive: true });
  try {
    await symlink(
      source,
      destination,
      process.platform === "win32" ? "junction" : undefined,
    );
  } catch (error) {
    if (error.code !== "EEXIST") {
      throw error;
    }
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
    await linkOrCopy(wayfern.bundlePath, path.join(installDir, "Wayfern.app"));
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

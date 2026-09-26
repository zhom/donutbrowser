// rclone's S3 server, the storage under both sync test harnesses
// (scripts/sync-test-harness.mjs and e2e/run.mjs).
//
// MinIO used to play this part, but MinIO withdrew its community binaries and
// every dl.min.io path now answers 410. rclone still publishes a single binary
// per platform, and `rclone serve s3` speaks enough of the API for these tests:
// path-style addressing, a static key pair, and bucket creation (a bucket is a
// directory under the served root). It has no health endpoint, so wait for its
// port to accept connections.

import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, renameSync } from "node:fs";
import os from "node:os";
import path from "node:path";

// Pinned so a fresh rclone release cannot change what CI runs underneath us.
export const RCLONE_VERSION = "v1.75.1";

function rcloneAsset() {
  const platform = os.platform();
  const arch = os.arch() === "arm64" ? "arm64" : "amd64";
  if (platform === "darwin") return `rclone-${RCLONE_VERSION}-osx-${arch}`;
  if (platform === "linux") return `rclone-${RCLONE_VERSION}-linux-${arch}`;
  if (platform === "win32") return `rclone-${RCLONE_VERSION}-windows-amd64`;
  throw new Error(`Unsupported platform: ${platform}-${os.arch()}`);
}

// Mirrors src-tauri/download-xray.mjs: PowerShell owns zip extraction on
// Windows, `unzip` everywhere else. Both are present on every runner these
// harnesses target.
function extractZip(archive, destinationDir) {
  const result =
    os.platform() === "win32"
      ? spawnSync(
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
        )
      : spawnSync(
          "unzip",
          ["-qq", "-j", "-o", archive, "*/rclone", "-d", destinationDir],
          { stdio: "inherit" },
        );
  if (result.status !== 0) {
    throw new Error("Failed to extract the rclone archive");
  }
}

/**
 * The rclone binary cached in `cacheDir`, fetched with `download(url, dest)`
 * the first time.
 */
export async function ensureRclone(cacheDir, download, log) {
  const isWindows = os.platform() === "win32";
  const binName = isWindows ? "rclone.exe" : "rclone";
  const bin = path.join(cacheDir, binName);
  if (existsSync(bin)) {
    return bin;
  }

  log("Downloading rclone binary...");
  mkdirSync(cacheDir, { recursive: true });
  const asset = rcloneAsset();
  const archive = path.join(cacheDir, `${asset}.zip`);
  await download(
    `https://github.com/rclone/rclone/releases/download/${RCLONE_VERSION}/${asset}.zip`,
    archive,
  );
  extractZip(archive, cacheDir);

  if (isWindows) {
    // Expand-Archive keeps the archive's own directory, so lift the binary out.
    const nested = path.join(cacheDir, asset, binName);
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

/** Arguments for `rclone serve s3` over `dataDir` at `address` (host:port). */
export function serveS3Args({ dataDir, address, accessKey, secretKey }) {
  return [
    "serve",
    "s3",
    dataDir,
    "--addr",
    address,
    "--auth-key",
    `${accessKey},${secretKey}`,
    "--force-path-style",
    "--vfs-cache-mode",
    "writes",
  ];
}

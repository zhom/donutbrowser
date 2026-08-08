import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const XRAY_VERSION = "v26.3.27";
export const XRAY_SOURCE_URL = `https://github.com/XTLS/Xray-core/tree/${XRAY_VERSION}`;
export const XRAY_LICENSE_FILE = "xray-LICENSE.txt";

export const XRAY_ASSETS = {
  "aarch64-apple-darwin": {
    name: "Xray-macos-arm64-v8a.zip",
    sha256: "2e93a67e8aa1936ecefb307e120830fcbd4c643ab9b1c46a2d0838d5f8409eaf",
  },
  "x86_64-apple-darwin": {
    name: "Xray-macos-64.zip",
    sha256: "f5b0471d3459eff1b82e48af0aeac186abcc3298210070afbbbd8437a4e8b203",
  },
  "x86_64-unknown-linux-gnu": {
    name: "Xray-linux-64.zip",
    sha256: "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae",
  },
  "aarch64-unknown-linux-gnu": {
    name: "Xray-linux-arm64-v8a.zip",
    sha256: "4d30283ae614e3057f730f67cd088a42be6fdf91f8639d82cb69e48cde80413c",
  },
  "x86_64-pc-windows-msvc": {
    name: "Xray-windows-64.zip",
    sha256: "d004c39288ce9ada487c6f398c7c545f7d749e44bdfdd59dbc9f865afba4e1ad",
  },
};

const MANIFEST_DIR = dirname(fileURLToPath(import.meta.url));

export function requestedTarget() {
  const targetIndex = process.argv.indexOf("--target");
  if (targetIndex !== -1 && process.argv[targetIndex + 1]) {
    return process.argv[targetIndex + 1];
  }
  if (process.env.TARGET) {
    return process.env.TARGET;
  }

  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = result.stdout?.match(/^host:\s*(.+)$/m);
  if (!match) {
    throw new Error("Unable to determine the Rust target");
  }
  return match[1].trim();
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function xrayBinaryName(target) {
  return `xray-${target}${target.includes("windows") ? ".exe" : ""}`;
}

export function xrayDownloadUrl(assetName) {
  return `https://github.com/XTLS/Xray-core/releases/download/${XRAY_VERSION}/${assetName}`;
}

// `powershell -Command "<script>" a b` appends the trailing values to the
// command text rather than binding them to $args, so the script ran with a
// null -LiteralPath. Handing the paths over as environment variables binds
// them for real and sidesteps quoting of Windows paths and spaces.
export function windowsExtractionInvocation(archive, destinationDir) {
  return {
    args: [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "Expand-Archive -LiteralPath $env:DONUT_XRAY_ARCHIVE -DestinationPath $env:DONUT_XRAY_DESTINATION -Force",
    ],
    env: {
      ...process.env,
      DONUT_XRAY_ARCHIVE: archive,
      DONUT_XRAY_DESTINATION: destinationDir,
    },
  };
}

function extractArchive(archive, destinationDir, windowsTarget) {
  if (windowsTarget) {
    const { args, env } = windowsExtractionInvocation(archive, destinationDir);
    const result = spawnSync("powershell", args, { stdio: "inherit", env });
    if (result.status !== 0) {
      throw new Error("Failed to extract the Xray-core archive");
    }
    // Upstream ships these lowercase inside Xray-windows-64.zip.
    return {
      binary: join(destinationDir, "xray.exe"),
      license: join(destinationDir, "LICENSE"),
    };
  }

  const result = spawnSync(
    "unzip",
    ["-qq", "-j", archive, "xray", "LICENSE", "-d", destinationDir],
    { stdio: "inherit" },
  );
  if (result.status !== 0) {
    throw new Error("Failed to extract the Xray-core archive");
  }

  return {
    binary: join(destinationDir, "xray"),
    license: join(destinationDir, "LICENSE"),
  };
}

/// Attempts for the archive download. A release asset fetch is a network call
/// on every CI job, and a single transport error ("fetch failed") has taken
/// whole builds down. Retrying is safe because the checksum below is verified
/// on every attempt, so a truncated or substituted archive still cannot pass.
const DOWNLOAD_ATTEMPTS = 3;

async function downloadVerifiedArchive(url, archive, expectedSha256) {
  let lastError;

  for (let attempt = 1; attempt <= DOWNLOAD_ATTEMPTS; attempt += 1) {
    try {
      const response = await fetch(url);
      if (!response.ok) {
        throw new Error(
          `Failed to download Xray-core (${response.status} ${response.statusText})`,
        );
      }
      writeFileSync(archive, Buffer.from(await response.arrayBuffer()));

      const actual = sha256(archive);
      if (actual !== expectedSha256) {
        throw new Error(
          `Xray-core checksum mismatch: expected ${expectedSha256}, got ${actual}`,
        );
      }
      return;
    } catch (error) {
      lastError = error;
      if (attempt < DOWNLOAD_ATTEMPTS) {
        console.warn(
          `Xray-core download attempt ${attempt} failed (${error.message}); retrying`,
        );
        await new Promise((resolve) => setTimeout(resolve, attempt * 2000));
      }
    }
  }

  throw lastError;
}

export async function downloadXray(target = requestedTarget()) {
  const asset = XRAY_ASSETS[target];
  if (!asset) {
    throw new Error(`Xray-core is not packaged for Rust target '${target}'`);
  }

  const windowsTarget = target.includes("windows");
  const destinationDir = join(MANIFEST_DIR, "binaries");
  const destination = join(destinationDir, xrayBinaryName(target));
  const licenseDestination = join(destinationDir, XRAY_LICENSE_FILE);
  const marker = `${destination}.source.json`;

  if (
    existsSync(destination) &&
    existsSync(licenseDestination) &&
    existsSync(marker)
  ) {
    try {
      const source = JSON.parse(readFileSync(marker, "utf8"));
      if (
        source.version === XRAY_VERSION &&
        source.archiveSha256 === asset.sha256 &&
        source.binarySha256 === sha256(destination) &&
        source.licenseSha256 === sha256(licenseDestination)
      ) {
        return destination;
      }
    } catch {
      // A partial or older cache entry is replaced from the verified archive.
    }
  }

  mkdirSync(destinationDir, { recursive: true });
  const scratch = mkdtempSync(join(tmpdir(), "donut-xray-"));
  try {
    const archive = join(scratch, basename(asset.name));
    await downloadVerifiedArchive(
      xrayDownloadUrl(asset.name),
      archive,
      asset.sha256,
    );

    const extracted = extractArchive(archive, scratch, windowsTarget);
    if (!existsSync(extracted.binary) || !existsSync(extracted.license)) {
      throw new Error(
        "The Xray-core archive did not contain its executable and license",
      );
    }
    copyFileSync(extracted.binary, destination);
    copyFileSync(extracted.license, licenseDestination);
    if (!windowsTarget) {
      chmodSync(destination, 0o755);
    }
    writeFileSync(
      marker,
      `${JSON.stringify(
        {
          version: XRAY_VERSION,
          archiveSha256: asset.sha256,
          binarySha256: sha256(destination),
          licenseSha256: sha256(licenseDestination),
        },
        null,
        2,
      )}\n`,
    );
    console.log(`Downloaded Xray-core ${XRAY_VERSION} to ${destination}`);
    return destination;
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  downloadXray().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}

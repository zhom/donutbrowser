import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  downloadVerifiedArchive,
  downloadXray,
  windowsExtractionInvocation,
  XRAY_ASSETS,
  XRAY_LICENSE_FILE,
  XRAY_SOURCE_URL,
  XRAY_VERSION,
  xrayBinaryName,
  xrayDownloadUrl,
} from "./download-xray.mjs";

const EXPECTED_ASSETS = {
  "aarch64-apple-darwin": [
    "Xray-macos-arm64-v8a.zip",
    "2e93a67e8aa1936ecefb307e120830fcbd4c643ab9b1c46a2d0838d5f8409eaf",
  ],
  "x86_64-apple-darwin": [
    "Xray-macos-64.zip",
    "f5b0471d3459eff1b82e48af0aeac186abcc3298210070afbbbd8437a4e8b203",
  ],
  "x86_64-unknown-linux-gnu": [
    "Xray-linux-64.zip",
    "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae",
  ],
  "aarch64-unknown-linux-gnu": [
    "Xray-linux-arm64-v8a.zip",
    "4d30283ae614e3057f730f67cd088a42be6fdf91f8639d82cb69e48cde80413c",
  ],
  "x86_64-pc-windows-msvc": [
    "Xray-windows-64.zip",
    "d004c39288ce9ada487c6f398c7c545f7d749e44bdfdd59dbc9f865afba4e1ad",
  ],
};

test("pins the official Xray-core release and supported assets", () => {
  assert.equal(XRAY_VERSION, "v26.3.27");
  assert.equal(
    XRAY_SOURCE_URL,
    "https://github.com/XTLS/Xray-core/tree/v26.3.27",
  );
  assert.deepEqual(
    Object.fromEntries(
      Object.entries(XRAY_ASSETS).map(([target, asset]) => [
        target,
        [asset.name, asset.sha256],
      ]),
    ),
    EXPECTED_ASSETS,
  );

  for (const asset of Object.values(XRAY_ASSETS)) {
    assert.match(asset.sha256, /^[a-f0-9]{64}$/);
    assert.equal(
      xrayDownloadUrl(asset.name),
      `https://github.com/XTLS/Xray-core/releases/download/v26.3.27/${asset.name}`,
    );
  }
});

test("uses the sidecar filenames expected by Tauri", () => {
  assert.equal(XRAY_LICENSE_FILE, "xray-LICENSE.txt");
  assert.equal(
    xrayBinaryName("aarch64-apple-darwin"),
    "xray-aarch64-apple-darwin",
  );
  assert.equal(
    xrayBinaryName("x86_64-pc-windows-msvc"),
    "xray-x86_64-pc-windows-msvc.exe",
  );
});

test("bundles the upstream license in Tauri and portable releases", async () => {
  const tauriConfig = JSON.parse(
    await readFile(new URL("./tauri.conf.json", import.meta.url), "utf8"),
  );
  assert.equal(
    tauriConfig.bundle.resources[`binaries/${XRAY_LICENSE_FILE}`],
    "licenses/Xray-core-LICENSE.txt",
  );

  for (const workflow of ["release.yml", "rolling-release.yml"]) {
    const contents = await readFile(
      new URL(`../.github/workflows/${workflow}`, import.meta.url),
      "utf8",
    );
    assert.match(
      contents,
      /cp "src-tauri\/binaries\/xray-LICENSE\.txt" "\$PORTABLE_DIR\/licenses\/Xray-core-LICENSE\.txt"/,
    );
  }
});

test("binds Windows extraction paths that PowerShell can actually read", () => {
  const { args, env } = windowsExtractionInvocation(
    "C:\\Users\\runner\\AppData\\Local\\Temp\\donut xray\\Xray-windows-64.zip",
    "C:\\Users\\runner\\AppData\\Local\\Temp\\donut xray",
  );

  // `powershell -Command "<script>" a b` folds the trailing values into the
  // command text instead of populating $args, which left -LiteralPath null.
  const script = args.at(-1);
  assert.equal(args.at(-2), "-Command");
  assert.doesNotMatch(script, /\$args/);
  assert.match(script, /-LiteralPath \$env:DONUT_XRAY_ARCHIVE\b/);
  assert.match(script, /-DestinationPath \$env:DONUT_XRAY_DESTINATION\b/);
  assert.equal(
    env.DONUT_XRAY_ARCHIVE,
    "C:\\Users\\runner\\AppData\\Local\\Temp\\donut xray\\Xray-windows-64.zip",
  );
  assert.equal(
    env.DONUT_XRAY_DESTINATION,
    "C:\\Users\\runner\\AppData\\Local\\Temp\\donut xray",
  );
});

test("rejects an unsupported target before downloading", async () => {
  await assert.rejects(
    downloadXray("riscv64gc-unknown-linux-gnu"),
    /not packaged for Rust target/,
  );
});

// `constructor`, `__proto__` and friends answer a plain `XRAY_ASSETS[target]`
// lookup, and `target` picks the path this script writes into src-tauri.
test("rejects inherited object keys as targets", async () => {
  for (const target of ["__proto__", "constructor", "toString"]) {
    await assert.rejects(
      downloadXray(target),
      /not packaged for Rust target/,
      target,
    );
  }
});

async function withStubbedFetch(body, run) {
  const scratch = mkdtempSync(join(tmpdir(), "donut-xray-test-"));
  const archive = join(scratch, "Xray-linux-64.zip");
  const realFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response(body);
  try {
    await run(archive);
  } finally {
    globalThis.fetch = realFetch;
    rmSync(scratch, { recursive: true, force: true });
  }
}

test("writes the archive once the pinned digest matches", async () => {
  const body = Buffer.from("xray archive bytes");
  const digest = createHash("sha256").update(body).digest("hex");

  await withStubbedFetch(body, async (archive) => {
    await downloadVerifiedArchive(
      "https://example.invalid/x.zip",
      archive,
      digest,
    );
    assert.deepEqual(readFileSync(archive), body);
  });
});

// The bytes are hashed in memory and only then written, so a substituted or
// truncated response never lands on disk for a later step to pick up.
test("leaves nothing on disk when the payload fails its checksum", async () => {
  await withStubbedFetch(Buffer.from("tampered"), async (archive) => {
    await assert.rejects(
      downloadVerifiedArchive(
        "https://example.invalid/x.zip",
        archive,
        "0".repeat(64),
      ),
      /checksum mismatch/,
    );
    assert.equal(existsSync(archive), false);
  });
});

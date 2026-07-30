import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { XRAY_SOURCE_URL } from "../src-tauri/download-xray.mjs";
import {
  collectPnpmLicenses,
  collectReachableRustLicenses,
  normalizeLicenseExpression,
  prepareLicenseInventory,
} from "./generate-licenses.mjs";

test("normalizes legacy dual-license metadata into SPDX expressions", () => {
  assert.equal(
    normalizeLicenseExpression("MIT/Apache-2.0"),
    "Apache-2.0 OR MIT",
  );
  assert.equal(
    normalizeLicenseExpression("Apache-2.0 OR MIT"),
    "Apache-2.0 OR MIT",
  );
  assert.throws(() => normalizeLicenseExpression(""), /declare a license/);
  assert.throws(
    () => normalizeLicenseExpression("not/a/license"),
    /Invalid SPDX expression/,
  );
  for (const invalid of [
    "NOASSERTION",
    "Definitely-Not-A-License",
    "MPL-999.0",
  ]) {
    assert.throws(
      () => normalizeLicenseExpression(invalid),
      /Invalid SPDX expression/,
    );
  }
});

test("collects only normal Rust dependencies reachable from the app", () => {
  const metadata = {
    packages: [
      { id: "app", name: "app", license: "AGPL-3.0" },
      { id: "runtime", name: "runtime", license: "MIT" },
      { id: "nested", name: "nested", license: "Apache-2.0" },
      { id: "build", name: "build", license: "MIT" },
      { id: "dev", name: "dev", license: "MIT" },
    ],
    resolve: {
      root: "app",
      nodes: [
        {
          id: "app",
          deps: [
            { pkg: "runtime", dep_kinds: [{ kind: null }] },
            { pkg: "build", dep_kinds: [{ kind: "build" }] },
            { pkg: "dev", dep_kinds: [{ kind: "dev" }] },
          ],
        },
        {
          id: "runtime",
          deps: [{ pkg: "nested", dep_kinds: [{ kind: null }] }],
        },
        { id: "nested", deps: [] },
        { id: "build", deps: [] },
        { id: "dev", deps: [] },
      ],
    },
  };

  assert.deepEqual(collectReachableRustLicenses(metadata), [
    { name: "runtime", license: "MIT" },
    { name: "nested", license: "Apache-2.0" },
  ]);
});

test("flattens pnpm groups and emits a stable name-and-license-only list", () => {
  const pnpmEntries = collectPnpmLicenses({
    MIT: [
      {
        name: "zeta",
        license: "MIT",
        versions: ["1.2.3"],
        author: "Not included",
      },
      { name: "@next/swc-darwin-arm64", license: "MIT" },
      { name: "@next/swc-linux-x64-gnu", license: "MIT" },
    ],
    "Apache-2.0": [
      { name: "@img/sharp-darwin-arm64" },
      { name: "@img/sharp-linux-x64" },
    ],
    "LGPL-3.0-or-later": [
      { name: "@img/sharp-libvips-darwin-arm64" },
      { name: "@img/sharp-libvips-linux-x64" },
    ],
    "MIT OR Apache-2.0": [{ name: "alpha" }],
  });
  const inventory = prepareLicenseInventory([
    ...pnpmEntries,
    { name: "zeta", license: "MIT", copyright: "Not included" },
  ]);

  assert.deepEqual(inventory, [
    { name: "alpha", license: "Apache-2.0 OR MIT" },
    { name: "zeta", license: "MIT" },
  ]);
  assert.deepEqual(Object.keys(inventory[0]).sort(), ["license", "name"]);
});

test("pnpm inventory is stable across host-native build packages", () => {
  const reportForHost = (swc, sharp, libvips) => ({
    MIT: [
      { name: "shared-runtime" },
      { name: swc },
      { name: sharp, license: "Apache-2.0" },
      { name: libvips, license: "LGPL-3.0-or-later" },
    ],
  });

  const darwin = collectPnpmLicenses(
    reportForHost(
      "@next/swc-darwin-arm64",
      "@img/sharp-darwin-arm64",
      "@img/sharp-libvips-darwin-arm64",
    ),
  );
  const linux = collectPnpmLicenses(
    reportForHost(
      "@next/swc-linux-x64-gnu",
      "@img/sharp-linux-x64",
      "@img/sharp-libvips-linux-x64",
    ),
  );

  assert.deepEqual(darwin, linux);
  assert.deepEqual(darwin, [{ name: "shared-runtime", license: "MIT" }]);
});

test("generated inventory includes the bundled sidecar and Tauri opener", async () => {
  const inventory = JSON.parse(
    await readFile(
      new URL("../src/generated/licenses.json", import.meta.url),
      "utf8",
    ),
  );

  assert.ok(
    inventory.some(
      (entry) =>
        entry.name === "Donut Browser" && entry.license === "AGPL-3.0-only",
    ),
  );
  assert.ok(
    inventory.some(
      (entry) => entry.name === "Xray-core" && entry.license === "MPL-2.0",
    ),
  );
  assert.ok(
    inventory.some(
      (entry) =>
        entry.name === "tauri-plugin-opener" &&
        entry.license === "Apache-2.0 OR MIT",
    ),
  );
  assert.ok(
    inventory.every(
      (entry) =>
        Object.keys(entry).length === 2 &&
        typeof entry.name === "string" &&
        typeof entry.license === "string",
    ),
  );
});

test("generated Xray source link matches the packaged release", async () => {
  const source = JSON.parse(
    await readFile(
      new URL("../src/generated/xray-source.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(source, { sourceUrl: XRAY_SOURCE_URL });
});

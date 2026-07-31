import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import parseSpdxExpression from "spdx-expression-parse";
import { XRAY_SOURCE_URL } from "../src-tauri/download-xray.mjs";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const PROJECT_ROOT = resolve(SCRIPT_DIR, "..");
const OUTPUT_PATH = resolve(PROJECT_ROOT, "src/generated/licenses.json");
const XRAY_SOURCE_OUTPUT_PATH = resolve(
  PROJECT_ROOT,
  "src/generated/xray-source.json",
);
const MAX_COMMAND_OUTPUT = 64 * 1024 * 1024;

export const RELEASE_TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "aarch64-unknown-linux-gnu",
  "x86_64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
];

export const MANUAL_LICENSES = [
  {
    name: "Donut Browser",
    license: "AGPL-3.0-only",
  },
  {
    name: "Xray-core",
    license: "MPL-2.0",
  },
];

const LEGACY_LICENSE_EXPRESSIONS = new Map([
  ["Apache-2.0 / MIT", "Apache-2.0 OR MIT"],
  ["Apache-2.0/MIT", "Apache-2.0 OR MIT"],
  ["BSD-3-Clause/MIT", "BSD-3-Clause OR MIT"],
  ["MIT/Apache-2.0", "Apache-2.0 OR MIT"],
  ["MIT OR Apache-2.0", "Apache-2.0 OR MIT"],
  ["Unlicense/MIT", "MIT OR Unlicense"],
]);

const HOST_ONLY_PNPM_NATIVE_PREFIXES = [
  "@img/sharp-",
  "@img/sharp-libvips-",
  "@next/swc-",
];

function validateLicenseExpression(expression) {
  try {
    parseSpdxExpression(expression);
  } catch {
    throw new Error(`Invalid SPDX expression: ${expression}`);
  }
}

export function normalizeLicenseExpression(value) {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error("Every shipped dependency must declare a license");
  }

  const expression =
    LEGACY_LICENSE_EXPRESSIONS.get(value.trim()) ?? value.trim();
  validateLicenseExpression(expression);
  return expression;
}

export function collectReachableRustLicenses(metadata) {
  const root = metadata.resolve?.root;
  if (!root) {
    throw new Error("Cargo metadata did not identify the root package");
  }

  const packages = new Map(
    metadata.packages.map((dependency) => [dependency.id, dependency]),
  );
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const pending = [root];
  const visited = new Set();
  const result = [];

  while (pending.length > 0) {
    const packageId = pending.pop();
    if (!packageId || visited.has(packageId)) continue;
    visited.add(packageId);

    if (packageId !== root) {
      const dependency = packages.get(packageId);
      if (!dependency) {
        throw new Error(`Cargo metadata is missing package ${packageId}`);
      }
      result.push({
        name: dependency.name,
        license: dependency.license,
      });
    }

    const node = nodes.get(packageId);
    if (!node) continue;
    for (const dependency of node.deps) {
      const isRuntimeDependency = dependency.dep_kinds.some(
        ({ kind }) => kind === null,
      );
      if (isRuntimeDependency) pending.push(dependency.pkg);
    }
  }

  return result;
}

export function collectPnpmLicenses(report) {
  return Object.entries(report).flatMap(([groupLicense, dependencies]) =>
    dependencies
      .filter(
        (dependency) =>
          !HOST_ONLY_PNPM_NATIVE_PREFIXES.some((prefix) =>
            dependency.name.startsWith(prefix),
          ),
      )
      .map((dependency) => ({
        name: dependency.name,
        license: dependency.license ?? groupLicense,
      })),
  );
}

export function prepareLicenseInventory(entries) {
  const unique = new Map();

  for (const entry of entries) {
    if (typeof entry.name !== "string" || entry.name.trim() === "") {
      throw new Error("Every shipped dependency must have a name");
    }
    const name = entry.name.trim();
    const license = normalizeLicenseExpression(entry.license);
    unique.set(`${name}\0${license}`, { name, license });
  }

  return [...unique.values()].sort((left, right) => {
    const leftName = left.name.toLowerCase();
    const rightName = right.name.toLowerCase();
    if (leftName < rightName) return -1;
    if (leftName > rightName) return 1;
    if (left.name < right.name) return -1;
    if (left.name > right.name) return 1;
    return left.license < right.license
      ? -1
      : Number(left.license > right.license);
  });
}

function commandOutput(command, args) {
  // pnpm ships only a `pnpm.cmd` batch shim on Windows, and Node refuses to
  // spawn batch files without a shell (CVE-2024-27980), so `execFileSync`
  // fails with EINVAL there. Every argument below is a literal from this file,
  // so routing that one call through cmd.exe interpolates nothing.
  const needsShell = process.platform === "win32" && command === "pnpm";
  return execFileSync(needsShell ? "pnpm.cmd" : command, args, {
    cwd: PROJECT_ROOT,
    encoding: "utf8",
    maxBuffer: MAX_COMMAND_OUTPUT,
    shell: needsShell,
    windowsHide: true,
  });
}

function generateInventory() {
  const entries = [...MANUAL_LICENSES];

  const pnpmReport = JSON.parse(
    commandOutput("pnpm", [
      "--filter",
      "donutbrowser",
      "licenses",
      "list",
      "--prod",
      "--json",
    ]),
  );
  entries.push(...collectPnpmLicenses(pnpmReport));

  for (const target of RELEASE_TARGETS) {
    const metadata = JSON.parse(
      commandOutput("cargo", [
        "metadata",
        "--locked",
        "--format-version",
        "1",
        "--filter-platform",
        target,
        "--manifest-path",
        "src-tauri/Cargo.toml",
      ]),
    );
    entries.push(...collectReachableRustLicenses(metadata));
  }

  return prepareLicenseInventory(entries);
}

function main() {
  const outputs = [
    {
      path: OUTPUT_PATH,
      contents: `${JSON.stringify(generateInventory(), null, 2)}\n`,
    },
    {
      path: XRAY_SOURCE_OUTPUT_PATH,
      contents: `${JSON.stringify({ sourceUrl: XRAY_SOURCE_URL }, null, 2)}\n`,
    },
  ];

  if (process.argv.includes("--check")) {
    for (const output of outputs) {
      const current = readFileSync(output.path, "utf8");
      if (current !== output.contents) {
        throw new Error(`${output.path} is stale; run pnpm licenses:generate`);
      }
    }
    return;
  }

  for (const output of outputs) {
    mkdirSync(dirname(output.path), { recursive: true });
    writeFileSync(output.path, output.contents);
  }
}

const isDirectRun =
  process.argv[1] &&
  fileURLToPath(import.meta.url) === resolve(process.argv[1]);
if (isDirectRun) main();

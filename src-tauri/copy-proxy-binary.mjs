import { execSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const MANIFEST_DIR = dirname(fileURLToPath(import.meta.url));
const PROFILE = process.env.PROFILE || "debug";

function getTarget() {
  if (process.env.TARGET) return process.env.TARGET;
  try {
    const output = execSync("rustc -vV", { encoding: "utf-8" });
    const match = output.match(/host:\s*(.+)/);
    if (match) return match[1].trim();
  } catch {}
  return "unknown";
}

function getHostTarget() {
  try {
    const output = execSync("rustc -vV", { encoding: "utf-8" });
    const match = output.match(/host:\s*(.+)/);
    if (match) return match[1].trim();
  } catch {}
  return "unknown";
}

const TARGET = getTarget();
const HOST_TARGET = getHostTarget();
const isWindows = TARGET.includes("windows");

// Determine source directory
let srcDir;
if (TARGET === HOST_TARGET || TARGET === "unknown") {
  srcDir = join(MANIFEST_DIR, "target", PROFILE === "release" ? "release" : "debug");
} else {
  srcDir = join(MANIFEST_DIR, "target", TARGET, PROFILE === "release" ? "release" : "debug");
}

const destDir = join(MANIFEST_DIR, "binaries");
mkdirSync(destDir, { recursive: true });

function copyBinary(baseName) {
  const binName = isWindows ? `${baseName}.exe` : baseName;
  const source = join(srcDir, binName);

  let destName = `${baseName}-${TARGET}`;
  if (isWindows) destName += ".exe";
  const dest = join(destDir, destName);

  if (existsSync(source)) {
    copyFileSync(source, dest);
    console.log(`Copied ${binName} to ${dest}`);
  } else {
    console.log(`Warning: Binary not found at ${source}`);
    console.log(`Building ${baseName} binary...`);

    const buildArgs = ["build", "--bin", baseName];
    if (PROFILE === "release") buildArgs.push("--release");
    if (TARGET !== "unknown" && TARGET !== HOST_TARGET) {
      buildArgs.push("--target", TARGET);
    }

    execSync(`cargo ${buildArgs.join(" ")}`, {
      cwd: MANIFEST_DIR,
      stdio: "inherit",
    });

    if (existsSync(source)) {
      copyFileSync(source, dest);
      console.log(`Built and copied ${binName} to ${dest}`);
    } else {
      console.error(`Error: Failed to build ${baseName} binary`);
      process.exit(1);
    }
  }
}

copyBinary("donut-proxy");
copyBinary("donut-daemon");

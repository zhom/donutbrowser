import { chmod, mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import {
  redactSensitiveText,
  sensitiveVariants,
} from "../../scripts/redact-sensitive-text.mjs";

const MAX_LOG_BYTES = 512 * 1024;

async function logFiles(directory, fileNamePattern = /\.(?:log|txt)$/iu) {
  const entries = await readdir(directory, { withFileTypes: true }).catch(
    () => [],
  );
  return entries
    .filter((entry) => entry.isFile() && fileNamePattern.test(entry.name))
    .map((entry) => path.join(directory, entry.name))
    .sort();
}

async function diagnosticSources(runRoot) {
  const sources = await logFiles(path.join(runRoot, "logs"));
  const sessions = await readdir(path.join(runRoot, "sessions"), {
    withFileTypes: true,
  }).catch(() => []);
  for (const session of sessions.filter((entry) => entry.isDirectory())) {
    const root = path.join(runRoot, "sessions", session.name);
    sources.push(...(await logFiles(path.join(root, "donut", "logs"))));
    sources.push(
      ...(await logFiles(path.join(root, "tmp"), /^donut-proxy-.*\.log$/iu)),
    );
  }
  return sources;
}

export async function assertSafeDiagnostics(
  diagnosticsRoot,
  sensitiveValues = [],
) {
  const entries = await readdir(diagnosticsRoot, { withFileTypes: true });
  for (const entry of entries) {
    if (!entry.isFile() || !/\.(?:json|log)$/iu.test(entry.name)) {
      throw new Error(`Unsafe diagnostics entry: ${entry.name}`);
    }
    const content = await readFile(
      path.join(diagnosticsRoot, entry.name),
      "utf8",
    );
    for (const value of sensitiveVariants(sensitiveValues)) {
      if (content.includes(value)) {
        throw new Error(
          `Sensitive value survived diagnostics redaction in ${entry.name}`,
        );
      }
    }
  }
}

export async function createSafeDiagnostics(
  runRoot,
  { suite, failed, sensitiveValues = [] },
) {
  const diagnosticsRoot = path.join(runRoot, "diagnostics");
  await mkdir(diagnosticsRoot, { recursive: true, mode: 0o700 });
  await chmod(diagnosticsRoot, 0o700);

  const sources = await diagnosticSources(runRoot);
  for (const [index, source] of sources.entries()) {
    const content = await readFile(source, "utf8").catch(() => "");
    const tail = content.slice(-MAX_LOG_BYTES);
    const destination = path.join(
      diagnosticsRoot,
      `${String(index + 1).padStart(3, "0")}.log`,
    );
    await writeFile(
      destination,
      redactSensitiveText(tail, { sensitiveValues }),
      { mode: 0o600 },
    );
    await chmod(destination, 0o600);
  }

  const summaryPath = path.join(diagnosticsRoot, "summary.json");
  await writeFile(
    summaryPath,
    `${JSON.stringify(
      {
        suite,
        status: failed ? "failed" : "passed",
        sanitized_log_files: sources.length,
      },
      null,
      2,
    )}\n`,
    { mode: 0o600 },
  );
  await chmod(summaryPath, 0o600);
  await assertSafeDiagnostics(diagnosticsRoot, sensitiveValues);
  return diagnosticsRoot;
}

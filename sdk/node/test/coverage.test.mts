/**
 * The SDK cannot silently drift from the app's API.
 *
 * `sdk/api-paths.json` is generated from `src-tauri/src/api_server.rs` and
 * lists every operation the desktop app publishes. These tests hold it against
 * the SDK's own table in both directions, so a new endpoint in the app fails
 * here until it is wrapped or deliberately omitted with a reason.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { DonutClient, OMITTED, OPERATIONS } from "../src/index.mts";

const SNAPSHOT = fileURLToPath(new URL("../../api-paths.json", import.meta.url));

interface Snapshot {
  source: string;
  operation_count: number;
  operations: { operation_id: string; method: string; path: string }[];
}

function snapshot(): Snapshot {
  return JSON.parse(readFileSync(SNAPSHOT, "utf8")) as Snapshot;
}

function published(): Set<string> {
  return new Set(snapshot().operations.map((entry) => `${entry.method} ${entry.path}`));
}

test("the snapshot is readable and not empty", () => {
  const document = snapshot();
  assert.equal(document.source, "src-tauri/src/api_server.rs");
  assert.equal(document.operation_count, document.operations.length);
  assert.ok(document.operation_count > 0);
  assert.equal(
    published().size,
    document.operation_count,
    "the app has two identical operations",
  );
});

test("every published operation is wrapped or omitted", () => {
  const known = new Set([...OPERATIONS.keys(), ...OMITTED.keys()]);
  const missing = [...published()].filter((key) => !known.has(key)).sort();
  assert.deepEqual(
    missing,
    [],
    `the app publishes operations this SDK does not handle: ${missing.join(", ")}. ` +
      "Wrap each one, or add it to OMITTED with a reason.",
  );
});

test("the SDK claims nothing the app does not publish", () => {
  const live = published();
  const stale = [...OPERATIONS.keys(), ...OMITTED.keys()].filter((key) => !live.has(key)).sort();
  assert.deepEqual(
    stale,
    [],
    `this SDK handles operations the app no longer publishes: ${stale.join(", ")}. ` +
      "Regenerate the snapshot with sdk/tools/extract-api-paths.py, then drop or fix each entry.",
  );
});

test("an operation is either wrapped or omitted but not both", () => {
  const both = [...OPERATIONS.keys()].filter((key) => OMITTED.has(key)).sort();
  assert.deepEqual(both, [], `listed twice: ${both.join(", ")}`);
});

test("every omission gives a reason", () => {
  for (const [operation, reason] of OMITTED) {
    assert.ok(reason.trim().length > 40, `${operation} is omitted without a real reason`);
  }
});

test("every wrapped operation names a real method", () => {
  const prototype = DonutClient.prototype as unknown as Record<string, unknown>;
  for (const [operation, name] of OPERATIONS) {
    assert.equal(
      typeof prototype[name],
      "function",
      `${operation} names ${name}, which is not a method`,
    );
  }
});

test("no two operations share a method", () => {
  const names = [...OPERATIONS.values()];
  const duplicates = [...new Set(names.filter((name, index) => names.indexOf(name) !== index))];
  assert.deepEqual(
    duplicates,
    [],
    `one method is claimed by several operations: ${duplicates.join(", ")}`,
  );
});

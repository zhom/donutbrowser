import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { STEP_TYPES, TARGETED } from "./recipes.ts";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const RUST = readFileSync(
  path.join(HERE, "..", "..", "src-tauri", "src", "recipes.rs"),
  "utf8",
);

function rustList(name) {
  const start = RUST.indexOf(`const ${name}`);
  assert.ok(start >= 0, `${name} is no longer declared in recipes.rs`);
  const body = RUST.slice(RUST.indexOf("=", start), RUST.indexOf("];", start));
  return [...body.matchAll(/"([a-zA-Z]+)"/g)].map((match) => match[1]);
}

test("the editor offers exactly the step kinds the backend validates", () => {
  assert.deepEqual([...STEP_TYPES], rustList("RECIPE_STEP_TYPES"));
});

test("the kinds that need a target agree on both sides", () => {
  assert.deepEqual([...TARGETED], rustList("RECIPE_TARGETED_TYPES"));
  for (const kind of TARGETED) assert.ok(STEP_TYPES.includes(kind), kind);
});

test("the backend still validates a recipe before it leaves the machine", () => {
  assert.match(RUST, /fn validate_recipe\(/);
});

test("the editor reads its step lists from the shared module", () => {
  const editor = readFileSync(
    path.join(HERE, "..", "components", "agent-recipe-steps.tsx"),
    "utf8",
  );
  assert.ok(!/const STEP_TYPES/.test(editor), "a second STEP_TYPES list");
  assert.ok(!/const TARGETED/.test(editor), "a second TARGETED list");
  assert.match(editor, /from "@\/lib\/recipes"/);
});

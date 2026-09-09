import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  AGENT_GOAL_MAX_CHARS,
  agentGoalProblem,
  agentStepSummary,
  isRunCancellable,
  isRunOver,
  parseAllowedHosts,
} from "./agent.ts";

/**
 * The agent run form's validation, and the two places it has to agree with the
 * Rust wire.
 *
 * The form disables its submit button on `agentGoalProblem` and sends whatever
 * `parseAllowedHosts` produced; `src-tauri/src/agent.rs` refuses the same goal
 * and normalises the same hosts before the request leaves the machine. If the
 * two drift, the button says one thing and the backend does another — the count
 * shown next to the field stops describing what actually travels, and a goal
 * the form accepts is refused with a code the user cannot act on.
 */

const HERE = path.dirname(fileURLToPath(import.meta.url));
const RUST = readFileSync(
  path.join(HERE, "..", "..", "src-tauri", "src", "agent.rs"),
  "utf8",
);

test("a goal has to say something", () => {
  assert.equal(agentGoalProblem(""), "empty");
  assert.equal(agentGoalProblem("   "), "empty");
  assert.equal(agentGoalProblem("\n\t "), "empty");
  assert.equal(agentGoalProblem(" buy milk "), null);
});

test("a goal longer than the ceiling is refused before it costs a round trip", () => {
  assert.equal(agentGoalProblem("a".repeat(AGENT_GOAL_MAX_CHARS)), null);
  assert.equal(
    agentGoalProblem("a".repeat(AGENT_GOAL_MAX_CHARS + 1)),
    "tooLong",
  );
  // Counted in characters, not UTF-16 code units: a goal written in emoji or in
  // any astral script is not silently half as long as the counter claims.
  assert.equal(agentGoalProblem("😀".repeat(AGENT_GOAL_MAX_CHARS)), null);
  assert.equal(
    agentGoalProblem("😀".repeat(AGENT_GOAL_MAX_CHARS + 1)),
    "tooLong",
  );
});

test("the goal ceiling is the same number the Rust wire enforces", () => {
  const match = RUST.match(/MAX_GOAL_CHARS:\s*usize\s*=\s*(\d+);/);
  assert.ok(match, "MAX_GOAL_CHARS is no longer declared in agent.rs");
  assert.equal(
    Number(match[1]),
    AGENT_GOAL_MAX_CHARS,
    "the form would accept a goal the backend refuses, or refuse one it accepts",
  );
});

test("an allowlist is trimmed, lowercased and de-duplicated", () => {
  assert.deepEqual(
    parseAllowedHosts("Example.com\n  example.com \n\ndocs.example.com"),
    ["example.com", "docs.example.com"],
  );
  // Commas as well as newlines, because both are how a person pastes a list.
  assert.deepEqual(parseAllowedHosts("a.com, b.com , a.com"), [
    "a.com",
    "b.com",
  ]);
  assert.deepEqual(parseAllowedHosts("   "), []);
  assert.deepEqual(parseAllowedHosts(""), []);
});

test("the allowlist rule matches the one the Rust wire applies", () => {
  // `normalise_hosts` runs again on the way out, so a rule only one side knows
  // makes the count beside the field a lie about what actually travels.
  const body = RUST.slice(RUST.indexOf("pub fn normalise_hosts"));
  assert.match(body, /trim\(\)\.to_lowercase\(\)/);
  assert.match(body, /seen\.contains\(&host\)/);
  assert.match(body, /host\.is_empty\(\)/);
});

test("only a finished run stops being cancellable", () => {
  for (const status of ["queued", "running"]) {
    assert.equal(isRunOver({ status }), false, status);
    assert.equal(isRunCancellable({ status }), true, status);
  }
  for (const status of ["succeeded", "failed", "cancelled"]) {
    assert.equal(isRunOver({ status }), true, status);
    assert.equal(isRunCancellable({ status }), false, status);
  }
});

test("a status this release has never heard of is neither over nor cancellable", () => {
  // The state machine is the server's. Treating an unknown status as finished
  // would strand a live run with no cancel button; treating it as cancellable
  // would offer an action the backend refuses.
  assert.equal(isRunOver({ status: "paused_for_review" }), false);
  assert.equal(isRunCancellable({ status: "paused_for_review" }), false);
});

test("a step is summarised from whatever field the server actually sent", () => {
  assert.equal(
    agentStepSummary({ index: 0, kind: "thought", text: "  look for it  " }),
    "look for it",
  );
  assert.equal(
    agentStepSummary({ index: 1, kind: "tool", tool: "click" }),
    "click",
  );
  assert.equal(
    agentStepSummary({ index: 2, kind: "error", message: "timed out" }),
    "timed out",
  );
  // `summary` wins when the server sends both, so a purpose-written line is
  // never passed over for a raw tool name.
  assert.equal(
    agentStepSummary({
      index: 3,
      kind: "tool",
      summary: "opened the report",
      tool: "click",
    }),
    "opened the report",
  );
});

test("a step nobody described summarises as nothing rather than as junk", () => {
  // The renderer falls back to naming the kind. Returning "[object Object]" or
  // an empty string here would put a blank row under a real step.
  assert.equal(agentStepSummary({ index: 0, kind: "finish" }), null);
  assert.equal(agentStepSummary({ index: 1, kind: "tool", text: "   " }), null);
  assert.equal(
    agentStepSummary({ index: 2, kind: "tool", args: { selector: "#buy" } }),
    null,
  );
});

test("a recipe step is an object the API will accept, never a line of prose", () => {
  const source = readFileSync(
    new URL("../components/agent-recipe-steps.tsx", import.meta.url),
    "utf8",
  );

  // The wire shape is the API's, so the editor must offer exactly the kinds it
  // validates. A kind here that the server does not know is a save that fails
  // for the user with nothing on screen to explain it.
  const offered = source
    .slice(source.indexOf("const STEP_TYPES"), source.indexOf("];"))
    .match(/"[a-zA-Z]+"/g)
    .map((quoted) => quoted.slice(1, -1));
  assert.deepEqual(offered, [
    "navigate",
    "click",
    "type",
    "waitFor",
    "extract",
    "pressKey",
    "scroll",
    "screenshot",
    "sleep",
  ]);

  // And the kinds that name an element are exactly the ones the validator
  // requires a target for, on both sides.
  const targeted = source
    .slice(
      source.indexOf("const TARGETED"),
      source.indexOf("];", source.indexOf("const TARGETED")),
    )
    .match(/"[a-zA-Z]+"/g)
    .map((quoted) => quoted.slice(1, -1));
  assert.deepEqual(targeted, ["click", "type", "waitFor", "extract"]);

  const rust = readFileSync(
    new URL("../../src-tauri/src/agent.rs", import.meta.url),
    "utf8",
  );
  for (const kind of offered) {
    assert.ok(
      rust.includes(`"${kind}"`),
      `${kind} must also be accepted by validate_recipe in agent.rs`,
    );
  }
});

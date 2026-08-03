import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { SCHEDULE_BOUNDS } from "./cookie-bot-limits.ts";

/**
 * The enrolment form validates against bounds that belong to the server. This
 * file is the tripwire for that mirror.
 *
 * The platform list (`BOT_PLATFORMS`) and the preflight refusals are both
 * pinned — one by a cross-referenced comment in `cookie_bot.rs`, the other by
 * tests beside it. These four numbers were not pinned by anything, so infra
 * could widen `max_minutes` to 180 and the desktop would go on refusing 150
 * with no field-level explanation, or cap sites at 25 and let a user fill a
 * form the PUT answers with `COOKIE_BOT_INVALID_SCHEDULE`.
 */

test("the mirrored bounds are exactly what the server enforces", () => {
  // Read off `validateScheduleBody` / `normaliseSites` and the constants in
  // donutbrowser-infra's apps/backend/src/cookie-bot/cookie-bot-schedule.ts.
  // Changing a number here without changing it there is the bug.
  assert.deepEqual(
    { ...SCHEDULE_BOUNDS },
    { minMaxMinutes: 5, maxMaxMinutes: 120, minSites: 1, maxSites: 40 },
  );
});

test("the bounds describe a range a user can actually satisfy", () => {
  assert.ok(
    SCHEDULE_BOUNDS.minMaxMinutes < SCHEDULE_BOUNDS.maxMaxMinutes,
    "an empty minute range would disable every enrolment",
  );
  assert.ok(
    SCHEDULE_BOUNDS.minSites >= 1,
    "the bot browses declared sites only, so at least one is required",
  );
  assert.ok(SCHEDULE_BOUNDS.minSites <= SCHEDULE_BOUNDS.maxSites);
});

test("the enrolment form takes its bounds from here, not from its own literals", () => {
  const source = readFileSync(
    fileURLToPath(
      new URL("../components/cookie-bot-enrol-dialog.tsx", import.meta.url),
    ),
    "utf8",
  );
  assert.match(
    source,
    /from "@\/lib\/cookie-bot-limits"/,
    "the dialog must import the shared bounds",
  );
  for (const name of [
    "MIN_MAX_MINUTES",
    "MAX_MAX_MINUTES",
    "MIN_SITES",
    "MAX_SITES",
  ]) {
    assert.doesNotMatch(
      source,
      new RegExp(`const ${name}\\s*=\\s*\\d`),
      `${name} must be derived from SCHEDULE_BOUNDS, not re-declared as a literal`,
    );
  }
});

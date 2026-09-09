import assert from "node:assert/strict";
import test from "node:test";
import { scheduleLanes, scheduleSlots } from "./schedule-layout.ts";

function booking(profile_id, minute, options = {}) {
  return {
    profile_id,
    profile_name: profile_id,
    enabled: true,
    timezone: "UTC",
    days_mask: 127,
    run_at_minute: minute,
    max_minutes: 30,
    ...options,
  };
}

test("all slots survive, coincident weekdays merge, and legacy schedules still render", () => {
  const legacy = booking("legacy", 90);
  assert.deepEqual(scheduleSlots(legacy), [
    { days_mask: 127, run_at_minute: 90 },
  ]);
  const multi = booking("multiple", 120, {
    slots: [
      { days_mask: 1, run_at_minute: 120 },
      { days_mask: 2, run_at_minute: 120 },
      { days_mask: 127, run_at_minute: 600 },
    ],
  });
  const lanes = scheduleLanes([legacy, multi]).get("UTC");
  assert.deepEqual(
    lanes.map(({ minute, days }) => [minute, days]),
    [
      [90, 127],
      [120, 3],
      [600, 127],
    ],
  );
  assert.equal(lanes[0].overlaps, 0, "adjacent reservations do not overlap");
});

test("overlaps honor weekday masks, disabled and blocked schedules, and timezone boundaries", () => {
  const lanes = scheduleLanes([
    booking("monday", 120, { days_mask: 1 }),
    booking("same", 135, { days_mask: 1 }),
    booking("tuesday", 135, { days_mask: 2 }),
    booking("disabled", 130, { enabled: false }),
    booking("blocked", 130, { blocked_by: "no_proxy" }),
    booking("other-zone", 135, { timezone: "Asia/Yerevan" }),
  ]);
  assert.deepEqual(
    Object.fromEntries(
      lanes.get("UTC").map((lane) => [lane.schedule.profile_id, lane.overlaps]),
    ),
    { monday: 1, blocked: 0, disabled: 0, same: 1, tuesday: 0 },
  );
  assert.equal(lanes.get("Asia/Yerevan")[0].overlaps, 0);
});

test("Sunday reservations crossing midnight overlap Monday without inventing daily conflicts", () => {
  const lanes = scheduleLanes([
    booking("sunday", 1430, { days_mask: 64 }),
    booking("monday", 5, { days_mask: 1 }),
    booking("tuesday", 5, { days_mask: 2 }),
  ]).get("UTC");
  assert.equal(
    lanes.find((lane) => lane.schedule.profile_id === "sunday").overlaps,
    1,
  );
  assert.equal(
    lanes.find((lane) => lane.schedule.profile_id === "tuesday").overlaps,
    0,
  );
});

test("a confirmed reschedule keeps its visual identity", () => {
  const before = scheduleLanes([booking("same", 120)]).get("UTC")[0];
  const after = scheduleLanes([booking("same", 180)]).get("UTC")[0];
  assert.equal(before.id, after.id);
  assert.notEqual(before.minute, after.minute);
});

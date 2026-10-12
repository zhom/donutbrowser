import assert from "node:assert/strict";
import test from "node:test";
import {
  isoToLocalInput,
  isWebhookUrl,
  minuteToTime,
  scheduleBody,
  timeToMinute,
} from "./schedules.ts";

test("times convert both ways and refuse what is not a time", () => {
  assert.equal(minuteToTime(9 * 60 + 5), "09:05");
  assert.equal(timeToMinute("09:05"), 545);
  assert.equal(timeToMinute("23:59"), 1439);
  for (const bad of ["24:00", "9", "ab:cd", "12:60"]) {
    assert.equal(timeToMinute(bad), null, bad);
  }
  assert.equal(isoToLocalInput(null), "");
  assert.match(
    isoToLocalInput("2030-01-02T03:04:00Z"),
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/,
  );
});

test("a schedule sends only the fields its cadence reads", () => {
  const base = {
    name: "  Nightly ",
    recipeId: "r1",
    target: "fleet",
    profileIds: ["p1"],
    daysMask: 31,
    runAtMinute: 540,
    timezone: "UTC",
    runOnceAt: "2030-01-01T00:00:00.000Z",
    intervalMinutes: 60,
  };
  const weekly = scheduleBody({ ...base, cadence: "weekly" });
  assert.equal(weekly.name, "Nightly");
  assert.deepEqual(
    [weekly.daysMask, weekly.runAtMinute, weekly.timezone],
    [31, 540, "UTC"],
  );
  assert.equal(weekly.runOnceAt, undefined);
  assert.equal(weekly.intervalMinutes, undefined);
  const once = scheduleBody({ ...base, cadence: "once" });
  assert.equal(once.runOnceAt, "2030-01-01T00:00:00.000Z");
  assert.equal(once.daysMask, undefined);
  const interval = scheduleBody({ ...base, cadence: "interval" });
  assert.equal(interval.intervalMinutes, 60);
  assert.equal(interval.timezone, undefined);
});

test("a webhook is https without credentials, and an empty one clears it", () => {
  assert.equal(isWebhookUrl(""), true);
  assert.equal(isWebhookUrl("https://hooks.example.com/donut"), true);
  for (const bad of [
    "http://hooks.example.com/donut",
    "https://user:pass@hooks.example.com/",
    "not a url",
  ]) {
    assert.equal(isWebhookUrl(bad), false, bad);
  }
  const body = scheduleBody({
    name: "n",
    recipeId: "r",
    target: "fleet",
    profileIds: ["p"],
    cadence: "interval",
    intervalMinutes: 60,
    notifyEmail: false,
    webhookUrl: "  ",
  });
  assert.equal(body.notifyEmail, false);
  assert.equal(body.webhookUrl, null);
});

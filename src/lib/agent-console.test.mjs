import assert from "node:assert/strict";
import test from "node:test";
import {
  activityDetail,
  activityStats,
  appendActivity,
  collapseProgressRuns,
  isSessionActive,
  isSessionEarlier,
  MAX_ACTIVITY,
  openRequests,
  profileRollup,
  progressPercent,
  quotaLeft,
  sessionLabel,
  threadForSession,
  upsertSession,
  upsertThreadItem,
} from "./agent-console.ts";

const NOW = 10_000_000;

function session(overrides = {}) {
  return {
    session_id: "s1",
    client_name: "claude-code",
    client_version: "2.0.0",
    connected_at: NOW - 60_000,
    last_seen_at: NOW,
    calls: 0,
    errors: 0,
    ended: false,
    status: null,
    ...overrides,
  };
}

function item(overrides = {}) {
  return {
    id: 1,
    at: NOW,
    kind: "question",
    session_id: "s1",
    text: "Which account?",
    profile_id: null,
    choices: [],
    state: "open",
    answer: null,
    answered_at: null,
    delivered_to: [],
    done: null,
    total: null,
    ...overrides,
  };
}

function call(overrides = {}) {
  return {
    id: 1,
    at: NOW,
    session_id: "s1",
    tool: "navigate",
    profile_id: "p1",
    profile_count: null,
    ok: true,
    error_code: null,
    duration_ms: 100,
    detail: null,
    ...overrides,
  };
}

test("a session is active for 90 seconds after its last call, unless it ended", () => {
  assert.equal(
    isSessionActive(session({ last_seen_at: NOW - 90_000 }), NOW),
    true,
  );
  assert.equal(
    isSessionActive(session({ last_seen_at: NOW - 90_001 }), NOW),
    false,
  );
  assert.equal(isSessionActive(session({ ended: true }), NOW), false);
});

test("a session idle for more than 30 minutes moves behind Show earlier", () => {
  assert.equal(
    isSessionEarlier(session({ last_seen_at: NOW - 30 * 60_000 }), NOW),
    false,
  );
  assert.equal(
    isSessionEarlier(session({ last_seen_at: NOW - 30 * 60_000 - 1 }), NOW),
    true,
  );
});

test("only open questions and help requests need the person, oldest first", () => {
  const thread = [
    item({ id: 5, kind: "help" }),
    item({ id: 2 }),
    item({ id: 3, state: "answered" }),
    item({ id: 4, kind: "note", state: "pending" }),
    item({ id: 6, kind: "help", state: "dismissed" }),
  ];
  assert.deepEqual(
    openRequests(thread).map((entry) => entry.id),
    [2, 5],
  );
});

test("sessions upsert by id and stay ordered by last call", () => {
  let sessions = upsertSession(
    [],
    session({ session_id: "a", last_seen_at: NOW - 10 }),
  );
  sessions = upsertSession(
    sessions,
    session({ session_id: "b", last_seen_at: NOW }),
  );
  sessions = upsertSession(
    sessions,
    session({ session_id: "a", last_seen_at: NOW + 5, calls: 3 }),
  );
  assert.deepEqual(
    sessions.map((entry) => entry.session_id),
    ["a", "b"],
  );
  assert.equal(sessions[0].calls, 3);
});

test("thread items upsert by id and keep id order", () => {
  let thread = upsertThreadItem([], item({ id: 2 }));
  thread = upsertThreadItem(thread, item({ id: 1 }));
  thread = upsertThreadItem(
    thread,
    item({ id: 2, state: "answered", answer: "A" }),
  );
  assert.deepEqual(
    thread.map((entry) => [entry.id, entry.state]),
    [
      [1, "open"],
      [2, "answered"],
    ],
  );
});

test("activity appends only newer entries and keeps the newest thousand", () => {
  const held = [call({ id: 1 }), call({ id: 2 })];
  assert.deepEqual(
    appendActivity(held, [call({ id: 2 }), call({ id: 3 })]).map(
      (entry) => entry.id,
    ),
    [1, 2, 3],
  );
  assert.equal(appendActivity(held, [call({ id: 1 })]), held);
  const many = Array.from({ length: MAX_ACTIVITY }, (_, index) =>
    call({ id: index }),
  );
  const next = appendActivity(many, [call({ id: MAX_ACTIVITY })]);
  assert.equal(next.length, MAX_ACTIVITY);
  assert.equal(next[0].id, 1);
});

test("one agent's thread holds its items and notes sent to everyone", () => {
  const thread = [
    item({ id: 1, session_id: "a" }),
    item({ id: 2, session_id: "b" }),
    item({ id: 3, kind: "note", session_id: null, state: "pending" }),
    item({ id: 4, kind: "note", session_id: "b", state: "pending" }),
  ];
  assert.deepEqual(
    threadForSession(thread, "a").map((entry) => entry.id),
    [1, 3],
  );
  assert.equal(threadForSession(thread, null).length, 4);
});

test("a run of progress items collapses to its latest", () => {
  const thread = [
    item({ id: 1, kind: "progress", state: "none" }),
    item({ id: 2, kind: "progress", state: "none" }),
    item({ id: 3 }),
    item({ id: 4, kind: "progress", state: "none" }),
    item({ id: 5, kind: "progress", state: "none", session_id: "s2" }),
  ];
  assert.deepEqual(
    collapseProgressRuns(thread).map((entry) => entry.id),
    [2, 3, 4, 5],
  );
});

test("activity details are read per tool and never invented", () => {
  assert.deepEqual(activityDetail(call({ detail: "example.com" })), {
    kind: "host",
    host: "example.com",
  });
  assert.deepEqual(activityDetail(call({ tool: "type_text", detail: "42" })), {
    kind: "typed",
    count: 42,
  });
  assert.deepEqual(
    activityDetail(call({ tool: "batch_run", profile_count: 7 })),
    {
      kind: "profiles",
      count: 7,
    },
  );
  assert.equal(activityDetail(call({ tool: "click", detail: "x" })), null);
  assert.equal(
    activityDetail(call({ tool: "type_locator", detail: "many" })),
    null,
  );
  assert.equal(activityDetail(call({ detail: null })), null);
});

test("stats cover the last five minutes only", () => {
  const activity = [
    call({ id: 1, at: NOW - 6 * 60_000, ok: false, duration_ms: 9000 }),
    call({ id: 2, at: NOW - 60_000, duration_ms: 100 }),
    call({ id: 3, at: NOW - 1000, ok: false, duration_ms: 300 }),
  ];
  assert.deepEqual(activityStats(activity, NOW), {
    calls: 2,
    errors: 1,
    errorRate: 0.5,
    averageMs: 200,
  });
  assert.deepEqual(activityStats([], NOW), {
    calls: 0,
    errors: 0,
    errorRate: 0,
    averageMs: null,
  });
});

test("the profile rollup shows the latest call and the person's holds", () => {
  const activity = [
    call({ id: 1, profile_id: "p1", at: NOW - 200_000 }),
    call({ id: 2, profile_id: "p2", at: NOW - 1000 }),
    call({
      id: 3,
      profile_id: "p3",
      at: NOW - 500,
      ok: false,
      error_code: "LOCATOR_NO_MATCH",
    }),
    call({ id: 4, profile_id: null, at: NOW }),
  ];
  const holds = [
    { profile_id: "p4", since: NOW - 100, note: null, request_id: null },
  ];
  const rows = profileRollup(activity, holds, NOW);
  assert.deepEqual(
    rows.map((row) => [row.profile_id, row.state]),
    [
      ["p4", "held"],
      ["p3", "error"],
      ["p2", "working"],
      ["p1", "idle"],
    ],
  );
  assert.equal(rows[0].last, null);
});

test("quota and progress helpers hide what they cannot show", () => {
  assert.equal(quotaLeft(null), null);
  assert.equal(quotaLeft({ limit: 0, used: 5, resets_in_secs: null }), null);
  assert.deepEqual(quotaLeft({ limit: 100, used: 140, resets_in_secs: 10 }), {
    left: 0,
    limit: 100,
  });
  assert.equal(progressPercent(3, null), null);
  assert.equal(progressPercent(3, 0), null);
  assert.equal(progressPercent(1, 3), 33);
  assert.equal(progressPercent(9, 3), 100);
});

test("sessionLabel names the website, other clients, and nameless ones", () => {
  assert.equal(
    sessionLabel(
      { client_name: "donutbrowser-web" },
      "MCP client",
      "Donut website",
    ),
    "Donut website",
  );
  assert.equal(
    sessionLabel({ client_name: " Cursor " }, "MCP client", "Donut website"),
    "Cursor",
  );
  assert.equal(sessionLabel({ client_name: null }, "MCP client"), "MCP client");
  assert.equal(sessionLabel(null, "MCP client"), "MCP client");
});

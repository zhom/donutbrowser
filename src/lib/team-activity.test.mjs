import assert from "node:assert/strict";
import test from "node:test";
import {
  actionKey,
  actorKind,
  canChangeAccess,
  fieldKeys,
  sinceFor,
  targetLabel,
} from "./team-activity.ts";

test("known actions have words and unknown ones do not", () => {
  assert.equal(actionKey("profile.bulk_update"), "profileBulkUpdate");
  assert.equal(actionKey("extension.group_create"), "extensionGroupCreate");
  assert.equal(actionKey("member.role_change"), "memberRoleChange");
  assert.equal(actionKey("profile.teleport"), null);
});

test("only the fields this build can name are shown", () => {
  assert.deepEqual(fieldKeys({ fields: ["tags", "dns_server", "secret"] }), [
    "tags",
    "dnsServer",
  ]);
  assert.deepEqual(fieldKeys({ fields: "tags" }), []);
  assert.deepEqual(fieldKeys(null), []);
});

test("a target reads as its local name first, then the log's, then a short id", () => {
  const names = new Map([["p1", "Shop"]]);
  assert.equal(
    targetLabel({ type: "profile", id: "p1", name: "Old" }, names),
    "Shop",
  );
  assert.equal(
    targetLabel({ type: "profile", id: "p2", name: "Old" }, names),
    "Old",
  );
  assert.equal(
    targetLabel({ type: "profile", id: "0123456789abcdef", name: null }, names),
    "01234567…",
  );
  assert.equal(
    targetLabel({ type: "member", id: null, name: "a@b.c" }, names),
    "a@b.c",
  );
  assert.equal(targetLabel({ type: null, id: null, name: null }, names), null);
});

test("an actor kind this build does not know is not shown", () => {
  assert.equal(actorKind("mcp"), "mcp");
  assert.equal(actorKind("satellite"), null);
});

test("a period reads back that many days, and all reads everything", () => {
  const now = new Date("2026-10-12T12:00:00Z");
  assert.equal(sinceFor("1", now), "2026-10-11T12:00:00.000Z");
  assert.equal(sinceFor("30", now), "2026-09-12T12:00:00.000Z");
  assert.equal(sinceFor("all", now), undefined);
});

test("the owner is never limited, and an admin limits only members below it", () => {
  const member = (role) => ({
    userId: "u",
    email: null,
    role,
    groupAccess: null,
  });
  assert.equal(canChangeAccess("owner", member("owner")), false);
  assert.equal(canChangeAccess("owner", member("admin")), true);
  assert.equal(canChangeAccess("admin", member("admin")), false);
  assert.equal(canChangeAccess("admin", member("launcher")), true);
  assert.equal(canChangeAccess("member", member("launcher")), false);
});

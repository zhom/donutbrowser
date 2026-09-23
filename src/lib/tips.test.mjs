import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  autoTipCandidates,
  isPlanTip,
  pickAutoTip,
  TIPS,
  tipsFor,
} from "./tips.ts";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const LOCALES = path.join(HERE, "..", "i18n", "locales");

/** An install that uses nothing a tip talks about. */
const IDLE = {
  profiles: [],
  currentOs: "macos",
  groupCount: 0,
  extensionGroupCount: 0,
  inTeam: false,
  cookieBotEnrolled: false,
  syncServerConfigured: false,
};

const ALL_PLAN = {
  active: true,
  cloudBackup: true,
  cookieBot: true,
  crossOsFingerprints: true,
  browserAutomation: true,
  agentAutomation: true,
  teamCollaboration: true,
  remoteControl: true,
};

function ids(tips) {
  return tips.map((tip) => tip.id);
}

const NONE = {
  active: false,
  cloudBackup: false,
  cookieBot: false,
  crossOsFingerprints: false,
  browserAutomation: false,
  agentAutomation: false,
  teamCollaboration: false,
  remoteControl: false,
};

test("tip ids are unique and every tip has copy in every locale", () => {
  const ids = TIPS.map((tip) => tip.id);
  assert.equal(new Set(ids).size, ids.length);
  const locales = readdirSync(LOCALES).filter((name) => name.endsWith(".json"));
  assert.ok(locales.length >= 2, "expected several locale files");
  for (const name of locales) {
    const bundle = JSON.parse(readFileSync(path.join(LOCALES, name), "utf8"));
    for (const id of ids) {
      const item = bundle.tips?.items?.[id];
      assert.ok(item, `${name} is missing tips.items.${id}`);
      for (const field of ["label", "title", "body", "action"]) {
        assert.equal(
          typeof item[field],
          "string",
          `${name}: tips.items.${id}.${field} must be a string`,
        );
        assert.ok(
          item[field].trim().length > 0,
          `${name}: tips.items.${id}.${field} is empty`,
        );
      }
    }
  }
});

test("a free install is offered every essential and no plan tip", () => {
  const offered = tipsFor(NONE);
  assert.ok(offered.length > 0);
  assert.ok(offered.every((tip) => !isPlanTip(tip)));
  assert.equal(offered.length, TIPS.filter((tip) => !isPlanTip(tip)).length);
});

test("a plan tip needs an active plan with that capability", () => {
  const solo = { ...NONE, active: true, cloudBackup: true, cookieBot: true };
  const offered = tipsFor(solo).map((tip) => tip.id);
  assert.ok(offered.includes("cloudBackup"));
  assert.ok(offered.includes("cookieBot"));
  assert.ok(!offered.includes("team"), "solo has no team collaboration");
  assert.ok(!offered.includes("remoteControl"));

  const lapsed = { ...solo, active: false };
  assert.ok(
    tipsFor(lapsed).every((tip) => !isPlanTip(tip)),
    "a lapsed plan is offered only the essentials",
  );
});

test("the automatic flow picks one unseen tip at random", () => {
  const offered = tipsFor(NONE);
  assert.equal(
    pickAutoTip(offered, [], IDLE, () => 0),
    offered[0],
  );
  assert.equal(
    pickAutoTip(offered, [], IDLE, () => 0.999_999),
    offered.at(-1),
    "the top of the random range lands on the last candidate",
  );
  assert.equal(
    pickAutoTip(offered, [offered[0].id], IDLE, () => 0),
    offered[1],
    "a seen tip is never picked",
  );
  assert.equal(pickAutoTip(offered, ids(offered), IDLE), null);

  const picked = new Set();
  for (let roll = 0; roll < offered.length; roll += 1) {
    picked.add(pickAutoTip(offered, [], IDLE, () => roll / offered.length)?.id);
  }
  assert.equal(
    picked.size,
    offered.length,
    "every candidate can come up, not only the first in catalog order",
  );
});

test("an install that uses nothing yet is offered every unseen tip", () => {
  const offered = tipsFor(ALL_PLAN);
  assert.deepEqual(ids(autoTipCandidates(offered, [], IDLE)), ids(offered));
});

test("a tip for a feature already in use is skipped", () => {
  const offered = tipsFor(ALL_PLAN);
  const skipped = (usage) =>
    ids(offered).filter(
      (id) => !ids(autoTipCandidates(offered, [], usage)).includes(id),
    );
  const withProfile = (profile) => ({ ...IDLE, profiles: [profile] });

  assert.deepEqual(skipped(withProfile({ dns_blocklist: "pro" })), [
    "dnsBlocklist",
  ]);
  assert.deepEqual(
    skipped(withProfile({ dns_blocklist: null })),
    [],
    "a profile with DNS blocking off does not count",
  );
  assert.deepEqual(skipped(withProfile({ password_protected: true })), [
    "profilePassword",
  ]);
  assert.deepEqual(skipped(withProfile({ clear_on_close: true })), [
    "clearOnClose",
  ]);
  assert.deepEqual(skipped(withProfile({ group_id: "g1" })), ["groups"]);
  assert.deepEqual(skipped({ ...IDLE, groupCount: 2 }), ["groups"]);
  assert.deepEqual(skipped(withProfile({ extension_group_id: "e1" })), [
    "extensionGroups",
  ]);
  assert.deepEqual(skipped({ ...IDLE, extensionGroupCount: 1 }), [
    "extensionGroups",
  ]);
  assert.deepEqual(skipped(withProfile({ sync_mode: "Encrypted" })), [
    "selfHostedSync",
    "cloudBackup",
  ]);
  assert.deepEqual(skipped(withProfile({ sync_mode: "Disabled" })), []);
  assert.deepEqual(skipped({ ...IDLE, syncServerConfigured: true }), [
    "selfHostedSync",
  ]);
  assert.deepEqual(
    skipped(
      withProfile({ host_os: "macos", wayfern_config: { os: "windows" } }),
    ),
    ["crossOs"],
  );
  assert.deepEqual(
    skipped(withProfile({ wayfern_config: { os: "windows" } })),
    ["crossOs"],
    "without a recorded host, this desktop's OS is the native one",
  );
  assert.deepEqual(
    skipped(withProfile({ host_os: "macos", wayfern_config: { os: "macos" } })),
    [],
    "a native fingerprint is not a cross-OS one",
  );
  assert.deepEqual(skipped({ ...IDLE, proxyChecked: true }), ["proxyCheck"]);
  assert.deepEqual(skipped({ ...IDLE, fingerprintGateChanged: true }), [
    "fingerprintGate",
  ]);
  assert.deepEqual(skipped({ ...IDLE, isDefaultBrowser: true }), [
    "defaultBrowser",
  ]);
  assert.deepEqual(skipped({ ...IDLE, trashUsed: true }), ["trash"]);
  assert.deepEqual(skipped({ ...IDLE, apiEnabled: true }), [
    "localApi",
    "automation",
  ]);
  assert.deepEqual(skipped({ ...IDLE, mcpEnabled: true }), [
    "localApi",
    "automation",
  ]);
  assert.deepEqual(skipped({ ...IDLE, cookieBotEnrolled: true }), [
    "cookieBot",
  ]);
  assert.deepEqual(skipped({ ...IDLE, agentUsed: true }), ["agent"]);
  assert.deepEqual(skipped({ ...IDLE, inTeam: true }), ["team"]);
  assert.deepEqual(skipped({ ...IDLE, remoteControlEnabled: true }), [
    "remoteControl",
  ]);
});

test("only the command palette and import tips have no usage signal", () => {
  assert.deepEqual(ids(TIPS.filter((tip) => tip.inUse === undefined)), [
    "commandPalette",
    "importProfiles",
  ]);
});

test("the pick skips tips in use and gives up when nothing is left", () => {
  const offered = tipsFor(NONE);
  const unseen = ["dnsBlocklist", "trash"];
  const seen = ids(offered).filter((id) => !unseen.includes(id));
  const dnsOn = { ...IDLE, profiles: [{ dns_blocklist: "light" }] };
  for (const roll of [0, 0.5, 0.99]) {
    assert.equal(pickAutoTip(offered, seen, dnsOn, () => roll)?.id, "trash");
  }
  assert.equal(pickAutoTip(offered, seen, { ...dnsOn, trashUsed: true }), null);
});

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { isPlanTip, pickAutoTip, TIPS, tipsFor } from "./tips.ts";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const LOCALES = path.join(HERE, "..", "i18n", "locales");

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

test("the automatic flow picks the first unseen tip in catalog order", () => {
  const offered = tipsFor(NONE);
  assert.equal(pickAutoTip(offered, []), offered[0]);
  assert.equal(pickAutoTip(offered, [offered[0].id]), offered[1]);
  assert.equal(
    pickAutoTip(
      offered,
      offered.map((tip) => tip.id),
    ),
    null,
  );
});

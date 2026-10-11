import assert from "node:assert/strict";
import test from "node:test";
import {
  SOUND_CONTROL_SELECTOR,
  SOUND_GAP_MS,
  SOUNDS,
  soundForControl,
} from "./ui-sounds.ts";

/**
 * What is pinned here is "never annoying": every sound is short and quiet,
 * starts without a click, can not repeat faster than its gap, and only the
 * controls a person deliberately used make one.
 */

const length = (voices) =>
  Math.max(...voices.map((v) => v.delay + v.attack + v.decay));

test("every sound is short, quiet and starts softly", () => {
  for (const [name, voices] of Object.entries(SOUNDS)) {
    assert.ok(voices.length > 0, name);
    assert.ok(length(voices) <= 0.3, `${name} lasts ${length(voices)} s`);
    for (const voice of voices) {
      assert.ok(voice.peak > 0 && voice.peak <= 0.1, `${name} peak`);
      assert.ok(voice.attack >= 0.001, `${name} must not start with a click`);
    }
    assert.ok(SOUND_GAP_MS[name] > 0, `${name} needs a gap`);
  }
  assert.ok(length(SOUNDS.tap) <= 0.04, "a tap is a tick, not a tone");
  assert.ok(
    SOUND_GAP_MS.error >= 1000 && SOUND_GAP_MS.success >= 1000,
    "a run of results makes one sound, not a burst",
  );
});

const control = (overrides) => ({
  role: null,
  tag: "button",
  checked: null,
  disabled: false,
  silenced: false,
  ...overrides,
});

test("a button taps when pressed, and once only", () => {
  assert.equal(soundForControl(control({}), "press", false), "tap");
  assert.equal(
    soundForControl(control({}), "activate", false),
    null,
    "the click after a pointer press must not tap again",
  );
  assert.equal(soundForControl(control({}), "activate", true), "tap");
  assert.equal(
    soundForControl(control({ tag: "div", role: "option" }), "press", false),
    "tap",
  );
  assert.equal(soundForControl(control({ tag: "a" }), "press", false), "tap");
});

test("a toggle sounds its new state once it changed", () => {
  const on = control({ role: "switch", checked: "true" });
  const off = control({ role: "checkbox", checked: "false" });
  assert.equal(soundForControl(on, "press", false), null);
  assert.equal(soundForControl(on, "activate", false), "toggleOn");
  assert.equal(soundForControl(off, "activate", true), "toggleOff");
});

test("disabled, silenced and passive elements make no sound", () => {
  assert.equal(
    soundForControl(control({ disabled: true }), "press", false),
    null,
  );
  assert.equal(
    soundForControl(control({ silenced: true }), "press", false),
    null,
  );
  for (const tag of ["div", "input", "span", "textarea"]) {
    assert.equal(soundForControl(control({ tag }), "press", false), null, tag);
  }
  assert.ok(!SOUND_CONTROL_SELECTOR.includes("input"));
  assert.ok(SOUND_CONTROL_SELECTOR.includes('[role="switch"]'));
});

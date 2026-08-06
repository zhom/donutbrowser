import assert from "node:assert/strict";
import test from "node:test";
import {
  DEFAULT_DECORATION_LAYOUT,
  parseDecorationLayout,
} from "./window-decorations.ts";

/**
 * The app draws its own titlebar on Linux, so it owns the window controls —
 * and where they go is a desktop-wide user preference. This parser is the only
 * thing standing between that preference and the buttons we render, and only
 * GNOME can be exercised on the machine this was written on, so KDE's real
 * layout strings are pinned here instead.
 */

test("GNOME's default puts every control on the right", () => {
  assert.deepEqual(parseDecorationLayout(":minimize,maximize,close"), {
    left: [],
    right: ["minimize", "maximize", "close"],
  });
});

test("a left-hand layout is honored", () => {
  // GNOME users who prefer macOS ordering set exactly this.
  assert.deepEqual(parseDecorationLayout("close,minimize,maximize:"), {
    left: ["close", "minimize", "maximize"],
    right: [],
  });
});

test("controls can be split across both sides", () => {
  assert.deepEqual(parseDecorationLayout("close:minimize,maximize"), {
    left: ["close"],
    right: ["minimize", "maximize"],
  });
});

test("GTK's own default drops the appmenu it cannot draw", () => {
  assert.deepEqual(parseDecorationLayout("appmenu:close"), {
    left: [],
    right: ["close"],
  });
});

test("non-button tokens are ignored rather than rendered", () => {
  // `icon`, `menu`, `appmenu` and `spacer` are all legal GTK tokens for things
  // this titlebar does not draw.
  assert.deepEqual(parseDecorationLayout("icon,menu:spacer,close"), {
    left: [],
    right: ["close"],
  });
});

test("KDE's extra decoration buttons are ignored", () => {
  // KWin offers buttons GTK has no concept of. kde-gtk-config maps what it can
  // and may pass these through; rendering an unknown box would be worse than
  // dropping it, which is what GTK itself does.
  assert.deepEqual(
    parseDecorationLayout(
      "menu,applicationmenu:shade,keepabove,keepbelow,help,minimize,maximize,close",
    ),
    { left: [], right: ["minimize", "maximize", "close"] },
  );
});

test("a duplicated control is rendered once", () => {
  assert.deepEqual(parseDecorationLayout("close:close,minimize"), {
    left: ["close"],
    right: ["minimize"],
  });
});

test("whitespace and capitalization are tolerated", () => {
  assert.deepEqual(parseDecorationLayout(" : Minimize , CLOSE "), {
    left: [],
    right: ["minimize", "close"],
  });
});

test("a string with no colon is entirely the left side, as GTK reads it", () => {
  // `g_strsplit(layout, ":", 2)` leaves the right-hand token NULL, so GTK puts
  // every button on the left. No mainstream desktop emits this, but matching
  // GTK is the only defensible reading.
  assert.deepEqual(parseDecorationLayout("minimize,close"), {
    left: ["minimize", "close"],
    right: [],
  });
});

test("only the first colon splits the sides", () => {
  // GTK's split has a limit of 2, so the second colon is not a separator: the
  // right side becomes the single token "minimize:maximize", which matches no
  // button name and is dropped — exactly as GTK drops it.
  assert.deepEqual(parseDecorationLayout("close:minimize:maximize"), {
    left: ["close"],
    right: [],
  });
});

test("missing, empty and unusable layouts fall back to the default", () => {
  const fallback = { left: [], right: ["minimize", "maximize", "close"] };
  for (const input of [null, undefined, "", "   "]) {
    assert.deepEqual(parseDecorationLayout(input), fallback, `input: ${input}`);
  }
  // A layout naming only buttons we cannot draw would otherwise leave the user
  // with no way to close the window.
  assert.deepEqual(parseDecorationLayout("appmenu:spacer"), fallback);
  assert.deepEqual(parseDecorationLayout(":"), fallback);
});

test("the documented default parses to the default", () => {
  assert.deepEqual(parseDecorationLayout(DEFAULT_DECORATION_LAYOUT), {
    left: [],
    right: ["minimize", "maximize", "close"],
  });
});

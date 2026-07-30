import assert from "node:assert/strict";
import test from "node:test";
import Color from "color";
import {
  getDerivedThemeColors,
  getThemeByColors,
  getThemeMode,
  normalizeThemeColors,
  THEMES,
} from "./themes.ts";

const AA_TEXT = 4.5;
const UI_BOUNDARY = 3;
const SEMANTIC_PAIRS = [
  ["background", "foreground"],
  ["card", "card-foreground"],
  ["popover", "popover-foreground"],
  ["primary", "primary-foreground"],
  ["secondary", "secondary-foreground"],
  ["muted", "muted-foreground"],
  ["accent", "accent-foreground"],
  ["destructive", "destructive-foreground"],
  ["success", "success-foreground"],
  ["warning", "warning-foreground"],
];
const CONTENT_SURFACES = ["background", "card", "popover", "muted"];
const CONTEXTUAL_ROLES = ["primary", "destructive", "success", "warning"];
const LIGHT_THEMES = new Set([
  "ayu-light",
  "catppuccin-latte",
  "github-light",
  "gruvbox-light",
  "rose-pine-dawn",
  "solarized-light",
]);

function contrast(foreground, background) {
  return Color(foreground).contrast(Color(background));
}

function tintedSurface(surface, fill, opacity = 0.1) {
  return Color(surface).mix(Color(fill), opacity);
}

function assertNoContrastFailures(failures) {
  assert.deepEqual(failures, [], failures.join("\n"));
}

test("every theme declares the intended appearance mode", () => {
  assert.equal(THEMES.length, 20);
  assert.equal(new Set(THEMES.map((theme) => theme.id)).size, THEMES.length);
  assert.deepEqual(
    THEMES.filter((theme) => theme.mode === "light")
      .map((theme) => theme.id)
      .sort(),
    [...LIGHT_THEMES].sort(),
  );
  for (const theme of THEMES) {
    assert.equal(theme.mode, LIGHT_THEMES.has(theme.id) ? "light" : "dark");
    assert.equal(getThemeMode(theme.colors), theme.mode);
  }
});

test("every semantic fill and foreground pair meets WCAG AA", () => {
  const failures = [];
  for (const theme of THEMES) {
    for (const [surface, content] of SEMANTIC_PAIRS) {
      const ratio = contrast(
        theme.colors[`--${content}`],
        theme.colors[`--${surface}`],
      );
      if (ratio < AA_TEXT) {
        failures.push(
          `${theme.id}: ${surface}/${content} ${ratio.toFixed(2)}:1`,
        );
      }
    }
  }
  assertNoContrastFailures(failures);
});

test("foreground and muted text remain readable on every content surface", () => {
  const failures = [];
  for (const theme of THEMES) {
    for (const content of ["foreground", "muted-foreground"]) {
      for (const surface of CONTENT_SURFACES) {
        const ratio = contrast(
          theme.colors[`--${content}`],
          theme.colors[`--${surface}`],
        );
        if (ratio < AA_TEXT) {
          failures.push(
            `${theme.id}: ${content}/${surface} ${ratio.toFixed(2)}:1`,
          );
        }
      }
    }
  }
  assertNoContrastFailures(failures);
});

test("derived contextual text and control boundaries remain accessible", () => {
  const failures = [];
  for (const theme of THEMES) {
    const derived = getDerivedThemeColors(theme.colors);
    for (const role of CONTEXTUAL_ROLES) {
      for (const surface of ["background", "card", "popover", "muted"]) {
        const surfaceColor = theme.colors[`--${surface}`];
        const contexts = [[surface, surfaceColor]];
        if (surface !== "muted") {
          contexts.push(
            ...[0.1, 0.2].map((opacity) => [
              `${surface} with ${role} ${opacity * 100}% tint`,
              tintedSurface(surfaceColor, theme.colors[`--${role}`], opacity),
            ]),
          );
        }
        for (const [context, background] of contexts) {
          const ratio = contrast(derived[`--${role}-text`], background);
          if (ratio < AA_TEXT) {
            failures.push(
              `${theme.id}: ${role}-text/${context} ${ratio.toFixed(2)}:1`,
            );
          }
        }
      }
    }
    for (const boundary of ["--input", "--ring"]) {
      for (const surface of ["background", "card", "popover", "muted"]) {
        const ratio = contrast(derived[boundary], theme.colors[`--${surface}`]);
        if (ratio < UI_BOUNDARY) {
          failures.push(
            `${theme.id}: ${boundary}/${surface} ${ratio.toFixed(2)}:1`,
          );
        }
      }
    }
  }
  assertNoContrastFailures(failures);
});

test("unmatched custom palettes derive their mode from the background", () => {
  const darkCustom = { ...THEMES[0].colors, "--background": "#20242b" };
  const lightCustom = { ...THEMES[0].colors, "--background": "#f4f1e8" };
  assert.equal(getThemeMode(darkCustom), "dark");
  assert.equal(getThemeMode(lightCustom), "light");
});

test("saved copies of the previous presets migrate without claiming edits", () => {
  const houston = THEMES.find((theme) => theme.id === "houston");
  const legacyHouston = {
    ...houston.colors,
    "--secondary-foreground": "#f7f7f8",
    "--accent-foreground": "#f7f7f8",
    "--destructive-foreground": "#f7f7f8",
  };
  assert.equal(getThemeByColors(legacyHouston)?.id, "houston");
  assert.deepEqual(normalizeThemeColors(legacyHouston), houston.colors);

  const editedHouston = { ...legacyHouston, "--accent": "#123456" };
  assert.equal(getThemeByColors(editedHouston), undefined);
  assert.equal(normalizeThemeColors(editedHouston), editedHouston);
});

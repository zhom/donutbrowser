/**
 * Single source of truth for keyboard shortcuts. Each entry declares both how
 * to MATCH a real keyboard event (lowercase `key` + modifiers) and how to
 * DISPLAY it to the user. The display side branches on platform so macOS sees
 * the ⌘ glyph while everyone else sees `Ctrl`.
 */

export type ShortcutGroup =
  | "navigation"
  | "actions"
  | "view"
  | "profiles"
  | "groups";

export interface ShortcutDef {
  /** Stable identifier — used by the global listener to dispatch to handlers. */
  id: ShortcutId;
  /** Translation key for the displayed label in the shortcuts page / palette. */
  labelKey: string;
  group: ShortcutGroup;
  /** Lowercased `KeyboardEvent.key`, e.g. "k", ",", "/". */
  key: string;
  /** Require the primary modifier (Cmd on mac, Ctrl elsewhere). */
  mod?: boolean;
  shift?: boolean;
  alt?: boolean;
}

export type ShortcutId =
  | "openPalette"
  | "openShortcuts"
  | "importProfile"
  | "goProfiles"
  | "goProxies"
  | "goExtensions"
  | "goGroups"
  | "goIntegrations"
  | "goAccount"
  | "goSettings";

export const SHORTCUTS: ShortcutDef[] = [
  // Actions
  {
    id: "openPalette",
    labelKey: "shortcuts.openPalette",
    group: "actions",
    key: "k",
    mod: true,
  },
  {
    id: "openShortcuts",
    labelKey: "shortcuts.openShortcuts",
    group: "actions",
    key: "/",
    mod: true,
  },
  {
    id: "importProfile",
    labelKey: "shortcuts.importProfile",
    group: "actions",
    key: "o",
    mod: true,
  },
  // Navigation
  {
    id: "goProfiles",
    labelKey: "shortcuts.goProfiles",
    group: "navigation",
    key: "p",
    mod: true,
  },
  {
    id: "goProxies",
    labelKey: "shortcuts.goProxies",
    group: "navigation",
    key: "n",
    mod: true,
  },
  {
    id: "goExtensions",
    labelKey: "shortcuts.goExtensions",
    group: "navigation",
    key: "e",
    mod: true,
  },
  {
    id: "goGroups",
    labelKey: "shortcuts.goGroups",
    group: "navigation",
    key: "g",
    mod: true,
  },
  {
    id: "goIntegrations",
    labelKey: "shortcuts.goIntegrations",
    group: "navigation",
    key: "i",
    mod: true,
  },
  {
    id: "goAccount",
    labelKey: "shortcuts.goAccount",
    group: "navigation",
    key: "a",
    mod: true,
  },
  {
    id: "goSettings",
    labelKey: "shortcuts.goSettings",
    group: "navigation",
    key: ",",
    mod: true,
  },
];

/**
 * Match Mod+1..9 to the group at that index (1-based). Returns the digit
 * pressed, or null. Used by the global keydown handler before falling back to
 * the static SHORTCUTS table.
 */
export function matchesGroupDigit(e: KeyboardEvent): number | null {
  if (e.key < "1" || e.key > "9") return null;
  const mod = isMac() ? e.metaKey : e.ctrlKey;
  const oppositeMod = isMac() ? e.ctrlKey : e.metaKey;
  if (!mod || oppositeMod || e.shiftKey || e.altKey) return null;
  return Number(e.key);
}

/**
 * Build display tokens for a Mod+digit group shortcut. Mirrors `formatShortcut`.
 */
export function formatGroupShortcut(digit: number): string[] {
  const mac = isMac();
  return [mac ? "⌘" : "Ctrl", String(digit)];
}

export function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  // userAgentData is preferred but not in all browsers; fall back to platform.
  // `navigator.platform` is deprecated but still works in Tauri's webview.
  const ua = navigator.userAgent || "";
  const platform =
    (navigator as unknown as { userAgentData?: { platform?: string } })
      .userAgentData?.platform ??
    navigator.platform ??
    "";
  return /Mac|iPhone|iPad|iPod/.test(platform) || /Mac OS X/.test(ua);
}

/**
 * Render a shortcut as the platform-correct token list. The shortcuts page and
 * the command palette both consume this so the glyphs stay in sync.
 *
 * On macOS: ["⌘", "⇧", "⌥", "K"]
 * Elsewhere: ["Ctrl", "Shift", "Alt", "K"]
 */
export function formatShortcut(s: ShortcutDef): string[] {
  const mac = isMac();
  const tokens: string[] = [];
  if (s.mod) tokens.push(mac ? "⌘" : "Ctrl");
  if (s.shift) tokens.push(mac ? "⇧" : "Shift");
  if (s.alt) tokens.push(mac ? "⌥" : "Alt");
  tokens.push(prettyKey(s.key));
  return tokens;
}

function prettyKey(key: string): string {
  if (key.length === 1) return key.toUpperCase();
  // Named keys like "Enter", "Escape", etc. would already be capitalized.
  return key;
}

/**
 * Match a real `KeyboardEvent` against a shortcut definition. Returns true
 * only when modifiers are an exact match (so Ctrl+Shift+K doesn't fire
 * Ctrl+K).
 */
export function matchesShortcut(s: ShortcutDef, e: KeyboardEvent): boolean {
  if (e.key.toLowerCase() !== s.key.toLowerCase()) return false;
  const mod = isMac() ? e.metaKey : e.ctrlKey;
  const oppositeMod = isMac() ? e.ctrlKey : e.metaKey;
  if (Boolean(s.mod) !== mod) return false;
  // Reject the wrong-platform modifier so Ctrl+K on macOS doesn't accidentally
  // trigger something that only expects ⌘+K.
  if (oppositeMod) return false;
  if (Boolean(s.shift) !== e.shiftKey) return false;
  if (Boolean(s.alt) !== e.altKey) return false;
  return true;
}

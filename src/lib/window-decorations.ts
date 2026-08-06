/**
 * Parsing for the desktop's titlebar button layout on Linux.
 *
 * The backend reads `GtkSettings::gtk-decoration-layout`, which GNOME populates
 * from `org.gnome.desktop.wm.preferences button-layout` and KDE populates via
 * `kde-gtk-config` from KWin's decoration settings. The grammar is the one GTK
 * itself parses: a comma-separated list of button names for the left side, a
 * `:`, then the list for the right side. Either side may be empty.
 *
 *   ":minimize,maximize,close"   GNOME default — everything on the right
 *   "close,minimize,maximize:"   macOS-like — everything on the left
 *   "appmenu:close"              upstream GTK default
 */

/** What the backend reports about this window's decorations. */
export interface WindowDecorationsInfo {
  /** True when the app owns the titlebar and must draw controls and edges. */
  client_side: boolean;
  /** The desktop's button layout; only meaningful when `client_side`. */
  layout: string | null;
}

/** Controls this app can actually draw. */
export type WindowControl = "minimize" | "maximize" | "close";

export interface DecorationLayout {
  left: WindowControl[];
  right: WindowControl[];
}

/** What an unconfigured GNOME or KDE session shows. */
export const DEFAULT_DECORATION_LAYOUT = ":minimize,maximize,close";

const CONTROLS: WindowControl[] = ["minimize", "maximize", "close"];

function parseSide(side: string, seen: Set<WindowControl>): WindowControl[] {
  const out: WindowControl[] = [];
  for (const raw of side.split(",")) {
    const name = raw.trim().toLowerCase();
    // Everything else a desktop can put here is deliberately dropped rather
    // than rendered as an unknown box: `icon`, `menu`, `appmenu` and `spacer`
    // from GTK, plus KDE's extras (`shade`, `above`/`keepabove`,
    // `below`/`keepbelow`, `help`, `applicationmenu`, `ontop`). Silently
    // ignoring an unrecognized token is also what GTK does.
    if (!CONTROLS.includes(name as WindowControl)) {
      continue;
    }
    const control = name as WindowControl;
    // A desktop could list the same button on both sides; the shared `seen` set
    // means the first occurrence wins and it is never drawn twice.
    if (seen.has(control)) {
      continue;
    }
    seen.add(control);
    out.push(control);
  }
  return out;
}

/**
 * Split a layout string into the buttons to draw on each side.
 *
 * Falls back to the GNOME/KDE default whenever the string is missing, empty, or
 * names no button this app can draw — a titlebar with no way to close the
 * window would be far worse than one that ignores an exotic preference.
 */
export function parseDecorationLayout(
  layout: string | null | undefined,
): DecorationLayout {
  const source = layout?.trim() ? layout : DEFAULT_DECORATION_LAYOUT;
  // GTK splits on the FIRST colon only, and a string with no colon at all is
  // entirely the left side (`g_strsplit(layout, ":", 2)` leaves the right
  // token NULL). Matching that exactly beats inventing a friendlier rule.
  const split = source.indexOf(":");
  const leftRaw = split === -1 ? source : source.slice(0, split);
  const rightRaw = split === -1 ? "" : source.slice(split + 1);

  const seen = new Set<WindowControl>();
  const left = parseSide(leftRaw, seen);
  const right = parseSide(rightRaw, seen);

  if (left.length === 0 && right.length === 0) {
    return { left: [], right: [...CONTROLS] };
  }
  return { left, right };
}

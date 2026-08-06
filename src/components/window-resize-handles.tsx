"use client";

import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Mirrors the API's own `ResizeDirection`, which it declares but does not
 * export. Structurally identical, so a drift would fail the call below.
 */
type ResizeDirection =
  | "East"
  | "North"
  | "NorthEast"
  | "NorthWest"
  | "South"
  | "SouthEast"
  | "SouthWest"
  | "West";

/**
 * Mouse resize areas for a window with no server-side decorations.
 *
 * `gtk_window_set_decorated(false)` removes GTK's own invisible resize border
 * along with the frame, so without these the window can only be resized through
 * window-manager shortcuts (Super+right-drag and friends). Each handle hands the
 * pointer to the compositor via `begin_resize_drag`, which is the same call
 * GTK's client-side decorations make, so edge snapping and the resize cursor
 * come from the WM exactly as they do for a native window.
 *
 * Rendered only where the app owns the frame; on macOS the native titlebar is
 * still in place and the system draws its own resize edges.
 */

/**
 * GTK's own grab area is far wider, but all of it sits *outside* the window in
 * the shadow margin. Ours is inside, so every pixel is taken from real content.
 */
/** Top edge only — it overlaps the 44px-tall window controls. */
const TOP_EDGE = "6px";
/** Sides and bottom overlap nothing, so they can be comfortably grabbable. */
const EDGE = "8px";
/** Corners need to win over the edges that overlap them. */
const CORNER = "16px";

interface Handle {
  direction: ResizeDirection;
  style: React.CSSProperties;
  cursor: string;
}

const HANDLES: Handle[] = [
  // Edges.
  {
    direction: "North",
    cursor: "ns-resize",
    style: { top: 0, left: CORNER, right: CORNER, height: TOP_EDGE },
  },
  {
    direction: "South",
    cursor: "ns-resize",
    style: { bottom: 0, left: CORNER, right: CORNER, height: EDGE },
  },
  {
    direction: "West",
    cursor: "ew-resize",
    style: { left: 0, top: CORNER, bottom: CORNER, width: EDGE },
  },
  {
    direction: "East",
    cursor: "ew-resize",
    style: { right: 0, top: CORNER, bottom: CORNER, width: EDGE },
  },
  // Corners, drawn after the edges so they sit on top of the overlap.
  {
    direction: "NorthWest",
    cursor: "nwse-resize",
    style: { top: 0, left: 0, width: CORNER, height: TOP_EDGE },
  },
  {
    direction: "NorthEast",
    cursor: "nesw-resize",
    style: { top: 0, right: 0, width: CORNER, height: TOP_EDGE },
  },
  {
    direction: "SouthWest",
    cursor: "nesw-resize",
    style: { bottom: 0, left: 0, width: CORNER, height: CORNER },
  },
  {
    direction: "SouthEast",
    cursor: "nwse-resize",
    style: { bottom: 0, right: 0, width: CORNER, height: CORNER },
  },
];

export function WindowResizeHandles({ isMaximized }: { isMaximized: boolean }) {
  // A maximized window has no resizable edge, and leaving the strips live would
  // put invisible hit areas over real content. Tauri's own built-in undecorated
  // resizing disables itself while maximized for the same reason.
  if (isMaximized) {
    return null;
  }

  const startResize =
    (direction: ResizeDirection) => (e: React.PointerEvent) => {
      // Left button only: right-click belongs to the WM/window menu, and a
      // middle-click drag should not resize.
      if (e.button !== 0) {
        return;
      }
      e.preventDefault();
      e.stopPropagation();
      void getCurrentWindow()
        .startResizeDragging(direction)
        .catch((error: unknown) => {
          console.error("Failed to start window resize:", error);
        });
    };

  return (
    <>
      {HANDLES.map((handle) => (
        <div
          key={handle.direction}
          // Deliberately BELOW the window controls (z-100000) rather than
          // above: a real CSD window keeps its resize border outside the
          // buttons, but ours is inside the window, so layering it on top
          // would steal the corner of whichever control sits in that corner.
          // Edges still win over ordinary content, which is all they need.
          className="fixed z-[99998] pointer-events-auto"
          style={{ ...handle.style, cursor: handle.cursor }}
          onPointerDown={startResize(handle.direction)}
          aria-hidden="true"
        />
      ))}
    </>
  );
}

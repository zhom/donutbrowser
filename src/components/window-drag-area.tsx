"use client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useWindowDecorations } from "@/hooks/use-window-decorations";
import { getCurrentOS, type OperatingSystem } from "@/lib/platform";
import type { WindowControl } from "@/lib/window-decorations";
import { WindowResizeHandles } from "./window-resize-handles";

export function WindowDragArea() {
  const { t } = useTranslation();
  const [platform, setPlatform] = useState<OperatingSystem | null>(null);
  const [isMaximized, setIsMaximized] = useState(false);
  const decorations = useWindowDecorations();

  useEffect(() => {
    setPlatform(getCurrentOS());
  }, []);

  useEffect(() => {
    if (platform !== "windows" && platform !== "linux") return;
    const win = getCurrentWindow();
    let cancelled = false;
    const sync = async () => {
      try {
        const maximized = await win.isMaximized();
        if (!cancelled) setIsMaximized(maximized);
      } catch (error) {
        console.error("Failed to read window maximized state:", error);
      }
    };
    void sync();
    const unlistenPromise = win.onResized(() => {
      void sync();
    });
    return () => {
      cancelled = true;
      void unlistenPromise.then((unlisten) => {
        unlisten();
      });
    };
  }, [platform]);

  const handleMinimize = async () => {
    try {
      await getCurrentWindow().minimize();
    } catch (error) {
      console.error("Failed to minimize window:", error);
    }
  };

  const handleToggleMaximize = async () => {
    try {
      await getCurrentWindow().toggleMaximize();
    } catch (error) {
      console.error("Failed to toggle window maximize:", error);
    }
  };

  const handleClose = async () => {
    try {
      await getCurrentWindow().close();
    } catch (error) {
      console.error("Failed to close window:", error);
    }
  };

  const renderControl = (control: WindowControl) => {
    switch (control) {
      case "minimize":
        return (
          <button
            key="minimize"
            type="button"
            onClick={() => {
              void handleMinimize();
            }}
            className="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
            aria-label={t("common.window.minimize")}
          >
            <svg
              width="10"
              height="1"
              viewBox="0 0 10 1"
              fill="currentColor"
              role="img"
              aria-label={t("common.window.minimize")}
            >
              <rect width="10" height="1" />
            </svg>
          </button>
        );
      case "maximize":
        return (
          <button
            key="maximize"
            type="button"
            onClick={() => {
              void handleToggleMaximize();
            }}
            className="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
            aria-label={
              isMaximized
                ? t("common.window.restore")
                : t("common.window.maximize")
            }
          >
            {isMaximized ? (
              <svg
                width="10"
                height="10"
                viewBox="0 0 10 10"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.2"
                role="img"
                aria-label={t("common.window.restore")}
              >
                <rect x="1" y="3" width="6" height="6" />
                <path d="M3 3 V1 H9 V7 H7" />
              </svg>
            ) : (
              <svg
                width="10"
                height="10"
                viewBox="0 0 10 10"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.2"
                role="img"
                aria-label={t("common.window.maximize")}
              >
                <rect x="1" y="1" width="8" height="8" />
              </svg>
            )}
          </button>
        );
      case "close":
        return (
          <button
            key="close"
            type="button"
            onClick={() => {
              void handleClose();
            }}
            className="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-destructive hover:text-destructive-foreground"
            aria-label={t("common.window.close")}
          >
            <svg
              width="10"
              height="10"
              viewBox="0 0 10 10"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.2"
              role="img"
              aria-label={t("common.window.close")}
            >
              <line x1="1" y1="1" x2="9" y2="9" />
              <line x1="9" y1="1" x2="1" y2="9" />
            </svg>
          </button>
        );
    }
  };

  if (!platform || platform === "unknown") {
    return null;
  }

  // macOS: nothing to render here. The transparent native titlebar (set via
  // `set_transparent_titlebar(true)` in src-tauri/src/lib.rs) lets the OS
  // handle dragging directly, and the sys-bar inside `home-header.tsx`
  // declares its own `data-tauri-drag-region` overlay for the WebView area.
  // The previous full-width fixed z-[999999] button was stealing every
  // click in the top 40px of the window.
  if (platform === "macos") {
    return null;
  }

  // Linux: the window has no server-side decorations, so the app owns both the
  // controls and the resize edges. Which buttons appear and on which side is a
  // desktop-wide preference (GNOME's `button-layout`, KWin's decoration
  // settings), read through GTK so both environments are honored.
  if (platform === "linux") {
    // Not resolved yet, or the session keeps server-side decorations (KDE on
    // Wayland — see `use_client_side_decorations` in the backend). Either way
    // there is a real titlebar and nothing for the app to draw.
    if (!decorations.resolved || !decorations.clientSide) {
      return null;
    }
    const { layout } = decorations;
    return (
      <>
        {/* Dropping decorations also drops the compositor's drop shadow, and
            neither Tauri nor tao exposes a Linux shadow API. Without some edge
            the window is invisible against a similarly coloured desktop, so
            draw a hairline. Not rounded: that needs a transparent window, which
            would conflict with the WebView and the resize strips. */}
        {!isMaximized && (
          <div
            aria-hidden="true"
            className="pointer-events-none fixed inset-0 z-[99999] border border-border"
          />
        )}
        <WindowResizeHandles isMaximized={isMaximized} />
        {layout.left.length > 0 && (
          <div className="fixed top-0 left-0 z-[100000] flex h-11 items-center select-none pointer-events-auto">
            {layout.left.map(renderControl)}
          </div>
        )}
        {layout.right.length > 0 && (
          <div className="fixed top-0 right-0 z-[100000] flex h-11 items-center select-none pointer-events-auto">
            {layout.right.map(renderControl)}
          </div>
        )}
      </>
    );
  }

  // Windows: minimize/maximize/close controls anchored at the top-right
  // corner of the sys-bar. The HomeHeader's own drag-region overlay handles
  // window dragging via Tauri 2, so we don't need a separate draggable spacer
  // covering the whole width.
  return (
    <div
      className="fixed top-0 right-0 z-50 flex h-11 items-center select-none"
      aria-hidden="false"
    >
      {(["minimize", "maximize", "close"] as WindowControl[]).map(
        renderControl,
      )}
    </div>
  );
}

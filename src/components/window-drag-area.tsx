"use client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

type Platform = "macos" | "windows" | "linux";

function detectPlatform(): Platform {
  const userAgent = navigator.userAgent.toLowerCase();
  if (userAgent.includes("mac")) return "macos";
  if (userAgent.includes("win")) return "windows";
  return "linux";
}

export function WindowDragArea() {
  const { t } = useTranslation();
  const [platform, setPlatform] = useState<Platform | null>(null);

  useEffect(() => {
    setPlatform(detectPlatform());
  }, []);

  const handlePointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();

    const startDrag = async () => {
      try {
        const window = getCurrentWindow();
        await window.startDragging();
      } catch (error) {
        console.error("Failed to start window dragging:", error);
      }
    };

    void startDrag();
  };

  // Linux: system decorations handle everything
  if (!platform || platform === "linux") {
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

  // Windows: minimize/close controls anchored at the top-right corner of
  // the sys-bar. The HomeHeader's own drag-region overlay handles window
  // dragging via Tauri 2, so we don't need a separate draggable spacer
  // covering the whole width.
  const handleMinimize = async () => {
    try {
      await getCurrentWindow().minimize();
    } catch (error) {
      console.error("Failed to minimize window:", error);
    }
  };

  const handleClose = async () => {
    try {
      await getCurrentWindow().close();
    } catch (error) {
      console.error("Failed to close window:", error);
    }
  };
  void handlePointerDown; // kept for backwards-compat; not used on Windows now

  return (
    <div
      className="fixed top-0 right-0 z-50 flex items-center h-11 select-none"
      aria-hidden="false"
    >
      <button
        type="button"
        onClick={() => {
          void handleMinimize();
        }}
        className="flex items-center justify-center w-11 h-full hover:bg-muted/50 transition-colors text-muted-foreground hover:text-foreground"
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
      <button
        type="button"
        onClick={() => {
          void handleClose();
        }}
        className="flex items-center justify-center w-11 h-full hover:bg-destructive/90 transition-colors text-muted-foreground hover:text-destructive-foreground"
        aria-label={t("common.buttons.close")}
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.2"
          role="img"
          aria-label={t("common.buttons.close")}
        >
          <line x1="1" y1="1" x2="9" y2="9" />
          <line x1="9" y1="1" x2="1" y2="9" />
        </svg>
      </button>
    </div>
  );
}

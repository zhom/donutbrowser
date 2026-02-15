"use client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";

type Platform = "macos" | "windows" | "linux";

function detectPlatform(): Platform {
  const userAgent = navigator.userAgent.toLowerCase();
  if (userAgent.includes("mac")) return "macos";
  if (userAgent.includes("win")) return "windows";
  return "linux";
}

export function WindowDragArea() {
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

  // macOS: transparent drag area overlay
  if (platform === "macos") {
    return (
      <button
        type="button"
        className="fixed top-0 right-0 left-0 h-10 bg-transparent border-0 z-[999999] select-none"
        data-window-drag-area="true"
        onPointerDown={handlePointerDown}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
        }}
      />
    );
  }

  // Windows: custom title bar with drag area + minimize/close buttons
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

  return (
    <div
      className="fixed top-0 right-0 left-0 h-10 z-[999999] flex items-center select-none"
      data-window-drag-area="true"
    >
      {/* Draggable area */}
      <button
        type="button"
        className="flex-1 h-full bg-transparent border-0 cursor-default"
        onPointerDown={handlePointerDown}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
        }}
      />
      {/* Window control buttons */}
      <div className="flex items-center h-full">
        <button
          type="button"
          onClick={handleMinimize}
          className="flex items-center justify-center w-12 h-full hover:bg-muted/50 transition-colors text-muted-foreground hover:text-foreground"
        >
          <svg
            width="10"
            height="1"
            viewBox="0 0 10 1"
            fill="currentColor"
            role="img"
            aria-label="Minimize"
          >
            <rect width="10" height="1" />
          </svg>
        </button>
        <button
          type="button"
          onClick={handleClose}
          className="flex items-center justify-center w-12 h-full hover:bg-destructive/90 transition-colors text-muted-foreground hover:text-destructive-foreground"
        >
          <svg
            width="10"
            height="10"
            viewBox="0 0 10 10"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.2"
            role="img"
            aria-label="Close"
          >
            <line x1="1" y1="1" x2="9" y2="9" />
            <line x1="9" y1="1" x2="1" y2="9" />
          </svg>
        </button>
      </div>
    </div>
  );
}

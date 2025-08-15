"use client";

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";

export function WindowDragArea() {
  const [isMacOS, setIsMacOS] = useState(false);

  useEffect(() => {
    // Check if we're on macOS using user agent detection
    const checkPlatform = () => {
      const userAgent = navigator.userAgent.toLowerCase();
      setIsMacOS(userAgent.includes("mac"));
    };

    checkPlatform();
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

  // Only render on macOS and when no dialogs are open
  if (!isMacOS) {
    return null;
  }

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

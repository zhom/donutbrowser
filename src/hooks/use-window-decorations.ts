"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import {
  type DecorationLayout,
  parseDecorationLayout,
  type WindowDecorationsInfo,
} from "@/lib/window-decorations";

export interface WindowDecorationsState {
  /** True when the app owns the titlebar and must draw controls and edges. */
  clientSide: boolean;
  /** Which controls go on which side. Meaningless unless `clientSide`. */
  layout: DecorationLayout;
  /** False until the backend has answered. */
  resolved: boolean;
}

const PENDING: WindowDecorationsState = {
  clientSide: false,
  layout: { left: [], right: [] },
  resolved: false,
};

/**
 * The window's decoration state, shared by everything that has to agree about
 * it.
 *
 * The controls and the header padding that clears them are rendered by two
 * different components. Fetching this separately in each let them disagree for
 * a frame — or indefinitely, if one missed the change event — and the visible
 * result is window controls sitting on top of the search box. One subscription
 * per consumer, one shared derivation.
 */
export function useWindowDecorations(): WindowDecorationsState {
  const [state, setState] = useState<WindowDecorationsState>(PENDING);

  useEffect(() => {
    let cancelled = false;
    const read = async () => {
      try {
        const value = await invoke<WindowDecorationsInfo>(
          "get_window_decoration_layout",
        );
        if (cancelled) return;
        setState({
          clientSide: value.client_side,
          layout: parseDecorationLayout(value.layout),
          resolved: true,
        });
      } catch (error) {
        console.error("Failed to read window decoration layout:", error);
        // Assume the platform still draws the titlebar: drawing a second one
        // over a real one is worse than drawing none.
        if (!cancelled) {
          setState({ ...PENDING, resolved: true });
        }
      }
    };
    void read();
    // The user can rearrange titlebar buttons while the app is running.
    const unlisten = listen("window-decoration-layout-changed", () => {
      void read();
    });
    return () => {
      cancelled = true;
      void unlisten.then((fn) => {
        fn();
      });
    };
  }, []);

  return state;
}

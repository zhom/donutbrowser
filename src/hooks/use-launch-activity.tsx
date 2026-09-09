"use client";

import { listen } from "@tauri-apps/api/event";
import { createContext, useContext, useEffect, useState } from "react";

export type LaunchStage =
  | "queued"
  | "preparing"
  | "network"
  | "extensions"
  | "starting"
  | "running"
  | "failed";
export interface LaunchEvent {
  id: string;
  stage: LaunchStage;
  timestamp: number;
  error?: string | null;
}
const LaunchActivity = createContext<Record<string, LaunchEvent[]>>({});

export function LaunchActivityProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [activity, setActivity] = useState<Record<string, LaunchEvent[]>>({});
  useEffect(() => {
    let active = true;
    const subscription = listen<LaunchEvent>(
      "profile-launch-stage",
      ({ payload }) => {
        if (!active) return;
        setActivity((previous) => {
          const next = { ...previous };
          next[payload.id] =
            payload.stage === "queued"
              ? [payload]
              : [...(previous[payload.id] ?? []), payload].slice(-16);
          // This is a recent-operation receipt, not an unbounded log store.
          const ids = Object.keys(next);
          if (ids.length > 100) {
            ids.sort(
              (a, b) =>
                (next[a].at(-1)?.timestamp ?? 0) -
                (next[b].at(-1)?.timestamp ?? 0),
            );
            delete next[ids[0]];
          }
          return next;
        });
      },
    );
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, []);
  return (
    <LaunchActivity.Provider value={activity}>
      {children}
    </LaunchActivity.Provider>
  );
}

export function useLaunchActivity(profileId: string) {
  return useContext(LaunchActivity)[profileId] ?? [];
}

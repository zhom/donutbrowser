"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { BrowserProfile } from "@/types";

export function useProfileReferences(enabled: boolean) {
  const [profiles, setProfiles] = useState<BrowserProfile[] | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let revision = 0;
    const load = async () => {
      const request = ++revision;
      try {
        const result = await invoke<BrowserProfile[]>("list_browser_profiles");
        if (active && request === revision) {
          setProfiles(result);
          setFailed(false);
        }
      } catch {
        if (active && request === revision) {
          setProfiles(null);
          setFailed(true);
        }
      }
    };
    const subscription = listen("profiles-changed", () => void load());
    void load();
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, [enabled]);
  return { profiles, failed };
}

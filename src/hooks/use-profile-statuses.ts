import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { ProfileStatus } from "@/types";

/** Every profile status, in order, kept current by `profile-statuses-changed`. */
export function useProfileStatuses(): ProfileStatus[] {
  const [statuses, setStatuses] = useState<ProfileStatus[]>([]);

  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    const load = async () => {
      try {
        const next = await invoke<ProfileStatus[]>("list_profile_statuses");
        if (mounted) setStatuses(next);
      } catch (error) {
        console.error("Failed to load profile statuses:", error);
      }
    };
    void load();
    void listen("profile-statuses-changed", () => {
      void load();
    }).then((stop) => {
      if (mounted) unlisten = stop;
      else stop();
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  return statuses;
}

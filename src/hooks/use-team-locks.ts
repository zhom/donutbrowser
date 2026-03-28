import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { ProfileLockInfo } from "@/types";

export function useTeamLocks(currentUserId?: string) {
  const [locks, setLocks] = useState<ProfileLockInfo[]>([]);

  const fetchLocks = useCallback(async () => {
    try {
      const result = await invoke<ProfileLockInfo[]>("get_team_locks");
      setLocks(result);
    } catch {
      // Not connected to a team or not logged in
    }
  }, []);

  useEffect(() => {
    void fetchLocks();

    const unlistenAcquired = listen<{ profileId: string }>(
      "team-lock-acquired",
      () => void fetchLocks(),
    );
    const unlistenReleased = listen<{ profileId: string }>(
      "team-lock-released",
      () => void fetchLocks(),
    );

    return () => {
      void unlistenAcquired.then((fn) => {
        fn();
      });
      void unlistenReleased.then((fn) => {
        fn();
      });
    };
  }, [fetchLocks]);

  const isProfileLocked = useCallback(
    (profileId: string): boolean => {
      const lock = locks.find((l) => l.profileId === profileId);
      if (!lock) return false;
      if (currentUserId && lock.lockedBy === currentUserId) return false;
      return true;
    },
    [locks, currentUserId],
  );

  const getLockInfo = useCallback(
    (profileId: string): ProfileLockInfo | undefined => {
      return locks.find((l) => l.profileId === profileId);
    },
    [locks],
  );

  return { locks, isProfileLocked, getLockInfo, refetchLocks: fetchLocks };
}

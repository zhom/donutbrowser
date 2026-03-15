import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { SyncSessionInfo } from "@/types";

/**
 * Hook to track active synchronizer sessions and provide helper methods
 * for determining if a profile is a leader, follower, or desynced.
 */
export function useSyncSessions() {
  const [sessions, setSessions] = useState<SyncSessionInfo[]>([]);

  const loadSessions = useCallback(async () => {
    try {
      const data = await invoke<SyncSessionInfo[]>("get_sync_sessions");
      setSessions(data);
    } catch (err) {
      console.error("Failed to load sync sessions:", err);
    }
  }, []);

  useEffect(() => {
    let changedUnlisten: (() => void) | undefined;
    let endedUnlisten: (() => void) | undefined;

    const setup = async () => {
      await loadSessions();

      changedUnlisten = await listen<SyncSessionInfo>(
        "sync-session-changed",
        (event) => {
          setSessions((prev) => {
            const idx = prev.findIndex((s) => s.id === event.payload.id);
            if (idx >= 0) {
              const next = [...prev];
              next[idx] = event.payload;
              return next;
            }
            return [...prev, event.payload];
          });
        },
      );

      endedUnlisten = await listen<string>("sync-session-ended", (event) => {
        setSessions((prev) => prev.filter((s) => s.id !== event.payload));
      });
    };

    void setup();

    return () => {
      changedUnlisten?.();
      endedUnlisten?.();
    };
  }, [loadSessions]);

  /** Find the session a profile belongs to and its role */
  const getProfileSyncInfo = useCallback(
    (
      profileId: string,
    ):
      | {
          session: SyncSessionInfo;
          isLeader: boolean;
          failedAtUrl: string | null;
        }
      | undefined => {
      for (const session of sessions) {
        if (session.leader_profile_id === profileId) {
          return { session, isLeader: true, failedAtUrl: null };
        }
        const follower = session.followers.find(
          (f) => f.profile_id === profileId,
        );
        if (follower) {
          return {
            session,
            isLeader: false,
            failedAtUrl: follower.failed_at_url,
          };
        }
      }
      return undefined;
    },
    [sessions],
  );

  return { sessions, getProfileSyncInfo, loadSessions };
}

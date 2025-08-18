import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { BrowserProfile, GroupWithCount } from "@/types";

interface UseProfileEventsReturn {
  profiles: BrowserProfile[];
  groups: GroupWithCount[];
  runningProfiles: Set<string>;
  isLoading: boolean;
  error: string | null;
  loadProfiles: () => Promise<void>;
  loadGroups: () => Promise<void>;
  clearError: () => void;
}

/**
 * Custom hook to manage profile-related state and listen for backend events.
 * This hook eliminates the need for manual UI refreshes by automatically
 * updating state when the backend emits profile change events.
 */
export function useProfileEvents(): UseProfileEventsReturn {
  const [profiles, setProfiles] = useState<BrowserProfile[]>([]);
  const [groups, setGroups] = useState<GroupWithCount[]>([]);
  const [runningProfiles, setRunningProfiles] = useState<Set<string>>(
    new Set(),
  );
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Load profiles from backend
  const loadProfiles = useCallback(async () => {
    try {
      const profileList = await invoke<BrowserProfile[]>(
        "list_browser_profiles",
      );
      setProfiles(profileList);
      setError(null);
    } catch (err: unknown) {
      console.error("Failed to load profiles:", err);
      setError(`Failed to load profiles: ${JSON.stringify(err)}`);
    }
  }, []);

  // Load groups from backend
  const loadGroups = useCallback(async () => {
    try {
      const groupsWithCounts = await invoke<GroupWithCount[]>(
        "get_groups_with_profile_counts",
      );
      setGroups(groupsWithCounts);
      setError(null);
    } catch (err) {
      console.error("Failed to load groups with counts:", err);
      setGroups([]);
    }
  }, []);

  // Clear error state
  const clearError = useCallback(() => {
    setError(null);
  }, []);

  // Initial load and event listeners setup
  useEffect(() => {
    let profilesUnlisten: (() => void) | undefined;
    let runningUnlisten: (() => void) | undefined;

    const setupListeners = async () => {
      try {
        // Initial load
        await Promise.all([loadProfiles(), loadGroups()]);

        // Listen for profile changes (create, delete, rename, update, etc.)
        profilesUnlisten = await listen("profiles-changed", () => {
          console.log(
            "Received profiles-changed event, reloading profiles and groups",
          );
          void loadProfiles();
          void loadGroups();
        });

        // Listen for profile running state changes
        runningUnlisten = await listen<{ id: string; is_running: boolean }>(
          "profile-running-changed",
          (event) => {
            const { id, is_running } = event.payload;
            setRunningProfiles((prev) => {
              const next = new Set(prev);
              if (is_running) {
                next.add(id);
              } else {
                next.delete(id);
              }
              return next;
            });
          },
        );

        console.log("Profile event listeners set up successfully");
      } catch (err) {
        console.error("Failed to setup profile event listeners:", err);
        setError(
          `Failed to setup profile event listeners: ${JSON.stringify(err)}`,
        );
      } finally {
        setIsLoading(false);
      }
    };

    void setupListeners();

    // Cleanup listeners on unmount
    return () => {
      if (profilesUnlisten) profilesUnlisten();
      if (runningUnlisten) runningUnlisten();
    };
  }, [loadProfiles, loadGroups]);

  // Sync profile running states periodically to ensure consistency
  useEffect(() => {
    const syncRunningStates = async () => {
      if (profiles.length === 0) return;

      try {
        const statusChecks = profiles.map(async (profile) => {
          try {
            const isRunning = await invoke<boolean>("check_browser_status", {
              profile,
            });
            return { id: profile.id, isRunning };
          } catch (error) {
            console.error(
              `Failed to check status for profile ${profile.name}:`,
              error,
            );
            return { id: profile.id, isRunning: false };
          }
        });

        const statuses = await Promise.all(statusChecks);

        setRunningProfiles((prev) => {
          const next = new Set(prev);
          let hasChanges = false;

          statuses.forEach(({ id, isRunning }) => {
            if (isRunning && !prev.has(id)) {
              next.add(id);
              hasChanges = true;
            } else if (!isRunning && prev.has(id)) {
              next.delete(id);
              hasChanges = true;
            }
          });

          return hasChanges ? next : prev;
        });
      } catch (error) {
        console.error("Failed to sync profile running states:", error);
      }
    };

    // Initial sync
    void syncRunningStates();

    // Sync every 30 seconds to catch any missed events
    const interval = setInterval(() => {
      void syncRunningStates();
    }, 30000);

    return () => clearInterval(interval);
  }, [profiles]);

  return {
    profiles,
    groups,
    runningProfiles,
    isLoading,
    error,
    loadProfiles,
    loadGroups,
    clearError,
  };
}

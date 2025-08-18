import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { GroupWithCount } from "@/types";

/**
 * Custom hook to manage group-related state and listen for backend events.
 * This hook eliminates the need for manual UI refreshes by automatically
 * updating state when the backend emits group change events.
 */
export function useGroupEvents() {
  const [groups, setGroups] = useState<GroupWithCount[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Load groups from backend
  const loadGroups = useCallback(async () => {
    try {
      const groupsWithCounts = await invoke<GroupWithCount[]>(
        "get_groups_with_profile_counts",
      );
      setGroups(groupsWithCounts);
      setError(null);
    } catch (err: unknown) {
      console.error("Failed to load groups:", err);
      setError(`Failed to load groups: ${JSON.stringify(err)}`);
    }
  }, []);

  // Clear error state
  const clearError = useCallback(() => {
    setError(null);
  }, []);

  // Initial load and event listeners setup
  useEffect(() => {
    let groupsUnlisten: (() => void) | undefined;

    const setupListeners = async () => {
      try {
        // Initial load
        await loadGroups();

        // Listen for group changes (create, delete, rename, update, etc.)
        groupsUnlisten = await listen("groups-changed", () => {
          console.log("Received groups-changed event, reloading groups");
          void loadGroups();
        });

        // Also listen for profile changes since groups show profile counts
        const profilesUnlisten = await listen("profiles-changed", () => {
          console.log(
            "Received profiles-changed event, reloading groups for updated counts",
          );
          void loadGroups();
        });

        // Store both listeners for cleanup
        groupsUnlisten = () => {
          groupsUnlisten?.();
          profilesUnlisten();
        };

        console.log("Group event listeners set up successfully");
      } catch (err) {
        console.error("Failed to setup group event listeners:", err);
        setError(
          `Failed to setup group event listeners: ${JSON.stringify(err)}`,
        );
      } finally {
        setIsLoading(false);
      }
    };

    void setupListeners();

    // Cleanup listeners on unmount
    return () => {
      if (groupsUnlisten) groupsUnlisten();
    };
  }, [loadGroups]);

  return {
    groups,
    isLoading,
    error,
    loadGroups,
    clearError,
  };
}

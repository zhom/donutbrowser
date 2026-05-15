import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import i18n from "@/i18n";
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
      setError(
        i18n.t("errors.loadGroupsFailed", { error: JSON.stringify(err) }),
      );
    }
  }, []);

  // Clear error state
  const clearError = useCallback(() => {
    setError(null);
  }, []);

  // Initial load and event listeners setup
  useEffect(() => {
    let groupsUnlisten: (() => void) | undefined;
    let profilesUnlisten: (() => void) | undefined;

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
        profilesUnlisten = await listen("profiles-changed", () => {
          console.log(
            "Received profiles-changed event, reloading groups for updated counts",
          );
          void loadGroups();
        });

        console.log("Group event listeners set up successfully");
      } catch (err) {
        console.error("Failed to setup group event listeners:", err);
        setError(
          i18n.t("errors.setupGroupListenersFailed", {
            error: JSON.stringify(err),
          }),
        );
      } finally {
        setIsLoading(false);
      }
    };

    void setupListeners();

    // Cleanup listeners on unmount.
    // NOTE: the previous version stored both unlisten fns by reassigning
    // `groupsUnlisten` to a wrapper that called itself, which produced a
    // `Maximum call stack size exceeded` crash whenever this effect tore
    // down. React's reconciler then bailed out mid-commit and left stale
    // overlay nodes in the DOM, blocking every subsequent click in the
    // window. Holding the two unlisten fns in separate locals avoids both
    // problems.
    return () => {
      if (groupsUnlisten) groupsUnlisten();
      if (profilesUnlisten) profilesUnlisten();
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

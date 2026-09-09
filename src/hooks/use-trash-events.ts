import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import i18n from "@/i18n";
import type { TrashedProfileSummary } from "@/types";

/**
 * The trash as the backend reports it, kept current by the `trash-changed`
 * event every trash mutation (delete, restore, purge, expiry sweep) emits.
 */
export function useTrashEvents() {
  const [entries, setEntries] = useState<TrashedProfileSummary[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadEntries = useCallback(async () => {
    try {
      const listed = await invoke<TrashedProfileSummary[]>(
        "list_trashed_profiles",
      );
      setEntries(listed);
      setError(null);
    } catch (err: unknown) {
      console.error("Failed to load trash:", err);
      setError(
        i18n.t("trash.loadFailed", {
          error: err instanceof Error ? err.message : String(err),
        }),
      );
    }
  }, []);

  useEffect(() => {
    let trashUnlisten: (() => void) | undefined;
    let cancelled = false;

    const setupListeners = async () => {
      try {
        await loadEntries();
        const unlisten = await listen("trash-changed", () => {
          void loadEntries();
        });
        // The effect may already be torn down by the time listen() resolves.
        if (cancelled) {
          unlisten();
        } else {
          trashUnlisten = unlisten;
        }
      } catch (err) {
        console.error("Failed to set up trash event listener:", err);
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };

    void setupListeners();

    return () => {
      cancelled = true;
      if (trashUnlisten) trashUnlisten();
    };
  }, [loadEntries]);

  return { entries, isLoading, error, reload: loadEntries };
}

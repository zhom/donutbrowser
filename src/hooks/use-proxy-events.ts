import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { StoredProxy } from "@/types";

/**
 * Custom hook to manage proxy-related state and listen for backend events.
 * This hook eliminates the need for manual UI refreshes by automatically
 * updating state when the backend emits proxy change events.
 */
export function useProxyEvents() {
  const [storedProxies, setStoredProxies] = useState<StoredProxy[]>([]);
  const [proxyUsage, setProxyUsage] = useState<Record<string, number>>({});
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Load proxy usage (how many profiles are using each proxy)
  const loadProxyUsage = useCallback(async () => {
    try {
      const profiles = await invoke<Array<{ proxy_id?: string }>>(
        "list_browser_profiles",
      );
      const counts: Record<string, number> = {};
      for (const p of profiles) {
        if (p.proxy_id) counts[p.proxy_id] = (counts[p.proxy_id] ?? 0) + 1;
      }
      setProxyUsage(counts);
    } catch (err) {
      console.error("Failed to load proxy usage:", err);
      // Don't set error for non-critical proxy usage
    }
  }, []);

  // Load proxies from backend
  const loadProxies = useCallback(async () => {
    try {
      const stored = await invoke<StoredProxy[]>("get_stored_proxies");
      setStoredProxies(stored);
      await loadProxyUsage();
      setError(null);
    } catch (err: unknown) {
      console.error("Failed to load proxies:", err);
      setError(`Failed to load proxies: ${JSON.stringify(err)}`);
    }
  }, [loadProxyUsage]);

  // Clear error state
  const clearError = useCallback(() => {
    setError(null);
  }, []);

  // Initial load and event listeners setup
  useEffect(() => {
    let proxiesUnlisten: (() => void) | undefined;
    let profilesUnlisten: (() => void) | undefined;
    let storedProxiesUnlisten: (() => void) | undefined;

    const setupListeners = async () => {
      try {
        // Initial load
        await loadProxies();

        // Listen for proxy changes (create, delete, update, start, stop, etc.)
        proxiesUnlisten = await listen("proxies-changed", () => {
          console.log("Received proxies-changed event, reloading proxies");
          void loadProxies();
        });

        // Listen for profile changes to update proxy usage counts
        profilesUnlisten = await listen("profiles-changed", () => {
          console.log("Received profiles-changed event, reloading proxy usage");
          void loadProxyUsage();
        });

        // Listen for profile updates to update proxy usage counts
        storedProxiesUnlisten = await listen("stored-proxies-changed", () => {
          console.log(
            "Received stored-proxies-changed event, reloading proxies",
          );
          void loadProxies();
        });

        console.log("Proxy event listeners set up successfully");
      } catch (err) {
        console.error("Failed to setup proxy event listeners:", err);
        setError(
          `Failed to setup proxy event listeners: ${JSON.stringify(err)}`,
        );
      } finally {
        setIsLoading(false);
      }
    };

    void setupListeners();

    // Cleanup listeners on unmount
    return () => {
      if (proxiesUnlisten) proxiesUnlisten();
      if (profilesUnlisten) profilesUnlisten();
      if (storedProxiesUnlisten) storedProxiesUnlisten();
    };
  }, [loadProxies, loadProxyUsage]);

  return {
    storedProxies,
    proxyUsage,
    isLoading,
    error,
    loadProxies,
    clearError,
  };
}

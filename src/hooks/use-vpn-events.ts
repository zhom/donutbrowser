import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { VpnConfig } from "@/types";

/**
 * Custom hook to manage VPN-related state and listen for backend events.
 * This hook eliminates the need for manual UI refreshes by automatically
 * updating state when the backend emits VPN change events.
 */
export function useVpnEvents() {
  const [vpnConfigs, setVpnConfigs] = useState<VpnConfig[]>([]);
  const [vpnUsage, setVpnUsage] = useState<Record<string, number>>({});
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadVpnUsage = useCallback(async () => {
    try {
      const profiles = await invoke<Array<{ vpn_id?: string }>>(
        "list_browser_profiles",
      );
      const counts: Record<string, number> = {};
      for (const p of profiles) {
        if (p.vpn_id) counts[p.vpn_id] = (counts[p.vpn_id] ?? 0) + 1;
      }
      setVpnUsage(counts);
    } catch (err) {
      console.error("Failed to load VPN usage:", err);
    }
  }, []);

  const loadVpnConfigs = useCallback(async () => {
    try {
      const configs = await invoke<VpnConfig[]>("list_vpn_configs");
      setVpnConfigs(configs);
      await loadVpnUsage();
      setError(null);
    } catch (err: unknown) {
      console.error("Failed to load VPN configs:", err);
      setError(`Failed to load VPN configs: ${JSON.stringify(err)}`);
    }
  }, [loadVpnUsage]);

  const clearError = useCallback(() => {
    setError(null);
  }, []);

  useEffect(() => {
    let vpnConfigsUnlisten: (() => void) | undefined;
    let profilesUnlisten: (() => void) | undefined;

    const setupListeners = async () => {
      try {
        await loadVpnConfigs();

        vpnConfigsUnlisten = await listen("vpn-configs-changed", () => {
          void loadVpnConfigs();
        });

        profilesUnlisten = await listen("profiles-changed", () => {
          void loadVpnUsage();
        });
      } catch (err) {
        console.error("Failed to setup VPN event listeners:", err);
        setError(`Failed to setup VPN event listeners: ${JSON.stringify(err)}`);
      } finally {
        setIsLoading(false);
      }
    };

    void setupListeners();

    return () => {
      if (vpnConfigsUnlisten) vpnConfigsUnlisten();
      if (profilesUnlisten) profilesUnlisten();
    };
  }, [loadVpnConfigs, loadVpnUsage]);

  return {
    vpnConfigs,
    vpnUsage,
    isLoading,
    error,
    loadVpnConfigs,
    clearError,
  };
}

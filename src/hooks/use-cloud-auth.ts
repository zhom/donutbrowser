import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { CloudAuthState, CloudUser } from "@/types";

interface UseCloudAuthReturn {
  user: CloudUser | null;
  isLoggedIn: boolean;
  isLoading: boolean;
  requestOtp: (email: string) => Promise<string>;
  verifyOtp: (email: string, code: string) => Promise<CloudAuthState>;
  logout: () => Promise<void>;
  refreshProfile: () => Promise<CloudUser>;
}

export function useCloudAuth(): UseCloudAuthReturn {
  const [authState, setAuthState] = useState<CloudAuthState | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const loadUser = useCallback(async () => {
    try {
      const state = await invoke<CloudAuthState | null>("cloud_get_user");
      setAuthState(state);
    } catch (error) {
      console.error("Failed to load cloud auth state:", error);
      setAuthState(null);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadUser();

    const unlistenExpired = listen("cloud-auth-expired", () => {
      setAuthState(null);
    });

    const unlistenChanged = listen("cloud-auth-changed", () => {
      loadUser();
    });

    return () => {
      void unlistenExpired.then((unlisten) => {
        unlisten();
      });
      void unlistenChanged.then((unlisten) => {
        unlisten();
      });
    };
  }, [loadUser]);

  const requestOtp = useCallback(async (email: string): Promise<string> => {
    return invoke<string>("cloud_request_otp", { email });
  }, []);

  const verifyOtp = useCallback(
    async (email: string, code: string): Promise<CloudAuthState> => {
      const state = await invoke<CloudAuthState>("cloud_verify_otp", {
        email,
        code,
      });
      setAuthState(state);
      return state;
    },
    [],
  );

  const logout = useCallback(async () => {
    await invoke("cloud_logout");
    setAuthState(null);
  }, []);

  const refreshProfile = useCallback(async (): Promise<CloudUser> => {
    const user = await invoke<CloudUser>("cloud_refresh_profile");
    setAuthState((prev) =>
      prev
        ? { ...prev, user }
        : { user, logged_in_at: new Date().toISOString() },
    );
    return user;
  }, []);

  return {
    user: authState?.user ?? null,
    isLoggedIn: authState !== null,
    isLoading,
    requestOtp,
    verifyOtp,
    logout,
    refreshProfile,
  };
}

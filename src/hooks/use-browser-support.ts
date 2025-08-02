import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import type { BrowserProfile } from "@/types";

export function useBrowserSupport() {
  const [supportedBrowsers, setSupportedBrowsers] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const loadSupportedBrowsers = async () => {
      try {
        setIsLoading(true);
        setError(null);
        const browsers = await invoke<string[]>("get_supported_browsers");
        setSupportedBrowsers(browsers);
      } catch (err) {
        console.error("Failed to load supported browsers:", err);
        setError(
          err instanceof Error
            ? err.message
            : "Failed to load supported browsers",
        );
      } finally {
        setIsLoading(false);
      }
    };

    void loadSupportedBrowsers();
  }, []);

  const isBrowserSupported = (browser: string): boolean => {
    return supportedBrowsers.includes(browser);
  };

  const checkBrowserSupport = async (browser: string): Promise<boolean> => {
    try {
      return await invoke<boolean>("is_browser_supported_on_platform", {
        browserStr: browser,
      });
    } catch (err) {
      console.error(`Failed to check support for browser ${browser}:`, err);
      return false;
    }
  };

  return {
    supportedBrowsers,
    isLoading,
    error,
    isBrowserSupported,
    checkBrowserSupport,
  };
}

/**
 * Hook for managing browser state and enforcing single-instance rules for Tor and Mullvad browsers
 */
export function useBrowserState(
  profiles: BrowserProfile[],
  runningProfiles: Set<string>,
  isUpdating?: (browser: string) => boolean,
) {
  const [isClient, setIsClient] = useState(false);

  useEffect(() => {
    setIsClient(true);
  }, []);

  /**
   * Check if a browser type allows only one instance to run at a time
   */
  const isSingleInstanceBrowser = useCallback(
    (browserType: string): boolean => {
      return browserType === "tor-browser" || browserType === "mullvad-browser";
    },
    [],
  );

  /**
   * Check if any instance of a specific browser type is currently running
   */
  const isAnyInstanceRunning = useCallback(
    (browserType: string): boolean => {
      if (!isClient) return false;
      return profiles.some(
        (p) => p.browser === browserType && runningProfiles.has(p.name),
      );
    },
    [profiles, runningProfiles, isClient],
  );

  /**
   * Check if a profile can be launched (not disabled by single-instance rules)
   */
  const canLaunchProfile = useCallback(
    (profile: BrowserProfile): boolean => {
      if (!isClient) return false;

      const isRunning = runningProfiles.has(profile.name);
      const isBrowserUpdating = isUpdating?.(profile.browser) ?? false;

      // If the profile is already running, it can always be stopped
      if (isRunning) return true;

      // If THIS specific browser is updating or downloading, block this profile
      if (isBrowserUpdating) {
        return false;
      }

      // For single-instance browsers, check if any instance is running
      if (isSingleInstanceBrowser(profile.browser)) {
        return !isAnyInstanceRunning(profile.browser);
      }

      return true;
    },
    [
      runningProfiles,
      isClient,
      isUpdating,
      isSingleInstanceBrowser,
      isAnyInstanceRunning,
    ],
  );

  /**
   * Check if a profile can be used for opening links
   * This is more restrictive than canLaunchProfile as it considers running state
   */
  const canUseProfileForLinks = useCallback(
    (profile: BrowserProfile): boolean => {
      if (!isClient) return false;

      const isRunning = runningProfiles.has(profile.name);
      const isBrowserUpdating = isUpdating?.(profile.browser) ?? false;

      // If this specific browser is updating or downloading, block it
      if (isBrowserUpdating) {
        return false;
      }

      // For single-instance browsers (Tor and Mullvad)
      if (isSingleInstanceBrowser(profile.browser)) {
        const runningInstancesOfType = profiles.filter(
          (p) => p.browser === profile.browser && runningProfiles.has(p.name),
        );

        // If no instances are running, any profile of this type can be used
        if (runningInstancesOfType.length === 0) {
          return true;
        }

        // If instances are running, only the running ones can be used
        return isRunning;
      }

      // For other browsers, any profile can be used
      return true;
    },
    [profiles, runningProfiles, isClient, isSingleInstanceBrowser, isUpdating],
  );

  /**
   * Check if a profile can be selected for actions (delete, move group, etc.)
   */
  const canSelectProfile = useCallback(
    (profile: BrowserProfile): boolean => {
      if (!isClient) return false;

      const isBrowserUpdating = isUpdating?.(profile.browser) ?? false;

      // If this specific browser is updating or downloading, block selection
      if (isBrowserUpdating) {
        return false;
      }

      return true;
    },
    [isClient, isUpdating],
  );

  /**
   * Get tooltip content for a profile's launch button
   */
  const getLaunchTooltipContent = useCallback(
    (profile: BrowserProfile): string => {
      if (!isClient) return "Loading...";

      const isRunning = runningProfiles.has(profile.name);
      const isBrowserUpdating = isUpdating?.(profile.browser) ?? false;

      if (isRunning) {
        return "";
      }

      if (isBrowserUpdating) {
        return `${getBrowserDisplayName(profile.browser)} is being updated. Please wait for the update to complete.`;
      }

      if (
        isSingleInstanceBrowser(profile.browser) &&
        !canLaunchProfile(profile)
      ) {
        const browserDisplayName =
          profile.browser === "tor-browser" ? "TOR" : "Mullvad";
        return `Only one ${browserDisplayName} browser instance can run at a time. Stop the running ${browserDisplayName} browser first.`;
      }

      return "";
    },
    [
      runningProfiles,
      isClient,
      isUpdating,
      isSingleInstanceBrowser,
      canLaunchProfile,
    ],
  );

  /**
   * Get tooltip content for profile selection (for opening links)
   */
  const getProfileTooltipContent = useCallback(
    (profile: BrowserProfile): string | null => {
      if (!isClient) return null;

      const canUseForLinks = canUseProfileForLinks(profile);

      if (canUseForLinks) return null;

      const isBrowserUpdating = isUpdating?.(profile.browser) ?? false;

      if (isBrowserUpdating) {
        return `${getBrowserDisplayName(profile.browser)} is being updated. Please wait for the update to complete.`;
      }

      if (isSingleInstanceBrowser(profile.browser)) {
        const browserDisplayName =
          profile.browser === "tor-browser" ? "TOR" : "Mullvad";
        const runningInstancesOfType = profiles.filter(
          (p) => p.browser === profile.browser && runningProfiles.has(p.name),
        );

        if (runningInstancesOfType.length > 0) {
          const runningProfileNames = runningInstancesOfType
            .map((p) => p.name)
            .join(", ");
          return `${browserDisplayName} browser is already running (${runningProfileNames}). Only one instance can run at a time.`;
        }
      }

      return "This profile cannot be used for opening links right now.";
    },
    [
      profiles,
      runningProfiles,
      isClient,
      canUseProfileForLinks,
      isSingleInstanceBrowser,
      isUpdating,
    ],
  );

  return {
    isClient,
    isSingleInstanceBrowser,
    isAnyInstanceRunning,
    canLaunchProfile,
    canUseProfileForLinks,
    canSelectProfile,
    getLaunchTooltipContent,
    getProfileTooltipContent,
  };
}

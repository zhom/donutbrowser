import { getBrowserDisplayName } from "@/lib/browser-utils";
import { dismissToast, showUnifiedVersionUpdateToast } from "@/lib/toast-utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

interface VersionUpdateProgress {
  current_browser: string;
  total_browsers: number;
  completed_browsers: number;
  new_versions_found: number;
  browser_new_versions: number;
  status: string; // "updating", "completed", "error"
}

interface BackgroundUpdateResult {
  browser: string;
  new_versions_count: number;
  total_versions_count: number;
  updated_successfully: boolean;
  error?: string;
}

interface BrowserVersionsResult {
  versions: string[];
  new_versions_count?: number;
  total_versions_count: number;
}

export function useVersionUpdater() {
  const [isUpdating, setIsUpdating] = useState(false);
  const [lastUpdateTime, setLastUpdateTime] = useState<number | null>(null);
  const [timeUntilNextUpdate, setTimeUntilNextUpdate] = useState<number>(0);
  const [updateProgress, setUpdateProgress] =
    useState<VersionUpdateProgress | null>(null);

  // Listen for version update progress events
  useEffect(() => {
    const unlisten = listen<VersionUpdateProgress>(
      "version-update-progress",
      (event) => {
        const progress = event.payload;
        setUpdateProgress(progress);

        if (progress.status === "updating") {
          setIsUpdating(true);

          // Show unified progress toast
          const currentBrowserName = progress.current_browser
            ? getBrowserDisplayName(progress.current_browser)
            : undefined;

          showUnifiedVersionUpdateToast("Checking for browser updates...", {
            description: currentBrowserName
              ? `Fetching ${currentBrowserName} release information...`
              : "Initializing version check...",
            progress: {
              current: progress.completed_browsers,
              total: progress.total_browsers,
              found: progress.new_versions_found,
              current_browser: currentBrowserName,
            },
          });
        } else if (progress.status === "completed") {
          setIsUpdating(false);
          setUpdateProgress(null);
          dismissToast("unified-version-update");

          if (progress.new_versions_found > 0) {
            toast.success(
              `Found ${progress.new_versions_found} new browser versions!`,
              {
                duration: 4000,
                description:
                  "Version information has been updated in the background",
              },
            );
          } else {
            toast.success("No new browser versions found", {
              duration: 3000,
              description: "All browser versions are up to date",
            });
          }

          // Refresh status
          void loadUpdateStatus();
        } else if (progress.status === "error") {
          setIsUpdating(false);
          setUpdateProgress(null);
          dismissToast("unified-version-update");

          toast.error("Failed to update browser versions", {
            duration: 4000,
            description: "Check your internet connection and try again",
          });
        }
      },
    );

    return () => {
      void unlisten.then((fn) => {
        fn();
      });
    };
  }, []);

  // Load update status on mount and periodically
  useEffect(() => {
    void loadUpdateStatus();

    // Update status every minute
    const interval = setInterval(() => {
      void loadUpdateStatus();
    }, 60000);

    return () => {
      clearInterval(interval);
    };
  }, []);

  const loadUpdateStatus = useCallback(async () => {
    try {
      const [lastUpdate, timeUntilNext] = await invoke<[number | null, number]>(
        "get_version_update_status",
      );
      setLastUpdateTime(lastUpdate);
      setTimeUntilNextUpdate(timeUntilNext);
    } catch (error) {
      console.error("Failed to load version update status:", error);
    }
  }, []);

  const triggerManualUpdate = useCallback(async () => {
    try {
      setIsUpdating(true);
      const results = await invoke<BackgroundUpdateResult[]>(
        "trigger_manual_version_update",
      );

      const totalNewVersions = results.reduce(
        (sum, result) => sum + result.new_versions_count,
        0,
      );
      const successfulUpdates = results.filter(
        (r) => r.updated_successfully,
      ).length;
      const failedUpdates = results.filter(
        (r) => !r.updated_successfully,
      ).length;

      if (failedUpdates > 0) {
        toast.warning("Update completed with some errors", {
          description: `${totalNewVersions} new versions found, ${failedUpdates} browsers failed to update`,
          duration: 5000,
        });
      } else if (totalNewVersions > 0) {
        toast.success("Browser versions updated successfully", {
          description: `Updated ${successfulUpdates} browsers successfully`,
          duration: 4000,
        });
      } else {
        toast.success("No new browser versions found", {
          description: "All browser versions are up to date",
          duration: 3000,
        });
      }

      await loadUpdateStatus();
      return results;
    } catch (error) {
      console.error("Failed to trigger manual update:", error);
      toast.error("Failed to update browser versions", {
        description:
          error instanceof Error ? error.message : "Unknown error occurred",
        duration: 4000,
      });
      throw error;
    } finally {
      setIsUpdating(false);
    }
  }, [loadUpdateStatus]);

  const fetchBrowserVersionsWithNewCount = useCallback(
    async (browserStr: string) => {
      try {
        const result = await invoke<BrowserVersionsResult>(
          "fetch_browser_versions_with_count",
          { browserStr },
        );

        // Show notification about new versions if any were found
        if (result.new_versions_count && result.new_versions_count > 0) {
          const browserName = getBrowserDisplayName(browserStr);
          toast.success(
            `Found ${result.new_versions_count} new ${browserName} versions!`,
            {
              duration: 3000,
              description: `Total available: ${result.total_versions_count} versions`,
            },
          );
        }

        return result;
      } catch (error) {
        console.error("Failed to fetch browser versions with count:", error);
        throw error;
      }
    },
    [],
  );

  const formatTimeUntilUpdate = useCallback((seconds: number): string => {
    if (seconds <= 0) return "Update overdue";

    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);

    if (hours > 0) {
      return `${hours}h ${minutes}m`;
    }
    if (minutes > 0) {
      return `${minutes}m`;
    }
    return "< 1m";
  }, []);

  const formatLastUpdateTime = useCallback(
    (timestamp: number | null): string => {
      if (!timestamp) return "Never";

      const date = new Date(timestamp * 1000);
      const now = new Date();
      const diffMs = now.getTime() - date.getTime();
      const diffHours = Math.floor(diffMs / (1000 * 60 * 60));
      const diffMinutes = Math.floor((diffMs % (1000 * 60 * 60)) / (1000 * 60));

      if (diffHours > 0) {
        return `${diffHours}h ${diffMinutes}m ago`;
      }
      if (diffMinutes > 0) {
        return `${diffMinutes}m ago`;
      }
      return "Just now";
    },
    [],
  );

  return {
    isUpdating,
    lastUpdateTime,
    timeUntilNextUpdate,
    updateProgress,
    triggerManualUpdate,
    fetchBrowserVersionsWithNewCount,
    formatTimeUntilUpdate,
    formatLastUpdateTime,
    loadUpdateStatus,
  };
}

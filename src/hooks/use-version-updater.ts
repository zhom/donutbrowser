import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import {
  dismissToast,
  showAutoUpdateToast,
  showErrorToast,
  showSuccessToast,
  showUnifiedVersionUpdateToast,
} from "@/lib/toast-utils";

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

interface AutoUpdateEvent {
  browser: string;
  new_version: string;
  notification_id: string;
  affected_profiles: string[];
}

export function useVersionUpdater() {
  const [isUpdating, setIsUpdating] = useState(false);
  const [lastUpdateTime, setLastUpdateTime] = useState<number | null>(null);
  const [timeUntilNextUpdate, setTimeUntilNextUpdate] = useState<number>(0);
  const [updateProgress, setUpdateProgress] =
    useState<VersionUpdateProgress | null>(null);

  // Track active downloads to prevent duplicates
  const activeDownloads = useRef(new Set<string>());

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

  // Listen for version update progress events
  useEffect(() => {
    let unlistenFn: (() => void) | null = null;

    const setupListener = async () => {
      try {
        unlistenFn = await listen<VersionUpdateProgress>(
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
                onCancel: () => dismissToast("unified-version-update"),
              });
            } else if (progress.status === "completed") {
              setIsUpdating(false);
              setUpdateProgress(null);
              dismissToast("unified-version-update");

              if (progress.new_versions_found > 0) {
                showSuccessToast("Browser versions updated successfully", {
                  duration: 5000,
                  description:
                    "Auto-downloads will start shortly for available updates.",
                });
              } else {
                showSuccessToast("No new browser versions found", {
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

              showErrorToast("Failed to update browser versions", {
                duration: 6000,
                description: "Check your internet connection and try again",
              });
            }
          },
        );
      } catch (error) {
        console.error(
          "Failed to setup version update progress listener:",
          error,
        );
      }
    };

    setupListener();

    return () => {
      if (unlistenFn) {
        try {
          unlistenFn();
        } catch (error) {
          console.error(
            "Failed to cleanup version update progress listener:",
            error,
          );
        }
      }
    };
  }, [loadUpdateStatus]);

  // Listen for browser auto-update events
  useEffect(() => {
    let unlistenFn: (() => void) | null = null;

    const setupListener = async () => {
      try {
        unlistenFn = await listen<AutoUpdateEvent>(
          "browser-auto-update-available",
          (event) => {
            const handleAutoUpdate = async () => {
              const { browser, new_version, notification_id } = event.payload;
              console.log("Browser auto-update event received:", event.payload);

              const browserDisplayName = getBrowserDisplayName(browser);
              const downloadKey = `${browser}-${new_version}`;

              // Check if this download is already in progress
              if (activeDownloads.current.has(downloadKey)) {
                console.log(
                  `Download already in progress for ${browserDisplayName} ${new_version}, skipping`,
                );
                return;
              }

              // Mark download as active
              activeDownloads.current.add(downloadKey);

              try {
                // Show auto-update start notification
                showAutoUpdateToast(browserDisplayName, new_version, {
                  description: `Downloading ${browserDisplayName} ${new_version} automatically. Progress will be shown below.`,
                });

                // Dismiss the update notification in the backend
                await invoke("dismiss_update_notification", {
                  notificationId: notification_id,
                });

                // Check if browser already exists before downloading
                const isDownloaded = await invoke<boolean>(
                  "check_browser_exists",
                  {
                    browserStr: browser,
                    version: new_version,
                  },
                );

                if (isDownloaded) {
                  // Browser already exists, skip download and go straight to profile update
                  console.log(
                    `${browserDisplayName} ${new_version} already exists, skipping download`,
                  );

                  showSuccessToast(
                    `${browserDisplayName} ${new_version} already available`,
                    {
                      description: "Updating profile configurations...",
                      duration: 3000,
                    },
                  );
                } else {
                  // Download the browser - this will trigger download progress events automatically
                  await invoke("download_browser", {
                    browserStr: browser,
                    version: new_version,
                  });
                }

                // Complete the update with auto-update of profile versions
                const updatedProfiles = await invoke<string[]>(
                  "complete_browser_update_with_auto_update",
                  {
                    browser,
                    newVersion: new_version,
                  },
                );

                // Show success message based on whether profiles were updated
                if (updatedProfiles.length > 0) {
                  const profileText =
                    updatedProfiles.length === 1
                      ? `Profile "${updatedProfiles[0]}" has been updated`
                      : `${updatedProfiles.length} profiles have been updated`;

                  showSuccessToast(`${browserDisplayName} update completed`, {
                    description: `${profileText} to version ${new_version}. You can now launch your browsers with the latest version.`,
                    duration: 6000,
                  });
                } else {
                  showSuccessToast(`${browserDisplayName} update completed`, {
                    description: `Version ${new_version} is now available. Running profiles will use the new version when restarted.`,
                    duration: 6000,
                  });
                }
              } catch (error) {
                console.error("Failed to handle browser auto-update:", error);

                let errorMessage = "Unknown error occurred";
                if (error instanceof Error) {
                  errorMessage = error.message;
                } else if (typeof error === "string") {
                  errorMessage = error;
                } else if (
                  error &&
                  typeof error === "object" &&
                  "message" in error
                ) {
                  errorMessage = String(error.message);
                }

                showErrorToast(`Failed to auto-update ${browserDisplayName}`, {
                  description: errorMessage,
                  duration: 8000,
                });
              } finally {
                // Remove from active downloads
                activeDownloads.current.delete(downloadKey);
              }
            };

            // Call the async handler
            void handleAutoUpdate();
          },
        );
      } catch (error) {
        console.error("Failed to setup browser auto-update listener:", error);
      }
    };

    setupListener();

    return () => {
      if (unlistenFn) {
        try {
          unlistenFn();
        } catch (error) {
          console.error(
            "Failed to cleanup browser auto-update listener:",
            error,
          );
        }
      }
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
  }, [loadUpdateStatus]);

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
        showErrorToast("Update completed with some errors", {
          description: `${totalNewVersions} new versions found, ${failedUpdates} browsers failed to update`,
          duration: 5000,
        });
      } else if (totalNewVersions > 0) {
        showSuccessToast("Browser versions updated successfully", {
          description: `Found ${totalNewVersions} new versions across ${successfulUpdates} browsers. Auto-downloads will start shortly.`,
          duration: 4000,
        });
      } else {
        showSuccessToast("No new browser versions found", {
          description: "All browser versions are up to date",
          duration: 3000,
        });
      }

      await loadUpdateStatus();
      return results;
    } catch (error) {
      console.error("Failed to trigger manual update:", error);
      let errorMessage = "Unknown error occurred";
      if (error instanceof Error) {
        errorMessage = error.message;
      } else if (typeof error === "string") {
        errorMessage = error;
      } else if (error && typeof error === "object" && "message" in error) {
        errorMessage = String(error.message);
      }

      showErrorToast("Failed to update browser versions", {
        description: errorMessage,
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
          showSuccessToast(
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
    if (seconds < 60) {
      return `${seconds} seconds`;
    }
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) {
      return `${minutes} minute${minutes === 1 ? "" : "s"}`;
    }
    const hours = Math.floor(minutes / 60);
    return `${hours} hour${hours === 1 ? "" : "s"}`;
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

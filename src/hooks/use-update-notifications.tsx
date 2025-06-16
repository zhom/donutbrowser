import { getBrowserDisplayName } from "@/lib/browser-utils";
import { showToast } from "@/lib/toast-utils";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";

interface UpdateNotification {
  id: string;
  browser: string;
  current_version: string;
  new_version: string;
  affected_profiles: string[];
  is_stable_update: boolean;
  timestamp: number;
  is_rolling_release: boolean;
}

export function useUpdateNotifications(
  onProfilesUpdated?: () => Promise<void>,
) {
  const [notifications, setNotifications] = useState<UpdateNotification[]>([]);
  const [updatingBrowsers, setUpdatingBrowsers] = useState<Set<string>>(
    new Set(),
  );
  const [processedNotifications, setProcessedNotifications] = useState<
    Set<string>
  >(new Set());

  // Add refs to track ongoing operations to prevent duplicates
  const isCheckingForUpdates = useRef(false);
  const activeDownloads = useRef<Set<string>>(new Set()); // Track "browser-version" keys

  const checkForUpdates = useCallback(async () => {
    // Prevent multiple simultaneous calls
    if (isCheckingForUpdates.current) {
      console.log("Already checking for updates, skipping duplicate call");
      return;
    }

    isCheckingForUpdates.current = true;

    try {
      const updates = await invoke<UpdateNotification[]>(
        "check_for_browser_updates",
      );

      // Filter out already processed notifications
      const newUpdates = updates.filter((notification) => {
        return !processedNotifications.has(notification.id);
      });

      setNotifications(newUpdates);

      // Automatically start downloads for new update notifications
      for (const notification of newUpdates) {
        if (!processedNotifications.has(notification.id)) {
          setProcessedNotifications((prev) =>
            new Set(prev).add(notification.id),
          );
          // Start automatic update without user interaction
          void handleAutoUpdate(
            notification.browser,
            notification.new_version,
            notification.id,
          );
        }
      }
    } catch (error) {
      console.error("Failed to check for updates:", error);
    } finally {
      isCheckingForUpdates.current = false;
    }
  }, [processedNotifications]);

  const handleAutoUpdate = useCallback(
    async (browser: string, newVersion: string, notificationId: string) => {
      const downloadKey = `${browser}-${newVersion}`;

      // Check if this download is already in progress
      if (activeDownloads.current.has(downloadKey)) {
        console.log(
          `Download already in progress for ${downloadKey}, skipping duplicate`,
        );
        return;
      }

      // Mark download as active
      activeDownloads.current.add(downloadKey);

      try {
        setUpdatingBrowsers((prev) => new Set(prev).add(browser));
        const browserDisplayName = getBrowserDisplayName(browser);

        // Dismiss the notification in the backend
        await invoke("dismiss_update_notification", {
          notificationId,
        });

        // Show update available toast and start download immediately
        showToast({
          id: `auto-update-started-${browser}-${newVersion}`,
          type: "loading",
          title: `${browserDisplayName} update available`,
          description: `Version ${newVersion} is now being downloaded. Browser launch will be disabled until update completes.`,
          duration: 4000,
        });

        try {
          // Check if browser already exists before downloading
          const isDownloaded = await invoke<boolean>("check_browser_exists", {
            browserStr: browser,
            version: newVersion,
          });

          if (isDownloaded) {
            // Browser already exists, skip download and go straight to profile update
            console.log(
              `${browserDisplayName} ${newVersion} already exists, skipping download`,
            );
          } else {
            // Don't mark as auto-update - we want to show full download progress
            // Download the browser (progress will be handled by use-browser-download hook)
            await invoke("download_browser", {
              browserStr: browser,
              version: newVersion,
            });
          }

          // Complete the update with auto-update of profile versions
          const updatedProfiles = await invoke<string[]>(
            "complete_browser_update_with_auto_update",
            {
              browser,
              newVersion,
            },
          );

          // Show success message based on whether profiles were updated
          if (updatedProfiles.length > 0) {
            const profileText =
              updatedProfiles.length === 1
                ? `Profile "${updatedProfiles[0]}" has been updated`
                : `${updatedProfiles.length} profiles have been updated`;

            showToast({
              id: `auto-update-success-${browser}-${newVersion}`,
              type: "success",
              title: `${browserDisplayName} update completed`,
              description: `${profileText} to version ${newVersion}. You can now launch your browsers with the latest version.`,
              duration: 6000,
            });
          } else {
            showToast({
              id: `auto-update-success-${browser}-${newVersion}`,
              type: "success",
              title: `${browserDisplayName} update completed`,
              description: `Version ${newVersion} is now available. Running profiles will use the new version when restarted.`,
              duration: 6000,
            });
          }

          // Trigger profile refresh to update UI with new versions
          if (onProfilesUpdated) {
            void onProfilesUpdated();
          }
        } catch (downloadError) {
          console.error("Failed to download browser:", downloadError);

          showToast({
            id: `auto-update-error-${browser}-${newVersion}`,
            type: "error",
            title: `Failed to download ${browserDisplayName} ${newVersion}`,
            description: String(downloadError),
            duration: 8000,
          });
          throw downloadError;
        }

        // Don't call checkForUpdates() again here as it can cause recursion and duplicates
        // The periodic checks will handle finding any remaining updates
      } catch (error) {
        console.error("Failed to start auto-update:", error);
        const browserDisplayName = getBrowserDisplayName(browser);
        showToast({
          id: `auto-update-error-${browser}-${newVersion}`,
          type: "error",
          title: `Failed to update ${browserDisplayName}`,
          description: String(error),
          duration: 8000,
        });
      } finally {
        // Remove from active downloads and updating browsers
        activeDownloads.current.delete(downloadKey);
        setUpdatingBrowsers((prev) => {
          const next = new Set(prev);
          next.delete(browser);
          return next;
        });
      }
    },
    [onProfilesUpdated],
  );

  // Clean up notifications when they're no longer needed
  useEffect(() => {
    // Remove notifications that have been processed
    setNotifications((prev) =>
      prev.filter(
        (notification) => !processedNotifications.has(notification.id),
      ),
    );
  }, [processedNotifications]);

  return {
    notifications,
    checkForUpdates,
    isUpdating: (browser: string) => updatingBrowsers.has(browser),
  };
}

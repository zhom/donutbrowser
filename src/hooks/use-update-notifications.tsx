import { invoke } from "@tauri-apps/api/core";
import { useCallback, useRef, useState } from "react";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import { dismissToast, showToast } from "@/lib/toast-utils";

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

  const isUpdating = useCallback(
    (browser: string) => updatingBrowsers.has(browser),
    [updatingBrowsers],
  );

  // Add refs to track ongoing operations to prevent duplicates
  const isCheckingForUpdates = useRef(false);
  // Track browser types being downloaded (not browser-version pairs)
  const activeDownloads = useRef<Set<string>>(new Set()); // Track browser types

  const handleAutoUpdate = useCallback(
    async (browser: string, newVersion: string, notificationId: string) => {
      // Check if this browser type is already being downloaded
      if (activeDownloads.current.has(browser)) {
        console.log(
          `Download already in progress for browser type ${browser}, skipping duplicate auto-update`,
        );
        return;
      }

      // Mark browser type as active and disable browser
      activeDownloads.current.add(browser);
      setUpdatingBrowsers((prev) => new Set(prev).add(browser));

      try {
        const browserDisplayName = getBrowserDisplayName(browser);

        // Dismiss the notification in the backend
        await invoke("dismiss_update_notification", {
          notificationId,
        });

        // Show update started notification
        showToast({
          id: `auto-update-started-${browser}-${newVersion}`,
          type: "loading",
          title: `${browserDisplayName} update started`,
          description: `Version ${newVersion} download will begin shortly. Browser launch is disabled until update completes.`,
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

            showToast({
              id: `auto-update-skip-download-${browser}-${newVersion}`,
              type: "success",
              title: `${browserDisplayName} ${newVersion} already available`,
              description: "Updating profile configurations...",
              duration: 3000,
            });
          } else {
            // Show download starting notification
            showToast({
              id: `auto-update-download-starting-${browser}-${newVersion}`,
              type: "loading",
              title: `Starting ${browserDisplayName} ${newVersion} download`,
              description: "Download progress will be shown below...",
              duration: 4000,
            });

            // Download the browser - this will trigger download progress events automatically
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

          if (onProfilesUpdated) {
            await onProfilesUpdated();
          }
        } catch (downloadError) {
          console.error("Failed to download browser:", downloadError);

          dismissToast(`download-${browser}-${newVersion}`);

          showToast({
            id: `auto-update-error-${browser}-${newVersion}`,
            type: "error",
            title: `Failed to download ${browserDisplayName} ${newVersion}`,
            description: String(downloadError),
            duration: 8000,
          });
          throw downloadError;
        }
      } catch (error) {
        console.error("Failed to start auto-update:", error);
        throw error;
      } finally {
        // Clean up - remove browser type from active downloads
        activeDownloads.current.delete(browser);
        setUpdatingBrowsers((prev) => {
          const next = new Set(prev);
          next.delete(browser);
          return next;
        });
      }
    },
    [onProfilesUpdated],
  );

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
  }, [processedNotifications, handleAutoUpdate]);

  return {
    notifications,
    isUpdating,
    checkForUpdates,
  };
}

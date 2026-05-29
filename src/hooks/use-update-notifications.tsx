import { invoke } from "@tauri-apps/api/core";
import { useCallback, useRef, useState } from "react";
import i18n from "@/i18n";
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
          title: i18n.t("versionUpdater.toast.updateStarted", {
            browser: browserDisplayName,
          }),
          description: i18n.t("versionUpdater.toast.updateStartedDescription", {
            version: newVersion,
          }),
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
              title: i18n.t("versionUpdater.toast.alreadyAvailable", {
                browser: browserDisplayName,
                version: newVersion,
              }),
              description: i18n.t("versionUpdater.toast.updatingProfiles"),
              duration: 3000,
            });
          } else {
            // Show download starting notification
            showToast({
              id: `auto-update-download-starting-${browser}-${newVersion}`,
              type: "loading",
              title: i18n.t("versionUpdater.toast.downloadStarting", {
                browser: browserDisplayName,
                version: newVersion,
              }),
              description: i18n.t("versionUpdater.toast.downloadProgressBelow"),
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
            const description =
              updatedProfiles.length === 1
                ? i18n.t("versionUpdater.toast.singleProfileUpdated", {
                    name: updatedProfiles[0],
                    version: newVersion,
                  })
                : i18n.t("versionUpdater.toast.multipleProfilesUpdated", {
                    count: updatedProfiles.length,
                    version: newVersion,
                  });

            showToast({
              id: `auto-update-success-${browser}-${newVersion}`,
              type: "success",
              title: i18n.t("versionUpdater.toast.updateCompleted", {
                browser: browserDisplayName,
              }),
              description,
              duration: 6000,
            });
          } else {
            showToast({
              id: `auto-update-success-${browser}-${newVersion}`,
              type: "success",
              title: i18n.t("versionUpdater.toast.updateCompleted", {
                browser: browserDisplayName,
              }),
              description: i18n.t("versionUpdater.toast.versionAvailable", {
                version: newVersion,
              }),
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
            title: i18n.t("browserDownload.toast.downloadFailed", {
              browser: browserDisplayName,
              version: newVersion,
            }),
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

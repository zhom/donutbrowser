import { UpdateNotificationComponent } from "@/components/update-notification";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import { showToast } from "@/lib/toast-utils";
import { invoke } from "@tauri-apps/api/core";
import React, { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

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
  const [dismissedNotifications, setDismissedNotifications] = useState<
    Set<string>
  >(new Set());

  const checkForUpdates = useCallback(async () => {
    try {
      const updates = await invoke<UpdateNotification[]>(
        "check_for_browser_updates",
      );

      // Filter out dismissed notifications unless they're for a newer version
      const filteredUpdates = updates.filter((notification) => {
        // Check if this exact notification was dismissed
        if (dismissedNotifications.has(notification.id)) {
          return false;
        }

        // Check if we dismissed an older version for this browser
        const dismissedForBrowser = Array.from(dismissedNotifications).find(
          (dismissedId) => {
            const parts = dismissedId.split("_");
            if (parts.length >= 2) {
              const browser = parts[0];
              return browser === notification.browser;
            }
            return false;
          },
        );

        if (dismissedForBrowser) {
          // Extract the dismissed version to compare
          const dismissedParts = dismissedForBrowser.split("_to_");
          if (dismissedParts.length === 2) {
            const dismissedToVersion = dismissedParts[1];
            // Only show if this is a newer version than what was dismissed
            return notification.new_version !== dismissedToVersion;
          }
        }

        return true;
      });

      setNotifications(filteredUpdates);

      // Show toasts for new notifications - we'll define handleUpdate and handleDismiss separately
      // to avoid circular dependencies
    } catch (error) {
      console.error("Failed to check for updates:", error);
    }
  }, [dismissedNotifications]);

  const handleUpdate = useCallback(
    async (browser: string, newVersion: string) => {
      try {
        setUpdatingBrowsers((prev) => new Set(prev).add(browser));
        const browserDisplayName = getBrowserDisplayName(browser);

        // Dismiss all notifications for this browser first
        const browserNotifications = notifications.filter(
          (n) => n.browser === browser,
        );
        for (const notification of browserNotifications) {
          toast.dismiss(notification.id);
          await invoke("dismiss_update_notification", {
            notificationId: notification.id,
          });
        }

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
            // Mark download as auto-update in the backend for toast suppression
            await invoke("mark_auto_update_download", {
              browser,
              version: newVersion,
            });

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
              type: "success",
              title: `${browserDisplayName} update completed`,
              description: `${profileText} to version ${newVersion}. Running profiles were not updated and can be updated manually.`,
              duration: 5000,
            });
          } else {
            showToast({
              type: "success",
              title: `${browserDisplayName} update ready`,
              description:
                "All affected profiles are currently running. Stop them and manually update their versions to use the new version.",
              duration: 5000,
            });
          }

          // Trigger profile refresh to update UI with new versions
          if (onProfilesUpdated) {
            void onProfilesUpdated();
          }
        } catch (downloadError) {
          console.error("Failed to download browser:", downloadError);

          // Clean up auto-update tracking on error
          try {
            await invoke("remove_auto_update_download", {
              browser,
              version: newVersion,
            });
          } catch (e) {
            console.error("Failed to clean up auto-update tracking:", e);
          }

          showToast({
            type: "error",
            title: `Failed to download ${browserDisplayName} ${newVersion}`,
            description: String(downloadError),
            duration: 6000,
          });
          throw downloadError;
        }

        // Refresh notifications to clear any remaining ones
        await checkForUpdates();
      } catch (error) {
        console.error("Failed to start update:", error);
        const browserDisplayName = getBrowserDisplayName(browser);
        showToast({
          type: "error",
          title: `Failed to update ${browserDisplayName}`,
          description: String(error),
          duration: 6000,
        });
      } finally {
        setUpdatingBrowsers((prev) => {
          const next = new Set(prev);
          next.delete(browser);
          return next;
        });
      }
    },
    [notifications, checkForUpdates, onProfilesUpdated],
  );

  const handleDismiss = useCallback(
    async (notificationId: string) => {
      try {
        toast.dismiss(notificationId);
        await invoke("dismiss_update_notification", { notificationId });

        // Track this notification as dismissed to prevent showing it again
        setDismissedNotifications((prev) => new Set(prev).add(notificationId));

        await checkForUpdates();
      } catch (error) {
        console.error("Failed to dismiss notification:", error);
      }
    },
    [checkForUpdates],
  );

  // Separate effect to show toasts when notifications change
  useEffect(() => {
    for (const notification of notifications) {
      const isUpdating = updatingBrowsers.has(notification.browser);

      toast.custom(
        () => (
          <UpdateNotificationComponent
            notification={notification}
            onUpdate={handleUpdate}
            onDismiss={handleDismiss}
            isUpdating={isUpdating}
          />
        ),
        {
          id: notification.id,
          duration: Number.POSITIVE_INFINITY, // Persistent until user action
          position: "top-right",
          // Remove transparent styling to fix background issue
          style: undefined,
        },
      );
    }
  }, [notifications, updatingBrowsers, handleUpdate, handleDismiss]);

  return {
    notifications,
    checkForUpdates,
    isUpdating: (browser: string) => updatingBrowsers.has(browser),
  };
}

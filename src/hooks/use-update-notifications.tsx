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
}

export function useUpdateNotifications() {
  const [notifications, setNotifications] = useState<UpdateNotification[]>([]);
  const [updatingBrowsers, setUpdatingBrowsers] = useState<Set<string>>(
    new Set(),
  );
  const [isClient, setIsClient] = useState(false);

  // Ensure we're on the client side to prevent hydration mismatches
  useEffect(() => {
    setIsClient(true);
  }, []);

  const checkForUpdates = useCallback(async () => {
    if (!isClient) return; // Only run on client side

    try {
      const updates = await invoke<UpdateNotification[]>(
        "check_for_browser_updates",
      );
      setNotifications(updates);

      // Show toasts for new notifications - we'll define handleUpdate and handleDismiss separately
      // to avoid circular dependencies
    } catch (error) {
      console.error("Failed to check for updates:", error);
    }
  }, [isClient]);

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
    [notifications, checkForUpdates],
  );

  const handleDismiss = useCallback(
    async (notificationId: string) => {
      if (!isClient) return; // Only run on client side

      try {
        toast.dismiss(notificationId);
        await invoke("dismiss_update_notification", { notificationId });
        await checkForUpdates();
      } catch (error) {
        console.error("Failed to dismiss notification:", error);
      }
    },
    [checkForUpdates, isClient],
  );

  // Separate effect to show toasts when notifications change
  useEffect(() => {
    if (!isClient) return;

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
  }, [notifications, updatingBrowsers, handleUpdate, handleDismiss, isClient]);

  return {
    notifications,
    checkForUpdates,
    isUpdating: (browser: string) => updatingBrowsers.has(browser),
  };
}

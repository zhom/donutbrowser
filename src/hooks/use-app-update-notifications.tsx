"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { AppUpdateToast } from "@/components/app-update-toast";
import { showToast } from "@/lib/toast-utils";
import type { AppUpdateInfo } from "@/types";

export function useAppUpdateNotifications() {
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [isUpdating, setIsUpdating] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<string>("");
  const [isClient, setIsClient] = useState(false);
  const [dismissedVersion, setDismissedVersion] = useState<string | null>(null);

  // Ensure we're on the client side to prevent hydration mismatches
  useEffect(() => {
    setIsClient(true);
  }, []);

  const checkForAppUpdates = useCallback(async () => {
    if (!isClient) return;

    try {
      const update = await invoke<AppUpdateInfo | null>(
        "check_for_app_updates",
      );

      // Don't show update if this version was already dismissed
      if (update && update.new_version !== dismissedVersion) {
        setUpdateInfo(update);
      } else if (update) {
        console.log("Update available but dismissed:", update.new_version);
      }
    } catch (error) {
      console.error("Failed to check for app updates:", error);
    }
  }, [isClient, dismissedVersion]);

  const checkForAppUpdatesManual = useCallback(async () => {
    if (!isClient) return;

    try {
      console.log("Triggering manual app update check...");
      const update = await invoke<AppUpdateInfo | null>(
        "check_for_app_updates_manual",
      );
      console.log("Manual check result:", update);

      // Always show manual check results, even if previously dismissed
      setUpdateInfo(update);
    } catch (error) {
      console.error("Failed to manually check for app updates:", error);
    }
  }, [isClient]);

  const handleAppUpdate = useCallback(async (appUpdateInfo: AppUpdateInfo) => {
    try {
      setIsUpdating(true);
      setUpdateProgress("Starting update...");

      await invoke("download_and_install_app_update", {
        updateInfo: appUpdateInfo,
      });
    } catch (error) {
      console.error("Failed to update app:", error);
      showToast({
        type: "error",
        title: "Failed to update Donut Browser",
        description: String(error),
        duration: 6000,
      });
      setIsUpdating(false);
      setUpdateProgress("");
    }
  }, []);

  const dismissAppUpdate = useCallback(() => {
    if (!isClient) return;

    // Remember the dismissed version so we don't show it again
    if (updateInfo) {
      setDismissedVersion(updateInfo.new_version);
      console.log("Dismissed app update version:", updateInfo.new_version);
    }

    setUpdateInfo(null);
    toast.dismiss("app-update");
  }, [isClient, updateInfo]);

  // Listen for app update availability
  useEffect(() => {
    if (!isClient) return;

    const unlistenUpdate = listen<AppUpdateInfo>(
      "app-update-available",
      (event) => {
        console.log("App update available:", event.payload);
        setUpdateInfo(event.payload);
      },
    );

    const unlistenProgress = listen<string>("app-update-progress", (event) => {
      console.log("App update progress:", event.payload);
      setUpdateProgress(event.payload);
    });

    return () => {
      void unlistenUpdate.then((unlisten) => {
        unlisten();
      });
      void unlistenProgress.then((unlisten) => {
        unlisten();
      });
    };
  }, [isClient]);

  // Show toast when update is available
  useEffect(() => {
    if (!isClient || !updateInfo) return;

    toast.custom(
      () => (
        <AppUpdateToast
          updateInfo={updateInfo}
          onUpdate={handleAppUpdate}
          onDismiss={dismissAppUpdate}
          isUpdating={isUpdating}
          updateProgress={updateProgress}
        />
      ),
      {
        id: "app-update",
        duration: Number.POSITIVE_INFINITY, // Persistent until user action
        position: "top-left",
        style: {
          zIndex: 99999, // Ensure app updates appear above dialogs
          pointerEvents: "auto", // Ensure app updates remain interactive
        },
      },
    );
  }, [
    updateInfo,
    handleAppUpdate,
    dismissAppUpdate,
    isUpdating,
    updateProgress,
    isClient,
  ]);

  // Check for app updates on startup
  useEffect(() => {
    if (!isClient) return;

    // Check for updates immediately on startup
    void checkForAppUpdates();
  }, [isClient, checkForAppUpdates]);

  return {
    updateInfo,
    isUpdating,
    checkForAppUpdates,
    checkForAppUpdatesManual,
    dismissAppUpdate,
  };
}

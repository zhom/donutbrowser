"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";
import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile, SyncSettings } from "@/types";

interface ProfileSyncDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  onSyncConfigOpen: () => void;
}

export function ProfileSyncDialog({
  isOpen,
  onClose,
  profile,
  onSyncConfigOpen,
}: ProfileSyncDialogProps) {
  const [isSaving, setIsSaving] = useState(false);
  const [isSyncing, setIsSyncing] = useState(false);
  const [syncEnabled, setSyncEnabled] = useState(
    profile?.sync_enabled ?? false,
  );
  const [hasConfig, setHasConfig] = useState(false);
  const [isCheckingConfig, setIsCheckingConfig] = useState(false);

  const checkSyncConfig = useCallback(async () => {
    setIsCheckingConfig(true);
    try {
      const settings = await invoke<SyncSettings>("get_sync_settings");
      setHasConfig(Boolean(settings.sync_server_url && settings.sync_token));
    } catch {
      setHasConfig(false);
    } finally {
      setIsCheckingConfig(false);
    }
  }, []);

  const handleOpenChange = useCallback(
    (open: boolean) => {
      if (open && profile) {
        setSyncEnabled(profile.sync_enabled ?? false);
        void checkSyncConfig();
      }
      if (!open) {
        onClose();
      }
    },
    [profile, onClose, checkSyncConfig],
  );

  const handleToggleSync = useCallback(async () => {
    if (!profile) return;

    if (!hasConfig) {
      showErrorToast("Please configure sync service first");
      onSyncConfigOpen();
      onClose();
      return;
    }

    setIsSaving(true);
    try {
      await invoke("set_profile_sync_enabled", {
        profileId: profile.id,
        enabled: !syncEnabled,
      });
      setSyncEnabled(!syncEnabled);
      showSuccessToast(
        !syncEnabled ? "Sync enabled - syncing now..." : "Sync disabled",
      );
    } catch (error) {
      console.error("Failed to toggle sync:", error);
      showErrorToast("Failed to update sync settings");
    } finally {
      setIsSaving(false);
    }
  }, [profile, syncEnabled, hasConfig, onSyncConfigOpen, onClose]);

  const handleSyncNow = useCallback(async () => {
    if (!profile) return;

    if (!hasConfig) {
      showErrorToast("Please configure sync service first");
      onSyncConfigOpen();
      onClose();
      return;
    }

    setIsSyncing(true);
    try {
      await invoke("request_profile_sync", { profileId: profile.id });
      showSuccessToast("Sync queued");
    } catch (error) {
      console.error("Failed to queue sync:", error);
      showErrorToast("Failed to queue sync");
    } finally {
      setIsSyncing(false);
    }
  }, [profile, hasConfig, onSyncConfigOpen, onClose]);

  const formatLastSync = (timestamp?: number) => {
    if (!timestamp) return "Never";
    const date = new Date(timestamp * 1000);
    return date.toLocaleString();
  };

  if (!profile) return null;

  return (
    <Dialog open={isOpen} onOpenChange={handleOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Profile Sync</DialogTitle>
          <DialogDescription>
            Manage sync settings for &quot;{profile.name}&quot;
          </DialogDescription>
        </DialogHeader>

        {isCheckingConfig ? (
          <div className="flex justify-center py-8">
            <div className="w-6 h-6 rounded-full border-2 border-current animate-spin border-t-transparent" />
          </div>
        ) : (
          <div className="grid gap-4 py-4">
            {!hasConfig && (
              <div className="p-3 text-sm rounded-md bg-muted">
                <p className="mb-2">Sync service not configured.</p>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    onSyncConfigOpen();
                    onClose();
                  }}
                >
                  Configure Sync Service
                </Button>
              </div>
            )}

            {hasConfig && (
              <>
                <div className="flex justify-between items-center">
                  <div className="space-y-0.5">
                    <Label htmlFor="sync-enabled">Sync Enabled</Label>
                    <p className="text-sm text-muted-foreground">
                      Sync this profile across devices
                    </p>
                  </div>
                  <Checkbox
                    id="sync-enabled"
                    checked={syncEnabled}
                    onCheckedChange={handleToggleSync}
                    disabled={isSaving}
                  />
                </div>

                <div className="space-y-2">
                  <Label>Last Synced</Label>
                  <div className="flex gap-2 items-center">
                    <Badge variant="outline">
                      {formatLastSync(profile.last_sync)}
                    </Badge>
                    {syncEnabled && (
                      <Badge
                        variant={profile.last_sync ? "default" : "secondary"}
                      >
                        {profile.last_sync ? "Synced" : "Pending"}
                      </Badge>
                    )}
                  </div>
                </div>
              </>
            )}
          </div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
          {hasConfig && syncEnabled && (
            <LoadingButton onClick={handleSyncNow} isLoading={isSyncing}>
              Sync Now
            </LoadingButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

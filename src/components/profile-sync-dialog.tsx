"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile, SyncMode, SyncSettings } from "@/types";
import { isSyncEnabled } from "@/types";

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
  const { t } = useTranslation();
  const { user: cloudUser } = useCloudAuth();
  const isCloudSyncEligible =
    cloudUser != null &&
    cloudUser.plan !== "free" &&
    (cloudUser.subscriptionStatus === "active" ||
      cloudUser.planPeriod === "lifetime");
  const [isSaving, setIsSaving] = useState(false);
  const [isSyncing, setIsSyncing] = useState(false);
  const [syncMode, setSyncMode] = useState<SyncMode>(
    profile?.sync_mode ?? "Disabled",
  );
  const [hasSelfHostedConfig, setHasSelfHostedConfig] = useState(false);
  const [hasE2ePassword, setHasE2ePassword] = useState(false);
  const [isCheckingConfig, setIsCheckingConfig] = useState(false);

  const hasConfig = isCloudSyncEligible || hasSelfHostedConfig;

  const checkSyncConfig = useCallback(async () => {
    setIsCheckingConfig(true);
    try {
      const settings = await invoke<SyncSettings>("get_sync_settings");
      setHasSelfHostedConfig(
        Boolean(settings.sync_server_url && settings.sync_token),
      );
      const hasPassword = await invoke<boolean>("check_has_e2e_password");
      setHasE2ePassword(hasPassword);
    } catch {
      setHasSelfHostedConfig(false);
    } finally {
      setIsCheckingConfig(false);
    }
  }, []);

  const handleOpenChange = useCallback(
    (open: boolean) => {
      if (open && profile) {
        setSyncMode(profile.sync_mode ?? "Disabled");
        void checkSyncConfig();
      }
      if (!open) {
        onClose();
      }
    },
    [profile, onClose, checkSyncConfig],
  );

  const handleModeChange = useCallback(
    async (newMode: string) => {
      if (!profile) return;

      if (!hasConfig) {
        showErrorToast(t("sync.mode.noPasswordWarning"));
        onSyncConfigOpen();
        onClose();
        return;
      }

      if (newMode === "Encrypted" && !hasE2ePassword) {
        showErrorToast(t("sync.mode.passwordRequired"));
        return;
      }

      setIsSaving(true);
      try {
        await invoke("set_profile_sync_mode", {
          profileId: profile.id,
          syncMode: newMode,
        });
        setSyncMode(newMode as SyncMode);
        showSuccessToast(
          newMode !== "Disabled"
            ? t("sync.mode.enabledToast")
            : t("sync.mode.disabledToast"),
        );
      } catch (error) {
        console.error("Failed to set sync mode:", error);
        showErrorToast(String(error));
      } finally {
        setIsSaving(false);
      }
    },
    [profile, hasConfig, hasE2ePassword, onSyncConfigOpen, onClose, t],
  );

  const handleSyncNow = useCallback(async () => {
    if (!profile) return;

    if (!hasConfig) {
      showErrorToast(t("sync.mode.noPasswordWarning"));
      onSyncConfigOpen();
      onClose();
      return;
    }

    setIsSyncing(true);
    try {
      await invoke("request_profile_sync", { profileId: profile.id });
      showSuccessToast(t("sync.mode.syncQueued"));
    } catch (error) {
      console.error("Failed to queue sync:", error);
      showErrorToast(String(error));
    } finally {
      setIsSyncing(false);
    }
  }, [profile, hasConfig, onSyncConfigOpen, onClose, t]);

  const formatLastSync = (timestamp?: number) => {
    if (!timestamp) return t("common.labels.never", "Never");
    const date = new Date(timestamp * 1000);
    return date.toLocaleString();
  };

  if (!profile) return null;

  return (
    <Dialog open={isOpen} onOpenChange={handleOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("sync.mode.title", "Profile Sync")}</DialogTitle>
          <DialogDescription>
            {t("sync.mode.description", {
              name: profile.name,
              defaultValue: `Manage sync settings for "${profile.name}"`,
            })}
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
                <p className="mb-2">
                  {t("sync.mode.notConfigured", "Sync service not configured.")}
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    onSyncConfigOpen();
                    onClose();
                  }}
                >
                  {t("sync.mode.configureService", "Configure Sync Service")}
                </Button>
              </div>
            )}

            {hasConfig && (
              <>
                <RadioGroup
                  value={syncMode}
                  onValueChange={handleModeChange}
                  disabled={isSaving}
                  className="grid gap-3"
                >
                  <div className="flex items-start space-x-3">
                    <RadioGroupItem value="Disabled" id="sync-disabled" />
                    <Label htmlFor="sync-disabled" className="cursor-pointer">
                      <span className="font-medium">
                        {t("sync.mode.disabled", "Disabled")}
                      </span>
                      <p className="text-sm text-muted-foreground">
                        {t(
                          "sync.mode.disabledDescription",
                          "No sync for this profile",
                        )}
                      </p>
                    </Label>
                  </div>

                  <div className="flex items-start space-x-3">
                    <RadioGroupItem value="Regular" id="sync-regular" />
                    <Label htmlFor="sync-regular" className="cursor-pointer">
                      <span className="font-medium">
                        {t("sync.mode.regular", "Regular Sync")}
                      </span>
                      <p className="text-sm text-muted-foreground">
                        {t(
                          "sync.mode.regularDescription",
                          "Fast sync, unencrypted",
                        )}
                      </p>
                    </Label>
                  </div>

                  <div className="flex items-start space-x-3">
                    <RadioGroupItem value="Encrypted" id="sync-encrypted" />
                    <Label htmlFor="sync-encrypted" className="cursor-pointer">
                      <span className="font-medium">
                        {t("sync.mode.encrypted", "E2E Encrypted Sync")}
                      </span>
                      <p className="text-sm text-muted-foreground">
                        {t(
                          "sync.mode.encryptedDescription",
                          "Encrypted before upload. Server never sees plaintext data.",
                        )}
                      </p>
                    </Label>
                  </div>
                </RadioGroup>

                {syncMode === "Encrypted" && !hasE2ePassword && (
                  <div className="p-3 text-sm rounded-md bg-destructive/10 text-destructive">
                    {t(
                      "sync.mode.noPasswordWarning",
                      "E2E password not set. Please set a password in Settings.",
                    )}
                  </div>
                )}

                <div className="space-y-2">
                  <Label>{t("sync.mode.lastSynced", "Last Synced")}</Label>
                  <div className="flex gap-2 items-center">
                    <Badge variant="outline">
                      {formatLastSync(profile.last_sync)}
                    </Badge>
                    {isSyncEnabled(profile) && (
                      <Badge
                        variant={profile.last_sync ? "default" : "secondary"}
                      >
                        {profile.last_sync
                          ? t("common.status.synced")
                          : t("common.status.pending")}
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
            {t("common.buttons.close")}
          </Button>
          {hasConfig && isSyncEnabled(profile) && (
            <LoadingButton onClick={handleSyncNow} isLoading={isSyncing}>
              {t("sync.mode.syncNow", "Sync Now")}
            </LoadingButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

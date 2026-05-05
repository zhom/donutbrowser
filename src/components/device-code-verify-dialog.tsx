"use client";

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

interface DeviceCodeVerifyDialogProps {
  isOpen: boolean;
  onClose: (loginOccurred?: boolean) => void;
}

/**
 * Dedicated dialog for pasting and verifying the cloud device-link code.
 * Opens after the user clicks "Login" in the sync config dialog so the
 * verify step is a focused step on its own — and so it doesn't visually
 * stack with other dialogs (e.g. the profile selector triggered by a
 * deep link) sharing the same view.
 */
export function DeviceCodeVerifyDialog({
  isOpen,
  onClose,
}: DeviceCodeVerifyDialogProps) {
  const { t } = useTranslation();
  const { exchangeDeviceCode } = useCloudAuth();
  const [linkCode, setLinkCode] = useState("");
  const [isVerifying, setIsVerifying] = useState(false);

  // Reset the field when the dialog reopens so a stale code from a
  // previous attempt doesn't auto-populate.
  useEffect(() => {
    if (isOpen) {
      setLinkCode("");
    }
  }, [isOpen]);

  const handleVerify = async () => {
    const trimmed = linkCode.trim();
    if (!trimmed) return;
    setIsVerifying(true);
    try {
      await exchangeDeviceCode(trimmed);
      showSuccessToast(t("sync.cloud.loginSuccess"));
      try {
        await invoke("restart_sync_service");
      } catch (e) {
        console.error("Failed to restart sync service:", e);
      }
      onClose(true);
    } catch (error) {
      console.error("Device-code exchange failed:", error);
      showErrorToast(String(error));
    } finally {
      setIsVerifying(false);
    }
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose(false);
      }}
    >
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("sync.cloud.verifyAndLogin")}</DialogTitle>
          <DialogDescription>
            {t("sync.cloud.deviceLinkInstructions")}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-4 py-4">
          <div className="space-y-2">
            <Label htmlFor="device-link-code">
              {t("sync.cloud.linkCodeLabel")}
            </Label>
            <Input
              id="device-link-code"
              placeholder={t("sync.cloud.linkCodePlaceholder")}
              value={linkCode}
              onChange={(e) => {
                setLinkCode(e.target.value);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" && linkCode.trim()) {
                  void handleVerify();
                }
              }}
              autoComplete="off"
              spellCheck={false}
              autoFocus
            />
            <LoadingButton
              onClick={() => void handleVerify()}
              isLoading={isVerifying}
              disabled={!linkCode.trim()}
              className="w-full"
            >
              {isVerifying
                ? t("sync.cloud.loggingIn")
                : t("sync.cloud.verifyAndLogin")}
            </LoadingButton>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

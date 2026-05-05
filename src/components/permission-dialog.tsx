"use client";

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BsCamera, BsMic } from "react-icons/bs";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { PermissionType } from "@/hooks/use-permissions";
import { usePermissions } from "@/hooks/use-permissions";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { RippleButton } from "./ui/ripple";

interface PermissionDialogProps {
  isOpen: boolean;
  onClose: () => void;
  permissionType: PermissionType;
  /**
   * Fired when the displayed permission becomes granted. The just-granted
   * type is passed through so the parent can act optimistically — its own
   * usePermissions instance polls on a 5 s cadence and would otherwise be
   * stale right after the macOS system prompt is accepted, leaving the
   * dialog open in a confusing state.
   */
  onPermissionGranted?: (justGranted: PermissionType) => void;
}

export function PermissionDialog({
  isOpen,
  onClose,
  permissionType,
  onPermissionGranted,
}: PermissionDialogProps) {
  const { t } = useTranslation();
  const [isRequesting, setIsRequesting] = useState(false);
  const [isWaitingForGrant, setIsWaitingForGrant] = useState(false);
  const [isMacOS, setIsMacOS] = useState(false);
  const {
    requestPermission,
    isMicrophoneAccessGranted,
    isCameraAccessGranted,
  } = usePermissions();

  // Check if we're on macOS and close dialog if not
  useEffect(() => {
    const userAgent = navigator.userAgent;
    const isMac = userAgent.includes("Mac");
    setIsMacOS(isMac);

    // If not macOS, close the dialog as permissions aren't needed
    if (!isMac) {
      onClose();
    }
  }, [onClose]);

  // Get current permission status
  const isCurrentPermissionGranted =
    permissionType === "microphone"
      ? isMicrophoneAccessGranted
      : isCameraAccessGranted;

  // Mirror the latest permission state into a ref so the deferred timeout
  // callback can read it without being recreated on every state change.
  const isCurrentPermissionGrantedRef = useRef(isCurrentPermissionGranted);
  useEffect(() => {
    isCurrentPermissionGrantedRef.current = isCurrentPermissionGranted;
  }, [isCurrentPermissionGranted]);

  // When the permission becomes granted, fire a success toast and let the
  // parent decide what to do next (progress to the other permission, or close).
  // We deliberately do NOT keep the dialog around to show a "Done" state —
  // the toast is the confirmation, and the dialog closes immediately.
  // Use a ref to ensure we only fire the toast once per grant transition.
  const grantedToastFiredForRef = useRef<PermissionType | null>(null);
  useEffect(() => {
    if (!isOpen) {
      grantedToastFiredForRef.current = null;
      return;
    }
    if (
      isCurrentPermissionGranted &&
      grantedToastFiredForRef.current !== permissionType
    ) {
      grantedToastFiredForRef.current = permissionType;
      showSuccessToast(
        permissionType === "microphone"
          ? t("permissionDialog.grantedToastMicrophone")
          : t("permissionDialog.grantedToastCamera"),
      );
      onPermissionGranted?.(permissionType);
    }
  }, [
    isCurrentPermissionGranted,
    isOpen,
    onPermissionGranted,
    permissionType,
    t,
  ]);

  // Pending-grant timeout: triggered after the user clicks "Grant Access"
  // to give the macOS permission state a few seconds to propagate to our poll.
  const waitTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // If permission becomes granted during the wait window, end the wait early.
  useEffect(() => {
    if (isWaitingForGrant && isCurrentPermissionGranted) {
      if (waitTimeoutRef.current) {
        clearTimeout(waitTimeoutRef.current);
        waitTimeoutRef.current = null;
      }
      setIsWaitingForGrant(false);
    }
  }, [isWaitingForGrant, isCurrentPermissionGranted]);

  // Clear any pending timeout on unmount.
  useEffect(() => {
    return () => {
      if (waitTimeoutRef.current) {
        clearTimeout(waitTimeoutRef.current);
        waitTimeoutRef.current = null;
      }
    };
  }, []);

  const getPermissionIcon = (type: PermissionType) => {
    switch (type) {
      case "microphone":
        return <BsMic className="w-8 h-8" />;
      case "camera":
        return <BsCamera className="w-8 h-8" />;
    }
  };

  const getPermissionTitle = (type: PermissionType) => {
    switch (type) {
      case "microphone":
        return t("permissionDialog.titleMicrophone");
      case "camera":
        return t("permissionDialog.titleCamera");
    }
  };

  const getPermissionDescription = (type: PermissionType) => {
    switch (type) {
      case "microphone":
        return t("permissionDialog.descMicrophone");
      case "camera":
        return t("permissionDialog.descCamera");
    }
  };

  const handleRequestPermission = async () => {
    setIsRequesting(true);
    try {
      await requestPermission(permissionType);
      // The macOS permission poll runs every 5 s, so the new state can take
      // a moment to surface. Keep the grant button in its busy state for
      // that window so the user has clear feedback, and notify them if the
      // grant still hasn't landed by the end.
      setIsWaitingForGrant(true);
      if (waitTimeoutRef.current) {
        clearTimeout(waitTimeoutRef.current);
      }
      waitTimeoutRef.current = setTimeout(() => {
        waitTimeoutRef.current = null;
        setIsWaitingForGrant(false);
        if (!isCurrentPermissionGrantedRef.current) {
          showErrorToast(
            permissionType === "microphone"
              ? t("permissionDialog.stillNotGrantedMicrophone")
              : t("permissionDialog.stillNotGrantedCamera"),
          );
        }
      }, 5000);
    } catch (error) {
      console.error("Failed to request permission:", error);
      showErrorToast(t("permissionDialog.requestFailed"));
    } finally {
      setIsRequesting(false);
    }
  };

  // Don't render if not macOS
  if (!isMacOS) {
    return null;
  }

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader className="text-center">
          <div className="flex justify-center items-center mx-auto mb-4 w-16 h-16 bg-primary/15 rounded-full">
            {getPermissionIcon(permissionType)}
          </div>
          <DialogTitle className="text-xl">
            {getPermissionTitle(permissionType)}
          </DialogTitle>
          <DialogDescription className="text-base">
            {getPermissionDescription(permissionType)}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {!isCurrentPermissionGranted && (
            <div className="p-3 bg-warning/10 rounded-lg">
              <p className="text-sm text-warning">
                {permissionType === "microphone"
                  ? t("permissionDialog.notGrantedMicrophone")
                  : t("permissionDialog.notGrantedCamera")}
              </p>
            </div>
          )}
        </div>

        <DialogFooter className="gap-2">
          <RippleButton
            variant="outline"
            onClick={onClose}
            className="min-w-24"
          >
            {t("permissionDialog.cancelButton")}
          </RippleButton>

          {!isCurrentPermissionGranted && (
            <LoadingButton
              isLoading={isRequesting || isWaitingForGrant}
              onClick={() => {
                handleRequestPermission().catch((err: unknown) => {
                  console.error(err);
                });
              }}
              className="min-w-24"
            >
              {t("permissionDialog.grantAccessButton")}
            </LoadingButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

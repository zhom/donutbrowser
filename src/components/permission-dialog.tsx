"use client";

import { useEffect, useState } from "react";
import { BsCamera, BsMic } from "react-icons/bs";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
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

interface PermissionDialogProps {
  isOpen: boolean;
  onClose: () => void;
  permissionType: PermissionType;
  onPermissionGranted?: () => void;
}

export function PermissionDialog({
  isOpen,
  onClose,
  permissionType,
  onPermissionGranted,
}: PermissionDialogProps) {
  const [isRequesting, setIsRequesting] = useState(false);
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

  // Auto-close dialog when permission is granted
  useEffect(() => {
    if (isCurrentPermissionGranted && isOpen) {
      onPermissionGranted?.();
    }
  }, [isCurrentPermissionGranted, isOpen, onPermissionGranted]);

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
        return "Microphone Access Required";
      case "camera":
        return "Camera Access Required";
    }
  };

  const getPermissionDescription = (type: PermissionType) => {
    switch (type) {
      case "microphone":
        return "Donut Browser needs access to your microphone to enable microphone functionality in web browsers. Each website that wants to use your microphone will still ask for your permission individually.";
      case "camera":
        return "Donut Browser needs access to your camera to enable camera functionality in web browsers. Each website that wants to use your camera will still ask for your permission individually.";
    }
  };

  const handleRequestPermission = async () => {
    setIsRequesting(true);
    try {
      await requestPermission(permissionType);
      showSuccessToast(
        `${getPermissionTitle(permissionType).replace(
          " Required",
          "",
        )} permission requested`,
      );
    } catch (error) {
      console.error("Failed to request permission:", error);
      showErrorToast("Failed to request permission");
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
          <div className="flex justify-center items-center mx-auto mb-4 w-16 h-16 bg-blue-100 rounded-full dark:bg-blue-900">
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
          {isCurrentPermissionGranted && (
            <div className="p-3 bg-green-50 rounded-lg dark:bg-green-900/20">
              <p className="text-sm text-green-800 dark:text-green-200">
                ✅ Permission granted! Browsers launched from Donut Browser can
                now access your {permissionType}.
              </p>
            </div>
          )}

          {!isCurrentPermissionGranted && (
            <div className="p-3 bg-amber-50 rounded-lg dark:bg-amber-900/20">
              <p className="text-sm text-amber-800 dark:text-amber-200">
                ⚠️ Permission not granted. Click the button below to request
                access to your {permissionType}.
              </p>
            </div>
          )}
        </div>

        <DialogFooter className="gap-2">
          <Button variant="outline" onClick={onClose}>
            {isCurrentPermissionGranted ? "Done" : "Cancel"}
          </Button>

          {!isCurrentPermissionGranted && (
            <LoadingButton
              isLoading={isRequesting}
              onClick={() => {
                handleRequestPermission().catch(console.error);
              }}
              className="min-w-24"
            >
              Grant Access
            </LoadingButton>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

interface LaunchOnLoginDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function LaunchOnLoginDialog({
  isOpen,
  onClose,
}: LaunchOnLoginDialogProps) {
  const [isEnabling, setIsEnabling] = useState(false);
  const [isDeclining, setIsDeclining] = useState(false);

  const handleEnable = useCallback(async () => {
    setIsEnabling(true);
    try {
      await invoke("enable_launch_on_login");
      showSuccessToast("Launch on login enabled");
      onClose();
    } catch (error) {
      console.error("Failed to enable launch on login:", error);
      showErrorToast("Failed to enable launch on login", {
        description:
          error instanceof Error ? error.message : "Please try again",
      });
    } finally {
      setIsEnabling(false);
    }
  }, [onClose]);

  const handleDecline = useCallback(async () => {
    setIsDeclining(true);
    try {
      await invoke("decline_launch_on_login");
      onClose();
    } catch (error) {
      console.error("Failed to decline launch on login:", error);
      showErrorToast("Failed to save preference", {
        description:
          error instanceof Error ? error.message : "Please try again",
      });
    } finally {
      setIsDeclining(false);
    }
  }, [onClose]);

  return (
    <Dialog open={isOpen}>
      <DialogContent
        className="sm:max-w-sm"
        onEscapeKeyDown={(e) => e.preventDefault()}
        onPointerDownOutside={(e) => e.preventDefault()}
        onInteractOutside={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>Enable Launch on Login?</DialogTitle>
        </DialogHeader>

        <p className="text-sm text-muted-foreground">
          Running in the background helps keep your proxies and browsers alive.
        </p>

        <DialogFooter className="flex-row justify-between sm:justify-between">
          <Button
            variant="ghost"
            onClick={handleDecline}
            disabled={isEnabling || isDeclining}
          >
            {isDeclining ? "..." : "Don't Ask Again"}
          </Button>
          <LoadingButton
            onClick={handleEnable}
            isLoading={isEnabling}
            disabled={isDeclining}
          >
            Enable
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { showErrorToast } from "@/lib/toast-utils";

interface CommercialTrialModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CommercialTrialModal({
  isOpen,
  onClose,
}: CommercialTrialModalProps) {
  const [isAcknowledging, setIsAcknowledging] = useState(false);

  const handleAcknowledge = useCallback(async () => {
    setIsAcknowledging(true);
    try {
      await invoke("acknowledge_trial_expiration");
      onClose();
    } catch (error) {
      console.error("Failed to acknowledge trial expiration:", error);
      showErrorToast("Failed to save acknowledgment", {
        description:
          error instanceof Error ? error.message : "Please try again",
      });
    } finally {
      setIsAcknowledging(false);
    }
  }, [onClose]);

  return (
    <Dialog open={isOpen}>
      <DialogContent
        className="sm:max-w-md"
        onEscapeKeyDown={(e) => e.preventDefault()}
        onPointerDownOutside={(e) => e.preventDefault()}
        onInteractOutside={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>Commercial Trial Expired</DialogTitle>
          <DialogDescription>
            Your 2-week commercial trial period has ended.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <p className="text-sm text-muted-foreground">
            If you are using Donut Browser for business purposes, you need to
            purchase a commercial license to continue. You can still use it for
            personal use for free.
          </p>
        </div>

        <DialogFooter>
          <LoadingButton
            onClick={handleAcknowledge}
            isLoading={isAcknowledging}
          >
            I Understand
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

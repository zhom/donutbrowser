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
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

interface WayfernTermsDialogProps {
  isOpen: boolean;
  onAccepted: () => void;
}

export function WayfernTermsDialog({
  isOpen,
  onAccepted,
}: WayfernTermsDialogProps) {
  const [isAccepting, setIsAccepting] = useState(false);

  const handleAccept = useCallback(async () => {
    setIsAccepting(true);
    try {
      await invoke("accept_wayfern_terms");
      showSuccessToast("Terms accepted successfully");
      onAccepted();
    } catch (error) {
      console.error("Failed to accept terms:", error);
      showErrorToast("Failed to accept terms", {
        description:
          error instanceof Error ? error.message : "Please try again",
      });
    } finally {
      setIsAccepting(false);
    }
  }, [onAccepted]);

  return (
    <Dialog open={isOpen}>
      <DialogContent
        className="sm:max-w-lg"
        onEscapeKeyDown={(e) => e.preventDefault()}
        onPointerDownOutside={(e) => e.preventDefault()}
        onInteractOutside={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>Wayfern Terms and Conditions</DialogTitle>
          <DialogDescription>
            Before using Donut Browser, you must read and agree to Wayfern's
            Terms and Conditions.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <p className="text-sm text-muted-foreground">
            Please review the Terms and Conditions at:
          </p>
          <a
            href="https://wayfern.com/terms-and-conditions"
            target="_blank"
            rel="noopener noreferrer"
            className="text-primary hover:underline text-sm font-medium block"
          >
            https://wayfern.com/terms-and-conditions
          </a>
          <p className="text-sm text-muted-foreground">
            By clicking "I Accept", you agree to be bound by these terms.
          </p>
        </div>

        <DialogFooter>
          <LoadingButton onClick={handleAccept} isLoading={isAccepting}>
            I Accept
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation();
  const [isAccepting, setIsAccepting] = useState(false);

  const handleAccept = useCallback(async () => {
    setIsAccepting(true);
    try {
      await invoke("accept_wayfern_terms");
      showSuccessToast(t("wayfernTerms.acceptSuccess"));
      onAccepted();
    } catch (error) {
      console.error("Failed to accept terms:", error);
      showErrorToast(t("wayfernTerms.acceptFailed"), {
        description:
          error instanceof Error ? error.message : t("wayfernTerms.tryAgain"),
      });
    } finally {
      setIsAccepting(false);
    }
  }, [onAccepted, t]);

  return (
    <Dialog open={isOpen}>
      <DialogContent
        className="sm:max-w-lg"
        onEscapeKeyDown={(e) => {
          e.preventDefault();
        }}
        onPointerDownOutside={(e) => {
          e.preventDefault();
        }}
        onInteractOutside={(e) => {
          e.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle>{t("wayfernTerms.title")}</DialogTitle>
          <DialogDescription>{t("wayfernTerms.description")}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <p className="text-sm text-muted-foreground">
            {t("wayfernTerms.reviewLabel")}
          </p>
          <a
            href="https://wayfern.com/tos"
            target="_blank"
            rel="noopener noreferrer"
            className="text-primary hover:underline text-sm font-medium block"
          >
            https://wayfern.com/tos
          </a>
          <p className="text-sm text-muted-foreground">
            {t("wayfernTerms.agreeNotice")}
          </p>
        </div>

        <DialogFooter>
          <LoadingButton onClick={handleAccept} isLoading={isAccepting}>
            {t("wayfernTerms.acceptButton")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

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
import { showErrorToast } from "@/lib/toast-utils";

interface CommercialTrialModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CommercialTrialModal({
  isOpen,
  onClose,
}: CommercialTrialModalProps) {
  const { t } = useTranslation();
  const [isAcknowledging, setIsAcknowledging] = useState(false);

  const handleAcknowledge = useCallback(async () => {
    setIsAcknowledging(true);
    try {
      await invoke("acknowledge_trial_expiration");
      onClose();
    } catch (error) {
      console.error("Failed to acknowledge trial expiration:", error);
      showErrorToast(t("commercialTrial.failed"), {
        description:
          error instanceof Error
            ? error.message
            : t("commercialTrial.tryAgain"),
      });
    } finally {
      setIsAcknowledging(false);
    }
  }, [onClose, t]);

  return (
    <Dialog open={isOpen}>
      <DialogContent
        className="sm:max-w-md"
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
          <DialogTitle>{t("commercialTrial.title")}</DialogTitle>
          <DialogDescription>
            {t("commercialTrial.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <p className="text-sm text-muted-foreground">
            {t("commercialTrial.body")}
          </p>
        </div>

        <DialogFooter>
          <LoadingButton
            onClick={handleAcknowledge}
            isLoading={isAcknowledging}
          >
            {t("commercialTrial.understandButton")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

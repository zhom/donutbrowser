"use client";

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";

interface WindowResizeWarningDialogProps {
  isOpen: boolean;
  onResult: (proceed: boolean) => void;
  browserType?: string;
}

export function WindowResizeWarningDialog({
  isOpen,
  onResult,
  browserType,
}: WindowResizeWarningDialogProps) {
  const { t } = useTranslation();
  const [dontShowAgain, setDontShowAgain] = useState(false);

  useEffect(() => {
    if (isOpen) {
      setDontShowAgain(false);
    }
  }, [isOpen]);

  const handleContinue = async () => {
    if (dontShowAgain) {
      try {
        await invoke("dismiss_window_resize_warning");
      } catch (error) {
        console.error("Failed to dismiss window resize warning:", error);
      }
    }
    onResult(true);
  };

  const handleCancel = () => {
    onResult(false);
  };

  const isCamoufox = browserType === "camoufox";

  const title = isCamoufox
    ? t("warnings.windowResizeCamoufoxTitle")
    : t("warnings.windowResizeTitle");

  const description = isCamoufox
    ? t("warnings.windowResizeCamoufoxDescription")
    : t("warnings.windowResizeDescription");

  return (
    <Dialog open={isOpen}>
      <DialogContent
        className="sm:max-w-sm"
        onEscapeKeyDown={(e) => e.preventDefault()}
        onPointerDownOutside={(e) => e.preventDefault()}
        onInteractOutside={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>

        <p className="text-sm text-muted-foreground">{description}</p>

        <div className="flex items-center space-x-2">
          <Checkbox
            id="dont-show-again"
            checked={dontShowAgain}
            onCheckedChange={(checked) => setDontShowAgain(checked === true)}
          />
          <Label htmlFor="dont-show-again" className="text-sm">
            {t("warnings.dontShowAgain")}
          </Label>
        </div>

        <DialogFooter className="flex-row justify-between sm:justify-between">
          <Button variant="ghost" onClick={handleCancel}>
            {t("warnings.cancel")}
          </Button>
          <Button onClick={handleContinue}>{t("warnings.continue")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

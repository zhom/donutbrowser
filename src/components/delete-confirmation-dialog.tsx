"use client";

import { useTranslation } from "react-i18next";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { LoadingButton } from "./loading-button";
import { RippleButton } from "./ui/ripple";

interface DeleteConfirmationDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: () => void | Promise<void>;
  title: string;
  description: string;
  confirmButtonText?: string;
  isLoading?: boolean;
  profileIds?: string[];
  profiles?: { id: string; name: string }[];
}

export function DeleteConfirmationDialog({
  isOpen,
  onClose,
  onConfirm,
  title,
  description,
  confirmButtonText,
  isLoading = false,
  profileIds,
  profiles = [],
}: DeleteConfirmationDialogProps) {
  const { t } = useTranslation();
  const handleConfirm = async () => {
    await onConfirm();
  };

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
          {profileIds && profileIds.length > 0 && (
            <div className="mt-4">
              <p className="text-sm font-medium mb-2">
                {t("deleteDialog.profilesToDelete")}
              </p>
              <div className="bg-muted rounded-md p-3 max-h-32 overflow-y-auto">
                <ul className="space-y-1">
                  {profileIds.map((id) => {
                    const profile = profiles.find((p) => p.id === id);
                    const displayName = profile ? profile.name : id;
                    return (
                      <li key={id} className="text-sm text-muted-foreground">
                        • {displayName}
                      </li>
                    );
                  })}
                </ul>
              </div>
            </div>
          )}
        </DialogHeader>
        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={onClose}
            disabled={isLoading}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            variant="destructive"
            onClick={() => void handleConfirm()}
            isLoading={isLoading}
          >
            {confirmButtonText ?? t("common.buttons.delete")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

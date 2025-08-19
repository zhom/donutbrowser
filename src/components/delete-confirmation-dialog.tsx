"use client";

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
  confirmButtonText = "Delete",
  isLoading = false,
  profileIds,
  profiles = [],
}: DeleteConfirmationDialogProps) {
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
                Profiles to be deleted:
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
            Cancel
          </RippleButton>
          <LoadingButton
            variant="destructive"
            onClick={() => void handleConfirm()}
            isLoading={isLoading}
          >
            {confirmButtonText}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

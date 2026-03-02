"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { showErrorToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";
import { LoadingButton } from "./loading-button";
import { RippleButton } from "./ui/ripple";

interface CloneProfileDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  onCloneComplete?: () => void;
}

export function CloneProfileDialog({
  isOpen,
  onClose,
  profile,
  onCloneComplete,
}: CloneProfileDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = React.useState("");
  const [isLoading, setIsLoading] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (isOpen && profile) {
      const defaultName = `${profile.name} (Copy)`;
      setName(defaultName);
      setTimeout(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      }, 0);
    } else {
      setIsLoading(false);
    }
  }, [isOpen, profile]);

  if (!profile) return null;

  const handleClone = async () => {
    if (!name.trim() || isLoading) return;
    setIsLoading(true);
    try {
      await invoke<BrowserProfile>("clone_profile", {
        profileId: profile.id,
        name: name.trim(),
      });
      onClose();
      onCloneComplete?.();
    } catch (err: unknown) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      showErrorToast(`Failed to clone profile: ${errorMessage}`);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("profileInfo.clone.title")}</DialogTitle>
          <DialogDescription>
            {t("profileInfo.clone.description")}
          </DialogDescription>
        </DialogHeader>
        <Input
          ref={inputRef}
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void handleClone();
          }}
          placeholder={t("profileInfo.clone.namePlaceholder")}
          disabled={isLoading}
        />
        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={onClose}
            disabled={isLoading}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            onClick={() => void handleClone()}
            isLoading={isLoading}
            disabled={!name.trim()}
          >
            {t("profileInfo.clone.button")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

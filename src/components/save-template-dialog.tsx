"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
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
import { Input } from "@/components/ui/input";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile, ProfileTemplate } from "@/types";

/** Saves a profile's settings, without its name, note or device, as a template. */
export function SaveTemplateDialog({
  profile,
  onClose,
}: {
  profile: BrowserProfile | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = React.useState("");
  const [isSaving, setIsSaving] = React.useState(false);

  React.useEffect(() => {
    if (profile) setName(profile.name);
  }, [profile]);

  const save = async () => {
    if (!profile || !name.trim()) return;
    setIsSaving(true);
    try {
      const template = await invoke<ProfileTemplate>(
        "save_profile_template_from_profile",
        { profileId: profile.id, name: name.trim() },
      );
      showSuccessToast(t("templates.saved", { name: template.name }));
      onClose();
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog
      open={profile !== null}
      onOpenChange={(open) => {
        if (!open && !isSaving) onClose();
      }}
    >
      <DialogContent className="max-w-md" data-slot="save-template-dialog">
        <DialogHeader>
          <DialogTitle>{t("templates.saveCurrent")}</DialogTitle>
          <DialogDescription>
            {t("templates.saveFromProfileDescription")}
          </DialogDescription>
        </DialogHeader>
        <Input
          variant="soft"
          autoFocus
          value={name}
          maxLength={60}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void save();
            }
          }}
          placeholder={t("templates.namePlaceholder")}
          aria-label={t("templates.namePlaceholder")}
          disabled={isSaving}
        />
        <DialogFooter>
          <Button variant="ghost" onClick={onClose} disabled={isSaving}>
            {t("common.buttons.cancel")}
          </Button>
          <LoadingButton
            data-slot="save-template-confirm"
            isLoading={isSaving}
            disabled={!name.trim()}
            onClick={() => void save()}
          >
            {t("templates.save")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

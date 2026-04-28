"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { ProfileGroup } from "@/types";
import { RippleButton } from "./ui/ripple";

interface EditGroupDialogProps {
  isOpen: boolean;
  onClose: () => void;
  group: ProfileGroup | null;
  onGroupUpdated: (group: ProfileGroup) => void;
}

export function EditGroupDialog({
  isOpen,
  onClose,
  group,
  onGroupUpdated,
}: EditGroupDialogProps) {
  const { t } = useTranslation();
  const [groupName, setGroupName] = useState("");
  const [isUpdating, setIsUpdating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (group) {
      setGroupName(group.name);
    } else {
      setGroupName("");
    }
    setError(null);
  }, [group]);

  const handleUpdate = useCallback(async () => {
    if (!group || !groupName.trim()) return;

    setIsUpdating(true);
    setError(null);
    try {
      const updatedGroup = await invoke<ProfileGroup>("update_profile_group", {
        groupId: group.id,
        name: groupName.trim(),
      });

      toast.success(t("groups.updateSuccess"));
      onGroupUpdated(updatedGroup);
      onClose();
    } catch (err) {
      console.error("Failed to update group:", err);
      const errorMessage =
        err instanceof Error ? err.message : t("groups.updateFailed");
      setError(errorMessage);
      toast.error(errorMessage);
    } finally {
      setIsUpdating(false);
    }
  }, [group, groupName, onGroupUpdated, onClose, t]);

  const handleClose = useCallback(() => {
    setError(null);
    onClose();
  }, [onClose]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("groups.editTitle")}</DialogTitle>
          <DialogDescription>{t("groups.editDescription")}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="group-name">{t("groups.form.name")}</Label>
            <Input
              id="group-name"
              placeholder={t("groups.form.namePlaceholder")}
              value={groupName}
              onChange={(e) => {
                setGroupName(e.target.value);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" && groupName.trim()) {
                  void handleUpdate();
                }
              }}
              disabled={isUpdating}
            />
          </div>

          {error && (
            <div className="p-3 text-sm text-destructive bg-destructive/10 rounded-md">
              {error}
            </div>
          )}
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={handleClose}
            disabled={isUpdating}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            isLoading={isUpdating}
            onClick={() => void handleUpdate()}
            disabled={!groupName.trim() || groupName === group?.name}
          >
            {t("groups.edit")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

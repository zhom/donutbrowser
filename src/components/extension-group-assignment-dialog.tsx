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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { BrowserProfile, ExtensionGroup } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ExtensionGroupAssignmentDialogProps {
  isOpen: boolean;
  onClose: () => void;
  selectedProfiles: string[];
  onAssignmentComplete: () => void;
  profiles?: BrowserProfile[];
}

export function ExtensionGroupAssignmentDialog({
  isOpen,
  onClose,
  selectedProfiles,
  onAssignmentComplete,
  profiles = [],
}: ExtensionGroupAssignmentDialogProps) {
  const { t } = useTranslation();
  const [groups, setGroups] = useState<ExtensionGroup[]>([]);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isAssigning, setIsAssigning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadGroups = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const groupList = await invoke<ExtensionGroup[]>("list_extension_groups");
      setGroups(groupList);
    } catch (err) {
      console.error("Failed to load extension groups:", err);
      setError(
        err instanceof Error ? err.message : "Failed to load extension groups",
      );
    } finally {
      setIsLoading(false);
    }
  }, []);

  const handleAssign = useCallback(async () => {
    setIsAssigning(true);
    setError(null);
    try {
      for (const profileId of selectedProfiles) {
        await invoke("assign_extension_group_to_profile", {
          profileId,
          extensionGroupId: selectedGroupId,
        });
      }

      toast.success(t("extensions.assignSuccess"));
      onAssignmentComplete();
      onClose();
    } catch (err) {
      console.error("Failed to assign extension group:", err);
      const errorMessage =
        err instanceof Error ? err.message : "Failed to assign extension group";
      setError(errorMessage);
      toast.error(errorMessage);
    } finally {
      setIsAssigning(false);
    }
  }, [selectedProfiles, selectedGroupId, onAssignmentComplete, onClose, t]);

  useEffect(() => {
    if (isOpen) {
      void loadGroups();
      setSelectedGroupId(null);
      setError(null);
    }
  }, [isOpen, loadGroups]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("extensions.assignTitle")}</DialogTitle>
          <DialogDescription>
            {t("extensions.assignDescription", {
              count: selectedProfiles.length,
            })}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label>{t("extensions.assignTitle")}:</Label>
            <div className="p-3 bg-muted rounded-md max-h-32 overflow-y-auto">
              <ul className="text-sm space-y-1">
                {selectedProfiles.map((profileId) => {
                  const profile = profiles.find(
                    (p: BrowserProfile) => p.id === profileId,
                  );
                  const displayName = profile ? profile.name : profileId;
                  return (
                    <li key={profileId} className="truncate">
                      • {displayName}
                    </li>
                  );
                })}
              </ul>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="extension-group-select">
              {t("extensions.extensionGroup")}:
            </Label>
            {isLoading ? (
              <div className="text-sm text-muted-foreground">
                {t("common.buttons.loading")}
              </div>
            ) : (
              <Select
                value={selectedGroupId || "none"}
                onValueChange={(value) => {
                  setSelectedGroupId(value === "none" ? null : value);
                }}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">
                    {t("extensions.noGroup")}
                  </SelectItem>
                  {groups.map((group) => (
                    <SelectItem key={group.id} value={group.id}>
                      {group.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
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
            onClick={onClose}
            disabled={isAssigning}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            isLoading={isAssigning}
            onClick={() => void handleAssign()}
            disabled={isLoading}
          >
            {t("common.buttons.apply")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

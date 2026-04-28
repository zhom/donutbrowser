"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { toast } from "sonner";
import { CreateGroupDialog } from "@/components/create-group-dialog";
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
import type { BrowserProfile, ProfileGroup } from "@/types";
import { RippleButton } from "./ui/ripple";

interface GroupAssignmentDialogProps {
  isOpen: boolean;
  onClose: () => void;
  selectedProfiles: string[];
  onAssignmentComplete: () => void;
  profiles?: BrowserProfile[];
}

export function GroupAssignmentDialog({
  isOpen,
  onClose,
  selectedProfiles,
  onAssignmentComplete,
  profiles = [],
}: GroupAssignmentDialogProps) {
  const { t } = useTranslation();
  const [groups, setGroups] = useState<ProfileGroup[]>([]);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isAssigning, setIsAssigning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);

  const loadGroups = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const groupList = await invoke<ProfileGroup[]>("get_profile_groups");
      setGroups(groupList);
    } catch (err) {
      console.error("Failed to load groups:", err);
      setError(
        err instanceof Error ? err.message : t("groupManagement.loadFailed"),
      );
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  const handleAssign = useCallback(async () => {
    setIsAssigning(true);
    setError(null);
    try {
      await invoke("assign_profiles_to_group", {
        profileIds: selectedProfiles,
        groupId: selectedGroupId,
      });

      const groupName = selectedGroupId
        ? groups.find((g) => g.id === selectedGroupId)?.name ||
          t("groups.unknownGroup")
        : t("groups.defaultGroup");

      toast.success(
        t("groups.assignSuccess", {
          count: selectedProfiles.length,
          group: groupName,
        }),
      );
      onAssignmentComplete();
      onClose();
    } catch (err) {
      console.error("Failed to assign profiles to group:", err);
      const errorMessage =
        err instanceof Error
          ? err.message
          : t("groupAssignment.failedFallback");
      setError(errorMessage);
      toast.error(errorMessage);
    } finally {
      setIsAssigning(false);
    }
  }, [
    selectedProfiles,
    selectedGroupId,
    groups,
    onAssignmentComplete,
    onClose,
    t,
  ]);

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
          <DialogTitle>{t("groupAssignment.title")}</DialogTitle>
          <DialogDescription>
            {selectedProfiles.length === 1
              ? t("groupAssignment.description_one", {
                  count: selectedProfiles.length,
                })
              : t("groupAssignment.description_other", {
                  count: selectedProfiles.length,
                })}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label>{t("groupAssignment.selectedProfilesLabel")}</Label>
            <div className="p-3 bg-muted rounded-md max-h-32 overflow-y-auto">
              <ul className="text-sm space-y-1">
                {selectedProfiles.map((profileId) => {
                  // Find the profile name for display
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
            <div className="flex justify-between items-center">
              <Label htmlFor="group-select">
                {t("groupAssignment.assignGroupLabel")}
              </Label>
              <RippleButton
                size="sm"
                variant="outline"
                className="h-7 px-2 text-xs"
                onClick={() => {
                  setCreateDialogOpen(true);
                }}
              >
                <GoPlus className="mr-1 w-3 h-3" />{" "}
                {t("groupManagement.createGroup")}
              </RippleButton>
            </div>
            {isLoading ? (
              <div className="text-sm text-muted-foreground">
                {t("groupManagement.loading")}
              </div>
            ) : (
              <Select
                value={selectedGroupId ?? "default"}
                onValueChange={(value) => {
                  setSelectedGroupId(value === "default" ? null : value);
                }}
              >
                <SelectTrigger>
                  <SelectValue placeholder={t("groupAssignment.placeholder")} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="default">
                    {t("groups.defaultGroupNoGroup")}
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
            {t("groupAssignment.assignButton")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
      <CreateGroupDialog
        isOpen={createDialogOpen}
        onClose={() => {
          setCreateDialogOpen(false);
        }}
        onGroupCreated={(group) => {
          setGroups((prev) => [...prev, group]);
          setSelectedGroupId(group.id);
          setCreateDialogOpen(false);
        }}
      />
    </Dialog>
  );
}

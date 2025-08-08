"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
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

      toast.success("Group updated successfully");
      onGroupUpdated(updatedGroup);
      onClose();
    } catch (err) {
      console.error("Failed to update group:", err);
      const errorMessage =
        err instanceof Error ? err.message : "Failed to update group";
      setError(errorMessage);
      toast.error(errorMessage);
    } finally {
      setIsUpdating(false);
    }
  }, [group, groupName, onGroupUpdated, onClose]);

  const handleClose = useCallback(() => {
    setError(null);
    onClose();
  }, [onClose]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Edit Group</DialogTitle>
          <DialogDescription>
            Update the name of the group "{group?.name}".
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="group-name">Group Name</Label>
            <Input
              id="group-name"
              placeholder="Enter group name..."
              value={groupName}
              onChange={(e) => setGroupName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && groupName.trim()) {
                  void handleUpdate();
                }
              }}
              disabled={isUpdating}
            />
          </div>

          {error && (
            <div className="p-3 text-sm text-red-600 bg-red-50 rounded-md dark:bg-red-900/20 dark:text-red-400">
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
            Cancel
          </RippleButton>
          <LoadingButton
            isLoading={isUpdating}
            onClick={() => void handleUpdate()}
            disabled={!groupName.trim() || groupName === group?.name}
          >
            Update Group
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

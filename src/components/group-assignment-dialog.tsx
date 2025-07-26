"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { ProfileGroup } from "@/types";

interface GroupAssignmentDialogProps {
  isOpen: boolean;
  onClose: () => void;
  selectedProfiles: string[];
  onAssignmentComplete: () => void;
}

export function GroupAssignmentDialog({
  isOpen,
  onClose,
  selectedProfiles,
  onAssignmentComplete,
}: GroupAssignmentDialogProps) {
  const [groups, setGroups] = useState<ProfileGroup[]>([]);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isAssigning, setIsAssigning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadGroups = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const groupList = await invoke<ProfileGroup[]>("get_profile_groups");
      setGroups(groupList);
    } catch (err) {
      console.error("Failed to load groups:", err);
      setError(err instanceof Error ? err.message : "Failed to load groups");
    } finally {
      setIsLoading(false);
    }
  }, []);

  const handleAssign = useCallback(async () => {
    setIsAssigning(true);
    setError(null);
    try {
      await invoke("assign_profiles_to_group", {
        profileNames: selectedProfiles,
        groupId: selectedGroupId,
      });

      const groupName = selectedGroupId
        ? groups.find((g) => g.id === selectedGroupId)?.name || "Unknown Group"
        : "Default";

      toast.success(
        `Successfully assigned ${selectedProfiles.length} profile(s) to ${groupName}`,
      );
      onAssignmentComplete();
      onClose();
    } catch (err) {
      console.error("Failed to assign profiles to group:", err);
      const errorMessage =
        err instanceof Error
          ? err.message
          : "Failed to assign profiles to group";
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
          <DialogTitle>Assign to Group</DialogTitle>
          <DialogDescription>
            Assign {selectedProfiles.length} selected profile(s) to a group.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label>Selected Profiles:</Label>
            <div className="p-3 bg-muted rounded-md max-h-32 overflow-y-auto">
              <ul className="text-sm space-y-1">
                {selectedProfiles.map((profileName) => (
                  <li key={profileName} className="truncate">
                    • {profileName}
                  </li>
                ))}
              </ul>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="group-select">Assign to Group:</Label>
            {isLoading ? (
              <div className="text-sm text-muted-foreground">
                Loading groups...
              </div>
            ) : (
              <Select
                value={selectedGroupId || "default"}
                onValueChange={(value) => {
                  setSelectedGroupId(value === "default" ? null : value);
                }}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select a group" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="default">Default (No Group)</SelectItem>
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
            <div className="p-3 text-sm text-red-600 bg-red-50 rounded-md dark:bg-red-900/20 dark:text-red-400">
              {error}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={isAssigning}>
            Cancel
          </Button>
          <LoadingButton
            isLoading={isAssigning}
            onClick={() => void handleAssign()}
            disabled={isLoading}
          >
            Assign
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

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
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { BrowserProfile, ProfileGroup } from "@/types";
import { RippleButton } from "./ui/ripple";

interface DeleteGroupDialogProps {
  isOpen: boolean;
  onClose: () => void;
  group: ProfileGroup | null;
  onGroupDeleted: () => void;
}

export function DeleteGroupDialog({
  isOpen,
  onClose,
  group,
  onGroupDeleted,
}: DeleteGroupDialogProps) {
  const [associatedProfiles, setAssociatedProfiles] = useState<
    BrowserProfile[]
  >([]);
  const [deleteAction, setDeleteAction] = useState<"move" | "delete">("move");
  const [isDeleting, setIsDeleting] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadAssociatedProfiles = useCallback(async () => {
    if (!group) return;

    setIsLoading(true);
    setError(null);
    try {
      const allProfiles = await invoke<BrowserProfile[]>(
        "list_browser_profiles",
      );
      const groupProfiles = allProfiles.filter(
        (profile) => profile.group_id === group.id,
      );
      setAssociatedProfiles(groupProfiles);
    } catch (err) {
      console.error("Failed to load associated profiles:", err);
      setError(err instanceof Error ? err.message : "Failed to load profiles");
    } finally {
      setIsLoading(false);
    }
  }, [group]);

  useEffect(() => {
    if (isOpen && group) {
      void loadAssociatedProfiles();
    }
  }, [isOpen, group, loadAssociatedProfiles]);

  const handleDelete = useCallback(async () => {
    if (!group) return;

    setIsDeleting(true);
    setError(null);
    try {
      if (deleteAction === "delete" && associatedProfiles.length > 0) {
        // Delete all associated profiles first
        const profileNames = associatedProfiles.map((p) => p.name);
        await invoke("delete_selected_profiles", { profileNames });
      } else if (deleteAction === "move" && associatedProfiles.length > 0) {
        // Move profiles to default group (null group_id)
        const profileNames = associatedProfiles.map((p) => p.name);
        await invoke("assign_profiles_to_group", {
          profileNames,
          groupId: null,
        });
      }

      // Delete the group
      await invoke("delete_profile_group", { groupId: group.id });

      toast.success("Group deleted successfully");
      onGroupDeleted();
      onClose();
    } catch (err) {
      console.error("Failed to delete group:", err);
      const errorMessage =
        err instanceof Error ? err.message : "Failed to delete group";
      setError(errorMessage);
      toast.error(errorMessage);
    } finally {
      setIsDeleting(false);
    }
  }, [group, deleteAction, associatedProfiles, onGroupDeleted, onClose]);

  const handleClose = useCallback(() => {
    setError(null);
    setDeleteAction("move");
    setAssociatedProfiles([]);
    onClose();
  }, [onClose]);

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Delete Group</DialogTitle>
          <DialogDescription>
            This action cannot be undone. This will permanently delete the group
            "{group?.name}".
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {isLoading ? (
            <div className="text-sm text-muted-foreground">
              Loading associated profiles...
            </div>
          ) : (
            <>
              {associatedProfiles.length > 0 && (
                <div className="space-y-3">
                  <div className="space-y-2">
                    <Label>
                      Associated Profiles ({associatedProfiles.length})
                    </Label>
                    <ScrollArea className="h-32 w-full border rounded-md p-3">
                      <div className="space-y-1">
                        {associatedProfiles.map((profile) => (
                          <div key={profile.id} className="text-sm">
                            • {profile.name}
                          </div>
                        ))}
                      </div>
                    </ScrollArea>
                  </div>

                  <div className="space-y-3">
                    <Label>What should happen to these profiles?</Label>
                    <RadioGroup
                      value={deleteAction}
                      onValueChange={(value) =>
                        setDeleteAction(value as "move" | "delete")
                      }
                    >
                      <div className="flex items-center space-x-2">
                        <RadioGroupItem value="move" id="move" />
                        <Label htmlFor="move" className="text-sm">
                          Move profiles to Default group
                        </Label>
                      </div>
                      <div className="flex items-center space-x-2">
                        <RadioGroupItem value="delete" id="delete" />
                        <Label
                          htmlFor="delete"
                          className="text-sm text-red-600"
                        >
                          Delete profiles along with the group
                        </Label>
                      </div>
                    </RadioGroup>
                  </div>
                </div>
              )}

              {associatedProfiles.length === 0 && !isLoading && (
                <div className="text-sm text-muted-foreground">
                  This group has no associated profiles.
                </div>
              )}
            </>
          )}

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
            disabled={isDeleting}
          >
            Cancel
          </RippleButton>
          <LoadingButton
            variant="destructive"
            isLoading={isDeleting}
            onClick={() => void handleDelete()}
            disabled={isLoading}
          >
            Delete Group
            {deleteAction === "delete" &&
              associatedProfiles.length > 0 &&
              " & Profiles"}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

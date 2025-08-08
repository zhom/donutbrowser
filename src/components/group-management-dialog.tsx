"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { GoPlus } from "react-icons/go";
import { LuPencil, LuTrash2 } from "react-icons/lu";
import { CreateGroupDialog } from "@/components/create-group-dialog";
import { DeleteGroupDialog } from "@/components/delete-group-dialog";
import { EditGroupDialog } from "@/components/edit-group-dialog";
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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { ProfileGroup } from "@/types";
import { RippleButton } from "./ui/ripple";

interface GroupManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onGroupManagementComplete: () => void;
}

export function GroupManagementDialog({
  isOpen,
  onClose,
  onGroupManagementComplete,
}: GroupManagementDialogProps) {
  const [groups, setGroups] = useState<ProfileGroup[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Dialog states
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [editDialogOpen, setEditDialogOpen] = useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [selectedGroup, setSelectedGroup] = useState<ProfileGroup | null>(null);

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

  const handleGroupCreated = useCallback(
    (newGroup: ProfileGroup) => {
      setGroups((prev) => [...prev, newGroup]);
      onGroupManagementComplete();
    },
    [onGroupManagementComplete],
  );

  const handleGroupUpdated = useCallback(
    (updatedGroup: ProfileGroup) => {
      setGroups((prev) =>
        prev.map((group) =>
          group.id === updatedGroup.id ? updatedGroup : group,
        ),
      );
      onGroupManagementComplete();
    },
    [onGroupManagementComplete],
  );

  const handleGroupDeleted = useCallback(() => {
    void loadGroups();
    onGroupManagementComplete();
  }, [loadGroups, onGroupManagementComplete]);

  const handleEditGroup = useCallback((group: ProfileGroup) => {
    setSelectedGroup(group);
    setEditDialogOpen(true);
  }, []);

  const handleDeleteGroup = useCallback((group: ProfileGroup) => {
    setSelectedGroup(group);
    setDeleteDialogOpen(true);
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadGroups();
    }
  }, [isOpen, loadGroups]);

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>Manage Profile Groups</DialogTitle>
            <DialogDescription>
              Create, edit, and delete profile groups. Profiles without a group
              will appear in the "Default" group.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            {/* Create new group button */}
            <div className="flex justify-between items-center">
              <Label>Groups</Label>
              <RippleButton
                size="sm"
                onClick={() => setCreateDialogOpen(true)}
                className="flex gap-2 items-center"
              >
                <GoPlus className="w-4 h-4" />
                Create
              </RippleButton>
            </div>

            {error && (
              <div className="p-3 text-sm text-red-600 bg-red-50 rounded-md dark:bg-red-900/20 dark:text-red-400">
                {error}
              </div>
            )}

            {/* Groups list */}
            {isLoading ? (
              <div className="text-sm text-muted-foreground">
                Loading groups...
              </div>
            ) : groups.length === 0 ? (
              <div className="text-sm text-muted-foreground">
                No groups created yet. Create your first group using the button
                above.
              </div>
            ) : (
              <div className="border rounded-md">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Name</TableHead>
                      <TableHead className="w-24">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {groups.map((group) => (
                      <TableRow key={group.id}>
                        <TableCell className="font-medium">
                          {group.name}
                        </TableCell>
                        <TableCell>
                          <div className="flex gap-1">
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => handleEditGroup(group)}
                            >
                              <LuPencil className="w-4 h-4" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => handleDeleteGroup(group)}
                            >
                              <LuTrash2 className="w-4 h-4" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </div>

          <DialogFooter>
            <RippleButton variant="outline" onClick={onClose}>
              Close
            </RippleButton>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <CreateGroupDialog
        isOpen={createDialogOpen}
        onClose={() => setCreateDialogOpen(false)}
        onGroupCreated={handleGroupCreated}
      />

      <EditGroupDialog
        isOpen={editDialogOpen}
        onClose={() => setEditDialogOpen(false)}
        group={selectedGroup}
        onGroupUpdated={handleGroupUpdated}
      />

      <DeleteGroupDialog
        isOpen={deleteDialogOpen}
        onClose={() => setDeleteDialogOpen(false)}
        group={selectedGroup}
        onGroupDeleted={handleGroupDeleted}
      />
    </>
  );
}

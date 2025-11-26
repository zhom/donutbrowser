"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { GoPlus } from "react-icons/go";
import { LuPencil, LuTrash2 } from "react-icons/lu";
import { CreateGroupDialog } from "@/components/create-group-dialog";
import { DeleteGroupDialog } from "@/components/delete-group-dialog";
import { EditGroupDialog } from "@/components/edit-group-dialog";
import { Badge } from "@/components/ui/badge";
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
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { GroupWithCount, ProfileGroup } from "@/types";
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
  const [groups, setGroups] = useState<GroupWithCount[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Dialog states
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [editDialogOpen, setEditDialogOpen] = useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [selectedGroup, setSelectedGroup] = useState<GroupWithCount | null>(
    null,
  );

  const loadGroups = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const groupList = await invoke<GroupWithCount[]>(
        "get_groups_with_profile_counts",
      );
      setGroups(groupList);
    } catch (err) {
      console.error("Failed to load groups:", err);
      setError(err instanceof Error ? err.message : "Failed to load groups");
    } finally {
      setIsLoading(false);
    }
  }, []);

  const handleGroupCreated = useCallback(
    (_newGroup: ProfileGroup) => {
      void loadGroups();
      onGroupManagementComplete();
    },
    [loadGroups, onGroupManagementComplete],
  );

  const handleGroupUpdated = useCallback(
    (_updatedGroup: ProfileGroup) => {
      void loadGroups();
      onGroupManagementComplete();
    },
    [loadGroups, onGroupManagementComplete],
  );

  const handleGroupDeleted = useCallback(() => {
    void loadGroups();
    onGroupManagementComplete();
  }, [loadGroups, onGroupManagementComplete]);

  const handleEditGroup = useCallback((group: GroupWithCount) => {
    setSelectedGroup(group);
    setEditDialogOpen(true);
  }, []);

  const handleDeleteGroup = useCallback((group: GroupWithCount) => {
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
                <ScrollArea className="h-[240px]">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Name</TableHead>
                        <TableHead className="w-20">Profiles</TableHead>
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
                            <Badge variant="secondary">{group.count}</Badge>
                          </TableCell>
                          <TableCell>
                            <div className="flex gap-1">
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => handleEditGroup(group)}
                                  >
                                    <LuPencil className="w-4 h-4" />
                                  </Button>
                                </TooltipTrigger>
                                <TooltipContent>
                                  <p>Edit group</p>
                                </TooltipContent>
                              </Tooltip>
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => handleDeleteGroup(group)}
                                  >
                                    <LuTrash2 className="w-4 h-4" />
                                  </Button>
                                </TooltipTrigger>
                                <TooltipContent>
                                  <p>Delete group</p>
                                </TooltipContent>
                              </Tooltip>
                            </div>
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </ScrollArea>
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

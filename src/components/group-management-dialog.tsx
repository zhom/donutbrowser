"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { GoPlus } from "react-icons/go";
import { LuPencil, LuTrash2 } from "react-icons/lu";
import { CreateGroupDialog } from "@/components/create-group-dialog";
import { DeleteGroupDialog } from "@/components/delete-group-dialog";
import { EditGroupDialog } from "@/components/edit-group-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { GroupWithCount, ProfileGroup } from "@/types";
import { RippleButton } from "./ui/ripple";

type SyncStatus = "disabled" | "syncing" | "synced" | "error" | "waiting";

function getSyncStatusDot(
  group: GroupWithCount,
  liveStatus: SyncStatus | undefined,
): { color: string; tooltip: string; animate: boolean } {
  const status = liveStatus ?? (group.sync_enabled ? "synced" : "disabled");

  switch (status) {
    case "syncing":
      return { color: "bg-yellow-500", tooltip: "Syncing...", animate: true };
    case "synced":
      return {
        color: "bg-green-500",
        tooltip: group.last_sync
          ? `Synced ${new Date(group.last_sync * 1000).toLocaleString()}`
          : "Synced",
        animate: false,
      };
    case "waiting":
      return {
        color: "bg-yellow-500",
        tooltip: "Waiting to sync",
        animate: false,
      };
    case "error":
      return { color: "bg-red-500", tooltip: "Sync error", animate: false };
    default:
      return { color: "bg-gray-400", tooltip: "Not synced", animate: false };
  }
}

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
  const [groupSyncStatus, setGroupSyncStatus] = useState<
    Record<string, SyncStatus>
  >({});
  const [groupInUse, setGroupInUse] = useState<Record<string, boolean>>({});
  const [isTogglingSync, setIsTogglingSync] = useState<Record<string, boolean>>(
    {},
  );

  // Listen for group sync status events
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      unlisten = await listen<{ id: string; status: string }>(
        "group-sync-status",
        (event) => {
          const { id, status } = event.payload;
          setGroupSyncStatus((prev) => ({
            ...prev,
            [id]: status as SyncStatus,
          }));
        },
      );
    };

    void setupListener();
    return () => {
      unlisten?.();
    };
  }, []);

  const loadGroups = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const groupList = await invoke<GroupWithCount[]>(
        "get_groups_with_profile_counts",
      );
      setGroups(groupList);

      // Check which groups are in use by synced profiles
      const inUse: Record<string, boolean> = {};
      for (const group of groupList) {
        try {
          const inUseBySynced = await invoke<boolean>(
            "is_group_in_use_by_synced_profile",
            { groupId: group.id },
          );
          inUse[group.id] = inUseBySynced;
        } catch (_error) {
          // Ignore errors
        }
      }
      setGroupInUse(inUse);
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

  const handleToggleSync = useCallback(
    async (group: GroupWithCount) => {
      setIsTogglingSync((prev) => ({ ...prev, [group.id]: true }));
      try {
        await invoke("set_group_sync_enabled", {
          groupId: group.id,
          enabled: !group.sync_enabled,
        });
        showSuccessToast(group.sync_enabled ? "Sync disabled" : "Sync enabled");
        await loadGroups();
      } catch (error) {
        console.error("Failed to toggle sync:", error);
        showErrorToast(
          error instanceof Error ? error.message : "Failed to update sync",
        );
      } finally {
        setIsTogglingSync((prev) => ({ ...prev, [group.id]: false }));
      }
    },
    [loadGroups],
  );

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
                        <TableHead className="w-24">Sync</TableHead>
                        <TableHead className="w-24">Actions</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {groups.map((group) => {
                        const syncDot = getSyncStatusDot(
                          group,
                          groupSyncStatus[group.id],
                        );
                        return (
                          <TableRow key={group.id}>
                            <TableCell className="font-medium">
                              <div className="flex items-center gap-2">
                                <Tooltip>
                                  <TooltipTrigger asChild>
                                    <div
                                      className={`w-2 h-2 rounded-full shrink-0 ${syncDot.color} ${
                                        syncDot.animate ? "animate-pulse" : ""
                                      }`}
                                    />
                                  </TooltipTrigger>
                                  <TooltipContent>
                                    <p>{syncDot.tooltip}</p>
                                  </TooltipContent>
                                </Tooltip>
                                {group.name}
                              </div>
                            </TableCell>
                            <TableCell>
                              <Badge variant="secondary">{group.count}</Badge>
                            </TableCell>
                            <TableCell>
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <div className="flex items-center">
                                    <Checkbox
                                      checked={group.sync_enabled}
                                      onCheckedChange={() =>
                                        handleToggleSync(group)
                                      }
                                      disabled={
                                        isTogglingSync[group.id] ||
                                        groupInUse[group.id]
                                      }
                                    />
                                  </div>
                                </TooltipTrigger>
                                <TooltipContent>
                                  {groupInUse[group.id] ? (
                                    <p>
                                      Sync cannot be disabled while this group
                                      is used by synced profiles
                                    </p>
                                  ) : (
                                    <p>
                                      {group.sync_enabled
                                        ? "Disable sync"
                                        : "Enable sync"}
                                    </p>
                                  )}
                                </TooltipContent>
                              </Tooltip>
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
                        );
                      })}
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

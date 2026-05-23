"use client";

import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  type RowSelectionState,
  type SortingState,
  useReactTable,
} from "@tanstack/react-table";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import {
  LuChevronDown,
  LuChevronUp,
  LuFolder,
  LuPencil,
  LuRefreshCw,
  LuTrash2,
} from "react-icons/lu";
import { CreateGroupDialog } from "@/components/create-group-dialog";
import {
  DataTableActionBar,
  DataTableActionBarAction,
  DataTableActionBarSelection,
} from "@/components/data-table-action-bar";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { DeleteGroupDialog } from "@/components/delete-group-dialog";
import { EditGroupDialog } from "@/components/edit-group-dialog";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
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
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
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
import { parseBackendError, translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { GroupWithCount, ProfileGroup } from "@/types";
import { RippleButton } from "./ui/ripple";

type SyncStatus = "disabled" | "syncing" | "synced" | "error" | "waiting";

function getSyncStatusDot(
  group: GroupWithCount,
  liveStatus: SyncStatus | undefined,
  t: (key: string, options?: Record<string, unknown>) => string,
  errorMessage?: string,
): { color: string; tooltip: string; animate: boolean } {
  const status = liveStatus ?? (group.sync_enabled ? "synced" : "disabled");

  switch (status) {
    case "syncing":
      return {
        color: "bg-warning",
        tooltip: t("syncTooltips.syncing"),
        animate: true,
      };
    case "synced":
      return {
        color: "bg-success",
        tooltip: group.last_sync
          ? t("syncTooltips.syncedAt", {
              time: new Date(group.last_sync * 1000).toLocaleString(),
            })
          : t("syncTooltips.synced"),
        animate: false,
      };
    case "waiting":
      return {
        color: "bg-warning",
        tooltip: t("syncTooltips.waiting"),
        animate: false,
      };
    case "error":
      return {
        color: "bg-destructive",
        tooltip: errorMessage
          ? t("syncTooltips.errorWith", { error: errorMessage })
          : t("syncTooltips.error"),
        animate: false,
      };
    default:
      return {
        color: "bg-muted-foreground",
        tooltip: t("syncTooltips.notSynced"),
        animate: false,
      };
  }
}

interface GroupManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onGroupManagementComplete: () => void;
  subPage?: boolean;
}

export function GroupManagementDialog({
  isOpen,
  onClose,
  onGroupManagementComplete,
  subPage,
}: GroupManagementDialogProps) {
  const { t } = useTranslation();
  const [groups, setGroups] = useState<GroupWithCount[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Dialog states
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [editDialogOpen, setEditDialogOpen] = useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [bulkDeleteOpen, setBulkDeleteOpen] = useState(false);
  const [isBulkDeleting, setIsBulkDeleting] = useState(false);
  const [selectedGroup, setSelectedGroup] = useState<GroupWithCount | null>(
    null,
  );
  const [groupSyncStatus, setGroupSyncStatus] = useState<
    Record<string, SyncStatus>
  >({});
  const [groupSyncErrors, setGroupSyncErrors] = useState<
    Record<string, string>
  >({});
  const [groupInUse, setGroupInUse] = useState<Record<string, boolean>>({});
  const [isTogglingSync, setIsTogglingSync] = useState<Record<string, boolean>>(
    {},
  );

  // Table state
  const [sorting, setSorting] = useState<SortingState>([
    { id: "name", desc: false },
  ]);
  const [rowSelection, setRowSelection] = useState<RowSelectionState>({});

  // Listen for group sync status events
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      unlisten = await listen<{ id: string; status: string; error?: string }>(
        "group-sync-status",
        (event) => {
          const { id, status, error } = event.payload;
          setGroupSyncStatus((prev) => ({
            ...prev,
            [id]: status as SyncStatus,
          }));
          if (error) {
            setGroupSyncErrors((prev) => ({ ...prev, [id]: error }));
          }
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
      setError(
        err instanceof Error ? err.message : t("groupManagement.loadFailed"),
      );
    } finally {
      setIsLoading(false);
    }
  }, [t]);

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
        showSuccessToast(
          group.sync_enabled
            ? t("proxies.management.syncDisabled")
            : t("proxies.management.syncEnabled"),
        );
        await loadGroups();
      } catch (error) {
        console.error("Failed to toggle sync:", error);
        showErrorToast(
          parseBackendError(error)
            ? translateBackendError(t, error)
            : t("proxies.management.updateSyncFailed"),
        );
      } finally {
        setIsTogglingSync((prev) => ({ ...prev, [group.id]: false }));
      }
    },
    [loadGroups, t],
  );

  useEffect(() => {
    if (isOpen) {
      void loadGroups();
    } else {
      // Drop any selection when the dialog closes so the floating
      // action bar (portaled to body) doesn't linger on the page.
      setRowSelection({});
    }
  }, [isOpen, loadGroups]);

  const columns = useMemo<ColumnDef<GroupWithCount>[]>(
    () => [
      {
        id: "select",
        size: 36,
        enableSorting: false,
        header: ({ table }) => (
          <Checkbox
            checked={
              table.getIsAllRowsSelected()
                ? true
                : table.getIsSomeRowsSelected()
                  ? "indeterminate"
                  : false
            }
            onCheckedChange={(value) => {
              table.toggleAllRowsSelected(!!value);
            }}
            aria-label={t("common.aria.selectAll")}
            disabled={table.getRowModel().rows.length === 0}
          />
        ),
        cell: ({ row }) => (
          <Checkbox
            checked={row.getIsSelected()}
            onCheckedChange={(value) => {
              row.toggleSelected(!!value);
            }}
            aria-label={t("common.aria.selectRow")}
          />
        ),
      },
      {
        accessorKey: "name",
        enableSorting: true,
        sortingFn: "alphanumeric",
        header: ({ column }) => (
          <Button
            variant="ghost"
            onClick={() => {
              column.toggleSorting(column.getIsSorted() === "asc");
            }}
            className="justify-start p-0 h-auto font-semibold text-left cursor-pointer"
          >
            {t("common.labels.name")}
            {column.getIsSorted() === "asc" ? (
              <LuChevronUp className="ml-2 size-4" />
            ) : column.getIsSorted() === "desc" ? (
              <LuChevronDown className="ml-2 size-4" />
            ) : null}
          </Button>
        ),
        cell: ({ row }) => {
          const group = row.original;
          const syncDot = getSyncStatusDot(
            group,
            groupSyncStatus[group.id],
            t,
            groupSyncErrors[group.id],
          );
          return (
            <div className="flex items-center gap-2 font-medium">
              <Tooltip>
                <TooltipTrigger asChild>
                  <div
                    className={`size-2 rounded-full shrink-0 ${syncDot.color} ${
                      syncDot.animate ? "animate-pulse" : ""
                    }`}
                  />
                </TooltipTrigger>
                <TooltipContent>
                  <p>{syncDot.tooltip}</p>
                </TooltipContent>
              </Tooltip>
              <LuFolder className="size-4 text-muted-foreground" />
              {group.name}
            </div>
          );
        },
      },
      {
        id: "count",
        size: 80,
        enableSorting: false,
        header: () => t("groupManagement.profilesCol"),
        cell: ({ row }) => (
          <Badge variant="secondary">{row.original.count}</Badge>
        ),
      },
      {
        id: "sync",
        size: 96,
        enableSorting: false,
        header: () => t("proxies.management.syncCol"),
        cell: ({ row }) => {
          const group = row.original;
          const locked = groupInUse[group.id];
          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="inline-flex items-center">
                  <AnimatedSwitch
                    checked={group.sync_enabled}
                    onCheckedChange={() => handleToggleSync(group)}
                    disabled={isTogglingSync[group.id] || locked}
                  />
                </span>
              </TooltipTrigger>
              <TooltipContent>
                {locked ? (
                  <p>{t("syncTooltips.lockedInUse")}</p>
                ) : (
                  <p>
                    {group.sync_enabled
                      ? t("syncTooltips.disable")
                      : t("syncTooltips.enable")}
                  </p>
                )}
              </TooltipContent>
            </Tooltip>
          );
        },
      },
      {
        id: "actions",
        size: 96,
        enableSorting: false,
        header: () => t("common.labels.actions"),
        cell: ({ row }) => {
          const group = row.original;
          return (
            <div className="flex gap-1">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      handleEditGroup(group);
                    }}
                  >
                    <LuPencil className="size-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  <p>{t("groupManagement.editGroupTooltip")}</p>
                </TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      handleDeleteGroup(group);
                    }}
                  >
                    <LuTrash2 className="size-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  <p>{t("groupManagement.deleteGroupTooltip")}</p>
                </TooltipContent>
              </Tooltip>
            </div>
          );
        },
      },
    ],
    [
      t,
      groupSyncStatus,
      groupSyncErrors,
      groupInUse,
      isTogglingSync,
      handleToggleSync,
      handleEditGroup,
      handleDeleteGroup,
    ],
  );

  const table = useReactTable({
    data: groups,
    columns,
    state: { sorting, rowSelection },
    onSortingChange: setSorting,
    onRowSelectionChange: setRowSelection,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getRowId: (row) => row.id,
  });

  const selectedRows = table.getFilteredSelectedRowModel().rows;
  const selectedGroupsForBulk = useMemo(
    () => selectedRows.map((row) => row.original),
    [selectedRows],
  );
  const selectedNames = useMemo(
    () => selectedGroupsForBulk.map((g) => g.name).join(", "),
    [selectedGroupsForBulk],
  );

  const handleBulkDelete = useCallback(async () => {
    if (selectedGroupsForBulk.length === 0) return;
    setIsBulkDeleting(true);
    try {
      const ids = selectedGroupsForBulk.map((g) => g.id);
      const results = await Promise.allSettled(
        ids.map((groupId) => invoke("delete_profile_group", { groupId })),
      );
      const failed = results.filter((r) => r.status === "rejected");
      if (failed.length > 0) {
        showErrorToast(t("groups.deleteFailed"));
      } else {
        showSuccessToast(t("groups.deleteSuccess"));
      }
      table.toggleAllRowsSelected(false);
      setBulkDeleteOpen(false);
      await loadGroups();
      onGroupManagementComplete();
    } catch (err) {
      console.error("Bulk group delete failed:", err);
      showErrorToast(
        err instanceof Error ? err.message : t("groups.deleteFailed"),
      );
    } finally {
      setIsBulkDeleting(false);
    }
  }, [selectedGroupsForBulk, table, loadGroups, onGroupManagementComplete, t]);

  const handleBulkToggleSync = useCallback(async () => {
    if (selectedGroupsForBulk.length === 0) return;
    const allOn = selectedGroupsForBulk.every((g) => g.sync_enabled);
    const targetEnabled = !allOn;
    const targets = selectedGroupsForBulk.filter((g) =>
      targetEnabled ? !g.sync_enabled : g.sync_enabled && !groupInUse[g.id],
    );
    if (targets.length === 0) return;
    const results = await Promise.allSettled(
      targets.map((group) =>
        invoke("set_group_sync_enabled", {
          groupId: group.id,
          enabled: targetEnabled,
        }),
      ),
    );
    const firstRejection = results.find((r) => r.status === "rejected") as
      | PromiseRejectedResult
      | undefined;
    if (firstRejection) {
      showErrorToast(
        parseBackendError(firstRejection.reason)
          ? translateBackendError(t, firstRejection.reason)
          : t("proxies.management.updateSyncFailed"),
      );
    } else {
      showSuccessToast(
        targetEnabled
          ? t("proxies.management.syncEnabled")
          : t("proxies.management.syncDisabled"),
      );
    }
    await loadGroups();
  }, [selectedGroupsForBulk, groupInUse, loadGroups, t]);

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
        <DialogContent className="max-w-2xl max-h-[90vh] flex flex-col">
          {!subPage && (
            <DialogHeader>
              <DialogTitle>{t("groups.management")}</DialogTitle>
              <DialogDescription>
                {t("groups.noGroupDescription")}
              </DialogDescription>
            </DialogHeader>
          )}

          <div className="flex flex-col gap-4 flex-1 min-h-0">
            <div className="flex items-start justify-between gap-3">
              <div className="flex flex-col gap-1">
                <h2 className="text-base font-semibold">
                  {t("groups.pageTitle")}
                </h2>
                <p className="text-xs text-muted-foreground">
                  {t("groups.pageDescription")}
                </p>
              </div>
              <RippleButton
                size="sm"
                onClick={() => {
                  setCreateDialogOpen(true);
                }}
                className="flex gap-2 items-center shrink-0"
              >
                <GoPlus className="size-4" />
                {t("proxies.management.create")}
              </RippleButton>
            </div>

            {error && (
              <div className="p-3 text-sm text-destructive bg-destructive/10 rounded-md">
                {error}
              </div>
            )}

            {/* Groups list */}
            {isLoading ? (
              <div className="text-sm text-muted-foreground">
                {t("common.buttons.loading")}
              </div>
            ) : groups.length === 0 ? (
              <div className="text-sm text-muted-foreground">
                {t("groups.noGroupsDescription")}
              </div>
            ) : (
              <FadingScrollArea
                className="flex-1 min-h-0"
                style={
                  {
                    "--scroll-fade-top-offset": "32px",
                  } as React.CSSProperties
                }
              >
                <Table>
                  <TableHeader className="sticky top-0 z-10 bg-background">
                    {table.getHeaderGroups().map((headerGroup) => (
                      <TableRow key={headerGroup.id}>
                        {headerGroup.headers.map((header) => (
                          <TableHead
                            key={header.id}
                            style={{
                              width: header.column.columnDef.size
                                ? `${header.column.getSize()}px`
                                : undefined,
                            }}
                          >
                            {header.isPlaceholder
                              ? null
                              : flexRender(
                                  header.column.columnDef.header,
                                  header.getContext(),
                                )}
                          </TableHead>
                        ))}
                      </TableRow>
                    ))}
                  </TableHeader>
                  <TableBody>
                    {table.getRowModel().rows.map((row) => (
                      <TableRow
                        key={row.id}
                        data-state={row.getIsSelected() && "selected"}
                      >
                        {row.getVisibleCells().map((cell) => (
                          <TableCell
                            key={cell.id}
                            style={{
                              width: cell.column.columnDef.size
                                ? `${cell.column.getSize()}px`
                                : undefined,
                            }}
                          >
                            {flexRender(
                              cell.column.columnDef.cell,
                              cell.getContext(),
                            )}
                          </TableCell>
                        ))}
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </FadingScrollArea>
            )}
          </div>

          {!subPage && (
            <DialogFooter>
              <RippleButton variant="outline" onClick={onClose}>
                {t("common.buttons.close")}
              </RippleButton>
            </DialogFooter>
          )}
        </DialogContent>
      </Dialog>

      {isOpen && (
        <DataTableActionBar table={table}>
          <DataTableActionBarSelection table={table} />
          <DataTableActionBarAction
            tooltip={t("syncTooltips.bulkToggle")}
            onClick={() => {
              void handleBulkToggleSync();
            }}
            size="icon"
          >
            <LuRefreshCw />
          </DataTableActionBarAction>
          <DataTableActionBarAction
            tooltip={t("common.buttons.delete")}
            onClick={() => setBulkDeleteOpen(true)}
            size="icon"
            variant="destructive"
            className="border-destructive bg-destructive/50 hover:bg-destructive/70"
          >
            <LuTrash2 />
          </DataTableActionBarAction>
        </DataTableActionBar>
      )}

      <DeleteConfirmationDialog
        isOpen={bulkDeleteOpen}
        onClose={() => {
          if (!isBulkDeleting) setBulkDeleteOpen(false);
        }}
        onConfirm={handleBulkDelete}
        title={t("groupManagement.bulkDelete.title")}
        description={t("groupManagement.bulkDelete.description", {
          count: selectedGroupsForBulk.length,
          names: selectedNames,
        })}
        confirmButtonText={t("groupManagement.bulkDelete.confirmButton")}
        isLoading={isBulkDeleting}
      />

      <CreateGroupDialog
        isOpen={createDialogOpen}
        onClose={() => {
          setCreateDialogOpen(false);
        }}
        onGroupCreated={handleGroupCreated}
      />

      <EditGroupDialog
        isOpen={editDialogOpen}
        onClose={() => {
          setEditDialogOpen(false);
        }}
        group={selectedGroup}
        onGroupUpdated={handleGroupUpdated}
      />

      <DeleteGroupDialog
        isOpen={deleteDialogOpen}
        onClose={() => {
          setDeleteDialogOpen(false);
        }}
        group={selectedGroup}
        onGroupDeleted={handleGroupDeleted}
      />
    </>
  );
}

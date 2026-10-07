"use client";

import { type RowSelectionState } from "@tanstack/react-table";
import {
  type LegacyColumnDef as ColumnDef,
  getCoreRowModel,
  getSortedRowModel,
  useLegacyTable as useReactTable,
} from "@tanstack/react-table/legacy";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import {
  LuBookmark,
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
import { GroupBookmarksDialog } from "@/components/group-bookmarks-dialog";
import {
  configureEntityColumns,
  ManagedDataTable,
  useEntityTableTools,
} from "@/components/managed-data-table";
import { ProfileUsageButton } from "@/components/profile-usage-button";
import { SortableColumnHeader } from "@/components/table-controls";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useProfileReferences } from "@/hooks/use-profile-references";
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
  const { profiles: referencedProfiles, failed: referencesFailed } =
    useProfileReferences(isOpen);
  const [groups, setGroups] = useState<GroupWithCount[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Dialog states
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [editDialogOpen, setEditDialogOpen] = useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [bookmarksDialogOpen, setBookmarksDialogOpen] = useState(false);
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

  const handleEditBookmarks = useCallback((group: GroupWithCount) => {
    setSelectedGroup(group);
    setBookmarksDialogOpen(true);
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
          <SortableColumnHeader
            column={column}
            label={t("common.labels.name")}
          />
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
            <div className="flex min-w-0 items-center gap-2 font-medium">
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
              <LuFolder className="size-4 shrink-0 text-muted-foreground" />
              <span className="truncate">{group.name}</span>
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
          <ProfileUsageButton
            label={t("appFeedback.assignedProfiles", {
              name: row.original.name,
            })}
            failed={referencesFailed}
            profiles={
              referencedProfiles?.filter(
                (profile) => profile.group_id === row.original.id,
              ) ?? null
            }
          />
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
        size: 132,
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
                    data-testid="group-bookmarks-button"
                    aria-label={t("groupBookmarks.editTooltip", {
                      n: group.bookmark_count ?? 0,
                    })}
                    onClick={() => {
                      handleEditBookmarks(group);
                    }}
                  >
                    <LuBookmark className="size-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  <p>
                    {t("groupBookmarks.editTooltip", {
                      n: group.bookmark_count ?? 0,
                    })}
                  </p>
                </TooltipContent>
              </Tooltip>
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
      referencedProfiles,
      referencesFailed,
      groupSyncStatus,
      groupSyncErrors,
      groupInUse,
      isTogglingSync,
      handleToggleSync,
      handleEditGroup,
      handleDeleteGroup,
      handleEditBookmarks,
    ],
  );

  const tableTools = useEntityTableTools("groups", groups, setRowSelection);
  const table = useReactTable({
    data: tableTools.filters.rows,
    columns: useMemo(() => configureEntityColumns(columns), [columns]),
    state: {
      sorting: tableTools.view.preferences.sorting,
      rowSelection,
      columnVisibility: tableTools.visibility,
      columnSizing: tableTools.view.preferences.sizing,
    },
    onSortingChange: tableTools.view.setSorting,
    onColumnSizingChange: tableTools.view.setSizing,
    columnResizeMode: "onChange",
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
      const firstRejection = results.find((r) => r.status === "rejected") as
        | PromiseRejectedResult
        | undefined;
      if (firstRejection) {
        showErrorToast(
          parseBackendError(firstRejection.reason)
            ? translateBackendError(t, firstRejection.reason)
            : t("groups.deleteFailed"),
        );
      } else {
        showSuccessToast(t("groups.deleteSuccess"));
      }
      table.toggleAllRowsSelected(false);
      setBulkDeleteOpen(false);
      await loadGroups();
      onGroupManagementComplete();
    } catch (err) {
      console.error("Bulk group delete failed:", err);
      showErrorToast(translateBackendError(t, err));
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
        <DialogContent className="flex max-h-[85vh] max-w-[min(80rem,calc(100%-4rem))] flex-col">
          {!subPage && (
            <DialogHeader>
              <DialogTitle>{t("groups.management")}</DialogTitle>
              <DialogDescription>
                {t("groups.noGroupDescription")}
              </DialogDescription>
            </DialogHeader>
          )}

          <div className="@container flex min-h-0 w-full flex-1 flex-col">
            <div className="flex shrink-0 flex-wrap items-center justify-between gap-2">
              <div
                data-slot="group-summary-pill"
                className="inline-flex h-7 items-center justify-center gap-1.5 rounded-md bg-accent px-3 text-sm font-medium whitespace-nowrap text-accent-foreground"
              >
                <span>{t("groups.pageTitle")}</span>
                <span
                  data-slot="group-summary-count"
                  className="text-xs tabular-nums"
                >
                  {groups.length}
                </span>
              </div>
              <RippleButton
                size="sm"
                onClick={() => {
                  setCreateDialogOpen(true);
                }}
                className="flex shrink-0 items-center gap-2"
                aria-label={t("common.buttons.create")}
              >
                <GoPlus className="size-4" />
                <span className="hidden @2xl:inline">
                  {t("common.buttons.create")}
                </span>
              </RippleButton>
            </div>

            {error && (
              <div className="mt-4 rounded-md bg-destructive/10 p-3 text-sm text-destructive-text">
                {error}
              </div>
            )}

            {/* Groups list */}
            <ManagedDataTable
              table={table}
              tools={tableTools}
              loading={isLoading}
              emptyText={t("groups.noGroupsDescription")}
            />
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
          >
            <LuRefreshCw />
          </DataTableActionBarAction>
          <DataTableActionBarAction
            tooltip={t("common.buttons.delete")}
            onClick={() => setBulkDeleteOpen(true)}
            variant="destructive"
            className="bg-destructive/12 text-destructive-text hover:bg-destructive/20"
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

      <GroupBookmarksDialog
        isOpen={bookmarksDialogOpen}
        onClose={() => {
          setBookmarksDialogOpen(false);
        }}
        groupId={selectedGroup?.id ?? null}
        groupName={selectedGroup?.name ?? ""}
        onBookmarksSaved={() => {
          void loadGroups();
          onGroupManagementComplete();
        }}
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

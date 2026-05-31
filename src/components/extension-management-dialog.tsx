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
import { FaChrome, FaFirefox } from "react-icons/fa";
import { GoPlus } from "react-icons/go";
import {
  LuChevronDown,
  LuChevronUp,
  LuExternalLink,
  LuPencil,
  LuPuzzle,
  LuRefreshCw,
  LuTrash2,
  LuUpload,
} from "react-icons/lu";
import {
  DataTableActionBar,
  DataTableActionBarAction,
  DataTableActionBarSelection,
} from "@/components/data-table-action-bar";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ProBadge } from "@/components/ui/pro-badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
import type { Extension, ExtensionGroup } from "@/types";
import { DeleteConfirmationDialog } from "./delete-confirmation-dialog";
import { RippleButton } from "./ui/ripple";

type SyncStatus = "disabled" | "syncing" | "synced" | "error" | "waiting";

function getSyncStatusDot(
  item: { sync_enabled?: boolean; last_sync?: number },
  liveStatus: SyncStatus | undefined,
  t: (key: string, options?: Record<string, unknown>) => string,
): { color: string; tooltip: string; animate: boolean } {
  const status = liveStatus ?? (item.sync_enabled ? "synced" : "disabled");

  switch (status) {
    case "syncing":
      return {
        color: "bg-warning",
        tooltip: t("profileTable.syncTooltipSyncing"),
        animate: true,
      };
    case "synced":
      return {
        color: "bg-success",
        tooltip: item.last_sync
          ? t("profileTable.syncTooltipSyncedAt", {
              time: new Date(item.last_sync * 1000).toLocaleString(),
            })
          : t("profileTable.syncTooltipSynced"),
        animate: false,
      };
    case "waiting":
      return {
        color: "bg-warning",
        tooltip: t("profileTable.syncTooltipWaiting"),
        animate: false,
      };
    case "error":
      return {
        color: "bg-destructive",
        tooltip: t("profileTable.syncTooltipError"),
        animate: false,
      };
    default:
      return {
        color: "bg-muted-foreground",
        tooltip: t("profileTable.syncTooltipNotSynced"),
        animate: false,
      };
  }
}

interface ExtensionManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
  limitedMode: boolean;
  subPage?: boolean;
  /** Which tab is displayed when the dialog mounts; defaults to "extensions". */
  initialTab?: "extensions" | "groups";
}

export function ExtensionManagementDialog({
  isOpen,
  onClose,
  limitedMode,
  subPage,
  initialTab = "extensions",
}: ExtensionManagementDialogProps) {
  const { t } = useTranslation();
  const [extensions, setExtensions] = useState<Extension[]>([]);
  const [extensionGroups, setExtensionGroups] = useState<ExtensionGroup[]>([]);
  const [isLoading, setIsLoading] = useState(false);

  // Extension upload state
  const [isUploading, setIsUploading] = useState(false);
  const [extensionName, setExtensionName] = useState("");
  const [showUploadForm, setShowUploadForm] = useState(false);
  const [pendingFile, setPendingFile] = useState<{
    name: string;
    data: number[];
  } | null>(null);

  // Group state
  const [showCreateGroup, setShowCreateGroup] = useState(false);
  const [newGroupName, setNewGroupName] = useState("");
  const [editingGroup, setEditingGroup] = useState<ExtensionGroup | null>(null);
  const [editGroupName, setEditGroupName] = useState("");
  const [editGroupExtensionIds, setEditGroupExtensionIds] = useState<string[]>(
    [],
  );

  // Delete state
  const [extensionToDelete, setExtensionToDelete] = useState<Extension | null>(
    null,
  );
  const [groupToDelete, setGroupToDelete] = useState<ExtensionGroup | null>(
    null,
  );
  const [isDeleting, setIsDeleting] = useState(false);

  // Bulk delete state
  const [bulkExtDeleteOpen, setBulkExtDeleteOpen] = useState(false);
  const [bulkGroupDeleteOpen, setBulkGroupDeleteOpen] = useState(false);

  // Table state
  const [extSorting, setExtSorting] = useState<SortingState>([]);
  const [extRowSelection, setExtRowSelection] = useState<RowSelectionState>({});
  const [groupSorting, setGroupSorting] = useState<SortingState>([]);
  const [groupRowSelection, setGroupRowSelection] = useState<RowSelectionState>(
    {},
  );

  // Edit extension state
  const [editingExtension, setEditingExtension] = useState<Extension | null>(
    null,
  );
  const [editExtensionName, setEditExtensionName] = useState("");
  const [pendingUpdateFile, setPendingUpdateFile] = useState<{
    name: string;
    data: number[];
  } | null>(null);

  // Extension icons
  const [extensionIcons, setExtensionIcons] = useState<Record<string, string>>(
    {},
  );

  // Sync state
  const [extSyncStatus, setExtSyncStatus] = useState<
    Record<string, SyncStatus>
  >({});
  const [isTogglingExtSync, setIsTogglingExtSync] = useState<
    Record<string, boolean>
  >({});
  const [isTogglingGroupSync, setIsTogglingGroupSync] = useState<
    Record<string, boolean>
  >({});

  // Tab — keyed off `initialTab` so remounting the dialog with a new initial
  // tab (e.g. via the Mod+E shortcut toggle) jumps to that tab.
  const [activeTab, setActiveTab] = useState<"extensions" | "groups">(
    initialTab,
  );

  const loadData = useCallback(async () => {
    if (limitedMode) return;
    setIsLoading(true);
    try {
      const [exts, groups] = await Promise.all([
        invoke<Extension[]>("list_extensions"),
        invoke<ExtensionGroup[]>("list_extension_groups"),
      ]);
      setExtensions(exts);
      setExtensionGroups(groups);
    } catch {
      // User may not have pro subscription
      setExtensions([]);
      setExtensionGroups([]);
    } finally {
      setIsLoading(false);
    }
  }, [limitedMode]);

  const loadIcons = useCallback(async (exts: Extension[]) => {
    const icons: Record<string, string> = {};
    for (const ext of exts) {
      try {
        const icon = await invoke<string | null>("get_extension_icon", {
          extensionId: ext.id,
        });
        if (icon) {
          icons[ext.id] = icon;
        }
      } catch {
        // Icon not available
      }
    }
    setExtensionIcons(icons);
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadData();
    } else {
      // Drop selection when the dialog closes so the floating action
      // bars (portaled to body) don't linger on the page.
      setExtRowSelection({});
      setGroupRowSelection({});
    }
  }, [isOpen, loadData]);

  useEffect(() => {
    if (extensions.length > 0) {
      void loadIcons(extensions);
    }
  }, [extensions, loadIcons]);

  // Listen for extension sync status events
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      unlisten = await listen<{ id: string; status: string }>(
        "extension-sync-status",
        (event) => {
          const { id, status } = event.payload;
          setExtSyncStatus((prev) => ({
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

  const handleToggleExtSync = useCallback(
    async (ext: Extension) => {
      setIsTogglingExtSync((prev) => ({ ...prev, [ext.id]: true }));
      try {
        await invoke("set_extension_sync_enabled", {
          extensionId: ext.id,
          enabled: !ext.sync_enabled,
        });
        showSuccessToast(
          ext.sync_enabled
            ? t("extensions.syncDisabled")
            : t("extensions.syncEnabled"),
        );
        void loadData();
      } catch (err) {
        showErrorToast(
          parseBackendError(err)
            ? translateBackendError(t, err)
            : t("proxies.management.updateSyncFailed"),
        );
      } finally {
        setIsTogglingExtSync((prev) => ({ ...prev, [ext.id]: false }));
      }
    },
    [loadData, t],
  );

  const handleToggleGroupSync = useCallback(
    async (group: ExtensionGroup) => {
      setIsTogglingGroupSync((prev) => ({ ...prev, [group.id]: true }));
      try {
        await invoke("set_extension_group_sync_enabled", {
          extensionGroupId: group.id,
          enabled: !group.sync_enabled,
        });
        showSuccessToast(
          group.sync_enabled
            ? t("extensions.syncDisabled")
            : t("extensions.syncEnabled"),
        );
        void loadData();
      } catch (err) {
        showErrorToast(
          parseBackendError(err)
            ? translateBackendError(t, err)
            : t("proxies.management.updateSyncFailed"),
        );
      } finally {
        setIsTogglingGroupSync((prev) => ({ ...prev, [group.id]: false }));
      }
    },
    [loadData, t],
  );

  const handleUpdateExtension = useCallback(async () => {
    if (!editingExtension || !editExtensionName.trim()) return;
    try {
      await invoke("update_extension", {
        extensionId: editingExtension.id,
        name: editExtensionName.trim(),
        fileName: pendingUpdateFile?.name ?? null,
        fileData: pendingUpdateFile?.data ?? null,
      });
      showSuccessToast(t("extensions.updateSuccess"));
      setEditingExtension(null);
      setEditExtensionName("");
      setPendingUpdateFile(null);
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    }
  }, [editingExtension, editExtensionName, pendingUpdateFile, loadData, t]);

  const handleEditFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;

      const validExtensions = [".xpi", ".crx", ".zip"];
      const isValid = validExtensions.some((ext) =>
        file.name.toLowerCase().endsWith(ext),
      );
      if (!isValid) {
        showErrorToast(t("extensions.invalidFileType"));
        return;
      }

      const reader = new FileReader();
      reader.onload = (event) => {
        const arrayBuffer = event.target?.result as ArrayBuffer;
        const data = Array.from(new Uint8Array(arrayBuffer));
        setPendingUpdateFile({ name: file.name, data });
      };
      reader.readAsArrayBuffer(file);
      e.target.value = "";
    },
    [t],
  );

  const handleFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;

      const validExtensions = [".xpi", ".crx", ".zip"];
      const isValid = validExtensions.some((ext) =>
        file.name.toLowerCase().endsWith(ext),
      );
      if (!isValid) {
        showErrorToast(t("extensions.invalidFileType"));
        return;
      }

      const reader = new FileReader();
      reader.onload = (event) => {
        const arrayBuffer = event.target?.result as ArrayBuffer;
        const data = Array.from(new Uint8Array(arrayBuffer));
        const baseName = file.name
          .replace(/\.(xpi|crx|zip)$/i, "")
          .replace(/[-_]/g, " ");
        setExtensionName(baseName);
        setPendingFile({ name: file.name, data });
        setShowUploadForm(true);
      };
      reader.onerror = () => {
        showErrorToast(t("extensions.readError"));
      };
      reader.readAsArrayBuffer(file);

      // Reset input
      e.target.value = "";
    },
    [t],
  );

  const handleUpload = useCallback(async () => {
    if (!pendingFile || !extensionName.trim()) return;
    setIsUploading(true);
    try {
      await invoke("add_extension", {
        name: extensionName.trim(),
        fileName: pendingFile.name,
        fileData: pendingFile.data,
      });
      showSuccessToast(t("extensions.uploadSuccess"));
      setShowUploadForm(false);
      setPendingFile(null);
      setExtensionName("");
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    } finally {
      setIsUploading(false);
    }
  }, [pendingFile, extensionName, loadData, t]);

  const handleDeleteExtension = useCallback(async () => {
    if (!extensionToDelete) return;
    setIsDeleting(true);
    try {
      await invoke("delete_extension", { extensionId: extensionToDelete.id });
      showSuccessToast(t("extensions.deleteSuccess"));
      setExtensionToDelete(null);
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    } finally {
      setIsDeleting(false);
    }
  }, [extensionToDelete, loadData, t]);

  const handleCreateGroup = useCallback(async () => {
    if (!newGroupName.trim()) return;
    try {
      await invoke("create_extension_group", { name: newGroupName.trim() });
      showSuccessToast(t("extensions.groupCreateSuccess"));
      setShowCreateGroup(false);
      setNewGroupName("");
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    }
  }, [newGroupName, loadData, t]);

  const handleSaveGroupEdits = useCallback(async () => {
    if (!editingGroup || !editGroupName.trim()) return;
    try {
      // Update group name
      await invoke("update_extension_group", {
        groupId: editingGroup.id,
        name: editGroupName.trim(),
      });

      // Compute diff of extensions
      const originalIds = new Set(editingGroup.extension_ids);
      const newIds = new Set(editGroupExtensionIds);

      // Add new extensions
      for (const extId of editGroupExtensionIds) {
        if (!originalIds.has(extId)) {
          await invoke("add_extension_to_group", {
            groupId: editingGroup.id,
            extensionId: extId,
          });
        }
      }

      // Remove removed extensions
      for (const extId of editingGroup.extension_ids) {
        if (!newIds.has(extId)) {
          await invoke("remove_extension_from_group", {
            groupId: editingGroup.id,
            extensionId: extId,
          });
        }
      }

      showSuccessToast(t("extensions.groupUpdateSuccess"));
      setEditingGroup(null);
      setEditGroupName("");
      setEditGroupExtensionIds([]);
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    }
  }, [editingGroup, editGroupName, editGroupExtensionIds, loadData, t]);

  const handleDeleteGroup = useCallback(async () => {
    if (!groupToDelete) return;
    setIsDeleting(true);
    try {
      await invoke("delete_extension_group", { groupId: groupToDelete.id });
      showSuccessToast(t("extensions.groupDeleteSuccess"));
      setGroupToDelete(null);
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    } finally {
      setIsDeleting(false);
    }
  }, [groupToDelete, loadData, t]);

  const selectedExtensions = useMemo(
    () => extensions.filter((ext) => extRowSelection[ext.id]),
    [extensions, extRowSelection],
  );

  const selectedGroups = useMemo(
    () => extensionGroups.filter((group) => groupRowSelection[group.id]),
    [extensionGroups, groupRowSelection],
  );

  const handleBulkDeleteExtensions = useCallback(async () => {
    if (selectedExtensions.length === 0) return;
    setIsDeleting(true);
    try {
      await Promise.allSettled(
        selectedExtensions.map((ext) =>
          invoke("delete_extension", { extensionId: ext.id }),
        ),
      );
      showSuccessToast(t("extensions.deleteSuccess"));
      setBulkExtDeleteOpen(false);
      setExtRowSelection({});
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    } finally {
      setIsDeleting(false);
    }
  }, [selectedExtensions, loadData, t]);

  const handleBulkDeleteGroups = useCallback(async () => {
    if (selectedGroups.length === 0) return;
    setIsDeleting(true);
    try {
      await Promise.allSettled(
        selectedGroups.map((group) =>
          invoke("delete_extension_group", { groupId: group.id }),
        ),
      );
      showSuccessToast(t("extensions.groupDeleteSuccess"));
      setBulkGroupDeleteOpen(false);
      setGroupRowSelection({});
      void loadData();
    } catch (err) {
      showErrorToast(err instanceof Error ? err.message : String(err));
    } finally {
      setIsDeleting(false);
    }
  }, [selectedGroups, loadData, t]);

  const handleBulkToggleExtSync = useCallback(async () => {
    if (selectedExtensions.length === 0) return;
    const allOn = selectedExtensions.every((e) => e.sync_enabled);
    const targetEnabled = !allOn;
    const results = await Promise.allSettled(
      selectedExtensions.map((ext) =>
        invoke("set_extension_sync_enabled", {
          extensionId: ext.id,
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
          ? t("extensions.syncEnabled")
          : t("extensions.syncDisabled"),
      );
    }
    void loadData();
  }, [selectedExtensions, loadData, t]);

  const handleBulkToggleGroupSync = useCallback(async () => {
    if (selectedGroups.length === 0) return;
    const allOn = selectedGroups.every((g) => g.sync_enabled);
    const targetEnabled = !allOn;
    const results = await Promise.allSettled(
      selectedGroups.map((group) =>
        invoke("set_extension_group_sync_enabled", {
          extensionGroupId: group.id,
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
          ? t("extensions.syncEnabled")
          : t("extensions.syncDisabled"),
      );
    }
    void loadData();
  }, [selectedGroups, loadData, t]);

  const renderCompatIcons = useCallback(
    (compat: string[]) => {
      const hasChromium = compat.includes("chromium");
      const hasFirefox = compat.includes("firefox");
      if (!hasChromium && !hasFirefox) return null;
      return (
        <div className="flex items-center gap-1 shrink-0">
          {hasChromium && (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="inline-flex">
                  <FaChrome className="size-3.5 text-muted-foreground" />
                </span>
              </TooltipTrigger>
              <TooltipContent>
                {t("extensions.compatibility.chromium")}
              </TooltipContent>
            </Tooltip>
          )}
          {hasFirefox && (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="inline-flex">
                  <FaFirefox className="size-3.5 text-muted-foreground" />
                </span>
              </TooltipTrigger>
              <TooltipContent>
                {t("extensions.compatibility.firefox")}
              </TooltipContent>
            </Tooltip>
          )}
        </div>
      );
    },
    [t],
  );

  const renderExtensionIcon = useCallback(
    (ext: Extension, size: "sm" | "md" = "md") => {
      const sizeClass = size === "sm" ? "size-4" : "size-5";
      if (extensionIcons[ext.id]) {
        return (
          // biome-ignore lint/performance/noImgElement: base64 data URI icons cannot use next/image
          <img
            src={extensionIcons[ext.id]}
            alt=""
            className={`${sizeClass} shrink-0 rounded-sm`}
          />
        );
      }
      return (
        <LuPuzzle className={`${sizeClass} shrink-0 text-muted-foreground`} />
      );
    },
    [extensionIcons],
  );

  const MAX_VISIBLE_ICONS = 3;

  const extensionColumns = useMemo<ColumnDef<Extension>[]>(
    () => [
      {
        id: "select",
        size: 36,
        enableSorting: false,
        header: ({ table }) => (
          <Checkbox
            checked={
              table.getIsAllRowsSelected() ||
              (table.getIsSomeRowsSelected() && "indeterminate")
            }
            onCheckedChange={(value) => {
              table.toggleAllRowsSelected(!!value);
            }}
            aria-label={t("common.aria.selectAll")}
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
        id: "icon",
        size: 36,
        enableSorting: false,
        header: () => null,
        cell: ({ row }) => renderExtensionIcon(row.original, "sm"),
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
        cell: ({ row }) => (
          <span className="text-sm font-medium truncate min-w-0 block">
            {row.original.name}
          </span>
        ),
      },
      {
        id: "compat",
        enableSorting: false,
        header: () => null,
        cell: ({ row }) =>
          renderCompatIcons(row.original.browser_compatibility),
      },
      {
        id: "sync",
        size: 88,
        enableSorting: false,
        header: () => null,
        cell: ({ row }) => {
          const ext = row.original;
          const syncDot = getSyncStatusDot(ext, extSyncStatus[ext.id], t);
          return (
            <div className="flex items-center gap-2 shrink-0">
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
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="inline-flex items-center shrink-0">
                    <AnimatedSwitch
                      checked={ext.sync_enabled}
                      onCheckedChange={() => void handleToggleExtSync(ext)}
                      disabled={isTogglingExtSync[ext.id]}
                    />
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  <p>
                    {ext.sync_enabled
                      ? t("syncTooltips.disable")
                      : t("syncTooltips.enable")}
                  </p>
                </TooltipContent>
              </Tooltip>
            </div>
          );
        },
      },
      {
        id: "actions",
        enableSorting: false,
        header: () => null,
        cell: ({ row }) => {
          const ext = row.original;
          return (
            <div className="flex gap-0.5 justify-end shrink-0">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="size-7 p-0"
                    onClick={() => {
                      setEditingExtension(ext);
                      setEditExtensionName(ext.name);
                      setPendingUpdateFile(null);
                    }}
                  >
                    <LuPencil className="size-3.5" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("extensions.editExtension")}</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="size-7 p-0"
                    onClick={() => {
                      setExtensionToDelete(ext);
                    }}
                  >
                    <LuTrash2 className="size-3.5" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("extensions.delete")}</TooltipContent>
              </Tooltip>
            </div>
          );
        },
      },
    ],
    [
      t,
      extSyncStatus,
      isTogglingExtSync,
      handleToggleExtSync,
      renderExtensionIcon,
      renderCompatIcons,
    ],
  );

  const extTable = useReactTable({
    data: extensions,
    columns: extensionColumns,
    state: { sorting: extSorting, rowSelection: extRowSelection },
    onSortingChange: setExtSorting,
    onRowSelectionChange: setExtRowSelection,
    enableRowSelection: () => !limitedMode,
    getSortedRowModel: getSortedRowModel(),
    getCoreRowModel: getCoreRowModel(),
    getRowId: (row) => row.id,
  });

  const groupColumns = useMemo<ColumnDef<ExtensionGroup>[]>(
    () => [
      {
        id: "select",
        size: 36,
        enableSorting: false,
        header: ({ table }) => (
          <Checkbox
            checked={
              table.getIsAllRowsSelected() ||
              (table.getIsSomeRowsSelected() && "indeterminate")
            }
            onCheckedChange={(value) => {
              table.toggleAllRowsSelected(!!value);
            }}
            aria-label={t("common.aria.selectAll")}
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
        cell: ({ row }) => (
          <span className="font-medium text-sm truncate min-w-0 block">
            {row.original.name}
          </span>
        ),
      },
      {
        id: "extensions",
        enableSorting: false,
        header: () => null,
        cell: ({ row }) => {
          const group = row.original;
          const groupExts = group.extension_ids
            .map((id) => extensions.find((e) => e.id === id))
            .filter(Boolean) as Extension[];
          const visibleExts = groupExts.slice(0, MAX_VISIBLE_ICONS);
          const overflowCount = groupExts.length - MAX_VISIBLE_ICONS;
          return (
            <div className="flex items-center gap-1 shrink-0">
              {visibleExts.map((ext) => (
                <Tooltip key={ext.id}>
                  <TooltipTrigger asChild>
                    <span className="inline-flex">
                      {renderExtensionIcon(ext, "sm")}
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>{ext.name}</TooltipContent>
                </Tooltip>
              ))}
              {overflowCount > 0 && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Badge
                      variant="secondary"
                      className="text-xs h-5 px-1.5 shrink-0"
                    >
                      +{overflowCount}
                    </Badge>
                  </TooltipTrigger>
                  <TooltipContent>
                    <div className="space-y-0.5">
                      {groupExts.slice(MAX_VISIBLE_ICONS).map((ext) => (
                        <p key={ext.id} className="text-xs">
                          {ext.name}
                        </p>
                      ))}
                    </div>
                  </TooltipContent>
                </Tooltip>
              )}
              {groupExts.length === 0 && (
                <span className="text-xs text-muted-foreground">
                  {t("extensions.noExtensionsInGroup")}
                </span>
              )}
            </div>
          );
        },
      },
      {
        id: "sync",
        size: 88,
        enableSorting: false,
        header: () => null,
        cell: ({ row }) => {
          const group = row.original;
          const groupSyncDot = getSyncStatusDot(
            group,
            extSyncStatus[group.id],
            t,
          );
          return (
            <div className="flex items-center gap-2 shrink-0">
              <Tooltip>
                <TooltipTrigger asChild>
                  <div
                    className={`size-2 rounded-full shrink-0 ${groupSyncDot.color} ${
                      groupSyncDot.animate ? "animate-pulse" : ""
                    }`}
                  />
                </TooltipTrigger>
                <TooltipContent>
                  <p>{groupSyncDot.tooltip}</p>
                </TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="inline-flex items-center shrink-0">
                    <AnimatedSwitch
                      checked={group.sync_enabled}
                      onCheckedChange={() => void handleToggleGroupSync(group)}
                      disabled={isTogglingGroupSync[group.id]}
                    />
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  <p>
                    {group.sync_enabled
                      ? t("syncTooltips.disable")
                      : t("syncTooltips.enable")}
                  </p>
                </TooltipContent>
              </Tooltip>
            </div>
          );
        },
      },
      {
        id: "actions",
        enableSorting: false,
        header: () => null,
        cell: ({ row }) => {
          const group = row.original;
          return (
            <div className="flex gap-0.5 justify-end shrink-0">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="size-7 p-0"
                    onClick={() => {
                      setEditingGroup(group);
                      setEditGroupName(group.name);
                      setEditGroupExtensionIds([...group.extension_ids]);
                    }}
                  >
                    <LuPencil className="size-3.5" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("common.buttons.edit")}</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="size-7 p-0"
                    onClick={() => {
                      setGroupToDelete(group);
                    }}
                  >
                    <LuTrash2 className="size-3.5" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("extensions.deleteGroup")}</TooltipContent>
              </Tooltip>
            </div>
          );
        },
      },
    ],
    [
      t,
      extensions,
      extSyncStatus,
      isTogglingGroupSync,
      handleToggleGroupSync,
      renderExtensionIcon,
    ],
  );

  const groupTable = useReactTable({
    data: extensionGroups,
    columns: groupColumns,
    state: { sorting: groupSorting, rowSelection: groupRowSelection },
    onSortingChange: setGroupSorting,
    onRowSelectionChange: setGroupRowSelection,
    enableRowSelection: () => !limitedMode,
    getSortedRowModel: getSortedRowModel(),
    getCoreRowModel: getCoreRowModel(),
    getRowId: (row) => row.id,
  });

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
        <DialogContent className="max-w-4xl max-h-[90vh] flex flex-col">
          {!subPage && (
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <LuPuzzle className="size-5" />
                {t("extensions.title")}
                {limitedMode && <ProBadge />}
              </DialogTitle>
              <DialogDescription>
                {t("extensions.description")}
              </DialogDescription>
            </DialogHeader>
          )}

          <div className="relative flex-1 min-h-0 flex flex-col">
            {limitedMode && (
              <>
                <div className="absolute inset-0 backdrop-blur-[6px] bg-background/30 z-[1]" />
                <div className="absolute inset-y-0 left-0 w-6 bg-linear-to-r from-background to-transparent z-[2]" />
                <div className="absolute inset-y-0 right-0 w-6 bg-linear-to-l from-background to-transparent z-[2]" />
                <div className="absolute inset-x-0 top-0 h-6 bg-linear-to-b from-background to-transparent z-[2]" />
                <div className="absolute inset-x-0 bottom-0 h-6 bg-linear-to-t from-background to-transparent z-[2]" />
                <div className="absolute inset-0 flex items-center justify-center z-[3]">
                  <div className="flex items-center gap-2 rounded-md bg-background/80 px-3 py-1.5">
                    <ProBadge />
                    <span className="text-sm font-medium text-muted-foreground">
                      {t("extensions.proRequired")}
                    </span>
                  </div>
                </div>
              </>
            )}

            <AnimatedTabs
              key={initialTab}
              value={activeTab}
              onValueChange={(v) => setActiveTab(v as "extensions" | "groups")}
              className="flex-1 min-h-0 flex flex-col"
            >
              <div className="flex items-center justify-between gap-3 shrink-0">
                <AnimatedTabsList>
                  <AnimatedTabsTrigger
                    value="extensions"
                    disabled={limitedMode}
                  >
                    <span>{t("extensions.extensionsTab")}</span>
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {extensions.length}
                    </span>
                  </AnimatedTabsTrigger>
                  <AnimatedTabsTrigger value="groups" disabled={limitedMode}>
                    <span>{t("extensions.groupsTab")}</span>
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {extensionGroups.length}
                    </span>
                  </AnimatedTabsTrigger>
                </AnimatedTabsList>
                <div className="flex items-center gap-2">
                  {activeTab === "extensions" && (
                    <RippleButton
                      size="sm"
                      variant="outline"
                      disabled={limitedMode}
                      onClick={() =>
                        document.getElementById("ext-file-input")?.click()
                      }
                    >
                      <LuUpload className="size-4" />
                      {t("extensions.upload")}
                    </RippleButton>
                  )}
                  {activeTab === "groups" && (
                    <RippleButton
                      size="sm"
                      disabled={limitedMode}
                      onClick={() => setShowCreateGroup(true)}
                    >
                      <GoPlus className="size-4" />
                      {t("extensions.newGroup")}
                    </RippleButton>
                  )}
                </div>
              </div>

              {/* Notice */}
              <div className="rounded-md bg-muted/50 p-3 text-sm text-muted-foreground mt-4 shrink-0">
                {t("extensions.managedNotice")}
              </div>

              <AnimatedTabsContent
                value="extensions"
                className="mt-4 flex-1 min-h-0 data-[state=active]:flex flex-col"
              >
                <div className="flex flex-col gap-4 flex-1 min-h-0">
                  <Input
                    id="ext-file-input"
                    type="file"
                    accept=".xpi,.crx,.zip"
                    className="hidden"
                    onChange={handleFileSelect}
                    disabled={limitedMode}
                  />

                  {/* Upload form */}
                  {showUploadForm && pendingFile && (
                    <div className="space-y-3 rounded-md border p-3">
                      <div className="text-sm text-muted-foreground">
                        {t("extensions.selectedFile")}:{" "}
                        <span className="font-medium text-foreground">
                          {pendingFile.name}
                        </span>
                      </div>
                      <div className="flex gap-2">
                        <Input
                          value={extensionName}
                          onChange={(e) => {
                            setExtensionName(e.target.value);
                          }}
                          placeholder={t("extensions.namePlaceholder")}
                          className="flex-1"
                        />
                        <RippleButton
                          size="sm"
                          onClick={() => void handleUpload()}
                          disabled={isUploading || !extensionName.trim()}
                        >
                          {isUploading
                            ? t("common.buttons.loading")
                            : t("common.buttons.add")}
                        </RippleButton>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => {
                            setShowUploadForm(false);
                            setPendingFile(null);
                            setExtensionName("");
                          }}
                        >
                          {t("common.buttons.cancel")}
                        </Button>
                      </div>
                    </div>
                  )}

                  {/* Extensions list */}
                  {isLoading ? (
                    <div className="text-sm text-muted-foreground">
                      {t("common.buttons.loading")}
                    </div>
                  ) : extensions.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      {t("extensions.empty")}
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
                          {extTable.getHeaderGroups().map((headerGroup) => (
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
                          {extTable.getRowModel().rows.map((row) => (
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
              </AnimatedTabsContent>

              <AnimatedTabsContent
                value="groups"
                className="mt-4 flex-1 min-h-0 data-[state=active]:flex flex-col"
              >
                <div className="flex flex-col gap-4 flex-1 min-h-0">
                  {/* Create group form */}
                  {showCreateGroup && (
                    <div className="flex gap-2 items-center">
                      <Input
                        value={newGroupName}
                        onChange={(e) => {
                          setNewGroupName(e.target.value);
                        }}
                        placeholder={t("extensions.groupNamePlaceholder")}
                        className="flex-1"
                        onKeyDown={(e) => {
                          if (e.key === "Enter") void handleCreateGroup();
                        }}
                      />
                      <RippleButton
                        size="sm"
                        onClick={() => void handleCreateGroup()}
                        disabled={!newGroupName.trim()}
                      >
                        {t("common.buttons.create")}
                      </RippleButton>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => {
                          setShowCreateGroup(false);
                          setNewGroupName("");
                        }}
                      >
                        {t("common.buttons.cancel")}
                      </Button>
                    </div>
                  )}

                  {/* Groups list */}
                  {extensionGroups.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      {t("extensions.noGroups")}
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
                          {groupTable.getHeaderGroups().map((headerGroup) => (
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
                          {groupTable.getRowModel().rows.map((row) => (
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
              </AnimatedTabsContent>
            </AnimatedTabs>
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

      {/* Group editing dialog */}
      <Dialog
        open={editingGroup !== null}
        onOpenChange={(open) => {
          if (!open) {
            setEditingGroup(null);
            setEditGroupName("");
            setEditGroupExtensionIds([]);
          }
        }}
      >
        <DialogContent className="max-w-lg max-h-[90vh] flex flex-col">
          <DialogHeader>
            <DialogTitle>{t("extensions.editGroup")}</DialogTitle>
            <DialogDescription>
              {t("extensions.editGroupDescription")}
            </DialogDescription>
          </DialogHeader>

          <ScrollArea className="overflow-y-auto flex-1 -mx-6 px-6">
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>{t("common.labels.name")}</Label>
                <Input
                  value={editGroupName}
                  onChange={(e) => {
                    setEditGroupName(e.target.value);
                  }}
                  placeholder={t("extensions.groupNamePlaceholder")}
                />
              </div>

              {extensions.filter((e) => !editGroupExtensionIds.includes(e.id))
                .length > 0 && (
                <div className="space-y-2">
                  <Label>{t("extensions.addToGroup")}</Label>
                  <Select
                    value=""
                    onValueChange={(extId) => {
                      setEditGroupExtensionIds((prev) => [...prev, extId]);
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue placeholder={t("extensions.addToGroup")} />
                    </SelectTrigger>
                    <SelectContent>
                      {extensions
                        .filter((e) => !editGroupExtensionIds.includes(e.id))
                        .map((ext) => (
                          <SelectItem key={ext.id} value={ext.id}>
                            <div className="flex items-center gap-2">
                              {renderExtensionIcon(ext, "sm")}
                              {ext.name}
                            </div>
                          </SelectItem>
                        ))}
                    </SelectContent>
                  </Select>
                </div>
              )}

              <div className="space-y-2">
                <Label>{t("extensions.groupExtensions")}</Label>
                {editGroupExtensionIds.length === 0 ? (
                  <div className="text-sm text-muted-foreground py-2">
                    {t("extensions.noExtensionsInGroup")}
                  </div>
                ) : (
                  <div className="space-y-1 max-h-[200px] overflow-y-auto">
                    {editGroupExtensionIds.map((extId) => {
                      const ext = extensions.find((e) => e.id === extId);
                      if (!ext) return null;
                      return (
                        <div
                          key={extId}
                          className="flex items-center gap-2 rounded-md border px-2 py-1.5"
                        >
                          {renderExtensionIcon(ext, "sm")}
                          <span className="text-sm flex-1 truncate min-w-0">
                            {ext.name}
                          </span>
                          {renderCompatIcons(ext.browser_compatibility)}
                          <Button
                            variant="ghost"
                            size="sm"
                            className="size-6 p-0 shrink-0"
                            onClick={() => {
                              setEditGroupExtensionIds((prev) =>
                                prev.filter((id) => id !== extId),
                              );
                            }}
                          >
                            <LuTrash2 className="size-3" />
                          </Button>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>
          </ScrollArea>

          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setEditingGroup(null);
                setEditGroupName("");
                setEditGroupExtensionIds([]);
              }}
            >
              {t("common.buttons.cancel")}
            </Button>
            <RippleButton
              onClick={() => void handleSaveGroupEdits()}
              disabled={!editGroupName.trim()}
            >
              {t("common.buttons.save")}
            </RippleButton>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Extension editing dialog */}
      <Dialog
        open={editingExtension !== null}
        onOpenChange={(open) => {
          if (!open) {
            setEditingExtension(null);
            setEditExtensionName("");
            setPendingUpdateFile(null);
          }
        }}
      >
        <DialogContent className="max-w-lg max-h-[90vh] flex flex-col">
          <DialogHeader>
            <DialogTitle>{t("extensions.editExtension")}</DialogTitle>
            <DialogDescription>
              {t("extensions.editExtensionDescription")}
            </DialogDescription>
          </DialogHeader>

          <ScrollArea className="overflow-y-auto flex-1 -mx-6 px-6">
            {editingExtension && (
              <div className="space-y-4">
                <div className="space-y-2">
                  <Label>{t("common.labels.name")}</Label>
                  <Input
                    value={editExtensionName}
                    onChange={(e) => {
                      setEditExtensionName(e.target.value);
                    }}
                    placeholder={t("extensions.namePlaceholder")}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") void handleUpdateExtension();
                    }}
                  />
                </div>

                {/* Metadata from manifest.json */}
                <div className="rounded-md border p-3 space-y-2">
                  <Label className="text-xs text-muted-foreground uppercase tracking-wide">
                    {t("extensions.metadata")}
                  </Label>
                  <div className="grid grid-cols-[auto,1fr] gap-x-3 gap-y-1.5 text-sm">
                    {editingExtension.version && (
                      <>
                        <span className="text-muted-foreground">
                          {t("extensions.version")}
                        </span>
                        <span>{editingExtension.version}</span>
                      </>
                    )}
                    {editingExtension.author && (
                      <>
                        <span className="text-muted-foreground">
                          {t("extensions.author")}
                        </span>
                        <span>{editingExtension.author}</span>
                      </>
                    )}
                    {editingExtension.description && (
                      <>
                        <span className="text-muted-foreground">
                          {t("common.labels.description")}
                        </span>
                        <span className="line-clamp-3">
                          {editingExtension.description}
                        </span>
                      </>
                    )}
                    <span className="text-muted-foreground">
                      {t("extensions.compatibility.label")}
                    </span>
                    <div className="flex items-center gap-1">
                      {renderCompatIcons(
                        editingExtension.browser_compatibility,
                      )}
                    </div>
                    <span className="text-muted-foreground">
                      {t("common.labels.type")}
                    </span>
                    <span>.{editingExtension.file_type}</span>
                    {editingExtension.homepage_url && (
                      <>
                        <span className="text-muted-foreground">
                          {t("extensions.homepage")}
                        </span>
                        <a
                          href={editingExtension.homepage_url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-primary hover:underline flex items-center gap-1 truncate"
                        >
                          <span className="truncate">
                            {editingExtension.homepage_url}
                          </span>
                          <LuExternalLink className="size-3 shrink-0" />
                        </a>
                      </>
                    )}
                    {!editingExtension.version &&
                      !editingExtension.author &&
                      !editingExtension.description &&
                      !editingExtension.homepage_url && (
                        <span className="col-span-2 text-muted-foreground text-xs">
                          {t("extensions.noMetadata")}
                        </span>
                      )}
                  </div>
                </div>

                {/* Re-upload */}
                <div className="space-y-2">
                  <Label>{t("extensions.reupload")}</Label>
                  <div className="flex gap-2 items-center">
                    <RippleButton
                      size="sm"
                      variant="outline"
                      onClick={() =>
                        document.getElementById("ext-edit-file-input")?.click()
                      }
                    >
                      <LuUpload className="size-3 mr-1" />
                      {t("extensions.selectFile")}
                    </RippleButton>
                    <input
                      id="ext-edit-file-input"
                      type="file"
                      accept=".xpi,.crx,.zip"
                      className="hidden"
                      onChange={handleEditFileSelect}
                    />
                    {pendingUpdateFile && (
                      <span className="text-xs text-muted-foreground truncate max-w-[200px]">
                        {pendingUpdateFile.name}
                      </span>
                    )}
                  </div>
                </div>
              </div>
            )}
          </ScrollArea>

          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setEditingExtension(null);
                setEditExtensionName("");
                setPendingUpdateFile(null);
              }}
            >
              {t("common.buttons.cancel")}
            </Button>
            <RippleButton
              onClick={() => void handleUpdateExtension()}
              disabled={!editExtensionName.trim()}
            >
              {t("common.buttons.save")}
            </RippleButton>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete extension confirmation */}
      <DeleteConfirmationDialog
        isOpen={extensionToDelete !== null}
        onClose={() => {
          setExtensionToDelete(null);
        }}
        onConfirm={handleDeleteExtension}
        title={t("extensions.deleteConfirmTitle")}
        description={t("extensions.deleteConfirmDescription", {
          name: extensionToDelete?.name ?? "",
        })}
        isLoading={isDeleting}
      />

      {/* Delete group confirmation */}
      <DeleteConfirmationDialog
        isOpen={groupToDelete !== null}
        onClose={() => {
          setGroupToDelete(null);
        }}
        onConfirm={handleDeleteGroup}
        title={t("extensions.deleteGroupConfirmTitle")}
        description={t("extensions.deleteGroupConfirmDescription", {
          name: groupToDelete?.name ?? "",
        })}
        isLoading={isDeleting}
      />

      {/* Bulk delete extensions confirmation */}
      <DeleteConfirmationDialog
        isOpen={bulkExtDeleteOpen}
        onClose={() => {
          setBulkExtDeleteOpen(false);
        }}
        onConfirm={handleBulkDeleteExtensions}
        title={t("extensions.bulkDelete.extensionsTitle")}
        description={t("extensions.bulkDelete.extensionsDescription", {
          count: selectedExtensions.length,
          names: selectedExtensions.map((ext) => ext.name).join(", "),
        })}
        confirmButtonText={t("extensions.bulkDelete.confirmButton")}
        isLoading={isDeleting}
      />

      {/* Bulk delete groups confirmation */}
      <DeleteConfirmationDialog
        isOpen={bulkGroupDeleteOpen}
        onClose={() => {
          setBulkGroupDeleteOpen(false);
        }}
        onConfirm={handleBulkDeleteGroups}
        title={t("extensions.bulkDelete.groupsTitle")}
        description={t("extensions.bulkDelete.groupsDescription", {
          count: selectedGroups.length,
          names: selectedGroups.map((group) => group.name).join(", "),
        })}
        confirmButtonText={t("extensions.bulkDelete.confirmButton")}
        isLoading={isDeleting}
      />

      {/* Bulk action bars — only mount the active tab's bar; an always-
          mounted DataTableActionBar (even with visible=false) keeps an
          AnimatePresence wrapper alive that intermittently captured pointer
          input on the proxy/extension subpages. */}
      {isOpen && activeTab === "extensions" && (
        <DataTableActionBar table={extTable}>
          <DataTableActionBarSelection table={extTable} />
          <DataTableActionBarAction
            tooltip={t("syncTooltips.bulkToggle")}
            size="icon"
            onClick={() => {
              void handleBulkToggleExtSync();
            }}
          >
            <LuRefreshCw />
          </DataTableActionBarAction>
          <DataTableActionBarAction
            tooltip={t("common.buttons.delete")}
            variant="destructive"
            size="icon"
            className="border-destructive bg-destructive/50 hover:bg-destructive/70"
            onClick={() => {
              setBulkExtDeleteOpen(true);
            }}
          >
            <LuTrash2 />
          </DataTableActionBarAction>
        </DataTableActionBar>
      )}

      {isOpen && activeTab === "groups" && (
        <DataTableActionBar table={groupTable}>
          <DataTableActionBarSelection table={groupTable} />
          <DataTableActionBarAction
            tooltip={t("syncTooltips.bulkToggle")}
            size="icon"
            onClick={() => {
              void handleBulkToggleGroupSync();
            }}
          >
            <LuRefreshCw />
          </DataTableActionBarAction>
          <DataTableActionBarAction
            tooltip={t("common.buttons.delete")}
            variant="destructive"
            size="icon"
            className="border-destructive bg-destructive/50 hover:bg-destructive/70"
            onClick={() => {
              setBulkGroupDeleteOpen(true);
            }}
          >
            <LuTrash2 />
          </DataTableActionBarAction>
        </DataTableActionBar>
      )}
    </>
  );
}

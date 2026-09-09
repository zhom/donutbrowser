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
import { emit, listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import {
  LuChevronDown,
  LuChevronUp,
  LuDownload,
  LuPencil,
  LuRefreshCw,
  LuTrash2,
  LuUpload,
} from "react-icons/lu";
import { toast } from "sonner";
import {
  DataTableActionBar,
  DataTableActionBarAction,
  DataTableActionBarSelection,
} from "@/components/data-table-action-bar";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { ProfileUsageButton } from "@/components/profile-usage-button";
import { ProxyExportDialog } from "@/components/proxy-export-dialog";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { ProxyImportDialog } from "@/components/proxy-import-dialog";
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
import { useProfileReferences } from "@/hooks/use-profile-references";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { useVpnEvents } from "@/hooks/use-vpn-events";
import { parseBackendError, translateBackendError } from "@/lib/backend-errors";
import { isFirstHopEncrypted, proxyProtocolToken } from "@/lib/proxy-string";
import { canonicalProxyType } from "@/lib/proxy-type";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { StoredProxy, VpnConfig } from "@/types";
import { ProxyCheckButton, ProxyUdpBadge } from "./proxy-check-button";
import { RippleButton } from "./ui/ripple";
import { VpnCheckButton } from "./vpn-check-button";
import { VpnFormDialog } from "./vpn-form-dialog";
import { VpnImportDialog } from "./vpn-import-dialog";

type SyncStatus = "disabled" | "syncing" | "synced" | "error" | "waiting";

function getSyncStatusDot(
  item: { sync_enabled?: boolean; last_sync?: number },
  liveStatus: SyncStatus | undefined,
  t: (key: string, options?: Record<string, unknown>) => string,
  errorMessage?: string,
): { color: string; tooltip: string; animate: boolean } {
  const status = liveStatus ?? (item.sync_enabled ? "synced" : "disabled");

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
        tooltip: item.last_sync
          ? t("syncTooltips.syncedAt", {
              time: new Date(item.last_sync * 1000).toLocaleString(),
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

interface ProxyManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  /** Which tab to display first when the dialog mounts; defaults to "proxies". */
  initialTab?: "proxies" | "vpns";
}

export function ProxyManagementDialog({
  isOpen,
  onClose,
  subPage,
  initialTab = "proxies",
}: ProxyManagementDialogProps) {
  const { t } = useTranslation();
  const { profiles: referencedProfiles, failed: referencesFailed } =
    useProfileReferences(isOpen);
  // Proxy state
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [showImportDialog, setShowImportDialog] = useState(false);
  const [showExportDialog, setShowExportDialog] = useState(false);
  const [editingProxy, setEditingProxy] = useState<StoredProxy | null>(null);
  const [proxyToDelete, setProxyToDelete] = useState<StoredProxy | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [proxySyncStatus, setProxySyncStatus] = useState<
    Record<string, SyncStatus>
  >({});
  const [proxySyncErrors, setProxySyncErrors] = useState<
    Record<string, string>
  >({});
  const [proxyInUse, setProxyInUse] = useState<Record<string, boolean>>({});
  const [isTogglingSync, setIsTogglingSync] = useState<Record<string, boolean>>(
    {},
  );

  // VPN state
  const [showVpnForm, setShowVpnForm] = useState(false);
  const [showVpnImportDialog, setShowVpnImportDialog] = useState(false);
  const [editingVpn, setEditingVpn] = useState<VpnConfig | null>(null);
  const [vpnToDelete, setVpnToDelete] = useState<VpnConfig | null>(null);
  const [isDeletingVpn, setIsDeletingVpn] = useState(false);
  const [checkingVpnId, setCheckingVpnId] = useState<string | null>(null);
  const [vpnSyncStatus, setVpnSyncStatus] = useState<
    Record<string, SyncStatus>
  >({});
  const [vpnSyncErrors, setVpnSyncErrors] = useState<Record<string, string>>(
    {},
  );
  const [vpnInUse, setVpnInUse] = useState<Record<string, boolean>>({});
  const [isTogglingVpnSync, setIsTogglingVpnSync] = useState<
    Record<string, boolean>
  >({});

  // Table state
  const [proxiesSorting, setProxiesSorting] = useState<SortingState>([
    { id: "name", desc: false },
  ]);
  const [proxiesRowSelection, setProxiesRowSelection] =
    useState<RowSelectionState>({});
  const [vpnsSorting, setVpnsSorting] = useState<SortingState>([
    { id: "name", desc: false },
  ]);
  const [vpnsRowSelection, setVpnsRowSelection] = useState<RowSelectionState>(
    {},
  );

  // Track the active tab so we can scope the floating action bar (portaled
  // to body) to only the currently visible list. Initial value comes from
  // initialTab; subsequent changes drive the animated tabs via onValueChange.
  const [activeTab, setActiveTab] = useState<"proxies" | "vpns">(initialTab);
  // Reset selections when the dialog closes so the floating action bar
  // (portaled to body) doesn't linger on the page across navigations.
  useEffect(() => {
    if (!isOpen) {
      setProxiesRowSelection({});
      setVpnsRowSelection({});
    }
  }, [isOpen]);

  // Bulk delete state
  const [isBulkDeletingProxies, setIsBulkDeletingProxies] = useState(false);
  const [showBulkDeleteProxiesDialog, setShowBulkDeleteProxiesDialog] =
    useState(false);
  const [isBulkDeletingVpns, setIsBulkDeletingVpns] = useState(false);
  const [showBulkDeleteVpnsDialog, setShowBulkDeleteVpnsDialog] =
    useState(false);

  const { storedProxies: rawProxies, proxyUsage, isLoading } = useProxyEvents();
  const { vpnConfigs, vpnUsage, isLoading: isLoadingVpns } = useVpnEvents();

  // Filter out cloud-managed and cloud-derived proxies (cloud proxies are
  // deprecated). Memoized — without this the derived array gets a new
  // reference on every render, which made the [storedProxies] effect below
  // refire every render → re-set state → re-render, freezing the page once
  // the dialog mounted. Keeping the reference stable when the input is
  // unchanged is what every consumer (useReactTable, useEffect, selection
  // logic) actually wants.
  const storedProxies = useMemo(
    () =>
      rawProxies
        .filter((p) => !p.is_cloud_managed && !p.is_cloud_derived)
        .sort((a, b) =>
          a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
        ),
    [rawProxies],
  );

  // Listen for proxy sync status events
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      unlisten = await listen<{ id: string; status: string; error?: string }>(
        "proxy-sync-status",
        (event) => {
          const { id, status, error } = event.payload;
          setProxySyncStatus((prev) => ({
            ...prev,
            [id]: status as SyncStatus,
          }));
          if (error) {
            setProxySyncErrors((prev) => ({ ...prev, [id]: error }));
          }
        },
      );
    };

    void setupListener();
    return () => {
      unlisten?.();
    };
  }, []);

  // Listen for VPN sync status events
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      unlisten = await listen<{ id: string; status: string; error?: string }>(
        "vpn-sync-status",
        (event) => {
          const { id, status, error } = event.payload;
          setVpnSyncStatus((prev) => ({
            ...prev,
            [id]: status as SyncStatus,
          }));
          if (error) {
            setVpnSyncErrors((prev) => ({ ...prev, [id]: error }));
          }
        },
      );
    };

    void setupListener();
    return () => {
      unlisten?.();
    };
  }, []);

  // Load whether sync is required by an assigned profile.
  useEffect(() => {
    const loadProxyInUse = async () => {
      const inUse: Record<string, boolean> = {};
      for (const proxy of storedProxies) {
        try {
          const inUseBySynced = await invoke<boolean>(
            "is_proxy_in_use_by_synced_profile",
            { proxyId: proxy.id },
          );
          inUse[proxy.id] = inUseBySynced;
        } catch (_error) {
          // Ignore errors
        }
      }
      setProxyInUse(inUse);
    };
    if (storedProxies.length > 0) {
      void loadProxyInUse();
    }
  }, [storedProxies]);

  // Load VPN in-use status
  useEffect(() => {
    const loadVpnInUse = async () => {
      const inUse: Record<string, boolean> = {};
      for (const vpn of vpnConfigs) {
        try {
          const inUseBySynced = await invoke<boolean>(
            "is_vpn_in_use_by_synced_profile",
            { vpnId: vpn.id },
          );
          inUse[vpn.id] = inUseBySynced;
        } catch (_error) {
          // Ignore errors
        }
      }
      setVpnInUse(inUse);
    };
    if (vpnConfigs.length > 0) {
      void loadVpnInUse();
    }
  }, [vpnConfigs]);

  // Proxy handlers
  const handleDeleteProxy = useCallback((proxy: StoredProxy) => {
    setProxyToDelete(proxy);
  }, []);

  const handleConfirmDelete = useCallback(async () => {
    if (!proxyToDelete) return;
    setIsDeleting(true);
    try {
      await invoke("delete_stored_proxy", { proxyId: proxyToDelete.id });
      toast.success(t("proxies.management.deleteSuccess"));
      await emit("stored-proxies-changed");
    } catch (error) {
      console.error("Failed to delete proxy:", error);
      toast.error(t("proxies.management.deleteFailed"));
    } finally {
      setIsDeleting(false);
      setProxyToDelete(null);
    }
  }, [proxyToDelete, t]);

  const handleCreateProxy = useCallback(() => {
    setEditingProxy(null);
    setShowProxyForm(true);
  }, []);

  const handleEditProxy = useCallback((proxy: StoredProxy) => {
    setEditingProxy(proxy);
    setShowProxyForm(true);
  }, []);

  const handleProxyFormClose = useCallback(() => {
    setShowProxyForm(false);
    setEditingProxy(null);
  }, []);

  const handleToggleSync = useCallback(
    async (proxy: StoredProxy) => {
      setIsTogglingSync((prev) => ({ ...prev, [proxy.id]: true }));
      try {
        await invoke("set_proxy_sync_enabled", {
          proxyId: proxy.id,
          enabled: !proxy.sync_enabled,
        });
        showSuccessToast(
          proxy.sync_enabled
            ? t("proxies.management.syncDisabled")
            : t("proxies.management.syncEnabled"),
        );
        await emit("stored-proxies-changed");
      } catch (error) {
        console.error("Failed to toggle sync:", error);
        showErrorToast(
          parseBackendError(error)
            ? translateBackendError(t, error)
            : t("proxies.management.updateSyncFailed"),
        );
      } finally {
        setIsTogglingSync((prev) => ({ ...prev, [proxy.id]: false }));
      }
    },
    [t],
  );

  // VPN handlers
  const handleDeleteVpn = useCallback((vpn: VpnConfig) => {
    setVpnToDelete(vpn);
  }, []);

  const handleConfirmDeleteVpn = useCallback(async () => {
    if (!vpnToDelete) return;
    setIsDeletingVpn(true);
    try {
      await invoke("delete_vpn_config", { vpnId: vpnToDelete.id });
      toast.success(t("vpns.management.deleteSuccess"));
      await emit("vpn-configs-changed");
    } catch (error) {
      console.error("Failed to delete VPN:", error);
      toast.error(t("vpns.management.deleteFailed"));
    } finally {
      setIsDeletingVpn(false);
      setVpnToDelete(null);
    }
  }, [vpnToDelete, t]);

  const handleCreateVpn = useCallback(() => {
    setEditingVpn(null);
    setShowVpnForm(true);
  }, []);

  const handleEditVpn = useCallback((vpn: VpnConfig) => {
    setEditingVpn(vpn);
    setShowVpnForm(true);
  }, []);

  const handleVpnFormClose = useCallback(() => {
    setShowVpnForm(false);
    setEditingVpn(null);
  }, []);

  const handleToggleVpnSync = useCallback(
    async (vpn: VpnConfig) => {
      setIsTogglingVpnSync((prev) => ({ ...prev, [vpn.id]: true }));
      try {
        await invoke("set_vpn_sync_enabled", {
          vpnId: vpn.id,
          enabled: !vpn.sync_enabled,
        });
        showSuccessToast(
          vpn.sync_enabled
            ? t("proxies.management.syncDisabled")
            : t("proxies.management.syncEnabled"),
        );
        await emit("vpn-configs-changed");
      } catch (error) {
        console.error("Failed to toggle VPN sync:", error);
        showErrorToast(
          parseBackendError(error)
            ? translateBackendError(t, error)
            : t("proxies.management.updateSyncFailed"),
        );
      } finally {
        setIsTogglingVpnSync((prev) => ({ ...prev, [vpn.id]: false }));
      }
    },
    [t],
  );

  const proxyColumns = useMemo<ColumnDef<StoredProxy>[]>(
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
          />
        ),
        cell: ({ row }) => (
          <Checkbox
            checked={row.getIsSelected()}
            disabled={!row.getCanSelect()}
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
            className="h-auto cursor-pointer justify-start p-0 text-left font-semibold"
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
          <span className="block truncate font-medium">
            {row.original.name}
          </span>
        ),
      },
      {
        id: "protocol",
        size: 96,
        enableSorting: false,
        header: () => t("proxies.management.protocolCol"),
        cell: ({ row }) => {
          const proxyType = row.original.proxy_settings.proxy_type;
          // Shadowsocks keeps its cipher in `username`, and the cipher is what
          // decides this hop.
          const cipher = row.original.proxy_settings.username;
          const encrypted = isFirstHopEncrypted(proxyType, cipher);
          // A stored Shadowsocks proxy can carry no cipher at all -
          // `parse_txt_proxies` reads `ss://host:8388` as username None, and
          // `import_proxies_json` stores proxy_type and username verbatim, and
          // with none recorded the honest answer is "undecided", not
          // "plaintext". The add/edit form says exactly that, so this cell
          // saying "not encrypted" made the app contradict itself about one
          // proxy. Only the sentence changes: `encrypted` stays false, so the
          // warning tint and everything downstream still fail closed.
          const cipherUndecided =
            !encrypted &&
            canonicalProxyType(proxyType) === "ss" &&
            (cipher ?? "").trim().length === 0;
          // The type is free text from the REST API, so an unrecognised one
          // still prints itself; only a blank one falls back to the label.
          const protocolLabel =
            proxyProtocolToken(proxyType) ||
            t("proxies.management.protocolUnknown");
          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  className={cn(
                    "font-mono text-[10px] tracking-wider uppercase",
                    encrypted ? "text-muted-foreground" : "text-warning-text",
                  )}
                >
                  {protocolLabel}
                </span>
              </TooltipTrigger>
              <TooltipContent>
                <p>
                  {encrypted
                    ? t("proxies.management.firstHopEncryptedTooltip")
                    : cipherUndecided
                      ? t("proxies.management.firstHopCipherTooltip")
                      : t("proxies.management.firstHopPlaintextTooltip")}
                </p>
              </TooltipContent>
            </Tooltip>
          );
        },
      },
      {
        id: "hostPort",
        enableSorting: false,
        header: () => t("proxies.management.hostPort"),
        cell: ({ row }) => (
          <span className="block truncate font-mono text-xs text-muted-foreground">
            {row.original.proxy_settings.host}:
            {row.original.proxy_settings.port}
          </span>
        ),
      },
      {
        // WebRTC is UDP, so whether this proxy carries it decides whether a
        // profile on it can route WebRTC at all. That belongs in the table,
        // not only behind the check popover.
        id: "udp",
        size: 72,
        enableSorting: false,
        header: () => t("proxies.management.udpCol"),
        cell: ({ row }) => <ProxyUdpBadge proxy={row.original} />,
      },
      {
        id: "usage",
        size: 80,
        enableSorting: false,
        header: () => t("proxies.management.usage"),
        cell: ({ row }) => (
          <ProfileUsageButton
            label={t("appFeedback.assignedProfiles", {
              name: row.original.name,
            })}
            failed={referencesFailed}
            profiles={
              referencedProfiles?.filter(
                (profile) => profile.proxy_id === row.original.id,
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
          const proxy = row.original;
          const locked = proxyInUse[proxy.id];
          const syncDot = getSyncStatusDot(
            proxy,
            proxySyncStatus[proxy.id],
            t,
            proxySyncErrors[proxy.id],
          );
          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  className="inline-flex items-center gap-2"
                  title={syncDot.tooltip}
                >
                  <span
                    aria-hidden="true"
                    className={cn(
                      "size-2 shrink-0 rounded-full",
                      syncDot.color,
                    )}
                  />
                  <AnimatedSwitch
                    aria-label={`${t("proxies.management.syncCol")}: ${proxy.name}`}
                    checked={proxy.sync_enabled}
                    onCheckedChange={() => void handleToggleSync(proxy)}
                    disabled={isTogglingSync[proxy.id] || locked}
                  />
                </span>
              </TooltipTrigger>
              <TooltipContent>
                {locked ? (
                  <p>{t("syncTooltips.lockedInUse")}</p>
                ) : (
                  <p>
                    {proxy.sync_enabled
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
        size: 144,
        enableSorting: false,
        header: () => t("common.labels.actions"),
        cell: ({ row }) => {
          const proxy = row.original;
          return (
            <div className="flex gap-1">
              <ProxyCheckButton proxy={proxy} />
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      handleEditProxy(proxy);
                    }}
                  >
                    <LuPencil className="size-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  <p>{t("proxies.management.editProxy")}</p>
                </TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        handleDeleteProxy(proxy);
                      }}
                      disabled={(proxyUsage[proxy.id] ?? 0) > 0}
                    >
                      <LuTrash2 className="size-4" />
                    </Button>
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  {(proxyUsage[proxy.id] ?? 0) > 0 ? (
                    <p>
                      {(proxyUsage[proxy.id] ?? 0) === 1
                        ? t("proxies.management.cannotDelete_one", {
                            count: proxyUsage[proxy.id],
                          })
                        : t("proxies.management.cannotDelete_other", {
                            count: proxyUsage[proxy.id],
                          })}
                    </p>
                  ) : (
                    <p>{t("proxies.management.deleteProxy")}</p>
                  )}
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
      proxySyncStatus,
      proxySyncErrors,
      proxyUsage,
      isTogglingSync,
      proxyInUse,
      handleToggleSync,
      handleEditProxy,
      handleDeleteProxy,
    ],
  );

  const proxiesTable = useReactTable({
    data: storedProxies,
    columns: proxyColumns,
    state: {
      sorting: proxiesSorting,
      rowSelection: proxiesRowSelection,
    },
    onSortingChange: setProxiesSorting,
    onRowSelectionChange: setProxiesRowSelection,
    enableRowSelection: (row) => !proxyInUse[row.original.id],
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getRowId: (row) => row.id,
  });

  const vpnColumns = useMemo<ColumnDef<VpnConfig>[]>(
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
          />
        ),
        cell: ({ row }) => (
          <Checkbox
            checked={row.getIsSelected()}
            disabled={!row.getCanSelect()}
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
            className="h-auto cursor-pointer justify-start p-0 text-left font-semibold"
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
          return (
            <span className="block truncate font-medium">
              {row.original.name}
            </span>
          );
        },
      },
      {
        id: "type",
        size: 96,
        enableSorting: false,
        header: () => t("common.labels.type"),
        cell: () => <Badge variant="outline">WG</Badge>,
      },
      {
        id: "usage",
        size: 80,
        enableSorting: false,
        header: () => t("proxies.management.usage"),
        cell: ({ row }) => (
          <ProfileUsageButton
            label={t("appFeedback.assignedProfiles", {
              name: row.original.name,
            })}
            failed={referencesFailed}
            profiles={
              referencedProfiles?.filter(
                (profile) => profile.vpn_id === row.original.id,
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
          const vpn = row.original;
          const locked = vpnInUse[vpn.id];
          const syncDot = getSyncStatusDot(
            vpn,
            vpnSyncStatus[vpn.id],
            t,
            vpnSyncErrors[vpn.id],
          );
          return (
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  className="inline-flex items-center gap-2"
                  title={syncDot.tooltip}
                >
                  <span
                    aria-hidden="true"
                    className={cn(
                      "size-2 shrink-0 rounded-full",
                      syncDot.color,
                    )}
                  />
                  <AnimatedSwitch
                    aria-label={`${t("proxies.management.syncCol")}: ${vpn.name}`}
                    checked={vpn.sync_enabled}
                    onCheckedChange={() => void handleToggleVpnSync(vpn)}
                    disabled={isTogglingVpnSync[vpn.id] || locked}
                  />
                </span>
              </TooltipTrigger>
              <TooltipContent>
                {locked ? (
                  <p>{t("syncTooltips.lockedInUse")}</p>
                ) : (
                  <p>
                    {vpn.sync_enabled
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
        size: 144,
        enableSorting: false,
        header: () => t("common.labels.actions"),
        cell: ({ row }) => {
          const vpn = row.original;
          return (
            <div className="flex gap-1">
              <VpnCheckButton
                vpnId={vpn.id}
                vpnName={vpn.name}
                checkingVpnId={checkingVpnId}
                setCheckingVpnId={setCheckingVpnId}
              />
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      handleEditVpn(vpn);
                    }}
                  >
                    <LuPencil className="size-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  <p>{t("vpns.management.editVpn")}</p>
                </TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        handleDeleteVpn(vpn);
                      }}
                      disabled={(vpnUsage[vpn.id] ?? 0) > 0}
                    >
                      <LuTrash2 className="size-4" />
                    </Button>
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  {(vpnUsage[vpn.id] ?? 0) > 0 ? (
                    <p>
                      {(vpnUsage[vpn.id] ?? 0) === 1
                        ? t("vpns.management.cannotDelete_one", {
                            count: vpnUsage[vpn.id],
                          })
                        : t("vpns.management.cannotDelete_other", {
                            count: vpnUsage[vpn.id],
                          })}
                    </p>
                  ) : (
                    <p>{t("vpns.management.deleteVpn")}</p>
                  )}
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
      vpnSyncStatus,
      vpnSyncErrors,
      vpnUsage,
      isTogglingVpnSync,
      vpnInUse,
      checkingVpnId,
      handleToggleVpnSync,
      handleEditVpn,
      handleDeleteVpn,
    ],
  );

  const vpnsTable = useReactTable({
    data: vpnConfigs,
    columns: vpnColumns,
    state: {
      sorting: vpnsSorting,
      rowSelection: vpnsRowSelection,
    },
    onSortingChange: setVpnsSorting,
    onRowSelectionChange: setVpnsRowSelection,
    enableRowSelection: (row) => !vpnInUse[row.original.id],
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getRowId: (row) => row.id,
  });

  const selectedProxies = proxiesTable
    .getFilteredSelectedRowModel()
    .rows.map((row) => row.original);
  const selectedVpns = vpnsTable
    .getFilteredSelectedRowModel()
    .rows.map((row) => row.original);

  // Row selection is gated on sync ownership, which says nothing about local
  // use, so a selection can hold entries the per-row delete refuses. Deleting
  // one anyway leaves every profile that points at it holding an id that
  // resolves to nothing, and the browser then launches with no upstream.
  const deletableProxies = selectedProxies.filter(
    (proxy) => (proxyUsage[proxy.id] ?? 0) === 0,
  );
  const skippedProxyCount = selectedProxies.length - deletableProxies.length;
  const deletableVpns = selectedVpns.filter(
    (vpn) => (vpnUsage[vpn.id] ?? 0) === 0,
  );
  const skippedVpnCount = selectedVpns.length - deletableVpns.length;

  const handleBulkDeleteProxies = useCallback(async () => {
    if (selectedProxies.length === 0) return;
    setIsBulkDeletingProxies(true);
    try {
      const results = await Promise.allSettled(
        deletableProxies.map((proxy) =>
          invoke("delete_stored_proxy", { proxyId: proxy.id }),
        ),
      );
      const failed = results.filter((r) => r.status === "rejected").length;
      const succeeded = results.length - failed;
      if (succeeded > 0) {
        toast.success(t("proxies.management.deleteSuccess"));
      }
      if (failed > 0) {
        toast.error(t("proxies.management.deleteFailed"));
      }
      if (skippedProxyCount > 0) {
        toast.warning(
          t("proxies.bulkDelete.skippedProxies", { count: skippedProxyCount }),
        );
      }
      await emit("stored-proxies-changed");
      setProxiesRowSelection({});
    } finally {
      setIsBulkDeletingProxies(false);
      setShowBulkDeleteProxiesDialog(false);
    }
  }, [selectedProxies, deletableProxies, skippedProxyCount, t]);

  const handleBulkDeleteVpns = useCallback(async () => {
    if (selectedVpns.length === 0) return;
    setIsBulkDeletingVpns(true);
    try {
      const results = await Promise.allSettled(
        deletableVpns.map((vpn) =>
          invoke("delete_vpn_config", { vpnId: vpn.id }),
        ),
      );
      const failed = results.filter((r) => r.status === "rejected").length;
      const succeeded = results.length - failed;
      if (succeeded > 0) {
        toast.success(t("vpns.management.deleteSuccess"));
      }
      if (failed > 0) {
        toast.error(t("vpns.management.deleteFailed"));
      }
      if (skippedVpnCount > 0) {
        toast.warning(
          t("proxies.bulkDelete.skippedVpns", { count: skippedVpnCount }),
        );
      }
      await emit("vpn-configs-changed");
      setVpnsRowSelection({});
    } finally {
      setIsBulkDeletingVpns(false);
      setShowBulkDeleteVpnsDialog(false);
    }
  }, [selectedVpns, deletableVpns, skippedVpnCount, t]);

  // Bulk-toggle sync: if every selectable row has sync ON, turn them all
  // OFF; otherwise turn them all ON. Items locked by a synced profile
  // (proxyInUse / vpnInUse) are skipped silently when the target is OFF.
  const handleBulkToggleProxiesSync = useCallback(async () => {
    if (selectedProxies.length === 0) return;
    const allOn = selectedProxies.every((p) => p.sync_enabled);
    const targetEnabled = !allOn;
    const targets = selectedProxies.filter((p) =>
      targetEnabled ? !p.sync_enabled : p.sync_enabled && !proxyInUse[p.id],
    );
    if (targets.length === 0) return;
    const results = await Promise.allSettled(
      targets.map((proxy) =>
        invoke("set_proxy_sync_enabled", {
          proxyId: proxy.id,
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
    await emit("stored-proxies-changed");
  }, [selectedProxies, proxyInUse, t]);

  const handleBulkToggleVpnsSync = useCallback(async () => {
    if (selectedVpns.length === 0) return;
    const allOn = selectedVpns.every((v) => v.sync_enabled);
    const targetEnabled = !allOn;
    const targets = selectedVpns.filter((v) =>
      targetEnabled ? !v.sync_enabled : v.sync_enabled && !vpnInUse[v.id],
    );
    if (targets.length === 0) return;
    const results = await Promise.allSettled(
      targets.map((vpn) =>
        invoke("set_vpn_sync_enabled", {
          vpnId: vpn.id,
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
    await emit("vpn-configs-changed");
  }, [selectedVpns, vpnInUse, t]);

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
        <DialogContent className="flex max-h-[85vh] max-w-[min(80rem,calc(100%-4rem))] flex-col">
          {!subPage && (
            <DialogHeader>
              <DialogTitle>{t("proxies.management.title")}</DialogTitle>
              <DialogDescription>
                {t("proxies.management.description")}
              </DialogDescription>
            </DialogHeader>
          )}

          <div className="@container flex min-h-0 w-full flex-1 flex-col">
            <AnimatedTabs
              key={initialTab}
              defaultValue={initialTab}
              onValueChange={(v) => setActiveTab(v as "proxies" | "vpns")}
              className="flex min-h-0 flex-1 flex-col"
            >
              <div className="flex shrink-0 flex-wrap items-center justify-between gap-2">
                <AnimatedTabsList>
                  <AnimatedTabsTrigger value="proxies">
                    <span>{t("proxies.management.tabProxies")}</span>
                    <span className="text-xs tabular-nums">
                      {storedProxies.length}
                    </span>
                  </AnimatedTabsTrigger>
                  <AnimatedTabsTrigger value="vpns">
                    <span>{t("proxies.management.tabVpns")}</span>
                    <span className="text-xs tabular-nums">
                      {vpnConfigs.length}
                    </span>
                  </AnimatedTabsTrigger>
                </AnimatedTabsList>
                <div className="flex items-center gap-2">
                  {activeTab === "proxies" && (
                    <>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <RippleButton
                            size="sm"
                            variant="outline"
                            onClick={() => {
                              setShowImportDialog(true);
                            }}
                            className="flex items-center gap-2"
                            aria-label={t("common.buttons.import")}
                          >
                            <LuUpload className="size-4" />
                            <span className="hidden @2xl:inline">
                              {t("common.buttons.import")}
                            </span>
                          </RippleButton>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>{t("common.buttons.import")}</p>
                        </TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <RippleButton
                            size="sm"
                            variant="outline"
                            onClick={() => {
                              setShowExportDialog(true);
                            }}
                            className="flex items-center gap-2"
                            aria-label={t("common.buttons.export")}
                            disabled={storedProxies.length === 0}
                          >
                            <LuDownload className="size-4" />
                            <span className="hidden @2xl:inline">
                              {t("common.buttons.export")}
                            </span>
                          </RippleButton>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>{t("common.buttons.export")}</p>
                        </TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <RippleButton
                            size="sm"
                            onClick={handleCreateProxy}
                            className="flex items-center gap-2"
                            aria-label={t("proxies.management.newProxy")}
                          >
                            <GoPlus className="size-4" />
                            <span className="hidden @2xl:inline">
                              {t("proxies.management.newProxy")}
                            </span>
                          </RippleButton>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>{t("proxies.management.newProxy")}</p>
                        </TooltipContent>
                      </Tooltip>
                    </>
                  )}
                  {activeTab === "vpns" && (
                    <>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <RippleButton
                            size="sm"
                            variant="outline"
                            onClick={() => {
                              setShowVpnImportDialog(true);
                            }}
                            className="flex items-center gap-2"
                            aria-label={t("common.buttons.import")}
                          >
                            <LuUpload className="size-4" />
                            <span className="hidden @2xl:inline">
                              {t("common.buttons.import")}
                            </span>
                          </RippleButton>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>{t("common.buttons.import")}</p>
                        </TooltipContent>
                      </Tooltip>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <RippleButton
                            size="sm"
                            onClick={handleCreateVpn}
                            className="flex items-center gap-2"
                            aria-label={t("proxies.management.newVpn")}
                          >
                            <GoPlus className="size-4" />
                            <span className="hidden @2xl:inline">
                              {t("proxies.management.newVpn")}
                            </span>
                          </RippleButton>
                        </TooltipTrigger>
                        <TooltipContent>
                          <p>{t("proxies.management.newVpn")}</p>
                        </TooltipContent>
                      </Tooltip>
                    </>
                  )}
                </div>
              </div>

              <AnimatedTabsContent
                value="proxies"
                className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
              >
                <div className="flex min-h-0 flex-1 flex-col gap-4">
                  {isLoading ? (
                    <div className="text-sm text-muted-foreground">
                      {t("proxies.management.loading")}
                    </div>
                  ) : storedProxies.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      {t("proxies.management.noneCreated")}
                    </div>
                  ) : (
                    <FadingScrollArea
                      className={cn(
                        "min-h-0 flex-1",
                        selectedProxies.length > 0 && "pb-16",
                      )}
                      style={
                        {
                          "--scroll-fade-top-offset": "32px",
                        } as React.CSSProperties
                      }
                    >
                      <Table
                        className="w-full table-fixed"
                        containerClassName="overflow-visible"
                      >
                        <TableHeader className="sticky top-0 z-10 bg-background [&_tr]:border-0">
                          {proxiesTable.getHeaderGroups().map((headerGroup) => (
                            <TableRow
                              key={headerGroup.id}
                              className="border-0!"
                            >
                              {headerGroup.headers.map((header) => (
                                <TableHead
                                  key={header.id}
                                  style={{
                                    width:
                                      header.column.id === "name" ||
                                      header.column.id === "hostPort"
                                        ? undefined
                                        : `${header.column.getSize()}px`,
                                  }}
                                  className={cn(
                                    // name and hostPort emit no width, so
                                    // fixed layout splits the remaining
                                    // space evenly between them (hostPort
                                    // hides below @2xl, leaving name all
                                    // of it).
                                    header.column.id === "name" && "max-w-0",
                                    header.column.id === "hostPort" &&
                                      "hidden max-w-0 @2xl:table-cell",
                                    (header.column.id === "protocol" ||
                                      header.column.id === "type") &&
                                      "hidden @2xl:table-cell",
                                  )}
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
                          {proxiesTable.getRowModel().rows.map((row) => (
                            <TableRow
                              key={row.id}
                              data-state={row.getIsSelected() && "selected"}
                              className="border-0! hover:bg-muted"
                            >
                              {row.getVisibleCells().map((cell) => (
                                <TableCell
                                  key={cell.id}
                                  style={{
                                    width:
                                      cell.column.id === "name" ||
                                      cell.column.id === "hostPort"
                                        ? undefined
                                        : `${cell.column.getSize()}px`,
                                  }}
                                  className={cn(
                                    cell.column.id === "name" && "max-w-0",
                                    cell.column.id === "hostPort" &&
                                      "hidden max-w-0 @2xl:table-cell",
                                    (cell.column.id === "protocol" ||
                                      cell.column.id === "type") &&
                                      "hidden @2xl:table-cell",
                                  )}
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
                value="vpns"
                className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
              >
                <div className="flex min-h-0 flex-1 flex-col gap-4">
                  {isLoadingVpns ? (
                    <div className="text-sm text-muted-foreground">
                      {t("vpns.management.loading")}
                    </div>
                  ) : vpnConfigs.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      {t("vpns.management.noneCreated")}
                    </div>
                  ) : (
                    <FadingScrollArea
                      className={cn(
                        "min-h-0 flex-1",
                        selectedVpns.length > 0 && "pb-16",
                      )}
                      style={
                        {
                          "--scroll-fade-top-offset": "32px",
                        } as React.CSSProperties
                      }
                    >
                      <Table
                        className="w-full table-fixed"
                        containerClassName="overflow-visible"
                      >
                        <TableHeader className="sticky top-0 z-10 bg-background [&_tr]:border-0">
                          {vpnsTable.getHeaderGroups().map((headerGroup) => (
                            <TableRow
                              key={headerGroup.id}
                              className="border-0!"
                            >
                              {headerGroup.headers.map((header) => (
                                <TableHead
                                  key={header.id}
                                  style={{
                                    width:
                                      header.column.id === "name" ||
                                      header.column.id === "hostPort"
                                        ? undefined
                                        : `${header.column.getSize()}px`,
                                  }}
                                  className={cn(
                                    // name and hostPort emit no width, so
                                    // fixed layout splits the remaining
                                    // space evenly between them (hostPort
                                    // hides below @2xl, leaving name all
                                    // of it).
                                    header.column.id === "name" && "max-w-0",
                                    header.column.id === "hostPort" &&
                                      "hidden max-w-0 @2xl:table-cell",
                                    (header.column.id === "protocol" ||
                                      header.column.id === "type") &&
                                      "hidden @2xl:table-cell",
                                  )}
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
                          {vpnsTable.getRowModel().rows.map((row) => (
                            <TableRow
                              key={row.id}
                              data-state={row.getIsSelected() && "selected"}
                              className="border-0! hover:bg-muted"
                            >
                              {row.getVisibleCells().map((cell) => (
                                <TableCell
                                  key={cell.id}
                                  style={{
                                    width:
                                      cell.column.id === "name" ||
                                      cell.column.id === "hostPort"
                                        ? undefined
                                        : `${cell.column.getSize()}px`,
                                  }}
                                  className={cn(
                                    cell.column.id === "name" && "max-w-0",
                                    cell.column.id === "hostPort" &&
                                      "hidden max-w-0 @2xl:table-cell",
                                    (cell.column.id === "protocol" ||
                                      cell.column.id === "type") &&
                                      "hidden @2xl:table-cell",
                                  )}
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

      <ProxyFormDialog
        isOpen={showProxyForm}
        onClose={handleProxyFormClose}
        editingProxy={editingProxy}
      />
      <DeleteConfirmationDialog
        isOpen={proxyToDelete !== null}
        onClose={() => {
          setProxyToDelete(null);
        }}
        onConfirm={handleConfirmDelete}
        title={t("proxies.management.deleteTitle")}
        description={t("proxies.management.deleteDescription", {
          name: proxyToDelete?.name ?? "",
        })}
        confirmButtonText={t("common.buttons.delete")}
        isLoading={isDeleting}
      />
      <ProxyImportDialog
        isOpen={showImportDialog}
        onClose={() => {
          setShowImportDialog(false);
        }}
      />
      <ProxyExportDialog
        isOpen={showExportDialog}
        onClose={() => {
          setShowExportDialog(false);
        }}
      />
      <VpnFormDialog
        isOpen={showVpnForm}
        onClose={handleVpnFormClose}
        editingVpn={editingVpn}
      />
      <DeleteConfirmationDialog
        isOpen={vpnToDelete !== null}
        onClose={() => {
          setVpnToDelete(null);
        }}
        onConfirm={handleConfirmDeleteVpn}
        title={t("vpns.management.deleteTitle")}
        description={t("vpns.management.deleteDescription", {
          name: vpnToDelete?.name ?? "",
        })}
        confirmButtonText={t("common.buttons.delete")}
        isLoading={isDeletingVpn}
      />
      <VpnImportDialog
        isOpen={showVpnImportDialog}
        onClose={() => {
          setShowVpnImportDialog(false);
        }}
      />
      {isOpen && activeTab === "proxies" && (
        <DataTableActionBar table={proxiesTable}>
          <DataTableActionBarSelection table={proxiesTable} />
          <DataTableActionBarAction
            tooltip={t("syncTooltips.bulkToggle")}
            onClick={() => void handleBulkToggleProxiesSync()}
            size="icon"
          >
            <LuRefreshCw />
          </DataTableActionBarAction>
          <DataTableActionBarAction
            tooltip={t("common.buttons.delete")}
            onClick={() => {
              if (skippedProxyCount > 0 && deletableProxies.length === 0) {
                toast.warning(
                  t("proxies.bulkDelete.skippedProxies", {
                    count: skippedProxyCount,
                  }),
                );
                return;
              }
              setShowBulkDeleteProxiesDialog(true);
            }}
            size="icon"
            variant="destructive"
            className="border-destructive bg-destructive hover:bg-destructive"
          >
            <LuTrash2 />
          </DataTableActionBarAction>
        </DataTableActionBar>
      )}
      {isOpen && activeTab === "vpns" && (
        <DataTableActionBar table={vpnsTable}>
          <DataTableActionBarSelection table={vpnsTable} />
          <DataTableActionBarAction
            tooltip={t("syncTooltips.bulkToggle")}
            onClick={() => void handleBulkToggleVpnsSync()}
            size="icon"
          >
            <LuRefreshCw />
          </DataTableActionBarAction>
          <DataTableActionBarAction
            tooltip={t("common.buttons.delete")}
            onClick={() => {
              if (skippedVpnCount > 0 && deletableVpns.length === 0) {
                toast.warning(
                  t("proxies.bulkDelete.skippedVpns", {
                    count: skippedVpnCount,
                  }),
                );
                return;
              }
              setShowBulkDeleteVpnsDialog(true);
            }}
            size="icon"
            variant="destructive"
            className="border-destructive bg-destructive hover:bg-destructive"
          >
            <LuTrash2 />
          </DataTableActionBarAction>
        </DataTableActionBar>
      )}
      <DeleteConfirmationDialog
        isOpen={showBulkDeleteProxiesDialog}
        onClose={() => {
          setShowBulkDeleteProxiesDialog(false);
        }}
        onConfirm={handleBulkDeleteProxies}
        title={t("proxies.bulkDelete.proxiesTitle")}
        description={t("proxies.bulkDelete.proxiesDescription", {
          count: deletableProxies.length,
          names: deletableProxies.map((p) => p.name).join(", "),
        })}
        confirmButtonText={t("proxies.bulkDelete.confirmButton", {
          count: deletableProxies.length,
        })}
        isLoading={isBulkDeletingProxies}
      />
      <DeleteConfirmationDialog
        isOpen={showBulkDeleteVpnsDialog}
        onClose={() => {
          setShowBulkDeleteVpnsDialog(false);
        }}
        onConfirm={handleBulkDeleteVpns}
        title={t("proxies.bulkDelete.vpnsTitle")}
        description={t("proxies.bulkDelete.vpnsDescription", {
          count: deletableVpns.length,
          names: deletableVpns.map((v) => v.name).join(", "),
        })}
        confirmButtonText={t("proxies.bulkDelete.confirmButton", {
          count: deletableVpns.length,
        })}
        isLoading={isBulkDeletingVpns}
      />
    </>
  );
}

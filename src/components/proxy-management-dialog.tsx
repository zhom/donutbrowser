"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { LuDownload, LuPencil, LuTrash2, LuUpload } from "react-icons/lu";
import { toast } from "sonner";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { ProxyExportDialog } from "@/components/proxy-export-dialog";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { ProxyImportDialog } from "@/components/proxy-import-dialog";
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
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { useVpnEvents } from "@/hooks/use-vpn-events";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { ProxyCheckResult, StoredProxy, VpnConfig } from "@/types";
import { ProxyCheckButton } from "./proxy-check-button";
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
}

export function ProxyManagementDialog({
  isOpen,
  onClose,
}: ProxyManagementDialogProps) {
  const { t } = useTranslation();
  // Proxy state
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [showImportDialog, setShowImportDialog] = useState(false);
  const [showExportDialog, setShowExportDialog] = useState(false);
  const [editingProxy, setEditingProxy] = useState<StoredProxy | null>(null);
  const [proxyToDelete, setProxyToDelete] = useState<StoredProxy | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [checkingProxyId, setCheckingProxyId] = useState<string | null>(null);
  const [proxyCheckResults, setProxyCheckResults] = useState<
    Record<string, ProxyCheckResult>
  >({});
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

  const { storedProxies: rawProxies, proxyUsage, isLoading } = useProxyEvents();
  const { vpnConfigs, vpnUsage, isLoading: isLoadingVpns } = useVpnEvents();

  // Filter out cloud-managed and cloud-derived proxies (cloud proxies are deprecated)
  const storedProxies = rawProxies
    .filter((p) => !p.is_cloud_managed && !p.is_cloud_derived)
    .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));

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

  // Load cached check results on mount and when proxies change
  useEffect(() => {
    const loadCachedResults = async () => {
      const results: Record<string, ProxyCheckResult> = {};
      const inUse: Record<string, boolean> = {};
      for (const proxy of storedProxies) {
        try {
          const cached = await invoke<ProxyCheckResult | null>(
            "get_cached_proxy_check",
            { proxyId: proxy.id },
          );
          if (cached) {
            results[proxy.id] = cached;
          }

          const inUseBySynced = await invoke<boolean>(
            "is_proxy_in_use_by_synced_profile",
            { proxyId: proxy.id },
          );
          inUse[proxy.id] = inUseBySynced;
        } catch (_error) {
          // Ignore errors
        }
      }
      setProxyCheckResults(results);
      setProxyInUse(inUse);
    };
    if (storedProxies.length > 0) {
      void loadCachedResults();
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
          error instanceof Error
            ? error.message
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
          error instanceof Error
            ? error.message
            : t("proxies.management.updateSyncFailed"),
        );
      } finally {
        setIsTogglingVpnSync((prev) => ({ ...prev, [vpn.id]: false }));
      }
    },
    [t],
  );

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose}>
        <DialogContent className="max-w-2xl max-h-[90vh] flex flex-col">
          <DialogHeader>
            <DialogTitle>{t("proxies.management.title")}</DialogTitle>
            <DialogDescription>
              {t("proxies.management.description")}
            </DialogDescription>
          </DialogHeader>

          <ScrollArea className="overflow-y-auto flex-1">
            <Tabs defaultValue="proxies">
              <TabsList className="w-full">
                <TabsTrigger value="proxies" className="flex-1">
                  {t("proxies.management.tabProxies")}
                </TabsTrigger>
                <TabsTrigger value="vpns" className="flex-1">
                  {t("proxies.management.tabVpns")}
                </TabsTrigger>
              </TabsList>

              <TabsContent value="proxies">
                <div className="space-y-4">
                  <div className="flex justify-between items-center">
                    <div className="flex gap-2">
                      <RippleButton
                        size="sm"
                        variant="outline"
                        onClick={() => {
                          setShowImportDialog(true);
                        }}
                        className="flex gap-2 items-center"
                      >
                        <LuUpload className="w-4 h-4" />
                        {t("common.buttons.import")}
                      </RippleButton>
                      <RippleButton
                        size="sm"
                        variant="outline"
                        onClick={() => {
                          setShowExportDialog(true);
                        }}
                        className="flex gap-2 items-center"
                        disabled={storedProxies.length === 0}
                      >
                        <LuDownload className="w-4 h-4" />
                        {t("common.buttons.export")}
                      </RippleButton>
                    </div>
                    <div className="flex gap-2">
                      <RippleButton
                        size="sm"
                        onClick={handleCreateProxy}
                        className="flex gap-2 items-center"
                      >
                        <GoPlus className="w-4 h-4" />
                        {t("proxies.management.create")}
                      </RippleButton>
                    </div>
                  </div>

                  {isLoading ? (
                    <div className="text-sm text-muted-foreground">
                      {t("proxies.management.loading")}
                    </div>
                  ) : storedProxies.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      {t("proxies.management.noneCreated")}
                    </div>
                  ) : (
                    <div className="border rounded-md">
                      <ScrollArea className="h-[240px]">
                        <Table>
                          <TableHeader>
                            <TableRow>
                              <TableHead>{t("common.labels.name")}</TableHead>
                              <TableHead className="w-20">
                                {t("proxies.management.usage")}
                              </TableHead>
                              <TableHead className="w-24">
                                {t("proxies.management.syncCol")}
                              </TableHead>
                              <TableHead className="w-24">
                                {t("common.labels.actions")}
                              </TableHead>
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {storedProxies.map((proxy) => {
                              const syncDot = getSyncStatusDot(
                                proxy,
                                proxySyncStatus[proxy.id],
                                t,
                                proxySyncErrors[proxy.id],
                              );
                              return (
                                <TableRow key={proxy.id}>
                                  <TableCell className="font-medium">
                                    <div className="flex items-center gap-2">
                                      <Tooltip>
                                        <TooltipTrigger asChild>
                                          <div
                                            className={`w-2 h-2 rounded-full shrink-0 ${syncDot.color} ${
                                              syncDot.animate
                                                ? "animate-pulse"
                                                : ""
                                            }`}
                                          />
                                        </TooltipTrigger>
                                        <TooltipContent>
                                          <p>{syncDot.tooltip}</p>
                                        </TooltipContent>
                                      </Tooltip>
                                      {proxy.name}
                                    </div>
                                  </TableCell>
                                  <TableCell>
                                    <Badge variant="secondary">
                                      {proxyUsage[proxy.id] ?? 0}
                                    </Badge>
                                  </TableCell>
                                  <TableCell>
                                    <Tooltip>
                                      <TooltipTrigger asChild>
                                        <div className="flex items-center">
                                          <Checkbox
                                            checked={proxy.sync_enabled}
                                            onCheckedChange={() =>
                                              void handleToggleSync(proxy)
                                            }
                                            disabled={
                                              isTogglingSync[proxy.id] ||
                                              proxyInUse[proxy.id]
                                            }
                                          />
                                        </div>
                                      </TooltipTrigger>
                                      <TooltipContent>
                                        {proxyInUse[proxy.id] ? (
                                          <p>
                                            {t(
                                              "proxies.management.syncCannotDisable",
                                            )}
                                          </p>
                                        ) : (
                                          <p>
                                            {proxy.sync_enabled
                                              ? t(
                                                  "proxies.management.disableSync",
                                                )
                                              : t(
                                                  "proxies.management.enableSync",
                                                )}
                                          </p>
                                        )}
                                      </TooltipContent>
                                    </Tooltip>
                                  </TableCell>
                                  <TableCell>
                                    <div className="flex gap-1">
                                      <ProxyCheckButton
                                        proxy={proxy}
                                        profileId={proxy.id}
                                        checkingProfileId={checkingProxyId}
                                        cachedResult={
                                          proxyCheckResults[proxy.id]
                                        }
                                        setCheckingProfileId={
                                          setCheckingProxyId
                                        }
                                        onCheckComplete={(result) => {
                                          setProxyCheckResults((prev) => ({
                                            ...prev,
                                            [proxy.id]: result,
                                          }));
                                        }}
                                        onCheckFailed={(result) => {
                                          setProxyCheckResults((prev) => ({
                                            ...prev,
                                            [proxy.id]: result,
                                          }));
                                        }}
                                      />
                                      <Tooltip>
                                        <TooltipTrigger asChild>
                                          <Button
                                            variant="ghost"
                                            size="sm"
                                            onClick={() => {
                                              handleEditProxy(proxy);
                                            }}
                                          >
                                            <LuPencil className="w-4 h-4" />
                                          </Button>
                                        </TooltipTrigger>
                                        <TooltipContent>
                                          <p>
                                            {t("proxies.management.editProxy")}
                                          </p>
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
                                              disabled={
                                                (proxyUsage[proxy.id] ?? 0) > 0
                                              }
                                            >
                                              <LuTrash2 className="w-4 h-4" />
                                            </Button>
                                          </span>
                                        </TooltipTrigger>
                                        <TooltipContent>
                                          {(proxyUsage[proxy.id] ?? 0) > 0 ? (
                                            <p>
                                              {(proxyUsage[proxy.id] ?? 0) === 1
                                                ? t(
                                                    "proxies.management.cannotDelete_one",
                                                    {
                                                      count:
                                                        proxyUsage[proxy.id],
                                                    },
                                                  )
                                                : t(
                                                    "proxies.management.cannotDelete_other",
                                                    {
                                                      count:
                                                        proxyUsage[proxy.id],
                                                    },
                                                  )}
                                            </p>
                                          ) : (
                                            <p>
                                              {t(
                                                "proxies.management.deleteProxy",
                                              )}
                                            </p>
                                          )}
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
              </TabsContent>

              <TabsContent value="vpns">
                <div className="space-y-4">
                  <div className="flex justify-between items-center">
                    <div className="flex gap-2">
                      <RippleButton
                        size="sm"
                        variant="outline"
                        onClick={() => {
                          setShowVpnImportDialog(true);
                        }}
                        className="flex gap-2 items-center"
                      >
                        <LuUpload className="w-4 h-4" />
                        {t("common.buttons.import")}
                      </RippleButton>
                    </div>
                    <RippleButton
                      size="sm"
                      onClick={handleCreateVpn}
                      className="flex gap-2 items-center"
                    >
                      <GoPlus className="w-4 h-4" />
                      {t("proxies.management.create")}
                    </RippleButton>
                  </div>

                  {isLoadingVpns ? (
                    <div className="text-sm text-muted-foreground">
                      {t("vpns.management.loading")}
                    </div>
                  ) : vpnConfigs.length === 0 ? (
                    <div className="text-sm text-muted-foreground">
                      {t("vpns.management.noneCreated")}
                    </div>
                  ) : (
                    <div className="border rounded-md">
                      <ScrollArea className="h-[240px]">
                        <Table>
                          <TableHeader>
                            <TableRow>
                              <TableHead>{t("common.labels.name")}</TableHead>
                              <TableHead className="w-16">
                                {t("common.labels.type")}
                              </TableHead>
                              <TableHead className="w-20">
                                {t("proxies.management.usage")}
                              </TableHead>
                              <TableHead className="w-24">
                                {t("proxies.management.syncCol")}
                              </TableHead>
                              <TableHead className="w-24">
                                {t("common.labels.actions")}
                              </TableHead>
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {vpnConfigs.map((vpn) => {
                              const syncDot = getSyncStatusDot(
                                vpn,
                                vpnSyncStatus[vpn.id],
                                t,
                                vpnSyncErrors[vpn.id],
                              );
                              return (
                                <TableRow key={vpn.id}>
                                  <TableCell className="font-medium">
                                    <div className="flex items-center gap-2">
                                      <Tooltip>
                                        <TooltipTrigger asChild>
                                          <div
                                            className={`w-2 h-2 rounded-full shrink-0 ${syncDot.color} ${
                                              syncDot.animate
                                                ? "animate-pulse"
                                                : ""
                                            }`}
                                          />
                                        </TooltipTrigger>
                                        <TooltipContent>
                                          <p>{syncDot.tooltip}</p>
                                        </TooltipContent>
                                      </Tooltip>
                                      {vpn.name}
                                    </div>
                                  </TableCell>
                                  <TableCell>
                                    <Badge variant="outline">WG</Badge>
                                  </TableCell>
                                  <TableCell>
                                    <Badge variant="secondary">
                                      {vpnUsage[vpn.id] ?? 0}
                                    </Badge>
                                  </TableCell>
                                  <TableCell>
                                    <Tooltip>
                                      <TooltipTrigger asChild>
                                        <div className="flex items-center">
                                          <Checkbox
                                            checked={vpn.sync_enabled}
                                            onCheckedChange={() =>
                                              void handleToggleVpnSync(vpn)
                                            }
                                            disabled={
                                              isTogglingVpnSync[vpn.id] ||
                                              vpnInUse[vpn.id]
                                            }
                                          />
                                        </div>
                                      </TooltipTrigger>
                                      <TooltipContent>
                                        {vpnInUse[vpn.id] ? (
                                          <p>
                                            {t(
                                              "vpns.management.syncCannotDisable",
                                            )}
                                          </p>
                                        ) : (
                                          <p>
                                            {vpn.sync_enabled
                                              ? t(
                                                  "proxies.management.disableSync",
                                                )
                                              : t(
                                                  "proxies.management.enableSync",
                                                )}
                                          </p>
                                        )}
                                      </TooltipContent>
                                    </Tooltip>
                                  </TableCell>
                                  <TableCell>
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
                                            <LuPencil className="w-4 h-4" />
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
                                              disabled={
                                                (vpnUsage[vpn.id] ?? 0) > 0
                                              }
                                            >
                                              <LuTrash2 className="w-4 h-4" />
                                            </Button>
                                          </span>
                                        </TooltipTrigger>
                                        <TooltipContent>
                                          {(vpnUsage[vpn.id] ?? 0) > 0 ? (
                                            <p>
                                              {(vpnUsage[vpn.id] ?? 0) === 1
                                                ? t(
                                                    "vpns.management.cannotDelete_one",
                                                    { count: vpnUsage[vpn.id] },
                                                  )
                                                : t(
                                                    "vpns.management.cannotDelete_other",
                                                    { count: vpnUsage[vpn.id] },
                                                  )}
                                            </p>
                                          ) : (
                                            <p>
                                              {t("vpns.management.deleteVpn")}
                                            </p>
                                          )}
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
              </TabsContent>
            </Tabs>
          </ScrollArea>

          <DialogFooter>
            <RippleButton variant="outline" onClick={onClose}>
              {t("common.buttons.close")}
            </RippleButton>
          </DialogFooter>
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
    </>
  );
}

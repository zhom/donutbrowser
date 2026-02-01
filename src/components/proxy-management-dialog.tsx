"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { ProxyCheckResult, StoredProxy } from "@/types";
import { ProxyCheckButton } from "./proxy-check-button";
import { RippleButton } from "./ui/ripple";

type SyncStatus = "disabled" | "syncing" | "synced" | "error" | "waiting";

function getSyncStatusDot(
  proxy: StoredProxy,
  liveStatus: SyncStatus | undefined,
): { color: string; tooltip: string; animate: boolean } {
  const status = liveStatus ?? (proxy.sync_enabled ? "synced" : "disabled");

  switch (status) {
    case "syncing":
      return { color: "bg-yellow-500", tooltip: "Syncing...", animate: true };
    case "synced":
      return {
        color: "bg-green-500",
        tooltip: proxy.last_sync
          ? `Synced ${new Date(proxy.last_sync * 1000).toLocaleString()}`
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

interface ProxyManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function ProxyManagementDialog({
  isOpen,
  onClose,
}: ProxyManagementDialogProps) {
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
  const [proxyInUse, setProxyInUse] = useState<Record<string, boolean>>({});
  const [isTogglingSync, setIsTogglingSync] = useState<Record<string, boolean>>(
    {},
  );

  const { storedProxies, proxyUsage, isLoading } = useProxyEvents();

  // Listen for proxy sync status events
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      unlisten = await listen<{ id: string; status: string }>(
        "proxy-sync-status",
        (event) => {
          const { id, status } = event.payload;
          setProxySyncStatus((prev) => ({
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

  const handleDeleteProxy = useCallback((proxy: StoredProxy) => {
    // Open in-app confirmation dialog
    setProxyToDelete(proxy);
  }, []);

  const handleConfirmDelete = useCallback(async () => {
    if (!proxyToDelete) return;
    setIsDeleting(true);
    try {
      await invoke("delete_stored_proxy", { proxyId: proxyToDelete.id });
      toast.success("Proxy deleted successfully");
      await emit("stored-proxies-changed");
    } catch (error) {
      console.error("Failed to delete proxy:", error);
      toast.error("Failed to delete proxy");
    } finally {
      setIsDeleting(false);
      setProxyToDelete(null);
    }
  }, [proxyToDelete]);

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

  const handleToggleSync = useCallback(async (proxy: StoredProxy) => {
    setIsTogglingSync((prev) => ({ ...prev, [proxy.id]: true }));
    try {
      await invoke("set_proxy_sync_enabled", {
        proxyId: proxy.id,
        enabled: !proxy.sync_enabled,
      });
      showSuccessToast(proxy.sync_enabled ? "Sync disabled" : "Sync enabled");
      await emit("stored-proxies-changed");
    } catch (error) {
      console.error("Failed to toggle sync:", error);
      showErrorToast(
        error instanceof Error ? error.message : "Failed to update sync",
      );
    } finally {
      setIsTogglingSync((prev) => ({ ...prev, [proxy.id]: false }));
    }
  }, []);

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>Proxy Management</DialogTitle>
            <DialogDescription>
              Manage your saved proxy configurations for reuse across profiles
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            {/* Proxy actions */}
            <div className="flex justify-between items-center">
              <div className="flex gap-2">
                <RippleButton
                  size="sm"
                  variant="outline"
                  onClick={() => setShowImportDialog(true)}
                  className="flex gap-2 items-center"
                >
                  <LuUpload className="w-4 h-4" />
                  Import
                </RippleButton>
                <RippleButton
                  size="sm"
                  variant="outline"
                  onClick={() => setShowExportDialog(true)}
                  className="flex gap-2 items-center"
                  disabled={storedProxies.length === 0}
                >
                  <LuDownload className="w-4 h-4" />
                  Export
                </RippleButton>
              </div>
              <RippleButton
                size="sm"
                onClick={handleCreateProxy}
                className="flex gap-2 items-center"
              >
                <GoPlus className="w-4 h-4" />
                Create
              </RippleButton>
            </div>

            {/* Proxies list */}
            {isLoading ? (
              <div className="text-sm text-muted-foreground">
                Loading proxies...
              </div>
            ) : storedProxies.length === 0 ? (
              <div className="text-sm text-muted-foreground">
                No proxies created yet. Create your first proxy using the button
                above.
              </div>
            ) : (
              <div className="border rounded-md">
                <ScrollArea className="h-[240px]">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Name</TableHead>
                        <TableHead className="w-20">Usage</TableHead>
                        <TableHead className="w-24">Sync</TableHead>
                        <TableHead className="w-24">Actions</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {storedProxies.map((proxy) => {
                        const syncDot = getSyncStatusDot(
                          proxy,
                          proxySyncStatus[proxy.id],
                        );
                        return (
                          <TableRow key={proxy.id}>
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
                                        handleToggleSync(proxy)
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
                                      Sync cannot be disabled while this proxy
                                      is used by synced profiles
                                    </p>
                                  ) : (
                                    <p>
                                      {proxy.sync_enabled
                                        ? "Disable sync"
                                        : "Enable sync"}
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
                                  cachedResult={proxyCheckResults[proxy.id]}
                                  setCheckingProfileId={setCheckingProxyId}
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
                                      onClick={() => handleEditProxy(proxy)}
                                    >
                                      <LuPencil className="w-4 h-4" />
                                    </Button>
                                  </TooltipTrigger>
                                  <TooltipContent>
                                    <p>Edit proxy</p>
                                  </TooltipContent>
                                </Tooltip>
                                <Tooltip>
                                  <TooltipTrigger asChild>
                                    <span>
                                      <Button
                                        variant="ghost"
                                        size="sm"
                                        onClick={() => handleDeleteProxy(proxy)}
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
                                        Cannot delete: in use by{" "}
                                        {proxyUsage[proxy.id]} profile
                                        {proxyUsage[proxy.id] > 1 ? "s" : ""}
                                      </p>
                                    ) : (
                                      <p>Delete proxy</p>
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

          <DialogFooter>
            <RippleButton variant="outline" onClick={onClose}>
              Close
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
        onClose={() => setProxyToDelete(null)}
        onConfirm={handleConfirmDelete}
        title="Delete Proxy"
        description={`This action cannot be undone. This will permanently delete the proxy "${proxyToDelete?.name ?? ""}".`}
        confirmButtonText="Delete"
        isLoading={isDeleting}
      />
      <ProxyImportDialog
        isOpen={showImportDialog}
        onClose={() => setShowImportDialog(false)}
      />
      <ProxyExportDialog
        isOpen={showExportDialog}
        onClose={() => setShowExportDialog(false)}
      />
    </>
  );
}

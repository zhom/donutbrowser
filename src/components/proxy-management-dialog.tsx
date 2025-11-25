"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import * as React from "react";
import { useCallback, useState } from "react";
import { FiEdit2, FiPlus, FiTrash2, FiWifi } from "react-icons/fi";
import { toast } from "sonner";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useProxyEvents } from "@/hooks/use-proxy-events";
import { trimName } from "@/lib/name-utils";
import type { ProxyCheckResult, StoredProxy } from "@/types";
import { ProxyCheckButton } from "./proxy-check-button";
import { RippleButton } from "./ui/ripple";

interface ProxyManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function ProxyManagementDialog({
  isOpen,
  onClose,
}: ProxyManagementDialogProps) {
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [editingProxy, setEditingProxy] = useState<StoredProxy | null>(null);
  const [proxyToDelete, setProxyToDelete] = useState<StoredProxy | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [checkingProxyId, setCheckingProxyId] = useState<string | null>(null);
  const [proxyCheckResults, setProxyCheckResults] = useState<
    Record<string, ProxyCheckResult>
  >({});

  const { storedProxies, proxyUsage, isLoading } = useProxyEvents();

  // Load cached check results on mount and when proxies change
  React.useEffect(() => {
    const loadCachedResults = async () => {
      const results: Record<string, ProxyCheckResult> = {};
      for (const proxy of storedProxies) {
        try {
          const cached = await invoke<ProxyCheckResult | null>(
            "get_cached_proxy_check",
            { proxyId: proxy.id },
          );
          if (cached) {
            results[proxy.id] = cached;
          }
        } catch (_error) {
          // Ignore errors
        }
      }
      setProxyCheckResults(results);
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

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose}>
        <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
          <DialogHeader className="flex-shrink-0">
            <div className="flex gap-2 items-center">
              <FiWifi className="w-5 h-5" />
              <DialogTitle>Proxy Management</DialogTitle>
            </div>
          </DialogHeader>

          <div className="flex flex-col flex-1 gap-4 py-4 min-h-0">
            {/* Header with Create Button */}
            <div className="flex flex-shrink-0 justify-between items-center">
              <div>
                <h3 className="text-lg font-medium">Stored Proxies</h3>
                <p className="text-sm text-muted-foreground">
                  Manage your saved proxy configurations for reuse across
                  profiles
                </p>
              </div>
              <RippleButton
                onClick={handleCreateProxy}
                className="flex gap-2 items-center"
              >
                <FiPlus className="w-4 h-4" />
                Create Proxy
              </RippleButton>
            </div>

            {/* Proxy List - Scrollable */}
            <div className="flex-1 min-h-0">
              {isLoading && (
                <div className="flex justify-center items-center py-6">
                  <div className="w-8 h-8 rounded-full border-b-2 animate-spin border-primary"></div>
                </div>
              )}
              {storedProxies.length === 0 && !isLoading ? (
                <div className="flex flex-col justify-center items-center h-32 text-center">
                  <FiWifi className="mx-auto mb-4 w-12 h-12 text-muted-foreground" />
                  <p className="mb-2 text-muted-foreground">
                    No proxies configured
                  </p>
                  <p className="mb-4 text-sm text-muted-foreground">
                    Create your first proxy configuration to get started
                  </p>
                  <RippleButton variant="outline" onClick={handleCreateProxy}>
                    <FiPlus className="mr-2 w-4 h-4" />
                    Create First Proxy
                  </RippleButton>
                </div>
              ) : (
                <ScrollArea className="h-[240px] pr-2">
                  <div className="space-y-2">
                    {storedProxies.map((proxy) => (
                      <div
                        key={proxy.id}
                        className="flex justify-between items-center p-1 rounded border bg-card"
                      >
                        <div className="flex-1 ml-2 min-w-0">
                          {proxy.name.length > 30 ? (
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <span className="block font-medium truncate text-card-foreground">
                                  {trimName(proxy.name)}
                                </span>
                              </TooltipTrigger>
                              <TooltipContent>
                                <span className="text-sm font-medium text-card-foreground">
                                  {proxy.name}
                                </span>
                              </TooltipContent>
                            </Tooltip>
                          ) : (
                            <span className="text-sm font-medium text-card-foreground">
                              {proxy.name}
                            </span>
                          )}
                        </div>
                        <div className="mr-2">
                          <Badge variant="secondary">
                            {proxyUsage[proxy.id] ?? 0}
                          </Badge>
                        </div>
                        <div className="flex shrink-0 gap-1 items-center">
                          <ProxyCheckButton
                            proxy={proxy}
                            checkingProxyId={checkingProxyId}
                            cachedResult={proxyCheckResults[proxy.id]}
                            setCheckingProxyId={setCheckingProxyId}
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
                                <FiEdit2 className="w-4 h-4" />
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
                                  className="text-destructive hover:text-destructive"
                                  disabled={(proxyUsage[proxy.id] ?? 0) > 0}
                                >
                                  <FiTrash2 className="w-4 h-4" />
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
                      </div>
                    ))}
                  </div>
                </ScrollArea>
              )}
            </div>
          </div>

          <DialogFooter className="flex-shrink-0">
            <RippleButton onClick={onClose}>Close</RippleButton>
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
    </>
  );
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { trimName } from "@/lib/name-utils";
import type { StoredProxy } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyManagementDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function ProxyManagementDialog({
  isOpen,
  onClose,
}: ProxyManagementDialogProps) {
  const [storedProxies, setStoredProxies] = useState<StoredProxy[]>([]);
  const [loading, setLoading] = useState(false);
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [editingProxy, setEditingProxy] = useState<StoredProxy | null>(null);
  const [proxyUsage, setProxyUsage] = useState<Record<string, number>>({});
  const [proxyToDelete, setProxyToDelete] = useState<StoredProxy | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const loadStoredProxies = useCallback(async () => {
    try {
      setLoading(true);
      const proxies = await invoke<StoredProxy[]>("get_stored_proxies");
      setStoredProxies(proxies);
    } catch (error) {
      console.error("Failed to load stored proxies:", error);
      toast.error("Failed to load proxies");
    } finally {
      setLoading(false);
    }
  }, []);

  const loadProxyUsage = useCallback(async () => {
    try {
      const profiles = await invoke<Array<{ proxy_id?: string }>>(
        "list_browser_profiles",
      );
      const counts: Record<string, number> = {};
      for (const p of profiles) {
        if (p.proxy_id) counts[p.proxy_id] = (counts[p.proxy_id] ?? 0) + 1;
      }
      setProxyUsage(counts);
    } catch (_err) {
      // ignore non-critical errors
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      loadStoredProxies();
      void loadProxyUsage();
    }
  }, [isOpen, loadStoredProxies, loadProxyUsage]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      try {
        unlisten = await listen("profile-updated", () => {
          void loadProxyUsage();
        });
      } catch (_err) {
        // ignore non-critical errors
      }
    };
    if (isOpen) void setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, [isOpen, loadProxyUsage]);

  // Keep list in sync with external changes (e.g., created from CreateProfileDialog)
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      try {
        unlisten = await listen("stored-proxies-changed", () => {
          void loadStoredProxies();
          void loadProxyUsage();
        });
      } catch (_err) {
        // ignore non-critical errors
      }
    };
    if (isOpen) void setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, [isOpen, loadStoredProxies, loadProxyUsage]);

  const handleDeleteProxy = useCallback((proxy: StoredProxy) => {
    // Open in-app confirmation dialog
    setProxyToDelete(proxy);
  }, []);

  const handleConfirmDelete = useCallback(async () => {
    if (!proxyToDelete) return;
    setIsDeleting(true);
    try {
      await invoke("delete_stored_proxy", { proxyId: proxyToDelete.id });
      setStoredProxies((prev) => prev.filter((p) => p.id !== proxyToDelete.id));
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

  const handleProxySaved = useCallback((savedProxy: StoredProxy) => {
    setStoredProxies((prev) => {
      const existingIndex = prev.findIndex((p) => p.id === savedProxy.id);
      if (existingIndex >= 0) {
        // Update existing proxy
        const updated = [...prev];
        updated[existingIndex] = savedProxy;
        return updated;
      } else {
        // Add new proxy
        return [...prev, savedProxy];
      }
    });
    setShowProxyForm(false);
    setEditingProxy(null);
    void emit("stored-proxies-changed");
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
              {loading ? (
                <div className="flex justify-center items-center h-32">
                  <p className="text-sm text-muted-foreground">
                    Loading proxies...
                  </p>
                </div>
              ) : storedProxies.length === 0 ? (
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
                <div className="overflow-y-auto pr-2 space-y-2 h-full">
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
                      <div className="flex flex-shrink-0 gap-1 items-center">
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
                                Cannot delete: in use by {proxyUsage[proxy.id]}{" "}
                                profile
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
        onSave={handleProxySaved}
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

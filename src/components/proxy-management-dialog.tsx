"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { FiEdit2, FiPlus, FiTrash2, FiWifi } from "react-icons/fi";
import { toast } from "sonner";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
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
import type { StoredProxy } from "@/types";

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

  useEffect(() => {
    if (isOpen) {
      loadStoredProxies();
    }
  }, [isOpen, loadStoredProxies]);

  const handleDeleteProxy = useCallback(async (proxy: StoredProxy) => {
    if (
      !confirm(`Are you sure you want to delete the proxy "${proxy.name}"?`)
    ) {
      return;
    }

    try {
      await invoke("delete_stored_proxy", { proxyId: proxy.id });
      setStoredProxies((prev) => prev.filter((p) => p.id !== proxy.id));
      toast.success("Proxy deleted successfully");
    } catch (error) {
      console.error("Failed to delete proxy:", error);
      toast.error("Failed to delete proxy");
    }
  }, []);

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
  }, []);

  const handleProxyFormClose = useCallback(() => {
    setShowProxyForm(false);
    setEditingProxy(null);
  }, []);

  const trimName = useCallback((name: string) => {
    return name.length > 30 ? `${name.substring(0, 30)}...` : name;
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
              <Button
                onClick={handleCreateProxy}
                className="flex gap-2 items-center"
              >
                <FiPlus className="w-4 h-4" />
                Create Proxy
              </Button>
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
                  <Button variant="outline" onClick={handleCreateProxy}>
                    <FiPlus className="mr-2 w-4 h-4" />
                    Create First Proxy
                  </Button>
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
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => handleDeleteProxy(proxy)}
                              className="text-destructive hover:text-destructive"
                            >
                              <FiTrash2 className="w-4 h-4" />
                            </Button>
                          </TooltipTrigger>
                          <TooltipContent>
                            <p>Delete proxy</p>
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
            <Button onClick={onClose}>Close</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ProxyFormDialog
        isOpen={showProxyForm}
        onClose={handleProxyFormClose}
        onSave={handleProxySaved}
        editingProxy={editingProxy}
      />
    </>
  );
}

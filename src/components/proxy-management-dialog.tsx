"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import * as React from "react";
import { useCallback, useState } from "react";
import { GoPlus } from "react-icons/go";
import { LuPencil, LuTrash2 } from "react-icons/lu";
import { toast } from "sonner";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import { useProxyEvents } from "@/hooks/use-proxy-events";
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
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>Proxy Management</DialogTitle>
            <DialogDescription>
              Manage your saved proxy configurations for reuse across profiles
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            {/* Create new proxy button */}
            <div className="flex justify-between items-center">
              <Label>Proxies</Label>
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
                        <TableHead className="w-24">Actions</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {storedProxies.map((proxy) => (
                        <TableRow key={proxy.id}>
                          <TableCell className="font-medium">
                            {proxy.name}
                          </TableCell>
                          <TableCell>
                            <Badge variant="secondary">
                              {proxyUsage[proxy.id] ?? 0}
                            </Badge>
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
                                      disabled={(proxyUsage[proxy.id] ?? 0) > 0}
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
                      ))}
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
    </>
  );
}

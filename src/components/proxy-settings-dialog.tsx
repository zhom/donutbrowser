"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { FiPlus } from "react-icons/fi";
import { toast } from "sonner";
import { ProxyFormDialog } from "@/components/proxy-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { StoredProxy } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxySettingsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (proxyId: string | null) => void;
  initialProxyId?: string | null;
  browserType?: string;
}

export function ProxySettingsDialog({
  isOpen,
  onClose,
  onSave,
  initialProxyId,
  browserType,
}: ProxySettingsDialogProps) {
  const [storedProxies, setStoredProxies] = useState<StoredProxy[]>([]);
  const [selectedProxyId, setSelectedProxyId] = useState<string | null>(
    initialProxyId || null,
  );
  const [loading, setLoading] = useState(false);
  const [showProxyForm, setShowProxyForm] = useState(false);
  const [proxyUsage, setProxyUsage] = useState<Record<string, number>>({});

  // Helper to determine if proxy should be disabled for the selected browser
  const isProxyDisabled = browserType === "tor-browser";

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
        if (p.proxy_id) {
          counts[p.proxy_id] = (counts[p.proxy_id] ?? 0) + 1;
        }
      }
      setProxyUsage(counts);
    } catch (error) {
      // Non-fatal
      console.error("Failed to load proxy usage:", error);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      loadStoredProxies();
      void loadProxyUsage();
      if (isProxyDisabled) {
        setSelectedProxyId(null);
      } else {
        // Reset to initial proxy ID when dialog opens
        setSelectedProxyId(initialProxyId || null);
      }
    }
  }, [
    isOpen,
    isProxyDisabled,
    loadStoredProxies,
    initialProxyId,
    loadProxyUsage,
  ]);

  // Refresh usage when profiles change
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      try {
        unlisten = await listen("profile-updated", () => {
          void loadProxyUsage();
        });
      } catch (e) {
        console.error(e);
      }
    };
    if (isOpen) void setup();
    return () => {
      if (unlisten) unlisten();
    };
  }, [isOpen, loadProxyUsage]);

  const handleCreateProxy = useCallback(() => {
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
    setSelectedProxyId(savedProxy.id);
    setShowProxyForm(false);
  }, []);

  const handleProxyFormClose = useCallback(() => {
    setShowProxyForm(false);
  }, []);

  const handleSave = () => {
    onSave(selectedProxyId);
  };

  const hasChanged = () => {
    return selectedProxyId !== initialProxyId;
  };

  return (
    <>
      <Dialog
        open={isOpen}
        onOpenChange={(open) => {
          if (!open) {
            onClose();
          }
        }}
      >
        <DialogContent className="max-w-md max-h-[80vh] my-8 flex flex-col">
          <DialogHeader className="flex-shrink-0">
            <DialogTitle>Proxy Settings</DialogTitle>
          </DialogHeader>

          <div className="grid gap-6 py-4">
            {isProxyDisabled && (
              <div className="p-4 bg-yellow-50 rounded-md border border-yellow-200 dark:bg-yellow-900/20 dark:border-yellow-800">
                <p className="text-sm text-yellow-800 dark:text-yellow-200">
                  Tor Browser has its own built-in proxy system and doesn't
                  support additional proxy configuration.
                </p>
              </div>
            )}

            {!isProxyDisabled && (
              <>
                {/* Proxy Selection */}
                <div className="space-y-3">
                  <div className="flex justify-between items-center">
                    <Label className="text-base font-medium">
                      Select Proxy
                    </Label>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <RippleButton
                          variant="outline"
                          size="sm"
                          onClick={handleCreateProxy}
                          className="flex gap-2 items-center"
                        >
                          <FiPlus className="w-4 h-4" />
                          Create
                        </RippleButton>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p>Create a new proxy configuration</p>
                      </TooltipContent>
                    </Tooltip>
                  </div>

                  <div className="overflow-y-auto p-2 space-y-2 h-full">
                    <Button
                      variant="ghost"
                      onClick={() => setSelectedProxyId(null)}
                      asChild
                    >
                      <Card
                        className={cn(
                          "w-full bg-card cursor-pointer transition-colors",
                          selectedProxyId === null
                            ? "ring-2 ring-blue-500"
                            : "",
                        )}
                      >
                        <CardContent className="p-4 w-full">
                          <div className="flex items-center space-x-3">
                            <input
                              type="radio"
                              id="no-proxy"
                              name="proxy-selection"
                              checked={selectedProxyId === null}
                              onChange={() => setSelectedProxyId(null)}
                            />
                            <div className="flex gap-2 items-center">
                              <Label
                                htmlFor="no-proxy"
                                className="font-medium cursor-pointer"
                              >
                                No Proxy
                              </Label>
                            </div>
                          </div>
                        </CardContent>
                      </Card>
                    </Button>

                    {loading ? (
                      <p className="text-sm text-muted-foreground">
                        Loading proxies...
                      </p>
                    ) : (
                      storedProxies.map((proxy) => (
                        <Button
                          key={proxy.id}
                          variant="ghost"
                          onClick={() => setSelectedProxyId(proxy.id)}
                          asChild
                        >
                          <Card
                            className={cn(
                              "w-full bg-card cursor-pointer transition-colors",
                              selectedProxyId === proxy.id
                                ? "ring-2 ring-blue-500"
                                : "",
                            )}
                          >
                            <CardContent className="p-4 w-full">
                              <div className="flex items-center space-x-3">
                                <input
                                  type="radio"
                                  id={`proxy-${proxy.id}`}
                                  name="proxy-selection"
                                  checked={selectedProxyId === proxy.id}
                                  onChange={() => setSelectedProxyId(proxy.id)}
                                />
                                <div className="flex gap-2 items-center">
                                  <Label
                                    htmlFor={`proxy-${proxy.id}`}
                                    className="font-medium cursor-pointer"
                                  >
                                    {proxy.name}
                                  </Label>
                                  <Badge variant="outline">
                                    {proxy.proxy_settings.proxy_type.toUpperCase()}
                                  </Badge>
                                  <Badge>{proxyUsage[proxy.id] ?? 0}</Badge>
                                </div>
                              </div>
                            </CardContent>
                          </Card>
                        </Button>
                      ))
                    )}

                    {!loading && storedProxies.length === 0 && (
                      <div className="py-4 text-center">
                        <p className="mb-2 text-sm text-muted-foreground">
                          No saved proxies available.
                        </p>
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={handleCreateProxy}
                        >
                          <FiPlus className="mr-2 w-4 h-4" />
                          Create First Proxy
                        </Button>
                      </div>
                    )}
                  </div>
                </div>
              </>
            )}
          </div>

          <DialogFooter>
            <RippleButton variant="outline" onClick={onClose}>
              Cancel
            </RippleButton>
            <RippleButton onClick={handleSave} disabled={!hasChanged()}>
              Save
            </RippleButton>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ProxyFormDialog
        isOpen={showProxyForm}
        onClose={handleProxyFormClose}
        onSave={handleProxySaved}
      />
    </>
  );
}

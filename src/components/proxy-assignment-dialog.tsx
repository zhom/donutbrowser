"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { BrowserProfile, StoredProxy } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyAssignmentDialogProps {
  isOpen: boolean;
  onClose: () => void;
  selectedProfiles: string[];
  onAssignmentComplete: () => void;
  profiles?: BrowserProfile[];
  storedProxies?: StoredProxy[];
}

export function ProxyAssignmentDialog({
  isOpen,
  onClose,
  selectedProfiles,
  onAssignmentComplete,
  profiles = [],
  storedProxies = [],
}: ProxyAssignmentDialogProps) {
  const [selectedProxyId, setSelectedProxyId] = useState<string | null>(null);
  const [isAssigning, setIsAssigning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleAssign = useCallback(async () => {
    setIsAssigning(true);
    setError(null);
    try {
      // Filter out TOR browser profiles as they don't support proxies
      const validProfiles = selectedProfiles.filter((profileId) => {
        const profile = profiles.find((p) => p.id === profileId);
        return profile && profile.browser !== "tor-browser";
      });

      if (validProfiles.length === 0) {
        setError("No valid profiles selected.");
        setIsAssigning(false);
        return;
      }

      // Update each profile's proxy sequentially to avoid file locking issues
      for (const profileId of validProfiles) {
        await invoke("update_profile_proxy", {
          profileId,
          proxyId: selectedProxyId,
        });
      }

      // Notify other parts of the app so usage counts and lists refresh
      await emit("profile-updated");
      onAssignmentComplete();
      onClose();
    } catch (err) {
      console.error("Failed to assign proxies to profiles:", err);
      const errorMessage =
        err instanceof Error
          ? err.message
          : "Failed to assign proxies to profiles";
      setError(errorMessage);
      toast.error(errorMessage);
    } finally {
      setIsAssigning(false);
    }
  }, [
    selectedProfiles,
    selectedProxyId,
    profiles,
    onAssignmentComplete,
    onClose,
  ]);

  useEffect(() => {
    if (isOpen) {
      setSelectedProxyId(null);
      setError(null);
    }
  }, [isOpen]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Assign Proxy</DialogTitle>
          <DialogDescription>
            Assign a proxy to {selectedProfiles.length} selected profile(s).
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label>Selected Profiles:</Label>
            <div className="p-3 bg-muted rounded-md max-h-32 overflow-y-auto">
              <ul className="text-sm space-y-1">
                {selectedProfiles.map((profileId) => {
                  const profile = profiles.find(
                    (p: BrowserProfile) => p.id === profileId,
                  );
                  const displayName = profile ? profile.name : profileId;
                  const isTorBrowser = profile?.browser === "tor-browser";
                  return (
                    <li key={profileId} className="truncate">
                      • {displayName}
                      {isTorBrowser && (
                        <span className="ml-2 text-xs text-muted-foreground">
                          (TOR - no proxy support)
                        </span>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="proxy-select">Assign Proxy:</Label>
            <Select
              value={selectedProxyId || "none"}
              onValueChange={(value) => {
                setSelectedProxyId(value === "none" ? null : value);
              }}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select a proxy" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">No Proxy</SelectItem>
                {storedProxies.map((proxy) => (
                  <SelectItem key={proxy.id} value={proxy.id}>
                    {proxy.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {error && (
            <div className="p-3 text-sm text-red-600 bg-red-50 rounded-md dark:bg-red-900/20 dark:text-red-400">
              {error}
            </div>
          )}
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={onClose}
            disabled={isAssigning}
          >
            Cancel
          </RippleButton>
          <LoadingButton
            isLoading={isAssigning}
            onClick={() => void handleAssign()}
          >
            Assign
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

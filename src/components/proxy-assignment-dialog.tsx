"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
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
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { BrowserProfile, StoredProxy, VpnConfig } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyAssignmentDialogProps {
  isOpen: boolean;
  onClose: () => void;
  selectedProfiles: string[];
  onAssignmentComplete: () => void;
  profiles?: BrowserProfile[];
  storedProxies?: StoredProxy[];
  vpnConfigs?: VpnConfig[];
}

export function ProxyAssignmentDialog({
  isOpen,
  onClose,
  selectedProfiles,
  onAssignmentComplete,
  profiles = [],
  storedProxies = [],
  vpnConfigs = [],
}: ProxyAssignmentDialogProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectionType, setSelectionType] = useState<"none" | "proxy" | "vpn">(
    "none",
  );
  const [isAssigning, setIsAssigning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleValueChange = useCallback((value: string) => {
    if (value === "none") {
      setSelectedId(null);
      setSelectionType("none");
    } else if (value.startsWith("vpn-")) {
      setSelectedId(value.slice(4));
      setSelectionType("vpn");
    } else {
      setSelectedId(value);
      setSelectionType("proxy");
    }
  }, []);

  const handleAssign = useCallback(async () => {
    setIsAssigning(true);
    setError(null);
    try {
      const validProfiles = selectedProfiles.filter((profileId) => {
        const profile = profiles.find((p) => p.id === profileId);
        return profile;
      });

      if (validProfiles.length === 0) {
        setError("No valid profiles selected.");
        setIsAssigning(false);
        return;
      }

      for (const profileId of validProfiles) {
        if (selectionType === "vpn") {
          await invoke("update_profile_vpn", {
            profileId,
            vpnId: selectedId,
          });
        } else {
          await invoke("update_profile_proxy", {
            profileId,
            proxyId: selectionType === "proxy" ? selectedId : null,
          });
        }
      }

      await emit("profile-updated");
      onAssignmentComplete();
      onClose();
    } catch (err) {
      console.error("Failed to assign proxy/VPN to profiles:", err);
      const errorMessage =
        err instanceof Error
          ? err.message
          : "Failed to assign proxy/VPN to profiles";
      setError(errorMessage);
      toast.error(errorMessage);
    } finally {
      setIsAssigning(false);
    }
  }, [
    selectedProfiles,
    selectedId,
    selectionType,
    profiles,
    onAssignmentComplete,
    onClose,
  ]);

  useEffect(() => {
    if (isOpen) {
      setSelectedId(null);
      setSelectionType("none");
      setError(null);
    }
  }, [isOpen]);

  const selectValue =
    selectionType === "none"
      ? "none"
      : selectionType === "vpn"
        ? `vpn-${selectedId}`
        : (selectedId ?? "none");

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Assign Proxy / VPN</DialogTitle>
          <DialogDescription>
            Assign a proxy or VPN to {selectedProfiles.length} selected
            profile(s).
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
                  return (
                    <li key={profileId} className="truncate">
                      &bull; {displayName}
                    </li>
                  );
                })}
              </ul>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="proxy-vpn-select">Assign Proxy / VPN:</Label>
            <Select value={selectValue} onValueChange={handleValueChange}>
              <SelectTrigger>
                <SelectValue placeholder="Select a proxy or VPN" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">None</SelectItem>
                {storedProxies.length > 0 && (
                  <SelectGroup>
                    <SelectLabel>Proxies</SelectLabel>
                    {storedProxies.map((proxy) => (
                      <SelectItem key={proxy.id} value={proxy.id}>
                        {proxy.name}
                        {proxy.is_cloud_managed ? " (Included)" : ""}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                )}
                {vpnConfigs.length > 0 && (
                  <SelectGroup>
                    <SelectLabel>VPNs</SelectLabel>
                    {vpnConfigs.map((vpn) => (
                      <SelectItem key={vpn.id} value={`vpn-${vpn.id}`}>
                        <span className="flex items-center gap-1">
                          <Badge
                            variant="outline"
                            className="text-[10px] px-1 py-0 leading-tight"
                          >
                            {vpn.vpn_type === "WireGuard" ? "WG" : "OVPN"}
                          </Badge>
                          {vpn.name}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectGroup>
                )}
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

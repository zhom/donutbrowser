"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { LuCheck, LuChevronsUpDown } from "react-icons/lu";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
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
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
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
  const [proxyPopoverOpen, setProxyPopoverOpen] = useState(false);
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
            <Popover open={proxyPopoverOpen} onOpenChange={setProxyPopoverOpen}>
              <PopoverTrigger asChild>
                <Button
                  variant="outline"
                  role="combobox"
                  aria-expanded={proxyPopoverOpen}
                  className="w-full justify-between font-normal"
                >
                  {(() => {
                    if (selectionType === "none") return "None";
                    if (selectionType === "vpn") {
                      const vpn = vpnConfigs.find((v) => v.id === selectedId);
                      return vpn
                        ? `${vpn.vpn_type === "WireGuard" ? "WG" : "OVPN"} — ${vpn.name}`
                        : "None";
                    }
                    const proxy = storedProxies.find(
                      (p) => p.id === selectedId,
                    );
                    return proxy
                      ? `${proxy.name}${proxy.is_cloud_managed ? " (Included)" : ""}`
                      : "None";
                  })()}
                  <LuChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
                </Button>
              </PopoverTrigger>
              <PopoverContent className="w-[240px] p-0" sideOffset={8}>
                <Command>
                  <CommandInput placeholder="Search proxies or VPNs..." />
                  <CommandList>
                    <CommandEmpty>No proxies or VPNs found.</CommandEmpty>
                    <CommandGroup>
                      <CommandItem
                        value="__none__"
                        onSelect={() => {
                          handleValueChange("none");
                          setProxyPopoverOpen(false);
                        }}
                      >
                        <LuCheck
                          className={cn(
                            "mr-2 h-4 w-4",
                            selectionType === "none"
                              ? "opacity-100"
                              : "opacity-0",
                          )}
                        />
                        None
                      </CommandItem>
                      {storedProxies.map((proxy) => (
                        <CommandItem
                          key={proxy.id}
                          value={proxy.name}
                          onSelect={() => {
                            handleValueChange(proxy.id);
                            setProxyPopoverOpen(false);
                          }}
                        >
                          <LuCheck
                            className={cn(
                              "mr-2 h-4 w-4",
                              selectionType === "proxy" &&
                                selectedId === proxy.id
                                ? "opacity-100"
                                : "opacity-0",
                            )}
                          />
                          {proxy.name}
                          {proxy.is_cloud_managed ? " (Included)" : ""}
                        </CommandItem>
                      ))}
                    </CommandGroup>
                    {vpnConfigs.length > 0 && (
                      <CommandGroup heading="VPNs">
                        {vpnConfigs.map((vpn) => (
                          <CommandItem
                            key={vpn.id}
                            value={`vpn-${vpn.name}`}
                            onSelect={() => {
                              handleValueChange(`vpn-${vpn.id}`);
                              setProxyPopoverOpen(false);
                            }}
                          >
                            <LuCheck
                              className={cn(
                                "mr-2 h-4 w-4",
                                selectionType === "vpn" && selectedId === vpn.id
                                  ? "opacity-100"
                                  : "opacity-0",
                              )}
                            />
                            <Badge
                              variant="outline"
                              className="text-[10px] px-1 py-0 leading-tight mr-1"
                            >
                              {vpn.vpn_type === "WireGuard" ? "WG" : "OVPN"}
                            </Badge>
                            {vpn.name}
                          </CommandItem>
                        ))}
                      </CommandGroup>
                    )}
                  </CommandList>
                </Command>
              </PopoverContent>
            </Popover>
          </div>

          {error && (
            <div className="p-3 text-sm text-destructive bg-destructive/10 rounded-md">
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

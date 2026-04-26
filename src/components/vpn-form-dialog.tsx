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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { VpnConfig } from "@/types";

interface VpnFormDialogProps {
  isOpen: boolean;
  onClose: () => void;
  editingVpn?: VpnConfig | null;
}

interface WireGuardFormData {
  name: string;
  privateKey: string;
  address: string;
  dns: string;
  mtu: string;
  peerPublicKey: string;
  peerEndpoint: string;
  allowedIps: string;
  persistentKeepalive: string;
  presharedKey: string;
}

const defaultWireGuardForm: WireGuardFormData = {
  name: "",
  privateKey: "",
  address: "",
  dns: "",
  mtu: "",
  peerPublicKey: "",
  peerEndpoint: "",
  allowedIps: "0.0.0.0/0, ::/0",
  persistentKeepalive: "",
  presharedKey: "",
};

function buildWireGuardConfig(form: WireGuardFormData): string {
  const lines: string[] = ["[Interface]"];
  lines.push(`PrivateKey = ${form.privateKey.trim()}`);
  lines.push(`Address = ${form.address.trim()}`);
  if (form.dns.trim()) lines.push(`DNS = ${form.dns.trim()}`);
  if (form.mtu.trim()) lines.push(`MTU = ${form.mtu.trim()}`);
  lines.push("");
  lines.push("[Peer]");
  lines.push(`PublicKey = ${form.peerPublicKey.trim()}`);
  lines.push(`Endpoint = ${form.peerEndpoint.trim()}`);
  lines.push(`AllowedIPs = ${form.allowedIps.trim()}`);
  if (form.persistentKeepalive.trim())
    lines.push(`PersistentKeepalive = ${form.persistentKeepalive.trim()}`);
  if (form.presharedKey.trim())
    lines.push(`PresharedKey = ${form.presharedKey.trim()}`);
  return lines.join("\n");
}

export function VpnFormDialog({
  isOpen,
  onClose,
  editingVpn,
}: VpnFormDialogProps) {
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [wireGuardForm, setWireGuardForm] =
    useState<WireGuardFormData>(defaultWireGuardForm);

  const resetForms = useCallback(() => {
    setWireGuardForm(defaultWireGuardForm);
  }, []);

  useEffect(() => {
    if (isOpen) {
      if (editingVpn) {
        setWireGuardForm({ ...defaultWireGuardForm, name: editingVpn.name });
      } else {
        resetForms();
      }
    }
  }, [isOpen, editingVpn, resetForms]);

  const handleClose = useCallback(() => {
    if (!isSubmitting) {
      onClose();
    }
  }, [isSubmitting, onClose]);

  const handleSubmit = useCallback(async () => {
    if (editingVpn) {
      const name = wireGuardForm.name.trim();

      if (!name) {
        toast.error("VPN name is required");
        return;
      }

      setIsSubmitting(true);
      try {
        await invoke("update_vpn_config", {
          vpnId: editingVpn.id,
          name,
        });
        await emit("vpn-configs-changed");
        toast.success("VPN updated successfully");
        onClose();
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : String(error);
        toast.error(`Failed to update VPN: ${errorMessage}`);
      } finally {
        setIsSubmitting(false);
      }
      return;
    }

    const { name, privateKey, address, peerPublicKey, peerEndpoint } =
      wireGuardForm;

    if (!name.trim()) {
      toast.error("VPN name is required");
      return;
    }
    if (!privateKey.trim()) {
      toast.error("Private key is required");
      return;
    }
    if (!address.trim()) {
      toast.error("Address is required");
      return;
    }
    if (!peerPublicKey.trim()) {
      toast.error("Peer public key is required");
      return;
    }
    if (!peerEndpoint.trim()) {
      toast.error("Peer endpoint is required");
      return;
    }

    setIsSubmitting(true);
    try {
      const configData = buildWireGuardConfig(wireGuardForm);
      await invoke("create_vpn_config_manual", {
        name: name.trim(),
        vpnType: "WireGuard",
        configData,
      });
      await emit("vpn-configs-changed");
      toast.success("WireGuard VPN created successfully");
      onClose();
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(`Failed to create VPN: ${errorMessage}`);
    } finally {
      setIsSubmitting(false);
    }
  }, [editingVpn, wireGuardForm, onClose]);

  const updateWireGuard = useCallback(
    (field: keyof WireGuardFormData, value: string) => {
      setWireGuardForm((prev) => ({ ...prev, [field]: value }));
    },
    [],
  );

  const dialogTitle = editingVpn ? "Edit VPN" : "Create WireGuard VPN";
  const dialogDescription = editingVpn
    ? "Update the name of your VPN configuration."
    : "Enter your WireGuard interface and peer details.";

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{dialogTitle}</DialogTitle>
          <DialogDescription>{dialogDescription}</DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[60vh] pr-4">
          <div className="grid gap-4 py-2">
            <div className="grid gap-2">
              <Label htmlFor="wg-name">Name</Label>
              <Input
                id="wg-name"
                value={wireGuardForm.name}
                onChange={(e) => {
                  updateWireGuard("name", e.target.value);
                }}
                placeholder="e.g. Home WireGuard"
                disabled={isSubmitting}
              />
            </div>

            {!editingVpn && (
              <>
                <div className="grid gap-2">
                  <Label htmlFor="wg-private-key">Private Key</Label>
                  <Input
                    id="wg-private-key"
                    value={wireGuardForm.privateKey}
                    onChange={(e) => {
                      updateWireGuard("privateKey", e.target.value);
                    }}
                    placeholder="Base64-encoded private key"
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="wg-address">Address</Label>
                  <Input
                    id="wg-address"
                    value={wireGuardForm.address}
                    onChange={(e) => {
                      updateWireGuard("address", e.target.value);
                    }}
                    placeholder="e.g. 10.0.0.2/24"
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid grid-cols-2 gap-4">
                  <div className="grid gap-2">
                    <Label htmlFor="wg-dns">DNS (optional)</Label>
                    <Input
                      id="wg-dns"
                      value={wireGuardForm.dns}
                      onChange={(e) => {
                        updateWireGuard("dns", e.target.value);
                      }}
                      placeholder="e.g. 1.1.1.1"
                      disabled={isSubmitting}
                    />
                  </div>

                  <div className="grid gap-2">
                    <Label htmlFor="wg-mtu">MTU (optional)</Label>
                    <Input
                      id="wg-mtu"
                      type="number"
                      value={wireGuardForm.mtu}
                      onChange={(e) => {
                        updateWireGuard("mtu", e.target.value);
                      }}
                      placeholder="e.g. 1420"
                      disabled={isSubmitting}
                    />
                  </div>
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="wg-peer-public-key">Peer Public Key</Label>
                  <Input
                    id="wg-peer-public-key"
                    value={wireGuardForm.peerPublicKey}
                    onChange={(e) => {
                      updateWireGuard("peerPublicKey", e.target.value);
                    }}
                    placeholder="Base64-encoded peer public key"
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="wg-peer-endpoint">Peer Endpoint</Label>
                  <Input
                    id="wg-peer-endpoint"
                    value={wireGuardForm.peerEndpoint}
                    onChange={(e) => {
                      updateWireGuard("peerEndpoint", e.target.value);
                    }}
                    placeholder="e.g. vpn.example.com:51820"
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="wg-allowed-ips">Allowed IPs</Label>
                  <Input
                    id="wg-allowed-ips"
                    value={wireGuardForm.allowedIps}
                    onChange={(e) => {
                      updateWireGuard("allowedIps", e.target.value);
                    }}
                    placeholder="e.g. 0.0.0.0/0, ::/0"
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid grid-cols-2 gap-4">
                  <div className="grid gap-2">
                    <Label htmlFor="wg-keepalive">
                      Persistent Keepalive (optional)
                    </Label>
                    <Input
                      id="wg-keepalive"
                      type="number"
                      value={wireGuardForm.persistentKeepalive}
                      onChange={(e) => {
                        updateWireGuard("persistentKeepalive", e.target.value);
                      }}
                      placeholder="e.g. 25"
                      disabled={isSubmitting}
                    />
                  </div>

                  <div className="grid gap-2">
                    <Label htmlFor="wg-preshared-key">
                      Preshared Key (optional)
                    </Label>
                    <Input
                      id="wg-preshared-key"
                      value={wireGuardForm.presharedKey}
                      onChange={(e) => {
                        updateWireGuard("presharedKey", e.target.value);
                      }}
                      placeholder="Base64-encoded preshared key"
                      disabled={isSubmitting}
                    />
                  </div>
                </div>
              </>
            )}
          </div>
        </ScrollArea>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={handleClose}
            disabled={isSubmitting}
          >
            Cancel
          </RippleButton>
          <LoadingButton isLoading={isSubmitting} onClick={handleSubmit}>
            {editingVpn ? "Update VPN" : "Create VPN"}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

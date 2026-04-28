"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation();
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
        toast.error(t("vpns.form.nameRequired"));
        return;
      }

      setIsSubmitting(true);
      try {
        await invoke("update_vpn_config", {
          vpnId: editingVpn.id,
          name,
        });
        await emit("vpn-configs-changed");
        toast.success(t("vpns.form.updated"));
        onClose();
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : String(error);
        toast.error(t("vpns.form.updateFailed", { error: errorMessage }));
      } finally {
        setIsSubmitting(false);
      }
      return;
    }

    const { name, privateKey, address, peerPublicKey, peerEndpoint } =
      wireGuardForm;

    if (!name.trim()) {
      toast.error(t("vpns.form.nameRequired"));
      return;
    }
    if (!privateKey.trim()) {
      toast.error(t("vpns.form.privateKeyRequired"));
      return;
    }
    if (!address.trim()) {
      toast.error(t("vpns.form.addressRequired"));
      return;
    }
    if (!peerPublicKey.trim()) {
      toast.error(t("vpns.form.peerPublicKeyRequired"));
      return;
    }
    if (!peerEndpoint.trim()) {
      toast.error(t("vpns.form.peerEndpointRequired"));
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
      toast.success(t("vpns.form.created"));
      onClose();
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(t("vpns.form.createFailed", { error: errorMessage }));
    } finally {
      setIsSubmitting(false);
    }
  }, [editingVpn, wireGuardForm, onClose, t]);

  const updateWireGuard = useCallback(
    (field: keyof WireGuardFormData, value: string) => {
      setWireGuardForm((prev) => ({ ...prev, [field]: value }));
    },
    [],
  );

  const dialogTitle = editingVpn
    ? t("vpns.form.titleEdit")
    : t("vpns.form.titleCreate");
  const dialogDescription = editingVpn
    ? t("vpns.form.descEdit")
    : t("vpns.form.descCreate");

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
              <Label htmlFor="wg-name">{t("vpns.form.name")}</Label>
              <Input
                id="wg-name"
                value={wireGuardForm.name}
                onChange={(e) => {
                  updateWireGuard("name", e.target.value);
                }}
                placeholder={t("vpns.form.namePlaceholder")}
                disabled={isSubmitting}
              />
            </div>

            {!editingVpn && (
              <>
                <div className="grid gap-2">
                  <Label htmlFor="wg-private-key">
                    {t("vpns.form.privateKey")}
                  </Label>
                  <Input
                    id="wg-private-key"
                    value={wireGuardForm.privateKey}
                    onChange={(e) => {
                      updateWireGuard("privateKey", e.target.value);
                    }}
                    placeholder={t("vpns.form.privateKeyPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="wg-address">{t("vpns.form.address")}</Label>
                  <Input
                    id="wg-address"
                    value={wireGuardForm.address}
                    onChange={(e) => {
                      updateWireGuard("address", e.target.value);
                    }}
                    placeholder={t("vpns.form.addressPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid grid-cols-2 gap-4">
                  <div className="grid gap-2">
                    <Label htmlFor="wg-dns">{t("vpns.form.dnsOptional")}</Label>
                    <Input
                      id="wg-dns"
                      value={wireGuardForm.dns}
                      onChange={(e) => {
                        updateWireGuard("dns", e.target.value);
                      }}
                      placeholder={t("vpns.form.dnsPlaceholder")}
                      disabled={isSubmitting}
                    />
                  </div>

                  <div className="grid gap-2">
                    <Label htmlFor="wg-mtu">{t("vpns.form.mtuOptional")}</Label>
                    <Input
                      id="wg-mtu"
                      type="number"
                      value={wireGuardForm.mtu}
                      onChange={(e) => {
                        updateWireGuard("mtu", e.target.value);
                      }}
                      placeholder={t("vpns.form.mtuPlaceholder")}
                      disabled={isSubmitting}
                    />
                  </div>
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="wg-peer-public-key">
                    {t("vpns.form.peerPublicKey")}
                  </Label>
                  <Input
                    id="wg-peer-public-key"
                    value={wireGuardForm.peerPublicKey}
                    onChange={(e) => {
                      updateWireGuard("peerPublicKey", e.target.value);
                    }}
                    placeholder={t("vpns.form.peerPublicKeyPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="wg-peer-endpoint">
                    {t("vpns.form.peerEndpoint")}
                  </Label>
                  <Input
                    id="wg-peer-endpoint"
                    value={wireGuardForm.peerEndpoint}
                    onChange={(e) => {
                      updateWireGuard("peerEndpoint", e.target.value);
                    }}
                    placeholder={t("vpns.form.peerEndpointPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="wg-allowed-ips">
                    {t("vpns.form.allowedIps")}
                  </Label>
                  <Input
                    id="wg-allowed-ips"
                    value={wireGuardForm.allowedIps}
                    onChange={(e) => {
                      updateWireGuard("allowedIps", e.target.value);
                    }}
                    placeholder={t("vpns.form.allowedIpsPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid grid-cols-2 gap-4">
                  <div className="grid gap-2">
                    <Label htmlFor="wg-keepalive">
                      {t("vpns.form.keepaliveOptional")}
                    </Label>
                    <Input
                      id="wg-keepalive"
                      type="number"
                      value={wireGuardForm.persistentKeepalive}
                      onChange={(e) => {
                        updateWireGuard("persistentKeepalive", e.target.value);
                      }}
                      placeholder={t("vpns.form.keepalivePlaceholder")}
                      disabled={isSubmitting}
                    />
                  </div>

                  <div className="grid gap-2">
                    <Label htmlFor="wg-preshared-key">
                      {t("vpns.form.presharedKeyOptional")}
                    </Label>
                    <Input
                      id="wg-preshared-key"
                      value={wireGuardForm.presharedKey}
                      onChange={(e) => {
                        updateWireGuard("presharedKey", e.target.value);
                      }}
                      placeholder={t("vpns.form.presharedKeyPlaceholder")}
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
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton isLoading={isSubmitting} onClick={handleSubmit}>
            {editingVpn
              ? t("vpns.form.updateButton")
              : t("vpns.form.createButton")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

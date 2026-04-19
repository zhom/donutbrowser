"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { StoredProxy } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyFormData {
  name: string;
  proxy_type: string;
  host: string;
  port: number;
  username: string;
  password: string;
}

interface ProxyFormDialogProps {
  isOpen: boolean;
  onClose: () => void;
  editingProxy?: StoredProxy | null;
}

const DEFAULT_FORM: ProxyFormData = {
  name: "",
  proxy_type: "http",
  host: "",
  port: 8080,
  username: "",
  password: "",
};

export function ProxyFormDialog({
  isOpen,
  onClose,
  editingProxy,
}: ProxyFormDialogProps) {
  const { t } = useTranslation();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [form, setForm] = useState<ProxyFormData>(DEFAULT_FORM);

  const resetForm = useCallback(() => {
    setForm(DEFAULT_FORM);
  }, []);

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    if (!editingProxy) {
      resetForm();
      return;
    }

    setForm({
      name: editingProxy.name,
      proxy_type: editingProxy.proxy_settings.proxy_type,
      host: editingProxy.proxy_settings.host,
      port: editingProxy.proxy_settings.port,
      username: editingProxy.proxy_settings.username ?? "",
      password: editingProxy.proxy_settings.password ?? "",
    });
  }, [editingProxy, isOpen, resetForm]);

  const handleSubmit = useCallback(async () => {
    if (!form.name.trim()) {
      toast.error(t("proxies.form.nameRequired", "Proxy name is required"));
      return;
    }

    if (!form.host.trim() || !form.port) {
      toast.error(
        t("proxies.form.hostPortRequired", "Host and port are required"),
      );
      return;
    }

    if (
      form.proxy_type === "ss" &&
      (!form.username.trim() || !form.password.trim())
    ) {
      toast.error(
        t(
          "proxies.form.ssCipherRequired",
          "Cipher and password are required for Shadowsocks",
        ),
      );
      return;
    }

    setIsSubmitting(true);
    try {
      const payload = {
        name: form.name.trim(),
        proxySettings: {
          proxy_type: form.proxy_type,
          host: form.host.trim(),
          port: form.port,
          username: form.username.trim() || undefined,
          password: form.password.trim() || undefined,
        },
      };

      if (editingProxy) {
        await invoke("update_stored_proxy", {
          proxyId: editingProxy.id,
          ...payload,
        });
        toast.success(t("toasts.success.proxyUpdated"));
      } else {
        await invoke("create_stored_proxy", payload);
        toast.success(t("toasts.success.proxyCreated"));
      }

      onClose();
    } catch (error) {
      console.error("Failed to save proxy:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(`Failed to save proxy: ${errorMessage}`);
    } finally {
      setIsSubmitting(false);
    }
  }, [editingProxy, form, onClose, t]);

  const handleClose = useCallback(() => {
    if (!isSubmitting) {
      onClose();
    }
  }, [isSubmitting, onClose]);

  const isFormValid =
    form.name.trim() &&
    form.host.trim() &&
    form.port > 0 &&
    form.port <= 65535 &&
    (form.proxy_type !== "ss" ||
      (form.username.trim() && form.password.trim()));

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {editingProxy ? t("proxies.edit") : t("proxies.add")}
          </DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="proxy-name">{t("proxies.form.name")}</Label>
            <Input
              id="proxy-name"
              value={form.name}
              onChange={(e) => {
                setForm({ ...form, name: e.target.value });
              }}
              placeholder={t("proxies.form.namePlaceholder")}
              disabled={isSubmitting}
            />
          </div>

          <div className="grid gap-2">
            <Label>{t("proxies.form.type")}</Label>
            <Select
              value={form.proxy_type}
              onValueChange={(value) => {
                setForm({ ...form, proxy_type: value });
              }}
              disabled={isSubmitting}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select proxy type" />
              </SelectTrigger>
              <SelectContent>
                {["http", "https", "socks4", "socks5", "ss"].map((type) => (
                  <SelectItem key={type} value={type}>
                    {type === "ss" ? "Shadowsocks" : type.toUpperCase()}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="grid gap-2">
              <Label htmlFor="proxy-host">{t("proxies.form.host")}</Label>
              <Input
                id="proxy-host"
                value={form.host}
                onChange={(e) => {
                  setForm({ ...form, host: e.target.value });
                }}
                placeholder={t("proxies.form.hostPlaceholder")}
                disabled={isSubmitting}
              />
            </div>

            <div className="grid gap-2">
              <Label htmlFor="proxy-port">{t("proxies.form.port")}</Label>
              <Input
                id="proxy-port"
                type="number"
                value={form.port}
                onChange={(e) => {
                  setForm({
                    ...form,
                    port: Number.parseInt(e.target.value, 10) || 0,
                  });
                }}
                placeholder={t("proxies.form.portPlaceholder")}
                min="1"
                max="65535"
                disabled={isSubmitting}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="grid gap-2">
              <Label htmlFor="proxy-username">
                {form.proxy_type === "ss"
                  ? t("proxies.form.cipher")
                  : `${t("proxies.form.username")} (${t("proxies.form.usernamePlaceholder")})`}
              </Label>
              <Input
                id="proxy-username"
                value={form.username}
                onChange={(e) => {
                  setForm({ ...form, username: e.target.value });
                }}
                placeholder={
                  form.proxy_type === "ss"
                    ? t("proxies.form.cipherPlaceholder")
                    : t("proxies.form.usernamePlaceholder")
                }
                disabled={isSubmitting}
              />
            </div>

            <div className="grid gap-2">
              <Label htmlFor="proxy-password">
                {form.proxy_type === "ss"
                  ? t("proxies.form.password")
                  : `${t("proxies.form.password")} (${t("proxies.form.passwordPlaceholder")})`}
              </Label>
              <Input
                id="proxy-password"
                type="password"
                value={form.password}
                onChange={(e) => {
                  setForm({ ...form, password: e.target.value });
                }}
                placeholder={t("proxies.form.passwordPlaceholder")}
                disabled={isSubmitting}
              />
            </div>
          </div>
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={handleClose}
            disabled={isSubmitting}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            isLoading={isSubmitting}
            onClick={handleSubmit}
            disabled={!isFormValid}
          >
            {editingProxy ? t("proxies.edit") : t("proxies.add")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

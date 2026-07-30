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
import { Textarea } from "@/components/ui/textarea";
import { translateBackendError } from "@/lib/backend-errors";
import type { StoredProxy } from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyFormData {
  name: string;
  proxy_type: string;
  host: string;
  port: number;
  username: string;
  password: string;
  vless_uri: string;
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
  vless_uri: "",
};

interface VlessEndpoint {
  host: string;
  port: number;
}

function parseVlessEndpoint(uri: string): VlessEndpoint | null {
  try {
    const parsed = new URL(uri.trim());
    const port = Number.parseInt(parsed.port, 10);
    if (
      parsed.protocol !== "vless:" ||
      !parsed.hostname ||
      !Number.isInteger(port) ||
      port < 1 ||
      port > 65535
    ) {
      return null;
    }

    const host =
      parsed.hostname.startsWith("[") && parsed.hostname.endsWith("]")
        ? parsed.hostname.slice(1, -1)
        : parsed.hostname;
    return { host, port };
  } catch {
    return null;
  }
}

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
      vless_uri: editingProxy.proxy_settings.vless_uri ?? "",
    });
  }, [editingProxy, isOpen, resetForm]);

  const handleSubmit = useCallback(async () => {
    if (!form.name.trim()) {
      toast.error(t("proxies.form.nameRequired"));
      return;
    }

    const isVless = form.proxy_type === "vless";
    const vlessEndpoint = isVless ? parseVlessEndpoint(form.vless_uri) : null;

    if (isVless && !form.vless_uri.trim()) {
      toast.error(t("proxies.form.vlessUriRequired"));
      return;
    }

    if (isVless && !vlessEndpoint) {
      toast.error(t("proxies.form.vlessUriInvalid"));
      return;
    }

    if (!isVless && (!form.host.trim() || !form.port)) {
      toast.error(t("proxies.form.hostPortRequired"));
      return;
    }

    if (
      form.proxy_type === "ss" &&
      (!form.username.trim() || !form.password.trim())
    ) {
      toast.error(t("proxies.form.ssCipherRequired"));
      return;
    }

    setIsSubmitting(true);
    try {
      const payload = {
        name: form.name.trim(),
        proxySettings: {
          proxy_type: form.proxy_type,
          host: vlessEndpoint?.host ?? form.host.trim(),
          port: vlessEndpoint?.port ?? form.port,
          username: isVless ? undefined : form.username.trim() || undefined,
          password: isVless ? undefined : form.password.trim() || undefined,
          vless_uri: isVless ? form.vless_uri.trim() : undefined,
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
      toast.error(
        t("proxies.form.saveFailed", {
          error: translateBackendError(t, error),
        }),
      );
    } finally {
      setIsSubmitting(false);
    }
  }, [editingProxy, form, onClose, t]);

  const handleClose = useCallback(() => {
    if (!isSubmitting) {
      onClose();
    }
  }, [isSubmitting, onClose]);

  const isVless = form.proxy_type === "vless";
  const vlessEndpoint = isVless ? parseVlessEndpoint(form.vless_uri) : null;
  const hasInvalidVlessUri =
    isVless && form.vless_uri.trim().length > 0 && !vlessEndpoint;
  const isFormValid =
    form.name.trim() &&
    (isVless
      ? vlessEndpoint !== null
      : form.host.trim() &&
        form.port > 0 &&
        form.port <= 65535 &&
        (form.proxy_type !== "ss" ||
          (form.username.trim() && form.password.trim())));

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {editingProxy ? t("proxies.edit") : t("proxies.add")}
          </DialogTitle>
        </DialogHeader>

        <div className="@container grid gap-4 py-4">
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
            <Label htmlFor="proxy-type">{t("proxies.form.type")}</Label>
            <Select
              value={form.proxy_type}
              onValueChange={(value) => {
                setForm({ ...form, proxy_type: value });
              }}
              disabled={isSubmitting}
            >
              <SelectTrigger id="proxy-type">
                <SelectValue placeholder={t("proxies.form.selectType")} />
              </SelectTrigger>
              <SelectContent>
                {["http", "https", "socks4", "socks5", "ss", "vless"].map(
                  (type) => (
                    <SelectItem key={type} value={type}>
                      {type === "ss"
                        ? "Shadowsocks"
                        : type === "vless"
                          ? t("proxies.form.vlessType")
                          : type.toUpperCase()}
                    </SelectItem>
                  ),
                )}
              </SelectContent>
            </Select>
          </div>

          {isVless ? (
            <div className="grid gap-2">
              <Label htmlFor="proxy-vless-uri">
                {t("proxies.form.vlessUri")}
              </Label>
              <Textarea
                id="proxy-vless-uri"
                value={form.vless_uri}
                onChange={(e) => {
                  setForm({ ...form, vless_uri: e.target.value });
                }}
                placeholder={t("proxies.form.vlessUriPlaceholder")}
                disabled={isSubmitting}
                aria-invalid={hasInvalidVlessUri}
                aria-describedby="proxy-vless-uri-help"
                autoCapitalize="none"
                autoComplete="off"
                spellCheck={false}
                className="min-h-24 resize-y font-mono text-xs leading-relaxed"
              />
              <p
                id="proxy-vless-uri-help"
                className={
                  hasInvalidVlessUri
                    ? "text-xs text-destructive-text"
                    : "text-xs text-muted-foreground"
                }
                role={hasInvalidVlessUri ? "alert" : undefined}
              >
                {hasInvalidVlessUri
                  ? t("proxies.form.vlessUriInvalid")
                  : t("proxies.form.vlessUriHint")}
              </p>
            </div>
          ) : (
            <>
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

              <div className="grid grid-cols-1 gap-4 @sm:grid-cols-2">
                <div className="grid gap-2">
                  <Label htmlFor="proxy-username">
                    {form.proxy_type === "ss"
                      ? t("proxies.form.cipher")
                      : t("proxies.form.username")}
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
                    {t("proxies.form.password")}
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
            </>
          )}
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

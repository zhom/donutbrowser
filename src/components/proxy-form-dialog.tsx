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
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { ProxySettings, StoredProxy } from "@/types";
import { RippleButton } from "./ui/ripple";

interface RegularFormData {
  name: string;
  proxy_type: string;
  host: string;
  port: number;
  username: string;
  password: string;
}

interface DynamicFormData {
  name: string;
  url: string;
  format: string;
}

type ProxyMode = "regular" | "dynamic";

interface ProxyFormDialogProps {
  isOpen: boolean;
  onClose: () => void;
  editingProxy?: StoredProxy | null;
}

export function ProxyFormDialog({
  isOpen,
  onClose,
  editingProxy,
}: ProxyFormDialogProps) {
  const { t } = useTranslation();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isTesting, setIsTesting] = useState(false);
  const [mode, setMode] = useState<ProxyMode>("regular");
  const [regularForm, setRegularForm] = useState<RegularFormData>({
    name: "",
    proxy_type: "http",
    host: "",
    port: 8080,
    username: "",
    password: "",
  });
  const [dynamicForm, setDynamicForm] = useState<DynamicFormData>({
    name: "",
    url: "",
    format: "json",
  });

  const resetForm = useCallback(() => {
    setRegularForm({
      name: "",
      proxy_type: "http",
      host: "",
      port: 8080,
      username: "",
      password: "",
    });
    setDynamicForm({
      name: "",
      url: "",
      format: "json",
    });
    setMode("regular");
  }, []);

  useEffect(() => {
    if (isOpen) {
      if (editingProxy) {
        if (editingProxy.dynamic_proxy_url) {
          setMode("dynamic");
          setDynamicForm({
            name: editingProxy.name,
            url: editingProxy.dynamic_proxy_url,
            format: editingProxy.dynamic_proxy_format || "json",
          });
        } else {
          setMode("regular");
          setRegularForm({
            name: editingProxy.name,
            proxy_type: editingProxy.proxy_settings.proxy_type,
            host: editingProxy.proxy_settings.host,
            port: editingProxy.proxy_settings.port,
            username: editingProxy.proxy_settings.username || "",
            password: editingProxy.proxy_settings.password || "",
          });
        }
      } else {
        resetForm();
      }
    }
  }, [isOpen, editingProxy, resetForm]);

  const handleTestDynamic = useCallback(async () => {
    if (!dynamicForm.url.trim()) {
      toast.error(t("proxies.dynamic.urlRequired"));
      return;
    }
    setIsTesting(true);
    try {
      const settings = await invoke<ProxySettings>("fetch_dynamic_proxy", {
        url: dynamicForm.url.trim(),
        format: dynamicForm.format,
      });
      toast.success(
        t("proxies.dynamic.testSuccess", {
          host: settings.host,
          port: settings.port,
        }),
      );
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(t("proxies.dynamic.testFailed", { error: errorMessage }));
    } finally {
      setIsTesting(false);
    }
  }, [dynamicForm, t]);

  const handleSubmit = useCallback(async () => {
    if (mode === "regular") {
      if (!regularForm.name.trim()) {
        toast.error(t("proxies.form.nameRequired", "Proxy name is required"));
        return;
      }
      if (!regularForm.host.trim() || !regularForm.port) {
        toast.error(
          t("proxies.form.hostPortRequired", "Host and port are required"),
        );
        return;
      }
    } else {
      if (!dynamicForm.name.trim()) {
        toast.error(t("proxies.form.nameRequired", "Proxy name is required"));
        return;
      }
      if (!dynamicForm.url.trim()) {
        toast.error(t("proxies.dynamic.urlRequired"));
        return;
      }
    }

    setIsSubmitting(true);
    try {
      if (editingProxy) {
        if (mode === "dynamic") {
          await invoke("update_stored_proxy", {
            proxyId: editingProxy.id,
            name: dynamicForm.name.trim(),
            dynamicProxyUrl: dynamicForm.url.trim(),
            dynamicProxyFormat: dynamicForm.format,
          });
        } else {
          await invoke("update_stored_proxy", {
            proxyId: editingProxy.id,
            name: regularForm.name.trim(),
            proxySettings: {
              proxy_type: regularForm.proxy_type,
              host: regularForm.host.trim(),
              port: regularForm.port,
              username: regularForm.username.trim() || undefined,
              password: regularForm.password.trim() || undefined,
            },
          });
        }
        toast.success(t("toasts.success.proxyUpdated"));
      } else {
        if (mode === "dynamic") {
          await invoke("create_stored_proxy", {
            name: dynamicForm.name.trim(),
            dynamicProxyUrl: dynamicForm.url.trim(),
            dynamicProxyFormat: dynamicForm.format,
          });
        } else {
          await invoke("create_stored_proxy", {
            name: regularForm.name.trim(),
            proxySettings: {
              proxy_type: regularForm.proxy_type,
              host: regularForm.host.trim(),
              port: regularForm.port,
              username: regularForm.username.trim() || undefined,
              password: regularForm.password.trim() || undefined,
            },
          });
        }
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
  }, [mode, regularForm, dynamicForm, editingProxy, onClose, t]);

  const handleClose = useCallback(() => {
    if (!isSubmitting) {
      onClose();
    }
  }, [isSubmitting, onClose]);

  const isRegularValid =
    regularForm.name.trim() &&
    regularForm.host.trim() &&
    regularForm.port > 0 &&
    regularForm.port <= 65535;

  const isDynamicValid = dynamicForm.name.trim() && dynamicForm.url.trim();

  const isFormValid = mode === "regular" ? isRegularValid : isDynamicValid;

  const isEditingDynamic = editingProxy?.dynamic_proxy_url != null;

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {editingProxy ? t("proxies.edit") : t("proxies.add")}
          </DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          {!editingProxy && (
            <Tabs value={mode} onValueChange={(v) => setMode(v as ProxyMode)}>
              <TabsList className="w-full">
                <TabsTrigger value="regular" className="flex-1">
                  {t("proxies.tabs.regular")}
                </TabsTrigger>
                <TabsTrigger value="dynamic" className="flex-1">
                  {t("proxies.tabs.dynamic")}
                </TabsTrigger>
              </TabsList>
            </Tabs>
          )}

          {editingProxy && isEditingDynamic && (
            <p className="text-xs text-muted-foreground">
              {t("proxies.dynamic.description")}
            </p>
          )}

          {mode === "regular" ? (
            <>
              <div className="grid gap-2">
                <Label htmlFor="proxy-name">{t("proxies.form.name")}</Label>
                <Input
                  id="proxy-name"
                  value={regularForm.name}
                  onChange={(e) =>
                    setRegularForm({ ...regularForm, name: e.target.value })
                  }
                  placeholder="e.g. Office Proxy, Home VPN, etc."
                  disabled={isSubmitting}
                />
              </div>

              <div className="grid gap-2">
                <Label>{t("proxies.form.type")}</Label>
                <Select
                  value={regularForm.proxy_type}
                  onValueChange={(value) =>
                    setRegularForm({ ...regularForm, proxy_type: value })
                  }
                  disabled={isSubmitting}
                >
                  <SelectTrigger>
                    <SelectValue placeholder="Select proxy type" />
                  </SelectTrigger>
                  <SelectContent>
                    {["http", "https", "socks4", "socks5"].map((type) => (
                      <SelectItem key={type} value={type}>
                        {type.toUpperCase()}
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
                    value={regularForm.host}
                    onChange={(e) =>
                      setRegularForm({ ...regularForm, host: e.target.value })
                    }
                    placeholder={t("proxies.form.hostPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="proxy-port">{t("proxies.form.port")}</Label>
                  <Input
                    id="proxy-port"
                    type="number"
                    value={regularForm.port}
                    onChange={(e) =>
                      setRegularForm({
                        ...regularForm,
                        port: parseInt(e.target.value, 10) || 0,
                      })
                    }
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
                    {t("proxies.form.username")} (
                    {t("proxies.form.usernamePlaceholder")})
                  </Label>
                  <Input
                    id="proxy-username"
                    value={regularForm.username}
                    onChange={(e) =>
                      setRegularForm({
                        ...regularForm,
                        username: e.target.value,
                      })
                    }
                    placeholder={t("proxies.form.usernamePlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>

                <div className="grid gap-2">
                  <Label htmlFor="proxy-password">
                    {t("proxies.form.password")} (
                    {t("proxies.form.passwordPlaceholder")})
                  </Label>
                  <Input
                    id="proxy-password"
                    type="password"
                    value={regularForm.password}
                    onChange={(e) =>
                      setRegularForm({
                        ...regularForm,
                        password: e.target.value,
                      })
                    }
                    placeholder={t("proxies.form.passwordPlaceholder")}
                    disabled={isSubmitting}
                  />
                </div>
              </div>
            </>
          ) : (
            <>
              <div className="grid gap-2">
                <Label htmlFor="dynamic-name">{t("proxies.form.name")}</Label>
                <Input
                  id="dynamic-name"
                  value={dynamicForm.name}
                  onChange={(e) =>
                    setDynamicForm({ ...dynamicForm, name: e.target.value })
                  }
                  placeholder="e.g. My Tunnel"
                  disabled={isSubmitting}
                />
              </div>

              <div className="grid gap-2">
                <Label htmlFor="dynamic-url">{t("proxies.dynamic.url")}</Label>
                <Input
                  id="dynamic-url"
                  value={dynamicForm.url}
                  onChange={(e) =>
                    setDynamicForm({ ...dynamicForm, url: e.target.value })
                  }
                  placeholder={t("proxies.dynamic.urlPlaceholder")}
                  disabled={isSubmitting}
                />
              </div>

              <div className="grid gap-2">
                <Label>{t("proxies.dynamic.format")}</Label>
                <Select
                  value={dynamicForm.format}
                  onValueChange={(value) =>
                    setDynamicForm({ ...dynamicForm, format: value })
                  }
                  disabled={isSubmitting}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="json">
                      {t("proxies.dynamic.formatJson")}
                    </SelectItem>
                    <SelectItem value="text">
                      {t("proxies.dynamic.formatText")}
                    </SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {dynamicForm.format === "json"
                    ? t("proxies.dynamic.formatJsonHint")
                    : t("proxies.dynamic.formatTextHint")}
                </p>
              </div>

              <RippleButton
                variant="outline"
                size="sm"
                onClick={handleTestDynamic}
                disabled={isSubmitting || isTesting || !dynamicForm.url.trim()}
              >
                {isTesting
                  ? t("proxies.dynamic.testing")
                  : t("proxies.dynamic.testUrl")}
              </RippleButton>
            </>
          )}
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={handleClose}
            disabled={isSubmitting}
          >
            {t("common.cancel", "Cancel")}
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

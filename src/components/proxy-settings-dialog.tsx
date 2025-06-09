"use client";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useEffect, useState } from "react";

interface ProxySettings {
  enabled: boolean;
  proxy_type: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
}

interface ProxySettingsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (proxySettings: ProxySettings) => void;
  initialSettings?: ProxySettings;
  browserType?: string;
}

export function ProxySettingsDialog({
  isOpen,
  onClose,
  onSave,
  initialSettings,
  browserType,
}: ProxySettingsDialogProps) {
  const [settings, setSettings] = useState<ProxySettings>({
    enabled: initialSettings?.enabled ?? false,
    proxy_type: initialSettings?.proxy_type ?? "http",
    host: initialSettings?.host ?? "",
    port: initialSettings?.port ?? 8080,
    username: initialSettings?.username ?? "",
    password: initialSettings?.password ?? "",
  });

  const [initialSettingsState, setInitialSettingsState] =
    useState<ProxySettings>({
      enabled: false,
      proxy_type: "http",
      host: "",
      port: 8080,
      username: "",
      password: "",
    });

  useEffect(() => {
    if (isOpen && initialSettings) {
      const newSettings = {
        enabled: initialSettings.enabled,
        proxy_type: initialSettings.proxy_type,
        host: initialSettings.host,
        port: initialSettings.port,
        username: initialSettings.username ?? "",
        password: initialSettings.password ?? "",
      };
      setSettings(newSettings);
      setInitialSettingsState(newSettings);
    } else if (isOpen) {
      const defaultSettings = {
        enabled: false,
        proxy_type: "http",
        host: "",
        port: 80,
        username: "",
        password: "",
      };
      setSettings(defaultSettings);
      setInitialSettingsState(defaultSettings);
    }
  }, [isOpen, initialSettings]);

  const handleSubmit = () => {
    onSave(settings);
  };

  // Check if settings have changed
  const hasChanged = () => {
    return (
      settings.enabled !== initialSettingsState.enabled ||
      settings.proxy_type !== initialSettingsState.proxy_type ||
      settings.host !== initialSettingsState.host ||
      settings.port !== initialSettingsState.port ||
      settings.username !== initialSettingsState.username ||
      settings.password !== initialSettingsState.password
    );
  };

  // Helper to determine if proxy should be disabled for the selected browser
  const isProxyDisabled = browserType === "tor-browser";

  // Update proxy enabled state when browser is tor-browser
  useEffect(() => {
    if (browserType === "tor-browser" && settings.enabled) {
      setSettings((prev) => ({ ...prev, enabled: false }));
    }
  }, [browserType, settings.enabled]);

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Proxy Settings</DialogTitle>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          <div className="flex items-center space-x-2">
            {isProxyDisabled ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <div className="flex items-center space-x-2 opacity-50">
                    <Checkbox
                      id="proxy-enabled"
                      checked={false}
                      disabled={true}
                    />
                    <Label htmlFor="proxy-enabled" className="text-gray-500">
                      Enable Proxy
                    </Label>
                  </div>
                </TooltipTrigger>
                <TooltipContent>
                  <p>
                    Tor Browser has its own built-in proxy system and
                    doesn&apos;t support additional proxy configuration
                  </p>
                </TooltipContent>
              </Tooltip>
            ) : (
              <>
                <Checkbox
                  id="proxy-enabled"
                  checked={settings.enabled}
                  onCheckedChange={(checked) => {
                    setSettings({ ...settings, enabled: checked as boolean });
                  }}
                />
                <Label htmlFor="proxy-enabled">Enable Proxy</Label>
              </>
            )}
          </div>

          {settings.enabled && !isProxyDisabled && (
            <>
              <div className="grid gap-2">
                <Label>Proxy Type</Label>
                <Select
                  value={settings.proxy_type}
                  onValueChange={(value) => {
                    setSettings({
                      ...settings,
                      proxy_type: value,
                    });
                  }}
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

              <div className="grid gap-2">
                <Label htmlFor="host">Host</Label>
                <Input
                  id="host"
                  value={settings.host}
                  onChange={(e) => {
                    setSettings({ ...settings, host: e.target.value });
                  }}
                  placeholder="e.g. 127.0.0.1"
                />
              </div>

              <div className="grid gap-2">
                <Label htmlFor="port">Port</Label>
                <Input
                  id="port"
                  type="number"
                  value={settings.port}
                  onChange={(e) => {
                    setSettings({
                      ...settings,
                      port: Number.parseInt(e.target.value, 10) || 0,
                    });
                  }}
                  placeholder="e.g. 8080"
                  min="1"
                  max="65535"
                />
              </div>

              <div className="grid gap-2">
                <Label htmlFor="username">Username (optional)</Label>
                <Input
                  id="username"
                  value={settings.username}
                  onChange={(e) => {
                    setSettings({ ...settings, username: e.target.value });
                  }}
                  placeholder="Proxy username"
                />
              </div>

              <div className="grid gap-2">
                <Label htmlFor="password">Password (optional)</Label>
                <Input
                  id="password"
                  type="password"
                  value={settings.password}
                  onChange={(e) => {
                    setSettings({ ...settings, password: e.target.value });
                  }}
                  placeholder="Proxy password"
                />
              </div>
            </>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={handleSubmit}
            disabled={
              !hasChanged() ||
              (!isProxyDisabled &&
                settings.enabled &&
                (!settings.host || !settings.port))
            }
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

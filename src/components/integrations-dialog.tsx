"use client";

import { invoke } from "@tauri-apps/api/core";
import { Eye, EyeOff } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuPlug } from "react-icons/lu";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useWayfernTerms } from "@/hooks/use-wayfern-terms";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { CopyToClipboard } from "./ui/copy-to-clipboard";

interface AppSettings {
  api_enabled: boolean;
  api_port: number;
  api_token?: string;
  mcp_enabled: boolean;
  mcp_port?: number;
  mcp_token?: string;
}

interface McpConfig {
  port: number;
  token: string;
}

interface IntegrationsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
}

export function IntegrationsDialog({
  isOpen,
  onClose,
  subPage,
}: IntegrationsDialogProps) {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<AppSettings>({
    api_enabled: false,
    api_port: 10108,
    api_token: undefined,
    mcp_enabled: false,
    mcp_port: undefined,
    mcp_token: undefined,
  });
  const [apiServerPort, setApiServerPort] = useState<number | null>(null);
  const [mcpConfig, setMcpConfig] = useState<McpConfig | null>(null);
  const [, setMcpRunning] = useState(false);
  const [showApiToken, setShowApiToken] = useState(false);
  const [showMcpToken, setShowMcpToken] = useState(false);
  const [isApiStarting, setIsApiStarting] = useState(false);
  const [isMcpStarting, setIsMcpStarting] = useState(false);
  const [mcpInClaudeDesktop, setMcpInClaudeDesktop] = useState(false);
  const [mcpInClaudeCode, setMcpInClaudeCode] = useState(false);

  const { termsAccepted } = useWayfernTerms();

  const loadSettings = useCallback(async () => {
    try {
      const loaded = await invoke<AppSettings>("get_app_settings");
      setSettings(loaded);
    } catch (e) {
      console.error("Failed to load settings:", e);
    }
  }, []);

  const loadMcpConfig = useCallback(async () => {
    try {
      const config = await invoke<McpConfig | null>("get_mcp_config");
      setMcpConfig(config);
    } catch (e) {
      console.error("Failed to get MCP config:", e);
    }
  }, []);

  const loadMcpServerStatus = useCallback(async () => {
    try {
      const isRunning = await invoke<boolean>("get_mcp_server_status");
      setMcpRunning(isRunning);
    } catch (e) {
      console.error("Failed to get MCP server status:", e);
    }
  }, []);

  const loadApiServerStatus = useCallback(async () => {
    try {
      const port = await invoke<number | null>("get_api_server_status");
      setApiServerPort(port);
    } catch (e) {
      console.error("Failed to get API server status:", e);
    }
  }, []);

  const loadClaudeDesktopStatus = useCallback(async () => {
    try {
      const exists = await invoke<boolean>("is_mcp_in_claude_desktop");
      setMcpInClaudeDesktop(exists);
    } catch {
      // Not critical
    }
  }, []);

  const loadClaudeCodeStatus = useCallback(async () => {
    try {
      const exists = await invoke<boolean>("is_mcp_in_claude_code");
      setMcpInClaudeCode(exists);
    } catch {
      // Claude CLI may not be installed
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadSettings();
      void loadApiServerStatus();
      void loadMcpConfig();
      void loadMcpServerStatus();
      void loadClaudeDesktopStatus();
      void loadClaudeCodeStatus();
    }
  }, [
    isOpen,
    loadSettings,
    loadApiServerStatus,
    loadMcpConfig,
    loadMcpServerStatus,
    loadClaudeDesktopStatus,
    loadClaudeCodeStatus,
  ]);

  const handleApiToggle = async (enabled: boolean) => {
    setIsApiStarting(true);
    try {
      if (enabled) {
        const port = await invoke<number>("start_api_server", {
          port: settings.api_port,
        });
        setApiServerPort(port);
        const next = await invoke<AppSettings>("save_app_settings", {
          settings: { ...settings, api_enabled: true },
        });
        setSettings(next);
        showSuccessToast(t("integrations.apiStarted", { port }));
      } else {
        await invoke("stop_api_server");
        setApiServerPort(null);
        const next = await invoke<AppSettings>("save_app_settings", {
          settings: { ...settings, api_enabled: false, api_token: null },
        });
        setSettings(next);
        showSuccessToast(t("integrations.apiStopped"));
      }
    } catch (e) {
      console.error("Failed to toggle API:", e);
      showErrorToast(t("integrations.apiToggleFailed"), {
        description:
          e instanceof Error ? e.message : t("integrations.apiUnknownError"),
      });
    } finally {
      setIsApiStarting(false);
    }
  };

  const handleMcpToggle = async (enabled: boolean) => {
    setIsMcpStarting(true);
    try {
      if (enabled) {
        const port = await invoke<number>("start_mcp_server");
        const next = await invoke<AppSettings>("save_app_settings", {
          settings: { ...settings, mcp_enabled: true, mcp_port: port },
        });
        setSettings(next);
        void loadMcpConfig();
        showSuccessToast(t("integrations.mcpStarted", { port }));
      } else {
        await invoke("stop_mcp_server");
        const next = await invoke<AppSettings>("save_app_settings", {
          settings: { ...settings, mcp_enabled: false },
        });
        setSettings(next);
        setMcpConfig(null);
        showSuccessToast(t("integrations.mcpStopped"));
      }
    } catch (e) {
      console.error("Failed to toggle MCP server:", e);
      showErrorToast(t("integrations.mcpToggleFailed"), {
        description:
          e instanceof Error ? e.message : t("integrations.apiUnknownError"),
      });
    } finally {
      setIsMcpStarting(false);
    }
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      subPage={subPage}
    >
      <DialogContent className="max-w-xl max-h-[80vh] my-8 flex flex-col">
        {!subPage && (
          <DialogHeader className="shrink-0">
            <DialogTitle>{t("integrations.title")}</DialogTitle>
          </DialogHeader>
        )}

        <div className="overflow-y-auto flex-1 min-h-0">
          <AnimatedTabs defaultValue="api">
            <AnimatedTabsList>
              <AnimatedTabsTrigger value="api">
                {t("integrations.tabApi")}
              </AnimatedTabsTrigger>
              <AnimatedTabsTrigger value="mcp">
                {t("integrations.tabMcp")}
              </AnimatedTabsTrigger>
            </AnimatedTabsList>

            <AnimatedTabsContent
              value="api"
              className="mt-4 flex flex-col gap-4"
            >
              <div className="rounded-md border bg-card p-4 flex flex-col gap-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="flex items-start gap-3">
                    <LuPlug className="size-5 mt-0.5 text-muted-foreground" />
                    <div className="flex flex-col gap-1">
                      <Label className="text-sm font-medium">
                        {t("integrations.apiEnableLabel")}
                      </Label>
                      <p className="text-xs text-muted-foreground">
                        {t("integrations.apiEnableDescription")}
                      </p>
                    </div>
                  </div>
                  <AnimatedSwitch
                    checked={apiServerPort !== null}
                    disabled={isApiStarting}
                    onCheckedChange={(checked) => void handleApiToggle(checked)}
                  />
                </div>

                {apiServerPort && (
                  <div className="flex items-center gap-2 text-xs">
                    <span className="size-1.5 rounded-full bg-success" />
                    <span className="text-muted-foreground">
                      {t("integrations.apiRunningOn")}
                    </span>
                    <code className="rounded bg-muted px-2 py-1 font-mono text-[11px]">
                      http://127.0.0.1:{apiServerPort}
                    </code>
                  </div>
                )}
              </div>

              {settings.api_enabled && (
                <>
                  <div className="grid grid-cols-2 gap-4">
                    <div className="rounded-md border bg-card p-4 flex flex-col gap-2">
                      <Label className="text-[10px] uppercase tracking-wide text-muted-foreground">
                        {t("integrations.apiPortLabel")}
                      </Label>
                      <div className="flex items-center gap-2">
                        <Input
                          type="number"
                          value={settings.api_port}
                          onChange={(e) => {
                            const val = Number.parseInt(e.target.value, 10);
                            if (!Number.isNaN(val)) {
                              setSettings({ ...settings, api_port: val });
                            }
                          }}
                          className="w-24 font-mono"
                          min={1}
                          max={65535}
                        />
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={
                            isApiStarting || apiServerPort === settings.api_port
                          }
                          onClick={async () => {
                            const port = settings.api_port;
                            if (port < 1 || port > 65535) {
                              showErrorToast(t("integrations.apiInvalidPort"), {
                                description: t(
                                  "integrations.apiInvalidPortDescription",
                                ),
                              });
                              return;
                            }
                            setIsApiStarting(true);
                            try {
                              await invoke("stop_api_server");
                              const next = await invoke<AppSettings>(
                                "save_app_settings",
                                { settings },
                              );
                              setSettings(next);
                              const actualPort = await invoke<number>(
                                "start_api_server",
                                { port },
                              );
                              setApiServerPort(actualPort);
                              if (actualPort !== port) {
                                showErrorToast(
                                  t("integrations.apiPortInUse", { port }),
                                  {
                                    description: t(
                                      "integrations.apiFallbackPort",
                                      { port: actualPort },
                                    ),
                                  },
                                );
                              } else {
                                showSuccessToast(
                                  t("integrations.apiRunning", {
                                    port: actualPort,
                                  }),
                                );
                              }
                            } catch (e) {
                              showErrorToast(t("integrations.apiStartFailed"), {
                                description:
                                  e instanceof Error
                                    ? e.message
                                    : t("integrations.apiUnknownError"),
                              });
                            } finally {
                              setIsApiStarting(false);
                            }
                          }}
                        >
                          {t("common.buttons.save")}
                        </Button>
                      </div>
                    </div>

                    <div className="rounded-md border bg-card p-4 flex flex-col gap-2">
                      <div className="flex items-center justify-between">
                        <Label className="text-[10px] uppercase tracking-wide text-muted-foreground">
                          {t("integrations.apiTokenLabel")}
                        </Label>
                      </div>
                      <div className="flex items-center gap-2">
                        <div className="relative flex-1">
                          <Input
                            type={showApiToken ? "text" : "password"}
                            value={settings.api_token ?? ""}
                            readOnly
                            className="font-mono pr-10"
                          />
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
                            onClick={() => {
                              setShowApiToken(!showApiToken);
                            }}
                          >
                            {showApiToken ? (
                              <EyeOff className="size-4" />
                            ) : (
                              <Eye className="size-4" />
                            )}
                          </Button>
                        </div>
                        <CopyToClipboard
                          text={settings.api_token ?? ""}
                          successMessage={t("integrations.tokenCopied")}
                        />
                      </div>
                    </div>
                  </div>

                  <div className="rounded-md border bg-card p-4 flex flex-col gap-2">
                    <div className="flex items-center justify-between">
                      <Label className="text-[10px] uppercase tracking-wide text-muted-foreground">
                        {t("integrations.apiExampleRequest")}
                      </Label>
                      <CopyToClipboard
                        text={`curl -H "Authorization: Bearer ${settings.api_token ?? "${TOKEN}"}" \\\n     http://127.0.0.1:${apiServerPort ?? settings.api_port}/v1/profiles`}
                        successMessage={t("common.buttons.copied")}
                      />
                    </div>
                    <pre className="font-mono text-[11px] whitespace-pre overflow-x-auto bg-background rounded p-3">
                      {`curl -H "Authorization: Bearer \${TOKEN}" \\
     http://127.0.0.1:${apiServerPort ?? settings.api_port}/v1/profiles`}
                    </pre>
                  </div>
                </>
              )}
            </AnimatedTabsContent>

            <AnimatedTabsContent value="mcp" className="mt-4">
              <div className="flex items-start gap-x-3">
                <Checkbox
                  id="mcp-enabled"
                  checked={settings.mcp_enabled && mcpConfig !== null}
                  disabled={!termsAccepted || isMcpStarting}
                  onCheckedChange={(checked) => void handleMcpToggle(!!checked)}
                  className="mt-0.5"
                />
                <div className="grid gap-1 leading-none">
                  <Label
                    htmlFor="mcp-enabled"
                    className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                  >
                    {t("integrations.mcpEnableLabel")}
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    {t("integrations.mcpEnableDescription")}
                    {!termsAccepted && (
                      <span className="ml-1 text-warning">
                        {t("integrations.mcpAcceptTermsFirst")}
                      </span>
                    )}
                  </p>
                </div>
              </div>

              {mcpConfig && (
                <div className="mt-6 flex flex-col gap-5 pt-5 border-t">
                  <div className="flex flex-col gap-1.5">
                    <Label className="text-[10px] uppercase tracking-wide text-muted-foreground">
                      {t("integrations.mcp.url")}
                    </Label>
                    <div className="flex items-center gap-x-2">
                      <div className="relative flex-1">
                        <Input
                          type={showMcpToken ? "text" : "password"}
                          value={`http://127.0.0.1:${mcpConfig.port}/mcp/${mcpConfig.token}`}
                          readOnly
                          className="font-mono text-xs pr-10"
                        />
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
                          onClick={() => {
                            setShowMcpToken(!showMcpToken);
                          }}
                        >
                          {showMcpToken ? (
                            <EyeOff className="size-4" />
                          ) : (
                            <Eye className="size-4" />
                          )}
                        </Button>
                      </div>
                      <CopyToClipboard
                        text={`http://127.0.0.1:${mcpConfig.port}/mcp/${mcpConfig.token}`}
                        successMessage={t("integrations.mcp.urlCopied")}
                      />
                    </div>
                  </div>

                  <div className="flex flex-col gap-2">
                    <Label className="text-[10px] uppercase tracking-wide text-muted-foreground">
                      {t("integrations.mcp.clientsLabel")}
                    </Label>
                    <div className="flex items-center justify-between gap-x-3 py-1">
                      <span className="text-sm">
                        {t("integrations.mcp.claudeDesktopTitle")}
                      </span>
                      {mcpInClaudeDesktop ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={async () => {
                            try {
                              await invoke("remove_mcp_from_claude_desktop");
                              setMcpInClaudeDesktop(false);
                              showSuccessToast(
                                t("integrations.mcp.removedFromClaudeDesktop"),
                              );
                            } catch (e) {
                              showErrorToast(String(e));
                            }
                          }}
                        >
                          {t("integrations.mcp.removeFromClaudeDesktop")}
                        </Button>
                      ) : (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={async () => {
                            try {
                              await invoke("add_mcp_to_claude_desktop");
                              setMcpInClaudeDesktop(true);
                              showSuccessToast(
                                t("integrations.mcp.addedToClaudeDesktop"),
                              );
                            } catch (e) {
                              showErrorToast(String(e));
                            }
                          }}
                        >
                          {t("integrations.mcp.addToClaudeDesktop")}
                        </Button>
                      )}
                    </div>
                    <div className="flex items-center justify-between gap-x-3 py-1">
                      <span className="text-sm">
                        {t("integrations.mcp.claudeCodeTitle")}
                      </span>
                      {mcpInClaudeCode ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={async () => {
                            try {
                              await invoke("remove_mcp_from_claude_code");
                              setMcpInClaudeCode(false);
                              showSuccessToast(
                                t("integrations.mcp.removedFromClaudeCode"),
                              );
                            } catch (e) {
                              showErrorToast(String(e));
                            }
                          }}
                        >
                          {t("integrations.mcp.removeFromClaudeCode")}
                        </Button>
                      ) : (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={async () => {
                            try {
                              await invoke("add_mcp_to_claude_code");
                              setMcpInClaudeCode(true);
                              showSuccessToast(
                                t("integrations.mcp.addedToClaudeCode"),
                              );
                            } catch (e) {
                              showErrorToast(String(e));
                            }
                          }}
                        >
                          {t("integrations.mcp.addToClaudeCode")}
                        </Button>
                      )}
                    </div>
                  </div>
                </div>
              )}
            </AnimatedTabsContent>
          </AnimatedTabs>
        </div>
      </DialogContent>
    </Dialog>
  );
}

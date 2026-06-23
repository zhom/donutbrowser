"use client";

import { invoke } from "@tauri-apps/api/core";
import { Eye, EyeOff } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuAppWindow,
  LuCheck,
  LuCodeXml,
  LuPlug,
  LuTerminal,
  LuTrash2,
  LuZap,
} from "react-icons/lu";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useWayfernTerms } from "@/hooks/use-wayfern-terms";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
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

type AgentCategory = "desktop-app" | "cli" | "editor" | "editor-ext";

interface McpAgentInfo {
  id: string;
  display_name: string;
  category: AgentCategory;
  connected: boolean;
  detected: boolean;
}

interface IntegrationsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  /** Which tab is displayed when the dialog mounts; defaults to "api". */
  initialTab?: "api" | "mcp";
}

function AgentIcon({ category }: { category: AgentCategory }) {
  const className = "size-4 text-muted-foreground";
  switch (category) {
    case "desktop-app":
      return <LuAppWindow className={className} />;
    case "editor":
      return <LuCodeXml className={className} />;
    case "editor-ext":
      return <LuPlug className={className} />;
    case "cli":
      return <LuTerminal className={className} />;
  }
}

function categoryLabel(
  t: (k: string) => string,
  category: AgentCategory,
): string {
  switch (category) {
    case "desktop-app":
      return t("integrations.mcp.category.desktopApp");
    case "editor":
      return t("integrations.mcp.category.editor");
    case "editor-ext":
      return t("integrations.mcp.category.editorExt");
    case "cli":
      return t("integrations.mcp.category.cli");
  }
}

export function IntegrationsDialog({
  isOpen,
  onClose,
  subPage,
  initialTab = "api",
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
  const [showMcpUrl, setShowMcpUrl] = useState(false);
  const [isApiStarting, setIsApiStarting] = useState(false);
  const [isMcpStarting, setIsMcpStarting] = useState(false);
  const [agents, setAgents] = useState<McpAgentInfo[]>([]);
  const [busyAgentIds, setBusyAgentIds] = useState<Set<string>>(new Set());
  const [apiPortDraft, setApiPortDraft] = useState<string>("10108");

  const { termsAccepted } = useWayfernTerms();

  const loadSettings = useCallback(async () => {
    try {
      const loaded = await invoke<AppSettings>("get_app_settings");
      setSettings(loaded);
      setApiPortDraft(String(loaded.api_port ?? ""));
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

  const loadAgents = useCallback(async () => {
    try {
      const list = await invoke<McpAgentInfo[]>("list_mcp_agents");
      setAgents(list);
    } catch (e) {
      console.error("Failed to list MCP agents:", e);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadSettings();
      void loadApiServerStatus();
      void loadMcpConfig();
      void loadMcpServerStatus();
      void loadAgents();
    }
  }, [
    isOpen,
    loadSettings,
    loadApiServerStatus,
    loadMcpConfig,
    loadMcpServerStatus,
    loadAgents,
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
        void loadAgents();
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

  const markAgentBusy = (id: string, busy: boolean) => {
    setBusyAgentIds((prev) => {
      const next = new Set(prev);
      if (busy) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const handleAddAgent = async (agent: McpAgentInfo) => {
    markAgentBusy(agent.id, true);
    try {
      await invoke("add_mcp_to_agent", { agentId: agent.id });
      showSuccessToast(
        t("integrations.mcp.addedToClient", { name: agent.display_name }),
      );
      void loadAgents();
    } catch (e) {
      showErrorToast(translateBackendError(t, e), {
        description: agent.display_name,
      });
    } finally {
      markAgentBusy(agent.id, false);
    }
  };

  const handleRemoveAgent = async (agent: McpAgentInfo) => {
    markAgentBusy(agent.id, true);
    try {
      await invoke("remove_mcp_from_agent", { agentId: agent.id });
      showSuccessToast(
        t("integrations.mcp.removedFromClient", { name: agent.display_name }),
      );
      void loadAgents();
    } catch (e) {
      showErrorToast(translateBackendError(t, e), {
        description: agent.display_name,
      });
    } finally {
      markAgentBusy(agent.id, false);
    }
  };

  const mcpUrl = mcpConfig
    ? `http://127.0.0.1:${mcpConfig.port}/mcp/${mcpConfig.token}`
    : "";

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      subPage={subPage}
    >
      <DialogContent className="flex max-h-[calc(100vh-5rem)] max-w-3xl flex-col">
        {!subPage && (
          <DialogHeader className="shrink-0">
            <DialogTitle>{t("integrations.title")}</DialogTitle>
          </DialogHeader>
        )}

        <div
          className={cn(
            "min-h-0 flex-1 overflow-y-auto",
            subPage && "mx-auto w-full max-w-3xl",
          )}
        >
          <AnimatedTabs key={initialTab} defaultValue={initialTab}>
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
              className="@container mt-4 flex flex-col gap-4"
            >
              <div className="flex flex-col gap-4 rounded-md border bg-card p-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="flex items-start gap-3">
                    <LuPlug className="mt-0.5 size-5 text-muted-foreground" />
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
                  <div className="grid grid-cols-1 gap-4 @2xl:grid-cols-2">
                    <div className="flex flex-col gap-2 rounded-md border bg-card p-4">
                      <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
                        {t("integrations.apiPortLabel")}
                      </Label>
                      <div className="flex items-center gap-2">
                        <Input
                          type="number"
                          value={apiPortDraft}
                          onChange={(e) => {
                            setApiPortDraft(e.target.value);
                            const val = Number.parseInt(e.target.value, 10);
                            if (
                              !Number.isNaN(val) &&
                              val >= 1 &&
                              val <= 65535
                            ) {
                              setSettings({ ...settings, api_port: val });
                            }
                          }}
                          onBlur={() => {
                            const val = Number.parseInt(apiPortDraft, 10);
                            if (Number.isNaN(val) || val < 1 || val > 65535) {
                              setApiPortDraft(String(settings.api_port));
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

                    <div className="flex flex-col gap-2 rounded-md border bg-card p-4">
                      <div className="flex items-center justify-between">
                        <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
                          {t("integrations.apiTokenLabel")}
                        </Label>
                      </div>
                      <div className="flex items-center gap-2">
                        <div className="relative flex-1">
                          <Input
                            type={showApiToken ? "text" : "password"}
                            value={settings.api_token ?? ""}
                            readOnly
                            className="pr-10 font-mono"
                          />
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="absolute top-0 right-0 h-full px-3 hover:bg-transparent"
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

                  <div className="flex flex-col gap-2 rounded-md border bg-card p-4">
                    <div className="flex items-center justify-between">
                      <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
                        {t("integrations.apiExampleRequest")}
                      </Label>
                      <CopyToClipboard
                        text={`curl -H "Authorization: Bearer ${settings.api_token ?? "${TOKEN}"}" \\\n     http://127.0.0.1:${apiServerPort ?? settings.api_port}/v1/profiles`}
                        successMessage={t("common.buttons.copied")}
                      />
                    </div>
                    <pre className="overflow-x-auto rounded bg-background p-3 font-mono text-[11px] whitespace-pre">
                      {`curl -H "Authorization: Bearer \${TOKEN}" \\
     http://127.0.0.1:${apiServerPort ?? settings.api_port}/v1/profiles`}
                    </pre>
                  </div>
                </>
              )}
            </AnimatedTabsContent>

            <AnimatedTabsContent
              value="mcp"
              className="mt-4 flex flex-col gap-5"
            >
              <div className="flex flex-col gap-4 rounded-md border bg-card p-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="flex items-start gap-3">
                    <LuZap className="mt-0.5 size-5 text-muted-foreground" />
                    <div className="flex flex-col gap-1">
                      <Label className="text-sm font-medium">
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
                  <AnimatedSwitch
                    checked={settings.mcp_enabled && mcpConfig !== null}
                    disabled={!termsAccepted || isMcpStarting}
                    onCheckedChange={(checked) => void handleMcpToggle(checked)}
                  />
                </div>
              </div>

              {mcpConfig && (
                <>
                  <div className="flex flex-col gap-2 rounded-md border bg-card p-4">
                    <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
                      {t("integrations.mcp.url")}
                    </Label>
                    <div className="flex items-center gap-x-2">
                      <div className="relative flex-1">
                        <Input
                          type={showMcpUrl ? "text" : "password"}
                          value={mcpUrl}
                          readOnly
                          className="pr-10 font-mono text-xs"
                        />
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="absolute top-0 right-0 h-full px-3 hover:bg-transparent"
                          onClick={() => {
                            setShowMcpUrl(!showMcpUrl);
                          }}
                        >
                          {showMcpUrl ? (
                            <EyeOff className="size-4" />
                          ) : (
                            <Eye className="size-4" />
                          )}
                        </Button>
                      </div>
                      <CopyToClipboard
                        text={mcpUrl}
                        successMessage={t("integrations.mcp.urlCopied")}
                      />
                    </div>
                  </div>

                  <div className="@container flex flex-col gap-3">
                    <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
                      {t("integrations.mcp.clientsLabel")}
                    </Label>
                    <div className="grid grid-cols-1 gap-3 @2xl:grid-cols-2">
                      {agents.map((agent) => {
                        const busy = busyAgentIds.has(agent.id);
                        return (
                          <div
                            key={agent.id}
                            className="flex items-center gap-3 rounded-md border bg-card px-3 py-2.5"
                          >
                            <div className="grid size-8 shrink-0 place-items-center rounded-md bg-muted">
                              <AgentIcon category={agent.category} />
                            </div>
                            <div className="min-w-0 flex-1">
                              <p className="truncate text-sm font-medium">
                                {agent.display_name}
                              </p>
                              <p className="text-[10px] tracking-wide text-muted-foreground uppercase">
                                {categoryLabel(t, agent.category)}
                              </p>
                            </div>
                            {agent.connected ? (
                              <div className="flex items-center gap-1">
                                <span className="inline-flex items-center gap-1 rounded-md border bg-muted px-2 py-1 text-[10px] font-medium tracking-wide text-foreground uppercase">
                                  <LuCheck className="size-3" />
                                  {t("integrations.mcp.connected")}
                                </span>
                                <Button
                                  type="button"
                                  variant="ghost"
                                  size="icon"
                                  className="size-8 text-muted-foreground hover:text-destructive"
                                  disabled={busy}
                                  onClick={() => void handleRemoveAgent(agent)}
                                  aria-label={t(
                                    "integrations.mcp.removeAriaLabel",
                                    {
                                      name: agent.display_name,
                                    },
                                  )}
                                >
                                  <LuTrash2 className="size-4" />
                                </Button>
                              </div>
                            ) : (
                              <Button
                                size="sm"
                                variant="outline"
                                disabled={busy}
                                onClick={() => void handleAddAgent(agent)}
                              >
                                {t("integrations.mcp.add")}
                              </Button>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  </div>
                </>
              )}
            </AnimatedTabsContent>
          </AnimatedTabs>
        </div>
      </DialogContent>
    </Dialog>
  );
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import { Eye, EyeOff } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
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
  config_json: string;
}

interface IntegrationsDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function IntegrationsDialog({
  isOpen,
  onClose,
}: IntegrationsDialogProps) {
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
  const [_mcpRunning, setMcpRunning] = useState(false);
  const [showApiToken, setShowApiToken] = useState(false);
  const [showMcpToken, setShowMcpToken] = useState(false);
  const [isApiStarting, setIsApiStarting] = useState(false);
  const [isMcpStarting, setIsMcpStarting] = useState(false);

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

  useEffect(() => {
    if (isOpen) {
      loadSettings();
      loadApiServerStatus();
      loadMcpConfig();
      loadMcpServerStatus();
    }
  }, [
    isOpen,
    loadSettings,
    loadApiServerStatus,
    loadMcpConfig,
    loadMcpServerStatus,
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
        showSuccessToast(`API server started on port ${port}`);
      } else {
        await invoke("stop_api_server");
        setApiServerPort(null);
        const next = await invoke<AppSettings>("save_app_settings", {
          settings: { ...settings, api_enabled: false, api_token: null },
        });
        setSettings(next);
        showSuccessToast("API server stopped");
      }
    } catch (e) {
      console.error("Failed to toggle API:", e);
      showErrorToast("Failed to toggle API server", {
        description: e instanceof Error ? e.message : "Unknown error",
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
        loadMcpConfig();
        showSuccessToast(`MCP server started on port ${port}`);
      } else {
        await invoke("stop_mcp_server");
        const next = await invoke<AppSettings>("save_app_settings", {
          settings: { ...settings, mcp_enabled: false },
        });
        setSettings(next);
        setMcpConfig(null);
        showSuccessToast("MCP server stopped");
      }
    } catch (e) {
      console.error("Failed to toggle MCP server:", e);
      showErrorToast("Failed to toggle MCP server", {
        description: e instanceof Error ? e.message : "Unknown error",
      });
    } finally {
      setIsMcpStarting(false);
    }
  };

  const obfuscateToken = (token: string) =>
    "•".repeat(Math.min(token.length, 32));

  const getFormattedMcpConfig = () => {
    if (!mcpConfig) return "";
    return JSON.stringify(
      {
        mcpServers: {
          "donut-browser": {
            url: `http://127.0.0.1:${mcpConfig.port}/mcp`,
            headers: {
              Authorization: `Bearer ${mcpConfig.token}`,
            },
          },
        },
      },
      null,
      2,
    );
  };

  const getObfuscatedMcpConfig = () => {
    if (!mcpConfig) return "";
    return JSON.stringify(
      {
        mcpServers: {
          "donut-browser": {
            url: `http://127.0.0.1:${mcpConfig.port}/mcp`,
            headers: {
              Authorization: `Bearer ${obfuscateToken(mcpConfig.token)}`,
            },
          },
        },
      },
      null,
      2,
    );
  };

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-xl max-h-[80vh] my-8 flex flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle>Integrations</DialogTitle>
        </DialogHeader>

        <div className="overflow-y-auto flex-1 min-h-0">
          <Tabs defaultValue="api" className="w-full">
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="api">Local API</TabsTrigger>
              <TabsTrigger value="mcp">MCP (AI Assistants)</TabsTrigger>
            </TabsList>

            <TabsContent value="api" className="space-y-4 mt-4">
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="api-enabled"
                  checked={apiServerPort !== null}
                  disabled={isApiStarting}
                  onCheckedChange={handleApiToggle}
                />
                <div className="grid gap-1.5 leading-none">
                  <Label
                    htmlFor="api-enabled"
                    className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                  >
                    Enable Local API Server
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    Allow managing profiles, groups, and proxies via REST API.
                  </p>
                </div>
              </div>

              {settings.api_enabled && (
                <div className="space-y-4 p-4 rounded-md border bg-muted/40">
                  <div className="space-y-2">
                    <Label className="text-sm font-medium">Port</Label>
                    <div className="flex items-center space-x-2">
                      <Input
                        value={apiServerPort ?? settings.api_port}
                        readOnly
                        className="w-24 font-mono"
                      />
                      <span className="text-xs text-muted-foreground">
                        Server is running
                      </span>
                    </div>
                  </div>

                  <div className="space-y-2">
                    <Label className="text-sm font-medium">
                      Authentication Token
                    </Label>
                    <div className="flex items-center space-x-2">
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
                          onClick={() => setShowApiToken(!showApiToken)}
                        >
                          {showApiToken ? (
                            <EyeOff className="h-4 w-4" />
                          ) : (
                            <Eye className="h-4 w-4" />
                          )}
                        </Button>
                      </div>
                      <CopyToClipboard
                        text={settings.api_token ?? ""}
                        successMessage="Token copied"
                      />
                    </div>
                    <p className="text-xs text-muted-foreground">
                      Include in Authorization header: Bearer {"<token>"}
                    </p>
                  </div>
                </div>
              )}
            </TabsContent>

            <TabsContent value="mcp" className="space-y-4 mt-4">
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="mcp-enabled"
                  checked={settings.mcp_enabled && mcpConfig !== null}
                  disabled={!termsAccepted || isMcpStarting}
                  onCheckedChange={handleMcpToggle}
                />
                <div className="grid gap-1.5 leading-none">
                  <Label
                    htmlFor="mcp-enabled"
                    className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                  >
                    Enable MCP Server (Model Context Protocol)
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    Allow AI assistants like Claude Desktop to control browsers.
                    {!termsAccepted && (
                      <span className="ml-1 text-orange-600">
                        (Accept Wayfern terms in Settings first)
                      </span>
                    )}
                  </p>
                </div>
              </div>

              {mcpConfig && (
                <div className="space-y-4 p-4 rounded-md border bg-muted/40">
                  <div className="space-y-2">
                    <Label className="text-sm font-medium">
                      Claude Desktop Configuration
                    </Label>
                    <p className="text-xs text-muted-foreground">
                      Copy this configuration to your Claude Desktop config file
                      at{" "}
                      <code className="bg-muted px-1 rounded">
                        ~/.config/claude/claude_desktop_config.json
                      </code>
                    </p>
                  </div>

                  <div className="relative">
                    <pre className="p-3 text-xs font-mono rounded-md bg-background border overflow-x-auto whitespace-pre">
                      {showMcpToken
                        ? getFormattedMcpConfig()
                        : getObfuscatedMcpConfig()}
                    </pre>
                    <div className="absolute top-2 right-2 flex items-center space-x-1">
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-7 w-7 p-0"
                        onClick={() => setShowMcpToken(!showMcpToken)}
                      >
                        {showMcpToken ? (
                          <EyeOff className="h-3.5 w-3.5" />
                        ) : (
                          <Eye className="h-3.5 w-3.5" />
                        )}
                      </Button>
                      <CopyToClipboard
                        text={getFormattedMcpConfig()}
                        successMessage="Configuration copied"
                      />
                    </div>
                  </div>

                  <div className="space-y-2">
                    <Label className="text-sm font-medium">
                      Available Tools
                    </Label>
                    <ul className="list-disc ml-5 space-y-0.5 text-xs text-muted-foreground">
                      <li>list_profiles - List browser profiles</li>
                      <li>run_profile - Launch a browser</li>
                      <li>kill_profile - Stop a running browser</li>
                      <li>get_profile_status - Check if browser is running</li>
                      <li>list_groups, create_group, etc. - Manage groups</li>
                      <li>list_proxies, create_proxy, etc. - Manage proxies</li>
                    </ul>
                  </div>
                </div>
              )}
            </TabsContent>
          </Tabs>
        </div>
      </DialogContent>
    </Dialog>
  );
}

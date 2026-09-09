"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Eye, EyeOff } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { IconType } from "react-icons";
import {
  LuAppWindow,
  LuCheck,
  LuCloud,
  LuCodeXml,
  LuPlug,
  LuTerminal,
  LuTrash2,
  LuZap,
} from "react-icons/lu";
import {
  SiClaude,
  SiCursor,
  SiWindsurf,
  SiZedindustries,
} from "react-icons/si";
import { VscVscode } from "react-icons/vsc";
import { IntegrationDiagnostics } from "@/components/integration-diagnostics";
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
import { OperationFlow } from "@/components/ui/operation-flow";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { useWayfernTerms } from "@/hooks/use-wayfern-terms";
import { translateBackendError } from "@/lib/backend-errors";
import { canUseRemoteControl } from "@/lib/entitlements";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import { CopyToClipboard } from "./ui/copy-to-clipboard";

/** Where an agent points to drive this browser through Donut cloud. */
const REMOTE_MCP_URL = "https://api.donutbrowser.com/api/mcp";

/**
 * fx refuses a literal Authorization header in its config and reads the
 * bearer token from this variable instead, so its install cannot carry the
 * credential and the user has to export it themselves.
 */
const FX_TOKEN_ENV = "DONUT_MCP_TOKEN";
const FX_AGENT_ID = "fx";

interface AppSettings {
  api_enabled: boolean;
  api_port: number;
  api_token?: string;
  mcp_enabled: boolean;
  mcp_port?: number;
  mcp_token?: string;
  mcp_remote_enabled: boolean;
  /** The remote MCP credential, stored with the same posture as `mcp_token`. */
  mcp_remote_key?: string | null;
}

interface McpConfig {
  port: number;
  token: string;
}

interface McpRemoteStatus {
  enabled: boolean;
  connected: boolean;
  instanceId: string;
  lastError: string | null;
}

/**
 * Only the prefix leaves the backend; the plaintext is installed into agent
 * configs by the app itself. Both spellings are read because the spec writes
 * the Rust field as `token_prefix` while the sibling status struct serialises
 * camelCase, and a mismatch here would silently render a present credential
 * as missing.
 */
interface McpRemoteCredential {
  present: boolean;
  tokenPrefix?: string | null;
  token_prefix?: string | null;
}

/**
 * What a rotation answers. The key is stored and installed by the time this
 * arrives; `failed_clients` are the ids of the clients whose config could not
 * be rewritten, for the user to retry, never a reason to mint again.
 */
interface McpRemoteCredentialRotation
  extends Pick<McpRemoteCredential, "tokenPrefix" | "token_prefix"> {
  failed_clients?: string[];
}

function credentialPrefixOf(
  credential: Pick<McpRemoteCredential, "tokenPrefix" | "token_prefix"> | null,
): string | null {
  return credential?.tokenPrefix ?? credential?.token_prefix ?? null;
}

type AgentCategory = "desktop-app" | "cli" | "editor" | "editor-ext";

/** The two places a client can be pointed at; the `target` of `add_mcp_to_agent`. */
type McpEndpoint = "local" | "remote";

interface McpAgentInfo {
  id: string;
  display_name: string;
  category: AgentCategory;
  connected: boolean;
  detected: boolean;
  /** Which Donut endpoint the agent's existing entry points at, when connected. */
  endpoint?: McpEndpoint | null;
}

type IntegrationsTab = "api" | "mcp" | "remote";

function otherEndpoint(endpoint: McpEndpoint): McpEndpoint {
  return endpoint === "local" ? "remote" : "local";
}

interface IntegrationsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  /** Which tab is displayed when the dialog mounts; defaults to "api". */
  initialTab?: IntegrationsTab;
}

function AgentIcon({ category, id }: { category: AgentCategory; id: string }) {
  const className = "size-5 shrink-0 text-muted-foreground";
  const marks: Record<string, IconType> = {
    "claude-desktop": SiClaude,
    "claude-code": SiClaude,
    cursor: SiCursor,
    vscode: VscVscode,
    windsurf: SiWindsurf,
    zed: SiZedindustries,
  };
  const brand = marks[id];
  if (brand) {
    const Mark = brand;
    return <Mark aria-hidden="true" className={className} />;
  }
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
    mcp_remote_enabled: false,
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
  const [remote, setRemote] = useState<McpRemoteStatus | null>(null);
  const [isRemoteStarting, setIsRemoteStarting] = useState(false);
  const [credential, setCredential] = useState<McpRemoteCredential | null>(
    null,
  );
  const [isRotatingCredential, setIsRotatingCredential] = useState(false);
  const [activeTab, setActiveTab] = useState<IntegrationsTab>(initialTab);
  // Local MCP is removed; an in-app attempt to enable it opens this dialog.
  const [localDeprecatedOpen, setLocalDeprecatedOpen] = useState(false);
  // Mod+I re-targets an open dialog through `initialTab`. The tabs are
  // controlled (see `shownTab`), so a remount would not adopt the new value;
  // this is React's adjust-state-on-prop-change form of the same thing.
  const [adoptedInitialTab, setAdoptedInitialTab] = useState(initialTab);
  if (adoptedInitialTab !== initialTab) {
    setAdoptedInitialTab(initialTab);
    setActiveTab(initialTab);
  }

  const { termsAccepted } = useWayfernTerms();
  const { user, isLoggedIn, isLoading: authLoading } = useCloudAuth();
  // Asked of the SERVER, not inferred.
  //
  // Two wrong answers were tried before this one. `canUseRemoteControl(user)`
  // reads the cached entitlement, which is computed for this account alone, so
  // an entitled enterprise MEMBER was told to buy the plan they are already on.
  // Falling back to `remote.connected` was worse in BOTH directions: an open
  // socket only proves the plan is active, so every paying plan connects and
  // would have been handed an endpoint that answers 402; and the flag is live,
  // so an outage, a sleep or a slot conflict flipped an entitled member back to
  // the upgrade nag, permanently in the slot-taken case.
  //
  // `GET /api/mcp/status` answers the remote-control question directly and is
  // deliberately ungated so it can be asked before you are allowed anything.
  const [serverEntitled, setServerEntitled] = useState<boolean | null>(null);
  const remoteEntitled = serverEntitled ?? canUseRemoteControl(user);
  // The local cache is only a placeholder until the server answers; treating
  // "not yet known" as "no" renders an upgrade prompt at somebody who has paid.
  const remoteEntitlementKnown =
    serverEntitled !== null || (!authLoading && user !== null);

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

  const loadRemoteStatus = useCallback(async () => {
    try {
      setRemote(await invoke<McpRemoteStatus>("get_mcp_remote_status"));
    } catch (e) {
      console.error("Failed to get MCP remote status:", e);
    }
  }, []);

  const loadCredential = useCallback(async () => {
    try {
      setCredential(
        await invoke<McpRemoteCredential>("get_mcp_remote_credential"),
      );
    } catch (e) {
      console.error("Failed to get MCP remote credential:", e);
    }
  }, []);

  /**
   * The server's own verdict on remote control, cached for this dialog.
   *
   * A network call, so it never gates the status line: `remoteEntitled` falls
   * back to the local cache until this lands, and only the endpoint panels
   * wait on `remoteEntitlementKnown`.
   */
  const loadRemoteEntitlement = useCallback(async () => {
    if (!isLoggedIn) return;
    try {
      setServerEntitled(
        await invoke<boolean>("get_remote_control_entitlement"),
      );
    } catch {
      // Offline, or the endpoint is unreachable. Leave the local cache in
      // charge rather than asserting an answer we do not have.
    }
  }, [isLoggedIn]);

  useEffect(() => {
    if (isOpen) {
      void loadSettings();
      void loadApiServerStatus();
      void loadMcpConfig();
      void loadMcpServerStatus();
      void loadAgents();
      void loadRemoteStatus();
      void loadCredential();
      void loadRemoteEntitlement();
    }
  }, [
    isOpen,
    loadSettings,
    loadApiServerStatus,
    loadMcpConfig,
    loadMcpServerStatus,
    loadAgents,
    loadRemoteStatus,
    loadCredential,
    loadRemoteEntitlement,
  ]);

  // The bridge reconnects on its own timer, so its state changes while this
  // dialog is open and nothing the user did caused it. Subscribed rather than
  // polled: a socket that drops and comes back should be visible immediately,
  // not on the next tick of an interval nobody wants to run.
  useEffect(() => {
    const unlisten = listen<McpRemoteStatus>("mcp-remote-status", (event) => {
      setRemote(event.payload);
    });
    return () => {
      void unlisten.then((off) => {
        off();
      });
    };
  }, []);

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
        description: translateBackendError(t, e),
      });
    } finally {
      setIsApiStarting(false);
    }
  };

  const handleMcpToggle = async (enabled: boolean) => {
    setIsMcpStarting(true);
    try {
      if (enabled) {
        // Local MCP is removed. The command refuses (MCP_LOCAL_REMOVED); open
        // the dialog that points the user at remote MCP instead of enabling.
        try {
          await invoke<number>("start_mcp_server");
        } catch {
          setLocalDeprecatedOpen(true);
        }
        return;
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
        description: translateBackendError(t, e),
      });
    } finally {
      setIsMcpStarting(false);
    }
  };

  const handleRemoteToggle = async (enabled: boolean) => {
    setIsRemoteStarting(true);
    try {
      const next = await invoke<McpRemoteStatus>(
        enabled ? "start_mcp_remote_bridge" : "stop_mcp_remote_bridge",
      );
      setRemote(next);
      setSettings((prev) => ({ ...prev, mcp_remote_enabled: enabled }));
      showSuccessToast(
        enabled
          ? t("integrations.remote.enabled")
          : t("integrations.remote.disabled"),
      );
    } catch (e) {
      console.error("Failed to toggle remote control:", e);
      showErrorToast(t("integrations.remote.toggleFailed"), {
        description: translateBackendError(t, e),
      });
    } finally {
      setIsRemoteStarting(false);
    }
  };

  /**
   * A rotation succeeds once the key is stored. A client it could not be
   * written into is named here so the user knows which one to retry, rather
   * than surfacing as a failed rotation that invites a second mint.
   */
  const reportFailedClients = (rotated: McpRemoteCredentialRotation) => {
    const failed = rotated.failed_clients ?? [];
    if (failed.length === 0) return;
    const names = failed.map(
      (id) => agents.find((agent) => agent.id === id)?.display_name ?? id,
    );
    showErrorToast(
      t("integrations.remote.credentialClientsFailed", {
        clients: names.join(", "),
      }),
    );
  };

  const handleRotateCredential = async () => {
    const hadCredential = credential?.present ?? false;
    setIsRotatingCredential(true);
    try {
      const rotated = await invoke<McpRemoteCredentialRotation>(
        "rotate_mcp_remote_credential",
      );
      setCredential({
        present: true,
        tokenPrefix: credentialPrefixOf(rotated),
      });
      showSuccessToast(
        hadCredential
          ? t("integrations.remote.credentialRotated")
          : t("integrations.remote.credentialCreated"),
      );
      reportFailedClients(rotated);
      // The backend reinstalls every remote client with the new key and the
      // plaintext behind the fx export line changes with it.
      void loadAgents();
      void loadSettings();
    } catch (e) {
      console.error("Failed to rotate MCP remote credential:", e);
      showErrorToast(t("integrations.remote.credentialRotateFailed"), {
        description: translateBackendError(t, e),
      });
    } finally {
      setIsRotatingCredential(false);
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

  /**
   * The mint that a first install triggers, shared between clicks. Two
   * clients added back to back would otherwise each mint a key, and the
   * second mint retires the first one from under the client just written.
   */
  const pendingMint = useRef<Promise<void> | null>(null);

  const ensureCredential = () => {
    if (credential?.present) return Promise.resolve();
    if (!pendingMint.current) {
      pendingMint.current = invoke<McpRemoteCredentialRotation>(
        "rotate_mcp_remote_credential",
      )
        .then((minted) => {
          setCredential({
            present: true,
            tokenPrefix: credentialPrefixOf(minted),
          });
          reportFailedClients(minted);
        })
        .finally(() => {
          pendingMint.current = null;
          // Whatever the mint answered, what is on disk is what the page
          // shows: a key stored before a later step failed is still a key.
          void loadCredential();
        });
    }
    return pendingMint.current;
  };

  /**
   * Installs an endpoint into an agent. "Add" and "Switch" are the same
   * write, because the installer replaces the Donut entry wholesale; only the
   * toast differs.
   */
  const installEndpoint = async (
    agent: McpAgentInfo,
    target: McpEndpoint,
    successMessage: string,
  ) => {
    markAgentBusy(agent.id, true);
    try {
      // The remote installer writes the stored credential and refuses when
      // there is none, so the first client mints it here: one click for the
      // user. The local server authenticates through its URL instead.
      if (target === "remote") await ensureCredential();
      await invoke("add_mcp_to_agent", { agentId: agent.id, target });
      showSuccessToast(successMessage);
      if (target === "remote") {
        void loadCredential();
        void loadSettings();
      }
      void loadAgents();
    } catch (e) {
      showErrorToast(translateBackendError(t, e), {
        description: agent.display_name,
      });
    } finally {
      markAgentBusy(agent.id, false);
    }
  };

  const handleAddAgent = (agent: McpAgentInfo, target: McpEndpoint) =>
    installEndpoint(
      agent,
      target,
      t("integrations.mcp.addedToClient", { name: agent.display_name }),
    );

  const handleSwitchAgent = (agent: McpAgentInfo, target: McpEndpoint) =>
    installEndpoint(
      agent,
      target,
      target === "remote"
        ? t("integrations.remote.switchedClient", { name: agent.display_name })
        : t("integrations.mcp.switchedToLocal", { name: agent.display_name }),
    );

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

  const credentialPresent = credential?.present ?? false;
  const credentialPrefix = credentialPrefixOf(credential);
  // The example names the credential it expects by its visible prefix; the
  // dialog never has the plaintext on screen, so the line is a template either
  // way and copying it hands over exactly what is shown.
  const exampleBearer = credentialPrefix
    ? t("integrations.remote.credentialPrefix", { prefix: credentialPrefix })
    : "${TOKEN}";
  const remoteExampleRequest = [
    `curl -X POST -H "Authorization: Bearer ${exampleBearer}" \\`,
    `     -H "Content-Type: application/json" \\`,
    `     -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \\`,
    `     ${REMOTE_MCP_URL}`,
  ].join("\n");
  const fxExportLine = settings.mcp_remote_key
    ? `export ${FX_TOKEN_ENV}=${settings.mcp_remote_key}`
    : null;

  // Remote control is not something a regular user is told about: the tab
  // exists only for an account entitled to it, or while the bridge is already
  // on so it can always be switched off. Nothing else on the page names it.
  const remoteTabShown =
    remoteEntitled || settings.mcp_remote_enabled || (remote?.enabled ?? false);
  // A tab that is not offered cannot be the one displayed, whether the caller
  // asked for it or it vanished under the user.
  const shownTab: IntegrationsTab =
    activeTab === "remote" && !remoteTabShown ? "api" : activeTab;

  /**
   * The clients grid, once for both tabs. `target` is the endpoint this tab
   * installs; a client already on the other endpoint is told so and offered
   * the switch, which is the same install with a different toast.
   */
  const clientsGrid = (target: McpEndpoint) => (
    <div className="@container flex flex-col gap-3">
      <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
        {t("integrations.mcp.clientsLabel")}
      </Label>
      <div className="grid grid-cols-1 gap-3 @2xl:grid-cols-2">
        {agents.map((agent) => {
          const busy = busyAgentIds.has(agent.id);
          const onOtherEndpoint =
            agent.connected && agent.endpoint === otherEndpoint(target);
          const onThisEndpoint = agent.connected && !onOtherEndpoint;
          return (
            <div
              key={agent.id}
              className="flex flex-col justify-center gap-2 rounded-md border bg-card px-3 py-2.5"
            >
              <div className="flex items-center gap-3">
                <AgentIcon id={agent.id} category={agent.category} />
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
                    {onOtherEndpoint ? (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={busy}
                        onClick={() => void handleSwitchAgent(agent, target)}
                      >
                        {target === "remote"
                          ? t("integrations.remote.switchToRemote")
                          : t("integrations.mcp.switchToLocal")}
                      </Button>
                    ) : (
                      <span className="inline-flex items-center gap-1 text-xs font-medium text-foreground">
                        <LuCheck className="size-3" />
                        {t("appFeedback.configured")}
                      </span>
                    )}
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="size-8 text-muted-foreground hover:text-destructive-text"
                      disabled={busy}
                      onClick={() => void handleRemoveAgent(agent)}
                      aria-label={t("integrations.mcp.removeAriaLabel", {
                        name: agent.display_name,
                      })}
                    >
                      <LuTrash2 className="size-4" />
                    </Button>
                  </div>
                ) : (
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={busy}
                    onClick={() => void handleAddAgent(agent, target)}
                  >
                    {t("integrations.mcp.add")}
                  </Button>
                )}
              </div>
              {agent.connected && (
                <details data-slot="client-route" className="text-xs">
                  <summary className="cursor-pointer rounded-sm py-1 text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring">
                    {t("appFeedback.routeDetails")}
                  </summary>
                  <OperationFlow
                    label={t("appFeedback.routeDetails")}
                    active={
                      agent.endpoint === "remote"
                        ? remote?.connected
                          ? 2
                          : 1
                        : 0
                    }
                    steps={[
                      {
                        id: "client",
                        label: agent.display_name,
                        detail: t("appFeedback.configured"),
                      },
                      {
                        id: "endpoint",
                        label: t(
                          agent.endpoint === "remote"
                            ? "integrations.tabRemote"
                            : "integrations.tabMcp",
                        ),
                        detail:
                          agent.endpoint === "remote" ? (
                            <span className="break-all">{REMOTE_MCP_URL}</span>
                          ) : (
                            t("integrations.mcp.deprecatedBannerTitle")
                          ),
                      },
                      {
                        id: "device",
                        label: t("appFeedback.thisDevice"),
                        detail: t(
                          agent.endpoint === "remote" && remote?.connected
                            ? "appFeedback.reachable"
                            : "appFeedback.notVerified",
                        ),
                      },
                    ]}
                  />
                  <p className="pt-1 leading-relaxed text-muted-foreground">
                    {t("appFeedback.remoteProbeScope")}
                  </p>
                </details>
              )}
              {onOtherEndpoint && (
                <p className="text-xs text-warning-text">
                  {target === "remote"
                    ? t("appFeedback.configuredLocal")
                    : t("appFeedback.configuredRemote")}
                </p>
              )}
              {target === "remote" &&
                onThisEndpoint &&
                agent.id === FX_AGENT_ID && (
                  <div className="flex items-center justify-between gap-2">
                    <p className="text-xs text-muted-foreground">
                      {t("integrations.remote.fxHint")}
                    </p>
                    {fxExportLine && (
                      <CopyToClipboard
                        variant="ghost"
                        size="sm"
                        className="shrink-0"
                        text={fxExportLine}
                        successMessage={t("integrations.remote.fxExportCopied")}
                      />
                    )}
                  </div>
                )}
            </div>
          );
        })}
      </div>
    </div>
  );

  return (
    <>
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

          <div className="min-h-0 flex-1 overflow-y-auto">
            <div className={cn(subPage && "mx-auto w-full max-w-4xl")}>
              <AnimatedTabs
                value={shownTab}
                onValueChange={(value) =>
                  setActiveTab(value as IntegrationsTab)
                }
              >
                <AnimatedTabsList>
                  <AnimatedTabsTrigger value="api">
                    {t("integrations.tabApi")}
                  </AnimatedTabsTrigger>
                  <AnimatedTabsTrigger value="mcp">
                    {t("integrations.tabMcp")}
                  </AnimatedTabsTrigger>
                  {remoteTabShown && (
                    <AnimatedTabsTrigger value="remote">
                      <LuCloud className="size-3.5" />
                      {t("integrations.tabRemote")}
                    </AnimatedTabsTrigger>
                  )}
                </AnimatedTabsList>

                <AnimatedTabsContent
                  value="api"
                  className="@container mt-4 flex flex-col gap-4"
                >
                  <IntegrationDiagnostics
                    key={`${apiServerPort}:${settings.api_token ?? ""}`}
                    target="api"
                  />
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
                        onCheckedChange={(checked) =>
                          void handleApiToggle(checked)
                        }
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
                                if (
                                  Number.isNaN(val) ||
                                  val < 1 ||
                                  val > 65535
                                ) {
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
                                isApiStarting ||
                                apiServerPort === settings.api_port
                              }
                              onClick={async () => {
                                const port = settings.api_port;
                                if (port < 1 || port > 65535) {
                                  showErrorToast(
                                    t("integrations.apiInvalidPort"),
                                    {
                                      description: t(
                                        "integrations.apiInvalidPortDescription",
                                      ),
                                    },
                                  );
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
                                  showErrorToast(
                                    t("integrations.apiStartFailed"),
                                    {
                                      description: translateBackendError(t, e),
                                    },
                                  );
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

                {remoteTabShown && (
                  <AnimatedTabsContent
                    value="remote"
                    className="mt-4 flex flex-col gap-5"
                  >
                    <IntegrationDiagnostics
                      key={`${credentialPrefixOf(credential)}:${remote?.enabled}`}
                      target="remote"
                    />
                    <div className="flex flex-col gap-4 rounded-md border bg-card p-4">
                      <div className="flex items-start justify-between gap-3">
                        <div className="flex items-start gap-3">
                          <LuCloud className="mt-0.5 size-5 text-muted-foreground" />
                          <div className="flex flex-col gap-1">
                            <Label className="text-sm font-medium">
                              {t("integrations.remote.enableLabel")}
                            </Label>
                            <p className="text-xs text-muted-foreground">
                              {t("integrations.remote.enableDescription")}
                            </p>
                          </div>
                        </div>
                        <AnimatedSwitch
                          checked={remote?.enabled ?? false}
                          disabled={
                            !termsAccepted || !isLoggedIn || isRemoteStarting
                          }
                          onCheckedChange={(checked) =>
                            void handleRemoteToggle(checked)
                          }
                        />
                      </div>

                      {!isLoggedIn && (
                        <p className="text-xs text-warning-text">
                          {t("integrations.remote.signInRequired")}
                        </p>
                      )}
                      {isLoggedIn && !termsAccepted && (
                        <p className="text-xs text-warning-text">
                          {t("integrations.mcpAcceptTermsFirst")}
                        </p>
                      )}

                      {remote?.enabled && (
                        <div className="flex items-center gap-2 text-xs">
                          <span
                            className={cn(
                              "size-1.5 rounded-full",
                              remote.connected
                                ? "bg-success"
                                : "bg-muted-foreground",
                            )}
                          />
                          <span className="text-muted-foreground">
                            {/* Three states, not two. A refusal the bridge will
                            keep receiving (an unentitled plan, a taken slot,
                            a dead credential) re-dials for ever without ever
                            connecting, and "Connecting as" over the top of the
                            error underneath it read as a hang rather than an
                            answer. */}
                            {remote.connected
                              ? t("integrations.remote.connected")
                              : remote.lastError
                                ? t("integrations.remote.notConnected")
                                : t("integrations.remote.connecting")}
                          </span>
                          {/* Only under a phrase that governs it. "Connected as"
                          and "Connecting as" are open phrases; "Not connected"
                          is closed, and the id dangling after it read as a
                          sentence fragment in all ten locales. */}
                          {!(remote.lastError && !remote.connected) && (
                            <code className="rounded bg-muted px-2 py-1 font-mono text-[11px]">
                              {remote.instanceId}
                            </code>
                          )}
                        </div>
                      )}

                      {remote?.enabled &&
                        !remote.connected &&
                        remote.lastError && (
                          <p className="text-xs text-destructive-text">
                            {translateBackendError(t, remote.lastError)}
                          </p>
                        )}
                    </div>

                    {remote?.enabled && (
                      <>
                        {/* Everything below is gated on entitlement: handing an
                        unentitled customer a URL, a credential or an "Add"
                        button that can only answer 402 is the same
                        wrong-diagnosis trap as telling them their desktop is
                        offline. The warning explains why it is not shown. */}
                        {remoteEntitlementKnown && !remoteEntitled && (
                          <div className="rounded-md border border-warning/50 bg-warning/10 p-4">
                            <p className="text-xs text-warning-text">
                              {t("integrations.remote.notEntitled")}
                            </p>
                          </div>
                        )}

                        {(remoteEntitled || !remoteEntitlementKnown) && (
                          <>
                            <div className="flex flex-col gap-2 rounded-md border bg-card p-4">
                              <div className="flex items-center justify-between gap-3">
                                <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
                                  {t("integrations.remote.credentialLabel")}
                                </Label>
                                <Button
                                  size="sm"
                                  variant="outline"
                                  disabled={isRotatingCredential}
                                  onClick={() => void handleRotateCredential()}
                                >
                                  {credentialPresent
                                    ? t("integrations.remote.credentialRotate")
                                    : t("integrations.remote.credentialCreate")}
                                </Button>
                              </div>
                              {credentialPresent ? (
                                credentialPrefix && (
                                  <code className="w-fit rounded bg-muted px-2 py-1 font-mono text-[11px]">
                                    {t("integrations.remote.credentialPrefix", {
                                      prefix: credentialPrefix,
                                    })}
                                  </code>
                                )
                              ) : (
                                <p className="text-xs text-muted-foreground">
                                  {t("integrations.remote.credentialNone")}
                                </p>
                              )}
                              <p className="text-xs text-muted-foreground">
                                {t("integrations.remote.credentialHint")}
                              </p>
                            </div>

                            <div className="flex flex-col gap-2 rounded-md border bg-card p-4">
                              <div className="flex items-center justify-between">
                                <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
                                  {t("integrations.remote.endpointLabel")}
                                </Label>
                                <CopyToClipboard
                                  text={REMOTE_MCP_URL}
                                  successMessage={t(
                                    "integrations.remote.endpointCopied",
                                  )}
                                />
                              </div>
                              <Input
                                value={REMOTE_MCP_URL}
                                readOnly
                                className="font-mono text-xs"
                              />
                              <p className="text-xs text-muted-foreground">
                                {t("integrations.remote.endpointDescription")}
                              </p>
                            </div>

                            <div className="flex flex-col gap-2 rounded-md border bg-card p-4">
                              <div className="flex items-center justify-between">
                                <Label className="text-[10px] tracking-wide text-muted-foreground uppercase">
                                  {t("integrations.remote.exampleRequest")}
                                </Label>
                                <CopyToClipboard
                                  text={remoteExampleRequest}
                                  successMessage={t("common.buttons.copied")}
                                />
                              </div>
                              <pre className="overflow-x-auto rounded bg-background p-3 font-mono text-[11px] whitespace-pre">
                                {remoteExampleRequest}
                              </pre>
                            </div>

                            {clientsGrid("remote")}
                          </>
                        )}
                      </>
                    )}
                  </AnimatedTabsContent>
                )}

                <AnimatedTabsContent
                  value="mcp"
                  className="mt-4 flex flex-col gap-5"
                >
                  <div className="flex flex-col gap-2 rounded-md border border-warning/50 bg-warning/10 p-4">
                    <p className="text-sm font-medium">
                      {t("integrations.mcp.deprecatedBannerTitle")}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {t("integrations.mcp.deprecatedBannerBody")}
                    </p>
                    <div>
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        className="mt-1"
                        onClick={() => {
                          setActiveTab("remote");
                        }}
                      >
                        {t("integrations.mcp.deprecatedCta")}
                      </Button>
                    </div>
                  </div>

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
                              <span className="ml-1 text-warning-text">
                                {t("integrations.mcpAcceptTermsFirst")}
                              </span>
                            )}
                          </p>
                        </div>
                      </div>
                      <AnimatedSwitch
                        checked={settings.mcp_enabled && mcpConfig !== null}
                        disabled={!termsAccepted || isMcpStarting}
                        onCheckedChange={(checked) =>
                          void handleMcpToggle(checked)
                        }
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

                      {clientsGrid("local")}
                    </>
                  )}
                </AnimatedTabsContent>
              </AnimatedTabs>
            </div>
          </div>
        </DialogContent>
      </Dialog>
      <Dialog open={localDeprecatedOpen} onOpenChange={setLocalDeprecatedOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("mcpLocalDeprecated.title")}</DialogTitle>
          </DialogHeader>
          <p className="text-sm text-muted-foreground">
            {t("mcpLocalDeprecated.description")}
          </p>
          <div className="mt-2 flex justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setLocalDeprecatedOpen(false);
              }}
            >
              {t("common.buttons.close")}
            </Button>
            <Button
              type="button"
              onClick={() => {
                setLocalDeprecatedOpen(false);
                setActiveTab("remote");
              }}
            >
              {t("integrations.mcp.deprecatedCta")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

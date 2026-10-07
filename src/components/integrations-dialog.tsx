"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuArrowRight,
  LuCloud,
  LuEye,
  LuEyeOff,
  LuPlug,
  LuRefreshCw,
  LuTrash2,
} from "react-icons/lu";
import {
  type CodeExample,
  CodeExamplesDialog,
} from "@/components/code-examples-dialog";
import { IntegrationDiagnostics } from "@/components/integration-diagnostics";
import { LoadingButton } from "@/components/loading-button";
import { AgentIcon } from "@/components/mcp-agent-icon";
import { AnimatedDisclosureContent } from "@/components/ui/animated-disclosure";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Button } from "@/components/ui/button";
import { CodeSnippet } from "@/components/ui/code-snippet";
import { ConfirmationMark } from "@/components/ui/confirmation-mark";
import { CopyToClipboard } from "@/components/ui/copy-to-clipboard";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { OperationFlow } from "@/components/ui/operation-flow";
import {
  FeatureIcon,
  SECTION_LABEL_CLASS,
  SettingsPanel,
  SettingsRow,
  StatusLight,
} from "@/components/ui/settings-panel";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { useWayfernTerms } from "@/hooks/use-wayfern-terms";
import {
  curlSnippet,
  LOCAL_API_EXAMPLES,
  maskToken,
  mcpClientConfig,
  oneLine,
  PROFILE_ID_PLACEHOLDER,
  REMOTE_MCP_EXAMPLES,
  REQUEST_LANGUAGES,
  requestSnippet,
} from "@/lib/api-examples";
import { translateBackendError } from "@/lib/backend-errors";
import { canUseRemoteControl } from "@/lib/entitlements";
import {
  type AgentCategory,
  credentialPrefixOf,
  FX_AGENT_ID,
  fxExportLine,
  type McpAgentInfo,
  type McpRemoteCredential,
  type McpRemoteCredentialRotation,
  type McpRemoteStatus,
  REMOTE_MCP_URL,
} from "@/lib/mcp";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";

interface AppSettings {
  api_enabled: boolean;
  api_port: number;
  api_token?: string;
  mcp_remote_enabled: boolean;
  mcp_remote_key?: string | null;
}

type IntegrationsTab = "api" | "remote";

interface IntegrationsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  /** Which tab is displayed when the dialog mounts; defaults to "api". */
  initialTab?: IntegrationsTab;
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
    mcp_remote_enabled: false,
  });
  const [apiServerPort, setApiServerPort] = useState<number | null>(null);
  const [showApiToken, setShowApiToken] = useState(false);
  const [isApiStarting, setIsApiStarting] = useState(false);
  const [agents, setAgents] = useState<McpAgentInfo[]>([]);
  const [busyAgentIds, setBusyAgentIds] = useState<Set<string>>(new Set());
  const [apiPortDraft, setApiPortDraft] = useState<string>("10108");
  const [remote, setRemote] = useState<McpRemoteStatus | null>(null);
  const [isRemoteStarting, setIsRemoteStarting] = useState(false);
  const [credential, setCredential] = useState<McpRemoteCredential | null>(
    null,
  );
  const [isRotatingCredential, setIsRotatingCredential] = useState(false);
  const [apiExamplesOpen, setApiExamplesOpen] = useState(false);
  const [remoteExamplesOpen, setRemoteExamplesOpen] = useState(false);
  const apiSwitchId = useId();
  const apiPortId = useId();
  const apiTokenId = useId();
  const remoteSwitchId = useId();
  const [activeTab, setActiveTab] = useState<IntegrationsTab>(initialTab);
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
  // "not yet known" as "no" hides the endpoint from somebody who has paid.
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
      void loadAgents();
      void loadRemoteStatus();
      void loadCredential();
      void loadRemoteEntitlement();
    }
  }, [
    isOpen,
    loadSettings,
    loadApiServerStatus,
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
   * The installer replaces the Donut entry wholesale, so a client whose entry
   * points anywhere else is fixed by the same write.
   */
  const handleAddAgent = async (agent: McpAgentInfo) => {
    markAgentBusy(agent.id, true);
    try {
      // The installer writes the stored credential and refuses when there is
      // none, so the first client mints it here: one click for the user.
      await ensureCredential();
      await invoke("add_mcp_to_agent", { agentId: agent.id });
      showSuccessToast(
        t("integrations.mcp.addedToClient", { name: agent.display_name }),
      );
      void loadCredential();
      void loadSettings();
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

  const handleSavePort = async () => {
    const port = settings.api_port;
    if (port < 1 || port > 65535) {
      showErrorToast(t("integrations.apiInvalidPort"), {
        description: t("integrations.apiInvalidPortDescription"),
      });
      return;
    }
    setIsApiStarting(true);
    try {
      await invoke("stop_api_server");
      const next = await invoke<AppSettings>("save_app_settings", {
        settings,
      });
      setSettings(next);
      const actualPort = await invoke<number>("start_api_server", { port });
      setApiServerPort(actualPort);
      if (actualPort !== port) {
        showErrorToast(t("integrations.apiPortInUse", { port }), {
          description: t("integrations.apiFallbackPort", { port: actualPort }),
        });
      } else {
        showSuccessToast(t("integrations.apiRunning", { port: actualPort }));
      }
    } catch (e) {
      showErrorToast(t("integrations.apiStartFailed"), {
        description: translateBackendError(t, e),
      });
    } finally {
      setIsApiStarting(false);
    }
  };

  const credentialPresent = credential?.present ?? false;
  const credentialPrefix = credentialPrefixOf(credential);
  // The examples name the credential they expect by its visible prefix; the
  // dialog never has the plaintext on screen, so the lines are templates
  // either way and copying one hands over exactly what is shown.
  const exampleBearer = credentialPrefix
    ? t("integrations.remote.credentialPrefix", { prefix: credentialPrefix })
    : "${TOKEN}";
  const remoteTarget = { baseUrl: REMOTE_MCP_URL, token: exampleBearer };
  const remoteQuickStart = oneLine(
    curlSnippet(REMOTE_MCP_EXAMPLES[0].request, remoteTarget),
  );
  const fxLine = fxExportLine(settings.mcp_remote_key);

  const apiBaseUrl = `http://127.0.0.1:${apiServerPort ?? settings.api_port}`;
  const apiToken = settings.api_token ?? "";
  const maskedApiToken = maskToken(apiToken);
  // The local token is this machine's own, so a copied example carries it
  // and runs as pasted; the page itself shows only its ends.
  const apiDisplayTarget = { baseUrl: apiBaseUrl, token: maskedApiToken };
  const apiCopyTarget = { baseUrl: apiBaseUrl, token: apiToken || "TOKEN" };
  const apiQuickStart = LOCAL_API_EXAMPLES[0].request;

  const apiExamples: CodeExample[] = LOCAL_API_EXAMPLES.map((example) => ({
    id: example.id,
    title: t(`integrations.examples.items.${example.id}.title`),
    description: t(`integrations.examples.items.${example.id}.description`),
    method: example.request.method,
    paid: example.paid,
    response: example.response,
    snippets: Object.fromEntries(
      REQUEST_LANGUAGES.map((language) => [
        language,
        {
          display: requestSnippet(language, example.request, apiDisplayTarget),
          copy: requestSnippet(language, example.request, apiCopyTarget),
        },
      ]),
    ),
  }));

  const remoteConfig = mcpClientConfig(REMOTE_MCP_URL, exampleBearer);
  const remoteExamples: CodeExample[] = [
    ...REMOTE_MCP_EXAMPLES.map((example) => ({
      id: example.id,
      title: t(`integrations.examples.items.${example.id}.title`),
      description: t(`integrations.examples.items.${example.id}.description`),
      method: example.request.method,
      response: example.response,
      snippets: Object.fromEntries(
        REQUEST_LANGUAGES.map((language) => {
          const code = requestSnippet(language, example.request, remoteTarget);
          return [language, { display: code, copy: code }];
        }),
      ),
    })),
    {
      id: "clientConfig",
      title: t("integrations.examples.items.clientConfig.title"),
      description: t("integrations.examples.items.clientConfig.description"),
      snippets: { json: { display: remoteConfig, copy: remoteConfig } },
    },
  ];

  // Remote control is not something a regular user is told about: the tab
  // exists only for an account entitled to it, or while the bridge is already
  // on so it can always be switched off. Nothing else on the page names it.
  const remoteTabShown =
    remoteEntitled || settings.mcp_remote_enabled || (remote?.enabled ?? false);
  // A tab that is not offered cannot be the one displayed, whether the caller
  // asked for it or it vanished under the user.
  const shownTab: IntegrationsTab =
    activeTab === "remote" && !remoteTabShown ? "api" : activeTab;

  const clientsGrid = (
    <section className="flex flex-col gap-2">
      <h3 className={SECTION_LABEL_CLASS}>
        {t("integrations.mcp.clientsLabel")}
      </h3>
      <div className="grid grid-cols-1 gap-2 @2xl:grid-cols-2">
        {agents.map((agent) => {
          const busy = busyAgentIds.has(agent.id);
          return (
            <div
              key={agent.id}
              className={cn(
                "flex flex-col justify-center gap-2 rounded-xl px-3 py-2.5 transition-colors duration-200",
                agent.connected ? "bg-foreground/5" : "bg-foreground/3",
              )}
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
                    <span className="inline-flex items-center gap-1 text-xs font-medium text-foreground">
                      <ConfirmationMark />
                      {t("appFeedback.configured")}
                    </span>
                    <Button
                      type="button"
                      variant="subtle"
                      size="icon"
                      className="size-8 rounded-lg hover:text-destructive-text"
                      disabled={busy}
                      onClick={() => void handleRemoveAgent(agent)}
                      aria-label={t("integrations.mcp.removeAriaLabel", {
                        name: agent.display_name,
                      })}
                    >
                      <LuTrash2 className="size-3.5" />
                    </Button>
                  </div>
                ) : (
                  <Button
                    size="sm"
                    variant="soft"
                    className="h-8 rounded-lg text-xs"
                    disabled={busy}
                    aria-busy={busy}
                    onClick={() => void handleAddAgent(agent)}
                  >
                    {t("integrations.mcp.add")}
                  </Button>
                )}
              </div>
              {agent.connected && (
                <details data-slot="client-route" className="text-xs">
                  <summary className="w-fit cursor-pointer rounded-sm py-1 text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring">
                    {t("appFeedback.routeDetails")}
                  </summary>
                  <OperationFlow
                    label={t("appFeedback.routeDetails")}
                    active={remote?.connected ? 2 : 1}
                    steps={[
                      {
                        id: "client",
                        label: agent.display_name,
                        detail: t("appFeedback.configured"),
                      },
                      {
                        id: "endpoint",
                        label: t("integrations.tabRemote"),
                        detail: (
                          <span className="break-all">{REMOTE_MCP_URL}</span>
                        ),
                      },
                      {
                        id: "device",
                        label: t("appFeedback.thisDevice"),
                        detail: t(
                          remote?.connected
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
              {agent.connected && agent.id === FX_AGENT_ID && (
                <div className="flex items-center justify-between gap-2">
                  <p className="text-xs text-muted-foreground">
                    {t("integrations.remote.fxHint")}
                  </p>
                  {fxLine && (
                    <CopyToClipboard
                      variant="subtle"
                      className="size-8 shrink-0 rounded-lg"
                      text={fxLine}
                      successMessage={t("integrations.remote.fxExportCopied")}
                    />
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );

  const examplesButton = (onClick: () => void) => (
    <Button
      type="button"
      variant="subtle"
      size="sm"
      className="group/examples h-7 gap-1 rounded-lg px-2 text-xs"
      onClick={onClick}
    >
      {t("integrations.examples.open")}
      <LuArrowRight className="size-3.5 transition-transform duration-150 group-hover/examples:translate-x-0.5 motion-reduce:transition-none" />
    </Button>
  );

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

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className={cn(subPage && "mx-auto w-full max-w-4xl")}>
            <AnimatedTabs
              value={shownTab}
              onValueChange={(value) => setActiveTab(value as IntegrationsTab)}
            >
              <AnimatedTabsList>
                <AnimatedTabsTrigger value="api">
                  {t("integrations.tabApi")}
                </AnimatedTabsTrigger>
                {remoteTabShown && (
                  <AnimatedTabsTrigger value="remote">
                    <LuCloud className="size-3.5" />
                    {t("integrations.tabRemote")}
                  </AnimatedTabsTrigger>
                )}
              </AnimatedTabsList>

              <AnimatedTabsContent value="api" className="@container mt-4">
                <SettingsPanel>
                  <div className="flex items-start gap-3">
                    <FeatureIcon active={apiServerPort !== null}>
                      <LuPlug />
                    </FeatureIcon>
                    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <Label
                        htmlFor={apiSwitchId}
                        className="text-sm font-medium"
                      >
                        {t("integrations.apiEnableLabel")}
                      </Label>
                      <p className="text-xs leading-relaxed text-muted-foreground">
                        {t("integrations.apiEnableDescription")}
                      </p>
                    </div>
                    <AnimatedSwitch
                      id={apiSwitchId}
                      checked={apiServerPort !== null}
                      disabled={isApiStarting}
                      onCheckedChange={(checked) =>
                        void handleApiToggle(checked)
                      }
                    />
                  </div>
                  <AnimatedDisclosureContent open={apiServerPort !== null}>
                    <div className="flex flex-wrap items-center gap-2 pt-3 text-xs @md:pl-12">
                      <StatusLight tone="success" live />
                      <span className="text-muted-foreground">
                        {t("integrations.apiRunningOn")}
                      </span>
                      <CodeSnippet
                        code={`http://127.0.0.1:${apiServerPort ?? settings.api_port}`}
                        className="w-fit"
                      />
                    </div>
                  </AnimatedDisclosureContent>
                </SettingsPanel>

                <AnimatedDisclosureContent open={settings.api_enabled}>
                  <div className="flex flex-col gap-4 pt-3">
                    <SettingsPanel className="divide-y divide-foreground/6 py-3">
                      <SettingsRow
                        label={t("integrations.apiPortLabel")}
                        description={t("integrations.apiPortDescription")}
                        htmlFor={apiPortId}
                      >
                        <Input
                          id={apiPortId}
                          type="number"
                          variant="soft"
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
                          className="h-8 w-24 font-mono text-xs md:text-xs"
                          min={1}
                          max={65535}
                        />
                        <LoadingButton
                          size="sm"
                          variant="soft"
                          className="h-8 rounded-lg text-xs"
                          isLoading={isApiStarting}
                          disabled={apiServerPort === settings.api_port}
                          onClick={() => void handleSavePort()}
                        >
                          {t("common.buttons.save")}
                        </LoadingButton>
                      </SettingsRow>
                      <SettingsRow
                        label={t("integrations.apiTokenLabel")}
                        description={t("integrations.apiTokenDescription")}
                        htmlFor={apiTokenId}
                      >
                        <div className="relative w-full @xl:w-72">
                          <Input
                            id={apiTokenId}
                            type={showApiToken ? "text" : "password"}
                            variant="soft"
                            value={apiToken}
                            readOnly
                            className="h-8 pr-16 font-mono text-xs md:text-xs"
                          />
                          <div className="absolute inset-y-0 right-1 flex items-center gap-0.5">
                            <Button
                              type="button"
                              variant="subtle"
                              size="icon"
                              className="size-6 rounded-md"
                              aria-label={
                                showApiToken
                                  ? t("common.aria.hideToken")
                                  : t("common.aria.showToken")
                              }
                              onClick={() => {
                                setShowApiToken(!showApiToken);
                              }}
                            >
                              {showApiToken ? (
                                <LuEyeOff className="size-3.5" />
                              ) : (
                                <LuEye className="size-3.5" />
                              )}
                            </Button>
                            <CopyToClipboard
                              text={apiToken}
                              variant="subtle"
                              className="size-6 rounded-md"
                              successMessage={t("integrations.tokenCopied")}
                            />
                          </div>
                        </div>
                      </SettingsRow>
                    </SettingsPanel>

                    <section className="flex flex-col gap-2">
                      <div className="flex items-center justify-between gap-2">
                        <h3 className={SECTION_LABEL_CLASS}>
                          {t("integrations.apiExampleRequest")}
                        </h3>
                        {examplesButton(() => {
                          setApiExamplesOpen(true);
                        })}
                      </div>
                      <CodeSnippet
                        prompt
                        data-testid="integrations-api-example"
                        code={oneLine(
                          curlSnippet(apiQuickStart, apiDisplayTarget),
                        )}
                        copyText={oneLine(
                          curlSnippet(apiQuickStart, apiCopyTarget),
                        )}
                        highlights={[maskedApiToken]}
                        copyLabel={t("integrations.apiExampleRequest")}
                        successMessage={t("common.buttons.copied")}
                      />
                    </section>
                  </div>
                </AnimatedDisclosureContent>

                <IntegrationDiagnostics
                  key={`${apiServerPort}:${settings.api_token ?? ""}`}
                  target="api"
                  className="mt-4"
                />

                <CodeExamplesDialog
                  open={apiExamplesOpen}
                  onOpenChange={setApiExamplesOpen}
                  title={t("integrations.examples.apiTitle")}
                  description={t("integrations.examples.apiDescription")}
                  examples={apiExamples}
                  highlights={[maskedApiToken, PROFILE_ID_PLACEHOLDER]}
                  note={t("integrations.examples.tokenIncluded")}
                  copyMessage={t("common.buttons.copied")}
                />
              </AnimatedTabsContent>

              {remoteTabShown && (
                <AnimatedTabsContent value="remote" className="@container mt-4">
                  <SettingsPanel>
                    <div className="flex items-start gap-3">
                      <FeatureIcon active={remote?.connected ?? false}>
                        <LuCloud />
                      </FeatureIcon>
                      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                        <Label
                          htmlFor={remoteSwitchId}
                          className="text-sm font-medium"
                        >
                          {t("integrations.remote.enableLabel")}
                        </Label>
                        <p className="text-xs leading-relaxed text-muted-foreground">
                          {t("integrations.remote.enableDescription")}
                        </p>
                      </div>
                      <AnimatedSwitch
                        id={remoteSwitchId}
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
                      <p className="pt-3 text-xs text-warning-text @md:pl-12">
                        {t("integrations.remote.signInRequired")}
                      </p>
                    )}
                    {isLoggedIn && !termsAccepted && (
                      <p className="pt-3 text-xs text-warning-text @md:pl-12">
                        {t("integrations.mcpAcceptTermsFirst")}
                      </p>
                    )}

                    <AnimatedDisclosureContent open={remote?.enabled ?? false}>
                      {remote && (
                        <div className="flex flex-col gap-2 pt-3 @md:pl-12">
                          <div className="flex flex-wrap items-center gap-2 text-xs">
                            {/* Three states, not two. A refusal the bridge
                              will keep receiving (an unentitled plan, a taken
                              slot, a dead credential) re-dials for ever
                              without ever connecting, and "Connecting as"
                              over the top of the error underneath it read as
                              a hang rather than an answer. */}
                            <StatusLight
                              tone={
                                remote.connected
                                  ? "success"
                                  : remote.lastError
                                    ? "destructive"
                                    : "muted"
                              }
                              live={remote.connected || !remote.lastError}
                            />
                            <span className="text-muted-foreground">
                              {remote.connected
                                ? t("integrations.remote.connected")
                                : remote.lastError
                                  ? t("integrations.remote.notConnected")
                                  : t("integrations.remote.connecting")}
                            </span>
                            {/* Only under a phrase that governs it. "Connected
                              as" and "Connecting as" are open phrases; "Not
                              connected" is closed, and the id dangling after
                              it read as a sentence fragment in all ten
                              locales. */}
                            {!(remote.lastError && !remote.connected) && (
                              <code className="rounded-md bg-foreground/5 px-2 py-0.5 font-mono text-[11px]">
                                {remote.instanceId}
                              </code>
                            )}
                          </div>
                          {!remote.connected && remote.lastError && (
                            <p className="text-xs text-destructive-text">
                              {translateBackendError(t, remote.lastError)}
                            </p>
                          )}
                        </div>
                      )}
                    </AnimatedDisclosureContent>
                  </SettingsPanel>

                  {/* Gated on entitlement: handing an unentitled customer a
                    URL, a credential or an "Add" button that can only answer
                    402 is the same wrong-diagnosis trap as telling them their
                    desktop is offline. */}
                  <AnimatedDisclosureContent
                    open={
                      (remote?.enabled ?? false) &&
                      (remoteEntitled || !remoteEntitlementKnown)
                    }
                  >
                    <div className="flex flex-col gap-4 pt-3">
                      <SettingsPanel className="divide-y divide-foreground/6 py-3">
                        <SettingsRow
                          label={t("integrations.remote.credentialLabel")}
                          description={t("integrations.remote.credentialHint")}
                        >
                          {credentialPresent ? (
                            credentialPrefix && (
                              <code className="rounded-md bg-foreground/5 px-2 py-1 font-mono text-[11px]">
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
                          <LoadingButton
                            size="sm"
                            variant="soft"
                            className="h-8 shrink-0 gap-1.5 rounded-lg text-xs"
                            isLoading={isRotatingCredential}
                            onClick={() => void handleRotateCredential()}
                          >
                            {credentialPresent && (
                              <LuRefreshCw className="size-3.5" />
                            )}
                            {credentialPresent
                              ? t("integrations.remote.credentialRotate")
                              : t("integrations.remote.credentialCreate")}
                          </LoadingButton>
                        </SettingsRow>
                        <SettingsRow
                          label={t("integrations.remote.endpointLabel")}
                          description={t(
                            "integrations.remote.endpointDescription",
                          )}
                        >
                          <CodeSnippet
                            code={REMOTE_MCP_URL}
                            copyLabel={t("integrations.remote.endpointLabel")}
                            successMessage={t(
                              "integrations.remote.endpointCopied",
                            )}
                            className="w-full @xl:w-auto"
                          />
                        </SettingsRow>
                      </SettingsPanel>

                      <section className="flex flex-col gap-2">
                        <div className="flex items-center justify-between gap-2">
                          <h3 className={SECTION_LABEL_CLASS}>
                            {t("integrations.remote.exampleRequest")}
                          </h3>
                          {examplesButton(() => {
                            setRemoteExamplesOpen(true);
                          })}
                        </div>
                        <CodeSnippet
                          prompt
                          code={remoteQuickStart}
                          highlights={[exampleBearer]}
                          copyLabel={t("integrations.remote.exampleRequest")}
                          successMessage={t("common.buttons.copied")}
                        />
                      </section>

                      {clientsGrid}
                    </div>
                  </AnimatedDisclosureContent>

                  <IntegrationDiagnostics
                    key={`${credentialPrefixOf(credential)}:${remote?.enabled}`}
                    target="remote"
                    className="mt-4"
                  />

                  <CodeExamplesDialog
                    open={remoteExamplesOpen}
                    onOpenChange={setRemoteExamplesOpen}
                    title={t("integrations.examples.remoteTitle")}
                    description={t("integrations.examples.remoteDescription")}
                    examples={remoteExamples}
                    highlights={[exampleBearer]}
                    note={t("integrations.examples.credentialTemplate")}
                    copyMessage={t("common.buttons.copied")}
                  />
                </AnimatedTabsContent>
              )}
            </AnimatedTabs>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuCheck } from "react-icons/lu";
import { AgentIcon } from "@/components/mcp-agent-icon";
import { Button } from "@/components/ui/button";
import { CopyToClipboard } from "@/components/ui/copy-to-clipboard";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { OperationFlow } from "@/components/ui/operation-flow";
import { translateBackendError } from "@/lib/backend-errors";
import {
  credentialPrefixOf,
  FX_AGENT_ID,
  fxExportLine,
  localMcpClients,
  type McpAgentInfo,
  type McpRemoteCredential,
  type McpRemoteCredentialRotation,
  type McpRemoteStatus,
  REMOTE_MCP_URL,
} from "@/lib/mcp";
import { showErrorToast } from "@/lib/toast-utils";

type ClientResult =
  | { state: "moving" }
  | { state: "moved" }
  | { state: "failed"; message: string };

/** The stations of the move: remote MCP on, a credential, the clients. */
const STAGE_BRIDGE = 0;
const STAGE_CREDENTIAL = 1;
const STAGE_CLIENTS = 2;

interface SettingsSlice {
  mcp_enabled: boolean;
  mcp_remote_key?: string | null;
}

interface MigrationDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /**
   * Something on disk changed (clients moved, the local server turned off),
   * so the owner can reload what it shows.
   */
  onChanged?: () => void;
}

/**
 * Moves every client off the removed local MCP server to remote MCP, in the
 * app: remote MCP on, a credential, then each client rewritten, each with its
 * own result. The body mounts with the dialog, so every opening starts fresh.
 */
export function McpMigrationDialog({
  open,
  onOpenChange,
  onChanged,
}: MigrationDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        data-slot="mcp-migration"
        className="p-5 sm:max-w-lg sm:p-6"
      >
        <MigrationBody
          onClose={() => onOpenChange(false)}
          onChanged={onChanged}
        />
      </DialogContent>
    </Dialog>
  );
}

function MigrationBody({
  onClose,
  onChanged,
}: {
  onClose: () => void;
  onChanged?: () => void;
}) {
  const { t } = useTranslation();
  // Fixed when the dialog opens, so a moved client keeps its row.
  const [clients, setClients] = useState<McpAgentInfo[] | null>(null);
  const [results, setResults] = useState<Record<string, ClientResult>>({});
  const [bridgeOn, setBridgeOn] = useState(false);
  const [credential, setCredential] = useState<{
    prefix: string | null;
  } | null>(null);
  const [remoteKey, setRemoteKey] = useState<string | null>(null);
  const [localServerOn, setLocalServerOn] = useState(false);
  const [phase, setPhase] = useState<"intro" | "running" | "finished">("intro");
  const [stage, setStage] = useState(STAGE_BRIDGE);
  const [stageError, setStageError] = useState<string | null>(null);
  const [turningOff, setTurningOff] = useState(false);
  const [turnedOff, setTurnedOff] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const quiet = <T,>(read: Promise<T>) =>
      read.catch((error: unknown) => {
        console.error("Failed to read the remote MCP state:", error);
        return null;
      });
    void Promise.all([
      quiet(invoke<McpAgentInfo[]>("list_mcp_agents")),
      quiet(invoke<SettingsSlice>("get_app_settings")),
      quiet(invoke<McpRemoteStatus>("get_mcp_remote_status")),
      quiet(invoke<McpRemoteCredential>("get_mcp_remote_credential")),
    ]).then(([agents, settings, remote, stored]) => {
      if (cancelled) return;
      setClients(localMcpClients(agents ?? []));
      setLocalServerOn(settings?.mcp_enabled ?? false);
      setRemoteKey(settings?.mcp_remote_key ?? null);
      setBridgeOn(remote?.enabled ?? false);
      if (stored?.present)
        setCredential({ prefix: credentialPrefixOf(stored) });
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const setResult = (id: string, result: ClientResult) =>
    setResults((previous) => ({ ...previous, [id]: result }));

  /**
   * One pass of the move. A retry runs it again: the bridge and the
   * credential are skipped once they are in place, and a client that moved
   * is not written twice.
   */
  const run = async () => {
    if (!clients) return;
    setPhase("running");
    setStageError(null);
    try {
      if (!bridgeOn) {
        setStage(STAGE_BRIDGE);
        const status = await invoke<McpRemoteStatus>("start_mcp_remote_bridge");
        setBridgeOn(status.enabled);
      }
      if (!credential) {
        setStage(STAGE_CREDENTIAL);
        // Asked again rather than trusted from the opening: minting when a
        // key exists would retire the key other clients already carry.
        const stored = await invoke<McpRemoteCredential>(
          "get_mcp_remote_credential",
        );
        if (stored.present) {
          setCredential({ prefix: credentialPrefixOf(stored) });
        } else {
          const minted = await invoke<McpRemoteCredentialRotation>(
            "rotate_mcp_remote_credential",
          );
          setCredential({ prefix: credentialPrefixOf(minted) });
        }
      }
    } catch (error) {
      setStageError(translateBackendError(t, error));
      setPhase("finished");
      return;
    }

    setStage(STAGE_CLIENTS);
    for (const client of clients) {
      if (results[client.id]?.state === "moved") continue;
      setResult(client.id, { state: "moving" });
      try {
        await invoke("add_mcp_to_agent", {
          agentId: client.id,
          target: "remote",
        });
        setResult(client.id, { state: "moved" });
      } catch (error) {
        setResult(client.id, {
          state: "failed",
          message: translateBackendError(t, error),
        });
      }
    }

    // The plaintext behind the credential copy and the fx export line.
    try {
      const settings = await invoke<SettingsSlice>("get_app_settings");
      setRemoteKey(settings.mcp_remote_key ?? null);
      setLocalServerOn(settings.mcp_enabled);
    } catch (error) {
      console.error("Failed to reload the settings after the move:", error);
    }
    setPhase("finished");
    onChanged?.();
  };

  const turnOffLocalServer = async () => {
    setTurningOff(true);
    try {
      await invoke("turn_off_local_mcp_server");
      setLocalServerOn(false);
      setTurnedOff(true);
      onChanged?.();
    } catch (error) {
      showErrorToast(t("mcpMigration.localServerOffFailed"), {
        description: translateBackendError(t, error),
      });
    } finally {
      setTurningOff(false);
    }
  };

  const total = clients?.length ?? 0;
  const countIn = (state: ClientResult["state"]) =>
    clients?.filter((client) => results[client.id]?.state === state).length ??
    0;
  const moved = countIn("moved");
  const failed = countIn("failed");
  const running = phase === "running";
  const finished = phase === "finished";
  const succeeded = finished && stageError === null && failed === 0;
  const waiting = t("appFeedback.waiting");
  const fxLine = fxExportLine(remoteKey);

  const steps = [
    {
      id: "bridge",
      label: t("mcpMigration.stepBridge"),
      detail: bridgeOn
        ? t("mcpMigration.bridgeOn")
        : running && stage === STAGE_BRIDGE
          ? t("mcpMigration.bridgeWorking")
          : waiting,
    },
    {
      id: "credential",
      label: t("integrations.remote.credentialLabel"),
      detail: credential ? (
        credential.prefix ? (
          <span className="font-mono">
            {t("integrations.remote.credentialPrefix", {
              prefix: credential.prefix,
            })}
          </span>
        ) : (
          t("appFeedback.configured")
        )
      ) : running && stage === STAGE_CREDENTIAL ? (
        t("mcpMigration.credentialWorking")
      ) : (
        waiting
      ),
    },
    {
      id: "clients",
      label: t("mcpMigration.stepClients"),
      detail:
        total === 0
          ? t("mcpMigration.clientsNone")
          : t("mcpMigration.clientsProgress", { moved, total }),
    },
  ];

  return (
    <div className="flex min-w-0 flex-col gap-5">
      <DialogHeader>
        <DialogTitle>
          {succeeded ? t("mcpMigration.doneTitle") : t("mcpMigration.title")}
        </DialogTitle>
        {phase === "intro" && (
          <DialogDescription className="text-pretty">
            {t("mcpMigration.body")}
          </DialogDescription>
        )}
      </DialogHeader>

      {phase !== "intro" && (
        <OperationFlow
          label={t("mcpMigration.title")}
          steps={steps}
          active={finished && stageError === null ? STAGE_CLIENTS : stage}
          failed={finished && !succeeded}
          busy={running}
        />
      )}

      {clients !== null && (
        <div className="flex min-w-0 flex-col gap-2">
          {phase === "intro" && (
            <p className="text-sm text-pretty">
              {total > 0
                ? t("mcpMigration.changes")
                : t("mcpMigration.changesNone")}
            </p>
          )}
          {total > 0 && (
            <ul data-slot="mcp-migration-clients" className="flex flex-col">
              {clients.map((client) => {
                const result = results[client.id];
                return (
                  <li
                    key={client.id}
                    data-slot="mcp-migration-client"
                    data-agent-id={client.id}
                    data-state={result?.state ?? "waiting"}
                    className="flex min-w-0 items-start gap-3 py-1.5"
                  >
                    <AgentIcon id={client.id} category={client.category} />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium">
                        {client.display_name}
                      </p>
                      {result?.state === "failed" && (
                        <p className="text-xs break-words text-destructive-text">
                          {result.message}
                        </p>
                      )}
                      {client.id === FX_AGENT_ID &&
                        result?.state === "moved" && (
                          <div className="flex items-center gap-2">
                            <p className="min-w-0 flex-1 text-xs text-muted-foreground">
                              {t("integrations.remote.fxHint")}
                            </p>
                            {fxLine && (
                              <CopyToClipboard
                                variant="ghost"
                                className="size-7 shrink-0"
                                text={fxLine}
                                successMessage={t(
                                  "integrations.remote.fxExportCopied",
                                )}
                              />
                            )}
                          </div>
                        )}
                    </div>
                    {phase !== "intro" && (
                      <ClientStatus result={result} waiting={waiting} />
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      )}

      {/* Mounted from the start, so the outcome is announced when it lands. */}
      <div
        aria-live="polite"
        className="flex min-w-0 flex-col gap-3 empty:hidden"
      >
        {finished && stageError && (
          <p className="text-sm break-words text-destructive-text">
            {stageError}
          </p>
        )}
        {finished && stageError === null && (moved > 0 || failed > 0) && (
          <p data-slot="mcp-migration-summary" className="text-sm">
            {failed > 0
              ? t("mcpMigration.failed", { count: failed })
              : t("mcpMigration.summary", { count: moved })}
          </p>
        )}
        {finished && stageError === null && credential && (
          <div
            data-slot="mcp-migration-manual"
            className="flex min-w-0 flex-col gap-2 rounded-md bg-muted/50 px-3 py-2.5"
          >
            <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1">
              <dt className="text-xs text-muted-foreground">
                {t("integrations.remote.endpointLabel")}
              </dt>
              <dd className="font-mono text-xs break-all">{REMOTE_MCP_URL}</dd>
              <dd>
                <CopyToClipboard
                  variant="ghost"
                  className="size-7"
                  text={REMOTE_MCP_URL}
                  successMessage={t("integrations.remote.endpointCopied")}
                />
              </dd>
              {remoteKey && (
                <>
                  <dt className="text-xs text-muted-foreground">
                    {t("integrations.remote.credentialLabel")}
                  </dt>
                  <dd className="font-mono text-xs break-all">
                    {t("integrations.remote.credentialPrefix", {
                      prefix: credential.prefix ?? remoteKey.slice(0, 12),
                    })}
                  </dd>
                  <dd>
                    <CopyToClipboard
                      variant="ghost"
                      className="size-7"
                      text={remoteKey}
                      successMessage={t("mcpMigration.credentialCopied")}
                    />
                  </dd>
                </>
              )}
            </dl>
            <p className="text-xs text-pretty text-muted-foreground">
              {t("mcpMigration.manual")}
            </p>
          </div>
        )}
        {succeeded && (localServerOn || turnedOff) && (
          <div
            data-slot="mcp-migration-local"
            className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2"
          >
            {turnedOff ? (
              <p className="inline-flex items-center gap-1.5 text-xs font-medium">
                <LuCheck aria-hidden="true" className="size-3.5" />
                {t("mcpMigration.localServerIsOff")}
              </p>
            ) : (
              <>
                <p className="min-w-0 flex-1 text-xs text-pretty text-muted-foreground">
                  {t("mcpMigration.localServer")}
                </p>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  data-slot="mcp-migration-local-off"
                  disabled={turningOff}
                  onClick={() => void turnOffLocalServer()}
                >
                  {t("mcpMigration.localServerOff")}
                </Button>
              </>
            )}
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2">
        {(phase === "intro" || (finished && !succeeded)) && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            data-slot="mcp-migration-dismiss"
            className="text-muted-foreground hover:text-foreground"
            onClick={onClose}
          >
            {phase === "intro"
              ? t("mcpMigration.notNow")
              : t("common.buttons.close")}
          </Button>
        )}
        {succeeded ? (
          <Button
            type="button"
            size="sm"
            data-slot="mcp-migration-done"
            className="ml-auto"
            onClick={onClose}
          >
            {t("mcpMigration.done")}
          </Button>
        ) : (
          <Button
            type="button"
            size="sm"
            data-slot="mcp-migration-start"
            className="ml-auto"
            disabled={clients === null || running}
            onClick={() => void run()}
          >
            {finished ? t("common.buttons.retry") : t("mcpMigration.start")}
          </Button>
        )}
      </div>
    </div>
  );
}

function ClientStatus({
  result,
  waiting,
}: {
  result: ClientResult | undefined;
  waiting: string;
}) {
  const { t } = useTranslation();
  switch (result?.state) {
    case "moved":
      return (
        <span className="inline-flex shrink-0 items-center gap-1 text-xs leading-5 font-medium text-foreground">
          <LuCheck aria-hidden="true" className="size-3.5" />
          {t("mcpMigration.clientMoved")}
        </span>
      );
    case "failed":
      return (
        <span className="shrink-0 text-xs leading-5 text-destructive-text">
          {t("mcpMigration.clientFailed")}
        </span>
      );
    case "moving":
      return (
        <span className="shrink-0 text-xs leading-5 text-muted-foreground">
          {t("mcpMigration.clientMoving")}
        </span>
      );
    default:
      return (
        <span className="shrink-0 text-xs leading-5 text-muted-foreground">
          {waiting}
        </span>
      );
  }
}

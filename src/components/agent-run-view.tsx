"use client";

import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AgentStepList,
  agentCloseReasonLabel,
  agentStatusLabel,
  agentStatusTone,
} from "@/components/agent-shared";
import {
  formatElapsed,
  parseIso,
  StatusDot,
  useSecondTicker,
} from "@/components/cookie-bot-shared";
import { Button } from "@/components/ui/button";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { RippleButton } from "@/components/ui/ripple";
import { Skeleton } from "@/components/ui/skeleton";
import { useAgentRun } from "@/hooks/use-agent-run";
import {
  type AgentRunView as AgentRun,
  cancelAgentRun,
  isRunCancellable,
  isRunOver,
} from "@/lib/agent";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";

/** Seconds a run has been going, or ran for. Null when it never started. */
function elapsedSeconds(run: AgentRun | null, now: number): number | null {
  if (!run) return null;
  const started = parseIso(run.startedAt ?? run.createdAt);
  if (!started) return null;
  const ended = parseIso(run.endedAt);
  const end = ended ? ended.getTime() : now;
  return Math.max(0, Math.floor((end - started.getTime()) / 1000));
}

/**
 * The result the run reported, as text.
 *
 * A plain string is shown as written. Anything structured is pretty-printed
 * rather than rendered as `[object Object]`, and a result the server sent as
 * nothing at all shows nothing instead of the word "null".
 */
function resultText(result: unknown): string | null {
  if (result === null || result === undefined) return null;
  if (typeof result === "string") {
    return result.trim().length > 0 ? result : null;
  }
  try {
    return JSON.stringify(result, null, 2);
  } catch {
    return null;
  }
}

interface AgentRunViewProps {
  runId: string;
  profiles: BrowserProfile[];
  /** Shown as the panel's back action, when there is somewhere to go back to. */
  onBack?: () => void;
  backLabel?: string;
  /** Called after a cancel lands, so a history list can re-read its rows. */
  onChanged?: () => void;
}

export function AgentRunPanel({
  runId,
  profiles,
  onBack,
  backLabel,
  onChanged,
}: AgentRunViewProps) {
  const { t } = useTranslation();
  const { run, steps, isLoading, error, streamConnected, refresh } =
    useAgentRun(runId);
  const [isCancelling, setIsCancelling] = useState(false);

  const live = run !== null && !isRunOver(run);
  const now = useSecondTicker(live);
  const elapsed = elapsedSeconds(run, now);

  const profileName = useMemo(() => {
    if (!run) return null;
    return (
      profiles.find((profile) => profile.id === run.profileId)?.name ?? null
    );
  }, [profiles, run]);

  const handleCancel = useCallback(async () => {
    setIsCancelling(true);
    try {
      await cancelAgentRun(runId);
      showSuccessToast(t("agent.live.cancelled"));
      await refresh();
      onChanged?.();
    } catch (cancelError) {
      showErrorToast(translateBackendError(t, cancelError));
    } finally {
      setIsCancelling(false);
    }
  }, [runId, refresh, onChanged, t]);

  const closeReason = agentCloseReasonLabel(t, run?.closeReason);
  const result = resultText(run?.result);
  const tokens = (run?.tokensIn ?? 0) + (run?.tokensOut ?? 0);

  if (isLoading && run === null) {
    return (
      <div
        data-slot="agent-run-panel"
        className="flex min-h-0 flex-1 flex-col gap-3"
      >
        <Skeleton className="h-16 w-full" />
        {Array.from({ length: 5 }, (_, index) => (
          <Skeleton
            key={`agent-step-skeleton-${index}`}
            className="h-6 w-full"
          />
        ))}
      </div>
    );
  }

  if (run === null) {
    return (
      <div
        data-slot="agent-run-panel"
        className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 py-16 text-center"
      >
        <p className="text-sm text-muted-foreground">
          {error === null
            ? t("agent.live.notFound")
            : translateBackendError(t, error)}
        </p>
        <RippleButton
          variant="outline"
          size="sm"
          onClick={() => {
            void refresh();
          }}
        >
          {t("common.buttons.retry")}
        </RippleButton>
      </div>
    );
  }

  return (
    <div
      data-slot="agent-run-panel"
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      <div className="shrink-0 rounded-md border border-border bg-card p-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-1">
            <span className="flex items-center gap-2 text-sm">
              <StatusDot tone={agentStatusTone(run.status)} pulse={live} />
              <span className="font-medium text-foreground">
                {agentStatusLabel(t, run.status)}
              </span>
              <span className="text-muted-foreground">
                {t("agent.live.onProfile", {
                  profile: profileName ?? t("agent.history.unknownProfile"),
                  target: t(
                    run.target === "fleet"
                      ? "agent.target.fleet"
                      : "agent.target.desktop",
                  ),
                })}
              </span>
            </span>
            <p className="break-words text-xs text-muted-foreground">
              {run.goal}
            </p>
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {onBack && (
              <Button variant="outline" size="sm" onClick={onBack}>
                {backLabel ?? t("common.buttons.back")}
              </Button>
            )}
            {isRunCancellable(run) && (
              <RippleButton
                variant="destructive"
                size="sm"
                disabled={isCancelling}
                onClick={() => {
                  void handleCancel();
                }}
              >
                {isCancelling
                  ? t("agent.live.cancelling")
                  : t("agent.live.cancel")}
              </RippleButton>
            )}
          </div>
        </div>

        <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-2 text-xs @2xl:grid-cols-4">
          <Metric
            label={t("agent.live.elapsed")}
            value={elapsed === null ? "—" : formatElapsed(elapsed)}
          />
          <Metric
            label={t("agent.live.steps")}
            value={String(run.steps ?? steps.length)}
          />
          <Metric
            label={t("agent.live.tokens")}
            value={tokens > 0 ? tokens.toLocaleString() : "—"}
          />
          <Metric
            label={t("agent.live.cost")}
            value={
              typeof run.costUsd === "number"
                ? t("agent.live.costValue", { amount: run.costUsd.toFixed(4) })
                : "—"
            }
          />
        </dl>

        {closeReason !== null && (
          <p className="mt-3 text-xs text-muted-foreground">
            {t("agent.live.endedBecause", { reason: closeReason })}
          </p>
        )}
        {run.errorCode && (
          <p className="mt-1 text-xs text-destructive-text">
            {translateBackendError(t, JSON.stringify({ code: run.errorCode }))}
          </p>
        )}
        {live && !streamConnected && (
          <p className="mt-3 text-xs text-muted-foreground">
            {t("agent.live.streamOffline")}
          </p>
        )}
      </div>

      <FadingScrollArea className="min-h-0 flex-1">
        <div className="pr-1">
          <AgentStepList
            steps={steps}
            emptyLabel={
              live ? t("agent.live.waiting") : t("agent.live.noSteps")
            }
          />
          {result !== null && (
            <div className="mt-3 flex flex-col gap-1 rounded-md border border-border bg-card p-3">
              <span className="text-xs font-medium text-foreground">
                {t("agent.live.result")}
              </span>
              <pre className="overflow-x-auto whitespace-pre-wrap break-words text-xs text-muted-foreground">
                {result}
              </pre>
            </div>
          )}
        </div>
      </FadingScrollArea>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="tabular-nums text-foreground">{value}</dd>
    </div>
  );
}

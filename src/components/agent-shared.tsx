"use client";

import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { LuBot } from "react-icons/lu";
// The status dot, the wall clock and the timestamp formatters are the app's
// one implementation of each. They live beside the cookie bot because it needed
// them first; importing them is the point of them being exported.
import {
  formatDateTime,
  parseIso,
  StatusDot,
  type StatusTone,
} from "@/components/cookie-bot-shared";
import { ProBadge } from "@/components/ui/pro-badge";
import { type AgentStep, agentStepSummary } from "@/lib/agent";
import { cn } from "@/lib/utils";

/**
 * Why the agent cannot be used right now.
 *
 * Three genuinely different states, and collapsing them into one "unavailable"
 * would tell a signed-out user their plan is wrong and a Solo user to sign in.
 */
export type AgentUnavailableReason = "signIn" | "plan" | "notConfigured";

/**
 * What the page shows instead of a form it cannot honour.
 *
 * Never a disabled form: a control that exists to be pressed and cannot be is
 * worse than an explanation, because the user has to try it to find out.
 */
export function AgentUnavailable({
  reason,
  className,
}: {
  reason: AgentUnavailableReason;
  className?: string;
}) {
  const { t } = useTranslation();
  return (
    <div
      data-slot="agent-unavailable"
      className={cn(
        "flex min-h-0 flex-1 flex-col items-center justify-center gap-3 py-16 text-center",
        className,
      )}
    >
      <LuBot className="size-12 text-muted-foreground" />
      <div className="flex items-center gap-2">
        <p className="text-sm font-medium text-foreground">
          {t(`agent.unavailable.${reason}Title`)}
        </p>
        {reason === "plan" && <ProBadge />}
      </div>
      <p className="max-w-md text-xs text-muted-foreground">
        {t(`agent.unavailable.${reason}Hint`)}
      </p>
    </div>
  );
}

/** The tone a run's status paints with. Mirrors the app's one dot vocabulary. */
export function agentStatusTone(status: string): StatusTone {
  switch (status) {
    case "succeeded":
      return "success";
    case "running":
      return "live";
    case "queued":
      return "muted";
    case "cancelled":
      return "warning";
    case "failed":
      return "destructive";
    default:
      return "muted";
  }
}

const KNOWN_STATUSES = new Set([
  "queued",
  "running",
  "succeeded",
  "failed",
  "cancelled",
]);

/**
 * A run's status as a person reads it.
 *
 * A status this release has never heard of renders as "unknown" rather than as
 * the server's own word: the state machine is the backend's and it ships in
 * English, which is exactly what the translation rule exists to keep off screen.
 */
export function agentStatusLabel(t: TFunction, status: string): string {
  return KNOWN_STATUSES.has(status)
    ? t(`agent.status.${status}`)
    : t("agent.status.unknown");
}

const CLOSE_REASONS: Record<string, string> = {
  completed: "completed",
  budget: "budget",
  cancelled: "cancelled",
  navigation_blocked: "navigationBlocked",
  target_offline: "targetOffline",
  target_error: "targetError",
  model_error: "modelError",
  model_refused: "modelRefused",
  model_stopped: "modelStopped",
  context_exceeded: "contextExceeded",
  orphaned: "orphaned",
  internal_error: "internalError",
};

/**
 * Why a run ended, or null when the server named a reason this release has no
 * words for. Rendering nothing beats rendering `target_offline` at a customer.
 */
export function agentCloseReasonLabel(
  t: TFunction,
  reason: string | null | undefined,
): string | null {
  if (!reason) return null;
  const key = CLOSE_REASONS[reason];
  return key ? t(`agent.closeReason.${key}`) : null;
}

const KNOWN_STEP_KINDS = new Set(["thought", "tool", "error", "finish"]);

export function agentStepKindLabel(t: TFunction, kind: string): string {
  return KNOWN_STEP_KINDS.has(kind)
    ? t(`agent.step.${kind}`)
    : t("agent.step.unknown");
}

function stepTone(kind: string): StatusTone {
  switch (kind) {
    case "error":
      return "destructive";
    case "finish":
      return "success";
    case "tool":
      return "live";
    default:
      return "muted";
  }
}

/**
 * A run's transcript.
 *
 * Rendered from whatever steps are held right now, with no entrance animation
 * and no reveal to wait on: the list is filled by an HTTP read before the
 * stream says anything, so a page opened mid-run shows what already happened
 * instead of an empty panel.
 */
export function AgentStepList({
  steps,
  emptyLabel,
}: {
  steps: AgentStep[];
  emptyLabel: string;
}) {
  const { t } = useTranslation();

  if (steps.length === 0) {
    return (
      <p
        data-slot="agent-transcript-empty"
        className="py-8 text-center text-sm text-muted-foreground"
      >
        {emptyLabel}
      </p>
    );
  }

  return (
    <ol data-slot="agent-transcript" className="flex flex-col gap-0.5">
      {steps.map((step) => {
        const summary = agentStepSummary(step);
        const at = parseIso(step.at) ? formatDateTime(step.at) : null;
        return (
          <li
            key={step.index}
            className="flex items-start gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted/30"
          >
            <span className="mt-1 flex shrink-0 items-center gap-2">
              <StatusDot tone={stepTone(step.kind)} />
              <span className="w-6 text-right tabular-nums text-muted-foreground">
                {step.index + 1}
              </span>
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="font-medium text-foreground">
                {agentStepKindLabel(t, step.kind)}
              </span>
              {summary !== null && (
                <span className="break-words text-muted-foreground">
                  {summary}
                </span>
              )}
            </span>
            {at !== null && (
              <span className="shrink-0 tabular-nums text-muted-foreground">
                {at}
              </span>
            )}
          </li>
        );
      })}
    </ol>
  );
}

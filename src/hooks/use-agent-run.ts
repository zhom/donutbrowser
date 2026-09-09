"use client";

import { useCallback, useEffect, useId, useRef, useState } from "react";
import {
  type AgentRunView,
  type AgentStep,
  getAgentRun,
  isRunOver,
  onAgentStatus,
  onAgentStep,
  onAgentStream,
  startAgentRunEvents,
  stopAgentRunEvents,
} from "@/lib/agent";

/**
 * One run, watched.
 *
 * Two sources feed the same list and they overlap on purpose. The transcript is
 * READ FIRST, over HTTP, so a page opened halfway through a run shows the steps
 * that already happened instead of an empty panel waiting for the next one. The
 * stream then replays the same transcript and continues live, and both are
 * merged by step index — the index is the server's own ordinal, so a replayed
 * step and its live twin collapse into one row rather than doubling it.
 */
export interface AgentRunSnapshot {
  /** The run as last described, or null before the first read. */
  run: AgentRunView | null;
  /** Everything it has done, in server order. */
  steps: AgentStep[];
  /** True only while the first read of a newly watched run is in flight. */
  isLoading: boolean;
  /** The last read failure, as a backend error envelope. */
  error: unknown;
  /** Whether steps are currently arriving. */
  streamConnected: boolean;
}

export interface UseAgentRunResult extends AgentRunSnapshot {
  /** Re-read the run and its transcript. */
  refresh: () => Promise<void>;
}

const EMPTY: AgentRunSnapshot = {
  run: null,
  steps: [],
  isLoading: false,
  error: null,
  streamConnected: false,
};

/**
 * Which run each mounted panel wants streamed, coalesced into one decision.
 *
 * The desktop streams a single run, and React runs an unmounting panel's
 * cleanup and the next panel's effect back to back. Sending a stop and a start
 * as two separate commands makes that swap a race: they are handled on the Rust
 * side as independent tasks, so a stop landing second kills the stream the new
 * panel just asked for and the run sits under "steps are not arriving" while
 * nothing reconnects. Coalescing into a microtask means only the settled wish
 * is ever sent — and `start_agent_run_events` already replaces whatever it was
 * watching, so switching runs is one command, not two.
 */
const wishes = new Map<string, string>();
let applied: string | null = null;
let applyScheduled = false;

function applyWishes() {
  let next: string | null = null;
  for (const wish of wishes.values()) next = wish;
  if (next === applied) return;
  applied = next;
  if (next === null) {
    void stopAgentRunEvents().catch((error: unknown) => {
      console.error("Failed to stop the agent step stream:", error);
    });
    return;
  }
  void startAgentRunEvents(next).catch((error: unknown) => {
    // A signed-out or unentitled desktop refuses the subscription. The steps
    // already read over HTTP still render, so this is logged, not shown.
    console.error("Failed to subscribe to agent run events:", error);
  });
}

function scheduleApplyWishes() {
  if (applyScheduled) return;
  applyScheduled = true;
  queueMicrotask(() => {
    applyScheduled = false;
    applyWishes();
  });
}

function mergeStep(steps: AgentStep[], incoming: AgentStep): AgentStep[] {
  const next = steps.filter((step) => step.index !== incoming.index);
  next.push(incoming);
  next.sort((a, b) => a.index - b.index);
  return next;
}

/**
 * @param runId the run to watch, or null to watch nothing. Changing it swaps
 *   the whole subscription: the desktop streams one run at a time, because the
 *   page only ever shows one.
 */
export function useAgentRun(runId: string | null): UseAgentRunResult {
  const [snapshot, setSnapshot] = useState<AgentRunSnapshot>(EMPTY);
  const consumerId = useId();
  // Every async continuation checks this before it writes. React runs mount
  // effects twice in development and a user can switch runs faster than a read
  // completes, so without it a stale transcript lands on top of a newer one.
  const generation = useRef(0);

  const load = useCallback(
    async (id: string, forGeneration: number, withSpinner: boolean) => {
      if (withSpinner) {
        setSnapshot((prev) => ({ ...prev, isLoading: true, error: null }));
      }
      try {
        const detail = await getAgentRun(id);
        if (generation.current !== forGeneration) return;
        const { transcript, ...run } = detail;
        setSnapshot((prev) => ({
          ...prev,
          run,
          // The transcript is authoritative for what has already happened.
          // Live steps that arrived while this read was in flight keep their
          // place, because merging is by index either way.
          steps: (transcript ?? []).reduce(mergeStep, prev.steps),
          isLoading: false,
          error: null,
        }));
      } catch (error) {
        if (generation.current !== forGeneration) return;
        // A background refresh that fails leaves the previous steps on screen:
        // a run's history does not stop being true because one read did.
        setSnapshot((prev) => ({
          ...prev,
          isLoading: false,
          error: withSpinner ? error : prev.error,
        }));
      }
    },
    [],
  );

  useEffect(() => {
    const forGeneration = ++generation.current;
    if (!runId) {
      setSnapshot(EMPTY);
      wishes.delete(consumerId);
      scheduleApplyWishes();
      return;
    }

    setSnapshot({ ...EMPTY, isLoading: true });

    const offs: Promise<() => void>[] = [
      onAgentStep((event) => {
        if (generation.current !== forGeneration || event.runId !== runId) {
          return;
        }
        if (!event.step || typeof event.step.index !== "number") return;
        setSnapshot((prev) => ({
          ...prev,
          steps: mergeStep(prev.steps, event.step),
        }));
      }),
      onAgentStatus((event) => {
        if (generation.current !== forGeneration || event.runId !== runId) {
          return;
        }
        setSnapshot((prev) => ({
          ...prev,
          run: prev.run
            ? {
                ...prev.run,
                status: event.status,
                closeReason: event.closeReason ?? prev.run.closeReason,
                errorCode: event.errorCode ?? prev.run.errorCode,
                result: event.result ?? prev.run.result,
              }
            : prev.run,
        }));
        // A finished run has final counters — tokens, cost, ended_at — that the
        // status frame does not carry, so the view is re-read once rather than
        // left showing the figures it had a second before the end.
        if (isRunOver({ status: event.status })) {
          void load(runId, forGeneration, false);
        }
      }),
      onAgentStream((status) => {
        if (generation.current !== forGeneration || status.runId !== runId) {
          return;
        }
        setSnapshot((prev) => ({ ...prev, streamConnected: status.connected }));
      }),
    ];

    // The transcript read and the subscription start together. The read is what
    // fills the panel; the stream is what keeps it filling.
    void load(runId, forGeneration, true);
    wishes.set(consumerId, runId);
    scheduleApplyWishes();

    return () => {
      generation.current += 1;
      for (const off of offs) {
        void off.then((unlisten) => {
          unlisten();
        });
      }
      wishes.delete(consumerId);
      scheduleApplyWishes();
    };
  }, [runId, load, consumerId]);

  const refresh = useCallback(async () => {
    if (!runId) return;
    await load(runId, generation.current, true);
  }, [runId, load]);

  return { ...snapshot, refresh };
}

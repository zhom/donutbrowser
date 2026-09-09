"use client";

import { motion, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";
import { useInputModality } from "@/hooks/use-input-modality";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { cn } from "@/lib/utils";

export interface OperationStep {
  id: string;
  label: string;
  detail: ReactNode;
}

type NodeState = "done" | "active" | "failed" | "pending";

/**
 * A measured relationship or operation as a row of stations.
 *
 * Every station before the current one is settled and wears a check; the
 * current one is a ring, or a cross when the operation failed there; the
 * ones after it wait as small dots. The wire between stations fills as they
 * settle, and while the operation is busy a pulse travels the wire into the
 * station being worked on. Reaching the last station with nothing failed
 * settles the whole row.
 */
export function OperationFlow({
  steps,
  active,
  failed = false,
  busy = false,
  label,
}: {
  steps: OperationStep[];
  /** The station the operation is at. */
  active: number;
  /** The operation stopped at `active`. */
  failed?: boolean;
  /** The operation is still working towards `active`. */
  busy?: boolean;
  label: string;
}) {
  const reduced = useReducedMotion();
  const modality = useInputModality();
  const animate = !reduced && modality !== "keyboard";
  const last = steps.length - 1;
  const current = Math.max(0, Math.min(last, active));
  if (steps.length < 2) return null;

  const complete = !busy && !failed && current === last;
  const stateOf = (index: number): NodeState => {
    if (complete || index < current) return "done";
    if (index === current) return failed ? "failed" : "active";
    return "pending";
  };
  const at = (index: number) => `${(index / last) * 100}%`;
  const settle = { duration: animate ? 0.35 : 0, ease: MOTION_EASE_OUT };

  return (
    <div
      data-slot="operation-flow"
      data-state={
        failed ? "failed" : busy ? "busy" : complete ? "done" : "active"
      }
      role="group"
      aria-label={label}
      className="min-w-0 py-2"
    >
      <div
        data-slot="operation-track"
        aria-hidden="true"
        className="relative mb-3 h-4"
        style={{ marginInline: `${50 / steps.length}%` }}
      >
        {steps.slice(0, -1).map((step, index) => {
          // The wire into a station fills once that station is settled.
          const filled =
            complete ||
            index + 1 < current ||
            (index + 1 === current && !busy && !failed);
          const pulsing = busy && index === current - 1;
          return (
            <span
              key={step.id}
              data-slot="operation-wire"
              data-filled={filled}
              className="absolute top-1/2 h-0.5 -translate-y-1/2 rounded-full bg-border"
              style={{ left: at(index), width: `${100 / last}%` }}
            >
              <motion.span
                className="absolute inset-y-0 left-0 rounded-full bg-foreground"
                initial={false}
                animate={{ width: filled ? "100%" : "0%" }}
                transition={settle}
              />
              {pulsing && (
                <motion.span
                  data-slot="operation-pulse"
                  className="absolute top-1/2 size-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-foreground"
                  initial={false}
                  animate={{ left: animate ? ["0%", "100%"] : "100%" }}
                  transition={
                    animate
                      ? {
                          duration: 1.1,
                          repeat: Number.POSITIVE_INFINITY,
                          ease: "easeInOut",
                        }
                      : { duration: 0 }
                  }
                />
              )}
            </span>
          );
        })}
        {steps.map((step, index) => {
          const state = stateOf(index);
          return (
            <span
              key={step.id}
              data-slot={
                index === current ? "operation-marker" : "operation-node"
              }
              data-node-state={state}
              className={cn(
                "absolute top-1/2 grid -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full",
                state === "pending"
                  ? "size-1.5 bg-muted-foreground"
                  : "size-3.5",
                state === "done" && "bg-foreground text-background",
                state === "active" &&
                  "border-2 border-foreground bg-background",
                state === "failed" &&
                  "border-2 border-destructive bg-background text-destructive-text",
              )}
              style={{ left: at(index) }}
            >
              {state === "done" && (
                <svg
                  aria-hidden="true"
                  viewBox="0 0 10 10"
                  className="size-2"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth={2}
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <motion.path
                    d="M2 5.2 L4.2 7.4 L8 3"
                    initial={animate ? { pathLength: 0 } : false}
                    animate={{ pathLength: 1 }}
                    transition={{
                      duration: animate ? 0.25 : 0,
                      ease: "easeOut",
                    }}
                  />
                </svg>
              )}
              {state === "failed" && (
                <svg
                  aria-hidden="true"
                  viewBox="0 0 10 10"
                  className="size-2"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth={2}
                  strokeLinecap="round"
                >
                  <motion.path
                    d="M2.5 2.5 L7.5 7.5 M7.5 2.5 L2.5 7.5"
                    initial={animate ? { pathLength: 0 } : false}
                    animate={{ pathLength: 1 }}
                    transition={{
                      duration: animate ? 0.25 : 0,
                      ease: "easeOut",
                    }}
                  />
                </svg>
              )}
              {state === "active" && (
                <span className="size-1 rounded-full bg-foreground" />
              )}
            </span>
          );
        })}
      </div>
      <ol
        className="grid"
        style={{
          gridTemplateColumns: `repeat(${steps.length}, minmax(0, 1fr))`,
        }}
      >
        {steps.map((step, index) => {
          const state = stateOf(index);
          return (
            <li
              key={step.id}
              aria-current={index === current ? "step" : undefined}
              data-node-state={state}
              className="min-w-0 px-1.5 text-center"
            >
              <p
                className={cn(
                  "break-words text-xs font-medium transition-colors duration-200",
                  state === "pending"
                    ? "text-muted-foreground"
                    : "text-foreground",
                  state === "failed" && "text-destructive-text",
                )}
              >
                {step.label}
              </p>
              <div className="mt-1 break-words text-xs leading-relaxed text-muted-foreground">
                {step.detail}
              </div>
            </li>
          );
        })}
      </ol>
    </div>
  );
}

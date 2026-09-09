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

/** A measured relationship or operation, with labels independent of its marker. */
export function OperationFlow({
  steps,
  active,
  failed = false,
  label,
}: {
  steps: OperationStep[];
  active: number;
  failed?: boolean;
  label: string;
}) {
  const reduced = useReducedMotion();
  const modality = useInputModality();
  const current = Math.max(0, Math.min(steps.length - 1, active));
  if (steps.length < 2) return null;

  return (
    <div
      data-slot="operation-flow"
      role="group"
      aria-label={label}
      className="min-w-0 py-2"
    >
      <div
        data-slot="operation-track"
        aria-hidden="true"
        className="relative mb-3 h-3"
        style={{ marginInline: `${50 / steps.length}%` }}
      >
        <span className="absolute inset-x-0 top-[5px] h-0.5 rounded-full bg-border" />
        {steps.map((step, index) => (
          <span
            key={step.id}
            className="absolute top-1 size-1 -translate-x-1/2 rounded-full bg-muted-foreground"
            style={{ left: `${(index / (steps.length - 1)) * 100}%` }}
          />
        ))}
        <motion.span
          data-slot="operation-marker"
          initial={false}
          animate={{ left: `${(current / (steps.length - 1)) * 100}%` }}
          transition={{
            duration: reduced || modality === "keyboard" ? 0 : 0.22,
            ease: MOTION_EASE_OUT,
          }}
          className={cn(
            "absolute top-0 size-3 -translate-x-1/2 rounded-full bg-foreground",
            failed && "bg-destructive",
          )}
        />
      </div>
      <ol
        className="grid"
        style={{
          gridTemplateColumns: `repeat(${steps.length}, minmax(0, 1fr))`,
        }}
      >
        {steps.map((step, index) => (
          <li
            key={step.id}
            aria-current={index === current ? "step" : undefined}
            className="min-w-0 px-1.5 text-center"
          >
            <p className="break-words text-xs font-medium">{step.label}</p>
            <div className="mt-1 break-words text-xs leading-relaxed text-muted-foreground">
              {step.detail}
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}

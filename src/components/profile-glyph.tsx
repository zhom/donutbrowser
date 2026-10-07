"use client";

import { motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useState } from "react";
import { LuFlame, LuLock } from "react-icons/lu";
import { cn } from "@/lib/utils";

const SIZE = 56;
const CENTER = SIZE / 2;
const CORE_Y = 25;
const BOUND = 21;
const RIDGES = [2.5, 6.75, 11, 15.25, 19.5];
const ORBIT = 25.5;
/** How often the next ridge re-forms while the profile is created. */
const BUSY_STEP_MS = 140;

function hash(text: string) {
  let value = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    value ^= text.charCodeAt(i);
    value = Math.imul(value, 0x01000193);
  }
  return value >>> 0;
}

function random(seed: number) {
  let state = seed || 1;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const round = (n: number) => Number(n.toFixed(2));

/** One loop of the print: an arch over the core whose legs run down to the
 * print's round edge, so the inner loops reach further down. */
function ridgePath(radius: number) {
  const end = CENTER + Math.sqrt(BOUND * BOUND - radius * radius);
  const drop = end - CORE_Y;
  const flare = radius < RIDGES[RIDGES.length - 1] ? 1 : 0;
  const left = CENTER - radius;
  const right = CENTER + radius;
  return [
    `M${round(left - flare)} ${round(end)}`,
    `C${round(left - flare)} ${round(end - drop * 0.5)} ${left} ${round(CORE_Y + drop * 0.3)} ${left} ${CORE_Y}`,
    `A${radius} ${radius} 0 0 1 ${right} ${CORE_Y}`,
    `C${right} ${round(CORE_Y + drop * 0.3)} ${round(right + flare)} ${round(end - drop * 0.5)} ${round(right + flare)} ${round(end)}`,
  ].join("");
}

const PATHS = RIDGES.map(ridgePath);

interface Segment {
  id: "head" | "tail";
  offset: number;
  length: number;
}

/**
 * Where each ridge breaks and ends, drawn from the name. `step` counts the
 * ridges re-formed while the profile is created, core first; 0 is the name's
 * own print.
 */
function breaks(seed: string, step: number): Segment[][] {
  const base = seed.trim().toLowerCase();
  return RIDGES.map((_, ridge) => {
    const round =
      step > ridge ? Math.floor((step - 1 - ridge) / RIDGES.length) + 1 : 0;
    const next = random(hash(`${base}#${ridge}#${round}`));
    const start = next() * 0.1;
    const stop = 0.9 + next() * 0.1;
    const middle = 0.25 + next() * 0.5;
    const half = 0.025 + next() * 0.03;
    return [
      { id: "head", offset: start, length: middle - half - start },
      { id: "tail", offset: middle + half, length: stop - middle - half },
    ];
  });
}

/**
 * The new profile's mark in the creation dialog: a fingerprint whose ridges
 * break where the name says, so they shift as it is typed. A proxy puts a dot
 * in orbit, password protection or ephemeral mode add their badge, and while
 * the profile is created the ridges re-form one by one from the core outward,
 * with the same spring as typing. It is decoration only: the form states every
 * one of these in words.
 */
export function ProfileGlyph({
  seed,
  routed,
  locked,
  ephemeral,
  busy = false,
  className,
}: {
  seed: string;
  routed: boolean;
  locked: boolean;
  ephemeral: boolean;
  busy?: boolean;
  className?: string;
}) {
  const reduceMotion = useReducedMotion() ?? false;
  const [step, setStep] = useState(0);
  useEffect(() => {
    setStep(0);
    if (!busy || reduceMotion) return;
    const timer = window.setInterval(
      () => setStep((current) => current + 1),
      BUSY_STEP_MS,
    );
    return () => window.clearInterval(timer);
  }, [busy, reduceMotion]);
  const segments = useMemo(() => breaks(seed, step), [seed, step]);
  const empty = seed.trim().length === 0;
  const tone = `var(--chart-${(hash(seed.trim().toLowerCase()) % 5) + 1})`;
  const spring = reduceMotion
    ? { duration: 0 }
    : { type: "spring" as const, stiffness: 140, damping: 18, mass: 0.8 };
  const pop = reduceMotion
    ? { duration: 0 }
    : { type: "spring" as const, stiffness: 520, damping: 24 };

  return (
    <span
      aria-hidden="true"
      className={cn("relative inline-grid size-11 shrink-0", className)}
    >
      <svg
        aria-hidden="true"
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        className="size-full transition-[color] duration-500"
        style={{
          color: empty
            ? "var(--muted-foreground)"
            : `color-mix(in oklab, ${tone} 70%, var(--foreground))`,
        }}
      >
        <circle
          cx={CENTER}
          cy={CENTER}
          r={CENTER}
          className="fill-foreground/5"
        />
        <motion.g
          initial={false}
          animate={{ opacity: ephemeral ? 0.45 : 1 }}
          transition={{ duration: reduceMotion ? 0 : 0.3 }}
        >
          <g transform={`rotate(-10 ${CENTER} ${CENTER})`}>
            {PATHS.map((path, ridge) => (
              <g key={path} opacity={1 - ridge * 0.12}>
                {segments[ridge].map((segment) => (
                  <motion.path
                    key={segment.id}
                    d={path}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth={2}
                    strokeLinecap="round"
                    initial={
                      reduceMotion
                        ? false
                        : { pathOffset: segment.offset, pathLength: 0 }
                    }
                    animate={{
                      pathOffset: segment.offset,
                      pathLength: segment.length,
                    }}
                    transition={spring}
                  />
                ))}
              </g>
            ))}
          </g>
        </motion.g>
        <g className="origin-center [transform-box:view-box] motion-safe:animate-[spin_7s_linear_infinite]">
          <motion.circle
            cx={CENTER}
            cy={CENTER - ORBIT}
            r={2}
            className="fill-primary"
            initial={false}
            animate={{ scale: routed ? 1 : 0 }}
            transition={pop}
          />
        </g>
      </svg>
      <motion.span
        className="absolute -right-0.5 -bottom-0.5 grid size-5 place-items-center rounded-full bg-background"
        initial={false}
        animate={{ scale: locked || ephemeral ? 1 : 0 }}
        transition={pop}
      >
        <span className="grid size-4 place-items-center rounded-full bg-foreground text-background">
          {ephemeral ? (
            <LuFlame className="size-2.5" />
          ) : (
            <LuLock className="size-2.5" />
          )}
        </span>
      </motion.span>
    </span>
  );
}

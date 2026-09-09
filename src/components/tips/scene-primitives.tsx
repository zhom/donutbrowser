"use client";

import {
  motion,
  type TargetAndTransition,
  type Transition,
  useReducedMotion,
} from "motion/react";
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export const VIEW_W = 320;
export const VIEW_H = 160;

/**
 * Shared timing for one looping scene. Every element in a scene keys off the
 * same duration and its own `times`, so the parts stay in step without any
 * orchestration. With reduced motion a scene shows its resting frame: the
 * last keyframe of every value, no loop.
 */
export function useScene(duration: number) {
  const reduce = useReducedMotion() ?? false;
  const kf = <T,>(values: T[]): T | T[] =>
    reduce ? (values[values.length - 1] as T) : values;
  const tr = (times: number[], extra?: Transition): Transition =>
    reduce
      ? { duration: 0 }
      : {
          duration,
          times,
          repeat: Number.POSITIVE_INFINITY,
          ease: "easeInOut",
          ...extra,
        };
  return { reduce, kf, tr };
}

export interface Point {
  x: number;
  y: number;
}

/** Points along a cubic bezier, so a dot can travel a drawn wire. */
export function bezier(
  p0: Point,
  p1: Point,
  p2: Point,
  p3: Point,
  steps: number,
): Point[] {
  const out: Point[] = [];
  for (let i = 0; i <= steps; i += 1) {
    const t = i / steps;
    const mt = 1 - t;
    out.push({
      x:
        mt ** 3 * p0.x +
        3 * mt ** 2 * t * p1.x +
        3 * mt * t ** 2 * p2.x +
        t ** 3 * p3.x,
      y:
        mt ** 3 * p0.y +
        3 * mt ** 2 * t * p1.y +
        3 * mt * t ** 2 * p2.y +
        t ** 3 * p3.y,
    });
  }
  return out;
}

/**
 * Keyframes that hold at the first point, travel through every point between
 * `from` and `to` (as fractions of the loop), then hold at the last point.
 */
export function travel(points: Point[], from: number, to: number) {
  const last = points.length - 1;
  const times = [
    0,
    ...points.map((_, index) => from + ((to - from) * index) / last),
    1,
  ];
  return {
    cx: [points[0].x, ...points.map((p) => p.x), points[last].x],
    cy: [points[0].y, ...points.map((p) => p.y), points[last].y],
    times,
  };
}

export function Scene({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <svg
      data-slot="tip-scene"
      viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
      className={cn("h-full w-full text-muted-foreground", className)}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

/** A browser window: a frame, a title bar with one tab, and some text lines. */
export function Window({
  x,
  y,
  width,
  height,
  lines = 3,
  className,
  children,
}: {
  x: number;
  y: number;
  width: number;
  height: number;
  lines?: number;
  className?: string;
  children?: ReactNode;
}) {
  const barY = y + 14;
  return (
    <g className={className}>
      <rect x={x} y={y} width={width} height={height} rx={6} />
      <path d={`M${x} ${barY} H${x + width}`} />
      <rect
        x={x + 6}
        y={y + 4}
        width={Math.min(24, width / 3)}
        height={6}
        rx={2}
      />
      {Array.from({ length: lines }, (_, index) => {
        const lineY = barY + 14 + index * 12;
        const lineWidth = (width - 24) * (index % 2 === 0 ? 0.8 : 0.55);
        return lineY < y + height - 8 ? (
          <path
            key={index}
            d={`M${x + 12} ${lineY} h${lineWidth}`}
            strokeWidth={2}
          />
        ) : null;
      })}
      {children}
    </g>
  );
}

export function Laptop({ x, y }: { x: number; y: number }) {
  return (
    <g>
      <rect x={x} y={y} width={56} height={36} rx={3} />
      <path d={`M${x - 6} ${y + 41} H${x + 62}`} strokeWidth={2} />
    </g>
  );
}

export function Monitor({ x, y }: { x: number; y: number }) {
  return (
    <g>
      <rect x={x} y={y} width={56} height={40} rx={3} />
      <path d={`M${x + 28} ${y + 40} v8 M${x + 18} ${y + 48} h20`} />
    </g>
  );
}

export function Server({ x, y }: { x: number; y: number }) {
  return (
    <g>
      <rect x={x} y={y} width={40} height={56} rx={3} />
      <path d={`M${x} ${y + 19} h40 M${x} ${y + 37} h40`} />
      {[9, 28, 47].map((offset) => (
        <circle
          key={offset}
          cx={x + 32}
          cy={y + offset}
          r={1.5}
          fill="currentColor"
          stroke="none"
        />
      ))}
    </g>
  );
}

export function Person({ cx, cy }: { cx: number; cy: number }) {
  return (
    <g>
      <circle cx={cx} cy={cy - 9} r={6} />
      <path d={`M${cx - 12} ${cy + 10} a12 12 0 0 1 24 0`} />
    </g>
  );
}

export function Cloud({ cx, cy }: { cx: number; cy: number }) {
  return (
    <path
      d={`M${cx - 22} ${cy + 8} a9 9 0 0 1 2 -17 a14 14 0 0 1 27 -5 a10 10 0 0 1 10 22 z`}
    />
  );
}

export function Globe({ cx, cy, r }: { cx: number; cy: number; r: number }) {
  return (
    <g>
      <circle cx={cx} cy={cy} r={r} />
      <ellipse cx={cx} cy={cy} rx={r * 0.42} ry={r} />
      <path d={`M${cx - r} ${cy} h${r * 2}`} />
      <path
        d={`M${cx - r * 0.86} ${cy - r * 0.5} q${r * 0.86} ${r * 0.28} ${r * 1.72} 0`}
      />
    </g>
  );
}

export function Bin({ x, y }: { x: number; y: number }) {
  return (
    <path
      d={`M${x} ${y} h22 M${x + 3} ${y} v17 a2 2 0 0 0 2 2 h12 a2 2 0 0 0 2 -2 v-17 M${x + 8} ${y} v-3 h6 v3 M${x + 8} ${y + 5} v9 M${x + 14} ${y + 5} v9`}
    />
  );
}

/** A padlock; the shackle is its own element so a scene can lift it. */
export function Padlock({
  x,
  y,
  shackle,
  className,
}: {
  x: number;
  y: number;
  /** Motion props for the shackle group. */
  shackle?: { animate: TargetAndTransition; transition: Transition };
  className?: string;
}) {
  return (
    <g className={className}>
      <rect x={x} y={y} width={16} height={12} rx={2} />
      <circle cx={x + 8} cy={y + 6} r={1.5} fill="currentColor" stroke="none" />
      <motion.path
        d={`M${x + 4} ${y} v-4 a4 4 0 0 1 8 0 v4`}
        initial={false}
        {...shackle}
      />
    </g>
  );
}

/** A puzzle piece with a bump on top and one on the right, for extensions. */
export function puzzlePath(x: number, y: number, size: number): string {
  const r = size * 0.16;
  const side = size / 2 - r;
  return `M${x} ${y} h${side} a${r} ${r} 0 1 1 ${r * 2} 0 h${side} v${side} a${r} ${r} 0 1 1 0 ${r * 2} v${side} h-${size} z`;
}

export function Puzzle({
  x,
  y,
  size,
  className,
}: {
  x: number;
  y: number;
  size: number;
  className?: string;
}) {
  return <path d={puzzlePath(x, y, size)} className={className} />;
}

export function Keycap({
  x,
  y,
  width,
  label,
  className,
  animate,
  transition,
}: {
  x: number;
  y: number;
  width: number;
  label: string;
  className?: string;
  animate?: TargetAndTransition;
  transition?: Transition;
}) {
  return (
    <motion.g
      className={className}
      initial={false}
      animate={animate}
      transition={transition}
    >
      <rect x={x} y={y} width={width} height={18} rx={4} />
      <text
        x={x + width / 2}
        y={y + 9}
        textAnchor="middle"
        dominantBaseline="central"
        fontSize={9}
        fontFamily="inherit"
        fill="currentColor"
        stroke="none"
      >
        {label}
      </text>
    </motion.g>
  );
}

export function Cursor({
  animate,
  transition,
  className,
}: {
  animate: TargetAndTransition;
  transition: Transition;
  className?: string;
}) {
  return (
    <motion.path
      d="M0 0 L0 11 L3 8.5 L5 13 L7 12 L5 7.5 L9 7.5 Z"
      fill="currentColor"
      className={className}
      initial={false}
      animate={animate}
      transition={transition}
    />
  );
}

export function Check({
  x,
  y,
  size = 12,
  className,
  animate,
  transition,
}: {
  x: number;
  y: number;
  size?: number;
  className?: string;
  animate: TargetAndTransition;
  transition: Transition;
}) {
  return (
    <motion.path
      d={`M${x} ${y} l${size / 3} ${size / 3} l${(size * 2) / 3} -${(size * 2) / 3}`}
      className={className}
      strokeWidth={2}
      initial={false}
      animate={animate}
      transition={transition}
    />
  );
}

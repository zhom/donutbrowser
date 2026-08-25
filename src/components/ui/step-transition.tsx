"use client";

import { motion, useReducedMotion } from "motion/react";
import type { Key, ReactNode } from "react";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { cn } from "@/lib/utils";

interface StepTransitionProps {
  transitionKey: Key;
  direction: 1 | -1;
  children: ReactNode;
  className?: string;
}

/**
 * Slides a step into place when it changes.
 *
 * Nothing here decides whether the step is on screen. It used to: an
 * `AnimatePresence mode="wait"` held the incoming panel unmounted until the
 * outgoing one finished its exit, and both panels faded from `opacity: 0`. Both
 * halves put the content behind an animation, and an animation is not a
 * guarantee — `requestAnimationFrame` stalls whenever the webview is occluded,
 * unfocused or throttled. When it stalled the dialog was left showing the old
 * step forever, or a panel frozen at zero opacity, with the new step absent
 * from the DOM entirely.
 *
 * So the step renders immediately and at full opacity, and the only animated
 * property is a few pixels of travel. If the animation never runs, the content
 * is still there, still readable, a hair off its final position. Motion may
 * decorate a transition; it may never be what performs one.
 */
export function StepTransition({
  transitionKey,
  direction,
  children,
  className,
}: StepTransitionProps) {
  const reduceMotion = useReducedMotion();

  return (
    <motion.div
      // Remounting on the key is what replaces the old step. It is synchronous,
      // so the swap does not depend on any animation finishing.
      key={transitionKey}
      initial={reduceMotion ? false : { x: direction * 6 }}
      animate={{ x: 0 }}
      transition={{ duration: 0.18, ease: MOTION_EASE_OUT }}
      className={cn(className)}
    >
      {children}
    </motion.div>
  );
}

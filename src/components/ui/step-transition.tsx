"use client";

import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { Key, ReactNode } from "react";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { cn } from "@/lib/utils";

interface StepTransitionProps {
  transitionKey: Key;
  direction: 1 | -1;
  children: ReactNode;
  className?: string;
}

export function StepTransition({
  transitionKey,
  direction,
  children,
  className,
}: StepTransitionProps) {
  const reduceMotion = useReducedMotion();

  return (
    <AnimatePresence initial={false} mode="wait" custom={direction}>
      <motion.div
        key={transitionKey}
        custom={direction}
        variants={{
          enter: (customDirection: 1 | -1) => ({
            opacity: 0,
            x: reduceMotion ? 0 : customDirection * 6,
          }),
          center: {
            opacity: 1,
            x: 0,
            transition: {
              duration: reduceMotion ? 0.16 : 0.18,
              ease: MOTION_EASE_OUT,
            },
          },
          exit: (customDirection: 1 | -1) => ({
            opacity: 0,
            x: reduceMotion ? 0 : customDirection * -6,
            transition: {
              duration: reduceMotion ? 0.16 : 0.12,
              ease: MOTION_EASE_OUT,
            },
          }),
        }}
        initial="enter"
        animate="center"
        exit="exit"
        className={cn(className)}
      >
        {children}
      </motion.div>
    </AnimatePresence>
  );
}

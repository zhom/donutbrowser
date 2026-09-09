"use client";

import { motion, useReducedMotion } from "motion/react";
import type { ComponentProps, ReactNode } from "react";
import { useInputModality } from "@/hooks/use-input-modality";
import { MOTION_EASE_OUT, MOTION_SPRING_POSITION } from "@/lib/motion";
import { cn } from "@/lib/utils";

interface AnimatedDisclosureItemProps
  extends ComponentProps<typeof motion.div> {
  children: ReactNode;
}

export function AnimatedDisclosureItem({
  children,
  ...props
}: AnimatedDisclosureItemProps) {
  const reduceMotion = useReducedMotion();

  return (
    <motion.div
      layout={reduceMotion ? false : "position"}
      transition={reduceMotion ? { duration: 0 } : MOTION_SPRING_POSITION}
      {...props}
    >
      {children}
    </motion.div>
  );
}

export function AnimatedDisclosureChevron({
  open,
  children,
  className,
}: {
  open: boolean;
  children: ReactNode;
  className?: string;
}) {
  const reduceMotion = useReducedMotion();

  return (
    <motion.span
      aria-hidden="true"
      animate={{ rotate: open ? 90 : 0 }}
      transition={{
        duration: reduceMotion ? 0 : 0.16,
        ease: MOTION_EASE_OUT,
      }}
      className={cn("inline-flex shrink-0", className)}
    >
      {children}
    </motion.span>
  );
}

export function AnimatedDisclosureContent({
  open,
  children,
  className,
}: {
  open: boolean;
  children: ReactNode;
  className?: string;
}) {
  const reduceMotion = useReducedMotion();
  const inputModality = useInputModality();

  if (!open) return null;

  return (
    <motion.div
      initial={reduceMotion || inputModality === "keyboard" ? false : { y: -4 }}
      animate={{ y: 0 }}
      transition={{
        duration: reduceMotion || inputModality === "keyboard" ? 0 : 0.16,
        ease: MOTION_EASE_OUT,
      }}
      className={cn(className)}
    >
      {children}
    </motion.div>
  );
}

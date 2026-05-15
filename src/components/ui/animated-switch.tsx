"use client";

import { motion } from "motion/react";
import { Switch as SwitchPrimitive } from "radix-ui";
import type * as React from "react";

import { cn } from "@/lib/utils";

const MotionThumb = motion.create(SwitchPrimitive.Thumb);

type AnimatedSwitchProps = React.ComponentProps<typeof SwitchPrimitive.Root>;

/**
 * Toggle switch with a thumb that slides between the off (left) and on
 * (right) positions and squashes wider while pressed. Animated via Framer
 * Motion — no layout shift when the parent's width changes, and the
 * pressed state is purely visual so external onCheckedChange semantics
 * stay identical to a Radix Switch.
 */
function AnimatedSwitch({ className, ...props }: AnimatedSwitchProps) {
  return (
    <SwitchPrimitive.Root
      data-slot="animated-switch"
      className={cn(
        "peer relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border border-transparent",
        "bg-input data-[state=checked]:bg-primary",
        "transition-colors duration-200 ease-out",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <MotionThumb
        data-slot="animated-switch-thumb"
        className={cn(
          "pointer-events-none block size-4 rounded-full shadow-sm ring-0",
          "bg-background data-[state=checked]:bg-primary-foreground",
        )}
        layout
        transition={{ type: "spring", stiffness: 700, damping: 32, mass: 0.5 }}
        whileTap={{ width: 22 }}
        style={{ marginLeft: 2, marginRight: 2 }}
      />
    </SwitchPrimitive.Root>
  );
}

export type { AnimatedSwitchProps };
export { AnimatedSwitch };

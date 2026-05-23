"use client";

import { motion } from "motion/react";
import { Switch as SwitchPrimitive } from "radix-ui";
import type * as React from "react";

import { cn } from "@/lib/utils";

const MotionThumb = motion.create(SwitchPrimitive.Thumb);

type AnimatedSwitchProps = React.ComponentProps<typeof SwitchPrimitive.Root>;

/**
 * Switch whose thumb actually slides between off and on. The Root flips
 * its flex alignment on `data-state=checked`, which moves the Thumb's
 * layout box; Framer Motion's `layout` prop tweens between the two
 * positions. The thumb also squashes wider while pressed.
 */
function AnimatedSwitch({ className, ...props }: AnimatedSwitchProps) {
  return (
    <SwitchPrimitive.Root
      data-slot="animated-switch"
      className={cn(
        "peer relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center justify-start rounded-full border border-transparent px-[2px]",
        "bg-input data-[state=checked]:bg-primary data-[state=checked]:justify-end",
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
        whileTap={{ width: 20 }}
      />
    </SwitchPrimitive.Root>
  );
}

export type { AnimatedSwitchProps };
export { AnimatedSwitch };

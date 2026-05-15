"use client";

import * as TabsPrimitive from "@radix-ui/react-tabs";
import { motion } from "motion/react";
import * as React from "react";

import { useControlledState } from "@/hooks/use-controlled-state";
import { cn } from "@/lib/utils";

interface AnimatedTabsContextValue {
  activeValue: string | undefined;
  hoveredValue: string | null;
  setHoveredValue: (value: string | null) => void;
  indicatorId: string;
}

const AnimatedTabsContext =
  React.createContext<AnimatedTabsContextValue | null>(null);

function useAnimatedTabs() {
  const ctx = React.useContext(AnimatedTabsContext);
  if (!ctx) {
    throw new Error(
      "AnimatedTabsTrigger must be rendered inside <AnimatedTabs>",
    );
  }
  return ctx;
}

type AnimatedTabsProps = React.ComponentProps<typeof TabsPrimitive.Root>;

function AnimatedTabs({
  value: valueProp,
  defaultValue,
  onValueChange,
  children,
  ...props
}: AnimatedTabsProps) {
  const [activeValue, setActiveValue] = useControlledState({
    value: valueProp,
    defaultValue,
    onChange: onValueChange,
  });
  const [hoveredValue, setHoveredValue] = React.useState<string | null>(null);
  const indicatorId = React.useId();

  return (
    <AnimatedTabsContext.Provider
      value={{
        activeValue,
        hoveredValue,
        setHoveredValue,
        indicatorId,
      }}
    >
      <TabsPrimitive.Root
        data-slot="animated-tabs"
        value={activeValue}
        defaultValue={defaultValue}
        onValueChange={setActiveValue}
        {...props}
      >
        {children}
      </TabsPrimitive.Root>
    </AnimatedTabsContext.Provider>
  );
}

type AnimatedTabsListProps = React.ComponentProps<typeof TabsPrimitive.List>;

function AnimatedTabsList({
  className,
  onMouseLeave,
  ...props
}: AnimatedTabsListProps) {
  const { setHoveredValue } = useAnimatedTabs();
  return (
    <TabsPrimitive.List
      data-slot="animated-tabs-list"
      className={cn(
        "relative inline-flex items-center gap-1 rounded-md p-0",
        className,
      )}
      onMouseLeave={(event) => {
        setHoveredValue(null);
        onMouseLeave?.(event);
      }}
      {...props}
    />
  );
}

type AnimatedTabsTriggerProps = React.ComponentProps<
  typeof TabsPrimitive.Trigger
>;

function AnimatedTabsTrigger({
  value,
  className,
  children,
  onMouseEnter,
  ...props
}: AnimatedTabsTriggerProps) {
  const { activeValue, hoveredValue, setHoveredValue, indicatorId } =
    useAnimatedTabs();
  // The visible pill follows hover when present, otherwise sits on the
  // active tab. Framer's `layoutId` handles the slide animation between
  // mounted instances; only the trigger whose `value` matches `shownValue`
  // renders the indicator, so the transition is a single-element move.
  const shownValue = hoveredValue ?? activeValue;
  const showIndicator = shownValue === value;
  const isActive = activeValue === value;

  return (
    <TabsPrimitive.Trigger
      data-slot="animated-tabs-trigger"
      value={value}
      onMouseEnter={(event) => {
        setHoveredValue(value);
        onMouseEnter?.(event);
      }}
      className={cn(
        "relative isolate inline-flex h-7 cursor-pointer items-center justify-center gap-1.5 whitespace-nowrap rounded-md px-3 text-sm font-medium transition-colors duration-150",
        "text-muted-foreground hover:text-foreground",
        isActive && "text-foreground",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
        "disabled:pointer-events-none disabled:opacity-50",
        className,
      )}
      {...props}
    >
      {showIndicator && (
        <motion.span
          layoutId={`animated-tabs-indicator-${indicatorId}`}
          className="absolute inset-0 -z-10 rounded-md bg-accent"
          transition={{ type: "spring", stiffness: 360, damping: 32 }}
        />
      )}
      {children}
    </TabsPrimitive.Trigger>
  );
}

const AnimatedTabsContent = TabsPrimitive.Content;

export type {
  AnimatedTabsListProps,
  AnimatedTabsProps,
  AnimatedTabsTriggerProps,
};
export {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
};

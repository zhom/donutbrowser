"use client";

import { type HTMLAttributes, useRef } from "react";
import { useScrollFade } from "@/hooks/use-scroll-fade";
import { cn } from "@/lib/utils";

export type FadingScrollAreaProps = HTMLAttributes<HTMLDivElement>;

/**
 * Scrollable container with top/bottom fade overlays. The fades only become
 * visible when the matching direction is actually scrollable. Use in place
 * of `<div className="border rounded-md max-h-[...] overflow-auto">` for
 * lists that should match the borderless aesthetic of the profile table.
 */
export function FadingScrollArea({
  className,
  children,
  ...props
}: FadingScrollAreaProps) {
  const ref = useRef<HTMLDivElement>(null);
  useScrollFade(ref);

  return (
    <div
      ref={ref}
      className={cn("overflow-y-auto scroll-fade", className)}
      {...props}
    >
      {children}
    </div>
  );
}

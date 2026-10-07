"use client";

import type { ReactNode } from "react";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

export const SECTION_LABEL_CLASS =
  "text-[11px] font-medium tracking-wide text-muted-foreground uppercase";

/** One quiet surface for a group of related settings. */
export function SettingsPanel({
  children,
  className,
  ...props
}: React.ComponentProps<"section">) {
  return (
    <section
      className={cn("rounded-xl bg-foreground/3 p-4", className)}
      {...props}
    >
      {children}
    </section>
  );
}

/** A setting with its explanation on the left and its control on the right. */
export function SettingsRow({
  label,
  description,
  htmlFor,
  children,
  className,
}: {
  label: ReactNode;
  description?: ReactNode;
  htmlFor?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col gap-2 py-3 first:pt-0 last:pb-0 @xl:flex-row @xl:items-center @xl:justify-between @xl:gap-6",
        className,
      )}
    >
      <div className="flex min-w-0 flex-col gap-0.5 @xl:max-w-[50%]">
        <Label htmlFor={htmlFor} className="text-sm font-medium">
          {label}
        </Label>
        {description && (
          <p className="text-xs leading-relaxed text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      <div className="flex min-w-0 items-center gap-2 @xl:justify-end">
        {children}
      </div>
    </div>
  );
}

/** The icon beside a feature's on/off switch; it lights up while the feature runs. */
export function FeatureIcon({
  children,
  active,
}: {
  children: ReactNode;
  active: boolean;
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "grid size-9 shrink-0 place-items-center rounded-lg transition-colors duration-200 [&_svg]:size-4.5",
        active
          ? "bg-success/12 text-success-text"
          : "bg-foreground/6 text-muted-foreground",
      )}
    >
      {children}
    </span>
  );
}

/** A status light. `live` pulses for a connection that is up right now. */
export function StatusLight({
  tone,
  live = false,
  className,
}: {
  tone: "success" | "warning" | "destructive" | "muted";
  live?: boolean;
  className?: string;
}) {
  const color = {
    success: "bg-success",
    warning: "bg-warning",
    destructive: "bg-destructive",
    muted: "bg-muted-foreground",
  }[tone];
  return (
    <span
      aria-hidden="true"
      className={cn("relative inline-flex size-2 shrink-0", className)}
    >
      {live && (
        <span
          className={cn(
            "absolute inset-0 animate-ping rounded-full opacity-60 motion-reduce:hidden",
            color,
          )}
        />
      )}
      <span className={cn("relative size-2 rounded-full", color)} />
    </span>
  );
}

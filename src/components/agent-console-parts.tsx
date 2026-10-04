"use client";

import type { TFunction } from "i18next";
import { type ReactNode, useCallback, useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import {
  type AgentActivity,
  activityDetail,
  MAX_TEXT_CHARS,
} from "@/lib/agent-console";
import { cn } from "@/lib/utils";

export function useNow(intervalMs = 15_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => {
      setNow(Date.now());
    }, intervalMs);
    return () => {
      window.clearInterval(id);
    };
  }, [intervalMs]);
  return now;
}

const RELATIVE_UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["day", 86_400],
  ["hour", 3600],
  ["minute", 60],
];

export function useTimeFormat() {
  const { t, i18n } = useTranslation();
  const language = i18n.language;

  const relative = useCallback(
    (at: number, now: number) => {
      const formatter = new Intl.RelativeTimeFormat(language, {
        numeric: "auto",
      });
      const seconds = Math.round((at - now) / 1000);
      for (const [unit, size] of RELATIVE_UNITS) {
        if (Math.abs(seconds) >= size) {
          return formatter.format(Math.round(seconds / size), unit);
        }
      }
      return formatter.format(0, "second");
    },
    [language],
  );

  const clock = useCallback(
    (at: number) =>
      new Date(at).toLocaleTimeString(language, {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      }),
    [language],
  );

  const shortClock = useCallback(
    (at: number) =>
      new Date(at).toLocaleTimeString(language, {
        hour: "2-digit",
        minute: "2-digit",
      }),
    [language],
  );

  const duration = useCallback(
    (ms: number) =>
      ms < 1000
        ? t("agent.activity.durationMs", { ms })
        : t("agent.activity.durationSeconds", {
            seconds: (ms / 1000).toFixed(ms < 10_000 ? 1 : 0),
          }),
    [t],
  );

  return { relative, clock, shortClock, duration };
}

export function NoteAction({
  label,
  confirmLabel,
  title,
  placeholder,
  icon,
  onConfirm,
  disabled,
  variant = "outline",
  size = "sm",
  testId,
  className,
}: {
  label: string;
  confirmLabel?: string;
  title?: string;
  placeholder?: string;
  icon?: ReactNode;
  onConfirm: (note: string | null) => Promise<boolean>;
  disabled?: boolean;
  variant?: "outline" | "default" | "ghost" | "secondary";
  size?: "sm" | "default";
  testId?: string;
  className?: string;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const noteId = useId();

  const submit = async () => {
    setBusy(true);
    const trimmed = note.trim();
    const ok = await onConfirm(trimmed.length > 0 ? trimmed : null);
    setBusy(false);
    if (ok) {
      setNote("");
      setOpen(false);
    }
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant={variant}
          size={size}
          disabled={disabled}
          data-testid={testId}
          className={cn("gap-1.5", className)}
        >
          {icon}
          {label}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 p-3">
        <form
          className="flex flex-col gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <Label htmlFor={noteId} className="text-xs">
            {title ?? t("agent.actions.noteOptional")}
          </Label>
          <Textarea
            id={noteId}
            value={note}
            rows={3}
            maxLength={MAX_TEXT_CHARS}
            placeholder={placeholder}
            className="min-h-16 text-sm"
            data-testid={testId ? `${testId}-note` : undefined}
            onChange={(event) => {
              setNote(event.target.value);
            }}
          />
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                setOpen(false);
              }}
            >
              {t("common.buttons.cancel")}
            </Button>
            <Button
              type="submit"
              size="sm"
              disabled={busy}
              data-testid={testId ? `${testId}-confirm` : undefined}
            >
              {confirmLabel ?? label}
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  );
}

/** What the page shows in place of a feature that cannot work yet. */
export function SetupPanel({
  icon,
  title,
  body,
  testId,
  children,
  className,
}: {
  icon: ReactNode;
  title: string;
  body: string;
  testId: string;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <section
      data-testid={testId}
      aria-label={title}
      className={cn(
        "flex min-h-0 flex-1 flex-col items-center justify-center gap-3 px-4 py-12 text-center",
        className,
      )}
    >
      <span className="text-muted-foreground [&>svg]:size-10">{icon}</span>
      <h3 className="text-sm font-medium text-foreground">{title}</h3>
      <p className="max-w-md text-xs text-muted-foreground">{body}</p>
      {children}
    </section>
  );
}

export function ProgressBar({
  percent,
  label,
}: {
  percent: number;
  label: string;
}) {
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      className="h-1 w-full overflow-hidden rounded-full bg-muted"
    >
      <div
        className="h-full rounded-full bg-primary transition-[width] duration-300 motion-reduce:transition-none"
        style={{ width: `${percent}%` }}
      />
    </div>
  );
}

export function activityDetailText(
  t: TFunction,
  entry: AgentActivity,
): string | null {
  const detail = activityDetail(entry);
  if (!detail) return null;
  switch (detail.kind) {
    case "host":
      return detail.host;
    case "typed":
      return t("agent.activity.typed", { count: detail.count });
    case "profiles":
      return t("agent.activity.profiles", { count: detail.count });
  }
}

"use client";

import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { LuPencil, LuTrash2 } from "react-icons/lu";
import {
  describeCadence,
  minutesToClock,
  StatusDot,
  scheduleBlockedReason,
  scheduleTone,
} from "@/components/cookie-bot-shared";
import { Button } from "@/components/ui/button";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { CookieBotSchedule } from "@/lib/cookie-bot";
import { cn } from "@/lib/utils";

/** A slot holding this many enrolments is worth flagging: the fleet leases a
 * handful of machines per platform, so a pile-up at one minute is a real
 * capacity fact, not a decoration. */
const CROWDED_SLOT = 4;

interface Slot {
  hour: number;
  entries: CookieBotSchedule[];
}

interface CookieBotScheduleTabProps {
  schedules: CookieBotSchedule[];
  isLoading: boolean;
  currentUserId: string | null;
  canEditOthers: boolean;
  onEdit: (schedule: CookieBotSchedule) => void;
  onRemove: (schedule: CookieBotSchedule) => void;
}

/**
 * The night, drawn as a night. Every enrolment the caller can see sits under
 * the hour it starts, so two operators aiming at the same profile — or twelve
 * profiles aiming at 02:00 — is visible before it becomes a 409 at 02:00.
 */
export function CookieBotScheduleTab({
  schedules,
  isLoading,
  currentUserId,
  canEditOthers,
  onEdit,
  onRemove,
}: CookieBotScheduleTabProps) {
  const { t } = useTranslation();

  const rows = useMemo(() => buildRows(schedules), [schedules]);

  if (isLoading && schedules.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3 pt-2">
        {Array.from({ length: 5 }, (_, i) => (
          <div key={`slot-skeleton-${i}`} className="flex items-center gap-3">
            <Skeleton className="h-3 w-10" />
            <Skeleton
              className="h-3"
              style={{ width: `${30 + ((i * 17) % 40)}%` }}
            />
          </div>
        ))}
      </div>
    );
  }

  if (schedules.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center py-16">
        <p className="text-sm text-muted-foreground">
          {t("cookieBot.schedule.empty")}
        </p>
      </div>
    );
  }

  return (
    <FadingScrollArea
      className="min-h-0 flex-1"
      style={{ "--scroll-fade-top-offset": "16px" } as React.CSSProperties}
    >
      <div className="flex flex-col pr-1">
        {rows.map((row) =>
          row.kind === "gap" ? (
            <div
              key={`gap-${row.from}`}
              className="flex h-6 items-center gap-2 pl-14 text-[10px] uppercase tracking-wide text-muted-foreground"
            >
              <span>
                {t("cookieBot.schedule.quietHours", { count: row.count })}
              </span>
              <span className="h-px flex-1 rounded-full bg-border" />
            </div>
          ) : (
            <div key={`slot-${row.slot.hour}`} className="flex gap-3 pb-4">
              <span className="w-14 shrink-0 pt-1 text-right text-xs tabular-nums text-muted-foreground">
                {minutesToClock(row.slot.hour * 60)}
              </span>
              <div className="flex min-w-0 flex-1 flex-col gap-0.5 border-l border-border pl-3">
                {row.slot.entries.map((schedule) => {
                  const mine =
                    !schedule.owner_user_id ||
                    schedule.owner_user_id === currentUserId;
                  const editable = mine || canEditOthers;
                  const blocked = scheduleBlockedReason(t, schedule);
                  return (
                    <div
                      key={`${schedule.owner_user_id ?? "me"}-${schedule.profile_id}`}
                      className="group flex h-7 items-center gap-2 rounded-md px-2 text-xs transition-colors duration-100 hover:bg-accent hover:text-accent-foreground"
                    >
                      <StatusDot
                        tone={scheduleTone(schedule)}
                        className="size-1.5"
                      />
                      <span className="min-w-0 flex-1 truncate">
                        {schedule.profile_name}
                      </span>
                      {/* "This cannot run" beats "it runs nightly": an
                          enrolment the server will refuse should not read as a
                          cadence it is about to keep. */}
                      <span
                        className={cn(
                          "shrink-0 text-[10px] uppercase tracking-wide",
                          blocked
                            ? "text-warning-text"
                            : "text-muted-foreground",
                        )}
                      >
                        {blocked ?? describeCadence(t, schedule.days_mask)}
                      </span>
                      <span className="shrink-0 tabular-nums text-muted-foreground">
                        {minutesToClock(schedule.run_at_minute)}
                      </span>
                      {!mine && schedule.owner_email && (
                        <span className="hidden max-w-40 shrink-0 truncate text-[10px] uppercase tracking-wide text-muted-foreground @2xl:inline">
                          {schedule.owner_email}
                        </span>
                      )}
                      <SlotAction
                        label={t("cookieBot.enrolled.edit")}
                        forbidden={!editable}
                        onClick={() => {
                          onEdit(schedule);
                        }}
                      >
                        <LuPencil className="size-3.5" />
                      </SlotAction>
                      <SlotAction
                        label={t("cookieBot.schedule.unenrol")}
                        forbidden={!editable}
                        destructive
                        onClick={() => {
                          onRemove(schedule);
                        }}
                      >
                        <LuTrash2 className="size-3.5" />
                      </SlotAction>
                    </div>
                  );
                })}
                {row.slot.entries.length >= CROWDED_SLOT && (
                  <span className="px-2 pt-1 text-[11px] text-warning-text">
                    {t("cookieBot.schedule.crowded", {
                      count: row.slot.entries.length,
                    })}
                  </span>
                )}
              </div>
            </div>
          ),
        )}
      </div>
    </FadingScrollArea>
  );
}

function SlotAction({
  label,
  forbidden,
  destructive,
  onClick,
  children,
}: {
  label: string;
  forbidden: boolean;
  destructive?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="inline-flex shrink-0">
          <Button
            variant="ghost"
            size="icon"
            className={cn(
              "size-7",
              destructive && "text-destructive-text hover:bg-destructive/10",
            )}
            aria-label={label}
            disabled={forbidden}
            onClick={onClick}
          >
            {children}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>
        {forbidden ? t("cookieBot.conflict.replaceForbidden") : label}
      </TooltipContent>
    </Tooltip>
  );
}

type Row =
  | { kind: "slot"; slot: Slot }
  | { kind: "gap"; from: number; count: number };

/**
 * Groups enrolments into the hour they start and collapses the empty stretches
 * between them. A 24-row skeleton of empty hours would be a grid pretending to
 * be information.
 */
function buildRows(schedules: CookieBotSchedule[]): Row[] {
  const byHour = new Map<number, CookieBotSchedule[]>();
  for (const schedule of schedules) {
    const hour = Math.floor(schedule.run_at_minute / 60) % 24;
    const bucket = byHour.get(hour);
    if (bucket) bucket.push(schedule);
    else byHour.set(hour, [schedule]);
  }

  const hours = [...byHour.keys()].sort((a, b) => a - b);
  const rows: Row[] = [];
  let previous: number | null = null;
  for (const hour of hours) {
    if (previous !== null && hour - previous > 1) {
      rows.push({
        kind: "gap",
        from: previous + 1,
        count: hour - previous - 1,
      });
    }
    const entries = (byHour.get(hour) ?? []).sort(
      (a, b) =>
        a.run_at_minute - b.run_at_minute ||
        a.profile_name.localeCompare(b.profile_name),
    );
    rows.push({ kind: "slot", slot: { hour, entries } });
    previous = hour;
  }
  return rows;
}

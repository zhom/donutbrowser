"use client";

import { motion, useReducedMotion } from "motion/react";
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
import { useInputModality } from "@/hooks/use-input-modality";
import type { CookieBotSchedule } from "@/lib/cookie-bot";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { scheduleLanes } from "@/lib/schedule-layout";
import { cn } from "@/lib/utils";

interface CookieBotScheduleTabProps {
  schedules: CookieBotSchedule[];
  isLoading: boolean;
  currentUserId: string | null;
  canEditOthers: boolean;
  onEdit: (schedule: CookieBotSchedule) => void;
  onRemove: (schedule: CookieBotSchedule) => void;
}

export function CookieBotScheduleTab({
  schedules,
  isLoading,
  currentUserId,
  canEditOthers,
  onEdit,
  onRemove,
}: CookieBotScheduleTabProps) {
  const { t } = useTranslation();
  const reduced = useReducedMotion();
  const modality = useInputModality();
  const groups = useMemo(() => scheduleLanes(schedules), [schedules]);
  if (isLoading && !schedules.length)
    return <Skeleton className="h-32 w-full" />;
  if (!schedules.length)
    return (
      <p className="py-16 text-center text-sm text-muted-foreground">
        {t("cookieBot.schedule.empty")}
      </p>
    );

  return (
    <FadingScrollArea className="min-h-0 flex-1">
      <div data-slot="schedule-timeline" className="space-y-8 pr-2 pb-4">
        <p className="text-xs leading-relaxed text-muted-foreground">
          {t("appFeedback.requestedSchedule")}
        </p>
        {[...groups].map(([timezone, lanes]) => {
          const start = Math.max(
            0,
            Math.floor(
              (Math.min(...lanes.map((lane) => lane.minute)) - 30) / 60,
            ) * 60,
          );
          const end = Math.max(
            start + 120,
            Math.ceil(
              (Math.max(...lanes.map((lane) => lane.minute + lane.duration)) +
                30) /
                60,
            ) * 60,
          );
          const span = end - start;
          return (
            <section key={timezone} aria-label={timezone} className="min-w-0">
              <h3 className="mb-3 text-sm font-medium">
                {timezone || t("common.labels.default")}
              </h3>
              <div
                className="grid grid-cols-[minmax(0,1fr)_minmax(100px,2fr)_4rem] gap-x-3 text-[11px] tabular-nums text-muted-foreground"
                aria-hidden="true"
              >
                <span />
                <div className="flex justify-between gap-2 pb-2">
                  {[start, Math.round((start + span / 2) / 15) * 15, end].map(
                    (minute) => (
                      <span key={minute}>
                        {minutesToClock(minute)}
                        {minute >= 1440 ? " +1" : ""}
                      </span>
                    ),
                  )}
                </div>
              </div>
              {lanes.map((lane) => {
                const { schedule } = lane;
                const editable =
                  !schedule.owner_user_id ||
                  schedule.owner_user_id === currentUserId ||
                  canEditOthers;
                const blocked = scheduleBlockedReason(t, schedule);
                return (
                  <div
                    key={lane.id}
                    data-slot="schedule-lane"
                    data-profile-id={schedule.profile_id}
                    data-minute={lane.minute}
                    data-overlaps={lane.overlaps}
                    className="grid min-w-0 grid-cols-[minmax(0,1fr)_minmax(100px,2fr)_4rem] items-center gap-x-3 py-2"
                  >
                    <div className="min-w-0">
                      <button
                        type="button"
                        disabled={!editable}
                        onClick={() => onEdit(schedule)}
                        className="flex w-full items-start gap-2 rounded-sm text-left text-sm focus-visible:outline-2 focus-visible:outline-ring disabled:cursor-default"
                      >
                        <StatusDot
                          tone={scheduleTone(schedule)}
                          className="mt-1.5 shrink-0"
                        />
                        <span className="min-w-0 break-words font-medium">
                          {schedule.profile_name}
                        </span>
                      </button>
                      {schedule.owner_email &&
                        schedule.owner_user_id !== currentUserId && (
                          <p className="mt-1 break-words text-xs text-muted-foreground">
                            {schedule.owner_email}
                          </p>
                        )}
                      <p className="mt-1 text-xs text-muted-foreground">
                        {describeCadence(t, lane.days)}
                      </p>
                      <p className="mt-0.5 text-xs tabular-nums text-muted-foreground">
                        {minutesToClock(lane.minute)} ·{" "}
                        {t("cookieBot.chart.minutes", {
                          minutes: lane.duration,
                        })}
                      </p>
                      {blocked && (
                        <p className="mt-1 text-xs text-warning-text">
                          {blocked}
                        </p>
                      )}
                      {lane.overlaps > 0 && (
                        <p className="mt-1 text-xs text-warning-text">
                          {t("appFeedback.overlappingBookings", {
                            count: lane.overlaps,
                          })}
                        </p>
                      )}
                    </div>
                    <button
                      type="button"
                      disabled={!editable}
                      onClick={() => onEdit(schedule)}
                      aria-label={`${t("cookieBot.enrolled.edit")}: ${schedule.profile_name}, ${minutesToClock(lane.minute)}, ${timezone}`}
                      className="relative h-10 min-w-0 rounded-md focus-visible:outline-2 focus-visible:outline-ring disabled:cursor-default"
                    >
                      <span
                        aria-hidden="true"
                        className="absolute inset-x-0 top-1/2 h-0.5 -translate-y-1/2 rounded-full bg-border"
                      />
                      <motion.span
                        aria-hidden="true"
                        initial={false}
                        animate={{
                          left: `${((lane.minute - start) / span) * 100}%`,
                          width: `${(lane.duration / span) * 100}%`,
                        }}
                        transition={{
                          duration:
                            reduced || modality === "keyboard" ? 0 : 0.22,
                          ease: MOTION_EASE_OUT,
                        }}
                        className={cn(
                          "pointer-events-none absolute top-3 h-4 min-w-0.5 rounded-sm bg-foreground",
                          (!schedule.enabled || blocked) &&
                            "bg-muted-foreground",
                          lane.overlaps > 0 && "bg-warning",
                        )}
                      />
                    </button>
                    <div className="flex items-center justify-end">
                      <SlotAction
                        label={t("cookieBot.enrolled.edit")}
                        forbidden={!editable}
                        onClick={() => onEdit(schedule)}
                      >
                        <LuPencil className="size-3.5" />
                      </SlotAction>
                      <SlotAction
                        label={t("cookieBot.schedule.unenrol")}
                        forbidden={!editable}
                        destructive
                        onClick={() => onRemove(schedule)}
                      >
                        <LuTrash2 className="size-3.5" />
                      </SlotAction>
                    </div>
                  </div>
                );
              })}
            </section>
          );
        })}
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

"use client";

import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { LuCookie, LuPencil, LuTrash2 } from "react-icons/lu";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Tooltip as ChartTooltip,
  ResponsiveContainer,
  XAxis,
  YAxis,
} from "recharts";
import type { RunFilter } from "@/components/cookie-bot-activity";
import {
  describeCadence,
  formatDate,
  formatDateTime,
  minutesToClock,
  parseIso,
  StatusDot,
  scheduleBlockedReason,
  scheduleTone,
  sessionDisplayName,
  sessionPhaseLabel,
  sessionTone,
  useNextDue,
} from "@/components/cookie-bot-shared";
import { Button } from "@/components/ui/button";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { RippleButton } from "@/components/ui/ripple";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type {
  CookieBotRun,
  CookieBotSchedule,
  RemoteHoursQuota,
} from "@/lib/cookie-bot";
import type { RemoteSessionState } from "@/lib/remote-sessions";
import { cn } from "@/lib/utils";
import type { BrowserProfile } from "@/types";

const CHART_NIGHTS = 30;

interface CookieBotOverviewProps {
  schedules: CookieBotSchedule[];
  runs: CookieBotRun[];
  live: RemoteSessionState[];
  quota: RemoteHoursQuota | null;
  profiles: BrowserProfile[];
  isLoading: boolean;
  currentUserId: string | null;
  onEnrol: () => void;
  onEditSchedule: (schedule: CookieBotSchedule) => void;
  onRemoveSchedule: (schedule: CookieBotSchedule) => void;
  onJumpToActivity: (filter: RunFilter) => void;
}

export function CookieBotOverview({
  schedules,
  runs,
  live,
  quota,
  profiles,
  isLoading,
  currentUserId,
  onEnrol,
  onEditSchedule,
  onRemoveSchedule,
  onJumpToActivity,
}: CookieBotOverviewProps) {
  const { t } = useTranslation();
  const { next, nextAt, dueCount } = useNextDue(schedules);
  const profileIndex = useMemo(
    () => new Map(profiles.map((p) => [p.id, p])),
    [profiles],
  );

  const recent = useMemo(() => summariseRecent(runs), [runs]);
  const chartData = useMemo(() => nightlyMinutes(runs), [runs]);
  const exhausted =
    quota !== null && quota.granted_hours > 0 && quota.remaining_hours <= 0;
  const resetDate = formatDate(quota?.period_end);

  if (!isLoading && schedules.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 py-16 text-center">
        <LuCookie className="size-12 text-muted-foreground" />
        <div>
          <p className="text-sm font-medium text-foreground">
            {t("cookieBot.empty.title")}
          </p>
          <p className="mt-1 max-w-md text-xs text-muted-foreground">
            {t("cookieBot.empty.hint")}
          </p>
        </div>
        <RippleButton size="sm" onClick={onEnrol}>
          {t("cookieBot.empty.cta")}
        </RippleButton>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <TonightStrip
        live={live}
        nextAt={nextAt}
        nextMinute={next?.run_at_minute ?? null}
        dueCount={dueCount}
        profileIndex={profileIndex}
      />

      {exhausted && (
        <div className="shrink-0 rounded-md border border-warning/50 bg-warning/10 px-3 py-2 text-xs text-warning-text">
          {resetDate
            ? t("cookieBot.hours.exhaustedOn", { date: resetDate })
            : t("cookieBot.hours.exhausted")}
        </div>
      )}

      <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs">
        <span className="text-muted-foreground">
          {t("cookieBot.lastDay.label")}
        </span>
        {recent.total === 0 ? (
          <span className="text-muted-foreground">
            {t("cookieBot.lastDay.none")}
          </span>
        ) : (
          <>
            <RecentSegment
              label={t("cookieBot.lastDay.ran", { count: recent.succeeded })}
              tone="text-foreground"
              onClick={() => {
                onJumpToActivity("succeeded");
              }}
            />
            {recent.partial > 0 && (
              <RecentSegment
                label={t("cookieBot.lastDay.partial", {
                  count: recent.partial,
                })}
                tone="text-warning-text"
                onClick={() => {
                  onJumpToActivity("partial");
                }}
              />
            )}
            {recent.failed > 0 && (
              <RecentSegment
                label={t("cookieBot.lastDay.failed", { count: recent.failed })}
                tone="text-destructive-text"
                onClick={() => {
                  onJumpToActivity("failed");
                }}
              />
            )}
          </>
        )}
      </div>

      <div className="shrink-0">
        <p className="mb-1 text-xs font-medium text-foreground">
          {t("cookieBot.chart.machineTime")}
        </p>
        <div className="h-[clamp(140px,20vh,260px)] w-full">
          {isLoading && runs.length === 0 ? (
            <Skeleton className="size-full" />
          ) : (
            <ResponsiveContainer
              width="100%"
              height="100%"
              minWidth={1}
              minHeight={1}
            >
              <AreaChart
                data={chartData}
                margin={{ top: 6, right: 8, bottom: 0, left: 0 }}
              >
                <defs>
                  <linearGradient
                    id="cookieBotMinutesGradient"
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="1"
                  >
                    <stop
                      offset="0%"
                      stopColor="var(--chart-1)"
                      stopOpacity={0.5}
                    />
                    <stop
                      offset="100%"
                      stopColor="var(--chart-1)"
                      stopOpacity={0.1}
                    />
                  </linearGradient>
                </defs>
                <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
                <XAxis
                  dataKey="label"
                  className="text-xs"
                  tick={{ fill: "var(--muted-foreground)" }}
                  minTickGap={24}
                />
                <YAxis
                  className="text-xs"
                  tick={{ fill: "var(--muted-foreground)" }}
                  width={36}
                />
                <ChartTooltip
                  content={({ active, payload, label }) => {
                    if (!active || !payload?.length) return null;
                    const minutes = Number(payload[0]?.value ?? 0);
                    return (
                      <div className="rounded-lg border bg-popover px-3 py-2 shadow-lg">
                        <p className="text-xs font-medium text-popover-foreground">
                          {String(label)}
                        </p>
                        <p className="text-xs tabular-nums text-muted-foreground">
                          {t("cookieBot.chart.minutes", { minutes })}
                        </p>
                      </div>
                    );
                  }}
                />
                <Area
                  type="monotone"
                  dataKey="minutes"
                  stroke="var(--chart-1)"
                  fill="url(#cookieBotMinutesGradient)"
                  strokeWidth={1.5}
                  isAnimationActive={false}
                />
              </AreaChart>
            </ResponsiveContainer>
          )}
        </div>
      </div>

      <FadingScrollArea
        className="min-h-0 flex-1"
        style={{ "--scroll-fade-top-offset": "32px" } as React.CSSProperties}
      >
        <Table
          className="w-full table-fixed"
          containerClassName="overflow-visible"
        >
          <TableHeader className="sticky top-0 z-10 bg-background">
            <TableRow>
              <TableHead className="max-w-0">
                {t("cookieBot.enrolled.columnProfile")}
              </TableHead>
              <TableHead className="hidden w-32 @2xl:table-cell">
                {t("cookieBot.enrolled.columnCadence")}
              </TableHead>
              <TableHead className="w-20">
                {t("cookieBot.enrolled.columnTime")}
              </TableHead>
              <TableHead className="hidden w-40 @3xl:table-cell">
                {t("cookieBot.enrolled.columnNextRun")}
              </TableHead>
              <TableHead className="hidden w-40 @4xl:table-cell">
                {t("cookieBot.enrolled.columnLastRun")}
              </TableHead>
              <TableHead className="w-20" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {isLoading && schedules.length === 0
              ? Array.from({ length: 5 }, (_, i) => (
                  <TableRow key={`enrolled-skeleton-${i}`}>
                    <TableCell colSpan={6}>
                      <div className="flex items-center gap-3">
                        <Skeleton
                          className="h-3"
                          style={{ width: `${30 + ((i * 17) % 40)}%` }}
                        />
                        <div className="flex-1" />
                        <Skeleton className="h-3 w-16" />
                        <Skeleton className="h-3 w-10" />
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              : schedules.map((schedule) => (
                  <EnrolledRow
                    key={`${schedule.owner_user_id ?? "me"}-${schedule.profile_id}`}
                    schedule={schedule}
                    mine={
                      !schedule.owner_user_id ||
                      schedule.owner_user_id === currentUserId
                    }
                    exhausted={exhausted}
                    onEdit={onEditSchedule}
                    onRemove={onRemoveSchedule}
                  />
                ))}
          </TableBody>
        </Table>
      </FadingScrollArea>
    </div>
  );
}

function RecentSegment({
  label,
  tone,
  onClick,
}: {
  label: string;
  tone: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "cursor-pointer tabular-nums underline-offset-2 transition-colors duration-100 hover:underline",
        tone,
      )}
    >
      {label}
    </button>
  );
}

function TonightStrip({
  live,
  nextAt,
  nextMinute,
  dueCount,
  profileIndex,
}: {
  live: RemoteSessionState[];
  nextAt: Date | null;
  nextMinute: number | null;
  dueCount: number;
  profileIndex: Map<string, BrowserProfile>;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const running = live.length > 0;

  return (
    <div className="flex shrink-0 items-center gap-3 rounded-md border border-border bg-card px-3 py-2.5">
      <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
        {running ? t("cookieBot.running.label") : t("cookieBot.tonight.label")}
      </span>
      <AnimatePresence mode="wait" initial={false}>
        <motion.div
          key={running ? `running-${live.length}` : "idle"}
          initial={{ opacity: reduceMotion ? 1 : 0.55 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: reduceMotion ? 0.01 : 0.12 }}
          className="flex min-w-0 flex-1 items-center gap-2"
        >
          {running ? (
            <>
              <StatusDot
                tone={sessionTone(live[0])}
                pulse={live[0].state === "provisioning"}
              />
              <span className="min-w-0 truncate text-sm font-medium text-foreground">
                {sessionDisplayName(live[0], profileIndex, undefined) ??
                  t("cookieBot.live.unnamedSession")}
              </span>
              <span className="shrink-0 text-sm text-muted-foreground">
                {sessionPhaseLabel(t, live[0])}
              </span>
              {live.length > 1 && (
                <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                  {t("cookieBot.running.more", { count: live.length - 1 })}
                </span>
              )}
            </>
          ) : nextMinute === null ? (
            <span className="text-sm text-muted-foreground">
              {t("cookieBot.tonight.nothingScheduled")}
            </span>
          ) : (
            <span className="text-sm tabular-nums text-foreground">
              {t("cookieBot.tonight.nextRun", {
                time: minutesToClock(nextMinute),
              })}
              {nextAt ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="ml-2 cursor-default text-muted-foreground">
                      {t("cookieBot.tonight.dueCount", { count: dueCount })}
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>
                    {formatDateTime(nextAt.toISOString()) ?? ""}
                  </TooltipContent>
                </Tooltip>
              ) : (
                <span className="ml-2 text-muted-foreground">
                  {t("cookieBot.tonight.dueCount", { count: dueCount })}
                </span>
              )}
            </span>
          )}
        </motion.div>
      </AnimatePresence>
    </div>
  );
}

function EnrolledRow({
  schedule,
  mine,
  exhausted,
  onEdit,
  onRemove,
}: {
  schedule: CookieBotSchedule;
  mine: boolean;
  exhausted: boolean;
  onEdit: (schedule: CookieBotSchedule) => void;
  onRemove: (schedule: CookieBotSchedule) => void;
}) {
  const { t } = useTranslation();
  const blocked = scheduleBlockedReason(t, schedule);

  return (
    <TableRow className="hover:bg-muted/30">
      <TableCell className="max-w-0 truncate">
        <span className="flex items-center gap-2">
          <StatusDot tone={scheduleTone(schedule)} />
          <span className="min-w-0 truncate">{schedule.profile_name}</span>
          {!mine && schedule.owner_email && (
            <span className="shrink-0 text-[10px] uppercase tracking-wide text-muted-foreground">
              {schedule.owner_email}
            </span>
          )}
        </span>
      </TableCell>
      <TableCell className="hidden text-muted-foreground @2xl:table-cell">
        {describeCadence(t, schedule.days_mask)}
      </TableCell>
      <TableCell className="tabular-nums">
        {minutesToClock(schedule.run_at_minute)}
      </TableCell>
      {/* The server publishes why tonight would be refused on every read. A
          next-run time the enrolment cannot keep is worse than no time. */}
      <TableCell className="hidden tabular-nums text-muted-foreground @3xl:table-cell">
        {blocked ? (
          <span className="text-warning-text">{blocked}</span>
        ) : exhausted ? (
          <span className="text-warning-text">
            {t("cookieBot.enrolled.pausedNoHours")}
          </span>
        ) : (
          (formatDateTime(schedule.next_run_at) ?? "—")
        )}
      </TableCell>
      <TableCell className="hidden tabular-nums text-muted-foreground @4xl:table-cell">
        {formatDateTime(schedule.last_run_at) ??
          t("cookieBot.enrolled.neverRun")}
      </TableCell>
      <TableCell>
        <div className="flex items-center justify-end gap-0.5">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-7"
                aria-label={t("cookieBot.enrolled.edit")}
                onClick={() => {
                  onEdit(schedule);
                }}
              >
                <LuPencil className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{t("cookieBot.enrolled.edit")}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="size-7 text-destructive-text hover:bg-destructive/10"
                aria-label={t("cookieBot.schedule.unenrol")}
                onClick={() => {
                  onRemove(schedule);
                }}
              >
                <LuTrash2 className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{t("cookieBot.schedule.unenrol")}</TooltipContent>
          </Tooltip>
        </div>
      </TableCell>
    </TableRow>
  );
}

/* -------------------------------------------------------------------------- */
/* Derivations — all of these read run rows the server wrote. Nothing here     */
/* predicts, estimates or fills in a value the backend did not report.        */
/* -------------------------------------------------------------------------- */

function summariseRecent(runs: CookieBotRun[]) {
  const cutoff = Date.now() - 24 * 60 * 60 * 1000;
  let succeeded = 0;
  let partial = 0;
  let failed = 0;
  let total = 0;
  for (const run of runs) {
    const at = parseIso(run.started_at ?? run.scheduled_for);
    if (!at || at.getTime() < cutoff) continue;
    total += 1;
    if (run.status === "succeeded") succeeded += 1;
    else if (run.status === "partial" || run.status === "skipped") partial += 1;
    else if (run.status === "failed") failed += 1;
  }
  return { succeeded, partial, failed, total };
}

function nightlyMinutes(runs: CookieBotRun[]) {
  const buckets = new Map<string, number>();
  const days: { key: string; label: string }[] = [];
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  for (let i = CHART_NIGHTS - 1; i >= 0; i -= 1) {
    const day = new Date(today);
    day.setDate(day.getDate() - i);
    const key = dayKey(day);
    buckets.set(key, 0);
    days.push({
      key,
      label: day.toLocaleDateString(undefined, {
        day: "numeric",
        month: "short",
      }),
    });
  }
  for (const run of runs) {
    const at = parseIso(run.started_at ?? run.scheduled_for);
    if (!at) continue;
    const key = dayKey(at);
    if (!buckets.has(key)) continue;
    buckets.set(key, (buckets.get(key) ?? 0) + run.billed_seconds / 60);
  }
  return days.map(({ key, label }) => ({
    label,
    minutes: Math.round(buckets.get(key) ?? 0),
  }));
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

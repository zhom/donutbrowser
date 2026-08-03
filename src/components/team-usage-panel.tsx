"use client";

import { motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { useTranslation } from "react-i18next";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type {
  NameType,
  ValueType,
} from "recharts/types/component/DefaultTooltipContent";
import type { TooltipContentProps } from "recharts/types/component/Tooltip";
import { formatHours, RemoteHoursMeter } from "@/components/cookie-bot-shared";
import { AnimatedDisclosureItem } from "@/components/ui/animated-disclosure";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { translateBackendError } from "@/lib/backend-errors";
import {
  type CookieBotUsage,
  type CookieBotUsageMember,
  getCookieBotUsage,
  type RemoteHoursQuota,
} from "@/lib/cookie-bot";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { cn } from "@/lib/utils";

/** How many past billing periods the selector offers. */
const PERIOD_COUNT = 6;

function periodKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`;
}

function recentPeriods(): { value: string; label: string }[] {
  const now = new Date();
  return Array.from({ length: PERIOD_COUNT }, (_, index) => {
    const date = new Date(now.getFullYear(), now.getMonth() - index, 1);
    return {
      value: periodKey(date),
      label: date.toLocaleDateString(undefined, {
        month: "long",
        year: "numeric",
      }),
    };
  });
}

/** The part of an address that identifies the person, for a dense axis. */
function shortName(email: string): string {
  const local = email.split("@")[0] ?? email;
  return local.length > 14 ? `${local.slice(0, 13)}…` : local;
}

interface MemberDatum {
  name: string;
  email: string;
  bot: number;
  interactive: number;
  total: number;
  runs: number;
  /** How many of those runs did not do what they were asked. */
  runsFailed: number;
  sessions: number;
}

interface TeamUsagePanelProps {
  /**
   * The live pooled budget. Only used before the selected period's own figures
   * arrive, so the block is never empty on first paint; once `usage` lands the
   * period's numbers win, because looking at June must show what June allowed
   * rather than what is left today.
   */
  quota?: RemoteHoursQuota | null;
  className?: string;
}

/**
 * Who spent the pooled remote hours this period.
 *
 * The account page owns plan truth — what was bought — and this is the other
 * half of that: what it was spent on and by whom. Every figure is served; the
 * desktop computes no allowance and no share of one.
 */
export function TeamUsagePanel({ quota, className }: TeamUsagePanelProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const periods = React.useMemo(() => recentPeriods(), []);
  const [period, setPeriod] = React.useState(periods[0].value);
  const [usage, setUsage] = React.useState<CookieBotUsage | null>(null);
  const [isLoading, setIsLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let active = true;
    setIsLoading(true);
    setError(null);
    void getCookieBotUsage(period)
      .then((result) => {
        if (active) setUsage(result);
      })
      .catch((err: unknown) => {
        if (active) {
          setUsage(null);
          setError(translateBackendError(t as never, err));
        }
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [period, t]);

  // Heaviest first: the whole point of the view is to make the biggest
  // consumer the first thing read, both in the chart and in the table.
  const members: MemberDatum[] = React.useMemo(() => {
    if (!usage) return [];
    return [...usage.members]
      .sort(
        (a: CookieBotUsageMember, b: CookieBotUsageMember) =>
          b.used_hours - a.used_hours,
      )
      .map((member) => ({
        name: shortName(member.email),
        email: member.email,
        bot: member.bot_hours,
        interactive: member.interactive_hours,
        total: member.used_hours,
        runs: member.bot_runs,
        runsFailed: member.bot_runs_failed,
        sessions: member.sessions,
      }));
  }, [usage]);

  const heaviest = members[0]?.total ?? 0;
  const isSolo = members.length <= 1;

  // The meter takes a quota shape; the usage response carries the same numbers
  // for the period being looked at, so it is adapted rather than
  // re-implemented. The live quota is only the stand-in until it arrives.
  const pooled: RemoteHoursQuota | null = React.useMemo(
    () =>
      usage
        ? {
            granted_hours: usage.granted_hours,
            used_hours: usage.used_hours,
            remaining_hours: usage.remaining_hours,
            period_start: usage.period_start,
            period_end: usage.period_end,
            team_id: usage.team_id,
            seats: usage.seats,
            per_seat_hours: 0,
            members: [],
          }
        : (quota ?? null),
    [usage, quota],
  );

  const renderTooltip = React.useCallback(
    ({ active, payload }: TooltipContentProps<ValueType, NameType>) => {
      if (!active || !payload || payload.length === 0) return null;
      const datum = payload[0].payload as MemberDatum;
      return (
        <div className="rounded-md border border-border bg-popover px-2.5 py-2 text-xs text-popover-foreground shadow-sm">
          <p className="font-medium">{datum.email}</p>
          <p className="mt-1 flex items-center justify-between gap-4 tabular-nums">
            <span className="text-chart-1">
              {t("cookieBot.team.legendBot")}
            </span>
            <span>
              {t("cookieBot.team.hours", { hours: formatHours(datum.bot) })}
            </span>
          </p>
          <p className="flex items-center justify-between gap-4 tabular-nums">
            <span className="text-chart-2">
              {t("cookieBot.team.legendInteractive")}
            </span>
            <span>
              {t("cookieBot.team.hours", {
                hours: formatHours(datum.interactive),
              })}
            </span>
          </p>
        </div>
      );
    },
    [t],
  );

  return (
    <div className={cn("flex flex-col gap-3", className)}>
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-medium">{t("cookieBot.team.title")}</h3>
        <Select
          value={period}
          onValueChange={(value) => {
            setPeriod(value);
          }}
        >
          <SelectTrigger className="h-8 w-[150px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {periods.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* Driven by the selected period's own figures, not by the live quota:
          choosing June must show what June was allowed, not what is left
          today. */}
      <div className="flex flex-col gap-1.5">
        {pooled ? (
          <p className="text-xs tabular-nums text-muted-foreground">
            {t("cookieBot.team.pooled", {
              used: formatHours(pooled.used_hours),
              total: formatHours(pooled.granted_hours),
              // `count` (not `seats`) so i18next can pluralise: "1 seat" and
              // "across 4 seats" are different sentences in most locales.
              count: pooled.seats,
            })}
          </p>
        ) : (
          <Skeleton className="h-3 w-56" />
        )}
        <RemoteHoursMeter
          quota={pooled}
          isLoading={isLoading && usage === null}
          variant="inline"
        />
      </div>

      {error ? (
        <p className="py-6 text-center text-xs text-destructive-text">
          {error}
        </p>
      ) : isLoading && !usage ? (
        <div className="space-y-2">
          <Skeleton className="h-[180px] w-full" />
          <Skeleton className="h-6 w-full" />
          <Skeleton className="h-6 w-full" />
        </div>
      ) : members.length === 0 ? (
        <p className="py-6 text-center text-xs text-muted-foreground">
          {t("cookieBot.team.noActivity")}
        </p>
      ) : (
        <>
          {!isSolo && (
            <>
              <div className="h-[clamp(160px,22vh,240px)] w-full">
                <ResponsiveContainer
                  width="100%"
                  height="100%"
                  minWidth={1}
                  minHeight={1}
                >
                  <AreaChart
                    data={members}
                    margin={{ top: 8, right: 8, bottom: 0, left: 0 }}
                  >
                    <defs>
                      <linearGradient
                        id="cookieBotHoursGradient"
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
                      <linearGradient
                        id="interactiveHoursGradient"
                        x1="0"
                        y1="0"
                        x2="0"
                        y2="1"
                      >
                        <stop
                          offset="0%"
                          stopColor="var(--chart-2)"
                          stopOpacity={0.5}
                        />
                        <stop
                          offset="100%"
                          stopColor="var(--chart-2)"
                          stopOpacity={0.1}
                        />
                      </linearGradient>
                    </defs>
                    <CartesianGrid
                      strokeDasharray="3 3"
                      className="stroke-muted"
                    />
                    <XAxis
                      dataKey="name"
                      className="text-xs"
                      tick={{ fill: "var(--muted-foreground)" }}
                      interval={0}
                    />
                    <YAxis
                      className="text-xs"
                      tick={{ fill: "var(--muted-foreground)" }}
                      width={40}
                    />
                    <Tooltip content={renderTooltip} />
                    {/* `linear` because the axis is a ranking, not time: a
                        monotone curve would invent values between people. */}
                    <Area
                      type="linear"
                      dataKey="bot"
                      stackId="1"
                      stroke="var(--chart-1)"
                      fill="url(#cookieBotHoursGradient)"
                      strokeWidth={1.5}
                      isAnimationActive={false}
                    />
                    <Area
                      type="linear"
                      dataKey="interactive"
                      stackId="1"
                      stroke="var(--chart-2)"
                      fill="url(#interactiveHoursGradient)"
                      strokeWidth={1.5}
                      isAnimationActive={false}
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>

              <div className="flex items-center justify-center gap-6">
                <div className="flex items-center gap-2">
                  <div
                    className="size-2.5 rounded"
                    style={{ backgroundColor: "var(--chart-1)" }}
                  />
                  <span className="text-xs text-muted-foreground">
                    {t("cookieBot.team.legendBot")}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <div
                    className="size-2.5 rounded"
                    style={{ backgroundColor: "var(--chart-2)" }}
                  />
                  <span className="text-xs text-muted-foreground">
                    {t("cookieBot.team.legendInteractive")}
                  </span>
                </div>
              </div>
            </>
          )}

          {isSolo && (
            <p className="text-xs text-muted-foreground">
              {t("cookieBot.team.soloNote")}
            </p>
          )}

          <div className="overflow-hidden rounded-md border border-border">
            <div className="grid grid-cols-[1fr_auto_auto_5rem] items-center gap-3 border-b border-border bg-muted/40 px-3 py-1.5 text-[10px] tracking-wide text-muted-foreground uppercase">
              <span>{t("cookieBot.team.columnMember")}</span>
              <span className="text-right">
                {t("cookieBot.team.columnRuns")}
              </span>
              <span className="text-right">
                {t("cookieBot.team.columnHours")}
              </span>
              <span className="text-right">
                {t("cookieBot.team.columnShare")}
              </span>
            </div>
            {members.map((member, index) => (
              // The ranking genuinely re-orders when the period changes, so the
              // rows travel to their new places instead of teleporting. Layout
              // only — the row is fully rendered and readable on first paint.
              <AnimatedDisclosureItem
                key={member.email}
                className="grid grid-cols-[1fr_auto_auto_5rem] items-center gap-3 px-3 py-1.5 text-xs"
              >
                <span
                  className={cn(
                    "min-w-0 truncate",
                    index === 0 ? "font-medium text-foreground" : "",
                  )}
                  title={member.email}
                >
                  {member.email}
                </span>
                {/* The failure count is already on the wire and answers the
                    question the run count cannot: whether the hours bought
                    anything. */}
                <span className="text-right tabular-nums text-muted-foreground">
                  {member.runs}
                  {member.runsFailed > 0 && (
                    <span className="ml-1 text-warning-text">
                      {t("cookieBot.team.runsFailed", {
                        n: member.runsFailed,
                      })}
                    </span>
                  )}
                </span>
                <span
                  className={cn(
                    "text-right tabular-nums",
                    index === 0
                      ? "font-semibold text-foreground"
                      : "text-muted-foreground",
                  )}
                >
                  {t("cookieBot.team.hours", {
                    hours: formatHours(member.total),
                  })}
                </span>
                <span className="h-1 overflow-hidden rounded-full bg-muted">
                  {/* Radius on the track, scale on the fill: a rounded cap that
                      is being scaled flips shape mid-transition. `initial=
                      {false}` keeps the first paint at the true share. */}
                  <motion.span
                    initial={false}
                    animate={{
                      scaleX: heaviest > 0 ? member.total / heaviest : 0,
                    }}
                    transition={
                      reduceMotion
                        ? { duration: 0 }
                        : { duration: 0.22, ease: MOTION_EASE_OUT }
                    }
                    style={{ transformOrigin: "left", willChange: "transform" }}
                    className={cn(
                      "block h-full w-full rounded-full",
                      index === 0 ? "bg-foreground" : "bg-muted-foreground",
                    )}
                  />
                </span>
              </AnimatedDisclosureItem>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

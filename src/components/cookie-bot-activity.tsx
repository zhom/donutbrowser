"use client";

import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuSearch } from "react-icons/lu";
import {
  formatDateTime,
  formatDuration,
  formatElapsed,
  hasRunCounters,
  indexProfiles,
  indexRunsById,
  indexRunsBySession,
  outcomeLabel,
  parseIso,
  runStatusLabel,
  runStatusTone,
  StatusDot,
  sessionCloseReason,
  sessionDisplayName,
  sessionElapsedSeconds,
  sessionPhaseLabel,
  sessionTone,
  useSecondTicker,
} from "@/components/cookie-bot-shared";
import { Button } from "@/components/ui/button";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
import { translateBackendError } from "@/lib/backend-errors";
import { type CookieBotRun, cancelCookieBotRun } from "@/lib/cookie-bot";
import { MOTION_EASE_OUT } from "@/lib/motion";
import {
  type RemoteSessionState,
  stopRemoteSession,
} from "@/lib/remote-sessions";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile } from "@/types";

export type RunFilter = "all" | "succeeded" | "partial" | "failed";

interface CookieBotActivityProps {
  live: RemoteSessionState[];
  streamConnected: boolean;
  runs: CookieBotRun[];
  isLoading: boolean;
  profiles: BrowserProfile[];
  showOperator: boolean;
  filter: RunFilter;
  onFilterChange: (filter: RunFilter) => void;
  onRefresh: () => void;
}

export function CookieBotActivity({
  live,
  streamConnected,
  runs,
  isLoading,
  profiles,
  showOperator,
  filter,
  onFilterChange,
  onRefresh,
}: CookieBotActivityProps) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");

  const profileIndex = useMemo(() => indexProfiles(profiles), [profiles]);
  const runsBySession = useMemo(() => indexRunsBySession(runs), [runs]);
  const runsById = useMemo(() => indexRunsById(runs), [runs]);

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return runs.filter((run) => {
      if (filter !== "all" && run.status !== filter) return false;
      if (!needle) return true;
      const name = run.profile_name ?? profileIndex.get(run.profile_id)?.name;
      return (
        (name ?? "").toLowerCase().includes(needle) ||
        (run.email ?? "").toLowerCase().includes(needle)
      );
    });
  }, [runs, filter, search, profileIndex]);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <LiveSessions
        live={live}
        streamConnected={streamConnected}
        profileIndex={profileIndex}
        runsBySession={runsBySession}
        runsById={runsById}
        onChanged={onRefresh}
      />

      <div className="flex shrink-0 items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <LuSearch className="absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
            }}
            className="h-8 pl-8 text-sm"
            placeholder={t("cookieBot.history.searchPlaceholder")}
          />
        </div>
        <Select
          value={filter}
          onValueChange={(value) => {
            onFilterChange(value as RunFilter);
          }}
        >
          <SelectTrigger className="h-8 w-[150px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">
              {t("cookieBot.history.filterAll")}
            </SelectItem>
            <SelectItem value="succeeded">
              {t("cookieBot.history.filterComplete")}
            </SelectItem>
            <SelectItem value="partial">
              {t("cookieBot.history.filterPartial")}
            </SelectItem>
            <SelectItem value="failed">
              {t("cookieBot.history.filterFailed")}
            </SelectItem>
          </SelectContent>
        </Select>
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
              <TableHead className="w-40">
                {t("cookieBot.history.columnStarted")}
              </TableHead>
              <TableHead className="max-w-0">
                {t("cookieBot.history.columnProfile")}
              </TableHead>
              <TableHead className="hidden w-24 @2xl:table-cell">
                {t("cookieBot.history.columnDuration")}
              </TableHead>
              <TableHead className="hidden w-20 text-right @3xl:table-cell">
                {t("cookieBot.history.columnSites")}
              </TableHead>
              <TableHead className="w-32">
                {t("cookieBot.history.columnStatus")}
              </TableHead>
              {showOperator && (
                <TableHead className="hidden max-w-0 @4xl:table-cell">
                  {t("cookieBot.history.columnOperator")}
                </TableHead>
              )}
            </TableRow>
          </TableHeader>
          <TableBody>
            {isLoading && runs.length === 0 ? (
              Array.from({ length: 6 }, (_, i) => (
                <TableRow key={`skeleton-${i}`}>
                  <TableCell colSpan={showOperator ? 6 : 5}>
                    <div className="flex items-center gap-3">
                      <Skeleton className="h-3 w-28" />
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
            ) : filtered.length === 0 ? (
              <TableRow className="border-0! hover:bg-transparent">
                <TableCell colSpan={showOperator ? 6 : 5} className="py-16">
                  <p className="text-center text-sm text-muted-foreground">
                    {runs.length === 0
                      ? t("cookieBot.history.empty")
                      : t("cookieBot.history.noMatch")}
                  </p>
                </TableCell>
              </TableRow>
            ) : (
              filtered.map((run) => (
                <RunRow
                  key={run.id}
                  run={run}
                  profileName={
                    run.profile_name ??
                    profileIndex.get(run.profile_id)?.name ??
                    null
                  }
                  showOperator={showOperator}
                />
              ))
            )}
          </TableBody>
        </Table>
      </FadingScrollArea>
    </div>
  );
}

function RunRow({
  run,
  profileName,
  showOperator,
}: {
  run: CookieBotRun;
  profileName: string | null;
  showOperator: boolean;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const reduceMotion = useReducedMotion();

  const started = parseIso(run.started_at);
  const ended = parseIso(run.ended_at);
  const durationSeconds =
    started && ended
      ? Math.max(0, Math.floor((ended.getTime() - started.getTime()) / 1000))
      : run.billed_seconds > 0
        ? run.billed_seconds
        : null;

  const countersKnown = hasRunCounters(run);
  const hasDetail = Boolean(run.outcome_code) || run.sites_failed > 0;

  return (
    <>
      <TableRow
        className={cn("hover:bg-muted/30", hasDetail && "cursor-pointer")}
        onClick={() => {
          if (hasDetail) setExpanded((open) => !open);
        }}
      >
        <TableCell className="tabular-nums text-muted-foreground">
          {formatDateTime(run.started_at ?? run.scheduled_for) ?? "—"}
        </TableCell>
        <TableCell className="max-w-0 truncate">
          {profileName ?? t("cookieBot.history.unknownProfile")}
        </TableCell>
        <TableCell className="hidden tabular-nums @2xl:table-cell">
          {durationSeconds === null ? "—" : formatDuration(t, durationSeconds)}
        </TableCell>
        {/* An em dash, not a confident `0/12`: the counters are not written
            until the fleet's figures are ingested, and printing the column
            default as a fact tells a paying user their run did nothing. */}
        <TableCell className="hidden text-right tabular-nums text-muted-foreground @3xl:table-cell">
          {!countersKnown ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="cursor-default">—</span>
              </TooltipTrigger>
              <TooltipContent>
                {t("cookieBot.history.sitesUnknown")}
              </TooltipContent>
            </Tooltip>
          ) : run.sites_total > 0 ? (
            `${run.sites_visited}/${run.sites_total}`
          ) : (
            String(run.sites_visited)
          )}
        </TableCell>
        <TableCell>
          <span className="flex items-center gap-2 text-xs">
            <StatusDot tone={runStatusTone(run.status)} />
            {runStatusLabel(t, run.status)}
          </span>
        </TableCell>
        {showOperator && (
          <TableCell className="hidden max-w-0 truncate text-muted-foreground @4xl:table-cell">
            {run.email ?? "—"}
          </TableCell>
        )}
      </TableRow>
      {hasDetail && (
        <TableRow className="border-0! hover:bg-transparent">
          <TableCell
            colSpan={showOperator ? 6 : 5}
            className={cn("p-0", !expanded && "border-0!")}
          >
            <AnimatePresence initial={false}>
              {expanded && (
                <motion.div
                  initial={{ opacity: 0, y: reduceMotion ? 0 : -4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: reduceMotion ? 0 : -4 }}
                  transition={{
                    duration: reduceMotion ? 0.15 : 0.16,
                    ease: MOTION_EASE_OUT,
                  }}
                  className="flex flex-col gap-1 px-2 pb-3 text-xs text-muted-foreground"
                >
                  {run.outcome_code && (
                    <span>
                      {t("cookieBot.history.outcome", {
                        reason: outcomeLabel(t, run.outcome_code) ?? "",
                      })}
                    </span>
                  )}
                  {run.sites_failed > 0 && (
                    <span>
                      {t("cookieBot.history.sitesFailed", {
                        count: run.sites_failed,
                      })}
                    </span>
                  )}
                  {run.consent_dismissed > 0 && (
                    <span>
                      {t("cookieBot.history.consentHandled", {
                        count: run.consent_dismissed,
                      })}
                    </span>
                  )}
                </motion.div>
              )}
            </AnimatePresence>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

/* -------------------------------------------------------------------------- */
/* Live                                                                       */
/* -------------------------------------------------------------------------- */

function LiveSessions({
  live,
  streamConnected,
  profileIndex,
  runsBySession,
  runsById,
  onChanged,
}: {
  live: RemoteSessionState[];
  streamConnected: boolean;
  profileIndex: Map<string, BrowserProfile>;
  runsBySession: Map<string, CookieBotRun>;
  runsById: Map<string, CookieBotRun>;
  onChanged: () => void;
}) {
  const { t } = useTranslation();
  const now = useSecondTicker(live.length > 0);

  if (live.length === 0) {
    return (
      <div className="flex shrink-0 items-center gap-2 rounded-md border border-border bg-card px-3 py-2.5">
        <StatusDot tone="muted" />
        <span className="text-sm text-muted-foreground">
          {streamConnected
            ? t("cookieBot.live.idle")
            : t("cookieBot.live.streamOffline")}
        </span>
      </div>
    );
  }

  return (
    <div className="flex shrink-0 flex-col gap-2">
      {!streamConnected && (
        <p className="text-xs text-warning-text">
          {t("cookieBot.live.streamOfflineDetail")}
        </p>
      )}
      {live.map((session) => (
        <LiveSessionRow
          key={session.session_id}
          session={session}
          now={now}
          name={sessionDisplayName(
            session,
            profileIndex,
            session.run_id
              ? runsById.get(session.run_id)
              : runsBySession.get(session.session_id),
          )}
          run={
            session.run_id
              ? runsById.get(session.run_id)
              : runsBySession.get(session.session_id)
          }
          onChanged={onChanged}
        />
      ))}
    </div>
  );
}

function LiveSessionRow({
  session,
  now,
  name,
  run,
  onChanged,
}: {
  session: RemoteSessionState;
  now: number;
  name: string | null;
  run: CookieBotRun | undefined;
  onChanged: () => void;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const [isStopping, setIsStopping] = useState(false);

  const elapsed = sessionElapsedSeconds(session, now);
  const phase = sessionPhaseLabel(t, session);
  const tone = sessionTone(session);
  const closeReason = sessionCloseReason(t, session);
  // Only once the backend has actually written a counter. Until then the bar
  // sat at zero for the whole run and read as "nothing is happening".
  const countersKnown = run ? hasRunCounters(run) : false;
  const total = run?.sites_total ?? 0;
  const visited = run?.sites_visited ?? 0;
  const progress =
    countersKnown && total > 0 ? Math.min(1, visited / total) : null;
  // A night longer than one session's cap is split into chunks, and the run row
  // is the only place that can say which one is running. `chunk_index` counts
  // chunks STARTED — the server bumps it as it launches each one and treats 0
  // as "never got going" — so it already reads as a 1-based position and must
  // not be incremented again.
  const chunks =
    run && run.chunks_total > 1 && run.chunk_index > 0
      ? t("cookieBot.live.chunk", {
          index: Math.min(run.chunk_index, run.chunks_total),
          total: run.chunks_total,
        })
      : null;

  const stop = useCallback(async () => {
    setIsStopping(true);
    try {
      if (session.run_id) {
        await cancelCookieBotRun(session.run_id);
      } else {
        await stopRemoteSession(session.session_id);
      }
      showSuccessToast(t("cookieBot.running.stopped"));
      onChanged();
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsStopping(false);
    }
  }, [session.run_id, session.session_id, onChanged, t]);

  return (
    <div className="flex flex-col gap-2 rounded-md border border-border bg-card px-3 py-2.5">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <StatusDot tone={tone} pulse={session.state === "provisioning"} />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
          {name ?? t("cookieBot.live.unnamedSession")}
        </span>

        {/* The phase swaps in place: the words change, the row does not move.
            The slot is a fixed width so the elapsed clock beside it never
            shifts, and the entering label starts at 0.55 rather than 0 — if
            the animation never runs, the single most important live signal on
            the screen is still legible. */}
        <span className="w-36 shrink-0 text-right">
          <AnimatePresence mode="wait" initial={false}>
            <motion.span
              key={phase}
              initial={{ opacity: reduceMotion ? 1 : 0.55 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: reduceMotion ? 0.01 : 0.12 }}
              className="block truncate text-xs text-muted-foreground"
            >
              {phase}
            </motion.span>
          </AnimatePresence>
        </span>

        <span className="shrink-0 text-xs tabular-nums text-foreground">
          {elapsed === null ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="cursor-default text-muted-foreground">—</span>
              </TooltipTrigger>
              <TooltipContent>
                {t("cookieBot.live.notStartedYet")}
              </TooltipContent>
            </Tooltip>
          ) : (
            formatElapsed(elapsed)
          )}
        </span>

        <Button
          variant="outline"
          size="sm"
          className="h-7 shrink-0 text-xs"
          disabled={isStopping}
          onClick={() => {
            void stop();
          }}
        >
          {t("cookieBot.running.stop")}
        </Button>
      </div>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span className="tabular-nums">
          {countersKnown && total > 0
            ? t("cookieBot.live.sitesProgress", { visited, total })
            : t("cookieBot.live.sitesUnknown")}
        </span>
        <span className="tabular-nums">
          {countersKnown && run
            ? t("cookieBot.live.consentHandled", {
                count: run.consent_dismissed,
              })
            : t("cookieBot.live.consentUnknown")}
        </span>
        <span className="tabular-nums">
          {session.billed_seconds !== null &&
          session.billed_seconds !== undefined
            ? t("cookieBot.live.billed", {
                duration: formatElapsed(session.billed_seconds),
              })
            : t("cookieBot.live.billedUnknown")}
        </span>
        {chunks && <span className="tabular-nums">{chunks}</span>}
        {closeReason && (
          <span className="text-destructive-text">{closeReason}</span>
        )}
      </div>

      {progress !== null && (
        <div className="h-1 overflow-hidden rounded-full bg-muted">
          <motion.div
            initial={false}
            animate={{ scaleX: progress }}
            transition={
              reduceMotion
                ? { duration: 0 }
                : { duration: 0.22, ease: MOTION_EASE_OUT }
            }
            style={{ transformOrigin: "left", willChange: "transform" }}
            className="h-full w-full bg-success"
          />
        </div>
      )}
    </div>
  );
}

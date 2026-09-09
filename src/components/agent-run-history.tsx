"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentRunPanel } from "@/components/agent-run-view";
import { agentStatusLabel, agentStatusTone } from "@/components/agent-shared";
import { formatDateTime, StatusDot } from "@/components/cookie-bot-shared";
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
import { type AgentRunView, getAgentRuns } from "@/lib/agent";
import { translateBackendError } from "@/lib/backend-errors";
import type { BrowserProfile } from "@/types";

const PAGE_SIZE = 25;

interface AgentRunHistoryProps {
  profiles: BrowserProfile[];
  /** Reported so the page can tell a cloud misconfiguration from an outage. */
  onLoadError?: (error: unknown) => void;
}

/**
 * Every run this account has, newest first.
 *
 * Remounted by the page (through a `key`) whenever something is written, rather
 * than taking a "reload me" prop: a write invalidates the pages already loaded
 * as well as the first one, so starting over is the correct response and a
 * fresh mount is the honest way to say so.
 */
export function AgentRunHistory({
  profiles,
  onLoadError,
}: AgentRunHistoryProps) {
  const { t } = useTranslation();
  const [runs, setRuns] = useState<AgentRunView[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  const profileNames = useMemo(() => {
    const index = new Map<string, string>();
    for (const profile of profiles) index.set(profile.id, profile.name);
    return index;
  }, [profiles]);

  const loadFirstPage = useCallback(async () => {
    setIsLoading(true);
    try {
      const page = await getAgentRuns({ limit: PAGE_SIZE });
      setRuns(page.runs);
      setCursor(page.nextCursor ?? null);
      setError(null);
    } catch (loadError) {
      setError(loadError);
      onLoadError?.(loadError);
    } finally {
      setIsLoading(false);
    }
  }, [onLoadError]);

  const loadNextPage = useCallback(async () => {
    if (!cursor) return;
    setIsLoadingMore(true);
    try {
      const page = await getAgentRuns({ limit: PAGE_SIZE, cursor });
      // De-duplicated by id rather than blindly appended: a run that finished
      // between the two reads shifts the keyset, and the same row arriving
      // twice would render twice.
      setRuns((prev) => {
        const seen = new Set(prev.map((run) => run.id));
        return [...prev, ...page.runs.filter((run) => !seen.has(run.id))];
      });
      setCursor(page.nextCursor ?? null);
    } catch (loadError) {
      setError(loadError);
    } finally {
      setIsLoadingMore(false);
    }
  }, [cursor]);

  useEffect(() => {
    void loadFirstPage();
  }, [loadFirstPage]);

  if (selectedRunId !== null) {
    return (
      <AgentRunPanel
        runId={selectedRunId}
        profiles={profiles}
        backLabel={t("agent.history.backToList")}
        onBack={() => {
          setSelectedRunId(null);
        }}
        onChanged={() => {
          void loadFirstPage();
        }}
      />
    );
  }

  return (
    <div
      data-slot="agent-run-history"
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      {error !== null && (
        <div className="flex shrink-0 items-center gap-3 rounded-md border border-destructive/50 bg-destructive/10 p-3">
          <p className="min-w-0 flex-1 text-sm text-destructive-text">
            {translateBackendError(t, error)}
          </p>
          <RippleButton
            variant="outline"
            size="sm"
            onClick={() => {
              void loadFirstPage();
            }}
          >
            {t("common.buttons.retry")}
          </RippleButton>
        </div>
      )}

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
                {t("agent.history.columnStarted")}
              </TableHead>
              <TableHead className="hidden w-40 @2xl:table-cell">
                {t("agent.history.columnProfile")}
              </TableHead>
              <TableHead className="max-w-0">
                {t("agent.history.columnGoal")}
              </TableHead>
              <TableHead className="hidden w-24 @3xl:table-cell">
                {t("agent.history.columnTarget")}
              </TableHead>
              <TableHead className="w-32">
                {t("agent.history.columnStatus")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {isLoading && runs.length === 0 ? (
              Array.from({ length: 6 }, (_, index) => (
                <TableRow key={`agent-run-skeleton-${index}`}>
                  <TableCell colSpan={5}>
                    <div className="flex items-center gap-3">
                      <Skeleton className="h-3 w-28" />
                      <Skeleton
                        className="h-3"
                        style={{ width: `${30 + ((index * 17) % 40)}%` }}
                      />
                      <div className="flex-1" />
                      <Skeleton className="h-3 w-16" />
                    </div>
                  </TableCell>
                </TableRow>
              ))
            ) : runs.length === 0 ? (
              <TableRow className="border-0! hover:bg-transparent">
                <TableCell colSpan={5} className="py-16">
                  <p
                    data-slot="agent-history-empty"
                    className="text-center text-sm text-muted-foreground"
                  >
                    {t("agent.history.empty")}
                  </p>
                </TableCell>
              </TableRow>
            ) : (
              runs.map((run) => (
                <TableRow
                  key={run.id}
                  className="cursor-pointer hover:bg-muted/30"
                  onClick={() => {
                    setSelectedRunId(run.id);
                  }}
                >
                  <TableCell className="tabular-nums text-muted-foreground">
                    {formatDateTime(run.startedAt ?? run.createdAt) ?? "—"}
                  </TableCell>
                  <TableCell className="hidden truncate @2xl:table-cell">
                    {profileNames.get(run.profileId) ??
                      t("agent.history.unknownProfile")}
                  </TableCell>
                  <TableCell className="max-w-0 truncate">{run.goal}</TableCell>
                  <TableCell className="hidden @3xl:table-cell">
                    {t(
                      run.target === "fleet"
                        ? "agent.target.fleet"
                        : "agent.target.desktop",
                    )}
                  </TableCell>
                  <TableCell>
                    <span className="flex items-center gap-2 text-xs">
                      <StatusDot tone={agentStatusTone(run.status)} />
                      {agentStatusLabel(t, run.status)}
                    </span>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>

        {cursor !== null && runs.length > 0 && (
          <div className="flex justify-center py-3">
            <RippleButton
              variant="outline"
              size="sm"
              disabled={isLoadingMore}
              onClick={() => {
                void loadNextPage();
              }}
            >
              {isLoadingMore
                ? t("common.buttons.loading")
                : t("agent.history.loadMore")}
            </RippleButton>
          </div>
        )}
      </FadingScrollArea>
    </div>
  );
}

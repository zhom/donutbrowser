"use client";

import * as React from "react";
import { useTranslation } from "react-i18next";
import {
  formatDateTime,
  formatDuration,
  hasRunCounters,
  outcomeLabel,
  runStatusLabel,
  runStatusTone,
  StatusDot,
} from "@/components/cookie-bot-shared";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { translateBackendError } from "@/lib/backend-errors";
import {
  type CookieBotRun,
  cancelCookieBotRun,
  getCookieBotRuns,
} from "@/lib/cookie-bot";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

const RUN_PAGE_SIZE = 25;

/** Statuses that are still moving, so the row can offer a stop. */
const IN_FLIGHT = new Set(["pending", "running"]);

function runDurationSeconds(run: CookieBotRun): number | null {
  if (run.billed_seconds > 0) return run.billed_seconds;
  const started = run.started_at ? new Date(run.started_at).getTime() : NaN;
  const ended = run.ended_at ? new Date(run.ended_at).getTime() : NaN;
  if (!Number.isNaN(started) && !Number.isNaN(ended) && ended > started) {
    return (ended - started) / 1000;
  }
  return null;
}

interface CookieBotRunsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profileId: string | null;
  profileName?: string;
  /** Called after a run is cancelled, so shared state can be re-read. */
  onRunCancelled?: () => void;
}

/**
 * What the bot actually did for one profile, reached from that profile's row.
 *
 * The Cookie Bot page owns the fleet-wide activity view; this is the same data
 * narrowed to a single profile, which is the question an operator asks while
 * looking at the table. Every number is the server's — the desktop keeps no run
 * history of its own — and the status vocabulary is the shared one, so a status
 * cannot read differently in two places.
 */
export function CookieBotRunsDialog({
  isOpen,
  onClose,
  profileId,
  profileName,
  onRunCancelled,
}: CookieBotRunsDialogProps) {
  const { t } = useTranslation();
  const [runs, setRuns] = React.useState<CookieBotRun[]>([]);
  const [isLoading, setIsLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [cancellingId, setCancellingId] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    if (!profileId) return;
    setIsLoading(true);
    setError(null);
    try {
      const page = await getCookieBotRuns({ profileId, limit: RUN_PAGE_SIZE });
      setRuns(page.runs);
    } catch (err) {
      setError(translateBackendError(t as never, err));
    } finally {
      setIsLoading(false);
    }
  }, [profileId, t]);

  React.useEffect(() => {
    if (!isOpen) return;
    setRuns([]);
    void load();
  }, [isOpen, load]);

  const handleCancel = React.useCallback(
    async (run: CookieBotRun) => {
      setCancellingId(run.id);
      try {
        await cancelCookieBotRun(run.id);
        showSuccessToast(t("cookieBot.running.stopped"));
        onRunCancelled?.();
        await load();
      } catch (err) {
        showErrorToast(translateBackendError(t as never, err));
      } finally {
        setCancellingId(null);
      }
    },
    [load, onRunCancelled, t],
  );

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="flex max-h-[80vh] max-w-2xl flex-col">
        <DialogHeader className="shrink-0">
          <DialogTitle>{t("cookieBot.history.title")}</DialogTitle>
          <DialogDescription>
            {profileName ?? t("cookieBot.history.allProfiles")}
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 overflow-y-auto">
          {error ? (
            <p className="py-8 text-center text-sm text-destructive-text">
              {error}
            </p>
          ) : isLoading && runs.length === 0 ? (
            <div className="space-y-2 py-2">
              {Array.from({ length: 5 }, (_, index) => (
                <Skeleton
                  key={`run-skeleton-${index}`}
                  className="h-7 w-full"
                />
              ))}
            </div>
          ) : runs.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              {t("cookieBot.history.empty")}
            </p>
          ) : (
            <Table>
              <TableHeader className="sticky top-0 z-10 bg-background">
                <TableRow>
                  <TableHead>{t("cookieBot.history.columnStarted")}</TableHead>
                  <TableHead>{t("cookieBot.history.columnDuration")}</TableHead>
                  <TableHead>{t("cookieBot.history.columnSites")}</TableHead>
                  <TableHead>{t("cookieBot.history.columnStatus")}</TableHead>
                  <TableHead />
                </TableRow>
              </TableHeader>
              <TableBody>
                {runs.map((run) => {
                  const duration = runDurationSeconds(run);
                  const started =
                    formatDateTime(run.started_at ?? run.scheduled_for) ?? "—";
                  return (
                    <TableRow key={run.id}>
                      <TableCell className="text-xs tabular-nums whitespace-nowrap">
                        {started}
                      </TableCell>
                      <TableCell className="text-xs tabular-nums">
                        {duration === null ? "—" : formatDuration(t, duration)}
                      </TableCell>
                      {/* Never a confident `0/12`: nothing writes these
                          counters yet, so the column default is not a fact
                          about what the bot did. */}
                      <TableCell className="text-xs tabular-nums">
                        {hasRunCounters(run)
                          ? t("cookieBot.history.sitesVisited", {
                              visited: run.sites_visited,
                              total: run.sites_total,
                            })
                          : t("cookieBot.history.sitesUnknown")}
                        {run.sites_failed > 0 && (
                          <span className="ml-1 text-warning-text">
                            {t("cookieBot.history.sitesFailed", {
                              count: run.sites_failed,
                            })}
                          </span>
                        )}
                      </TableCell>
                      <TableCell className="text-xs">
                        <span className="flex items-center gap-1.5">
                          <StatusDot
                            tone={runStatusTone(run.status)}
                            pulse={run.status === "running"}
                            className="size-1.5"
                          />
                          {runStatusLabel(t, run.status)}
                        </span>
                        {run.outcome_code && (
                          <span className="mt-0.5 block text-[11px] text-muted-foreground">
                            {outcomeLabel(t, run.outcome_code)}
                          </span>
                        )}
                      </TableCell>
                      <TableCell className="text-right">
                        {IN_FLIGHT.has(run.status) && (
                          <LoadingButton
                            size="sm"
                            variant="outline"
                            isLoading={cancellingId === run.id}
                            onClick={() => {
                              void handleCancel(run);
                            }}
                            className="h-6 text-[11px]"
                          >
                            {t("cookieBot.running.stop")}
                          </LoadingButton>
                        )}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

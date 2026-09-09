"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuLock, LuRotateCcw, LuTrash2 } from "react-icons/lu";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { LoadingButton } from "@/components/loading-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { RippleButton } from "@/components/ui/ripple";
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
import { useTrashEvents } from "@/hooks/use-trash-events";
import { translateBackendError } from "@/lib/backend-errors";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import { formatBytes } from "@/lib/format-bytes";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile, TrashedProfileSummary } from "@/types";

interface TrashPageProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
}

const SECONDS_PER_DAY = 24 * 60 * 60;

/** Whole days left before the entry is purged, never below zero. */
function daysUntil(expiresAt: number, nowSeconds: number): number {
  return Math.max(0, Math.ceil((expiresAt - nowSeconds) / SECONDS_PER_DAY));
}

export function TrashPage({ isOpen, onClose, subPage }: TrashPageProps) {
  const { t, i18n } = useTranslation();
  const { entries, isLoading, error } = useTrashEvents();
  const [busyId, setBusyId] = useState<string | null>(null);
  const [entryToPurge, setEntryToPurge] =
    useState<TrashedProfileSummary | null>(null);
  const [emptyConfirmOpen, setEmptyConfirmOpen] = useState(false);
  const [isEmptying, setIsEmptying] = useState(false);

  const dateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        dateStyle: "medium",
        timeStyle: "short",
      }),
    [i18n.language],
  );
  const nowSeconds = Math.floor(Date.now() / 1000);

  const handleRestore = useCallback(
    async (entry: TrashedProfileSummary) => {
      setBusyId(entry.id);
      try {
        const restored = await invoke<BrowserProfile>(
          "restore_trashed_profile",
          { profileId: entry.id },
        );
        showSuccessToast(t("trash.restored", { name: restored.name }));
      } catch (err: unknown) {
        console.error("Failed to restore trashed profile:", err);
        showErrorToast(
          t("trash.restoreFailed", { error: translateBackendError(t, err) }),
        );
      } finally {
        setBusyId(null);
      }
    },
    [t],
  );

  const handlePurge = useCallback(async () => {
    if (!entryToPurge) return;
    const entry = entryToPurge;
    setBusyId(entry.id);
    try {
      await invoke("purge_trashed_profile", { profileId: entry.id });
      showSuccessToast(t("trash.deletedForever", { name: entry.name }));
      setEntryToPurge(null);
    } catch (err: unknown) {
      console.error("Failed to purge trashed profile:", err);
      showErrorToast(
        t("trash.deleteForeverFailed", {
          error: translateBackendError(t, err),
        }),
      );
    } finally {
      setBusyId(null);
    }
  }, [entryToPurge, t]);

  const handleEmptyTrash = useCallback(async () => {
    setIsEmptying(true);
    try {
      const removed = await invoke<number>("empty_trash");
      showSuccessToast(t("trash.emptied", { count: removed }));
      setEmptyConfirmOpen(false);
    } catch (err: unknown) {
      console.error("Failed to empty trash:", err);
      showErrorToast(
        t("trash.emptyFailed", { error: translateBackendError(t, err) }),
      );
    } finally {
      setIsEmptying(false);
    }
  }, [t]);

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
        <DialogContent className="flex max-h-[85vh] max-w-[min(80rem,calc(100%-4rem))] flex-col">
          {!subPage && (
            <DialogHeader>
              <DialogTitle>{t("trash.title")}</DialogTitle>
              <DialogDescription>{t("trash.description")}</DialogDescription>
            </DialogHeader>
          )}

          <div
            data-slot="trash-page"
            className="@container flex min-h-0 w-full flex-1 flex-col"
          >
            <div className="flex shrink-0 flex-wrap items-center justify-between gap-2">
              <div
                data-slot="trash-summary-pill"
                className="inline-flex h-7 items-center justify-center gap-1.5 rounded-md bg-accent px-3 text-sm font-medium whitespace-nowrap text-accent-foreground"
              >
                <span>{t("trash.title")}</span>
                <span
                  data-slot="trash-summary-count"
                  className="text-xs tabular-nums"
                >
                  {entries.length}
                </span>
              </div>
              <RippleButton
                size="sm"
                variant="outline"
                data-slot="trash-empty"
                disabled={entries.length === 0 || isLoading}
                onClick={() => {
                  setEmptyConfirmOpen(true);
                }}
                className="flex shrink-0 items-center gap-2"
                aria-label={t("trash.emptyTrash")}
              >
                <LuTrash2 className="size-4" />
                <span className="hidden @2xl:inline">
                  {t("trash.emptyTrash")}
                </span>
              </RippleButton>
            </div>

            {subPage && (
              <p className="mt-2 shrink-0 text-xs text-muted-foreground">
                {t("trash.description")}
              </p>
            )}

            {error && (
              <div className="mt-4 rounded-md bg-destructive/10 p-3 text-sm text-destructive-text">
                {error}
              </div>
            )}

            {isLoading ? (
              <div className="mt-4 text-sm text-muted-foreground">
                {t("common.buttons.loading")}
              </div>
            ) : entries.length === 0 ? (
              <div
                data-slot="trash-empty-state"
                className="mt-4 flex flex-1 flex-col items-center justify-center gap-1 py-12 text-center"
              >
                <p className="text-sm font-medium text-foreground">
                  {t("trash.empty")}
                </p>
                <p className="max-w-sm text-xs text-muted-foreground">
                  {t("trash.emptyHint")}
                </p>
              </div>
            ) : (
              <div className="mt-4 min-h-0 flex-1 overflow-auto rounded-md border border-border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{t("trash.columns.name")}</TableHead>
                      <TableHead className="hidden @xl:table-cell">
                        {t("trash.columns.browser")}
                      </TableHead>
                      <TableHead className="hidden @2xl:table-cell">
                        {t("trash.columns.deleted")}
                      </TableHead>
                      <TableHead>{t("trash.columns.expires")}</TableHead>
                      <TableHead className="hidden @xl:table-cell text-right">
                        {t("trash.columns.size")}
                      </TableHead>
                      <TableHead className="text-right">
                        {t("common.labels.actions")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {entries.map((entry) => {
                      const busy = busyId === entry.id;
                      const daysLeft = daysUntil(entry.expires_at, nowSeconds);
                      return (
                        <TableRow key={entry.id} data-trash-entry-id={entry.id}>
                          <TableCell className="max-w-64">
                            <div className="flex min-w-0 items-center gap-2">
                              <span className="truncate text-sm font-medium">
                                {entry.name}
                              </span>
                              {entry.password_protected && (
                                <Tooltip delayDuration={300}>
                                  <TooltipTrigger asChild>
                                    <span
                                      role="img"
                                      className="grid shrink-0 place-items-center text-muted-foreground"
                                      aria-label={t("trash.passwordProtected")}
                                    >
                                      <LuLock className="size-3.5" />
                                    </span>
                                  </TooltipTrigger>
                                  <TooltipContent>
                                    {t("trash.passwordProtected")}
                                  </TooltipContent>
                                </Tooltip>
                              )}
                            </div>
                          </TableCell>
                          <TableCell className="hidden @xl:table-cell text-sm text-muted-foreground">
                            {getBrowserDisplayName(entry.browser)}{" "}
                            <span className="tabular-nums">
                              {entry.version}
                            </span>
                          </TableCell>
                          <TableCell className="hidden @2xl:table-cell text-sm tabular-nums text-muted-foreground">
                            {dateFormatter.format(entry.deleted_at * 1000)}
                          </TableCell>
                          <TableCell
                            className={cn(
                              "text-sm tabular-nums",
                              daysLeft <= 1
                                ? "text-warning-text"
                                : "text-muted-foreground",
                            )}
                          >
                            {daysLeft === 0
                              ? t("trash.expiresToday")
                              : t("trash.expiresIn", { count: daysLeft })}
                          </TableCell>
                          <TableCell className="hidden @xl:table-cell text-right text-sm tabular-nums text-muted-foreground">
                            {formatBytes(entry.size_bytes)}
                          </TableCell>
                          <TableCell className="text-right">
                            <div className="flex items-center justify-end gap-1">
                              <LoadingButton
                                size="sm"
                                variant="outline"
                                isLoading={busy}
                                disabled={busyId !== null}
                                data-slot="trash-restore"
                                onClick={() => {
                                  void handleRestore(entry);
                                }}
                                className="gap-1.5"
                              >
                                <LuRotateCcw className="size-3.5" />
                                {t("trash.restore")}
                              </LoadingButton>
                              <Tooltip delayDuration={300}>
                                <TooltipTrigger asChild>
                                  <RippleButton
                                    size="icon"
                                    variant="ghost"
                                    disabled={busyId !== null}
                                    data-slot="trash-delete-forever"
                                    aria-label={t("trash.deleteForever")}
                                    className="size-8 text-muted-foreground hover:text-destructive-text"
                                    onClick={() => {
                                      setEntryToPurge(entry);
                                    }}
                                  >
                                    <LuTrash2 className="size-3.5" />
                                  </RippleButton>
                                </TooltipTrigger>
                                <TooltipContent>
                                  {t("trash.deleteForever")}
                                </TooltipContent>
                              </Tooltip>
                            </div>
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>

      <DeleteConfirmationDialog
        isOpen={entryToPurge !== null}
        onClose={() => {
          setEntryToPurge(null);
        }}
        onConfirm={handlePurge}
        title={t("trash.confirmDeleteForeverTitle")}
        description={t("trash.confirmDeleteForeverDescription", {
          name: entryToPurge?.name ?? "",
        })}
        confirmButtonText={t("trash.deleteForever")}
        isLoading={entryToPurge !== null && busyId === entryToPurge.id}
      />

      <DeleteConfirmationDialog
        isOpen={emptyConfirmOpen}
        onClose={() => {
          setEmptyConfirmOpen(false);
        }}
        onConfirm={handleEmptyTrash}
        title={t("trash.confirmEmptyTitle")}
        description={t("trash.confirmEmptyDescription", {
          count: entries.length,
        })}
        confirmButtonText={t("trash.emptyTrash")}
        isLoading={isEmptying}
      />
    </>
  );
}

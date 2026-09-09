"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuPause, LuPlay, LuTriangleAlert } from "react-icons/lu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast } from "@/lib/toast-utils";
import type { SyncSessionInfo, SyncWindowLayout } from "@/types";

const LAYOUTS: SyncWindowLayout[] = ["grid", "columns", "cascade"];

interface SynchronizerPanelProps {
  sessions: SyncSessionInfo[];
  /** Fold the record a command returned back into the list it came from. */
  onSessionChanged: (session: SyncSessionInfo) => void;
}

export function SynchronizerPanel({
  sessions,
  onSessionChanged,
}: SynchronizerPanelProps) {
  const { t } = useTranslation();
  const [layout, setLayout] = useState<SyncWindowLayout>("grid");
  const [busy, setBusy] = useState<string | null>(null);

  const run = useCallback(
    async (key: string, action: () => Promise<SyncSessionInfo | void>) => {
      if (busy !== null) return;
      setBusy(key);
      try {
        const updated = await action();
        if (updated) onSessionChanged(updated);
      } catch (err) {
        console.error(`Synchronizer action ${key} failed:`, err);
        showErrorToast(translateBackendError(t, err));
      } finally {
        setBusy(null);
      }
    },
    [busy, onSessionChanged, t],
  );

  if (sessions.length === 0) return null;

  return (
    <div data-slot="synchronizer-panel" className="grid gap-2 pb-2.5">
      {sessions.map((session) => (
        <section
          key={session.id}
          data-slot="synchronizer-session"
          data-session-id={session.id}
          data-paused={session.paused || undefined}
          className="rounded-lg border bg-card p-3"
        >
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
            <div className="min-w-0 flex-1">
              <span className="text-xs text-muted-foreground">
                {t("profiles.synchronizer.leader")}
              </span>
              <p
                data-slot="synchronizer-panel-leader"
                className="text-sm font-medium break-words"
              >
                {session.leader_profile_name}
              </p>
            </div>

            <Button
              type="button"
              variant="outline"
              size="sm"
              data-slot="synchronizer-panel-pause"
              disabled={busy !== null}
              onClick={() =>
                void run(`pause:${session.id}`, () =>
                  invoke<SyncSessionInfo>("set_sync_session_paused", {
                    sessionId: session.id,
                    paused: !session.paused,
                  }),
                )
              }
            >
              {session.paused ? (
                <LuPlay className="size-3.5" aria-hidden="true" />
              ) : (
                <LuPause className="size-3.5" aria-hidden="true" />
              )}
              {session.paused
                ? t("profiles.synchronizer.resumeMirroring")
                : t("profiles.synchronizer.pauseMirroring")}
            </Button>

            <Select
              value={layout}
              onValueChange={(value) => {
                setLayout(value as SyncWindowLayout);
              }}
            >
              <SelectTrigger
                size="sm"
                className="w-36"
                data-slot="synchronizer-panel-layout"
                aria-label={t("profiles.synchronizer.layoutLabel")}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LAYOUTS.map((option) => (
                  <SelectItem key={option} value={option}>
                    {t(`profiles.synchronizer.layout.${option}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Button
              type="button"
              variant="outline"
              size="sm"
              data-slot="synchronizer-panel-arrange"
              disabled={busy !== null || session.followers.length === 0}
              onClick={() =>
                void run(`arrange:${session.id}`, () =>
                  invoke<SyncSessionInfo>("arrange_sync_windows", {
                    sessionId: session.id,
                    layout,
                  }),
                )
              }
            >
              {t("profiles.synchronizer.arrange")}
            </Button>

            <Button
              type="button"
              variant="destructive"
              size="sm"
              data-slot="synchronizer-panel-stop"
              disabled={busy !== null}
              onClick={() =>
                void run(`stop:${session.id}`, async () => {
                  await invoke("stop_sync_session", { sessionId: session.id });
                })
              }
            >
              {t("profiles.synchronizer.stopSession")}
            </Button>
          </div>

          {session.paused && (
            <p
              role="status"
              data-slot="synchronizer-panel-paused-note"
              className="mt-2 text-xs text-muted-foreground"
            >
              {t("profiles.synchronizer.pausedNote")}
            </p>
          )}

          {session.followers.length === 0 ? (
            <p className="mt-2 text-xs text-muted-foreground">
              {t("profiles.synchronizer.sessionEmpty")}
            </p>
          ) : (
            <ScrollArea className="mt-2 max-h-40">
              <ul className="grid gap-1">
                {session.followers.map((follower) => {
                  const desynced = follower.failed_at_url !== null;
                  return (
                    <li
                      key={follower.profile_id}
                      data-slot="synchronizer-panel-follower"
                      data-profile-id={follower.profile_id}
                      data-held={follower.held || undefined}
                      className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 rounded-md px-1 py-1"
                    >
                      <span className="min-w-0 flex-1 text-sm break-words">
                        {follower.profile_name}
                      </span>

                      {desynced ? (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Badge
                              variant="outline"
                              className="shrink-0 gap-1 border-warning/50 px-1.5 py-0 text-[10px] text-warning-text"
                            >
                              <LuTriangleAlert
                                className="size-3"
                                aria-hidden="true"
                              />
                              {t("profiles.synchronizer.stateDesynced")}
                            </Badge>
                          </TooltipTrigger>
                          <TooltipContent className="max-w-[250px]">
                            {t("profiles.synchronizer.desyncedTooltip", {
                              url: follower.failed_at_url ?? "",
                            })}
                          </TooltipContent>
                        </Tooltip>
                      ) : (
                        <Badge
                          variant="outline"
                          data-slot="synchronizer-panel-follower-state"
                          className="shrink-0 px-1.5 py-0 text-[10px] text-muted-foreground"
                        >
                          {follower.held
                            ? t("profiles.synchronizer.stateHeld")
                            : session.paused
                              ? t("profiles.synchronizer.statePaused")
                              : t("profiles.synchronizer.stateMirroring")}
                        </Badge>
                      )}

                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        data-slot="synchronizer-panel-hold"
                        disabled={busy !== null}
                        onClick={() =>
                          void run(
                            `hold:${session.id}:${follower.profile_id}`,
                            () =>
                              invoke<SyncSessionInfo>(
                                "set_sync_follower_held",
                                {
                                  sessionId: session.id,
                                  followerProfileId: follower.profile_id,
                                  held: !follower.held,
                                },
                              ),
                          )
                        }
                      >
                        {follower.held
                          ? t("profiles.synchronizer.rejoin")
                          : t("profiles.synchronizer.holdOut")}
                      </Button>

                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        data-slot="synchronizer-panel-remove"
                        disabled={busy !== null}
                        onClick={() =>
                          void run(
                            `remove:${session.id}:${follower.profile_id}`,
                            async () => {
                              await invoke("remove_sync_follower", {
                                sessionId: session.id,
                                followerProfileId: follower.profile_id,
                              });
                            },
                          )
                        }
                      >
                        {t("common.buttons.stop")}
                      </Button>
                    </li>
                  );
                })}
              </ul>
            </ScrollArea>
          )}
        </section>
      ))}
    </div>
  );
}

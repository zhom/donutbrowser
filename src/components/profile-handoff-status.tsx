"use client";

import { motion, useReducedMotion } from "motion/react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuArrowDownToLine,
  LuCheck,
  LuCloud,
  LuMonitor,
  LuTriangleAlert,
} from "react-icons/lu";
import { useInputModality } from "@/hooks/use-input-modality";
import { MOTION_EASE_OUT } from "@/lib/motion";
import type { RemoteHandoffState } from "@/lib/remote-sessions";
import { cn } from "@/lib/utils";
import { Button } from "./ui/button";

interface ProfileHandoffStatusProps {
  profileId: string;
  state: RemoteHandoffState | null;
  syncStatus?: { status: string; error?: string };
  compact?: boolean;
  onOpenSync?: () => void;
}

export function ProfileHandoffStatus({
  profileId,
  state,
  syncStatus,
  compact = false,
  onOpenSync,
}: ProfileHandoffStatusProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const inputModality = useInputModality();
  const [observed, setObserved] = useState({
    profileId,
    state,
    returned: false,
  });

  let returned = observed.returned;
  if (observed.profileId !== profileId || observed.state !== state) {
    returned =
      observed.profileId === profileId &&
      state === null &&
      (observed.state === "pending_sync" || observed.returned);
    setObserved({ profileId, state, returned });
  }

  if (!state && !returned) return null;

  const failed = state === "pending_sync" && syncStatus?.status === "error";
  const phase = state ?? "returned";
  const label = failed
    ? t("profileMotion.handoffError")
    : state === "running"
      ? t("profileMotion.handoffRemote")
      : state === "pending_sync"
        ? t("profileMotion.handoffReturning")
        : t("profileMotion.handoffReturned");
  const Icon = failed
    ? LuTriangleAlert
    : state === "running"
      ? LuCloud
      : state === "pending_sync"
        ? LuArrowDownToLine
        : LuCheck;

  if (compact) {
    return (
      <span
        data-slot="profile-handoff-status"
        data-profile-id={profileId}
        data-handoff-state={phase}
        className={cn(
          "inline-flex min-w-0 items-center gap-1 text-[11px]",
          failed ? "text-destructive-text" : "text-muted-foreground",
        )}
      >
        <Icon aria-hidden="true" className="size-3 shrink-0" />
        <span className="truncate" title={label}>
          {label}
        </span>
      </span>
    );
  }

  return (
    <section
      data-slot="profile-handoff-detail"
      data-profile-id={profileId}
      data-handoff-state={phase}
      aria-label={t("profileMotion.handoffTitle")}
      className="flex flex-col gap-3 rounded-md bg-muted/40 p-3"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p
            role="status"
            className={cn(
              "text-sm font-medium",
              failed && "text-destructive-text",
            )}
          >
            {label}
          </p>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
            {failed
              ? t("profileMotion.handoffErrorHint")
              : state === "running"
                ? t("profiles.remote.runningTooltip")
                : state === "pending_sync"
                  ? t("profiles.remote.pendingSyncTooltip")
                  : t("profileMotion.handoffReturnedHint")}
          </p>
        </div>
        {returned && (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 shrink-0 px-2 text-xs"
            onClick={() => setObserved({ profileId, state, returned: false })}
          >
            {t("common.buttons.close")}
          </Button>
        )}
      </div>
      <div className="mx-auto grid w-full max-w-64 grid-cols-2 items-center gap-y-1.5">
        <LuCloud
          aria-hidden="true"
          className="mx-auto size-4 text-muted-foreground"
        />
        <LuMonitor
          aria-hidden="true"
          className="mx-auto size-4 text-muted-foreground"
        />
        <div aria-hidden="true" className="relative col-span-2 h-3">
          <span className="absolute top-1/2 left-1/4 h-0.5 w-1/2 -translate-y-1/2 rounded-full bg-border" />
          <motion.span
            data-slot="profile-handoff-marker"
            initial={false}
            animate={{
              x:
                state === "running"
                  ? "0%"
                  : state === "pending_sync"
                    ? "50%"
                    : "100%",
            }}
            transition={{
              duration: reduceMotion || inputModality === "keyboard" ? 0 : 0.2,
              ease: MOTION_EASE_OUT,
            }}
            className="absolute top-0 left-1/4 flex w-1/2 justify-start"
          >
            <span
              className={cn(
                "size-3 shrink-0 -translate-x-1/2 rounded-full bg-foreground",
                failed && "bg-destructive",
              )}
            />
          </motion.span>
        </div>
        <span className="text-center text-xs text-muted-foreground">
          {t("profileMotion.handoffRemoteLocation")}
        </span>
        <span className="text-center text-xs text-muted-foreground">
          {t("profileMotion.handoffLocalLocation")}
        </span>
      </div>
      {failed && onOpenSync && (
        <Button
          variant="ghost"
          size="sm"
          className="h-auto self-start px-0 py-1 text-xs"
          onClick={onOpenSync}
        >
          {t("profileMotion.handoffSyncDetails")}
        </Button>
      )}
    </section>
  );
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { LuLoaderCircle } from "react-icons/lu";
import { SynchronizerRehearsal } from "@/components/synchronizer-rehearsal";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { parseBackendError, translateBackendError } from "@/lib/backend-errors";
import { isCrossOsProfile } from "@/lib/browser-utils";
import type {
  BrowserProfile,
  SyncSessionInfo,
  WayfernFingerprintConfig,
} from "@/types";

function getScreenSize(
  profile: BrowserProfile,
): { w: number; h: number } | null {
  // An identity-backed profile stores no device, only the user's edits, so a
  // screen size is available only when the user pinned one.
  const fp =
    profile.wayfern_config?.fingerprint ??
    profile.wayfern_config?.identity_overrides;
  if (!fp) return null;
  try {
    const parsed: WayfernFingerprintConfig = JSON.parse(fp);
    const w = parsed.screenWidth ?? parsed.windowInnerWidth;
    const h = parsed.screenHeight ?? parsed.windowInnerHeight;
    if (w && h) return { w, h };
  } catch {
    // ignore
  }
  return null;
}

interface SyncFollowerDialogProps {
  isOpen: boolean;
  onClose: () => void;
  leaderProfile: BrowserProfile | null;
  allProfiles: BrowserProfile[];
  runningProfiles: Set<string>;
}

export function SyncFollowerDialog({
  isOpen,
  onClose,
  leaderProfile,
  allProfiles,
  runningProfiles,
}: SyncFollowerDialogProps) {
  const { t } = useTranslation();
  const checkboxPrefix = useId();
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [startingProfiles, setStartingProfiles] = useState<
    BrowserProfile[] | null
  >(null);
  const [startFailure, setStartFailure] = useState<{ error: unknown } | null>(
    null,
  );
  const startPending = useRef(false);
  const isStarting = startingProfiles !== null;

  const eligibleProfiles = useMemo(
    () =>
      startingProfiles ??
      allProfiles.filter(
        (p) =>
          p.id !== leaderProfile?.id &&
          p.browser === "wayfern" &&
          !runningProfiles.has(p.id) &&
          !isCrossOsProfile(p),
      ),
    [allProfiles, leaderProfile?.id, runningProfiles, startingProfiles],
  );
  const selectedProfiles = eligibleProfiles.filter((profile) =>
    selectedIds.has(profile.id),
  );

  useEffect(() => {
    if (!isOpen) {
      setSelectedIds(new Set());
      setStartFailure(null);
    }
  }, [isOpen]);

  const leaderScreenSize = useMemo(
    () => (leaderProfile ? getScreenSize(leaderProfile) : null),
    [leaderProfile],
  );

  const handleToggle = useCallback((id: string, checked: boolean) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (checked) {
        next.add(id);
      } else {
        next.delete(id);
      }
      return next;
    });
  }, []);

  const handleStart = async () => {
    if (!leaderProfile || selectedProfiles.length === 0 || startPending.current)
      return;
    startPending.current = true;
    setStartingProfiles(selectedProfiles);
    setStartFailure(null);
    try {
      // The initial session event precedes CDP readiness. Only this response
      // confirms that actions can actually reach the followers.
      await invoke<SyncSessionInfo>("start_sync_session", {
        leaderProfileId: leaderProfile.id,
        followerProfileIds: selectedProfiles.map((profile) => profile.id),
      });
      setSelectedIds(new Set());
      onClose();
    } catch (err) {
      console.error("Failed to start sync session:", err);
      setStartFailure({ error: err });
    } finally {
      startPending.current = false;
      setStartingProfiles(null);
    }
  };

  const handleOpenChange = useCallback(
    (open: boolean) => {
      if (!open && !startPending.current) {
        setSelectedIds(new Set());
        setStartFailure(null);
        onClose();
      }
    },
    [onClose],
  );

  return (
    <Dialog open={isOpen} onOpenChange={handleOpenChange}>
      <DialogContent
        className="max-w-lg"
        dismissible={!isStarting}
        data-slot="synchronizer-follower-dialog"
      >
        <DialogHeader>
          <DialogTitle>
            {t("profiles.synchronizer.selectFollowers")}
          </DialogTitle>
          <DialogDescription>
            {t("profiles.synchronizer.selectFollowersDesc")}
          </DialogDescription>
        </DialogHeader>

        {leaderProfile && (
          <div className="space-y-5">
            <SynchronizerRehearsal
              key={`${leaderProfile.id}:${selectedProfiles.map((profile) => profile.id).join(",")}`}
              leader={leaderProfile}
              followers={selectedProfiles}
              disabled={isStarting}
            />

            <div className="rounded-lg bg-muted/30">
              <ScrollArea className="h-[clamp(120px,24vh,16rem)]">
                <div className="space-y-1 p-2">
                  {eligibleProfiles.length === 0 ? (
                    <p className="py-4 text-center text-sm text-muted-foreground">
                      {t("profiles.synchronizer.wayfernOnly")}
                    </p>
                  ) : (
                    eligibleProfiles.map((profile) => {
                      const followerSize = getScreenSize(profile);
                      const isFlaky =
                        leaderScreenSize &&
                        followerSize &&
                        (leaderScreenSize.w !== followerSize.w ||
                          leaderScreenSize.h !== followerSize.h);

                      return (
                        <label
                          key={profile.id}
                          htmlFor={`${checkboxPrefix}-${profile.id}`}
                          data-slot="synchronizer-follower-option"
                          data-profile-id={profile.id}
                          className="flex min-w-0 cursor-pointer items-center gap-3 rounded-md p-2 hover:bg-accent hover:text-accent-foreground has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring has-[:disabled]:cursor-default"
                        >
                          <Checkbox
                            id={`${checkboxPrefix}-${profile.id}`}
                            aria-label={profile.name}
                            disabled={isStarting}
                            checked={selectedIds.has(profile.id)}
                            onCheckedChange={(checked) => {
                              handleToggle(profile.id, checked === true);
                            }}
                          />
                          <span className="min-w-0 flex-1 text-sm break-words">
                            {profile.name}
                          </span>
                          {isFlaky && (
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <Badge
                                  variant="outline"
                                  className="shrink-0 border-warning/50 px-1.5 py-0 text-[10px] text-warning-text"
                                >
                                  {t("profiles.synchronizer.flakyBadge")}
                                </Badge>
                              </TooltipTrigger>
                              <TooltipContent className="max-w-[250px]">
                                {t("profiles.synchronizer.flakyTooltip")}
                              </TooltipContent>
                            </Tooltip>
                          )}
                        </label>
                      );
                    })
                  )}
                </div>
              </ScrollArea>
            </div>
          </div>
        )}

        {isStarting && (
          <p
            role="status"
            data-slot="synchronizer-starting"
            className="flex items-center gap-2 text-sm text-muted-foreground"
          >
            <LuLoaderCircle
              className="size-4 shrink-0 motion-safe:animate-spin"
              aria-hidden="true"
            />
            {t("synchronizerPreview.starting")}
          </p>
        )}
        {startFailure && (
          <div
            role="alert"
            data-slot="synchronizer-start-error"
            className="space-y-1 text-sm text-destructive-text"
          >
            <p>{t("synchronizerPreview.startError")}</p>
            {parseBackendError(startFailure.error) && (
              <p>{translateBackendError(t, startFailure.error)}</p>
            )}
          </div>
        )}

        <DialogFooter>
          <Button
            variant="ghost"
            disabled={isStarting}
            onClick={() => {
              handleOpenChange(false);
            }}
          >
            {t("common.buttons.cancel")}
          </Button>
          <Button
            data-slot="synchronizer-start"
            disabled={selectedProfiles.length === 0 || isStarting}
            onClick={() => void handleStart()}
          >
            {t("profiles.synchronizer.startSession")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

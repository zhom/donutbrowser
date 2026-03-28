"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
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
import { isCrossOsProfile } from "@/lib/browser-utils";
import { showErrorToast } from "@/lib/toast-utils";
import type {
  BrowserProfile,
  SyncSessionInfo,
  WayfernFingerprintConfig,
} from "@/types";
import { RippleButton } from "./ui/ripple";

function getScreenSize(
  profile: BrowserProfile,
): { w: number; h: number } | null {
  const fp = profile.wayfern_config?.fingerprint;
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
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  const eligibleProfiles = allProfiles.filter(
    (p) =>
      p.id !== leaderProfile?.id &&
      p.browser === "wayfern" &&
      !runningProfiles.has(p.id) &&
      !isCrossOsProfile(p),
  );

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

  const handleStart = useCallback(() => {
    if (!leaderProfile || selectedIds.size === 0) return;
    const ids = Array.from(selectedIds);
    const leaderId = leaderProfile.id;
    setSelectedIds(new Set());
    onClose();

    invoke<SyncSessionInfo>("start_sync_session", {
      leaderProfileId: leaderId,
      followerProfileIds: ids,
    }).catch((err) => {
      console.error("Failed to start sync session:", err);
      showErrorToast(err instanceof Error ? err.message : String(err));
    });
  }, [leaderProfile, selectedIds, onClose]);

  const handleOpenChange = useCallback(
    (open: boolean) => {
      if (!open) {
        setSelectedIds(new Set());
        onClose();
      }
    },
    [onClose],
  );

  return (
    <Dialog open={isOpen} onOpenChange={handleOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {t("profiles.synchronizer.selectFollowers")}
          </DialogTitle>
          <DialogDescription>
            {t("profiles.synchronizer.selectFollowersDesc")}
          </DialogDescription>
        </DialogHeader>

        {leaderProfile && (
          <div className="space-y-3">
            <div className="flex items-center gap-2 p-2 rounded-md bg-primary/10 border border-primary/20">
              <Badge variant="default" className="text-xs">
                {t("profiles.synchronizer.leader")}
              </Badge>
              <span className="text-sm font-medium truncate">
                {leaderProfile.name}
              </span>
            </div>

            <div className="border rounded-md">
              <ScrollArea className="h-[150px]">
                <div className="space-y-1 p-2">
                  {eligibleProfiles.length === 0 ? (
                    <p className="text-sm text-muted-foreground py-4 text-center">
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
                        <div
                          key={profile.id}
                          className="flex items-center gap-3 p-2 rounded-md hover:bg-accent cursor-pointer"
                          onClick={() => {
                            handleToggle(
                              profile.id,
                              !selectedIds.has(profile.id),
                            );
                          }}
                          onKeyDown={() => {}}
                          role="button"
                          tabIndex={0}
                        >
                          <Checkbox
                            checked={selectedIds.has(profile.id)}
                            onCheckedChange={(checked) => {
                              handleToggle(profile.id, checked === true);
                            }}
                          />
                          <span className="text-sm truncate flex-1">
                            {profile.name}
                          </span>
                          {isFlaky && (
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <Badge
                                  variant="outline"
                                  className="text-[10px] px-1.5 py-0 text-warning border-warning/50 shrink-0"
                                >
                                  {t("profiles.synchronizer.flakyBadge")}
                                </Badge>
                              </TooltipTrigger>
                              <TooltipContent className="max-w-[250px]">
                                {t("profiles.synchronizer.flakyTooltip")}
                              </TooltipContent>
                            </Tooltip>
                          )}
                        </div>
                      );
                    })
                  )}
                </div>
              </ScrollArea>
            </div>
          </div>
        )}

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={() => {
              handleOpenChange(false);
            }}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <RippleButton disabled={selectedIds.size === 0} onClick={handleStart}>
            {t("profiles.synchronizer.startSession")}
          </RippleButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { LuCookie, LuSearch } from "react-icons/lu";
import {
  CookieBotActivity,
  type RunFilter,
} from "@/components/cookie-bot-activity";
import { CookieBotEnrolDialog } from "@/components/cookie-bot-enrol-dialog";
import { CookieBotOverview } from "@/components/cookie-bot-overview";
import { CookieBotScheduleTab } from "@/components/cookie-bot-schedule";
import {
  preflight,
  preflightReason,
  RemoteHoursMeter,
} from "@/components/cookie-bot-shared";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { TeamUsagePanel } from "@/components/team-usage-panel";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
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
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Input } from "@/components/ui/input";
import { ProBadge } from "@/components/ui/pro-badge";
import { RippleButton } from "@/components/ui/ripple";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cookieBotScopeFor, useCookieBot } from "@/hooks/use-cookie-bot";
import { translateBackendError } from "@/lib/backend-errors";
import {
  type CookieBotRun,
  type CookieBotSchedule,
  deleteCookieBotSchedule,
  getCookieBotRuns,
} from "@/lib/cookie-bot";
import { canUseCookieBot, getEntitlements } from "@/lib/entitlements";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile, CloudUser } from "@/types";

export type CookieBotTab = "overview" | "schedule" | "activity" | "team";

/** How often the run rows are re-read while something is running. A run's site
 * counter advances without a session transition, so the stream alone would show
 * a frozen number for an hour. */
const LIVE_POLL_MS = 15_000;
const RUN_PAGE_SIZE = 100;

interface CookieBotPageProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  initialTab?: CookieBotTab;
  profiles: BrowserProfile[];
  cloudUser: CloudUser | null;
  /** Opens the profile's sync settings for an end-to-end encrypted profile. */
  onOpenProfileSync: (profile: BrowserProfile) => void;
  /** Opens proxy assignment for profiles with no exit node. */
  onAssignProxy: (profileIds: string[]) => void;
}

export function CookieBotPage({
  isOpen,
  onClose,
  subPage,
  initialTab = "overview",
  profiles,
  cloudUser,
  onOpenProfileSync,
  onAssignProxy,
}: CookieBotPageProps) {
  const { t } = useTranslation();
  const entitlements = getEntitlements(cloudUser);
  const unlocked = canUseCookieBot(cloudUser);
  const isTeam = entitlements.teamCollaboration && Boolean(cloudUser?.teamId);
  const isOwnerOrAdmin =
    cloudUser?.teamRole === "owner" || cloudUser?.teamRole === "admin";
  const showTeamTab = isTeam && cloudUser?.teamRole === "owner";
  const scope = cookieBotScopeFor(cloudUser);

  // Enrolments, the pooled budget and the live sessions all come from the one
  // shared store the profile table reads, so an edit here moves both at once.
  //
  // Enabled is `unlocked`, NOT `isOpen && unlocked`: the store is a module
  // singleton whose enabled flag is set by whichever consumer's effect ran
  // last, so a closed page passing `false` would switch the event stream off
  // underneath the rail and the profile table.
  const {
    schedules: schedulesByProfile,
    liveSessions,
    quota,
    isLoading: isStoreLoading,
    error: storeError,
    streamConnected,
    refresh: refreshStore,
  } = useCookieBot(unlocked, scope);

  const schedules = useMemo(
    () =>
      Object.values(schedulesByProfile).sort(
        (a, b) =>
          a.run_at_minute - b.run_at_minute ||
          a.profile_name.localeCompare(b.profile_name),
      ),
    [schedulesByProfile],
  );
  const live = useMemo(() => Object.values(liveSessions), [liveSessions]);
  const liveCount = live.length;

  const [activeTab, setActiveTab] = useState<CookieBotTab>(initialTab);
  const [runs, setRuns] = useState<CookieBotRun[]>([]);
  const [isLoadingRuns, setIsLoadingRuns] = useState(true);
  const [runsError, setRunsError] = useState<unknown>(null);
  const [runFilter, setRunFilter] = useState<RunFilter>("all");
  const hasLoadedRuns = useRef(false);

  const [pickerOpen, setPickerOpen] = useState(false);
  const [enrolTargets, setEnrolTargets] = useState<BrowserProfile[]>([]);
  const [editing, setEditing] = useState<CookieBotSchedule | null>(null);
  const [enrolOpen, setEnrolOpen] = useState(false);
  const [pendingRemoval, setPendingRemoval] =
    useState<CookieBotSchedule | null>(null);
  const [isRemoving, setIsRemoving] = useState(false);

  const isLoading = isStoreLoading || isLoadingRuns;
  const loadError: unknown = runsError ?? storeError;

  /**
   * Runs are the one thing the shared store does not hold: only this page and
   * the per-profile history read them, and they page. `withSpinner` is false
   * for every background pass, because swapping a correct table for a skeleton
   * every fifteen seconds is worse than a row being a few seconds stale.
   */
  const loadRuns = useCallback(
    async (withSpinner: boolean) => {
      if (withSpinner) setIsLoadingRuns(true);
      try {
        const page = await getCookieBotRuns({ scope, limit: RUN_PAGE_SIZE });
        setRuns(page.runs);
        setRunsError(null);
      } catch (error) {
        // A background refresh that fails leaves the previous rows on screen.
        if (withSpinner) setRunsError(error);
      } finally {
        if (withSpinner) setIsLoadingRuns(false);
      }
    },
    [scope],
  );

  const reload = useCallback(() => {
    void refreshStore();
    void loadRuns(true);
  }, [refreshStore, loadRuns]);

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

  useEffect(() => {
    if (!isOpen || !unlocked) {
      hasLoadedRuns.current = false;
      return;
    }
    // The first pass shows a skeleton. Every later pass — a session appearing
    // or closing, or the poll below — swaps rows in silently. Both matter: the
    // live set changing means a run row moved, and a run's site counter
    // advances with no session transition at all.
    const first = !hasLoadedRuns.current;
    hasLoadedRuns.current = true;
    void loadRuns(first);

    if (liveCount === 0) return;
    const id = window.setInterval(() => {
      void loadRuns(false);
    }, LIVE_POLL_MS);
    return () => {
      window.clearInterval(id);
    };
  }, [isOpen, unlocked, liveCount, loadRuns]);

  const enrolledIds = useMemo(
    () => new Set(Object.keys(schedulesByProfile)),
    [schedulesByProfile],
  );

  const openEnrolFor = useCallback(
    (targets: BrowserProfile[], schedule: CookieBotSchedule | null) => {
      if (targets.length === 0) return;
      setEnrolTargets(targets);
      setEditing(schedule);
      setEnrolOpen(true);
    },
    [],
  );

  const handleEditSchedule = useCallback(
    (schedule: CookieBotSchedule) => {
      const profile = profiles.find((p) => p.id === schedule.profile_id);
      if (!profile) {
        showErrorToast(t("cookieBot.enrolled.profileMissing"));
        return;
      }
      openEnrolFor([profile], schedule);
    },
    [profiles, openEnrolFor, t],
  );

  const confirmRemoval = useCallback(async () => {
    if (!pendingRemoval) return;
    setIsRemoving(true);
    try {
      await deleteCookieBotSchedule(pendingRemoval.profile_id);
      showSuccessToast(t("cookieBot.schedule.unenrolled"));
      setPendingRemoval(null);
      reload();
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsRemoving(false);
    }
  }, [pendingRemoval, reload, t]);

  const dialogBody = !unlocked ? (
    <LockedState />
  ) : (
    <div className="@container flex min-h-0 w-full flex-1 flex-col">
      <AnimatedTabs
        value={activeTab}
        onValueChange={(value) => {
          setActiveTab(value as CookieBotTab);
        }}
        className="flex min-h-0 flex-1 flex-col"
      >
        <div className="flex shrink-0 flex-wrap items-center justify-between gap-2">
          <AnimatedTabsList>
            <AnimatedTabsTrigger value="overview">
              <span>{t("cookieBot.tabs.overview")}</span>
              <span className="text-xs tabular-nums">{schedules.length}</span>
            </AnimatedTabsTrigger>
            <AnimatedTabsTrigger value="schedule">
              {t("cookieBot.tabs.schedule")}
            </AnimatedTabsTrigger>
            <AnimatedTabsTrigger value="activity">
              <span>{t("cookieBot.tabs.activity")}</span>
              {live.length > 0 && (
                <span className="size-1.5 rounded-full bg-success" />
              )}
            </AnimatedTabsTrigger>
            {showTeamTab && (
              <AnimatedTabsTrigger value="team">
                {t("cookieBot.tabs.team")}
              </AnimatedTabsTrigger>
            )}
          </AnimatedTabsList>

          <div className="flex items-center gap-3">
            <RemoteHoursMeter
              quota={quota}
              isLoading={isStoreLoading && quota === null}
              variant="compact"
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <RippleButton
                  size="sm"
                  className="flex items-center gap-2"
                  aria-label={t("cookieBot.enrolled.enrolProfiles")}
                  onClick={() => {
                    setPickerOpen(true);
                  }}
                >
                  <GoPlus className="size-4" />
                  <span className="hidden @2xl:inline">
                    {t("cookieBot.enrolled.enrolProfiles")}
                  </span>
                </RippleButton>
              </TooltipTrigger>
              <TooltipContent>
                {t("cookieBot.enrolled.enrolProfiles")}
              </TooltipContent>
            </Tooltip>
          </div>
        </div>

        {loadError !== null && (
          <div className="mt-4 flex shrink-0 items-center gap-3 rounded-md border border-destructive/50 bg-destructive/10 p-3">
            <p className="min-w-0 flex-1 text-sm text-destructive-text">
              {translateBackendError(t, loadError)}
            </p>
            <RippleButton variant="outline" size="sm" onClick={reload}>
              {t("common.buttons.retry")}
            </RippleButton>
          </div>
        )}

        <AnimatedTabsContent
          value="overview"
          className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
        >
          <CookieBotOverview
            schedules={schedules}
            runs={runs}
            live={live}
            quota={quota}
            profiles={profiles}
            isLoading={isLoading}
            currentUserId={cloudUser?.id ?? null}
            onEnrol={() => {
              setPickerOpen(true);
            }}
            onEditSchedule={handleEditSchedule}
            onRemoveSchedule={setPendingRemoval}
            onJumpToActivity={(filter) => {
              setRunFilter(filter);
              setActiveTab("activity");
            }}
          />
        </AnimatedTabsContent>

        <AnimatedTabsContent
          value="schedule"
          className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
        >
          <CookieBotScheduleTab
            schedules={schedules}
            isLoading={isStoreLoading}
            currentUserId={cloudUser?.id ?? null}
            canEditOthers={isOwnerOrAdmin}
            onEdit={handleEditSchedule}
            onRemove={setPendingRemoval}
          />
        </AnimatedTabsContent>

        <AnimatedTabsContent
          value="activity"
          className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
        >
          <CookieBotActivity
            live={live}
            streamConnected={streamConnected}
            runs={runs}
            isLoading={isLoadingRuns}
            profiles={profiles}
            showOperator={isTeam}
            filter={runFilter}
            onFilterChange={setRunFilter}
            onRefresh={reload}
          />
        </AnimatedTabsContent>

        {showTeamTab && (
          <AnimatedTabsContent
            value="team"
            className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
          >
            {/* One team-usage implementation, shared with the Account page,
                so the two never disagree about who spent the pool. */}
            <TeamUsagePanel quota={quota} className="min-h-0 flex-1" />
          </AnimatedTabsContent>
        )}
      </AnimatedTabs>
    </div>
  );

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
        <DialogContent className="flex max-h-[85vh] max-w-[min(80rem,calc(100%-4rem))] flex-col">
          {!subPage && (
            <DialogHeader>
              <DialogTitle>{t("cookieBot.title")}</DialogTitle>
              <DialogDescription>
                {t("cookieBot.description")}
              </DialogDescription>
            </DialogHeader>
          )}
          {dialogBody}
        </DialogContent>
      </Dialog>

      <EnrolPickerDialog
        isOpen={pickerOpen}
        profiles={profiles}
        enrolledIds={enrolledIds}
        onClose={() => {
          setPickerOpen(false);
        }}
        onConfirm={(selected) => {
          setPickerOpen(false);
          openEnrolFor(selected, null);
        }}
      />

      <CookieBotEnrolDialog
        isOpen={enrolOpen}
        onClose={() => {
          setEnrolOpen(false);
        }}
        profiles={enrolTargets}
        existing={editing}
        onSaved={reload}
        onOpenProfileSync={onOpenProfileSync}
        onAssignProxy={onAssignProxy}
      />

      <DeleteConfirmationDialog
        isOpen={pendingRemoval !== null}
        onClose={() => {
          setPendingRemoval(null);
        }}
        onConfirm={() => {
          void confirmRemoval();
        }}
        title={t("cookieBot.schedule.unenrolTitle", {
          name: pendingRemoval?.profile_name ?? "",
        })}
        description={t("cookieBot.schedule.unenrolDescription")}
        confirmButtonText={t("cookieBot.schedule.unenrol")}
        isLoading={isRemoving}
      />
    </>
  );
}

function LockedState() {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 py-16 text-center">
      <LuCookie className="size-12 text-muted-foreground" />
      <div className="flex items-center gap-2">
        <p className="text-sm font-medium text-foreground">
          {t("cookieBot.locked.title")}
        </p>
        <ProBadge />
      </div>
      <p className="max-w-md text-xs text-muted-foreground">
        {t("cookieBot.locked.hint")}
      </p>
    </div>
  );
}

/**
 * Picking which profiles to enrol. Ineligible profiles are shown with their
 * reason rather than hidden, so the list matches what the operator sees in the
 * main table and the refusal is discovered here, not at 02:00.
 */
function EnrolPickerDialog({
  isOpen,
  profiles,
  enrolledIds,
  onClose,
  onConfirm,
}: {
  isOpen: boolean;
  profiles: BrowserProfile[];
  enrolledIds: Set<string>;
  onClose: () => void;
  onConfirm: (selected: BrowserProfile[]) => void;
}) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!isOpen) return;
    setSearch("");
    setSelected(new Set());
  }, [isOpen]);

  const rows = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return profiles
      .filter((profile) => profile.name.toLowerCase().includes(needle))
      .map((profile) => ({
        profile,
        check: preflight(profile),
        enrolled: enrolledIds.has(profile.id),
      }))
      .sort((a, b) => {
        if (a.check.eligible !== b.check.eligible) {
          return a.check.eligible ? -1 : 1;
        }
        return a.profile.name.localeCompare(b.profile.name);
      });
  }, [profiles, search, enrolledIds]);

  const chosen = useMemo(
    () => profiles.filter((profile) => selected.has(profile.id)),
    [profiles, selected],
  );

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="flex max-h-[70vh] max-w-lg flex-col">
        <DialogHeader>
          <DialogTitle>{t("cookieBot.picker.title")}</DialogTitle>
          <DialogDescription>
            {t("cookieBot.picker.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="relative shrink-0">
          <LuSearch className="absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
            }}
            className="h-8 pl-8 text-sm"
            placeholder={t("cookieBot.picker.searchPlaceholder")}
          />
        </div>

        <FadingScrollArea className="min-h-0 flex-1">
          <div className="flex flex-col gap-0.5 pr-1">
            {rows.length === 0 ? (
              <p className="py-8 text-center text-sm text-muted-foreground">
                {t("cookieBot.picker.noProfiles")}
              </p>
            ) : (
              rows.map(({ profile, check, enrolled }) => (
                <label
                  key={profile.id}
                  htmlFor={`cookie-bot-pick-${profile.id}`}
                  className={cn(
                    "flex h-8 cursor-pointer items-center gap-2 rounded-md px-2 text-xs transition-colors duration-100 hover:bg-accent hover:text-accent-foreground",
                    !check.eligible && "cursor-not-allowed opacity-60",
                  )}
                >
                  <Checkbox
                    id={`cookie-bot-pick-${profile.id}`}
                    checked={selected.has(profile.id)}
                    disabled={!check.eligible}
                    onCheckedChange={(value) => {
                      setSelected((prev) => {
                        const next = new Set(prev);
                        if (value === true) next.add(profile.id);
                        else next.delete(profile.id);
                        return next;
                      });
                    }}
                  />
                  <span className="min-w-0 flex-1 truncate">
                    {profile.name}
                  </span>
                  {enrolled && (
                    <span className="shrink-0 text-[10px] uppercase tracking-wide text-muted-foreground">
                      {t("cookieBot.picker.alreadyEnrolled")}
                    </span>
                  )}
                  {!check.eligible && (
                    <span className="shrink-0 text-muted-foreground">
                      {preflightReason(t, check)}
                    </span>
                  )}
                </label>
              ))
            )}
          </div>
        </FadingScrollArea>

        <DialogFooter>
          <Button variant="outline" size="sm" onClick={onClose}>
            {t("common.buttons.cancel")}
          </Button>
          <RippleButton
            size="sm"
            disabled={chosen.length === 0}
            onClick={() => {
              onConfirm(chosen);
            }}
          >
            {t("cookieBot.picker.continue", { count: chosen.length })}
          </RippleButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

"use client";

import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuChevronRight, LuInfo } from "react-icons/lu";
import {
  CADENCES,
  type CadenceId,
  cadenceForMask,
  clockToMinutes,
  enableProfileSync,
  formatHours,
  minutesToClock,
  nightsPerWeek,
  type PreflightResult,
  preflight,
  preflightFixLabel,
  preflightReason,
  profileTimezone,
  RemoteHoursMeter,
  resolvedOs,
} from "@/components/cookie-bot-shared";
import {
  AnimatedTabs,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { AutoHeight } from "@/components/ui/auto-height";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { RippleButton } from "@/components/ui/ripple";
import { StepTransition } from "@/components/ui/step-transition";
import { Textarea } from "@/components/ui/textarea";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { cookieBotScopeFor, useCookieBot } from "@/hooks/use-cookie-bot";
import { parseBackendError, translateBackendError } from "@/lib/backend-errors";
import {
  type CookieBotConflict,
  type CookieBotPlatform,
  type CookieBotPreset,
  type CookieBotPresetList,
  type CookieBotSchedule,
  type CookieBotScheduleInput,
  checkCookieBotConflicts,
  getCookieBotPresets,
  saveCookieBotSchedule,
} from "@/lib/cookie-bot";
import { SCHEDULE_BOUNDS } from "@/lib/cookie-bot-limits";
import { canUseCookieBot } from "@/lib/entitlements";
import { MOTION_EASE_OUT } from "@/lib/motion";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile } from "@/types";

/**
 * The cap when the server has not published one for the chosen preset. It is a
 * user-facing ceiling on machine time, not a description of what the bot does
 * with it — the contract allows 5..120 and this sits comfortably inside.
 */
const FALLBACK_MAX_MINUTES = 40;

/**
 * The server's schedule bounds. Mirrored, never re-declared: see
 * `src/lib/cookie-bot-limits.ts` and the test that pins them.
 */
const {
  minMaxMinutes: MIN_MAX_MINUTES,
  maxMaxMinutes: MAX_MAX_MINUTES,
  minSites: MIN_SITES,
  maxSites: MAX_SITES,
} = SCHEDULE_BOUNDS;

/** The default start: deep enough into the night to be plausible anywhere. */
const DEFAULT_RUN_AT_MINUTE = 2 * 60;

const PRESET_LABEL_KEYS: Record<string, string> = {
  light: "cookieBot.preset.light",
  balanced: "cookieBot.preset.balanced",
  deep: "cookieBot.preset.deep",
};

interface EnrolTarget {
  profile: BrowserProfile;
  check: PreflightResult;
}

interface ConflictNotice {
  email: string;
  time: string;
  profileIds: string[];
}

export interface CookieBotEnrolDialogProps {
  isOpen: boolean;
  onClose: () => void;
  /** The profiles being enrolled. One for the fast path, many for a bulk enrol. */
  profiles: BrowserProfile[];
  /** Pre-fills the form when editing an existing enrolment. */
  existing?: CookieBotSchedule | null;
  /** Extra work after the shared store has already been refreshed. */
  onSaved?: () => void;
  /**
   * Opens the profile's sync settings, for an end-to-end encrypted profile.
   * Omitted where there is no sub-page to hand off to; the reason still shows,
   * only the one-click repair is absent.
   */
  onOpenProfileSync?: (profile: BrowserProfile) => void;
  /** Opens proxy assignment for profiles with no exit node. */
  onAssignProxy?: (profileIds: string[]) => void;
}

export function CookieBotEnrolDialog({
  isOpen,
  onClose,
  profiles,
  existing,
  onSaved,
  onOpenProfileSync,
  onAssignProxy,
}: CookieBotEnrolDialogProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const { user } = useCloudAuth();
  // The same entitlement answer every other consumer of the shared store
  // passes; see the note in cookie-bot-page.tsx.
  const { quota, refresh: refreshCookieBot } = useCookieBot(
    canUseCookieBot(user),
    cookieBotScopeFor(user),
  );
  const canReplaceOthers =
    !user?.teamId || user.teamRole === "owner" || user.teamRole === "admin";

  const [presets, setPresets] = useState<CookieBotPresetList | null>(null);
  const [isLoadingPresets, setIsLoadingPresets] = useState(false);
  const presetList = useMemo(() => presets?.presets ?? [], [presets]);
  const defaultPreset = useMemo(() => pickDefaultPreset(presets), [presets]);

  /**
   * Read the server's catalogue of intensities.
   *
   * An imperative loader rather than an effect keyed off an attempt counter,
   * because the enrolment cannot name a preset without this: a transient
   * failure of a secondary request disables the PRIMARY action, so the retry
   * has to be a real call the button can make, not a state flip a lint fix can
   * quietly drop from a dependency array.
   */
  const loadPresets = useCallback(async () => {
    setIsLoadingPresets(true);
    try {
      setPresets(await getCookieBotPresets());
    } catch {
      // Losing the catalogue costs the depth control and blocks the save; the
      // note beside the retry button says so.
      setPresets(null);
    } finally {
      setIsLoadingPresets(false);
    }
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    void loadPresets();
  }, [isOpen, loadPresets]);

  const [preset, setPreset] = useState<string>("");
  const [runAt, setRunAt] = useState<string>(
    minutesToClock(DEFAULT_RUN_AT_MINUTE),
  );
  const [daysMask, setDaysMask] = useState<number>(CADENCES[0].mask);
  const [maxMinutes, setMaxMinutes] = useState<number>(FALLBACK_MAX_MINUTES);
  const [maxMinutesTouched, setMaxMinutesTouched] = useState(false);
  const [sitesText, setSitesText] = useState("");
  const [adjustOpen, setAdjustOpen] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [fixingId, setFixingId] = useState<string | null>(null);
  const [conflict, setConflict] = useState<ConflictNotice | null>(null);
  const [conflictAcknowledged, setConflictAcknowledged] = useState(false);

  const isEdit = Boolean(existing);
  const single = profiles.length === 1 ? profiles[0] : null;

  // Reset to the defaults every time the dialog is opened, so a previous
  // enrolment's answers never leak into the next one.
  useEffect(() => {
    if (!isOpen) return;
    setPreset(existing?.preset ?? defaultPreset?.id ?? "");
    setRunAt(minutesToClock(existing?.run_at_minute ?? DEFAULT_RUN_AT_MINUTE));
    setDaysMask(existing?.days_mask ?? CADENCES[0].mask);
    setMaxMinutes(
      existing?.max_minutes ??
        defaultPreset?.typical_minutes ??
        FALLBACK_MAX_MINUTES,
    );
    setMaxMinutesTouched(Boolean(existing));
    setSitesText((existing?.sites ?? []).join("\n"));
    setAdjustOpen(false);
    setConflict(null);
    setConflictAcknowledged(false);
    setIsSaving(false);
  }, [isOpen, existing, defaultPreset]);

  // Switching depth moves the cap with it, until the operator sets their own.
  useEffect(() => {
    if (maxMinutesTouched) return;
    const chosen = presetList.find((p) => p.id === preset);
    if (chosen?.typical_minutes) setMaxMinutes(chosen.typical_minutes);
  }, [preset, presetList, maxMinutesTouched]);

  const targets: EnrolTarget[] = useMemo(
    () => profiles.map((profile) => ({ profile, check: preflight(profile) })),
    [profiles],
  );
  const eligible = useMemo(
    () => targets.filter((target) => target.check.eligible),
    [targets],
  );
  const blocked = useMemo(
    () => targets.filter((target) => !target.check.eligible),
    [targets],
  );

  const runAtMinute = clockToMinutes(runAt);
  const sites = useMemo(() => normaliseSites(sitesText), [sitesText]);
  const sitesTooMany = sites.length > MAX_SITES;
  // v1 browses the user's declared sites and nothing else, so an empty list is
  // not a schedule the server can accept — it 400s with COOKIE_BOT_SITE_LIMIT
  // and, before this, `canSubmit` did not ask. The three-click happy path
  // (bot cell -> Enrol -> "Enrol tonight") posted `sites: []` and failed every
  // single time, with the only input the bot cannot run without hidden inside a
  // collapsed disclosure.
  const sitesTooFew = sites.length < MIN_SITES;
  const maxMinutesValid =
    Number.isFinite(maxMinutes) &&
    maxMinutes >= MIN_MAX_MINUTES &&
    maxMinutes <= MAX_MAX_MINUTES;
  const presetsUnavailable = presets === null;

  // A week's machine time from the operator's own two numbers. The budget it is
  // compared against is the server's; nothing here decides entitlement.
  const weeklyHours = (nightsPerWeek(daysMask) * maxMinutes) / 60;
  const remainingHours = quota?.remaining_hours ?? null;
  const overBudget =
    remainingHours !== null && weeklyHours > remainingHours && !isEdit;

  const canSubmit =
    eligible.length > 0 &&
    preset.length > 0 &&
    runAtMinute !== null &&
    maxMinutesValid &&
    !sitesTooMany &&
    !sitesTooFew &&
    !isSaving;

  // A single-profile enrolment asks the server up front whether a teammate
  // already owns this profile's night, so the one decision that matters is
  // made before the user commits rather than after.
  useEffect(() => {
    if (!isOpen || !single || isEdit) return;
    let cancelled = false;
    void checkCookieBotConflicts(single.id, {})
      .then((found) => {
        if (cancelled) return;
        const overlapping = found.filter((c) => c.enabled);
        if (overlapping.length === 0) return;
        setConflict(toNotice(overlapping[0], [single.id]));
      })
      .catch(() => {
        // A conflict check that cannot run is not a reason to block enrolment;
        // the save path re-detects the same 409 and shows the same block.
      });
    return () => {
      cancelled = true;
    };
  }, [isOpen, single, isEdit]);

  const buildInput = useCallback(
    (profile: BrowserProfile): CookieBotScheduleInput | null => {
      const minute = clockToMinutes(runAt);
      const platform = resolvedOs(profile);
      if (minute === null || !platform) return null;
      return {
        profile_name: profile.name,
        platform: platform as CookieBotPlatform,
        enabled: true,
        run_at_minute: minute,
        days_mask: daysMask,
        timezone: profileTimezone(profile),
        preset,
        max_minutes: Math.round(maxMinutes),
        sites,
      };
    },
    [runAt, daysMask, preset, maxMinutes, sites],
  );

  const submit = useCallback(
    async (acknowledge: boolean, only?: string[]) => {
      const list = only
        ? eligible.filter((target) => only.includes(target.profile.id))
        : eligible;
      if (list.length === 0) return;
      setIsSaving(true);
      let saved = 0;
      const conflicted: string[] = [];
      let firstError: unknown = null;
      let conflictParams: { email: string; time: string } | null = null;

      for (const target of list) {
        const input = buildInput(target.profile);
        if (!input) continue;
        try {
          await saveCookieBotSchedule(target.profile.id, input, acknowledge);
          saved += 1;
        } catch (error) {
          const parsed = parseBackendError(error);
          if (parsed?.code === "COOKIE_BOT_SCHEDULE_CONFLICT") {
            conflicted.push(target.profile.id);
            if (!conflictParams) {
              conflictParams = {
                email: parsed.params?.email ?? "",
                time: parsed.params?.time ?? minutesToClock(runAtMinute ?? 0),
              };
            }
            continue;
          }
          if (!firstError) firstError = error;
        }
      }

      setIsSaving(false);

      if (conflicted.length > 0 && conflictParams) {
        setConflict({
          email: conflictParams.email,
          time: conflictParams.time,
          profileIds: conflicted,
        });
        if (saved > 0) {
          void refreshCookieBot();
          onSaved?.();
        }
        return;
      }

      if (firstError) {
        showErrorToast(translateBackendError(t, firstError));
        if (saved > 0) {
          void refreshCookieBot();
          onSaved?.();
        }
        return;
      }

      if (saved > 0) {
        showSuccessToast(
          isEdit
            ? t("cookieBot.enrol.saved")
            : t("cookieBot.enrol.enrolled", { count: saved }),
        );
        void refreshCookieBot();
        onSaved?.();
        onClose();
      }
    },
    [
      eligible,
      buildInput,
      isEdit,
      onSaved,
      onClose,
      t,
      runAtMinute,
      refreshCookieBot,
    ],
  );

  const applyFix = useCallback(
    async (target: EnrolTarget) => {
      const { profile, check } = target;
      if (check.fix === "proxy") {
        onAssignProxy?.([profile.id]);
        return;
      }
      if (check.fix === "syncSettings") {
        onOpenProfileSync?.(profile);
        return;
      }
      if (check.fix !== "sync") return;
      setFixingId(profile.id);
      try {
        await enableProfileSync(profile.id);
      } catch (error) {
        showErrorToast(
          parseBackendError(error)
            ? translateBackendError(t, error)
            : t("cookieBot.preflight.fixFailed"),
        );
      } finally {
        setFixingId(null);
      }
    },
    [onAssignProxy, onOpenProfileSync, t],
  );

  const showConflict = conflict !== null && !conflictAcknowledged;

  const title = isEdit
    ? t("cookieBot.enrol.editTitle")
    : single
      ? t("cookieBot.enrol.titleOne", { name: single.name })
      : t("cookieBot.enrol.titleCount", { count: profiles.length });

  const cadenceId = cadenceForMask(daysMask);
  // One complete sentence per cadence rather than a label spliced into a
  // fragment: "Runs Nightly at 02:00" only reads correctly in English, and a
  // translator needs the whole clause to reorder.
  const summaryKey = cadenceId
    ? `cookieBot.enrol.summary${cadenceId[0].toUpperCase()}${cadenceId.slice(1)}`
    : "cookieBot.enrol.summaryCustom";

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="flex max-h-[80vh] max-w-md flex-col">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {t("cookieBot.enrol.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto">
          {/* The whole default, said once, as one sentence. One key, not five
              fragments: a translator has to be free to reorder it. */}
          <p className="text-sm tabular-nums text-foreground">
            {t(summaryKey, {
              count: nightsPerWeek(daysMask),
              time: minutesToClock(runAtMinute ?? DEFAULT_RUN_AT_MINUTE),
              minutes: Math.round(maxMinutes),
            })}
          </p>

          <div className="flex flex-col gap-1">
            <p
              className={cn(
                "text-xs tabular-nums",
                overBudget ? "text-warning-text" : "text-muted-foreground",
              )}
            >
              {remainingHours === null
                ? t("cookieBot.hours.estimateOnly", {
                    hours: formatHours(weeklyHours),
                  })
                : overBudget
                  ? t("cookieBot.hours.estimateOverBudget", {
                      hours: formatHours(weeklyHours),
                      remaining: formatHours(remainingHours),
                    })
                  : t("cookieBot.hours.estimate", {
                      hours: formatHours(weeklyHours),
                      remaining: formatHours(remainingHours),
                    })}
            </p>
            <RemoteHoursMeter
              quota={quota}
              isLoading={false}
              variant="inline"
            />
          </div>

          {blocked.length > 0 && (
            <div className="flex flex-col gap-2 rounded-md border border-warning/50 bg-warning/10 p-3">
              <p className="text-xs font-medium text-warning-text">
                {t("cookieBot.preflight.ineligible", {
                  count: blocked.length,
                })}
              </p>
              {blocked.map((target) => {
                const reachable =
                  target.check.fix === "sync" ||
                  (target.check.fix === "proxy" && Boolean(onAssignProxy)) ||
                  (target.check.fix === "syncSettings" &&
                    Boolean(onOpenProfileSync));
                const fixLabel = reachable
                  ? preflightFixLabel(t, target.check.fix)
                  : null;
                return (
                  <div
                    key={target.profile.id}
                    className="flex h-7 items-center gap-2 text-xs"
                  >
                    <span className="min-w-0 flex-1 truncate text-foreground">
                      {target.profile.name}
                    </span>
                    <span className="flex shrink-0 items-center gap-1 text-muted-foreground">
                      {preflightReason(t, target.check)}
                      {target.check.code === "noExitNode" && <ExitNodeHint />}
                    </span>
                    {fixLabel && (
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-6 shrink-0 text-[11px]"
                        disabled={fixingId === target.profile.id}
                        onClick={() => {
                          void applyFix(target);
                        }}
                      >
                        {fixLabel}
                      </Button>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          {/* Sites is not an adjustment: v1 browses the user's declared list
              and nothing else, so it is the one input without which there is no
              run. It sat inside the collapsed "Adjust schedule" disclosure,
              which made the default path a form the server always refused. */}
          <Field label={t("cookieBot.enrol.sitesLabel")}>
            <Textarea
              value={sitesText}
              onChange={(event) => {
                setSitesText(event.target.value);
              }}
              rows={4}
              placeholder={t("cookieBot.enrol.sitesPlaceholder")}
              className="text-xs"
              aria-invalid={sitesTooMany || sitesTooFew}
            />
            <p
              className={cn(
                "mt-1 text-[11px]",
                sitesTooMany
                  ? "text-destructive-text"
                  : "text-muted-foreground",
              )}
            >
              {sitesTooMany
                ? t("cookieBot.enrol.sitesTooMany", { max: MAX_SITES })
                : sitesTooFew
                  ? t("cookieBot.enrol.sitesRequired")
                  : t("cookieBot.enrol.sitesHint", { count: sites.length })}
            </p>
          </Field>

          <div className="rounded-md border border-border">
            <button
              type="button"
              onClick={() => {
                setAdjustOpen((open) => !open);
              }}
              aria-expanded={adjustOpen}
              className="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-left text-xs font-medium text-foreground transition-colors duration-100 hover:bg-accent hover:text-accent-foreground"
            >
              <motion.span
                aria-hidden="true"
                animate={{ rotate: adjustOpen ? 90 : 0 }}
                transition={{
                  duration: reduceMotion ? 0 : 0.16,
                  ease: MOTION_EASE_OUT,
                }}
                className="inline-flex shrink-0"
              >
                <LuChevronRight className="size-3.5" />
              </motion.span>
              {t("cookieBot.enrol.adjust")}
            </button>

            <AutoHeight deps={[adjustOpen, presetList.length]}>
              <AnimatePresence initial={false}>
                {adjustOpen && (
                  <motion.div
                    initial={{ opacity: 0, y: reduceMotion ? 0 : -4 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: reduceMotion ? 0 : -4 }}
                    transition={{
                      duration: reduceMotion ? 0.15 : 0.16,
                      ease: MOTION_EASE_OUT,
                    }}
                    className="flex flex-col gap-3 border-t border-border px-3 py-3"
                  >
                    <Field label={t("cookieBot.enrol.cadenceLabel")}>
                      <AnimatedTabs
                        value={cadenceId ?? "custom"}
                        onValueChange={(value) => {
                          const match = CADENCES.find(
                            (c) => c.id === (value as CadenceId),
                          );
                          if (match) setDaysMask(match.mask);
                        }}
                      >
                        <AnimatedTabsList>
                          {CADENCES.map((cadence) => (
                            <AnimatedTabsTrigger
                              key={cadence.id}
                              value={cadence.id}
                              className="h-7 px-2.5 text-xs"
                            >
                              {t(cadence.labelKey)}
                            </AnimatedTabsTrigger>
                          ))}
                        </AnimatedTabsList>
                      </AnimatedTabs>
                    </Field>

                    <div className="flex gap-4">
                      <Field label={t("cookieBot.enrol.timeLabel")}>
                        <Input
                          type="time"
                          value={runAt}
                          onChange={(event) => {
                            setRunAt(event.target.value);
                          }}
                          className="h-8 w-28 font-mono tabular-nums"
                        />
                      </Field>
                      <Field label={t("cookieBot.enrol.maxMinutesLabel")}>
                        <Input
                          type="number"
                          min={MIN_MAX_MINUTES}
                          max={MAX_MAX_MINUTES}
                          value={String(maxMinutes)}
                          onChange={(event) => {
                            setMaxMinutesTouched(true);
                            setMaxMinutes(Number(event.target.value));
                          }}
                          className="h-8 w-24 tabular-nums"
                        />
                      </Field>
                    </div>
                    <p className="text-[11px] text-muted-foreground">
                      {t("cookieBot.enrol.timeHint")}
                    </p>

                    {presetList.length > 0 && (
                      <Field label={t("cookieBot.enrol.intensityLabel")}>
                        <AnimatedTabs
                          value={preset}
                          onValueChange={(value) => {
                            setPreset(value);
                            setMaxMinutesTouched(false);
                          }}
                        >
                          <AnimatedTabsList>
                            {presetList.map((item) => (
                              <AnimatedTabsTrigger
                                key={item.id}
                                value={item.id}
                                className="h-7 px-2.5 text-xs"
                              >
                                {presetLabel(t, item)}
                              </AnimatedTabsTrigger>
                            ))}
                          </AnimatedTabsList>
                        </AnimatedTabs>
                      </Field>
                    )}
                  </motion.div>
                )}
              </AnimatePresence>
            </AutoHeight>
          </div>

          {presetsUnavailable && (
            <div className="flex items-center gap-2">
              <p className="min-w-0 flex-1 text-xs text-muted-foreground">
                {t("cookieBot.enrol.presetsUnavailable")}
              </p>
              <Button
                variant="outline"
                size="sm"
                className="h-6 shrink-0 text-[11px]"
                disabled={isLoadingPresets}
                onClick={() => {
                  void loadPresets();
                }}
              >
                {t("common.buttons.retry")}
              </Button>
            </div>
          )}
        </div>

        {/* The footer is one slot: either the actions, or the single decision
            a teammate's existing enrolment forces. StepTransition is the shell's
            own forward/back language, reused rather than reinvented, and this
            genuinely is a step. */}
        <StepTransition
          transitionKey={showConflict ? "conflict" : "actions"}
          direction={showConflict ? 1 : -1}
          className="shrink-0"
        >
          {showConflict && conflict ? (
            <div className="flex flex-col gap-2 border-t border-border pt-3">
              <p className="text-sm font-medium text-foreground">
                {t("cookieBot.conflict.title", {
                  email: conflict.email,
                  time: conflict.time,
                })}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("cookieBot.conflict.detail")}
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <Button variant="outline" size="sm" onClick={onClose}>
                  {t("cookieBot.conflict.keepTheirs")}
                </Button>
                {canReplaceOthers ? (
                  <RippleButton
                    size="sm"
                    disabled={isSaving}
                    onClick={() => {
                      setConflictAcknowledged(true);
                      void submit(true, conflict.profileIds);
                    }}
                  >
                    {t("cookieBot.conflict.replace")}
                  </RippleButton>
                ) : (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="inline-flex">
                        <Button size="sm" disabled>
                          {t("cookieBot.conflict.replace")}
                        </Button>
                      </span>
                    </TooltipTrigger>
                    <TooltipContent>
                      {t("cookieBot.conflict.replaceForbidden")}
                    </TooltipContent>
                  </Tooltip>
                )}
                {conflict.email.length > 0 && (
                  <Button
                    variant="ghost"
                    size="sm"
                    className="text-xs text-muted-foreground hover:text-foreground"
                    onClick={() => {
                      void navigator.clipboard
                        .writeText(conflict.email)
                        .then(() => {
                          showSuccessToast(t("cookieBot.conflict.emailCopied"));
                        })
                        .catch(() => {
                          showErrorToast(t("cookieBot.conflict.copyFailed"));
                        });
                    }}
                  >
                    {t("cookieBot.conflict.askThem", { email: conflict.email })}
                  </Button>
                )}
              </div>
            </div>
          ) : (
            <div className="flex items-center justify-end gap-2 pt-1">
              <Button variant="outline" size="sm" onClick={onClose}>
                {t("common.buttons.cancel")}
              </Button>
              <RippleButton
                size="sm"
                autoFocus
                disabled={!canSubmit}
                onClick={() => {
                  void submit(conflictAcknowledged);
                }}
              >
                {confirmLabel(t, {
                  isEdit,
                  eligible: eligible.length,
                  total: targets.length,
                  saving: isSaving,
                  needsSites: sitesTooFew,
                  needsPreset: preset.length === 0,
                })}
              </RippleButton>
            </div>
          )}
        </StepTransition>
      </DialogContent>
    </Dialog>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      {children}
    </div>
  );
}

/**
 * Why a proxy is not optional, at the point where the refusal happens. A run
 * without one leaves the fleet's own datacenter address, and hours of traffic
 * from a hosting ASN costs the profile more than never warming it.
 */
function ExitNodeHint() {
  const { t } = useTranslation();
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="inline-flex cursor-default text-warning-text">
          <LuInfo className="size-3.5" />
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-64">
        {t("cookieBot.preflight.exitNodeHint")}
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * What the confirm button says, including WHY it is disabled.
 *
 * A greyed "Enrol tonight" with the explanation twelve pixels away in a
 * different colour is a dead end: the user clicks a button that answers
 * nothing. Each blocked state names itself instead.
 */
function confirmLabel(
  t: ReturnType<typeof useTranslation>["t"],
  state: {
    isEdit: boolean;
    eligible: number;
    total: number;
    saving: boolean;
    needsSites: boolean;
    needsPreset: boolean;
  },
): string {
  if (state.saving) return t("cookieBot.enrol.saving");
  if (state.eligible === 0 && !state.isEdit)
    return t("cookieBot.enrol.fixFirst");
  if (state.needsSites) return t("cookieBot.enrol.addSitesFirst");
  if (state.needsPreset) return t("cookieBot.enrol.presetsMissing");
  if (state.isEdit) return t("common.buttons.save");
  if (state.eligible < state.total) {
    return t("cookieBot.enrol.confirmSome", {
      eligible: state.eligible,
      total: state.total,
    });
  }
  return t("cookieBot.enrol.confirm");
}

function presetLabel(
  t: ReturnType<typeof useTranslation>["t"],
  preset: CookieBotPreset,
): string {
  const key = PRESET_LABEL_KEYS[preset.id];
  if (key) return t(key);
  // A preset newer than this build still renders: the server ships an English
  // label with it, which beats printing a bare id.
  return preset.name ?? preset.id;
}

function pickDefaultPreset(
  presets: CookieBotPresetList | null,
): CookieBotPreset | null {
  if (!presets) return null;
  const byId = presets.default_preset
    ? presets.presets.find((p) => p.id === presets.default_preset)
    : undefined;
  return (
    byId ??
    presets.presets.find((p) => p.recommended) ??
    presets.presets[0] ??
    null
  );
}

/**
 * The operator's own list, tidied: one entry per line, a bare host promoted to
 * https, duplicates dropped. No entry is ever added that the user did not type.
 */
function normaliseSites(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const withScheme = /^https?:\/\//i.test(line) ? line : `https://${line}`;
    if (seen.has(withScheme)) continue;
    seen.add(withScheme);
    out.push(withScheme);
  }
  return out;
}

function toNotice(
  conflict: CookieBotConflict,
  profileIds: string[],
): ConflictNotice {
  return {
    email: conflict.email,
    time: minutesToClock(conflict.run_at_minute),
    profileIds,
  };
}

"use client";

import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuChevronRight,
  LuInfo,
  LuPencil,
  LuPlus,
  LuTrash2,
  LuX,
} from "react-icons/lu";
import {
  cadenceForMask,
  clockToMinutes,
  DAYS_NIGHTLY,
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
  scheduleSlots,
  templateErrorMessage,
  weekdayNames,
  weeklyRuns,
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
  type CookieBotSlot,
  type CookieBotTemplate,
  type CookieBotUserTemplate,
  checkCookieBotConflicts,
  createCookieBotUserTemplate,
  deleteCookieBotUserTemplate,
  getCookieBotPresets,
  getCookieBotUserTemplates,
  isUserTemplateId,
  saveCookieBotSchedule,
  updateCookieBotUserTemplate,
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

/**
 * The two bounds the server publishes in `presets().limits` and nothing else
 * mirrors. Used only when a deployment predates that object: reading a missing
 * bound as 0 would leave "Add a time" permanently disabled and refuse every
 * name the user could type.
 */
const FALLBACK_MAX_SLOTS = 14;
const FALLBACK_MAX_TEMPLATE_NAME = 80;

/** The default start: deep enough into the night to be plausible anywhere. */
const DEFAULT_RUN_AT_MINUTE = 2 * 60;

const PRESET_LABEL_KEYS: Record<string, string> = {
  light: "cookieBot.preset.light",
  balanced: "cookieBot.preset.balanced",
  deep: "cookieBot.preset.deep",
};

/**
 * Local names for the curated templates this build knows, on the same contract
 * as the presets above: the server ships English so a template added after this
 * release still renders, and a recognised id uses the translated label instead.
 */
const TEMPLATE_LABEL_KEYS: Record<string, { name: string; hint: string }> = {
  "low-intent-purchaser": {
    name: "cookieBot.template.lowIntentPurchaser",
    hint: "cookieBot.template.lowIntentPurchaserHint",
  },
};

/** Where this enrolment's sites come from. */
type SiteSource = "own" | "template" | "saved";

/**
 * One row of the calendar, as the form holds it.
 *
 * The time stays a string because that is what a `<input type="time">` owns: a
 * half-typed "0" is a state the user passes through, and round-tripping it
 * through minutes would rewrite the field under the cursor.
 */
interface SlotDraft {
  /** Stable identity, so removing a row does not re-key the ones after it. */
  key: string;
  daysMask: number;
  runAt: string;
}

let slotKeySeq = 0;

function makeSlot(daysMask: number, runAt: string): SlotDraft {
  slotKeySeq += 1;
  return { key: `slot-${slotKeySeq}`, daysMask, runAt };
}

/** What a profile that has never been enrolled starts on: every night, 02:00. */
function defaultSlot(): SlotDraft {
  return makeSlot(DAYS_NIGHTLY, minutesToClock(DEFAULT_RUN_AT_MINUTE));
}

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
  const templateList = useMemo(() => presets?.templates ?? [], [presets]);
  const defaultPreset = useMemo(() => pickDefaultPreset(presets), [presets]);
  const maxSlots = presets?.limits?.max_slots ?? FALLBACK_MAX_SLOTS;
  const maxNameLength =
    presets?.limits?.max_template_name_length ?? FALLBACK_MAX_TEMPLATE_NAME;

  /**
   * Read the server's catalogue of intensities and curated templates.
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

  const [userTemplates, setUserTemplates] = useState<
    CookieBotUserTemplate[] | null
  >(null);
  const [templatesFailed, setTemplatesFailed] = useState(false);
  const [isLoadingTemplates, setIsLoadingTemplates] = useState(false);

  /**
   * The user's own saved lists.
   *
   * Kept apart from the preset load rather than folded into it: these are the
   * only thing on this screen a user can create, so a failure here has to stay
   * visible and retryable next to the list itself, while a preset failure
   * blocks the save outright.
   */
  const loadUserTemplates = useCallback(async () => {
    setIsLoadingTemplates(true);
    try {
      setUserTemplates(await getCookieBotUserTemplates());
      setTemplatesFailed(false);
    } catch {
      setUserTemplates(null);
      setTemplatesFailed(true);
    } finally {
      setIsLoadingTemplates(false);
    }
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    void loadPresets();
    void loadUserTemplates();
  }, [isOpen, loadPresets, loadUserTemplates]);

  const [preset, setPreset] = useState<string>("");
  const [slots, setSlots] = useState<SlotDraft[]>(() => [defaultSlot()]);
  const [maxMinutes, setMaxMinutes] = useState<number>(FALLBACK_MAX_MINUTES);
  const [maxMinutesTouched, setMaxMinutesTouched] = useState(false);
  const [source, setSource] = useState<SiteSource>("own");
  const [templateId, setTemplateId] = useState("");
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
    // `scheduleSlots` and not `existing.slots`: a server that predates
    // multi-slot sends only the mirrored pair, and reading the field directly
    // would open the editor on an empty calendar and then save that emptiness
    // over a schedule that was firing perfectly well.
    const restored = (existing ? scheduleSlots(existing) : []).map((slot) =>
      makeSlot(slot.days_mask, minutesToClock(slot.run_at_minute)),
    );
    setSlots(restored.length > 0 ? restored : [defaultSlot()]);
    setMaxMinutes(
      existing?.max_minutes ??
        defaultPreset?.typical_minutes ??
        FALLBACK_MAX_MINUTES,
    );
    setMaxMinutesTouched(Boolean(existing));
    const storedTemplate = existing?.template_id ?? "";
    setTemplateId(storedTemplate);
    setSource(
      storedTemplate
        ? isUserTemplateId(storedTemplate)
          ? "saved"
          : "template"
        : "own",
    );
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

  /** The calendar as the wire spells it, with unreadable times dropped. */
  const wireSlots = useMemo<CookieBotSlot[]>(() => {
    const out: CookieBotSlot[] = [];
    for (const slot of slots) {
      const minute = clockToMinutes(slot.runAt);
      if (minute === null || slot.daysMask === 0) continue;
      out.push({ days_mask: slot.daysMask, run_at_minute: minute });
    }
    return out;
  }, [slots]);

  // A row the user is still typing into is not a row that can be saved, and it
  // is not an error either — the confirm button names the missing piece.
  const slotsIncomplete = wireSlots.length !== slots.length;
  const firstMinute = wireSlots[0]?.run_at_minute ?? DEFAULT_RUN_AT_MINUTE;

  /**
   * Rows that repeat a (days, time) another row already claims.
   *
   * The server de-duplicates a slot list before it stores it, so submitting two
   * identical rows silently stores one: the calendar comes back from the next
   * open a row shorter, at a different length than the user left it, with
   * nothing having said which row went or why. Refused here instead — the row
   * is marked and the confirm button names the reason, the same contract the
   * empty-weekday case already follows.
   *
   * Only an EXACT repeat counts. Two rows sharing one weekday at one minute
   * also collapse to a single run, but they still describe nights the other
   * does not, so the hours estimate accounts for them (see `weeklyRuns`) rather
   * than the form refusing them.
   */
  const duplicateSlotKeys = useMemo(() => {
    const claimed = new Set<number>();
    const repeats = new Set<string>();
    for (const slot of slots) {
      const minute = clockToMinutes(slot.runAt);
      if (minute === null || slot.daysMask === 0) continue;
      const packed = minute * 128 + slot.daysMask;
      if (claimed.has(packed)) repeats.add(slot.key);
      else claimed.add(packed);
    }
    return repeats;
  }, [slots]);
  const hasDuplicateSlot = duplicateSlotKeys.size > 0;

  const sites = useMemo(() => normaliseSites(sitesText), [sitesText]);
  const usingTemplate = source !== "own";
  const chosenSaved = useMemo(
    () => userTemplates?.find((item) => item.id === templateId) ?? null,
    [userTemplates, templateId],
  );
  // A saved list this enrolment names but the account no longer has — deleted
  // from another device, most likely. Saving would 404, so it is said here
  // rather than after the round trip.
  const savedMissing =
    source === "saved" &&
    templateId.length > 0 &&
    userTemplates !== null &&
    chosenSaved === null;

  const sitesTooMany = !usingTemplate && sites.length > MAX_SITES;
  // v1 browses the user's declared sites and nothing else, so an empty list is
  // not a schedule the server can accept — it 400s with COOKIE_BOT_SITE_LIMIT
  // and, before this, `canSubmit` did not ask. The three-click happy path
  // (bot cell -> Enrol -> "Enrol tonight") posted `sites: []` and failed every
  // single time, with the only input the bot cannot run without hidden inside a
  // collapsed disclosure. A template is the other way to answer the same
  // question, so it satisfies this too.
  const sitesTooFew = !usingTemplate && sites.length < MIN_SITES;
  const templateMissing = usingTemplate && templateId.length === 0;
  const maxMinutesValid =
    Number.isFinite(maxMinutes) &&
    maxMinutes >= MIN_MAX_MINUTES &&
    maxMinutes <= MAX_MAX_MINUTES;
  const presetsUnavailable = presets === null;

  // A week's machine time from the operator's own numbers, summed across every
  // slot: three times a night costs three times the hours, and this is the only
  // place that shows it before the commit. The budget it is compared against is
  // the server's; nothing here decides entitlement.
  const runsPerWeek = weeklyRuns(wireSlots);
  const weeklyHours = (runsPerWeek * maxMinutes) / 60;
  const remainingHours = quota?.remaining_hours ?? null;
  const overBudget =
    remainingHours !== null && weeklyHours > remainingHours && !isEdit;

  const canSubmit =
    eligible.length > 0 &&
    preset.length > 0 &&
    wireSlots.length > 0 &&
    !slotsIncomplete &&
    !hasDuplicateSlot &&
    wireSlots.length <= maxSlots &&
    maxMinutesValid &&
    !sitesTooMany &&
    !sitesTooFew &&
    !templateMissing &&
    !savedMissing &&
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
      const platform = resolvedOs(profile);
      if (wireSlots.length === 0 || !platform) return null;
      return {
        profile_name: profile.name,
        platform: platform as CookieBotPlatform,
        enabled: true,
        // The pair is the mirror of the first slot, sent for a server that
        // predates multi-slot scheduling: it stores the first time rather than
        // nothing at all.
        run_at_minute: wireSlots[0].run_at_minute,
        days_mask: wireSlots[0].days_mask,
        slots: wireSlots,
        timezone: profileTimezone(profile),
        preset,
        // A template stands IN FOR the site list. The server refuses a body
        // carrying both, so the unused half is sent empty rather than stale.
        template_id: usingTemplate ? templateId : undefined,
        max_minutes: Math.round(maxMinutes),
        sites: usingTemplate ? [] : sites,
      };
    },
    [wireSlots, preset, maxMinutes, sites, usingTemplate, templateId],
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
                time: parsed.params?.time ?? minutesToClock(firstMinute),
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
        // A refusal that names a saved list is translated as one: the shared
        // table does not carry the template codes, and "Something went wrong:
        // COOKIE_BOT_TEMPLATE_NOT_FOUND" tells nobody to pick another list.
        showErrorToast(templateErrorMessage(t, firstError));
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
      firstMinute,
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

  // One complete sentence per cadence rather than a label spliced into a
  // fragment: "Runs Nightly at 02:00" only reads correctly in English, and a
  // translator needs the whole clause to reorder. A calendar with more than one
  // start time has no single hour to name, so it gets its own sentence instead
  // of the first slot's time standing in for all of them.
  const oneSlot = wireSlots.length === 1;
  const cadenceId = oneSlot ? cadenceForMask(wireSlots[0].days_mask) : null;
  const summaryKey = !oneSlot
    ? "cookieBot.enrol.summarySlots"
    : cadenceId
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
              count: oneSlot
                ? nightsPerWeek(wireSlots[0].days_mask)
                : runsPerWeek,
              time: minutesToClock(firstMinute),
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

          {/* Sites is not an adjustment: v1 browses one declared list and
              nothing else, so it is the one input without which there is no
              run. It sat inside the collapsed "Adjust schedule" disclosure,
              which made the default path a form the server always refused. */}
          <Field label={t("cookieBot.enrol.sitesLabel")}>
            <AnimatedTabs
              value={source}
              onValueChange={(value) => {
                setSource(value as SiteSource);
                // The chosen id belongs to exactly one tab. Carrying a built-in
                // id into the saved tab would show nothing selected and still
                // submit the built-in.
                setTemplateId("");
              }}
            >
              <AnimatedTabsList>
                <AnimatedTabsTrigger value="own" className="h-7 px-2.5 text-xs">
                  {t("cookieBot.enrol.sourceOwn")}
                </AnimatedTabsTrigger>
                <AnimatedTabsTrigger
                  value="template"
                  className="h-7 px-2.5 text-xs"
                >
                  {t("cookieBot.enrol.sourceCurated")}
                </AnimatedTabsTrigger>
                <AnimatedTabsTrigger
                  value="saved"
                  className="h-7 px-2.5 text-xs"
                >
                  {t("cookieBot.enrol.sourceSaved")}
                </AnimatedTabsTrigger>
              </AnimatedTabsList>
            </AnimatedTabs>

            <AutoHeight
              className="mt-2"
              deps={[
                source,
                sitesText,
                templateId,
                templateList.length,
                userTemplates?.length ?? -1,
                isLoadingTemplates,
              ]}
            >
              <AnimatePresence initial={false} mode="wait">
                <motion.div
                  key={source}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{
                    duration: reduceMotion ? 0 : 0.12,
                    ease: MOTION_EASE_OUT,
                  }}
                >
                  {source === "own" && (
                    <OwnSitesPanel
                      value={sitesText}
                      onChange={setSitesText}
                      sites={sites}
                      tooMany={sitesTooMany}
                      tooFew={sitesTooFew}
                      maxNameLength={maxNameLength}
                      onSaved={(created) => {
                        setUserTemplates((current) => [
                          created,
                          ...(current ?? []),
                        ]);
                      }}
                    />
                  )}
                  {source === "template" && (
                    <CuratedPanel
                      templates={templateList}
                      selectedId={templateId}
                      onSelect={setTemplateId}
                    />
                  )}
                  {source === "saved" && (
                    <SavedPanel
                      templates={userTemplates}
                      isLoading={isLoadingTemplates}
                      failed={templatesFailed}
                      selectedId={templateId}
                      missing={savedMissing}
                      maxNameLength={maxNameLength}
                      onSelect={setTemplateId}
                      onRetry={() => {
                        void loadUserTemplates();
                      }}
                      onChanged={setUserTemplates}
                      onDeselect={() => {
                        setTemplateId("");
                      }}
                    />
                  )}
                </motion.div>
              </AnimatePresence>
            </AutoHeight>
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

            <AutoHeight deps={[adjustOpen, presetList.length, slots]}>
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
                    <Field label={t("cookieBot.enrol.calendarLabel")}>
                      <div className="flex flex-col gap-1.5">
                        {slots.map((slot, index) => (
                          <SlotRow
                            key={slot.key}
                            slot={slot}
                            duplicate={duplicateSlotKeys.has(slot.key)}
                            canRemove={slots.length > 1}
                            onChange={(next) => {
                              setSlots((current) =>
                                current.map((item, at) =>
                                  at === index ? { ...item, ...next } : item,
                                ),
                              );
                            }}
                            onRemove={() => {
                              setSlots((current) =>
                                current.filter((_, at) => at !== index),
                              );
                            }}
                          />
                        ))}
                      </div>
                      {slots.length < maxSlots ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="mt-1 h-6 w-fit gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                          onClick={() => {
                            // The new row copies the last one's days and lands
                            // an hour later: a second run at the same minute as
                            // the first is the one thing it must not default to.
                            const last = slots[slots.length - 1];
                            const minute = clockToMinutes(last?.runAt ?? "");
                            setSlots((current) => [
                              ...current,
                              makeSlot(
                                last?.daysMask ?? DAYS_NIGHTLY,
                                minutesToClock((minute ?? 0) + 60),
                              ),
                            ]);
                          }}
                        >
                          <LuPlus className="size-3" />
                          {t("cookieBot.enrol.addSlot")}
                        </Button>
                      ) : (
                        <p className="mt-1 text-[11px] text-muted-foreground">
                          {t("cookieBot.enrol.slotsFull", { max: maxSlots })}
                        </p>
                      )}
                      <p className="text-[11px] text-muted-foreground">
                        {t("cookieBot.enrol.timeHint")}
                      </p>
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
                  needsTemplate: templateMissing || savedMissing,
                  needsSlot: slotsIncomplete || wireSlots.length === 0,
                  duplicateSlot: hasDuplicateSlot,
                  overSlotCap: wireSlots.length > maxSlots,
                  maxSlots,
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

/* -------------------------------------------------------------------------- */
/* Calendar                                                                   */
/* -------------------------------------------------------------------------- */

/**
 * One weekday-set and one time.
 *
 * Days are a toggle strip rather than the three named cadences this control
 * used to offer: those name a mask, and a calendar of several rows needs each
 * row to be able to say something the three cannot. A single row with every day
 * lit is still "every night", which is what the sentence at the top of the
 * dialog goes on calling it.
 */
function SlotRow({
  slot,
  duplicate,
  canRemove,
  onChange,
  onRemove,
}: {
  slot: SlotDraft;
  /** This row repeats a (days, time) an earlier row already claims. */
  duplicate: boolean;
  canRemove: boolean;
  onChange: (next: Partial<SlotDraft>) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const days = useMemo(() => weekdayNames(), []);
  // A row with no day, or a half-typed time, is what the confirm button is
  // refusing to save. Marking it here is what connects that refusal to the
  // control the user has to touch — the button names the reason, this names
  // the row.
  const noDay = slot.daysMask === 0;
  // A repeat is marked on the time rather than the days because changing the
  // time is what resolves it without giving up a night the row was added for.
  const badTime = clockToMinutes(slot.runAt) === null || duplicate;

  return (
    <div className="flex items-center gap-2">
      <div
        role="group"
        aria-label={t("cookieBot.enrol.daysLabel")}
        className={cn(
          "flex shrink-0 gap-0.5 rounded-sm",
          noDay && "ring-1 ring-destructive/60",
        )}
      >
        {days.map((day, index) => {
          const bit = 1 << index;
          const on = (slot.daysMask & bit) !== 0;
          return (
            <button
              key={day.long}
              type="button"
              aria-pressed={on}
              aria-label={day.long}
              title={day.long}
              onClick={() => {
                onChange({ daysMask: slot.daysMask ^ bit });
              }}
              className={cn(
                "size-6 cursor-pointer rounded-sm text-[11px] font-medium transition-colors duration-100",
                on
                  ? "bg-foreground text-background"
                  : "bg-muted text-muted-foreground hover:bg-accent hover:text-accent-foreground",
              )}
            >
              {day.narrow}
            </button>
          );
        })}
      </div>
      <Input
        type="time"
        aria-label={t("cookieBot.enrol.timeLabel")}
        aria-invalid={badTime}
        title={duplicate ? t("cookieBot.enrol.duplicateSlot") : undefined}
        value={slot.runAt}
        onChange={(event) => {
          onChange({ runAt: event.target.value });
        }}
        className="h-7 w-24 font-mono text-xs tabular-nums"
      />
      {canRemove && (
        <button
          type="button"
          aria-label={t("cookieBot.enrol.removeSlot")}
          onClick={onRemove}
          className="inline-flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-sm text-muted-foreground transition-colors duration-100 hover:bg-accent hover:text-foreground"
        >
          <LuX className="size-3.5" />
        </button>
      )}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Site sources                                                               */
/* -------------------------------------------------------------------------- */

/** The user's own list, plus the one-step way to keep it for next time. */
function OwnSitesPanel({
  value,
  onChange,
  sites,
  tooMany,
  tooFew,
  maxNameLength,
  onSaved,
}: {
  value: string;
  onChange: (next: string) => void;
  sites: string[];
  tooMany: boolean;
  tooFew: boolean;
  maxNameLength: number;
  onSaved: (created: CookieBotUserTemplate) => void;
}) {
  const { t } = useTranslation();
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState("");
  const [isSaving, setIsSaving] = useState(false);

  const save = useCallback(async () => {
    if (name.trim().length === 0 || sites.length === 0) return;
    setIsSaving(true);
    try {
      onSaved(await createCookieBotUserTemplate(name.trim(), sites));
      showSuccessToast(t("cookieBot.enrol.listSaved"));
      setNaming(false);
      setName("");
    } catch (error) {
      showErrorToast(templateErrorMessage(t, error));
    } finally {
      setIsSaving(false);
    }
  }, [name, sites, onSaved, t]);

  return (
    <div>
      <Textarea
        value={value}
        onChange={(event) => {
          onChange(event.target.value);
        }}
        rows={4}
        placeholder={t("cookieBot.enrol.sitesPlaceholder")}
        className="text-xs"
        aria-invalid={tooMany || tooFew}
      />
      <p
        className={cn(
          "mt-1 text-[11px]",
          tooMany ? "text-destructive-text" : "text-muted-foreground",
        )}
      >
        {tooMany
          ? t("cookieBot.enrol.sitesTooMany", { max: MAX_SITES })
          : tooFew
            ? t("cookieBot.enrol.sitesRequired")
            : t("cookieBot.enrol.sitesHint", { count: sites.length })}
      </p>

      {sites.length > 0 &&
        (naming ? (
          <div className="mt-2 flex items-center gap-1.5">
            <Input
              autoFocus
              value={name}
              maxLength={maxNameLength}
              placeholder={t("cookieBot.enrol.listNamePlaceholder")}
              onChange={(event) => {
                setName(event.target.value);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void save();
                }
                if (event.key === "Escape") setNaming(false);
              }}
              className="h-7 flex-1 text-xs"
            />
            <Button
              size="sm"
              className="h-7 shrink-0 text-[11px]"
              disabled={isSaving || name.trim().length === 0}
              onClick={() => {
                void save();
              }}
            >
              {t("common.buttons.save")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 shrink-0 text-[11px] text-muted-foreground"
              onClick={() => {
                setNaming(false);
              }}
            >
              {t("common.buttons.cancel")}
            </Button>
          </div>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="mt-1 h-6 w-fit px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
            onClick={() => {
              setNaming(true);
            }}
          >
            {t("cookieBot.enrol.saveAsList")}
          </Button>
        ))}
    </div>
  );
}

/**
 * The curated templates.
 *
 * The addresses are deliberately not shown and the copy says why as the feature
 * it is: the list is maintained server-side, and each profile draws its own
 * sample from it so no two profiles browse the same set. A published list would
 * be one anybody could match on, which is the weakness the sampling exists to
 * avoid.
 */
function CuratedPanel({
  templates,
  selectedId,
  onSelect,
}: {
  templates: CookieBotTemplate[];
  selectedId: string;
  onSelect: (id: string) => void;
}) {
  const { t } = useTranslation();

  if (templates.length === 0) {
    return (
      <p className="text-[11px] text-muted-foreground">
        {t("cookieBot.enrol.curatedEmpty")}
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      {templates.map((template) => {
        const selected = template.id === selectedId;
        const keys = TEMPLATE_LABEL_KEYS[template.id];
        return (
          <button
            key={template.id}
            type="button"
            aria-pressed={selected}
            onClick={() => {
              onSelect(template.id);
            }}
            className={cn(
              "flex w-full cursor-pointer flex-col gap-0.5 rounded-md border px-3 py-2 text-left transition-colors duration-100",
              selected
                ? "border-foreground/40 bg-accent"
                : "border-border hover:bg-accent/50",
            )}
          >
            <span className="flex items-center gap-2">
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground">
                {keys ? t(keys.name) : (template.name ?? template.id)}
              </span>
              <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
                {t("cookieBot.enrol.listSites", {
                  count: template.site_count,
                })}
              </span>
            </span>
            <span className="text-[11px] leading-snug text-muted-foreground">
              {keys ? t(keys.hint) : (template.description ?? "")}
            </span>
          </button>
        );
      })}
      <p className="text-[11px] leading-snug text-muted-foreground">
        {t("cookieBot.enrol.curatedNote")}
      </p>
    </div>
  );
}

/** The user's saved lists: pick one, rename one, delete one. */
function SavedPanel({
  templates,
  isLoading,
  failed,
  selectedId,
  missing,
  maxNameLength,
  onSelect,
  onRetry,
  onChanged,
  onDeselect,
}: {
  templates: CookieBotUserTemplate[] | null;
  isLoading: boolean;
  failed: boolean;
  selectedId: string;
  missing: boolean;
  maxNameLength: number;
  onSelect: (id: string) => void;
  onRetry: () => void;
  onChanged: (next: CookieBotUserTemplate[]) => void;
  onDeselect: () => void;
}) {
  const { t } = useTranslation();
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameText, setRenameText] = useState("");
  const [confirmingId, setConfirmingId] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  // Every write merges into the NEWEST list rather than into the one its
  // handler closed over. A rename and a delete on two different rows overlap
  // whenever the first request is slow, and merging into a stale array would
  // resurrect the deleted row on screen while the server has already dropped it.
  const latest = useRef(templates);
  useEffect(() => {
    latest.current = templates;
  }, [templates]);

  const rename = useCallback(
    async (id: string) => {
      const name = renameText.trim();
      if (name.length === 0) return;
      setBusyId(id);
      try {
        const updated = await updateCookieBotUserTemplate(id, { name });
        onChanged(
          (latest.current ?? []).map((item) =>
            item.id === id ? updated : item,
          ),
        );
        showSuccessToast(t("cookieBot.enrol.listRenamed"));
        setRenamingId(null);
      } catch (error) {
        showErrorToast(templateErrorMessage(t, error));
      } finally {
        setBusyId(null);
      }
    },
    [renameText, onChanged, t],
  );

  const remove = useCallback(
    async (id: string) => {
      setBusyId(id);
      try {
        await deleteCookieBotUserTemplate(id);
        onChanged((latest.current ?? []).filter((item) => item.id !== id));
        // The enrolment being edited must not be left pointing at a list that
        // no longer exists — the save would 404 with nothing on screen to say
        // which field was wrong.
        if (id === selectedId) onDeselect();
        showSuccessToast(t("cookieBot.enrol.listDeleted"));
        setConfirmingId(null);
      } catch (error) {
        showErrorToast(templateErrorMessage(t, error));
      } finally {
        setBusyId(null);
      }
    },
    [onChanged, onDeselect, selectedId, t],
  );

  if (failed) {
    return (
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-[11px] text-muted-foreground">
          {t("cookieBot.enrol.savedUnavailable")}
        </p>
        <Button
          variant="outline"
          size="sm"
          className="h-6 shrink-0 text-[11px]"
          disabled={isLoading}
          onClick={onRetry}
        >
          {t("common.buttons.retry")}
        </Button>
      </div>
    );
  }

  if (templates === null) {
    return (
      <p className="text-[11px] text-muted-foreground">
        {t("common.buttons.loading")}
      </p>
    );
  }

  if (templates.length === 0) {
    return (
      <p className="text-[11px] leading-snug text-muted-foreground">
        {t("cookieBot.enrol.savedEmpty")}
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      {templates.map((template) => {
        const selected = template.id === selectedId;
        const busy = busyId === template.id;
        if (renamingId === template.id) {
          return (
            <div key={template.id} className="flex items-center gap-1.5">
              <Input
                autoFocus
                value={renameText}
                maxLength={maxNameLength}
                onChange={(event) => {
                  setRenameText(event.target.value);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void rename(template.id);
                  }
                  if (event.key === "Escape") setRenamingId(null);
                }}
                className="h-7 flex-1 text-xs"
              />
              <Button
                size="sm"
                className="h-7 shrink-0 text-[11px]"
                disabled={busy || renameText.trim().length === 0}
                onClick={() => {
                  void rename(template.id);
                }}
              >
                {t("common.buttons.save")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-7 shrink-0 text-[11px] text-muted-foreground"
                onClick={() => {
                  setRenamingId(null);
                }}
              >
                {t("common.buttons.cancel")}
              </Button>
            </div>
          );
        }

        return (
          <div
            key={template.id}
            className={cn(
              "flex items-center gap-1 rounded-md border pl-3 pr-1 transition-colors duration-100",
              selected ? "border-foreground/40 bg-accent" : "border-border",
            )}
          >
            <button
              type="button"
              aria-pressed={selected}
              onClick={() => {
                onSelect(template.id);
              }}
              className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 py-2 text-left"
            >
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground">
                {template.name}
              </span>
              <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
                {t("cookieBot.enrol.listSites", {
                  count: template.sites.length,
                })}
              </span>
            </button>
            {confirmingId === template.id ? (
              <>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 shrink-0 px-1.5 text-[11px] text-destructive-text"
                  disabled={busy}
                  onClick={() => {
                    void remove(template.id);
                  }}
                >
                  {t("cookieBot.enrol.listDeleteConfirm")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 shrink-0 px-1.5 text-[11px] text-muted-foreground"
                  onClick={() => {
                    setConfirmingId(null);
                  }}
                >
                  {t("common.buttons.cancel")}
                </Button>
              </>
            ) : (
              <>
                <button
                  type="button"
                  aria-label={t("cookieBot.enrol.listRename")}
                  title={t("cookieBot.enrol.listRename")}
                  onClick={() => {
                    setRenameText(template.name);
                    setRenamingId(template.id);
                  }}
                  className="inline-flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-sm text-muted-foreground transition-colors duration-100 hover:bg-accent hover:text-foreground"
                >
                  <LuPencil className="size-3" />
                </button>
                <button
                  type="button"
                  aria-label={t("common.buttons.delete")}
                  title={t("common.buttons.delete")}
                  onClick={() => {
                    setConfirmingId(template.id);
                  }}
                  className="inline-flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-sm text-muted-foreground transition-colors duration-100 hover:bg-accent hover:text-destructive-text"
                >
                  <LuTrash2 className="size-3" />
                </button>
              </>
            )}
          </div>
        );
      })}
      {missing && (
        <p className="text-[11px] text-warning-text">
          {t("cookieBot.enrol.templateMissing")}
        </p>
      )}
      <p className="text-[11px] leading-snug text-muted-foreground">
        {t("cookieBot.enrol.savedNote")}
      </p>
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
    needsTemplate: boolean;
    needsSlot: boolean;
    /** Two rows claim the same days and time; the server would store one. */
    duplicateSlot: boolean;
    /**
     * Only reachable by editing an enrolment written while the server allowed
     * more start times than it does now. Rare, and still owed a reason: a mute
     * button on a form the user did not fill in is the worst version of this.
     */
    overSlotCap: boolean;
    maxSlots: number;
    needsPreset: boolean;
  },
): string {
  if (state.saving) return t("cookieBot.enrol.saving");
  if (state.eligible === 0 && !state.isEdit)
    return t("cookieBot.enrol.fixFirst");
  if (state.needsSites) return t("cookieBot.enrol.addSitesFirst");
  if (state.needsTemplate) return t("cookieBot.enrol.pickListFirst");
  if (state.needsSlot) return t("cookieBot.enrol.finishCalendarFirst");
  if (state.duplicateSlot) return t("cookieBot.enrol.duplicateSlot");
  if (state.overSlotCap)
    return t("cookieBot.enrol.slotsFull", { max: state.maxSlots });
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

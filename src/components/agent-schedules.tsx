"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { CopyToClipboard } from "@/components/ui/copy-to-clipboard";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ProBadge } from "@/components/ui/pro-badge";
import { RippleButton } from "@/components/ui/ripple";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { translateBackendError } from "@/lib/backend-errors";
import { type AgentRecipe, getAgentRecipes } from "@/lib/recipes";
import {
  type AgentSchedule,
  type AgentScheduleRun,
  createSchedule,
  deleteSchedule,
  isoToLocalInput,
  isWebhookUrl,
  listScheduleRuns,
  listSchedules,
  localTimezone,
  MAX_INTERVAL_MINUTES,
  MAX_SCHEDULE_NAME_CHARS,
  MAX_SCHEDULE_PROFILES,
  MIN_INTERVAL_MINUTES,
  minuteToTime,
  rotateWebhookSecret,
  runScheduleNow,
  type ScheduleCadence,
  type ScheduleTarget,
  scheduleBody,
  testScheduleNotice,
  timeToMinute,
  updateSchedule,
  WEEKDAYS,
} from "@/lib/schedules";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile } from "@/types";

const RUN_STATUSES = [
  "pending",
  "running",
  "succeeded",
  "partial",
  "failed",
  "cancelled",
] as const;

/** Short weekday names in the app's language, Monday first. */
function useWeekdayNames(): string[] {
  const { i18n } = useTranslation();
  return useMemo(() => {
    const format = new Intl.DateTimeFormat(i18n.language, { weekday: "short" });
    // 2024-01-01 was a Monday.
    return WEEKDAYS.map((day) =>
      format.format(new Date(Date.UTC(2024, 0, 1 + day, 12))),
    );
  }, [i18n.language]);
}

function formatWhen(iso: string | null, language: string): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return new Intl.DateTimeFormat(language, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function AgentSchedules({
  profiles,
  canRunOnDesktop,
}: {
  profiles: BrowserProfile[];
  /** A schedule on this computer needs browser automation (Pro and up). */
  canRunOnDesktop: boolean;
}) {
  const { t, i18n } = useTranslation();
  const weekdays = useWeekdayNames();
  const [schedules, setSchedules] = useState<AgentSchedule[]>([]);
  const [recipes, setRecipes] = useState<AgentRecipe[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<unknown>(null);
  const [editing, setEditing] = useState<AgentSchedule | "new" | null>(null);
  const [pendingRemoval, setPendingRemoval] = useState<AgentSchedule | null>(
    null,
  );
  const [isRemoving, setIsRemoving] = useState(false);
  const [runsFor, setRunsFor] = useState<string | null>(null);
  const [runs, setRuns] = useState<AgentScheduleRun[]>([]);

  const load = useCallback(async () => {
    setIsLoading(true);
    try {
      const [nextSchedules, nextRecipes] = await Promise.all([
        listSchedules(),
        getAgentRecipes(),
      ]);
      setSchedules(nextSchedules);
      setRecipes(nextRecipes);
      setError(null);
    } catch (loadError) {
      setError(loadError);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const recipeName = (id: string | null) =>
    recipes.find((recipe) => recipe.id === id)?.name ?? "—";

  const summary = (schedule: AgentSchedule) => {
    if (schedule.cadence === "once") {
      return t("agent.schedules.summaryOnce", {
        when: formatWhen(schedule.runOnceAt, i18n.language),
      });
    }
    if (schedule.cadence === "interval") {
      return t("agent.schedules.summaryInterval", {
        minutes: schedule.intervalMinutes ?? 0,
      });
    }
    const days = WEEKDAYS.filter((day) => schedule.daysMask & (1 << day))
      .map((day) => weekdays[day])
      .join(", ");
    return t("agent.schedules.summaryWeekly", {
      days,
      time: minuteToTime(schedule.runAtMinute),
      timezone: schedule.timezone,
    });
  };

  const toggleEnabled = async (schedule: AgentSchedule, enabled: boolean) => {
    try {
      const updated = await updateSchedule(schedule.id, { enabled });
      setSchedules((current) =>
        current.map((item) => (item.id === schedule.id ? updated : item)),
      );
    } catch (toggleError) {
      showErrorToast(translateBackendError(t, toggleError));
    }
  };

  const runNow = async (schedule: AgentSchedule) => {
    try {
      await runScheduleNow(schedule.id);
      showSuccessToast(t("agent.schedules.runStarted"));
    } catch (runError) {
      showErrorToast(translateBackendError(t, runError));
    }
  };

  const showRuns = async (schedule: AgentSchedule) => {
    if (runsFor === schedule.id) {
      setRunsFor(null);
      return;
    }
    try {
      setRuns(await listScheduleRuns(schedule.id));
      setRunsFor(schedule.id);
    } catch (runsError) {
      showErrorToast(translateBackendError(t, runsError));
    }
  };

  const confirmRemoval = async () => {
    if (!pendingRemoval) return;
    setIsRemoving(true);
    try {
      await deleteSchedule(pendingRemoval.id);
      showSuccessToast(t("agent.schedules.deleted"));
      setPendingRemoval(null);
      void load();
    } catch (removeError) {
      showErrorToast(translateBackendError(t, removeError));
    } finally {
      setIsRemoving(false);
    }
  };

  return (
    <div
      data-slot="agent-schedules"
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      <div className="flex shrink-0 items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">
          {t("agent.schedules.hint")}
        </p>
        <RippleButton
          size="sm"
          data-slot="agent-schedule-new"
          className="flex h-8 items-center gap-1.5 rounded-lg text-xs"
          disabled={editing !== null || recipes.length === 0}
          onClick={() => setEditing("new")}
        >
          <GoPlus className="size-3.5" />
          {t("agent.schedules.new")}
        </RippleButton>
      </div>

      {error !== null && (
        <p className="shrink-0 rounded-xl bg-destructive/10 p-3 text-sm text-destructive-text">
          {translateBackendError(t, error)}
        </p>
      )}
      {!isLoading && error === null && recipes.length === 0 && (
        <p className="shrink-0 rounded-xl bg-foreground/3 p-3 text-xs text-muted-foreground">
          {t("agent.schedules.needsRecipe")}
        </p>
      )}

      {editing !== null && (
        <ScheduleEditor
          schedule={editing === "new" ? null : editing}
          recipes={recipes}
          profiles={profiles}
          canRunOnDesktop={canRunOnDesktop}
          weekdays={weekdays}
          onCancel={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void load();
          }}
        />
      )}

      <FadingScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-1 pr-1">
          {isLoading && schedules.length === 0 ? (
            Array.from({ length: 3 }, (_, index) => (
              <Skeleton
                key={`agent-schedule-skeleton-${index}`}
                className="h-14 w-full"
              />
            ))
          ) : schedules.length === 0 ? (
            <p
              data-slot="agent-schedules-empty"
              className="py-16 text-center text-sm text-muted-foreground"
            >
              {t("agent.schedules.empty")}
            </p>
          ) : (
            schedules.map((schedule) => (
              <div
                key={schedule.id}
                data-slot="agent-schedule"
                data-schedule-id={schedule.id}
                className="flex flex-col gap-2 rounded-xl bg-foreground/3 p-3 transition-colors duration-150 hover:bg-foreground/5"
              >
                <div className="flex items-center gap-3">
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span className="truncate text-sm font-medium text-foreground">
                      {schedule.name}
                    </span>
                    <span className="truncate text-xs text-muted-foreground">
                      {[
                        recipeName(schedule.recipeId),
                        summary(schedule),
                        t(
                          schedule.target === "fleet"
                            ? "agent.schedules.targetFleet"
                            : "agent.schedules.targetDesktop",
                        ),
                      ].join(" · ")}
                    </span>
                    <span className="truncate text-xs text-muted-foreground">
                      {schedule.enabled
                        ? t("agent.schedules.nextRun", {
                            when: formatWhen(schedule.nextRunAt, i18n.language),
                          })
                        : t("agent.schedules.paused")}
                    </span>
                  </div>
                  <AnimatedSwitch
                    aria-label={t("agent.schedules.enabled", {
                      name: schedule.name,
                    })}
                    checked={schedule.enabled}
                    onCheckedChange={(checked) =>
                      void toggleEnabled(schedule, checked === true)
                    }
                  />
                  <Button
                    variant="soft"
                    size="sm"
                    data-slot="agent-schedule-run-now"
                    className="h-8 rounded-lg text-xs"
                    onClick={() => void runNow(schedule)}
                  >
                    {t("agent.schedules.runNow")}
                  </Button>
                  <Button
                    variant="subtle"
                    size="sm"
                    className="h-8 rounded-lg text-xs"
                    aria-expanded={runsFor === schedule.id}
                    onClick={() => void showRuns(schedule)}
                  >
                    {t("agent.schedules.runs")}
                  </Button>
                  <Button
                    variant="subtle"
                    size="sm"
                    className="h-8 rounded-lg text-xs"
                    onClick={() => setEditing(schedule)}
                  >
                    {t("common.buttons.edit")}
                  </Button>
                  <Button
                    variant="subtle"
                    size="sm"
                    className="h-8 rounded-lg text-xs hover:bg-destructive/10 hover:text-destructive-text"
                    onClick={() => setPendingRemoval(schedule)}
                  >
                    {t("common.buttons.delete")}
                  </Button>
                </div>
                {runsFor === schedule.id && (
                  <ul
                    data-slot="agent-schedule-runs"
                    className="flex flex-col gap-1 border-t border-border pt-2 text-xs"
                  >
                    {runs.length === 0 ? (
                      <li className="text-muted-foreground">
                        {t("agent.schedules.noRuns")}
                      </li>
                    ) : (
                      runs.slice(0, 10).map((run) => (
                        <li
                          key={run.id}
                          className="flex items-center justify-between gap-2"
                        >
                          <span className="text-muted-foreground">
                            {formatWhen(run.scheduledFor, i18n.language)}
                          </span>
                          <span>
                            {(RUN_STATUSES as readonly string[]).includes(
                              run.status,
                            )
                              ? t(`agent.schedules.status.${run.status}`)
                              : run.status}
                          </span>
                        </li>
                      ))
                    )}
                  </ul>
                )}
              </div>
            ))
          )}
        </div>
      </FadingScrollArea>

      <DeleteConfirmationDialog
        isOpen={pendingRemoval !== null}
        onClose={() => setPendingRemoval(null)}
        onConfirm={() => void confirmRemoval()}
        title={t("agent.schedules.deleteTitle", {
          name: pendingRemoval?.name ?? "",
        })}
        description={t("agent.schedules.deleteDescription")}
        confirmButtonText={t("common.buttons.delete")}
        isLoading={isRemoving}
      />
    </div>
  );
}

function ScheduleEditor({
  schedule,
  recipes,
  profiles,
  canRunOnDesktop,
  weekdays,
  onCancel,
  onSaved,
}: {
  schedule: AgentSchedule | null;
  recipes: AgentRecipe[];
  profiles: BrowserProfile[];
  canRunOnDesktop: boolean;
  weekdays: string[];
  onCancel: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(schedule?.name ?? "");
  const [recipeId, setRecipeId] = useState(
    schedule?.recipeId ?? recipes[0]?.id ?? "",
  );
  const [profileIds, setProfileIds] = useState<string[]>(
    schedule?.profileIds ?? [],
  );
  const [target, setTarget] = useState<ScheduleTarget>(
    schedule?.target ?? "fleet",
  );
  const [cadence, setCadence] = useState<ScheduleCadence>(
    schedule?.cadence ?? "weekly",
  );
  // Monday to Friday until the person picks.
  const [daysMask, setDaysMask] = useState(schedule?.daysMask ?? 0b0011111);
  const [time, setTime] = useState(
    minuteToTime(schedule?.runAtMinute ?? 9 * 60),
  );
  const timezone = schedule?.timezone ?? localTimezone();
  const [runOnceAt, setRunOnceAt] = useState(
    isoToLocalInput(schedule?.runOnceAt),
  );
  const [intervalMinutes, setIntervalMinutes] = useState(
    String(schedule?.intervalMinutes ?? 60),
  );
  const [isSaving, setIsSaving] = useState(false);
  const [notifyEmail, setNotifyEmail] = useState(schedule?.notifyEmail ?? true);
  const [webhookUrl, setWebhookUrl] = useState(schedule?.webhookUrl ?? "");
  const [webhookSecret, setWebhookSecret] = useState(
    schedule?.webhookSecret ?? null,
  );
  const [isTesting, setIsTesting] = useState(false);
  const webhookValid = isWebhookUrl(webhookUrl);

  const minute = timeToMinute(time);
  const interval = Number.parseInt(intervalMinutes, 10);
  const onceDate = runOnceAt ? new Date(runOnceAt) : null;
  const cadenceReady =
    cadence === "weekly"
      ? daysMask > 0 && minute !== null
      : cadence === "once"
        ? onceDate !== null &&
          !Number.isNaN(onceDate.getTime()) &&
          onceDate.getTime() > Date.now()
        : Number.isInteger(interval) &&
          interval >= MIN_INTERVAL_MINUTES &&
          interval <= MAX_INTERVAL_MINUTES;
  const canSave =
    name.trim().length > 0 &&
    recipeId !== "" &&
    profileIds.length > 0 &&
    (target === "fleet" || canRunOnDesktop) &&
    cadenceReady &&
    webhookValid &&
    !isSaving;

  const sendTestNotice = async () => {
    if (!schedule) return;
    setIsTesting(true);
    try {
      const result = await testScheduleNotice(schedule.id);
      const failed = result.email === "failed" || result.webhook === "failed";
      if (failed) {
        showErrorToast(
          t("agent.schedules.notices.testFailed", {
            status: result.webhookStatus ?? "-",
          }),
        );
      } else {
        showSuccessToast(t("agent.schedules.notices.testSent"));
      }
    } catch (testError) {
      showErrorToast(translateBackendError(t, testError));
    } finally {
      setIsTesting(false);
    }
  };

  const newSecret = async () => {
    if (!schedule) return;
    try {
      const updated = await rotateWebhookSecret(schedule.id);
      setWebhookSecret(updated.webhookSecret);
      showSuccessToast(t("agent.schedules.notices.secretRotated"));
    } catch (rotateError) {
      showErrorToast(translateBackendError(t, rotateError));
    }
  };

  const toggleProfile = (id: string, on: boolean) => {
    setProfileIds((current) =>
      on
        ? [...current, id].slice(0, MAX_SCHEDULE_PROFILES)
        : current.filter((item) => item !== id),
    );
  };

  const save = async () => {
    if (!canSave) return;
    setIsSaving(true);
    const input = {
      name,
      recipeId,
      target,
      profileIds,
      cadence,
      daysMask,
      runAtMinute: minute ?? 0,
      timezone,
      runOnceAt: onceDate?.toISOString(),
      intervalMinutes: interval,
      notifyEmail,
      webhookUrl: webhookUrl.trim() || null,
    };
    try {
      if (schedule) {
        await updateSchedule(schedule.id, scheduleBody(input));
        showSuccessToast(t("agent.schedules.updated"));
      } else {
        await createSchedule({ ...input, enabled: true });
        showSuccessToast(t("agent.schedules.created"));
      }
      onSaved();
    } catch (saveError) {
      showErrorToast(translateBackendError(t, saveError));
    } finally {
      setIsSaving(false);
    }
  };

  const labelClass = "text-xs font-medium text-muted-foreground";
  return (
    <form
      data-slot="agent-schedule-editor"
      className="flex shrink-0 flex-col gap-3 rounded-xl bg-foreground/3 p-3"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col gap-2">
          <Label htmlFor="agent-schedule-name" className={labelClass}>
            {t("agent.schedules.name")}
          </Label>
          <Input
            id="agent-schedule-name"
            value={name}
            maxLength={MAX_SCHEDULE_NAME_CHARS}
            placeholder={t("agent.schedules.namePlaceholder")}
            onChange={(event) => setName(event.target.value)}
          />
        </div>
        <div className="flex flex-col gap-2">
          <Label className={labelClass}>{t("agent.schedules.recipe")}</Label>
          <Select value={recipeId} onValueChange={setRecipeId}>
            <SelectTrigger data-slot="agent-schedule-recipe">
              <SelectValue placeholder={t("agent.schedules.recipe")} />
            </SelectTrigger>
            <SelectContent>
              {recipes.map((recipe) => (
                <SelectItem key={recipe.id} value={recipe.id}>
                  {recipe.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <Label className={labelClass}>
          {t("agent.schedules.profiles", { n: profileIds.length })}
        </Label>
        <div className="flex max-h-36 flex-col gap-0.5 overflow-y-auto rounded-lg bg-foreground/3 p-1.5">
          {profiles.map((profile) => {
            const id = `agent-schedule-profile-${profile.id}`;
            return (
              <div
                key={profile.id}
                className="flex items-center gap-2 px-1 py-0.5"
              >
                <Checkbox
                  id={id}
                  data-slot="agent-schedule-profile"
                  checked={profileIds.includes(profile.id)}
                  onCheckedChange={(checked) =>
                    toggleProfile(profile.id, checked === true)
                  }
                />
                <label htmlFor={id} className="min-w-0 truncate text-sm">
                  {profile.name}
                </label>
              </div>
            );
          })}
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col gap-2">
          <Label className={labelClass}>{t("agent.schedules.target")}</Label>
          <Select
            value={target}
            onValueChange={(value) => setTarget(value as ScheduleTarget)}
          >
            <SelectTrigger data-slot="agent-schedule-target">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="fleet">
                {t("agent.schedules.targetFleet")}
              </SelectItem>
              <SelectItem value="desktop" disabled={!canRunOnDesktop}>
                <span className="flex items-center gap-2">
                  {t("agent.schedules.targetDesktop")}
                  {!canRunOnDesktop && <ProBadge />}
                </span>
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="flex flex-col gap-2">
          <Label className={labelClass}>{t("agent.schedules.cadence")}</Label>
          <Select
            value={cadence}
            onValueChange={(value) => setCadence(value as ScheduleCadence)}
          >
            <SelectTrigger data-slot="agent-schedule-cadence">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="weekly">
                {t("agent.schedules.cadenceWeekly")}
              </SelectItem>
              <SelectItem value="once">
                {t("agent.schedules.cadenceOnce")}
              </SelectItem>
              <SelectItem value="interval">
                {t("agent.schedules.cadenceInterval")}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      {cadence === "weekly" && (
        <div className="flex flex-wrap items-end gap-3">
          <fieldset className="flex flex-col gap-2">
            <legend className={cn(labelClass, "mb-2")}>
              {t("agent.schedules.days")}
            </legend>
            <div className="flex gap-1">
              {WEEKDAYS.map((day) => {
                const on = (daysMask & (1 << day)) !== 0;
                return (
                  <Button
                    key={day}
                    type="button"
                    size="sm"
                    variant={on ? "soft" : "subtle"}
                    aria-pressed={on}
                    data-slot="agent-schedule-day"
                    className={cn(
                      "h-8 min-w-10 rounded-lg px-2 text-xs",
                      !on && "text-muted-foreground",
                    )}
                    onClick={() => setDaysMask(daysMask ^ (1 << day))}
                  >
                    {weekdays[day]}
                  </Button>
                );
              })}
            </div>
          </fieldset>
          <div className="flex flex-col gap-2">
            <Label htmlFor="agent-schedule-time" className={labelClass}>
              {t("agent.schedules.time")}
            </Label>
            <Input
              id="agent-schedule-time"
              type="time"
              value={time}
              className="w-32"
              onChange={(event) => setTime(event.target.value)}
            />
          </div>
          <p className="pb-2 text-xs text-muted-foreground">
            {t("agent.schedules.timezone", { timezone })}
          </p>
        </div>
      )}
      {cadence === "once" && (
        <div className="flex flex-col gap-2">
          <Label htmlFor="agent-schedule-once" className={labelClass}>
            {t("agent.schedules.runOnceAt")}
          </Label>
          <Input
            id="agent-schedule-once"
            type="datetime-local"
            value={runOnceAt}
            className="w-64"
            onChange={(event) => setRunOnceAt(event.target.value)}
          />
        </div>
      )}
      {cadence === "interval" && (
        <div className="flex flex-col gap-2">
          <Label htmlFor="agent-schedule-interval" className={labelClass}>
            {t("agent.schedules.intervalMinutes", {
              min: MIN_INTERVAL_MINUTES,
              max: MAX_INTERVAL_MINUTES,
            })}
          </Label>
          <Input
            id="agent-schedule-interval"
            type="number"
            min={MIN_INTERVAL_MINUTES}
            max={MAX_INTERVAL_MINUTES}
            value={intervalMinutes}
            className="w-32"
            onChange={(event) => setIntervalMinutes(event.target.value)}
          />
        </div>
      )}

      <div className="flex flex-col gap-2" data-slot="agent-schedule-notices">
        <Label className={labelClass}>
          {t("agent.schedules.notices.title")}
        </Label>
        <label
          htmlFor="agent-schedule-notify-email"
          className="flex cursor-pointer items-center gap-2 text-sm"
        >
          <Checkbox
            id="agent-schedule-notify-email"
            checked={notifyEmail}
            onCheckedChange={(checked) => setNotifyEmail(checked === true)}
          />
          {t("agent.schedules.notices.email")}
        </label>
        <Input
          id="agent-schedule-webhook"
          type="url"
          inputMode="url"
          value={webhookUrl}
          placeholder={t("agent.schedules.notices.webhookPlaceholder")}
          aria-label={t("agent.schedules.notices.webhook")}
          aria-invalid={!webhookValid}
          onChange={(event) => setWebhookUrl(event.target.value)}
        />
        {!webhookValid && (
          <p className="text-xs text-destructive-text">
            {t("agent.schedules.notices.webhookInvalid")}
          </p>
        )}
        {schedule &&
          webhookSecret &&
          webhookUrl.trim() === schedule.webhookUrl && (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span className="min-w-0 truncate">
                {t("agent.schedules.notices.secret")}{" "}
                <code className="font-mono">{webhookSecret}</code>
              </span>
              <CopyToClipboard
                variant="subtle"
                className="size-7 shrink-0 rounded-lg"
                text={webhookSecret}
                aria-label={t("agent.schedules.notices.copySecret")}
              />
              <Button
                variant="subtle"
                size="sm"
                type="button"
                className="h-7 rounded-lg text-xs"
                onClick={() => void newSecret()}
              >
                {t("agent.schedules.notices.rotate")}
              </Button>
            </div>
          )}
        {schedule && (
          <Button
            variant="subtle"
            size="sm"
            type="button"
            className="h-8 self-start rounded-lg text-xs"
            disabled={isTesting}
            onClick={() => void sendTestNotice()}
            data-slot="agent-schedule-test-notice"
          >
            {t("agent.schedules.notices.test")}
          </Button>
        )}
      </div>

      <div className="flex items-center justify-end gap-2">
        <Button
          variant="subtle"
          size="sm"
          className="h-8 rounded-lg text-xs"
          onClick={onCancel}
          type="button"
        >
          {t("common.buttons.cancel")}
        </Button>
        <RippleButton
          type="submit"
          size="sm"
          data-slot="agent-schedule-save"
          className="h-8 rounded-lg text-xs"
          disabled={!canSave}
        >
          {isSaving ? t("common.buttons.saving") : t("common.buttons.save")}
        </RippleButton>
      </div>
    </form>
  );
}

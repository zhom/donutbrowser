"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuChevronRight } from "react-icons/lu";
import {
  AnimatedDisclosureChevron,
  AnimatedDisclosureContent,
} from "@/components/ui/animated-disclosure";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import {
  AGENT_EFFORTS,
  AGENT_GOAL_MAX_CHARS,
  AGENT_PLATFORMS,
  type AgentBudgets,
  type AgentEffort,
  type AgentPlatform,
  type AgentRecipe,
  type AgentRunView,
  type AgentTarget,
  agentGoalProblem,
  parseAllowedHosts,
  startAgentRun,
} from "@/lib/agent";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";

const TARGETS: readonly AgentTarget[] = ["desktop", "fleet"];

/** Minutes, because nobody thinks about a wall-clock budget in milliseconds. */
const MS_PER_MINUTE = 60_000;

/**
 * Read a positive whole number out of a text field, or null.
 *
 * Null is "the user did not set a ceiling", which is not the same as zero: an
 * unset budget lets the server apply its own default, and a zero would ask for
 * a run that stops before it starts.
 */
function positiveInteger(raw: string): number | null {
  const trimmed = raw.trim();
  if (trimmed.length === 0) return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value) || !Number.isInteger(value) || value <= 0) {
    return null;
  }
  return value;
}

/** True when the field has something in it that is not a usable ceiling. */
function isBadBudget(raw: string): boolean {
  return raw.trim().length > 0 && positiveInteger(raw) === null;
}

interface AgentRunFormProps {
  profiles: BrowserProfile[];
  recipes: AgentRecipe[];
  /** Pre-selected profile, when the page was opened from one. */
  defaultProfileId?: string | null;
  onStarted: (run: AgentRunView) => void;
}

export function AgentRunForm({
  profiles,
  recipes,
  defaultProfileId,
  onStarted,
}: AgentRunFormProps) {
  const { t } = useTranslation();

  const sortedProfiles = useMemo(
    () => [...profiles].sort((a, b) => a.name.localeCompare(b.name)),
    [profiles],
  );

  const [profileId, setProfileId] = useState<string | null>(
    defaultProfileId ?? sortedProfiles[0]?.id ?? null,
  );
  const [target, setTarget] = useState<AgentTarget>("desktop");
  const [platform, setPlatform] = useState<AgentPlatform | null>(null);
  const [goal, setGoal] = useState("");
  const [effort, setEffort] = useState<AgentEffort>("standard");
  const [maxSteps, setMaxSteps] = useState("");
  const [maxMinutes, setMaxMinutes] = useState("");
  const [maxTokens, setMaxTokens] = useState("");
  const [allowedHostsRaw, setAllowedHostsRaw] = useState("");
  const [limitsOpen, setLimitsOpen] = useState(false);
  const [isStarting, setIsStarting] = useState(false);

  const selectedProfile = useMemo(
    () => sortedProfiles.find((profile) => profile.id === profileId) ?? null,
    [sortedProfiles, profileId],
  );

  // A profile deleted from another surface must not leave the form pointing at
  // an id the backend would refuse.
  useEffect(() => {
    if (profileId && sortedProfiles.some((p) => p.id === profileId)) return;
    setProfileId(sortedProfiles[0]?.id ?? null);
  }, [sortedProfiles, profileId]);

  // A leased host has to be the machine the profile was built for, so the
  // platform follows the profile unless the user says otherwise.
  const resolvedPlatform: AgentPlatform | null = useMemo(() => {
    if (platform) return platform;
    const own = selectedProfile?.host_os;
    return own && (AGENT_PLATFORMS as readonly string[]).includes(own)
      ? (own as AgentPlatform)
      : null;
  }, [platform, selectedProfile]);

  const goalProblem = agentGoalProblem(goal);
  const goalLength = [...goal.trim()].length;
  const allowedHosts = useMemo(
    () => parseAllowedHosts(allowedHostsRaw),
    [allowedHostsRaw],
  );
  const budgetProblem =
    isBadBudget(maxSteps) || isBadBudget(maxMinutes) || isBadBudget(maxTokens);
  const needsPlatform = target === "fleet" && resolvedPlatform === null;

  const canSubmit =
    profileId !== null &&
    goalProblem === null &&
    !budgetProblem &&
    !needsPlatform &&
    !isStarting;

  const handleSubmit = useCallback(async () => {
    if (!profileId || goalProblem !== null || budgetProblem || needsPlatform) {
      return;
    }
    const budgets: AgentBudgets = {
      maxSteps: positiveInteger(maxSteps),
      maxWallMs: (() => {
        const minutes = positiveInteger(maxMinutes);
        return minutes === null ? null : minutes * MS_PER_MINUTE;
      })(),
      maxTokens: positiveInteger(maxTokens),
    };
    const hasBudget =
      budgets.maxSteps !== null ||
      budgets.maxWallMs !== null ||
      budgets.maxTokens !== null;

    setIsStarting(true);
    try {
      const run = await startAgentRun({
        profileId,
        target,
        goal: goal.trim(),
        platform: target === "fleet" ? resolvedPlatform : null,
        effort,
        budgets: hasBudget ? budgets : null,
        allowedHosts: allowedHosts.length > 0 ? allowedHosts : null,
      });
      showSuccessToast(t("agent.form.started"));
      setGoal("");
      onStarted(run);
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsStarting(false);
    }
  }, [
    profileId,
    goalProblem,
    budgetProblem,
    needsPlatform,
    maxSteps,
    maxMinutes,
    maxTokens,
    target,
    goal,
    resolvedPlatform,
    effort,
    allowedHosts,
    onStarted,
    t,
  ]);

  return (
    <form
      data-slot="agent-run-form"
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        void handleSubmit();
      }}
    >
      <div className="grid gap-4 @2xl:grid-cols-2">
        <div className="flex flex-col gap-2">
          <Label htmlFor="agent-profile">{t("agent.form.profile")}</Label>
          <Select
            value={profileId ?? undefined}
            onValueChange={setProfileId}
            disabled={sortedProfiles.length === 0}
          >
            <SelectTrigger id="agent-profile" className="w-full">
              <SelectValue placeholder={t("agent.form.profilePlaceholder")} />
            </SelectTrigger>
            <SelectContent>
              {sortedProfiles.map((profile) => (
                <SelectItem key={profile.id} value={profile.id}>
                  {profile.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {sortedProfiles.length === 0 && (
            <p className="text-xs text-muted-foreground">
              {t("agent.form.noProfiles")}
            </p>
          )}
        </div>

        <div className="flex flex-col gap-2">
          <Label htmlFor="agent-target">{t("agent.form.target")}</Label>
          <Select
            value={target}
            onValueChange={(value) => {
              setTarget(value as AgentTarget);
            }}
          >
            <SelectTrigger id="agent-target" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TARGETS.map((option) => (
                <SelectItem key={option} value={option}>
                  {t(`agent.target.${option}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            {t(
              target === "fleet"
                ? "agent.form.targetFleetHint"
                : "agent.form.targetDesktopHint",
            )}
          </p>
        </div>
      </div>

      {target === "fleet" && (
        <div className="flex flex-col gap-2">
          <Label htmlFor="agent-platform">{t("agent.form.platform")}</Label>
          <Select
            value={resolvedPlatform ?? undefined}
            onValueChange={(value) => {
              setPlatform(value as AgentPlatform);
            }}
          >
            <SelectTrigger id="agent-platform" className="w-full @2xl:w-1/2">
              <SelectValue placeholder={t("agent.form.platformPlaceholder")} />
            </SelectTrigger>
            <SelectContent>
              {AGENT_PLATFORMS.map((option) => (
                <SelectItem key={option} value={option}>
                  {t(`agent.platform.${option}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {needsPlatform && (
            <p className="text-xs text-destructive-text">
              {t("agent.form.platformRequired")}
            </p>
          )}
        </div>
      )}

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-2">
          <Label htmlFor="agent-goal">{t("agent.form.goal")}</Label>
          {/* A menu, not a select: inserting a recipe is an action, and the
              same recipe has to be insertable twice in a row — which a control
              that remembers the value it last held cannot do. */}
          {recipes.length > 0 && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="sm" className="h-7 text-xs">
                  {t("agent.form.recipeInsert")}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {recipes.map((recipe) => (
                  <DropdownMenuItem
                    key={recipe.id}
                    onSelect={() => {
                      // Appended rather than replacing: a saved goal is a
                      // starting point the user then edits, and silently
                      // discarding what they already typed is the one thing an
                      // insert must never do.
                      setGoal((current) => {
                        const addition = recipe.steps.join("\n");
                        return current.trim().length === 0
                          ? addition
                          : `${current.trimEnd()}\n${addition}`;
                      });
                    }}
                  >
                    {recipe.name}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>
        <Textarea
          id="agent-goal"
          value={goal}
          rows={5}
          placeholder={t("agent.form.goalPlaceholder")}
          onChange={(event) => {
            setGoal(event.target.value);
          }}
        />
        <div className="flex items-center justify-between gap-2 text-xs">
          {goalProblem === "tooLong" ? (
            <span className="text-destructive-text">
              {t("agent.form.goalTooLong")}
            </span>
          ) : (
            <span />
          )}
          <span className="shrink-0 tabular-nums text-muted-foreground">
            {t("agent.form.goalCount", {
              chars: goalLength,
              max: AGENT_GOAL_MAX_CHARS,
            })}
          </span>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <button
          type="button"
          aria-expanded={limitsOpen}
          data-slot="agent-limits-toggle"
          onClick={() => {
            setLimitsOpen((open) => !open);
          }}
          className="flex w-fit cursor-pointer items-center gap-2 rounded-sm text-left text-sm text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
        >
          <AnimatedDisclosureChevron open={limitsOpen}>
            <LuChevronRight className="size-3.5" />
          </AnimatedDisclosureChevron>
          {t("agent.form.limits")}
        </button>
        <AnimatedDisclosureContent
          open={limitsOpen}
          className="flex flex-col gap-4 pt-1"
        >
          <div className="grid gap-4 @2xl:grid-cols-3">
            <BudgetField
              id="agent-max-steps"
              label={t("agent.form.maxSteps")}
              value={maxSteps}
              onChange={setMaxSteps}
            />
            <BudgetField
              id="agent-max-minutes"
              label={t("agent.form.maxMinutes")}
              value={maxMinutes}
              onChange={setMaxMinutes}
            />
            <BudgetField
              id="agent-max-tokens"
              label={t("agent.form.maxTokens")}
              value={maxTokens}
              onChange={setMaxTokens}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            {t("agent.form.limitsHint")}
          </p>

          <div className="flex flex-col gap-2">
            <Label htmlFor="agent-effort">{t("agent.form.effort")}</Label>
            <Select
              value={effort}
              onValueChange={(value) => {
                setEffort(value as AgentEffort);
              }}
            >
              <SelectTrigger id="agent-effort" className="w-full @2xl:w-1/2">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {AGENT_EFFORTS.map((option) => (
                  <SelectItem key={option} value={option}>
                    {t(`agent.effort.${option}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="flex flex-col gap-2">
            <Label htmlFor="agent-hosts">{t("agent.form.allowedHosts")}</Label>
            <Textarea
              id="agent-hosts"
              value={allowedHostsRaw}
              rows={3}
              placeholder={t("agent.form.allowedHostsPlaceholder")}
              onChange={(event) => {
                setAllowedHostsRaw(event.target.value);
              }}
            />
            <p className="text-xs text-muted-foreground">
              {/* The joined list rather than a count: the user needs to see
                  exactly which hosts survive their pasting, and a number can
                  agree with a typo. */}
              {allowedHosts.length > 0
                ? t("agent.form.allowedHostsCount", {
                    hosts: allowedHosts.join(", "),
                  })
                : t("agent.form.allowedHostsHint")}
            </p>
          </div>
        </AnimatedDisclosureContent>
      </div>

      <div className="flex items-center justify-end gap-3">
        {goalProblem === "empty" && (
          <p className="text-xs text-muted-foreground">
            {t("agent.form.goalEmpty")}
          </p>
        )}
        <RippleButton type="submit" size="sm" disabled={!canSubmit}>
          {isStarting ? t("agent.form.starting") : t("agent.form.submit")}
        </RippleButton>
      </div>
    </form>
  );
}

function BudgetField({
  id,
  label,
  value,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (next: string) => void;
}) {
  const { t } = useTranslation();
  const invalid = isBadBudget(value);
  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        inputMode="numeric"
        value={value}
        aria-invalid={invalid}
        placeholder={t("agent.form.budgetUnset")}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      />
      {invalid && (
        <p className="text-xs text-destructive-text">
          {t("agent.form.budgetInvalid")}
        </p>
      )}
    </div>
  );
}

"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AgentRecipes } from "@/components/agent-recipes";
import { AgentRunForm } from "@/components/agent-run-form";
import { AgentRunHistory } from "@/components/agent-run-history";
import { AgentRunPanel } from "@/components/agent-run-view";
import {
  AgentUnavailable,
  type AgentUnavailableReason,
} from "@/components/agent-shared";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  type AgentRecipe,
  getAgentRecipes,
  getAgentRuns,
  isRunOver,
} from "@/lib/agent";
import { parseBackendError } from "@/lib/backend-errors";
import { canUseAgentAutomation } from "@/lib/entitlements";
import type { BrowserProfile, CloudUser } from "@/types";

export type AgentTab = "run" | "history" | "recipes";

interface AgentPageProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  initialTab?: AgentTab;
  profiles: BrowserProfile[];
  cloudUser: CloudUser | null;
}

/**
 * Whether a failure means the deployment has no agent at all.
 *
 * Distinct from every other refusal: a plan that does not include the agent and
 * a backend with no model credential are different problems with different
 * fixes, and telling a paying customer to upgrade because the server is
 * unconfigured is the confusing case this exists to avoid.
 */
function isNotConfigured(error: unknown): boolean {
  return parseBackendError(error)?.code === "AGENT_NOT_CONFIGURED";
}

export function AgentPage({
  isOpen,
  onClose,
  subPage,
  initialTab = "run",
  profiles,
  cloudUser,
}: AgentPageProps) {
  const { t } = useTranslation();
  const entitled = canUseAgentAutomation(cloudUser);
  const signedIn = Boolean(cloudUser);

  const [activeTab, setActiveTab] = useState<AgentTab>(initialTab);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [notConfigured, setNotConfigured] = useState(false);
  const [recipes, setRecipes] = useState<AgentRecipe[]>([]);
  const [recipesLoading, setRecipesLoading] = useState(false);
  const [recipesError, setRecipesError] = useState<unknown>(null);
  // Bumped after any write. It keys the history list, so a started or cancelled
  // run remounts it and it reads from the top instead of showing pages that
  // predate the change.
  const [reloadToken, setReloadToken] = useState(0);
  const hasAdopted = useRef(false);

  const unavailableReason: AgentUnavailableReason | null = !signedIn
    ? "signIn"
    : !entitled
      ? "plan"
      : notConfigured
        ? "notConfigured"
        : null;
  const available = unavailableReason === null;

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

  const loadRecipes = useCallback(async () => {
    setRecipesLoading(true);
    try {
      setRecipes(await getAgentRecipes());
      setRecipesError(null);
    } catch (error) {
      if (isNotConfigured(error)) setNotConfigured(true);
      setRecipesError(error);
    } finally {
      setRecipesLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!isOpen || !available) return;
    void loadRecipes();
  }, [isOpen, available, loadRecipes]);

  // A run that is still going when the page opens becomes the one on screen,
  // so reopening the app during a run lands on it rather than on an empty form
  // with the run only findable through the history tab. One row is read, not a
  // page: this is an adoption check, not a listing.
  useEffect(() => {
    if (!isOpen || !available || hasAdopted.current) return;
    hasAdopted.current = true;
    void (async () => {
      try {
        const page = await getAgentRuns({ limit: 1 });
        const newest = page.runs[0];
        if (newest && !isRunOver(newest)) setActiveRunId(newest.id);
      } catch (error) {
        if (isNotConfigured(error)) setNotConfigured(true);
      }
    })();
  }, [isOpen, available]);

  useEffect(() => {
    if (isOpen) return;
    hasAdopted.current = false;
  }, [isOpen]);

  const handleLoadError = useCallback((error: unknown) => {
    if (isNotConfigured(error)) setNotConfigured(true);
  }, []);

  const body = (
    <div className="@container flex min-h-0 w-full flex-1 flex-col">
      <AnimatedTabs
        value={activeTab}
        onValueChange={(value) => {
          setActiveTab(value as AgentTab);
        }}
        className="flex min-h-0 flex-1 flex-col"
      >
        {/* The tab strip renders whatever the account is entitled to, because
            `AGENT_NOT_CONFIGURED` is a server state discovered by asking: the
            chrome has to already be on screen when the answer arrives. Each
            panel then carries its own honest explanation instead of a form
            that cannot work. */}
        <AnimatedTabsList className="shrink-0">
          <AnimatedTabsTrigger value="run">
            {t("agent.tabs.run")}
          </AnimatedTabsTrigger>
          <AnimatedTabsTrigger value="history">
            {t("agent.tabs.history")}
          </AnimatedTabsTrigger>
          <AnimatedTabsTrigger value="recipes">
            {t("agent.tabs.recipes")}
          </AnimatedTabsTrigger>
        </AnimatedTabsList>

        <AnimatedTabsContent
          value="run"
          className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
        >
          {!available ? (
            <AgentUnavailable reason={unavailableReason} />
          ) : activeRunId !== null ? (
            <AgentRunPanel
              runId={activeRunId}
              profiles={profiles}
              backLabel={t("agent.live.newRun")}
              onBack={() => {
                setActiveRunId(null);
                setReloadToken((token) => token + 1);
              }}
              onChanged={() => {
                setReloadToken((token) => token + 1);
              }}
            />
          ) : (
            <AgentRunForm
              profiles={profiles}
              recipes={recipes}
              onStarted={(run) => {
                setActiveRunId(run.id);
                setReloadToken((token) => token + 1);
              }}
            />
          )}
        </AnimatedTabsContent>

        <AnimatedTabsContent
          value="history"
          className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
        >
          {available ? (
            <AgentRunHistory
              key={reloadToken}
              profiles={profiles}
              onLoadError={handleLoadError}
            />
          ) : (
            <AgentUnavailable reason={unavailableReason} />
          )}
        </AnimatedTabsContent>

        <AnimatedTabsContent
          value="recipes"
          className="mt-4 min-h-0 flex-1 flex-col data-[state=active]:flex"
        >
          {available ? (
            <AgentRecipes
              profiles={profiles}
              recipes={recipes}
              isLoading={recipesLoading}
              error={recipesError}
              onChanged={() => {
                void loadRecipes();
              }}
            />
          ) : (
            <AgentUnavailable reason={unavailableReason} />
          )}
        </AnimatedTabsContent>
      </AnimatedTabs>
    </div>
  );

  return (
    <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
      <DialogContent className="flex max-h-[85vh] max-w-[min(80rem,calc(100%-4rem))] flex-col">
        {!subPage && (
          <DialogHeader>
            <DialogTitle>{t("agent.title")}</DialogTitle>
            <DialogDescription>{t("agent.description")}</DialogDescription>
          </DialogHeader>
        )}
        {body}
      </DialogContent>
    </Dialog>
  );
}

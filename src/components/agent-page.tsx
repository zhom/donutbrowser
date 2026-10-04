"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuHourglass,
  LuLock,
  LuLogIn,
  LuPause,
  LuPlay,
  LuPlug,
  LuPower,
} from "react-icons/lu";
import { AgentActivityFeed } from "@/components/agent-activity";
import {
  SetupPanel,
  useNow,
  useTimeFormat,
} from "@/components/agent-console-parts";
import {
  AgentConversation,
  NeedsYouList,
} from "@/components/agent-conversation";
import { AgentProfiles } from "@/components/agent-profiles";
import { AgentRecipes } from "@/components/agent-recipes";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Button } from "@/components/ui/button";
import { CopyToClipboard } from "@/components/ui/copy-to-clipboard";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
} from "@/components/ui/popover";
import { ProBadge } from "@/components/ui/pro-badge";
import { RippleButton } from "@/components/ui/ripple";
import { Textarea } from "@/components/ui/textarea";
import type { AgentConsole } from "@/hooks/use-agent-console";
import { MAX_TEXT_CHARS, profileRollup, quotaLeft } from "@/lib/agent-console";
import { canUseRemoteControl, getEntitlements } from "@/lib/entitlements";
import type { McpRemoteStatus } from "@/lib/mcp";
import { showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
import type { BrowserProfile, CloudUser } from "@/types";

export type AgentTab = "conversation" | "profiles" | "activity" | "recipes";

const TABS: AgentTab[] = ["conversation", "profiles", "activity", "recipes"];

interface AgentPageProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  initialTab?: AgentTab;
  /** Reports manual tab changes, so Mod+J toggles from where the user is. */
  onTabChange?: (tab: AgentTab) => void;
  profiles: BrowserProfile[];
  cloudUser: CloudUser | null;
  agentConsole: AgentConsole;
  /** Opens Integrations on its remote tab. */
  onConnectAgent: () => void;
  onOpenAccount: () => void;
}

function useRemoteStatus(isOpen: boolean) {
  const [remote, setRemote] = useState<McpRemoteStatus | null>(null);
  const [known, setKnown] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    invoke<McpRemoteStatus>("get_mcp_remote_status")
      .then((status) => {
        if (!cancelled) setRemote(status);
      })
      .catch((error: unknown) => {
        console.warn("Failed to get MCP remote status:", error);
      })
      .finally(() => {
        if (!cancelled) setKnown(true);
      });
    const unlisten = listen<McpRemoteStatus>("mcp-remote-status", (event) => {
      setRemote(event.payload);
      setKnown(true);
    });
    return () => {
      cancelled = true;
      void unlisten.then((off) => {
        off();
      });
    };
  }, [isOpen]);

  return { remote, known };
}

function PauseControl({ agentConsole }: { agentConsole: AgentConsole }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const switchId = useId();
  const noteId = useId();
  const paused = agentConsole.paused !== null;

  const pause = async () => {
    setBusy(true);
    const trimmed = note.trim();
    const ok = await agentConsole.setPaused(true, trimmed || null);
    setBusy(false);
    if (ok) {
      setOpen(false);
      setNote("");
      showSuccessToast(t("agent.pause.pausedToast"));
    }
  };

  const resume = async () => {
    setBusy(true);
    const ok = await agentConsole.setPaused(false, null);
    setBusy(false);
    if (ok) showSuccessToast(t("agent.pause.resumedToast"));
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverAnchor asChild>
        <div className="flex items-center gap-2">
          <AnimatedSwitch
            id={switchId}
            checked={paused || open}
            disabled={busy}
            data-testid="agent-pause-switch"
            onCheckedChange={(checked) => {
              if (checked) setOpen(true);
              else if (paused) void resume();
              else setOpen(false);
            }}
          />
          <Label htmlFor={switchId} className="text-xs whitespace-nowrap">
            {t("agent.pause.label")}
          </Label>
        </div>
      </PopoverAnchor>
      <PopoverContent align="end" className="w-72 p-3">
        <form
          className="flex flex-col gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            void pause();
          }}
        >
          <Label htmlFor={noteId} className="text-xs">
            {t("agent.pause.noteLabel")}
          </Label>
          <Textarea
            id={noteId}
            rows={3}
            value={note}
            maxLength={MAX_TEXT_CHARS}
            placeholder={t("agent.pause.notePlaceholder")}
            className="min-h-16 text-sm"
            data-testid="agent-pause-note"
            onChange={(event) => {
              setNote(event.target.value);
            }}
          />
          <p className="text-[11px] text-muted-foreground">
            {t("agent.pause.explain")}
          </p>
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                setOpen(false);
              }}
            >
              {t("common.buttons.cancel")}
            </Button>
            <Button
              type="submit"
              size="sm"
              disabled={busy}
              data-testid="agent-pause-confirm"
            >
              {t("agent.pause.confirm")}
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  );
}

function PausedBanner({ agentConsole }: { agentConsole: AgentConsole }) {
  const { t } = useTranslation();
  const { shortClock } = useTimeFormat();
  const [busy, setBusy] = useState(false);
  const paused = agentConsole.paused;
  if (!paused) return null;
  return (
    <div
      role="status"
      data-testid="agent-paused-banner"
      className="flex shrink-0 flex-wrap items-center gap-3 rounded-md border border-warning/50 bg-warning/10 px-3 py-2 text-sm text-warning-text"
    >
      <LuPause className="size-4 shrink-0" aria-hidden="true" />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="font-medium">
          {t("agent.pause.banner", { time: shortClock(paused.since) })}
        </span>
        {paused.note && (
          <span className="truncate text-xs">
            {t("agent.pause.bannerNote", { note: paused.note })}
          </span>
        )}
      </div>
      <Button
        type="button"
        size="sm"
        variant="outline"
        className="gap-1.5"
        disabled={busy}
        data-testid="agent-resume"
        onClick={async () => {
          setBusy(true);
          const ok = await agentConsole.setPaused(false, null);
          setBusy(false);
          if (ok) showSuccessToast(t("agent.pause.resumedToast"));
        }}
      >
        <LuPlay className="size-3.5" aria-hidden="true" />
        {t("agent.pause.resume")}
      </Button>
    </div>
  );
}

export function AgentPage({
  isOpen,
  onClose,
  subPage,
  initialTab = "conversation",
  onTabChange,
  profiles,
  cloudUser,
  agentConsole,
  onConnectAgent,
  onOpenAccount,
}: AgentPageProps) {
  const { t } = useTranslation();
  const now = useNow();
  const { remote, known: remoteKnown } = useRemoteStatus(isOpen);

  const [activeTab, setActiveTab] = useState<AgentTab>(initialTab);
  const [adoptedInitialTab, setAdoptedInitialTab] = useState(initialTab);
  if (adoptedInitialTab !== initialTab) {
    setAdoptedInitialTab(initialTab);
    setActiveTab(initialTab);
  }
  const changeTab = useCallback(
    (tab: AgentTab) => {
      setActiveTab(tab);
      onTabChange?.(tab);
    },
    [onTabChange],
  );

  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    null,
  );
  const [composerProfileId, setComposerProfileId] = useState<string | null>(
    null,
  );
  const [focusComposer, setFocusComposer] = useState(false);
  const onComposerFocused = useCallback(() => {
    setFocusComposer(false);
  }, []);

  const signedIn = cloudUser !== null;
  const entitled = canUseRemoteControl(cloudUser);
  const automation = getEntitlements(cloudUser).browserAutomation;
  const blocker = !signedIn ? "signedOut" : !entitled ? "plan" : null;
  const remoteOff = remoteKnown && remote !== null && !remote.enabled;
  const hasSessions = agentConsole.sessions.length > 0;
  const quota = quotaLeft(agentConsole.quota);

  const effectiveSessionId =
    selectedSessionId !== null &&
    agentConsole.sessions.some(
      (session) => session.session_id === selectedSessionId,
    )
      ? selectedSessionId
      : null;

  const profileNames = useMemo(
    () => new Map(profiles.map((profile) => [profile.id, profile.name])),
    [profiles],
  );

  const preferredProfileIds = useMemo(
    () =>
      profileRollup(agentConsole.activity, agentConsole.holds, now).map(
        (row) => row.profile_id,
      ),
    [agentConsole.activity, agentConsole.holds, now],
  );

  const openComposerFor = useCallback(
    (profileId: string, sessionId: string | null) => {
      setComposerProfileId(profileId);
      setSelectedSessionId(sessionId);
      setFocusComposer(true);
      changeTab("conversation");
    },
    [changeTab],
  );

  const blockerPanel =
    blocker === "signedOut" ? (
      <SetupPanel
        testId="agent-setup-signed-out"
        icon={<LuLogIn />}
        title={t("agent.setup.signedOutTitle")}
        body={t("agent.setup.signedOutBody")}
      >
        <RippleButton
          size="sm"
          data-testid="agent-setup-sign-in"
          onClick={onOpenAccount}
        >
          {t("agent.setup.signedOutAction")}
        </RippleButton>
      </SetupPanel>
    ) : blocker === "plan" ? (
      <SetupPanel
        testId="agent-setup-plan"
        icon={<LuLock />}
        title={t("agent.setup.planTitle")}
        body={t("agent.setup.planBody")}
      >
        <div className="flex items-center gap-2">
          <ProBadge />
          <Button
            size="sm"
            variant="outline"
            data-testid="agent-setup-plan-account"
            onClick={onOpenAccount}
          >
            {t("agent.setup.planAction")}
          </Button>
        </div>
      </SetupPanel>
    ) : null;

  const remoteOffPanel = (
    <SetupPanel
      testId="agent-setup-remote-off"
      icon={<LuPower />}
      title={t("agent.setup.remoteOffTitle")}
      body={t("agent.setup.remoteOffBody")}
    >
      <RippleButton
        size="sm"
        data-testid="agent-setup-turn-on"
        onClick={onConnectAgent}
      >
        {t("agent.setup.remoteOffAction")}
      </RippleButton>
    </SetupPanel>
  );

  const waitingPanel = (
    <SetupPanel
      testId="agent-setup-waiting"
      icon={<LuHourglass />}
      title={t("agent.setup.waitingTitle")}
      body={t("agent.setup.waitingBody")}
    >
      <div className="flex w-full max-w-xl flex-col gap-2 text-left">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium text-foreground">
            {t("agent.setup.starterLabel")}
          </span>
          <CopyToClipboard
            text={t("agent.setup.starter")}
            size="sm"
            successMessage={t("agent.setup.starterCopied")}
            className="h-7 w-7 p-0"
          />
        </div>
        <p
          data-testid="agent-starter-text"
          className="max-h-40 overflow-y-auto rounded-md border border-border bg-muted/40 p-3 text-xs break-words whitespace-pre-wrap text-foreground select-text"
        >
          {t("agent.setup.starter")}
        </p>
        <Button
          size="sm"
          variant="outline"
          className="gap-1.5 self-center"
          data-testid="agent-setup-connect"
          onClick={onConnectAgent}
        >
          <LuPlug className="size-3.5" aria-hidden="true" />
          {t("agent.header.connect")}
        </Button>
      </div>
    </SetupPanel>
  );

  const conversationBody = () => {
    if (blockerPanel) return blockerPanel;
    const ready = agentConsole.loaded && remoteKnown;
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3">
        {agentConsole.openRequests.length > 0 && (
          <NeedsYouList
            agentConsole={agentConsole}
            profileNames={profileNames}
            now={now}
          />
        )}
        {!ready && !hasSessions ? null : !hasSessions ? (
          remoteOff ? (
            remoteOffPanel
          ) : (
            waitingPanel
          )
        ) : (
          <>
            {remoteOff && (
              <div
                data-testid="agent-setup-remote-off"
                className="flex shrink-0 flex-wrap items-center gap-3 rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground"
              >
                <LuPower className="size-3.5 shrink-0" aria-hidden="true" />
                <span className="min-w-0 flex-1">
                  {t("agent.setup.remoteOffInline")}
                </span>
                <Button
                  size="sm"
                  variant="outline"
                  data-testid="agent-setup-turn-on"
                  onClick={onConnectAgent}
                >
                  {t("agent.setup.remoteOffAction")}
                </Button>
              </div>
            )}
            <AgentConversation
              agentConsole={agentConsole}
              profiles={profiles}
              profileNames={profileNames}
              selectedSessionId={effectiveSessionId}
              onSelectSession={setSelectedSessionId}
              composerProfileId={composerProfileId}
              onComposerProfileChange={setComposerProfileId}
              focusComposer={focusComposer}
              onComposerFocused={onComposerFocused}
              preferredProfileIds={preferredProfileIds}
              now={now}
            />
          </>
        )}
      </div>
    );
  };

  const title = t("rail.agent");
  const description = t("agent.description");

  return (
    <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
      <DialogContent className="flex max-h-[85vh] max-w-[min(80rem,calc(100%-4rem))] flex-col">
        <div
          data-testid="agent-page"
          className="@container flex min-h-0 w-full flex-1 flex-col gap-3"
        >
          <div className="flex shrink-0 flex-wrap items-start justify-between gap-3">
            <div className="flex min-w-0 flex-col gap-1">
              {subPage ? (
                <>
                  <h2 className="text-base leading-none font-semibold">
                    {title}
                  </h2>
                  <p className="text-xs text-muted-foreground">{description}</p>
                </>
              ) : (
                <>
                  <DialogTitle>{title}</DialogTitle>
                  <DialogDescription>{description}</DialogDescription>
                </>
              )}
            </div>
            {blocker === null && (
              <div className="flex flex-wrap items-center gap-3">
                {quota && (
                  <span
                    data-testid="agent-quota"
                    className={cn(
                      "text-xs tabular-nums",
                      quota.left === 0
                        ? "text-warning-text"
                        : "text-muted-foreground",
                    )}
                  >
                    {t("agent.header.quota", {
                      left: quota.left,
                      limit: quota.limit,
                    })}
                  </span>
                )}
                <PauseControl agentConsole={agentConsole} />
                <RippleButton
                  size="sm"
                  className="flex items-center gap-2"
                  data-testid="agent-connect"
                  onClick={onConnectAgent}
                >
                  <LuPlug className="size-4" aria-hidden="true" />
                  {t("agent.header.connect")}
                </RippleButton>
              </div>
            )}
          </div>

          {blocker === null && <PausedBanner agentConsole={agentConsole} />}

          <AnimatedTabs
            value={activeTab}
            onValueChange={(value) => {
              changeTab(value as AgentTab);
            }}
            className="flex min-h-0 flex-1 flex-col"
          >
            <AnimatedTabsList className="shrink-0">
              {TABS.map((tab) => (
                <AnimatedTabsTrigger
                  key={tab}
                  value={tab}
                  data-testid={`agent-tab-${tab}`}
                >
                  {t(`agent.tabs.${tab}`)}
                  {tab === "conversation" &&
                    agentConsole.openRequests.length > 0 && (
                      <span className="rounded-full bg-warning/15 px-1.5 text-xs tabular-nums text-warning-text">
                        {agentConsole.openRequests.length}
                      </span>
                    )}
                </AnimatedTabsTrigger>
              ))}
            </AnimatedTabsList>

            <AnimatedTabsContent
              value="conversation"
              className="mt-3 min-h-0 flex-1 flex-col data-[state=active]:flex"
            >
              {conversationBody()}
            </AnimatedTabsContent>

            <AnimatedTabsContent
              value="profiles"
              className="mt-3 min-h-0 flex-1 flex-col data-[state=active]:flex"
            >
              {blockerPanel ?? (
                <AgentProfiles
                  agentConsole={agentConsole}
                  profileNames={profileNames}
                  now={now}
                  onNote={openComposerFor}
                />
              )}
            </AnimatedTabsContent>

            <AnimatedTabsContent
              value="activity"
              className="mt-3 min-h-0 flex-1 flex-col data-[state=active]:flex"
            >
              {blockerPanel ?? (
                <AgentActivityFeed
                  agentConsole={agentConsole}
                  profileNames={profileNames}
                  now={now}
                />
              )}
            </AnimatedTabsContent>

            <AnimatedTabsContent
              value="recipes"
              className="mt-3 min-h-0 flex-1 flex-col data-[state=active]:flex"
            >
              {blockerPanel ??
                (automation ? (
                  <AgentRecipes profiles={profiles} />
                ) : (
                  <SetupPanel
                    testId="agent-recipes-plan"
                    icon={<LuLock />}
                    title={t("agent.tabs.recipes")}
                    body={t("agent.recipes.needsAutomation")}
                  >
                    <div className="flex items-center gap-2">
                      <ProBadge />
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={onOpenAccount}
                      >
                        {t("agent.setup.planAction")}
                      </Button>
                    </div>
                  </SetupPanel>
                ))}
            </AnimatedTabsContent>
          </AnimatedTabs>
        </div>
      </DialogContent>
    </Dialog>
  );
}

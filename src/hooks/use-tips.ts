import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { TipsDialogMode } from "@/components/tips-dialog";
import { getAgentRuns } from "@/lib/agent";
import { effectivePlanOf, getEntitlements } from "@/lib/entitlements";
import { getCurrentOS } from "@/lib/platform";
import {
  type FeatureUsage,
  FRESH_LOGIN_WINDOW_MS,
  isPlanTip,
  pickAutoTip,
  TIP_AUTO_DELAY_MS,
  type TipDefinition,
  type TipId,
  type TipProfile,
  tipsFor,
} from "@/lib/tips";
import type { CloudUser } from "@/types";

/** Mirror of `settings_manager::TipsState`. */
export interface TipsState {
  auto_show: boolean;
  seen: string[];
  last_auto_shown_at: number | null;
  next_auto_show_at: number | null;
  auto_due: boolean;
}

/** What the app has already loaded that tells which features are in use. */
export interface TipUsageSource {
  /** False while any of the lists below is still loading. */
  loaded: boolean;
  profiles: readonly TipProfile[];
  groupCount: number;
  extensionGroupCount: number;
  cookieBotEnrolled: boolean;
  syncServerConfigured: boolean;
  proxyIds: readonly string[];
}

/** The settings fields the relevance checks read. */
interface UsageSettings {
  api_enabled?: boolean;
  mcp_enabled?: boolean;
  mcp_remote_enabled?: boolean;
  fingerprint_gate_disabled?: boolean;
}

/** A slow read must not hold the tip back for long. */
const USAGE_READ_TIMEOUT_MS = 5000;
const PROXY_HISTORY_BATCH = 16;

/**
 * The answer of one usage read, or undefined when it fails or is too slow.
 * An unknown fact counts as "not in use", so it never hides a tip.
 */
function known<T>(read: Promise<T>): Promise<T | undefined> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(
      () => resolve(undefined),
      USAGE_READ_TIMEOUT_MS,
    );
    read
      .then(resolve, (error: unknown) => {
        console.warn("Failed to read a tip usage fact:", error);
        resolve(undefined);
      })
      .finally(() => window.clearTimeout(timer));
  });
}

/** Whether any proxy has a check on record, stopping at the first one. */
async function anyProxyChecked(proxyIds: readonly string[]): Promise<boolean> {
  for (let start = 0; start < proxyIds.length; start += PROXY_HISTORY_BATCH) {
    const histories = await Promise.all(
      proxyIds
        .slice(start, start + PROXY_HISTORY_BATCH)
        .map((proxyId) =>
          invoke<unknown[]>("get_proxy_check_history", { proxyId }),
        ),
    );
    if (histories.some((history) => history.length > 0)) return true;
  }
  return false;
}

/**
 * The usage facts for the tips in `candidates`. The backend is asked only
 * for what a candidate's relevance check reads, so a user who has seen the
 * proxy tip never pays for a read of every proxy's check history.
 */
async function loadFeatureUsage(
  source: TipUsageSource,
  inTeam: boolean,
  candidates: ReadonlySet<TipId>,
): Promise<FeatureUsage> {
  const wants = (...ids: TipId[]) => ids.some((id) => candidates.has(id));
  const [settings, proxyChecked, isDefaultBrowser, trashUsed, agentUsed] =
    await Promise.all([
      wants("localApi", "automation", "remoteControl", "fingerprintGate")
        ? known(invoke<UsageSettings>("get_app_settings"))
        : undefined,
      wants("proxyCheck") ? known(anyProxyChecked(source.proxyIds)) : undefined,
      wants("defaultBrowser")
        ? known(invoke<boolean>("is_default_browser"))
        : undefined,
      wants("trash")
        ? known(
            invoke<unknown[]>("list_trashed_profiles").then(
              (trashed) => trashed.length > 0,
            ),
          )
        : undefined,
      wants("agent")
        ? known(getAgentRuns({ limit: 1 }).then((page) => page.runs.length > 0))
        : undefined,
    ]);
  return {
    profiles: source.profiles,
    currentOs: getCurrentOS(),
    groupCount: source.groupCount,
    extensionGroupCount: source.extensionGroupCount,
    inTeam,
    cookieBotEnrolled: source.cookieBotEnrolled,
    syncServerConfigured: source.syncServerConfigured,
    apiEnabled: settings?.api_enabled,
    mcpEnabled: settings?.mcp_enabled,
    remoteControlEnabled: settings?.mcp_remote_enabled,
    fingerprintGateChanged: settings?.fingerprint_gate_disabled,
    proxyChecked,
    isDefaultBrowser,
    trashUsed,
    agentUsed,
  };
}

export interface TipsDialogState {
  open: boolean;
  mode: TipsDialogMode;
  initialTipId: TipId | null;
  auto: boolean;
  /** Bumped on every open so the dialog remounts with fresh navigation state. */
  session: number;
}

interface PaidWelcomeState {
  plan: string;
  status: "pending" | "open" | "done";
}

interface UseTipsOptions {
  cloudUser: CloudUser | null;
  loggedInAt: string | null;
  /**
   * The app is settled enough to put a dialog in front of the user: not the
   * first-run session, terms accepted, nothing else blocking.
   */
  ready: boolean;
  /**
   * Another launch dialog has this launch (the remote MCP move), or has not
   * decided yet. The automatic tip waits, and skips the launch it loses.
   */
  autoTipHeld?: boolean;
  /** Read when the automatic tip is chosen, to skip features already in use. */
  usage: TipUsageSource;
}

/**
 * The tips flow: which tips this install may see, what has been seen, the
 * single tip that opens by itself every few days, and the welcome for an
 * account that just turned paid. State lives in the app settings; this hook
 * only decides.
 */
export function useTips({
  cloudUser,
  loggedInAt,
  ready,
  autoTipHeld = false,
  usage,
}: UseTipsOptions) {
  const [state, setState] = useState<TipsState | null>(null);
  const [dialog, setDialog] = useState<TipsDialogState>({
    open: false,
    mode: "browse",
    initialTipId: null,
    auto: false,
    session: 0,
  });
  const [paidWelcome, setPaidWelcome] = useState<PaidWelcomeState | null>(null);

  const entitlements = useMemo(() => getEntitlements(cloudUser), [cloudUser]);
  const tips = useMemo(() => tipsFor(entitlements), [entitlements]);
  const planTips = useMemo(() => tips.filter(isPlanTip), [tips]);

  useEffect(() => {
    let cancelled = false;
    invoke<TipsState>("get_tips_state")
      .then((loaded) => {
        if (!cancelled) setState(loaded);
      })
      .catch((error: unknown) => {
        console.error("Failed to load the tips state:", error);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const openTips = useCallback(
    (
      initialTipId: TipId | null = null,
      options: { mode?: TipsDialogMode; auto?: boolean } = {},
    ) => {
      setDialog((previous) => ({
        open: true,
        mode: options.mode ?? "browse",
        initialTipId,
        auto: options.auto ?? false,
        session: previous.session + 1,
      }));
    },
    [],
  );

  const closeTips = useCallback(() => {
    setDialog((previous) =>
      previous.open ? { ...previous, open: false } : previous,
    );
  }, []);

  // One observation per account and plan status. The backend remembers the
  // status and answers whether this is the moment to greet a new paid plan.
  const observedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!cloudUser) return;
    const paid = entitlements.active;
    const key = `${cloudUser.id}:${paid ? "paid" : "free"}`;
    if (observedRef.current === key) return;
    observedRef.current = key;
    const freshLogin =
      loggedInAt !== null &&
      Date.now() - Date.parse(loggedInAt) < FRESH_LOGIN_WINDOW_MS;
    invoke<boolean>("observe_cloud_plan", {
      userId: cloudUser.id,
      paid,
      freshLogin,
    })
      .then((due) => {
        if (!due) return;
        setPaidWelcome({ plan: effectivePlanOf(cloudUser), status: "pending" });
      })
      .catch((error: unknown) => {
        console.error("Failed to record the cloud plan:", error);
      });
  }, [cloudUser, entitlements.active, loggedInAt]);

  // The welcome waits for a quiet moment: the app settled and no tip open.
  useEffect(() => {
    if (!ready || dialog.open || paidWelcome?.status !== "pending") return;
    setPaidWelcome({ plan: paidWelcome.plan, status: "open" });
  }, [ready, dialog.open, paidWelcome]);

  // The usage lists change all the time (a profile starts, a group count
  // moves); they are read when the tip is chosen, not watched, so a change
  // does not restart the delay below.
  const usageRef = useRef(usage);
  useEffect(() => {
    usageRef.current = usage;
  });
  const inTeam = Boolean(cloudUser?.teamId);

  // The automatic tip: one random unseen tip for a feature the user does not
  // use yet, a moment after the app settles, never on top of the paid
  // welcome. Marked handled only once it opens, so a welcome arriving during
  // the delay simply takes its place.
  const autoHandledRef = useRef(false);
  useEffect(() => {
    if (!ready || !usage.loaded || !state || autoHandledRef.current) return;
    if (autoTipHeld || !state.auto_due || dialog.open) return;
    if (paidWelcome && paidWelcome.status !== "done") return;
    const unseen = tips.filter((tip) => !state.seen.includes(tip.id));
    if (unseen.length === 0) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        const facts = await loadFeatureUsage(
          usageRef.current,
          inTeam,
          new Set(unseen.map((tip) => tip.id)),
        );
        if (cancelled || autoHandledRef.current) return;
        const tip = pickAutoTip(tips, state.seen, facts);
        if (!tip) return;
        autoHandledRef.current = true;
        openTips(tip.id, { mode: "single", auto: true });
      })();
    }, TIP_AUTO_DELAY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [
    ready,
    autoTipHeld,
    usage.loaded,
    state,
    dialog.open,
    paidWelcome,
    tips,
    inTeam,
    openTips,
  ]);

  const markSeen = useCallback(async (id: TipId, auto: boolean) => {
    try {
      setState(await invoke<TipsState>("mark_tip_seen", { tipId: id, auto }));
    } catch (error) {
      console.error("Failed to remember the tip as seen:", error);
    }
  }, []);

  const setAutoShow = useCallback(async (enabled: boolean) => {
    try {
      setState(await invoke<TipsState>("set_tips_auto_show", { enabled }));
    } catch (error) {
      console.error("Failed to save the tips preference:", error);
    }
  }, []);

  const dismissPaidWelcome = useCallback(() => {
    setPaidWelcome((previous) =>
      previous ? { ...previous, status: "done" } : previous,
    );
  }, []);

  return {
    tips,
    planTips,
    seen: state?.seen ?? [],
    autoShow: state?.auto_show ?? true,
    dialog,
    openTips,
    closeTips,
    markSeen,
    setAutoShow,
    paidWelcome: {
      open: paidWelcome?.status === "open",
      plan: paidWelcome?.plan ?? "",
    },
    dismissPaidWelcome,
  };
}

export type TipsFlow = ReturnType<typeof useTips>;
export type { TipDefinition };

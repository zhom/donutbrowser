import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { TipsDialogMode } from "@/components/tips-dialog";
import { effectivePlanOf, getEntitlements } from "@/lib/entitlements";
import {
  FRESH_LOGIN_WINDOW_MS,
  isPlanTip,
  pickAutoTip,
  TIP_AUTO_DELAY_MS,
  type TipDefinition,
  type TipId,
  tipsFor,
} from "@/lib/tips";
import type { CloudUser } from "@/types";

/** Mirror of `settings_manager::TipsState`. */
export interface TipsState {
  auto_show: boolean;
  seen: string[];
  last_auto_shown_at: number | null;
  auto_due: boolean;
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
}

/**
 * The tips flow: which tips this install may see, what has been seen, the
 * one tip a day that opens by itself, and the welcome for an account that
 * just turned paid. State lives in the app settings; this hook only decides.
 */
export function useTips({ cloudUser, loggedInAt, ready }: UseTipsOptions) {
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

  // The automatic tip: one unseen tip, a moment after the app settles, and
  // never on top of the paid welcome. Marked handled only once it opens, so a
  // welcome arriving during the delay simply takes its place.
  const autoHandledRef = useRef(false);
  useEffect(() => {
    if (!ready || !state || autoHandledRef.current) return;
    if (!state.auto_due || dialog.open) return;
    if (paidWelcome && paidWelcome.status !== "done") return;
    const tip = pickAutoTip(tips, state.seen);
    if (!tip) return;
    const timer = window.setTimeout(() => {
      autoHandledRef.current = true;
      openTips(tip.id, { mode: "single", auto: true });
    }, TIP_AUTO_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [ready, state, dialog.open, paidWelcome, tips, openTips]);

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

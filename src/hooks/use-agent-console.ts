import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AGENT_CONSOLE_EVENTS,
  type AgentActivity,
  type AgentConsoleSnapshot,
  type AgentConsoleStatePayload,
  type AgentHold,
  type AgentSession,
  type AgentsPause,
  type AgentThreadItem,
  type AutomationQuota,
  answerAgentRequest,
  appendActivity,
  clearAgentActivity,
  dismissAgentRequest,
  getAgentConsole,
  handBackProfile,
  isOpenRequest,
  openRequests,
  sendAgentNote,
  sessionLabel,
  setAgentsPaused,
  showProfileWindow,
  takeOverProfile,
  upsertSession,
  upsertThreadItem,
} from "@/lib/agent-console";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showToast } from "@/lib/toast-utils";

interface ConsoleData {
  loaded: boolean;
  sessions: AgentSession[];
  activity: AgentActivity[];
  thread: AgentThreadItem[];
  holds: AgentHold[];
  paused: AgentsPause | null;
  quota: AutomationQuota | null;
}

const EMPTY: ConsoleData = {
  loaded: false,
  sessions: [],
  activity: [],
  thread: [],
  holds: [],
  paused: null,
  quota: null,
};

export interface AgentConsole extends ConsoleData {
  openRequests: AgentThreadItem[];
  answer: (requestId: number, answer: string) => Promise<boolean>;
  dismiss: (requestId: number) => Promise<boolean>;
  sendNote: (
    text: string,
    sessionId: string | null,
    profileId: string | null,
  ) => Promise<boolean>;
  takeOver: (profileId: string, note: string | null) => Promise<boolean>;
  handBack: (profileId: string, note: string | null) => Promise<boolean>;
  setPaused: (paused: boolean, note: string | null) => Promise<boolean>;
  clearActivity: () => Promise<boolean>;
  showWindow: (profileId: string) => Promise<boolean>;
}

interface UseAgentConsoleOptions {
  /** Navigates to the Agent page from a request notification. */
  onOpenPage: () => void;
  /** The Agent page is on screen, so a toast would repeat what it shows. */
  pageVisible: boolean;
}

export function useAgentConsole({
  onOpenPage,
  pageVisible,
}: UseAgentConsoleOptions): AgentConsole {
  const { t } = useTranslation();
  const [data, setData] = useState<ConsoleData>(EMPTY);
  const pendingActivity = useRef<AgentActivity[]>([]);
  const flushHandle = useRef<{ id: number; frame: boolean } | null>(null);
  const notified = useRef<Set<number>>(new Set());
  const sessionsRef = useRef<AgentSession[]>([]);
  const optionsRef = useRef({ onOpenPage, pageVisible, t });
  optionsRef.current = { onOpenPage, pageVisible, t };

  useEffect(() => {
    sessionsRef.current = data.sessions;
  }, [data.sessions]);

  const applySnapshot = useCallback((snapshot: AgentConsoleSnapshot) => {
    for (const item of snapshot.thread) {
      if (isOpenRequest(item)) notified.current.add(item.id);
    }
    setData((previous) => ({
      loaded: true,
      sessions: snapshot.sessions,
      activity: appendActivity(snapshot.activity, previous.activity),
      thread: snapshot.thread,
      holds: snapshot.holds,
      paused: snapshot.paused,
      quota: snapshot.quota,
    }));
  }, []);

  const refetch = useCallback(async () => {
    try {
      const snapshot = await getAgentConsole();
      setData((previous) => ({ ...previous, activity: [] }));
      applySnapshot(snapshot);
    } catch (error) {
      console.warn("Failed to read the agent console:", error);
    }
  }, [applySnapshot]);

  const notifyRequest = useCallback((item: AgentThreadItem) => {
    const {
      t: translate,
      pageVisible: visible,
      onOpenPage: open,
    } = optionsRef.current;
    const focused = typeof document !== "undefined" && document.hasFocus();
    if (!visible || !focused) {
      const session = sessionsRef.current.find(
        (entry) => entry.session_id === item.session_id,
      );
      const agent = sessionLabel(
        session,
        translate("agent.sessions.fallback"),
        translate("agent.sessions.website"),
      );
      showToast({
        id: `agent-request-${item.id}`,
        type: "info",
        title: translate(
          item.kind === "help"
            ? "agent.notify.helpTitle"
            : "agent.notify.questionTitle",
          { agent },
        ),
        description:
          item.text.length > 160 ? `${item.text.slice(0, 159)}…` : item.text,
        duration: 12000,
        action: { label: translate("agent.notify.open"), onClick: open },
      });
    }
    if (!focused) {
      void getCurrentWindow()
        .requestUserAttention(UserAttentionType.Informational)
        .catch(() => undefined);
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    const offs: Array<() => void> = [];

    const flush = () => {
      flushHandle.current = null;
      const batch = pendingActivity.current;
      if (batch.length === 0) return;
      pendingActivity.current = [];
      setData((previous) => ({
        ...previous,
        activity: appendActivity(previous.activity, batch),
      }));
    };
    const scheduleFlush = () => {
      if (flushHandle.current !== null) return;
      const hidden = typeof document !== "undefined" && document.hidden;
      flushHandle.current = hidden
        ? { id: window.setTimeout(flush, 250), frame: false }
        : { id: window.requestAnimationFrame(flush), frame: true };
    };

    void (async () => {
      const subscriptions = await Promise.all([
        listen<AgentSession>(AGENT_CONSOLE_EVENTS.session, (event) => {
          setData((previous) => ({
            ...previous,
            sessions: upsertSession(previous.sessions, event.payload),
          }));
        }),
        listen<AgentActivity>(AGENT_CONSOLE_EVENTS.activity, (event) => {
          pendingActivity.current.push(event.payload);
          if (pendingActivity.current.length > 1000) {
            pendingActivity.current = pendingActivity.current.slice(-1000);
          }
          scheduleFlush();
        }),
        listen<AgentThreadItem>(AGENT_CONSOLE_EVENTS.thread, (event) => {
          const item = event.payload;
          setData((previous) => ({
            ...previous,
            thread: upsertThreadItem(previous.thread, item),
          }));
          if (isOpenRequest(item) && !notified.current.has(item.id)) {
            notified.current.add(item.id);
            notifyRequest(item);
          }
        }),
        listen<AgentConsoleStatePayload>(
          AGENT_CONSOLE_EVENTS.state,
          (event) => {
            setData((previous) => ({
              ...previous,
              holds: event.payload.holds,
              paused: event.payload.paused,
              quota: event.payload.quota,
            }));
          },
        ),
        listen(AGENT_CONSOLE_EVENTS.cleared, () => {
          void refetch();
        }),
      ]);
      if (disposed) {
        for (const off of subscriptions) off();
        return;
      }
      offs.push(...subscriptions);
      try {
        const snapshot = await getAgentConsole();
        if (!disposed) applySnapshot(snapshot);
      } catch (error) {
        console.warn("Failed to read the agent console:", error);
        if (!disposed) setData((previous) => ({ ...previous, loaded: true }));
      }
    })();

    return () => {
      disposed = true;
      for (const off of offs) off();
      const handle = flushHandle.current;
      if (handle !== null) {
        if (handle.frame) window.cancelAnimationFrame(handle.id);
        else window.clearTimeout(handle.id);
        flushHandle.current = null;
      }
    };
  }, [applySnapshot, notifyRequest, refetch]);

  const run = useCallback(
    async (action: () => Promise<unknown>): Promise<boolean> => {
      try {
        await action();
        return true;
      } catch (error) {
        showErrorToast(translateBackendError(optionsRef.current.t, error));
        return false;
      }
    },
    [],
  );

  const answer = useCallback(
    (requestId: number, text: string) =>
      run(async () => {
        const item = await answerAgentRequest(requestId, text);
        setData((previous) => ({
          ...previous,
          thread: upsertThreadItem(previous.thread, item),
        }));
      }),
    [run],
  );

  const dismiss = useCallback(
    (requestId: number) =>
      run(async () => {
        const item = await dismissAgentRequest(requestId);
        setData((previous) => ({
          ...previous,
          thread: upsertThreadItem(previous.thread, item),
        }));
      }),
    [run],
  );

  const sendNote = useCallback(
    (text: string, sessionId: string | null, profileId: string | null) =>
      run(async () => {
        const item = await sendAgentNote(text, sessionId, profileId);
        setData((previous) => ({
          ...previous,
          thread: upsertThreadItem(previous.thread, item),
        }));
      }),
    [run],
  );

  const takeOver = useCallback(
    (profileId: string, note: string | null) =>
      run(async () => {
        const hold = await takeOverProfile(profileId, note);
        setData((previous) => ({
          ...previous,
          holds: [
            ...previous.holds.filter(
              (entry) => entry.profile_id !== hold.profile_id,
            ),
            hold,
          ],
        }));
      }),
    [run],
  );

  const handBack = useCallback(
    (profileId: string, note: string | null) =>
      run(async () => {
        await handBackProfile(profileId, note);
        setData((previous) => ({
          ...previous,
          holds: previous.holds.filter(
            (entry) => entry.profile_id !== profileId,
          ),
        }));
      }),
    [run],
  );

  const setPaused = useCallback(
    (paused: boolean, note: string | null) =>
      run(async () => {
        const next = await setAgentsPaused(paused, note);
        setData((previous) => ({ ...previous, paused: next }));
      }),
    [run],
  );

  const clearActivity = useCallback(
    () =>
      run(async () => {
        await clearAgentActivity();
        pendingActivity.current = [];
        setData((previous) => ({ ...previous, activity: [] }));
      }),
    [run],
  );

  const showWindow = useCallback(
    (profileId: string) => run(() => showProfileWindow(profileId)),
    [run],
  );

  const requests = useMemo(() => openRequests(data.thread), [data.thread]);

  return useMemo(
    () => ({
      ...data,
      openRequests: requests,
      answer,
      dismiss,
      sendNote,
      takeOver,
      handBack,
      setPaused,
      clearActivity,
      showWindow,
    }),
    [
      data,
      requests,
      answer,
      dismiss,
      sendNote,
      takeOver,
      handBack,
      setPaused,
      clearActivity,
      showWindow,
    ],
  );
}

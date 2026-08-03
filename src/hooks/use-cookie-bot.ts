"use client";

import { useCallback, useEffect, useId, useSyncExternalStore } from "react";
import {
  type CookieBotSchedule,
  type CookieBotScope,
  getCookieBotSchedules,
  getRemoteHoursQuota,
  type RemoteHoursQuota,
} from "@/lib/cookie-bot";
import {
  isSessionOver,
  onRemoteSessionSnapshot,
  onRemoteSessionState,
  onRemoteSessionStream,
  type RemoteSessionState,
  startRemoteSessionEvents,
  stopRemoteSessionEvents,
} from "@/lib/remote-sessions";

/**
 * One shared read of the cookie-bot plane.
 *
 * Several surfaces need the same three facts at once — the profile table, the
 * account page, the enrolment dialog — and each is mounted independently. A
 * per-component fetch would mean three requests on open and three different
 * answers after an edit, so the state lives in one module store and every
 * consumer subscribes to it.
 *
 * `POST /api/remote-sessions` answers `provisioning` and nothing more, so a
 * live run is only observable through the event stream. This store is what
 * starts that stream on the desktop: without it a signed-in user is blind
 * between launch and stop.
 */
export interface CookieBotSnapshot {
  /** Enrolments by profile id. */
  schedules: Record<string, CookieBotSchedule>;
  /** Remote sessions that have not closed yet, by profile id. */
  liveSessions: Record<string, RemoteSessionState>;
  /** The pooled remote-hour budget, or null before the first answer. */
  quota: RemoteHoursQuota | null;
  /** True only while the first load of a newly enabled session is in flight. */
  isLoading: boolean;
  /** The last load failure, as a backend error code envelope or message. */
  error: string | null;
  /** Whether transitions are currently arriving. */
  streamConnected: boolean;
}

const EMPTY: CookieBotSnapshot = {
  schedules: {},
  liveSessions: {},
  quota: null,
  isLoading: false,
  error: null,
  streamConnected: false,
};

type Listener = () => void;

const listeners = new Set<Listener>();
let snapshot: CookieBotSnapshot = EMPTY;
let subscriberCount = 0;
let unlisteners: (() => void)[] = [];
let attachGeneration = 0;
let enabled = false;
let scope: CookieBotScope = "mine";
let loadToken = 0;

function emit(next: Partial<CookieBotSnapshot>) {
  snapshot = { ...snapshot, ...next };
  for (const listener of listeners) listener();
}

function getSnapshot(): CookieBotSnapshot {
  return snapshot;
}

/** Sessions still in flight, keyed by the profile they are warming. */
function indexOpenSessions(
  sessions: RemoteSessionState[],
): Record<string, RemoteSessionState> {
  const next: Record<string, RemoteSessionState> = {};
  for (const session of sessions) {
    if (!session.profile_id || isSessionOver(session)) continue;
    next[session.profile_id] = session;
  }
  return next;
}

async function attachStream() {
  // Idempotent on the Rust side; calling it here is what covers the user who
  // signs in without restarting the app.
  try {
    await startRemoteSessionEvents();
  } catch (error) {
    // A signed-out or unentitled desktop refuses the subscription. That is not
    // a UI failure — the rest of the state still renders — so it is logged and
    // the stream simply stays disconnected.
    console.error("Failed to subscribe to remote session events:", error);
  }
}

async function attachListeners() {
  // `listen()` resolves a tick later, and subscribe/unsubscribe can both happen
  // before it does (React runs mount effects twice in development). Without
  // this generation check the first attach would install its handlers after the
  // detach had already run, leaking a listener that no unsubscribe can reach
  // and double-emitting every session transition for the rest of the session.
  const generation = ++attachGeneration;
  const offs = await Promise.all([
    onRemoteSessionState((session) => {
      if (!session.profile_id) return;
      const over = isSessionOver(session);
      const next = { ...snapshot.liveSessions };
      if (over) {
        delete next[session.profile_id];
      } else {
        next[session.profile_id] = session;
      }
      emit({ liveSessions: next });
      // A run that just ended has spent hours and may have moved the
      // schedule's next slot, so both are re-read rather than guessed at.
      if (over) void refreshCookieBot();
    }),
    onRemoteSessionSnapshot((payload) => {
      emit({ liveSessions: indexOpenSessions(payload.sessions) });
    }),
    onRemoteSessionStream((status) => {
      emit({ streamConnected: status.connected });
    }),
  ]);
  if (generation !== attachGeneration) {
    for (const off of offs) off();
    return;
  }
  unlisteners = offs;
}

function detachListeners() {
  attachGeneration += 1;
  for (const off of unlisteners) off();
  unlisteners = [];
}

async function load() {
  const token = ++loadToken;
  emit({ isLoading: snapshot.quota === null, error: null });
  const [schedules, quota] = await Promise.allSettled([
    getCookieBotSchedules(scope),
    getRemoteHoursQuota(),
  ]);
  // A later load (or a sign-out) has already superseded this one.
  if (token !== loadToken || !enabled) return;

  const next: Partial<CookieBotSnapshot> = { isLoading: false };
  if (schedules.status === "fulfilled") {
    const byProfile: Record<string, CookieBotSchedule> = {};
    for (const schedule of schedules.value.schedules) {
      byProfile[schedule.profile_id] = schedule;
    }
    next.schedules = byProfile;
  }
  if (quota.status === "fulfilled") next.quota = quota.value;

  // Both failing is a real outage worth surfacing; one failing leaves the
  // other half of the screen correct, which beats blanking everything.
  if (schedules.status === "rejected" && quota.status === "rejected") {
    next.error = String(schedules.reason);
  } else {
    next.error = null;
  }
  emit(next);
}

/** Re-read schedules and the quota. Call after any write. */
export async function refreshCookieBot(): Promise<void> {
  if (!enabled) return;
  await load();
}

function reconcile(nextEnabled: boolean, nextScope: CookieBotScope) {
  const scopeChanged = nextScope !== scope;
  scope = nextScope;
  if (nextEnabled === enabled) {
    if (nextEnabled && scopeChanged) void load();
    return;
  }
  enabled = nextEnabled;
  if (enabled) {
    void attachStream();
    void load();
  } else {
    loadToken++;
    void stopRemoteSessionEvents().catch((error: unknown) => {
      console.error("Failed to unsubscribe from remote session events:", error);
    });
    snapshot = EMPTY;
    for (const listener of listeners) listener();
  }
}

/**
 * What each mounted consumer wants. The store is a singleton with several
 * consumers whose lifetimes differ — the shell holds it open for the whole
 * session, a dialog only while it is on screen — so the wish is a union, never
 * last-writer-wins. Without this, closing the enrol dialog (which passes
 * `isOpen && entitled`) would tear down the event stream the shell and the
 * profile table are still reading from.
 */
const wishes = new Map<string, { enabled: boolean; scope: CookieBotScope }>();
let applyScheduled = false;

function applyWishes() {
  let nextEnabled = false;
  let nextScope: CookieBotScope = "mine";
  for (const wish of wishes.values()) {
    if (wish.enabled) nextEnabled = true;
    if (wish.scope === "team") nextScope = "team";
  }
  reconcile(nextEnabled, nextScope);
}

/**
 * React runs an effect's cleanup and its next body back to back, so a consumer
 * re-registering would otherwise be seen as a momentary "nobody wants this" and
 * flush the whole snapshot. Coalescing into a microtask means only the settled
 * state is ever acted on.
 */
function scheduleApplyWishes() {
  if (applyScheduled) return;
  applyScheduled = true;
  queueMicrotask(() => {
    applyScheduled = false;
    applyWishes();
  });
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  subscriberCount += 1;
  if (subscriberCount === 1) void attachListeners();
  return () => {
    listeners.delete(listener);
    subscriberCount -= 1;
    if (subscriberCount === 0) detachListeners();
  };
}

/**
 * Which enrolments to read. Derive it here rather than at each call site: the
 * store is shared, so two consumers asking for different scopes would refetch
 * over each other, and the server refuses `team` from a caller with no team.
 */
export function cookieBotScopeFor(
  user: { teamId?: string } | null | undefined,
): CookieBotScope {
  return user?.teamId ? "team" : "mine";
}

export interface UseCookieBotResult extends CookieBotSnapshot {
  refresh: () => Promise<void>;
  /** This profile's enrolment, or null. */
  scheduleFor: (profileId: string) => CookieBotSchedule | null;
  /** An unfinished remote session for this profile, or null. */
  liveSessionFor: (profileId: string) => RemoteSessionState | null;
}

/**
 * @param isEnabled the user is signed in and entitled. False keeps the store
 *   idle so an unentitled desktop never polls a route that will refuse it.
 * @param teamScope read the whole team's enrolments. Only pass `"team"` when
 *   the user actually belongs to one — the server refuses it otherwise.
 */
export function useCookieBot(
  isEnabled: boolean,
  teamScope: CookieBotScope = "mine",
): UseCookieBotResult {
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const consumerId = useId();

  useEffect(() => {
    wishes.set(consumerId, { enabled: isEnabled, scope: teamScope });
    scheduleApplyWishes();
    return () => {
      wishes.delete(consumerId);
      scheduleApplyWishes();
    };
  }, [consumerId, isEnabled, teamScope]);

  const scheduleFor = useCallback(
    (profileId: string) => state.schedules[profileId] ?? null,
    [state.schedules],
  );
  const liveSessionFor = useCallback(
    (profileId: string) => state.liveSessions[profileId] ?? null,
    [state.liveSessions],
  );

  return {
    ...state,
    refresh: refreshCookieBot,
    scheduleFor,
    liveSessionFor,
  };
}

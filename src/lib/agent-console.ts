import { invoke } from "@tauri-apps/api/core";

/**
 * The human side of an agent connected over the remote MCP bridge. Mirrors
 * `src-tauri/src/agent_console.rs`; timestamps are unix milliseconds.
 */

export interface AgentStatus {
  message: string;
  done: number | null;
  total: number | null;
  profile_id: string | null;
  updated_at: number;
}

export interface AgentSession {
  session_id: string;
  client_name: string | null;
  client_version: string | null;
  connected_at: number;
  last_seen_at: number;
  calls: number;
  errors: number;
  ended: boolean;
  status: AgentStatus | null;
}

export interface AgentActivity {
  id: number;
  at: number;
  session_id: string | null;
  tool: string;
  profile_id: string | null;
  profile_count: number | null;
  ok: boolean;
  error_code: string | null;
  duration_ms: number;
  /** Navigate: the host. Typing tools: characters typed, as a decimal string. */
  detail: string | null;
}

export type AgentThreadKind =
  | "question"
  | "help"
  | "note"
  | "progress"
  | "joined"
  | "left"
  | "feedback";

export type AgentThreadState =
  | "open"
  | "answered"
  | "dismissed"
  | "pending"
  | "delivered"
  | "none";

export interface AgentThreadItem {
  id: number;
  at: number;
  kind: AgentThreadKind;
  /** For a note: the target agent, null for all agents. */
  session_id: string | null;
  text: string;
  profile_id: string | null;
  choices: string[];
  state: AgentThreadState;
  answer: string | null;
  answered_at: number | null;
  delivered_to: string[];
  done: number | null;
  total: number | null;
  /** Set on a `feedback` item: what the agent sent to the Donut team. */
  feedback: { kind: FeedbackKind; logs: boolean } | null;
}

export type FeedbackKind = "bug" | "idea" | "praise" | "other";

export interface AgentHold {
  profile_id: string;
  since: number;
  note: string | null;
  request_id: number | null;
}

export interface AgentsPause {
  since: number;
  note: string | null;
}

/** `limit` 0 means unlimited. */
export interface AutomationQuota {
  limit: number;
  used: number;
  resets_in_secs: number | null;
}

export interface AgentConsoleSnapshot {
  sessions: AgentSession[];
  activity: AgentActivity[];
  thread: AgentThreadItem[];
  holds: AgentHold[];
  paused: AgentsPause | null;
  quota: AutomationQuota | null;
}

export interface AgentConsoleStatePayload {
  holds: AgentHold[];
  paused: AgentsPause | null;
  quota: AutomationQuota | null;
}

export const AGENT_CONSOLE_EVENTS = {
  session: "agent-console-session",
  activity: "agent-console-activity",
  thread: "agent-console-thread",
  state: "agent-console-state",
  cleared: "agent-console-cleared",
} as const;

const MAX_SESSIONS = 50;
export const MAX_ACTIVITY = 1000;
const MAX_THREAD = 2000;
export const MAX_TEXT_CHARS = 2000;
const ACTIVE_WINDOW_MS = 90_000;
const EARLIER_AFTER_MS = 30 * 60_000;
const STATS_WINDOW_MS = 5 * 60_000;

export function getAgentConsole(): Promise<AgentConsoleSnapshot> {
  return invoke<AgentConsoleSnapshot>("get_agent_console");
}

export function answerAgentRequest(
  requestId: number,
  answer: string,
): Promise<AgentThreadItem> {
  return invoke<AgentThreadItem>("answer_agent_request", { requestId, answer });
}

export function dismissAgentRequest(
  requestId: number,
): Promise<AgentThreadItem> {
  return invoke<AgentThreadItem>("dismiss_agent_request", { requestId });
}

export function sendAgentNote(
  text: string,
  sessionId: string | null,
  profileId: string | null,
): Promise<AgentThreadItem> {
  return invoke<AgentThreadItem>("send_agent_note", {
    text,
    sessionId,
    profileId,
  });
}

export function takeOverProfile(
  profileId: string,
  note: string | null,
): Promise<AgentHold> {
  return invoke<AgentHold>("take_over_profile", { profileId, note });
}

export function handBackProfile(
  profileId: string,
  note: string | null,
): Promise<void> {
  return invoke<void>("hand_back_profile", { profileId, note });
}

export function setAgentsPaused(
  paused: boolean,
  note: string | null,
): Promise<AgentsPause | null> {
  return invoke<AgentsPause | null>("set_agents_paused", { paused, note });
}

export function clearAgentActivity(): Promise<void> {
  return invoke<void>("clear_agent_activity");
}

export function showProfileWindow(profileId: string): Promise<void> {
  return invoke<void>("show_profile_window", { profileId });
}

/** The `clientInfo.name` Donut's own website sends. */
export const WEBSITE_CLIENT_NAME = "donutbrowser-web";

export function sessionLabel(
  session: Pick<AgentSession, "client_name"> | null | undefined,
  fallback: string,
  websiteLabel?: string,
): string {
  const name = session?.client_name?.trim();
  if (name === WEBSITE_CLIENT_NAME && websiteLabel) return websiteLabel;
  return name ? name : fallback;
}

export function isSessionActive(session: AgentSession, now: number): boolean {
  return !session.ended && now - session.last_seen_at <= ACTIVE_WINDOW_MS;
}

/** Idle long enough to sit behind "Show earlier". */
export function isSessionEarlier(session: AgentSession, now: number): boolean {
  return now - session.last_seen_at > EARLIER_AFTER_MS;
}

export function isOpenRequest(item: AgentThreadItem): boolean {
  return (
    (item.kind === "question" || item.kind === "help") && item.state === "open"
  );
}

/** Open questions and help requests, oldest first. */
export function openRequests(
  thread: readonly AgentThreadItem[],
): AgentThreadItem[] {
  return thread.filter(isOpenRequest).sort((a, b) => a.id - b.id);
}

function upsertById<T, K>(
  list: readonly T[],
  item: T,
  keyOf: (value: T) => K,
): T[] {
  const key = keyOf(item);
  const index = list.findIndex((value) => keyOf(value) === key);
  if (index === -1) return [...list, item];
  const next = [...list];
  next[index] = item;
  return next;
}

export function upsertSession(
  sessions: readonly AgentSession[],
  session: AgentSession,
): AgentSession[] {
  return upsertById(sessions, session, (value) => value.session_id)
    .sort((a, b) => b.last_seen_at - a.last_seen_at)
    .slice(0, MAX_SESSIONS);
}

export function upsertThreadItem(
  thread: readonly AgentThreadItem[],
  item: AgentThreadItem,
): AgentThreadItem[] {
  const next = upsertById(thread, item, (value) => value.id);
  if (next.length > thread.length && thread.length > 0) {
    const last = thread[thread.length - 1];
    if (last.id > item.id) next.sort((a, b) => a.id - b.id);
  }
  return next.length > MAX_THREAD ? next.slice(-MAX_THREAD) : next;
}

/** Appends entries newer than the newest one held, capped at the backend's limit. */
export function appendActivity(
  activity: readonly AgentActivity[],
  incoming: readonly AgentActivity[],
): AgentActivity[] {
  let newest = activity.length > 0 ? activity[activity.length - 1].id : -1;
  const fresh: AgentActivity[] = [];
  for (const entry of incoming) {
    if (entry.id > newest) {
      fresh.push(entry);
      newest = entry.id;
    }
  }
  if (fresh.length === 0) return activity as AgentActivity[];
  const next = [...activity, ...fresh];
  return next.length > MAX_ACTIVITY ? next.slice(-MAX_ACTIVITY) : next;
}

/** The thread one agent sees: its own items, notes sent to it, and notes to everyone. */
export function threadForSession(
  thread: readonly AgentThreadItem[],
  sessionId: string | null,
): AgentThreadItem[] {
  if (sessionId === null) return [...thread];
  return thread.filter(
    (item) =>
      item.session_id === sessionId ||
      (item.kind === "note" && item.session_id === null),
  );
}

/** Keeps only the latest item of every run of consecutive progress items. */
export function collapseProgressRuns(
  items: readonly AgentThreadItem[],
): AgentThreadItem[] {
  const out: AgentThreadItem[] = [];
  for (const item of items) {
    const previous = out[out.length - 1];
    if (
      item.kind === "progress" &&
      previous?.kind === "progress" &&
      previous.session_id === item.session_id
    ) {
      out[out.length - 1] = item;
    } else {
      out.push(item);
    }
  }
  return out;
}

const TYPING_TOOLS = new Set(["type_text", "type_by_index", "type_locator"]);

export type ActivityDetail =
  | { kind: "host"; host: string }
  | { kind: "typed"; count: number }
  | { kind: "profiles"; count: number };

export function activityDetail(entry: AgentActivity): ActivityDetail | null {
  if (entry.profile_count !== null && entry.profile_count !== undefined) {
    return { kind: "profiles", count: entry.profile_count };
  }
  const detail = entry.detail?.trim();
  if (!detail) return null;
  if (entry.tool === "navigate") return { kind: "host", host: detail };
  if (TYPING_TOOLS.has(entry.tool)) {
    const count = Number.parseInt(detail, 10);
    return Number.isFinite(count) && count >= 0
      ? { kind: "typed", count }
      : null;
  }
  return null;
}

export interface ActivityStats {
  calls: number;
  errors: number;
  /** 0 to 1; 0 when there were no calls. */
  errorRate: number;
  averageMs: number | null;
}

export function activityStats(
  activity: readonly AgentActivity[],
  now: number,
  windowMs: number = STATS_WINDOW_MS,
): ActivityStats {
  let calls = 0;
  let errors = 0;
  let total = 0;
  for (let index = activity.length - 1; index >= 0; index -= 1) {
    const entry = activity[index];
    if (now - entry.at > windowMs) break;
    calls += 1;
    if (!entry.ok) errors += 1;
    total += entry.duration_ms;
  }
  return {
    calls,
    errors,
    errorRate: calls === 0 ? 0 : errors / calls,
    averageMs: calls === 0 ? null : Math.round(total / calls),
  };
}

export type AgentProfileState = "held" | "error" | "working" | "idle";

export interface AgentProfileRow {
  profile_id: string;
  session_id: string | null;
  last: AgentActivity | null;
  hold: AgentHold | null;
  at: number;
  state: AgentProfileState;
}

/** Profiles agents touched, plus the ones the person holds, most recent first. */
export function profileRollup(
  activity: readonly AgentActivity[],
  holds: readonly AgentHold[],
  now: number,
): AgentProfileRow[] {
  const latest = new Map<string, AgentActivity>();
  for (const entry of activity) {
    if (!entry.profile_id) continue;
    const seen = latest.get(entry.profile_id);
    if (!seen || entry.at >= seen.at) latest.set(entry.profile_id, entry);
  }
  const holdsById = new Map(holds.map((hold) => [hold.profile_id, hold]));
  const ids = new Set([...latest.keys(), ...holdsById.keys()]);
  const rows: AgentProfileRow[] = [];
  for (const profileId of ids) {
    const last = latest.get(profileId) ?? null;
    const hold = holdsById.get(profileId) ?? null;
    const at = Math.max(last?.at ?? 0, hold?.since ?? 0);
    let state: AgentProfileState;
    if (hold) state = "held";
    else if (last && !last.ok) state = "error";
    else if (last && now - last.at <= ACTIVE_WINDOW_MS) state = "working";
    else state = "idle";
    rows.push({
      profile_id: profileId,
      session_id: last?.session_id ?? null,
      last,
      hold,
      at,
      state,
    });
  }
  return rows.sort((a, b) => b.at - a.at);
}

/** Calls left this hour, or null when the quota is unknown or unlimited. */
export function quotaLeft(
  quota: AutomationQuota | null,
): { left: number; limit: number } | null {
  if (!quota || quota.limit <= 0) return null;
  return { left: Math.max(0, quota.limit - quota.used), limit: quota.limit };
}

/** 0 to 100, or null when the status has no usable total. */
export function progressPercent(
  done: number | null,
  total: number | null,
): number | null {
  if (done === null || total === null || total <= 0) return null;
  return Math.min(100, Math.max(0, Math.round((done / total) * 100)));
}

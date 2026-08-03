import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Remote sessions: a profile opened on a leased Windows or macOS host.
 *
 * The desktop holds no part of the session's life. It asks the backend to
 * start or stop one and is told what happened; the fleet, the two-hour cap,
 * the profile lock and the billing all live server-side.
 */

/**
 * `provisioning` -> `ready` -> `live` -> `closed`, plus `error` for a session
 * that failed on the fleet.
 *
 * Kept open rather than a closed union: the state machine is the server's, and
 * a state added there must render as itself instead of failing to decode.
 */
export type RemoteSessionPhase = string;

export interface RemoteSessionState {
  session_id: string;
  profile_id?: string | null;
  platform?: string | null;
  /** Named to match the server's `RemoteSessionView.state`. */
  state: RemoteSessionPhase;
  /** The relay is up, so the session can actually be driven. */
  cdp_ready: boolean;
  /** `interactive` or `cookie_bot`. */
  kind?: string | null;
  /** Set when the session belongs to a cookie-bot run. */
  run_id?: string | null;
  /** The team the hours are attributed to. */
  team_id?: string | null;
  started_at?: string | null;
  /** When it finished. One timestamp, not a ready/closed pair. */
  ended_at?: string | null;
  /** Why it ended, e.g. `stopped_by_user`, `max_duration`. */
  close_reason?: string | null;
  /** What it has cost so far — a running figure while it is live. */
  billed_seconds?: number | null;
}

export interface RemoteSessionEnded {
  session_id: string;
  status: string;
  billed_seconds: number;
}

/**
 * States a session cannot leave under its own steam.
 *
 * `error` is terminal just as `closed` is, and there is never a `closed` frame
 * after one — the server keeps a fleet-reported failure visible rather than
 * flattening it into "finished". A consumer that only watched for `closed`
 * would leave a failed session pinned in its live list for ever, and the
 * profile would look busy until the app restarted.
 */
export function isSessionOver(session: RemoteSessionState): boolean {
  return session.state === "closed" || session.state === "error";
}

/** Everything the caller owns, pushed once when the stream connects. */
export interface RemoteSessionSnapshot {
  sessions: RemoteSessionState[];
}

/** Whether the desktop is currently receiving transitions. */
export interface RemoteSessionStreamStatus {
  connected: boolean;
  reason?: string | null;
}

/**
 * Tauri event names. A transition arrives here rather than being polled for:
 * `POST /api/remote-sessions` answers `provisioning` and nothing more, so
 * without the stream the desktop is blind between launch and stop.
 */
export const REMOTE_SESSION_EVENTS = {
  /** One session changed. Payload: `RemoteSessionState`. */
  state: "remote-session-state",
  /** Connect snapshot. Payload: `RemoteSessionSnapshot`. */
  snapshot: "remote-session-snapshot",
  /** Stream connectivity. Payload: `RemoteSessionStreamStatus`. */
  stream: "remote-session-stream",
} as const;

export function listRemoteSessions(): Promise<RemoteSessionState[]> {
  return invoke<RemoteSessionState[]>("list_remote_sessions");
}

export function getRemoteSession(
  sessionId: string,
): Promise<RemoteSessionState> {
  return invoke<RemoteSessionState>("get_remote_session", { sessionId });
}

export function stopRemoteSession(
  sessionId: string,
): Promise<RemoteSessionEnded> {
  return invoke<RemoteSessionEnded>("stop_remote_session", { sessionId });
}

/** Subscribe to transitions. Idempotent; call once the user is signed in. */
export function startRemoteSessionEvents(): Promise<void> {
  return invoke<void>("start_remote_session_events");
}

/** Unsubscribe. Call on sign-out. */
export function stopRemoteSessionEvents(): Promise<void> {
  return invoke<void>("stop_remote_session_events");
}

/** Whether the subscriber is alive, for a UI that mounted after it started. */
export function getRemoteSessionEventsStatus(): Promise<boolean> {
  return invoke<boolean>("get_remote_session_events_status");
}

export function onRemoteSessionState(
  handler: (session: RemoteSessionState) => void,
): Promise<UnlistenFn> {
  return listen<RemoteSessionState>(REMOTE_SESSION_EVENTS.state, (event) =>
    handler(event.payload),
  );
}

export function onRemoteSessionSnapshot(
  handler: (snapshot: RemoteSessionSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<RemoteSessionSnapshot>(
    REMOTE_SESSION_EVENTS.snapshot,
    (event) => handler(event.payload),
  );
}

export function onRemoteSessionStream(
  handler: (status: RemoteSessionStreamStatus) => void,
): Promise<UnlistenFn> {
  return listen<RemoteSessionStreamStatus>(
    REMOTE_SESSION_EVENTS.stream,
    (event) => handler(event.payload),
  );
}

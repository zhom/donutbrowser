import { invoke } from "@tauri-apps/api/core";

/**
 * Cookie bot: overnight warming of a synced profile on a leased remote host.
 *
 * Nothing about HOW the bot browses is here. The schedule, the calendar maths,
 * what a preset expands to, the site ordering, the dwell model and the pooled
 * budget are all held server-side. This module sends the user's own scalars —
 * when, how long, which of their sites, which preset id — and renders back what
 * the server reports.
 */

/** Bit 0 = Monday, bit 6 = Sunday. */
export const COOKIE_BOT_DAY_BITS = [1, 2, 4, 8, 16, 32, 64] as const;

/** Hosts the fleet can lease. Linux is refused at enrolment. */
export type CookieBotPlatform = "windows" | "macos";

/** `mine` shows the caller's enrolments, `team` the whole team's. */
export type CookieBotScope = "mine" | "team";

export interface CookieBotSchedule {
  profile_id: string;
  profile_name: string;
  platform: string;
  enabled: boolean;
  /** Minutes past local midnight the run is anchored to. */
  run_at_minute: number;
  /** Bitmask of local weekdays, bit 0 = Monday. */
  days_mask: number;
  timezone: string;
  /** Opaque server-issued preset id. */
  preset: string;
  max_minutes: number;
  sites: string[];
  jitter_seconds: number;

  // The profile facts the desktop declared, echoed back on every read, so the
  // UI can tell when the server's copy of a profile has gone stale.
  sync_enabled: boolean;
  encrypted_sync: boolean;
  has_proxy: boolean;
  touch_fingerprint: boolean;
  sticky_exit: boolean;
  /** When those facts were last refreshed. */
  profile_state_at?: string | null;

  /**
   * Why tonight would be refused, or null. One of the run outcome codes.
   *
   * The server computes this on every read so a broken enrolment is visible the
   * moment it breaks, rather than first announcing itself as a skipped run at
   * 02:00.
   */
  blocked_by?: string | null;

  next_run_at?: string | null;
  last_run_at?: string | null;
  last_run_id?: string | null;
  owner_user_id?: string | null;
  owner_email?: string | null;
  updated_at?: string | null;
}

/**
 * What the desktop sends when enrolling or editing. `next_run_at` is absent by
 * design: the server recomputes it and ignores any client value.
 */
export interface CookieBotScheduleInput {
  profile_name: string;
  platform: CookieBotPlatform;
  enabled: boolean;
  run_at_minute: number;
  days_mask: number;
  timezone: string;
  preset: string;
  max_minutes: number;
  sites: string[];
  jitter_seconds?: number;
}

export interface CookieBotScheduleList {
  schedules: CookieBotSchedule[];
  team_id?: string | null;
  scope?: string | null;
}

/** A teammate's enrolment of the same profile. */
export interface CookieBotConflict {
  user_id: string;
  email: string;
  run_at_minute: number;
  timezone: string;
  days_mask: number;
  enabled: boolean;
  /** The two enrolments share a weekday and fire within an hour. */
  overlaps: boolean;
}

export interface CookieBotScheduleSaved {
  schedule: CookieBotSchedule;
  /** Repeated on an acknowledged write, so the warning can stay on screen. */
  conflicts: CookieBotConflict[];
}

export interface CookieBotRun {
  id: string;
  profile_id: string;
  profile_name?: string | null;
  user_id?: string | null;
  email?: string | null;
  team_id?: string | null;
  /** `schedule` or `manual`. */
  trigger: string;
  /**
   * `pending` | `running` | `succeeded` | `partial` | `failed` | `skipped` |
   * `cancelled`.
   */
  status: string;
  scheduled_for: string;
  /** The jittered instant the run was allowed to start. */
  dispatch_after?: string | null;
  started_at?: string | null;
  ended_at?: string | null;
  /** The night's whole budget, which may span several browser sessions. */
  max_minutes: number;
  /** How many sessions this night is split into, and which one is running. */
  chunks_total: number;
  chunk_index: number;
  sites_total: number;
  sites_visited: number;
  sites_failed: number;
  consent_dismissed: number;
  billed_seconds: number;
  /** Why it ended the way it did, e.g. `profile_locked`, `no_capacity`. */
  outcome_code?: string | null;
  session_id?: string | null;
}

export interface CookieBotRunPage {
  runs: CookieBotRun[];
  /** Keyset cursor for the next page; null on the last one. */
  next_before?: string | null;
}

export interface CookieBotRunStarted {
  run: CookieBotRun;
  session_id?: string | null;
}

/**
 * A named intensity. The client learns only enough to label the choice and
 * show its rough cost; what it expands to is the server's.
 */
export interface CookieBotPreset {
  id: string;
  typical_minutes?: number | null;
  recommended: boolean;
  /** Server-supplied English label, so a preset newer than this build still
   * renders. Prefer a local `t()` key for a known id. */
  name?: string | null;
  description?: string | null;
}

export interface CookieBotPresetList {
  presets: CookieBotPreset[];
  default_preset?: string | null;
}

export interface RemoteHoursBreakdown {
  interactive_hours: number;
  bot_hours: number;
}

export interface RemoteHoursMember {
  user_id: string;
  email: string;
  role?: string | null;
  used_hours: number;
  interactive_hours: number;
  bot_hours: number;
}

/**
 * The single pooled remote-hour budget. Bot and interactive sessions share it;
 * the breakdown is reporting, never a sub-cap.
 */
export interface RemoteHoursQuota {
  granted_hours: number;
  remaining_hours: number;
  used_hours: number;
  period_start?: string | null;
  period_end?: string | null;
  /** `user` or `team`. */
  scope?: string | null;
  team_id?: string | null;
  seats: number;
  per_seat_hours: number;
  breakdown?: RemoteHoursBreakdown | null;
  members: RemoteHoursMember[];
}

export interface CookieBotUsageMember {
  user_id: string;
  email: string;
  role?: string | null;
  interactive_hours: number;
  bot_hours: number;
  used_hours: number;
  sessions: number;
  bot_runs: number;
  bot_runs_failed: number;
}

export interface CookieBotUsageProfile {
  profile_id: string;
  profile_name?: string | null;
  owner_email?: string | null;
  bot_hours: number;
  runs: number;
  /** How many of those runs did not do what they were asked. */
  runs_failed: number;
  last_run_at?: string | null;
  last_status?: string | null;
}

export interface CookieBotUsage {
  /** `YYYY-MM`. */
  period: string;
  period_start?: string | null;
  period_end?: string | null;
  team_id?: string | null;
  seats: number;
  granted_hours: number;
  used_hours: number;
  remaining_hours: number;
  members: CookieBotUsageMember[];
  profiles: CookieBotUsageProfile[];
}

export function getCookieBotSchedules(
  scope?: CookieBotScope,
): Promise<CookieBotScheduleList> {
  return invoke<CookieBotScheduleList>("get_cookie_bot_schedules", { scope });
}

/** `null` means the profile is not enrolled, which is a state, not a failure. */
export function getCookieBotSchedule(
  profileId: string,
): Promise<CookieBotSchedule | null> {
  return invoke<CookieBotSchedule | null>("get_cookie_bot_schedule", {
    profileId,
  });
}

/**
 * Create or replace an enrolment. A teammate's existing enrolment refuses the
 * first write with `COOKIE_BOT_SCHEDULE_CONFLICT`; repeating it with
 * `acknowledgeConflict` goes through.
 */
export function saveCookieBotSchedule(
  profileId: string,
  schedule: CookieBotScheduleInput,
  acknowledgeConflict = false,
): Promise<CookieBotScheduleSaved> {
  return invoke<CookieBotScheduleSaved>("save_cookie_bot_schedule", {
    profileId,
    schedule,
    acknowledgeConflict,
  });
}

/** `false` means there was nothing enrolled to remove. */
export function deleteCookieBotSchedule(profileId: string): Promise<boolean> {
  return invoke<boolean>("delete_cookie_bot_schedule", { profileId });
}

/** Who else already warms this profile, without writing anything. */
export function checkCookieBotConflicts(
  profileId: string,
  options: {
    runAtMinute?: number;
    timezone?: string;
    daysMask?: number;
  } = {},
): Promise<CookieBotConflict[]> {
  return invoke<CookieBotConflict[]>("check_cookie_bot_conflicts", {
    profileId,
    runAtMinute: options.runAtMinute,
    timezone: options.timezone,
    daysMask: options.daysMask,
  });
}

export function getCookieBotRuns(
  options: {
    profileId?: string;
    scope?: CookieBotScope;
    limit?: number;
    before?: string;
  } = {},
): Promise<CookieBotRunPage> {
  return invoke<CookieBotRunPage>("get_cookie_bot_runs", {
    profileId: options.profileId,
    scope: options.scope,
    limit: options.limit,
    before: options.before,
  });
}

/** Start a run now. The preset and sites come from the stored enrolment. */
export function runCookieBotNow(
  profileId: string,
  maxMinutes?: number,
): Promise<CookieBotRunStarted> {
  return invoke<CookieBotRunStarted>("run_cookie_bot_now", {
    profileId,
    maxMinutes,
  });
}

export function cancelCookieBotRun(runId: string): Promise<CookieBotRun> {
  return invoke<CookieBotRun>("cancel_cookie_bot_run", { runId });
}

export function getCookieBotPresets(): Promise<CookieBotPresetList> {
  return invoke<CookieBotPresetList>("get_cookie_bot_presets");
}

export function getRemoteHoursQuota(): Promise<RemoteHoursQuota> {
  return invoke<RemoteHoursQuota>("get_remote_hours_quota");
}

/** Per-member and per-profile spend for a calendar month (`YYYY-MM`). */
export function getCookieBotUsage(period?: string): Promise<CookieBotUsage> {
  return invoke<CookieBotUsage>("get_cookie_bot_usage", { period });
}

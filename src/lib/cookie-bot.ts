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

/**
 * What marks a `template_id` as one of the USER's own rather than a curated one.
 *
 * The two kinds share one field and behave in opposite ways — a curated
 * template's URLs are server-owned and expanded per profile at dispatch, a
 * user's are copied onto the enrolment when it is saved. An id read as the wrong
 * kind is a schedule that browses the wrong list, so every question about which
 * kind an id is goes through the helper below rather than a `startsWith` at the
 * call site.
 */
export const COOKIE_BOT_USER_TEMPLATE_PREFIX = "user:";

export function isUserTemplateId(id: string | null | undefined): boolean {
  return (
    typeof id === "string" && id.startsWith(COOKIE_BOT_USER_TEMPLATE_PREFIX)
  );
}

/** Hosts the fleet can lease. Linux is refused at enrolment. */
export type CookieBotPlatform = "windows" | "macos";

/** `mine` shows the caller's enrolments, `team` the whole team's. */
export type CookieBotScope = "mine" | "team";

/** One time-of-day an enrolment fires, on a set of local weekdays. */
export interface CookieBotSlot {
  /** Bitmask of local weekdays, bit 0 = Monday. At least one bit set. */
  days_mask: number;
  /** Minutes past local midnight, in the schedule's timezone. */
  run_at_minute: number;
}

export interface CookieBotSchedule {
  profile_id: string;
  profile_name: string;
  platform: string;
  enabled: boolean;
  /**
   * Minutes past local midnight the FIRST slot is anchored to. The server
   * mirrors `slots[0]` onto this pair on every write.
   */
  run_at_minute: number;
  /** The first slot's weekdays, bit 0 = Monday. See `run_at_minute`. */
  days_mask: number;
  /**
   * Every time-of-day this enrolment fires.
   *
   * Optional because a server that predates multi-slot scheduling sends only
   * the mirrored pair above. Read it through `scheduleSlots()` rather than
   * directly, so the fallback happens in one place instead of at each renderer
   * — an empty list here means "this server did not say", never "never fires".
   */
  slots?: CookieBotSlot[];
  timezone: string;
  /** Opaque server-issued preset id. */
  preset: string;
  /**
   * The template the site list came from, or null for the user's own list.
   *
   * A built-in id means `sites` is EMPTY on purpose: those URLs are curated
   * server-side and deliberately never sent to a client. A `user:<uuid>` id is
   * provenance — the sites were copied onto the enrolment and are present.
   */
  template_id?: string | null;
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
  /** Mirror of `slots[0]`, for a server that predates multi-slot scheduling. */
  run_at_minute: number;
  /** Mirror of `slots[0]`. See `run_at_minute`. */
  days_mask: number;
  /**
   * The whole calendar. Omit it — never send an empty array — for "one slot,
   * from the pair above": the server refuses an empty list, because a schedule
   * that fires at no time is a mistake rather than a way to pause one.
   */
  slots?: CookieBotSlot[];
  timezone: string;
  preset: string;
  /**
   * A browsing template instead of a typed site list. Mutually exclusive with a
   * non-empty `sites`: the server refuses a write carrying both.
   */
  template_id?: string;
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

/**
 * A curated browsing template: a named answer to "what is this profile for",
 * picked INSTEAD of typing a site list.
 *
 * Carries a count and never the URLs. That is the product working as designed —
 * the pool is curated server-side and each profile draws its own sample from it,
 * so the template never becomes one recognisable fleet-wide set of visits. Any
 * copy describing this must say so as the feature it is.
 */
export interface CookieBotTemplate {
  id: string;
  /** How many sites this template browses. Not which. */
  site_count: number;
  /** Server-supplied English fallbacks, for a template newer than this build. */
  name?: string | null;
  description?: string | null;
}

/**
 * The server's own form bounds, when it publishes them.
 *
 * Every field is optional: a deployment that predates this object sends none of
 * them, and treating a missing bound as `0` would refuse every value the form
 * can produce. `SCHEDULE_BOUNDS` in `cookie-bot-limits.ts` is the fallback.
 */
export interface CookieBotLimits {
  min_minutes?: number | null;
  max_minutes?: number | null;
  min_sites?: number | null;
  max_sites?: number | null;
  /** Most entries a calendar may carry. */
  max_slots?: number | null;
  /** Longest name a saved site list may be given. */
  max_template_name_length?: number | null;
}

export interface CookieBotPresetList {
  presets: CookieBotPreset[];
  default_preset?: string | null;
  /**
   * The curated templates on offer. Served with the presets so one added
   * server-side becomes selectable without a desktop release.
   */
  templates?: CookieBotTemplate[];
  limits?: CookieBotLimits | null;
}

/**
 * One of the caller's OWN saved site lists.
 *
 * Carries its URLs, unlike {@link CookieBotTemplate}: they are the user's own
 * and there is nothing to withhold. Applying one COPIES the sites onto the
 * enrolment, so editing a list later does not silently change what an existing
 * schedule browses.
 */
export interface CookieBotUserTemplate {
  /**
   * Already prefixed `user:<uuid>` — the value `template_id` takes verbatim.
   * Nothing on this side assembles that convention.
   */
  id: string;
  name: string;
  sites: string[];
  updated_at?: string | null;
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

/** Every site list this user has saved, most recently edited first. */
export function getCookieBotUserTemplates(): Promise<CookieBotUserTemplate[]> {
  return invoke<CookieBotUserTemplate[]>("get_cookie_bot_user_templates");
}

/** Save the current site list under a name. */
export function createCookieBotUserTemplate(
  name: string,
  sites: string[],
): Promise<CookieBotUserTemplate> {
  return invoke<CookieBotUserTemplate>("create_cookie_bot_user_template", {
    name,
    sites,
  });
}

/**
 * Rename a saved list, replace its sites, or both.
 *
 * Send only what changed. A rename that also carried the site list would
 * silently revert an edit made to it from another device in between.
 */
export function updateCookieBotUserTemplate(
  id: string,
  changes: { name?: string; sites?: string[] },
): Promise<CookieBotUserTemplate> {
  return invoke<CookieBotUserTemplate>("update_cookie_bot_user_template", {
    id,
    name: changes.name,
    sites: changes.sites,
  });
}

/**
 * Delete a saved list. `false` means there was nothing left to delete, which is
 * a success — enrolments that used it keep the sites they copied either way.
 */
export function deleteCookieBotUserTemplate(id: string): Promise<boolean> {
  return invoke<boolean>("delete_cookie_bot_user_template", { id });
}

export function getRemoteHoursQuota(): Promise<RemoteHoursQuota> {
  return invoke<RemoteHoursQuota>("get_remote_hours_quota");
}

/** Per-member and per-profile spend for a calendar month (`YYYY-MM`). */
export function getCookieBotUsage(period?: string): Promise<CookieBotUsage> {
  return invoke<CookieBotUsage>("get_cookie_bot_usage", { period });
}

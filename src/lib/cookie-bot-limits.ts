/**
 * The server's schedule bounds, mirrored for form validation.
 *
 * These numbers are NOT the client's to choose. They belong to
 * `validateScheduleBody` and `normaliseSites` in donutbrowser-infra's
 * `apps/backend/src/cookie-bot/cookie-bot.service.ts`, which refuses anything
 * outside them with `COOKIE_BOT_INVALID_SCHEDULE` or `COOKIE_BOT_SITE_LIMIT`.
 * They are mirrored here only so the enrolment form can refuse a value before
 * it costs a round trip, and so the reason lands on the field rather than in a
 * toast that names no field at all.
 *
 * Kept in a module of their own, with no imports, so `cookie-bot-limits.test.mjs`
 * can pin them. A bound widened server-side (`max_minutes` to 180, sites capped
 * at 25) then shows up as a failing assertion instead of as a form that blocks
 * a legal value or accepts an illegal one.
 *
 * @see cookie-bot-limits.test.mjs — the tripwire.
 */
export const SCHEDULE_BOUNDS = {
  /** `MIN_MAX_MINUTES` in cookie-bot-schedule.ts. */
  minMaxMinutes: 5,
  /** `MAX_MAX_MINUTES` in cookie-bot-schedule.ts. */
  maxMaxMinutes: 120,
  /**
   * v1 browses the user's declared sites and nothing else, so an enrolment
   * with none is one the server cannot act on. `normaliseSites` rejects an
   * empty list.
   */
  minSites: 1,
  /** `MAX_SITES` in cookie-bot-schedule.ts. */
  maxSites: 40,
} as const;

/**
 * The longest command the harness ever waits on: `download_browser` pulling a
 * published Wayfern build of about 1 GB, which a slow link needs the better
 * part of half an hour for.
 *
 * Every clock around that command is derived from this one number so they can
 * never disagree again. The session script timeout is this value; the client
 * gives up a little later; the driver's outer per-command bound
 * (`--command-timeout`) later still. Ordered that way, a download that is
 * genuinely too slow surfaces as the driver's own script-timeout error rather
 * than as a torn connection somewhere in between.
 */
export const WAYFERN_DOWNLOAD_TIMEOUT_MS = 30 * 60 * 1000;

/** How long the client waits on a download command before it gives up. */
export const WAYFERN_DOWNLOAD_CLIENT_TIMEOUT_MS =
  WAYFERN_DOWNLOAD_TIMEOUT_MS + 20_000;

/** The driver's outer per-command bound, in the whole seconds its flag takes. */
export const DRIVER_COMMAND_TIMEOUT_SECONDS =
  Math.ceil(WAYFERN_DOWNLOAD_TIMEOUT_MS / 1000) + 60;

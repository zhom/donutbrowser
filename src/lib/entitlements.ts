import type { CloudUser, Entitlements } from "@/types";

const DEFAULT_REQUESTS_PER_HOUR = 100;

interface Capabilities {
  browserAutomation: boolean;
  crossOsFingerprints: boolean;
  cloudBackup: boolean;
  teamCollaboration: boolean;
  cookieBot: boolean;
}

const NONE: Entitlements = {
  active: false,
  browserAutomation: false,
  crossOsFingerprints: false,
  cloudBackup: false,
  teamCollaboration: false,
  cookieBot: false,
  profileLimit: 0,
  requestsPerHour: 0,
  remoteBrowserHours: 0,
};

// Mirror of PLAN_CAPABILITIES in apps/backend/src/plans/entitlements.ts. Keep in
// sync — a new plan must be declared here too, or it falls back to DEFAULT_PAID.
const PLAN_CAPABILITIES: Record<string, Capabilities> = {
  starter: {
    browserAutomation: false,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: false,
    cookieBot: false,
  },
  pro: {
    browserAutomation: true,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: false,
    cookieBot: true,
  },
  team: {
    browserAutomation: true,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: true,
    cookieBot: true,
  },
  enterprise: {
    browserAutomation: true,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: true,
    cookieBot: true,
  },
};

// Unknown paid plan -> pro-level (never team), matching the backend default.
const DEFAULT_PAID: Capabilities = {
  browserAutomation: true,
  crossOsFingerprints: true,
  cloudBackup: true,
  teamCollaboration: false,
  cookieBot: true,
};

/**
 * The user's effective entitlements. Prefers the backend-resolved object the
 * desktop attaches to CloudUser; only falls back to deriving from the plan
 * fields when it's missing (older cached state). The fallback mirrors the
 * backend matrix in `apps/backend/src/plans/entitlements.ts`.
 */
export function getEntitlements(
  user: CloudUser | null | undefined,
): Entitlements {
  if (user?.entitlements) {
    const server = user.entitlements;
    // A backend (or a cached login) older than the cookie-bot release omits
    // these two keys. Reading them as `undefined` would hide a paid feature
    // from a paying customer with nothing logged anywhere, so resolve them
    // here — the one place every caller already goes through. Cookie Bot is
    // remote automation on leased hardware, so it tracks `browserAutomation`
    // exactly; `remoteBrowserHours` stays 0 because the spendable figure is
    // whatever `get_remote_hours_quota` reports, never a client guess.
    return {
      ...server,
      cookieBot: server.cookieBot ?? server.browserAutomation,
      remoteBrowserHours: server.remoteBrowserHours ?? 0,
    };
  }
  if (!user) return NONE;

  const active =
    user.plan !== "free" &&
    (user.subscriptionStatus === "active" || user.planPeriod === "lifetime");
  if (!active) return NONE;

  const caps = PLAN_CAPABILITIES[user.plan] ?? DEFAULT_PAID;
  return {
    active: true,
    browserAutomation: caps.browserAutomation,
    crossOsFingerprints: caps.crossOsFingerprints,
    cloudBackup: caps.cloudBackup,
    teamCollaboration: caps.teamCollaboration,
    cookieBot: caps.cookieBot,
    profileLimit: user.profileLimit,
    requestsPerHour: caps.browserAutomation ? DEFAULT_REQUESTS_PER_HOUR : 0,
    remoteBrowserHours: 0,
  };
}

/**
 * Whether this user may enrol profiles in Cookie Bot. Every gate in the UI
 * goes through here so a plan change is one edit, and so the Pro badge and the
 * control it guards can never disagree.
 */
export function canUseCookieBot(user: CloudUser | null | undefined): boolean {
  const entitlements = getEntitlements(user);
  return entitlements.active && entitlements.cookieBot;
}

/**
 * Only a team owner sees per-member attribution. An admin can change team
 * settings but the pooled spend is the owner's bill.
 */
export function isTeamOwner(user: CloudUser | null | undefined): boolean {
  return (
    getEntitlements(user).teamCollaboration &&
    user?.teamRole === "owner" &&
    Boolean(user.teamId)
  );
}

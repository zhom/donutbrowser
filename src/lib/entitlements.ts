import type { CloudUser, Entitlements } from "@/types";

const DEFAULT_REQUESTS_PER_HOUR = 100;

interface Capabilities {
  browserAutomation: boolean;
  crossOsFingerprints: boolean;
  cloudBackup: boolean;
  teamCollaboration: boolean;
  cookieBot: boolean;
  remoteInteractive: boolean;
  remoteControl: boolean;
  agentAutomation: boolean;
}

const NONE: Entitlements = {
  active: false,
  browserAutomation: false,
  crossOsFingerprints: false,
  cloudBackup: false,
  teamCollaboration: false,
  cookieBot: false,
  remoteInteractive: false,
  remoteControl: false,
  agentAutomation: false,
  profileLimit: 0,
  requestsPerHour: 0,
  remoteBrowserHours: 0,
};

// Mirror of the plan capability matrix the API resolves. Keep in sync — a new
// plan must be declared here too, or it falls back to DEFAULT_PAID.
const PLAN_CAPABILITIES: Record<string, Capabilities> = {
  // The one row where cookieBot, browserAutomation and remoteInteractive all
  // disagree: solo pays for a nightly bot and nothing else that drives a
  // browser. No fingerprint editing either.
  solo: {
    browserAutomation: false,
    crossOsFingerprints: false,
    cloudBackup: true,
    teamCollaboration: false,
    cookieBot: true,
    remoteInteractive: false,
    remoteControl: false,
    agentAutomation: false,
  },
  pro: {
    browserAutomation: true,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: false,
    cookieBot: true,
    remoteInteractive: true,
    remoteControl: false,
    agentAutomation: true,
  },
  team: {
    browserAutomation: true,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: true,
    cookieBot: true,
    remoteInteractive: true,
    remoteControl: false,
    agentAutomation: true,
  },
  // The only tier that may drive this desktop from donutbrowser.com.
  enterprise: {
    browserAutomation: true,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: true,
    cookieBot: true,
    remoteInteractive: true,
    remoteControl: true,
    agentAutomation: true,
  },
};

// Unknown paid plan -> pro-level (never team), the conservative reading.
// remoteControl is the one exception and is withheld: nobody is paying for a
// capability that has no price, and an unrecognised plan string must not open
// an internet-facing hook into this machine.
const DEFAULT_PAID: Capabilities = {
  browserAutomation: true,
  crossOsFingerprints: true,
  cloudBackup: true,
  teamCollaboration: false,
  cookieBot: true,
  remoteInteractive: true,
  remoteControl: false,
  agentAutomation: true,
};

/**
 * The user's effective entitlements. Prefers the backend-resolved object the
 * desktop attaches to CloudUser; only falls back to deriving from the plan
 * fields when it's missing (older cached state). The fallback mirrors the
 * capability matrix the API resolves.
 */
export function getEntitlements(
  user: CloudUser | null | undefined,
): Entitlements {
  if (user?.entitlements) {
    const server = user.entitlements;
    // A backend (or a cached login) older than the current release omits these
    // keys. Reading them as `undefined` would hide a paid feature from a paying
    // customer with nothing logged anywhere, so resolve them here — the one
    // place every caller already goes through.
    //
    // Both absent flags fall back to `browserAutomation`, which is what they
    // were derived from before solo existed: on every plan a pre-solo backend
    // knows about, automation implied both the bot and interactive remote
    // control. A solo user never hits this branch — the backend that can put
    // them on solo is by definition new enough to send both keys.
    //
    // `remoteBrowserHours` stays 0 because the spendable figure is whatever
    // `get_remote_hours_quota` reports, never a client guess.
    return {
      ...server,
      cookieBot: server.cookieBot ?? server.browserAutomation,
      remoteInteractive: server.remoteInteractive ?? server.browserAutomation,
      // Not back-filled from anything. A backend too old to send this key is a
      // backend with no remote-control endpoint to be entitled to, so `false`
      // is the true answer rather than a conservative guess.
      remoteControl: server.remoteControl ?? false,
      // Not back-filled either, and for the same reason: a backend that omits
      // this key serves no `api/agent` routes, so reading it as `false` is the
      // truth rather than a conservative guess.
      agentAutomation: server.agentAutomation ?? false,
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
    remoteInteractive: caps.remoteInteractive,
    remoteControl: caps.remoteControl,
    agentAutomation: caps.agentAutomation,
    profileLimit: user.profileLimit,
    requestsPerHour: caps.browserAutomation ? DEFAULT_REQUESTS_PER_HOUR : 0,
    remoteBrowserHours: 0,
  };
}

/**
 * The plan this account is served under. A team member's own `plan` is
 * `"free"` (the owner pays) while the backend resolves `effectivePlan` to the
 * owner's tier. The login response and an older backend omit the key, and then
 * the row's own plan is the only answer there is. Every "what plan is this?"
 * label or gate goes through here; billing surfaces keep reading `plan`,
 * because a seat has no subscription of its own.
 */
export function effectivePlanOf(user: CloudUser | null | undefined): string {
  return user?.effectivePlan ?? user?.plan ?? "free";
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
 * Whether this user may drive this desktop from donutbrowser.com.
 *
 * The bridge itself does not read this, the server decides who may send work,
 * and a cached entitlement that is a refresh cycle out of date must not be what
 * refuses a customer their own machine. This is what the UI reads to explain
 * WHY a connected desktop cannot be driven.
 */
export function canUseRemoteControl(
  user: CloudUser | null | undefined,
): boolean {
  const entitlements = getEntitlements(user);
  return entitlements.active && entitlements.remoteControl;
}

/**
 * Whether this user may start agent runs.
 *
 * The page reads this to decide between the run form and an honest explanation
 * of what is missing; it never invents a gate of its own. The API is the real
 * authority and answers `AGENT_NOT_ENTITLED` regardless, so a cached
 * entitlement that is one refresh out of date costs a translated refusal, not a
 * hidden feature.
 */
export function canUseAgentAutomation(
  user: CloudUser | null | undefined,
): boolean {
  const entitlements = getEntitlements(user);
  return entitlements.active && entitlements.agentAutomation;
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

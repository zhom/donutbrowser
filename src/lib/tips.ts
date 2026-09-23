import type { AppPage } from "@/components/rail-nav";
import type { BrowserProfile, Entitlements } from "@/types";

/**
 * The plan capabilities a tip can be gated on. A tip that names one of these
 * is offered only when the signed-in plan grants it, so nobody is walked
 * through a feature they cannot open.
 */
export type TipRequirement = Extract<
  keyof Entitlements,
  | "cloudBackup"
  | "cookieBot"
  | "crossOsFingerprints"
  | "browserAutomation"
  | "agentAutomation"
  | "teamCollaboration"
  | "remoteControl"
>;

/** Where a tip's action button takes the user. */
export type TipAction =
  | { kind: "page"; page: AppPage }
  | { kind: "settings"; section: string }
  | { kind: "palette" };

export type TipId =
  | "dnsBlocklist"
  | "proxyCheck"
  | "groups"
  | "commandPalette"
  | "fingerprintGate"
  | "profilePassword"
  | "clearOnClose"
  | "defaultBrowser"
  | "extensionGroups"
  | "selfHostedSync"
  | "trash"
  | "localApi"
  | "importProfiles"
  | "cloudBackup"
  | "cookieBot"
  | "crossOs"
  | "automation"
  | "agent"
  | "team"
  | "remoteControl";

/** The profile fields the relevance checks read. */
export type TipProfile = Pick<
  BrowserProfile,
  | "dns_blocklist"
  | "password_protected"
  | "clear_on_close"
  | "group_id"
  | "extension_group_id"
  | "sync_mode"
  | "host_os"
> & { wayfern_config?: { os?: string } };

/**
 * What this install already does, as far as the tips care. The first group
 * is data the app has loaded anyway; the optional facts are read from the
 * backend only when a tip that needs them is a candidate, and a fact that is
 * missing (not read, or the read failed) counts as "not in use".
 */
export interface FeatureUsage {
  profiles: readonly TipProfile[];
  /** The OS of this desktop, to tell a cross-OS fingerprint from a native one. */
  currentOs: string;
  groupCount: number;
  extensionGroupCount: number;
  inTeam: boolean;
  cookieBotEnrolled: boolean;
  syncServerConfigured: boolean;
  apiEnabled?: boolean;
  mcpEnabled?: boolean;
  remoteControlEnabled?: boolean;
  /** The user turned the pre-launch fingerprint gate off, so they know it. */
  fingerprintGateChanged?: boolean;
  proxyChecked?: boolean;
  isDefaultBrowser?: boolean;
  trashUsed?: boolean;
  agentUsed?: boolean;
}

export interface TipDefinition {
  id: TipId;
  action: TipAction;
  requires?: TipRequirement;
  /**
   * True when the user already uses the feature, so the automatic flow skips
   * the tip. Absent when nothing on this install tells: the command palette
   * and import leave no trace.
   */
  inUse?: (usage: FeatureUsage) => boolean;
}

function anyProfile(
  usage: FeatureUsage,
  test: (profile: TipProfile) => boolean,
): boolean {
  return usage.profiles.some(test);
}

function syncsProfiles(usage: FeatureUsage): boolean {
  return anyProfile(
    usage,
    (profile) => profile.sync_mode != null && profile.sync_mode !== "Disabled",
  );
}

/**
 * Every tip, in the order the catalog lists them. The essentials come first
 * because they apply to every install; the plan tips follow, and each one is
 * skipped for a plan that lacks the capability. The automatic flow does not
 * follow this order: it picks at random (see `pickAutoTip`).
 *
 * The copy lives under `tips.items.<id>` in every locale: `label`, `title`,
 * `body` and `action`. `src/lib/tips.test.mjs` checks that no tip is missing its text.
 */
export const TIPS: readonly TipDefinition[] = [
  {
    id: "dnsBlocklist",
    action: { kind: "settings", section: "dns" },
    inUse: (usage) => anyProfile(usage, (profile) => !!profile.dns_blocklist),
  },
  {
    id: "proxyCheck",
    action: { kind: "page", page: "proxies" },
    inUse: (usage) => usage.proxyChecked === true,
  },
  {
    id: "groups",
    action: { kind: "page", page: "groups" },
    inUse: (usage) =>
      usage.groupCount > 0 ||
      anyProfile(usage, (profile) => !!profile.group_id),
  },
  { id: "commandPalette", action: { kind: "palette" } },
  {
    id: "fingerprintGate",
    action: { kind: "settings", section: "advanced" },
    inUse: (usage) => usage.fingerprintGateChanged === true,
  },
  {
    id: "profilePassword",
    action: { kind: "page", page: "profiles" },
    inUse: (usage) =>
      anyProfile(usage, (profile) => profile.password_protected === true),
  },
  {
    id: "clearOnClose",
    action: { kind: "page", page: "profiles" },
    inUse: (usage) =>
      anyProfile(usage, (profile) => profile.clear_on_close === true),
  },
  {
    id: "defaultBrowser",
    action: { kind: "settings", section: "default" },
    inUse: (usage) => usage.isDefaultBrowser === true,
  },
  {
    id: "extensionGroups",
    action: { kind: "page", page: "extensions" },
    inUse: (usage) =>
      usage.extensionGroupCount > 0 ||
      anyProfile(usage, (profile) => !!profile.extension_group_id),
  },
  {
    id: "selfHostedSync",
    action: { kind: "page", page: "account" },
    inUse: (usage) => usage.syncServerConfigured || syncsProfiles(usage),
  },
  {
    id: "trash",
    action: { kind: "page", page: "trash" },
    inUse: (usage) => usage.trashUsed === true,
  },
  {
    id: "localApi",
    action: { kind: "page", page: "integrations" },
    inUse: (usage) => usage.apiEnabled === true || usage.mcpEnabled === true,
  },
  { id: "importProfiles", action: { kind: "page", page: "import" } },
  {
    id: "cloudBackup",
    action: { kind: "page", page: "account" },
    requires: "cloudBackup",
    inUse: syncsProfiles,
  },
  {
    id: "cookieBot",
    action: { kind: "page", page: "cookieBot" },
    requires: "cookieBot",
    inUse: (usage) => usage.cookieBotEnrolled,
  },
  {
    id: "crossOs",
    action: { kind: "page", page: "profiles" },
    requires: "crossOsFingerprints",
    inUse: (usage) =>
      anyProfile(usage, (profile) => {
        const reported = profile.wayfern_config?.os;
        const native = profile.host_os ?? usage.currentOs;
        return !!reported && reported !== native;
      }),
  },
  {
    id: "automation",
    action: { kind: "page", page: "integrations" },
    requires: "browserAutomation",
    inUse: (usage) => usage.apiEnabled === true || usage.mcpEnabled === true,
  },
  {
    id: "agent",
    action: { kind: "page", page: "agent" },
    requires: "agentAutomation",
    inUse: (usage) => usage.agentUsed === true,
  },
  {
    id: "team",
    action: { kind: "page", page: "account" },
    requires: "teamCollaboration",
    inUse: (usage) => usage.inTeam,
  },
  {
    id: "remoteControl",
    action: { kind: "page", page: "integrations" },
    requires: "remoteControl",
    inUse: (usage) => usage.remoteControlEnabled === true,
  },
];

/** How long after the app settles the automatic tip waits before opening. */
export const TIP_AUTO_DELAY_MS = 2500;

/**
 * A sign-in younger than this counts as "just came back from the website",
 * which is when a paid account seen for the first time gets its welcome.
 */
export const FRESH_LOGIN_WINDOW_MS = 15 * 60 * 1000;

export function isPlanTip(tip: TipDefinition): boolean {
  return tip.requires !== undefined;
}

/** The tips this plan may see: every essential, plus the plan tips it unlocks. */
export function tipsFor(
  entitlements: Pick<Entitlements, "active" | TipRequirement>,
): TipDefinition[] {
  return TIPS.filter(
    (tip) =>
      tip.requires === undefined ||
      (entitlements.active && entitlements[tip.requires]),
  );
}

/** The tips the automatic flow may still open: unseen, and not in use. */
export function autoTipCandidates(
  tips: readonly TipDefinition[],
  seen: readonly string[],
  usage: FeatureUsage,
): TipDefinition[] {
  return tips.filter(
    (tip) => !seen.includes(tip.id) && !(tip.inUse?.(usage) ?? false),
  );
}

/**
 * The one tip the automatic flow opens: a random unseen tip for a feature
 * the user does not use yet, or null when none is left. `random` returns a
 * number in [0, 1), like `Math.random`.
 */
export function pickAutoTip(
  tips: readonly TipDefinition[],
  seen: readonly string[],
  usage: FeatureUsage,
  random: () => number = Math.random,
): TipDefinition | null {
  const candidates = autoTipCandidates(tips, seen, usage);
  if (candidates.length === 0) return null;
  const index = Math.min(
    candidates.length - 1,
    Math.floor(random() * candidates.length),
  );
  return candidates[index];
}

export function tipTextKeys(id: TipId): {
  /** The short name the catalog lists the tip under. */
  label: string;
  title: string;
  body: string;
  action: string;
} {
  return {
    label: `tips.items.${id}.label`,
    title: `tips.items.${id}.title`,
    body: `tips.items.${id}.body`,
    action: `tips.items.${id}.action`,
  };
}

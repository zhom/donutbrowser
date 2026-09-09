import type { AppPage } from "@/components/rail-nav";
import type { Entitlements } from "@/types";

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

export interface TipDefinition {
  id: TipId;
  action: TipAction;
  requires?: TipRequirement;
}

/**
 * Every tip, in the order the automatic flow offers them. The essentials come
 * first because they apply to every install; the plan tips follow, and each
 * one is skipped for a plan that lacks the capability.
 *
 * The copy lives under `tips.items.<id>` in every locale: `label`, `title`,
 * `body` and `action`. `src/lib/tips.test.mjs` checks that no tip is missing its text.
 */
export const TIPS: readonly TipDefinition[] = [
  { id: "dnsBlocklist", action: { kind: "settings", section: "dns" } },
  { id: "proxyCheck", action: { kind: "page", page: "proxies" } },
  { id: "groups", action: { kind: "page", page: "groups" } },
  { id: "commandPalette", action: { kind: "palette" } },
  {
    id: "fingerprintGate",
    action: { kind: "settings", section: "advanced" },
  },
  { id: "profilePassword", action: { kind: "page", page: "profiles" } },
  { id: "clearOnClose", action: { kind: "page", page: "profiles" } },
  { id: "defaultBrowser", action: { kind: "settings", section: "default" } },
  { id: "extensionGroups", action: { kind: "page", page: "extensions" } },
  { id: "selfHostedSync", action: { kind: "page", page: "account" } },
  { id: "trash", action: { kind: "page", page: "trash" } },
  { id: "localApi", action: { kind: "page", page: "integrations" } },
  { id: "importProfiles", action: { kind: "page", page: "import" } },
  {
    id: "cloudBackup",
    action: { kind: "page", page: "account" },
    requires: "cloudBackup",
  },
  {
    id: "cookieBot",
    action: { kind: "page", page: "cookieBot" },
    requires: "cookieBot",
  },
  {
    id: "crossOs",
    action: { kind: "page", page: "profiles" },
    requires: "crossOsFingerprints",
  },
  {
    id: "automation",
    action: { kind: "page", page: "integrations" },
    requires: "browserAutomation",
  },
  {
    id: "agent",
    action: { kind: "page", page: "agent" },
    requires: "agentAutomation",
  },
  {
    id: "team",
    action: { kind: "page", page: "account" },
    requires: "teamCollaboration",
  },
  {
    id: "remoteControl",
    action: { kind: "page", page: "integrations" },
    requires: "remoteControl",
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

/** The first tip the user has not seen yet, or null when they have seen them all. */
export function pickAutoTip(
  tips: readonly TipDefinition[],
  seen: readonly string[],
): TipDefinition | null {
  return tips.find((tip) => !seen.includes(tip.id)) ?? null;
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

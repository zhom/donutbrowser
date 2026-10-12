import { invoke } from "@tauri-apps/api/core";

/** One line of the team's log, as the cloud returns it. */
export interface TeamActivityItem {
  id: number;
  at: string;
  actor: { userId: string | null; email: string | null; kind: string };
  action: string;
  target: { type: string | null; id: string | null; name: string | null };
  detail: Record<string, unknown> | null;
  source: "server" | "client";
}

export interface TeamActivityPage {
  items: TeamActivityItem[];
  nextCursor: string | null;
}

/** The actions this build has words for, by the key under `teamActivity.actions`. */
const ACTION_KEYS: Record<string, string> = {
  "member.invite": "memberInvite",
  "member.join": "memberJoin",
  "member.remove": "memberRemove",
  "member.role_change": "memberRoleChange",
  "invitation.cancel": "invitationCancel",
  "profile.launch": "profileLaunch",
  "profile.stop": "profileStop",
  "profile.create": "profileCreate",
  "profile.rename": "profileRename",
  "profile.update": "profileUpdate",
  "profile.edit": "profileUpdate",
  "profile.bulk_update": "profileBulkUpdate",
  "profile.delete": "profileDelete",
  "profile.restore": "profileRestore",
  "profile.sync_on": "profileSyncOn",
  "profile.sync_off": "profileSyncOff",
  "proxy.create": "proxyCreate",
  "proxy.update": "proxyUpdate",
  "proxy.delete": "proxyDelete",
  "group.create": "groupCreate",
  "group.update": "groupUpdate",
  "group.delete": "groupDelete",
  "vpn.create": "vpnCreate",
  "vpn.update": "vpnUpdate",
  "vpn.delete": "vpnDelete",
  "extension.add": "extensionAdd",
  "extension.update": "extensionUpdate",
  "extension.delete": "extensionDelete",
  "extension.group_create": "extensionGroupCreate",
  "extension.group_update": "extensionGroupUpdate",
  "extension.group_delete": "extensionGroupDelete",
  "template.create": "templateCreate",
  "template.update": "templateUpdate",
  "template.delete": "templateDelete",
  "status.create": "statusCreate",
  "status.update": "statusUpdate",
  "status.delete": "statusDelete",
};

/** The fields a desktop names in `detail.fields`, by their key under `teamActivity.fields`. */
const FIELD_KEYS: Record<string, string> = {
  name: "name",
  group: "group",
  tags: "tags",
  note: "note",
  clear_on_close: "clearOnClose",
  window_color: "windowColor",
  launch_hook: "launchHook",
  proxy_bypass_rules: "proxyBypassRules",
  dns_blocklist: "dnsBlocklist",
  dns_server: "dnsServer",
  start_urls: "startUrls",
  status: "status",
  fingerprint: "fingerprint",
  proxy: "proxy",
  vpn: "vpn",
  extension_group: "extensionGroup",
  bookmarks: "bookmarks",
  extensions: "extensions",
};

const ACTOR_KINDS = new Set([
  "desktop",
  "web",
  "mcp",
  "service",
  "remote_vm",
  "system",
]);

/** The translation key for `action`, or null for one this build does not know. */
export function actionKey(action: string): string | null {
  return ACTION_KEYS[action] ?? null;
}

/** The translation keys of the fields an edit names, in order, known ones only. */
export function fieldKeys(detail: TeamActivityItem["detail"]): string[] {
  const fields = detail?.fields;
  if (!Array.isArray(fields)) return [];
  return fields.flatMap((field) =>
    typeof field === "string" && FIELD_KEYS[field] ? [FIELD_KEYS[field]] : [],
  );
}

/** The actor kind to name, or null when the line needs none. */
export function actorKind(kind: string): string | null {
  return ACTOR_KINDS.has(kind) ? kind : null;
}

/**
 * What to call the target: the name this computer knows it by, else the name
 * in the log, else a short id. Null when the action has no target.
 */
export function targetLabel(
  target: TeamActivityItem["target"],
  localNames: ReadonlyMap<string, string>,
): string | null {
  if (!target.id) return target.name;
  return localNames.get(target.id) ?? target.name ?? shortId(target.id);
}

function shortId(id: string): string {
  return id.length > 8 ? `${id.slice(0, 8)}…` : id;
}

export const ACTIVITY_PAGE = 50;

export function listTeamActivity(
  cursor: string | null,
): Promise<TeamActivityPage> {
  return invoke<TeamActivityPage>("list_team_activity", {
    cursor,
    limit: ACTIVITY_PAGE,
  });
}

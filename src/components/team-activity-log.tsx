"use client";

import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { translateBackendError } from "@/lib/backend-errors";
import {
  ACTIVITY_PERIODS,
  ACTIVITY_TARGET_TYPES,
  type ActivityFilter,
  type ActivityPeriod,
  actionKey,
  actorKind,
  exportTeamActivityCsv,
  fieldKeys,
  listTeamActivity,
  listTeamMembers,
  sinceFor,
  type TeamActivityItem,
  type TeamMember,
  targetLabel,
} from "@/lib/team-activity";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

/** The value a filter Select holds for "no filter". */
const ANY = "any";

interface Named {
  id: string;
  name: string;
}

/**
 * The names this computer has for the team's items. The log may carry ids
 * only (the team's data is end-to-end encrypted), and a local name is also
 * newer than the one written at the time.
 */
async function loadLocalNames(): Promise<Map<string, string>> {
  const lists = await Promise.allSettled([
    invoke<Named[]>("list_browser_profiles"),
    invoke<Named[]>("get_stored_proxies"),
    invoke<Named[]>("get_profile_groups"),
    invoke<Named[]>("list_vpn_configs"),
    invoke<Named[]>("list_extensions"),
    invoke<Named[]>("list_extension_groups"),
    invoke<Named[]>("list_profile_templates"),
    invoke<Named[]>("list_profile_statuses"),
  ]);
  const names = new Map<string, string>();
  for (const list of lists) {
    if (list.status !== "fulfilled" || !Array.isArray(list.value)) continue;
    for (const item of list.value) {
      if (item?.id && item.name) names.set(item.id, item.name);
    }
  }
  return names;
}

export function TeamActivityLog() {
  const { t, i18n } = useTranslation();
  const [items, setItems] = React.useState<TeamActivityItem[]>([]);
  const [nextCursor, setNextCursor] = React.useState<string | null>(null);
  const [localNames, setLocalNames] = React.useState<Map<string, string>>(
    () => new Map(),
  );
  const [isLoading, setIsLoading] = React.useState(true);
  const [isLoadingMore, setIsLoadingMore] = React.useState(false);
  const [isExporting, setIsExporting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [members, setMembers] = React.useState<TeamMember[]>([]);
  const [actor, setActor] = React.useState(ANY);
  const [targetType, setTargetType] = React.useState(ANY);
  const [period, setPeriod] = React.useState<ActivityPeriod>("all");

  const filter = React.useMemo<ActivityFilter>(
    () => ({
      actorUserId: actor === ANY ? undefined : actor,
      targetType: targetType === ANY ? undefined : targetType,
      since: sinceFor(period),
    }),
    [actor, targetType, period],
  );

  React.useEffect(() => {
    void listTeamMembers()
      .then(setMembers)
      .catch(() => setMembers([]));
  }, []);

  const load = React.useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const [page, names] = await Promise.all([
        listTeamActivity(null, filter),
        loadLocalNames(),
      ]);
      setItems(page.items);
      setNextCursor(page.nextCursor);
      setLocalNames(names);
    } catch (err) {
      setError(translateBackendError(t, err));
    } finally {
      setIsLoading(false);
    }
  }, [t, filter]);

  React.useEffect(() => {
    void load();
  }, [load]);

  const exportCsv = async () => {
    const path = await save({
      defaultPath: `team-activity-${new Date().toISOString().slice(0, 10)}.csv`,
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (!path) return;
    setIsExporting(true);
    try {
      const rows = await exportTeamActivityCsv(path, filter);
      showSuccessToast(t("teamActivity.exported", { count: rows }));
    } catch (err) {
      showErrorToast(translateBackendError(t, err));
    } finally {
      setIsExporting(false);
    }
  };

  const loadMore = async () => {
    if (!nextCursor) return;
    setIsLoadingMore(true);
    try {
      const page = await listTeamActivity(nextCursor, filter);
      setItems((current) => [...current, ...page.items]);
      setNextCursor(page.nextCursor);
    } catch (err) {
      setError(translateBackendError(t, err));
    } finally {
      setIsLoadingMore(false);
    }
  };

  const formatAt = (at: string) => {
    const date = new Date(at);
    return Number.isNaN(date.getTime())
      ? at
      : date.toLocaleString(i18n.language, {
          dateStyle: "medium",
          timeStyle: "short",
        });
  };

  const roleLabel = (role: string) =>
    role === "owner"
      ? t("sync.team.roleOwner")
      : role === "admin"
        ? t("sync.team.roleAdmin")
        : role === "launcher"
          ? t("sync.team.roleLauncher")
          : t("sync.team.roleMember");

  /** "Member → Launcher" for a role change, or the groups an access change gave. */
  const memberChange = (item: TeamActivityItem) => {
    const { from, to, groups } = item.detail ?? {};
    if (
      item.action === "member.role_change" &&
      typeof from === "string" &&
      typeof to === "string"
    ) {
      return `${roleLabel(from)} → ${roleLabel(to)}`;
    }
    if (item.action === "member.access_change") {
      if (groups === "all") return t("teamAccess.allGroups");
      if (typeof groups === "number") {
        return t("teamAccess.someGroups", { count: groups });
      }
    }
    return null;
  };

  const describe = (item: TeamActivityItem) => {
    const target =
      targetLabel(item.target, localNames) ?? t("teamActivity.noTarget");
    const key = actionKey(item.action);
    return key
      ? t(`teamActivity.actions.${key}`, { target })
      : t("teamActivity.actions.other", { action: item.action, target });
  };

  return (
    <div className="flex flex-col gap-3" data-testid="team-activity">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-sm font-medium">{t("teamActivity.title")}</h3>
          <p className="text-xs text-muted-foreground">
            {t("teamActivity.description")}
          </p>
        </div>
        <div className="flex shrink-0 gap-2">
          <Button
            variant="soft"
            size="sm"
            onClick={() => void exportCsv()}
            disabled={isExporting || isLoading}
            data-testid="team-activity-export"
          >
            {t("teamActivity.exportCsv")}
          </Button>
          <Button
            variant="soft"
            size="sm"
            onClick={() => void load()}
            disabled={isLoading}
          >
            {t("common.buttons.refresh")}
          </Button>
        </div>
      </div>

      <div className="flex flex-wrap gap-2">
        <Select value={actor} onValueChange={setActor}>
          <SelectTrigger
            size="sm"
            variant="soft"
            className="w-[200px]"
            aria-label={t("teamActivity.filterMember")}
            data-testid="team-activity-member"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ANY}>{t("teamActivity.anyMember")}</SelectItem>
            {members.map((member) => (
              <SelectItem key={member.userId} value={member.userId}>
                {member.email ?? member.userId}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={targetType} onValueChange={setTargetType}>
          <SelectTrigger
            size="sm"
            variant="soft"
            className="w-[160px]"
            aria-label={t("teamActivity.filterType")}
            data-testid="team-activity-type"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ANY}>{t("teamActivity.anyType")}</SelectItem>
            {ACTIVITY_TARGET_TYPES.map((type) => (
              <SelectItem key={type} value={type}>
                {t(`teamActivity.types.${type}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select
          value={period}
          onValueChange={(value) => setPeriod(value as ActivityPeriod)}
        >
          <SelectTrigger
            size="sm"
            variant="soft"
            className="w-[150px]"
            aria-label={t("teamActivity.filterPeriod")}
            data-testid="team-activity-period"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {ACTIVITY_PERIODS.map((value) => (
              <SelectItem key={value} value={value}>
                {t(`teamActivity.periods.${value}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {isLoading ? (
        <div className="space-y-2">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
        </div>
      ) : error && items.length === 0 ? (
        <p className="py-6 text-center text-xs text-destructive-text">
          {error}
        </p>
      ) : items.length === 0 ? (
        <p className="py-6 text-center text-xs text-muted-foreground">
          {t("teamActivity.empty")}
        </p>
      ) : (
        <ol className="flex flex-col gap-0.5">
          {items.map((item) => {
            const fields = fieldKeys(item.detail);
            const kind = actorKind(item.actor.kind);
            const notes = [
              memberChange(item),
              fields.length > 0 &&
                t("teamActivity.changedFields", {
                  fields: fields
                    .map((field) => t(`teamActivity.fields.${field}`))
                    .join(", "),
                }),
              kind && kind !== "system" && t(`teamActivity.via.${kind}`),
            ].filter(Boolean);
            return (
              <li
                key={item.id}
                className="rounded-md px-2 py-1.5 hover:bg-foreground/5"
                data-action={item.action}
              >
                <div className="flex items-baseline justify-between gap-3 text-sm">
                  <span className="min-w-0 truncate">
                    <span className="font-medium">
                      {item.actor.email ??
                        (item.actor.kind === "system"
                          ? t("teamActivity.system")
                          : t("teamActivity.formerMember"))}
                    </span>{" "}
                    {describe(item)}
                  </span>
                  <time
                    className="shrink-0 text-xs tabular-nums text-muted-foreground"
                    dateTime={item.at}
                  >
                    {formatAt(item.at)}
                  </time>
                </div>
                {notes.length > 0 && (
                  <p className="truncate text-xs text-muted-foreground">
                    {notes.join(" · ")}
                  </p>
                )}
              </li>
            );
          })}
        </ol>
      )}

      {error && items.length > 0 && (
        <p className="text-xs text-destructive-text">{error}</p>
      )}
      {nextCursor && !isLoading && (
        <Button
          variant="subtle"
          size="sm"
          className="self-center"
          onClick={() => void loadMore()}
          disabled={isLoadingMore}
        >
          {t("teamActivity.loadMore")}
        </Button>
      )}
    </div>
  );
}

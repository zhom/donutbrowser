"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Skeleton } from "@/components/ui/skeleton";
import { translateBackendError } from "@/lib/backend-errors";
import {
  canChangeAccess,
  listTeamMembers,
  setTeamMemberAccess,
  type TeamMember,
  UNGROUPED_ACCESS,
} from "@/lib/team-activity";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

interface Group {
  id: string;
  name: string;
}

/** Which profile groups each member may reach. Owners and admins only. */
export function TeamAccess({ actorRole }: { actorRole: string | undefined }) {
  const { t } = useTranslation();
  const [members, setMembers] = React.useState<TeamMember[]>([]);
  const [groups, setGroups] = React.useState<Group[]>([]);
  const [isLoading, setIsLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [editing, setEditing] = React.useState<string | null>(null);
  const [everyGroup, setEveryGroup] = React.useState(true);
  const [chosen, setChosen] = React.useState<Set<string>>(() => new Set());
  const [isSaving, setIsSaving] = React.useState(false);

  React.useEffect(() => {
    void Promise.all([
      listTeamMembers(),
      invoke<Group[]>("get_profile_groups").catch(() => []),
    ])
      .then(([loaded, localGroups]) => {
        setMembers(loaded);
        setGroups(
          [...localGroups].sort((a, b) => a.name.localeCompare(b.name)),
        );
      })
      .catch((err) => setError(translateBackendError(t, err)))
      .finally(() => setIsLoading(false));
  }, [t]);

  const roleLabel = (role: string) =>
    role === "owner"
      ? t("sync.team.roleOwner")
      : role === "admin"
        ? t("sync.team.roleAdmin")
        : role === "launcher"
          ? t("sync.team.roleLauncher")
          : t("sync.team.roleMember");

  const accessLabel = (access: string[] | null) =>
    access === null
      ? t("teamAccess.allGroups")
      : t("teamAccess.someGroups", { count: access.length });

  const startEditing = (member: TeamMember) => {
    setEditing(member.userId);
    setEveryGroup(member.groupAccess === null);
    setChosen(new Set(member.groupAccess ?? []));
  };

  const toggle = (id: string, on: boolean) =>
    setChosen((current) => {
      const next = new Set(current);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const saveAccess = async (member: TeamMember) => {
    setIsSaving(true);
    try {
      const updated = await setTeamMemberAccess(
        member.userId,
        everyGroup ? null : [...chosen],
      );
      setMembers((current) =>
        current.map((item) =>
          item.userId === member.userId
            ? {
                ...item,
                groupAccess:
                  updated?.groupAccess ?? (everyGroup ? null : [...chosen]),
              }
            : item,
        ),
      );
      setEditing(null);
      showSuccessToast(t("teamAccess.saved"));
    } catch (err) {
      showErrorToast(translateBackendError(t, err));
    } finally {
      setIsSaving(false);
    }
  };

  const options: Group[] = [
    ...groups,
    { id: UNGROUPED_ACCESS, name: t("teamAccess.ungrouped") },
  ];

  return (
    <div className="flex flex-col gap-3" data-testid="team-access">
      <div>
        <h3 className="text-sm font-medium">{t("teamAccess.title")}</h3>
        <p className="text-xs text-muted-foreground">
          {t("teamAccess.description")}
        </p>
      </div>

      {isLoading ? (
        <div className="space-y-2">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
        </div>
      ) : error ? (
        <p className="py-6 text-center text-xs text-destructive-text">
          {error}
        </p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {members.map((member) => (
            <li
              key={member.userId}
              className="rounded-md px-2 py-1.5 hover:bg-foreground/5"
              data-member={member.userId}
            >
              <div className="flex items-center justify-between gap-3 text-sm">
                <span className="min-w-0 truncate">
                  <span className="font-medium">
                    {member.email ?? member.userId}
                  </span>{" "}
                  <span className="text-xs text-muted-foreground">
                    {roleLabel(member.role)} ·{" "}
                    {accessLabel(
                      member.role === "owner" ? null : member.groupAccess,
                    )}
                  </span>
                </span>
                {canChangeAccess(actorRole, member) &&
                  editing !== member.userId && (
                    <Button
                      variant="subtle"
                      size="sm"
                      onClick={() => startEditing(member)}
                    >
                      {t("teamAccess.change")}
                    </Button>
                  )}
              </div>

              {editing === member.userId && (
                <div className="mt-2 flex flex-col gap-3 rounded-md bg-foreground/5 p-3">
                  <RadioGroup
                    value={everyGroup ? "all" : "some"}
                    onValueChange={(value) => setEveryGroup(value === "all")}
                  >
                    <label
                      htmlFor={`access-all-${member.userId}`}
                      className="flex cursor-pointer items-center gap-2 text-sm"
                    >
                      <RadioGroupItem
                        id={`access-all-${member.userId}`}
                        value="all"
                      />
                      {t("teamAccess.allGroups")}
                    </label>
                    <label
                      htmlFor={`access-some-${member.userId}`}
                      className="flex cursor-pointer items-center gap-2 text-sm"
                    >
                      <RadioGroupItem
                        id={`access-some-${member.userId}`}
                        value="some"
                      />
                      {t("teamAccess.onlyThese")}
                    </label>
                  </RadioGroup>

                  {!everyGroup && (
                    <div className="flex max-h-56 flex-col gap-1.5 overflow-y-auto pl-6">
                      {options.map((group) => (
                        <label
                          key={group.id}
                          htmlFor={`access-${member.userId}-${group.id}`}
                          className="flex cursor-pointer items-center gap-2 text-sm"
                        >
                          <Checkbox
                            id={`access-${member.userId}-${group.id}`}
                            checked={chosen.has(group.id)}
                            onCheckedChange={(checked) =>
                              toggle(group.id, checked === true)
                            }
                          />
                          {group.name}
                        </label>
                      ))}
                    </div>
                  )}

                  <div className="flex justify-end gap-2">
                    <Button
                      variant="subtle"
                      size="sm"
                      onClick={() => setEditing(null)}
                      disabled={isSaving}
                    >
                      {t("common.buttons.cancel")}
                    </Button>
                    <Button
                      size="sm"
                      onClick={() => void saveAccess(member)}
                      disabled={isSaving}
                      data-testid="team-access-save"
                    >
                      {t("common.buttons.save")}
                    </Button>
                  </div>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

"use client";

import { useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuAppWindow,
  LuHand,
  LuMessageSquareText,
  LuSearch,
  LuUndo2,
} from "react-icons/lu";
import {
  activityDetailText,
  NoteAction,
  useTimeFormat,
} from "@/components/agent-console-parts";
import { DATA_TABLE_CLASSES } from "@/components/table-controls";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { AgentConsole } from "@/hooks/use-agent-console";
import {
  type AgentProfileRow,
  type AgentProfileState,
  profileRollup,
  sessionLabel,
} from "@/lib/agent-console";
import { cn } from "@/lib/utils";

const STATE_CLASS: Record<AgentProfileState, string> = {
  held: "bg-primary/10 text-primary-text",
  working: "bg-success/10 text-success-text",
  error: "bg-destructive/10 text-destructive-text",
  idle: "bg-foreground/6 text-muted-foreground",
};

export function AgentProfiles({
  agentConsole,
  profileNames,
  now,
  onNote,
}: {
  agentConsole: AgentConsole;
  profileNames: Map<string, string>;
  now: number;
  /** Opens the composer about this profile, addressed to this agent. */
  onNote: (profileId: string, sessionId: string | null) => void;
}) {
  const { t } = useTranslation();
  const { relative } = useTimeFormat();
  const [query, setQuery] = useState("");
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const searchId = useId();
  const errorsId = useId();

  const sessionsById = useMemo(
    () =>
      new Map(
        agentConsole.sessions.map((session) => [session.session_id, session]),
      ),
    [agentConsole.sessions],
  );

  const nameOf = (profileId: string) =>
    profileNames.get(profileId) ?? t("common.labels.unknownProfile");

  const allRows = useMemo(
    () => profileRollup(agentConsole.activity, agentConsole.holds, now),
    [agentConsole.activity, agentConsole.holds, now],
  );

  const rows = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return allRows.filter((row) => {
      if (errorsOnly && row.state !== "error") return false;
      if (!needle) return true;
      const name = profileNames.get(row.profile_id) ?? "";
      return name.toLowerCase().includes(needle);
    });
  }, [allRows, errorsOnly, query, profileNames]);

  const visibleSelected = rows.filter((row) => selected.has(row.profile_id));
  const toTakeOver = visibleSelected.filter((row) => row.hold === null);
  const toHandBack = visibleSelected.filter((row) => row.hold !== null);
  const allChecked =
    rows.length > 0 && rows.every((row) => selected.has(row.profile_id));

  const runAll = async (
    targets: AgentProfileRow[],
    action: (profileId: string) => Promise<boolean>,
  ) => {
    const results = await Promise.all(
      targets.map((row) => action(row.profile_id)),
    );
    setSelected(new Set());
    return results.every(Boolean);
  };

  return (
    <div
      data-testid="agent-profiles"
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      <div className="flex shrink-0 flex-wrap items-center gap-3">
        <Label htmlFor={searchId} className="sr-only">
          {t("agent.profiles.search")}
        </Label>
        <div className="relative">
          <LuSearch className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            id={searchId}
            variant="soft"
            value={query}
            placeholder={t("agent.profiles.search")}
            className="h-8 w-56 pl-8 text-xs md:text-xs"
            data-testid="agent-profiles-search"
            onChange={(event) => {
              setQuery(event.target.value);
            }}
          />
        </div>
        <div className="flex items-center gap-2">
          <Checkbox
            id={errorsId}
            checked={errorsOnly}
            data-testid="agent-profiles-errors-only"
            onCheckedChange={(checked) => {
              setErrorsOnly(checked === true);
            }}
          />
          <Label htmlFor={errorsId} className="text-xs">
            {t("agent.filters.errorsOnly")}
          </Label>
        </div>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <NoteAction
            label={t("agent.profiles.takeOverSelected")}
            icon={<LuHand className="size-3.5" aria-hidden="true" />}
            placeholder={t("agent.actions.takeOverPlaceholder")}
            disabled={toTakeOver.length === 0}
            testId="agent-take-over-selected"
            onConfirm={(note) =>
              runAll(toTakeOver, (profileId) =>
                agentConsole.takeOver(profileId, note),
              )
            }
          />
          <NoteAction
            label={t("agent.profiles.handBackSelected")}
            icon={<LuUndo2 className="size-3.5" aria-hidden="true" />}
            placeholder={t("agent.actions.handBackPlaceholder")}
            disabled={toHandBack.length === 0}
            testId="agent-hand-back-selected"
            onConfirm={(note) =>
              runAll(toHandBack, (profileId) =>
                agentConsole.handBack(profileId, note),
              )
            }
          />
        </div>
      </div>

      {allRows.length === 0 ? (
        <p className="py-16 text-center text-sm text-muted-foreground">
          {t("agent.profiles.empty")}
        </p>
      ) : rows.length === 0 ? (
        <p className="py-16 text-center text-sm text-muted-foreground">
          {t("agent.profiles.noMatches")}
        </p>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <Table className="data-table">
            <TableHeader className={DATA_TABLE_CLASSES.header}>
              <TableRow className={DATA_TABLE_CLASSES.headerRow}>
                <TableHead className={cn(DATA_TABLE_CLASSES.head, "w-8")}>
                  <Checkbox
                    checked={allChecked}
                    aria-label={t("common.aria.selectAll")}
                    onCheckedChange={(checked) => {
                      setSelected(
                        checked === true
                          ? new Set(rows.map((row) => row.profile_id))
                          : new Set(),
                      );
                    }}
                  />
                </TableHead>
                <TableHead className={DATA_TABLE_CLASSES.head}>
                  {t("agent.profiles.columns.profile")}
                </TableHead>
                <TableHead
                  className={cn(
                    DATA_TABLE_CLASSES.head,
                    "hidden @2xl:table-cell",
                  )}
                >
                  {t("agent.profiles.columns.agent")}
                </TableHead>
                <TableHead className={DATA_TABLE_CLASSES.head}>
                  {t("agent.profiles.columns.lastAction")}
                </TableHead>
                <TableHead
                  className={cn(
                    DATA_TABLE_CLASSES.head,
                    "hidden @xl:table-cell",
                  )}
                >
                  {t("agent.profiles.columns.result")}
                </TableHead>
                <TableHead
                  className={cn(
                    DATA_TABLE_CLASSES.head,
                    "hidden @xl:table-cell",
                  )}
                >
                  {t("agent.profiles.columns.when")}
                </TableHead>
                <TableHead className={DATA_TABLE_CLASSES.head}>
                  {t("agent.profiles.columns.state")}
                </TableHead>
                <TableHead
                  className={cn(DATA_TABLE_CLASSES.head, "text-right")}
                >
                  {t("common.labels.actions")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row) => {
                const name = nameOf(row.profile_id);
                const detail = row.last
                  ? activityDetailText(t, row.last)
                  : null;
                return (
                  <TableRow
                    key={row.profile_id}
                    data-table-row={row.profile_id}
                    className={DATA_TABLE_CLASSES.row}
                    data-testid={`agent-profile-${row.profile_id}`}
                    data-state={
                      selected.has(row.profile_id) ? "selected" : undefined
                    }
                  >
                    <TableCell>
                      <Checkbox
                        checked={selected.has(row.profile_id)}
                        aria-label={t("agent.profiles.selectRow", { name })}
                        onCheckedChange={(checked) => {
                          setSelected((previous) => {
                            const next = new Set(previous);
                            if (checked === true) next.add(row.profile_id);
                            else next.delete(row.profile_id);
                            return next;
                          });
                        }}
                      />
                    </TableCell>
                    <TableCell className="max-w-56">
                      <span className="block truncate text-sm font-medium">
                        {name}
                      </span>
                      {row.hold?.note && (
                        <span className="block truncate text-[11px] text-muted-foreground">
                          {row.hold.note}
                        </span>
                      )}
                    </TableCell>
                    <TableCell className="hidden max-w-40 truncate text-xs text-muted-foreground @2xl:table-cell">
                      {row.session_id
                        ? sessionLabel(
                            sessionsById.get(row.session_id),
                            t("agent.sessions.fallback"),
                            t("agent.sessions.website"),
                          )
                        : "—"}
                    </TableCell>
                    <TableCell className="max-w-64">
                      {row.last ? (
                        <span className="flex min-w-0 items-center gap-2 text-xs">
                          <span className="shrink-0 font-mono text-foreground">
                            {row.last.tool}
                          </span>
                          {detail && (
                            <span className="truncate text-muted-foreground">
                              {detail}
                            </span>
                          )}
                        </span>
                      ) : (
                        <span className="text-xs text-muted-foreground">—</span>
                      )}
                    </TableCell>
                    <TableCell className="hidden text-xs @xl:table-cell">
                      {row.last ? (
                        row.last.ok ? (
                          <span className="text-success-text">
                            {t("agent.result.ok")}
                          </span>
                        ) : (
                          <span className="font-mono text-destructive-text">
                            {row.last.error_code ?? t("agent.result.error")}
                          </span>
                        )
                      ) : (
                        <span className="text-muted-foreground">—</span>
                      )}
                    </TableCell>
                    <TableCell className="hidden text-xs tabular-nums text-muted-foreground @xl:table-cell">
                      <time dateTime={new Date(row.at).toISOString()}>
                        {relative(row.at, now)}
                      </time>
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant="soft"
                        className={cn("font-normal", STATE_CLASS[row.state])}
                      >
                        {t(`agent.profiles.state.${row.state}`)}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        {row.hold ? (
                          <NoteAction
                            label={t("agent.actions.handBack")}
                            icon={
                              <LuUndo2
                                className="size-3.5"
                                aria-hidden="true"
                              />
                            }
                            placeholder={t("agent.actions.handBackPlaceholder")}
                            testId="agent-hand-back"
                            onConfirm={(note) =>
                              agentConsole.handBack(row.profile_id, note)
                            }
                          />
                        ) : (
                          <NoteAction
                            label={t("agent.actions.takeOver")}
                            icon={
                              <LuHand className="size-3.5" aria-hidden="true" />
                            }
                            placeholder={t("agent.actions.takeOverPlaceholder")}
                            testId="agent-take-over"
                            onConfirm={(note) =>
                              agentConsole.takeOver(row.profile_id, note)
                            }
                          />
                        )}
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Button
                              type="button"
                              variant="subtle"
                              size="icon"
                              className="row-reveal size-8 rounded-lg"
                              aria-label={t("agent.actions.showWindow")}
                              data-testid="agent-profile-show-window"
                              onClick={() => {
                                void agentConsole.showWindow(row.profile_id);
                              }}
                            >
                              <LuAppWindow className="size-3.5" />
                            </Button>
                          </TooltipTrigger>
                          <TooltipContent>
                            {t("agent.actions.showWindow")}
                          </TooltipContent>
                        </Tooltip>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Button
                              type="button"
                              variant="subtle"
                              size="icon"
                              className="row-reveal size-8 rounded-lg"
                              aria-label={t("agent.actions.note")}
                              data-testid="agent-profile-note"
                              onClick={() => {
                                onNote(row.profile_id, row.session_id);
                              }}
                            >
                              <LuMessageSquareText className="size-3.5" />
                            </Button>
                          </TooltipTrigger>
                          <TooltipContent>
                            {t("agent.actions.note")}
                          </TooltipContent>
                        </Tooltip>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}

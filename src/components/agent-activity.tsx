"use client";

import { useVirtualizer } from "@tanstack/react-virtual";
import { useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuTrash2 } from "react-icons/lu";
import {
  activityDetailText,
  useTimeFormat,
} from "@/components/agent-console-parts";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { AgentConsole } from "@/hooks/use-agent-console";
import {
  type AgentActivity,
  activityStats,
  sessionLabel,
} from "@/lib/agent-console";
import { cn } from "@/lib/utils";

const ALL = "__all__";
const ROW_HEIGHT = 36;

export function AgentActivityFeed({
  agentConsole,
  profileNames,
  now,
}: {
  agentConsole: AgentConsole;
  profileNames: Map<string, string>;
  now: number;
}) {
  const { t } = useTranslation();
  const { clock, duration } = useTimeFormat();
  const [agent, setAgent] = useState(ALL);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [query, setQuery] = useState("");
  const [clearing, setClearing] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const agentId = useId();
  const errorsId = useId();
  const searchId = useId();

  const fallback = t("agent.sessions.fallback");
  const website = t("agent.sessions.website");
  const sessionsById = useMemo(
    () =>
      new Map(
        agentConsole.sessions.map((session) => [session.session_id, session]),
      ),
    [agentConsole.sessions],
  );

  const entries = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const out: AgentActivity[] = [];
    for (let index = agentConsole.activity.length - 1; index >= 0; index -= 1) {
      const entry = agentConsole.activity[index];
      if (agent !== ALL && entry.session_id !== agent) continue;
      if (errorsOnly && entry.ok) continue;
      if (needle) {
        const name = entry.profile_id
          ? (profileNames.get(entry.profile_id) ?? "")
          : "";
        if (!name.toLowerCase().includes(needle)) continue;
      }
      out.push(entry);
    }
    return out;
  }, [agentConsole.activity, agent, errorsOnly, query, profileNames]);

  const stats = useMemo(
    () => activityStats(agentConsole.activity, now),
    [agentConsole.activity, now],
  );

  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  return (
    <div
      data-testid="agent-activity"
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      <div className="flex shrink-0 flex-wrap items-center gap-3">
        <Label htmlFor={agentId} className="sr-only">
          {t("agent.activity.agentFilter")}
        </Label>
        <Select value={agent} onValueChange={setAgent}>
          <SelectTrigger
            id={agentId}
            className="h-8 w-48 text-xs"
            data-testid="agent-activity-agent"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>{t("agent.sessions.all")}</SelectItem>
            {agentConsole.sessions.map((session) => (
              <SelectItem key={session.session_id} value={session.session_id}>
                {sessionLabel(session, fallback, website)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Label htmlFor={searchId} className="sr-only">
          {t("agent.activity.profileSearch")}
        </Label>
        <Input
          id={searchId}
          value={query}
          placeholder={t("agent.activity.profileSearch")}
          className="h-8 w-48 text-sm"
          data-testid="agent-activity-search"
          onChange={(event) => {
            setQuery(event.target.value);
          }}
        />
        <div className="flex items-center gap-2">
          <Checkbox
            id={errorsId}
            checked={errorsOnly}
            data-testid="agent-activity-errors-only"
            onCheckedChange={(checked) => {
              setErrorsOnly(checked === true);
            }}
          />
          <Label htmlFor={errorsId} className="text-xs">
            {t("agent.filters.errorsOnly")}
          </Label>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="ml-auto gap-1.5"
          disabled={clearing || agentConsole.activity.length === 0}
          data-testid="agent-activity-clear"
          onClick={async () => {
            setClearing(true);
            await agentConsole.clearActivity();
            setClearing(false);
          }}
        >
          <LuTrash2 className="size-3.5" aria-hidden="true" />
          {t("common.buttons.clear")}
        </Button>
      </div>

      <p
        className="shrink-0 text-xs tabular-nums text-muted-foreground"
        data-testid="agent-activity-stats"
      >
        {stats.calls === 0
          ? t("agent.activity.statsNone")
          : t("agent.activity.stats", {
              calls: stats.calls,
              rate: Math.round(stats.errorRate * 100),
              average: duration(stats.averageMs ?? 0),
            })}
      </p>

      <div
        ref={scrollRef}
        role="list"
        aria-label={t("agent.activity.label")}
        data-testid="agent-activity-list"
        className="min-h-0 flex-1 overflow-y-auto rounded-md border border-border"
      >
        {entries.length === 0 ? (
          <p className="py-16 text-center text-sm text-muted-foreground">
            {agentConsole.activity.length === 0
              ? t("agent.activity.empty")
              : t("agent.activity.noMatches")}
          </p>
        ) : (
          <div
            className="relative w-full"
            style={{ height: virtualizer.getTotalSize() }}
          >
            {virtualizer.getVirtualItems().map((row) => {
              const entry = entries[row.index];
              const detail = activityDetailText(t, entry);
              return (
                <div
                  key={entry.id}
                  role="listitem"
                  data-testid={`agent-activity-${entry.id}`}
                  className="absolute top-0 left-0 flex w-full items-center gap-3 border-b border-border px-3 text-xs"
                  style={{
                    height: ROW_HEIGHT,
                    transform: `translateY(${row.start}px)`,
                  }}
                >
                  <time
                    dateTime={new Date(entry.at).toISOString()}
                    className="shrink-0 whitespace-nowrap tabular-nums text-muted-foreground"
                  >
                    {clock(entry.at)}
                  </time>
                  <span className="hidden w-28 shrink-0 truncate text-muted-foreground @2xl:inline">
                    {entry.session_id
                      ? sessionLabel(
                          sessionsById.get(entry.session_id),
                          fallback,
                          website,
                        )
                      : fallback}
                  </span>
                  <span className="w-32 shrink-0 truncate font-mono text-foreground">
                    {entry.tool}
                  </span>
                  <span className="w-36 shrink-0 truncate text-foreground">
                    {entry.profile_id
                      ? (profileNames.get(entry.profile_id) ??
                        t("common.labels.unknownProfile"))
                      : ""}
                  </span>
                  <span className="hidden min-w-0 flex-1 truncate text-muted-foreground @xl:inline">
                    {detail ?? ""}
                  </span>
                  <span className="ml-auto w-14 shrink-0 text-right tabular-nums text-muted-foreground">
                    {duration(entry.duration_ms)}
                  </span>
                  <span
                    className={cn(
                      "max-w-44 shrink-0 truncate rounded px-1.5 py-0.5",
                      entry.ok
                        ? "bg-success/10 text-success-text"
                        : "bg-destructive/10 font-mono text-destructive-text",
                    )}
                    title={entry.error_code ?? undefined}
                  >
                    {entry.ok
                      ? t("agent.result.ok")
                      : (entry.error_code ?? t("agent.result.error"))}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

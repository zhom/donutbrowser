"use client";

import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuActivity,
  LuAppWindow,
  LuCheck,
  LuChevronsUpDown,
  LuHand,
  LuMessageCircleQuestion,
  LuSend,
  LuUndo2,
} from "react-icons/lu";
import {
  NoteAction,
  ProgressBar,
  useTimeFormat,
} from "@/components/agent-console-parts";
import { StatusDot } from "@/components/cookie-bot-shared";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import type { AgentConsole } from "@/hooks/use-agent-console";
import {
  type AgentSession,
  type AgentThreadItem,
  collapseProgressRuns,
  isSessionActive,
  isSessionEarlier,
  MAX_TEXT_CHARS,
  progressPercent,
  sessionLabel,
  threadForSession,
} from "@/lib/agent-console";
import { cn } from "@/lib/utils";
import type { BrowserProfile } from "@/types";

type SessionNamer = (sessionId: string | null) => string;

function useSessionNamer(sessions: readonly AgentSession[]): SessionNamer {
  const { t } = useTranslation();
  return useMemo(() => {
    const byId = new Map(
      sessions.map((session) => [session.session_id, session]),
    );
    const fallback = t("agent.sessions.fallback");
    const website = t("agent.sessions.website");
    return (sessionId: string | null) =>
      sessionLabel(sessionId ? byId.get(sessionId) : null, fallback, website);
  }, [sessions, t]);
}

function ProfileChip({ name }: { name: string }) {
  return (
    <span className="max-w-48 truncate rounded-md bg-foreground/6 px-1.5 py-0.5 text-[11px] text-foreground">
      {name}
    </span>
  );
}

function RequestCard({
  item,
  agentName,
  profileName,
  agentConsole,
  now,
}: {
  item: AgentThreadItem;
  agentName: string;
  profileName: string | null;
  agentConsole: AgentConsole;
  now: number;
}) {
  const { t } = useTranslation();
  const { relative } = useTimeFormat();
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const baseId = useId();
  const isHelp = item.kind === "help";

  const send = async (text: string) => {
    setBusy(true);
    const ok = await agentConsole.answer(item.id, text);
    setBusy(false);
    if (ok) setAnswer("");
  };

  const dismissButton = (
    <Button
      type="button"
      variant="subtle"
      size="sm"
      className="h-8 rounded-lg text-xs"
      disabled={busy}
      data-testid="agent-dismiss"
      onClick={() => {
        void agentConsole.dismiss(item.id);
      }}
    >
      {t("agent.actions.dismiss")}
    </Button>
  );

  return (
    <article
      data-testid={`agent-request-${item.id}`}
      aria-labelledby={`${baseId}-title`}
      className="flex flex-col gap-2 rounded-xl bg-warning/10 p-3"
    >
      <header className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
        {isHelp ? (
          <LuHand className="size-3.5 text-warning-text" aria-hidden="true" />
        ) : (
          <LuMessageCircleQuestion
            className="size-3.5 text-warning-text"
            aria-hidden="true"
          />
        )}
        <span id={`${baseId}-title`} className="font-medium text-foreground">
          {t(
            isHelp ? "agent.request.helpTitle" : "agent.request.questionTitle",
            { agent: agentName },
          )}
        </span>
        <time dateTime={new Date(item.at).toISOString()}>
          {relative(item.at, now)}
        </time>
        {profileName && <ProfileChip name={profileName} />}
      </header>
      <p className="text-sm break-words whitespace-pre-wrap text-foreground">
        {item.text}
      </p>
      {isHelp ? (
        <div className="flex flex-wrap items-center justify-end gap-2">
          {item.profile_id && (
            <Button
              type="button"
              variant="soft"
              size="sm"
              className="h-8 gap-1.5 rounded-lg text-xs"
              data-testid="agent-show-window"
              onClick={() => {
                if (item.profile_id)
                  void agentConsole.showWindow(item.profile_id);
              }}
            >
              <LuAppWindow className="size-3.5" aria-hidden="true" />
              {t("agent.actions.showWindow")}
            </Button>
          )}
          <NoteAction
            label={t("agent.actions.handBack")}
            icon={<LuUndo2 className="size-3.5" aria-hidden="true" />}
            placeholder={t("agent.actions.handBackPlaceholder")}
            testId="agent-hand-back"
            onConfirm={(note) => agentConsole.answer(item.id, note ?? "")}
          />
          {dismissButton}
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          {item.choices.length > 0 && (
            <div
              role="group"
              aria-label={t("agent.request.choices")}
              className="flex flex-wrap gap-2"
            >
              {item.choices.map((choice) => (
                <Button
                  key={choice}
                  type="button"
                  size="sm"
                  variant="soft"
                  className="h-8 rounded-lg text-xs"
                  disabled={busy}
                  data-testid="agent-choice"
                  onClick={() => {
                    void send(choice);
                  }}
                >
                  {choice}
                </Button>
              ))}
            </div>
          )}
          <form
            className="flex min-w-48 flex-1 items-center gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              if (answer.trim()) void send(answer.trim());
            }}
          >
            <Label htmlFor={`${baseId}-answer`} className="sr-only">
              {t("agent.request.answerLabel")}
            </Label>
            <Input
              id={`${baseId}-answer`}
              variant="soft"
              value={answer}
              maxLength={MAX_TEXT_CHARS}
              placeholder={t("agent.request.answerPlaceholder")}
              className="h-8 text-xs md:text-xs"
              data-testid="agent-answer-input"
              onChange={(event) => {
                setAnswer(event.target.value);
              }}
            />
            <Button
              type="submit"
              size="sm"
              className="h-8 gap-1.5 rounded-lg text-xs"
              disabled={busy || answer.trim().length === 0}
              data-testid="agent-answer-send"
            >
              <LuSend className="size-3.5" aria-hidden="true" />
              {t("agent.request.send")}
            </Button>
          </form>
          {dismissButton}
        </div>
      )}
    </article>
  );
}

/** Open questions and help requests from every agent. */
export function NeedsYouList({
  agentConsole,
  profileNames,
  now,
}: {
  agentConsole: AgentConsole;
  profileNames: Map<string, string>;
  now: number;
}) {
  const { t } = useTranslation();
  const nameOf = useSessionNamer(agentConsole.sessions);
  const requests = agentConsole.openRequests;

  return (
    <section
      data-testid="agent-needs-you"
      aria-labelledby="agent-needs-you-title"
      className="flex max-h-[40vh] shrink-0 flex-col gap-2 overflow-y-auto"
    >
      <h3
        id="agent-needs-you-title"
        className="flex items-center gap-2 text-xs font-semibold tracking-wide text-warning-text uppercase"
      >
        {t("agent.request.needsYou")}
        <span className="tabular-nums">{requests.length}</span>
      </h3>
      <p className="sr-only" aria-live="polite">
        {t("agent.request.waitingCount", { count: requests.length })}
      </p>
      {requests.map((item) => (
        <RequestCard
          key={item.id}
          item={item}
          agentName={nameOf(item.session_id)}
          profileName={
            item.profile_id
              ? (profileNames.get(item.profile_id) ??
                t("common.labels.unknownProfile"))
              : null
          }
          agentConsole={agentConsole}
          now={now}
        />
      ))}
    </section>
  );
}

function SessionButton({
  session,
  selected,
  onSelect,
  now,
}: {
  session: AgentSession;
  selected: boolean;
  onSelect: () => void;
  now: number;
}) {
  const { t } = useTranslation();
  const active = isSessionActive(session, now);
  const percent = session.status
    ? progressPercent(session.status.done, session.status.total)
    : null;
  const name = sessionLabel(
    session,
    t("agent.sessions.fallback"),
    t("agent.sessions.website"),
  );
  return (
    <button
      type="button"
      aria-pressed={selected}
      data-testid={`agent-session-${session.session_id}`}
      onClick={onSelect}
      className={cn(
        "flex w-full cursor-pointer flex-col gap-1 rounded-lg px-2 py-2 text-left transition-colors duration-100 hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
        selected && "bg-foreground/7 hover:bg-foreground/7",
      )}
    >
      <span className="flex min-w-0 items-center gap-2">
        <StatusDot tone={active ? "success" : "muted"} />
        <span className="truncate text-sm font-medium">{name}</span>
        {session.client_version && (
          <span className="shrink-0 text-[11px] text-muted-foreground">
            {session.client_version}
          </span>
        )}
        <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">
          {session.ended
            ? t("agent.sessions.ended")
            : active
              ? t("agent.sessions.active")
              : t("agent.sessions.idle")}
        </span>
      </span>
      {session.status && (
        <span className="truncate text-xs text-muted-foreground">
          {session.status.message}
        </span>
      )}
      {session.status && percent !== null && (
        <span className="flex items-center gap-2">
          <ProgressBar
            percent={percent}
            label={t("agent.sessions.progress", {
              done: session.status.done ?? 0,
              total: session.status.total ?? 0,
            })}
          />
          <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
            {t("agent.sessions.progress", {
              done: session.status.done ?? 0,
              total: session.status.total ?? 0,
            })}
          </span>
        </span>
      )}
      <span
        className={cn(
          "text-[11px] tabular-nums",
          session.errors > 0
            ? "text-destructive-text"
            : "text-muted-foreground",
        )}
      >
        {t("agent.sessions.counts", {
          calls: session.calls,
          errors: session.errors,
        })}
      </span>
    </button>
  );
}

function SessionList({
  sessions,
  selectedSessionId,
  onSelectSession,
  now,
}: {
  sessions: AgentSession[];
  selectedSessionId: string | null;
  onSelectSession: (sessionId: string | null) => void;
  now: number;
}) {
  const { t } = useTranslation();
  const [showEarlier, setShowEarlier] = useState(false);
  const earlier = sessions.filter(
    (session) =>
      isSessionEarlier(session, now) &&
      session.session_id !== selectedSessionId,
  );
  const visible = showEarlier
    ? sessions
    : sessions.filter((session) => !earlier.includes(session));

  return (
    <nav
      aria-label={t("agent.sessions.title")}
      data-testid="agent-sessions"
      className="flex max-h-56 min-h-0 flex-col gap-1 overflow-y-auto @3xl:max-h-none"
    >
      <button
        type="button"
        aria-pressed={selectedSessionId === null}
        data-testid="agent-session-all"
        onClick={() => {
          onSelectSession(null);
        }}
        className={cn(
          "flex w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-2 text-left text-sm font-medium transition-colors duration-100 hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
          selectedSessionId === null && "bg-foreground/7 hover:bg-foreground/7",
        )}
      >
        {t("agent.sessions.all")}
        <span className="ml-auto text-[11px] tabular-nums text-muted-foreground">
          {sessions.filter((session) => isSessionActive(session, now)).length}
        </span>
      </button>
      {visible.map((session) => (
        <SessionButton
          key={session.session_id}
          session={session}
          selected={selectedSessionId === session.session_id}
          onSelect={() => {
            onSelectSession(session.session_id);
          }}
          now={now}
        />
      ))}
      {earlier.length > 0 && (
        <button
          type="button"
          aria-expanded={showEarlier}
          onClick={() => {
            setShowEarlier((value) => !value);
          }}
          className="cursor-pointer rounded-md px-2 py-1.5 text-left text-xs text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          {showEarlier
            ? t("agent.sessions.hideEarlier")
            : t("agent.sessions.showEarlier", { count: earlier.length })}
        </button>
      )}
    </nav>
  );
}

function ThreadEntry({
  item,
  nameOf,
  profileName,
  now,
}: {
  item: AgentThreadItem;
  nameOf: SessionNamer;
  profileName: string | null;
  now: number;
}) {
  const { t } = useTranslation();
  const { relative, shortClock } = useTimeFormat();
  const time = (
    <time dateTime={new Date(item.at).toISOString()}>
      {relative(item.at, now)}
    </time>
  );

  if (item.kind === "joined" || item.kind === "left") {
    return (
      <p className="flex items-center justify-center gap-1.5 text-[11px] text-muted-foreground">
        {t(
          item.kind === "joined" ? "agent.thread.joined" : "agent.thread.left",
          { agent: nameOf(item.session_id) },
        )}
        <span aria-hidden="true">·</span>
        {time}
      </p>
    );
  }

  if (item.kind === "progress") {
    const hasTotal = item.done !== null && item.total !== null;
    return (
      <p className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
        <LuActivity className="size-3 shrink-0" aria-hidden="true" />
        <span className="shrink-0 font-medium">{nameOf(item.session_id)}</span>
        <span className="min-w-0 truncate">{item.text}</span>
        {hasTotal && (
          <span className="shrink-0 tabular-nums">
            {t("agent.sessions.progress", {
              done: item.done,
              total: item.total,
            })}
          </span>
        )}
        <span className="ml-auto shrink-0">{time}</span>
      </p>
    );
  }

  if (item.kind === "note") {
    const delivery =
      item.state === "delivered"
        ? item.answered_at
          ? t("agent.thread.noteSeenAt", { time: shortClock(item.answered_at) })
          : t("agent.thread.noteSeen")
        : t("agent.thread.notePending");
    return (
      <div className="ml-auto flex max-w-[85%] flex-col items-end gap-1">
        <div className="rounded-lg bg-primary px-3 py-2 text-sm break-words whitespace-pre-wrap text-primary-foreground">
          {item.text}
        </div>
        <span className="flex flex-wrap items-center justify-end gap-x-1.5 text-[11px] text-muted-foreground">
          <span>
            {item.session_id
              ? t("agent.thread.noteTo", { agent: nameOf(item.session_id) })
              : t("agent.thread.noteToAll")}
          </span>
          {profileName && <ProfileChip name={profileName} />}
          <span aria-hidden="true">·</span>
          <span
            className={cn(item.state === "delivered" && "text-success-text")}
          >
            {delivery}
          </span>
          <span aria-hidden="true">·</span>
          {time}
        </span>
      </div>
    );
  }

  const isHelp = item.kind === "help";
  let outcome: string;
  if (item.state === "open") outcome = t("agent.thread.waiting");
  else if (item.state === "dismissed") outcome = t("agent.thread.dismissed");
  else if (isHelp)
    outcome = item.answer
      ? t("agent.thread.handedBackNote", { note: item.answer })
      : t("agent.thread.handedBack");
  else outcome = t("agent.thread.answered", { answer: item.answer ?? "" });

  return (
    <div className="flex max-w-[85%] flex-col gap-1">
      <span className="flex flex-wrap items-center gap-x-1.5 text-[11px] text-muted-foreground">
        <span className="font-medium text-foreground">
          {nameOf(item.session_id)}
        </span>
        {profileName && <ProfileChip name={profileName} />}
        <span aria-hidden="true">·</span>
        {time}
      </span>
      <div className="flex items-start gap-2 rounded-xl bg-foreground/5 px-3 py-2 text-sm text-foreground">
        {isHelp ? (
          <LuHand className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
        ) : (
          <LuMessageCircleQuestion
            className="mt-0.5 size-3.5 shrink-0"
            aria-hidden="true"
          />
        )}
        <span className="min-w-0 break-words whitespace-pre-wrap">
          {item.text}
        </span>
      </div>
      <span
        className={cn(
          "text-xs break-words",
          item.state === "open" ? "text-warning-text" : "text-muted-foreground",
        )}
      >
        {outcome}
      </span>
    </div>
  );
}

function ProfilePicker({
  profiles,
  value,
  onChange,
  preferredIds,
}: {
  profiles: BrowserProfile[];
  value: string | null;
  onChange: (profileId: string | null) => void;
  preferredIds: readonly string[];
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const listId = useId();

  const options = useMemo(() => {
    const preferred = new Map(preferredIds.map((id, index) => [id, index]));
    const needle = query.trim().toLowerCase();
    const matches = needle
      ? profiles.filter((profile) =>
          profile.name.toLowerCase().includes(needle),
        )
      : profiles;
    return [...matches]
      .sort((a, b) => {
        const rankA = preferred.get(a.id) ?? Number.POSITIVE_INFINITY;
        const rankB = preferred.get(b.id) ?? Number.POSITIVE_INFINITY;
        if (rankA !== rankB) return rankA - rankB;
        return a.name.localeCompare(b.name);
      })
      .slice(0, 50);
  }, [profiles, preferredIds, query]);

  const selected = value
    ? (profiles.find((profile) => profile.id === value)?.name ??
      t("common.labels.unknownProfile"))
    : null;

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) setQuery("");
      }}
    >
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="soft"
          size="sm"
          role="combobox"
          aria-expanded={open}
          aria-controls={listId}
          aria-label={t("agent.composer.profile")}
          data-testid="agent-composer-profile"
          className="h-8 max-w-56 justify-between gap-2 rounded-lg text-xs font-normal"
        >
          <span className="truncate">
            {selected ?? t("agent.composer.noProfile")}
          </span>
          <LuChevronsUpDown
            className="size-3.5 shrink-0 opacity-50"
            aria-hidden="true"
          />
        </Button>
      </PopoverTrigger>
      <PopoverContent id={listId} align="start" className="w-64 p-0">
        <Command shouldFilter={false}>
          <CommandInput
            value={query}
            onValueChange={setQuery}
            placeholder={t("agent.composer.profileSearch")}
          />
          <CommandList>
            <CommandEmpty>{t("agent.composer.profileNotFound")}</CommandEmpty>
            <CommandGroup>
              {query.trim().length === 0 && (
                <CommandItem
                  value="__none__"
                  onSelect={() => {
                    onChange(null);
                    setOpen(false);
                  }}
                >
                  <LuCheck
                    className={cn(
                      "size-4",
                      value === null ? "opacity-100" : "opacity-0",
                    )}
                    aria-hidden="true"
                  />
                  {t("agent.composer.noProfile")}
                </CommandItem>
              )}
              {options.map((profile) => (
                <CommandItem
                  key={profile.id}
                  value={profile.id}
                  onSelect={() => {
                    onChange(profile.id);
                    setOpen(false);
                  }}
                >
                  <LuCheck
                    className={cn(
                      "size-4",
                      value === profile.id ? "opacity-100" : "opacity-0",
                    )}
                    aria-hidden="true"
                  />
                  <span className="truncate">{profile.name}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function Composer({
  agentConsole,
  sessionId,
  sessionName,
  profiles,
  profileId,
  onProfileChange,
  focusRequested,
  onFocused,
  preferredIds,
}: {
  agentConsole: AgentConsole;
  sessionId: string | null;
  sessionName: string;
  profiles: BrowserProfile[];
  profileId: string | null;
  onProfileChange: (profileId: string | null) => void;
  focusRequested: boolean;
  onFocused: () => void;
  preferredIds: readonly string[];
}) {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const inputId = useId();
  const hintId = useId();

  useEffect(() => {
    if (!focusRequested) return;
    textareaRef.current?.focus();
    onFocused();
  }, [focusRequested, onFocused]);

  const submit = async () => {
    const trimmed = text.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    const ok = await agentConsole.sendNote(trimmed, sessionId, profileId);
    setBusy(false);
    if (ok) setText("");
  };

  return (
    <form
      className="flex shrink-0 flex-col gap-2 border-t border-foreground/6 p-3"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <Label htmlFor={inputId} className="sr-only">
        {t("agent.composer.label")}
      </Label>
      <Textarea
        ref={textareaRef}
        id={inputId}
        rows={2}
        value={text}
        maxLength={MAX_TEXT_CHARS}
        aria-describedby={hintId}
        placeholder={
          sessionId
            ? t("agent.composer.placeholderOne", { agent: sessionName })
            : t("agent.composer.placeholderAll")
        }
        variant="soft"
        className="min-h-14 resize-none text-sm"
        data-testid="agent-composer-input"
        onChange={(event) => {
          setText(event.target.value);
        }}
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !event.shiftKey &&
            !event.nativeEvent.isComposing
          ) {
            event.preventDefault();
            void submit();
          }
        }}
      />
      <div className="flex flex-wrap items-center gap-2">
        <ProfilePicker
          profiles={profiles}
          value={profileId}
          onChange={onProfileChange}
          preferredIds={preferredIds}
        />
        <span
          id={hintId}
          className="hidden text-[11px] text-muted-foreground @2xl:inline"
        >
          {t("agent.composer.hint")}
        </span>
        <Button
          type="submit"
          size="sm"
          className="ml-auto h-8 gap-1.5 rounded-lg text-xs"
          disabled={busy || text.trim().length === 0}
          data-testid="agent-composer-send"
        >
          <LuSend className="size-3.5" aria-hidden="true" />
          {t("agent.composer.send")}
        </Button>
      </div>
    </form>
  );
}

export function AgentConversation({
  agentConsole,
  profiles,
  profileNames,
  selectedSessionId,
  onSelectSession,
  composerProfileId,
  onComposerProfileChange,
  focusComposer,
  onComposerFocused,
  preferredProfileIds,
  now,
}: {
  agentConsole: AgentConsole;
  profiles: BrowserProfile[];
  profileNames: Map<string, string>;
  selectedSessionId: string | null;
  onSelectSession: (sessionId: string | null) => void;
  composerProfileId: string | null;
  onComposerProfileChange: (profileId: string | null) => void;
  focusComposer: boolean;
  onComposerFocused: () => void;
  preferredProfileIds: readonly string[];
  now: number;
}) {
  const { t } = useTranslation();
  const nameOf = useSessionNamer(agentConsole.sessions);
  const threadRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);

  const items = useMemo(
    () =>
      collapseProgressRuns(
        threadForSession(agentConsole.thread, selectedSessionId),
      ),
    [agentConsole.thread, selectedSessionId],
  );

  useEffect(() => {
    const element = threadRef.current;
    if (element && stickToBottom.current && items.length > 0) {
      element.scrollTop = element.scrollHeight;
    }
  }, [items]);

  return (
    <div className="grid min-h-0 flex-1 grid-rows-[auto_minmax(12rem,1fr)] gap-3 @3xl:grid-cols-[16rem_minmax(0,1fr)] @3xl:grid-rows-[minmax(0,1fr)]">
      <SessionList
        sessions={agentConsole.sessions}
        selectedSessionId={selectedSessionId}
        onSelectSession={onSelectSession}
        now={now}
      />
      <div className="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-xl bg-foreground/3">
        <div
          ref={threadRef}
          role="log"
          aria-label={t("agent.thread.label")}
          data-testid="agent-thread"
          className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3"
          onScroll={() => {
            const element = threadRef.current;
            if (!element) return;
            stickToBottom.current =
              element.scrollHeight - element.scrollTop - element.clientHeight <
              48;
          }}
        >
          {items.length === 0 ? (
            <p className="m-auto max-w-sm text-center text-xs text-muted-foreground">
              {t("agent.thread.empty")}
            </p>
          ) : (
            items.map((item) => (
              <ThreadEntry
                key={item.id}
                item={item}
                nameOf={nameOf}
                profileName={
                  item.profile_id
                    ? (profileNames.get(item.profile_id) ??
                      t("common.labels.unknownProfile"))
                    : null
                }
                now={now}
              />
            ))
          )}
        </div>
        <Composer
          agentConsole={agentConsole}
          sessionId={selectedSessionId}
          sessionName={nameOf(selectedSessionId)}
          profiles={profiles}
          profileId={composerProfileId}
          onProfileChange={onComposerProfileChange}
          focusRequested={focusComposer}
          onFocused={onComposerFocused}
          preferredIds={preferredProfileIds}
        />
      </div>
    </div>
  );
}

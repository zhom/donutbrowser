"use client";

import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import { invoke } from "@tauri-apps/api/core";
import { motion, useReducedMotion } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuBug,
  LuCheck,
  LuChevronRight,
  LuFileText,
  LuHeart,
  LuLightbulb,
  LuMail,
  LuMessageSquareMore,
  LuSend,
} from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { AnimatedDisclosureChevron } from "@/components/ui/animated-disclosure";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SoftFields } from "@/components/ui/field-variant";
import { Input } from "@/components/ui/input";
import { StepTransition } from "@/components/ui/step-transition";
import { Textarea } from "@/components/ui/textarea";
import { useInputModality } from "@/hooks/use-input-modality";
import type { FeedbackKind } from "@/lib/agent-console";
import { translateBackendError } from "@/lib/backend-errors";
import { formatBytes } from "@/lib/format-bytes";
import { MOTION_SPRING_POSITION } from "@/lib/motion";
import { isMacOS } from "@/lib/platform";
import { cn } from "@/lib/utils";
import { RippleButton } from "./ui/ripple";

/** Mirrors `feedback::MAX_MESSAGE_CHARS`. */
const MAX_MESSAGE_CHARS = 5000;
/** The counter shows once the message gets this close to the limit. */
const COUNTER_FROM = MAX_MESSAGE_CHARS - 500;

const KINDS: {
  id: FeedbackKind;
  Icon: React.ComponentType<{ className?: string }>;
}[] = [
  { id: "bug", Icon: LuBug },
  { id: "idea", Icon: LuLightbulb },
  { id: "praise", Icon: LuHeart },
  { id: "other", Icon: LuMessageSquareMore },
];

interface SystemInfo {
  app_version: string;
  os: string;
  arch: string;
}

interface FeedbackReceipt {
  id: string;
}

interface FeedbackDialogProps {
  isOpen: boolean;
  onClose: () => void;
  /** The signed-in account's email; replies go there. */
  accountEmail: string | null;
}

/**
 * Feedback to the Donut team. A draft survives closing the dialog and is only
 * cleared once it has been sent. Logs are on by default for a bug, the one
 * kind where they nearly always matter, and the person can read exactly what
 * would be attached before sending.
 */
export function FeedbackDialog({
  isOpen,
  onClose,
  accountEmail,
}: FeedbackDialogProps) {
  const { t, i18n } = useTranslation();
  const reduceMotion = useReducedMotion();
  const inputModality = useInputModality();
  const animate = !reduceMotion && inputModality === "pointer";

  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [message, setMessage] = useState("");
  const [includeLogs, setIncludeLogs] = useState(true);
  const [logsChosen, setLogsChosen] = useState(false);
  const [email, setEmail] = useState("");
  const [previewOpen, setPreviewOpen] = useState(false);
  const [logs, setLogs] = useState<string | null>(null);
  const [logsLoading, setLogsLoading] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState<{ logs: boolean; replyTo: string | null }>();
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const messageRef = useRef<HTMLTextAreaElement>(null);
  const previewRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (!isOpen || systemInfo) return;
    invoke<SystemInfo>("get_system_info")
      .then(setSystemInfo)
      .catch((err: unknown) => {
        console.warn("Failed to read system info:", err);
      });
  }, [isOpen, systemInfo]);

  const chooseKind = useCallback(
    (next: FeedbackKind) => {
      setKind(next);
      if (!logsChosen) setIncludeLogs(next === "bug");
    },
    [logsChosen],
  );

  const loadLogs = useCallback(async () => {
    setLogsLoading(true);
    try {
      setLogs(await invoke<string>("preview_feedback_logs"));
    } catch (err) {
      setError(translateBackendError(t, err));
    } finally {
      setLogsLoading(false);
    }
  }, [t]);

  const togglePreview = useCallback(() => {
    const opening = !previewOpen;
    setPreviewOpen(opening);
    if (opening) void loadLogs();
  }, [loadLogs, previewOpen]);

  // The newest lines are at the end; that is what the reader wants to see.
  useEffect(() => {
    if (!previewOpen || logs === null || !previewRef.current) return;
    previewRef.current.scrollTop = previewRef.current.scrollHeight;
  }, [logs, previewOpen]);

  const handleClose = useCallback(() => {
    if (sent) {
      setSent(undefined);
      setMessage("");
      setKind("bug");
      setIncludeLogs(true);
      setLogsChosen(false);
      setPreviewOpen(false);
      setLogs(null);
    }
    setError(null);
    onClose();
  }, [onClose, sent]);

  const canSend = message.trim().length > 0 && !sending;

  const send = useCallback(async () => {
    if (!message.trim() || sending) return;
    setSending(true);
    setError(null);
    try {
      await invoke<FeedbackReceipt>("send_feedback", {
        kind,
        message,
        includeLogs,
        email: accountEmail ? null : email.trim() || null,
        locale: i18n.language,
      });
      setSent({
        logs: includeLogs,
        replyTo: accountEmail ?? (email.trim() || null),
      });
    } catch (err) {
      console.error("Failed to send feedback:", err);
      setError(translateBackendError(t, err));
    } finally {
      setSending(false);
    }
  }, [
    accountEmail,
    email,
    i18n.language,
    includeLogs,
    kind,
    message,
    sending,
    t,
  ]);

  const modKey = isMacOS() ? "⌘" : "Ctrl";
  const logsBytes = useMemo(
    () => (logs === null ? null : new Blob([logs]).size),
    [logs],
  );

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) handleClose();
      }}
    >
      <DialogContent
        className="max-w-lg"
        data-dialog="feedback"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          messageRef.current?.focus();
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            if (!sent) void send();
          }
        }}
      >
        <StepTransition
          transitionKey={sent ? "sent" : "compose"}
          direction={sent ? 1 : -1}
          className="grid min-w-0 gap-4"
        >
          {sent ? (
            <div
              data-slot="feedback-sent"
              className="grid justify-items-center gap-2 py-6 text-center"
            >
              <motion.div
                initial={animate ? { scale: 0.6 } : false}
                animate={{ scale: 1 }}
                transition={animate ? MOTION_SPRING_POSITION : { duration: 0 }}
                className="mb-2 grid size-14 place-items-center rounded-full bg-success/15 text-success-text"
              >
                <LuCheck className="size-7" aria-hidden="true" />
              </motion.div>
              <DialogTitle>{t("feedback.success.title")}</DialogTitle>
              <DialogDescription className="max-w-sm">
                {sent.logs
                  ? t("feedback.success.bodyWithLogs")
                  : t("feedback.success.body")}
              </DialogDescription>
              {sent.replyTo && (
                <p className="max-w-sm text-xs break-words text-muted-foreground">
                  {t("feedback.success.reply", { email: sent.replyTo })}
                </p>
              )}
              <RippleButton
                autoFocus
                className="mt-4 min-w-28"
                onClick={handleClose}
              >
                {t("feedback.success.done")}
              </RippleButton>
            </div>
          ) : (
            <>
              <DialogHeader>
                <DialogTitle>{t("feedback.title")}</DialogTitle>
                <DialogDescription>
                  {t("feedback.description")}
                </DialogDescription>
              </DialogHeader>

              <SoftFields>
                <RadioGroupPrimitive.Root
                  value={kind}
                  onValueChange={(value) => {
                    chooseKind(value as FeedbackKind);
                  }}
                  aria-label={t("feedback.kindLabel")}
                  className="grid grid-cols-4 gap-2"
                >
                  {KINDS.map(({ id, Icon }) => (
                    <RadioGroupPrimitive.Item
                      key={id}
                      value={id}
                      data-slot="feedback-kind"
                      data-kind={id}
                      className={cn(
                        "flex cursor-pointer flex-col items-center gap-1.5 rounded-lg px-2 py-2.5 text-xs font-medium transition-[color,background-color,box-shadow] duration-150 outline-none",
                        "bg-foreground/5 text-muted-foreground hover:bg-foreground/8 hover:text-foreground",
                        "focus-visible:ring-2 focus-visible:ring-ring",
                        "data-[state=checked]:bg-primary/10 data-[state=checked]:text-foreground data-[state=checked]:inset-ring data-[state=checked]:inset-ring-primary/50",
                      )}
                    >
                      <Icon className="size-4" aria-hidden="true" />
                      <span className="max-w-full truncate">
                        {t(`feedback.kinds.${id}.label`)}
                      </span>
                    </RadioGroupPrimitive.Item>
                  ))}
                </RadioGroupPrimitive.Root>

                <div className="grid gap-1">
                  <Textarea
                    ref={messageRef}
                    data-slot="feedback-message"
                    aria-label={t("feedback.messageLabel")}
                    placeholder={t(`feedback.kinds.${kind}.placeholder`)}
                    value={message}
                    maxLength={MAX_MESSAGE_CHARS}
                    rows={6}
                    className="min-h-32 resize-none"
                    onChange={(event) => {
                      setMessage(event.target.value);
                    }}
                  />
                  {message.length >= COUNTER_FROM && (
                    <p className="text-right text-[11px] text-muted-foreground tabular-nums">
                      {t("feedback.counter", {
                        count: message.length,
                        max: MAX_MESSAGE_CHARS,
                      })}
                    </p>
                  )}
                </div>

                <div className="grid gap-2 rounded-lg bg-foreground/5 p-3">
                  <div className="flex items-start gap-3">
                    <LuFileText
                      className="mt-0.5 size-4 shrink-0 text-muted-foreground"
                      aria-hidden="true"
                    />
                    <div className="grid min-w-0 flex-1 gap-0.5">
                      <label
                        htmlFor="feedback-include-logs"
                        className="cursor-pointer text-sm font-medium"
                      >
                        {t("feedback.logs.label")}
                      </label>
                      <p className="text-xs text-muted-foreground">
                        {t("feedback.logs.description")}
                      </p>
                    </div>
                    <AnimatedSwitch
                      id="feedback-include-logs"
                      data-slot="feedback-include-logs"
                      checked={includeLogs}
                      onCheckedChange={(checked) => {
                        setIncludeLogs(checked);
                        setLogsChosen(true);
                      }}
                    />
                  </div>
                  {includeLogs && (
                    <div className="pl-7">
                      <button
                        type="button"
                        data-slot="feedback-preview-logs"
                        aria-expanded={previewOpen}
                        onClick={togglePreview}
                        className="inline-flex cursor-pointer items-center gap-1 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                      >
                        <AnimatedDisclosureChevron open={previewOpen}>
                          <LuChevronRight className="size-3" />
                        </AnimatedDisclosureChevron>
                        {t("feedback.logs.preview")}
                        {logsBytes !== null && logsBytes > 0 && (
                          <span className="tabular-nums">
                            · {formatBytes(logsBytes)}
                          </span>
                        )}
                      </button>
                      {previewOpen && (
                        <textarea
                          ref={previewRef}
                          readOnly
                          data-slot="feedback-logs-preview"
                          aria-label={t("feedback.logs.preview")}
                          rows={10}
                          spellCheck={false}
                          value={
                            logsLoading && logs === null
                              ? t("feedback.logs.loading")
                              : logs
                                ? logs
                                : t("feedback.logs.empty")
                          }
                          className="mt-2 block h-44 w-full resize-none overflow-auto rounded-md bg-background p-2 font-mono text-[10px] leading-relaxed break-all whitespace-pre-wrap text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                        />
                      )}
                    </div>
                  )}
                </div>

                {accountEmail ? (
                  <p className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                    <LuMail className="size-3.5 shrink-0" aria-hidden="true" />
                    <span className="min-w-0 truncate">
                      {t("feedback.email.signedIn", { email: accountEmail })}
                    </span>
                  </p>
                ) : (
                  <div className="grid gap-1.5">
                    <label
                      htmlFor="feedback-email"
                      className="flex items-baseline gap-1.5 text-sm font-medium"
                    >
                      {t("feedback.email.label")}
                      <span className="text-xs font-normal text-muted-foreground">
                        {t("feedback.email.optional")}
                      </span>
                    </label>
                    <Input
                      id="feedback-email"
                      data-slot="feedback-email"
                      type="email"
                      autoComplete="email"
                      placeholder={t("feedback.email.placeholder")}
                      value={email}
                      onChange={(event) => {
                        setEmail(event.target.value);
                      }}
                    />
                  </div>
                )}

                {error && (
                  <p
                    role="alert"
                    className="rounded-md bg-destructive/10 p-3 text-sm break-words text-destructive-text"
                  >
                    {error}
                  </p>
                )}
              </SoftFields>

              <DialogFooter className="items-center">
                {systemInfo && (
                  <p className="mr-auto min-w-0 truncate text-[11px] text-muted-foreground">
                    {t("feedback.included", {
                      version: systemInfo.app_version,
                      os: systemInfo.os,
                      arch: systemInfo.arch,
                    })}
                  </p>
                )}
                <RippleButton
                  variant="outline"
                  onClick={handleClose}
                  disabled={sending}
                >
                  {t("common.buttons.cancel")}
                </RippleButton>
                <LoadingButton
                  data-slot="feedback-send"
                  isLoading={sending}
                  disabled={!canSend}
                  onClick={() => void send()}
                >
                  <LuSend className="size-3.5" aria-hidden="true" />
                  {t("feedback.send")}
                  <kbd className="ml-1 hidden rounded bg-primary-foreground/15 px-1 font-sans text-[10px] leading-4 sm:inline">
                    {modKey}↵
                  </kbd>
                </LoadingButton>
              </DialogFooter>
            </>
          )}
        </StepTransition>
      </DialogContent>
    </Dialog>
  );
}

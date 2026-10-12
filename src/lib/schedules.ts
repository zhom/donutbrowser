import { invoke } from "@tauri-apps/api/core";

/** How often a schedule fires. */
export type ScheduleCadence = "weekly" | "once" | "interval";

/** Where it runs: on this computer, or on the remote fleet. */
export type ScheduleTarget = "desktop" | "fleet";

/** A schedule as the cloud keeps it. */
export interface AgentSchedule {
  id: string;
  name: string;
  enabled: boolean;
  target: ScheduleTarget;
  recipeId: string | null;
  profileIds: string[];
  groupId: string | null;
  cadence: ScheduleCadence;
  /** bit0 = Monday … bit6 = Sunday. */
  daysMask: number;
  /** Minutes after midnight, in `timezone`. */
  runAtMinute: number;
  timezone: string;
  runOnceAt: string | null;
  intervalMinutes: number | null;
  nextRunAt: string | null;
  lastRunAt: string | null;
  /** Email the owner when a run fails, and when it works again. */
  notifyEmail: boolean;
  /** An https address the cloud posts the same notices to. */
  webhookUrl: string | null;
  /** Signs each webhook body (`X-Donut-Signature: sha256=<HMAC>`). */
  webhookSecret: string | null;
}

export interface AgentScheduleRun {
  id: string;
  status: string;
  scheduledFor: string;
  label: string;
}

/** What a new or changed schedule sends. The cloud checks every field. */
export interface ScheduleInput {
  name: string;
  recipeId: string;
  target: ScheduleTarget;
  profileIds: string[];
  cadence: ScheduleCadence;
  daysMask?: number;
  runAtMinute?: number;
  timezone?: string;
  runOnceAt?: string;
  intervalMinutes?: number;
  enabled?: boolean;
  notifyEmail?: boolean;
  /** Null clears the webhook. */
  webhookUrl?: string | null;
}

/** The cloud's own limits, so the form never offers what it refuses. */
export const MIN_INTERVAL_MINUTES = 15;
export const MAX_INTERVAL_MINUTES = 10_080;
export const MAX_SCHEDULE_PROFILES = 100;
export const MAX_SCHEDULE_NAME_CHARS = 80;

/** Monday first, as `daysMask` counts. */
export const WEEKDAYS = [0, 1, 2, 3, 4, 5, 6] as const;

export function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** `09:30` from minutes after midnight. */
export function minuteToTime(minute: number): string {
  const hours = Math.floor(minute / 60) % 24;
  const minutes = minute % 60;
  return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}`;
}

/** Minutes after midnight from `09:30`, or null. */
export function timeToMinute(time: string): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(time.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return hours * 60 + minutes;
}

/** A `datetime-local` value (local time) from an ISO instant. */
export function isoToLocalInput(iso: string | null | undefined): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** Only the fields its cadence reads, so the cloud never sees a mix. */
export function scheduleBody(input: ScheduleInput): Record<string, unknown> {
  const body: Record<string, unknown> = {
    name: input.name.trim(),
    recipeId: input.recipeId,
    target: input.target,
    profileIds: input.profileIds,
    cadence: input.cadence,
  };
  if (input.enabled !== undefined) body.enabled = input.enabled;
  if (input.notifyEmail !== undefined) body.notifyEmail = input.notifyEmail;
  if (input.webhookUrl !== undefined) {
    body.webhookUrl = input.webhookUrl?.trim() || null;
  }
  if (input.cadence === "weekly") {
    body.daysMask = input.daysMask;
    body.runAtMinute = input.runAtMinute;
    body.timezone = input.timezone;
  } else if (input.cadence === "once") {
    body.runOnceAt = input.runOnceAt;
  } else {
    body.intervalMinutes = input.intervalMinutes;
  }
  return body;
}

export function listSchedules(): Promise<AgentSchedule[]> {
  return invoke<AgentSchedule[] | null>("list_agent_schedules").then(
    (list) => list ?? [],
  );
}

export function createSchedule(input: ScheduleInput): Promise<AgentSchedule> {
  return invoke<AgentSchedule>("create_agent_schedule", {
    schedule: scheduleBody(input),
  });
}

export function updateSchedule(
  id: string,
  patch: Record<string, unknown>,
): Promise<AgentSchedule> {
  return invoke<AgentSchedule>("update_agent_schedule", { id, patch });
}

export function deleteSchedule(id: string): Promise<void> {
  return invoke<void>("delete_agent_schedule", { id });
}

export function runScheduleNow(id: string): Promise<unknown> {
  return invoke("run_agent_schedule_now", { id });
}

/** Whether `url` is an address the cloud takes for a webhook. It checks again. */
export function isWebhookUrl(url: string): boolean {
  const value = url.trim();
  if (!value) return true;
  try {
    const parsed = new URL(value);
    return (
      parsed.protocol === "https:" &&
      !parsed.username &&
      !parsed.password &&
      value.length <= 2048
    );
  } catch {
    return false;
  }
}

export interface NoticeTestResult {
  email: "sent" | "off" | "failed";
  webhook: "sent" | "off" | "failed";
  webhookStatus: number | null;
}

export function testScheduleNotice(id: string): Promise<NoticeTestResult> {
  return invoke<NoticeTestResult>("test_agent_schedule_notice", { id });
}

export function rotateWebhookSecret(id: string): Promise<AgentSchedule> {
  return updateSchedule(id, { rotateWebhookSecret: true });
}

export function listScheduleRuns(id: string): Promise<AgentScheduleRun[]> {
  return invoke<AgentScheduleRun[] | null>("list_agent_schedule_runs", {
    id,
  }).then((runs) => runs ?? []);
}

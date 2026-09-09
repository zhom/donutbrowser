import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Agent runs: a goal the cloud pursues on one profile.
 *
 * The desktop holds no part of the run's life. It asks the backend to start,
 * read or cancel one and renders back what it is told; the model, the tool
 * loop, the navigation allowlist and the budget accounting all live server
 * side. Mirrors the wire types in `src-tauri/src/agent.rs`, which speaks the
 * same camelCase the backend does, so one spelling survives end to end.
 */

/** `desktop` drives this machine; `fleet` drives a leased host. */
export type AgentTarget = "desktop" | "fleet";

/** Operating systems a leased host can be. */
export type AgentPlatform = "windows" | "macos" | "linux";

export const AGENT_PLATFORMS: readonly AgentPlatform[] = [
  "windows",
  "macos",
  "linux",
];

/** How hard the model is allowed to think per step. */
export type AgentEffort = "standard" | "high";

export const AGENT_EFFORTS: readonly AgentEffort[] = ["standard", "high"];

/**
 * `queued` -> `running` -> `succeeded` | `failed` | `cancelled`.
 *
 * Kept open rather than a closed union: the state machine is the server's, and
 * a status added there must render as itself instead of failing to decode.
 */
export type AgentRunStatus = string;

/** Ceilings a run stops at rather than running until the money does. */
export interface AgentBudgets {
  maxSteps?: number | null;
  maxWallMs?: number | null;
  maxTokens?: number | null;
}

export interface AgentRunView {
  id: string;
  profileId: string;
  target: string;
  platform?: string | null;
  goal: string;
  status: AgentRunStatus;
  model?: string | null;
  effort?: string | null;
  budgets?: AgentBudgets | null;
  allowedHosts?: string[] | null;
  /** The leased session the run is driving, when it is a fleet run. */
  remoteSessionId?: string | null;
  /** Why it ended, e.g. `completed`, `budget`, `navigation_blocked`. */
  closeReason?: string | null;
  /** The refusal code when it failed, one of the `AGENT_*` set. */
  errorCode?: string | null;
  result?: unknown;
  tokensIn?: number | null;
  tokensOut?: number | null;
  costUsd?: number | null;
  steps?: number | null;
  createdAt?: string | null;
  startedAt?: string | null;
  endedAt?: string | null;
  updatedAt?: string | null;
}

/** What kind of thing one transcript entry is. */
export type AgentStepKind = "thought" | "tool" | "error" | "finish" | string;

/**
 * One transcript entry.
 *
 * Only the three fields every kind carries are named. The rest — a thought's
 * text, a tool's name and arguments, an error's message — is whatever the
 * server sent, because the tool set grows without a desktop release. Read it
 * through `agentStepSummary`, never by reaching for one field at a call site.
 */
export interface AgentStep {
  index: number;
  at?: string | null;
  kind: AgentStepKind;
  [field: string]: unknown;
}

export interface AgentRunDetail extends AgentRunView {
  transcript: AgentStep[];
}

export interface AgentRunPage {
  runs: AgentRunView[];
  nextCursor?: string | null;
}

/** The step kinds the API validates. */
export type RecipeStepType =
  | "navigate"
  | "click"
  | "type"
  | "waitFor"
  | "extract"
  | "pressKey"
  | "scroll"
  | "screenshot"
  | "sleep";

/**
 * One step of a recipe, in the shape the API validates.
 *
 * A step is an object naming its kind, not a line of prose: the API refuses
 * anything else, so an editor that produced text could only produce something
 * to be rejected on save. A step that names an element carries exactly one of
 * `selector` or `locator`.
 */
export interface RecipeStep {
  type: RecipeStepType;
  url?: string;
  text?: string;
  name?: string;
  attr?: string;
  key?: string;
  direction?: "up" | "down";
  amount?: number;
  ms?: number;
  timeoutMs?: number;
  fullPage?: boolean;
  selector?: string;
  locator?: {
    role?: string;
    name?: string;
    nameContains?: string;
    text?: string;
    textContains?: string;
  };
}

/** A saved goal: a name and the steps it expands to. */
export interface AgentRecipe {
  id: string;
  name: string;
  steps: RecipeStep[];
  createdAt?: string | null;
  updatedAt?: string | null;
}

export interface StartAgentRunInput {
  profileId: string;
  target: AgentTarget;
  goal: string;
  /** Required by the server for a fleet run; the desktop fills it from the
   * profile's own operating system when the form leaves it unset. */
  platform?: AgentPlatform | null;
  effort?: AgentEffort | null;
  budgets?: AgentBudgets | null;
  allowedHosts?: string[] | null;
}

/**
 * The longest goal the desktop will send. Mirrors `MAX_GOAL_CHARS` in
 * `src-tauri/src/agent.rs`, which refuses anything longer before it costs a
 * round trip.
 */
export const AGENT_GOAL_MAX_CHARS = 4000;

/** Statuses a run cannot leave under its own steam. */
const TERMINAL_STATUSES = new Set(["succeeded", "failed", "cancelled"]);

export function isRunOver(run: { status: AgentRunStatus }): boolean {
  return TERMINAL_STATUSES.has(run.status);
}

/** Whether a run can still be cancelled. */
export function isRunCancellable(run: { status: AgentRunStatus }): boolean {
  return run.status === "queued" || run.status === "running";
}

/**
 * Why a goal would be refused, or null when it is fine.
 *
 * Shared by the form and by anything that submits one, so the button's disabled
 * state and the refusal the backend would produce can never disagree. Counted
 * in characters rather than bytes, matching the Rust ceiling: a goal written in
 * Japanese is not three times shorter than the same goal in English.
 */
export type AgentGoalProblem = "empty" | "tooLong";

export function agentGoalProblem(goal: string): AgentGoalProblem | null {
  const trimmed = goal.trim();
  if (trimmed.length === 0) return "empty";
  if ([...trimmed].length > AGENT_GOAL_MAX_CHARS) return "tooLong";
  return null;
}

/**
 * Turn a textarea of hosts into the list the backend takes.
 *
 * Split on newlines and commas because both are how a person pastes a list.
 * Lowercased and de-duplicated to match `normalise_hosts` in the Rust wire: the
 * two must agree, or the count the form shows is not the count that travels.
 */
export function parseAllowedHosts(raw: string): string[] {
  const out: string[] = [];
  for (const piece of raw.split(/[\n,]/)) {
    const host = piece.trim().toLowerCase();
    if (host.length === 0 || out.includes(host)) continue;
    out.push(host);
  }
  return out;
}

/**
 * The one line that describes a step.
 *
 * Picks the first field the server actually sent from a list of the spellings
 * the tool set uses, so a new tool renders its own words rather than a blank
 * row. Falls back to null and lets the caller name the kind instead — inventing
 * a sentence for a step nobody described would be worse than saying less.
 */
const SUMMARY_FIELDS = [
  "summary",
  "text",
  "message",
  "detail",
  "tool",
  "name",
  "action",
] as const;

export function agentStepSummary(step: AgentStep): string | null {
  for (const field of SUMMARY_FIELDS) {
    const value = step[field];
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
}

export function startAgentRun(
  input: StartAgentRunInput,
): Promise<AgentRunView> {
  return invoke<AgentRunView>("start_agent_run", { input });
}

export function getAgentRuns(params: {
  limit?: number;
  cursor?: string | null;
}): Promise<AgentRunPage> {
  return invoke<AgentRunPage>("get_agent_runs", {
    limit: params.limit ?? null,
    cursor: params.cursor ?? null,
  });
}

export function getAgentRun(runId: string): Promise<AgentRunDetail> {
  return invoke<AgentRunDetail>("get_agent_run", { runId });
}

export function cancelAgentRun(runId: string): Promise<AgentRunView> {
  return invoke<AgentRunView>("cancel_agent_run", { runId });
}

export function getAgentRecipes(): Promise<AgentRecipe[]> {
  return invoke<AgentRecipe[]>("get_agent_recipes");
}

export function createAgentRecipe(
  name: string,
  steps: RecipeStep[],
): Promise<AgentRecipe> {
  return invoke<AgentRecipe>("create_agent_recipe", { name, steps });
}

export function updateAgentRecipe(
  id: string,
  name: string,
  steps: RecipeStep[],
): Promise<AgentRecipe> {
  return invoke<AgentRecipe>("update_agent_recipe", { id, name, steps });
}

export function deleteAgentRecipe(id: string): Promise<boolean> {
  return invoke<boolean>("delete_agent_recipe", { id });
}

/** Subscribe to one run's steps. Idempotent for the run already watched. */
export function startAgentRunEvents(runId: string): Promise<void> {
  return invoke<void>("start_agent_run_events", { runId });
}

/** Unsubscribe. Safe when nothing is running. */
export function stopAgentRunEvents(): Promise<void> {
  return invoke<void>("stop_agent_run_events");
}

/** Which run the desktop is streaming, for a view that mounted after it began. */
export function getAgentRunEventsStatus(): Promise<string | null> {
  return invoke<string | null>("get_agent_run_events_status");
}

/** One step arrived. */
export interface AgentStepEvent {
  runId: string;
  at?: string | null;
  step: AgentStep;
}

/** A run changed status. */
export interface AgentStatusEvent {
  runId: string;
  at?: string | null;
  status: AgentRunStatus;
  closeReason?: string | null;
  errorCode?: string | null;
  result?: unknown;
}

/** Whether steps are currently arriving. */
export interface AgentStreamStatus {
  connected: boolean;
  runId: string;
  reason?: string | null;
}

/**
 * Tauri event names. Steps arrive here rather than being polled for: creating a
 * run answers `queued` and nothing else, so without the stream the page is
 * blind between submit and finish.
 */
export const AGENT_EVENTS = {
  /** One step. Payload: `AgentStepEvent`. */
  step: "agent-run-step",
  /** One status transition. Payload: `AgentStatusEvent`. */
  status: "agent-run-status",
  /** Stream connectivity. Payload: `AgentStreamStatus`. */
  stream: "agent-run-stream",
} as const;

export function onAgentStep(
  handler: (event: AgentStepEvent) => void,
): Promise<UnlistenFn> {
  return listen<AgentStepEvent>(AGENT_EVENTS.step, (event) =>
    handler(event.payload),
  );
}

export function onAgentStatus(
  handler: (event: AgentStatusEvent) => void,
): Promise<UnlistenFn> {
  return listen<AgentStatusEvent>(AGENT_EVENTS.status, (event) =>
    handler(event.payload),
  );
}

export function onAgentStream(
  handler: (status: AgentStreamStatus) => void,
): Promise<UnlistenFn> {
  return listen<AgentStreamStatus>(AGENT_EVENTS.stream, (event) =>
    handler(event.payload),
  );
}

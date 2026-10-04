import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Recipes: saved step sequences an agent can repeat on other profiles.
 * Storage and running stay in Donut cloud; this is the thin client over the
 * commands in `src-tauri/src/recipes.rs`.
 */

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

/** The kinds a recipe may hold, in the order the editor offers them. */
export const STEP_TYPES: readonly RecipeStepType[] = [
  "navigate",
  "click",
  "type",
  "waitFor",
  "extract",
  "pressKey",
  "scroll",
  "screenshot",
  "sleep",
];

/** The kinds that name an element, and so carry exactly one target. */
export const TARGETED: readonly RecipeStepType[] = [
  "click",
  "type",
  "waitFor",
  "extract",
];

/** A targeted step carries exactly one of `selector` or `locator`. */
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

export interface AgentRecipe {
  id: string;
  name: string;
  steps: RecipeStep[];
  createdAt?: string | null;
  updatedAt?: string | null;
}

export function isTargeted(type: RecipeStepType): boolean {
  return TARGETED.includes(type);
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

export interface RecordingStatus {
  profile_id: string | null;
  steps: RecipeStep[];
  recording: boolean;
}

const RECORDING_EVENTS = {
  step: "recipe-recording-step",
  ended: "recipe-recording-ended",
} as const;

export function getRecipeRecording(): Promise<RecordingStatus> {
  return invoke<RecordingStatus>("get_recipe_recording");
}

export function startRecipeRecording(
  profileId: string,
): Promise<RecordingStatus> {
  return invoke<RecordingStatus>("start_recipe_recording", { profileId });
}

export function stopRecipeRecording(): Promise<RecordingStatus> {
  return invoke<RecordingStatus>("stop_recipe_recording");
}

export function onRecordingStep(
  handler: (step: RecipeStep) => void,
): Promise<UnlistenFn> {
  return listen<RecipeStep>(RECORDING_EVENTS.step, (event) => {
    handler(event.payload);
  });
}

export function onRecordingEnded(handler: () => void): Promise<UnlistenFn> {
  return listen(RECORDING_EVENTS.ended, () => {
    handler();
  });
}

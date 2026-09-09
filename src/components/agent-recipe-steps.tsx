"use client";

import { useTranslation } from "react-i18next";
import { LuArrowDown, LuArrowUp, LuTrash2 } from "react-icons/lu";
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
import type { RecipeStep, RecipeStepType } from "@/lib/agent";

/** The kinds a recipe may hold, in the order the editor offers them. */
const STEP_TYPES: RecipeStepType[] = [
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
const TARGETED: RecipeStepType[] = ["click", "type", "waitFor", "extract"];

export function isTargeted(type: RecipeStepType): boolean {
  return TARGETED.includes(type);
}

/** A step of `type` with the fields that kind needs, and nothing else. */
export function emptyStep(type: RecipeStepType): RecipeStep {
  switch (type) {
    case "navigate":
      return { type, url: "" };
    case "click":
      return { type, locator: { name: "" } };
    case "type":
      return { type, text: "", locator: { name: "" } };
    case "waitFor":
      return { type, locator: { name: "" } };
    case "extract":
      return { type, name: "", locator: { name: "" } };
    case "pressKey":
      return { type, key: "" };
    case "scroll":
      return { type, direction: "down" };
    case "screenshot":
      return { type };
    case "sleep":
      return { type, ms: 1000 };
  }
}

/**
 * Whether a step is complete enough to save.
 *
 * Deliberately the same shape the Rust command checks and the API enforces: an
 * unknown kind, or a targeted step with both a selector and a locator or with
 * neither, is refused here rather than sent and rejected.
 */
export function isStepComplete(step: RecipeStep): boolean {
  const hasSelector = Boolean(step.selector?.trim());
  const hasLocator = Boolean(
    step.locator &&
      Object.values(step.locator).some((value) => Boolean(value?.trim())),
  );
  if (isTargeted(step.type) && hasSelector === hasLocator) return false;

  switch (step.type) {
    case "navigate":
      return Boolean(step.url?.trim());
    case "type":
      return Boolean(step.text?.trim());
    case "extract":
      return Boolean(step.name?.trim());
    case "pressKey":
      return Boolean(step.key?.trim());
    case "sleep":
      return typeof step.ms === "number" && step.ms > 0;
    default:
      return true;
  }
}

/** Drop the empty fields the editor keeps for its own inputs. */
export function tidyStep(step: RecipeStep): RecipeStep {
  const tidy: RecipeStep = { type: step.type };
  const copyText = (key: "url" | "text" | "name" | "key" | "selector") => {
    const value = step[key]?.trim();
    if (value) tidy[key] = value;
  };
  copyText("url");
  copyText("text");
  copyText("name");
  copyText("key");
  copyText("selector");
  if (step.locator) {
    const locator: Record<string, string> = {};
    for (const [field, value] of Object.entries(step.locator)) {
      if (value?.trim()) locator[field] = value.trim();
    }
    if (Object.keys(locator).length > 0) tidy.locator = locator;
  }
  if (step.type === "scroll") tidy.direction = step.direction ?? "down";
  if (step.type === "sleep") tidy.ms = step.ms ?? 1000;
  if (step.type === "screenshot" && step.fullPage) tidy.fullPage = true;
  if (step.type === "extract" && step.attr?.trim())
    tidy.attr = step.attr.trim();
  return tidy;
}

/**
 * A recipe's steps, edited as rows rather than as free text.
 *
 * The API validates a typed step object, so a text box could only produce
 * something to be rejected on save. A row per step cannot express an invalid
 * one, and it is the same shape the recorder writes.
 */
export function RecipeStepsEditor({
  steps,
  onChange,
  disabled,
}: {
  steps: RecipeStep[];
  onChange: (steps: RecipeStep[]) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();

  const update = (index: number, next: RecipeStep) => {
    onChange(steps.map((step, i) => (i === index ? next : step)));
  };
  const move = (index: number, by: number) => {
    const target = index + by;
    if (target < 0 || target >= steps.length) return;
    const next = [...steps];
    [next[index], next[target]] = [next[target], next[index]];
    onChange(next);
  };

  return (
    <div className="flex flex-col gap-2" data-slot="agent-recipe-steps">
      {steps.map((step, index) => (
        <div
          // Rows are reordered and removed, so the index is the identity here.
          // eslint-disable-next-line react/no-array-index-key
          key={`step-${index}`}
          className="flex flex-col gap-2 rounded-md border border-border p-2"
          data-slot="agent-recipe-step"
        >
          <div className="flex items-center gap-2">
            <Select
              value={step.type}
              disabled={disabled}
              onValueChange={(value) => {
                update(index, emptyStep(value as RecipeStepType));
              }}
            >
              <SelectTrigger className="h-8 w-40 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STEP_TYPES.map((type) => (
                  <SelectItem key={type} value={type}>
                    {t(`agent.recipes.stepTypes.${type}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div className="flex-1" />
            <Button
              variant="ghost"
              size="icon"
              type="button"
              disabled={disabled || index === 0}
              aria-label={t("agent.recipes.moveStepUp")}
              onClick={() => {
                move(index, -1);
              }}
            >
              <LuArrowUp className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              type="button"
              disabled={disabled || index === steps.length - 1}
              aria-label={t("agent.recipes.moveStepDown")}
              onClick={() => {
                move(index, 1);
              }}
            >
              <LuArrowDown className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              type="button"
              disabled={disabled}
              aria-label={t("agent.recipes.removeStep")}
              onClick={() => {
                onChange(steps.filter((_, i) => i !== index));
              }}
            >
              <LuTrash2 className="size-3.5" />
            </Button>
          </div>

          <StepFields
            step={step}
            disabled={disabled}
            onChange={(next) => {
              update(index, next);
            }}
          />
        </div>
      ))}
    </div>
  );
}

function StepFields({
  step,
  onChange,
  disabled,
}: {
  step: RecipeStep;
  onChange: (step: RecipeStep) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const set = (patch: Partial<RecipeStep>) => {
    onChange({ ...step, ...patch });
  };
  const setLocator = (field: string, value: string) => {
    onChange({
      ...step,
      selector: undefined,
      locator: { ...step.locator, [field]: value },
    });
  };

  return (
    <div className="grid gap-2 @md:grid-cols-2">
      {step.type === "navigate" && (
        <Field label={t("agent.recipes.fields.url")}>
          <Input
            value={step.url ?? ""}
            placeholder="https://example.com"
            disabled={disabled}
            onChange={(event) => {
              set({ url: event.target.value });
            }}
          />
        </Field>
      )}

      {step.type === "type" && (
        <Field label={t("agent.recipes.fields.text")}>
          <Input
            value={step.text ?? ""}
            disabled={disabled}
            onChange={(event) => {
              set({ text: event.target.value });
            }}
          />
        </Field>
      )}

      {step.type === "extract" && (
        <>
          <Field label={t("agent.recipes.fields.name")}>
            <Input
              value={step.name ?? ""}
              placeholder="price"
              disabled={disabled}
              onChange={(event) => {
                set({ name: event.target.value });
              }}
            />
          </Field>
          <Field label={t("agent.recipes.fields.attribute")}>
            <Input
              value={step.attr ?? ""}
              placeholder="href"
              disabled={disabled}
              onChange={(event) => {
                set({ attr: event.target.value });
              }}
            />
          </Field>
        </>
      )}

      {step.type === "pressKey" && (
        <Field label={t("agent.recipes.fields.key")}>
          <Input
            value={step.key ?? ""}
            placeholder="Enter"
            disabled={disabled}
            onChange={(event) => {
              set({ key: event.target.value });
            }}
          />
        </Field>
      )}

      {step.type === "scroll" && (
        <Field label={t("agent.recipes.fields.direction")}>
          <Select
            value={step.direction ?? "down"}
            disabled={disabled}
            onValueChange={(value) => {
              set({ direction: value as "up" | "down" });
            }}
          >
            <SelectTrigger className="h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="down">
                {t("agent.recipes.fields.down")}
              </SelectItem>
              <SelectItem value="up">{t("agent.recipes.fields.up")}</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      )}

      {step.type === "sleep" && (
        <Field label={t("agent.recipes.fields.milliseconds")}>
          <Input
            type="number"
            min={1}
            value={String(step.ms ?? 1000)}
            disabled={disabled}
            onChange={(event) => {
              set({ ms: Number(event.target.value) });
            }}
          />
        </Field>
      )}

      {step.type === "screenshot" && (
        <div className="flex items-center gap-x-2">
          <Checkbox
            id="agent-recipe-fullpage"
            checked={step.fullPage === true}
            disabled={disabled}
            onCheckedChange={(checked) => {
              set({ fullPage: checked === true });
            }}
          />
          <Label htmlFor="agent-recipe-fullpage">
            {t("agent.recipes.fields.fullPage")}
          </Label>
        </div>
      )}

      {isTargeted(step.type) && (
        <>
          <Field label={t("agent.recipes.fields.role")}>
            <Input
              value={step.locator?.role ?? ""}
              placeholder="button"
              disabled={disabled}
              onChange={(event) => {
                setLocator("role", event.target.value);
              }}
            />
          </Field>
          <Field label={t("agent.recipes.fields.elementName")}>
            <Input
              value={step.locator?.name ?? ""}
              placeholder="Buy now"
              disabled={disabled}
              onChange={(event) => {
                setLocator("name", event.target.value);
              }}
            />
          </Field>
          <Field label={t("agent.recipes.fields.selector")}>
            <Input
              value={step.selector ?? ""}
              placeholder="#buy"
              disabled={disabled}
              onChange={(event) => {
                onChange({
                  ...step,
                  locator: undefined,
                  selector: event.target.value,
                });
              }}
            />
          </Field>
        </>
      )}
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <Label className="text-xs">{label}</Label>
      {children}
    </div>
  );
}

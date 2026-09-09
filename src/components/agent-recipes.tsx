"use client";

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { RecipeRecorder } from "@/components/agent-recipe-recorder";
import {
  emptyStep,
  isStepComplete,
  RecipeStepsEditor,
  tidyStep,
} from "@/components/agent-recipe-steps";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { Button } from "@/components/ui/button";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import { Skeleton } from "@/components/ui/skeleton";
import {
  type AgentRecipe,
  createAgentRecipe,
  deleteAgentRecipe,
  type RecipeStep,
  updateAgentRecipe,
} from "@/lib/agent";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";

interface AgentRecipesProps {
  /** Running profiles are the ones a recording can be made from. */
  profiles: BrowserProfile[];
  recipes: AgentRecipe[];
  isLoading: boolean;
  error: unknown;
  /** Re-read the library after any write, so every surface agrees at once. */
  onChanged: () => void;
}

export function AgentRecipes({
  profiles,
  recipes,
  isLoading,
  error,
  onChanged,
}: AgentRecipesProps) {
  const { t } = useTranslation();
  /** The recipe being edited, `"new"` while composing one, or null. */
  const [editing, setEditing] = useState<AgentRecipe | "new" | null>(null);
  /** Steps a recording just produced, waiting to be reviewed and named. */
  const [recorded, setRecorded] = useState<RecipeStep[] | null>(null);
  const [pendingRemoval, setPendingRemoval] = useState<AgentRecipe | null>(
    null,
  );
  const [isRemoving, setIsRemoving] = useState(false);

  const confirmRemoval = useCallback(async () => {
    if (!pendingRemoval) return;
    setIsRemoving(true);
    try {
      await deleteAgentRecipe(pendingRemoval.id);
      showSuccessToast(t("agent.recipes.deleted"));
      setPendingRemoval(null);
      onChanged();
    } catch (removeError) {
      showErrorToast(translateBackendError(t, removeError));
    } finally {
      setIsRemoving(false);
    }
  }, [pendingRemoval, onChanged, t]);

  return (
    <div
      data-slot="agent-recipes"
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      <div className="flex shrink-0 items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">
          {t("agent.recipes.hint")}
        </p>
        <RippleButton
          size="sm"
          className="flex items-center gap-2"
          disabled={editing !== null}
          onClick={() => {
            setEditing("new");
          }}
        >
          <GoPlus className="size-4" />
          {t("agent.recipes.new")}
        </RippleButton>
      </div>

      {error !== null && (
        <p className="shrink-0 rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive-text">
          {translateBackendError(t, error)}
        </p>
      )}

      <RecipeRecorder
        profiles={profiles}
        onRecorded={(steps) => {
          // A recording is a draft, not a saved recipe: it opens the editor so
          // the person names it and sees every step before anything is stored.
          setRecorded(steps);
          setEditing("new");
        }}
      />

      {editing !== null && (
        <RecipeEditor
          recipe={editing === "new" ? null : editing}
          initialSteps={recorded}
          onCancel={() => {
            setEditing(null);
          }}
          onSaved={() => {
            setRecorded(null);
            setEditing(null);
            onChanged();
          }}
        />
      )}

      <FadingScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-1 pr-1">
          {isLoading && recipes.length === 0 ? (
            Array.from({ length: 3 }, (_, index) => (
              <Skeleton
                key={`agent-recipe-skeleton-${index}`}
                className="h-12 w-full"
              />
            ))
          ) : recipes.length === 0 ? (
            <p
              data-slot="agent-recipes-empty"
              className="py-16 text-center text-sm text-muted-foreground"
            >
              {t("agent.recipes.empty")}
            </p>
          ) : (
            recipes.map((recipe) => (
              <div
                key={recipe.id}
                className="flex items-center gap-3 rounded-md border border-border bg-card p-3"
              >
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate text-sm font-medium text-foreground">
                    {recipe.name}
                  </span>
                  <span className="truncate text-xs text-muted-foreground">
                    {t("agent.recipes.stepCount", {
                      steps: recipe.steps.length,
                    })}
                  </span>
                </div>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setEditing(recipe);
                  }}
                >
                  {t("common.buttons.edit")}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setPendingRemoval(recipe);
                  }}
                >
                  {t("common.buttons.delete")}
                </Button>
              </div>
            ))
          )}
        </div>
      </FadingScrollArea>

      <DeleteConfirmationDialog
        isOpen={pendingRemoval !== null}
        onClose={() => {
          setPendingRemoval(null);
        }}
        onConfirm={() => {
          void confirmRemoval();
        }}
        title={t("agent.recipes.deleteTitle", {
          name: pendingRemoval?.name ?? "",
        })}
        description={t("agent.recipes.deleteDescription")}
        confirmButtonText={t("common.buttons.delete")}
        isLoading={isRemoving}
      />
    </div>
  );
}

function RecipeEditor({
  recipe,
  initialSteps,
  onCancel,
  onSaved,
}: {
  recipe: AgentRecipe | null;
  /** Steps a recording produced, for a new recipe that starts from one. */
  initialSteps?: RecipeStep[] | null;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(recipe?.name ?? "");
  const [steps, setSteps] = useState<RecipeStep[]>(
    recipe?.steps ?? initialSteps ?? [],
  );
  const [isSaving, setIsSaving] = useState(false);

  // Editing a different recipe reuses this component, so the fields follow the
  // row the user actually clicked rather than keeping the previous one's text.
  useEffect(() => {
    setName(recipe?.name ?? "");
    setSteps(recipe?.steps ?? initialSteps ?? []);
  }, [recipe, initialSteps]);

  const canSave =
    name.trim().length > 0 &&
    steps.length > 0 &&
    steps.every(isStepComplete) &&
    !isSaving;

  const handleSave = useCallback(async () => {
    if (!canSave) return;
    setIsSaving(true);
    try {
      const payload = steps.map(tidyStep);
      if (recipe) {
        await updateAgentRecipe(recipe.id, name.trim(), payload);
        showSuccessToast(t("agent.recipes.updated"));
      } else {
        await createAgentRecipe(name.trim(), payload);
        showSuccessToast(t("agent.recipes.created"));
      }
      onSaved();
    } catch (saveError) {
      showErrorToast(translateBackendError(t, saveError));
    } finally {
      setIsSaving(false);
    }
  }, [canSave, recipe, name, steps, onSaved, t]);

  return (
    <form
      data-slot="agent-recipe-editor"
      className="flex shrink-0 flex-col gap-3 rounded-md border border-border bg-card p-3"
      onSubmit={(event) => {
        event.preventDefault();
        void handleSave();
      }}
    >
      <div className="flex flex-col gap-2">
        <Label htmlFor="agent-recipe-name">{t("agent.recipes.name")}</Label>
        <Input
          id="agent-recipe-name"
          value={name}
          placeholder={t("agent.recipes.namePlaceholder")}
          onChange={(event) => {
            setName(event.target.value);
          }}
        />
      </div>
      <div className="flex flex-col gap-2">
        <Label>{t("agent.recipes.steps")}</Label>
        <RecipeStepsEditor
          steps={steps}
          onChange={setSteps}
          disabled={isSaving}
        />
        <div className="flex items-center justify-between gap-2">
          <p className="text-xs text-muted-foreground">
            {t("agent.recipes.stepsHint")}
          </p>
          <Button
            variant="outline"
            size="sm"
            type="button"
            disabled={isSaving}
            onClick={() => {
              setSteps([...steps, emptyStep("navigate")]);
            }}
          >
            <GoPlus className="size-3.5" />
            {t("agent.recipes.addStep")}
          </Button>
        </div>
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button variant="outline" size="sm" onClick={onCancel} type="button">
          {t("common.buttons.cancel")}
        </Button>
        <RippleButton type="submit" size="sm" disabled={!canSave}>
          {isSaving ? t("common.buttons.saving") : t("common.buttons.save")}
        </RippleButton>
      </div>
    </form>
  );
}

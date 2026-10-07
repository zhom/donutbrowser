"use client";

import type { UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuCircleDot, LuSquare } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { StatusLight } from "@/components/ui/settings-panel";
import { translateBackendError } from "@/lib/backend-errors";
import {
  getRecipeRecording,
  onRecordingEnded,
  onRecordingStep,
  type RecipeStep,
  startRecipeRecording,
  stopRecipeRecording,
} from "@/lib/recipes";
import { showErrorToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";

/**
 * Record a task once in a real browser and keep the steps.
 *
 * The steps arrive one event at a time while the person works, so the panel
 * shows the recipe growing rather than a spinner that ends in a surprise.
 */
export function RecipeRecorder({
  profiles,
  onRecorded,
}: {
  profiles: BrowserProfile[];
  /** Hand the recorded steps to the editor, where they are reviewed and saved. */
  onRecorded: (steps: RecipeStep[]) => void;
}) {
  const { t } = useTranslation();
  const [profileId, setProfileId] = useState<string | null>(null);
  const [steps, setSteps] = useState<RecipeStep[]>([]);
  const [isRecording, setIsRecording] = useState(false);
  const [isBusy, setIsBusy] = useState(false);
  const unlisten = useRef<UnlistenFn[]>([]);

  // Only a running browser can be recorded: the capture is a live CDP session.
  const runnable = useMemo(
    () =>
      profiles
        .filter((profile) => Boolean(profile.process_id))
        .sort((a, b) => a.name.localeCompare(b.name)),
    [profiles],
  );

  // A recording survives this panel being closed and reopened, so the state
  // comes from the backend rather than from what this component remembers.
  useEffect(() => {
    void getRecipeRecording()
      .then((status) => {
        setIsRecording(status.recording);
        setSteps(status.steps);
        if (status.profile_id) setProfileId(status.profile_id);
      })
      .catch(() => {
        // A backend that cannot answer is simply not recording.
      });
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const step = await onRecordingStep((recorded) => {
        setSteps((previous) => [...previous, recorded]);
      });
      const ended = await onRecordingEnded(() => {
        setIsRecording(false);
      });
      if (cancelled) {
        step();
        ended();
        return;
      }
      unlisten.current = [step, ended];
    })();
    return () => {
      cancelled = true;
      for (const off of unlisten.current) off();
      unlisten.current = [];
    };
  }, []);

  const start = useCallback(async () => {
    if (!profileId) return;
    setIsBusy(true);
    try {
      const status = await startRecipeRecording(profileId);
      setSteps(status.steps);
      setIsRecording(true);
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsBusy(false);
    }
  }, [profileId, t]);

  const stop = useCallback(async () => {
    setIsBusy(true);
    try {
      const status = await stopRecipeRecording();
      setIsRecording(false);
      setSteps(status.steps);
      if (status.steps.length > 0) onRecorded(status.steps);
    } catch (error) {
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsBusy(false);
    }
  }, [onRecorded, t]);

  return (
    <div
      data-slot="agent-recipe-recorder"
      className="flex flex-col gap-3 rounded-xl bg-foreground/3 p-3"
    >
      <p className="text-xs text-muted-foreground">
        {t("agent.recipes.recording.description")}
      </p>

      {isRecording ? (
        <div className="flex items-center gap-2">
          <span className="flex items-center gap-2 text-sm">
            <StatusLight tone="destructive" live />
            {t("agent.recipes.recording.recording")}
          </span>
          <span className="text-xs text-muted-foreground">
            {t("agent.recipes.recording.steps", { count: steps.length })}
          </span>
          <div className="flex-1" />
          <Button
            size="sm"
            variant="soft"
            className="h-8 gap-1.5 rounded-lg text-xs"
            disabled={isBusy}
            aria-busy={isBusy}
            onClick={() => {
              void stop();
            }}
          >
            <LuSquare className="size-3.5" />
            {t("agent.recipes.recording.stop")}
          </Button>
        </div>
      ) : (
        <div className="flex flex-wrap items-end gap-2">
          <div className="flex min-w-48 flex-col gap-1">
            <Label
              htmlFor="agent-recorder-profile"
              className="text-xs font-medium text-muted-foreground"
            >
              {t("agent.recipes.recording.hint")}
            </Label>
            <Select
              value={profileId ?? ""}
              disabled={isBusy || runnable.length === 0}
              onValueChange={setProfileId}
            >
              <SelectTrigger
                id="agent-recorder-profile"
                variant="soft"
                className="h-8 w-full text-xs"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {runnable.map((profile) => (
                  <SelectItem key={profile.id} value={profile.id}>
                    {profile.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <Button
            size="sm"
            className="h-8 gap-1.5 rounded-lg text-xs"
            disabled={isBusy || !profileId}
            aria-busy={isBusy}
            onClick={() => {
              void start();
            }}
          >
            <LuCircleDot className="size-3.5" />
            {t("agent.recipes.recording.record")}
          </Button>
        </div>
      )}

      {!isRecording && steps.length === 0 && profileId && (
        <p className="text-xs text-muted-foreground">
          {t("agent.recipes.recording.empty")}
        </p>
      )}
    </div>
  );
}

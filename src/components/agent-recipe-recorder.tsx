"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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
import type { RecipeStep } from "@/lib/agent";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";

interface RecordingStatus {
  profile_id: string | null;
  steps: RecipeStep[];
  recording: boolean;
}

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
    void invoke<RecordingStatus>("get_recipe_recording")
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
      const step = await listen<RecipeStep>(
        "recipe-recording-step",
        (event) => {
          setSteps((previous) => [...previous, event.payload]);
        },
      );
      const ended = await listen("recipe-recording-ended", () => {
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
      const status = await invoke<RecordingStatus>("start_recipe_recording", {
        profileId,
      });
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
      const status = await invoke<RecordingStatus>("stop_recipe_recording");
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
      className="flex flex-col gap-3 rounded-md border border-border bg-card p-3"
    >
      <p className="text-xs text-muted-foreground">
        {t("agent.recipes.recording.description")}
      </p>

      {isRecording ? (
        <div className="flex items-center gap-2">
          <span className="flex items-center gap-1.5 text-sm">
            <LuCircleDot className="size-3.5 text-destructive" />
            {t("agent.recipes.recording.recording")}
          </span>
          <span className="text-xs text-muted-foreground">
            {t("agent.recipes.recording.steps", { count: steps.length })}
          </span>
          <div className="flex-1" />
          <Button
            size="sm"
            variant="outline"
            disabled={isBusy}
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
            <Label htmlFor="agent-recorder-profile" className="text-xs">
              {t("agent.recipes.recording.hint")}
            </Label>
            <Select
              value={profileId ?? ""}
              disabled={isBusy || runnable.length === 0}
              onValueChange={setProfileId}
            >
              <SelectTrigger
                id="agent-recorder-profile"
                className="h-8 text-xs"
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
            disabled={isBusy || !profileId}
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

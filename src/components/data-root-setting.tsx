"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { translateBackendError } from "@/lib/backend-errors";
import { formatBytes } from "@/lib/format-bytes";
import { showSuccessToast } from "@/lib/toast-utils";
import type { DataRootInfo, DataRootMoveProgress } from "@/types";

/** Join a chosen folder with the app's own folder name, both separator styles. */
function resolveDestination(folder: string, appDirectoryName: string): string {
  const separator = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
  const trimmed = folder.replace(/[\\/]+$/, "");
  return `${trimmed}${separator}${appDirectoryName}`;
}

function percentOf(progress: DataRootMoveProgress): number {
  if (progress.phase === "scanning") return 0;
  if (progress.phase !== "copying") return 100;
  if (progress.total_bytes > 0) {
    return Math.min(
      100,
      Math.round((progress.copied_bytes / progress.total_bytes) * 100),
    );
  }
  if (progress.total_files > 0) {
    return Math.min(
      100,
      Math.round((progress.copied_files / progress.total_files) * 100),
    );
  }
  return 0;
}

export function DataRootSetting() {
  const { t } = useTranslation();
  const [info, setInfo] = useState<DataRootInfo | null>(null);
  const [destination, setDestination] = useState<string | null>(null);
  const [progress, setProgress] = useState<DataRootMoveProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isMoving, setIsMoving] = useState(false);
  const movePending = useRef(false);

  const load = useCallback(async () => {
    try {
      setInfo(await invoke<DataRootInfo>("get_data_root_info"));
    } catch (err) {
      console.error("Failed to read the data directory:", err);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<DataRootMoveProgress>("data-root-move-progress", (event) => {
      setProgress(event.payload);
    }).then((stop) => {
      unlisten = stop;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  const handleChoose = useCallback(async () => {
    if (!info) return;
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: t("settings.dataRoot.chooseTitle"),
      });
      if (typeof picked === "string") {
        setError(null);
        setDestination(resolveDestination(picked, info.app_directory_name));
      }
    } catch (err) {
      console.error("Failed to choose a folder:", err);
      setError(translateBackendError(t, err));
    }
  }, [info, t]);

  const handleUseDefault = useCallback(async () => {
    try {
      setError(null);
      setInfo(await invoke<DataRootInfo>("clear_data_root_choice"));
      showSuccessToast(t("settings.dataRoot.usingDefault"));
    } catch (err) {
      console.error("Failed to clear the data directory choice:", err);
      setError(translateBackendError(t, err));
    }
  }, [t]);

  const handleMove = useCallback(async () => {
    if (!destination || movePending.current) return;
    movePending.current = true;
    setIsMoving(true);
    setError(null);
    try {
      const moved = await invoke<DataRootInfo>("move_data_root", {
        destination,
      });
      setInfo(moved);
      setDestination(null);
      showSuccessToast(t("settings.dataRoot.moved"));
    } catch (err) {
      console.error("Failed to move the data directory:", err);
      setError(translateBackendError(t, err));
      setProgress(null);
    } finally {
      movePending.current = false;
      setIsMoving(false);
    }
  }, [destination, t]);

  if (!info) {
    return (
      <p
        role="status"
        data-slot="data-root-loading"
        className="text-xs text-muted-foreground"
      >
        {t("settings.dataRoot.reading")}
      </p>
    );
  }

  const phaseLabel = progress
    ? t(`settings.dataRoot.phase.${progress.phase}`)
    : null;

  return (
    <div
      data-slot="data-root-setting"
      className="grid gap-3 rounded-lg border p-3"
    >
      <div className="grid gap-1">
        <span className="text-sm font-medium">
          {t("settings.dataRoot.current")}
        </span>
        <span
          data-slot="data-root-active-path"
          className="font-mono text-xs break-all text-muted-foreground select-all"
        >
          {info.active_path}
        </span>
        <span
          data-slot="data-root-size"
          className="text-xs text-muted-foreground"
        >
          {t("settings.dataRoot.usage", {
            size: formatBytes(info.size_bytes),
            files: info.file_count,
          })}
        </span>
      </div>

      <p className="text-xs text-muted-foreground">
        {t("settings.dataRoot.description")}
      </p>

      {info.active_path_missing && (
        <div
          role="alert"
          data-slot="data-root-missing"
          className="grid gap-2 text-xs text-warning-text"
        >
          <p>{t("settings.dataRoot.missing")}</p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="justify-self-start"
            data-slot="data-root-use-default"
            disabled={isMoving}
            onClick={() => void handleUseDefault()}
          >
            {t("settings.dataRoot.useDefault")}
          </Button>
        </div>
      )}

      {info.overridden_by_environment && (
        <p
          role="status"
          data-slot="data-root-overridden"
          className="text-xs text-warning-text"
        >
          {t("settings.dataRoot.environmentOverride")}
        </p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          data-slot="data-root-choose"
          disabled={isMoving}
          onClick={() => void handleChoose()}
        >
          {t("settings.dataRoot.choose")}
        </Button>
        <LoadingButton
          size="sm"
          data-slot="data-root-move"
          isLoading={isMoving}
          disabled={destination === null || isMoving}
          onClick={() => void handleMove()}
        >
          {t("settings.dataRoot.move")}
        </LoadingButton>
      </div>

      {destination && (
        <p className="text-xs text-muted-foreground">
          {t("settings.dataRoot.willMoveTo")}{" "}
          <span
            data-slot="data-root-destination"
            className="font-mono break-all text-foreground"
          >
            {destination}
          </span>
        </p>
      )}

      {progress && isMoving && (
        <div
          role="status"
          aria-live="polite"
          data-slot="data-root-progress"
          className="grid gap-1.5"
        >
          <Progress value={percentOf(progress)} />
          <span className="text-xs text-muted-foreground">
            {phaseLabel}
            {progress.phase === "copying" &&
              ` ${t("settings.dataRoot.copiedCount", {
                copied: progress.copied_files,
                total: progress.total_files,
              })}`}
          </span>
        </div>
      )}

      {info.restart_required && (
        <p
          role="status"
          data-slot="data-root-restart-required"
          className="text-xs text-warning-text"
        >
          {t("settings.dataRoot.restartRequired")}
        </p>
      )}

      {error && (
        <p
          role="alert"
          data-slot="data-root-error"
          className="text-xs text-destructive-text"
        >
          {error}
        </p>
      )}
    </div>
  );
}

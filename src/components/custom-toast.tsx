/**
 * Unified Toast System
 *
 * This module provides a comprehensive toast system that solves styling issues
 * and provides a single, flexible toast component for all use cases.
 *
 * Features:
 * - Proper background styling (no transparency issues)
 * - Loading states with spinners
 * - Progress bars for downloads/updates
 * - Success/error states
 * - Customizable icons and content
 * - Auto-update notifications
 *
 * Usage Examples:
 *
 * Simple loading toast:
 * ```
 * import { showToast } from "./custom-toast";
 * showToast({
 *   type: "loading",
 *   title: "Loading...",
 *   description: "Please wait..."
 * });
 * ```
 *
 * Auto-update toast:
 * ```
 * showAutoUpdateToast("Firefox", "125.0.1");
 * ```
 *
 * Download progress toast:
 * ```
 * showToast({
 *   type: "download",
 *   title: "Downloading Firefox 123.0",
 *   progress: { percentage: 45, speed: "2.5", eta: "30s" }
 * });
 * ```
 *
 * Version update progress:
 * ```
 * showToast({
 *   type: "version-update",
 *   title: "Updating browser versions",
 *   progress: { current: 3, total: 5, found: 12 }
 * });
 * ```
 */
/** biome-ignore-all lint/suspicious/noExplicitAny: TODO */

import { useTranslation } from "react-i18next";
import {
  LuCheckCheck,
  LuDownload,
  LuRefreshCw,
  LuTriangleAlert,
  LuX,
} from "react-icons/lu";
import type { ExternalToast } from "sonner";
import { RippleButton } from "./ui/ripple";

interface BaseToastProps {
  id?: string;
  title: string;
  description?: string;
  duration?: number;
  action?: ExternalToast["action"];
  onCancel?: () => void;
}

interface LoadingToastProps extends BaseToastProps {
  type: "loading";
}

interface SuccessToastProps extends BaseToastProps {
  type: "success";
}

interface ErrorToastProps extends BaseToastProps {
  type: "error";
}

interface DownloadToastProps extends BaseToastProps {
  type: "download";
  stage?:
    | "downloading"
    | "extracting"
    | "verifying"
    | "completed"
    | "downloading (twilight rolling release)";
  progress?: {
    percentage: number;
    speed?: string;
    eta?: string;
  };
}

interface VersionUpdateToastProps extends BaseToastProps {
  type: "version-update";
  progress?: {
    current: number;
    total: number;
    found: number;
    current_browser?: string;
  };
}

interface FetchingToastProps extends BaseToastProps {
  type: "fetching";
  browserName?: string;
}

interface TwilightUpdateToastProps extends BaseToastProps {
  type: "twilight-update";
  browserName?: string;
  hasUpdate?: boolean;
}

interface SyncProgressToastProps extends BaseToastProps {
  type: "sync-progress";
  progress?: {
    completed_files: number;
    total_files: number;
    completed_bytes: number;
    total_bytes: number;
    speed_bytes_per_sec: number;
    eta_seconds: number;
    failed_count: number;
    phase: string;
  };
}

type ToastProps =
  | LoadingToastProps
  | SuccessToastProps
  | ErrorToastProps
  | DownloadToastProps
  | VersionUpdateToastProps
  | FetchingToastProps
  | TwilightUpdateToastProps
  | SyncProgressToastProps;

function formatBytesCompact(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  const value = bytes / 1024 ** i;
  return `${i === 0 ? value : value.toFixed(1)} ${units[i]}`;
}

function formatSpeedCompact(bytesPerSec: number): string {
  if (bytesPerSec >= 1024 * 1024) {
    return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`;
  }
  return `${(bytesPerSec / 1024).toFixed(0)} KB/s`;
}

function formatEtaCompact(seconds: number): string {
  if (seconds >= 3600) {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    return `${h}h ${m}m`;
  }
  if (seconds >= 60) {
    return `${Math.floor(seconds / 60)} min`;
  }
  return `${Math.round(seconds)}s`;
}

function getToastIcon(type: ToastProps["type"], stage?: string) {
  switch (type) {
    case "success":
      return <LuCheckCheck className="flex-shrink-0 w-4 h-4 text-foreground" />;
    case "error":
      return (
        <LuTriangleAlert className="flex-shrink-0 w-4 h-4 text-foreground" />
      );
    case "download":
      if (stage === "completed") {
        return (
          <LuCheckCheck className="flex-shrink-0 w-4 h-4 text-foreground" />
        );
      }
      return <LuDownload className="flex-shrink-0 w-4 h-4 text-foreground" />;

    case "version-update":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 animate-spin text-foreground" />
      );
    case "fetching":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 animate-spin text-foreground" />
      );
    case "twilight-update":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 animate-spin text-foreground" />
      );
    case "sync-progress":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 animate-spin text-foreground" />
      );
    case "loading":
      return (
        <div className="flex-shrink-0 w-4 h-4 rounded-full border-2 animate-spin border-foreground border-t-transparent" />
      );
    default:
      return (
        <div className="flex-shrink-0 w-4 h-4 rounded-full border-2 animate-spin border-foreground border-t-transparent" />
      );
  }
}

export function UnifiedToast(props: ToastProps) {
  const { t } = useTranslation();
  const { title, description, type, action, onCancel } = props;
  const stage = "stage" in props ? props.stage : undefined;
  const progress = "progress" in props ? props.progress : undefined;

  return (
    <div className="flex items-start p-3 w-96 rounded-lg border shadow-lg bg-card border-border text-card-foreground">
      <div className="mr-3 mt-0.5">{getToastIcon(type, stage)}</div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center justify-between">
          <p className="text-sm font-semibold leading-tight text-foreground">
            {title}
          </p>
          {onCancel && (
            <button
              type="button"
              onClick={onCancel}
              className="ml-2 p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors flex-shrink-0"
              aria-label={t("common.buttons.cancel")}
            >
              <LuX className="w-3 h-3" />
            </button>
          )}
        </div>

        {/* Download progress */}
        {type === "download" &&
          progress &&
          "percentage" in progress &&
          stage === "downloading" && (
            <div className="mt-2 space-y-1">
              <div className="flex justify-between items-center">
                <p className="flex-1 min-w-0 text-xs text-muted-foreground">
                  {progress.percentage.toFixed(1)}%
                  {progress.speed && ` • ${progress.speed} MB/s`}
                  {progress.eta && ` • ${progress.eta} remaining`}
                </p>
              </div>
              <div className="w-full bg-muted rounded-full h-1.5">
                <div
                  className="bg-foreground h-1.5 rounded-full transition-all duration-300"
                  style={{ width: `${progress.percentage}%` }}
                />
              </div>
            </div>
          )}

        {/* Version update progress */}
        {type === "version-update" &&
          progress &&
          "current_browser" in progress && (
            <div className="mt-2 space-y-1">
              <p className="text-xs text-muted-foreground">
                {progress.current_browser && (
                  <>Looking for updates for {progress.current_browser}</>
                )}
              </p>
              <div className="flex items-center space-x-2">
                <div className="flex-1 bg-muted rounded-full h-1.5 min-w-0">
                  <div
                    className="bg-foreground h-1.5 rounded-full transition-all duration-300"
                    style={{
                      width: `${(progress.current / progress.total) * 100}%`,
                    }}
                  />
                </div>
                <span className="w-8 text-xs text-right whitespace-nowrap text-muted-foreground shrink-0">
                  {progress.current}/{progress.total}
                </span>
              </div>
            </div>
          )}

        {/* Sync progress */}
        {type === "sync-progress" &&
          progress &&
          "completed_files" in progress && (
            <div className="mt-1">
              <p className="text-xs text-muted-foreground">
                {progress.phase === "uploading" ? "Uploading" : "Downloading"}{" "}
                {progress.completed_files}/{progress.total_files} files
                {" \u2022 "}
                {formatBytesCompact(progress.completed_bytes)} /{" "}
                {formatBytesCompact(progress.total_bytes)}
                {progress.speed_bytes_per_sec > 0 && (
                  <>
                    {" \u2022 "}
                    {formatSpeedCompact(progress.speed_bytes_per_sec)}
                  </>
                )}
                {progress.eta_seconds > 0 &&
                  progress.completed_files < progress.total_files && (
                    <>
                      {" \u2022 ~"}
                      {formatEtaCompact(progress.eta_seconds)} remaining
                    </>
                  )}
              </p>
              {progress.failed_count > 0 && (
                <p className="text-xs text-destructive mt-0.5">
                  {progress.failed_count} file(s) failed
                </p>
              )}
            </div>
          )}

        {/* Twilight update progress */}
        {type === "twilight-update" && (
          <div className="mt-2">
            <p className="text-xs text-muted-foreground">
              {"hasUpdate" in props && props.hasUpdate
                ? "New twilight build available for download"
                : "Checking for twilight updates..."}
            </p>
            {props.browserName && (
              <p className="mt-1 text-xs text-muted-foreground">
                {props.browserName} • Rolling Release
              </p>
            )}
          </div>
        )}

        {/* Description */}
        {description && (
          <p className="mt-1 text-xs leading-tight text-muted-foreground">
            {description}
          </p>
        )}

        {/* Stage-specific descriptions for downloads */}
        {type === "download" && !description && (
          <>
            {stage === "extracting" && (
              <p className="mt-1 text-xs text-muted-foreground">
                Extracting browser files... Please do not close the app.
              </p>
            )}
            {stage === "verifying" && (
              <p className="mt-1 text-xs text-muted-foreground">
                Verifying browser files...
              </p>
            )}
            {stage === "downloading (twilight rolling release)" && (
              <p className="mt-1 text-xs text-muted-foreground">
                Downloading rolling release build...
              </p>
            )}
          </>
        )}
        {action &&
          "onClick" in (action as { onClick?: () => void; label?: string }) &&
          "label" in (action as { onClick?: () => void; label?: string }) && (
            <div className="mt-2 w-full">
              <RippleButton
                size="sm"
                className="ml-auto"
                onClick={
                  (action as { onClick: () => void; label: string }).onClick
                }
              >
                {(action as { onClick: () => void; label: string }).label}
              </RippleButton>
            </div>
          )}
      </div>
    </div>
  );
}

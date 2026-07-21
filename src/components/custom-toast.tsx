import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useTranslation } from "react-i18next";
import {
  LuCheckCheck,
  LuDownload,
  LuRefreshCw,
  LuTriangleAlert,
  LuX,
} from "react-icons/lu";
import type { ExternalToast } from "sonner";
import { MOTION_EASE_OUT } from "@/lib/motion";
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
  stage?: "downloading" | "extracting" | "verifying" | "completed";
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

function ProgressBar({
  percentage,
  className = "w-full",
}: {
  percentage: number;
  className?: string;
}) {
  return (
    <div className={`h-1.5 rounded-full bg-muted ${className}`}>
      <div
        className="h-1.5 rounded-full bg-foreground"
        style={{ width: `${percentage}%` }}
      />
    </div>
  );
}

function getToastIcon(type: ToastProps["type"], stage?: string) {
  switch (type) {
    case "success":
      return <LuCheckCheck className="size-4 shrink-0 text-foreground" />;
    case "error":
      return <LuTriangleAlert className="size-4 shrink-0 text-foreground" />;
    case "download":
      if (stage === "completed") {
        return <LuCheckCheck className="size-4 shrink-0 text-foreground" />;
      }
      return <LuDownload className="size-4 shrink-0 text-foreground" />;

    case "version-update":
      return (
        <LuRefreshCw className="size-4 shrink-0 animate-spin text-foreground" />
      );
    case "fetching":
      return (
        <LuRefreshCw className="size-4 shrink-0 animate-spin text-foreground" />
      );
    case "sync-progress":
      return (
        <LuRefreshCw className="size-4 shrink-0 animate-spin text-foreground" />
      );
    case "loading":
      return (
        <div className="size-4 shrink-0 animate-spin rounded-full border-2 border-foreground border-t-transparent" />
      );
    default:
      return (
        <div className="size-4 shrink-0 animate-spin rounded-full border-2 border-foreground border-t-transparent" />
      );
  }
}

export function UnifiedToast(props: ToastProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const { title, description, type, action, onCancel } = props;
  const stage = "stage" in props ? props.stage : undefined;
  const progress = "progress" in props ? props.progress : undefined;
  const stateKey = `${type}:${stage ?? "default"}`;

  return (
    <div className="flex w-full max-w-md items-start rounded-lg border border-border bg-card p-3 text-card-foreground shadow-lg">
      <div className="mt-0.5 mr-3">
        <AnimatePresence initial={false} mode="wait">
          <motion.div
            key={`icon-${stateKey}`}
            initial={{ opacity: 0, scale: reduceMotion ? 1 : 0.92 }}
            animate={{
              opacity: 1,
              scale: 1,
              transition: {
                duration: reduceMotion ? 0.15 : 0.16,
                ease: MOTION_EASE_OUT,
              },
            }}
            exit={{
              opacity: 0,
              scale: reduceMotion ? 1 : 0.92,
              transition: {
                duration: reduceMotion ? 0.15 : 0.12,
                ease: MOTION_EASE_OUT,
              },
            }}
          >
            {getToastIcon(type, stage)}
          </motion.div>
        </AnimatePresence>
      </div>
      <AnimatePresence initial={false} mode="wait">
        <motion.div
          key={stateKey}
          initial={{ opacity: 0, y: reduceMotion ? 0 : 4 }}
          animate={{
            opacity: 1,
            y: 0,
            transition: {
              duration: reduceMotion ? 0.15 : 0.16,
              ease: MOTION_EASE_OUT,
            },
          }}
          exit={{
            opacity: 0,
            y: reduceMotion ? 0 : -4,
            transition: {
              duration: reduceMotion ? 0.15 : 0.12,
              ease: MOTION_EASE_OUT,
            },
          }}
          className="min-w-0 flex-1"
        >
          <div className="flex items-center justify-between">
            <p className="text-sm/tight font-semibold text-foreground">
              {title}
            </p>
            {onCancel && (
              <button
                type="button"
                onClick={onCancel}
                className="ml-2 shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                aria-label={t("common.buttons.cancel")}
              >
                <LuX className="size-3" />
              </button>
            )}
          </div>

          {/* Download progress */}
          {type === "download" &&
            progress &&
            "percentage" in progress &&
            stage === "downloading" && (
              <div className="mt-2 space-y-1">
                <div className="flex items-center justify-between">
                  <p className="min-w-0 flex-1 text-xs text-muted-foreground">
                    {progress.percentage.toFixed(1)}%
                    {progress.speed && ` • ${progress.speed} MB/s`}
                    {progress.eta &&
                      ` • ${t("toasts.progress.remaining", { time: progress.eta })}`}
                  </p>
                </div>
                <ProgressBar percentage={progress.percentage} />
              </div>
            )}

          {/* Extraction / verification progress. Extraction reports a real
            percentage for most archive formats; when none is available yet
            (or the format can't measure progress) show an indeterminate bar. */}
          {type === "download" &&
            (stage === "extracting" || stage === "verifying") && (
              <div className="mt-2 space-y-1">
                {stage === "extracting" &&
                progress &&
                "percentage" in progress &&
                progress.percentage > 0 ? (
                  <>
                    <p className="text-xs text-muted-foreground">
                      {progress.percentage.toFixed(1)}%
                    </p>
                    <ProgressBar percentage={progress.percentage} />
                  </>
                ) : (
                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                    <div className="h-1.5 w-1/3 animate-progress-indeterminate rounded-full bg-foreground" />
                  </div>
                )}
              </div>
            )}

          {/* Version update progress */}
          {type === "version-update" &&
            progress &&
            "current_browser" in progress && (
              <div className="mt-2 space-y-1">
                <p className="text-xs text-muted-foreground">
                  {progress.current_browser &&
                    t("versionUpdater.toast.lookingForUpdates", {
                      browser: progress.current_browser,
                    })}
                </p>
                <div className="flex items-center gap-x-2">
                  <ProgressBar
                    percentage={(progress.current / progress.total) * 100}
                    className="min-w-0 flex-1"
                  />
                  <span className="w-8 shrink-0 text-right text-xs whitespace-nowrap text-muted-foreground">
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
                  {progress.phase === "uploading"
                    ? t("appUpdate.toast.uploading")
                    : t("appUpdate.toast.downloading")}{" "}
                  {t("toasts.progress.filesProgress", {
                    completed: progress.completed_files,
                    total: progress.total_files,
                  })}
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
                    progress.completed_files < progress.total_files &&
                    ` \u2022 ${t("toasts.progress.remaining", {
                      time: `~${formatEtaCompact(progress.eta_seconds)}`,
                    })}`}
                </p>
                {progress.failed_count > 0 && (
                  <p className="mt-0.5 text-xs text-destructive">
                    {t("toasts.progress.filesFailed", {
                      count: progress.failed_count,
                    })}
                  </p>
                )}
              </div>
            )}

          {/* Description */}
          {description && (
            <p className="mt-1 text-xs/tight text-muted-foreground">
              {description}
            </p>
          )}

          {/* Stage-specific descriptions for downloads */}
          {type === "download" && !description && (
            <>
              {stage === "extracting" && (
                <p className="mt-1 text-xs text-muted-foreground">
                  {t("browserDownload.toast.extracting")}
                </p>
              )}
              {stage === "verifying" && (
                <p className="mt-1 text-xs text-muted-foreground">
                  {t("browserDownload.toast.verifying")}
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
        </motion.div>
      </AnimatePresence>
    </div>
  );
}

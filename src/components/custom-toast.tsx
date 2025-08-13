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

import {
  LuCheckCheck,
  LuDownload,
  LuRefreshCw,
  LuRocket,
  LuTriangleAlert,
} from "react-icons/lu";
import type { ExternalToast } from "sonner";
import { RippleButton } from "./ui/ripple";

interface BaseToastProps {
  id?: string;
  title: string;
  description?: string;
  duration?: number;
  action?: ExternalToast["action"];
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

type ToastProps =
  | LoadingToastProps
  | SuccessToastProps
  | ErrorToastProps
  | DownloadToastProps
  | VersionUpdateToastProps
  | FetchingToastProps
  | TwilightUpdateToastProps;

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
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 text-foreground animate-spin" />
      );
    case "fetching":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 text-foreground animate-spin" />
      );
    case "twilight-update":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 text-foreground animate-spin" />
      );
    case "loading":
      return (
        <div className="flex-shrink-0 w-4 h-4 rounded-full border-2 border-foreground animate-spin border-t-transparent" />
      );
    default:
      return (
        <div className="flex-shrink-0 w-4 h-4 rounded-full border-2 border-foreground animate-spin border-t-transparent" />
      );
  }
}

export function UnifiedToast(props: ToastProps) {
  const { title, description, type, action } = props;
  const stage = "stage" in props ? props.stage : undefined;
  const progress = "progress" in props ? props.progress : undefined;

  return (
    <div className="flex items-start p-4 w-full max-w-md bg-card rounded-lg border border-border shadow-lg text-card-foreground">
      <div className="mr-3 mt-0.5">{getToastIcon(type, stage)}</div>
      <div className="flex-1 min-w-0">
        <p className="text-sm font-semibold text-foreground leading-tight">
          {title}
        </p>

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
                <span className="w-8 text-xs text-right text-muted-foreground whitespace-nowrap shrink-0">
                  {progress.current}/{progress.total}
                </span>
              </div>
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
                Extracting browser files...
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
          "onClick" in (action as any) &&
          "label" in (action as any) && (
            <div className="mt-2 w-full">
              <RippleButton
                size="sm"
                className="ml-auto"
                onClick={(action as any).onClick}
              >
                {(action as any).label}
              </RippleButton>
            </div>
          )}
      </div>
    </div>
  );
}

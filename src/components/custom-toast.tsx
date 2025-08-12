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

import {
  LuCheckCheck,
  LuDownload,
  LuRefreshCw,
  LuRocket,
  LuTriangleAlert,
} from "react-icons/lu";
import type { ExternalToast } from "sonner";
import { Button } from "./ui/button";
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
      return <LuCheckCheck className="flex-shrink-0 w-4 h-4 text-green-500" />;
    case "error":
      return <LuTriangleAlert className="flex-shrink-0 w-4 h-4 text-red-500" />;
    case "download":
      if (stage === "completed") {
        return (
          <LuCheckCheck className="flex-shrink-0 w-4 h-4 text-green-500" />
        );
      }
      return <LuDownload className="flex-shrink-0 w-4 h-4 text-blue-500" />;

    case "version-update":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 text-blue-500 animate-spin" />
      );
    case "fetching":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 text-blue-500 animate-spin" />
      );
    case "twilight-update":
      return (
        <LuRefreshCw className="flex-shrink-0 w-4 h-4 text-purple-500 animate-spin" />
      );
    case "loading":
      return (
        <div className="flex-shrink-0 w-4 h-4 rounded-full border-2 border-blue-500 animate-spin border-t-transparent" />
      );
    default:
      return (
        <div className="flex-shrink-0 w-4 h-4 rounded-full border-2 border-blue-500 animate-spin border-t-transparent" />
      );
  }
}

export function UnifiedToast(props: ToastProps) {
  const { title, description, type, action } = props;
  const stage = "stage" in props ? props.stage : undefined;
  const progress = "progress" in props ? props.progress : undefined;

  // Check if this is an auto-update toast
  const isAutoUpdate = title.includes("update started");

  return (
    <div
      className={`flex items-start p-3 w-96 rounded-lg border shadow-lg ${
        isAutoUpdate
          ? "bg-emerald-50 border-emerald-200 dark:bg-emerald-950 dark:border-emerald-800"
          : "bg-white border-gray-200 dark:bg-gray-800 dark:border-gray-700"
      }`}
      data-toast-type={isAutoUpdate ? "auto-update" : "default"}
    >
      <div className="mr-3 mt-0.5">
        {isAutoUpdate ? (
          <LuRocket className="flex-shrink-0 w-4 h-4 text-emerald-500" />
        ) : (
          getToastIcon(type, stage)
        )}
      </div>
      <div className="flex-1 min-w-0">
        <p
          className={`text-sm font-medium leading-tight ${
            isAutoUpdate
              ? "text-emerald-900 dark:text-emerald-100"
              : "text-gray-900 dark:text-white"
          }`}
        >
          {title}
        </p>

        {/* Download progress */}
        {type === "download" &&
          progress &&
          "percentage" in progress &&
          stage === "downloading" && (
            <div className="mt-2 space-y-1">
              <div className="flex justify-between items-center">
                <p className="flex-1 min-w-0 text-xs text-gray-600 dark:text-gray-300">
                  {progress.percentage.toFixed(1)}%
                  {progress.speed && ` • ${progress.speed} MB/s`}
                  {progress.eta && ` • ${progress.eta} remaining`}
                </p>
              </div>
              <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
                <div
                  className="bg-blue-500 h-1.5 rounded-full transition-all duration-300"
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
              <p className="text-xs text-gray-600 dark:text-gray-300">
                {progress.current_browser && (
                  <>Looking for updates for {progress.current_browser}</>
                )}
              </p>
              <div className="flex items-center space-x-2">
                <div className="flex-1 bg-gray-200 dark:bg-gray-700 rounded-full h-1.5 min-w-0">
                  <div
                    className="bg-blue-500 h-1.5 rounded-full transition-all duration-300"
                    style={{
                      width: `${(progress.current / progress.total) * 100}%`,
                    }}
                  />
                </div>
                <span className="w-8 text-xs text-right text-gray-500 whitespace-nowrap dark:text-gray-400 shrink-0">
                  {progress.current}/{progress.total}
                </span>
              </div>
            </div>
          )}

        {/* Twilight update progress */}
        {type === "twilight-update" && (
          <div className="mt-2">
            <p className="text-xs text-gray-600 dark:text-gray-300">
              {"hasUpdate" in props && props.hasUpdate
                ? "New twilight build available for download"
                : "Checking for twilight updates..."}
            </p>
            {props.browserName && (
              <p className="mt-1 text-xs text-purple-600 dark:text-purple-400">
                {props.browserName} • Rolling Release
              </p>
            )}
          </div>
        )}

        {/* Description */}
        {description && (
          <p
            className={`mt-1 text-xs leading-tight ${
              isAutoUpdate
                ? "text-emerald-700 dark:text-emerald-300"
                : "text-gray-600 dark:text-gray-300"
            }`}
          >
            {description}
          </p>
        )}

        {/* Stage-specific descriptions for downloads */}
        {type === "download" && !description && (
          <>
            {stage === "extracting" && (
              <p className="mt-1 text-xs text-gray-600 dark:text-gray-300">
                Extracting browser files...
              </p>
            )}
            {stage === "verifying" && (
              <p className="mt-1 text-xs text-gray-600 dark:text-gray-300">
                Verifying browser files...
              </p>
            )}
            {stage === "downloading (twilight rolling release)" && (
              <p className="mt-1 text-xs text-purple-600 dark:text-purple-400">
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

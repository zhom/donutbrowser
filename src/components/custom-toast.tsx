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

import React from "react";
import {
  LuCheckCheck,
  LuDownload,
  LuRefreshCw,
  LuTriangleAlert,
} from "react-icons/lu";

interface BaseToastProps {
  id?: string;
  title: string;
  description?: string;
  duration?: number;
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
      return <LuCheckCheck className="h-4 w-4 text-green-500 flex-shrink-0" />;
    case "error":
      return <LuTriangleAlert className="h-4 w-4 text-red-500 flex-shrink-0" />;
    case "download":
      if (stage === "completed") {
        return (
          <LuCheckCheck className="h-4 w-4 text-green-500 flex-shrink-0" />
        );
      }
      return <LuDownload className="h-4 w-4 text-blue-500 flex-shrink-0" />;
    case "version-update":
      return (
        <LuRefreshCw className="h-4 w-4 text-blue-500 animate-spin flex-shrink-0" />
      );
    case "fetching":
      return (
        <LuRefreshCw className="h-4 w-4 text-blue-500 animate-spin flex-shrink-0" />
      );
    case "twilight-update":
      return (
        <LuRefreshCw className="h-4 w-4 text-purple-500 animate-spin flex-shrink-0" />
      );
    default:
      return (
        <div className="animate-spin rounded-full h-4 w-4 border-2 border-blue-500 border-t-transparent flex-shrink-0" />
      );
  }
}

export function UnifiedToast(props: ToastProps) {
  const { title, description, type } = props;
  const stage = "stage" in props ? props.stage : undefined;
  const progress = "progress" in props ? props.progress : undefined;

  return (
    <div className="flex items-start w-full bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg p-3 shadow-lg">
      <div className="mr-3 mt-0.5">{getToastIcon(type, stage)}</div>
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium text-gray-900 dark:text-white leading-tight">
          {title}
        </p>

        {/* Download progress */}
        {type === "download" &&
          progress &&
          "percentage" in progress &&
          stage === "downloading" && (
            <div className="mt-2 space-y-1">
              <div className="flex justify-between items-center">
                <p className="text-xs text-gray-600 dark:text-gray-300 min-w-0 flex-1">
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
        {type === "version-update" && progress && "found" in progress && (
          <div className="mt-2 space-y-1">
            <p className="text-xs text-gray-600 dark:text-gray-300">
              {progress.found} new versions found so far
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
              <span className="text-xs text-gray-500 dark:text-gray-400 whitespace-nowrap shrink-0 w-8 text-right">
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
              <p className="text-xs text-purple-600 dark:text-purple-400 mt-1">
                {props.browserName} • Rolling Release
              </p>
            )}
          </div>
        )}

        {/* Description */}
        {description && (
          <p className="mt-1 text-xs text-gray-600 dark:text-gray-300 leading-tight">
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
                Verifying installation...
              </p>
            )}
            {stage === "downloading (twilight rolling release)" && (
              <p className="mt-1 text-xs text-purple-600 dark:text-purple-400">
                Downloading rolling release build...
              </p>
            )}
          </>
        )}
      </div>
    </div>
  );
}

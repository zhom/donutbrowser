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
import { toast as sonnerToast } from "sonner";
import {
  LuCheckCheck,
  LuTriangleAlert,
  LuDownload,
  LuRefreshCw,
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
  };
}

interface FetchingToastProps extends BaseToastProps {
  type: "fetching";
  browserName?: string;
}

type ToastProps =
  | LoadingToastProps
  | SuccessToastProps
  | ErrorToastProps
  | DownloadToastProps
  | VersionUpdateToastProps
  | FetchingToastProps;

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
    case "loading":
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
          </>
        )}
      </div>
    </div>
  );
}

// Unified toast function
export function showToast(props: ToastProps & { id?: string }) {
  const toastId = props.id ?? `toast-${props.type}-${Date.now()}`;

  // Improved duration logic - make toasts disappear more quickly
  let duration: number;
  if (props.duration !== undefined) {
    duration = props.duration;
  } else {
    switch (props.type) {
      case "loading":
      case "fetching":
        duration = 10000; // 10 seconds instead of infinite
        break;
      case "download":
        // Only keep infinite for active downloading, others get shorter durations
        if ("stage" in props && props.stage === "downloading") {
          duration = Number.POSITIVE_INFINITY;
        } else if ("stage" in props && props.stage === "completed") {
          duration = 3000; // Shorter duration for completed downloads
        } else {
          duration = 8000; // 8 seconds for extracting/verifying
        }
        break;
      case "version-update":
        duration = 15000; // 15 seconds instead of infinite
        break;
      case "success":
        duration = 3000; // Shorter success duration
        break;
      case "error":
        duration = 5000; // Reasonable error duration
        break;
      default:
        duration = 4000;
    }
  }

  if (props.type === "success") {
    sonnerToast.success(<UnifiedToast {...props} />, {
      id: toastId,
      duration,
      style: {
        background: "transparent",
        border: "none",
        boxShadow: "none",
        padding: 0,
      },
    });
  } else if (props.type === "error") {
    sonnerToast.error(<UnifiedToast {...props} />, {
      id: toastId,
      duration,
      style: {
        background: "transparent",
        border: "none",
        boxShadow: "none",
        padding: 0,
      },
    });
  } else {
    sonnerToast.custom((id) => <UnifiedToast {...props} />, {
      id: toastId,
      duration,
      style: {
        background: "transparent",
        border: "none",
        boxShadow: "none",
        padding: 0,
      },
    });
  }

  return toastId;
}

// Convenience functions for common use cases
export function showLoadingToast(
  title: string,
  options?: {
    id?: string;
    description?: string;
    duration?: number;
  }
) {
  return showToast({
    type: "loading",
    title,
    ...options,
  });
}

export function showDownloadToast(
  browserName: string,
  version: string,
  stage: "downloading" | "extracting" | "verifying" | "completed",
  progress?: { percentage: number; speed?: string; eta?: string },
  options?: { suppressCompletionToast?: boolean }
) {
  const title =
    stage === "completed"
      ? `${browserName} ${version} downloaded successfully!`
      : stage === "downloading"
        ? `Downloading ${browserName} ${version}`
        : stage === "extracting"
          ? `Extracting ${browserName} ${version}`
          : `Verifying ${browserName} ${version}`;

  // Don't show completion toast if suppressed (for auto-update scenarios)
  if (stage === "completed" && options?.suppressCompletionToast) {
    dismissToast(`download-${browserName.toLowerCase()}-${version}`);
    return;
  }

  return showToast({
    type: "download",
    title,
    stage,
    progress,
    id: `download-${browserName.toLowerCase()}-${version}`,
  });
}

export function showVersionUpdateToast(
  title: string,
  options?: {
    id?: string;
    description?: string;
    progress?: {
      current: number;
      total: number;
      found: number;
    };
    duration?: number;
  }
) {
  return showToast({
    type: "version-update",
    title,
    ...options,
  });
}

export function showFetchingToast(
  browserName: string,
  options?: {
    id?: string;
    description?: string;
    duration?: number;
  }
) {
  return showToast({
    type: "fetching",
    title: `Checking for new ${browserName} versions...`,
    description:
      options?.description ?? "Fetching latest release information...",
    browserName,
    ...options,
  });
}

export function showSuccessToast(
  title: string,
  options?: {
    id?: string;
    description?: string;
    duration?: number;
  }
) {
  return showToast({
    type: "success",
    title,
    ...options,
  });
}

export function showErrorToast(
  title: string,
  options?: {
    id?: string;
    description?: string;
    duration?: number;
  }
) {
  return showToast({
    type: "error",
    title,
    ...options,
  });
}

// Generic helper for dismissing toasts
export function dismissToast(id: string) {
  sonnerToast.dismiss(id);
}

// Dismiss all toasts
export function dismissAllToasts() {
  sonnerToast.dismiss();
}

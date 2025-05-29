import { UnifiedToast } from "@/components/custom-toast";
import React from "react";
import { toast as sonnerToast } from "sonner";

// Define toast types locally
export interface BaseToastProps {
  id?: string;
  title: string;
  description?: string;
  duration?: number;
}

export interface LoadingToastProps extends BaseToastProps {
  type: "loading";
}

export interface SuccessToastProps extends BaseToastProps {
  type: "success";
}

export interface ErrorToastProps extends BaseToastProps {
  type: "error";
}

export interface DownloadToastProps extends BaseToastProps {
  type: "download";
  stage?: "downloading" | "extracting" | "verifying" | "completed";
  progress?: {
    percentage: number;
    speed?: string;
    eta?: string;
  };
}

export interface VersionUpdateToastProps extends BaseToastProps {
  type: "version-update";
  progress?: {
    current: number;
    total: number;
    found: number;
  };
}

export interface FetchingToastProps extends BaseToastProps {
  type: "fetching";
  browserName?: string;
}

export type ToastProps =
  | LoadingToastProps
  | SuccessToastProps
  | ErrorToastProps
  | DownloadToastProps
  | VersionUpdateToastProps
  | FetchingToastProps;

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
    sonnerToast.success(React.createElement(UnifiedToast, props), {
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
    sonnerToast.error(React.createElement(UnifiedToast, props), {
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
    sonnerToast.custom((id) => React.createElement(UnifiedToast, props), {
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
  },
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
  options?: { suppressCompletionToast?: boolean },
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
  },
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
  },
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
  },
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
  },
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

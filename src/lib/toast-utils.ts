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

export interface VersionUpdateToastProps extends BaseToastProps {
  type: "version-update";
  progress?: {
    current: number;
    total: number;
    found: number;
    current_browser?: string;
  };
}

export interface FetchingToastProps extends BaseToastProps {
  type: "fetching";
  browserName?: string;
}

export interface TwilightUpdateToastProps extends BaseToastProps {
  type: "twilight-update";
  browserName?: string;
  hasUpdate?: boolean;
}

export type ToastProps =
  | LoadingToastProps
  | SuccessToastProps
  | ErrorToastProps
  | DownloadToastProps
  | VersionUpdateToastProps
  | FetchingToastProps
  | TwilightUpdateToastProps;

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
          duration = 3000;
        } else {
          duration = 20000;
        }
        break;
      case "version-update":
        duration = 15000;
        break;
      case "twilight-update":
        duration = 10000;
        break;
      case "success":
        duration = 3000;
        break;
      case "error":
        duration = 10000;
        break;
      default:
        duration = 5000;
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
        zIndex: 99999,
        pointerEvents: "auto",
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
        zIndex: 99999,
        pointerEvents: "auto",
      },
    });
  } else {
    sonnerToast.custom(() => React.createElement(UnifiedToast, props), {
      id: toastId,
      duration,
      style: {
        background: "transparent",
        border: "none",
        boxShadow: "none",
        padding: 0,
        zIndex: 99999,
        pointerEvents: "auto",
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
  stage:
    | "downloading"
    | "extracting"
    | "verifying"
    | "completed"
    | "downloading (twilight rolling release)",
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
          : stage === "downloading (twilight rolling release)"
            ? `Downloading ${browserName} ${version}`
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
      current_browser?: string;
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

export function showTwilightUpdateToast(
  browserName: string,
  options?: {
    id?: string;
    description?: string;
    hasUpdate?: boolean;
    duration?: number;
  },
) {
  return showToast({
    type: "twilight-update",
    title: options?.hasUpdate
      ? `${browserName} twilight update available`
      : `Checking for ${browserName} twilight updates...`,
    browserName,
    ...options,
  });
}

export function showAutoUpdateToast(
  browserName: string,
  version: string,
  options?: {
    id?: string;
    description?: string;
    duration?: number;
  },
) {
  return showToast({
    type: "loading",
    title: `${browserName} update started`,
    description:
      options?.description ??
      `Automatically downloading ${browserName} ${version}. Progress will be shown in download notifications.`,
    id: options?.id ?? `auto-update-${browserName.toLowerCase()}-${version}`,
    duration: options?.duration ?? 4000,
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

// Add a specific function for unified version update progress
export function showUnifiedVersionUpdateToast(
  title: string,
  options?: {
    id?: string;
    description?: string;
    progress?: {
      current: number;
      total: number;
      found: number;
      current_browser?: string;
    };
    duration?: number;
  },
) {
  return showToast({
    type: "version-update",
    title,
    id: "unified-version-update",
    duration: Number.POSITIVE_INFINITY, // Keep showing until completed
    ...options,
  });
}

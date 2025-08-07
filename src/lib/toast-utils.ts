import React from "react";
import { toast as sonnerToast } from "sonner";
import { UnifiedToast } from "@/components/custom-toast";

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
    current_browser?: string;
  };
}

type ToastProps =
  | SuccessToastProps
  | ErrorToastProps
  | DownloadToastProps
  | LoadingToastProps
  | VersionUpdateToastProps;

export function showToast(props: ToastProps & { id?: string }) {
  const toastId = props.id ?? `toast-${props.type}-${Date.now()}`;

  let duration: number;
  if (props.duration !== undefined) {
    duration = props.duration;
  } else {
    switch (props.type) {
      case "loading":
        duration = 10000;
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
      case "success":
        duration = 3000;
        break;
      case "error":
        duration = 10000;
        break;
      case "version-update":
        duration = 15000;
        break;
      default:
        duration = 5000;
    }
  }

  if (props.type === "success") {
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
  } else if (props.type === "error") {
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

export function dismissToast(id: string) {
  sonnerToast.dismiss(id);
}

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

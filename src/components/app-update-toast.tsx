"use client";

import { FaDownload, FaTimes } from "react-icons/fa";
import { LuCheckCheck, LuCog, LuRefreshCw } from "react-icons/lu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { AppUpdateInfo, AppUpdateProgress } from "@/types";

interface AppUpdateToastProps {
  updateInfo: AppUpdateInfo;
  onUpdate: (updateInfo: AppUpdateInfo) => Promise<void>;
  onDismiss: () => void;
  isUpdating?: boolean;
  updateProgress?: AppUpdateProgress | null;
}

function getStageIcon(stage?: string, isUpdating?: boolean) {
  if (!isUpdating) {
    return <FaDownload className="flex-shrink-0 w-5 h-5 text-blue-500" />;
  }

  switch (stage) {
    case "downloading":
      return <FaDownload className="flex-shrink-0 w-5 h-5 text-blue-500" />;
    case "extracting":
      return (
        <LuRefreshCw className="flex-shrink-0 w-5 h-5 text-blue-500 animate-spin" />
      );
    case "installing":
      return (
        <LuCog className="flex-shrink-0 w-5 h-5 text-blue-500 animate-spin" />
      );
    case "completed":
      return <LuCheckCheck className="flex-shrink-0 w-5 h-5 text-green-500" />;
    default:
      return (
        <LuRefreshCw className="flex-shrink-0 w-5 h-5 text-blue-500 animate-spin" />
      );
  }
}

function getStageDisplayName(stage?: string) {
  switch (stage) {
    case "downloading":
      return "Downloading";
    case "extracting":
      return "Extracting";
    case "installing":
      return "Installing";
    case "completed":
      return "Completed";
    default:
      return "Updating";
  }
}

export function AppUpdateToast({
  updateInfo,
  onUpdate,
  onDismiss,
  isUpdating = false,
  updateProgress,
}: AppUpdateToastProps) {
  const handleUpdateClick = async () => {
    await onUpdate(updateInfo);
  };

  const showDownloadProgress =
    isUpdating &&
    updateProgress?.stage === "downloading" &&
    updateProgress.percentage !== undefined;

  const showOtherStageProgress =
    isUpdating &&
    updateProgress &&
    (updateProgress.stage === "extracting" ||
      updateProgress.stage === "installing" ||
      updateProgress.stage === "completed");

  return (
    <div className="flex items-start p-4 w-full max-w-md bg-white rounded-lg border border-gray-200 shadow-lg dark:bg-gray-800 dark:border-gray-700">
      <div className="mr-3 mt-0.5">
        {getStageIcon(updateProgress?.stage, isUpdating)}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex gap-2 justify-between items-start">
          <div className="flex flex-col gap-1">
            <div className="flex gap-2 items-center">
              <span className="text-sm font-semibold text-foreground">
                {isUpdating
                  ? `${getStageDisplayName(updateProgress?.stage)} Donut Browser Update`
                  : "Donut Browser Update Available"}
              </span>
              <Badge
                variant={updateInfo.is_nightly ? "secondary" : "default"}
                className="text-xs"
              >
                {updateInfo.is_nightly ? "Nightly" : "Stable"}
              </Badge>
            </div>
            <div className="text-xs text-muted-foreground">
              {isUpdating ? (
                updateProgress?.message || "Updating..."
              ) : (
                <>
                  Update from {updateInfo.current_version} to{" "}
                  <span className="font-medium">{updateInfo.new_version}</span>
                </>
              )}
            </div>
          </div>

          {!isUpdating && (
            <Button
              variant="ghost"
              size="sm"
              onClick={onDismiss}
              className="p-0 w-6 h-6 shrink-0"
            >
              <FaTimes className="w-3 h-3" />
            </Button>
          )}
        </div>

        {/* Download progress */}
        {showDownloadProgress && updateProgress && (
          <div className="mt-2 space-y-1">
            <div className="flex justify-between items-center">
              <p className="flex-1 min-w-0 text-xs text-muted-foreground">
                {updateProgress.percentage?.toFixed(1)}%
                {updateProgress.speed && ` • ${updateProgress.speed} MB/s`}
                {updateProgress.eta && ` • ${updateProgress.eta} remaining`}
              </p>
            </div>
            <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
              <div
                className="bg-blue-500 h-1.5 rounded-full transition-all duration-300"
                style={{ width: `${updateProgress.percentage}%` }}
              />
            </div>
          </div>
        )}

        {/* Other stage progress (with visual indicators) */}
        {showOtherStageProgress && (
          <div className="mt-2 space-y-2">
            <p className="text-xs text-muted-foreground">
              {updateProgress.message}
            </p>

            {/* Progress indicator for non-downloading stages */}
            <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
              <div
                className={`h-1.5 rounded-full transition-all duration-500 ${
                  updateProgress.stage === "completed"
                    ? "bg-green-500 w-full"
                    : "bg-blue-500 w-full animate-pulse"
                }`}
              />
            </div>

            {updateProgress.stage === "extracting" && (
              <p className="text-xs text-muted-foreground">
                Preparing update files...
              </p>
            )}
            {updateProgress.stage === "installing" && (
              <p className="text-xs text-muted-foreground">
                Installing new version...
              </p>
            )}
            {updateProgress.stage === "completed" && (
              <p className="text-xs text-green-600 dark:text-green-400">
                Update completed! Restarting application...
              </p>
            )}
          </div>
        )}

        {!isUpdating && (
          <div className="flex gap-2 items-center mt-3">
            <Button
              onClick={() => void handleUpdateClick()}
              size="sm"
              className="flex gap-2 items-center text-xs"
            >
              <FaDownload className="w-3 h-3" />
              Update Now
            </Button>
            <Button
              variant="outline"
              onClick={onDismiss}
              size="sm"
              className="text-xs"
            >
              Later
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

"use client";

import { FaDownload, FaExternalLinkAlt, FaTimes } from "react-icons/fa";
import { LuCheckCheck, LuCog, LuRefreshCw } from "react-icons/lu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { AppUpdateInfo, AppUpdateProgress } from "@/types";
import { RippleButton } from "./ui/ripple";

interface AppUpdateToastProps {
  updateInfo: AppUpdateInfo;
  onUpdate: (updateInfo: AppUpdateInfo) => Promise<void>;
  onRestart: () => Promise<void>;
  onDismiss: () => void;
  isUpdating?: boolean;
  updateProgress?: AppUpdateProgress | null;
  updateReady?: boolean;
}

function getStageIcon(stage?: string, isUpdating?: boolean) {
  if (!isUpdating) {
    return <FaDownload className="flex-shrink-0 w-5 h-5" />;
  }

  switch (stage) {
    case "downloading":
      return <FaDownload className="flex-shrink-0 w-5 h-5" />;
    case "extracting":
      return <LuRefreshCw className="flex-shrink-0 w-5 h-5 animate-spin" />;
    case "installing":
      return <LuCog className="flex-shrink-0 w-5 h-5 animate-spin" />;
    case "completed":
      return <LuCheckCheck className="flex-shrink-0 w-5 h-5" />;
    default:
      return <LuRefreshCw className="flex-shrink-0 w-5 h-5 animate-spin" />;
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
  onRestart,
  onDismiss,
  isUpdating = false,
  updateProgress,
  updateReady = false,
}: AppUpdateToastProps) {
  const handleUpdateClick = async () => {
    await onUpdate(updateInfo);
  };

  const handleRestartClick = async () => {
    await onRestart();
  };

  const handleViewRelease = () => {
    if (updateInfo.release_page_url) {
      const event = new CustomEvent("url-open-request", {
        detail: updateInfo.release_page_url,
      });
      window.dispatchEvent(event);
    }
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
    <div className="flex items-start p-4 w-full max-w-md rounded-lg border shadow-lg bg-card border-border text-card-foreground">
      <div className="mr-3 mt-0.5">
        {updateReady ? (
          <LuCheckCheck className="flex-shrink-0 w-5 h-5 text-green-500" />
        ) : (
          getStageIcon(updateProgress?.stage, isUpdating)
        )}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex gap-2 justify-between items-start">
          <div className="flex flex-col gap-1">
            <div className="flex gap-2 items-center">
              <span className="text-sm font-semibold text-foreground">
                {updateReady
                  ? "The update is ready, restart app"
                  : isUpdating
                    ? `${getStageDisplayName(updateProgress?.stage)} Donut Browser Update`
                    : "Donut Browser Update Available"}
              </span>
              {!updateReady && (
                <Badge
                  variant={updateInfo.is_nightly ? "secondary" : "default"}
                  className="text-xs"
                >
                  {updateInfo.is_nightly ? "Nightly" : "Stable"}
                </Badge>
              )}
            </div>
            {!updateReady && (
              <div className="text-xs text-muted-foreground">
                {isUpdating ? (
                  updateProgress?.message || "Updating..."
                ) : (
                  <>
                    Update from {updateInfo.current_version} to{" "}
                    <span className="font-medium">
                      {updateInfo.new_version}
                    </span>
                    {updateInfo.manual_update_required && (
                      <span className="block mt-1 text-muted-foreground/80">
                        Manual download required on Linux
                      </span>
                    )}
                  </>
                )}
              </div>
            )}
          </div>

          {!isUpdating && !updateReady && (
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

        {!updateReady && showDownloadProgress && updateProgress && (
          <div className="mt-2 space-y-1">
            <div className="flex justify-between items-center">
              <p className="flex-1 min-w-0 text-xs text-muted-foreground">
                {updateProgress.percentage?.toFixed(1)}%
                {updateProgress.speed && ` • ${updateProgress.speed} MB/s`}
                {updateProgress.eta && ` • ${updateProgress.eta} remaining`}
              </p>
            </div>
            <div className="w-full bg-muted rounded-full h-1.5">
              <div
                className="bg-primary h-1.5 rounded-full transition-all duration-300"
                style={{ width: `${updateProgress.percentage}%` }}
              />
            </div>
          </div>
        )}

        {!updateReady && showOtherStageProgress && (
          <div className="mt-2 space-y-1">
            <div className="w-full bg-muted rounded-full h-1.5">
              <div
                className={`h-1.5 rounded-full transition-all duration-500 ${
                  updateProgress.stage === "completed"
                    ? "bg-green-500 w-full"
                    : "bg-primary w-full animate-pulse"
                }`}
              />
            </div>
          </div>
        )}

        {updateReady ? (
          <div className="flex gap-2 items-center mt-3">
            <RippleButton
              onClick={() => void handleRestartClick()}
              size="sm"
              className="flex gap-2 items-center text-xs"
            >
              <LuCheckCheck className="w-3 h-3" />
              Restart Now
            </RippleButton>
          </div>
        ) : (
          !isUpdating && (
            <div className="flex gap-2 items-center mt-3">
              {updateInfo.manual_update_required ? (
                <RippleButton
                  onClick={handleViewRelease}
                  size="sm"
                  className="flex gap-2 items-center text-xs"
                >
                  <FaExternalLinkAlt className="w-3 h-3" />
                  View Release
                </RippleButton>
              ) : (
                <RippleButton
                  onClick={() => void handleUpdateClick()}
                  size="sm"
                  className="flex gap-2 items-center text-xs"
                >
                  <FaDownload className="w-3 h-3" />
                  Download Update
                </RippleButton>
              )}
              <RippleButton
                variant="outline"
                onClick={onDismiss}
                size="sm"
                className="text-xs"
              >
                Later
              </RippleButton>
            </div>
          )
        )}
      </div>
    </div>
  );
}

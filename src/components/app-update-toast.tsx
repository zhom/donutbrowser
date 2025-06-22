"use client";

import { FaDownload, FaTimes } from "react-icons/fa";
import { LuRefreshCw } from "react-icons/lu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

interface AppUpdateInfo {
  current_version: string;
  new_version: string;
  release_notes: string;
  download_url: string;
  is_nightly: boolean;
  published_at: string;
}

interface AppUpdateToastProps {
  updateInfo: AppUpdateInfo;
  onUpdate: (updateInfo: AppUpdateInfo) => Promise<void>;
  onDismiss: () => void;
  isUpdating?: boolean;
  updateProgress?: string;
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

  return (
    <div className="flex items-start p-4 w-full max-w-md bg-white rounded-lg border border-gray-200 shadow-lg dark:bg-gray-800 dark:border-gray-700">
      <div className="mr-3 mt-0.5">
        {isUpdating ? (
          <LuRefreshCw className="flex-shrink-0 w-5 h-5 text-blue-500 animate-spin" />
        ) : (
          <FaDownload className="flex-shrink-0 w-5 h-5 text-blue-500" />
        )}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex gap-2 justify-between items-start">
          <div className="flex flex-col gap-1">
            <div className="flex gap-2 items-center">
              <span className="text-sm font-semibold text-foreground">
                Donut Browser Update Available
              </span>
              <Badge
                variant={updateInfo.is_nightly ? "secondary" : "default"}
                className="text-xs"
              >
                {updateInfo.is_nightly ? "Nightly" : "Stable"}
              </Badge>
            </div>
            <div className="text-xs text-muted-foreground">
              Update from {updateInfo.current_version} to{" "}
              <span className="font-medium">{updateInfo.new_version}</span>
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

        {isUpdating && updateProgress && (
          <div className="mt-2">
            <p className="text-xs text-muted-foreground">{updateProgress}</p>
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

"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import React from "react";
import { FaDownload, FaTimes } from "react-icons/fa";
import { LuRefreshCw } from "react-icons/lu";

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
    <div className="flex items-start w-full bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg p-4 shadow-lg max-w-md">
      <div className="mr-3 mt-0.5">
        {isUpdating ? (
          <LuRefreshCw className="h-5 w-5 text-blue-500 animate-spin flex-shrink-0" />
        ) : (
          <FaDownload className="h-5 w-5 text-blue-500 flex-shrink-0" />
        )}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-start justify-between gap-2">
          <div className="flex flex-col gap-1">
            <div className="flex items-center gap-2">
              <span className="font-semibold text-foreground text-sm">
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
              className="h-6 w-6 p-0 shrink-0"
            >
              <FaTimes className="h-3 w-3" />
            </Button>
          )}
        </div>

        {isUpdating && updateProgress && (
          <div className="mt-2">
            <p className="text-xs text-muted-foreground">{updateProgress}</p>
          </div>
        )}

        {!isUpdating && (
          <div className="flex items-center gap-2 mt-3">
            <Button
              onClick={() => void handleUpdateClick()}
              size="sm"
              className="flex items-center gap-2 text-xs"
            >
              <FaDownload className="h-3 w-3" />
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

        {updateInfo.release_notes && !isUpdating && (
          <div className="mt-2">
            <details className="text-xs">
              <summary className="cursor-pointer text-muted-foreground hover:text-foreground">
                Release Notes
              </summary>
              <div className="mt-1 text-muted-foreground whitespace-pre-wrap max-h-32 overflow-y-auto">
                {updateInfo.release_notes.length > 200
                  ? `${updateInfo.release_notes.substring(0, 200)}...`
                  : updateInfo.release_notes}
              </div>
            </details>
          </div>
        )}
      </div>
    </div>
  );
}

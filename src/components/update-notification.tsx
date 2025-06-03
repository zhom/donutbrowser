"use client";

/* eslint-disable @typescript-eslint/no-misused-promises */

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { getBrowserDisplayName } from "@/lib/browser-utils";
import React from "react";
import { FaDownload, FaTimes } from "react-icons/fa";
import { LuDownload } from "react-icons/lu";

interface UpdateNotification {
  id: string;
  browser: string;
  current_version: string;
  new_version: string;
  affected_profiles: string[];
  is_stable_update: boolean;
  timestamp: number;
}

interface UpdateNotificationProps {
  notification: UpdateNotification;
  onUpdate: (browser: string, newVersion: string) => Promise<void>;
  onDismiss: (notificationId: string) => Promise<void>;
  isUpdating?: boolean;
}

export function UpdateNotificationComponent({
  notification,
  onUpdate,
  onDismiss,
  isUpdating = false,
}: UpdateNotificationProps) {
  const browserDisplayName = getBrowserDisplayName(notification.browser);

  const profileText =
    notification.affected_profiles.length === 1
      ? `profile "${notification.affected_profiles[0]}"`
      : `${notification.affected_profiles.length} profiles`;

  const handleUpdateClick = async () => {
    // Dismiss the notification immediately to close the modal
    await onDismiss(notification.id);
    // Then start the update process
    await onUpdate(notification.browser, notification.new_version);
  };

  return (
    <div className="flex flex-col gap-3 p-4 max-w-md rounded-lg border shadow-lg bg-background border-border">
      <div className="flex gap-2 justify-between items-start">
        <div className="flex flex-col gap-1">
          <div className="flex gap-2 items-center">
            <span className="font-semibold text-foreground">
              {browserDisplayName} Update Available
            </span>
            <Badge
              variant={notification.is_stable_update ? "default" : "secondary"}
            >
              {notification.is_stable_update ? "Stable" : "Nightly"}
            </Badge>
          </div>
          <div className="text-sm text-muted-foreground">
            Update {profileText} from {notification.current_version} to{" "}
            <span className="font-medium">{notification.new_version}</span>
          </div>
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={async () => {
            await onDismiss(notification.id);
          }}
          className="p-0 w-6 h-6 shrink-0"
        >
          <FaTimes className="w-3 h-3" />
        </Button>
      </div>

      <div className="flex gap-2 items-center">
        <Button
          onClick={handleUpdateClick}
          disabled={isUpdating}
          size="sm"
          className="flex gap-2 items-center"
        >
          <FaDownload className="w-3 h-3" />
          Update
        </Button>
        <Button
          variant="outline"
          onClick={async () => {
            await onDismiss(notification.id);
          }}
          size="sm"
        >
          Later
        </Button>
      </div>

      {notification.affected_profiles.length > 1 && (
        <div className="text-xs text-muted-foreground">
          Affected profiles: {notification.affected_profiles.join(", ")}
        </div>
      )}
    </div>
  );
}

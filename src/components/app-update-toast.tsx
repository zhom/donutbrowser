"use client";

import { FaExternalLinkAlt, FaTimes } from "react-icons/fa";
import { LuCheckCheck } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import type { AppUpdateInfo } from "@/types";
import { RippleButton } from "./ui/ripple";

interface AppUpdateToastProps {
  updateInfo: AppUpdateInfo;
  onRestart: () => Promise<void>;
  onDismiss: () => void;
  updateReady?: boolean;
}

export function AppUpdateToast({
  updateInfo,
  onRestart,
  onDismiss,
  updateReady = false,
}: AppUpdateToastProps) {
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

  return (
    <div className="flex items-start p-4 w-full max-w-md rounded-lg border shadow-lg bg-card border-border text-card-foreground">
      <div className="mr-3 mt-0.5">
        <LuCheckCheck className="flex-shrink-0 w-5 h-5" />
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex gap-2 justify-between items-start">
          <div className="flex flex-col gap-1">
            <span className="text-sm font-semibold text-foreground">
              {updateReady
                ? "Update ready, restart to apply"
                : "Manual download required"}
            </span>
            <div className="text-xs text-muted-foreground">
              {updateInfo.current_version} → {updateInfo.new_version}
            </div>
          </div>

          <Button
            variant="ghost"
            size="sm"
            onClick={onDismiss}
            className="p-0 w-6 h-6 shrink-0"
          >
            <FaTimes className="w-3 h-3" />
          </Button>
        </div>

        <div className="flex gap-2 items-center mt-3">
          {updateReady ? (
            <RippleButton
              onClick={() => void handleRestartClick()}
              size="sm"
              className="flex gap-2 items-center text-xs"
            >
              <LuCheckCheck className="w-3 h-3" />
              Restart Now
            </RippleButton>
          ) : (
            updateInfo.manual_update_required && (
              <RippleButton
                onClick={handleViewRelease}
                size="sm"
                className="flex gap-2 items-center text-xs"
              >
                <FaExternalLinkAlt className="w-3 h-3" />
                View Release
              </RippleButton>
            )
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
      </div>
    </div>
  );
}

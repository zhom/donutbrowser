"use client";

import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation();
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
    <div className="flex w-full max-w-md items-start rounded-lg border border-border bg-card p-4 text-card-foreground shadow-lg">
      <div className="mt-0.5 mr-3">
        <LuCheckCheck className="size-5 shrink-0" />
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex items-start justify-between gap-2">
          <div className="flex flex-col gap-1">
            <span className="text-sm font-semibold text-foreground">
              {updateReady
                ? t("appUpdate.toast.updateReady")
                : updateInfo.repo_update
                  ? "Update available via package manager"
                  : t("appUpdate.toast.manualDownloadRequired")}
            </span>
            <div className="text-xs text-muted-foreground">
              {updateInfo.current_version} → {updateInfo.new_version}
            </div>
          </div>

          <Button
            variant="ghost"
            size="sm"
            onClick={onDismiss}
            className="size-6 shrink-0 p-0"
          >
            <FaTimes className="size-3" />
          </Button>
        </div>

        <div className="mt-3 flex items-center gap-2">
          {updateReady ? (
            <RippleButton
              onClick={() => void handleRestartClick()}
              size="sm"
              className="flex items-center gap-2 text-xs"
            >
              <LuCheckCheck className="size-3" />
              {t("appUpdate.toast.restartNow")}
            </RippleButton>
          ) : (
            !updateInfo.repo_update &&
            updateInfo.manual_update_required && (
              <RippleButton
                onClick={handleViewRelease}
                size="sm"
                className="flex items-center gap-2 text-xs"
              >
                <FaExternalLinkAlt className="size-3" />
                {t("appUpdate.toast.viewRelease")}
              </RippleButton>
            )
          )}
          <RippleButton
            variant="outline"
            onClick={onDismiss}
            size="sm"
            className="text-xs"
          >
            {t("appUpdate.toast.later")}
          </RippleButton>
        </div>
      </div>
    </div>
  );
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LuTriangleAlert } from "react-icons/lu";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { RippleButton } from "./ui/ripple";

export interface ConsistencyResult {
  consistent: boolean;
  checked: boolean;
  exit_ip: string | null;
  exit_country_code: string | null;
  exit_timezone: string | null;
  fingerprint_timezone: string | null;
  fingerprint_language: string | null;
  mismatches: string[];
}

const GLOBAL_DISABLE_KEY = "consistency-warn-disabled";
const perProfileKey = (id: string) => `consistency-warn-skip-${id}`;

export function isConsistencyWarningEnabled(): boolean {
  try {
    return localStorage.getItem(GLOBAL_DISABLE_KEY) !== "1";
  } catch {
    return true;
  }
}

export function isConsistencyWarningSuppressed(profileId: string): boolean {
  try {
    return (
      localStorage.getItem(GLOBAL_DISABLE_KEY) === "1" ||
      localStorage.getItem(perProfileKey(profileId)) === "1"
    );
  } catch {
    return false;
  }
}

interface ConsistencyWarningDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profileName: string;
  profileId: string;
  result: ConsistencyResult | null;
}

export function ConsistencyWarningDialog({
  isOpen,
  onClose,
  profileName,
  profileId,
  result,
}: ConsistencyWarningDialogProps) {
  const { t } = useTranslation();
  const [dontWarnAgain, setDontWarnAgain] = useState(false);
  const [isMatching, setIsMatching] = useState(false);

  const handleClose = () => {
    if (dontWarnAgain) {
      try {
        localStorage.setItem(perProfileKey(profileId), "1");
      } catch {
        // localStorage unavailable — nothing to persist
      }
    }
    setDontWarnAgain(false);
    onClose();
  };

  const mismatches = result?.mismatches ?? [];
  const exitIp = result?.exit_ip ?? null;

  const handleMatch = async () => {
    if (!exitIp) {
      return;
    }
    setIsMatching(true);
    try {
      await invoke("match_profile_fingerprint_to_exit", {
        profileId,
        exitIp,
      });
      showSuccessToast(t("consistencyWarning.matchSuccess"));
      handleClose();
    } catch (e) {
      showErrorToast(translateBackendError(t, e));
    } finally {
      setIsMatching(false);
    }
  };

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <LuTriangleAlert className="size-5 text-warning" />
            {t("consistencyWarning.title")}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-3 text-sm">
          <p className="text-muted-foreground">
            {t("consistencyWarning.intro", { name: profileName })}
          </p>

          <div className="space-y-2 rounded-md border border-warning/40 bg-warning/10 p-3">
            {mismatches.includes("timezone") && (
              <div>
                <p className="font-medium">
                  {t("consistencyWarning.timezoneTitle")}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t("consistencyWarning.timezoneDetail", {
                    exit: result?.exit_timezone ?? "?",
                    fingerprint: result?.fingerprint_timezone ?? "?",
                  })}
                </p>
              </div>
            )}
            {mismatches.includes("language") && (
              <div>
                <p className="font-medium">
                  {t("consistencyWarning.languageTitle")}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t("consistencyWarning.languageDetail", {
                    country: result?.exit_country_code ?? "?",
                    fingerprint: result?.fingerprint_language ?? "?",
                  })}
                </p>
              </div>
            )}
          </div>

          <p className="text-xs text-muted-foreground">
            {t("consistencyWarning.explainer")}
          </p>

          <label
            htmlFor="consistency-dont-warn"
            className="flex cursor-pointer items-center gap-2 text-xs"
          >
            <Checkbox
              id="consistency-dont-warn"
              checked={dontWarnAgain}
              onCheckedChange={(v) => setDontWarnAgain(v === true)}
            />
            {t("consistencyWarning.dontWarnAgain")}
          </label>
        </div>

        <div className="flex justify-end gap-2">
          <RippleButton
            variant="outline"
            onClick={handleClose}
            disabled={isMatching}
          >
            {t("common.buttons.close")}
          </RippleButton>
          {exitIp && (
            <RippleButton onClick={handleMatch} disabled={isMatching}>
              {isMatching
                ? t("consistencyWarning.matching")
                : t("consistencyWarning.matchToProxy")}
            </RippleButton>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

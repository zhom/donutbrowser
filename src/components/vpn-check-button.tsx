"use client";

import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { FiCheck } from "react-icons/fi";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { formatRelativeTime } from "@/lib/flag-utils";
import { MOTION_EASE_OUT } from "@/lib/motion";
import type { ProxyCheckResult } from "@/types";

interface VpnCheckButtonProps {
  vpnId: string;
  vpnName: string;
  checkingVpnId: string | null;
  setCheckingVpnId: (id: string | null) => void;
  disabled?: boolean;
}

export function VpnCheckButton({
  vpnId,
  vpnName,
  checkingVpnId,
  setCheckingVpnId,
  disabled = false,
}: VpnCheckButtonProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const [result, setResult] = React.useState<ProxyCheckResult | undefined>();

  const handleCheck = React.useCallback(async () => {
    if (checkingVpnId === vpnId) return;

    setCheckingVpnId(vpnId);
    try {
      const checkResult = await invoke<ProxyCheckResult>("check_vpn_validity", {
        vpnId,
      });
      setResult(checkResult);

      if (checkResult.is_valid) {
        toast.success(t("vpnCheck.valid", { name: vpnName }));
      } else {
        toast.error(t("vpnCheck.invalid", { name: vpnName }));
      }
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(t("vpnCheck.failed", { error: errorMessage }));

      setResult({
        ip: "",
        timestamp: Math.floor(Date.now() / 1000),
        is_valid: false,
      });
    } finally {
      setCheckingVpnId(null);
    }
  }, [vpnId, vpnName, checkingVpnId, setCheckingVpnId, t]);

  const isCurrentlyChecking = checkingVpnId === vpnId;
  const statusKey = isCurrentlyChecking
    ? "checking"
    : result?.is_valid
      ? "valid"
      : result && !result.is_valid
        ? "invalid"
        : "idle";

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="size-7 p-0"
          onClick={handleCheck}
          disabled={isCurrentlyChecking || disabled}
        >
          <AnimatePresence initial={false} mode="wait">
            <motion.span
              key={statusKey}
              initial={{ opacity: 0, scale: reduceMotion ? 1 : 0.9 }}
              animate={{
                opacity: 1,
                scale: 1,
                transition: {
                  duration: reduceMotion ? 0.15 : 0.16,
                  ease: MOTION_EASE_OUT,
                },
              }}
              exit={{
                opacity: 0,
                scale: reduceMotion ? 1 : 0.9,
                transition: {
                  duration: reduceMotion ? 0.15 : 0.1,
                  ease: MOTION_EASE_OUT,
                },
              }}
              className="inline-flex size-3 items-center justify-center"
            >
              {isCurrentlyChecking ? (
                <span className="size-3 animate-spin rounded-full border border-current border-t-transparent" />
              ) : result?.is_valid ? (
                <FiCheck className="size-3 text-success" />
              ) : result && !result.is_valid ? (
                <span className="text-sm text-destructive">✕</span>
              ) : (
                <FiCheck className="size-3" />
              )}
            </motion.span>
          </AnimatePresence>
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        {isCurrentlyChecking ? (
          <p>{t("vpnCheck.tooltipChecking")}</p>
        ) : result?.is_valid ? (
          <div className="space-y-1">
            <p>{t("vpnCheck.tooltipValid")}</p>
            <p className="text-xs text-primary-foreground/70">
              {t("vpnCheck.tooltipChecked", {
                time: formatRelativeTime(result.timestamp),
              })}
            </p>
          </div>
        ) : result && !result.is_valid ? (
          <div>
            <p>{t("vpnCheck.tooltipInvalid")}</p>
            <p className="text-xs text-primary-foreground/70">
              {t("vpnCheck.tooltipChecked", {
                time: formatRelativeTime(result.timestamp),
              })}
            </p>
          </div>
        ) : (
          <p>{t("vpnCheck.tooltipDefault")}</p>
        )}
      </TooltipContent>
    </Tooltip>
  );
}

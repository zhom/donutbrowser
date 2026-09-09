"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { FiCheck, FiX } from "react-icons/fi";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { formatRelativeTime } from "@/lib/flag-utils";
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

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="size-7 p-0"
          onClick={handleCheck}
          disabled={isCurrentlyChecking || disabled}
          aria-busy={isCurrentlyChecking}
          aria-label={t(
            isCurrentlyChecking
              ? "vpnCheck.tooltipChecking"
              : "vpnCheck.tooltipDefault",
          )}
        >
          <span
            aria-hidden="true"
            className="inline-flex size-3 items-center justify-center"
          >
            {isCurrentlyChecking ? (
              <span className="size-3 animate-spin rounded-full border border-current border-t-transparent motion-reduce:animate-none" />
            ) : result?.is_valid ? (
              <FiCheck className="size-3 text-success-text" />
            ) : result && !result.is_valid ? (
              <FiX className="size-3 text-destructive-text" />
            ) : (
              <FiCheck className="size-3" />
            )}
          </span>
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        {isCurrentlyChecking ? (
          <p>{t("vpnCheck.tooltipChecking")}</p>
        ) : result?.is_valid ? (
          <div className="space-y-1">
            <p>{t("vpnCheck.tooltipValid")}</p>
            <p className="text-xs text-primary-foreground">
              {t("vpnCheck.tooltipChecked", {
                time: formatRelativeTime(result.timestamp),
              })}
            </p>
          </div>
        ) : result && !result.is_valid ? (
          <div>
            <p>{t("vpnCheck.tooltipInvalid")}</p>
            <p className="text-xs text-primary-foreground">
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

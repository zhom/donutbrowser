"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { FiCheck } from "react-icons/fi";
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
        toast.success(`VPN "${vpnName}" configuration is valid`);
      } else {
        toast.error(`VPN "${vpnName}" configuration is invalid`);
      }
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(`VPN check failed: ${errorMessage}`);

      setResult({
        ip: "",
        timestamp: Math.floor(Date.now() / 1000),
        is_valid: false,
      });
    } finally {
      setCheckingVpnId(null);
    }
  }, [vpnId, vpnName, checkingVpnId, setCheckingVpnId]);

  const isCurrentlyChecking = checkingVpnId === vpnId;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 w-7 p-0"
          onClick={handleCheck}
          disabled={isCurrentlyChecking || disabled}
        >
          {isCurrentlyChecking ? (
            <div className="w-3 h-3 rounded-full border border-current animate-spin border-t-transparent" />
          ) : result?.is_valid ? (
            <FiCheck className="w-3 h-3 text-green-500" />
          ) : result && !result.is_valid ? (
            <span className="text-destructive text-sm">✕</span>
          ) : (
            <FiCheck className="w-3 h-3" />
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        {isCurrentlyChecking ? (
          <p>Checking VPN config...</p>
        ) : result?.is_valid ? (
          <div className="space-y-1">
            <p>Configuration valid</p>
            <p className="text-xs text-primary-foreground/70">
              Checked {formatRelativeTime(result.timestamp)}
            </p>
          </div>
        ) : result && !result.is_valid ? (
          <div>
            <p>Configuration invalid</p>
            <p className="text-xs text-primary-foreground/70">
              Checked {formatRelativeTime(result.timestamp)}
            </p>
          </div>
        ) : (
          <p>Check VPN config validity</p>
        )}
      </TooltipContent>
    </Tooltip>
  );
}

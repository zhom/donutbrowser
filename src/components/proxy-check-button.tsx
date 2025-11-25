"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { FiCheck } from "react-icons/fi";
import { toast } from "sonner";
import { FlagIcon } from "@/components/flag-icon";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { formatRelativeTime } from "@/lib/flag-utils";
import type { ProxyCheckResult, StoredProxy } from "@/types";

interface ProxyCheckButtonProps {
  proxy: StoredProxy;
  checkingProxyId: string | null;
  cachedResult?: ProxyCheckResult;
  onCheckComplete?: (result: ProxyCheckResult) => void;
  onCheckFailed?: (result: ProxyCheckResult) => void;
  disabled?: boolean;
  setCheckingProxyId?: (id: string | null) => void;
}

export function ProxyCheckButton({
  proxy,
  checkingProxyId,
  cachedResult,
  onCheckComplete,
  onCheckFailed,
  disabled = false,
  setCheckingProxyId,
}: ProxyCheckButtonProps) {
  const [localResult, setLocalResult] = React.useState<
    ProxyCheckResult | undefined
  >(cachedResult);

  React.useEffect(() => {
    setLocalResult(cachedResult);
  }, [cachedResult]);

  const handleCheck = React.useCallback(async () => {
    if (checkingProxyId === proxy.id) return;

    setCheckingProxyId?.(proxy.id);
    try {
      const result = await invoke<ProxyCheckResult>("check_proxy_validity", {
        proxyId: proxy.id,
        proxySettings: proxy.proxy_settings,
      });
      setLocalResult(result);
      onCheckComplete?.(result);

      // Show toast with location
      const locationParts: string[] = [];
      if (result.city) locationParts.push(result.city);
      if (result.country) locationParts.push(result.country);
      const location =
        locationParts.length > 0 ? locationParts.join(", ") : "Unknown";

      toast.success(
        <div className="flex items-center gap-2">
          Your proxy location is:
          <span>{location}</span>
          {result.country_code && (
            <FlagIcon countryCode={result.country_code} className="text-base" />
          )}
        </div>,
      );
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(`Proxy check failed: ${errorMessage}`);

      // Save failed check result
      const failedResult: ProxyCheckResult = {
        ip: "",
        city: undefined,
        country: undefined,
        country_code: undefined,
        timestamp: Math.floor(Date.now() / 1000),
        is_valid: false,
      };
      setLocalResult(failedResult);
      onCheckFailed?.(failedResult);
    } finally {
      setCheckingProxyId?.(null);
    }
  }, [
    proxy,
    checkingProxyId,
    onCheckComplete,
    onCheckFailed,
    setCheckingProxyId,
  ]);

  const isCurrentlyChecking = checkingProxyId === proxy.id;
  const result = localResult;

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
          ) : result?.is_valid && result.country_code ? (
            <span className="relative inline-flex items-center justify-center">
              <FlagIcon countryCode={result.country_code} className="h-2.5" />
              <FiCheck className="absolute bottom-[-6px] right-[-4px]" />
            </span>
          ) : result && !result.is_valid ? (
            <span className="text-red-600 text-sm">✕</span>
          ) : (
            <FiCheck className="w-3 h-3" />
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        {isCurrentlyChecking ? (
          <p>Checking proxy...</p>
        ) : result?.is_valid ? (
          <div className="space-y-1">
            <p className="flex items-center gap-1">
              {result.country_code && (
                <FlagIcon countryCode={result.country_code} />
              )}
              {[result.city, result.country].filter(Boolean).join(", ") ||
                "Unknown"}
            </p>
            <p className="text-xs text-muted-foreground">IP: {result.ip}</p>
            <p className="text-xs text-muted-foreground">
              Checked {formatRelativeTime(result.timestamp)}
            </p>
          </div>
        ) : result && !result.is_valid ? (
          <div>
            <p>Proxy check failed</p>
            <p className="text-xs text-muted-foreground">
              Failed {formatRelativeTime(result.timestamp)}
            </p>
          </div>
        ) : (
          <p>Check proxy validity</p>
        )}
      </TooltipContent>
    </Tooltip>
  );
}

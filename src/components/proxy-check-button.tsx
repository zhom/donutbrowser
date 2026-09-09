"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { FiCheck, FiX } from "react-icons/fi";
import { LuCircleDashed, LuCopy } from "react-icons/lu";
import { toast } from "sonner";
import { FlagIcon } from "@/components/flag-icon";
import { Button } from "@/components/ui/button";
import { ConfirmationMark } from "@/components/ui/confirmation-mark";
import { OperationFlow } from "@/components/ui/operation-flow";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { translateBackendError } from "@/lib/backend-errors";
import { formatRelativeTime } from "@/lib/flag-utils";
import { runProxyCheck, useProxyCheck } from "@/lib/proxy-check-store";
import type { ProxyCheckHistoryEntry, StoredProxy, UdpSupport } from "@/types";

const COPIED_MARK_MS = 1600;

const UDP_LABEL_KEYS: Record<UdpSupport, string> = {
  yes: "proxyCheck.udpYes",
  no: "proxyCheck.udpNo",
  unknown: "proxyCheck.udpUnknown",
};

const UDP_TOOLTIP_KEYS: Record<UdpSupport, string> = {
  yes: "proxyCheck.udpTooltipYes",
  no: "proxyCheck.udpTooltipNo",
  unknown: "proxyCheck.udpTooltipUnknown",
};

/** A verdict nobody established reads as muted, and a proxy that cannot carry
 * UDP reads as the caveat it is: WebRTC has nowhere to go through it. */
const UDP_TONE: Record<UdpSupport, string> = {
  yes: "text-foreground",
  no: "text-warning-text",
  unknown: "text-muted-foreground",
};

/**
 * Whether a proxy carries UDP, as a table cell. Reads the same shared receipt
 * the check button does, so a check run from any row updates every row.
 */
export function ProxyUdpBadge({ proxy }: { proxy: StoredProxy }) {
  const { t } = useTranslation();
  const { result } = useProxyCheck(proxy);
  const verdict: UdpSupport = result?.udp ?? "unknown";

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          data-slot="proxy-udp-verdict"
          data-udp={verdict}
          className={`font-mono text-[10px] tracking-wider uppercase ${UDP_TONE[verdict]}`}
        >
          {t(UDP_LABEL_KEYS[verdict])}
        </span>
      </TooltipTrigger>
      <TooltipContent>
        <p>{t(UDP_TOOLTIP_KEYS[verdict])}</p>
      </TooltipContent>
    </Tooltip>
  );
}

/** One remembered check, as a single line. */
function HistoryRow({ entry }: { entry: ProxyCheckHistoryEntry }) {
  const { t } = useTranslation();
  const verdict: UdpSupport = entry.udp ?? "unknown";
  return (
    <li
      data-slot="proxy-check-history-entry"
      className="flex items-baseline justify-between gap-3 py-1"
    >
      <span className="min-w-0 flex-1 truncate">
        <span
          className={entry.ok ? "text-foreground" : "text-destructive-text"}
        >
          {t(entry.ok ? "proxyCheck.historyOk" : "proxyCheck.historyFailed")}
        </span>
        {entry.ip && (
          <span className="ml-2 font-mono text-foreground">{entry.ip}</span>
        )}
        {entry.country_code && (
          <FlagIcon countryCode={entry.country_code} className="ml-1" />
        )}
        {entry.isp && <span className="ml-2">{entry.isp}</span>}
      </span>
      <span className="flex shrink-0 items-baseline gap-2 tabular-nums">
        <span className={UDP_TONE[verdict]}>
          {t("proxyCheck.udpLabel")} {t(UDP_LABEL_KEYS[verdict])}
        </span>
        {typeof entry.latency_ms === "number" && (
          <span>{t("proxyCheck.latencyValue", { ms: entry.latency_ms })}</span>
        )}
        <span>{formatRelativeTime(entry.timestamp)}</span>
      </span>
    </li>
  );
}

interface ProxyCheckButtonProps {
  proxy: StoredProxy;
  disabled?: boolean;
}

export function ProxyCheckButton({
  proxy,
  disabled = false,
}: ProxyCheckButtonProps) {
  const { t } = useTranslation();
  const [open, setOpen] = React.useState(false);
  const { result, checking, failure } = useProxyCheck(proxy);
  const [copied, setCopied] = React.useState(false);
  const copiedTimer = React.useRef<number | null>(null);
  const [history, setHistory] = React.useState<ProxyCheckHistoryEntry[] | null>(
    null,
  );

  /** The trail is read from disk rather than accumulated here, so it is the
   * same list however many windows or rows have run a check. */
  const loadHistory = React.useCallback(async () => {
    try {
      setHistory(
        await invoke<ProxyCheckHistoryEntry[]>("get_proxy_check_history", {
          proxyId: proxy.id,
        }),
      );
    } catch (error) {
      console.error("Failed to load the proxy check history:", error);
      setHistory([]);
    }
  }, [proxy.id]);

  React.useEffect(() => {
    if (open) void loadHistory();
  }, [open, loadHistory]);

  React.useEffect(
    () => () => {
      if (copiedTimer.current !== null) {
        window.clearTimeout(copiedTimer.current);
      }
    },
    [],
  );

  const handleCheck = React.useCallback(async () => {
    if (checking) return;
    try {
      const outcome = await runProxyCheck(proxy, (error) =>
        translateBackendError(t, error),
      );
      if (!outcome) return;

      const location =
        [outcome.city, outcome.country].filter(Boolean).join(", ") ||
        t("proxyCheck.unknownLocation");
      toast.success(
        <div className="flex flex-col">
          {t("proxyCheck.locationToast")}
          <div className="flex items-center whitespace-nowrap">
            {location}
            {outcome.country_code && (
              <FlagIcon
                countryCode={outcome.country_code}
                className="ml-1 text-sm"
              />
            )}
          </div>
        </div>,
      );
    } catch (error) {
      toast.error(
        t("proxyCheck.failed", { error: translateBackendError(t, error) }),
      );
    } finally {
      // A failed check is a line in the trail too, so the list is refreshed
      // either way.
      void loadHistory();
    }
  }, [checking, proxy, t, loadHistory]);

  const copyExitIp = React.useCallback(async (ip: string) => {
    try {
      await navigator.clipboard.writeText(ip);
    } catch {
      return;
    }
    setCopied(true);
    if (copiedTimer.current !== null) {
      window.clearTimeout(copiedTimer.current);
    }
    copiedTimer.current = window.setTimeout(() => {
      setCopied(false);
      copiedTimer.current = null;
    }, COPIED_MARK_MS);
  }, []);

  const location =
    (result && [result.city, result.country].filter(Boolean).join(", ")) ||
    t("proxyCheck.unknownLocation");

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              className="size-7 p-0"
              onClick={() => {
                if (!result) void handleCheck();
              }}
              disabled={disabled}
              aria-busy={checking}
              aria-label={t(
                checking
                  ? "proxyCheck.tooltipChecking"
                  : result
                    ? "appFeedback.routeDetails"
                    : "proxyCheck.tooltipDefault",
              )}
            >
              <span
                aria-hidden="true"
                className="inline-flex size-3 items-center justify-center"
              >
                {checking ? (
                  <span className="size-3 animate-spin rounded-full border border-current border-t-transparent motion-reduce:animate-none" />
                ) : result?.is_valid && result.country_code ? (
                  <span className="relative inline-flex items-center justify-center">
                    <FlagIcon
                      countryCode={result.country_code}
                      className="h-2.5"
                    />
                    <FiCheck className="absolute right-[-4px] bottom-[-6px]" />
                  </span>
                ) : result && !result.is_valid ? (
                  <FiX className="size-3 text-destructive-text" />
                ) : result?.is_valid ? (
                  <FiCheck className="size-3" />
                ) : (
                  // An empty status slot: no verdict yet, not a question.
                  <LuCircleDashed className="size-3.5 text-muted-foreground" />
                )}
              </span>
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>
          {checking ? (
            <p>{t("proxyCheck.tooltipChecking")}</p>
          ) : result?.is_valid ? (
            <div className="space-y-1">
              <p className="flex items-center gap-1">
                {result.country_code && (
                  <FlagIcon countryCode={result.country_code} />
                )}
                {location}
              </p>
              <p className="text-xs text-primary-foreground">
                {t("proxyCheck.tooltipIp", { ip: result.ip })}
              </p>
              <p className="text-xs text-primary-foreground">
                {t("proxyCheck.tooltipChecked", {
                  time: formatRelativeTime(result.timestamp),
                })}
              </p>
            </div>
          ) : result && !result.is_valid ? (
            <div>
              <p>{t("proxyCheck.tooltipFailedTitle")}</p>
              <p className="text-xs text-primary-foreground">
                {t("proxyCheck.tooltipFailed", {
                  time: formatRelativeTime(result.timestamp),
                })}
              </p>
            </div>
          ) : (
            <p>{t("appFeedback.notChecked")}</p>
          )}
        </TooltipContent>
      </Tooltip>
      <PopoverContent
        data-slot="proxy-route-details"
        align="end"
        className="w-[min(30rem,calc(100vw-2rem))] space-y-3 p-4"
      >
        <h3 className="break-words text-sm font-medium">{proxy.name}</h3>
        <OperationFlow
          label={t("appFeedback.routeDetails")}
          active={checking ? 1 : result?.is_valid ? 2 : 0}
          failed={!checking && !!result && !result.is_valid}
          steps={[
            {
              id: "device",
              label: t("appFeedback.thisDevice"),
              detail: t("appFeedback.connectionTest"),
            },
            {
              id: "proxy",
              label: t("profileInfo.fields.proxyVpn"),
              detail: `${proxy.proxy_settings.host}:${proxy.proxy_settings.port}`,
            },
            {
              id: "exit",
              label: t("appFeedback.verifiedExit"),
              detail: result?.is_valid ? (
                <>
                  <button
                    type="button"
                    data-slot="proxy-exit-ip"
                    onClick={() => void copyExitIp(result.ip)}
                    aria-label={t(
                      copied ? "common.buttons.copied" : "common.buttons.copy",
                    )}
                    className="inline-flex max-w-full items-center gap-1 rounded-sm font-mono text-foreground hover:text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring"
                  >
                    <span className="break-all">{result.ip}</span>
                    {copied ? (
                      <ConfirmationMark />
                    ) : (
                      <LuCopy aria-hidden="true" className="size-3 shrink-0" />
                    )}
                  </button>
                  <span className="block">{location}</span>
                </>
              ) : (
                t("appFeedback.notVerified")
              ),
            },
          ]}
        />
        <div
          role="status"
          className="space-y-1 text-xs leading-relaxed text-muted-foreground"
        >
          <p>
            {checking
              ? t("proxyCheck.tooltipChecking")
              : result
                ? t(
                    result.is_valid
                      ? "appFeedback.lastCheckPassed"
                      : "proxyCheck.tooltipFailedTitle",
                  )
                : t("appFeedback.notChecked")}
          </p>
          {result && (
            <p>
              {t("proxyCheck.tooltipChecked", {
                time: new Date(result.timestamp * 1000).toLocaleString(),
              })}
            </p>
          )}
          {result && (
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
              <dt>{t("proxyCheck.udpLabel")}</dt>
              <dd
                data-slot="proxy-udp-detail"
                className={UDP_TONE[result.udp ?? "unknown"]}
              >
                {t(UDP_LABEL_KEYS[result.udp ?? "unknown"])}
              </dd>
              {result.isp && (
                <>
                  <dt>{t("proxyCheck.ispLabel")}</dt>
                  <dd
                    data-slot="proxy-exit-isp"
                    className="break-words text-foreground"
                  >
                    {result.isp}
                  </dd>
                </>
              )}
              {result.timezone && (
                <>
                  <dt>{t("proxyCheck.timezoneLabel")}</dt>
                  <dd
                    data-slot="proxy-exit-timezone"
                    className="text-foreground"
                  >
                    {result.timezone}
                  </dd>
                </>
              )}
              {typeof result.latency_ms === "number" && (
                <>
                  <dt>{t("proxyCheck.latencyLabel")}</dt>
                  <dd className="tabular-nums text-foreground">
                    {t("proxyCheck.latencyValue", { ms: result.latency_ms })}
                  </dd>
                </>
              )}
            </dl>
          )}
          {failure && (
            <p className="break-words text-destructive-text">{failure}</p>
          )}
          <p>{t("appFeedback.routeScope")}</p>
        </div>
        <section
          data-slot="proxy-check-history"
          className="space-y-1 text-xs text-muted-foreground"
        >
          <h4 className="font-medium text-foreground">
            {t("proxyCheck.historyTitle")}
          </h4>
          {history && history.length > 0 ? (
            <ul className="max-h-48 divide-y divide-border overflow-y-auto">
              {history.map((entry, index) => (
                <HistoryRow key={`${entry.timestamp}-${index}`} entry={entry} />
              ))}
            </ul>
          ) : (
            <p>{t("proxyCheck.historyEmpty")}</p>
          )}
        </section>
        <Button
          size="sm"
          variant="secondary"
          disabled={checking || disabled}
          onClick={() => void handleCheck()}
        >
          {t("proxyCheck.tooltipDefault")}
        </Button>
      </PopoverContent>
    </Popover>
  );
}

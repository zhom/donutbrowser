"use client";

import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LuActivity } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { OperationFlow } from "@/components/ui/operation-flow";
import { SettingsPanel } from "@/components/ui/settings-panel";
import { translateBackendError } from "@/lib/backend-errors";
import { cn } from "@/lib/utils";

interface Diagnostic {
  configured: boolean;
  reachable: boolean | null;
  authorized: boolean | null;
  http_status: number | null;
  checked_at: number;
}

/**
 * A read-only probe of one integration route. The owner remounts it (via
 * `key`) whenever the credential or server it describes changes, so a receipt
 * never outlives the configuration it was taken against.
 */
export function IntegrationDiagnostics({
  target,
  className,
}: {
  target: "api" | "remote";
  className?: string;
}) {
  const { t } = useTranslation();
  const [result, setResult] = useState<Diagnostic | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const check = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      setResult(
        await invoke<Diagnostic>("check_integration_connection", { target }),
      );
    } catch (err) {
      setError(translateBackendError(t, err));
    } finally {
      setBusy(false);
    }
  };
  const verdict = (value: boolean | null | undefined) =>
    t(
      value == null
        ? "appFeedback.notVerified"
        : value
          ? "appFeedback.confirmed"
          : "appFeedback.unavailable",
    );
  return (
    <SettingsPanel
      data-slot="integration-diagnostics"
      className={cn("space-y-4", className)}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <h3 className="text-sm font-medium">
            {t("appFeedback.connectionTest")}
          </h3>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t(
              target === "api"
                ? "appFeedback.apiProbeScope"
                : "appFeedback.remoteProbeScope",
            )}
          </p>
        </div>
        <LoadingButton
          size="sm"
          variant="soft"
          className="h-8 gap-1.5 rounded-lg text-xs"
          isLoading={busy}
          onClick={() => void check()}
        >
          <LuActivity className="size-3.5" />
          {t("appFeedback.testConnection")}
        </LoadingButton>
      </div>
      <OperationFlow
        label={t("appFeedback.connectionTest")}
        // The station the probe stopped at: nothing configured stops at
        // "configured", unreachable at "reachable", a refused token at
        // "authorized".
        active={
          busy
            ? 1
            : !result
              ? 0
              : !result.configured
                ? 0
                : result.reachable === false
                  ? 1
                  : 2
        }
        busy={busy}
        failed={
          !!result &&
          (!result.configured ||
            result.reachable === false ||
            result.authorized === false)
        }
        steps={[
          {
            id: "configured",
            label: t("appFeedback.configured"),
            detail: verdict(result?.configured),
          },
          {
            id: "reachable",
            label: t("appFeedback.reachable"),
            detail: verdict(result?.reachable),
          },
          {
            id: "authorized",
            label: t("appFeedback.authorized"),
            detail: verdict(result?.authorized),
          },
        ]}
      />
      <div
        role="status"
        className="text-xs leading-relaxed text-muted-foreground tabular-nums"
      >
        {busy
          ? t("appFeedback.checking")
          : result
            ? t("proxyCheck.tooltipChecked", {
                time: new Date(result.checked_at * 1000).toLocaleString(),
              })
            : t("appFeedback.notChecked")}
        {result?.http_status != null && (
          <span className="ml-2 rounded-md bg-foreground/5 px-1.5 py-0.5 font-mono">
            HTTP {result.http_status}
          </span>
        )}
      </div>
      {error && (
        <p role="alert" className="break-words text-xs text-destructive-text">
          {error}
        </p>
      )}
    </SettingsPanel>
  );
}

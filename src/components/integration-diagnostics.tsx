"use client";

import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { OperationFlow } from "@/components/ui/operation-flow";
import { translateBackendError } from "@/lib/backend-errors";

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
}: {
  target: "api" | "remote";
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
    <section
      data-slot="integration-diagnostics"
      className="space-y-3 rounded-md bg-muted/30 p-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-medium">
          {t("appFeedback.connectionTest")}
        </h3>
        <Button
          size="sm"
          variant="secondary"
          disabled={busy}
          aria-busy={busy}
          onClick={() => void check()}
        >
          {t(busy ? "appFeedback.checking" : "appFeedback.testConnection")}
        </Button>
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
        className="text-xs leading-relaxed text-muted-foreground"
      >
        {busy
          ? t("appFeedback.checking")
          : result
            ? t("proxyCheck.tooltipChecked", {
                time: new Date(result.checked_at * 1000).toLocaleString(),
              })
            : t("appFeedback.notChecked")}
        {result?.http_status != null && (
          <span className="ml-2 font-mono">HTTP {result.http_status}</span>
        )}
      </div>
      <p className="text-xs leading-relaxed text-muted-foreground">
        {t(
          target === "api"
            ? "appFeedback.apiProbeScope"
            : "appFeedback.remoteProbeScope",
        )}
      </p>
      {error && (
        <p role="alert" className="break-words text-xs text-destructive-text">
          {error}
        </p>
      )}
    </section>
  );
}

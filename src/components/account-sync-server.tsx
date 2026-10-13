"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuServer } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import { SoftFields } from "@/components/ui/field-variant";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  FeatureIcon,
  SettingsPanel,
  StatusLight,
} from "@/components/ui/settings-panel";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { SyncServerCheck } from "@/types";

type Probe =
  | { state: "unknown" | "testing" | "unreachable" | "no-accounts" }
  | { state: "ready" }
  | { state: "mismatch"; address: string };

/** A quick check before saving. The backend compares the exact form. */
function sameAddress(a: string, b: string): boolean {
  const trim = (value: string) =>
    value.trim().replace(/\/+$/, "").toLowerCase();
  return trim(a) === trim(b);
}

/**
 * A signed-in paid account's own donut-sync server. The app signs in to it
 * with a token the Donut API binds to its address, so the plan's limit and
 * the team's rules apply there as on Donut cloud.
 */
export function AccountSyncServer({
  paid,
  teamSeat,
}: {
  paid: boolean;
  teamSeat: boolean;
}) {
  const { t } = useTranslation();
  const [current, setCurrent] = useState<string | null>(null);
  const [serverUrl, setServerUrl] = useState("");
  const [probe, setProbe] = useState<Probe>({ state: "unknown" });
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    void invoke<string | null>("get_account_sync_server")
      .then((server) => {
        setCurrent(server);
        setServerUrl(server ?? "");
      })
      .catch((error: unknown) => {
        console.error("Failed to load the account sync server:", error);
      });
  }, []);

  const restartSync = useCallback(async () => {
    try {
      await invoke("restart_sync_service");
    } catch (e) {
      console.error("Failed to restart sync service:", e);
    }
  }, []);

  const handleTest = useCallback(async () => {
    setProbe({ state: "testing" });
    try {
      const result = await invoke<SyncServerCheck>(
        "check_sync_server_connection",
        { serverUrl },
      );
      if (
        !result.server_reachable ||
        result.storage_ready === false ||
        result.storage_reachable === false
      ) {
        setProbe({ state: "unreachable" });
      } else if (!result.account_url) {
        setProbe({ state: "no-accounts" });
      } else if (!sameAddress(result.account_url, serverUrl)) {
        setProbe({ state: "mismatch", address: result.account_url });
      } else {
        setProbe({ state: "ready" });
      }
    } catch {
      setProbe({ state: "unreachable" });
    }
  }, [serverUrl]);

  const save = useCallback(
    async (next: string | null) => {
      setIsSaving(true);
      try {
        const saved = await invoke<string | null>("set_account_sync_server", {
          serverUrl: next,
        });
        setCurrent(saved);
        setServerUrl(saved ?? "");
        setProbe({ state: saved ? "ready" : "unknown" });
        await restartSync();
        showSuccessToast(
          saved
            ? t("account.ownServer.saved")
            : t("account.ownServer.backToCloud"),
        );
      } catch (error) {
        showErrorToast(translateBackendError(t as never, error));
      } finally {
        setIsSaving(false);
      }
    },
    [restartSync, t],
  );

  const probeTone = {
    unknown: "muted",
    testing: "muted",
    ready: "success",
    unreachable: "destructive",
    "no-accounts": "destructive",
    mismatch: "destructive",
  } as const;
  const probeLabel =
    probe.state === "mismatch"
      ? t("backendErrors.syncServerAddressMismatch", { address: probe.address })
      : {
          unknown: t("account.selfHosted.statusUnknown"),
          testing: t("appFeedback.checking"),
          ready: t("account.ownServer.accountsOn"),
          unreachable: t("backendErrors.syncServerUnreachable"),
          "no-accounts": t("backendErrors.syncServerNoAccounts"),
        }[probe.state];

  return (
    <SettingsPanel className="flex flex-col gap-4">
      <div className="flex items-start gap-3">
        <FeatureIcon active={current !== null}>
          <LuServer />
        </FeatureIcon>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{t("account.ownServer.title")}</p>
          <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
            {t("account.ownServer.description")}
          </p>
          <p className="mt-1.5 text-xs font-medium break-all">
            {current
              ? t("account.ownServer.current", { server: current })
              : t("account.ownServer.currentCloud")}
          </p>
        </div>
      </div>

      {paid ? (
        <>
          <SoftFields>
            <div className="space-y-1.5">
              <Label
                htmlFor="account-sync-server-url"
                className="text-xs font-medium text-muted-foreground"
              >
                {t("sync.serverUrl")}
              </Label>
              <Input
                id="account-sync-server-url"
                type="url"
                placeholder={t("sync.serverUrlPlaceholder")}
                value={serverUrl}
                onChange={(e) => {
                  setServerUrl(e.target.value);
                  setProbe({ state: "unknown" });
                }}
                autoComplete="off"
                spellCheck={false}
              />
            </div>
          </SoftFields>

          <ul className="flex flex-col gap-1 text-xs text-muted-foreground">
            <li>{t("account.ownServer.cloudOnly")}</li>
            {teamSeat && <li>{t("account.ownServer.teamNote")}</li>}
          </ul>

          <div className="flex flex-wrap items-center gap-2">
            <span
              role="status"
              className="mr-auto inline-flex min-w-0 items-center gap-2 text-xs text-muted-foreground"
            >
              <StatusLight
                tone={probeTone[probe.state]}
                live={probe.state === "testing" || probe.state === "ready"}
              />
              <span className="min-w-0 font-medium text-foreground">
                {probeLabel}
              </span>
            </span>
            {current && (
              <Button
                size="sm"
                variant="subtle"
                disabled={isSaving}
                onClick={() => void save(null)}
                className="h-8 rounded-lg text-xs"
              >
                {t("account.ownServer.useDonutCloud")}
              </Button>
            )}
            <LoadingButton
              size="sm"
              variant="soft"
              isLoading={probe.state === "testing"}
              disabled={!serverUrl.trim() || isSaving}
              onClick={() => void handleTest()}
              className="h-8 rounded-lg text-xs"
            >
              {t("account.selfHosted.testConnection")}
            </LoadingButton>
            <LoadingButton
              size="sm"
              isLoading={isSaving}
              disabled={
                !serverUrl.trim() ||
                probe.state === "testing" ||
                (current !== null && sameAddress(current, serverUrl))
              }
              onClick={() => void save(serverUrl)}
              className="h-8 rounded-lg text-xs"
            >
              {t("common.buttons.save")}
            </LoadingButton>
          </div>
        </>
      ) : (
        <p className="text-xs text-muted-foreground">
          {t("backendErrors.accountSyncPlanRequired")}
        </p>
      )}
    </SettingsPanel>
  );
}

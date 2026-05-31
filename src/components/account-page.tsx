"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuCloud,
  LuEye,
  LuEyeOff,
  LuLogOut,
  LuRefreshCw,
  LuUser,
} from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { SyncSettings } from "@/types";

interface AccountPageProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  onOpenSignIn: () => void;
}

type ConnectionStatus = "unknown" | "testing" | "connected" | "error";

export function AccountPage({
  isOpen,
  onClose,
  subPage,
  onOpenSignIn,
}: AccountPageProps) {
  const { t } = useTranslation();
  const {
    user,
    isLoggedIn,
    isLoading: isCloudLoading,
    logout,
    refreshProfile,
  } = useCloudAuth();
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isLoggingOut, setIsLoggingOut] = useState(false);

  // Self-hosted server state. Loaded once when the dialog opens and persisted
  // via `save_sync_settings` so the rest of the app picks up the new URL/token
  // from `SettingsManager`.
  const [serverUrl, setServerUrl] = useState("");
  const [token, setToken] = useState("");
  const [showToken, setShowToken] = useState(false);
  const [isSavingSelfHosted, setIsSavingSelfHosted] = useState(false);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [connectionStatus, setConnectionStatus] =
    useState<ConnectionStatus>("unknown");

  const hasConfig = Boolean(serverUrl && token);
  // Self-hosted and cloud are mutually exclusive — both share the same sync
  // engine and a profile can't be sync'd to two backends. The tab trigger is
  // disabled here AND the backend rejects mixed state (see `save_sync_settings`
  // / `cloud_logout`), so even if someone bypasses the UI we don't end up
  // with split-brain.
  const selfHostedDisabled = isLoggedIn || isCloudLoading;

  const handleRefresh = async () => {
    setIsRefreshing(true);
    try {
      await refreshProfile();
      showSuccessToast(t("account.refreshed"));
    } catch (e) {
      showErrorToast(String(e));
    } finally {
      setIsRefreshing(false);
    }
  };

  const handleLogout = async () => {
    setIsLoggingOut(true);
    try {
      await logout();
      // The backend wipes sync URL + token as part of cloud_logout (see
      // `cloud_auth::cloud_logout`); pull the now-empty settings back into
      // the form so a user who flips to the Self-hosted tab doesn't see the
      // pre-logout production URL still sitting there.
      await loadSelfHostedSettings();
      showSuccessToast(t("account.loggedOut"));
    } catch (e) {
      showErrorToast(String(e));
    } finally {
      setIsLoggingOut(false);
    }
  };

  const loadSelfHostedSettings = useCallback(async () => {
    try {
      const settings = await invoke<SyncSettings>("get_sync_settings");
      setServerUrl(settings.sync_server_url ?? "");
      setToken(settings.sync_token ?? "");
      setConnectionStatus(
        settings.sync_server_url && settings.sync_token ? "unknown" : "unknown",
      );
    } catch (error) {
      console.error("Failed to load sync settings:", error);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadSelfHostedSettings();
    }
  }, [isOpen, loadSelfHostedSettings]);

  const handleTestConnection = useCallback(async () => {
    if (!serverUrl) {
      showErrorToast(t("sync.config.serverUrlRequired"));
      return;
    }
    setIsTestingConnection(true);
    setConnectionStatus("testing");
    try {
      const healthUrl = `${serverUrl.replace(/\/$/, "")}/health`;
      const response = await fetch(healthUrl);
      if (response.ok) {
        setConnectionStatus("connected");
        showSuccessToast(t("sync.config.connectionSuccess"));
      } else {
        setConnectionStatus("error");
        showErrorToast(t("sync.config.serverError"));
      }
    } catch {
      setConnectionStatus("error");
      showErrorToast(t("sync.config.connectFailed"));
    } finally {
      setIsTestingConnection(false);
    }
  }, [serverUrl, t]);

  const handleSaveSelfHosted = useCallback(async () => {
    setIsSavingSelfHosted(true);
    try {
      await invoke<SyncSettings>("save_sync_settings", {
        syncServerUrl: serverUrl || null,
        syncToken: token || null,
      });
      try {
        await invoke("restart_sync_service");
      } catch (e) {
        console.error("Failed to restart sync service:", e);
      }
      showSuccessToast(t("sync.config.settingsSaved"));
    } catch (error) {
      console.error("Failed to save sync settings:", error);
      // Use the structured backend-error translator so the cloud-vs-self-
      // hosted mutex (`SELF_HOSTED_REQUIRES_LOGOUT`) shows a clear message
      // instead of the generic "save failed" toast.
      showErrorToast(translateBackendError(t as never, error));
    } finally {
      setIsSavingSelfHosted(false);
    }
  }, [serverUrl, token, t]);

  const handleDisconnectSelfHosted = useCallback(async () => {
    setIsSavingSelfHosted(true);
    try {
      await invoke<SyncSettings>("save_sync_settings", {
        syncServerUrl: null,
        syncToken: null,
      });
      try {
        await invoke("restart_sync_service");
      } catch (e) {
        console.error("Failed to restart sync service:", e);
      }
      setServerUrl("");
      setToken("");
      setConnectionStatus("unknown");
      showSuccessToast(t("sync.config.disconnected"));
    } catch (error) {
      console.error("Failed to disconnect:", error);
      showErrorToast(t("sync.config.disconnectFailed"));
    } finally {
      setIsSavingSelfHosted(false);
    }
  }, [t]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
      <DialogContent className="max-w-2xl flex flex-col">
        <div className="flex flex-col gap-4 p-4">
          <AnimatedTabs defaultValue="account">
            <AnimatedTabsList>
              <AnimatedTabsTrigger value="account">
                {t("account.tabs.account")}
              </AnimatedTabsTrigger>
              <AnimatedTabsTrigger
                value="self-hosted"
                disabled={selfHostedDisabled}
                title={
                  selfHostedDisabled
                    ? t("account.selfHosted.disabledWhileLoggedIn")
                    : undefined
                }
              >
                {t("account.tabs.selfHosted")}
              </AnimatedTabsTrigger>
            </AnimatedTabsList>

            <AnimatedTabsContent value="account" className="mt-4">
              <div className="flex flex-col gap-4">
                <div className="flex items-center gap-3">
                  <div className="grid place-items-center size-12 rounded-full bg-accent text-foreground shrink-0">
                    <LuUser className="size-6" />
                  </div>
                  <div className="min-w-0 flex-1">
                    {isLoggedIn && user ? (
                      <>
                        <h2 className="text-base font-semibold truncate">
                          {user.email}
                        </h2>
                        <p className="text-xs text-muted-foreground mt-0.5">
                          {t("account.plan", {
                            plan: user.plan,
                            period: user.planPeriod ?? "—",
                          })}
                        </p>
                      </>
                    ) : (
                      <>
                        <h2 className="text-base font-semibold">
                          {t("account.signedOut")}
                        </h2>
                        <p className="text-xs text-muted-foreground mt-0.5">
                          {t("account.signedOutDescription")}
                        </p>
                      </>
                    )}
                  </div>
                </div>

                {isLoggedIn && user && (
                  <div className="grid grid-cols-2 gap-2 text-xs">
                    <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
                      <p className="text-[10px] uppercase tracking-wide text-muted-foreground">
                        {t("account.fields.plan")}
                      </p>
                      <p className="mt-0.5 font-medium uppercase">
                        {user.plan}
                      </p>
                    </div>
                    <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
                      <p className="text-[10px] uppercase tracking-wide text-muted-foreground">
                        {t("account.fields.status")}
                      </p>
                      <p className="mt-0.5">{user.subscriptionStatus ?? "—"}</p>
                    </div>
                    {user.teamRole && (
                      <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
                        <p className="text-[10px] uppercase tracking-wide text-muted-foreground">
                          {t("account.fields.teamRole")}
                        </p>
                        <p className="mt-0.5">{user.teamRole}</p>
                      </div>
                    )}
                    {user.planPeriod && (
                      <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
                        <p className="text-[10px] uppercase tracking-wide text-muted-foreground">
                          {t("account.fields.period")}
                        </p>
                        <p className="mt-0.5">{user.planPeriod}</p>
                      </div>
                    )}
                    {typeof user.deviceOrdinal === "number" && (
                      <div className="rounded-md bg-muted/40 border border-border px-3 py-2">
                        <p className="text-[10px] uppercase tracking-wide text-muted-foreground">
                          {t("account.fields.device")}
                        </p>
                        <p className="mt-0.5">
                          {t("account.deviceOrdinal", {
                            ordinal: user.deviceOrdinal,
                            count: user.deviceCount ?? user.deviceOrdinal,
                          })}
                        </p>
                      </div>
                    )}
                  </div>
                )}

                {isLoggedIn &&
                  user &&
                  user.plan !== "free" &&
                  user.isPrimaryDevice === false && (
                    <p className="text-xs text-warning">
                      {t("account.automationPrimaryOnly")}
                    </p>
                  )}
                {isLoggedIn &&
                  user &&
                  user.plan !== "free" &&
                  user.isPrimaryDevice === true &&
                  (user.deviceCount ?? 1) > 1 && (
                    <p className="text-xs text-success">
                      {t("account.automationActiveHere")}
                    </p>
                  )}

                <div className="flex flex-wrap gap-2 mt-2">
                  {isLoggedIn ? (
                    <>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => {
                          void handleRefresh();
                        }}
                        disabled={isRefreshing}
                        className="h-8 text-xs gap-1.5"
                      >
                        <LuRefreshCw className="size-3" />
                        {t("account.refresh")}
                      </Button>
                      <LoadingButton
                        size="sm"
                        variant="destructive"
                        isLoading={isLoggingOut}
                        disabled={isRefreshing}
                        onClick={() => {
                          void handleLogout();
                        }}
                        className="h-8 text-xs gap-1.5"
                      >
                        <LuLogOut className="size-3" />
                        {t("account.logout")}
                      </LoadingButton>
                    </>
                  ) : (
                    <Button
                      size="sm"
                      onClick={onOpenSignIn}
                      className="h-8 text-xs gap-1.5"
                    >
                      <LuCloud className="size-3" />
                      {t("account.signIn")}
                    </Button>
                  )}
                </div>
              </div>
            </AnimatedTabsContent>

            <AnimatedTabsContent value="self-hosted" className="mt-4">
              {selfHostedDisabled ? (
                // Defensive: the tab trigger is disabled while the user is
                // logged in, so this branch shouldn't be reachable via UI —
                // but if state flips mid-render (e.g. a cloud login finishes
                // while the tab is open), show the explanation instead of
                // a silent empty card.
                <p className="text-sm text-muted-foreground">
                  {t("account.selfHosted.disabledWhileLoggedIn")}
                </p>
              ) : (
                <div className="flex flex-col gap-4">
                  <div>
                    <p className="text-sm font-medium">
                      {t("account.selfHosted.title")}
                    </p>
                    <p className="text-xs text-muted-foreground mt-0.5">
                      {t("account.selfHosted.description")}
                    </p>
                  </div>

                  <div className="space-y-1.5">
                    <Label htmlFor="self-hosted-server-url" className="text-xs">
                      {t("sync.serverUrl")}
                    </Label>
                    <Input
                      id="self-hosted-server-url"
                      type="url"
                      placeholder={t("sync.serverUrlPlaceholder")}
                      value={serverUrl}
                      onChange={(e) => {
                        setServerUrl(e.target.value);
                        setConnectionStatus("unknown");
                      }}
                      autoComplete="off"
                      spellCheck={false}
                    />
                  </div>

                  <div className="space-y-1.5">
                    <Label htmlFor="self-hosted-token" className="text-xs">
                      {t("sync.token")}
                    </Label>
                    <div className="relative">
                      <Input
                        id="self-hosted-token"
                        type={showToken ? "text" : "password"}
                        placeholder={t("sync.tokenPlaceholder")}
                        value={token}
                        onChange={(e) => {
                          setToken(e.target.value);
                          setConnectionStatus("unknown");
                        }}
                        autoComplete="off"
                        spellCheck={false}
                        className="pr-9"
                      />
                      <button
                        type="button"
                        onClick={() => {
                          setShowToken((v) => !v);
                        }}
                        aria-label={
                          showToken
                            ? t("common.aria.hideToken")
                            : t("common.aria.showToken")
                        }
                        className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-muted-foreground hover:text-foreground"
                      >
                        {showToken ? (
                          <LuEyeOff className="size-3.5" />
                        ) : (
                          <LuEye className="size-3.5" />
                        )}
                      </button>
                    </div>
                  </div>

                  <div className="flex items-center gap-2 text-xs">
                    <span className="text-muted-foreground">
                      {t("account.selfHosted.connectionStatus")}
                    </span>
                    {connectionStatus === "connected" && (
                      <Badge
                        variant="default"
                        className="text-success-foreground bg-success"
                      >
                        {t("sync.status.connected")}
                      </Badge>
                    )}
                    {connectionStatus === "error" && (
                      <Badge variant="destructive">
                        {t("sync.status.error")}
                      </Badge>
                    )}
                    {connectionStatus === "testing" && (
                      <Badge variant="secondary">
                        {t("sync.status.syncing")}
                      </Badge>
                    )}
                    {connectionStatus === "unknown" && (
                      <Badge variant="secondary">
                        {t("account.selfHosted.statusUnknown")}
                      </Badge>
                    )}
                  </div>

                  <div className="flex flex-wrap gap-2">
                    <LoadingButton
                      size="sm"
                      variant="outline"
                      isLoading={isTestingConnection}
                      disabled={!serverUrl || isSavingSelfHosted}
                      onClick={() => void handleTestConnection()}
                      className="h-8 text-xs"
                    >
                      {t("account.selfHosted.testConnection")}
                    </LoadingButton>
                    <LoadingButton
                      size="sm"
                      isLoading={isSavingSelfHosted}
                      disabled={!serverUrl || !token || isTestingConnection}
                      onClick={() => void handleSaveSelfHosted()}
                      className="h-8 text-xs"
                    >
                      {t("common.buttons.save")}
                    </LoadingButton>
                    {hasConfig && (
                      <Button
                        size="sm"
                        variant="destructive"
                        disabled={isSavingSelfHosted || isTestingConnection}
                        onClick={() => void handleDisconnectSelfHosted()}
                        className="h-8 text-xs"
                      >
                        {t("account.selfHosted.disconnect")}
                      </Button>
                    )}
                  </div>
                </div>
              )}
            </AnimatedTabsContent>
          </AnimatedTabs>
        </div>
      </DialogContent>
    </Dialog>
  );
}

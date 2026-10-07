"use client";

import { invoke } from "@tauri-apps/api/core";
import { type ReactNode, useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  LuArrowRight,
  LuCircleCheck,
  LuCloud,
  LuEye,
  LuEyeOff,
  LuLogOut,
  LuRefreshCw,
  LuServer,
  LuTriangleAlert,
  LuUser,
} from "react-icons/lu";
import {
  formatDate,
  formatHours,
  RemoteHoursMeter,
} from "@/components/cookie-bot-shared";
import { LoadingButton } from "@/components/loading-button";
import { TeamUsagePanel } from "@/components/team-usage-panel";
import {
  AnimatedTabs,
  AnimatedTabsContent,
  AnimatedTabsList,
  AnimatedTabsTrigger,
} from "@/components/ui/animated-tabs";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { SoftFields } from "@/components/ui/field-variant";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import {
  FeatureIcon,
  SECTION_LABEL_CLASS,
  SettingsPanel,
  StatusLight,
} from "@/components/ui/settings-panel";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { cookieBotScopeFor, useCookieBot } from "@/hooks/use-cookie-bot";
import { translateBackendError } from "@/lib/backend-errors";
import {
  canUseCookieBot,
  effectivePlanOf,
  getEntitlements,
  isTeamOwner,
} from "@/lib/entitlements";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import { cn } from "@/lib/utils";
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

  // Remote hours are plan truth, so they belong here rather than only next to
  // the controls that spend them. Until this landed, `remote-sessions/quota`
  // had no caller anywhere and a customer's first sight of their allowance was
  // a refused launch.
  const remoteHoursVisible = isLoggedIn && canUseCookieBot(user);
  const showTeamUsage = remoteHoursVisible && isTeamOwner(user);
  // A member's own row says "free" because the owner pays. The plan the seat
  // is served under is the one the customer expects to read here, and the
  // billing period slot names the seat instead, since a seat has no period.
  const effectivePlan = effectivePlanOf(user);
  const isTeamSeat = user != null && effectivePlan !== user.plan;
  const seatRole =
    user?.teamRole === "owner"
      ? t("sync.team.roleOwner")
      : user?.teamRole === "admin"
        ? t("sync.team.roleAdmin")
        : t("sync.team.roleMember");
  const seatLabel = user?.teamName
    ? t("account.teamSeat", { role: seatRole, team: user.teamName })
    : t("account.teamSeatUnnamed", { role: seatRole });
  const { quota, isLoading: isQuotaLoading } = useCookieBot(
    remoteHoursVisible,
    cookieBotScopeFor(user),
  );
  const [activeTab, setActiveTab] = useState("account");

  // Signing out (or losing the team) removes the tab while it is the selected
  // one, which would leave the page showing an empty panel with no trigger to
  // click back to.
  useEffect(() => {
    if (!showTeamUsage && activeTab === "team-usage") setActiveTab("account");
  }, [showTeamUsage, activeTab]);

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

  const subscriptionTone = (
    status: string | null | undefined,
  ): "success" | "warning" | "muted" => {
    if (status === "active" || status === "trialing") return "success";
    if (status === "past_due" || status === "unpaid") return "warning";
    return "muted";
  };

  const details: { id: string; label: string; value: ReactNode }[] = [];
  if (isLoggedIn && user) {
    details.push(
      {
        id: "plan",
        label: t("account.fields.plan"),
        value: <span className="uppercase">{effectivePlan}</span>,
      },
      {
        id: "status",
        label: t("account.fields.status"),
        value: (
          <span className="inline-flex items-center gap-1.5">
            <StatusLight tone={subscriptionTone(user.subscriptionStatus)} />
            {user.subscriptionStatus ?? "—"}
          </span>
        ),
      },
    );
    if (user.teamRole) {
      details.push({
        id: "teamRole",
        label: t("account.fields.teamRole"),
        value: user.teamRole,
      });
    }
    if (user.planPeriod) {
      details.push({
        id: "period",
        label: t("account.fields.period"),
        value: user.planPeriod,
      });
    }
    if (typeof user.deviceOrdinal === "number") {
      details.push({
        id: "device",
        label: t("account.fields.device"),
        value: t("account.deviceOrdinal", {
          ordinal: user.deviceOrdinal,
          count: user.deviceCount ?? user.deviceOrdinal,
        }),
      });
    }
  }

  const connectionTone = {
    unknown: "muted",
    testing: "muted",
    connected: "success",
    error: "destructive",
  } as const;
  const connectionLabel = {
    unknown: t("account.selfHosted.statusUnknown"),
    testing: t("appFeedback.checking"),
    connected: t("sync.status.connected"),
    error: t("sync.status.error"),
  };

  return (
    <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
      <DialogContent className="flex max-h-[calc(100vh-5rem)] max-w-3xl flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className={cn(subPage && "mx-auto w-full max-w-4xl")}>
            <AnimatedTabs value={activeTab} onValueChange={setActiveTab}>
              <AnimatedTabsList>
                <AnimatedTabsTrigger value="account">
                  {t("account.tabs.account")}
                </AnimatedTabsTrigger>
                {showTeamUsage && (
                  <AnimatedTabsTrigger value="team-usage">
                    {t("account.tabs.teamUsage")}
                  </AnimatedTabsTrigger>
                )}
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

              <AnimatedTabsContent
                value="account"
                className="@container mt-4 flex flex-col gap-3"
              >
                <SettingsPanel className="flex flex-col gap-4">
                  <div className="flex flex-wrap items-center gap-3">
                    <span
                      aria-hidden="true"
                      className={cn(
                        "grid size-11 shrink-0 place-items-center rounded-full text-base font-semibold uppercase",
                        isLoggedIn && user
                          ? "bg-primary/10 text-foreground"
                          : "bg-foreground/6 text-muted-foreground",
                      )}
                    >
                      {isLoggedIn && user ? (
                        user.email.charAt(0)
                      ) : (
                        <LuUser className="size-5" />
                      )}
                    </span>
                    <div className="min-w-0 flex-1">
                      {isLoggedIn && user ? (
                        <>
                          <h2 className="truncate text-sm font-semibold">
                            {user.email}
                          </h2>
                          <p className="mt-0.5 text-xs text-muted-foreground">
                            {t("account.plan", {
                              plan: effectivePlan,
                              period: isTeamSeat
                                ? seatLabel
                                : (user.planPeriod ?? "—"),
                            })}
                          </p>
                        </>
                      ) : (
                        <>
                          <h2 className="text-sm font-semibold">
                            {t("account.signedOut")}
                          </h2>
                          <p className="mt-0.5 text-xs text-muted-foreground">
                            {t("account.signedOutDescription")}
                          </p>
                        </>
                      )}
                    </div>
                    <div className="flex items-center gap-1.5">
                      {isLoggedIn ? (
                        <>
                          <Button
                            size="sm"
                            variant="soft"
                            onClick={() => {
                              void handleRefresh();
                            }}
                            disabled={isRefreshing || isLoggingOut}
                            aria-busy={isRefreshing}
                            className="h-8 gap-1.5 rounded-lg text-xs"
                          >
                            <LuRefreshCw
                              className={cn(
                                "size-3.5",
                                isRefreshing &&
                                  "animate-spin motion-reduce:animate-none",
                              )}
                            />
                            {t("account.refresh")}
                          </Button>
                          <LoadingButton
                            size="sm"
                            variant="soft"
                            isLoading={isLoggingOut}
                            disabled={isRefreshing}
                            onClick={() => {
                              void handleLogout();
                            }}
                            className="h-8 gap-1.5 rounded-lg text-xs text-destructive-text hover:bg-destructive/10"
                          >
                            <LuLogOut className="size-3.5" />
                            {t("account.logout")}
                          </LoadingButton>
                        </>
                      ) : (
                        <RippleButton
                          size="sm"
                          onClick={onOpenSignIn}
                          className="h-8 gap-1.5 rounded-lg text-xs"
                        >
                          <LuCloud className="size-3.5" />
                          {t("account.signIn")}
                        </RippleButton>
                      )}
                    </div>
                  </div>

                  {details.length > 0 && (
                    <dl className="grid grid-cols-2 gap-2 text-xs @xl:grid-cols-3">
                      {details.map((detail) => (
                        <div
                          key={detail.id}
                          className="min-w-0 rounded-lg bg-foreground/4 px-3 py-2"
                        >
                          <dt className="text-[10px] tracking-wide text-muted-foreground uppercase">
                            {detail.label}
                          </dt>
                          <dd className="mt-0.5 truncate font-medium">
                            {detail.value}
                          </dd>
                        </div>
                      ))}
                    </dl>
                  )}

                  {isLoggedIn &&
                    user &&
                    getEntitlements(user).browserAutomation &&
                    user.isPrimaryDevice === false && (
                      <p className="flex items-start gap-2 text-xs text-warning-text">
                        <LuTriangleAlert className="mt-0.5 size-3.5 shrink-0" />
                        {t("account.automationPrimaryOnly")}
                      </p>
                    )}
                  {isLoggedIn &&
                    user &&
                    getEntitlements(user).browserAutomation &&
                    user.isPrimaryDevice === true &&
                    (user.deviceCount ?? 1) > 1 && (
                      <p className="flex items-start gap-2 text-xs text-success-text">
                        <LuCircleCheck className="mt-0.5 size-3.5 shrink-0" />
                        {t("account.automationActiveHere")}
                      </p>
                    )}
                </SettingsPanel>

                {remoteHoursVisible && (
                  // A headline block, not one field among six: the allowance
                  // is the number a customer needs before a launch is
                  // refused, which is the only way they ever saw it before.
                  <SettingsPanel>
                    <div className="flex items-baseline justify-between gap-3">
                      <p className={SECTION_LABEL_CLASS}>
                        {t("cookieBot.hours.label")}
                      </p>
                      {formatDate(quota?.period_end) && (
                        <p className="text-xs tabular-nums text-muted-foreground">
                          {t("cookieBot.hours.resets", {
                            date: formatDate(quota?.period_end),
                          })}
                        </p>
                      )}
                    </div>
                    <p className="mt-2 text-2xl leading-none font-semibold tabular-nums">
                      {quota ? formatHours(quota.remaining_hours) : "—"}
                      <span className="ml-1.5 text-sm font-normal text-muted-foreground">
                        {t("cookieBot.hours.remainingOf", {
                          total: quota ? formatHours(quota.granted_hours) : "—",
                        })}
                      </span>
                    </p>
                    <RemoteHoursMeter
                      quota={quota}
                      isLoading={isQuotaLoading}
                      variant="inline"
                      className="mt-3"
                    />
                    <div className="mt-2 flex items-baseline justify-between gap-3">
                      <p className="text-xs tabular-nums text-muted-foreground">
                        {t("cookieBot.hours.used", {
                          used: quota ? formatHours(quota.used_hours) : "—",
                          total: quota ? formatHours(quota.granted_hours) : "—",
                        })}
                      </p>
                      {showTeamUsage && (
                        <Button
                          type="button"
                          variant="subtle"
                          size="sm"
                          onClick={() => {
                            setActiveTab("team-usage");
                          }}
                          className="group/team h-7 gap-1 rounded-lg px-2 text-xs"
                        >
                          {t("account.viewTeamUsage")}
                          <LuArrowRight className="size-3.5 transition-transform duration-150 group-hover/team:translate-x-0.5 motion-reduce:transition-none" />
                        </Button>
                      )}
                    </div>
                  </SettingsPanel>
                )}
              </AnimatedTabsContent>

              {showTeamUsage && (
                <AnimatedTabsContent value="team-usage" className="mt-4">
                  <TeamUsagePanel quota={quota} />
                </AnimatedTabsContent>
              )}

              <AnimatedTabsContent
                value="self-hosted"
                className="@container mt-4"
              >
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
                  <SettingsPanel className="flex flex-col gap-4">
                    <div className="flex items-start gap-3">
                      <FeatureIcon active={connectionStatus === "connected"}>
                        <LuServer />
                      </FeatureIcon>
                      <div className="min-w-0 flex-1">
                        <p className="text-sm font-medium">
                          {t("account.selfHosted.title")}
                        </p>
                        <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
                          {t("account.selfHosted.description")}
                        </p>
                      </div>
                    </div>

                    <SoftFields>
                      <div className="grid gap-4 @xl:grid-cols-2">
                        <div className="space-y-1.5">
                          <Label
                            htmlFor="self-hosted-server-url"
                            className={FIELD_LABEL_CLASS}
                          >
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
                          <Label
                            htmlFor="self-hosted-token"
                            className={FIELD_LABEL_CLASS}
                          >
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
                              className="pr-10"
                            />
                            <Button
                              type="button"
                              variant="subtle"
                              size="icon"
                              onClick={() => {
                                setShowToken((v) => !v);
                              }}
                              aria-label={
                                showToken
                                  ? t("common.aria.hideToken")
                                  : t("common.aria.showToken")
                              }
                              className="absolute top-1/2 right-1.5 size-6 -translate-y-1/2 rounded-md"
                            >
                              {showToken ? (
                                <LuEyeOff className="size-3.5" />
                              ) : (
                                <LuEye className="size-3.5" />
                              )}
                            </Button>
                          </div>
                        </div>
                      </div>
                    </SoftFields>

                    <div className="flex flex-wrap items-center gap-2">
                      <span
                        role="status"
                        className="mr-auto inline-flex items-center gap-2 text-xs text-muted-foreground"
                      >
                        <StatusLight
                          tone={connectionTone[connectionStatus]}
                          live={
                            connectionStatus === "testing" ||
                            connectionStatus === "connected"
                          }
                        />
                        <span>{t("account.selfHosted.connectionStatus")}</span>
                        <span className="font-medium text-foreground">
                          {connectionLabel[connectionStatus]}
                        </span>
                      </span>
                      {hasConfig && (
                        <Button
                          size="sm"
                          variant="subtle"
                          disabled={isSavingSelfHosted || isTestingConnection}
                          onClick={() => void handleDisconnectSelfHosted()}
                          className="h-8 rounded-lg text-xs hover:bg-destructive/10 hover:text-destructive-text"
                        >
                          {t("account.selfHosted.disconnect")}
                        </Button>
                      )}
                      <LoadingButton
                        size="sm"
                        variant="soft"
                        isLoading={isTestingConnection}
                        disabled={!serverUrl || isSavingSelfHosted}
                        onClick={() => void handleTestConnection()}
                        className="h-8 rounded-lg text-xs"
                      >
                        {t("account.selfHosted.testConnection")}
                      </LoadingButton>
                      <LoadingButton
                        size="sm"
                        isLoading={isSavingSelfHosted}
                        disabled={!serverUrl || !token || isTestingConnection}
                        onClick={() => void handleSaveSelfHosted()}
                        className="h-8 rounded-lg text-xs"
                      >
                        {t("common.buttons.save")}
                      </LoadingButton>
                    </div>
                  </SettingsPanel>
                )}
              </AnimatedTabsContent>
            </AnimatedTabs>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

const FIELD_LABEL_CLASS = "text-xs font-medium text-muted-foreground";

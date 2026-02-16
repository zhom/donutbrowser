"use client";

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuEye, LuEyeOff, LuLock } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { SyncSettings } from "@/types";

interface SyncConfigDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function SyncConfigDialog({ isOpen, onClose }: SyncConfigDialogProps) {
  const { t } = useTranslation();

  // Self-hosted state
  const [serverUrl, setServerUrl] = useState("");
  const [token, setToken] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isTesting, setIsTesting] = useState(false);
  const [showToken, setShowToken] = useState(false);

  // Cloud auth state
  const {
    user,
    isLoggedIn,
    isLoading: isCloudLoading,
    requestOtp,
    verifyOtp,
    logout,
  } = useCloudAuth();
  const [email, setEmail] = useState("");
  const [otpCode, setOtpCode] = useState("");
  const [codeSent, setCodeSent] = useState(false);
  const [isSendingCode, setIsSendingCode] = useState(false);
  const [isVerifying, setIsVerifying] = useState(false);

  const [activeTab, setActiveTab] = useState<string>("cloud");

  const isConnected = Boolean(serverUrl && token);

  const loadSettings = useCallback(async () => {
    setIsLoading(true);
    try {
      const settings = await invoke<SyncSettings>("get_sync_settings");
      setServerUrl(settings.sync_server_url || "");
      setToken(settings.sync_token || "");
    } catch (error) {
      console.error("Failed to load sync settings:", error);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      void loadSettings();
      setCodeSent(false);
      setOtpCode("");
      setEmail("");
    }
  }, [isOpen, loadSettings]);

  // Auto-select the appropriate tab based on connection state
  useEffect(() => {
    if (isCloudLoading) return;
    if (isLoggedIn) {
      setActiveTab("cloud");
    } else if (serverUrl && token) {
      setActiveTab("self-hosted");
    } else {
      setActiveTab("cloud");
    }
  }, [isCloudLoading, isLoggedIn, serverUrl, token]);

  const handleTestConnection = useCallback(async () => {
    if (!serverUrl) {
      showErrorToast("Please enter a server URL");
      return;
    }

    setIsTesting(true);
    try {
      const healthUrl = `${serverUrl.replace(/\/$/, "")}/health`;
      const response = await fetch(healthUrl);
      if (response.ok) {
        showSuccessToast("Connection successful!");
      } else {
        showErrorToast("Server responded with an error");
      }
    } catch {
      showErrorToast("Failed to connect to server");
    } finally {
      setIsTesting(false);
    }
  }, [serverUrl]);

  const handleSave = useCallback(async () => {
    setIsSaving(true);
    try {
      await invoke<SyncSettings>("save_sync_settings", {
        syncServerUrl: serverUrl || null,
        syncToken: token || null,
      });
      showSuccessToast("Sync settings saved");
      onClose();
    } catch (error) {
      console.error("Failed to save sync settings:", error);
      showErrorToast("Failed to save settings");
    } finally {
      setIsSaving(false);
    }
  }, [serverUrl, token, onClose]);

  const handleDisconnect = useCallback(async () => {
    setIsSaving(true);
    try {
      await invoke<SyncSettings>("save_sync_settings", {
        syncServerUrl: null,
        syncToken: null,
      });
      setServerUrl("");
      setToken("");
      showSuccessToast("Sync disconnected");
    } catch (error) {
      console.error("Failed to disconnect:", error);
      showErrorToast("Failed to disconnect");
    } finally {
      setIsSaving(false);
    }
  }, []);

  const handleSendCode = useCallback(async () => {
    if (!email) return;
    setIsSendingCode(true);
    try {
      await requestOtp(email);
      setCodeSent(true);
      showSuccessToast(t("sync.cloud.codeSent"));
    } catch (error) {
      console.error("Failed to send OTP:", error);
      showErrorToast(String(error));
    } finally {
      setIsSendingCode(false);
    }
  }, [email, requestOtp, t]);

  const handleVerifyOtp = useCallback(async () => {
    if (!email || !otpCode) return;
    setIsVerifying(true);
    try {
      await verifyOtp(email, otpCode);
      showSuccessToast(t("sync.cloud.loginSuccess"));
      // Restart sync service with cloud credentials
      try {
        await invoke("restart_sync_service");
      } catch (e) {
        console.error("Failed to restart sync service:", e);
      }
      // Auto-close dialog after successful login
      onClose();
    } catch (error) {
      console.error("OTP verification failed:", error);
      showErrorToast(String(error));
    } finally {
      setIsVerifying(false);
    }
  }, [email, otpCode, verifyOtp, t, onClose]);

  const handleCloudLogout = useCallback(async () => {
    try {
      await logout();
      showSuccessToast(t("sync.cloud.logoutSuccess"));
      // Restart sync service (will fall back to self-hosted or stop)
      try {
        await invoke("restart_sync_service");
      } catch (e) {
        console.error("Failed to restart sync service:", e);
      }
    } catch (error) {
      console.error("Failed to logout:", error);
      showErrorToast(String(error));
    }
  }, [logout, t]);

  // Determine which tabs are available
  const cloudBlocked = !isLoggedIn && isConnected;
  const selfHostedBlocked = isLoggedIn;

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("sync.title")}</DialogTitle>
          <DialogDescription>{t("sync.description")}</DialogDescription>
        </DialogHeader>

        {/* If cloud is logged in, don't show tabs at all - just show cloud account */}
        {isLoggedIn && user ? (
          <div className="grid gap-4 py-4">
            <div className="flex gap-2 items-center text-sm">
              <div className="w-2 h-2 rounded-full bg-green-500" />
              {t("sync.cloud.connected")}
            </div>

            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-muted-foreground">
                  {t("sync.cloud.email")}
                </span>
                <span>{user.email}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">
                  {t("sync.cloud.plan")}
                </span>
                <span className="capitalize">
                  {user.plan}
                  {user.planPeriod ? ` (${user.planPeriod})` : ""}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">
                  {t("sync.cloud.profiles")}
                </span>
                <span>
                  {t("sync.cloud.profileUsage", {
                    used: user.cloudProfilesUsed,
                    limit: user.profileLimit,
                  })}
                </span>
              </div>
              {user.proxyBandwidthLimitMb > 0 && (
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Proxy Bandwidth</span>
                  <span>
                    {user.proxyBandwidthUsedMb} / {user.proxyBandwidthLimitMb}{" "}
                    MB
                  </span>
                </div>
              )}
            </div>

            <div className="flex gap-2 pt-2">
              <Button variant="outline" className="flex-1" asChild>
                <a
                  href="https://donutbrowser.com/account"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {t("sync.cloud.manageAccount")}
                </a>
              </Button>
              <Button
                variant="outline"
                className="flex-1"
                onClick={handleCloudLogout}
              >
                {t("sync.cloud.logout")}
              </Button>
            </div>
          </div>
        ) : (
          <Tabs value={activeTab} onValueChange={setActiveTab}>
            <TabsList className="w-full">
              <TabsTrigger
                value="cloud"
                className="flex-1"
                disabled={cloudBlocked}
              >
                <span className="flex items-center gap-2">
                  {t("sync.cloud.tabLabel")}
                  {cloudBlocked && (
                    <LuLock className="w-3 h-3 text-muted-foreground" />
                  )}
                </span>
              </TabsTrigger>
              <TabsTrigger
                value="self-hosted"
                className="flex-1"
                disabled={selfHostedBlocked}
              >
                <span className="flex items-center gap-2">
                  {t("sync.cloud.selfHostedTabLabel")}
                  {selfHostedBlocked && (
                    <LuLock className="w-3 h-3 text-muted-foreground" />
                  )}
                </span>
              </TabsTrigger>
            </TabsList>

            <TabsContent value="cloud">
              {isCloudLoading ? (
                <div className="flex justify-center py-8">
                  <div className="w-6 h-6 rounded-full border-2 border-current animate-spin border-t-transparent" />
                </div>
              ) : (
                <div className="grid gap-4 py-4">
                  <div className="space-y-2">
                    <Label htmlFor="cloud-email">{t("sync.cloud.email")}</Label>
                    <div className="flex gap-2">
                      <Input
                        id="cloud-email"
                        type="email"
                        placeholder={t("sync.cloud.emailPlaceholder")}
                        value={email}
                        onChange={(e) => setEmail(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" && !codeSent) {
                            void handleSendCode();
                          }
                        }}
                      />
                      <LoadingButton
                        onClick={handleSendCode}
                        isLoading={isSendingCode}
                        disabled={!email || codeSent}
                        variant="outline"
                      >
                        {t("sync.cloud.sendCode")}
                      </LoadingButton>
                    </div>
                  </div>

                  {codeSent && (
                    <div className="space-y-2">
                      <Label htmlFor="cloud-otp">
                        {t("sync.cloud.verificationCode")}
                      </Label>
                      <Input
                        id="cloud-otp"
                        placeholder={t("sync.cloud.codePlaceholder")}
                        value={otpCode}
                        onChange={(e) => setOtpCode(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            void handleVerifyOtp();
                          }
                        }}
                      />
                      <LoadingButton
                        onClick={handleVerifyOtp}
                        isLoading={isVerifying}
                        disabled={!otpCode}
                        className="w-full"
                      >
                        {isVerifying
                          ? t("sync.cloud.loggingIn")
                          : t("sync.cloud.verifyAndLogin")}
                      </LoadingButton>
                    </div>
                  )}
                </div>
              )}
            </TabsContent>

            <TabsContent value="self-hosted">
              {isLoading ? (
                <div className="flex justify-center py-8">
                  <div className="w-6 h-6 rounded-full border-2 border-current animate-spin border-t-transparent" />
                </div>
              ) : (
                <div className="grid gap-4 py-4">
                  <div className="space-y-2">
                    <Label htmlFor="sync-server-url">
                      {t("sync.serverUrl")}
                    </Label>
                    <Input
                      id="sync-server-url"
                      placeholder={t("sync.serverUrlPlaceholder")}
                      value={serverUrl}
                      onChange={(e) => setServerUrl(e.target.value)}
                    />
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="sync-token">{t("sync.token")}</Label>
                    <div className="relative">
                      <Input
                        id="sync-token"
                        type={showToken ? "text" : "password"}
                        placeholder={t("sync.tokenPlaceholder")}
                        value={token}
                        onChange={(e) => setToken(e.target.value)}
                        className="pr-10"
                      />
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <button
                            type="button"
                            onClick={() => setShowToken(!showToken)}
                            className="absolute right-3 top-1/2 p-1 rounded-sm transition-colors transform -translate-y-1/2 hover:bg-accent"
                            aria-label={showToken ? "Hide token" : "Show token"}
                          >
                            {showToken ? (
                              <LuEyeOff className="w-4 h-4 text-muted-foreground hover:text-foreground" />
                            ) : (
                              <LuEye className="w-4 h-4 text-muted-foreground hover:text-foreground" />
                            )}
                          </button>
                        </TooltipTrigger>
                        <TooltipContent>
                          {showToken ? "Hide token" : "Show token"}
                        </TooltipContent>
                      </Tooltip>
                    </div>
                  </div>

                  {isConnected && (
                    <div className="flex gap-2 items-center text-sm text-muted-foreground">
                      <div className="w-2 h-2 rounded-full bg-green-500" />
                      {t("sync.status.connected")}
                    </div>
                  )}
                </div>
              )}

              <DialogFooter className="flex gap-2">
                {isConnected && (
                  <Button
                    variant="outline"
                    onClick={handleDisconnect}
                    disabled={isSaving}
                  >
                    Disconnect
                  </Button>
                )}
                <Button
                  variant="outline"
                  onClick={handleTestConnection}
                  disabled={isTesting || !serverUrl}
                >
                  {isTesting ? "Testing..." : "Test Connection"}
                </Button>
                <LoadingButton
                  onClick={handleSave}
                  isLoading={isSaving}
                  disabled={!serverUrl || !token}
                >
                  Save
                </LoadingButton>
              </DialogFooter>
            </TabsContent>
          </Tabs>
        )}
      </DialogContent>
    </Dialog>
  );
}

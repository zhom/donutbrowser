"use client";

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LuCloud, LuLogOut, LuRefreshCw, LuUser } from "react-icons/lu";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { useCloudAuth } from "@/hooks/use-cloud-auth";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";

interface AccountPageProps {
  isOpen: boolean;
  onClose: () => void;
  subPage?: boolean;
  onOpenSignIn: () => void;
}

export function AccountPage({
  isOpen,
  onClose,
  subPage,
  onOpenSignIn,
}: AccountPageProps) {
  const { t } = useTranslation();
  const { user, isLoggedIn, logout, refreshProfile } = useCloudAuth();
  const [isRefreshing, setIsRefreshing] = useState(false);

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
    try {
      await logout();
      showSuccessToast(t("account.loggedOut"));
    } catch (e) {
      showErrorToast(String(e));
    }
  };

  return (
    <Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
      <DialogContent className="max-w-2xl flex flex-col">
        <div className="flex flex-col gap-4 p-4">
          <div className="flex items-center gap-3">
            <div className="grid place-items-center w-12 h-12 rounded-full bg-accent text-foreground shrink-0">
              <LuUser className="w-6 h-6" />
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
                <p className="mt-0.5 font-medium uppercase">{user.plan}</p>
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
            </div>
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
                  <LuRefreshCw className="w-3 h-3" />
                  {t("account.refresh")}
                </Button>
                <Button
                  size="sm"
                  variant="destructive"
                  onClick={() => {
                    void handleLogout();
                  }}
                  className="h-8 text-xs gap-1.5"
                >
                  <LuLogOut className="w-3 h-3" />
                  {t("account.logout")}
                </Button>
              </>
            ) : (
              <Button
                size="sm"
                onClick={onOpenSignIn}
                className="h-8 text-xs gap-1.5"
              >
                <LuCloud className="w-3 h-3" />
                {t("account.signIn")}
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

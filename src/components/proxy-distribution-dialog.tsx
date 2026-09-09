"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { LoadingButton } from "@/components/loading-button";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type {
  BrowserProfile,
  ProxyAssignmentResult,
  ProxyDistributionPlan,
  StoredProxy,
} from "@/types";
import { RippleButton } from "./ui/ripple";

interface ProxyDistributionDialogProps {
  isOpen: boolean;
  onClose: () => void;
  selectedProfiles: string[];
  profiles: BrowserProfile[];
  storedProxies: StoredProxy[];
  onDistributionComplete: () => void;
}

const EMPTY_PLAN: ProxyDistributionPlan = {
  pairs: [],
  unpaired_profile_ids: [],
  unused_proxy_ids: [],
  running_profile_ids: [],
  shared_proxy_ids: [],
};

/** Toggle one id in a selection while keeping the source list's order. */
function toggle(selection: string[], id: string): string[] {
  return selection.includes(id)
    ? selection.filter((value) => value !== id)
    : [...selection, id];
}

export function ProxyDistributionDialog({
  isOpen,
  onClose,
  selectedProfiles,
  profiles,
  storedProxies,
  onDistributionComplete,
}: ProxyDistributionDialogProps) {
  const { t } = useTranslation();
  const rowId = useId();
  const [profileIds, setProfileIds] = useState<string[]>([]);
  const [proxyIds, setProxyIds] = useState<string[]>([]);
  const [allowSharing, setAllowSharing] = useState(false);
  const [plan, setPlan] = useState<ProxyDistributionPlan>(EMPTY_PLAN);
  const [failures, setFailures] = useState<ProxyAssignmentResult[]>([]);
  const [isDistributing, setIsDistributing] = useState(false);
  const planRequest = useRef(0);

  // A cloud proxy is issued per profile by the account, so handing one out
  // positionally would fight the thing that already owns it.
  const candidateProxies = useMemo(
    () =>
      storedProxies.filter(
        (proxy) => !proxy.is_cloud_managed && !proxy.is_cloud_derived,
      ),
    [storedProxies],
  );

  const candidateProfiles = useMemo(
    () => profiles.filter((profile) => selectedProfiles.includes(profile.id)),
    [profiles, selectedProfiles],
  );

  useEffect(() => {
    if (!isOpen) return;
    // Defaults that protect the user: only profiles that have no exit yet are
    // pre-ticked, so opening this on a mixed selection cannot move a profile
    // that is already routed.
    setProfileIds(
      candidateProfiles
        .filter((profile) => !profile.proxy_id && !profile.vpn_id)
        .map((profile) => profile.id),
    );
    setProxyIds(candidateProxies.map((proxy) => proxy.id));
    setAllowSharing(false);
    setFailures([]);
  }, [isOpen, candidateProfiles, candidateProxies]);

  useEffect(() => {
    if (!isOpen) return;
    if (profileIds.length === 0 || proxyIds.length === 0) {
      setPlan(EMPTY_PLAN);
      return;
    }
    const request = ++planRequest.current;
    void invoke<ProxyDistributionPlan>("plan_proxy_distribution", {
      profileIds,
      proxyIds,
      allowSharing,
    })
      .then((next) => {
        if (planRequest.current === request) setPlan(next);
      })
      .catch((error: unknown) => {
        console.error("Failed to plan proxy distribution:", error);
        if (planRequest.current === request) setPlan(EMPTY_PLAN);
      });
  }, [isOpen, profileIds, proxyIds, allowSharing]);

  // An empty list has nothing to select or clear, so the toggle says "Select
  // all" and is disabled rather than offering a click that does nothing.
  const allProfilesPicked =
    candidateProfiles.length > 0 &&
    profileIds.length === candidateProfiles.length;
  const allProxiesPicked =
    candidateProxies.length > 0 && proxyIds.length === candidateProxies.length;

  const nameOfProfile = useCallback(
    (id: string) => profiles.find((profile) => profile.id === id)?.name ?? id,
    [profiles],
  );
  const nameOfProxy = useCallback(
    (id: string) => storedProxies.find((proxy) => proxy.id === id)?.name ?? id,
    [storedProxies],
  );

  const handleDistribute = useCallback(async () => {
    setIsDistributing(true);
    setFailures([]);
    try {
      const results = await invoke<ProxyAssignmentResult[]>(
        "distribute_proxies_to_profiles",
        { pairs: plan.pairs },
      );
      const failed = results.filter((result) => !result.ok);
      const assigned = results.length - failed.length;
      setFailures(failed);
      await emit("profile-updated");
      onDistributionComplete();
      if (failed.length === 0) {
        showSuccessToast(t("proxyDistribution.assigned", { n: assigned }));
        onClose();
        return;
      }
      showErrorToast(
        t("proxyDistribution.partial", {
          assigned,
          failed: failed.length,
        }),
      );
    } catch (error) {
      console.error("Failed to distribute proxies:", error);
      showErrorToast(translateBackendError(t, error));
    } finally {
      setIsDistributing(false);
    }
  }, [plan.pairs, onClose, onDistributionComplete, t]);

  const summaryLines = useMemo(() => {
    const lines: { key: string; text: string }[] = [
      {
        key: "pairs",
        text: t("proxyDistribution.summaryPairs", {
          n: plan.pairs.length,
          profiles: profileIds.length,
        }),
      },
    ];
    if (plan.unpaired_profile_ids.length > 0) {
      lines.push({
        key: "unpaired",
        text: t("proxyDistribution.summaryUnpaired", {
          n: plan.unpaired_profile_ids.length,
          names: plan.unpaired_profile_ids.map(nameOfProfile).join(", "),
        }),
      });
    }
    if (plan.unused_proxy_ids.length > 0) {
      lines.push({
        key: "unused",
        text: t("proxyDistribution.summaryUnused", {
          n: plan.unused_proxy_ids.length,
          names: plan.unused_proxy_ids.map(nameOfProxy).join(", "),
        }),
      });
    }
    if (plan.shared_proxy_ids.length > 0) {
      lines.push({
        key: "shared",
        text: t("proxyDistribution.summaryShared", {
          n: plan.shared_proxy_ids.length,
          names: plan.shared_proxy_ids.map(nameOfProxy).join(", "),
        }),
      });
    }
    if (plan.running_profile_ids.length > 0) {
      lines.push({
        key: "running",
        text: t("proxyDistribution.summaryRunning", {
          n: plan.running_profile_ids.length,
          names: plan.running_profile_ids.map(nameOfProfile).join(", "),
        }),
      });
    }
    return lines;
  }, [plan, profileIds.length, nameOfProfile, nameOfProxy, t]);

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="flex max-h-[85vh] max-w-2xl flex-col">
        <DialogHeader>
          <DialogTitle>{t("proxyDistribution.title")}</DialogTitle>
          <DialogDescription>
            {t("proxyDistribution.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <div className="flex items-center justify-between gap-2">
                <Label>{t("proxyDistribution.profilesLabel")}</Label>
                <RippleButton
                  variant="ghost"
                  size="sm"
                  data-testid="distribute-toggle-profiles"
                  disabled={candidateProfiles.length === 0}
                  onClick={() => {
                    setProfileIds(
                      allProfilesPicked
                        ? []
                        : candidateProfiles.map((profile) => profile.id),
                    );
                  }}
                >
                  {allProfilesPicked
                    ? t("proxyDistribution.selectNone")
                    : t("proxyDistribution.selectAll")}
                </RippleButton>
              </div>
              <div className="max-h-56 space-y-2 overflow-y-auto rounded-md bg-muted p-3">
                {candidateProfiles.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    {t("proxyDistribution.noProfiles")}
                  </p>
                ) : (
                  candidateProfiles.map((profile) => (
                    <label
                      key={profile.id}
                      htmlFor={`${rowId}-profile-${profile.id}`}
                      className="flex cursor-pointer items-center gap-2 text-sm"
                    >
                      <Checkbox
                        id={`${rowId}-profile-${profile.id}`}
                        checked={profileIds.includes(profile.id)}
                        onCheckedChange={() => {
                          setProfileIds((current) =>
                            toggle(current, profile.id),
                          );
                        }}
                        aria-label={profile.name}
                      />
                      <span className="min-w-0 flex-1 truncate">
                        {profile.name}
                      </span>
                      {profile.proxy_id ? (
                        <span className="shrink-0 text-xs text-muted-foreground">
                          {nameOfProxy(profile.proxy_id)}
                        </span>
                      ) : null}
                    </label>
                  ))
                )}
              </div>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between gap-2">
                <Label>{t("proxyDistribution.proxiesLabel")}</Label>
                <RippleButton
                  variant="ghost"
                  size="sm"
                  data-testid="distribute-toggle-proxies"
                  disabled={candidateProxies.length === 0}
                  onClick={() => {
                    setProxyIds(
                      allProxiesPicked
                        ? []
                        : candidateProxies.map((proxy) => proxy.id),
                    );
                  }}
                >
                  {allProxiesPicked
                    ? t("proxyDistribution.selectNone")
                    : t("proxyDistribution.selectAll")}
                </RippleButton>
              </div>
              <div className="max-h-56 space-y-2 overflow-y-auto rounded-md bg-muted p-3">
                {candidateProxies.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    {t("proxyDistribution.noProxies")}
                  </p>
                ) : (
                  candidateProxies.map((proxy) => (
                    <label
                      key={proxy.id}
                      htmlFor={`${rowId}-proxy-${proxy.id}`}
                      className="flex cursor-pointer items-center gap-2 text-sm"
                    >
                      <Checkbox
                        id={`${rowId}-proxy-${proxy.id}`}
                        checked={proxyIds.includes(proxy.id)}
                        onCheckedChange={() => {
                          setProxyIds((current) => toggle(current, proxy.id));
                        }}
                        aria-label={proxy.name}
                      />
                      <span className="min-w-0 flex-1 truncate">
                        {proxy.name}
                      </span>
                    </label>
                  ))
                )}
              </div>
            </div>
          </div>

          <label
            htmlFor={`${rowId}-allow-sharing`}
            className="flex cursor-pointer items-start justify-between gap-4 rounded-md border border-border p-3"
          >
            <span className="space-y-1">
              <span className="block text-sm font-medium">
                {t("proxyDistribution.allowSharingLabel")}
              </span>
              <span className="block text-xs text-muted-foreground">
                {t("proxyDistribution.allowSharingHint")}
              </span>
            </span>
            <AnimatedSwitch
              id={`${rowId}-allow-sharing`}
              checked={allowSharing}
              onCheckedChange={setAllowSharing}
              aria-label={t("proxyDistribution.allowSharingLabel")}
            />
          </label>

          <div
            data-testid="distribute-summary"
            className="space-y-1 rounded-md bg-muted p-3 text-sm"
          >
            {summaryLines.map((line) => (
              <p
                key={line.key}
                className={
                  line.key === "pairs" ? "font-medium" : "text-muted-foreground"
                }
              >
                {line.text}
              </p>
            ))}
          </div>

          {failures.length > 0 && (
            <div className="space-y-1 rounded-md bg-destructive/10 p-3 text-sm text-destructive-text">
              <p className="font-medium">
                {t("proxyDistribution.failuresTitle", {
                  n: failures.length,
                })}
              </p>
              {failures.map((failure) => (
                <p key={failure.profile_id}>
                  {t("proxyDistribution.failureLine", {
                    name: nameOfProfile(failure.profile_id),
                    reason: translateBackendError(t, failure.error ?? ""),
                  })}
                </p>
              ))}
            </div>
          )}
        </div>

        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={onClose}
            disabled={isDistributing}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            isLoading={isDistributing}
            disabled={plan.pairs.length === 0}
            onClick={() => void handleDistribute()}
          >
            {t("proxyDistribution.distributeButton")}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

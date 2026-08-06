"use client";

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuTriangleAlert } from "react-icons/lu";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { translateBackendError } from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { ConsistencyResult, DetectedVpnExtension } from "@/types";
import { RippleButton } from "./ui/ripple";

export interface GateFindings {
  /// Extensions that can reroute traffic. A warning: the user may proceed.
  vpnExtensions: DetectedVpnExtension[];
  scanState: string;
  /// A measured exit/fingerprint mismatch. A block: the browser has not started.
  fingerprint: ConsistencyResult | null;
  /// A confirmed proxy-permission extension makes any exit measurement suspect.
  measurementUnreliable: boolean;
  /// The exit has not been measured yet; the launch itself will still check.
  probePending: boolean;
}

export interface GateDecision {
  proceed: boolean;
  ackFingerprint: boolean;
  ackExtensionKeys: string[];
  applyToRemaining: boolean;
}

interface PreLaunchGateDialogProps {
  isOpen: boolean;
  profileName: string;
  profileId: string;
  findings: GateFindings | null;
  /// How many further profiles are queued behind this one; >0 offers to apply
  /// the same decision to all of them.
  remainingCount: number;
  /// The single exit route. Every path out of this dialog calls it exactly
  /// once, so a caller awaiting a decision can never be left hanging.
  onResult: (decision: GateDecision) => void;
}

export function PreLaunchGateDialog({
  isOpen,
  profileName,
  profileId,
  findings,
  remainingCount,
  onResult,
}: PreLaunchGateDialogProps) {
  const { t } = useTranslation();
  const [ackFingerprint, setAckFingerprint] = useState(false);
  const [ackExtensions, setAckExtensions] = useState(false);
  const [applyToRemaining, setApplyToRemaining] = useState(false);
  const [isMatching, setIsMatching] = useState(false);
  // The dialog node is reused as the queue advances, so without this a double
  // click would decide for the next profile too.
  const [decided, setDecided] = useState(false);

  // Keyed on profileId, not just isOpen: a queued gate promotes the next
  // profile without ever closing the dialog, so an isOpen-only reset would
  // carry the previous profile's ticked boxes — and persist an acknowledgement
  // against a profile the user never saw.
  useEffect(() => {
    setAckFingerprint(false);
    setAckExtensions(false);
    setIsMatching(false);
    setDecided(false);
  }, []);

  useEffect(() => {
    if (isOpen) {
      setApplyToRemaining(false);
    }
  }, [isOpen]);

  const fingerprint = findings?.fingerprint ?? null;
  const extensions = findings?.vpnExtensions ?? [];
  const mismatches = fingerprint?.mismatches ?? [];
  const exitIp = fingerprint?.exit_ip ?? null;
  const isBlocked = fingerprint !== null;

  const decide = (proceed: boolean) => {
    if (decided) {
      return;
    }
    setDecided(true);
    onResult({
      proceed,
      ackFingerprint: ackFingerprint && isBlocked,
      ackExtensionKeys: ackExtensions ? extensions.map((e) => e.key) : [],
      applyToRemaining,
    });
  };

  const handleMatchFingerprint = async () => {
    if (!exitIp) {
      return;
    }
    setIsMatching(true);
    try {
      await invoke("match_profile_fingerprint_to_exit", {
        profileId,
        exitIp,
      });
      showSuccessToast(t("consistencyWarning.matchSuccess"));
      // The fingerprint the block was measured against no longer exists, so
      // this launch is abandoned rather than forced through with a stale
      // consent token; the user relaunches against the corrected profile.
      decide(false);
    } catch (e) {
      showErrorToast(translateBackendError(t, e));
    } finally {
      setIsMatching(false);
    }
  };

  const scanNotice = (() => {
    switch (findings?.scanState) {
      case "encrypted":
        return t("prelaunchGate.scanIncompleteEncrypted");
      case "ephemeral":
        return t("prelaunchGate.scanIncompleteEphemeral");
      case "partial":
        return t("prelaunchGate.scanIncompletePartial");
      case "missing":
        return t("prelaunchGate.scanIncompleteMissing");
      default:
        return null;
    }
  })();

  return (
    <Dialog open={isOpen}>
      <DialogContent className="sm:max-w-md" dismissible={false}>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <LuTriangleAlert className="size-5 text-warning-text" />
            {isBlocked
              ? t("prelaunchGate.titleBlocked")
              : t("prelaunchGate.titleWarning")}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-3 text-sm">
          <p className="text-muted-foreground">
            {t("prelaunchGate.intro", { name: profileName })}
          </p>

          {isBlocked && (
            <div className="space-y-2 rounded-md border border-destructive/50 bg-destructive/10 p-3">
              <p className="font-medium">
                {t("prelaunchGate.fingerprintHeading")}
              </p>
              {mismatches.includes("timezone") && (
                <p className="text-xs text-muted-foreground">
                  {t("consistencyWarning.timezoneDetail", {
                    exit: fingerprint?.exit_timezone ?? "?",
                    fingerprint: fingerprint?.fingerprint_timezone ?? "?",
                  })}
                </p>
              )}
              {mismatches.includes("language") && (
                <p className="text-xs text-muted-foreground">
                  {t("consistencyWarning.languageDetail", {
                    country: fingerprint?.exit_country_code ?? "?",
                    fingerprint: fingerprint?.fingerprint_language ?? "?",
                  })}
                </p>
              )}
              <p className="text-xs text-muted-foreground">
                {t("consistencyWarning.explainer")}
              </p>
            </div>
          )}

          {extensions.length > 0 && (
            <div className="space-y-2 rounded-md border border-warning/50 bg-warning/10 p-3">
              <p className="font-medium">
                {t("prelaunchGate.vpnExtensionHeading")}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("prelaunchGate.vpnExtensionIntro")}
              </p>
              <ul className="space-y-1">
                {extensions.map((ext) => (
                  <li key={ext.key} className="text-xs">
                    <span className="font-medium">{ext.name}</span>
                    <span className="text-muted-foreground">
                      {t("prelaunchGate.vpnExtensionEntry", {
                        version: ext.version ?? "",
                        capability:
                          ext.confidence === "confirmed"
                            ? t("prelaunchGate.vpnExtensionConfirmed")
                            : t("prelaunchGate.vpnExtensionLikely"),
                        source:
                          ext.source === "donut"
                            ? t("prelaunchGate.sourceDonut")
                            : t("prelaunchGate.sourceBrowser"),
                      })}
                    </span>
                  </li>
                ))}
              </ul>
              <p className="text-xs text-muted-foreground">
                {t("prelaunchGate.vpnExtensionExplainer")}
              </p>
            </div>
          )}

          {findings?.measurementUnreliable && isBlocked && (
            <p className="text-xs text-muted-foreground">
              {t("prelaunchGate.measurementUnreliable")}
            </p>
          )}

          {findings?.probePending && !isBlocked && (
            <p className="text-xs text-muted-foreground">
              {t("prelaunchGate.probePending")}
            </p>
          )}

          {scanNotice && (
            <p className="text-xs text-muted-foreground">{scanNotice}</p>
          )}

          <div className="space-y-2">
            {isBlocked && (
              <div className="flex items-center gap-x-2">
                <Checkbox
                  id="gate-ack-fingerprint"
                  checked={ackFingerprint}
                  onCheckedChange={(v) => setAckFingerprint(v === true)}
                />
                <Label htmlFor="gate-ack-fingerprint" className="text-xs">
                  {t("prelaunchGate.dontBlockAgain")}
                </Label>
              </div>
            )}
            {extensions.length > 0 && (
              <div className="flex items-center gap-x-2">
                <Checkbox
                  id="gate-ack-extensions"
                  checked={ackExtensions}
                  onCheckedChange={(v) => setAckExtensions(v === true)}
                />
                <Label htmlFor="gate-ack-extensions" className="text-xs">
                  {t("prelaunchGate.dontWarnExtensions")}
                </Label>
              </div>
            )}
            {remainingCount > 0 && (
              <div className="flex items-center gap-x-2">
                <Checkbox
                  id="gate-apply-remaining"
                  checked={applyToRemaining}
                  onCheckedChange={(v) => setApplyToRemaining(v === true)}
                />
                <Label htmlFor="gate-apply-remaining" className="text-xs">
                  {t("prelaunchGate.applyToRemaining")}
                </Label>
              </div>
            )}
          </div>
        </div>

        <DialogFooter className="flex-row justify-between sm:justify-between">
          {/* Cancel is the default action: the browser has not started, and
              not starting it is the safe outcome. */}
          <RippleButton
            variant="outline"
            onClick={() => decide(false)}
            disabled={isMatching || decided}
            autoFocus
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <div className="flex gap-2">
            {isBlocked && exitIp && (
              <RippleButton
                variant="outline"
                onClick={() => void handleMatchFingerprint()}
                disabled={isMatching || decided}
              >
                {isMatching
                  ? t("consistencyWarning.matching")
                  : t("consistencyWarning.matchToProxy")}
              </RippleButton>
            )}
            <RippleButton
              variant={isBlocked ? "destructive" : "default"}
              onClick={() => decide(true)}
              disabled={isMatching || decided}
            >
              {t("prelaunchGate.launchAnyway")}
            </RippleButton>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

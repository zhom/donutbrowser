"use client";

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
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
import type {
  ConsistencyResult,
  DetectedVpnExtension,
  ExtensionScanState,
} from "@/types";
import { RippleButton } from "./ui/ripple";

export interface GateFindings {
  /// Extensions that could reroute traffic. A warning: the user may proceed.
  vpnExtensions: DetectedVpnExtension[];
  scanState: ExtensionScanState;
  /// A measured exit/fingerprint mismatch. A block: the browser has not started.
  fingerprint: ConsistencyResult | null;
  /// An extension holds the proxy permission, so the exit measurement may not
  /// describe the route the browser takes. A caveat on the block, not a waiver.
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
  /// Identifies this specific request, so state resets even when one gate
  /// replaces another without the dialog ever closing.
  requestId: number;
  findings: GateFindings | null;
  /// How many further profiles are queued behind this one; >0 offers to apply
  /// the same decision to all of them.
  remainingCount: number;
  /// The single exit route. Every path out of this dialog calls it exactly
  /// once, so a caller awaiting a decision can never be left hanging.
  onResult: (decision: GateDecision) => void;
}

/// Everything the user can change while one gate is on screen, stamped with
/// the gate it belongs to.
interface GateAnswerState {
  requestId: number;
  ackFingerprint: boolean;
  ackExtensions: boolean;
  applyToRemaining: boolean;
  isMatching: boolean;
  decided: boolean;
}

/// How long after a decision the footer stops accepting another one. Long
/// enough that a double-click cannot answer the gate promoted by its first
/// half, short enough that nobody deliberately answering two queued gates in a
/// row notices it.
const DECISION_COOLDOWN_MS = 500;

function answersFor(requestId: number): GateAnswerState {
  return {
    requestId,
    ackFingerprint: false,
    ackExtensions: false,
    applyToRemaining: false,
    isMatching: false,
    decided: false,
  };
}

function ExtensionEntry({ extension }: { extension: DetectedVpnExtension }) {
  const { t } = useTranslation();
  const capability = t(
    extension.confidence === "confirmed"
      ? "prelaunchGate.vpnExtensionConfirmed"
      : extension.confidence === "likely"
        ? "prelaunchGate.vpnExtensionLikely"
        : "prelaunchGate.vpnExtensionCapability",
  );
  const source = t(
    extension.source === "donut"
      ? "prelaunchGate.sourceDonut"
      : "prelaunchGate.sourceBrowser",
  );

  return (
    <li className="text-xs">
      <span className="font-medium">{extension.name}</span>
      <span className="text-muted-foreground">
        {/* A version-less manifest is legal, and interpolating an empty string
            into the one template left a doubled space before the dash. */}
        {extension.version
          ? t("prelaunchGate.vpnExtensionEntry", {
              version: extension.version,
              capability,
              source,
            })
          : t("prelaunchGate.vpnExtensionEntryNoVersion", {
              capability,
              source,
            })}
      </span>
    </li>
  );
}

export function PreLaunchGateDialog({
  isOpen,
  profileName,
  profileId,
  requestId,
  findings,
  remainingCount,
  onResult,
}: PreLaunchGateDialogProps) {
  const { t } = useTranslation();
  // All mutable state is stamped with the request it belongs to, and anything
  // stamped with an older request is ignored rather than reset. The dialog
  // never unmounts and a queued gate promotes the next profile without ever
  // closing it, so state carried across that boundary would tick a checkbox
  // for a profile the user never saw — and `decided` carried across it left
  // every button disabled on a dialog that also refused Escape, which is the
  // freeze this shape exists to make unrepresentable.
  //
  // Deliberately not an effect keyed on `requestId`: a reset effect whose body
  // reads none of its dependencies is exactly what a lint autofix reduces to
  // `[]`, and that is how the freeze shipped.
  const [state, setState] = useState<GateAnswerState>(() => answersFor(0));
  const answers = state.requestId === requestId ? state : answersFor(requestId);

  // The gate on screen right now, readable from an async callback whose
  // closure was captured while an earlier gate was showing.
  const liveRequestRef = useRef(requestId);
  useEffect(() => {
    liveRequestRef.current = requestId;
  }, [requestId]);

  const patch = (next: Partial<GateAnswerState>) => {
    // A callback that resumes after its gate was answered must not write into
    // the slot the next gate is now using — that would silently untick boxes
    // the user has since ticked on a different profile.
    if (liveRequestRef.current !== requestId) {
      return;
    }
    setState((prev) => ({
      ...(prev.requestId === requestId ? prev : answersFor(requestId)),
      ...next,
      requestId,
    }));
  };
  const {
    ackFingerprint,
    ackExtensions,
    applyToRemaining,
    isMatching,
    decided,
  } = answers;

  const fingerprint = findings?.fingerprint ?? null;
  const extensions = findings?.vpnExtensions ?? [];
  // Two different claims, kept visually apart. The first names extensions as
  // VPN/proxy tools; the second says only that an extension holds Chromium's
  // proxy permission, which a download manager needs to route its own
  // transfers and which says nothing about what the extension is.
  const vpnExtensions = extensions.filter((e) => e.confidence !== "capability");
  const proxyCapableExtensions = extensions.filter(
    (e) => e.confidence === "capability",
  );
  const mismatches = fingerprint?.mismatches ?? [];
  const exitIp = fingerprint?.exit_ip ?? null;
  const isBlocked = fingerprint !== null;

  // Two guards, because answering a gate promotes the next one into the same
  // DOM node rather than closing the dialog. The ref settles one gate exactly
  // once even if two clicks land in the same React batch; the cooldown stops
  // the second half of a double-click from answering a dialog that appeared
  // between the two clicks and that nobody has read.
  const decidedRef = useRef<number | null>(null);
  const lastDecisionAtRef = useRef(Number.NEGATIVE_INFINITY);

  const decide = (proceed: boolean) => {
    if (decided || decidedRef.current === requestId) {
      return;
    }
    const now = performance.now();
    if (now - lastDecisionAtRef.current < DECISION_COOLDOWN_MS) {
      return;
    }
    decidedRef.current = requestId;
    lastDecisionAtRef.current = now;
    patch({ decided: true });
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
    const request = requestId;
    patch({ isMatching: true });
    try {
      await invoke("match_profile_fingerprint_to_exit", {
        profileId,
        exitIp,
      });
      showSuccessToast(t("consistencyWarning.matchSuccess"));
      patch({ isMatching: false });
      // Rewriting the fingerprint takes long enough for the user to dismiss
      // this gate meanwhile. The profile change still stands, but the launch
      // it belonged to is already settled, and deciding now would answer
      // whichever gate took its place.
      if (liveRequestRef.current !== request) {
        return;
      }
      // The fingerprint the block was measured against no longer exists, so
      // this launch is abandoned rather than forced through with a stale
      // consent token; the user relaunches against the corrected profile.
      decide(false);
    } catch (e) {
      showErrorToast(translateBackendError(t, e));
      patch({ isMatching: false });
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
    // Dismissible on purpose: cancelling is the safe outcome, so every way out
    // of this dialog — Escape, the close X, a click outside — resolves the
    // waiting launch as "don't start". A gate that can only be answered by two
    // buttons is one disabled button away from trapping the whole app.
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) {
          decide(false);
        }
      }}
    >
      <DialogContent className="sm:max-w-md">
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

          {vpnExtensions.length > 0 && (
            <div className="space-y-2 rounded-md border border-warning/50 bg-warning/10 p-3">
              <p className="font-medium">
                {t("prelaunchGate.vpnExtensionHeading")}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("prelaunchGate.vpnExtensionIntro")}
              </p>
              <ul className="space-y-1">
                {vpnExtensions.map((ext) => (
                  <ExtensionEntry key={ext.key} extension={ext} />
                ))}
              </ul>
              <p className="text-xs text-muted-foreground">
                {t("prelaunchGate.vpnExtensionExplainer")}
              </p>
            </div>
          )}

          {proxyCapableExtensions.length > 0 && (
            <div className="space-y-2 rounded-md border border-border bg-muted/40 p-3">
              <p className="font-medium">
                {t("prelaunchGate.proxyCapableHeading")}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("prelaunchGate.proxyCapableIntro")}
              </p>
              <ul className="space-y-1">
                {proxyCapableExtensions.map((ext) => (
                  <ExtensionEntry key={ext.key} extension={ext} />
                ))}
              </ul>
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
                  onCheckedChange={(v) => patch({ ackFingerprint: v === true })}
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
                  onCheckedChange={(v) => patch({ ackExtensions: v === true })}
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
                  onCheckedChange={(v) =>
                    patch({ applyToRemaining: v === true })
                  }
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
              not starting it is the safe outcome. Never disabled by `decided`
              — `decide` is already idempotent, and the one control that ends
              the dialog safely must not be something a stale flag can switch
              off. */}
          <RippleButton
            variant="outline"
            onClick={() => decide(false)}
            disabled={isMatching}
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

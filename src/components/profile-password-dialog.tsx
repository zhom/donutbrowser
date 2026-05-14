"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
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
import {
  extractLockoutSeconds,
  formatLockoutDuration,
  translateBackendError,
} from "@/lib/backend-errors";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";
import { LoadingButton } from "./loading-button";
import { RippleButton } from "./ui/ripple";

export type PasswordDialogMode = "set" | "unlock" | "change" | "remove";

interface ProfilePasswordDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profile: BrowserProfile | null;
  mode: PasswordDialogMode;
  onSuccess?: (profile: BrowserProfile) => void;
}

const MIN_LEN = 8;

export function ProfilePasswordDialog({
  isOpen,
  onClose,
  profile,
  mode,
  onSuccess,
}: ProfilePasswordDialogProps) {
  const { t } = useTranslation();
  const [oldPassword, setOldPassword] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [confirm, setConfirm] = React.useState("");
  const [isSubmitting, setIsSubmitting] = React.useState(false);
  const [lockoutSecondsRemaining, setLockoutSecondsRemaining] = React.useState<
    number | null
  >(null);
  const firstInputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!isOpen) return;
    setOldPassword("");
    setPassword("");
    setConfirm("");
    setIsSubmitting(false);
    setLockoutSecondsRemaining(null);
    const handle = window.setTimeout(() => firstInputRef.current?.focus(), 0);
    return () => {
      window.clearTimeout(handle);
    };
  }, [isOpen]);

  // Tick down the lockout timer
  React.useEffect(() => {
    if (lockoutSecondsRemaining == null) return;
    if (lockoutSecondsRemaining <= 0) {
      setLockoutSecondsRemaining(null);
      return;
    }
    const handle = window.setTimeout(() => {
      setLockoutSecondsRemaining((prev) => (prev == null ? null : prev - 1));
    }, 1000);
    return () => {
      window.clearTimeout(handle);
    };
  }, [lockoutSecondsRemaining]);

  if (!profile) return null;

  const needsConfirm = mode === "set" || mode === "change";
  const needsOldPassword = mode === "change" || mode === "remove";

  const validate = (): string | null => {
    if (needsOldPassword && !oldPassword) {
      return t("profilePassword.errors.oldPasswordRequired");
    }
    if (mode === "set" || mode === "change") {
      if (password.length < MIN_LEN) {
        return t("profilePassword.errors.tooShort", { min: MIN_LEN });
      }
      if (password !== confirm) {
        return t("profilePassword.errors.mismatch");
      }
    }
    if (mode === "unlock" && !password) {
      return t("profilePassword.errors.passwordRequired");
    }
    if (mode === "remove" && !oldPassword) {
      return t("profilePassword.errors.passwordRequired");
    }
    return null;
  };

  const handleSubmit = async () => {
    if (isSubmitting || lockoutSecondsRemaining != null) return;
    const error = validate();
    if (error) {
      showErrorToast(error);
      return;
    }
    setIsSubmitting(true);
    try {
      switch (mode) {
        case "set":
          await invoke("set_profile_password", {
            profileId: profile.id,
            password,
          });
          showSuccessToast(t("profilePassword.toasts.set"));
          break;
        case "unlock":
          await invoke("unlock_profile", {
            profileId: profile.id,
            password,
          });
          break;
        case "change":
          await invoke("change_profile_password", {
            profileId: profile.id,
            oldPassword,
            newPassword: password,
          });
          showSuccessToast(t("profilePassword.toasts.changed"));
          break;
        case "remove":
          await invoke("remove_profile_password", {
            profileId: profile.id,
            password: oldPassword,
          });
          showSuccessToast(t("profilePassword.toasts.removed"));
          break;
      }
      onSuccess?.(profile);
      onClose();
    } catch (err: unknown) {
      const lockoutSeconds = extractLockoutSeconds(err);
      if (lockoutSeconds != null) {
        setLockoutSecondsRemaining(lockoutSeconds);
      } else {
        showErrorToast(translateBackendError(t, err));
      }
    } finally {
      setIsSubmitting(false);
    }
  };

  const titleKey =
    mode === "set"
      ? "profilePassword.set.title"
      : mode === "unlock"
        ? "profilePassword.unlock.title"
        : mode === "change"
          ? "profilePassword.change.title"
          : "profilePassword.remove.title";

  const descriptionKey =
    mode === "set"
      ? "profilePassword.set.description"
      : mode === "unlock"
        ? "profilePassword.unlock.description"
        : mode === "change"
          ? "profilePassword.change.description"
          : "profilePassword.remove.description";

  const submitLabelKey =
    mode === "set"
      ? "profilePassword.set.button"
      : mode === "unlock"
        ? "profilePassword.unlock.button"
        : mode === "change"
          ? "profilePassword.change.button"
          : "profilePassword.remove.button";

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t(titleKey)}</DialogTitle>
          <DialogDescription>
            {t(descriptionKey, { name: profile.name })}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3">
          {(mode === "set" || mode === "change") && (
            <div className="rounded-md border border-warning/50 bg-warning/10 p-3 text-sm">
              <p className="font-medium text-warning-foreground">
                {t("profilePassword.warnings.forgetWarningTitle")}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("profilePassword.warnings.forgetWarningBody")}
              </p>
            </div>
          )}
          {lockoutSecondsRemaining != null && (
            <div className="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
              {t("backendErrors.lockedOut", {
                duration: formatLockoutDuration(t, lockoutSecondsRemaining),
              })}
            </div>
          )}
          {needsOldPassword && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="profile-pw-old">
                {mode === "remove"
                  ? t("profilePassword.fields.password")
                  : t("profilePassword.fields.currentPassword")}
              </Label>
              <Input
                ref={firstInputRef}
                id="profile-pw-old"
                type="password"
                value={oldPassword}
                onChange={(e) => setOldPassword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void handleSubmit();
                }}
                disabled={isSubmitting}
                autoComplete="current-password"
              />
            </div>
          )}
          {(mode === "set" || mode === "change" || mode === "unlock") && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="profile-pw-new">
                {mode === "unlock"
                  ? t("profilePassword.fields.password")
                  : t("profilePassword.fields.newPassword")}
              </Label>
              <Input
                ref={!needsOldPassword ? firstInputRef : undefined}
                id="profile-pw-new"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void handleSubmit();
                }}
                disabled={isSubmitting}
                autoComplete={
                  mode === "unlock" ? "current-password" : "new-password"
                }
              />
            </div>
          )}
          {needsConfirm && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="profile-pw-confirm">
                {t("profilePassword.fields.confirm")}
              </Label>
              <Input
                id="profile-pw-confirm"
                type="password"
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void handleSubmit();
                }}
                disabled={isSubmitting}
                autoComplete="new-password"
              />
            </div>
          )}
        </div>
        <DialogFooter>
          <RippleButton
            variant="outline"
            onClick={onClose}
            disabled={isSubmitting}
          >
            {t("common.buttons.cancel")}
          </RippleButton>
          <LoadingButton
            onClick={() => void handleSubmit()}
            isLoading={isSubmitting}
            disabled={lockoutSecondsRemaining != null}
            variant={mode === "remove" ? "destructive" : "default"}
          >
            {t(submitLabelKey)}
          </LoadingButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

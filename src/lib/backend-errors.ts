import type { TFunction } from "i18next";

/**
 * Backend error codes returned from Rust Tauri commands.
 * Keep this list in sync with the codes used in `src-tauri/src/profile/password.rs`.
 */
export type BackendErrorCode =
  | "INCORRECT_PASSWORD"
  | "LOCKED_OUT"
  | "PROFILE_NOT_FOUND"
  | "PROFILE_NOT_PROTECTED"
  | "PROFILE_ALREADY_PROTECTED"
  | "PROFILE_RUNNING"
  | "PROFILE_EPHEMERAL"
  | "PROFILE_MISSING_SALT"
  | "PROFILE_LOCKED"
  | "INVALID_PROFILE_ID"
  | "PASSWORD_TOO_SHORT"
  | "INVALID_LAUNCH_HOOK_URL"
  | "COOKIE_DB_LOCKED"
  | "COOKIE_DB_UNAVAILABLE"
  | "SELF_HOSTED_REQUIRES_LOGOUT"
  | "PROXY_NOT_FOUND"
  | "GROUP_NOT_FOUND"
  | "VPN_NOT_FOUND"
  | "EXTENSION_NOT_FOUND"
  | "EXTENSION_GROUP_NOT_FOUND"
  | "CANNOT_MODIFY_CLOUD_MANAGED_PROXY"
  | "SYNC_LOCKED_BY_PROFILE"
  | "SYNC_NOT_CONFIGURED"
  | "FINGERPRINT_REQUIRES_PRO"
  | "PROXY_NOT_WORKING"
  | "PROXY_PAYMENT_REQUIRED"
  | "VPN_NOT_WORKING"
  | "INTERNAL_ERROR";

export interface BackendError {
  code: BackendErrorCode;
  params?: Record<string, string>;
}

/**
 * Try to parse a backend error string as a structured `{code, params}` payload.
 * Returns null if the string isn't structured (e.g. raw error from a command
 * that doesn't yet emit codes — caller should fall back to showing the raw text).
 */
export function parseBackendError(err: unknown): BackendError | null {
  const message = err instanceof Error ? err.message : String(err);
  if (!message.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(message);
    if (
      parsed &&
      typeof parsed === "object" &&
      typeof parsed.code === "string"
    ) {
      return parsed as BackendError;
    }
  } catch {
    // not JSON
  }
  return null;
}

/**
 * Translate a backend error to a localized string. Falls back to the raw
 * message if the error isn't a structured backend error.
 */
export function translateBackendError(t: TFunction, err: unknown): string {
  const parsed = parseBackendError(err);
  if (!parsed) {
    return err instanceof Error ? err.message : String(err);
  }
  switch (parsed.code) {
    case "INCORRECT_PASSWORD":
      return t("backendErrors.incorrectPassword");
    case "LOCKED_OUT": {
      const seconds = Number.parseInt(parsed.params?.seconds ?? "0", 10);
      return t("backendErrors.lockedOut", {
        duration: formatLockoutDuration(t, seconds),
      });
    }
    case "PROFILE_NOT_FOUND":
      return t("backendErrors.profileNotFound");
    case "PROFILE_NOT_PROTECTED":
      return t("backendErrors.profileNotProtected");
    case "PROFILE_ALREADY_PROTECTED":
      return t("backendErrors.profileAlreadyProtected");
    case "PROFILE_RUNNING":
      return t("backendErrors.profileRunning");
    case "PROFILE_EPHEMERAL":
      return t("backendErrors.profileEphemeral");
    case "PROFILE_MISSING_SALT":
      return t("backendErrors.profileMissingSalt");
    case "PROFILE_LOCKED":
      return t("backendErrors.profileLocked");
    case "INVALID_PROFILE_ID":
      return t("backendErrors.invalidProfileId");
    case "PASSWORD_TOO_SHORT": {
      const min = Number.parseInt(parsed.params?.min ?? "8", 10);
      return t("backendErrors.passwordTooShort", { min });
    }
    case "INVALID_LAUNCH_HOOK_URL":
      return t("backendErrors.invalidLaunchHookUrl");
    case "COOKIE_DB_LOCKED":
      return t("backendErrors.cookieDbLocked");
    case "COOKIE_DB_UNAVAILABLE":
      return t("backendErrors.cookieDbUnavailable");
    case "SELF_HOSTED_REQUIRES_LOGOUT":
      return t("backendErrors.selfHostedRequiresLogout");
    case "PROXY_NOT_FOUND":
      return t("backendErrors.proxyNotFound");
    case "GROUP_NOT_FOUND":
      return t("backendErrors.groupNotFound");
    case "VPN_NOT_FOUND":
      return t("backendErrors.vpnNotFound");
    case "EXTENSION_NOT_FOUND":
      return t("backendErrors.extensionNotFound");
    case "EXTENSION_GROUP_NOT_FOUND":
      return t("backendErrors.extensionGroupNotFound");
    case "CANNOT_MODIFY_CLOUD_MANAGED_PROXY":
      return t("backendErrors.cannotModifyCloudManagedProxy");
    case "SYNC_LOCKED_BY_PROFILE":
      return t("backendErrors.syncLockedByProfile");
    case "SYNC_NOT_CONFIGURED":
      return t("backendErrors.syncNotConfigured");
    case "FINGERPRINT_REQUIRES_PRO":
      return t("backendErrors.fingerprintRequiresPro");
    case "PROXY_NOT_WORKING":
      return t("backendErrors.proxyNotWorking");
    case "PROXY_PAYMENT_REQUIRED":
      return t("backendErrors.proxyPaymentRequired");
    case "VPN_NOT_WORKING":
      return t("backendErrors.vpnNotWorking");
    case "INTERNAL_ERROR":
      return t("backendErrors.internal", {
        detail: parsed.params?.detail ?? "",
      });
    default:
      return err instanceof Error ? err.message : String(err);
  }
}

export function formatLockoutDuration(t: TFunction, seconds: number): string {
  if (seconds < 60)
    return t("backendErrors.lockedOutDuration.seconds", { seconds });
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60)
    return t("backendErrors.lockedOutDuration.minutes", { minutes });
  const hours = Math.ceil(minutes / 60);
  return t("backendErrors.lockedOutDuration.hours", { hours });
}

/**
 * Extract the lockout countdown in seconds from a backend error, or null.
 */
export function extractLockoutSeconds(err: unknown): number | null {
  const parsed = parseBackendError(err);
  if (parsed?.code !== "LOCKED_OUT") return null;
  const secs = Number.parseInt(parsed.params?.seconds ?? "0", 10);
  return Number.isFinite(secs) && secs > 0 ? secs : null;
}

/**
 * True if the error is a known structured backend error code.
 */
export function isBackendErrorCode(
  err: unknown,
  code: BackendErrorCode,
): boolean {
  return parseBackendError(err)?.code === code;
}

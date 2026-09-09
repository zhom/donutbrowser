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
  | "PROFILE_PAIRED_TWICE"
  | "URL_SCHEME_NOT_ALLOWED"
  | "TOOL_IS_LOCAL_ONLY"
  | "TYPING_TOO_LONG"
  | "PROFILE_EPHEMERAL"
  | "PROFILE_MISSING_SALT"
  | "PROFILE_LOCKED"
  | "INVALID_PROFILE_ID"
  | "PASSWORD_TOO_SHORT"
  | "INVALID_LAUNCH_HOOK_URL"
  | "COOKIE_DB_LOCKED"
  | "COOKIE_DB_UNAVAILABLE"
  | "COOKIE_IMPORT_BROWSER_RUNNING"
  | "COOKIE_IMPORT_PROFILE_PROTECTED"
  | "COOKIE_IMPORT_REMOTE_SESSION"
  | "COOKIE_IMPORT_NO_COOKIES"
  | "SELF_HOSTED_REQUIRES_LOGOUT"
  | "PROXY_NOT_FOUND"
  | "GROUP_NOT_FOUND"
  | "GROUP_ALREADY_EXISTS"
  | "NAME_CANNOT_BE_EMPTY"
  | "WAYFERN_VERSION_NOT_AVAILABLE"
  | "VPN_NOT_FOUND"
  | "EXTENSION_NOT_FOUND"
  | "EXTENSION_GROUP_NOT_FOUND"
  | "EXTENSION_UNSUPPORTED_FILE_TYPE"
  | "EXTENSION_DIR_NOT_FOUND"
  | "EXTENSION_NOT_A_DIRECTORY"
  | "EXTENSION_PATH_INVALID"
  | "EXTENSION_MANIFEST_MISSING"
  | "EXTENSION_MANIFEST_INVALID"
  | "EXTENSION_DIR_TOO_LARGE"
  | "EXTENSION_PATH_HAS_COMMA"
  | "EXTENSION_LINK_REQUIRES_DIRECTORY"
  | "EXTENSION_LINKED_CANNOT_SYNC"
  | "EXTENSION_URL_INVALID"
  | "EXTENSION_DOWNLOAD_FAILED"
  | "EXTENSION_NOT_AN_EXTENSION"
  | "EXTENSION_TOO_LARGE"
  | "CANNOT_MODIFY_CLOUD_MANAGED_PROXY"
  | "SYNC_LOCKED_BY_PROFILE"
  | "SYNC_NOT_CONFIGURED"
  | "FINGERPRINT_REQUIRES_PRO"
  | "PROXY_NOT_WORKING"
  | "PROXY_PAYMENT_REQUIRED"
  | "PROXY_TLS_HANDSHAKE_FAILED"
  | "VPN_NOT_WORKING"
  | "CAMOUFOX_IMPORT_DEPRECATED"
  | "PROXY_SIDECAR_VERSION_MISMATCH"
  | "UPDATE_CHECKSUMS_UNAVAILABLE"
  | "UPDATE_CHECKSUM_MISMATCH"
  | "BROWSER_CHECKSUM_UNAVAILABLE"
  | "BROWSER_CHECKSUM_MISMATCH"
  | "UPDATE_PROFILES_RUNNING"
  | "UPDATE_PREPARATION_FAILED"
  | "PROFILE_NAME_EXISTS"
  | "IMPORT_SOURCE_NOT_FOUND"
  | "IMPORT_SOURCE_NOT_CHROMIUM"
  | "IMPORT_SOURCE_BROWSER_RUNNING"
  | "IMPORT_NO_ITEMS"
  | "BROWSER_NOT_DOWNLOADED"
  | "ARCHIVE_EXTRACTION_FAILED"
  | "UNSUPPORTED_ARCHIVE_FORMAT"
  | "CLEAR_ON_CLOSE_UNAVAILABLE"
  | "PROXY_AND_VPN_MUTUALLY_EXCLUSIVE"
  | "FINGERPRINT_MATCH_FAILED"
  | "INVALID_DNS_RULES_JSON"
  | "UNSUPPORTED_DNS_RULES_FORMAT"
  | "DNS_RULES_SAVE_FAILED"
  | "DNS_RULES_EXPORT_FAILED"
  | "WAYFERN_TERMS_REQUIRED"
  | "API_PORT_UNAVAILABLE"
  | "MCP_SERVER_ALREADY_RUNNING"
  | "MCP_SERVER_NOT_RUNNING"
  | "MCP_PORT_UNAVAILABLE"
  | "MCP_CONFIGURATION_UNAVAILABLE"
  // Local MCP has been removed in favor of remote MCP.
  | "MCP_LOCAL_REMOVED"
  | "MCP_AGENT_UNKNOWN"
  | "MCP_AGENT_INSTALL_FAILED"
  | "MCP_AGENT_REMOVE_FAILED"
  | "MCP_REMOTE_REQUIRES_SIGN_IN"
  | "MCP_REMOTE_UNAUTHORIZED"
  | "MCP_REMOTE_SLOT_TAKEN"
  | "MCP_REMOTE_NOT_ENTITLED"
  | "MCP_REMOTE_UNREACHABLE"
  | "MCP_REMOTE_KEY_MISSING"
  | "MCP_REMOTE_KEY_LIMIT"
  | "MCP_REMOTE_KEY_UNAVAILABLE"
  | "VLESS_CONFIG_INVALID"
  | "XRAY_UNAVAILABLE"
  | "XRAY_UNSUPPORTED_OS"
  | "XRAY_START_FAILED"
  | "CLOUD_NOT_SIGNED_IN"
  | "CLOUD_UNREACHABLE"
  | "CLOUD_REQUEST_FAILED"
  | "REMOTE_RATE_LIMITED"
  | "REMOTE_NO_CAPACITY"
  // The deployment has no fleet credential at all, which is not the same as a
  // busy fleet — telling the two apart is the whole reason it is coded.
  | "REMOTE_NOT_CONFIGURED"
  // The launch was refused for a reason the user cannot wait out (a bad
  // request, a profile that will not run). `params.detail` carries the
  // server's own words.
  | "REMOTE_LAUNCH_REFUSED"
  // The fleet never answered a launch (timeout or connection failure), even on
  // the retry that de-duplicates on the idempotency key. Nothing was refused;
  // the user can only try again.
  | "REMOTE_FLEET_UNREACHABLE"
  // Already at the account's concurrent-session ceiling. `params.limit`/`live`.
  | "REMOTE_CONCURRENCY_LIMIT"
  // Asked to drive a session that has not reached `live` yet.
  | "REMOTE_SESSION_NOT_DRIVABLE"
  // The profile's OS is not one a leased host serves (e.g. an android
  // fingerprint). `params.platform` names it.
  | "REMOTE_PLATFORM_UNSUPPORTED"
  // Sync is on but no upload has ever completed, so a remote host would pull an
  // empty profile and overwrite the real one. Refused before any lease.
  | "REMOTE_PROFILE_NOT_SYNCED"
  | "REMOTE_NOT_ENTITLED"
  | "REMOTE_INTERACTIVE_NOT_ENTITLED"
  // The profile's exit only resolves on this computer, so a leased host cannot
  // use it. Its own code rather than the Cookie Bot's twin: the two refusals
  // name different features, and a user told their "Cookie Bot" needs a public
  // proxy while they were opening a browser by hand cannot act on that.
  | "REMOTE_REQUIRES_REMOTE_EXIT_NODE"
  // The exit's PROTOCOL is the problem, not its address. A VLESS server is as
  // publicly routable as any other, so the reachability check passes and the
  // sentence above sends the user to fix the one part of their config that was
  // already right. `params.kind` names the protocol.
  | "REMOTE_PROXY_KIND_UNSUPPORTED"
  | "REMOTE_SESSION_REFUSED"
  | "REMOTE_SESSION_NOT_FOUND"
  | "REMOTE_SESSION_CONFLICT"
  | "REMOTE_SYNC_IN_PROGRESS"
  | "REMOTE_HOURS_EXHAUSTED"
  | "PROFILE_RUNNING_REMOTELY"
  | "PROFILE_REMOTE_SYNC_PENDING"
  | "PROFILE_LOCKED_BY_MEMBER"
  | "PROFILE_LOCKED_ELSEWHERE"
  | "PROFILE_LOCK_UNAVAILABLE"
  | "NOT_TEAM_MEMBER"
  | "COOKIE_BOT_NOT_ENTITLED"
  | "COOKIE_BOT_NOT_ENROLLED"
  | "COOKIE_BOT_SCHEDULE_CONFLICT"
  | "COOKIE_BOT_RUN_IN_PROGRESS"
  | "COOKIE_BOT_RUN_NOT_FOUND"
  | "COOKIE_BOT_INVALID_SCHEDULE"
  | "COOKIE_BOT_INVALID_TIMEZONE"
  | "COOKIE_BOT_INVALID_PERIOD"
  | "COOKIE_BOT_SITE_LIMIT"
  | "COOKIE_BOT_REQUIRES_CLOUD_SYNC"
  | "COOKIE_BOT_ENCRYPTED_SYNC_UNSUPPORTED"
  | "COOKIE_BOT_UNKNOWN_PLATFORM"
  | "COOKIE_BOT_UNSUPPORTED_PLATFORM"
  | "COOKIE_BOT_REQUIRES_EXIT_NODE"
  // The profile HAS an exit, but only this machine can reach it (127.0.0.1, a
  // LAN address, a `.local` name). Its own code because the fix is different:
  // "attach a proxy" is unactionable advice for someone whose proxy is plainly
  // attached.
  | "COOKIE_BOT_REQUIRES_REMOTE_EXIT_NODE"
  // ...and its twin for an exit whose PROTOCOL no host can speak. Kept apart
  // from the address refusal because a VLESS server is publicly routable: the
  // user is not being asked for a different address, they are being asked for a
  // different kind of exit. `params.kind` names it.
  | "COOKIE_BOT_PROXY_KIND_UNSUPPORTED"
  // The server's own names for two refusals the schedule, profile-state and
  // run-now routes can return. `COOKIE_BOT_REQUIRES_PROXY` is the
  // server-side twin of the local `COOKIE_BOT_REQUIRES_EXIT_NODE` precondition;
  // without a case here the single most important refusal in the feature
  // rendered as the raw machine identifier.
  | "COOKIE_BOT_REQUIRES_PROXY"
  | "COOKIE_BOT_TOUCH_FINGERPRINT_UNSUPPORTED"
  // Agent runs. The first ten are the server's own refusals; the last two are
  // how a bodyless status on a recipe route is classified client-side, because
  // "that run does not exist" is the wrong sentence to show someone editing a
  // saved goal.
  | "AGENT_NOT_ENTITLED"
  | "AGENT_NOT_CONFIGURED"
  | "AGENT_TARGET_OFFLINE"
  | "AGENT_BUDGET_EXCEEDED"
  | "AGENT_RUN_LIMIT_REACHED"
  | "AGENT_RUN_NOT_FOUND"
  | "AGENT_RUN_NOT_CANCELLABLE"
  | "AGENT_GOAL_INVALID"
  | "AGENT_BUDGET_INVALID"
  | "AGENT_TOKEN_PROFILE_MISMATCH"
  | "AGENT_RECIPE_INVALID"
  | "AGENT_RECIPE_NOT_FOUND"
  | "FINGERPRINT_EXIT_MISMATCH"
  // The launch refuses instead of opening a window on an unmanaged device: a
  // silent fallback would leave the user browsing a random fingerprint while
  // the UI reported success, which is the worst failure an anti-detect product
  // can have. `detail` carries the underlying browser error for support.
  | "WAYFERN_FINGERPRINT_APPLY_FAILED"
  | "WAYFERN_FINGERPRINT_GENERATION_FAILED"
  // Its own code rather than a generation failure: the block applies to the
  // whole account rather than one profile, so "try again" is wrong advice, and
  // a user whose every profile refuses at once has to be told this is one
  // limit and not a broken install.
  | "WAYFERN_GENERATION_LIMIT_REACHED"
  // A cross-OS claim needs an active signed-in session, so an expired or
  // offline one cannot apply it. Distinct from the generic apply failure because
  // signing in again is the fix; `detail` names the claimed OS.
  | "WAYFERN_CROSS_OS_REQUIRES_PLAN"
  | "WAYFERN_IDENTITY_REFUSED"
  | "PROFILE_EXPORT_FAILED"
  | "PROFILE_EXPORT_TOO_LARGE"
  | "PROFILE_IMPORT_FAILED"
  | "PROFILE_IMPORT_TOO_NEW"
  | "PROFILE_IMPORT_UNSAFE_ARCHIVE"
  | "RECORDING_ALREADY_RUNNING"
  | "PROFILE_NOT_RUNNING"
  | "WAYFERN_152_REQUIRED"
  | "RECORDING_FAILED"
  | "LAUNCH_CONSENT_EXPIRED"
  | "VPN_WORKER_START_FAILED"
  | "EXIT_PROBE_FAILED"
  | "CAMOUFOX_REMOVED"
  | "NO_E2E_PASSWORD_SET"
  | "TRASH_ENTRY_NOT_FOUND"
  | "TRASH_RESTORE_CONFLICT"
  // Moving the data directory. Every refusal is its own code because every one
  // of them has a different fix: close the browsers, wait for the sync, pick
  // somewhere else, free some space.
  | "DATA_ROOT_SAME_AS_CURRENT"
  | "DATA_ROOT_DESTINATION_INSIDE_SOURCE"
  | "DATA_ROOT_BROWSER_RUNNING"
  | "DATA_ROOT_SYNC_IN_PROGRESS"
  // `params.required` / `params.available`, both in bytes as strings.
  | "DATA_ROOT_INSUFFICIENT_SPACE"
  | "DATA_ROOT_DESTINATION_NOT_WRITABLE"
  | "DATA_ROOT_DESTINATION_NOT_EMPTY"
  | "DATA_ROOT_MOVE_IN_PROGRESS"
  | "DATA_ROOT_COPY_FAILED"
  // The copy did not match the source, so nothing was deleted. Its own code
  // rather than a copy failure: the user's data is still where it was, and
  // saying so is the whole point of verifying before deleting.
  | "DATA_ROOT_VERIFY_FAILED"
  // Synchronised windows.
  | "SYNC_SESSION_NOT_FOUND"
  | "SYNC_FOLLOWER_NOT_FOUND"
  | "SYNC_SESSION_UNAVAILABLE"
  | "SYNC_DISPLAY_UNAVAILABLE"
  | "SYNC_DISPLAY_TOO_SMALL"
  | "SYNC_ARRANGE_FAILED"
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
/**
 * A leased-host operating system as a person reads it.
 *
 * macOS / Windows / Linux are proper nouns, the same in every locale, so they
 * are not translated — only mapped from the wire value the fleet uses.
 */
function remoteOsLabel(platform: string): string {
  switch (platform) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    default:
      return platform;
  }
}

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
    case "TOOL_IS_LOCAL_ONLY":
      return t("backendErrors.toolIsLocalOnly");
    case "TYPING_TOO_LONG":
      return t("backendErrors.typingTooLong", {
        seconds: parsed.params?.seconds ?? "",
        limit: parsed.params?.limit ?? "",
      });
    case "URL_SCHEME_NOT_ALLOWED":
      return t("backendErrors.urlSchemeNotAllowed");
    case "PROFILE_RUNNING":
      return t("backendErrors.profileRunning");
    case "PROFILE_PAIRED_TWICE":
      return t("backendErrors.profilePairedTwice");
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
    case "COOKIE_IMPORT_BROWSER_RUNNING":
      return t("backendErrors.cookieImportBrowserRunning");
    case "COOKIE_IMPORT_PROFILE_PROTECTED":
      return t("backendErrors.cookieImportProfileProtected");
    case "COOKIE_IMPORT_REMOTE_SESSION":
      return t("backendErrors.cookieImportRemoteSession");
    case "COOKIE_IMPORT_NO_COOKIES":
      return t("backendErrors.cookieImportNoCookies");
    case "SELF_HOSTED_REQUIRES_LOGOUT":
      return t("backendErrors.selfHostedRequiresLogout");
    case "PROXY_NOT_FOUND":
      return t("backendErrors.proxyNotFound");
    case "GROUP_NOT_FOUND":
      return t("backendErrors.groupNotFound");
    case "GROUP_ALREADY_EXISTS":
      return t("backendErrors.groupAlreadyExists");
    case "NAME_CANNOT_BE_EMPTY":
      return t("backendErrors.nameCannotBeEmpty");
    case "WAYFERN_VERSION_NOT_AVAILABLE":
      return t("backendErrors.wayfernVersionNotAvailable", {
        requested: parsed.params?.requested ?? "",
        current: parsed.params?.current ?? "",
      });
    case "VPN_NOT_FOUND":
      return t("backendErrors.vpnNotFound");
    case "EXTENSION_NOT_FOUND":
      return t("backendErrors.extensionNotFound");
    case "EXTENSION_GROUP_NOT_FOUND":
      return t("backendErrors.extensionGroupNotFound");
    case "EXTENSION_UNSUPPORTED_FILE_TYPE":
      return t("backendErrors.extensionUnsupportedFileType");
    case "EXTENSION_DIR_NOT_FOUND":
      return t("backendErrors.extensionDirNotFound");
    case "EXTENSION_NOT_A_DIRECTORY":
      return t("backendErrors.extensionNotADirectory");
    case "EXTENSION_PATH_INVALID":
      return t("backendErrors.extensionPathInvalid");
    case "EXTENSION_MANIFEST_MISSING":
      return t("backendErrors.extensionManifestMissing");
    case "EXTENSION_MANIFEST_INVALID":
      return t("backendErrors.extensionManifestInvalid");
    case "EXTENSION_DIR_TOO_LARGE":
      return t("backendErrors.extensionDirTooLarge");
    case "EXTENSION_PATH_HAS_COMMA":
      return t("backendErrors.extensionPathHasComma");
    case "EXTENSION_LINK_REQUIRES_DIRECTORY":
      return t("backendErrors.extensionLinkRequiresDirectory");
    case "EXTENSION_LINKED_CANNOT_SYNC":
      return t("backendErrors.extensionLinkedCannotSync");
    case "EXTENSION_URL_INVALID":
      return t("backendErrors.extensionUrlInvalid");
    case "EXTENSION_DOWNLOAD_FAILED":
      return t("backendErrors.extensionDownloadFailed");
    case "EXTENSION_NOT_AN_EXTENSION":
      return t("backendErrors.extensionNotAnExtension");
    case "EXTENSION_TOO_LARGE":
      return t("backendErrors.extensionTooLarge");
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
    case "PROXY_TLS_HANDSHAKE_FAILED":
      return t("backendErrors.proxyTlsHandshakeFailed", {
        proxy: parsed.params?.proxy ?? "",
      });
    case "VPN_NOT_WORKING":
      return t("backendErrors.vpnNotWorking");
    case "CAMOUFOX_IMPORT_DEPRECATED":
      return t("backendErrors.camoufoxImportDeprecated");
    case "PROXY_SIDECAR_VERSION_MISMATCH":
      return t("backendErrors.proxySidecarVersionMismatch");
    case "UPDATE_CHECKSUMS_UNAVAILABLE":
      return t("backendErrors.updateChecksumsUnavailable", {
        version: parsed.params?.version ?? "",
      });
    case "UPDATE_CHECKSUM_MISMATCH":
      return t("backendErrors.updateChecksumMismatch", {
        file: parsed.params?.file ?? "",
      });
    case "BROWSER_CHECKSUM_UNAVAILABLE":
      return t("backendErrors.browserChecksumUnavailable", {
        browser: parsed.params?.browser ?? "",
        version: parsed.params?.version ?? "",
      });
    case "BROWSER_CHECKSUM_MISMATCH":
      return t("backendErrors.browserChecksumMismatch", {
        browser: parsed.params?.browser ?? "",
        version: parsed.params?.version ?? "",
      });
    case "UPDATE_PROFILES_RUNNING":
      return t("backendErrors.updateProfilesRunning");
    case "UPDATE_PREPARATION_FAILED":
      return t("backendErrors.updatePreparationFailed");
    case "PROFILE_NAME_EXISTS":
      return t("backendErrors.profileNameExists", {
        name: parsed.params?.name ?? "",
      });
    case "IMPORT_SOURCE_NOT_FOUND":
      return t("backendErrors.importSourceNotFound");
    case "IMPORT_SOURCE_NOT_CHROMIUM":
      return parsed.params?.family
        ? t("backendErrors.importSourceNotChromiumNamed", {
            family: parsed.params.family,
          })
        : t("backendErrors.importSourceNotChromium");
    case "IMPORT_SOURCE_BROWSER_RUNNING":
      return t("backendErrors.importSourceBrowserRunning", {
        browser: parsed.params?.browser ?? "",
      });
    case "IMPORT_NO_ITEMS":
      return t("backendErrors.importNoItems");
    case "BROWSER_NOT_DOWNLOADED":
      return t("backendErrors.browserNotDownloaded", {
        browser: parsed.params?.browser ?? "",
      });
    case "ARCHIVE_EXTRACTION_FAILED":
      return t("backendErrors.archiveExtractionFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "UNSUPPORTED_ARCHIVE_FORMAT":
      return t("backendErrors.unsupportedArchiveFormat");
    case "PROXY_AND_VPN_MUTUALLY_EXCLUSIVE":
      return t("backendErrors.proxyAndVpnMutuallyExclusive");
    case "FINGERPRINT_MATCH_FAILED":
      return t("backendErrors.fingerprintMatchFailed");
    case "INVALID_DNS_RULES_JSON":
      return t("backendErrors.invalidDnsRulesJson");
    case "UNSUPPORTED_DNS_RULES_FORMAT":
      return t("backendErrors.unsupportedDnsRulesFormat", {
        format: parsed.params?.format ?? "",
      });
    case "DNS_RULES_SAVE_FAILED":
      return t("backendErrors.dnsRulesSaveFailed");
    case "DNS_RULES_EXPORT_FAILED":
      return t("backendErrors.dnsRulesExportFailed");
    case "WAYFERN_TERMS_REQUIRED":
      return t("backendErrors.wayfernTermsRequired");
    case "API_PORT_UNAVAILABLE":
      return t("backendErrors.apiPortUnavailable");
    case "MCP_SERVER_ALREADY_RUNNING":
      return t("backendErrors.mcpServerAlreadyRunning");
    case "MCP_SERVER_NOT_RUNNING":
      return t("backendErrors.mcpServerNotRunning");
    case "MCP_PORT_UNAVAILABLE":
      return t("backendErrors.mcpPortUnavailable");
    case "MCP_CONFIGURATION_UNAVAILABLE":
      return t("backendErrors.mcpConfigurationUnavailable");
    case "MCP_REMOTE_REQUIRES_SIGN_IN":
      return t("backendErrors.mcpRemoteRequiresSignIn");
    case "MCP_REMOTE_UNAUTHORIZED":
      return t("backendErrors.mcpRemoteUnauthorized");
    case "MCP_REMOTE_SLOT_TAKEN":
      return t("backendErrors.mcpRemoteSlotTaken");
    case "MCP_REMOTE_NOT_ENTITLED":
      return t("backendErrors.mcpRemoteNotEntitled");
    case "MCP_REMOTE_UNREACHABLE":
      return t("backendErrors.mcpRemoteUnreachable");
    case "MCP_REMOTE_KEY_MISSING":
      return t("backendErrors.mcpRemoteKeyMissing");
    case "MCP_REMOTE_KEY_LIMIT":
      return t("backendErrors.mcpRemoteKeyLimit");
    case "MCP_REMOTE_KEY_UNAVAILABLE":
      return t("backendErrors.mcpRemoteKeyUnavailable");
    case "MCP_LOCAL_REMOVED":
      return t("backendErrors.mcpLocalRemoved");
    case "MCP_AGENT_UNKNOWN":
      return t("backendErrors.mcpAgentUnknown");
    case "MCP_AGENT_INSTALL_FAILED":
      return t("backendErrors.mcpAgentInstallFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "MCP_AGENT_REMOVE_FAILED":
      return t("backendErrors.mcpAgentRemoveFailed", {
        detail: parsed.params?.detail ?? "",
      });
    // Donut supports exactly one VLESS shape (REALITY + XTLS Vision over TCP),
    // so most rejections mean "your server is a kind we do not support", not
    // "you mistyped". Name the unsupported part instead of implying a typo.
    case "VLESS_CONFIG_INVALID": {
      const reason = parsed.params?.reason;
      const known = [
        "security",
        "flow",
        "transport",
        "encryption",
        "headerType",
        "fingerprint",
        "sni",
        "publicKey",
        "scheme",
        "parameter",
        "malformed",
      ];
      if (reason && known.includes(reason)) {
        return t(`backendErrors.vlessUnsupported.${reason}`);
      }
      return t("backendErrors.vlessConfigInvalid");
    }
    case "XRAY_UNAVAILABLE":
      return t("backendErrors.xrayUnavailable");
    case "XRAY_UNSUPPORTED_OS":
      return t("backendErrors.xrayUnsupportedOs");
    case "XRAY_START_FAILED":
      return t("backendErrors.xrayStartFailed");
    case "CLEAR_ON_CLOSE_UNAVAILABLE":
      return t("backendErrors.clearOnCloseUnavailable");
    case "CLOUD_NOT_SIGNED_IN":
      return t("backendErrors.cloudNotSignedIn");
    case "CLOUD_UNREACHABLE":
      return t("backendErrors.cloudUnreachable");
    case "CLOUD_REQUEST_FAILED":
      return t("backendErrors.cloudRequestFailed");
    case "REMOTE_RATE_LIMITED":
      return t("backendErrors.remoteRateLimited");
    case "REMOTE_NO_CAPACITY": {
      // Newer backends name the platform; older ones (and a bare 503 mapped by
      // status alone) do not, so fall back to the platform-free line.
      const platform = parsed.params?.platform;
      return platform
        ? t("backendErrors.remoteNoCapacityPlatform", {
            platform: remoteOsLabel(platform),
          })
        : t("backendErrors.remoteNoCapacity");
    }
    case "REMOTE_NOT_CONFIGURED":
      return t("backendErrors.remoteNotConfigured");
    case "REMOTE_LAUNCH_REFUSED": {
      // The server's own words, when it gave any. Kept as a diagnostic; the
      // generic line stands alone when it did not.
      const detail = parsed.params?.detail?.trim();
      return detail
        ? t("backendErrors.remoteLaunchRefusedDetail", { detail })
        : t("backendErrors.remoteLaunchRefused");
    }
    case "REMOTE_FLEET_UNREACHABLE":
      return t("backendErrors.remoteFleetUnreachable");
    case "REMOTE_CONCURRENCY_LIMIT":
      return t("backendErrors.remoteConcurrencyLimit", {
        live: parsed.params?.live ?? "",
        limit: parsed.params?.limit ?? "",
      });
    case "REMOTE_SESSION_NOT_DRIVABLE":
      return t("backendErrors.remoteSessionNotDrivable");
    case "REMOTE_PLATFORM_UNSUPPORTED":
      return t("backendErrors.remotePlatformUnsupported", {
        platform: remoteOsLabel(parsed.params?.platform ?? ""),
      });
    case "REMOTE_PROFILE_NOT_SYNCED":
      return t("backendErrors.remoteProfileNotSynced");
    case "REMOTE_NOT_ENTITLED":
      return t("backendErrors.remoteNotEntitled");
    // Distinct from the above: the plan HAS remote hours, it just may not spend
    // them by hand (solo funds a nightly Cookie Bot only). Telling such a user
    // "your plan does not include remote execution" while their bot visibly
    // runs every night is the confusing case this code exists to avoid.
    case "REMOTE_INTERACTIVE_NOT_ENTITLED":
      return t("backendErrors.remoteInteractiveNotEntitled");
    case "REMOTE_REQUIRES_REMOTE_EXIT_NODE":
      return t("backendErrors.remoteRequiresRemoteExitNode");
    case "REMOTE_PROXY_KIND_UNSUPPORTED":
      return t("backendErrors.remoteProxyKindUnsupported", {
        kind: parsed.params?.kind ?? "",
      });
    case "REMOTE_SESSION_REFUSED":
      return t("backendErrors.remoteSessionRefused");
    case "REMOTE_SESSION_NOT_FOUND":
      return t("backendErrors.remoteSessionNotFound");
    case "REMOTE_SESSION_CONFLICT":
      return t("backendErrors.remoteSessionConflict");
    case "REMOTE_SYNC_IN_PROGRESS":
      return t("backendErrors.remoteSyncInProgress");
    case "REMOTE_HOURS_EXHAUSTED":
      return t("backendErrors.remoteHoursExhausted", {
        granted: parsed.params?.granted ?? "0",
        used: parsed.params?.used ?? "0",
      });
    case "PROFILE_RUNNING_REMOTELY":
      return t("backendErrors.profileRunningRemotely");
    case "PROFILE_REMOTE_SYNC_PENDING":
      return t("backendErrors.profileRemoteSyncPending");
    case "PROFILE_LOCKED_BY_MEMBER":
      return t("backendErrors.profileLockedByMember", {
        email: parsed.params?.email ?? "",
      });
    case "PROFILE_LOCKED_ELSEWHERE":
      return t("backendErrors.profileLockedElsewhere");
    case "PROFILE_LOCK_UNAVAILABLE":
      return t("backendErrors.profileLockUnavailable");
    case "NOT_TEAM_MEMBER":
      return t("backendErrors.notTeamMember");
    case "COOKIE_BOT_NOT_ENTITLED":
      return t("backendErrors.cookieBotNotEntitled");
    case "COOKIE_BOT_NOT_ENROLLED":
      return t("backendErrors.cookieBotNotEnrolled");
    case "COOKIE_BOT_SCHEDULE_CONFLICT":
      return t("backendErrors.cookieBotScheduleConflict", {
        email: parsed.params?.email ?? "",
        time: parsed.params?.time ?? "",
      });
    case "COOKIE_BOT_RUN_IN_PROGRESS":
      return t("backendErrors.cookieBotRunInProgress");
    case "COOKIE_BOT_RUN_NOT_FOUND":
      return t("backendErrors.cookieBotRunNotFound");
    case "COOKIE_BOT_INVALID_SCHEDULE":
      return t("backendErrors.cookieBotInvalidSchedule");
    case "COOKIE_BOT_INVALID_TIMEZONE":
      return t("backendErrors.cookieBotInvalidTimezone", {
        timezone: parsed.params?.timezone ?? "",
      });
    case "COOKIE_BOT_INVALID_PERIOD":
      return t("backendErrors.cookieBotInvalidPeriod");
    case "COOKIE_BOT_SITE_LIMIT":
      // The server sends both bounds. Defaulting `min` to 1 was not the
      // problem — the message never mentioned a minimum at all, so a user who
      // submitted no sites was told about a maximum they had not reached.
      return t("backendErrors.cookieBotSiteLimit", {
        min: parsed.params?.min ?? "1",
        max: parsed.params?.max ?? "40",
      });
    case "COOKIE_BOT_REQUIRES_CLOUD_SYNC":
      return t("backendErrors.cookieBotRequiresCloudSync");
    case "COOKIE_BOT_ENCRYPTED_SYNC_UNSUPPORTED":
      return t("backendErrors.cookieBotEncryptedSyncUnsupported");
    case "COOKIE_BOT_UNKNOWN_PLATFORM":
      return t("backendErrors.cookieBotUnknownPlatform");
    case "COOKIE_BOT_UNSUPPORTED_PLATFORM":
      return t("backendErrors.cookieBotUnsupportedPlatform", {
        platform: parsed.params?.platform ?? "",
      });
    case "COOKIE_BOT_REQUIRES_EXIT_NODE":
    // One condition, two names: the desktop refuses it locally as
    // REQUIRES_EXIT_NODE and the server refuses it as REQUIRES_PROXY. Both
    // resolve to the one sentence a user can act on.
    case "COOKIE_BOT_REQUIRES_PROXY":
      return t("backendErrors.cookieBotRequiresExitNode");
    case "COOKIE_BOT_REQUIRES_REMOTE_EXIT_NODE":
      return t("backendErrors.cookieBotRequiresRemoteExitNode");
    case "COOKIE_BOT_PROXY_KIND_UNSUPPORTED":
      return t("backendErrors.cookieBotProxyKindUnsupported", {
        kind: parsed.params?.kind ?? "",
      });
    case "COOKIE_BOT_TOUCH_FINGERPRINT_UNSUPPORTED":
      return t("backendErrors.cookieBotTouchFingerprintUnsupported");
    case "AGENT_NOT_ENTITLED":
      return t("backendErrors.agentNotEntitled");
    case "AGENT_NOT_CONFIGURED":
      return t("backendErrors.agentNotConfigured");
    case "AGENT_TARGET_OFFLINE":
      return t("backendErrors.agentTargetOffline");
    case "AGENT_BUDGET_EXCEEDED":
      return t("backendErrors.agentBudgetExceeded");
    case "AGENT_RUN_LIMIT_REACHED":
      return t("backendErrors.agentRunLimitReached");
    case "AGENT_RUN_NOT_FOUND":
      return t("backendErrors.agentRunNotFound");
    case "AGENT_RUN_NOT_CANCELLABLE":
      return t("backendErrors.agentRunNotCancellable");
    case "AGENT_GOAL_INVALID":
      return t("backendErrors.agentGoalInvalid");
    case "AGENT_BUDGET_INVALID":
      return t("backendErrors.agentBudgetInvalid");
    // The run was started against a profile the issued token does not cover.
    // Its own code because signing in again is the fix, not editing the goal.
    case "AGENT_TOKEN_PROFILE_MISMATCH":
      return t("backendErrors.agentTokenProfileMismatch");
    case "AGENT_RECIPE_INVALID":
      return t("backendErrors.agentRecipeInvalid");
    case "AGENT_RECIPE_NOT_FOUND":
      return t("backendErrors.agentRecipeNotFound");
    // The launch gate's block. The dialog renders the mismatch detail from
    // `params` itself; this string is the fallback for anywhere that only has
    // room for one sentence.
    case "FINGERPRINT_EXIT_MISMATCH":
      return t("backendErrors.fingerprintExitMismatch");
    case "WAYFERN_FINGERPRINT_APPLY_FAILED":
      return t("backendErrors.wayfernFingerprintApplyFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "WAYFERN_FINGERPRINT_GENERATION_FAILED":
      return t("backendErrors.wayfernFingerprintGenerationFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "WAYFERN_GENERATION_LIMIT_REACHED":
      return t("backendErrors.wayfernGenerationLimitReached");
    case "WAYFERN_CROSS_OS_REQUIRES_PLAN":
      return t("backendErrors.wayfernCrossOsRequiresPlan", {
        detail: parsed.params?.detail ?? "",
      });
    case "WAYFERN_IDENTITY_REFUSED":
      return t("backendErrors.wayfernIdentityRefused", {
        detail: parsed.params?.detail ?? "",
      });
    case "PROFILE_EXPORT_FAILED":
      return t("backendErrors.profileExportFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "PROFILE_EXPORT_TOO_LARGE":
      return t("backendErrors.profileExportTooLarge");
    case "PROFILE_IMPORT_FAILED":
      return t("backendErrors.profileImportFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "PROFILE_IMPORT_TOO_NEW":
      return t("backendErrors.profileImportTooNew");
    case "PROFILE_IMPORT_UNSAFE_ARCHIVE":
      return t("backendErrors.profileImportUnsafeArchive");
    case "RECORDING_ALREADY_RUNNING":
      return t("backendErrors.recordingAlreadyRunning");
    case "PROFILE_NOT_RUNNING":
      return t("backendErrors.profileNotRunning");
    case "WAYFERN_152_REQUIRED":
      return t("backendErrors.wayfern152Required");
    case "RECORDING_FAILED":
      return t("backendErrors.recordingFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "LAUNCH_CONSENT_EXPIRED":
      return t("backendErrors.launchConsentExpired");
    case "VPN_WORKER_START_FAILED":
      return t("backendErrors.vpnWorkerStartFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "EXIT_PROBE_FAILED":
      return t("backendErrors.exitProbeFailed");
    case "CAMOUFOX_REMOVED":
      return t("backendErrors.camoufoxRemoved");
    case "NO_E2E_PASSWORD_SET":
      return t("backendErrors.noE2ePasswordSet");
    case "TRASH_ENTRY_NOT_FOUND":
      return t("backendErrors.trashEntryNotFound");
    case "TRASH_RESTORE_CONFLICT":
      return t("backendErrors.trashRestoreConflict");
    case "DATA_ROOT_SAME_AS_CURRENT":
      return t("backendErrors.dataRootSameAsCurrent");
    case "DATA_ROOT_DESTINATION_INSIDE_SOURCE":
      return t("backendErrors.dataRootDestinationInsideSource");
    case "DATA_ROOT_BROWSER_RUNNING":
      return t("backendErrors.dataRootBrowserRunning");
    case "DATA_ROOT_SYNC_IN_PROGRESS":
      return t("backendErrors.dataRootSyncInProgress");
    // The figures arrive already written out. This module is imported by a
    // plain `node --test` run, so it stays free of project imports, and the
    // one place that knows the byte counts formats them.
    case "DATA_ROOT_INSUFFICIENT_SPACE":
      return t("backendErrors.dataRootInsufficientSpace", {
        required: parsed.params?.required ?? "",
        available: parsed.params?.available ?? "",
      });
    case "DATA_ROOT_DESTINATION_NOT_WRITABLE":
      return t("backendErrors.dataRootDestinationNotWritable");
    case "DATA_ROOT_DESTINATION_NOT_EMPTY":
      return t("backendErrors.dataRootDestinationNotEmpty");
    case "DATA_ROOT_MOVE_IN_PROGRESS":
      return t("backendErrors.dataRootMoveInProgress");
    case "DATA_ROOT_COPY_FAILED":
      return t("backendErrors.dataRootCopyFailed", {
        detail: parsed.params?.detail ?? "",
      });
    case "DATA_ROOT_VERIFY_FAILED":
      return t("backendErrors.dataRootVerifyFailed");
    case "SYNC_SESSION_NOT_FOUND":
      return t("backendErrors.syncSessionNotFound");
    case "SYNC_FOLLOWER_NOT_FOUND":
      return t("backendErrors.syncFollowerNotFound");
    case "SYNC_SESSION_UNAVAILABLE":
      return t("backendErrors.syncSessionUnavailable");
    case "SYNC_DISPLAY_UNAVAILABLE":
      return t("backendErrors.syncDisplayUnavailable");
    case "SYNC_DISPLAY_TOO_SMALL":
      return t("backendErrors.syncDisplayTooSmall", {
        windows: Number(parsed.params?.count ?? 0),
      });
    case "SYNC_ARRANGE_FAILED":
      return t("backendErrors.syncArrangeFailed");
    case "INTERNAL_ERROR":
      return t("backendErrors.internal", {
        detail: parsed.params?.detail ?? "",
      });
    default:
      // The payload parsed as a structured error but carries a code this build
      // does not know: the server can add codes faster than the desktop ships.
      // Returning the raw message here would render the literal JSON to the
      // user, so show a translated line that still names the code for support.
      return t("backendErrors.unknownCode", { code: String(parsed.code) });
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

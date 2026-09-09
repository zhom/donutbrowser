/**
 * Which app operation each client method wraps.
 *
 * This table is the SDK's half of a two-sided check. `sdk/api-paths.json` holds
 * every operation the desktop app publishes, generated from
 * `src-tauri/src/api_server.rs`. The test suite asserts the two agree exactly
 * in both directions, so:
 *
 * - an endpoint added to the app fails the SDK tests until it is wrapped here,
 *   or listed in `OMITTED` with a reason, and
 * - an entry here that the app no longer publishes fails too.
 *
 * The same table is mirrored in the Python package, and the same snapshot
 * proves it.
 */

/** `"<VERB> <path template>"`, exactly as the app publishes it. */
export type OperationKey = string;

/** Operation to the name of the `DonutClient` method that calls it. */
export const OPERATIONS: ReadonlyMap<OperationKey, string> = new Map([
  ["POST /v1/browsers/download", "downloadBrowser"],
  ["GET /v1/browsers/{browser}/versions", "listBrowserVersions"],
  ["GET /v1/browsers/{browser}/versions/{version}/downloaded", "isBrowserDownloaded"],
  ["GET /v1/cookie-bot/conflicts", "getCookieBotConflicts"],
  ["GET /v1/cookie-bot/presets", "listCookieBotPresets"],
  ["GET /v1/cookie-bot/runs", "listCookieBotRuns"],
  ["POST /v1/cookie-bot/runs", "startCookieBotRun"],
  ["DELETE /v1/cookie-bot/runs/{run_id}", "cancelCookieBotRun"],
  ["GET /v1/cookie-bot/schedules", "listCookieBotSchedules"],
  ["DELETE /v1/cookie-bot/schedules/{profile_id}", "deleteCookieBotSchedule"],
  ["GET /v1/cookie-bot/schedules/{profile_id}", "getCookieBotSchedule"],
  ["PUT /v1/cookie-bot/schedules/{profile_id}", "setCookieBotSchedule"],
  ["GET /v1/cookie-bot/usage", "getCookieBotUsage"],
  ["GET /v1/extension-groups", "listExtensionGroups"],
  ["POST /v1/extension-groups", "createExtensionGroup"],
  ["DELETE /v1/extension-groups/{id}", "deleteExtensionGroup"],
  ["GET /v1/extension-groups/{id}", "getExtensionGroup"],
  ["PUT /v1/extension-groups/{id}", "updateExtensionGroup"],
  ["DELETE /v1/extension-groups/{id}/extensions/{extension_id}", "removeExtensionFromGroup"],
  ["POST /v1/extension-groups/{id}/extensions/{extension_id}", "addExtensionToGroup"],
  ["GET /v1/extensions", "listExtensions"],
  ["POST /v1/extensions", "createExtension"],
  ["DELETE /v1/extensions/{id}", "deleteExtension"],
  ["GET /v1/extensions/{id}", "getExtension"],
  ["PUT /v1/extensions/{id}", "updateExtension"],
  ["GET /v1/groups", "listGroups"],
  ["POST /v1/groups", "createGroup"],
  ["DELETE /v1/groups/{id}", "deleteGroup"],
  ["GET /v1/groups/{id}", "getGroup"],
  ["PUT /v1/groups/{id}", "updateGroup"],
  ["GET /v1/profiles", "listProfiles"],
  ["POST /v1/profiles", "createProfile"],
  ["POST /v1/profiles/batch/run", "batchRunProfiles"],
  ["POST /v1/profiles/batch/stop", "batchStopProfiles"],
  ["POST /v1/profiles/distribute-proxies", "distributeProxies"],
  ["POST /v1/profiles/import", "importProfiles"],
  ["GET /v1/profiles/import/detect", "detectImportProfiles"],
  ["DELETE /v1/profiles/{id}", "deleteProfile"],
  ["GET /v1/profiles/{id}", "getProfile"],
  ["PUT /v1/profiles/{id}", "updateProfile"],
  ["POST /v1/profiles/{id}/agent/click", "agentClick"],
  ["POST /v1/profiles/{id}/agent/extract", "agentExtract"],
  ["POST /v1/profiles/{id}/agent/perceive", "agentPerceive"],
  ["POST /v1/profiles/{id}/agent/pick", "agentPick"],
  ["POST /v1/profiles/{id}/agent/resolve-locator", "agentResolveLocator"],
  ["POST /v1/profiles/{id}/agent/type", "agentType"],
  ["POST /v1/profiles/{id}/cloud-sync", "setProfileCloudSync"],
  ["POST /v1/profiles/{id}/cookies/import", "importProfileCookies"],
  ["POST /v1/profiles/{id}/kill", "killProfile"],
  ["POST /v1/profiles/{id}/open-url", "openUrl"],
  ["POST /v1/profiles/{id}/run", "runProfile"],
  ["POST /v1/profiles/{id}/run-remote", "runProfileRemote"],
  ["GET /v1/proxies", "listProxies"],
  ["POST /v1/proxies", "createProxy"],
  ["POST /v1/proxies/import", "importProxies"],
  ["DELETE /v1/proxies/{id}", "deleteProxy"],
  ["GET /v1/proxies/{id}", "getProxy"],
  ["PUT /v1/proxies/{id}", "updateProxy"],
  ["GET /v1/remote-hours", "getRemoteHours"],
  ["GET /v1/remote-sessions", "listRemoteSessions"],
  ["DELETE /v1/remote-sessions/{id}", "stopRemoteSession"],
  ["GET /v1/remote-sessions/{id}", "getRemoteSession"],
  ["GET /v1/tags", "listTags"],
  ["GET /v1/vpns", "listVpns"],
  ["POST /v1/vpns", "createVpn"],
  ["POST /v1/vpns/import", "importVpn"],
  ["DELETE /v1/vpns/{id}", "deleteVpn"],
  ["GET /v1/vpns/{id}", "getVpn"],
  ["PUT /v1/vpns/{id}", "updateVpn"],
  ["GET /v1/vpns/{id}/export", "exportVpn"],
]);

/** Operations this SDK deliberately does not call, and why. */
export const OMITTED: ReadonlyMap<OperationKey, string> = new Map([
  [
    "GET /v1/remote-sessions/{id}/cdp",
    "A WebSocket upgrade, not a request. fetch() cannot speak it, and bundling a " +
      "websocket implementation would end this package's zero-dependency promise for " +
      "one endpoint. DonutClient.remoteSessionCdpUrl() builds the ws:// address so a " +
      "websocket library of the caller's choosing can connect, sending the same " +
      "Authorization: Bearer header on the handshake.",
  ],
]);

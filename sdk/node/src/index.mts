/**
 * Donut Browser SDK: a thin client for the app's local REST API.
 *
 * The local API is off by default. Switch it on in the app under **Settings,
 * Integrations, Local API, "Enable Local API Server"**, and copy the port and
 * the authentication token from that screen.
 *
 * ```ts
 * import { DonutClient } from "@donutbrowser/sdk";
 *
 * const client = new DonutClient({ token: "..." });
 * await client.withProfile(profileId, { url: "https://example.com" }, async (session) => {
 *   console.log(session.cdpUrl);
 *   await client.agentClick(profileId, { locator: { role: "button", name: "Sign in" } });
 * });
 * ```
 */

export { DEFAULT_HOST, DEFAULT_PORT, DonutClient, RunSession } from "./client.mts";
export type { DonutClientOptions, RunProfileOptions } from "./client.mts";
export { OMITTED, OPERATIONS } from "./coverage.mts";
export type { OperationKey } from "./coverage.mts";
export {
  BadGateway,
  Conflict,
  DonutApiError,
  DonutConnectionError,
  DonutError,
  errorForStatus,
  Forbidden,
  NotFound,
  PaymentRequired,
  RateLimited,
  RequestTimeout,
  ServerError,
  ServiceUnavailable,
  Unauthorized,
  ValidationError,
} from "./errors.mts";
export type { DonutApiErrorInit } from "./errors.mts";
export type * from "./types.mts";

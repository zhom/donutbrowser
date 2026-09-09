/**
 * A thin client for the Donut Browser local REST API.
 *
 * Every method here is one request to one documented path. Nothing is cached,
 * nothing is retried, and nothing is invented: if a method exists below, the
 * app publishes that operation in its `/openapi.json`.
 *
 * No runtime dependencies. See `sdk/README.md`.
 */

import { DonutConnectionError, DonutError, errorForStatus } from "./errors.mts";
import type {
  AgentClick,
  AgentTyping,
  ApiGroupResponse,
  ApiProfileResponse,
  ApiProfilesResponse,
  ApiProxyResponse,
  ApiRemoteSessionsResponse,
  ApiVpnExportResponse,
  ApiVpnResponse,
  BatchRunResponse,
  BatchStopResponse,
  CookieBotConflictCheck,
  CookieBotPresetList,
  CookieBotRun,
  CookieBotRunPage,
  CookieBotRunStarted,
  CookieBotSchedule,
  CookieBotScheduleDeleted,
  CookieBotScheduleList,
  CookieBotScheduleSaved,
  CookieBotUsage,
  DetectedProfilesResponse,
  DistributeProxiesResponse,
  DownloadBrowserResponse,
  Extension,
  ExtensionGroup,
  Extraction,
  ExtractionField,
  ImportCookiesResponse,
  ImportProfileItem,
  ImportProxiesResponse,
  LocatorDescription,
  LocatorResolution,
  PerceptionPage,
  PickedElement,
  ProfileImportBatchResult,
  ProxyPair,
  ProxySettings,
  RemoteHoursQuota,
  RemoteSessionState,
  RunProfileResponse,
  RunRemoteResponse,
  SetCloudSyncResponse,
  StopRemoteResponse,
  WayfernConfig,
} from "./types.mts";

/** The port the app offers by default in Settings, Integrations, Local API. */
export const DEFAULT_PORT = 10108;

/** The API binds loopback only. It is never reachable from another machine. */
export const DEFAULT_HOST = "127.0.0.1";

const JSON_TYPE = "application/json";

export interface DonutClientOptions {
  /** Overrides `host` and `port` entirely. */
  baseUrl?: string;
  /** Falls back to `DONUT_API_TOKEN`. */
  token?: string;
  /** Falls back to `DONUT_API_PORT`, then to `10108`. */
  port?: number;
  host?: string;
  /** Per-request timeout in milliseconds. Defaults to 30000. */
  timeoutMs?: number;
  /** Where to read the fallbacks from. Defaults to `process.env`. */
  env?: Record<string, string | undefined>;
  /** Swappable for tests. Defaults to the global `fetch`. */
  fetch?: typeof fetch;
}

export interface RunProfileOptions {
  url?: string;
  headless?: boolean;
}

/** Drop every property the caller left `undefined`. */
function body(fields: Record<string, unknown>): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  for (const [name, value] of Object.entries(fields)) {
    if (value !== undefined) {
      result[name] = value;
    }
  }
  return result;
}

function query(fields: Record<string, string | number | boolean | undefined>): URLSearchParams {
  const params = new URLSearchParams();
  for (const [name, value] of Object.entries(fields)) {
    if (value !== undefined) {
      params.set(name, String(value));
    }
  }
  return params;
}

/** Escape one path segment so an id with a slash cannot forge a path. */
function segment(value: string): string {
  return encodeURIComponent(value);
}

/**
 * A connection to one running Donut Browser.
 *
 * The local API must be switched on first: **Settings, Integrations, Local
 * API, "Enable Local API Server"**. That screen shows the port and the
 * authentication token to use here.
 */
export class DonutClient {
  readonly baseUrl: string;
  readonly host: string;
  readonly port: number;
  readonly token: string;
  readonly timeoutMs: number;

  #prefix: string;
  #scheme: string;
  #fetch: typeof fetch;

  constructor(options: DonutClientOptions = {}) {
    // Reached without `@types/node`, so the package stays dependency-free even
    // for its own type-check.
    const ambient = globalThis as { process?: { env?: Record<string, string | undefined> } };
    const env = options.env ?? ambient.process?.env ?? {};

    const token = options.token ?? env.DONUT_API_TOKEN;
    if (!token) {
      throw new DonutError(
        "No API token. Pass { token }, or set DONUT_API_TOKEN. The token is shown " +
          "in the app under Settings, Integrations, Local API.",
      );
    }

    if (options.baseUrl) {
      const raw = options.baseUrl.includes("//")
        ? options.baseUrl
        : `http://${options.baseUrl}`;
      let parsed: URL;
      try {
        parsed = new URL(raw);
      } catch {
        throw new DonutError(`baseUrl is not a URL: ${options.baseUrl}`);
      }
      if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
        throw new DonutError(`baseUrl must be http or https, got ${parsed.protocol}`);
      }
      this.#scheme = parsed.protocol.replace(":", "");
      this.host = parsed.hostname;
      this.port = Number(parsed.port || (this.#scheme === "https" ? 443 : 80));
      this.#prefix = parsed.pathname.replace(/\/+$/, "");
    } else {
      let port = options.port;
      if (port === undefined && env.DONUT_API_PORT) {
        port = Number.parseInt(env.DONUT_API_PORT, 10);
        if (!Number.isFinite(port)) {
          throw new DonutError(`DONUT_API_PORT is not a number: ${env.DONUT_API_PORT}`);
        }
      }
      this.#scheme = "http";
      this.host = options.host ?? DEFAULT_HOST;
      this.port = port ?? DEFAULT_PORT;
      this.#prefix = "";
    }

    this.token = token;
    this.timeoutMs = options.timeoutMs ?? 30_000;
    this.baseUrl = `${this.#scheme}://${this.host}:${this.port}${this.#prefix}`;
    this.#fetch = options.fetch ?? globalThis.fetch;
  }

  // ------------------------------------------------------------------
  // Transport
  // ------------------------------------------------------------------

  async #request<T>(
    method: string,
    path: string,
    payload?: unknown,
    search?: URLSearchParams,
  ): Promise<T> {
    const encoded = search === undefined ? "" : search.toString();
    const suffix = encoded === "" ? "" : `?${encoded}`;
    const url = `${this.baseUrl}${path}${suffix}`;

    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.token}`,
      Accept: JSON_TYPE,
    };
    if (payload !== undefined) {
      headers["Content-Type"] = JSON_TYPE;
    }

    let response: Response;
    try {
      response = await this.#fetch(url, {
        method,
        headers,
        body: payload === undefined ? undefined : JSON.stringify(payload),
        signal: AbortSignal.timeout(this.timeoutMs),
      });
    } catch (cause) {
      throw new DonutConnectionError(
        `Could not reach Donut Browser at ${this.baseUrl} (${method} ${path}): ` +
          `${cause instanceof Error ? cause.message : String(cause)}. Is the app ` +
          "running with Settings, Integrations, Local API switched on?",
        { cause },
      );
    }

    const text = await response.text();
    if (!response.ok) {
      throw errorForStatus(response.status, text, {
        method,
        path,
        headers: response.headers,
      });
    }
    if (response.status === 204 || text.trim() === "") {
      return undefined as T;
    }
    try {
      return JSON.parse(text) as T;
    } catch (cause) {
      throw new DonutError(
        `${method} ${path} answered ${response.status} with a body that is not ` +
          `JSON: ${text.slice(0, 200)}`,
        { cause },
      );
    }
  }

  // ------------------------------------------------------------------
  // Profiles
  // ------------------------------------------------------------------

  /** GET /v1/profiles */
  listProfiles(): Promise<ApiProfilesResponse> {
    return this.#request("GET", "/v1/profiles");
  }

  /** GET /v1/profiles/{id} */
  getProfile(profileId: string): Promise<ApiProfileResponse> {
    return this.#request("GET", `/v1/profiles/${segment(profileId)}`);
  }

  /**
   * POST /v1/profiles
   *
   * `browser` must be `"wayfern"`; anything else is refused with 400.
   * `version` must already be downloaded, so omit it (or pass `"latest"`) to
   * take the newest local build.
   */
  createProfile(request: {
    name: string;
    browser: string;
    version?: string;
    /** Omit, or pass `""`, for a profile with no proxy. Excludes `vpnId`. */
    proxy_id?: string;
    /** Omit, or pass `""`, for a profile with no VPN. Excludes `proxyId`. */
    vpn_id?: string;
    launch_hook?: string;
    release_type?: string;
    wayfern_config?: WayfernConfig;
    group_id?: string;
    tags?: string[];
    ephemeral?: boolean;
    temporary?: boolean;
  }): Promise<ApiProfileResponse> {
    return this.#request("POST", "/v1/profiles", body({ ...request }));
  }

  /**
   * PUT /v1/profiles/{id}
   *
   * A profile's browser engine is fixed at creation, so there is no `browser`
   * property. Pass `proxy_id: ""` or `vpn_id: ""` to detach one; leaving
   * either out changes nothing.
   */
  updateProfile(
    profileId: string,
    request: {
      name?: string;
      version?: string;
      proxy_id?: string;
      vpn_id?: string;
      launch_hook?: string;
      release_type?: string;
      group_id?: string;
      tags?: string[];
      extension_group_id?: string;
      proxy_bypass_rules?: string[];
      /** `"Disabled"`, `"Regular"` or `"Encrypted"`. */
      sync_mode?: string;
      clear_on_close?: boolean;
    },
  ): Promise<ApiProfileResponse> {
    return this.#request("PUT", `/v1/profiles/${segment(profileId)}`, body({ ...request }));
  }

  /** DELETE /v1/profiles/{id} */
  deleteProfile(profileId: string): Promise<void> {
    return this.#request("DELETE", `/v1/profiles/${segment(profileId)}`);
  }

  /**
   * POST /v1/profiles/{id}/run
   *
   * Prefer {@link withProfile}, which stops the browser again afterwards.
   */
  runProfile(profileId: string, options: RunProfileOptions = {}): Promise<RunProfileResponse> {
    return this.#request(
      "POST",
      `/v1/profiles/${segment(profileId)}/run`,
      body({ url: options.url, headless: options.headless }),
    );
  }

  /** POST /v1/profiles/{id}/run-remote */
  runProfileRemote(profileId: string, options: { url?: string } = {}): Promise<RunRemoteResponse> {
    return this.#request(
      "POST",
      `/v1/profiles/${segment(profileId)}/run-remote`,
      body({ url: options.url }),
    );
  }

  /**
   * POST /v1/profiles/{id}/cloud-sync
   *
   * `mode` is `"Disabled"`, `"Regular"` or `"Encrypted"`. An encrypted profile
   * cannot be launched remotely: its key never leaves this machine, so a
   * remote host would download ciphertext.
   */
  setProfileCloudSync(profileId: string, mode: string): Promise<SetCloudSyncResponse> {
    return this.#request("POST", `/v1/profiles/${segment(profileId)}/cloud-sync`, { mode });
  }

  /** POST /v1/profiles/{id}/open-url */
  openUrl(profileId: string, url: string): Promise<void> {
    return this.#request("POST", `/v1/profiles/${segment(profileId)}/open-url`, { url });
  }

  /**
   * POST /v1/profiles/{id}/kill
   *
   * A 503 here means the fleet could not be reached and the remote browser is
   * *still running*, not that it stopped.
   */
  killProfile(profileId: string): Promise<void> {
    return this.#request("POST", `/v1/profiles/${segment(profileId)}/kill`);
  }

  /**
   * POST /v1/profiles/batch/run
   *
   * Answers 200 even when some profiles failed; read `results[].ok`.
   */
  batchRunProfiles(
    profileIds: string[],
    options: RunProfileOptions = {},
  ): Promise<BatchRunResponse> {
    return this.#request(
      "POST",
      "/v1/profiles/batch/run",
      body({ profile_ids: profileIds, url: options.url, headless: options.headless }),
    );
  }

  /** POST /v1/profiles/batch/stop */
  batchStopProfiles(profileIds: string[]): Promise<BatchStopResponse> {
    return this.#request("POST", "/v1/profiles/batch/stop", { profile_ids: profileIds });
  }

  /**
   * GET /v1/profiles/import/detect
   *
   * Without `folder` the app scans the default browser locations.
   */
  detectImportProfiles(options: { folder?: string } = {}): Promise<DetectedProfilesResponse> {
    return this.#request(
      "GET",
      "/v1/profiles/import/detect",
      undefined,
      query({ folder: options.folder }),
    );
  }

  /**
   * POST /v1/profiles/import
   *
   * `duplicate_strategy` is `"skip"` or `"rename"` (the default). Each item is
   * isolated: one failure does not stop the rest.
   */
  importProfiles(
    items: ImportProfileItem[],
    options: {
      group_id?: string;
      duplicate_strategy?: "skip" | "rename";
      wayfern_config?: WayfernConfig;
    } = {},
  ): Promise<ProfileImportBatchResult> {
    return this.#request("POST", "/v1/profiles/import", body({ items, ...options }));
  }

  /**
   * POST /v1/profiles/{id}/cookies/import
   *
   * `content` is a raw cookie file. The format is detected: a JSON array in
   * the Puppeteer style, or a Netscape `cookies.txt`.
   */
  importProfileCookies(profileId: string, content: string): Promise<ImportCookiesResponse> {
    return this.#request("POST", `/v1/profiles/${segment(profileId)}/cookies/import`, {
      content,
    });
  }

  /**
   * POST /v1/profiles/distribute-proxies
   *
   * Applies one proxy per profile. Configuration rather than automation, so it
   * costs no automation quota. Answers 200 even when some pairs failed: read
   * `results[].ok`, and note that a profile whose browser is running is refused
   * rather than moved.
   */
  distributeProxies(pairs: ProxyPair[]): Promise<DistributeProxiesResponse> {
    return this.#request("POST", "/v1/profiles/distribute-proxies", { pairs });
  }

  // ------------------------------------------------------------------
  // Agent: reading and driving a running profile
  // ------------------------------------------------------------------

  /**
   * POST /v1/profiles/{id}/agent/perceive
   *
   * When the answer says `truncated`, pass its `cursor` back to continue where
   * it stopped.
   */
  agentPerceive(
    profileId: string,
    request: {
      /** Total byte cap across cursor pages. Default 1 MiB, ceiling 4 MiB. */
      max_bytes?: number;
      /** Capture budget in ms. Default 5000, clamped to [100, 60000]. */
      budget_ms?: number;
      max_nodes?: number;
      include_text?: boolean;
      viewport_only?: boolean;
      /** `"reading"` (default) or `"visual"`. */
      text_order?: string;
      cursor?: string;
    } = {},
  ): Promise<PerceptionPage> {
    return this.#request(
      "POST",
      `/v1/profiles/${segment(profileId)}/agent/perceive`,
      body({ ...request }),
    );
  }

  /**
   * POST /v1/profiles/{id}/agent/resolve-locator
   *
   * Succeeds only when the locator matches exactly one element.
   */
  agentResolveLocator(
    profileId: string,
    request: { locator: LocatorDescription; candidate_limit?: number },
  ): Promise<LocatorResolution> {
    return this.#request(
      "POST",
      `/v1/profiles/${segment(profileId)}/agent/resolve-locator`,
      body({ ...request }),
    );
  }

  /**
   * POST /v1/profiles/{id}/agent/click
   *
   * `button` is `"left"` (the default), `"middle"`, `"right"`, `"back"` or
   * `"forward"`.
   */
  agentClick(
    profileId: string,
    request: { locator: LocatorDescription; button?: string; click_count?: number },
  ): Promise<AgentClick> {
    return this.#request(
      "POST",
      `/v1/profiles/${segment(profileId)}/agent/click`,
      body({ ...request }),
    );
  }

  /**
   * POST /v1/profiles/{id}/agent/type
   *
   * `wpm` is honoured by the fallback engine only; a recent Wayfern types at
   * the profile's own rhythm.
   */
  agentType(
    profileId: string,
    request: {
      locator: LocatorDescription;
      text: string;
      /** Empty the field first. Default true. */
      clear_first?: boolean;
      /** Mistype and correct a few characters, as a hand does. Default true. */
      typos?: boolean;
      wpm?: number;
    },
  ): Promise<AgentTyping> {
    return this.#request(
      "POST",
      `/v1/profiles/${segment(profileId)}/agent/type`,
      body({ ...request }),
    );
  }

  /**
   * POST /v1/profiles/{id}/agent/extract
   *
   * A container that matches nothing is a result with `stopReason` set to
   * `"no-container"`, not an error.
   */
  agentExtract(
    profileId: string,
    request: {
      container: LocatorDescription;
      field_map: ExtractionField[];
      next_page?: LocatorDescription;
      max_pages?: number;
      max_rows?: number;
      max_bytes?: number;
      max_nodes?: number;
      time_budget_ms?: number;
    },
  ): Promise<Extraction> {
    return this.#request(
      "POST",
      `/v1/profiles/${segment(profileId)}/agent/extract`,
      body({ ...request }),
    );
  }

  /**
   * POST /v1/profiles/{id}/agent/pick
   *
   * Arms a picker in the visible browser and waits for a human to click
   * something. Nothing picked inside `timeout_ms` throws `RequestTimeout`.
   */
  agentPick(profileId: string, request: { timeout_ms?: number } = {}): Promise<PickedElement> {
    return this.#request(
      "POST",
      `/v1/profiles/${segment(profileId)}/agent/pick`,
      body({ ...request }),
    );
  }

  // ------------------------------------------------------------------
  // Remote sessions
  // ------------------------------------------------------------------

  /** GET /v1/remote-sessions */
  listRemoteSessions(): Promise<ApiRemoteSessionsResponse> {
    return this.#request("GET", "/v1/remote-sessions");
  }

  /** GET /v1/remote-sessions/{id} */
  getRemoteSession(sessionId: string): Promise<RemoteSessionState> {
    return this.#request("GET", `/v1/remote-sessions/${segment(sessionId)}`);
  }

  /** DELETE /v1/remote-sessions/{id} */
  stopRemoteSession(sessionId: string): Promise<StopRemoteResponse> {
    return this.#request("DELETE", `/v1/remote-sessions/${segment(sessionId)}`);
  }

  /**
   * The websocket address of `GET /v1/remote-sessions/{id}/cdp`.
   *
   * That path is a WebSocket upgrade, not a request `fetch` can make, so this
   * builds the address and leaves the socket to a websocket library. Send the
   * same `Authorization: Bearer` header on the handshake.
   */
  remoteSessionCdpUrl(sessionId: string): string {
    const scheme = this.#scheme === "https" ? "wss" : "ws";
    return `${scheme}://${this.host}:${this.port}${this.#prefix}/v1/remote-sessions/${segment(
      sessionId,
    )}/cdp`;
  }

  /** GET /v1/remote-hours */
  getRemoteHours(): Promise<RemoteHoursQuota> {
    return this.#request("GET", "/v1/remote-hours");
  }

  // ------------------------------------------------------------------
  // Cookie bot
  // ------------------------------------------------------------------

  /**
   * GET /v1/cookie-bot/schedules
   *
   * `scope` is `"mine"` (the default) or `"team"`.
   */
  listCookieBotSchedules(options: { scope?: string } = {}): Promise<CookieBotScheduleList> {
    return this.#request(
      "GET",
      "/v1/cookie-bot/schedules",
      undefined,
      query({ scope: options.scope }),
    );
  }

  /** GET /v1/cookie-bot/schedules/{profile_id} */
  getCookieBotSchedule(profileId: string): Promise<CookieBotSchedule> {
    return this.#request("GET", `/v1/cookie-bot/schedules/${segment(profileId)}`);
  }

  /**
   * PUT /v1/cookie-bot/schedules/{profile_id}
   *
   * `run_at_minute` is minutes past local midnight (0 to 1439) and `days_mask`
   * is a weekday bitmask with bit 0 as Monday. A teammate already enrolling
   * this profile makes the write 409 until `acknowledge_conflict` is true.
   */
  setCookieBotSchedule(
    profileId: string,
    request: {
      enabled: boolean;
      run_at_minute: number;
      days_mask: number;
      timezone: string;
      /** Server-issued preset id from `listCookieBotPresets`. */
      preset: string;
      max_minutes: number;
      profile_name?: string;
      platform?: string;
      /** Absolute http(s) URLs to browse. The bot visits only these. */
      sites?: string[];
      jitter_seconds?: number;
      acknowledge_conflict?: boolean;
    },
  ): Promise<CookieBotScheduleSaved> {
    return this.#request(
      "PUT",
      `/v1/cookie-bot/schedules/${segment(profileId)}`,
      body({ ...request }),
    );
  }

  /** DELETE /v1/cookie-bot/schedules/{profile_id} */
  deleteCookieBotSchedule(profileId: string): Promise<CookieBotScheduleDeleted> {
    return this.#request("DELETE", `/v1/cookie-bot/schedules/${segment(profileId)}`);
  }

  /**
   * GET /v1/cookie-bot/conflicts
   *
   * A dry run: asks who else enrols this profile, without writing.
   */
  getCookieBotConflicts(
    profileId: string,
    options: { run_at_minute?: number; timezone?: string; days_mask?: number } = {},
  ): Promise<CookieBotConflictCheck> {
    return this.#request(
      "GET",
      "/v1/cookie-bot/conflicts",
      undefined,
      query({ profile_id: profileId, ...options }),
    );
  }

  /**
   * GET /v1/cookie-bot/runs
   *
   * Newest first. `before` is the `next_before` of the previous page.
   */
  listCookieBotRuns(
    options: { profile_id?: string; scope?: string; limit?: number; before?: string } = {},
  ): Promise<CookieBotRunPage> {
    return this.#request("GET", "/v1/cookie-bot/runs", undefined, query({ ...options }));
  }

  /**
   * POST /v1/cookie-bot/runs
   *
   * Answers 202: the run keeps going for minutes after this resolves. The
   * profile must already have a schedule, which is where the preset and the
   * site list live.
   */
  startCookieBotRun(request: {
    profile_id: string;
    max_minutes?: number;
  }): Promise<CookieBotRunStarted> {
    return this.#request("POST", "/v1/cookie-bot/runs", body({ ...request }));
  }

  /** DELETE /v1/cookie-bot/runs/{run_id} */
  cancelCookieBotRun(runId: string): Promise<CookieBotRun> {
    return this.#request("DELETE", `/v1/cookie-bot/runs/${segment(runId)}`);
  }

  /** GET /v1/cookie-bot/presets */
  listCookieBotPresets(): Promise<CookieBotPresetList> {
    return this.#request("GET", "/v1/cookie-bot/presets");
  }

  /**
   * GET /v1/cookie-bot/usage
   *
   * `period` is `YYYY-MM`, defaulting to the current UTC month.
   */
  getCookieBotUsage(options: { period?: string } = {}): Promise<CookieBotUsage> {
    return this.#request("GET", "/v1/cookie-bot/usage", undefined, query({ ...options }));
  }

  // ------------------------------------------------------------------
  // Groups and tags
  // ------------------------------------------------------------------

  /** GET /v1/groups */
  listGroups(): Promise<ApiGroupResponse[]> {
    return this.#request("GET", "/v1/groups");
  }

  /** GET /v1/groups/{id} */
  getGroup(groupId: string): Promise<ApiGroupResponse> {
    return this.#request("GET", `/v1/groups/${segment(groupId)}`);
  }

  /** POST /v1/groups */
  createGroup(name: string): Promise<ApiGroupResponse> {
    return this.#request("POST", "/v1/groups", { name });
  }

  /** PUT /v1/groups/{id} */
  updateGroup(groupId: string, name: string): Promise<ApiGroupResponse> {
    return this.#request("PUT", `/v1/groups/${segment(groupId)}`, { name });
  }

  /** DELETE /v1/groups/{id} */
  deleteGroup(groupId: string): Promise<void> {
    return this.#request("DELETE", `/v1/groups/${segment(groupId)}`);
  }

  /** GET /v1/tags */
  listTags(): Promise<string[]> {
    return this.#request("GET", "/v1/tags");
  }

  // ------------------------------------------------------------------
  // Proxies
  // ------------------------------------------------------------------

  /** GET /v1/proxies */
  listProxies(): Promise<ApiProxyResponse[]> {
    return this.#request("GET", "/v1/proxies");
  }

  /** GET /v1/proxies/{id} */
  getProxy(proxyId: string): Promise<ApiProxyResponse> {
    return this.#request("GET", `/v1/proxies/${segment(proxyId)}`);
  }

  /** POST /v1/proxies */
  createProxy(request: {
    name: string;
    proxy_settings: ProxySettings;
  }): Promise<ApiProxyResponse> {
    return this.#request("POST", "/v1/proxies", { ...request });
  }

  /** PUT /v1/proxies/{id} */
  updateProxy(
    proxyId: string,
    request: { name?: string; proxy_settings?: ProxySettings },
  ): Promise<ApiProxyResponse> {
    return this.#request("PUT", `/v1/proxies/${segment(proxyId)}`, body({ ...request }));
  }

  /** DELETE /v1/proxies/{id} */
  deleteProxy(proxyId: string): Promise<void> {
    return this.#request("DELETE", `/v1/proxies/${segment(proxyId)}`);
  }

  /**
   * POST /v1/proxies/import
   *
   * `format` is `"txt"` (one proxy per line) or `"json"` (a Donut proxy
   * export).
   */
  importProxies(request: {
    format: string;
    content: string;
    name_prefix?: string;
  }): Promise<ImportProxiesResponse> {
    return this.#request("POST", "/v1/proxies/import", body({ ...request }));
  }

  // ------------------------------------------------------------------
  // VPNs
  // ------------------------------------------------------------------

  /** GET /v1/vpns */
  listVpns(): Promise<ApiVpnResponse[]> {
    return this.#request("GET", "/v1/vpns");
  }

  /** GET /v1/vpns/{id} */
  getVpn(vpnId: string): Promise<ApiVpnResponse> {
    return this.#request("GET", `/v1/vpns/${segment(vpnId)}`);
  }

  /**
   * GET /v1/vpns/{id}/export
   *
   * Returns the decrypted `.conf` text. Treat it as a secret.
   */
  exportVpn(vpnId: string): Promise<ApiVpnExportResponse> {
    return this.#request("GET", `/v1/vpns/${segment(vpnId)}/export`);
  }

  /** POST /v1/vpns/import */
  importVpn(request: {
    /** Raw WireGuard `.conf` content. */
    content: string;
    filename: string;
    name?: string;
  }): Promise<ApiVpnResponse> {
    return this.#request("POST", "/v1/vpns/import", body({ ...request }));
  }

  /** POST /v1/vpns. `vpn_type` must be `"WireGuard"`. */
  createVpn(request: {
    name: string;
    vpn_type: string;
    config_data: string;
  }): Promise<ApiVpnResponse> {
    return this.#request("POST", "/v1/vpns", { ...request });
  }

  /** PUT /v1/vpns/{id} */
  updateVpn(vpnId: string, name: string): Promise<ApiVpnResponse> {
    return this.#request("PUT", `/v1/vpns/${segment(vpnId)}`, { name });
  }

  /** DELETE /v1/vpns/{id} */
  deleteVpn(vpnId: string): Promise<void> {
    return this.#request("DELETE", `/v1/vpns/${segment(vpnId)}`);
  }

  // ------------------------------------------------------------------
  // Extensions
  // ------------------------------------------------------------------

  /** GET /v1/extensions */
  listExtensions(): Promise<Extension[]> {
    return this.#request("GET", "/v1/extensions");
  }

  /** GET /v1/extensions/{id} */
  getExtension(extensionId: string): Promise<Extension> {
    return this.#request("GET", `/v1/extensions/${segment(extensionId)}`);
  }

  /**
   * POST /v1/extensions
   *
   * Either upload bytes (`file_name` plus `file_data_base64`) or point at a
   * path on this machine (`source_path`). Answers 201.
   */
  createExtension(request: {
    name?: string;
    /** Its suffix picks the type: `.crx` or `.zip`. */
    file_name?: string;
    file_data_base64?: string;
    source_path?: string;
    /** Load a `source_path` directory in place. A linked extension never syncs. */
    link?: boolean;
  }): Promise<Extension> {
    return this.#request("POST", "/v1/extensions", body({ ...request }));
  }

  /** PUT /v1/extensions/{id} */
  updateExtension(
    extensionId: string,
    request: {
      name?: string;
      file_name?: string;
      file_data_base64?: string;
      source_path?: string;
      link?: boolean;
    },
  ): Promise<Extension> {
    return this.#request("PUT", `/v1/extensions/${segment(extensionId)}`, body({ ...request }));
  }

  /** DELETE /v1/extensions/{id} */
  deleteExtension(extensionId: string): Promise<void> {
    return this.#request("DELETE", `/v1/extensions/${segment(extensionId)}`);
  }

  /** GET /v1/extension-groups */
  listExtensionGroups(): Promise<ExtensionGroup[]> {
    return this.#request("GET", "/v1/extension-groups");
  }

  /** GET /v1/extension-groups/{id} */
  getExtensionGroup(groupId: string): Promise<ExtensionGroup> {
    return this.#request("GET", `/v1/extension-groups/${segment(groupId)}`);
  }

  /** POST /v1/extension-groups. Answers 201. */
  createExtensionGroup(name: string): Promise<ExtensionGroup> {
    return this.#request("POST", "/v1/extension-groups", { name });
  }

  /**
   * PUT /v1/extension-groups/{id}
   *
   * `extension_ids` replaces the whole membership list. To change one member,
   * use {@link addExtensionToGroup} or {@link removeExtensionFromGroup}.
   */
  updateExtensionGroup(
    groupId: string,
    request: { name?: string; extension_ids?: string[] },
  ): Promise<ExtensionGroup> {
    return this.#request(
      "PUT",
      `/v1/extension-groups/${segment(groupId)}`,
      body({ ...request }),
    );
  }

  /** DELETE /v1/extension-groups/{id} */
  deleteExtensionGroup(groupId: string): Promise<void> {
    return this.#request("DELETE", `/v1/extension-groups/${segment(groupId)}`);
  }

  /** POST /v1/extension-groups/{id}/extensions/{extension_id} */
  addExtensionToGroup(groupId: string, extensionId: string): Promise<ExtensionGroup> {
    return this.#request(
      "POST",
      `/v1/extension-groups/${segment(groupId)}/extensions/${segment(extensionId)}`,
    );
  }

  /** DELETE /v1/extension-groups/{id}/extensions/{extension_id} */
  removeExtensionFromGroup(groupId: string, extensionId: string): Promise<ExtensionGroup> {
    return this.#request(
      "DELETE",
      `/v1/extension-groups/${segment(groupId)}/extensions/${segment(extensionId)}`,
    );
  }

  // ------------------------------------------------------------------
  // Browsers
  // ------------------------------------------------------------------

  /**
   * POST /v1/browsers/download
   *
   * Resolves once the build is on disk, so give the client a long
   * `timeoutMs`. A 409 means the same version is already downloading.
   */
  downloadBrowser(request: {
    browser: string;
    version: string;
  }): Promise<DownloadBrowserResponse> {
    return this.#request("POST", "/v1/browsers/download", { ...request });
  }

  /** GET /v1/browsers/{browser}/versions */
  listBrowserVersions(browser: string): Promise<string[]> {
    return this.#request("GET", `/v1/browsers/${segment(browser)}/versions`);
  }

  /** GET /v1/browsers/{browser}/versions/{version}/downloaded */
  isBrowserDownloaded(browser: string, version: string): Promise<boolean> {
    return this.#request(
      "GET",
      `/v1/browsers/${segment(browser)}/versions/${segment(version)}/downloaded`,
    );
  }

  // ------------------------------------------------------------------
  // Convenience
  // ------------------------------------------------------------------

  /**
   * Launch a profile, run `work`, then stop the browser again.
   *
   * ```ts
   * const rows = await client.withProfile(profileId, { headless: true }, async (session) => {
   *   console.log(session.cdpUrl);
   *   return client.agentExtract(profileId, { container, field_map });
   * });
   * ```
   *
   * The browser is stopped when `work` finishes, including when it throws. A
   * failure to stop never replaces the error `work` threw; it is attached to
   * that error's `cause` chain instead, and reachable on `session.cleanupError`.
   */
  async withProfile<T>(
    profileId: string,
    options: RunProfileOptions,
    work: (session: RunSession) => Promise<T> | T,
  ): Promise<T> {
    const response = await this.runProfile(profileId, options);
    const session = new RunSession(this, profileId, response);
    let failed = false;
    try {
      return await work(session);
    } catch (error) {
      failed = true;
      throw error;
    } finally {
      try {
        await this.killProfile(profileId);
      } catch (cleanupError) {
        session.cleanupError = cleanupError;
        if (!failed) {
          throw cleanupError;
        }
      }
    }
  }
}

/**
 * A profile launched by {@link DonutClient.withProfile}.
 *
 * It also implements `Symbol.asyncDispose`, so a runtime with `await using`
 * can hold one directly; `withProfile` is the form that works everywhere.
 */
export class RunSession {
  readonly client: DonutClient;
  readonly profileId: string;
  /** The whole body of `POST /v1/profiles/{id}/run`. */
  readonly response: RunProfileResponse;
  /** The browser's CDP port. */
  readonly remoteDebuggingPort: number;
  /** Whether the browser actually started headless. */
  readonly headless: boolean;
  /** A failure while stopping the browser, kept rather than thrown. */
  cleanupError: unknown = undefined;

  constructor(client: DonutClient, profileId: string, response: RunProfileResponse) {
    this.client = client;
    this.profileId = profileId;
    this.response = response;
    this.remoteDebuggingPort = response.remote_debugging_port;
    this.headless = response.headless;
  }

  /**
   * The browser's DevTools endpoint, e.g. `http://127.0.0.1:9222`.
   *
   * `GET {cdpUrl}/json/version` returns the `webSocketDebuggerUrl` a CDP
   * library connects to.
   */
  get cdpUrl(): string {
    return `http://${this.client.host}:${this.remoteDebuggingPort}`;
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.client.killProfile(this.profileId);
  }
}

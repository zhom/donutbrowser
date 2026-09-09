/**
 * Exceptions thrown by the Donut Browser SDK.
 *
 * The local REST API answers with a plain-text body and one of a small set of
 * statuses. Each status means one thing, so each gets its own class and a
 * caller can branch on `instanceof` instead of on a number:
 *
 * | Status | Class                 | Meaning                                   |
 * | -----: | --------------------- | ----------------------------------------- |
 * |    400 | `ValidationError`     | Malformed request, duplicate name         |
 * |    401 | `Unauthorized`        | Missing or wrong bearer token             |
 * |    402 | `PaymentRequired`     | Automation needs an active paid plan      |
 * |    403 | `Forbidden`           | Terms not accepted, or not signed in      |
 * |    404 | `NotFound`            | No such profile, group, proxy, ...        |
 * |    408 | `RequestTimeout`      | `agent/pick` waited and nothing was picked |
 * |    409 | `Conflict`            | Something else holds the profile          |
 * |    429 | `RateLimited`         | Quota spent; see `retryAfter`             |
 * |    500 | `ServerError`         | Internal failure                          |
 * |    502 | `BadGateway`          | The browser or relay answered wrongly     |
 * |    503 | `ServiceUnavailable`  | Cloud, fleet or lock service unreachable  |
 *
 * Some bodies are the structured `{"code": ..., "params": {...}}` strings the
 * desktop app shares with its own frontend. When one arrives, `code` and
 * `params` are filled in; otherwise `code` is `null` and `body` holds the
 * diagnostic text as sent.
 */

/** Base class for everything this package throws. */
export class DonutError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = new.target.name;
  }
}

/**
 * The app could not be reached at all.
 *
 * Usually means the local API is switched off, is listening on another port,
 * or the desktop app is not running.
 */
export class DonutConnectionError extends DonutError {}

export interface DonutApiErrorInit {
  method?: string;
  path?: string;
  headers?: Headers | Record<string, string>;
}

/** The app answered, and the answer was an error status. */
export class DonutApiError extends DonutError {
  status: number;
  body: string;
  method: string;
  path: string;
  headers: Record<string, string>;
  /** The `code` of a structured `{"code": ...}` body, else `null`. */
  code: string | null;
  /** The `params` of a structured body, else an empty object. */
  params: Record<string, unknown>;

  constructor(status: number, body: string, init: DonutApiErrorInit = {}) {
    const method = init.method ?? "";
    const path = init.path ?? "";
    const headers = normaliseHeaders(init.headers);

    let code: string | null = null;
    let params: Record<string, unknown> = {};
    const trimmed = body.trim();
    if (trimmed.startsWith("{")) {
      try {
        const decoded: unknown = JSON.parse(trimmed);
        if (decoded !== null && typeof decoded === "object") {
          const record = decoded as Record<string, unknown>;
          if (typeof record.code === "string") {
            code = record.code;
            if (record.params !== null && typeof record.params === "object") {
              params = record.params as Record<string, unknown>;
            }
          }
        }
      } catch {
        // Not JSON after all; the plain text below is the whole story.
      }
    }

    const where = `${method} ${path}`.trim();
    const detail = code ?? (trimmed || "(empty body)");
    super(where ? `${status} on ${where}: ${detail}` : `${status}: ${detail}`);

    this.status = status;
    this.body = body;
    this.method = method;
    this.path = path;
    this.headers = headers;
    this.code = code;
    this.params = params;
  }
}

/** 400: the request was malformed, duplicated a name, or named something unsupported. */
export class ValidationError extends DonutApiError {}

/** 401: no bearer token, the wrong one, or the local API has no token stored. */
export class Unauthorized extends DonutApiError {}

/** 402: this action needs an active paid plan, or the proxy behind it lapsed. */
export class PaymentRequired extends DonutApiError {}

/** 403: the Wayfern terms are not accepted, or this desktop is not signed in. */
export class Forbidden extends DonutApiError {}

/** 404: no entity with that id. */
export class NotFound extends DonutApiError {}

/** 408: `agentPick` waited its whole timeout and nothing was picked. */
export class RequestTimeout extends DonutApiError {}

/** 409: something else holds the profile — a browser, a teammate, a remote session. */
export class Conflict extends DonutApiError {}

/**
 * 500 and the other 5xx: the app, the fleet or an upstream failed.
 *
 * `BadGateway` and `ServiceUnavailable` extend this, so one
 * `instanceof ServerError` covers every server-side failure.
 */
export class ServerError extends DonutApiError {}

/** 502: the browser or the relay did not answer the way it documents. */
export class BadGateway extends ServerError {}

/**
 * 503: Donut cloud, the remote fleet, or the profile lock service is unreachable.
 *
 * Whatever was running keeps running: a 503 from `killProfile` or from stopping
 * a remote session means the browser is still up, not that it stopped.
 */
export class ServiceUnavailable extends ServerError {}

/**
 * 429: the shared automation quota is spent.
 *
 * `retryAfter` is the number of seconds the server asked the caller to wait,
 * taken from the `Retry-After` response header. It is `null` only when the
 * header is missing or unreadable.
 */
export class RateLimited extends DonutApiError {
  retryAfter: number | null;

  constructor(status: number, body: string, init: DonutApiErrorInit = {}) {
    super(status, body, init);
    const raw = this.headers["retry-after"];
    const seconds = raw === undefined ? Number.NaN : Number.parseInt(raw.trim(), 10);
    this.retryAfter = Number.isFinite(seconds) ? seconds : null;
  }
}

function normaliseHeaders(
  headers: Headers | Record<string, string> | undefined,
): Record<string, string> {
  const result: Record<string, string> = {};
  if (headers === undefined) {
    return result;
  }
  if (typeof (headers as Headers).forEach === "function" && !Array.isArray(headers)) {
    (headers as Headers).forEach((value, key) => {
      result[key.toLowerCase()] = value;
    });
    return result;
  }
  for (const [key, value] of Object.entries(headers as Record<string, string>)) {
    result[key.toLowerCase()] = value;
  }
  return result;
}

const BY_STATUS = new Map<number, typeof DonutApiError>([
  [400, ValidationError],
  [401, Unauthorized],
  [402, PaymentRequired],
  [403, Forbidden],
  [404, NotFound],
  [408, RequestTimeout],
  [409, Conflict],
  [429, RateLimited],
  [500, ServerError],
  [502, BadGateway],
  [503, ServiceUnavailable],
]);

/**
 * Build the error that belongs to `status`.
 *
 * A status with no class of its own becomes a plain `DonutApiError`, so a
 * future status added to the app still throws something a caller can catch
 * rather than escaping as a decode failure.
 */
export function errorForStatus(
  status: number,
  body: string,
  init: DonutApiErrorInit = {},
): DonutApiError {
  const known = BY_STATUS.get(status);
  if (known !== undefined) {
    return new known(status, body, init);
  }
  return status >= 500
    ? new ServerError(status, body, init)
    : new DonutApiError(status, body, init);
}

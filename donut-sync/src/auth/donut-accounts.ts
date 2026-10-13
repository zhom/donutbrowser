import { createHash } from "node:crypto";

export const DEFAULT_DONUT_API_URL = "https://api.donutbrowser.com";

/**
 * The canonical form of a sync server address, or null when the value cannot
 * be one. The desktop app and the Donut API write addresses the same way, so a
 * token bound to what a person typed matches what this server says it is.
 */
export function normalizeSyncServerUrl(raw: unknown): string | null {
  if (typeof raw !== "string" || raw.length > 2048) return null;
  let url: URL;
  try {
    url = new URL(raw.trim());
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  if (!url.hostname || url.username || url.password) return null;
  if (url.search || url.hash) return null;
  return `${url.protocol}//${url.host}${url.pathname.replace(/\/+$/, "")}`;
}

export type AccountTeamScope = {
  ownerId: string;
  teamId: string;
  teamProfileLimit: number;
  role?: string;
  groupIds?: string[] | null;
};

/** Whose a token is, as the Donut API answers for this server. */
export type AccountScope = {
  sub: string;
  /** The plan's profile limit. 0 is unlimited. */
  profileLimit: number;
  team: AccountTeamScope | null;
};

/**
 * Signs Donut accounts in to this server.
 *
 * A paid account's app sends a token the Donut API issued for this server's
 * address. The API says whose it is, with the plan's profile limit and the
 * team scope, so this server applies the same namespace, limit and team rules
 * as Donut's own sync server. Answers are cached briefly: a seat or plan
 * change applies within the cache time.
 */
export class DonutAccounts {
  static readonly CACHE_TTL_MS = 30_000;
  private static readonly REQUEST_TIMEOUT_MS = 10_000;
  private readonly cache = new Map<
    string,
    { value: AccountScope | null; expires: number }
  >();

  constructor(
    readonly publicUrl: string,
    private readonly apiUrl: string,
  ) {}

  /**
   * The account behind `token`, or null when the API refuses it. Throws when
   * the API cannot be asked, so the caller fails closed.
   */
  async resolve(token: string): Promise<AccountScope | null> {
    // Only the API can verify the signature. Asking it about tokens that are
    // plainly not for this server would let anyone who reaches this server
    // spend its share of the API's request limit.
    if (!this.claimsFitThisServer(token)) return null;
    const key = createHash("sha256").update(token).digest("hex");
    const now = Date.now();
    const cached = this.cache.get(key);
    if (cached && cached.expires > now) return cached.value;

    const response = await fetch(
      `${this.apiUrl}/api/auth/server-sync/introspect`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ token, server: this.publicUrl }),
        signal: AbortSignal.timeout(DonutAccounts.REQUEST_TIMEOUT_MS),
      },
    );
    if (!response.ok) {
      throw new Error(`introspection returned ${response.status}`);
    }
    const value = accountScopeOf(await response.json());

    if (this.cache.size > 10_000) this.cache.clear();
    this.cache.set(key, {
      value,
      expires: now + DonutAccounts.CACHE_TTL_MS,
    });
    return value;
  }

  /** The token's own, unverified claims name this server and are current. */
  private claimsFitThisServer(token: string): boolean {
    const payload = token.split(".")[1];
    if (!payload) return false;
    let claims: Record<string, unknown>;
    try {
      claims = JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
    } catch {
      return false;
    }
    return (
      claims?.scope === SERVER_SYNC_SCOPE &&
      claims.aud === this.publicUrl &&
      typeof claims.exp === "number" &&
      claims.exp * 1000 > Date.now()
    );
  }
}

/** The scope of a token the Donut API issues for a customer's server. */
const SERVER_SYNC_SCOPE = "server-sync";

function accountScopeOf(body: unknown): AccountScope | null {
  if (!body || typeof body !== "object") return null;
  const answer = body as Record<string, unknown>;
  if (answer.active !== true) return null;
  if (typeof answer.sub !== "string" || !/^[^/]+$/.test(answer.sub)) {
    return null;
  }
  // 0 means unlimited, so a missing limit refuses rather than reads as 0.
  if (typeof answer.profileLimit !== "number") return null;
  const team =
    answer.team && typeof answer.team === "object"
      ? (answer.team as AccountTeamScope)
      : null;
  return { sub: answer.sub, profileLimit: answer.profileLimit, team };
}

/** The account sign-in this server's environment turns on, or null. */
export function donutAccountsFromEnv(
  get: (key: string) => string | undefined,
): DonutAccounts | null {
  const publicUrl = normalizeSyncServerUrl(get("SYNC_PUBLIC_URL"));
  if (!publicUrl) return null;
  const apiUrl = (get("DONUT_API_URL") || DEFAULT_DONUT_API_URL).replace(
    /\/+$/,
    "",
  );
  return new DonutAccounts(publicUrl, apiUrl);
}

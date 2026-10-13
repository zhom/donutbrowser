import { timingSafeEqual } from "node:crypto";
import {
  type CanActivate,
  type ExecutionContext,
  Injectable,
  Logger,
  ServiceUnavailableException,
  UnauthorizedException,
} from "@nestjs/common";
import { ConfigService } from "@nestjs/config";
import type { Request } from "express";
import * as jwt from "jsonwebtoken";
import { type DonutAccounts, donutAccountsFromEnv } from "./donut-accounts.js";
import type { UserContext } from "./user-context.interface.js";

/** Constant-time string compare; false on length mismatch (no early return). */
function safeEqual(a: string, b: string): boolean {
  const ab = Buffer.from(a);
  const bb = Buffer.from(b);
  return ab.length === bb.length && timingSafeEqual(ab, bb);
}

type TeamScope = {
  ownerId: string;
  teamId: string;
  teamProfileLimit: number;
  role?: string;
  groupIds?: string[] | null;
};

const GROUP_ID = /^[A-Za-z0-9_-]{1,128}$/;

const TEAM_ROLES = ["owner", "admin", "member", "launcher"] as const;
type TeamRole = (typeof TEAM_ROLES)[number];

/** A role the backend names, or `member` for one this build does not know. */
function teamRoleOf(role: unknown): TeamRole {
  return TEAM_ROLES.includes(role as TeamRole) ? (role as TeamRole) : "member";
}

/**
 * A user's effective namespace: their own, or for a team member the shared
 * owner namespace with the team's role, group access and limit.
 */
function scopedContext(
  mode: "cloud" | "account",
  sub: string,
  profileLimit: number,
  scope: TeamScope | null,
): UserContext {
  let prefix = `users/${sub}/`;
  let teamRole: TeamRole | undefined;
  let groupIds: string[] | undefined;
  let limit = profileLimit;
  if (
    scope &&
    typeof scope.ownerId === "string" &&
    /^[^/]+$/.test(scope.ownerId)
  ) {
    prefix = `users/${scope.ownerId}/`;
    teamRole = teamRoleOf(scope.role);
    // A malformed id cannot name a group, so it grants nothing; it never
    // turns a limited member into an unlimited one.
    if (Array.isArray(scope.groupIds)) {
      groupIds = scope.groupIds.filter(
        (id): id is string => typeof id === "string" && GROUP_ID.test(id),
      );
    }
    if (scope.teamProfileLimit > 0) {
      limit = scope.teamProfileLimit;
    }
  }
  return {
    mode,
    prefix,
    profileLimit: limit,
    sub,
    ...(teamRole ? { teamRole } : {}),
    ...(groupIds ? { groupIds } : {}),
  };
}

/** Only a JWT can be a Donut account token; anything else skips the API. */
function looksLikeJwt(token: string): boolean {
  return /^[\w-]+\.[\w-]+\.[\w-]+$/.test(token);
}

@Injectable()
export class AuthGuard implements CanActivate {
  private readonly logger = new Logger(AuthGuard.name);
  private jwtPublicKey: string | null = null;
  private readonly backendInternalUrl: string | undefined;
  private readonly backendInternalKey: string | undefined;
  private readonly accounts: DonutAccounts | null;

  // Short-lived cache of the per-user team scope so membership revocation takes
  // effect quickly (within TTL) without a backend round-trip on every request.
  private readonly teamScopeCache = new Map<
    string,
    { value: TeamScope | null; expires: number }
  >();
  private static readonly TEAM_SCOPE_TTL_MS = 30_000;

  constructor(private configService: ConfigService) {
    const publicKey = this.configService.get<string>("SYNC_JWT_PUBLIC_KEY");
    if (publicKey) {
      this.jwtPublicKey = publicKey.replace(/\\n/g, "\n");
      this.logger.log("JWT public key configured — cloud auth enabled");
    }
    this.backendInternalUrl = this.configService.get<string>(
      "BACKEND_INTERNAL_URL",
    );
    this.backendInternalKey = this.configService.get<string>(
      "BACKEND_INTERNAL_KEY",
    );
    this.accounts = donutAccountsFromEnv((key) =>
      this.configService.get<string>(key),
    );
    if (this.accounts) {
      this.logger.log(
        `Donut account sign-in enabled for ${this.accounts.publicUrl}`,
      );
    }
  }

  /**
   * Resolve a cloud user's team scope via the backend (the ONLY authority for
   * team membership). Cached briefly. Throws on backend error so the caller can
   * fail closed (fall back to the user's own namespace, never a team one).
   */
  private async resolveTeamScope(sub: string): Promise<TeamScope | null> {
    if (!this.backendInternalUrl || !this.backendInternalKey) return null;

    const now = Date.now();
    const cached = this.teamScopeCache.get(sub);
    if (cached && cached.expires > now) return cached.value;

    const resp = await fetch(
      `${this.backendInternalUrl}/api/auth/internal/team-scope`,
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "x-internal-key": this.backendInternalKey,
        },
        body: JSON.stringify({ userId: sub }),
      },
    );
    if (!resp.ok) {
      throw new Error(`team-scope resolver returned ${resp.status}`);
    }
    // A user with no team is answered with an empty body, not `null`.
    const body = await resp.text();
    const value = (body ? JSON.parse(body) : null) as TeamScope | null;

    // Bound the cache; a coarse clear is fine since entries are cheap to rebuild.
    if (this.teamScopeCache.size > 10_000) this.teamScopeCache.clear();
    this.teamScopeCache.set(sub, {
      value: value ?? null,
      expires: now + AuthGuard.TEAM_SCOPE_TTL_MS,
    });
    return value ?? null;
  }

  async canActivate(context: ExecutionContext): Promise<boolean> {
    const request = context.switchToHttp().getRequest<Request>();
    const authHeader = request.headers.authorization;

    if (!authHeader?.startsWith("Bearer ")) {
      throw new UnauthorizedException(
        "Missing or invalid authorization header",
      );
    }

    const token = authHeader.substring(7);

    // Try SYNC_TOKEN first (self-hosted mode)
    const expectedToken = this.configService.get<string>("SYNC_TOKEN");
    if (expectedToken && safeEqual(token, expectedToken)) {
      (request as unknown as Record<string, unknown>).user = {
        mode: "self-hosted",
        prefix: "",
        profileLimit: 0,
      } satisfies UserContext;
      return true;
    }

    // Try JWT verification (cloud mode)
    if (this.jwtPublicKey) {
      try {
        const decoded = jwt.verify(token, this.jwtPublicKey, {
          algorithms: ["RS256"],
        }) as jwt.JwtPayload;

        if (decoded.scope !== "sync") {
          throw new Error("Token scope is not accepted");
        }

        const sub = typeof decoded.sub === "string" ? decoded.sub : "";
        // Validate the prefix claim SHAPE before trusting it as an S3 key
        // prefix. An empty/over-broad prefix would make validateKeyAccess
        // (`key.startsWith(prefix)`) authorize the entire bucket.
        const ownPrefix = decoded.prefix || `users/${sub}/`;
        if (
          typeof ownPrefix !== "string" ||
          !/^users\/[^/]+\/$/.test(ownPrefix)
        ) {
          throw new Error(`Invalid prefix claim: ${String(decoded.prefix)}`);
        }

        // Resolve the EFFECTIVE namespace: a team member's requests are scoped
        // to the shared team owner namespace. The JWT carries no team data — the
        // backend is the sole authority. On any resolver error we fail CLOSED:
        // fall back to the user's own namespace, never widening to a team one.
        const ownProfileLimit =
          typeof decoded.profileLimit === "number" ? decoded.profileLimit : 0;
        let scope: TeamScope | null = null;
        try {
          scope = sub ? await this.resolveTeamScope(sub) : null;
        } catch (err) {
          this.logger.warn(
            `Team scope resolution failed for ${sub}; using own namespace: ${
              err instanceof Error ? err.message : err
            }`,
          );
        }

        const user = scopedContext("cloud", sub, ownProfileLimit, scope);
        // The prefix claim, already checked above, names the own namespace.
        if (!scope) user.prefix = ownPrefix;
        (request as unknown as Record<string, unknown>).user = user;
        return true;
      } catch (err) {
        this.logger.warn(
          `JWT verification failed: ${err instanceof Error ? err.message : err}`,
        );
      }
    }

    // A Donut account on this server: the API says whose the token is.
    if (this.accounts && looksLikeJwt(token)) {
      let account: Awaited<ReturnType<DonutAccounts["resolve"]>>;
      try {
        account = await this.accounts.resolve(token);
      } catch (err) {
        this.logger.warn(
          `Donut account check failed: ${err instanceof Error ? err.message : err}`,
        );
        throw new ServiceUnavailableException(
          "The Donut account service could not be reached",
        );
      }
      if (account) {
        (request as unknown as Record<string, unknown>).user = scopedContext(
          "account",
          account.sub,
          account.profileLimit,
          account.team,
        );
        return true;
      }
    }

    // If SYNC_TOKEN is configured but didn't match, or JWT failed
    if (!expectedToken && !this.jwtPublicKey && !this.accounts) {
      throw new UnauthorizedException(
        "No auth method configured on server (set SYNC_TOKEN, SYNC_PUBLIC_URL or SYNC_JWT_PUBLIC_KEY)",
      );
    }

    throw new UnauthorizedException("Invalid sync token or JWT");
  }
}

import { timingSafeEqual } from "node:crypto";
import {
  type CanActivate,
  type ExecutionContext,
  Injectable,
  Logger,
  UnauthorizedException,
} from "@nestjs/common";
import { ConfigService } from "@nestjs/config";
import type { Request } from "express";
import * as jwt from "jsonwebtoken";
import type { UserContext } from "./user-context.interface.js";

/** Constant-time string compare; false on length mismatch (no early return). */
function safeEqual(a: string, b: string): boolean {
  const ab = Buffer.from(a);
  const bb = Buffer.from(b);
  return ab.length === bb.length && timingSafeEqual(ab, bb);
}

type TeamScope = { ownerId: string; teamId: string; teamProfileLimit: number };

@Injectable()
export class AuthGuard implements CanActivate {
  private readonly logger = new Logger(AuthGuard.name);
  private jwtPublicKey: string | null = null;
  private readonly backendInternalUrl: string | undefined;
  private readonly backendInternalKey: string | undefined;

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
    const value = (await resp.json()) as TeamScope | null;

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
        let effectivePrefix = ownPrefix;
        let effectiveProfileLimit =
          typeof decoded.profileLimit === "number" ? decoded.profileLimit : 0;
        try {
          const scope = sub ? await this.resolveTeamScope(sub) : null;
          if (scope && /^[^/]+$/.test(scope.ownerId)) {
            effectivePrefix = `users/${scope.ownerId}/`;
            if (scope.teamProfileLimit > 0) {
              effectiveProfileLimit = scope.teamProfileLimit;
            }
          }
        } catch (err) {
          this.logger.warn(
            `Team scope resolution failed for ${sub}; using own namespace: ${
              err instanceof Error ? err.message : err
            }`,
          );
        }

        (request as unknown as Record<string, unknown>).user = {
          mode: "cloud",
          prefix: effectivePrefix,
          profileLimit: effectiveProfileLimit,
          sub,
        } satisfies UserContext;
        return true;
      } catch (err) {
        this.logger.warn(
          `JWT verification failed: ${err instanceof Error ? err.message : err}`,
        );
      }
    }

    // If SYNC_TOKEN is configured but didn't match, or JWT failed
    if (!expectedToken && !this.jwtPublicKey) {
      throw new UnauthorizedException(
        "No auth method configured on server (set SYNC_TOKEN or SYNC_JWT_PUBLIC_KEY)",
      );
    }

    throw new UnauthorizedException("Invalid sync token or JWT");
  }
}

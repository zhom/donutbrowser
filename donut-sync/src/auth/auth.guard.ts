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

@Injectable()
export class AuthGuard implements CanActivate {
  private readonly logger = new Logger(AuthGuard.name);
  private jwtPublicKey: string | null = null;

  constructor(private configService: ConfigService) {
    const publicKey = this.configService.get<string>("SYNC_JWT_PUBLIC_KEY");
    if (publicKey) {
      this.jwtPublicKey = publicKey.replace(/\\n/g, "\n");
      this.logger.log("JWT public key configured — cloud auth enabled");
    }
  }

  canActivate(context: ExecutionContext): boolean {
    const request = context.switchToHttp().getRequest<Request>();
    const authHeader = request.headers.authorization;

    if (!authHeader || !authHeader.startsWith("Bearer ")) {
      throw new UnauthorizedException(
        "Missing or invalid authorization header",
      );
    }

    const token = authHeader.substring(7);

    // Try SYNC_TOKEN first (self-hosted mode)
    const expectedToken = this.configService.get<string>("SYNC_TOKEN");
    if (expectedToken && token === expectedToken) {
      (request as any).user = {
        mode: "self-hosted",
        prefix: "",
        teamPrefix: null,
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

        (request as any).user = {
          mode: "cloud",
          prefix: decoded.prefix || `users/${decoded.sub}/`,
          teamPrefix: decoded.teamPrefix || null,
          profileLimit: decoded.profileLimit || 0,
        } satisfies UserContext;
        return true;
      } catch {
        // JWT verification failed — fall through to error
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

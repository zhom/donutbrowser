import {
  type CanActivate,
  type ExecutionContext,
  Injectable,
  UnauthorizedException,
} from "@nestjs/common";
import { ConfigService } from "@nestjs/config";
import type { Request } from "express";

@Injectable()
export class AuthGuard implements CanActivate {
  constructor(private configService: ConfigService) {}

  canActivate(context: ExecutionContext): boolean {
    const request = context.switchToHttp().getRequest<Request>();
    const authHeader = request.headers.authorization;

    if (!authHeader || !authHeader.startsWith("Bearer ")) {
      throw new UnauthorizedException(
        "Missing or invalid authorization header",
      );
    }

    const token = authHeader.substring(7);
    const expectedToken = this.configService.get<string>("SYNC_TOKEN");

    if (!expectedToken) {
      throw new UnauthorizedException("Sync token not configured on server");
    }

    if (token !== expectedToken) {
      throw new UnauthorizedException("Invalid sync token");
    }

    return true;
  }
}

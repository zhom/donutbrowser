import { timingSafeEqual } from "node:crypto";
import {
  BadRequestException,
  Body,
  Controller,
  Headers,
  HttpCode,
  Post,
  UnauthorizedException,
} from "@nestjs/common";
import { ConfigService } from "@nestjs/config";
import { SyncService } from "./sync.service.js";

/** Constant-time string compare; false on length mismatch. */
function safeEqual(a: string, b: string): boolean {
  const ab = Buffer.from(a);
  const bb = Buffer.from(b);
  return ab.length === bb.length && timingSafeEqual(ab, bb);
}

@Controller("v1/internal")
export class InternalController {
  private readonly internalKey: string | undefined;

  constructor(
    private readonly syncService: SyncService,
    private readonly configService: ConfigService,
  ) {
    this.internalKey = this.configService.get<string>("INTERNAL_KEY");
  }

  @Post("cleanup-excess-profiles")
  @HttpCode(200)
  async cleanupExcessProfiles(
    @Headers("x-internal-key") key: string,
    @Body() body: { userId: string; maxProfiles: number },
  ) {
    if (!this.internalKey || !key || !safeEqual(key, this.internalKey)) {
      throw new UnauthorizedException("Invalid internal key");
    }

    // The userId is interpolated into a destructive S3 delete prefix
    // (users/{userId}/profiles/), so constrain it to a plain id — no empty
    // value, no slashes/dots that could widen or redirect the prefix.
    const userId = body?.userId;
    if (typeof userId !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(userId)) {
      throw new BadRequestException("Invalid userId");
    }
    const maxProfiles = body?.maxProfiles;
    if (!Number.isInteger(maxProfiles) || maxProfiles < 0) {
      throw new BadRequestException("Invalid maxProfiles");
    }

    return this.syncService.cleanupExcessProfiles(userId, maxProfiles);
  }
}

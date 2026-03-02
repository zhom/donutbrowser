import {
  Body,
  Controller,
  Headers,
  HttpCode,
  Post,
  UnauthorizedException,
} from "@nestjs/common";
import { ConfigService } from "@nestjs/config";
import { SyncService } from "./sync.service.js";

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
    if (!this.internalKey || key !== this.internalKey) {
      throw new UnauthorizedException("Invalid internal key");
    }

    return this.syncService.cleanupExcessProfiles(
      body.userId,
      body.maxProfiles,
    );
  }
}

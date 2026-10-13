import { Controller, Get, HttpException, HttpStatus } from "@nestjs/common";
import { ConfigService } from "@nestjs/config";
import { AppService } from "./app.service.js";
import { normalizeSyncServerUrl } from "./auth/donut-accounts.js";
import { SyncService } from "./sync/sync.service.js";

@Controller()
export class AppController {
  constructor(
    private readonly appService: AppService,
    private readonly syncService: SyncService,
    private readonly configService: ConfigService,
  ) {}

  @Get()
  getHello(): string {
    return this.appService.getHello();
  }

  @Get("health")
  getHealth(): { status: string } {
    return { status: "ok" };
  }

  // `storageEndpoint` is the host clients are handed in presigned URLs. The
  // server cannot tell whether a client can reach it, so report it and let
  // whoever is debugging a failing sync compare it against their network.
  // Self-hosted only — see getDiagnosticStorageEndpoint.
  //
  // `accountUrl` is the address Donut accounts sign in to this server with
  // (SYNC_PUBLIC_URL). The app checks it against the address it was given,
  // because a token is bound to one exact address.
  @Get("readyz")
  async getReadiness(): Promise<{
    status: string;
    s3: boolean;
    storageEndpoint?: string;
    accountUrl?: string;
  }> {
    const s3Ready = await this.syncService.checkS3Connectivity();
    const storageEndpoint = this.syncService.getDiagnosticStorageEndpoint();
    const accountUrl = normalizeSyncServerUrl(
      this.configService.get<string>("SYNC_PUBLIC_URL"),
    );
    const diagnostic = {
      ...(storageEndpoint ? { storageEndpoint } : {}),
      ...(accountUrl ? { accountUrl } : {}),
    };
    if (!s3Ready) {
      throw new HttpException(
        { status: "not ready", s3: false, ...diagnostic },
        HttpStatus.SERVICE_UNAVAILABLE,
      );
    }
    return { status: "ready", s3: true, ...diagnostic };
  }
}

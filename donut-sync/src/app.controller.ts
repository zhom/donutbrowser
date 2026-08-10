import { Controller, Get, HttpException, HttpStatus } from "@nestjs/common";
import { AppService } from "./app.service.js";
import { SyncService } from "./sync/sync.service.js";

@Controller()
export class AppController {
  constructor(
    private readonly appService: AppService,
    private readonly syncService: SyncService,
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
  @Get("readyz")
  async getReadiness(): Promise<{
    status: string;
    s3: boolean;
    storageEndpoint?: string;
  }> {
    const s3Ready = await this.syncService.checkS3Connectivity();
    const storageEndpoint = this.syncService.getDiagnosticStorageEndpoint();
    const diagnostic = storageEndpoint ? { storageEndpoint } : {};
    if (!s3Ready) {
      throw new HttpException(
        { status: "not ready", s3: false, ...diagnostic },
        HttpStatus.SERVICE_UNAVAILABLE,
      );
    }
    return { status: "ready", s3: true, ...diagnostic };
  }
}

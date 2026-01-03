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

  @Get("readyz")
  async getReadiness(): Promise<{ status: string; s3: boolean }> {
    const s3Ready = await this.syncService.checkS3Connectivity();
    if (!s3Ready) {
      throw new HttpException(
        { status: "not ready", s3: false },
        HttpStatus.SERVICE_UNAVAILABLE,
      );
    }
    return { status: "ready", s3: true };
  }
}

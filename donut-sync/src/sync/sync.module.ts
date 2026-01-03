import { Module } from "@nestjs/common";
import { AuthGuard } from "../auth/auth.guard.js";
import { SyncController } from "./sync.controller.js";
import { SyncService } from "./sync.service.js";

@Module({
  controllers: [SyncController],
  providers: [SyncService, AuthGuard],
  exports: [SyncService],
})
export class SyncModule {}

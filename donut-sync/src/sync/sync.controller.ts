import {
  Body,
  Controller,
  Get,
  type MessageEvent,
  Post,
  Sse,
  UseGuards,
} from "@nestjs/common";
import { map, type Observable } from "rxjs";
import { AuthGuard } from "../auth/auth.guard.js";
import type {
  DeletePrefixRequestDto,
  DeletePrefixResponseDto,
  DeleteRequestDto,
  DeleteResponseDto,
  ListRequestDto,
  ListResponseDto,
  PresignDownloadBatchRequestDto,
  PresignDownloadBatchResponseDto,
  PresignDownloadRequestDto,
  PresignDownloadResponseDto,
  PresignUploadBatchRequestDto,
  PresignUploadBatchResponseDto,
  PresignUploadRequestDto,
  PresignUploadResponseDto,
  StatRequestDto,
  StatResponseDto,
} from "./dto/sync.dto.js";
import { SyncService } from "./sync.service.js";

@Controller("v1/objects")
@UseGuards(AuthGuard)
export class SyncController {
  constructor(private readonly syncService: SyncService) {}

  @Post("stat")
  async stat(@Body() dto: StatRequestDto): Promise<StatResponseDto> {
    return this.syncService.stat(dto);
  }

  @Post("presign-upload")
  async presignUpload(
    @Body() dto: PresignUploadRequestDto,
  ): Promise<PresignUploadResponseDto> {
    return this.syncService.presignUpload(dto);
  }

  @Post("presign-download")
  async presignDownload(
    @Body() dto: PresignDownloadRequestDto,
  ): Promise<PresignDownloadResponseDto> {
    return this.syncService.presignDownload(dto);
  }

  @Post("delete")
  async delete(@Body() dto: DeleteRequestDto): Promise<DeleteResponseDto> {
    return this.syncService.delete(dto);
  }

  @Post("list")
  async list(@Body() dto: ListRequestDto): Promise<ListResponseDto> {
    return this.syncService.list(dto);
  }

  @Post("presign-upload-batch")
  async presignUploadBatch(
    @Body() dto: PresignUploadBatchRequestDto,
  ): Promise<PresignUploadBatchResponseDto> {
    return this.syncService.presignUploadBatch(dto);
  }

  @Post("presign-download-batch")
  async presignDownloadBatch(
    @Body() dto: PresignDownloadBatchRequestDto,
  ): Promise<PresignDownloadBatchResponseDto> {
    return this.syncService.presignDownloadBatch(dto);
  }

  @Post("delete-prefix")
  async deletePrefix(
    @Body() dto: DeletePrefixRequestDto,
  ): Promise<DeletePrefixResponseDto> {
    return this.syncService.deletePrefix(dto);
  }

  @Get("subscribe")
  @Sse()
  subscribe(): Observable<MessageEvent> {
    return this.syncService.subscribe(2000).pipe(
      map((event) => ({
        data: event,
      })),
    );
  }
}

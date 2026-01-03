import {
  CreateBucketCommand,
  DeleteObjectCommand,
  DeleteObjectsCommand,
  GetObjectCommand,
  HeadBucketCommand,
  HeadObjectCommand,
  ListObjectsV2Command,
  PutObjectCommand as PutCmd,
  PutObjectCommand,
  S3Client,
} from "@aws-sdk/client-s3";
import { getSignedUrl } from "@aws-sdk/s3-request-presigner";
import { Injectable, type OnModuleInit } from "@nestjs/common";
import { ConfigService } from "@nestjs/config";
import { interval, merge, type Observable, of, Subject } from "rxjs";
import { catchError, filter, map, startWith, switchMap } from "rxjs/operators";
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
  SubscribeEventDto,
} from "./dto/sync.dto.js";

@Injectable()
export class SyncService implements OnModuleInit {
  private s3Client: S3Client;
  private bucket: string;
  private lastKnownState: Map<string, string> = new Map();
  private changeSubject = new Subject<SubscribeEventDto>();
  private s3Ready = false;

  constructor(private configService: ConfigService) {
    const endpoint =
      this.configService.get<string>("S3_ENDPOINT") || "http://localhost:8987";
    const region = this.configService.get<string>("S3_REGION") || "us-east-1";
    const accessKeyId =
      this.configService.get<string>("S3_ACCESS_KEY_ID") || "minioadmin";
    const secretAccessKey =
      this.configService.get<string>("S3_SECRET_ACCESS_KEY") || "minioadmin";
    const forcePathStyle =
      this.configService.get<string>("S3_FORCE_PATH_STYLE") !== "false";

    this.bucket = this.configService.get<string>("S3_BUCKET") || "donut-sync";

    this.s3Client = new S3Client({
      endpoint,
      region,
      credentials: {
        accessKeyId,
        secretAccessKey,
      },
      forcePathStyle,
    });
  }

  async onModuleInit() {
    await this.ensureBucketExists();
  }

  private async ensureBucketExists(): Promise<void> {
    try {
      await this.s3Client.send(new HeadBucketCommand({ Bucket: this.bucket }));
      this.s3Ready = true;
    } catch (error: unknown) {
      const isNotFound =
        error &&
        typeof error === "object" &&
        "name" in error &&
        (error.name === "NotFound" ||
          error.name === "NoSuchBucket" ||
          error.name === "404");

      if (isNotFound) {
        try {
          await this.s3Client.send(
            new CreateBucketCommand({ Bucket: this.bucket }),
          );
          this.s3Ready = true;
        } catch (createError) {
          console.error("Failed to create S3 bucket:", createError);
          throw createError;
        }
      } else {
        console.error("S3 connection failed:", error);
        throw error;
      }
    }
  }

  isReady(): boolean {
    return this.s3Ready;
  }

  async checkS3Connectivity(): Promise<boolean> {
    try {
      await this.s3Client.send(new HeadBucketCommand({ Bucket: this.bucket }));
      return true;
    } catch {
      return false;
    }
  }

  async stat(dto: StatRequestDto): Promise<StatResponseDto> {
    try {
      const response = await this.s3Client.send(
        new HeadObjectCommand({
          Bucket: this.bucket,
          Key: dto.key,
        }),
      );

      return {
        exists: true,
        lastModified: response.LastModified?.toISOString(),
        size: response.ContentLength,
      };
    } catch (error: unknown) {
      if (
        error &&
        typeof error === "object" &&
        "name" in error &&
        error.name === "NotFound"
      ) {
        return { exists: false };
      }
      throw error;
    }
  }

  async presignUpload(
    dto: PresignUploadRequestDto,
  ): Promise<PresignUploadResponseDto> {
    const expiresIn = dto.expiresIn || 3600;
    const expiresAt = new Date(Date.now() + expiresIn * 1000);

    const command = new PutCmd({
      Bucket: this.bucket,
      Key: dto.key,
      ContentType: dto.contentType || "application/octet-stream",
    });

    const url = await getSignedUrl(this.s3Client, command, { expiresIn });

    return {
      url,
      expiresAt: expiresAt.toISOString(),
    };
  }

  async presignDownload(
    dto: PresignDownloadRequestDto,
  ): Promise<PresignDownloadResponseDto> {
    const expiresIn = dto.expiresIn || 3600;
    const expiresAt = new Date(Date.now() + expiresIn * 1000);

    const command = new GetObjectCommand({
      Bucket: this.bucket,
      Key: dto.key,
    });

    const url = await getSignedUrl(this.s3Client, command, { expiresIn });

    return {
      url,
      expiresAt: expiresAt.toISOString(),
    };
  }

  async delete(dto: DeleteRequestDto): Promise<DeleteResponseDto> {
    let deleted = false;
    let tombstoneCreated = false;

    try {
      await this.s3Client.send(
        new DeleteObjectCommand({
          Bucket: this.bucket,
          Key: dto.key,
        }),
      );
      deleted = true;
    } catch {
      deleted = false;
    }

    if (dto.tombstoneKey) {
      const tombstoneData = JSON.stringify({
        id: dto.key,
        deleted_at: dto.deletedAt || new Date().toISOString(),
      });

      await this.s3Client.send(
        new PutObjectCommand({
          Bucket: this.bucket,
          Key: dto.tombstoneKey,
          Body: tombstoneData,
          ContentType: "application/json",
        }),
      );
      tombstoneCreated = true;
    }

    return { deleted, tombstoneCreated };
  }

  async list(dto: ListRequestDto): Promise<ListResponseDto> {
    const response = await this.s3Client.send(
      new ListObjectsV2Command({
        Bucket: this.bucket,
        Prefix: dto.prefix,
        MaxKeys: dto.maxKeys || 1000,
        ContinuationToken: dto.continuationToken,
      }),
    );

    const objects = (response.Contents || []).map((obj) => ({
      key: obj.Key || "",
      lastModified: obj.LastModified?.toISOString() || "",
      size: obj.Size || 0,
    }));

    return {
      objects,
      isTruncated: response.IsTruncated || false,
      nextContinuationToken: response.NextContinuationToken,
    };
  }

  async presignUploadBatch(
    dto: PresignUploadBatchRequestDto,
  ): Promise<PresignUploadBatchResponseDto> {
    const expiresIn = dto.expiresIn || 3600;
    const expiresAt = new Date(Date.now() + expiresIn * 1000);

    const items = await Promise.all(
      dto.items.map(async (item) => {
        const command = new PutCmd({
          Bucket: this.bucket,
          Key: item.key,
          ContentType: item.contentType || "application/octet-stream",
        });

        const url = await getSignedUrl(this.s3Client, command, { expiresIn });

        return {
          key: item.key,
          url,
          expiresAt: expiresAt.toISOString(),
        };
      }),
    );

    return { items };
  }

  async presignDownloadBatch(
    dto: PresignDownloadBatchRequestDto,
  ): Promise<PresignDownloadBatchResponseDto> {
    const expiresIn = dto.expiresIn || 3600;
    const expiresAt = new Date(Date.now() + expiresIn * 1000);

    const items = await Promise.all(
      dto.keys.map(async (key) => {
        const command = new GetObjectCommand({
          Bucket: this.bucket,
          Key: key,
        });

        const url = await getSignedUrl(this.s3Client, command, { expiresIn });

        return {
          key,
          url,
          expiresAt: expiresAt.toISOString(),
        };
      }),
    );

    return { items };
  }

  async deletePrefix(
    dto: DeletePrefixRequestDto,
  ): Promise<DeletePrefixResponseDto> {
    let deletedCount = 0;
    let tombstoneCreated = false;
    let continuationToken: string | undefined;

    // Paginate through all objects with the prefix
    do {
      const listResponse = await this.s3Client.send(
        new ListObjectsV2Command({
          Bucket: this.bucket,
          Prefix: dto.prefix,
          MaxKeys: 1000,
          ContinuationToken: continuationToken,
        }),
      );

      const objects = listResponse.Contents || [];
      if (objects.length > 0) {
        // Delete objects in batches of 1000 (S3 limit)
        const deleteObjects = objects
          .filter((obj): obj is typeof obj & { Key: string } => !!obj.Key)
          .map((obj) => ({ Key: obj.Key }));

        if (deleteObjects.length > 0) {
          await this.s3Client.send(
            new DeleteObjectsCommand({
              Bucket: this.bucket,
              Delete: {
                Objects: deleteObjects,
                Quiet: true,
              },
            }),
          );
          deletedCount += deleteObjects.length;
        }
      }

      continuationToken = listResponse.NextContinuationToken;
    } while (continuationToken);

    // Create tombstone if requested
    if (dto.tombstoneKey && deletedCount > 0) {
      const tombstoneData = JSON.stringify({
        prefix: dto.prefix,
        deleted_at: dto.deletedAt || new Date().toISOString(),
        deleted_count: deletedCount,
      });

      await this.s3Client.send(
        new PutObjectCommand({
          Bucket: this.bucket,
          Key: dto.tombstoneKey,
          Body: tombstoneData,
          ContentType: "application/json",
        }),
      );
      tombstoneCreated = true;
    }

    return { deletedCount, tombstoneCreated };
  }

  subscribe(pollIntervalMs = 2000): Observable<SubscribeEventDto> {
    const prefixes = ["profiles/", "proxies/", "groups/", "tombstones/"];

    const pollChanges$ = interval(pollIntervalMs).pipe(
      startWith(0),
      switchMap(async () => {
        const events: SubscribeEventDto[] = [];
        const currentState = new Map<string, string>();

        for (const prefix of prefixes) {
          try {
            const result = await this.list({ prefix, maxKeys: 1000 });
            for (const obj of result.objects) {
              const stateKey = `${obj.key}:${obj.lastModified}`;
              currentState.set(obj.key, stateKey);

              const previousStateKey = this.lastKnownState.get(obj.key);
              if (previousStateKey !== stateKey) {
                events.push({
                  type: "change",
                  key: obj.key,
                  lastModified: obj.lastModified,
                  size: obj.size,
                });
              }
            }
          } catch (error) {
            console.error(`Failed to list prefix ${prefix}:`, error);
          }
        }

        for (const [key] of this.lastKnownState) {
          if (!currentState.has(key)) {
            events.push({
              type: "delete",
              key,
            });
          }
        }

        this.lastKnownState = currentState;
        return events;
      }),
      switchMap((events) => of(...events)),
      filter((event): event is SubscribeEventDto => event !== null),
      catchError((error) => {
        console.error("Error in subscribe poll:", error);
        return of({ type: "ping" as const });
      }),
    );

    const ping$ = interval(30000).pipe(map(() => ({ type: "ping" as const })));

    return merge(pollChanges$, ping$, this.changeSubject.asObservable());
  }

  emitChange(event: SubscribeEventDto) {
    this.changeSubject.next(event);
  }
}

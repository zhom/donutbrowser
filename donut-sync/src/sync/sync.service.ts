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
import {
  ForbiddenException,
  Injectable,
  Logger,
  type OnModuleInit,
} from "@nestjs/common";
import { ConfigService } from "@nestjs/config";
import { interval, merge, type Observable, of, Subject } from "rxjs";
import { catchError, filter, map, startWith, switchMap } from "rxjs/operators";
import type { UserContext } from "../auth/user-context.interface.js";
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
  private readonly logger = new Logger(SyncService.name);
  private s3Client: S3Client;
  private bucket: string;
  private changeSubject = new Subject<SubscribeEventDto>();
  private s3Ready = false;
  private backendInternalUrl: string | undefined;
  private backendInternalKey: string | undefined;

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

    this.backendInternalUrl = this.configService.get<string>(
      "BACKEND_INTERNAL_URL",
    );
    this.backendInternalKey = this.configService.get<string>(
      "BACKEND_INTERNAL_KEY",
    );
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
        } catch (createError: unknown) {
          // BucketAlreadyOwnedByYou means the bucket exists and we own it - this is fine
          const isAlreadyOwned =
            createError &&
            typeof createError === "object" &&
            "name" in createError &&
            createError.name === "BucketAlreadyOwnedByYou";
          if (isAlreadyOwned) {
            this.s3Ready = true;
          } else {
            console.error("Failed to create S3 bucket:", createError);
            throw createError;
          }
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

  /**
   * Scope a key to the user's prefix for cloud mode.
   * Self-hosted mode passes through unchanged.
   */
  private scopeKey(ctx: UserContext, key: string): string {
    if (ctx.mode === "self-hosted") return key;
    return `${ctx.prefix}${key}`;
  }

  /**
   * Validate that a key is accessible by the user.
   * For cloud mode, key must start with user's prefix or team prefix.
   */
  private validateKeyAccess(ctx: UserContext, key: string): void {
    if (ctx.mode === "self-hosted") return;

    if (key.startsWith(ctx.prefix)) return;
    if (ctx.teamPrefix && key.startsWith(ctx.teamPrefix)) return;

    throw new ForbiddenException("Access denied to this key");
  }

  async stat(dto: StatRequestDto, ctx: UserContext): Promise<StatResponseDto> {
    const key = this.scopeKey(ctx, dto.key);
    this.validateKeyAccess(ctx, key);

    try {
      const response = await this.s3Client.send(
        new HeadObjectCommand({
          Bucket: this.bucket,
          Key: key,
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
    ctx: UserContext,
  ): Promise<PresignUploadResponseDto> {
    const key = this.scopeKey(ctx, dto.key);
    this.validateKeyAccess(ctx, key);

    // Check profile limit for cloud users
    if (ctx.mode === "cloud" && ctx.profileLimit > 0) {
      await this.checkProfileLimit(ctx);
    }

    const expiresIn = dto.expiresIn || 3600;
    const expiresAt = new Date(Date.now() + expiresIn * 1000);

    const command = new PutCmd({
      Bucket: this.bucket,
      Key: key,
      ContentType: dto.contentType || "application/octet-stream",
    });

    const url = await getSignedUrl(this.s3Client, command, { expiresIn });

    // Report profile usage after upload presign if key is under profiles/
    if (ctx.mode === "cloud" && dto.key.startsWith("profiles/")) {
      this.reportProfileUsageAsync(ctx);
    }

    return {
      url,
      expiresAt: expiresAt.toISOString(),
    };
  }

  async presignDownload(
    dto: PresignDownloadRequestDto,
    ctx: UserContext,
  ): Promise<PresignDownloadResponseDto> {
    const key = this.scopeKey(ctx, dto.key);
    this.validateKeyAccess(ctx, key);

    const expiresIn = dto.expiresIn || 3600;
    const expiresAt = new Date(Date.now() + expiresIn * 1000);

    const command = new GetObjectCommand({
      Bucket: this.bucket,
      Key: key,
    });

    const url = await getSignedUrl(this.s3Client, command, { expiresIn });

    return {
      url,
      expiresAt: expiresAt.toISOString(),
    };
  }

  async delete(
    dto: DeleteRequestDto,
    ctx: UserContext,
  ): Promise<DeleteResponseDto> {
    const key = this.scopeKey(ctx, dto.key);
    this.validateKeyAccess(ctx, key);

    let deleted = false;
    let tombstoneCreated = false;

    try {
      await this.s3Client.send(
        new DeleteObjectCommand({
          Bucket: this.bucket,
          Key: key,
        }),
      );
      deleted = true;
    } catch {
      deleted = false;
    }

    if (dto.tombstoneKey) {
      const scopedTombstoneKey = this.scopeKey(ctx, dto.tombstoneKey);
      const tombstoneData = JSON.stringify({
        id: key,
        deleted_at: dto.deletedAt || new Date().toISOString(),
      });

      await this.s3Client.send(
        new PutObjectCommand({
          Bucket: this.bucket,
          Key: scopedTombstoneKey,
          Body: tombstoneData,
          ContentType: "application/json",
        }),
      );
      tombstoneCreated = true;
    }

    // Report profile usage after delete if key is under profiles/
    if (ctx.mode === "cloud" && dto.key.startsWith("profiles/")) {
      this.reportProfileUsageAsync(ctx);
    }

    return { deleted, tombstoneCreated };
  }

  async list(dto: ListRequestDto, ctx?: UserContext): Promise<ListResponseDto> {
    const prefix = ctx ? this.scopeKey(ctx, dto.prefix) : dto.prefix;

    const response = await this.s3Client.send(
      new ListObjectsV2Command({
        Bucket: this.bucket,
        Prefix: prefix,
        MaxKeys: dto.maxKeys || 1000,
        ContinuationToken: dto.continuationToken,
      }),
    );

    const userPrefix = ctx?.prefix || "";
    const objects = (response.Contents || []).map((obj) => {
      // Strip user prefix from returned keys so client sees relative keys
      let key = obj.Key || "";
      if (userPrefix && key.startsWith(userPrefix)) {
        key = key.substring(userPrefix.length);
      }
      return {
        key,
        lastModified: obj.LastModified?.toISOString() || "",
        size: obj.Size || 0,
      };
    });

    return {
      objects,
      isTruncated: response.IsTruncated || false,
      nextContinuationToken: response.NextContinuationToken,
    };
  }

  async presignUploadBatch(
    dto: PresignUploadBatchRequestDto,
    ctx: UserContext,
  ): Promise<PresignUploadBatchResponseDto> {
    // Check profile limit for cloud users
    if (ctx.mode === "cloud" && ctx.profileLimit > 0) {
      await this.checkProfileLimit(ctx);
    }

    const expiresIn = dto.expiresIn || 3600;
    const expiresAt = new Date(Date.now() + expiresIn * 1000);

    const items = await Promise.all(
      dto.items.map(async (item) => {
        const key = this.scopeKey(ctx, item.key);
        this.validateKeyAccess(ctx, key);

        const command = new PutCmd({
          Bucket: this.bucket,
          Key: key,
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

    // Report profile usage if any key is under profiles/
    if (
      ctx.mode === "cloud" &&
      dto.items.some((item) => item.key.startsWith("profiles/"))
    ) {
      this.reportProfileUsageAsync(ctx);
    }

    return { items };
  }

  async presignDownloadBatch(
    dto: PresignDownloadBatchRequestDto,
    ctx: UserContext,
  ): Promise<PresignDownloadBatchResponseDto> {
    const expiresIn = dto.expiresIn || 3600;
    const expiresAt = new Date(Date.now() + expiresIn * 1000);

    const items = await Promise.all(
      dto.keys.map(async (rawKey) => {
        const key = this.scopeKey(ctx, rawKey);
        this.validateKeyAccess(ctx, key);

        const command = new GetObjectCommand({
          Bucket: this.bucket,
          Key: key,
        });

        const url = await getSignedUrl(this.s3Client, command, { expiresIn });

        return {
          key: rawKey,
          url,
          expiresAt: expiresAt.toISOString(),
        };
      }),
    );

    return { items };
  }

  async deletePrefix(
    dto: DeletePrefixRequestDto,
    ctx: UserContext,
  ): Promise<DeletePrefixResponseDto> {
    const prefix = this.scopeKey(ctx, dto.prefix);
    let deletedCount = 0;
    let tombstoneCreated = false;
    let continuationToken: string | undefined;

    // Paginate through all objects with the prefix
    do {
      const listResponse = await this.s3Client.send(
        new ListObjectsV2Command({
          Bucket: this.bucket,
          Prefix: prefix,
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
      const scopedTombstoneKey = this.scopeKey(ctx, dto.tombstoneKey);
      const tombstoneData = JSON.stringify({
        prefix: dto.prefix,
        deleted_at: dto.deletedAt || new Date().toISOString(),
        deleted_count: deletedCount,
      });

      await this.s3Client.send(
        new PutObjectCommand({
          Bucket: this.bucket,
          Key: scopedTombstoneKey,
          Body: tombstoneData,
          ContentType: "application/json",
        }),
      );
      tombstoneCreated = true;
    }

    // Report profile usage after prefix delete if prefix is under profiles/
    if (ctx.mode === "cloud" && dto.prefix.startsWith("profiles/")) {
      this.reportProfileUsageAsync(ctx);
    }

    return { deletedCount, tombstoneCreated };
  }

  subscribe(
    ctx: UserContext,
    pollIntervalMs = 2000,
  ): Observable<SubscribeEventDto> {
    const basePrefixes = ["profiles/", "proxies/", "groups/", "tombstones/"];

    // Scope prefixes for cloud users; self-hosted gets root prefixes
    const prefixes =
      ctx.mode === "self-hosted"
        ? basePrefixes
        : basePrefixes.map((p) => `${ctx.prefix}${p}`);

    // Per-connection state (not shared across subscribers)
    let lastKnownState = new Map<string, string>();

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

              const previousStateKey = lastKnownState.get(obj.key);
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

        for (const [key] of lastKnownState) {
          if (!currentState.has(key)) {
            events.push({
              type: "delete",
              key,
            });
          }
        }

        lastKnownState = currentState;
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

  /**
   * Check if the user has reached their profile limit.
   * Counts objects in the profiles/ prefix.
   */
  private async checkProfileLimit(ctx: UserContext): Promise<void> {
    if (ctx.profileLimit <= 0) return; // 0 = unlimited

    const profilePrefix = `${ctx.prefix}profiles/`;
    const result = await this.s3Client.send(
      new ListObjectsV2Command({
        Bucket: this.bucket,
        Prefix: profilePrefix,
        MaxKeys: ctx.profileLimit + 1,
      }),
    );

    const count = result.Contents?.length || 0;
    if (count >= ctx.profileLimit) {
      throw new ForbiddenException(
        `Profile limit reached (${ctx.profileLimit}). Upgrade your plan for more profiles.`,
      );
    }
  }

  /**
   * Count the number of profile objects for a user.
   */
  private async countProfiles(ctx: UserContext): Promise<number> {
    const profilePrefix = `${ctx.prefix}profiles/`;
    let count = 0;
    let continuationToken: string | undefined;

    do {
      const result = await this.s3Client.send(
        new ListObjectsV2Command({
          Bucket: this.bucket,
          Prefix: profilePrefix,
          MaxKeys: 1000,
          ContinuationToken: continuationToken,
        }),
      );
      count += result.Contents?.length || 0;
      continuationToken = result.NextContinuationToken;
    } while (continuationToken);

    return count;
  }

  /**
   * Extract user ID from context prefix (e.g. "users/abc-123/" → "abc-123").
   */
  private extractUserId(ctx: UserContext): string | null {
    const match = ctx.prefix.match(/^users\/([^/]+)\/$/);
    return match ? match[1] : null;
  }

  /**
   * Fire-and-forget: count profiles and report to backend.
   */
  private reportProfileUsageAsync(ctx: UserContext): void {
    if (!this.backendInternalUrl || !this.backendInternalKey) return;

    const userId = this.extractUserId(ctx);
    if (!userId) return;

    this.countProfiles(ctx)
      .then((count) => this.reportProfileUsage(userId, count))
      .catch((err) =>
        this.logger.warn(`Failed to report profile usage: ${err.message}`),
      );
  }

  private async reportProfileUsage(
    userId: string,
    count: number,
  ): Promise<void> {
    const url = `${this.backendInternalUrl}/api/auth/internal/profile-usage`;
    const response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "x-internal-key": this.backendInternalKey ?? "undefined",
      },
      body: JSON.stringify({ userId, count }),
    });

    if (!response.ok) {
      this.logger.warn(
        `Profile usage report failed: ${response.status} ${response.statusText}`,
      );
    }
  }
}

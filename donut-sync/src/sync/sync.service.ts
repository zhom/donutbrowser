import { randomUUID } from "node:crypto";
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

/**
 * Marker object written under each scope (user / team / self-hosted root).
 * Subscribers HEAD this object on each poll and only LIST when its ETag has
 * changed, which keeps the steady-state polling cost down to one Class-B
 * HeadObject per scope per poll instead of N Class-A ListObjectsV2 calls.
 *
 * Filename starts with a dot so it sorts first and is unmistakably internal
 * to donut-sync; client `list()` calls strip it from results so it never
 * leaks into application data.
 */
const MANIFEST_KEY = ".donut-sync-manifest";

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
    if (ctx.teamPrefix && key.startsWith(ctx.teamPrefix)) return key;
    return `${ctx.prefix}${key}`;
  }

  /**
   * Return every scope prefix the given user can write to. For self-hosted
   * that's the bucket root (`""`); for cloud that's the user prefix plus an
   * optional team prefix.
   */
  private scopesFor(ctx: UserContext): string[] {
    if (ctx.mode === "self-hosted") return [""];
    const out = [ctx.prefix];
    if (ctx.teamPrefix) out.push(ctx.teamPrefix);
    return out;
  }

  /**
   * Bump the manifest object for the scope that owns `scopedKey`. Writers call
   * this fire-and-forget after any successful mutation so subscribers'
   * cheap HEAD polls observe an ETag change and pull a fresh listing.
   *
   * Slightly over-eager by design: we bump on presign-issue (rather than on
   * the actual S3 PUT), so a never-completed upload causes one wasted refresh
   * on other devices. That's strictly cheaper than verifying every upload.
   */
  private async bumpManifest(
    ctx: UserContext,
    scopedKey: string,
  ): Promise<void> {
    const scope = this.scopeForKey(ctx, scopedKey);
    if (scope === null) return;
    const key = `${scope}${MANIFEST_KEY}`;
    // Body just needs to be unique so the ETag changes; clients never read it.
    const body = JSON.stringify({
      updatedAt: new Date().toISOString(),
      nonce: randomUUID(),
    });
    try {
      await this.s3Client.send(
        new PutObjectCommand({
          Bucket: this.bucket,
          Key: key,
          Body: body,
          ContentType: "application/json",
        }),
      );
    } catch (err) {
      // Manifest bump failures must NEVER fail the user's request.
      // Subscribers fall back to detecting changes on their next listing.
      this.logger.warn(
        `Manifest bump failed for ${key}: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  }

  /**
   * Resolve which scope owns a fully-scoped key. Returns null if the key
   * doesn't belong to a known scope (which shouldn't happen in practice
   * because validateKeyAccess gates the write paths).
   */
  private scopeForKey(ctx: UserContext, scopedKey: string): string | null {
    if (ctx.mode === "self-hosted") return "";
    if (ctx.teamPrefix && scopedKey.startsWith(ctx.teamPrefix)) {
      return ctx.teamPrefix;
    }
    if (scopedKey.startsWith(ctx.prefix)) return ctx.prefix;
    return null;
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
        // S3 returns user metadata with lowercased keys and no `x-amz-meta-`
        // prefix. Clients read `updated-at` from here to resolve sync conflicts
        // without downloading the object body.
        metadata: response.Metadata,
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
      // Signed into the presigned URL as `x-amz-meta-*`. The client must send
      // exactly these headers on the PUT, so we echo them in the response.
      Metadata: dto.metadata,
    });

    const url = await getSignedUrl(this.s3Client, command, { expiresIn });

    // Report profile usage after upload presign if key is under profiles/
    if (ctx.mode === "cloud" && dto.key.startsWith("profiles/")) {
      this.reportProfileUsageAsync(ctx);
    }

    // Notify subscribers via the per-scope manifest. Fire-and-forget; a
    // failure here just means other devices pick up the change on their
    // next full listing instead of immediately.
    void this.bumpManifest(ctx, key);

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

    if (deleted || tombstoneCreated) {
      void this.bumpManifest(ctx, key);
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
    const teamPrefix = ctx?.teamPrefix || "";
    const objects = (response.Contents || [])
      // Don't leak donut-sync's internal manifest object to clients.
      .filter((obj) => !(obj.Key || "").endsWith(MANIFEST_KEY))
      .map((obj) => {
        let key = obj.Key || "";
        if (teamPrefix && key.startsWith(teamPrefix)) {
          key = key.substring(teamPrefix.length);
        } else if (userPrefix && key.startsWith(userPrefix)) {
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

    // One bump per scope touched by this batch (usually one).
    if (items.length > 0) {
      const scopesSeen = new Set<string>();
      for (const item of dto.items) {
        const key = this.scopeKey(ctx, item.key);
        const scope = this.scopeForKey(ctx, key);
        if (scope !== null && !scopesSeen.has(scope)) {
          scopesSeen.add(scope);
          // Use any key from the scope; bumpManifest only inspects scope.
          void this.bumpManifest(ctx, key);
        }
      }
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

    if (deletedCount > 0 || tombstoneCreated) {
      void this.bumpManifest(ctx, prefix);
    }

    return { deletedCount, tombstoneCreated };
  }

  /**
   * Long-lived per-client poll loop.
   *
   * Steady-state cost is one HEAD per scope per poll (Class B on R2). A LIST
   * (Class A) is only issued when:
   *   1. it's the client's first poll (need to seed the state map), or
   *   2. a write touched the scope and bumped its manifest ETag.
   *
   * This is *eventual* cross-device sync, gated by the poll interval.
   * Real-time push is intentionally not provided here — that lives in the
   * paid backend.
   */
  subscribe(
    ctx: UserContext,
    pollIntervalMs = 5000,
  ): Observable<SubscribeEventDto> {
    const basePrefixes = ["profiles/", "proxies/", "groups/", "tombstones/"];
    const scopes = this.scopesFor(ctx);

    // Per-connection state (not shared across subscribers).
    const lastManifestEtag = new Map<string, string | undefined>();
    let lastKnownState = new Map<string, string>();
    let initialized = false;

    const pollChanges$ = interval(pollIntervalMs).pipe(
      startWith(0),
      switchMap(async () => {
        const events: SubscribeEventDto[] = [];

        // Phase 1 — cheap HEAD on each scope's manifest. This is the
        // steady-state cost (Class B). If no manifest changed since the
        // last poll, we don't touch S3 again this tick.
        let anyScopeChanged = false;
        for (const scope of scopes) {
          const manifestKey = `${scope}${MANIFEST_KEY}`;
          let currentEtag: string | undefined;
          try {
            const head = await this.s3Client.send(
              new HeadObjectCommand({
                Bucket: this.bucket,
                Key: manifestKey,
              }),
            );
            currentEtag = head.ETag;
          } catch (err: unknown) {
            const status =
              err && typeof err === "object" && "$metadata" in err
                ? (err as { $metadata?: { httpStatusCode?: number } }).$metadata
                    ?.httpStatusCode
                : undefined;
            const name =
              err && typeof err === "object" && "name" in err
                ? (err as { name?: string }).name
                : undefined;
            if (name === "NotFound" || name === "NoSuchKey" || status === 404) {
              // No manifest yet — treat as "no changes" (undefined ETag).
              currentEtag = undefined;
            } else {
              this.logger.error(
                `Manifest HEAD failed for ${manifestKey}: ${err instanceof Error ? err.message : String(err)}`,
              );
              continue;
            }
          }

          const previousEtag = lastManifestEtag.get(scope);
          if (previousEtag !== currentEtag) {
            anyScopeChanged = true;
          }
          lastManifestEtag.set(scope, currentEtag);
        }

        // After the first poll, only run the LIST when something actually
        // changed in at least one scope.
        if (initialized && !anyScopeChanged) {
          return [];
        }

        // Phase 2 — one LIST per scope (not per base prefix). Filter to the
        // four base prefixes client-side. This is the cost we pay only when
        // a manifest told us there's something new to look at.
        const currentState = new Map<string, string>();
        for (const scope of scopes) {
          let continuationToken: string | undefined;
          do {
            try {
              const result = await this.s3Client.send(
                new ListObjectsV2Command({
                  Bucket: this.bucket,
                  Prefix: scope,
                  MaxKeys: 1000,
                  ContinuationToken: continuationToken,
                }),
              );

              for (const obj of result.Contents || []) {
                const fullKey = obj.Key;
                if (!fullKey) continue;
                const relativeKey = fullKey.startsWith(scope)
                  ? fullKey.substring(scope.length)
                  : fullKey;
                // Skip the manifest object itself + anything outside the
                // four data prefixes.
                if (relativeKey === MANIFEST_KEY) continue;
                if (!basePrefixes.some((bp) => relativeKey.startsWith(bp))) {
                  continue;
                }

                const lastModified = obj.LastModified?.toISOString() || "";
                const stateKey = `${relativeKey}:${lastModified}`;
                currentState.set(relativeKey, stateKey);

                const previousStateKey = lastKnownState.get(relativeKey);
                if (previousStateKey !== stateKey) {
                  events.push({
                    type: "change",
                    key: relativeKey,
                    lastModified,
                    size: obj.Size || 0,
                  });
                }
              }
              continuationToken = result.NextContinuationToken;
            } catch (err) {
              this.logger.error(
                `List failed for scope '${scope}': ${err instanceof Error ? err.message : String(err)}`,
              );
              continuationToken = undefined;
            }
          } while (continuationToken);
        }

        // Detect deletes by comparing key sets.
        for (const [key] of lastKnownState) {
          if (!currentState.has(key)) {
            events.push({ type: "delete", key });
          }
        }

        lastKnownState = currentState;
        initialized = true;
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

  async cleanupExcessProfiles(
    userId: string,
    maxProfiles: number,
  ): Promise<{ deletedProfiles: string[]; remaining: number }> {
    const userPrefix = `users/${userId}/`;
    const profilePrefix = `${userPrefix}profiles/`;

    // List all profile directories
    const profiles: { id: string; lastModified: Date }[] = [];
    let continuationToken: string | undefined;

    do {
      const result = await this.s3Client.send(
        new ListObjectsV2Command({
          Bucket: this.bucket,
          Prefix: profilePrefix,
          Delimiter: "/",
          MaxKeys: 1000,
          ContinuationToken: continuationToken,
        }),
      );

      if (result.CommonPrefixes) {
        for (const cp of result.CommonPrefixes) {
          if (!cp.Prefix) continue;
          const profileId = cp.Prefix.replace(profilePrefix, "").replace(
            /\/$/,
            "",
          );

          // Get creation time from first object in the profile directory
          const objects = await this.s3Client.send(
            new ListObjectsV2Command({
              Bucket: this.bucket,
              Prefix: cp.Prefix,
              MaxKeys: 1,
            }),
          );

          const firstObj = objects.Contents?.[0];
          profiles.push({
            id: profileId,
            lastModified: firstObj?.LastModified || new Date(0),
          });
        }
      }

      continuationToken = result.NextContinuationToken;
    } while (continuationToken);

    if (profiles.length <= maxProfiles) {
      return { deletedProfiles: [], remaining: profiles.length };
    }

    // Sort newest first — delete newest excess profiles
    profiles.sort(
      (a, b) => b.lastModified.getTime() - a.lastModified.getTime(),
    );

    const excessCount = profiles.length - maxProfiles;
    const toDelete = profiles.slice(0, excessCount);
    const deletedProfiles: string[] = [];

    for (const profile of toDelete) {
      const prefix = `${profilePrefix}${profile.id}/`;

      // Delete all objects under this profile
      let delToken: string | undefined;
      do {
        const listResult = await this.s3Client.send(
          new ListObjectsV2Command({
            Bucket: this.bucket,
            Prefix: prefix,
            MaxKeys: 1000,
            ContinuationToken: delToken,
          }),
        );

        const objects = listResult.Contents || [];
        if (objects.length > 0) {
          const deleteObjects = objects
            .filter((obj): obj is typeof obj & { Key: string } => !!obj.Key)
            .map((obj) => ({ Key: obj.Key }));

          if (deleteObjects.length > 0) {
            await this.s3Client.send(
              new DeleteObjectsCommand({
                Bucket: this.bucket,
                Delete: { Objects: deleteObjects, Quiet: true },
              }),
            );
          }
        }

        delToken = listResult.NextContinuationToken;
      } while (delToken);

      // Create tombstone
      const tombstoneKey = `${userPrefix}tombstones/profiles/${profile.id}`;
      const tombstoneData = JSON.stringify({
        prefix: `profiles/${profile.id}/`,
        deleted_at: new Date().toISOString(),
        reason: "excess_profile_cleanup",
      });

      await this.s3Client.send(
        new PutObjectCommand({
          Bucket: this.bucket,
          Key: tombstoneKey,
          Body: tombstoneData,
          ContentType: "application/json",
        }),
      );

      deletedProfiles.push(profile.id);
      this.logger.log(
        `Cleaned up excess profile ${profile.id} for user ${userId}`,
      );
    }

    // Report updated profile usage to backend
    const remaining = profiles.length - deletedProfiles.length;
    await this.reportProfileUsage(userId, remaining).catch((err) =>
      this.logger.warn(`Failed to report usage after cleanup: ${err.message}`),
    );

    return { deletedProfiles, remaining };
  }

  /**
   * Check if the user has reached their profile limit.
   * Counts objects in the profiles/ prefix.
   */
  private async checkProfileLimit(ctx: UserContext): Promise<void> {
    if (ctx.profileLimit <= 0) return; // 0 = unlimited

    let count = 0;

    const userResult = await this.s3Client.send(
      new ListObjectsV2Command({
        Bucket: this.bucket,
        Prefix: `${ctx.prefix}profiles/`,
        Delimiter: "/",
      }),
    );
    count += userResult.CommonPrefixes?.length || 0;

    if (ctx.teamPrefix && ctx.teamProfileLimit && ctx.teamProfileLimit > 0) {
      const teamResult = await this.s3Client.send(
        new ListObjectsV2Command({
          Bucket: this.bucket,
          Prefix: `${ctx.teamPrefix}profiles/`,
          Delimiter: "/",
        }),
      );
      const teamCount = teamResult.CommonPrefixes?.length || 0;
      if (teamCount >= ctx.teamProfileLimit) {
        throw new ForbiddenException(
          `Team profile limit reached (${ctx.teamProfileLimit}). Ask the team owner to upgrade.`,
        );
      }
    }

    if (count >= ctx.profileLimit) {
      throw new ForbiddenException(
        `Profile limit reached (${ctx.profileLimit}). Upgrade your plan for more profiles.`,
      );
    }
  }

  /**
   * Count the number of distinct profile directories for a user.
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
          Delimiter: "/",
          MaxKeys: 1000,
          ContinuationToken: continuationToken,
        }),
      );
      count += result.CommonPrefixes?.length || 0;
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

  private async countTeamProfiles(ctx: UserContext): Promise<number> {
    if (!ctx.teamPrefix) return 0;
    const profilePrefix = `${ctx.teamPrefix}profiles/`;
    let count = 0;
    let continuationToken: string | undefined;

    do {
      const result = await this.s3Client.send(
        new ListObjectsV2Command({
          Bucket: this.bucket,
          Prefix: profilePrefix,
          Delimiter: "/",
          MaxKeys: 1000,
          ContinuationToken: continuationToken,
        }),
      );
      count += result.CommonPrefixes?.length || 0;
      continuationToken = result.NextContinuationToken;
    } while (continuationToken);

    return count;
  }

  private extractTeamId(ctx: UserContext): string | null {
    if (!ctx.teamPrefix) return null;
    const match = ctx.teamPrefix.match(/^teams\/([^/]+)\/$/);
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
      .then(async (count) => {
        await this.reportProfileUsage(userId, count);

        if (ctx.teamPrefix) {
          const teamCount = await this.countTeamProfiles(ctx);
          const teamId = this.extractTeamId(ctx);
          if (teamId) {
            await this.reportProfileUsage(teamId, teamCount);
          }
        }
      })
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

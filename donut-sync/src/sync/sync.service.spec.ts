import { ForbiddenException, Logger } from "@nestjs/common";
import type { ConfigService } from "@nestjs/config";
import type { UserContext } from "../auth/user-context.interface.js";
import { SyncService } from "./sync.service.js";

const OWNER_PREFIX = "users/99999999-8888-7777-6666-555555555555/";

function serviceWithStubbedS3(): { service: SyncService; send: jest.Mock } {
  const env: Record<string, string> = {
    S3_ENDPOINT: "http://s3.invalid:9000",
    S3_ACCESS_KEY_ID: "test-access",
    S3_SECRET_ACCESS_KEY: "test-secret",
    S3_BUCKET: "test-bucket",
  };
  const config = { get: (key: string) => env[key] } as unknown as ConfigService;
  const service = new SyncService(config);
  const send = jest.fn().mockResolvedValue({ Contents: [] });
  (service as unknown as { s3Client: { send: jest.Mock } }).s3Client = {
    send,
  };
  return { service, send };
}

function teamContext(teamRole: UserContext["teamRole"]): UserContext {
  return { mode: "cloud", prefix: OWNER_PREFIX, profileLimit: 0, teamRole };
}

async function refusalCode(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (err) {
    expect(err).toBeInstanceOf(ForbiddenException);
    return ((err as ForbiddenException).getResponse() as { code?: string })
      .code;
  }
  throw new Error("the request was not refused");
}

describe("SyncService team roles", () => {
  beforeEach(() => {
    jest.spyOn(Logger.prototype, "warn").mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("lets a launcher sync what a session changed", async () => {
    const { service } = serviceWithStubbedS3();
    const launcher = teamContext("launcher");

    for (const key of [
      "profiles/p1/files/Default/Cookies",
      "profiles/p1/manifest.json",
      "teams/t1/profiles/p1/files/Default/Cookies",
      "teams/t1/profiles/p1/manifest.json",
    ]) {
      await expect(
        service.presignUpload({ key }, launcher),
      ).resolves.toHaveProperty("url");
    }
    await expect(
      service.presignUploadBatch(
        { items: [{ key: "profiles/p1/files/Default/Preferences" }] },
        launcher,
      ),
    ).resolves.toHaveProperty("items");
    await expect(
      service.delete({ key: "profiles/p1/files/Default/old.log" }, launcher),
    ).resolves.toHaveProperty("deleted", true);
  });

  it("refuses every other write from a launcher before it reaches S3", async () => {
    const { service, send } = serviceWithStubbedS3();
    const launcher = teamContext("launcher");

    for (const key of [
      "profiles/p1/metadata.json",
      "proxies/x.json",
      "groups/g.json",
      "teams/t1/profiles/p1/metadata.json",
      "teams/t1/tombstones/profiles/p1.json",
      "teams/t1/x/profiles/p1/manifest.json",
      "profiles/p1/files",
      "profiles/p1/files/",
    ]) {
      await expect(
        refusalCode(service.presignUpload({ key }, launcher)),
      ).resolves.toBe("TEAM_ROLE_READ_ONLY");
    }
    await expect(
      refusalCode(
        service.presignUploadBatch(
          {
            items: [
              { key: "profiles/p1/manifest.json" },
              { key: "profiles/p1/metadata.json" },
            ],
          },
          launcher,
        ),
      ),
    ).resolves.toBe("TEAM_ROLE_READ_ONLY");
    await expect(
      refusalCode(
        service.delete(
          {
            key: "profiles/p1/files/Default/Cookies",
            tombstoneKey: "tombstones/profiles/p1.json",
          },
          launcher,
        ),
      ),
    ).resolves.toBe("TEAM_ROLE_READ_ONLY");
    await expect(
      refusalCode(service.deletePrefix({ prefix: "profiles/p1/" }, launcher)),
    ).resolves.toBe("TEAM_ROLE_READ_ONLY");

    expect(send).not.toHaveBeenCalled();
  });

  it("lets a member change profile settings and delete profiles", async () => {
    const { service } = serviceWithStubbedS3();
    const member = teamContext("member");

    await expect(
      service.presignUpload({ key: "profiles/p1/metadata.json" }, member),
    ).resolves.toHaveProperty("url");
    await expect(
      service.deletePrefix(
        {
          prefix: "profiles/p1/",
          tombstoneKey: "tombstones/profiles/p1.json",
        },
        member,
      ),
    ).resolves.toHaveProperty("deletedCount", 0);
  });
});

describe("SyncService access by group", () => {
  const groups: Record<string, string | null> = {
    p1: "g1",
    p2: "g2",
    p3: null,
  };

  function limitedService(): { service: SyncService; send: jest.Mock } {
    const { service, send } = serviceWithStubbedS3();
    send.mockImplementation(
      async (command: { input: Record<string, unknown> }) => {
        const name = command.constructor.name;
        const key = String(command.input.Key ?? "");
        if (name === "HeadObjectCommand") {
          const match = /profiles\/([^/]+)\/metadata\.json$/.exec(key);
          const id = match?.[1];
          if (!id || !(id in groups)) {
            throw Object.assign(new Error("not found"), { name: "NotFound" });
          }
          const group = groups[id];
          return { Metadata: group ? { "group-id": group } : {} };
        }
        if (name === "ListObjectsV2Command") {
          return {
            Contents: [
              "profiles/p1/manifest.json",
              "profiles/p2/manifest.json",
              "teams/t1/profiles/p1/files/Default/Cookies",
              "groups/g1.json",
              "groups/g2.json",
              "proxies/x.json",
            ].map((relative) => ({ Key: `${OWNER_PREFIX}${relative}` })),
          };
        }
        return {};
      },
    );
    return { service, send };
  }

  const limited: UserContext = {
    mode: "cloud",
    prefix: OWNER_PREFIX,
    profileLimit: 0,
    teamRole: "member",
    groupIds: ["g1"],
  };

  async function denialCode(promise: Promise<unknown>): Promise<unknown> {
    return refusalCode(promise);
  }

  beforeEach(() => {
    jest.spyOn(Logger.prototype, "warn").mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("shows a limited member only the profiles and groups it may reach", async () => {
    const { service } = limitedService();
    await expect(
      service.stat({ key: "profiles/p1/manifest.json" }, limited),
    ).resolves.toBeDefined();
    for (const key of [
      "profiles/p2/manifest.json",
      "profiles/p3/manifest.json",
      "groups/g2.json",
    ]) {
      await expect(denialCode(service.stat({ key }, limited))).resolves.toBe(
        "TEAM_GROUP_ACCESS_DENIED",
      );
    }
    await expect(
      service.presignDownload({ key: "proxies/x.json" }, limited),
    ).resolves.toHaveProperty("url");

    const listed = await service.list({ prefix: "" }, limited);
    expect(listed.objects.map((object) => object.key)).toEqual([
      "profiles/p1/manifest.json",
      "teams/t1/profiles/p1/files/Default/Cookies",
      "groups/g1.json",
      "proxies/x.json",
    ]);
  });

  it("lets a limited member change profiles only inside its groups", async () => {
    const { service } = limitedService();
    await expect(
      service.presignUpload(
        { key: "profiles/new/metadata.json", metadata: { "group-id": "g1" } },
        limited,
      ),
    ).resolves.toHaveProperty("url");
    await expect(
      service.presignUpload(
        { key: "profiles/p1/files/Default/Cookies" },
        limited,
      ),
    ).resolves.toHaveProperty("url");

    for (const [key, metadata] of [
      ["profiles/new/metadata.json", { "group-id": "g2" }],
      ["profiles/new/metadata.json", undefined],
      ["profiles/new/files/Default/Cookies", undefined],
      ["profiles/p1/metadata.json", { "group-id": "g2" }],
      ["profiles/p2/files/Default/Cookies", undefined],
      ["groups/g2.json", undefined],
    ] as const) {
      await expect(
        denialCode(service.presignUpload({ key, metadata }, limited)),
      ).resolves.toBe("TEAM_GROUP_ACCESS_DENIED");
    }
    await expect(
      denialCode(service.deletePrefix({ prefix: "profiles/p2/" }, limited)),
    ).resolves.toBe("TEAM_GROUP_ACCESS_DENIED");
  });

  it("leaves a member without a limit free to reach every group", async () => {
    const { service } = limitedService();
    const everyGroup: UserContext = { ...limited, groupIds: undefined };
    await expect(
      service.stat({ key: "profiles/p2/manifest.json" }, everyGroup),
    ).resolves.toBeDefined();
    const listed = await service.list({ prefix: "" }, everyGroup);
    expect(listed.objects).toHaveLength(6);
  });
});

describe("SyncService restore points", () => {
  /** A bucket in memory, enough for the restore-point paths. */
  function memoryService(): {
    service: SyncService;
    objects: Map<string, string>;
  } {
    const { service, send } = serviceWithStubbedS3();
    const objects = new Map<string, string>();
    send.mockImplementation(
      async (command: { input: Record<string, unknown> }) => {
        const name = command.constructor.name;
        const input = command.input;
        if (name === "ListObjectsV2Command") {
          const prefix = String(input.Prefix ?? "");
          return {
            Contents: [...objects.keys()]
              .filter((key) => key.startsWith(prefix))
              .sort()
              .map((key) => ({ Key: key, Size: objects.get(key)?.length })),
          };
        }
        if (name === "CopyObjectCommand") {
          const source = decodeURIComponent(
            String(input.CopySource).split("/").slice(1).join("/"),
          );
          objects.set(String(input.Key), objects.get(source) ?? "");
          return {};
        }
        if (name === "PutObjectCommand") {
          objects.set(String(input.Key), String(input.Body ?? ""));
          return {};
        }
        if (name === "GetObjectCommand") {
          const body = objects.get(String(input.Key));
          if (body === undefined) {
            throw Object.assign(new Error("missing"), { name: "NoSuchKey" });
          }
          return { Body: { transformToString: async () => body } };
        }
        if (name === "DeleteObjectsCommand") {
          const doomed = (input.Delete as { Objects: { Key: string }[] })
            .Objects;
          for (const { Key } of doomed) objects.delete(Key);
          return {};
        }
        return {};
      },
    );
    return { service, objects };
  }

  const owner: UserContext = {
    mode: "cloud",
    prefix: OWNER_PREFIX,
    profileLimit: 0,
    teamRole: "owner",
  };
  const live = (relative: string) => `${OWNER_PREFIX}profiles/p1/${relative}`;

  beforeEach(() => {
    jest.spyOn(Logger.prototype, "log").mockImplementation(() => undefined);
    jest.spyOn(Logger.prototype, "warn").mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("keeps a copy of the files and manifest, and puts it back", async () => {
    const { service, objects } = memoryService();
    objects.set(live("manifest.json"), "manifest-1");
    objects.set(live("files/Default/Cookies"), "cookies-1");
    objects.set(live("metadata.json"), "settings");

    const point = await service.createSnapshot(
      { profileRoot: "profiles/p1/", kind: "manual" },
      owner,
    );
    expect(point).toMatchObject({ kind: "manual", files: 1 });
    expect(
      [...objects.keys()].filter((key) => key.includes("/snapshots/")),
    ).toHaveLength(3);

    objects.set(live("files/Default/Cookies"), "cookies-2");
    objects.set(live("files/Default/New"), "new");
    const restored = await service.restoreSnapshotFiles(
      { profileRoot: "profiles/p1/", id: point.id },
      owner,
    );
    expect(restored).toEqual({ restored: 1, removed: 1 });
    expect(objects.get(live("files/Default/Cookies"))).toBe("cookies-1");
    expect(objects.has(live("files/Default/New"))).toBe(false);
    expect(objects.get(live("metadata.json"))).toBe("settings");

    const { snapshots } = await service.listSnapshots(
      { profileRoot: "profiles/p1/" },
      owner,
    );
    expect(snapshots.map((s) => s.id)).toEqual([point.id]);
  });

  it("keeps the newest few of each kind", async () => {
    const { service, objects } = memoryService();
    objects.set(live("manifest.json"), "m");
    const ids: string[] = [];
    for (let i = 0; i < SyncService.SNAPSHOTS_PER_KIND + 2; i++) {
      ids.push(
        (
          await service.createSnapshot(
            { profileRoot: "profiles/p1/", kind: "auto" },
            owner,
          )
        ).id,
      );
    }
    await service.createSnapshot(
      { profileRoot: "profiles/p1/", kind: "manual" },
      owner,
    );
    const { snapshots } = await service.listSnapshots(
      { profileRoot: "profiles/p1/" },
      owner,
    );
    expect(snapshots.filter((s) => s.kind === "auto")).toHaveLength(
      SyncService.SNAPSHOTS_PER_KIND,
    );
    expect(snapshots.filter((s) => s.kind === "manual")).toHaveLength(1);
  });

  it("refuses a launcher and a root that is not one profile", async () => {
    const { service, objects } = memoryService();
    objects.set(live("manifest.json"), "m");
    await expect(
      refusalCode(
        service.createSnapshot(
          { profileRoot: "profiles/p1/" },
          { ...owner, teamRole: "launcher" },
        ),
      ),
    ).resolves.toBe("TEAM_ROLE_READ_ONLY");
    for (const profileRoot of ["profiles/", "proxies/x/", "profiles/p1"]) {
      await expect(
        service.createSnapshot({ profileRoot }, owner),
      ).rejects.toThrow();
    }
  });
});

describe("SyncService profile limit", () => {
  const ACCOUNT: UserContext = {
    mode: "account",
    prefix: OWNER_PREFIX,
    profileLimit: 2,
    sub: "u1",
  };

  function withStoredProfiles(...ids: string[]) {
    const { service, send } = serviceWithStubbedS3();
    send.mockImplementation(
      async (command: { input?: { Prefix?: string } }) => {
        if (command.input?.Prefix === `${OWNER_PREFIX}profiles/`) {
          return {
            CommonPrefixes: ids.map((id) => ({
              Prefix: `${OWNER_PREFIX}profiles/${id}/`,
            })),
          };
        }
        return {};
      },
    );
    return service;
  }

  beforeEach(() => {
    jest.spyOn(Logger.prototype, "warn").mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("keeps a full plan syncing its profiles and settings, but adds none", async () => {
    const service = withStoredProfiles("p1", "p2");

    for (const key of [
      "profiles/p1/files/Default/Cookies",
      "profiles/p2/metadata.json",
      "proxies/x.json",
      "groups/g.json",
    ]) {
      await expect(
        service.presignUpload({ key }, ACCOUNT),
      ).resolves.toHaveProperty("url");
    }
    await expect(
      service.presignUpload({ key: "profiles/p3/metadata.json" }, ACCOUNT),
    ).rejects.toBeInstanceOf(ForbiddenException);
    await expect(
      service.presignUploadBatch(
        {
          items: [
            { key: "profiles/p1/files/a" },
            { key: "profiles/p3/files/a" },
          ],
        },
        ACCOUNT,
      ),
    ).rejects.toBeInstanceOf(ForbiddenException);
  });

  it("counts a batch's new profiles together", async () => {
    const service = withStoredProfiles("p1");

    await expect(
      service.presignUploadBatch(
        {
          items: [
            { key: "profiles/p2/files/a" },
            { key: "profiles/p2/files/b" },
          ],
        },
        ACCOUNT,
      ),
    ).resolves.toHaveProperty("items");
    await expect(
      service.presignUploadBatch(
        {
          items: [
            { key: "profiles/p2/files/a" },
            { key: "profiles/p3/files/a" },
          ],
        },
        ACCOUNT,
      ),
    ).rejects.toBeInstanceOf(ForbiddenException);
  });

  it("writes no profile over the limit, and never limits the shared token", async () => {
    const service = withStoredProfiles("p1", "p2", "p3");

    await expect(
      service.presignUpload({ key: "profiles/p1/files/a" }, ACCOUNT),
    ).rejects.toBeInstanceOf(ForbiddenException);
    await expect(
      service.presignUpload({ key: "proxies/x.json" }, ACCOUNT),
    ).resolves.toHaveProperty("url");
    await expect(
      service.presignUpload(
        { key: "profiles/p9/files/a" },
        { mode: "self-hosted", prefix: "", profileLimit: 0 },
      ),
    ).resolves.toHaveProperty("url");
  });
});

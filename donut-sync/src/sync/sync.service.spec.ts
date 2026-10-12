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

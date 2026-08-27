import { INestApplication, Logger } from "@nestjs/common";
import { ConfigModule } from "@nestjs/config";
import { Test, TestingModule } from "@nestjs/testing";
import request from "supertest";
import { App } from "supertest/types";
import { AppController } from "./../src/app.controller.js";
import { AppService } from "./../src/app.service.js";
import { SyncModule } from "./../src/sync/sync.module.js";
import {
  configureTestEnv,
  TEST_S3_ENDPOINT,
  TEST_SYNC_TOKEN,
  waitForTestS3,
} from "./test-env.js";

// Presigning is offline, so this host never has to accept a connection — the
// assertions are about which host ends up in the signed URL.
const PUBLIC_ENDPOINT = "https://storage.example.com";

// Only needs to be present for the server to consider itself cloud-mode; no
// token is verified against it in these assertions.
const CLOUD_PUBLIC_KEY =
  "-----BEGIN PUBLIC KEY-----\nnot-a-real-key\n-----END PUBLIC KEY-----";

interface PresignResponse {
  url: string;
}

interface PresignBatchResponse {
  items: Array<{ key: string; url: string }>;
}

interface ReadyResponse {
  status: string;
  s3: boolean;
  storageEndpoint: string;
}

async function bootstrap(publicEndpoint: string | undefined) {
  configureTestEnv();
  if (publicEndpoint) {
    process.env.S3_PUBLIC_ENDPOINT = publicEndpoint;
  } else {
    delete process.env.S3_PUBLIC_ENDPOINT;
  }
  await waitForTestS3();

  const moduleFixture: TestingModule = await Test.createTestingModule({
    imports: [ConfigModule.forRoot({ isGlobal: true }), SyncModule],
    controllers: [AppController],
    providers: [AppService],
  }).compile();

  const app = moduleFixture.createNestApplication<INestApplication<App>>();
  await app.listen(0);
  return app;
}

// A self-hosted server usually reaches its storage over a private address the
// desktop client has no route to. Signing client URLs against that address
// handed every client a URL it could not open, so uploads failed at connect
// while /health and /readyz stayed green.
describe("presigned URL host", () => {
  describe("with S3_PUBLIC_ENDPOINT set", () => {
    let app: INestApplication<App>;

    beforeAll(async () => {
      app = await bootstrap(PUBLIC_ENDPOINT);
    });

    afterAll(async () => {
      delete process.env.S3_PUBLIC_ENDPOINT;
      await app.close();
    });

    it("signs single upload URLs against the public endpoint", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/presign-upload")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: "endpoint/single.txt" })
        .expect(200);

      const { url } = response.body as PresignResponse;
      expect(url.startsWith(PUBLIC_ENDPOINT)).toBe(true);
      expect(url).not.toContain(TEST_S3_ENDPOINT);
    });

    it("signs batch upload URLs against the public endpoint", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/presign-upload-batch")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ items: [{ key: "endpoint/a.txt" }, { key: "endpoint/b.txt" }] })
        .expect(200);

      const { items } = response.body as PresignBatchResponse;
      expect(items).toHaveLength(2);
      for (const item of items) {
        expect(item.url.startsWith(PUBLIC_ENDPOINT)).toBe(true);
      }
    });

    it("signs download URLs against the public endpoint", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/presign-download")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: "endpoint/single.txt" })
        .expect(200);

      const { url } = response.body as PresignResponse;
      expect(url.startsWith(PUBLIC_ENDPOINT)).toBe(true);
    });

    // The server's own S3 calls must keep using the private endpoint, or
    // pointing clients at a public address would break the server itself.
    it("still reaches storage over the private endpoint", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/stat")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: "endpoint/does-not-exist" })
        .expect(200);

      expect(response.body).toEqual({ exists: false });
    });

    it("reports the client-facing endpoint from /readyz", async () => {
      const response = await request(app.getHttpServer())
        .get("/readyz")
        .expect(200);

      const body = response.body as ReadyResponse;
      expect(body.s3).toBe(true);
      expect(body.storageEndpoint).toBe(PUBLIC_ENDPOINT);
    });
  });

  // /readyz has no auth, so a managed deployment must not publish its storage
  // host to anyone who can reach the probe.
  describe("in cloud mode", () => {
    let app: INestApplication<App>;
    const previousKey = process.env.SYNC_JWT_PUBLIC_KEY;

    beforeAll(async () => {
      process.env.SYNC_JWT_PUBLIC_KEY = CLOUD_PUBLIC_KEY;
      app = await bootstrap(PUBLIC_ENDPOINT);
    });

    afterAll(async () => {
      if (previousKey === undefined) {
        delete process.env.SYNC_JWT_PUBLIC_KEY;
      } else {
        process.env.SYNC_JWT_PUBLIC_KEY = previousKey;
      }
      delete process.env.S3_PUBLIC_ENDPOINT;
      await app.close();
    });

    it("withholds the storage endpoint from /readyz", async () => {
      const response = await request(app.getHttpServer())
        .get("/readyz")
        .expect(200);

      const body = response.body as ReadyResponse;
      expect(body.s3).toBe(true);
      expect(body.storageEndpoint).toBeUndefined();
      expect(JSON.stringify(body)).not.toContain("storage.example.com");
    });
  });

  describe("without S3_PUBLIC_ENDPOINT", () => {
    let app: INestApplication<App>;

    beforeAll(async () => {
      app = await bootstrap(undefined);
    });

    afterAll(async () => {
      await app.close();
    });

    it("falls back to S3_ENDPOINT", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/presign-upload")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: "endpoint/fallback.txt" })
        .expect(200);

      const { url } = response.body as PresignResponse;
      expect(url.startsWith(TEST_S3_ENDPOINT)).toBe(true);
    });

    it("reports the fallback endpoint from /readyz", async () => {
      const response = await request(app.getHttpServer())
        .get("/readyz")
        .expect(200);

      expect((response.body as ReadyResponse).storageEndpoint).toBe(
        TEST_S3_ENDPOINT,
      );
    });
  });
});

// The server cannot test whether a device can reach the endpoint it signs, so
// the only honest thing it can do is say what it is handing out. Without this,
// the one configuration that breaks every transfer boots completely silently.
describe("boot message about the presign endpoint", () => {
  let logs: string[];
  let warnings: string[];
  let logSpy: jest.SpyInstance;
  let warnSpy: jest.SpyInstance;

  beforeEach(() => {
    logs = [];
    warnings = [];
    logSpy = jest
      .spyOn(Logger.prototype, "log")
      .mockImplementation((message: unknown) => {
        logs.push(String(message));
      });
    warnSpy = jest
      .spyOn(Logger.prototype, "warn")
      .mockImplementation((message: unknown) => {
        warnings.push(String(message));
      });
  });

  afterEach(() => {
    logSpy.mockRestore();
    warnSpy.mockRestore();
  });

  it("says which host clients will be handed when S3_PUBLIC_ENDPOINT is unset", async () => {
    const app = await bootstrap(undefined);
    try {
      const spoken = [...logs, ...warnings].join("\n");
      expect(spoken).toContain("S3_PUBLIC_ENDPOINT");
      expect(spoken).toContain(TEST_S3_ENDPOINT);
    } finally {
      await app.close();
    }
  });

  // A single-label host is the documented compose default and cannot work for
  // any client, so it earns a warning rather than a note.
  it("warns loudly about a container-only host", async () => {
    const app = await bootstrap("http://minio:9000");
    try {
      const spoken = warnings.join("\n");
      expect(spoken).toContain("minio");
      expect(spoken).toContain("S3_PUBLIC_ENDPOINT");
    } finally {
      delete process.env.S3_PUBLIC_ENDPOINT;
      await app.close();
    }
  });

  // An operator who set the variable made a choice. Repeating the note at them
  // would train them to ignore it, and the warning above is for the value that
  // provably cannot work, not for every value the server cannot verify.
  it("stays quiet when an operator has chosen a routable endpoint", async () => {
    const app = await bootstrap(PUBLIC_ENDPOINT);
    try {
      const spoken = [...logs, ...warnings].join("\n");
      expect(spoken).not.toContain("S3_PUBLIC_ENDPOINT is not set");
    } finally {
      delete process.env.S3_PUBLIC_ENDPOINT;
      await app.close();
    }
  });
});

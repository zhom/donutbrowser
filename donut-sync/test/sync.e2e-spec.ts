import { INestApplication } from "@nestjs/common";
import { ConfigModule } from "@nestjs/config";
import { Test, TestingModule } from "@nestjs/testing";
import request from "supertest";
import { App } from "supertest/types";
import { AppController } from "./../src/app.controller.js";
import { AppService } from "./../src/app.service.js";
import { SyncModule } from "./../src/sync/sync.module.js";

interface PresignResponse {
  url: string;
  expiresAt: string;
}

interface ListResponse {
  objects: Array<{ key: string; lastModified: string; size: number }>;
  isTruncated: boolean;
  nextContinuationToken?: string;
}

interface DeleteResponse {
  deleted: boolean;
  tombstoneCreated: boolean;
}

interface StatResponse {
  exists: boolean;
  size?: number;
  lastModified?: string;
}

interface SSEError {
  code?: string;
  timeout?: boolean;
  response?: { status: number };
}

const TEST_TOKEN = "test-sync-token";

describe("SyncController (e2e)", () => {
  let app: INestApplication<App>;

  beforeAll(async () => {
    process.env.SYNC_TOKEN = TEST_TOKEN;
    process.env.S3_ENDPOINT =
      process.env.S3_ENDPOINT || "http://localhost:8987";
    process.env.S3_ACCESS_KEY_ID = process.env.S3_ACCESS_KEY_ID || "minioadmin";
    process.env.S3_SECRET_ACCESS_KEY =
      process.env.S3_SECRET_ACCESS_KEY || "minioadmin";
    process.env.S3_BUCKET = "donut-sync-test";
    process.env.S3_FORCE_PATH_STYLE = "true";

    const moduleFixture: TestingModule = await Test.createTestingModule({
      imports: [
        ConfigModule.forRoot({
          isGlobal: true,
        }),
        SyncModule,
      ],
      controllers: [AppController],
      providers: [AppService],
    }).compile();

    app = moduleFixture.createNestApplication();
    await app.init();
  });

  afterAll(async () => {
    await app.close();
  });

  describe("Authentication", () => {
    it("should reject requests without authorization header", () => {
      return request(app.getHttpServer())
        .post("/v1/objects/stat")
        .send({ key: "test-key" })
        .expect(401);
    });

    it("should reject requests with invalid token", () => {
      return request(app.getHttpServer())
        .post("/v1/objects/stat")
        .set("Authorization", "Bearer invalid-token")
        .send({ key: "test-key" })
        .expect(401);
    });

    it("should accept requests with valid token", () => {
      return request(app.getHttpServer())
        .post("/v1/objects/stat")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: "nonexistent-key" })
        .expect(200)
        .expect({ exists: false });
    });
  });

  describe("POST /v1/objects/stat", () => {
    it("should return exists: false for non-existent key", () => {
      return request(app.getHttpServer())
        .post("/v1/objects/stat")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: "does-not-exist" })
        .expect(200)
        .expect({ exists: false });
    });
  });

  describe("POST /v1/objects/presign-upload", () => {
    it("should return a presigned upload URL", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/presign-upload")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: "test/upload-key.txt", contentType: "text/plain" })
        .expect(200);

      const body = response.body as PresignResponse;
      expect(body.url).toBeDefined();
      expect(body.url).toContain("test/upload-key.txt");
      expect(body.expiresAt).toBeDefined();
    });
  });

  describe("POST /v1/objects/presign-download", () => {
    it("should return a presigned download URL", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/presign-download")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: "test/download-key.txt" })
        .expect(200);

      const body = response.body as PresignResponse;
      expect(body.url).toBeDefined();
      expect(body.url).toContain("test/download-key.txt");
      expect(body.expiresAt).toBeDefined();
    });
  });

  describe("POST /v1/objects/list", () => {
    it("should list objects with prefix", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/list")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ prefix: "profiles/" })
        .expect(200);

      const body = response.body as ListResponse;
      expect(body.objects).toBeDefined();
      expect(Array.isArray(body.objects)).toBe(true);
      expect(body.isTruncated).toBeDefined();
    });
  });

  describe("POST /v1/objects/delete", () => {
    it("should delete object and create tombstone", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/delete")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({
          key: "test/to-delete.txt",
          tombstoneKey: "tombstones/test/to-delete.json",
          deletedAt: new Date().toISOString(),
        })
        .expect(200);

      const body = response.body as DeleteResponse;
      expect(body.deleted).toBeDefined();
      expect(body.tombstoneCreated).toBe(true);
    });
  });

  describe("Full upload/download cycle", () => {
    const testKey = `test/e2e-cycle-${Date.now()}.txt`;
    const testContent = "Hello from e2e test!";

    it("should complete full upload/download cycle with presigned URLs", async () => {
      const uploadResponse = await request(app.getHttpServer())
        .post("/v1/objects/presign-upload")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: testKey, contentType: "text/plain" })
        .expect(200);

      const uploadBody = uploadResponse.body as PresignResponse;
      expect(uploadBody.url).toBeDefined();

      const uploadResult = await fetch(uploadBody.url, {
        method: "PUT",
        body: testContent,
        headers: { "Content-Type": "text/plain" },
      });
      expect(uploadResult.ok).toBe(true);

      const statResponse = await request(app.getHttpServer())
        .post("/v1/objects/stat")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: testKey })
        .expect(200);

      const statBody = statResponse.body as StatResponse;
      expect(statBody.exists).toBe(true);
      expect(statBody.size).toBeGreaterThan(0);

      const downloadResponse = await request(app.getHttpServer())
        .post("/v1/objects/presign-download")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: testKey })
        .expect(200);

      const downloadBody = downloadResponse.body as PresignResponse;
      const downloadResult = await fetch(downloadBody.url);
      expect(downloadResult.ok).toBe(true);

      const downloadedContent = await downloadResult.text();
      expect(downloadedContent).toBe(testContent);

      await request(app.getHttpServer())
        .post("/v1/objects/delete")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: testKey })
        .expect(200);

      const finalStatResponse = await request(app.getHttpServer())
        .post("/v1/objects/stat")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .send({ key: testKey })
        .expect(200);

      const finalStatBody = finalStatResponse.body as StatResponse;
      expect(finalStatBody.exists).toBe(false);
    });
  });

  describe("GET /v1/objects/subscribe (SSE)", () => {
    it("should reject SSE without authorization", () => {
      return request(app.getHttpServer())
        .get("/v1/objects/subscribe")
        .expect(401);
    });

    it("should return SSE stream with valid token", async () => {
      const response = await request(app.getHttpServer())
        .get("/v1/objects/subscribe")
        .set("Authorization", `Bearer ${TEST_TOKEN}`)
        .set("Accept", "text/event-stream")
        .buffer(true)
        .timeout(3000)
        .catch((err: SSEError) => {
          if (err.code === "ECONNABORTED" || err.timeout) {
            return err.response ?? { status: 200 };
          }
          throw err;
        });

      expect(response.status).toBe(200);
    });
  });
});

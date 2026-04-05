import type { Server } from "node:http";
import type { AddressInfo } from "node:net";
import { INestApplication } from "@nestjs/common";
import { ConfigModule } from "@nestjs/config";
import { Test, TestingModule } from "@nestjs/testing";
import request from "supertest";
import { App } from "supertest/types";
import { AppController } from "./../src/app.controller.js";
import { AppService } from "./../src/app.service.js";
import { SyncModule } from "./../src/sync/sync.module.js";
import {
  configureTestEnv,
  TEST_SYNC_TOKEN,
  waitForTestS3,
} from "./test-env.js";

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

describe("SyncController (e2e)", () => {
  let app: INestApplication<App>;

  beforeAll(async () => {
    configureTestEnv();
    await waitForTestS3();

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
    await app.listen(0);
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
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: "nonexistent-key" })
        .expect(200)
        .expect({ exists: false });
    });
  });

  describe("POST /v1/objects/stat", () => {
    it("should return exists: false for non-existent key", () => {
      return request(app.getHttpServer())
        .post("/v1/objects/stat")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: "does-not-exist" })
        .expect(200)
        .expect({ exists: false });
    });
  });

  describe("POST /v1/objects/presign-upload", () => {
    it("should return a presigned upload URL", async () => {
      const response = await request(app.getHttpServer())
        .post("/v1/objects/presign-upload")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
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
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
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
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
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
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
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
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
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
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: testKey })
        .expect(200);

      const statBody = statResponse.body as StatResponse;
      expect(statBody.exists).toBe(true);
      expect(statBody.size).toBeGreaterThan(0);

      const downloadResponse = await request(app.getHttpServer())
        .post("/v1/objects/presign-download")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: testKey })
        .expect(200);

      const downloadBody = downloadResponse.body as PresignResponse;
      const downloadResult = await fetch(downloadBody.url);
      expect(downloadResult.ok).toBe(true);

      const downloadedContent = await downloadResult.text();
      expect(downloadedContent).toBe(testContent);

      await request(app.getHttpServer())
        .post("/v1/objects/delete")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
        .send({ key: testKey })
        .expect(200);

      const finalStatResponse = await request(app.getHttpServer())
        .post("/v1/objects/stat")
        .set("Authorization", `Bearer ${TEST_SYNC_TOKEN}`)
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
      const address = (
        app.getHttpServer() as Server
      ).address() as AddressInfo | null;
      if (!address || typeof address === "string") {
        throw new Error("Expected app to be listening on a TCP port");
      }

      const response = await fetch(
        `http://127.0.0.1:${address.port}/v1/objects/subscribe`,
        {
          headers: {
            Accept: "text/event-stream",
            Authorization: `Bearer ${TEST_SYNC_TOKEN}`,
          },
        },
      );

      expect(response.status).toBe(200);
      expect(response.headers.get("content-type")).toContain(
        "text/event-stream",
      );
      await response.body?.cancel();
    });
  });
});

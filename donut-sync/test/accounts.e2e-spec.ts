import { randomUUID } from "node:crypto";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import type { INestApplication } from "@nestjs/common";
import { ConfigModule } from "@nestjs/config";
import { Test, type TestingModule } from "@nestjs/testing";
import * as jwt from "jsonwebtoken";
import request from "supertest";
import type { App } from "supertest/types";
import { AppController } from "./../src/app.controller.js";
import { AppService } from "./../src/app.service.js";
import { SyncModule } from "./../src/sync/sync.module.js";
import {
  configureTestEnv,
  TEST_SYNC_TOKEN,
  waitForTestS3,
} from "./test-env.js";

// Donut accounts on a server the customer runs: the Donut API (a stand-in
// here) says whose a token is, and this server applies the namespace, the
// plan's profile limit and the team scope against real storage.
describe("Donut account sign-in (e2e)", () => {
  const PUBLIC_URL = "https://sync.example.test";
  // Shaped like the Donut API's tokens for this server. The stand-in API
  // vouches for them by value; this server never sees the signing key.
  const tokenFor = (name: string) =>
    jwt.sign({ sub: name, scope: "server-sync", aud: PUBLIC_URL }, "test-key", {
      expiresIn: 300,
    });
  const OWNER_TOKEN = tokenFor("owner");
  const OTHER_TOKEN = tokenFor("other");
  const MEMBER_TOKEN = tokenFor("member");
  const STRANGER_TOKEN = tokenFor("stranger");
  const run = randomUUID().slice(0, 8);
  const OWNER = `owner-${run}`;
  const OTHER = `other-${run}`;
  const MEMBER = `member-${run}`;
  const tokens: Record<string, unknown> = {
    [OWNER_TOKEN]: {
      active: true,
      sub: OWNER,
      profileLimit: 1,
      team: null,
    },
    [OTHER_TOKEN]: {
      active: true,
      sub: OTHER,
      profileLimit: 5,
      team: null,
    },
    [MEMBER_TOKEN]: {
      active: true,
      sub: MEMBER,
      profileLimit: 0,
      team: {
        ownerId: OWNER,
        teamId: "team-1",
        teamProfileLimit: 1,
        role: "member",
        groupIds: null,
      },
    },
  };
  const asked: Array<{ token: string; server: string }> = [];
  const saved = {
    SYNC_PUBLIC_URL: process.env.SYNC_PUBLIC_URL,
    DONUT_API_URL: process.env.DONUT_API_URL,
  };
  let api: Server;
  let app: INestApplication<App>;

  beforeAll(async () => {
    api = createServer((req, res) => {
      let body = "";
      req.on("data", (chunk: Buffer) => {
        body += chunk.toString();
      });
      req.on("end", () => {
        const question = JSON.parse(body) as { token: string; server: string };
        asked.push(question);
        res.setHeader("Content-Type", "application/json");
        res.end(JSON.stringify(tokens[question.token] ?? { active: false }));
      });
    });
    await new Promise<void>((resolve) => api.listen(0, "127.0.0.1", resolve));
    const { port } = api.address() as AddressInfo;

    configureTestEnv();
    process.env.SYNC_PUBLIC_URL = `${PUBLIC_URL}/`;
    process.env.DONUT_API_URL = `http://127.0.0.1:${port}`;
    await waitForTestS3();

    const moduleFixture: TestingModule = await Test.createTestingModule({
      imports: [ConfigModule.forRoot({ isGlobal: true }), SyncModule],
      controllers: [AppController],
      providers: [AppService],
    }).compile();
    app = moduleFixture.createNestApplication();
    await app.listen(0);
  });

  afterAll(async () => {
    await app.close();
    api.closeAllConnections();
    await new Promise((resolve) => api.close(resolve));
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  });

  function upload(token: string, key: string) {
    return request(app.getHttpServer())
      .post("/v1/objects/presign-upload")
      .set("Authorization", `Bearer ${token}`)
      .send({ key, contentType: "application/json" });
  }

  async function put(token: string, key: string): Promise<void> {
    const response = await upload(token, key).expect(200);
    const stored = await fetch((response.body as { url: string }).url, {
      method: "PUT",
      body: "{}",
      headers: { "Content-Type": "application/json" },
    });
    expect(stored.ok).toBe(true);
  }

  async function listed(token: string): Promise<string[]> {
    const response = await request(app.getHttpServer())
      .post("/v1/objects/list")
      .set("Authorization", `Bearer ${token}`)
      .send({ prefix: "profiles/" })
      .expect(200);
    return (response.body as { objects: Array<{ key: string }> }).objects.map(
      (object) => object.key,
    );
  }

  it("names the sign-in address on /readyz", async () => {
    const response = await request(app.getHttpServer())
      .get("/readyz")
      .expect(200);
    expect(response.body).toMatchObject({ accountUrl: PUBLIC_URL });
  });

  it("keeps each account in its own space, apart from the shared token", async () => {
    await put(OWNER_TOKEN, "profiles/p1/metadata.json");

    expect(await listed(OWNER_TOKEN)).toContain("profiles/p1/metadata.json");
    expect(await listed(OTHER_TOKEN)).not.toContain(
      "profiles/p1/metadata.json",
    );
    // The shared token sees the bucket root, where account spaces are not.
    expect(await listed(TEST_SYNC_TOKEN)).not.toContain(
      "profiles/p1/metadata.json",
    );
    expect(asked.every((question) => question.server === PUBLIC_URL)).toBe(
      true,
    );
  });

  it("applies the plan's profile limit, and lets a full plan keep syncing", async () => {
    await put(OWNER_TOKEN, "profiles/p1/files/Default/Cookies");
    await upload(OWNER_TOKEN, "profiles/p2/metadata.json").expect(403);
    await upload(OWNER_TOKEN, "proxies/x.json").expect(200);
  });

  it("gives a team member the owner's space and the team's limit", async () => {
    expect(await listed(MEMBER_TOKEN)).toContain("profiles/p1/metadata.json");
    await upload(MEMBER_TOKEN, "profiles/p2/metadata.json").expect(403);
  });

  it("refuses a token the API does not vouch for", async () => {
    await upload(STRANGER_TOKEN, "profiles/p1/metadata.json").expect(401);
  });
});

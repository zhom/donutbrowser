import { generateKeyPairSync } from "node:crypto";
import {
  type ExecutionContext,
  Logger,
  UnauthorizedException,
} from "@nestjs/common";
import type { ConfigService } from "@nestjs/config";
import * as jwt from "jsonwebtoken";
import { AuthGuard } from "./auth.guard.js";
import type { UserContext } from "./user-context.interface.js";

const signingKeys = generateKeyPairSync("rsa", {
  modulusLength: 2048,
  publicKeyEncoding: { type: "spki", format: "pem" },
  privateKeyEncoding: { type: "pkcs8", format: "pem" },
});

const otherKeys = generateKeyPairSync("rsa", {
  modulusLength: 2048,
  publicKeyEncoding: { type: "spki", format: "pem" },
  privateKeyEncoding: { type: "pkcs8", format: "pem" },
});

const USER_ID = "11111111-2222-3333-4444-555555555555";
const SELF_HOSTED_TOKEN = "self-hosted-secret";

function guardWith(env: Record<string, string | undefined>): AuthGuard {
  const config = {
    get: (key: string) => env[key],
  } as unknown as ConfigService;
  return new AuthGuard(config);
}

function cloudGuard(): AuthGuard {
  return guardWith({ SYNC_JWT_PUBLIC_KEY: signingKeys.publicKey });
}

function contextFor(token: string): {
  context: ExecutionContext;
  request: { headers: Record<string, string>; user?: UserContext };
} {
  const request: { headers: Record<string, string>; user?: UserContext } = {
    headers: { authorization: `Bearer ${token}` },
  };
  const context = {
    switchToHttp: () => ({ getRequest: () => request }),
  } as unknown as ExecutionContext;
  return { context, request };
}

function sign(
  payload: Record<string, unknown>,
  privateKey: string = signingKeys.privateKey,
  expiresIn = 300,
): string {
  return jwt.sign(payload, privateKey, { algorithm: "RS256", expiresIn });
}

function syncToken(overrides: Record<string, unknown> = {}): string {
  return sign({
    sub: USER_ID,
    prefix: `users/${USER_ID}/`,
    profileLimit: 5,
    scope: "sync",
    ...overrides,
  });
}

describe("AuthGuard", () => {
  let warnSpy: jest.SpyInstance;

  beforeEach(() => {
    warnSpy = jest
      .spyOn(Logger.prototype, "warn")
      .mockImplementation(() => undefined);
    jest.spyOn(Logger.prototype, "log").mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("accepts a sync token and scopes the request to its own namespace", async () => {
    const { context, request } = contextFor(syncToken());

    await expect(cloudGuard().canActivate(context)).resolves.toBe(true);
    expect(request.user).toEqual({
      mode: "cloud",
      prefix: `users/${USER_ID}/`,
      profileLimit: 5,
      sub: USER_ID,
    });
  });

  it("keeps the team role the backend names for a team member", async () => {
    const OWNER_ID = "99999999-8888-7777-6666-555555555555";
    const answers = ["launcher", "superuser"];
    jest.spyOn(globalThis, "fetch").mockImplementation(async () => {
      const role = answers.shift();
      return new Response(
        JSON.stringify({
          ownerId: OWNER_ID,
          teamId: "team-1",
          teamProfileLimit: 50,
          role,
        }),
      );
    });
    const guard = () =>
      guardWith({
        SYNC_JWT_PUBLIC_KEY: signingKeys.publicKey,
        BACKEND_INTERNAL_URL: "http://backend.internal",
        BACKEND_INTERNAL_KEY: "internal-key",
      });

    const launcher = contextFor(syncToken());
    await expect(guard().canActivate(launcher.context)).resolves.toBe(true);
    expect(launcher.request.user).toEqual({
      mode: "cloud",
      prefix: `users/${OWNER_ID}/`,
      profileLimit: 50,
      sub: USER_ID,
      teamRole: "launcher",
    });

    // A role this build does not know never widens what the member may do.
    const unknown = contextFor(syncToken());
    await expect(guard().canActivate(unknown.context)).resolves.toBe(true);
    expect(unknown.request.user?.teamRole).toBe("member");
  });

  it("refuses an account access token signed with the same key", async () => {
    const accessToken = sign({
      sub: USER_ID,
      email: "owner@example.com",
      sid: "device-1",
    });
    const { context, request } = contextFor(accessToken);

    await expect(cloudGuard().canActivate(context)).rejects.toBeInstanceOf(
      UnauthorizedException,
    );
    expect(request.user).toBeUndefined();
    expect(warnSpy).toHaveBeenCalled();
  });

  it("refuses a per-profile agent token signed with the same key", async () => {
    const agentToken = sign({
      sub: USER_ID,
      email: "owner@example.com",
      sid: "agent:run-1",
      scope: "agent",
      profile: "profile-1",
    });
    const { context, request } = contextFor(agentToken);

    await expect(cloudGuard().canActivate(context)).rejects.toBeInstanceOf(
      UnauthorizedException,
    );
    expect(request.user).toBeUndefined();
  });

  it.each([
    ["an array", ["sync"]],
    ["another case", "SYNC"],
    ["an empty string", ""],
    ["null", null],
  ])("refuses a scope that is %s", async (_label, scope) => {
    const { context } = contextFor(syncToken({ scope }));

    await expect(cloudGuard().canActivate(context)).rejects.toBeInstanceOf(
      UnauthorizedException,
    );
  });

  it("refuses a sync token signed with a different key", async () => {
    const forged = sign(
      {
        sub: USER_ID,
        prefix: `users/${USER_ID}/`,
        profileLimit: 5,
        scope: "sync",
      },
      otherKeys.privateKey,
    );
    const { context } = contextFor(forged);

    await expect(cloudGuard().canActivate(context)).rejects.toBeInstanceOf(
      UnauthorizedException,
    );
  });

  it("refuses an expired sync token", async () => {
    const expired = sign(
      {
        sub: USER_ID,
        prefix: `users/${USER_ID}/`,
        profileLimit: 5,
        scope: "sync",
      },
      signingKeys.privateKey,
      -10,
    );
    const { context } = contextFor(expired);

    await expect(cloudGuard().canActivate(context)).rejects.toBeInstanceOf(
      UnauthorizedException,
    );
  });

  it("refuses a sync token whose prefix is not a single user namespace", async () => {
    const { context } = contextFor(syncToken({ prefix: "users/" }));

    await expect(cloudGuard().canActivate(context)).rejects.toBeInstanceOf(
      UnauthorizedException,
    );
  });

  it("still accepts the configured token of a self-hosted server", async () => {
    const guard = guardWith({
      SYNC_TOKEN: SELF_HOSTED_TOKEN,
      SYNC_JWT_PUBLIC_KEY: signingKeys.publicKey,
    });
    const { context, request } = contextFor(SELF_HOSTED_TOKEN);

    await expect(guard.canActivate(context)).resolves.toBe(true);
    expect(request.user).toEqual({
      mode: "self-hosted",
      prefix: "",
      profileLimit: 0,
    });
  });
});

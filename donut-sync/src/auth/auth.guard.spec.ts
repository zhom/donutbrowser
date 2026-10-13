import { generateKeyPairSync } from "node:crypto";
import {
  type ExecutionContext,
  Logger,
  ServiceUnavailableException,
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

  it("keeps the groups a limited member may reach, and drops malformed ids", async () => {
    const answers = [["g1", "bad/id", 7], null];
    jest.spyOn(globalThis, "fetch").mockImplementation(async () => {
      const groupIds = answers.shift();
      return new Response(
        JSON.stringify({
          ownerId: "99999999-8888-7777-6666-555555555555",
          teamId: "team-1",
          teamProfileLimit: 0,
          role: "member",
          groupIds,
        }),
      );
    });
    const guard = () =>
      guardWith({
        SYNC_JWT_PUBLIC_KEY: signingKeys.publicKey,
        BACKEND_INTERNAL_URL: "http://backend.internal",
        BACKEND_INTERNAL_KEY: "internal-key",
      });

    const limited = contextFor(syncToken());
    await expect(guard().canActivate(limited.context)).resolves.toBe(true);
    expect(limited.request.user?.groupIds).toEqual(["g1"]);

    const everyGroup = contextFor(syncToken());
    await expect(guard().canActivate(everyGroup.context)).resolves.toBe(true);
    expect(everyGroup.request.user?.groupIds).toBeUndefined();
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

describe("AuthGuard with Donut account sign-in", () => {
  const PUBLIC_URL = "https://sync.example.com";
  const OWNER_ID = "99999999-8888-7777-6666-555555555555";

  beforeEach(() => {
    jest.spyOn(Logger.prototype, "warn").mockImplementation(() => undefined);
    jest.spyOn(Logger.prototype, "log").mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  function accountGuard(): AuthGuard {
    return guardWith({
      SYNC_TOKEN: SELF_HOSTED_TOKEN,
      SYNC_PUBLIC_URL: `${PUBLIC_URL}/`,
      DONUT_API_URL: "http://api.test",
    });
  }

  // Signed with a key this server never sees: only the Donut API can say
  // whose a token is. The claims only decide whether to ask.
  const accountToken = (n = 1, claims: Record<string, unknown> = {}) =>
    sign(
      { sub: USER_ID, n, scope: "server-sync", aud: PUBLIC_URL, ...claims },
      otherKeys.privateKey,
    );

  function answering(...answers: unknown[]): jest.SpyInstance {
    return jest.spyOn(globalThis, "fetch").mockImplementation(async () => {
      const answer = answers.shift();
      if (answer instanceof Response) return answer;
      return new Response(JSON.stringify(answer));
    });
  }

  it("gives an account the API vouches for its own namespace and plan limit", async () => {
    const fetchSpy = answering({
      active: true,
      sub: USER_ID,
      profileLimit: 20,
      team: null,
    });
    const token = accountToken();
    const { context, request } = contextFor(token);

    await expect(accountGuard().canActivate(context)).resolves.toBe(true);
    expect(request.user).toEqual({
      mode: "account",
      prefix: `users/${USER_ID}/`,
      profileLimit: 20,
      sub: USER_ID,
    });
    // The API is asked about this server's own address, never the caller's.
    const [url, init] = fetchSpy.mock.calls[0];
    expect(url).toBe("http://api.test/api/auth/server-sync/introspect");
    expect(JSON.parse((init as RequestInit).body as string)).toEqual({
      token,
      server: PUBLIC_URL,
    });
  });

  it("puts a team member in the owner namespace with the team's rules", async () => {
    answering({
      active: true,
      sub: USER_ID,
      profileLimit: 0,
      team: {
        ownerId: OWNER_ID,
        teamId: "team-1",
        teamProfileLimit: 100,
        role: "launcher",
        groupIds: ["g1", "bad/id"],
      },
    });
    const { context, request } = contextFor(accountToken());

    await expect(accountGuard().canActivate(context)).resolves.toBe(true);
    expect(request.user).toEqual({
      mode: "account",
      prefix: `users/${OWNER_ID}/`,
      profileLimit: 100,
      sub: USER_ID,
      teamRole: "launcher",
      groupIds: ["g1"],
    });
  });

  it("refuses what the API does not vouch for, and asks once per token", async () => {
    // Refused, then active without a limit: 0 is unlimited, so no limit
    // refuses rather than reads as 0.
    const fetchSpy = answering(
      { active: false },
      { active: true, sub: USER_ID },
      { active: false },
    );
    const token = accountToken();

    for (let i = 0; i < 2; i++) {
      await expect(
        accountGuard().canActivate(contextFor(token).context),
      ).rejects.toBeInstanceOf(UnauthorizedException);
    }
    const guard = accountGuard();
    const other = accountToken(2);
    await expect(
      guard.canActivate(contextFor(other).context),
    ).rejects.toBeInstanceOf(UnauthorizedException);
    await expect(
      guard.canActivate(contextFor(other).context),
    ).rejects.toBeInstanceOf(UnauthorizedException);
    expect(fetchSpy).toHaveBeenCalledTimes(3);
  });

  it("fails closed when the API cannot be asked", async () => {
    answering(new Response("down", { status: 502 }));
    const { context, request } = contextFor(accountToken());

    await expect(accountGuard().canActivate(context)).rejects.toBeInstanceOf(
      ServiceUnavailableException,
    );
    expect(request.user).toBeUndefined();
  });

  it("never asks about a token that is not for this server", async () => {
    const fetchSpy = answering();
    for (const token of [
      accountToken(1, { aud: "https://other.example.com" }),
      accountToken(2, { scope: "sync" }),
      sign(
        { sub: USER_ID, scope: "server-sync", aud: PUBLIC_URL },
        otherKeys.privateKey,
        -10,
      ),
    ]) {
      await expect(
        accountGuard().canActivate(contextFor(token).context),
      ).rejects.toBeInstanceOf(UnauthorizedException);
    }
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("never sends the shared token or other strings to the API", async () => {
    const fetchSpy = answering();
    await expect(
      accountGuard().canActivate(contextFor(SELF_HOSTED_TOKEN).context),
    ).resolves.toBe(true);
    await expect(
      accountGuard().canActivate(contextFor("not a token").context),
    ).rejects.toBeInstanceOf(UnauthorizedException);
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});

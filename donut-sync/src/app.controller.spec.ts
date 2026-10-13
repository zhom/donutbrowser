import { ConfigService } from "@nestjs/config";
import { Test, type TestingModule } from "@nestjs/testing";
import { AppController } from "./app.controller.js";
import { AppService } from "./app.service.js";
import { SyncService } from "./sync/sync.service.js";

describe("AppController", () => {
  let appController: AppController;
  let env: Record<string, string>;

  beforeEach(async () => {
    env = {};
    const app: TestingModule = await Test.createTestingModule({
      controllers: [AppController],
      providers: [
        AppService,
        {
          provide: SyncService,
          useValue: {
            checkS3Connectivity: jest.fn().mockResolvedValue(true),
            getDiagnosticStorageEndpoint: jest.fn().mockReturnValue(undefined),
          },
        },
        {
          provide: ConfigService,
          useValue: { get: (key: string) => env[key] },
        },
      ],
    }).compile();

    appController = app.get<AppController>(AppController);
  });

  describe("root", () => {
    it("should return service name", () => {
      expect(appController.getHello()).toBe("Donut Sync Service");
    });
  });

  describe("health", () => {
    it("should return ok status", () => {
      expect(appController.getHealth()).toEqual({ status: "ok" });
    });
  });

  describe("readyz", () => {
    it("names the address Donut accounts sign in with, when they can", async () => {
      await expect(appController.getReadiness()).resolves.toEqual({
        status: "ready",
        s3: true,
      });
      env.SYNC_PUBLIC_URL = "https://Sync.Example.com/";
      await expect(appController.getReadiness()).resolves.toEqual({
        status: "ready",
        s3: true,
        accountUrl: "https://sync.example.com",
      });
    });
  });
});

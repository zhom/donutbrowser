import { Test, type TestingModule } from "@nestjs/testing";
import { AppController } from "./app.controller.js";
import { AppService } from "./app.service.js";
import { SyncService } from "./sync/sync.service.js";

describe("AppController", () => {
  let appController: AppController;

  beforeEach(async () => {
    const app: TestingModule = await Test.createTestingModule({
      controllers: [AppController],
      providers: [
        AppService,
        {
          provide: SyncService,
          useValue: {
            checkS3Connectivity: jest.fn().mockResolvedValue(true),
          },
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
});

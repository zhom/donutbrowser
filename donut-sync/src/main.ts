import { NestFactory } from "@nestjs/core";
import type { NestExpressApplication } from "@nestjs/platform-express";
import { AppModule } from "./app.module.js";
import { normalizeSyncServerUrl } from "./auth/donut-accounts.js";

const INSECURE_DEFAULT_TOKENS = new Set([
  "secret-sync-token",
  "CHANGE_ME_generate_a_long_random_secret",
  "CHANGE_ME",
]);

function validateEnv() {
  const token = process.env.SYNC_TOKEN;
  const publicUrl = process.env.SYNC_PUBLIC_URL;
  if (publicUrl && !normalizeSyncServerUrl(publicUrl)) {
    console.error(
      "SYNC_PUBLIC_URL must be the http(s) address Donut Browser uses for this server, e.g. https://sync.example.com",
    );
    process.exit(1);
  }
  if (!token && !publicUrl && !process.env.SYNC_JWT_PUBLIC_KEY) {
    console.error(
      "Set SYNC_TOKEN, SYNC_PUBLIC_URL (Donut account sign-in), or both",
    );
    process.exit(1);
  }
  // A static SYNC_TOKEN is the only credential on a self-hosted server that is
  // typically exposed on 0.0.0.0, so reject the shipped placeholders and any
  // token short enough to brute-force.
  if (token && (INSECURE_DEFAULT_TOKENS.has(token) || token.length < 24)) {
    console.error(
      "SYNC_TOKEN is a known default or too short. Set a long, random secret, e.g. `openssl rand -hex 32`.",
    );
    process.exit(1);
  }
}

async function bootstrap() {
  validateEnv();

  const app = await NestFactory.create<NestExpressApplication>(AppModule);

  // biome-ignore lint/correctness/useHookAtTopLevel: NestJS method, not a React hook
  app.useBodyParser("json", { limit: "50mb" });

  app.enableCors({
    origin: "*",
    methods: ["GET", "POST", "PUT", "DELETE", "OPTIONS"],
    allowedHeaders: ["Content-Type", "Authorization"],
  });

  const port = process.env.PORT ?? 3929;
  await app.listen(port);
  console.log(`Donut Sync service running on port ${port}`);
}
void bootstrap();

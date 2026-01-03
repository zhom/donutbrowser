import { NestFactory } from "@nestjs/core";
import { AppModule } from "./app.module.js";

function validateEnv() {
  const required = ["SYNC_TOKEN"];
  const missing = required.filter((key) => !process.env[key]);
  if (missing.length > 0) {
    console.error(
      `Missing required environment variables: ${missing.join(", ")}`,
    );
    process.exit(1);
  }
}

async function bootstrap() {
  validateEnv();

  const app = await NestFactory.create(AppModule);

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

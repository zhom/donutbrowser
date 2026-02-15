import { NestFactory } from "@nestjs/core";
import { AppModule } from "./app.module.js";

function validateEnv() {
  if (!process.env.SYNC_TOKEN && !process.env.SYNC_JWT_PUBLIC_KEY) {
    console.error("Either SYNC_TOKEN or SYNC_JWT_PUBLIC_KEY must be set");
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

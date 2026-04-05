import { ListBucketsCommand, S3Client } from "@aws-sdk/client-s3";

export const TEST_SYNC_TOKEN = "test-sync-token";
export const TEST_S3_ENDPOINT = "http://127.0.0.1:8987";

export function configureTestEnv() {
  process.env.SYNC_TOKEN ||= TEST_SYNC_TOKEN;
  process.env.S3_ENDPOINT ||= TEST_S3_ENDPOINT;
  process.env.S3_ACCESS_KEY_ID ||= "minioadmin";
  process.env.S3_SECRET_ACCESS_KEY ||= "minioadmin";
  process.env.S3_BUCKET ||= "donut-sync-test";
  process.env.S3_FORCE_PATH_STYLE ||= "true";
}

export async function waitForTestS3(timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  const s3Client = new S3Client({
    endpoint: TEST_S3_ENDPOINT,
    region: "us-east-1",
    credentials: {
      accessKeyId: "minioadmin",
      secretAccessKey: "minioadmin",
    },
    forcePathStyle: true,
  });

  while (Date.now() < deadline) {
    try {
      await s3Client.send(new ListBucketsCommand({}));
      return;
    } catch {}

    await new Promise((resolve) => setTimeout(resolve, 500));
  }

  throw new Error(`Timed out waiting for S3 at ${TEST_S3_ENDPOINT}`);
}

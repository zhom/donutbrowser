import { DonutClient } from "../src/index.mts";
import { FakeDonut } from "./fake-donut.mts";

export const TOKEN = "test-token-abc123";

/** Start a fake app, point a client at it, and always shut the server down. */
export async function withClient<T>(
  work: (client: DonutClient, fake: FakeDonut) => Promise<T>,
): Promise<T> {
  const fake = await new FakeDonut().start();
  try {
    const client = new DonutClient({
      token: TOKEN,
      port: fake.port,
      timeoutMs: 5_000,
      env: {},
    });
    return await work(client, fake);
  } finally {
    await fake.stop();
  }
}

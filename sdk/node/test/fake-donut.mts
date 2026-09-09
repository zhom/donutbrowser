/**
 * A stand-in for the desktop app's local REST API.
 *
 * It records what the client sent, byte for byte, and answers with whatever
 * the test queued. Nothing here reaches the network: it binds an ephemeral
 * loopback port and is torn down with the test.
 */

import { createServer } from "node:http";
import type { IncomingMessage, Server, ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

export interface RecordedRequest {
  method: string;
  target: string;
  path: string;
  query: Record<string, string>;
  headers: Record<string, string>;
  rawBody: string;
  json: unknown;
}

export interface QueuedResponse {
  status: number;
  body: string;
  headers: Record<string, string>;
  contentType: string;
}

export class FakeDonut {
  requests: RecordedRequest[] = [];
  responses: QueuedResponse[] = [];
  #server: Server | undefined = undefined;

  enqueueJson(payload: unknown, status = 200): void {
    this.responses.push({
      status,
      body: JSON.stringify(payload),
      headers: {},
      contentType: "application/json",
    });
  }

  enqueueEmpty(status = 204): void {
    this.responses.push({ status, body: "", headers: {}, contentType: "application/json" });
  }

  enqueueError(status: number, body = "", headers: Record<string, string> = {}): void {
    this.responses.push({ status, body, headers, contentType: "text/plain" });
  }

  enqueueRaw(status: number, body: string, contentType = "text/html"): void {
    this.responses.push({ status, body, headers: {}, contentType });
  }

  get port(): number {
    if (this.#server === undefined) {
      throw new Error("the fake server is not running");
    }
    return (this.#server.address() as AddressInfo).port;
  }

  get last(): RecordedRequest {
    const request = this.requests.at(-1);
    if (request === undefined) {
      throw new Error("the client sent nothing");
    }
    return request;
  }

  async start(): Promise<this> {
    const server = createServer((incoming: IncomingMessage, outgoing: ServerResponse) => {
      const chunks: Buffer[] = [];
      incoming.on("data", (chunk: Buffer) => chunks.push(chunk));
      incoming.on("end", () => {
        const rawBody = Buffer.concat(chunks).toString("utf8");
        const url = new URL(incoming.url ?? "/", "http://127.0.0.1");
        const headers: Record<string, string> = {};
        for (const [key, value] of Object.entries(incoming.headers)) {
          headers[key.toLowerCase()] = Array.isArray(value) ? value.join(", ") : (value ?? "");
        }

        this.requests.push({
          method: incoming.method ?? "",
          target: incoming.url ?? "",
          path: url.pathname,
          query: Object.fromEntries(url.searchParams.entries()),
          headers,
          rawBody,
          json: rawBody === "" ? null : JSON.parse(rawBody),
        });

        const queued = this.responses.shift() ?? {
          status: 200,
          body: "{}",
          headers: {},
          contentType: "application/json",
        };
        for (const [name, value] of Object.entries(queued.headers)) {
          outgoing.setHeader(name, value);
        }
        if (queued.body !== "") {
          outgoing.setHeader("Content-Type", queued.contentType);
        }
        outgoing.writeHead(queued.status);
        outgoing.end(queued.body);
      });
    });

    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    this.#server = server;
    return this;
  }

  async stop(): Promise<void> {
    const server = this.#server;
    if (server === undefined) {
      return;
    }
    this.#server = undefined;
    server.closeAllConnections();
    await new Promise<void>((resolve, reject) => {
      server.close((error) => (error ? reject(error) : resolve()));
    });
  }
}

/** Start a fake server, hand it to `work`, and always shut it down again. */
export async function withFakeDonut<T>(work: (fake: FakeDonut) => Promise<T>): Promise<T> {
  const fake = await new FakeDonut().start();
  try {
    return await work(fake);
  } finally {
    await fake.stop();
  }
}

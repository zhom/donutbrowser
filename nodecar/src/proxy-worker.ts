import { Server } from "proxy-chain";
import { getProxyConfig } from "./proxy-storage";

/**
 * Run a proxy server as a worker process
 * @param id The proxy configuration ID
 */
export async function runProxyWorker(id: string): Promise<void> {
  // Get the proxy configuration
  const config = getProxyConfig(id);
  
  if (!config) {
    console.error(`Proxy configuration ${id} not found`);
    process.exit(1);
  }
  
  // Create a new proxy server
  const server = new Server({
    port: config.localPort,
    host: "localhost",
    prepareRequestFunction: () => {
      return {
        upstreamProxyUrl: config.upstreamUrl,
        ignoreUpstreamProxyCertificate: config.ignoreProxyCertificate || false,
      };
    },
  });
  
  // Handle process termination
  process.on("SIGTERM", async () => {
    console.log(`Proxy worker ${id} received SIGTERM, shutting down...`);
    await server.close(true);
    process.exit(0);
  });
  
  process.on("SIGINT", async () => {
    console.log(`Proxy worker ${id} received SIGINT, shutting down...`);
    await server.close(true);
    process.exit(0);
  });
  
  // Start the server
  try {
    await server.listen();
    console.log(`Proxy worker ${id} started on port ${server.port}`);
    console.log(`Forwarding to upstream proxy: ${config.upstreamUrl}`);
  } catch (error) {
    console.error(`Failed to start proxy worker ${id}:`, error);
    process.exit(1);
  }
}
import { Server } from "proxy-chain";
import { getProxyConfig, updateProxyConfig } from "./proxy-storage";

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
    host: "127.0.0.1",
    prepareRequestFunction: () => {
      // If upstreamUrl is "DIRECT", don't use upstream proxy
      if (config.upstreamUrl === "DIRECT") {
        return {};
      }
      return {
        upstreamProxyUrl: config.upstreamUrl,
        ignoreUpstreamProxyCertificate: config.ignoreProxyCertificate ?? false,
      };
    },
  });

  // Handle process termination gracefully
  const gracefulShutdown = async (signal: string) => {
    console.log(`Proxy worker ${id} received ${signal}, shutting down...`);
    try {
      await server.close(true);
      console.log(`Proxy worker ${id} shut down successfully`);
    } catch (error) {
      console.error(`Error during shutdown for proxy ${id}:`, error);
    }
    process.exit(0);
  };

  process.on("SIGTERM", () => void gracefulShutdown("SIGTERM"));
  process.on("SIGINT", () => void gracefulShutdown("SIGINT"));

  // Handle uncaught exceptions
  process.on("uncaughtException", (error) => {
    console.error(`Uncaught exception in proxy worker ${id}:`, error);
    process.exit(1);
  });

  process.on("unhandledRejection", (reason) => {
    console.error(`Unhandled rejection in proxy worker ${id}:`, reason);
    process.exit(1);
  });

  // Start the server
  try {
    await server.listen();

    // Update the config with the actual port (in case it was auto-assigned)
    config.localPort = server.port;
    config.localUrl = `http://127.0.0.1:${server.port}`;
    updateProxyConfig(config);

    console.log(`Proxy worker ${id} started on port ${server.port}`);
    console.log(`Forwarding to upstream proxy: ${config.upstreamUrl}`);

    // Keep the process alive
    setInterval(() => {
      // Do nothing, just keep the process alive
    }, 60000);
  } catch (error) {
    console.error(`Failed to start proxy worker ${id}:`, error);
    process.exit(1);
  }
}

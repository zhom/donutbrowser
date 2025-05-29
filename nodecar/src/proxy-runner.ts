import { spawn } from "child_process";
import path from "path";
import getPort from "get-port";
import {
  type ProxyConfig,
  saveProxyConfig,
  getProxyConfig,
  deleteProxyConfig,
  isProcessRunning,
  generateProxyId,
} from "./proxy-storage";

/**
 * Start a proxy in a separate process
 * @param upstreamUrl The upstream proxy URL
 * @param options Optional configuration
 * @returns Promise resolving to the proxy configuration
 */
export async function startProxyProcess(
  upstreamUrl: string,
  options: { port?: number; ignoreProxyCertificate?: boolean } = {}
): Promise<ProxyConfig> {
  // Generate a unique ID for this proxy
  const id = generateProxyId();

  // Get a random available port if not specified
  const port = options.port || (await getPort());

  // Create the proxy configuration
  const config: ProxyConfig = {
    id,
    upstreamUrl,
    localPort: port,
    ignoreProxyCertificate: options.ignoreProxyCertificate || false,
  };

  // Save the configuration before starting the process
  saveProxyConfig(config);

  // Build the command arguments
  const args = ["proxy-worker", "start", "--id", id];

  // Spawn the process
  const child = spawn(
    process.execPath,
    [path.join(__dirname, "index.js"), ...args],
    {
      detached: true,
      stdio: "ignore",
    }
  );

  // Unref the child to allow the parent to exit independently
  child.unref();

  // Store the process ID
  config.pid = child.pid;
  config.localUrl = `http://localhost:${port}`;

  // Update the configuration with the process ID
  saveProxyConfig(config);

  // Wait a bit to ensure the proxy has started
  await new Promise((resolve) => setTimeout(resolve, 500));

  return config;
}

/**
 * Stop a proxy process
 * @param id The proxy ID to stop
 * @returns Promise resolving to true if stopped, false if not found
 */
export async function stopProxyProcess(id: string): Promise<boolean> {
  const config = getProxyConfig(id);

  if (!config || !config.pid) {
    return false;
  }

  try {
    // Check if the process is running
    if (isProcessRunning(config.pid)) {
      // Send SIGTERM to the process
      process.kill(config.pid);

      // Wait a bit to ensure the process has terminated
      await new Promise((resolve) => setTimeout(resolve, 300));
    }

    // Delete the configuration
    deleteProxyConfig(id);

    return true;
  } catch (error) {
    console.error(`Error stopping proxy ${id}:`, error);
    return false;
  }
}

/**
 * Stop all proxy processes
 * @returns Promise resolving when all proxies are stopped
 */
export async function stopAllProxyProcesses(): Promise<void> {
  const configs = require("./proxy-storage").listProxyConfigs();

  for (const config of configs) {
    await stopProxyProcess(config.id);
  }
}

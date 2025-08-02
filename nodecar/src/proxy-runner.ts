import { spawn } from "node:child_process";
import path from "node:path";
import getPort from "get-port";
import {
  deleteProxyConfig,
  generateProxyId,
  getProxyConfig,
  isProcessRunning,
  listProxyConfigs,
  type ProxyConfig,
  saveProxyConfig,
} from "./proxy-storage";

/**
 * Start a proxy in a separate process
 * @param upstreamUrl The upstream proxy URL (optional for direct proxy)
 * @param options Optional configuration
 * @returns Promise resolving to the proxy configuration
 */
export async function startProxyProcess(
  upstreamUrl?: string,
  options: { port?: number; ignoreProxyCertificate?: boolean } = {},
): Promise<ProxyConfig> {
  // Generate a unique ID for this proxy
  const id = generateProxyId();

  // Get a random available port if not specified
  const port = options.port ?? (await getPort());

  // Create the proxy configuration
  const config: ProxyConfig = {
    id,
    upstreamUrl: upstreamUrl || "DIRECT",
    localPort: port,
    ignoreProxyCertificate: options.ignoreProxyCertificate ?? false,
  };

  // Save the configuration before starting the process
  saveProxyConfig(config);

  // Build the command arguments
  const args = [
    path.join(__dirname, "index.js"),
    "proxy-worker",
    "start",
    "--id",
    id,
  ];

  // Spawn the process with proper detachment
  const child = spawn(process.execPath, args, {
    detached: true,
    stdio: ["ignore", "ignore", "ignore"], // Completely ignore all stdio
    cwd: process.cwd(),
  });

  // Unref the child to allow the parent to exit independently
  child.unref();

  // Store the process ID and local URL
  config.pid = child.pid;
  config.localUrl = `http://127.0.0.1:${port}`;

  // Update the configuration with the process ID
  saveProxyConfig(config);

  // Give the worker a moment to start before returning
  await new Promise((resolve) => setTimeout(resolve, 100));

  return config;
}

/**
 * Stop a proxy process
 * @param id The proxy ID to stop
 * @returns Promise resolving to true if stopped, false if not found
 */
export async function stopProxyProcess(id: string): Promise<boolean> {
  const config = getProxyConfig(id);

  if (!config?.pid) {
    // Try to delete the config anyway in case it exists without a PID
    deleteProxyConfig(id);
    return false;
  }

  try {
    // Check if the process is running
    if (isProcessRunning(config.pid)) {
      // Send SIGTERM to the process
      process.kill(config.pid, "SIGTERM");

      // Wait a bit to ensure the process has terminated
      await new Promise((resolve) => setTimeout(resolve, 500));

      // If still running, send SIGKILL
      if (isProcessRunning(config.pid)) {
        process.kill(config.pid, "SIGKILL");
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
    }

    // Delete the configuration
    deleteProxyConfig(id);

    return true;
  } catch (error) {
    console.error(`Error stopping proxy ${id}:`, error);
    // Delete the configuration even if stopping failed
    deleteProxyConfig(id);
    return false;
  }
}

/**
 * Stop all proxy processes
 * @returns Promise resolving when all proxies are stopped
 */
export async function stopAllProxyProcesses(): Promise<void> {
  const configs = listProxyConfigs();

  const stopPromises = configs.map((config) => stopProxyProcess(config.id));
  await Promise.all(stopPromises);
}

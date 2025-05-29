import { 
  startProxyProcess, 
  stopProxyProcess, 
  stopAllProxyProcesses 
} from "./proxy-runner";
import { listProxyConfigs } from "./proxy-storage";

// Type definitions
interface ProxyOptions {
  port?: number;
  ignoreProxyCertificate?: boolean;
}

/**
 * Start a local proxy server that forwards to an upstream proxy
 * @param upstreamProxyUrl The upstream proxy URL (protocol://[username:password@]host:port)
 * @param options Optional configuration
 * @returns Promise resolving to the local proxy URL
 */
export async function startProxy(
  upstreamProxyUrl: string,
  options: ProxyOptions = {}
): Promise<string> {
  const config = await startProxyProcess(upstreamProxyUrl, {
    port: options.port,
    ignoreProxyCertificate: options.ignoreProxyCertificate,
  });
  
  return config.localUrl || `http://localhost:${config.localPort}`;
}

/**
 * Stop a specific proxy by its upstream URL
 * @param upstreamProxyUrl The upstream proxy URL to stop
 * @returns Promise resolving to true if proxy was found and stopped, false otherwise
 */
export async function stopProxy(upstreamProxyUrl: string): Promise<boolean> {
  // Find all proxies with this upstream URL
  const configs = listProxyConfigs().filter(
    config => config.upstreamUrl === upstreamProxyUrl
  );
  
  if (configs.length === 0) {
    return false;
  }
  
  // Stop all matching proxies
  let success = true;
  for (const config of configs) {
    const stopped = await stopProxyProcess(config.id);
    if (!stopped) {
      success = false;
    }
  }
  
  return success;
}

/**
 * Get a list of all active proxy upstream URLs
 * @returns Array of upstream proxy URLs
 */
export function getActiveProxies(): string[] {
  return listProxyConfigs().map(config => config.upstreamUrl);
}

/**
 * Stop all active proxies
 * @returns Promise that resolves when all proxies are stopped
 */
export async function stopAllProxies(): Promise<void> {
  await stopAllProxyProcesses();
}

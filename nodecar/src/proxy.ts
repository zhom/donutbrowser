import {
  startProxyProcess,
  stopProxyProcess,
  stopAllProxyProcesses,
} from "./proxy-runner";
import { listProxyConfigs } from "./proxy-storage";

// Type definitions
interface ProxyOptions {
  port?: number;
  ignoreProxyCertificate?: boolean;
  username?: string;
  password?: string;
}

/**
 * Start a local proxy server that forwards to an upstream proxy
 * @param upstreamProxyHost The upstream proxy host
 * @param upstreamProxyPort The upstream proxy port
 * @param upstreamProxyType The upstream proxy type (http, https, socks4, socks5)
 * @param options Optional configuration including credentials
 * @returns Promise resolving to the local proxy URL
 */
export async function startProxy(
  upstreamProxyHost: string,
  upstreamProxyPort: number,
  upstreamProxyType: string,
  options: ProxyOptions = {}
): Promise<string> {
  // Construct the upstream proxy URL with credentials if provided
  let upstreamProxyUrl: string;
  if (options.username && options.password) {
    upstreamProxyUrl = `${upstreamProxyType}://${options.username}:${options.password}@${upstreamProxyHost}:${upstreamProxyPort}`;
  } else {
    upstreamProxyUrl = `${upstreamProxyType}://${upstreamProxyHost}:${upstreamProxyPort}`;
  }

  const config = await startProxyProcess(upstreamProxyUrl, {
    port: options.port,
    ignoreProxyCertificate: options.ignoreProxyCertificate,
  });

  return config.localUrl || `http://localhost:${config.localPort}`;
}

/**
 * Stop a specific proxy by its upstream host, port, and type
 * @param upstreamProxyHost The upstream proxy host
 * @param upstreamProxyPort The upstream proxy port
 * @param upstreamProxyType The upstream proxy type
 * @returns Promise resolving to true if proxy was found and stopped, false otherwise
 */
export async function stopProxy(
  upstreamProxyHost: string,
  upstreamProxyPort: number,
  upstreamProxyType: string
): Promise<boolean> {
  // Find all proxies with matching upstream details (ignoring credentials in URL)
  const configs = listProxyConfigs().filter((config) => {
    // Parse the upstream URL to extract host, port, and type
    try {
      const url = new URL(config.upstreamUrl);
      return (
        url.hostname === upstreamProxyHost &&
        Number.parseInt(url.port) === upstreamProxyPort &&
        url.protocol.replace(":", "") === upstreamProxyType
      );
    } catch {
      return false;
    }
  });

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
  return listProxyConfigs().map((config) => config.upstreamUrl);
}

/**
 * Stop all active proxies
 * @returns Promise that resolves when all proxies are stopped
 */
export async function stopAllProxies(): Promise<void> {
  await stopAllProxyProcesses();
}

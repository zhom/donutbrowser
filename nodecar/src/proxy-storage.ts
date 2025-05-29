import fs from "fs";
import path from "path";
import os from "os";

// Define the proxy configuration type
export interface ProxyConfig {
  id: string;
  upstreamUrl: string;
  localPort?: number;
  ignoreProxyCertificate?: boolean;
  localUrl?: string;
  pid?: number;
}

// Path to store proxy configurations
const STORAGE_DIR = path.join(os.tmpdir(), "donutbrowser", "proxies");

// Ensure storage directory exists
if (!fs.existsSync(STORAGE_DIR)) {
  fs.mkdirSync(STORAGE_DIR, { recursive: true });
}

/**
 * Save a proxy configuration to disk
 * @param config The proxy configuration to save
 */
export function saveProxyConfig(config: ProxyConfig): void {
  const filePath = path.join(STORAGE_DIR, `${config.id}.json`);
  fs.writeFileSync(filePath, JSON.stringify(config, null, 2));
}

/**
 * Get a proxy configuration by ID
 * @param id The proxy ID
 * @returns The proxy configuration or null if not found
 */
export function getProxyConfig(id: string): ProxyConfig | null {
  const filePath = path.join(STORAGE_DIR, `${id}.json`);

  if (!fs.existsSync(filePath)) {
    return null;
  }

  try {
    const content = fs.readFileSync(filePath, "utf-8");
    return JSON.parse(content) as ProxyConfig;
  } catch (error) {
    console.error(`Error reading proxy config ${id}:`, error);
    return null;
  }
}

/**
 * Delete a proxy configuration
 * @param id The proxy ID to delete
 * @returns True if deleted, false if not found
 */
export function deleteProxyConfig(id: string): boolean {
  const filePath = path.join(STORAGE_DIR, `${id}.json`);

  if (!fs.existsSync(filePath)) {
    return false;
  }

  try {
    fs.unlinkSync(filePath);
    return true;
  } catch (error) {
    console.error(`Error deleting proxy config ${id}:`, error);
    return false;
  }
}

/**
 * List all saved proxy configurations
 * @returns Array of proxy configurations
 */
export function listProxyConfigs(): ProxyConfig[] {
  if (!fs.existsSync(STORAGE_DIR)) {
    return [];
  }

  try {
    return fs
      .readdirSync(STORAGE_DIR)
      .filter((file) => file.endsWith(".json"))
      .map((file) => {
        try {
          const content = fs.readFileSync(
            path.join(STORAGE_DIR, file),
            "utf-8"
          );
          return JSON.parse(content) as ProxyConfig;
        } catch (error) {
          console.error(`Error reading proxy config ${file}:`, error);
          return null;
        }
      })
      .filter((config): config is ProxyConfig => config !== null);
  } catch (error) {
    console.error("Error listing proxy configs:", error);
    return [];
  }
}

/**
 * Update a proxy configuration
 * @param config The proxy configuration to update
 * @returns True if updated, false if not found
 */
export function updateProxyConfig(config: ProxyConfig): boolean {
  const filePath = path.join(STORAGE_DIR, `${config.id}.json`);

  if (!fs.existsSync(filePath)) {
    return false;
  }

  try {
    fs.writeFileSync(filePath, JSON.stringify(config, null, 2));
    return true;
  } catch (error) {
    console.error(`Error updating proxy config ${config.id}:`, error);
    return false;
  }
}

/**
 * Check if a proxy process is running
 * @param pid The process ID to check
 * @returns True if running, false otherwise
 */
export function isProcessRunning(pid: number): boolean {
  try {
    // The kill method with signal 0 doesn't actually kill the process
    // but checks if it exists
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return false;
  }
}

/**
 * Generate a unique ID for a proxy
 * @returns A unique ID string
 */
export function generateProxyId(): string {
  return `proxy_${Date.now()}_${Math.floor(Math.random() * 10000)}`;
}

import { spawn } from "node:child_process";
import path from "node:path";
import {
  type CamoufoxConfig,
  deleteCamoufoxConfig,
  generateCamoufoxId,
  getCamoufoxConfig,
  listCamoufoxConfigs,
  saveCamoufoxConfig,
} from "./camoufox-storage.js";

export interface CamoufoxLaunchOptions {
  // Operating system to use for fingerprint generation
  os?: "windows" | "macos" | "linux"[];

  // Blocking options
  block_images?: boolean;
  block_webrtc?: boolean;
  block_webgl?: boolean;

  // Security options
  disable_coop?: boolean;

  // Geolocation options
  geoip?: string | boolean;

  // UI behavior
  humanize?: boolean | number;

  // Localization
  locale?: string | string[];

  // Extensions and fonts
  addons?: string[];
  fonts?: string[];
  custom_fonts_only?: boolean;
  exclude_addons?: "UBO"[];

  // Screen and window
  screen?: {
    minWidth?: number;
    maxWidth?: number;
    minHeight?: number;
    maxHeight?: number;
  };
  window?: [number, number];

  fingerprint?: any;
  disableTheming?: boolean;
  showcursor?: boolean;

  // Version and mode
  ff_version?: number;
  headless?: boolean;
  main_world_eval?: boolean;

  // Custom executable path
  executable_path?: string;

  // Firefox preferences
  firefox_user_prefs?: Record<string, unknown>;
  user_data_dir?: string;

  // Proxy settings
  proxy?:
    | string
    | {
        server: string;
        username?: string;
        password?: string;
        bypass?: string;
      };

  // Cache and performance
  enable_cache?: boolean;

  // Additional options
  args?: string[];
  env?: Record<string, string | number | boolean>;
  debug?: boolean;
  virtual_display?: string;
  webgl_config?: [string, string];

  // Custom options
  timezone?: string;
  country?: string;
  geolocation?: {
    latitude: number;
    longitude: number;
    accuracy?: number;
  };
}

/**
 * Start a Camoufox instance in a separate process
 * @param options Camoufox launch options
 * @param profilePath Profile directory path
 * @param url Optional URL to open
 * @returns Promise resolving to the Camoufox configuration
 */
export async function startCamoufoxProcess(
  options: CamoufoxLaunchOptions = {},
  profilePath?: string,
  url?: string,
): Promise<CamoufoxConfig> {
  // Generate a unique ID for this instance
  const id = generateCamoufoxId();

  // Create the Camoufox configuration
  const config: CamoufoxConfig = {
    id,
    options,
    profilePath,
    url,
  };

  // Save the configuration before starting the process
  saveCamoufoxConfig(config);

  // Build the command arguments
  const args = [
    path.join(__dirname, "index.js"),
    "camoufox-worker",
    "start",
    "--id",
    id,
  ];

  // Spawn the process with proper detachment
  const child = spawn(process.execPath, args, {
    detached: true,
    stdio: ["ignore", "pipe", "pipe"], // Capture stdout and stderr for debugging
    cwd: process.cwd(),
    env: { ...process.env, NODE_ENV: "production" }, // Ensure consistent environment
  });

  saveCamoufoxConfig(config);

  // Wait for the worker to start successfully or fail
  return new Promise<CamoufoxConfig>((resolve, reject) => {
    let resolved = false;
    let stdoutBuffer = "";
    let stderrBuffer = "";

    const timeout = setTimeout(() => {
      if (!resolved) {
        resolved = true;
        reject(
          new Error(`Camoufox worker ${id} startup timeout after 30 seconds`),
        );
      }
    }, 30000);

    // Handle stdout - look for success JSON
    if (child.stdout) {
      child.stdout.on("data", (data) => {
        const output = data.toString();
        stdoutBuffer += output;

        // Look for success JSON message
        const lines = stdoutBuffer.split("\n");
        for (const line of lines) {
          if (line.trim()) {
            try {
              const parsed = JSON.parse(line.trim());
              if (
                parsed.success &&
                parsed.id === id &&
                parsed.port &&
                parsed.wsEndpoint
              ) {
                if (!resolved) {
                  resolved = true;
                  clearTimeout(timeout);
                  // Update config with server details
                  config.port = parsed.port;
                  config.wsEndpoint = parsed.wsEndpoint;
                  saveCamoufoxConfig(config);
                  child.unref(); // Allow parent to exit independently
                  resolve(config);
                  return;
                }
              }
            } catch {
              // Not JSON, continue
            }
          }
        }
      });
    }

    // Handle stderr - look for error JSON
    if (child.stderr) {
      child.stderr.on("data", (data) => {
        const output = data.toString();
        stderrBuffer += output;

        // Look for error JSON message
        const lines = stderrBuffer.split("\n");
        for (const line of lines) {
          if (line.trim()) {
            try {
              const parsed = JSON.parse(line.trim());
              if (parsed.error && parsed.id === id) {
                if (!resolved) {
                  resolved = true;
                  clearTimeout(timeout);
                  reject(
                    new Error(
                      `Camoufox worker failed: ${parsed.message || parsed.error}`,
                    ),
                  );
                  return;
                }
              }
            } catch {
              // Not JSON, continue
            }
          }
        }
      });
    }

    child.on("exit", (code, signal) => {
      if (!resolved) {
        resolved = true;
        clearTimeout(timeout);
        if (code !== 0) {
          reject(
            new Error(
              `Camoufox worker ${id} exited with code ${code} and signal ${signal}. Stderr: ${stderrBuffer}`,
            ),
          );
        } else {
          // Process exited successfully but we didn't get success message
          reject(
            new Error(
              `Camoufox worker ${id} exited without success confirmation`,
            ),
          );
        }
      }
    });
  });
}

/**
 * Stop a Camoufox process
 * @param id The Camoufox ID to stop
 * @returns Promise resolving to true if stopped, false if not found
 */
export async function stopCamoufoxProcess(id: string): Promise<boolean> {
  const config = getCamoufoxConfig(id);

  if (!config) {
    return false;
  }

  try {
    // If we have a port, try to gracefully shutdown the server
    if (config.port) {
      try {
        await fetch(`http://localhost:${config.port}/shutdown`, {
          method: "POST",
          signal: AbortSignal.timeout(5000),
        });
        // Wait a bit for graceful shutdown
        await new Promise((resolve) => setTimeout(resolve, 1000));
      } catch {
        // Graceful shutdown failed, continue with force stop
      }
    }

    // Delete the configuration
    deleteCamoufoxConfig(id);
    return true;
  } catch (error) {
    // Delete the configuration even if stopping failed
    deleteCamoufoxConfig(id);
    return false;
  }
}

/**
 * Stop all Camoufox processes
 * @returns Promise resolving when all instances are stopped
 */
export async function stopAllCamoufoxProcesses(): Promise<void> {
  const configs = listCamoufoxConfigs();

  const stopPromises = configs.map((config) => stopCamoufoxProcess(config.id));
  await Promise.all(stopPromises);
}

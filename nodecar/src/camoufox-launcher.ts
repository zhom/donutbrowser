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
  os?: "windows" | "macos" | "linux" | ("windows" | "macos" | "linux")[];

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

  // Spawn the process with proper detachment - similar to proxy implementation
  const child = spawn(process.execPath, args, {
    detached: true,
    stdio: ["ignore", "pipe", "pipe"], // Capture stdout and stderr for startup feedback
    cwd: process.cwd(),
    env: {
      ...process.env,
      NODE_ENV: "production",
      // Ensure Camoufox can find its dependencies
      NODE_PATH: process.env.NODE_PATH || "",
    },
  });

  // Wait for the worker to start successfully or fail - with shorter timeout for quick response
  return new Promise<CamoufoxConfig>((resolve, reject) => {
    let resolved = false;
    let stdoutBuffer = "";
    let stderrBuffer = "";

    // Shorter timeout for quick startup feedback
    const timeout = setTimeout(() => {
      if (!resolved) {
        resolved = true;
        child.kill("SIGKILL");
        reject(
          new Error(`Camoufox worker ${id} startup timeout after 5 seconds`),
        );
      }
    }, 5000);

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
              if (parsed.success && parsed.id === id && parsed.port) {
                if (!resolved) {
                  resolved = true;
                  clearTimeout(timeout);
                  // Update config with server details
                  config.port = parsed.port;
                  config.wsEndpoint = parsed.wsEndpoint;
                  saveCamoufoxConfig(config);
                  // Unref immediately after success to detach properly
                  child.unref();
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
    // Try to find and kill the worker process using multiple methods
    const { spawn } = await import("node:child_process");

    // Method 1: Kill by process pattern
    const killByPattern = spawn("pkill", ["-f", `camoufox-worker.*${id}`], {
      stdio: "ignore",
    });

    // Method 2: If we have a port (which is actually the process PID), kill by PID
    if (config.port) {
      try {
        process.kill(config.port, "SIGTERM");

        // Give it a moment to terminate gracefully
        await new Promise((resolve) => setTimeout(resolve, 2000));

        // Force kill if still running
        try {
          process.kill(config.port, "SIGKILL");
        } catch {
          // Process already terminated
        }
      } catch (error) {
        // Process not found or already terminated
      }
    }

    // Wait for pattern-based kill command to complete
    await new Promise<void>((resolve) => {
      killByPattern.on("exit", () => resolve());
      // Timeout after 3 seconds
      setTimeout(() => resolve(), 3000);
    });

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

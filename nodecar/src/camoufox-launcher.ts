import { spawn } from "node:child_process";
import path from "node:path";
import { launchOptions } from "donutbrowser-camoufox-js";
import type { LaunchOptions } from "donutbrowser-camoufox-js/dist/utils.js";
import {
  type CamoufoxConfig,
  deleteCamoufoxConfig,
  generateCamoufoxId,
  getCamoufoxConfig,
  listCamoufoxConfigs,
  saveCamoufoxConfig,
} from "./camoufox-storage.js";

/**
 * Convert camoufox fingerprint format to fingerprint-generator format
 * @param camoufoxFingerprint The camoufox fingerprint object
 * @returns fingerprint-generator object
 */
function convertCamoufoxToFingerprintGenerator(
  camoufoxFingerprint: Record<string, any>,
): any {
  const fingerprintObj: Record<string, any> = {
    navigator: {},
    screen: {},
    videoCard: {},
    headers: {},
    battery: {},
  };

  // Mapping from camoufox keys to fingerprint-generator structure based on the YAML
  const mappings: Record<string, string> = {
    // Navigator properties
    "navigator.userAgent": "navigator.userAgent",
    "navigator.platform": "navigator.platform",
    "navigator.hardwareConcurrency": "navigator.hardwareConcurrency",
    "navigator.maxTouchPoints": "navigator.maxTouchPoints",
    "navigator.doNotTrack": "navigator.doNotTrack",
    "navigator.appCodeName": "navigator.appCodeName",
    "navigator.appName": "navigator.appName",
    "navigator.appVersion": "navigator.appVersion",
    "navigator.oscpu": "navigator.oscpu",
    "navigator.product": "navigator.product",
    "navigator.language": "navigator.language",
    "navigator.languages": "navigator.languages",
    "navigator.globalPrivacyControl": "navigator.globalPrivacyControl",

    // Screen properties
    "screen.width": "screen.width",
    "screen.height": "screen.height",
    "screen.availWidth": "screen.availWidth",
    "screen.availHeight": "screen.availHeight",
    "screen.availTop": "screen.availTop",
    "screen.availLeft": "screen.availLeft",
    "screen.colorDepth": "screen.colorDepth",
    "screen.pixelDepth": "screen.pixelDepth",
    "window.outerWidth": "screen.outerWidth",
    "window.outerHeight": "screen.outerHeight",
    "window.innerWidth": "screen.innerWidth",
    "window.innerHeight": "screen.innerHeight",
    "window.screenX": "screen.screenX",
    "window.screenY": "screen.screenY",
    "screen.pageXOffset": "screen.pageXOffset",
    "screen.pageYOffset": "screen.pageYOffset",
    "window.devicePixelRatio": "screen.devicePixelRatio",
    "document.body.clientWidth": "screen.clientWidth",
    "document.body.clientHeight": "screen.clientHeight",

    // WebGL properties
    "webGl:vendor": "videoCard.vendor",
    "webGl:renderer": "videoCard.renderer",

    // Headers
    "headers.Accept-Encoding": "headers.Accept-Encoding",

    // Battery
    "battery:charging": "battery.charging",
    "battery:chargingTime": "battery.chargingTime",
    "battery:dischargingTime": "battery.dischargingTime",
  };

  // Apply mappings
  for (const [camoufoxKey, fingerprintPath] of Object.entries(mappings)) {
    if (camoufoxFingerprint[camoufoxKey] !== undefined) {
      const pathParts = fingerprintPath.split(".");
      let current = fingerprintObj;

      // Navigate to the nested property, creating objects as needed
      for (let i = 0; i < pathParts.length - 1; i++) {
        const part = pathParts[i];
        if (!current[part]) {
          current[part] = {};
        }
        current = current[part];
      }

      // Set the final value
      const finalKey = pathParts[pathParts.length - 1];
      current[finalKey] = camoufoxFingerprint[camoufoxKey];
    }
  }

  // Handle fonts separately
  if (camoufoxFingerprint.fonts && Array.isArray(camoufoxFingerprint.fonts)) {
    fingerprintObj.fonts = camoufoxFingerprint.fonts;
  }

  return { ...camoufoxFingerprint, ...fingerprintObj };
}

/**
 * Start a Camoufox instance in a separate process
 * @param options Camoufox launch options
 * @param profilePath Profile directory path
 * @param url Optional URL to open
 * @returns Promise resolving to the Camoufox configuration
 */
export async function startCamoufoxProcess(
  options: LaunchOptions = {},
  profilePath?: string,
  url?: string,
  customConfig?: string,
): Promise<CamoufoxConfig> {
  // Generate a unique ID for this instance
  const id = generateCamoufoxId();

  // Ensure profile path is absolute if provided
  const absoluteProfilePath = profilePath
    ? path.resolve(profilePath)
    : undefined;

  // Create the Camoufox configuration
  const config: CamoufoxConfig = {
    id,
    options: JSON.parse(JSON.stringify(options)), // Deep clone to avoid reference sharing
    profilePath: absoluteProfilePath,
    url,
    customConfig,
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
              if (parsed.success && parsed.id === id && parsed.processId) {
                if (!resolved) {
                  resolved = true;
                  clearTimeout(timeout);
                  config.processId = parsed.processId;
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
    // Method 1: If we have a process ID, kill by PID with proper signal sequence
    if (config.processId) {
      try {
        // First try SIGTERM for graceful shutdown
        process.kill(config.processId, "SIGTERM");
        // Give it more time to terminate gracefully (increased from 2s to 5s)
        await new Promise((resolve) => setTimeout(resolve, 5000));

        // Check if process is still running
        try {
          process.kill(config.processId, 0); // Signal 0 checks if process exists
          process.kill(config.processId, "SIGKILL");
        } catch {}
      } catch {}
    }

    // Method 2: Pattern-based kill as fallback
    const killByPattern = spawn(
      "pkill",
      ["-TERM", "-f", `camoufox-worker.*${id}`],
      {
        stdio: "ignore",
      },
    );

    // Wait for pattern-based kill command to complete
    await new Promise<void>((resolve) => {
      killByPattern.on("exit", () => resolve());
      // Timeout after 3 seconds
      setTimeout(() => resolve(), 3000);
    });

    // Final cleanup with SIGKILL if needed
    setTimeout(() => {
      spawn("pkill", ["-KILL", "-f", `camoufox-worker.*${id}`], {
        stdio: "ignore",
      });
    }, 1000);

    // Delete the configuration
    deleteCamoufoxConfig(id);
    return true;
  } catch {
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

interface GenerateConfigOptions {
  proxy?: string;
  maxWidth?: number;
  maxHeight?: number;
  minWidth?: number;
  minHeight?: number;
  geoip?: string | boolean;
  blockImages?: boolean;
  blockWebrtc?: boolean;
  blockWebgl?: boolean;
  executablePath?: string;
  fingerprint?: string;
}

/**
 * Generate Camoufox configuration using launchOptions
 * @param options Configuration options
 * @returns Promise resolving to the generated config JSON string
 */
export async function generateCamoufoxConfig(
  options: GenerateConfigOptions,
): Promise<string> {
  try {
    const launchOpts: any = {
      headless: false,
      i_know_what_im_doing: true,
      config: {
        disableTheming: true,
        showcursor: false,
      },
    };

    if (options.geoip) {
      launchOpts.geoip = true;
    }

    if (options.blockImages) {
      launchOpts.block_images = true;
    }
    if (options.blockWebrtc) {
      launchOpts.block_webrtc = true;
    }
    if (options.blockWebgl) {
      launchOpts.block_webgl = true;
    }

    if (options.executablePath) {
      launchOpts.executable_path = options.executablePath;
    }

    if (options.proxy) {
      launchOpts.proxy = options.proxy;
    }

    // If fingerprint is provided, use it and ignore other options except executable_path and block_*
    if (options.fingerprint) {
      try {
        const camoufoxFingerprint = JSON.parse(options.fingerprint);

        if (camoufoxFingerprint.timezone) {
          launchOpts.config.timezone = camoufoxFingerprint.timezone;
        }

        // Convert camoufox fingerprint format to fingerprint-generator format
        const fingerprintObj =
          convertCamoufoxToFingerprintGenerator(camoufoxFingerprint);
        launchOpts.fingerprint = fingerprintObj;
      } catch (error) {
        throw new Error(`Invalid fingerprint JSON: ${error}`);
      }
    } else {
      // Use individual options to build configuration

      // Build screen configuration with min/max dimensions
      const screen: {
        minWidth?: number;
        maxWidth?: number;
        minHeight?: number;
        maxHeight?: number;
      } = {};

      if (options.minWidth) screen.minWidth = options.minWidth;
      if (options.maxWidth) screen.maxWidth = options.maxWidth;
      if (options.minHeight) screen.minHeight = options.minHeight;
      if (options.maxHeight) screen.maxHeight = options.maxHeight;

      if (Object.keys(screen).length > 0) {
        launchOpts.screen = screen;
      }
    }

    // Generate the configuration using launchOptions
    const generatedOptions = await launchOptions(launchOpts);

    // Extract the environment variables that contain the config
    const envVars = generatedOptions.env || {};

    // Reconstruct the config from environment variables using getEnvVars utility
    let configStr = "";
    let chunkIndex = 1;

    while (envVars[`CAMOU_CONFIG_${chunkIndex}`]) {
      configStr += envVars[`CAMOU_CONFIG_${chunkIndex}`];
      chunkIndex++;
    }

    if (!configStr) {
      throw new Error("No configuration generated");
    }

    // Parse and return the config as JSON string
    const config = JSON.parse(configStr);
    return JSON.stringify(config);
  } catch (error) {
    throw new Error(`Failed to generate Camoufox config: ${error}`);
  }
}
